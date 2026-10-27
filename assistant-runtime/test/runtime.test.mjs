import assert from "node:assert/strict";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AssistantRuntime } from "../src/runtime.mjs";
import { pdfPages } from "./fixtures/pdf.mjs";
import {
  probeGuideline,
  probeToolName,
  probeTools,
} from "./fixtures/probe-tool.mjs";

const model = {
  id: "test/chat",
  supportedEndpoints: ["openai"],
  contextWindow: 32000,
  maxOutputTokens: 4096,
};
const token = "short-lived-credential-never-persist-123456789";
function stream(text, options = {}) {
  const { usage, omitUsage, ...responseOptions } = options;
  const chunks = [
    {
      choices: [
        {
          index: 0,
          delta: { role: "assistant", content: text },
          finish_reason: null,
        },
      ],
    },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage: omitUsage
        ? undefined
        : (usage ?? {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
          }),
    },
  ];
  return new Response(
    chunks
      .map(
        (chunk) =>
          `data: ${JSON.stringify({ id: "test", object: "chat.completion.chunk", created: 1, model: model.id, ...chunk })}\n\n`,
      )
      .join("") + "data: [DONE]\n\n",
    { headers: { "content-type": "text/event-stream" }, ...responseOptions },
  );
}
function toolStream(name, args, second) {
  const calls = [
    {
      index: 0,
      id: "call-one",
      type: "function",
      function: { name, arguments: JSON.stringify(args) },
    },
  ];
  if (second)
    calls.push({
      index: 1,
      id: "call-two",
      type: "function",
      function: { name: second.name, arguments: JSON.stringify(second.args) },
    });
  return new Response(
    `data: ${JSON.stringify({ id: "tool", choices: [{ index: 0, delta: { role: "assistant", tool_calls: calls }, finish_reason: null }] })}\n\ndata: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] })}\n\ndata: [DONE]\n\n`,
    { headers: { "content-type": "text/event-stream" } },
  );
}
function eventsResponse(events) {
  return new Response(
    events
      .map(
        (event) => `event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`,
      )
      .join(""),
    { headers: { "content-type": "text/event-stream" } },
  );
}
async function sandbox(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "ofox-runtime-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return {
    sessionId: "session-1",
    runId: "run-1",
    operationId: "operation-1",
    sessionFile: path.join(directory, "session.jsonl"),
    model,
    apiKey: token,
    apex: "ofox.io",
    prompt: "hello",
  };
}

class ProbeRuntime extends AssistantRuntime {
  tools(context) {
    return probeTools(this, context);
  }
}

const onePixelPng =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

async function writeImageHistory(sessionFile) {
  const at = "2026-10-26T17:05:14.661Z";
  const entries = [
    {
      type: "session",
      version: 3,
      id: "image-session",
      timestamp: at,
      cwd: "",
    },
    {
      type: "custom",
      customType: "ofox-desktop-run",
      data: { runId: "image-run", operationId: "image-operation" },
      id: "img00001",
      parentId: null,
      timestamp: at,
    },
    {
      type: "message",
      id: "img00002",
      parentId: "img00001",
      timestamp: at,
      message: {
        role: "user",
        content: [
          { type: "text", text: "What is in this picture?" },
          { type: "image", data: onePixelPng, mimeType: "image/png" },
        ],
        timestamp: 1,
      },
    },
    {
      type: "message",
      id: "img00003",
      parentId: "img00002",
      timestamp: at,
      message: {
        role: "assistant",
        content: [{ type: "text", text: "A single pixel." }],
        api: "openai-completions",
        provider: "openai",
        model: model.id,
        usage: {
          input: 1,
          output: 1,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: 2,
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
        },
        stopReason: "stop",
        timestamp: 2,
      },
    },
  ];
  await writeFile(
    sessionFile,
    entries.map((entry) => JSON.stringify(entry)).join("\n") + "\n",
  );
  return { assistantId: "img00003" };
}

test("a stop during preparation persists a retryable native turn without a model request", async (t) => {
  const params = await sandbox(t);
  const events = [];
  let requests = 0;
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async () => {
      requests++;
      return stream("Retry worked");
    },
  });
  t.after(() => runtime.close());
  const running = runtime.run(params);
  assert.deepEqual(await runtime.stop(params), { stopped: true });
  const rejected = await running;
  assert.equal(requests, 0);
  assert.equal(rejected.status, "stopped");
  assert.equal(
    rejected.entries.find((entry) => entry.role === "user").content,
    params.prompt,
  );
  const failed = rejected.entries.find((entry) => entry.role === "assistant");
  assert.equal(failed.stopReason, "aborted");
  assert.ok(
    events.some(
      (event) => event.type === "error" && event.data.code === "stopped",
    ),
  );
  assert.equal(events.at(-1).type, "run_end");
  assert.match(await readFile(params.sessionFile, "utf8"), /ofox-desktop-run/);
  assert.deepEqual((await runtime.load(params)).entries, rejected.entries);
  const retried = await runtime.run({
    ...params,
    runId: "retry-run",
    operationId: "retry-operation",
    regenerateEntryId: failed.id,
  });
  assert.equal(retried.status, "completed");
  assert.equal(requests, 1);
  assert.equal(
    retried.entries.find((entry) => entry.id === failed.id).active,
    false,
  );
});

