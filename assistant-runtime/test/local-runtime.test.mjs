import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
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
let id = 0;
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
                id: `local-${++id}`,
                type: "function",
                function: { name, arguments: JSON.stringify(args) },
              },
            ],
          },
        },
      ],
    },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
      usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
    },
  ]);
const reply = (
  usage = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
) =>
  sse([
    {
      choices: [
        { index: 0, delta: { role: "assistant", content: "Finished." } },
      ],
    },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage,
    },
  ]);
async function fixture(t, turns, host, extra = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), "ofox-local-runtime-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const requests = [],
    events = [],
    calls = [];
  const runtime = new AssistantRuntime({
    fetch: async (url, options) => {
      requests.push(JSON.parse(options.body));
      return turns[requests.length - 1]?.() ?? reply();
    },
    emit: (e) => events.push(e),
    hostCall: async (c) => {
      calls.push(c);
      return host(c.name, c.args);
    },
  });
  t.after(() => runtime.close());
  const params = {
    sessionId: "local-session",
    runId: "local-run",
    operationId: "local-operation",
    sessionFile: path.join(root, "session.jsonl"),
    model: {
      id: "test/local",
      pricingKnown: true,
      cost: { input: 1, output: 1, cacheRead: 0, cacheWrite: 0 },
      supportedEndpoints: ["openai"],
      contextWindow: 32000,
      maxOutputTokens: 1024,
    },
    apiKey: "local-test-key-never-persist",
    apex: "ofox.io",
    prompt: "Fix the tool",
    localConfig: {
      cwd: root,
      platform: process.platform === "win32" ? "windows" : "macos",
      skillPath: fileURLToPath(
        new URL("../skills/local-config/SKILL.md", import.meta.url),
      ),
      definition: { limits },
    },
    ...extra,
  };
  return { requests, events, calls, run: () => runtime.run(params) };
}
test("real Pi run stops the fifth repeated operation and emits warning before it", async (t) => {
  const f = await fixture(
    t,
    Array.from(
      { length: 7 },
      () => () => call("tool_status", { tool: "codex" }),
    ),
    () => ({ installed: true }),
  );
  const result = await f.run();
  assert.equal(f.calls.length, 4);
  assert.equal(result.status, "error");
  assert.equal(result.errorCode, "assistant_repeat_limit");
  assert.ok(
    f.events.some(
      (e) => e.type === "task_status" && e.data.warning === "repeat_warning",
    ),
  );
});
test("last model step has no tools and cannot execute a provider-invented call", async (t) => {
  const f = await fixture(
    t,
    [
      () => call("tool_status", { tool: "codex" }),
      () => call("tool_status", { tool: "claude" }),
      () => call("repair_launcher", { tool: "codex" }),
    ],
    () => ({ installed: true }),
  );
  // Inject a small budget through the trusted capability definition.
  const run = f.run; // fixture uses a shared definition intentionally only in this test.
  const old = limits.maxToolCalls;
  limits.maxToolCalls = 3;
  try {
    await run();
    assert.equal(f.calls.length, 2);
    assert.ok(!f.requests[2].tools?.length);
  } finally {
    limits.maxToolCalls = old;
  }
});
for (const apex of ["ofox.ai", "ofox.io"])
  test(`repair completion requires host verification in the diagnosed folder (${apex})`, async (t) => {
    const folder = "/problem";
    const f = await fixture(
      t,
      [
        () => call("diagnose_in_folder", { tool: "codex", folder }),
        () => call("rebind_tool", { tool: "codex" }),
        reply,
        () => call("verify_in_folder", { tool: "codex", folder }),
        reply,
      ],
      (name, args) =>
        name === "rebind_tool"
          ? { changed: true }
          : { folder: args.folder, verified: name === "verify_in_folder" },
      { apex },
    );
    const result = await f.run();
    assert.equal(result.status, "completed");
    assert.equal(f.requests.length, 5);
    assert.equal(result.entries.filter((e) => e.role === "user").length, 1);
    assert.ok(f.calls.some((c) => c.name === "verify_in_folder"));
  });
test("two unsuccessful completion continuations remain unverified, regardless of reply text", async (t) => {
  const f = await fixture(
    t,
    [
      () => call("diagnose_in_folder", { tool: "codex", folder: "/problem" }),
      () => call("write", { path: "/configuration", content: "safe" }),
      reply,
      reply,
      reply,
    ],
    (name) =>
      name === "write" ? { changed: true } : { verified: false, exists: true },
  );
  const result = await f.run();
  assert.equal(f.requests.length, 5);
  assert.equal(result.errorCode, "assistant_verification_required");
  assert.equal(result.entries.filter((e) => e.role === "user").length, 1);
});

test("unknown pricing blocks before the first provider request with a stable host error", async (t) => {
  const f = await fixture(t, [], () => ({}), {
    model: {
      id: "test/local",
      supportedEndpoints: ["openai"],
      contextWindow: 32000,
      maxOutputTokens: 1024,
      pricingKnown: false,
    },
  });
  const result = await f.run();
  assert.equal(f.requests.length, 0);
  assert.equal(result.errorCode, "assistant_cost_unavailable");
  assert.equal(result.errorKind, "not_sent");
});
test("paid usage stops additional model requests and preserves the completed request ledger", async (t) => {
  const f = await fixture(
    t,
    [() => call("tool_status", { tool: "codex" })],
    () => ({}),
    {
      model: {
        id: "test/local",
        supportedEndpoints: ["openai"],
        contextWindow: 32000,
        maxOutputTokens: 1024,
        pricingKnown: true,
        cost: { input: 100000, output: 100000, cacheRead: 0, cacheWrite: 0 },
      },
    },
  );
  const result = await f.run();
  assert.equal(f.requests.length, 1);
  assert.equal(result.errorCode, "assistant_cost_limit");
  assert.ok(result.entries.some((e) => e.role === "request"));
});

test("a partially failed repair still requires verification and never completes from prose alone", async (t) => {
  const f = await fixture(
    t,
    [
      () => call("diagnose_in_folder", { tool: "codex", folder: "/problem" }),
      () => call("rebind_tool", { tool: "codex" }),
      reply,
      reply,
      reply,
    ],
    (name, args) => {
      if (name === "rebind_tool")
        throw new Error("assistant_local_partial_change");
      return { folder: args.folder, verified: false };
    },
  );
  const result = await f.run();
  assert.equal(result.errorCode, "assistant_verification_required");
  assert.equal(f.requests.length, 5);
});

for (const [name, usage] of [
  [
    "unknown cache pricing",
    {
      prompt_tokens: 10,
      completion_tokens: 5,
      total_tokens: 15,
      prompt_tokens_details: { cached_tokens: 8 },
    },
  ],
  ["missing token usage", undefined],
])
  test(`${name} stops the local task without duplicating its request receipt`, async (t) => {
    const f = await fixture(
      t,
      [() => reply(usage === undefined ? {} : usage)],
      () => ({}),
      {
        model: {
          id: "test/local",
          supportedEndpoints: ["openai"],
          contextWindow: 32000,
          maxOutputTokens: 1024,
          pricingKnown: true,
          cost: { input: 1, output: 1 },
        },
      },
    );
    const result = await f.run();
    assert.equal(result.errorCode, "assistant_cost_unavailable");
    assert.equal(f.requests.length, 1);
    assert.equal(result.entries.filter((e) => e.role === "request").length, 1);
  });
