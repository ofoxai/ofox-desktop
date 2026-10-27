import assert from "node:assert/strict";
import {
  mkdtemp,
  readFile,
  rm,
  writeFile,
  copyFile,
  realpath,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AssistantRuntime } from "../src/runtime.mjs";
import { pdfPages } from "./fixtures/pdf.mjs";

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
    attachmentRoot: directory,
    attachments: [],
    selectedAttachmentIds: [],
    kind: "general",
    model,
    accessToken: token,
    apex: "ofox.io",
    prompt: "hello",
  };
}

test("rejected attachment preparation persists a retryable native turn without a model request", async (t) => {
  const params = await sandbox(t);
  const { createCanvas } = await import("@napi-rs/canvas");
  const canvas = createCanvas(32, 32);
  canvas.getContext("2d").fillRect(0, 0, 32, 32);
  await writeFile(
    path.join(params.attachmentRoot, "picture.png"),
    canvas.toBuffer("image/png"),
  );
  params.attachments = [
    {
      id: "picture",
      path: "picture.png",
      name: "picture.png",
      mimeType: "image/png",
    },
  ];
  params.selectedAttachmentIds = ["picture"];
  let requests = 0;
  let requestBody;
  const runtime = new AssistantRuntime({
    fetch: async (_url, init) => {
      requests++;
      requestBody = JSON.parse(init.body);
      return stream("Vision retry worked");
    },
  });
  t.after(() => runtime.close());
  const rejected = await runtime.run(params);
  assert.equal(requests, 0);
  assert.equal(rejected.status, "error");
  assert.match(rejected.errorMessage, /vision|images/i);
  const failed = rejected.entries.find((entry) => entry.role === "assistant");
  assert.ok(failed?.id);
  assert.match(await readFile(params.sessionFile, "utf8"), /ofox-desktop-run/);
  assert.deepEqual((await runtime.load(params)).entries, rejected.entries);
  const retried = await runtime.run({
    ...params,
    runId: "vision-retry",
    operationId: "vision-operation",
    model: { ...model, supportsImages: true },
    regenerateEntryId: failed.id,
    originalPrompt: params.prompt,
  });
  assert.equal(retried.status, "completed");
  assert.equal(requests, 1);
  assert.match(JSON.stringify(requestBody.messages), /image_url/);
  assert.equal(
    retried.entries.find((entry) => entry.id === failed.id).active,
    false,
  );
});

test("a vision model receives mixed PDF text and the actual image-only page numbers", async (t) => {
  const params = await sandbox(t);
  await writeFile(
    path.join(params.attachmentRoot, "mixed.pdf"),
    pdfPages(["Cover only", null, null, null]),
  );
  params.attachments = [{ id: "mixed", path: "mixed.pdf", name: "mixed.pdf" }];
  params.selectedAttachmentIds = ["mixed"];
  params.model = { ...model, supportsImages: true };
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
  const user = body.messages.find((message) => message.role === "user");
  const text = user.content
    .filter((part) => part.type === "text")
    .map((part) => part.text)
    .join("\n");
  assert.match(text, /Cover only/);
  assert.match(text, /pages 2, 3.*attached as images/i);
  assert.match(text, /remaining.*4.*read_attachment/i);
  assert.equal(
    user.content.filter((part) => part.type === "image_url").length,
    2,
  );
});