test("regenerating a native image turn needs a vision model before any request", async (t) => {
  const params = await sandbox(t);
  const { assistantId } = await writeImageHistory(params.sessionFile);
  let requests = 0;
  let body;
  const runtime = new AssistantRuntime({
    fetch: async (_url, init) => {
      requests++;
      body = JSON.parse(init.body);
      return stream("Vision regenerate worked");
    },
  });
  t.after(() => runtime.close());
  const rejected = await runtime.run({
    ...params,
    regenerateEntryId: assistantId,
  });
  assert.equal(requests, 0);
  assert.equal(rejected.status, "error");
  assert.match(rejected.errorMessage, /vision/i);
  const retried = await runtime.run({
    ...params,
    runId: "vision-run",
    operationId: "vision-operation",
    model: { ...model, supportsImages: true },
    regenerateEntryId: assistantId,
  });
  assert.equal(retried.status, "completed");
  assert.equal(requests, 1);
  assert.match(JSON.stringify(body.messages), /image_url/);
  assert.equal(
    retried.entries.find((entry) => entry.id === assistantId).active,
    false,
  );
});

test("one uncertain write prevents a second tool from the same SDK batch", async (t) => {
  const params = await sandbox(t);
  const writes = [];
  const runtime = new ProbeRuntime({
    fetch: async () =>
      toolStream(
        probeToolName,
        { command: "first" },
        { name: probeToolName, args: { command: "second" } },
      ),
    hostCall: async (call) => {
      writes.push(call.args.command);
      await new Promise((resolve) => setImmediate(resolve));
      throw Object.assign(new Error("assistant_operation_outcome_unknown"), {
        code: "unknown_outcome",
      });
    },
  });
  await runtime.run(params);
  assert.deepEqual(writes, ["first"]);
  await runtime.close();
});

test("the trusted debug gateway is used by the real SDK instead of the public Ofox endpoint", async (t) => {
  const params = {
    ...(await sandbox(t)),
    baseUrl: "http://localhost:8088",
    allowLocalGateway: true,
  };
  let actual;
  const runtime = new AssistantRuntime({
    fetch: async (url) => {
      actual = String(url);
      return stream("local fixture");
    },
  });
  const result = await runtime.run(params);
  assert.equal(actual, "http://localhost:8088/v1/chat/completions");
  assert.equal(result.status, "completed");
  await runtime.close();
});

test("real Pi SDK streams and reloads native history without persisting credentials or discovering tools", async (t) => {
  const params = await sandbox(t);
  const events = [];
  const requests = [];
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async (input, init) => {
      requests.push({
        url: String(input),
        headers: new Headers(init.headers),
        body: JSON.parse(init.body),
      });
      return stream(`Hello ${token} sk-ant-fakeSecretKey123456789`);
    },
  });
  const result = await runtime.run(params);
  assert.equal(result.status, "completed");
  assert.equal(requests.length, 1);
  assert.equal(requests[0].url, "https://api.ofox.io/v1/chat/completions");
  assert.equal(requests[0].headers.get("authorization"), `Bearer ${token}`);
  assert.equal(requests[0].body.tools, undefined);
  assert.match(requests[0].body.messages[0].content, /Ofox Assistant/);
  assert.match(
    events
      .filter((item) => item.type === "text_delta")
      .map((item) => item.data.delta)
      .join(""),
    /Hello \[REDACTED\]/,
  );
  assert.equal(JSON.stringify(events).includes(token), false);
  const native = await readFile(params.sessionFile, "utf8");
  assert.equal(native.includes(token), false);
  assert.equal(native.includes("fakeSecretKey"), false);
  assert.equal(result.entries.filter((item) => item.active).length, 2);
  assert.deepEqual(
    result.entries.map((item) => item.runId),
    [params.runId, params.runId],
  );
  assert.deepEqual(
    result.entries.map((item) => item.operationId),
    [params.operationId, params.operationId],
  );
  assert.equal(
    result.entries.find((item) => item.role === "assistant").final,
    true,
  );
  const loaded = await new AssistantRuntime().load(params);
  assert.deepEqual(loaded.entries, result.entries);
  await runtime.close();
});

test("native editing and regeneration keep superseded branches and resume after restart", async (t) => {
  const params = await sandbox(t);
  let response = 0;
  const runtime = new AssistantRuntime({
    fetch: async () => stream(`reply ${++response}`),
  });
  const first = await runtime.run(params);
  const initialUser = first.entries.find((item) => item.role === "user");
  const second = await runtime.run({
    ...params,
    runId: "run-2",
    operationId: "operation-2",
    prompt: "edited",
    editEntryId: initialUser.id,
  });
  assert.equal(
    second.entries.filter((item) => item.active && item.role === "user").at(-1)
      .content,
    "edited",
  );
  assert.equal(
    second.entries.find((item) => item.id === initialUser.id).active,
    false,
  );
  const assistant = second.entries.find(
    (item) => item.active && item.role === "assistant",
  );
  const third = await runtime.run({
    ...params,
    runId: "run-3",
    operationId: "operation-3",
    prompt: "edited",
    regenerateEntryId: assistant.id,
  });
  assert.equal(
    third.entries.find((item) => item.id === assistant.id).active,
    false,
  );
  assert.equal(
    third.entries
      .filter((item) => item.active && item.role === "assistant")
      .at(-1).content,
    "reply 3",
  );
  assert.equal(
    third.entries
      .filter((item) => item.active && item.role === "assistant")
      .at(-1).runId,
    "run-3",
  );
  assert.equal(
    third.entries.find((item) => item.id === assistant.id).runId,
    "run-2",
  );
  assert.deepEqual(
    (await new AssistantRuntime().load(params)).entries,
    third.entries,
  );
  await runtime.close();
});

