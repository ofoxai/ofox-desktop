// A reply that spends its whole output on reasoning says and does nothing.
// The run must not end "completed" with nothing delivered: the model is asked
// again, and a run that never gets an answer ends as an error.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AssistantRuntime } from "../src/runtime.mjs";

const sse = (chunks) =>
  new Response(
    chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`).join("") +
      "data: [DONE]\n\n",
    { headers: { "content-type": "text/event-stream" } },
  );
const usage = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 };
const reasoningOnly = () =>
  sse([
    {
      choices: [
        {
          index: 0,
          delta: { role: "assistant", reasoning_content: "Still thinking…" },
        },
      ],
    },
    { choices: [{ index: 0, delta: {}, finish_reason: "length" }], usage },
  ]);
const answer = () =>
  sse([
    {
      choices: [{ index: 0, delta: { role: "assistant", content: "Done." } }],
    },
    { choices: [{ index: 0, delta: {}, finish_reason: "stop" }], usage },
  ]);

async function run(t, turns) {
  const root = await mkdtemp(path.join(os.tmpdir(), "ofox-output-limit-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const requests = [];
  const runtime = new AssistantRuntime({
    retrySleep: async () => {},
    fetch: async (_url, options) => {
      requests.push(JSON.parse(options.body));
      return (turns[requests.length - 1] ?? answer)();
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run({
    sessionId: "limit-session",
    runId: "limit-run",
    operationId: "limit-operation",
    sessionFile: path.join(root, "session.jsonl"),
    model: {
      id: "test/limit",
      supportedEndpoints: ["openai"],
      contextWindow: 32000,
      maxOutputTokens: 1024,
      reasoning: true,
    },
    apiKey: "limit-test-key-never-persist",
    apex: "ofox.io",
    prompt: "Make the deck",
  });
  return { result, requests };
}

test("a reply cut off while still reasoning is asked again and then answers", async (t) => {
  const { result, requests } = await run(t, [reasoningOnly, answer]);
  assert.equal(requests.length, 2);
  assert.equal(result.status, "completed");
  assert.equal(result.entries.at(-1).content, "Done.");
  // The second request tells the model what happened, without showing the
  // user a message they did not write.
  assert.match(JSON.stringify(requests[1].messages), /output limit/);
  assert.equal(result.entries.filter((e) => e.role === "user").length, 1);
});

test("a reply that keeps running out of output ends as an error, not as completed", async (t) => {
  const { result, requests } = await run(t, [
    reasoningOnly,
    reasoningOnly,
    reasoningOnly,
    reasoningOnly,
  ]);
  assert.equal(requests.length, 3, "asked again twice, no more");
  assert.equal(result.status, "error");
  assert.match(result.errorMessage, /output limit/);
});

test("a long answer cut off at the limit is kept as it is", async (t) => {
  const cut = () =>
    sse([
      {
        choices: [
          { index: 0, delta: { role: "assistant", content: "A long ans" } },
        ],
      },
      { choices: [{ index: 0, delta: {}, finish_reason: "length" }], usage },
    ]);
  const { result, requests } = await run(t, [cut]);
  assert.equal(requests.length, 1);
  assert.equal(result.status, "completed");
});
