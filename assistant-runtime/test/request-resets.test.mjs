// A connection that drops on a model request must not throw away the task:
// the request is asked again, and what it may have cost counts toward the
// user's limit instead of ending the run.
import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { AssistantRuntime } from "../src/runtime.mjs";

const limits = {
  repeatWarning: 3,
  repeatStop: 5,
  maxDurationSeconds: 900,
  maxCostUsd: 1,
  maxToolCalls: 60,
  completionRetries: 2,
};
const sse = (chunks) =>
  new Response(
    chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`).join("") +
      "data: [DONE]\n\n",
    { headers: { "content-type": "text/event-stream" } },
  );
const usage = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 };
const reply = () =>
  sse([
    {
      choices: [
        { index: 0, delta: { role: "assistant", content: "Finished." } },
      ],
    },
    { choices: [{ index: 0, delta: {}, finish_reason: "stop" }], usage },
  ]);
const call = (name, args) =>
  sse([
    {
      choices: [
        {
          index: 0,
          delta: {
            role: "assistant",
            tool_calls: [
              {
                index: 0,
                id: "reset-call",
                type: "function",
                function: { name, arguments: JSON.stringify(args) },
              },
            ],
          },
        },
      ],
    },
    { choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }], usage },
  ]);

// A server that resets every connection on its first bytes. Reached over
// https those bytes are the TLS ClientHello (the request is never written);
// over http they are the request itself.
async function resetting(t) {
  const server = createServer((socket) => {
    socket.on("error", () => {});
    socket.once("data", () => socket.resetAndDestroy());
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => new Promise((resolve) => server.close(resolve)));
  return server.address().port;
}
const through = (scheme, port) => (options) =>
  fetch(`${scheme}://127.0.0.1:${port}/v1/chat/completions`, {
    method: "POST",
    body: options.body,
    signal: options.signal,
  });