test("provider errors never retry paid requests and stay scrubbed in native history", async (t) => {
  const params = await sandbox(t);
  let requests = 0;
  const runtime = new AssistantRuntime({
    fetch: async () => {
      requests++;
      return Response.json(
        { error: { message: `unavailable ${token}` } },
        { status: 429 },
      );
    },
  });
  const result = await runtime.run(params);
  assert.equal(requests, 1);
  assert.equal(result.status, "error");
  assert.equal(
    (await readFile(params.sessionFile, "utf8")).includes(token),
    false,
  );
  await runtime.close();
});

test("stop aborts the active streamed request and preserves its partial native turn", async (t) => {
  const params = await sandbox(t);
  let started;
  const ready = new Promise((resolve) => {
    started = resolve;
  });
  let requests = 0;
  const runtime = new AssistantRuntime({
    emit: (event) => {
      if (event.type === "text_delta") started();
    },
    fetch: async (_url, init) => {
      requests++;
      return new Response(
        new ReadableStream({
          start(controller) {
            const payload = {
              choices: [
                {
                  index: 0,
                  delta: { role: "assistant", content: "partial ".repeat(30) },
                  finish_reason: null,
                },
              ],
            };
            controller.enqueue(
              new TextEncoder().encode(`data: ${JSON.stringify(payload)}\n\n`),
            );
            init.signal.addEventListener(
              "abort",
              () => controller.error(init.signal.reason),
              { once: true },
            );
          },
        }),
        { headers: { "content-type": "text/event-stream" } },
      );
    },
  });
  const run = runtime.run(params);
  await ready;
  assert.deepEqual(await runtime.stop({ ...params, operationId: "wrong" }), {
    stopped: false,
  });
  assert.deepEqual(await runtime.stop(params), { stopped: true });
  const result = await run;
  assert.equal(result.status, "stopped");
  assert.equal(requests, 1);
  assert.match(
    result.entries.find((item) => item.role === "assistant").content,
    /partial/,
  );
  await runtime.close();
});

test("a stop arriving with run_start cannot start a paid request", async (t) => {
  const params = await sandbox(t);
  let requests = 0;
  let runtime;
  runtime = new AssistantRuntime({
    emit: (event) => {
      if (event.type === "run_start")
        queueMicrotask(() => void runtime.stop(params));
    },
    fetch: async () => {
      requests++;
      return stream("must not send");
    },
  });
  const result = await runtime.run(params);
  assert.equal(result.status, "stopped");
  assert.equal(requests, 0);
  await runtime.close();
});

test("manual compaction persists a native summary and keeps future turns resumable", async (t) => {
  const params = {
    ...(await sandbox(t)),
    model: {
      ...model,
      contextWindow: 1024,
      maxOutputTokens: 128,
      pricingKnown: true,
      cost: { input: 2, output: 10 },
    },
    prompt: "long context ".repeat(500),
  };
  let count = 0;
  const runtime = new AssistantRuntime({
    fetch: async () => stream(`reply or summary ${++count}`),
  });
  await runtime.run(params);
  await runtime.run({ ...params, runId: "second", operationId: "second" });
  const result = await runtime.run(
    { ...params, runId: "compact", operationId: "compact" },
    true,
  );
  assert.equal(result.status, "completed");
  assert.match(
    await readFile(params.sessionFile, "utf8"),
    /"type":"compaction"/,
  );
  const summaryUsage = result.entries.filter((entry) => entry.role === "usage");
  assert.ok(summaryUsage.length > 0);
  assert.ok(
    summaryUsage.every(
      (entry) =>
        entry.runId === "compact" &&
        entry.operationId === "compact" &&
        entry.type === "compaction" &&
        entry.pricingKnown &&
        entry.usageCoverage === "complete",
    ),
  );
  const requests = result.entries.filter(
    (entry) => entry.role === "assistant" || entry.role === "usage",
  );
  assert.equal(
    requests.reduce((sum, entry) => sum + entry.usage.input, 0),
    count * 10,
  );
  assert.equal(
    requests.reduce((sum, entry) => sum + entry.usage.output, 0),
    count * 5,
  );
  assert.deepEqual(
    (await new AssistantRuntime().load(params)).entries,
    result.entries,
  );
  const resumed = await runtime.run({
    ...params,
    runId: "resumed",
    operationId: "resumed",
    prompt: "continue",
  });
  assert.equal(resumed.status, "completed");
  const nativeEntries = (await readFile(params.sessionFile, "utf8"))
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  const withoutOwnUsage = nativeEntries.filter(
    (entry) => entry.customType !== "ofox-desktop-usage",
  );
  await writeFile(
    params.sessionFile,
    withoutOwnUsage.map((entry) => JSON.stringify(entry)).join("\n") + "\n",
  );
  const legacy = await new AssistantRuntime().load(params);
  assert.ok(
    legacy.entries.some(
      (entry) => entry.role === "usage" && !entry.pricingKnown,
    ),
  );
  assert.equal(
    legacy.entries
      .filter((entry) => entry.role === "assistant" || entry.role === "usage")
      .reduce((sum, entry) => sum + entry.usage.input, 0),
    count * 10,
  );
  await runtime.close();
});

