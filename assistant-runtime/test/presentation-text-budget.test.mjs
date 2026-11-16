import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AssistantRuntime } from "../src/runtime.mjs";

const limits = {
  repeatWarning: 3,
  repeatStop: 5,
  maxDurationSeconds: 900,
  maxCostUsd: 1,
  maxToolCalls: 20,
  completionRetries: 2,
};

async function runCostOnly(
  t,
  threshold,
  { omitUsage = false, noMonitor = false } = {},
) {
  const root = await mkdtemp(path.join(os.tmpdir(), "ofox-text-budget-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const requests = [];
  let hostCalls = 0;
  const runtime = new AssistantRuntime({
    retrySleep: async () => {},
    hostCall: async () => {
      hostCalls++;
      throw new Error("No local or MCP operation is authorized");
    },
    fetch: async (_url, options) => {
      requests.push(JSON.parse(options.body));
      const first = requests.length === 1;
      const chunks = [
        {
          choices: [
            {
              index: 0,
              delta: first
                ? {
                    role: "assistant",
                    tool_calls: [
                      {
                        index: 0,
                        id: "style-query",
                        type: "function",
                        function: {
                          name: "get_presentation_style",
                          arguments: '{"id":"cartesian"}',
                        },
                      },
                    ],
                  }
                : { role: "assistant", content: "Finished." },
            },
          ],
        },
        {
          choices: [
            {
              index: 0,
              delta: {},
              finish_reason: first ? "tool_calls" : "stop",
            },
          ],
          ...(omitUsage
            ? {}
            : {
                usage: {
                  prompt_tokens: 10,
                  completion_tokens: 5,
                  total_tokens: 15,
                },
              }),
        },
      ];
      return new Response(
        chunks.map((chunk) => `data: ${JSON.stringify(chunk)}\n\n`).join("") +
          "data: [DONE]\n\n",
        { headers: { "content-type": "text/event-stream" } },
      );
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run({
    sessionId: "cost-only-session",
    runId: "cost-only-run",
    operationId: "cost-only-operation",
    sessionFile: path.join(root, "session.jsonl"),
    model: {
      id: "test/text-budget",
      tools: true,
      pricingKnown: true,
      cost: { input: 10000, output: 10000, cacheRead: 0, cacheWrite: 0 },
      supportedEndpoints: ["openai"],
      contextWindow: 32000,
      maxOutputTokens: 1024,
    },
    apiKey: "synthetic-text-budget-key",
    apex: "ofox.io",
    prompt: "Query the presentation recipe, then finish.",
    capabilities: [],
    documents: { formats: ["pptx"] },
    ...(noMonitor
      ? {}
      : {
          mcp: {
            enabled: false,
            servers: {},
            limits: { ...limits, maxCostUsd: threshold },
          },
        }),
  });
  return { result, requests, hostCalls };
}

test("cost-only limits pause later text requests at the selected threshold without enabling local or MCP tools", async (t) => {
  const low = await runCostOnly(t, 0.1);
  assert.equal(low.requests.length, 1);
  assert.equal(low.result.errorCode, "assistant_cost_limit");
  assert.equal(low.hostCalls, 0);
  const names = low.requests[0].tools.map((tool) => tool.function.name);
  assert.ok(names.includes("get_presentation_style"));
  for (const name of [
    "read",
    "write",
    "edit",
    "bash",
    "run_command",
    "tool_search",
    "tool_status",
  ])
    assert.ok(!names.includes(name), name + " was not authorized");
  assert.equal(limits.maxCostUsd, 1, "the product default is unchanged");
  const next = await runCostOnly(t, 1);
  assert.equal(next.requests.length, 2);
  assert.equal(next.result.status, "completed");
  assert.equal(next.hostCalls, 0);
  const baseline = await runCostOnly(t, 1, { noMonitor: true });
  assert.deepEqual(
    [...names].sort(),
    baseline.requests[0].tools.map((tool) => tool.function.name).sort(),
    "cost-only limits expose exactly the original tool set",
  );
});

test("cost-only monitoring counts missing usage at its ceiling, never as zero", async (t) => {
  // At these rates one unreported request alone passes the US$0.10 threshold.
  const unknown = await runCostOnly(t, 0.1, { omitUsage: true });
  assert.equal(unknown.requests.length, 1);
  assert.equal(unknown.result.errorCode, "assistant_cost_limit");
  assert.equal(unknown.hostCalls, 0);
  // Under a threshold it does not reach, the run goes on and finishes.
  const roomy = await runCostOnly(t, 100000, { omitUsage: true });
  assert.equal(roomy.requests.length, 2);
  assert.equal(roomy.result.status, "completed");
});