test("a text-only model cannot silently summarize a mixed PDF as completely read", async (t) => {
  const params = await sandbox(t);
  await writeFile(
    path.join(params.attachmentRoot, "mixed.pdf"),
    pdfPages(["Cover only", null]),
  );
  params.attachments = [{ id: "mixed", path: "mixed.pdf", name: "mixed.pdf" }];
  params.selectedAttachmentIds = ["mixed"];
  let requests = 0;
  const runtime = new AssistantRuntime({
    fetch: async () => {
      requests++;
      return stream("Cover is not the whole document");
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(requests, 0);
  assert.equal(result.status, "error");
  assert.match(result.errorMessage, /pages 2.*vision model/i);
  const failed = result.entries.find((entry) => entry.role === "assistant");
  assert.deepEqual((await runtime.load(params)).entries, result.entries);
  const retried = await runtime.run({
    ...params,
    runId: "mixed-vision-retry",
    operationId: "mixed-vision-operation",
    model: { ...model, supportsImages: true },
    regenerateEntryId: failed.id,
    originalPrompt: params.prompt,
  });
  assert.equal(retried.status, "completed");
  assert.equal(requests, 1);
});

test("reading a mixed PDF through a tool discloses text coverage and missing pages", async (t) => {
  const params = await sandbox(t);
  await writeFile(
    path.join(params.attachmentRoot, "mixed.pdf"),
    pdfPages(["Cover only", null]),
  );
  params.attachments = [{ id: "mixed", path: "mixed.pdf", name: "mixed.pdf" }];
  let calls = 0;
  let body;
  const runtime = new AssistantRuntime({
    fetch: async (_url, init) => {
      body = JSON.parse(init.body);
      return ++calls === 1
        ? toolStream("read_attachment", { id: "mixed" })
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

test("one uncertain write prevents a second tool from the same SDK batch", async (t) => {
  const params = { ...(await sandbox(t)), kind: "configuration" };
  const writes = [];
  const runtime = new AssistantRuntime({
    fetch: async () =>
      toolStream(
        "run_command",
        { command: "first" },
        { name: "run_command", args: { command: "second" } },
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

test("oversized active memory fails before a provider request rather than silently truncating", async (t) => {
  const params = {
    ...(await sandbox(t)),
    memory: "保留的主动记忆".repeat(10000),
  };
  let requests = 0;
  const runtime = new AssistantRuntime({
    fetch: async () => {
      requests++;
      return stream("not sent");
    },
  });
  const rejected = await runtime.run(params);
  assert.equal(rejected.status, "error");
  assert.match(rejected.errorMessage, /memory.*budget/i);
  assert.ok(
    (await runtime.load(params)).entries.find((entry) => entry.role === "user"),
  );
  assert.equal(requests, 0);
  const retried = await runtime.run({
    ...params,
    runId: "short-memory",
    operationId: "short-memory-op",
    memory: "Use concise answers",
  });
  assert.equal(retried.status, "completed");
  assert.equal(requests, 1);
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
  assert.deepEqual(
    requests[0].body.tools.map((item) => item.function.name),
    ["read_attachment"],
  );
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

test("stopping during attachment preparation cannot start a later paid request", async (t) => {
  const params = await sandbox(t);
  await writeFile(
    path.join(params.attachmentRoot, "notes.txt"),
    "text attachment",
  );
  params.attachments = [{ id: "notes", path: "notes.txt", name: "notes.txt" }];
  params.selectedAttachmentIds = ["notes"];
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
  await runtime
    .run(params)
    .catch((error) => assert.match(error.message, /abort|stop/i));
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

test("configuration tool results are scrubbed before SDK persistence and unknown writes terminate", async (t) => {
  const params = { ...(await sandbox(t)), kind: "configuration" };
  let requests = 0;
  let writes = 0;
  const runtime = new AssistantRuntime({
    fetch: async () => {
      requests++;
      return toolStream("bind_tool", { tool: "codex" });
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

test("Anthropic and Responses use real SDK transports with OAuth only in Authorization", async (t) => {
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

test("a portable native history and relocated attachment can continue, edit and regenerate", async (t) => {
  const params = await sandbox(t);
  await copyFile(
    new URL("./fixtures/portable-session.jsonl", import.meta.url),
    params.sessionFile,
  );
  params.attachmentRoot = await realpath(params.attachmentRoot);
  const attachmentId = "f5835424-a0a7-4f48-b0f0-2c8e4fb03f1b";
  const attachmentPath = path.join(params.attachmentRoot, attachmentId);
  await writeFile(attachmentPath, "portable attachment content");
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
    prompt: "Read the restored attachment",
    attachments: [
      {
        id: attachmentId,
        path: attachmentPath,
        name: "note.txt",
        mimeType: "text/plain",
      },
    ],
    selectedAttachmentIds: [attachmentId],
  };
  const continued = await runtime.run(run);
  assert.match(JSON.stringify(body.messages), /Your portable note is ready/);
  assert.match(JSON.stringify(body.messages), /portable attachment content/);
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

test("read-only configuration never advertises diagnostic or mutation tools", async (t) => {
  const params = {
    ...(await sandbox(t)),
    kind: "configuration",
    permissionMode: "readOnly",
  };
  let names;
  const runtime = new AssistantRuntime({
    fetch: async (_url, options) => {
      names = JSON.parse(options.body).tools.map((tool) => tool.function.name);
      return stream("static diagnosis only");
    },
  });
  await runtime.run(params);
  assert.ok(names.includes("config_status"));
  for (const name of [
    "diagnose_tool",
    "run_command",
    "bind_tool",
    "set_tool_model",
    "install_tool",
    "update_tool",
  ])
    assert.ok(!names.includes(name), name);
  await runtime.close();
});