test("compaction without provider usage is explicitly incomplete after native reload", async (t) => {
  const params = {
    ...(await sandbox(t)),
    model: { ...model, contextWindow: 1024, maxOutputTokens: 128 },
    prompt: "long context ".repeat(500),
  };
  let compacting = false;
  const runtime = new AssistantRuntime({
    fetch: async () => stream("summary", { omitUsage: compacting }),
  });
  await runtime.run(params);
  await runtime.run({ ...params, runId: "second", operationId: "second" });
  compacting = true;
  const result = await runtime.run(
    { ...params, runId: "compact", operationId: "compact" },
    true,
  );
  assert.equal(result.usageCoverage, "incomplete");
  const summaryUsage = result.entries.filter((entry) => entry.role === "usage");
  assert.ok(summaryUsage.length > 0);
  assert.ok(
    summaryUsage.every(
      (entry) => entry.usageCoverage === "incomplete" && !entry.pricingKnown,
    ),
  );
  const loaded = await new AssistantRuntime().load(params);
  assert.equal(loaded.usageCoverage, "incomplete");
  assert.deepEqual(loaded.entries, result.entries);
  await runtime.close();
});

test("host tool results are scrubbed before SDK persistence and unknown writes terminate", async (t) => {
  const params = await sandbox(t);
  let requests = 0;
  let writes = 0;
  const runtime = new ProbeRuntime({
    fetch: async () => {
      requests++;
      return toolStream(probeToolName, { command: "write" });
    },
    hostCall: async (call) => {
      assert.equal(call.operationId, params.operationId);
      writes++;
      const error = new Error(`request may have applied ${token}`);
      error.code = -32000;
      error.outcomeUnknown = true;
      throw error;
    },
  });
  await runtime.run(params);
  assert.equal(writes, 1);
  assert.equal(requests, 1);
  const native = await readFile(params.sessionFile, "utf8");
  assert.equal(native.includes(token), false);
  assert.match(native, /unknown_outcome/);
  await runtime.close();
});

test("historical pricing availability survives load and missing cache rates stay unknown", async (t) => {
  const initial = await sandbox(t);
  const params = {
    ...initial,
    model: { ...model, pricingKnown: true, cost: { input: 2, output: 10 } },
  };
  let calls = 0;
  const runtime = new AssistantRuntime({
    fetch: async () =>
      stream("answer", {
        usage: {
          prompt_tokens: 10,
          completion_tokens: 5,
          total_tokens: 15,
          prompt_tokens_details: { cached_tokens: calls++ ? 5 : 0 },
        },
      }),
  });
  await runtime.run(params);
  const second = await runtime.run({
    ...params,
    runId: "cached",
    operationId: "cached",
  });
  const assistants = second.entries.filter(
    (entry) => entry.role === "assistant",
  );
  assert.equal(assistants[0].pricingKnown, true);
  assert.equal(assistants[1].pricingKnown, false);
  assert.equal(assistants[1].usage.cacheRead, 5);
  const loaded = await new AssistantRuntime().load(params);
  assert.deepEqual(loaded.entries, second.entries);
  await runtime.close();
});

test("Anthropic and Responses use real SDK transports with the API key only in Authorization", async (t) => {
  const initial = await sandbox(t);
  for (const endpoint of ["/v1/messages", "/v1/responses"]) {
    const params = {
      ...initial,
      model: { ...model, supportedEndpoints: [endpoint] },
      sessionFile: initial.sessionFile.replace(
        "session",
        endpoint.includes("messages") ? "anthropic" : "responses",
      ),
    };
    const requests = [];
    const runtime = new AssistantRuntime({
      fetch: async (input, init) => {
        requests.push({
          url: String(input),
          headers: new Headers(init.headers),
        });
        return endpoint.includes("messages")
          ? eventsResponse([
              {
                type: "message_start",
                message: {
                  id: "msg-1",
                  type: "message",
                  role: "assistant",
                  model: model.id,
                  content: [],
                  usage: { input_tokens: 10, output_tokens: 0 },
                },
              },
              {
                type: "content_block_start",
                index: 0,
                content_block: { type: "text", text: "" },
              },
              {
                type: "content_block_delta",
                index: 0,
                delta: { type: "text_delta", text: "Anthropic reply" },
              },
              { type: "content_block_stop", index: 0 },
              {
                type: "message_delta",
                delta: { stop_reason: "end_turn", stop_sequence: null },
                usage: { output_tokens: 5 },
              },
              { type: "message_stop" },
            ])
          : eventsResponse([
              { type: "response.created", response: { id: "resp-1" } },
              {
                type: "response.output_item.added",
                output_index: 0,
                item: {
                  id: "msg-1",
                  type: "message",
                  role: "assistant",
                  content: [],
                },
              },
              {
                type: "response.output_text.delta",
                output_index: 0,
                delta: "Responses reply",
              },
              {
                type: "response.output_item.done",
                output_index: 0,
                item: {
                  id: "msg-1",
                  type: "message",
                  role: "assistant",
                  content: [
                    {
                      type: "output_text",
                      text: "Responses reply",
                      annotations: [],
                    },
                  ],
                },
              },
              {
                type: "response.completed",
                response: {
                  id: "resp-1",
                  status: "completed",
                  output: [],
                  usage: {
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: 15,
                  },
                },
              },
            ]);
      },
    });
    const result = await runtime.run(params);
    assert.equal(result.status, "completed");
    assert.match(
      result.entries.find((item) => item.role === "assistant").content,
      /reply/,
    );
    assert.equal(requests.length, 1);
    assert.equal(requests[0].headers.get("authorization"), `Bearer ${token}`);
    assert.equal(requests[0].headers.get("x-api-key"), null);
    assert.equal(requests[0].url.includes(token), false);
    await runtime.close();
  }
});