async function fixture(t, turns, { model = {}, host = () => ({}) } = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), "ofox-reset-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const requests = [],
    events = [],
    calls = [];
  const runtime = new AssistantRuntime({
    retrySleep: async () => {},
    fetch: async (_url, options) => {
      requests.push(options.body);
      return (turns[requests.length - 1] ?? reply)(options);
    },
    emit: (event) => events.push(event),
    hostCall: async (c) => {
      calls.push(c);
      return host(c.name, c.args);
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run({
    sessionId: "reset-session",
    runId: "reset-run",
    operationId: "reset-operation",
    sessionFile: path.join(root, "session.jsonl"),
    model: {
      id: "test/reset",
      pricingKnown: true,
      cost: { input: 1, output: 1, cacheRead: 0, cacheWrite: 0 },
      supportedEndpoints: ["openai"],
      contextWindow: 32000,
      maxOutputTokens: 1024,
      ...model,
    },
    apiKey: "reset-test-key-never-persist",
    apex: "ofox.io",
    prompt: "Check the tool",
    localConfig: {
      cwd: root,
      platform: process.platform === "win32" ? "windows" : "macos",
      skillPath: fileURLToPath(
        new URL("../skills/local-config/SKILL.md", import.meta.url),
      ),
      definition: { limits },
    },
  });
  const receipts = result.entries
    .filter((entry) => entry.role === "request")
    .map((entry) => entry.request.status);
  const spent = events
    .filter((event) => event.type === "task_status" && "cost" in event.data)
    .map((event) => event.data.cost);
  return { result, requests, events, calls, receipts, spent };
}

test("a connection reset before the request was written costs nothing and is asked again", async (t) => {
  const port = await resetting(t);
  const f = await fixture(t, [through("https", port)]);
  assert.equal(f.result.status, "completed", f.result.errorMessage);
  assert.equal(f.requests.length, 2);
  assert.deepEqual(f.receipts, ["not_sent", "completed"]);
  assert.equal(f.result.usageCoverage, "complete");
  assert.deepEqual(
    f.events.filter((e) => e.type === "retrying").map((e) => e.data.attempt),
    [1],
  );
});

test("a reset after the request was written is asked again, and what it may have cost counts toward the limit", async (t) => {
  const port = await resetting(t);
  const f = await fixture(
    t,
    [
      () => call("tool_status", { tool: "codex" }),
      through("http", port),
      reply,
    ],
    { host: () => ({ installed: true }) },
  );
  assert.equal(f.result.status, "completed", f.result.errorMessage);
  assert.equal(f.result.errorCode, undefined);
  assert.equal(f.requests.length, 3);
  assert.equal(f.calls.length, 1);
  assert.deepEqual(f.receipts, ["completed", "unknown", "completed"]);
  // The receipts stay honest: one request's cost is not known.
  assert.equal(f.result.usageCoverage, "incomplete");
  // Two answered requests cost 15 tokens each; the unanswered one counts for
  // at least a full-length answer on top of them.
  const known = (2 * 15) / 1e6;
  assert.ok(
    f.spent.at(-1) >= known + 1024 / 1e6,
    `${f.spent.at(-1)} includes the unanswered request`,
  );
});

test("a request that keeps dropping ends as a network failure, not as a cost failure", async (t) => {
  const port = await resetting(t);
  const drop = through("http", port);
  const f = await fixture(t, [drop, drop, drop]);
  assert.equal(f.requests.length, 3);
  assert.equal(f.result.status, "error");
  assert.equal(f.result.errorKind, "network");
  assert.equal(f.result.errorCode, "ECONNRESET");
});

test("what an unanswered request may have cost still stops the task at the user's limit", async (t) => {
  const port = await resetting(t);
  const f = await fixture(
    t,
    [
      through("http", port),
      () => call("tool_status", { tool: "codex" }),
      reply,
    ],
    {
      // A full-length answer alone would cost 1024 x 2000 / 1e6 = US$2.
      model: {
        cost: { input: 2000, output: 2000, cacheRead: 0, cacheWrite: 0 },
      },
    },
  );
  // The dropped request is asked again; nothing further is spent after it.
  assert.equal(f.requests.length, 2);
  assert.equal(f.calls.length, 0);
  assert.equal(f.result.status, "error");
  assert.equal(f.result.errorCode, "assistant_cost_limit");
});

test("an image whose outcome is unknown fails that image only", async () => {
  const { mediaTools } = await import("../src/media-tools.mjs");
  const runtime = {
    hostTool: async () => {
      throw new Error("assistant_media_outcome_unknown: no answer arrived");
    },
    wrapTool: (context, name, description, parameters, execute) => ({
      name,
      execute: (id, args, signal) => execute(args, signal, id),
    }),
  };
  const context = {
    params: {
      media: { image: { model: "test/image", models: ["test/image"] } },
      documents: { presentationPlanning: "v1" },
    },
  };
  const image = mediaTools(runtime, context).find(
    (tool) => tool.name === "generate_image",
  );
  const result = await image.execute("image-1", {
    prompt: "a cover",
    presentationAssetId: "img-cover",
    planId: "plan-1",
    revision: 1,
  });
  assert.equal(result.isError, true);
  // No `unknown_outcome` code: that one ends the whole run.
  assert.doesNotMatch(JSON.stringify(result), /unknown_outcome/);
  assert.match(result.content[0].text, /may have been charged/);
  assert.match(result.content[0].text, /without it/);
  // Outside a planned deck the run still stops for the user to check.
  await assert.rejects(
    image.execute("image-2", { prompt: "a fox" }),
    /assistant_media_outcome_unknown/,
  );
});

test("the shell tool says it waits for approval and names the tools that do not", async (t) => {
  const f = await fixture(t, []);
  const system = JSON.parse(f.requests[0]).messages.find(
    (message) => message.role === "system" || message.role === "developer",
  );
  const text = JSON.stringify(system.content);
  assert.match(text, /approv/i);
  assert.match(text, /ls, find, grep/);
  assert.match(text, /assistant_approval_timeout/);
});