test("a portable native history can continue, edit and regenerate", async (t) => {
  const params = await sandbox(t);
  await copyFile(
    new URL("./fixtures/portable-session.jsonl", import.meta.url),
    params.sessionFile,
  );
  let body;
  const runtime = new AssistantRuntime({
    fetch: async (_url, options) => {
      body = JSON.parse(options.body);
      return stream("continued after restore");
    },
  });
  const loaded = await runtime.load(params);
  const originalUser = loaded.entries.find((entry) => entry.role === "user");
  assert.ok(originalUser);
  const run = {
    ...params,
    runId: "portable-run-2",
    operationId: "portable-op-2",
    prompt: "Continue the restored conversation",
  };
  const continued = await runtime.run(run);
  assert.match(JSON.stringify(body.messages), /Your portable note is ready/);
  assert.match(
    JSON.stringify(body.messages),
    /Continue the restored conversation/,
  );
  assert.equal(continued.status, "completed");
  const edited = await runtime.run({
    ...run,
    runId: "portable-run-3",
    operationId: "portable-op-3",
    prompt: "Edited after restore",
    editEntryId: originalUser.id,
  });
  assert.equal(
    edited.entries.find((entry) => entry.id === originalUser.id).active,
    false,
  );
  const assistant = edited.entries.findLast(
    (entry) => entry.active && entry.role === "assistant",
  );
  const regenerated = await runtime.run({
    ...run,
    runId: "portable-run-4",
    operationId: "portable-op-4",
    prompt: "Edited after restore",
    regenerateEntryId: assistant.id,
  });
  assert.equal(
    regenerated.entries.find((entry) => entry.id === assistant.id).active,
    false,
  );
  assert.equal(regenerated.status, "completed");
  await runtime.close();
});

test("legacy configuration and search parameters cannot add tools or replace the general role", async (t) => {
  const params = {
    ...(await sandbox(t)),
    kind: "configuration",
    permissionMode: "fullAccess",
    webSearch: true,
    memory: "Saved memory must not reach the model",
  };
  let body;
  const runtime = new AssistantRuntime({
    fetch: async (_url, options) => {
      body = JSON.parse(options.body);
      return stream("plain chat only");
    },
    hostCall: async () => assert.fail("Chat must not call the host"),
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.status, "completed");
  assert.equal(body.tools, undefined);
  assert.match(body.messages[0].content, /Ofox Assistant, a general/);
  assert.doesNotMatch(body.messages[0].content, /configuration assistant/);
  assert.doesNotMatch(JSON.stringify(body.messages), /Saved memory/);
});

test("the system prompt describes only the capabilities enabled for the run", async (t) => {
  const prompts = [];
  for (const Runtime of [AssistantRuntime, ProbeRuntime]) {
    const params = await sandbox(t);
    const runtime = new Runtime({
      fetch: async (_url, options) => {
        // Pi appends a <cwd> section after the preamble this runtime owns.
        prompts.push(
          JSON.parse(options.body).messages[0].content.split("\n<cwd>")[0],
        );
        return stream("described");
      },
    });
    t.after(() => runtime.close());
    assert.equal((await runtime.run(params)).status, "completed");
  }
  const [chat, withTool] = prompts;
  assert.match(chat, /^You are Ofox Assistant/);
  assert.doesNotMatch(
    chat,
    /tool|attachment|import|search|web|cite|URL|PDF|page/i,
  );
  assert.match(withTool, /Use only the provided tools\./);
  assert.ok(withTool.includes(probeGuideline));
});

// Imports files into the sandbox the way the host does: one directory per
// conversation, each attachment described by id, name, MIME type and path.
async function withAttachments(params, files) {
  const root = path.join(path.dirname(params.sessionFile), "attachments");
  await mkdir(root, { recursive: true });
  const attachments = [];
  for (const [name, bytes, mimeType] of files) {
    await writeFile(path.join(root, name), bytes);
    attachments.push({
      id: name,
      name,
      mimeType,
      sizeBytes: Buffer.byteLength(bytes),
      path: name,
    });
  }
  return { ...params, attachmentRoot: root, attachments };
}

function lastUserText(body) {
  const content = body.messages
    .filter((item) => item.role === "user")
    .at(-1).content;
  return typeof content === "string"
    ? content
    : content
        .filter((part) => part.type === "text")
        .map((part) => part.text)
        .join("\n");
}

const attachmentEvents = (events) =>
  events
    .filter((event) => event.type === "attachment")
    .map((event) => event.data);

test("read_attachment and its guidance exist only when the conversation has attachments", async (t) => {
  const seen = [];
  for (const supportsImages of [false, true]) {
    const params = await withAttachments(await sandbox(t), [
      ["notes.txt", "kept for later", "text/plain"],
    ]);
    const runtime = new AssistantRuntime({
      fetch: async (_url, options) => {
        seen.push(JSON.parse(options.body));
        return stream("ok");
      },
    });
    t.after(() => runtime.close());
    const result = await runtime.run({
      ...params,
      model: { ...model, supportsImages },
    });
    assert.equal(result.status, "completed");
  }
  const [textOnly, vision] = seen;
  for (const body of seen) {
    assert.deepEqual(
      body.tools.map((tool) => tool.function.name),
      ["read_attachment"],
    );
    assert.match(body.messages[0].content, /read_attachment reads UTF-8 text/);
    assert.doesNotMatch(lastUserText(body), /kept for later/);
  }
  assert.equal(
    textOnly.tools[0].function.parameters.properties.page,
    undefined,
  );
  assert.match(textOnly.messages[0].content, /cannot view images/);
  assert.doesNotMatch(textOnly.messages[0].content, /renders one PDF page/);
  assert.ok(vision.tools[0].function.parameters.properties.page);
  assert.match(vision.messages[0].content, /renders one PDF page/);
  assert.doesNotMatch(vision.messages[0].content, /cannot view images/);
});

test("selected attachments are reported, then inlined in the model request", async (t) => {
  const params = {
    ...(await withAttachments(await sandbox(t), [
      ["notes.txt", "Quarterly revenue grew", "text/plain"],
    ])),
    prompt: "Summarize",
    selectedAttachmentIds: ["notes.txt"],
  };
  const events = [];
  let body;
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async (_url, options) => {
      body = JSON.parse(options.body);
      return stream("Revenue grew");
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.status, "completed");
  assert.deepEqual(attachmentEvents(events), [
    {
      id: "notes.txt",
      status: "ready",
      code: null,
      details: {
        format: "text",
        characters: 22,
        inlineCharacters: 22,
        hasMore: false,
        truncated: false,
      },
    },
  ]);
  const order = events.map((event) => event.type);
  assert.ok(order.indexOf("run_start") < order.indexOf("attachment"));
  assert.ok(order.indexOf("attachment") < order.indexOf("text_delta"));
  assert.match(
    lastUserText(body),
    /^Summarize\n\n\[User attachment: "notes\.txt"; id=notes\.txt\]\nQuarterly revenue grew/,
  );
});

test("a blocked attachment fails the turn before any request and a vision retry rebuilds it", async (t) => {
  const params = {
    ...(await withAttachments(await sandbox(t), [
      ["photo.png", Buffer.from(onePixelPng, "base64"), "image/png"],
      ["broken.txt", Buffer.from([0x61, 0xff]), "text/plain"],
      ["notes.txt", "caption notes", "text/plain"],
    ])),
    prompt: "Describe the photo",
    selectedAttachmentIds: ["photo.png", "broken.txt", "notes.txt"],
  };
  const events = [];
  let requests = 0;
  let body;
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async (_url, init) => {
      requests++;
      body = JSON.parse(init.body);
      return stream("Vision retry worked");
    },
  });
  t.after(() => runtime.close());
  const rejected = await runtime.run(params);
  assert.equal(requests, 0);
  assert.equal(rejected.status, "error");
  assert.match(rejected.errorMessage, /^assistant_vision_model_required: /);
  assert.deepEqual(
    attachmentEvents(events).map(({ id, status, code }) => [id, status, code]),
    [
      ["photo.png", "model_unsupported", "assistant_vision_model_required"],
      ["broken.txt", "unreadable", "assistant_attachment_unreadable"],
      ["notes.txt", "ready", null],
    ],
  );
  assert.equal(
    events.find((event) => event.type === "error").data.code,
    "input_preparation_failed",
  );
  assert.equal(
    rejected.entries.find((entry) => entry.role === "user").content,
    params.prompt,
  );
  const failed = rejected.entries.find((entry) => entry.role === "assistant");
  assert.deepEqual((await runtime.load(params)).entries, rejected.entries);
  events.length = 0;
  const retried = await runtime.run({
    ...params,
    runId: "vision-retry",
    operationId: "vision-operation",
    model: { ...model, supportsImages: true },
    regenerateEntryId: failed.id,
    originalPrompt: params.prompt,
    selectedAttachmentIds: ["photo.png", "notes.txt"],
  });
  assert.equal(retried.status, "completed");
  assert.equal(requests, 1);
  assert.deepEqual(
    attachmentEvents(events).map(({ status }) => status),
    ["ready", "ready"],
  );
  assert.match(JSON.stringify(body.messages), /image_url/);
  assert.match(lastUserText(body), /^Describe the photo[\s\S]*caption notes/);
  assert.equal(
    retried.entries.find((entry) => entry.id === failed.id).active,
    false,
  );
});

test("regenerate rebuilds the original input and attachment selection, not the extracted prompt", async (t) => {
  const params = {
    ...(await withAttachments(await sandbox(t), [
      ["notes.txt", "NOTE-BODY", "text/plain"],
    ])),
    prompt: "Summarize",
    selectedAttachmentIds: ["notes.txt"],
  };
  const bodies = [];
  const events = [];
  let reply = 0;
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async (_url, init) => {
      bodies.push(JSON.parse(init.body));
      return stream(`reply ${++reply}`);
    },
  });
  t.after(() => runtime.close());
  const first = await runtime.run(params);
  const assistantOf = (result) =>
    result.entries.findLast(
      (entry) => entry.active && entry.role === "assistant",
    ).id;
  const occurrences = (text, pattern) => text.split(pattern).length - 1;
  const without = await runtime.run({
    ...params,
    runId: "without",
    operationId: "without",
    regenerateEntryId: assistantOf(first),
    originalPrompt: "Summarize",
    selectedAttachmentIds: [],
  });
  assert.equal(without.status, "completed");
  assert.equal(lastUserText(bodies[1]), "Summarize");
  events.length = 0;
  const rebuilt = await runtime.run({
    ...params,
    runId: "rebuilt",
    operationId: "rebuilt",
    regenerateEntryId: assistantOf(without),
    originalPrompt: "Summarize",
  });
  assert.equal(rebuilt.status, "completed");
  assert.equal(occurrences(lastUserText(bodies[2]), "NOTE-BODY"), 1);
  assert.match(lastUserText(bodies[2]), /^Summarize\n\n\[User attachment/);
  assert.deepEqual(
    attachmentEvents(events).map(({ id }) => id),
    ["notes.txt"],
  );
  // Without originalPrompt the native turn is replayed as stored; the
  // selection cannot add the attachment a second time.
  const replayed = await runtime.run({
    ...params,
    runId: "replayed",
    operationId: "replayed",
    regenerateEntryId: assistantOf(rebuilt),
  });
  assert.equal(replayed.status, "completed");
  assert.equal(occurrences(lastUserText(bodies[3]), "NOTE-BODY"), 1);
});

test("a failed preparation keeps native images so retrying it does not drop them", async (t) => {
  const params = await sandbox(t);
  const { assistantId } = await writeImageHistory(params.sessionFile);
  let body;
  const runtime = new AssistantRuntime({
    fetch: async (_url, init) => {
      body = JSON.parse(init.body);
      return stream("I see one pixel");
    },
  });
  t.after(() => runtime.close());
  const rejected = await runtime.run({
    ...params,
    regenerateEntryId: assistantId,
  });
  assert.equal(rejected.status, "error");
  const failed = rejected.entries.findLast(
    (entry) => entry.active && entry.role === "assistant",
  );
  const native = (await readFile(params.sessionFile, "utf8"))
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  const failedUser = native.find(
    (entry) => entry.id === failed.parentId && entry.type === "message",
  );
  assert.deepEqual(
    failedUser.message.content.map((part) => part.type),
    ["text", "image"],
  );
  const retried = await runtime.run({
    ...params,
    runId: "retry-failed",
    operationId: "retry-failed",
    model: { ...model, supportsImages: true },
    regenerateEntryId: failed.id,
  });
  assert.equal(retried.status, "completed");
  assert.match(JSON.stringify(body.messages.at(-1)), /image_url/);
  assert.equal(lastUserText(body), "What is in this picture?");
});

test("a vision model receives mixed PDF text and the actual image-only page numbers", async (t) => {
  const params = {
    ...(await withAttachments(await sandbox(t), [
      ["mixed.pdf", pdfPages(["Cover only", null, null, null])],
    ])),
    selectedAttachmentIds: ["mixed.pdf"],
    model: { ...model, supportsImages: true },
  };
  let body;
  const runtime = new AssistantRuntime({
    fetch: async (_url, init) => {
      body = JSON.parse(init.body);
      return stream("Read the mixed document");
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.status, "completed");
  const user = body.messages.findLast((message) => message.role === "user");
  const text = lastUserText(body);
  assert.match(text, /Cover only/);
  assert.match(text, /pages 2, 3.*attached as images/i);
  assert.match(text, /remaining.*4.*read_attachment/i);
  assert.equal(
    user.content.filter((part) => part.type === "image_url").length,
    2,
  );
});

test("a text-only model gets a mixed PDF's text with a partial outcome", async (t) => {
  const params = {
    ...(await withAttachments(await sandbox(t), [
      ["mixed.pdf", pdfPages(["Cover only", null])],
    ])),
    selectedAttachmentIds: ["mixed.pdf"],
  };
  const events = [];
  let body;
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async (_url, init) => {
      body = JSON.parse(init.body);
      return stream("The cover says Cover only; page 2 was not read");
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.status, "completed");
  const [outcome] = attachmentEvents(events);
  assert.equal(outcome.status, "partial");
  assert.equal(outcome.code, "assistant_attachment_partial");
  assert.deepEqual(outcome.details.pagesWithoutText, [2]);
  const text = lastUserText(body);
  assert.match(text, /Cover only/);
  assert.match(text, /PDF pages 2 have no extractable text/);
  assert.doesNotMatch(JSON.stringify(body.messages), /image_url/);
});

test("a PDF without text blocks a text-only model, and retries read the kept selection again", async (t) => {
  const params = {
    ...(await withAttachments(await sandbox(t), [
      ["scan.pdf", pdfPages([null])],
    ])),
    prompt: "What does the scan say?",
    selectedAttachmentIds: ["scan.pdf"],
  };
  const events = [];
  let requests = 0;
  let body;
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async (_url, init) => {
      requests++;
      body = JSON.parse(init.body);
      return stream("A red square");
    },
  });
  t.after(() => runtime.close());
  const failedAssistant = (result) =>
    result.entries.findLast(
      (entry) => entry.active && entry.role === "assistant",
    );
  const blocked = await runtime.run(params);
  assert.equal(requests, 0);
  assert.equal(blocked.status, "error");
  assert.match(blocked.errorMessage, /no extractable text.*vision model/);
  assert.match(
    await readFile(params.sessionFile, "utf8"),
    /"customType":"ofox-desktop-attachments"/,
  );
  // A plain retry (no originalPrompt or selection) reads the attachment
  // again instead of dropping it, and fails the same explicit way.
  events.length = 0;
  const again = await runtime.run({
    ...params,
    runId: "retry-text-only",
    operationId: "retry-text-only",
    selectedAttachmentIds: undefined,
    regenerateEntryId: failedAssistant(blocked).id,
  });
  assert.equal(requests, 0);
  assert.equal(again.status, "error");
  assert.deepEqual(
    attachmentEvents(events).map(({ id, code }) => [id, code]),
    [["scan.pdf", "assistant_vision_model_required"]],
  );
  events.length = 0;
  const retried = await runtime.run({
    ...params,
    runId: "retry-vision",
    operationId: "retry-vision",
    model: { ...model, supportsImages: true },
    selectedAttachmentIds: undefined,
    regenerateEntryId: failedAssistant(again).id,
  });
  assert.equal(retried.status, "completed");
  assert.equal(requests, 1);
  assert.deepEqual(
    attachmentEvents(events).map(({ status }) => status),
    ["ready"],
  );
  assert.match(
    lastUserText(body),
    /^What does the scan say\?\n\n\[User attachment/,
  );
  assert.match(JSON.stringify(body.messages), /image_url/);
  // Once sent, the turn replays as stored: nothing is inlined twice.
  events.length = 0;
  const replayed = await runtime.run({
    ...params,
    runId: "replay-sent",
    operationId: "replay-sent",
    model: { ...model, supportsImages: true },
    selectedAttachmentIds: undefined,
    regenerateEntryId: failedAssistant(retried).id,
  });
  assert.equal(replayed.status, "completed");
  assert.deepEqual(attachmentEvents(events), []);
  assert.equal(lastUserText(body).split("[User attachment").length - 1, 1);
});

test("stop interrupts a long PDF during preparation without a request", async (t) => {
  const params = {
    ...(await withAttachments(await sandbox(t), [
      [
        "long.pdf",
        pdfPages(Array.from({ length: 100 }, (_, index) => `Page ${index}`)),
      ],
    ])),
    selectedAttachmentIds: ["long.pdf"],
  };
  const events = [];
  let requests = 0;
  let runtime;
  runtime = new AssistantRuntime({
    emit: (event) => {
      events.push(event);
      if (event.type !== "run_start") return;
      // Stop after some turns of the event loop: only reachable while the
      // PDF is being read because extraction yields between pages.
      let turns = 0;
      const turn = () => {
        if (++turns === 20) void runtime.stop(params);
        else setImmediate(turn);
      };
      setImmediate(turn);
    },
    fetch: async () => {
      requests++;
      return stream("must not send");
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.status, "stopped");
  assert.equal(requests, 0);
  assert.deepEqual(attachmentEvents(events), []);
});

test("reading a mixed PDF through a tool discloses text coverage and missing pages", async (t) => {
  const params = await withAttachments(await sandbox(t), [
    ["mixed.pdf", pdfPages(["Cover only", null])],
  ]);
  let calls = 0;
  let body;
  const runtime = new AssistantRuntime({
    fetch: async (_url, init) => {
      body = JSON.parse(init.body);
      return ++calls === 1
        ? toolStream("read_attachment", { id: "mixed.pdf" })
        : stream("Choose a vision model for page 2");
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.status, "completed");
  const tool = body.messages.find((message) => message.role === "tool");
  assert.match(tool.content, /Cover only/);
  assert.match(tool.content, /pagesWithoutText.*2/);
  assert.match(tool.content, /vision model/i);
});

test("a vision model renders a PDF page through read_attachment", async (t) => {
  const params = {
    ...(await withAttachments(await sandbox(t), [
      ["scan.pdf", pdfPages([null])],
    ])),
    model: { ...model, supportsImages: true },
  };
  let calls = 0;
  let body;
  const runtime = new AssistantRuntime({
    fetch: async (_url, init) => {
      body = JSON.parse(init.body);
      return ++calls === 1
        ? toolStream("read_attachment", { id: "scan.pdf", page: 1 })
        : stream("A red square");
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.status, "completed");
  assert.match(JSON.stringify(body.messages), /data:image\/png;base64,/);
});

test("stopping during attachment preparation cannot start a later paid request", async (t) => {
  const params = {
    ...(await withAttachments(await sandbox(t), [
      ["notes.txt", "text attachment", "text/plain"],
    ])),
    selectedAttachmentIds: ["notes.txt"],
  };
  let requests = 0;
  let runtime;
  runtime = new AssistantRuntime({
    emit: (event) => {
      if (event.type === "run_start")
        queueMicrotask(() => void runtime.stop(params));
    },
    fetch: async () => {
      requests++;
      return stream("must not send");
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.status, "stopped");
  assert.equal(requests, 0);
});

test("a relocated attachment under an absolute host path continues a portable history", async (t) => {
  const params = await sandbox(t);
  await copyFile(
    new URL("./fixtures/portable-session.jsonl", import.meta.url),
    params.sessionFile,
  );
  const attachmentRoot = await realpath(path.dirname(params.sessionFile));
  const attachmentId = "f5835424-a0a7-4f48-b0f0-2c8e4fb03f1b";
  const attachmentPath = path.join(attachmentRoot, attachmentId);
  await writeFile(attachmentPath, "portable attachment content");
  let body;
  const runtime = new AssistantRuntime({
    fetch: async (_url, options) => {
      body = JSON.parse(options.body);
      return stream("continued after restore");
    },
  });
  t.after(() => runtime.close());
  const continued = await runtime.run({
    ...params,
    prompt: "Read the restored attachment",
    attachmentRoot,
    attachments: [
      {
        id: attachmentId,
        path: attachmentPath,
        name: "note.txt",
        mimeType: "text/plain",
        sizeBytes: 27,
      },
    ],
    selectedAttachmentIds: [attachmentId],
  });
  assert.equal(continued.status, "completed");
  assert.match(JSON.stringify(body.messages), /Your portable note is ready/);
  assert.match(JSON.stringify(body.messages), /portable attachment content/);
});

test("malformed attachment parameters are rejected before any request", async (t) => {
  const params = await sandbox(t);
  let requests = 0;
  const runtime = new AssistantRuntime({
    fetch: async () => {
      requests++;
      return stream("must not send");
    },
  });
  t.after(() => runtime.close());
  const attachment = { id: "a", name: "a.txt", path: "a.txt" };
  for (const invalid of [
    { attachments: [attachment] },
    { attachmentRoot: "relative", attachments: [attachment] },
    { attachmentRoot: os.tmpdir(), attachments: "a" },
    {
      attachmentRoot: os.tmpdir(),
      attachments: [attachment],
      selectedAttachmentIds: "a",
    },
  ])
    await assert.rejects(
      () => runtime.run({ ...params, ...invalid }),
      /attachment/i,
    );
  assert.equal(requests, 0);
});
