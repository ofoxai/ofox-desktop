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
  // A key-shaped string the run was not given is the conversation's own text.
  assert.equal(native.includes("sk-ant-fakeSecretKey123456789"), true);
  assert.equal(
    result.entries.filter((item) => item.active && item.role !== "request")
      .length,
    2,
  );
  assert.deepEqual(
    result.entries
      .filter((item) => item.role !== "request")
      .map((item) => item.runId),
    [params.runId, params.runId],
  );
  assert.deepEqual(
    result.entries
      .filter((item) => item.role !== "request")
      .map((item) => item.operationId),
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

// Everyday code and configuration that looks like credentials.
const lookalikes = [
  'OpenAI(api_key=os.environ["OPENAI_API_KEY"])',
  'password = input("Password: ")',
  "interface User { password: string; secret: boolean }",
  "Authorization: Bearer YOUR_TOKEN",
  "max_tokens: 4096",
  "sk-proj-EXAMPLEexample0123456789",
].join("\n");

function nativeTexts(native) {
  return native
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line))
    .filter(
      (entry) =>
        entry.type === "message" &&
        ["user", "assistant"].includes(entry.message.role),
    )
    .map((entry) =>
      typeof entry.message.content === "string"
        ? entry.message.content
        : entry.message.content
            .filter((part) => part.type === "text")
            .map((part) => part.text)
            .join(""),
    );
}

test("conversation text passes byte for byte; only the run's own key is replaced", async (t) => {
  const params = {
    ...(await sandbox(t)),
    prompt: `${lookalikes}\nmy key ${token}`,
  };
  const events = [];
  const bodies = [];
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async (_input, init) => {
      bodies.push(JSON.parse(init.body));
      return stream(`${lookalikes}\necho ${token}`);
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.status, "completed");
  const question = `${lookalikes}\nmy key [REDACTED]`;
  const answer = `${lookalikes}\necho [REDACTED]`;
  const sent = bodies[0].messages.at(-1).content;
  assert.equal(
    typeof sent === "string" ? sent : sent.map((part) => part.text).join(""),
    question,
  );
  assert.equal(
    events
      .filter((event) => event.type === "text_delta")
      .map((event) => event.data.delta)
      .join(""),
    answer,
  );
  assert.equal(
    events.find((event) => event.type === "message_end").data.content,
    answer,
  );
  assert.deepEqual(
    result.entries
      .filter((entry) => !["usage", "request"].includes(entry.role))
      .map((entry) => entry.content),
    [question, answer],
  );
  const native = await readFile(params.sessionFile, "utf8");
  assert.deepEqual(nativeTexts(native), [question, answer]);
  assert.equal(native.includes(token), false);
  assert.equal(JSON.stringify(events).includes(token), false);
  assert.deepEqual(
    (await new AssistantRuntime().load(params)).entries,
    result.entries,
  );
});

test("a key split across streamed deltas never reaches an event", async (t) => {
  const params = await sandbox(t);
  const events = [];
  const half = Math.floor(token.length / 2);
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async () =>
      new Response(
        [`before ${token.slice(0, half)}`, `${token.slice(half)} after`]
          .map(
            (content) =>
              `data: ${JSON.stringify({ id: "split", choices: [{ index: 0, delta: { content }, finish_reason: null }] })}\n\n`,
          )
          .join("") +
          `data: ${JSON.stringify({ id: "split", choices: [{ index: 0, delta: {}, finish_reason: "stop" }], usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 } })}\n\ndata: [DONE]\n\n`,
        { headers: { "content-type": "text/event-stream" } },
      ),
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.status, "completed");
  const deltas = events
    .filter((event) => event.type === "text_delta")
    .map((event) => event.data.delta);
  assert.equal(deltas.join(""), "before [REDACTED] after");
  assert.equal(
    deltas.some((delta) => delta.includes(token.slice(0, half))),
    false,
  );
});

test("an earlier key of the run handed over as knownSecrets is replaced too", async (t) => {
  const refused = "refused-credential-of-this-run-0123456789";
  const params = { ...(await sandbox(t)), knownSecrets: [refused] };
  const events = [];
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async () => stream(`old ${refused} new ${token}`),
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  assert.equal(result.entries.at(-1).content, "old [REDACTED] new [REDACTED]");
  const text = JSON.stringify(events);
  assert.equal(text.includes(refused) || text.includes(token), false);
  assert.equal(
    (await readFile(params.sessionFile, "utf8")).includes(refused),
    false,
  );
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

const userTurns = (projection) =>
  projection.entries.filter((entry) => entry.role === "user");
const activeTexts = (projection) =>
  projection.entries
    .filter(
      (entry) => entry.active && ["user", "assistant"].includes(entry.role),
    )
    .map((entry) => entry.content);
const nativeLines = async (sessionFile) =>
  (await readFile(sessionFile, "utf8"))
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));

test("a user turn names the message it follows, the same for each of its versions", async (t) => {
  const params = await sandbox(t);
  let response = 0;
  const runtime = new AssistantRuntime({
    fetch: async () => stream(`reply ${++response}`),
  });
  const first = await runtime.run({ ...params, prompt: "first" });
  const firstReply = first.entries.find((entry) => entry.role === "assistant");
  assert.equal(userTurns(first)[0].branchFrom, null);
  const second = await runtime.run({
    ...params,
    runId: "run-2",
    operationId: "operation-2",
    prompt: "second",
  });
  const question = userTurns(second).at(-1);
  // Its run marker sits between them: the nearest message is the reply.
  assert.equal(question.branchFrom, firstReply.id);
  const edited = await runtime.run({
    ...params,
    runId: "run-3",
    operationId: "operation-3",
    prompt: "second, edited",
    editEntryId: question.id,
  });
  const reply = edited.entries.findLast(
    (entry) => entry.active && entry.role === "assistant",
  );
  const regenerated = await runtime.run({
    ...params,
    runId: "run-4",
    operationId: "operation-4",
    regenerateEntryId: reply.id,
  });
  assert.deepEqual(
    userTurns(regenerated).map((entry) => [entry.content, entry.branchFrom]),
    [
      ["first", null],
      ["second", firstReply.id],
      ["second, edited", firstReply.id],
      ["second, edited", firstReply.id],
    ],
  );
  assert.ok(
    regenerated.entries
      .filter((entry) => entry.role !== "user")
      .every((entry) => !("branchFrom" in entry)),
  );
  // An edit of the first turn starts the conversation again.
  const restarted = await runtime.run({
    ...params,
    runId: "run-5",
    operationId: "operation-5",
    prompt: "first, edited",
    editEntryId: userTurns(first)[0].id,
  });
  assert.equal(userTurns(restarted).at(-1).branchFrom, null);
  assert.deepEqual(
    (await new AssistantRuntime().load(params)).entries,
    restarted.entries,
  );
  await runtime.close();
});

test("navigate shows the latest of another version and keeps it after reopening", async (t) => {
  const params = await sandbox(t);
  const bodies = [];
  let response = 0;
  const runtime = new AssistantRuntime({
    fetch: async (_url, init) => {
      bodies.push(JSON.parse(init.body));
      return stream(`reply ${++response}`);
    },
  });
  const first = await runtime.run({ ...params, prompt: "question A" });
  const original = userTurns(first)[0];
  const followed = await runtime.run({
    ...params,
    runId: "run-2",
    operationId: "operation-2",
    prompt: "follow-up",
  });
  await runtime.run({
    ...params,
    runId: "run-3",
    operationId: "operation-3",
    regenerateEntryId: followed.entries.findLast(
      (entry) => entry.role === "assistant",
    ).id,
  });
  const edited = await runtime.run({
    ...params,
    runId: "run-4",
    operationId: "operation-4",
    prompt: "question B",
    editEntryId: original.id,
  });
  assert.deepEqual(activeTexts(edited), ["question B", "reply 4"]);

  const shown = await runtime.navigate({
    sessionFile: params.sessionFile,
    entryId: original.id,
  });
  // The newest leaf under that turn: its regenerated follow-up.
  assert.deepEqual(activeTexts(shown), [
    "question A",
    "reply 1",
    "follow-up",
    "reply 3",
  ]);
  for (const text of ["question B", "reply 4", "reply 2"])
    assert.equal(
      shown.entries.find((entry) => entry.content === text).active,
      false,
      text,
    );
  // Kept by the file's last line, so a reopened history shows it too.
  const last = (await nativeLines(params.sessionFile)).at(-1);
  assert.equal(last.type, "custom");
  assert.equal(last.customType, "ofox-desktop-leaf");
  assert.deepEqual(
    (await new AssistantRuntime().load(params)).entries,
    shown.entries,
  );

  // The next turn continues the version shown.
  const next = await runtime.run({
    ...params,
    runId: "run-5",
    operationId: "operation-5",
    prompt: "and then?",
  });
  const context = JSON.stringify(bodies.at(-1).messages);
  for (const text of ["question A", "reply 1", "follow-up", "reply 3"])
    assert.match(context, new RegExp(text));
  assert.doesNotMatch(context, /question B|reply 2|reply 4/);
  assert.equal(
    userTurns(next).at(-1).branchFrom,
    shown.entries.find((entry) => entry.content === "reply 3").id,
  );

  // And back to the edited version.
  const back = await runtime.navigate({
    sessionFile: params.sessionFile,
    entryId: userTurns(edited).at(-1).id,
  });
  assert.deepEqual(activeTexts(back), ["question B", "reply 4"]);
  await runtime.close();
});

test("navigate refuses anything but a message of a saved history and changes nothing", async (t) => {
  const params = await sandbox(t);
  const runtime = new AssistantRuntime({ fetch: async () => stream("reply") });
  await assert.rejects(
    runtime.navigate({ sessionFile: params.sessionFile, entryId: "anything" }),
  );
  const done = await runtime.run(params);
  const user = userTurns(done)[0];
  const before = await readFile(params.sessionFile, "utf8");
  const marker = (await nativeLines(params.sessionFile)).find(
    (line) => line.customType === "ofox-desktop-run",
  );
  for (const entryId of ["missing", "", undefined, marker.id])
    await assert.rejects(
      runtime.navigate({ sessionFile: params.sessionFile, entryId }),
      String(entryId),
    );
  for (const sessionFile of ["session.jsonl", `${params.sessionFile}.txt`])
    await assert.rejects(runtime.navigate({ sessionFile, entryId: user.id }));
  assert.equal(await readFile(params.sessionFile, "utf8"), before);
  await runtime.close();
});

test("rate limit retries are bounded and stay scrubbed in native history", async (t) => {
  const params = await sandbox(t);
  let requests = 0;
  const runtime = new AssistantRuntime({
    retrySleep: async () => {},
    fetch: async () => {
      requests++;
      return Response.json(
        { error: { message: `unavailable ${token}` } },
        { status: 429 },
      );
    },
  });
  const result = await runtime.run(params);
  assert.equal(requests, 3);
  assert.equal(result.status, "error");
  assert.equal(
    (await readFile(params.sessionFile, "utf8")).includes(token),
    false,
  );
  await runtime.close();
});

function networkError(cause) {
  return new TypeError("fetch failed", { cause });
}
function systemError(code, syscall) {
  return Object.assign(new Error(`${syscall} ${code}`), { code, syscall });
}

test("a request refused with a 4xx or never sent reports why and costs a known zero", async (t) => {
  for (const [name, reply, expected] of [
    [
      "balance",
      () =>
        Response.json(
          {
            error: {
              message: "Insufficient credits. Current balance: $0.",
              type: "insufficient_credits",
              code: 402,
            },
          },
          { status: 402 },
        ),
      { errorStatus: 402 },
    ],
    [
      "unknown model",
      () =>
        Response.json(
          { error: { message: "model not found" } },
          { status: 404 },
        ),
      { errorStatus: 404 },
    ],
    [
      "connection refused on every address",
      () => {
        throw networkError(
          new AggregateError([
            systemError("ECONNREFUSED", "connect"),
            systemError("ECONNREFUSED", "connect"),
          ]),
        );
      },
      { errorKind: "not_sent", errorCode: "ECONNREFUSED" },
    ],
    [
      "unknown host",
      () => {
        throw networkError(systemError("ENOTFOUND", "getaddrinfo"));
      },
      { errorKind: "not_sent", errorCode: "ENOTFOUND" },
    ],
    [
      "unreachable network",
      () => {
        throw networkError(systemError("ENETUNREACH", "connect"));
      },
      { errorKind: "not_sent", errorCode: "ENETUNREACH" },
    ],
    [
      "connect timeout",
      () => {
        throw networkError(
          Object.assign(new Error("Connect Timeout Error"), {
            code: "UND_ERR_CONNECT_TIMEOUT",
          }),
        );
      },
      { errorKind: "not_sent", errorCode: "UND_ERR_CONNECT_TIMEOUT" },
    ],
    [
      "untrusted certificate",
      () => {
        throw networkError(
          Object.assign(new Error("certificate has expired"), {
            code: "CERT_HAS_EXPIRED",
          }),
        );
      },
      { errorKind: "not_sent", errorCode: "CERT_HAS_EXPIRED" },
    ],
    [
      "proxy refused the tunnel",
      () => {
        throw networkError(
          Object.assign(
            new Error("Proxy response (407) !== 200 when HTTP Tunneling"),
            { code: "UND_ERR_ABORTED" },
          ),
        );
      },
      { errorKind: "not_sent", errorCode: "UND_ERR_ABORTED" },
    ],
    [
      "proxy closed while opening the tunnel",
      () => {
        throw networkError(
          Object.assign(new Error("Proxy Connection failed"), {
            code: "UND_ERR_PRX_CONN",
            cause: Object.assign(new Error("other side closed"), {
              code: "UND_ERR_SOCKET",
            }),
          }),
        );
      },
      { errorKind: "not_sent", errorCode: "UND_ERR_PRX_CONN" },
    ],
    [
      "proxy's own TLS failed",
      () => {
        throw networkError(
          Object.assign(new Error("Secure Proxy Connection failed"), {
            code: "UND_ERR_PRX_TLS",
            cause: Object.assign(new Error("socket hang up"), {
              code: "ECONNRESET",
            }),
          }),
        );
      },
      { errorKind: "not_sent", errorCode: "UND_ERR_PRX_TLS" },
    ],
    [
      "SOCKS5 proxy refused the target",
      () => {
        throw networkError(
          Object.assign(
            new Error("SOCKS5 connection failed: Connection refused"),
            { code: "UND_ERR_SOCKS5_REPLY_5" },
          ),
        );
      },
      { errorKind: "not_sent", errorCode: "UND_ERR_SOCKS5_REPLY_5" },
    ],
  ]) {
    const params = await sandbox(t);
    const runtime = new AssistantRuntime({
      retrySleep: async () => {},
      fetch: async () => reply(),
    });
    const result = await runtime.run(params);
    assert.equal(result.status, "error", name);
    const failed = result.entries.at(-1);
    for (const [key, value] of Object.entries({
      errorStatus: undefined,
      errorKind: undefined,
      errorCode: undefined,
      ...expected,
    })) {
      assert.equal(result[key], value, `${name}: result ${key}`);
      assert.equal(failed[key], value, `${name}: entry ${key}`);
    }
    assert.equal(failed.usageCoverage, "complete", name);
    assert.equal(result.usageCoverage, "complete", name);
    // The reason survives a reload of the native history.
    assert.deepEqual(
      (await new AssistantRuntime().load(params)).entries,
      result.entries,
      name,
    );
    await runtime.close();
  }
});

test("a sent request that got no answer may have been billed, so its cost stays unknown", async (t) => {
  for (const [name, error, errorCode] of [
    [
      "closed after the request",
      networkError(
        Object.assign(new Error("other side closed"), {
          code: "UND_ERR_SOCKET",
        }),
      ),
      "UND_ERR_SOCKET",
    ],
    [
      "reset after the write",
      networkError(systemError("ECONNRESET", "read")),
      "ECONNRESET",
    ],
    [
      "unreachable while reading",
      networkError(systemError("EHOSTUNREACH", "read")),
      "EHOSTUNREACH",
    ],
    [
      "aborted for an unknown reason",
      networkError(
        Object.assign(new Error("Request aborted"), {
          code: "UND_ERR_ABORTED",
        }),
      ),
      "UND_ERR_ABORTED",
    ],
    [
      "timed out before the headers",
      Object.assign(new Error("The operation was aborted due to timeout"), {
        name: "TimeoutError",
      }),
      undefined,
    ],
  ]) {
    const params = await sandbox(t);
    const runtime = new AssistantRuntime({
      fetch: async () => {
        throw error;
      },
    });
    const result = await runtime.run(params);
    assert.equal(result.status, "error", name);
    assert.equal(result.errorKind, "network", name);
    assert.equal(result.errorCode, errorCode, name);
    assert.equal(result.entries.at(-1).usageCoverage, "incomplete", name);
    await runtime.close();
  }
});

test("a server error may have generated output, so its cost stays unknown", async (t) => {
  const params = await sandbox(t);
  const runtime = new AssistantRuntime({
    fetch: async () =>
      Response.json({ error: { message: "upstream failed" } }, { status: 500 }),
  });
  const result = await runtime.run(params);
  assert.equal(result.status, "error");
  assert.equal(result.errorStatus, 500);
  assert.equal(result.entries.at(-1).usageCoverage, "incomplete");
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

test("manual compaction tells the host when it starts and that it succeeded", async (t) => {
  const params = {
    ...(await sandbox(t)),
    model: { ...model, contextWindow: 1024, maxOutputTokens: 128 },
    prompt: "long context ".repeat(500),
  };
  const events = [];
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async () => stream("reply or summary"),
  });
  t.after(() => runtime.close());
  await runtime.run(params);
  await runtime.run({ ...params, runId: "second", operationId: "second" });
  events.length = 0;
  const result = await runtime.run(
    { ...params, runId: "compact", operationId: "compact" },
    true,
  );
  assert.equal(result.status, "completed");
  const compaction = events.filter((event) =>
    event.type.startsWith("compaction_"),
  );
  assert.deepEqual(
    compaction.map(({ type, runId, data }) => ({ type, runId, data })),
    [
      { type: "compaction_start", runId: "compact", data: {} },
      {
        type: "compaction_end",
        runId: "compact",
        data: { success: true, willRetry: false },
      },
    ],
  );
});

test("a compaction the provider rejects is reported to the host as unsuccessful", async (t) => {
  const params = {
    ...(await sandbox(t)),
    model: { ...model, contextWindow: 1024, maxOutputTokens: 128 },
    prompt: "long context ".repeat(500),
  };
  const events = [];
  let compacting = false;
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async () =>
      compacting
        ? new Response(JSON.stringify({ error: { message: "unavailable" } }), {
            status: 400,
            headers: { "content-type": "application/json" },
          })
        : stream("reply"),
  });
  t.after(() => runtime.close());
  await runtime.run(params);
  await runtime.run({ ...params, runId: "second", operationId: "second" });
  events.length = 0;
  compacting = true;
  await assert.rejects(
    runtime.run({ ...params, runId: "compact", operationId: "compact" }, true),
    /summarization failed/,
  );
  const ends = events.filter((event) => event.type === "compaction_end");
  assert.equal(ends.length, 1);
  assert.deepEqual(ends[0].data, { success: false, willRetry: false });
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

test("legacy configuration parameters cannot add tools or replace the general role", async (t) => {
  const params = {
    ...(await sandbox(t)),
    kind: "configuration",
    permissionMode: "fullAccess",
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

/** The preamble this runtime owns of the system prompt one run sends. */
async function systemPreamble(t, extra) {
  let preamble;
  const runtime = new AssistantRuntime({
    fetch: async (_url, options) => {
      preamble = JSON.parse(options.body)
        .messages[0].content.split("\n<cwd>")[0]
        .trimEnd();
      return stream("noted");
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run({ ...(await sandbox(t)), ...extra });
  assert.equal(result.status, "completed");
  return preamble;
}

test("every run knows a mermaid block is drawn as a diagram", async (t) => {
  const plain = await systemPreamble(t, {});
  assert.match(plain, /mermaid code block is drawn as a diagram/);
  assert.match(plain, /flowchart/);
});

test("custom instructions follow the role as standing instructions, without the run's key", async (t) => {
  const plain = await systemPreamble(t, {});
  // Empty or blank instructions are none.
  assert.equal(await systemPreamble(t, { instructions: "" }), plain);
  assert.equal(await systemPreamble(t, { instructions: " \n\t" }), plain);
  const instructed = await systemPreamble(t, {
    instructions: `Answer in haiku.\nSign as ${token}`,
    knownSecrets: [],
  });
  assert.ok(instructed.startsWith(`${plain}\n`), instructed);
  const added = instructed.slice(plain.length);
  assert.match(added, /standing instructions/);
  assert.ok(
    added.endsWith("Answer in haiku.\nSign as [REDACTED]"),
    JSON.stringify(added),
  );
  assert.ok(!instructed.includes(token));
});

test("custom instructions reach the model without being stored in native history", async (t) => {
  for (const apex of ["ofox.ai", "ofox.io"]) {
    const params = { ...(await sandbox(t)), apex };
    const instructions = "PRIVATE_STANDING_INSTRUCTION_SENTINEL";
    const prompts = [];
    const events = [];
    const options = {
      emit: (event) => events.push(event),
      fetch: async (url, options) => {
        assert.equal(new URL(url).hostname, `api.${apex}`);
        prompts.push(JSON.parse(options.body).messages[0].content);
        return stream("noted");
      },
    };
    const runtime = new AssistantRuntime(options);
    t.after(() => runtime.close());
    assert.equal(
      (await runtime.run({ ...params, instructions })).status,
      "completed",
    );
    assert.ok(prompts[0].includes(instructions));
    assert.ok(
      !(await readFile(params.sessionFile, "utf8")).includes(instructions),
    );
    assert.ok(!JSON.stringify(events).includes(instructions));
    await runtime.close();
    // Restart with cleared preferences: old system entries cannot revive them.
    const reopened = new AssistantRuntime(options);
    t.after(() => reopened.close());
    assert.equal(
      (
        await reopened.run({
          ...params,
          runId: "next-run",
          operationId: "next-operation",
          instructions: "",
        })
      ).status,
      "completed",
    );
    assert.ok(!JSON.stringify(prompts[1]).includes(instructions));
  }
});

test("custom instructions preserve literal replacement syntax", async (t) => {
  const instructions = "Keep $& and $$ literally.";
  assert.ok((await systemPreamble(t, { instructions })).endsWith(instructions));
});

test("instructions that are not text or over 4,000 characters are refused", async (t) => {
  // 4,000 characters, not UTF-16 units: an emoji is one character.
  assert.match(
    await systemPreamble(t, { instructions: "🦊".repeat(4000) }),
    /🦊$/,
  );
  for (const instructions of [42, ["a"], "a".repeat(4001)]) {
    const runtime = new AssistantRuntime({
      fetch: async () => assert.fail("No request for refused instructions"),
    });
    t.after(() => runtime.close());
    await assert.rejects(
      runtime.run({ ...(await sandbox(t)), instructions }),
      /Invalid instructions/,
    );
  }
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

// ── thinking levels ──────────────────────────────────────────────────────

// Pi's data for gpt-6-luna and claude-haiku-5-5, as the host's table has it.
const lunaLevels = {
  high: "high",
  low: "low",
  max: "max",
  medium: "medium",
  minimal: null,
  off: "none",
  xhigh: "xhigh",
};
const haikuLevels = {
  high: "high",
  low: "low",
  max: "max",
  medium: "medium",
  minimal: null,
  xhigh: "xhigh",
};

/** The models of default-requests.json, as the host sends them at the default. */
async function thinkingModels() {
  const claude = JSON.parse(
    await readFile(
      new URL("./fixtures/catalog-claude-model.json", import.meta.url),
      "utf8",
    ),
  ).runtime;
  const luna = { ...model, id: "openai/gpt-6-luna" };
  return {
    completions: {
      ...luna,
      supportedEndpoints: ["/v1/chat/completions", "/v1/responses"],
    },
    responses: { ...luna, supportedEndpoints: ["/v1/responses"] },
    anthropic: claude,
  };
}

/** What the host adds to a model for a run at a level (catalog.rs). */
function atLevel(name, input) {
  return name === "anthropic"
    ? {
        ...input,
        reasoning: true,
        thinkingLevelMap: haikuLevels,
        anthropicCompat: {
          forceAdaptiveThinking: true,
          supportsTemperature: false,
        },
      }
    : { ...input, reasoning: true, thinkingLevelMap: lunaLevels };
}

/** A reply in `name`'s protocol: some reasoning, then the text. */
function thinkingReply(name) {
  if (name === "completions")
    return new Response(
      [
        {
          choices: [
            {
              index: 0,
              delta: { role: "assistant", reasoning_content: "Weighing it" },
              finish_reason: null,
            },
          ],
        },
        {
          choices: [
            { index: 0, delta: { content: "Answer" }, finish_reason: null },
          ],
        },
        {
          choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
          usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
        },
      ]
        .map((chunk) => `data: ${JSON.stringify({ id: "t", ...chunk })}\n\n`)
        .join("") + "data: [DONE]\n\n",
      { headers: { "content-type": "text/event-stream" } },
    );
  if (name === "responses")
    return eventsResponse([
      { type: "response.created", response: { id: "r" } },
      {
        type: "response.output_item.added",
        output_index: 0,
        item: { id: "rs", type: "reasoning", summary: [] },
      },
      {
        type: "response.reasoning_summary_part.added",
        output_index: 0,
        summary_index: 0,
        part: { type: "summary_text", text: "" },
      },
      {
        type: "response.reasoning_summary_text.delta",
        output_index: 0,
        summary_index: 0,
        delta: "Weighing it",
      },
      {
        type: "response.output_item.done",
        output_index: 0,
        item: {
          id: "rs",
          type: "reasoning",
          summary: [{ type: "summary_text", text: "Weighing it" }],
        },
      },
      {
        type: "response.output_item.added",
        output_index: 1,
        item: { id: "m", type: "message", role: "assistant", content: [] },
      },
      { type: "response.output_text.delta", output_index: 1, delta: "Answer" },
      {
        type: "response.output_item.done",
        output_index: 1,
        item: {
          id: "m",
          type: "message",
          role: "assistant",
          content: [{ type: "output_text", text: "Answer", annotations: [] }],
        },
      },
      {
        type: "response.completed",
        response: {
          id: "r",
          status: "completed",
          output: [],
          usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
        },
      },
    ]);
  return eventsResponse([
    {
      type: "message_start",
      message: {
        id: "m",
        type: "message",
        role: "assistant",
        model: "anthropic/claude-haiku-5.5",
        content: [],
        usage: { input_tokens: 1, output_tokens: 0 },
      },
    },
    {
      type: "content_block_start",
      index: 0,
      content_block: { type: "thinking", thinking: "" },
    },
    {
      type: "content_block_delta",
      index: 0,
      delta: { type: "thinking_delta", thinking: "Weighing it" },
    },
    {
      type: "content_block_delta",
      index: 0,
      delta: { type: "signature_delta", signature: "sig" },
    },
    { type: "content_block_stop", index: 0 },
    {
      type: "content_block_start",
      index: 1,
      content_block: { type: "text", text: "" },
    },
    {
      type: "content_block_delta",
      index: 1,
      delta: { type: "text_delta", text: "Answer" },
    },
    { type: "content_block_stop", index: 1 },
    {
      type: "message_delta",
      delta: { stop_reason: "end_turn", stop_sequence: null },
      usage: { output_tokens: 1 },
    },
    { type: "message_stop" },
  ]);
}

/** One run of `input`; its raw request body and events. */
async function thinkingRun(t, name, input, extra = {}) {
  const params = await sandbox(t);
  const events = [];
  const bodies = [];
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async (_url, init) => {
      bodies.push(init.body);
      return thinkingReply(name);
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run({ ...params, model: input, ...extra });
  assert.equal(result.status, "completed", name);
  assert.equal(bodies.length, 1, name);
  // Pi uses forward slashes in the prompt even on Windows.
  const dir = JSON.stringify(
    path.dirname(params.sessionFile).replaceAll("\\", "/"),
  ).slice(1, -1);
  return {
    raw: bodies[0]
      .split(dir)
      .join("<dir>")
      .replace(
        /Current date: [^\\]+App language: zh\./,
        "Current date: <date> (<weekday>). Time zone: <zone>. App language: zh.",
      )
      .replace(/"prompt_cache_key":"[^"]+"/, '"prompt_cache_key":"<session>"'),
    body: JSON.parse(bodies[0]),
    events,
    result,
  };
}

test("a run without a thinking level sends exactly the request it sent before levels existed", async (t) => {
  const pinned = JSON.parse(
    await readFile(
      new URL("./fixtures/default-requests.json", import.meta.url),
      "utf8",
    ),
  );
  for (const [name, input] of Object.entries(await thinkingModels())) {
    const plain = await thinkingRun(t, name, input);
    assert.equal(plain.raw, pinned[name], name);
    assert.deepEqual(
      plain.events.find((event) => event.type === "run_start").data,
      { thinkingLevel: null },
    );
    // The catalog's thinking fields alone change nothing either.
    assert.equal(
      (await thinkingRun(t, name, atLevel(name, input))).raw,
      pinned[name],
      name,
    );
  }
});

test("empty custom instructions leave each provider's default request exactly as it was", async (t) => {
  const pinned = JSON.parse(
    await readFile(
      new URL("./fixtures/default-requests.json", import.meta.url),
      "utf8",
    ),
  );
  for (const [name, input] of Object.entries(await thinkingModels()))
    assert.equal(
      (await thinkingRun(t, name, input, { instructions: "" })).raw,
      pinned[name],
      name,
    );
});

test("a thinking level reaches each provider in its own form and its reasoning streams", async (t) => {
  const models = await thinkingModels();
  const level = async (name, thinkingLevel) => {
    const run = await thinkingRun(t, name, atLevel(name, models[name]), {
      thinkingLevel,
    });
    assert.deepEqual(
      run.events.find((event) => event.type === "run_start").data,
      { thinkingLevel },
      name,
    );
    assert.equal(
      run.events
        .filter((event) => event.type === "thinking_delta")
        .map((event) => event.data.delta)
        .join(""),
      "Weighing it",
      name,
    );
    const reply = run.result.entries.find((item) => item.role === "assistant");
    assert.equal(reply.content, "Answer", name);
    assert.equal(reply.thinking, "Weighing it", name);
    return run.body;
  };
  assert.equal((await level("completions", "high")).reasoning_effort, "high");
  // gpt-6-luna turns reasoning off with "none".
  assert.equal((await level("completions", "off")).reasoning_effort, "none");
  const responses = await level("responses", "low");
  assert.deepEqual(responses.reasoning, { effort: "low", summary: "auto" });
  assert.deepEqual((await level("responses", "off")).reasoning, {
    effort: "none",
  });
  const claude = await level("anthropic", "high");
  assert.equal(claude.thinking.type, "adaptive");
  assert.deepEqual(claude.output_config, { effort: "high" });
  assert.equal(claude.temperature, undefined);
  assert.deepEqual((await level("anthropic", "off")).thinking, {
    type: "disabled",
  });
});

test("an unknown thinking level is refused before any request", async (t) => {
  const params = await sandbox(t);
  let requests = 0;
  const runtime = new AssistantRuntime({
    fetch: async () => {
      requests++;
      return stream("must not send");
    },
  });
  t.after(() => runtime.close());
  await assert.rejects(
    () =>
      runtime.run({
        ...params,
        model: { ...model, reasoning: true },
        thinkingLevel: "extreme",
      }),
    /thinking level/i,
  );
  assert.equal(requests, 0);
});

test("reasoning end is signalled when the text begins, before its held-back text", async (t) => {
  const params = await sandbox(t);
  const events = [];
  const encoder = new TextEncoder();
  const chunk = (delta, finish = null) =>
    encoder.encode(
      `data: ${JSON.stringify({ id: "t", choices: [{ index: 0, delta, finish_reason: finish }], ...(finish ? { usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 } } : {}) })}\n\n`,
    );
  const runtime = new AssistantRuntime({
    emit: (event) => events.push({ ...event, at: Date.now() }),
    fetch: async () =>
      new Response(
        new ReadableStream({
          async start(controller) {
            controller.enqueue(
              chunk({ role: "assistant", reasoning_content: "Short thought" }),
            );
            await new Promise((resolve) => setTimeout(resolve, 50));
            // Shorter than the tail the runtime holds back for key scrubbing:
            // it is forwarded only when the text ends.
            controller.enqueue(chunk({ content: "Answer" }));
            await new Promise((resolve) => setTimeout(resolve, 300));
            controller.enqueue(chunk({}, "stop"));
            controller.enqueue(encoder.encode("data: [DONE]\n\n"));
            controller.close();
          },
        }),
        { headers: { "content-type": "text/event-stream" } },
      ),
  });
  t.after(() => runtime.close());
  const result = await runtime.run({
    ...params,
    model: { ...model, reasoning: true },
    thinkingLevel: "high",
  });
  assert.equal(result.status, "completed");
  const types = events.map((event) => event.type);
  const end = types.indexOf("thinking_end");
  const text = types.indexOf("text_delta");
  assert.ok(end !== -1, types.join(","));
  assert.equal(types.filter((type) => type === "thinking_end").length, 1);
  assert.deepEqual(events[end].data, {});
  // After the last of the reasoning, before the reply's first words.
  assert.ok(end > types.lastIndexOf("thinking_delta"), types.join(","));
  assert.ok(end < text, types.join(","));
  assert.ok(events[text].at - events[end].at >= 250);
  // No reasoning, no end.
  events.length = 0;
  const plain = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async () => stream("No thinking"),
  });
  t.after(() => plain.close());
  await plain.run({ ...(await sandbox(t)) });
  assert.equal(
    events.some((event) => event.type === "thinking_end"),
    false,
  );
});

test("a model that takes no tools gets none, even with files and web search", async (t) => {
  const params = await withAttachments(await sandbox(t), [
    ["notes.txt", "kept for later", "text/plain"],
  ]);
  let body;
  const runtime = new AssistantRuntime({
    fetch: async (_url, init) => {
      body = JSON.parse(init.body);
      return stream("ok");
    },
  });
  t.after(() => runtime.close());
  const result = await runtime.run({
    ...params,
    model: { ...model, tools: false },
    webSearch: true,
    allowedWebUrls: [],
  });
  assert.equal(result.status, "completed");
  assert.equal(body.tools, undefined);
  assert.doesNotMatch(body.messages[0].content, /read_attachment|web_search/);
});

test("reasoning start is signalled at once, before its held-back text", async (t) => {
  const params = await sandbox(t);
  const events = [];
  const encoder = new TextEncoder();
  const chunk = (delta, finish = null) =>
    encoder.encode(
      `data: ${JSON.stringify({ id: "t", choices: [{ index: 0, delta, finish_reason: finish }], ...(finish ? { usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 } } : {}) })}\n\n`,
    );
  const runtime = new AssistantRuntime({
    emit: (event) => events.push({ ...event, at: Date.now() }),
    fetch: async () =>
      new Response(
        new ReadableStream({
          async start(controller) {
            // Shorter than the tail the runtime holds back for key scrubbing.
            controller.enqueue(
              chunk({ role: "assistant", reasoning_content: "Short thought" }),
            );
            await new Promise((resolve) => setTimeout(resolve, 300));
            controller.enqueue(chunk({ content: "Answer" }));
            controller.enqueue(chunk({}, "stop"));
            controller.enqueue(encoder.encode("data: [DONE]\n\n"));
            controller.close();
          },
        }),
        { headers: { "content-type": "text/event-stream" } },
      ),
  });
  t.after(() => runtime.close());
  const result = await runtime.run({
    ...params,
    model: { ...model, reasoning: true },
    thinkingLevel: "high",
  });
  assert.equal(result.status, "completed");
  const types = events.map((event) => event.type);
  const start = types.indexOf("thinking_start");
  const delta = types.indexOf("thinking_delta");
  assert.ok(start !== -1 && start < delta, types.join(","));
  assert.equal(types.filter((type) => type === "thinking_start").length, 1);
  assert.deepEqual(events[start].data, {});
  // The start came with the first reasoning, not with its held-back text.
  assert.ok(events[delta].at - events[start].at >= 250);
  // No reasoning, no start.
  events.length = 0;
  const plain = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async () => stream("No thinking"),
  });
  t.after(() => plain.close());
  await plain.run({ ...(await sandbox(t)) });
  assert.equal(
    events.some((event) => event.type === "thinking_start"),
    false,
  );
});

test("live input prioritizes steering over follow-up and fences authority without replay", async (t) => {
  const params = await sandbox(t);
  const events = [];
  const requests = [];
  let release;
  const blocked = new Promise((resolve) => {
    release = resolve;
  });
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async (_url, input) => {
      requests.push(JSON.parse(input.body));
      if (requests.length === 1) await blocked;
      return stream(`answer ${requests.length}`);
    },
  });
  t.after(() => {
    release();
    return runtime.close();
  });
  let early;
  const running = runtime.run(params).then((result) => {
    early = result;
    return result;
  });
  const deadline = Date.now() + 10000;
  while (!requests.length && !early && Date.now() < deadline)
    await new Promise((resolve) => setTimeout(resolve, 5));
  assert.ok(requests.length, JSON.stringify(early));
  const input = (id, behavior, text) => ({
    sessionId: params.sessionId,
    runId: params.runId,
    operationId: params.operationId,
    id,
    behavior,
    text,
  });
  await assert.rejects(
    runtime.steer({
      ...input("wrong", "steer", "never sent"),
      operationId: "stale",
    }),
    /active run/,
  );
  await runtime.steer(input("follow", "followUp", "later follow-up"));
  await runtime.steer(input("steer", "steer", `urgent ${token}`));
  // A repeated IPC action must not enqueue or bill the input twice.
  await runtime.steer(input("steer", "steer", `urgent ${token}`));
  release();
  const result = await running;
  assert.equal(requests.length, 3);
  const users = result.entries.filter((entry) => entry.role === "user");
  assert.equal(users.length, 3);
  assert.equal(users[0].content, "hello");
  assert.match(users[1].content, /urgent/);
  assert.ok(!users[1].content.includes(token));
  assert.equal(users[2].content, "later follow-up");
  assert.deepEqual(
    users.map((entry) => entry.runId),
    [params.runId, "steer", "follow"],
  );
  assert.deepEqual(
    events
      .filter((event) => event.type === "steer_applied")
      .map((event) => event.data.id),
    ["steer", "follow"],
  );
  assert.ok(
    events
      .filter((event) => event.type === "steer_applied")
      .every((event) => event.data.previousEntryId),
  );
  assert.ok(!JSON.stringify(result).includes(token));
});

test("fork copies only the selected native context and continues independently", async (t) => {
  const params = await sandbox(t);
  const bodies = [];
  const runtime = new AssistantRuntime({
    fetch: async (_url, input) => {
      bodies.push(JSON.parse(input.body));
      return stream(`Answer ${bodies.length}`);
    },
  });
  t.after(() => runtime.close());
  const first = await runtime.run({
    ...params,
    prompt: "Original first question",
  });
  const entry = first.entries.find(
    (item) => item.role === "assistant" && item.active,
  ).id;
  await runtime.run({
    ...params,
    runId: "later-run",
    operationId: "later-operation",
    prompt: "Later source-only question",
  });
  const before = await readFile(params.sessionFile, "utf8");
  const targetFile = path.join(
    path.dirname(params.sessionFile),
    "fork",
    "session.jsonl",
  );
  const fork = await runtime.fork({
    sessionFile: params.sessionFile,
    entryId: entry,
    targetFile,
  });
  assert.deepEqual(
    fork.entries
      .filter((item) => ["user", "assistant"].includes(item.role))
      .map((item) => item.content),
    ["Original first question", "Answer 1"],
  );
  assert.ok(fork.entries.every((item) => item.active));
  assert.equal(await readFile(params.sessionFile, "utf8"), before);
  await runtime.run({
    ...params,
    sessionId: "fork-session",
    sessionFile: targetFile,
    runId: "fork-run",
    operationId: "fork-operation",
    prompt: "Continue fork",
  });
  const context = JSON.stringify(bodies.at(-1).messages);
  assert.match(context, /Original first question/);
  assert.match(context, /Answer 1/);
  assert.match(context, /Continue fork/);
  assert.doesNotMatch(context, /Later source-only question|Answer 2/);
  assert.equal(await readFile(params.sessionFile, "utf8"), before);
});

test("fork preserves a user-only native branch and refuses existing targets", async (t) => {
  const params = await sandbox(t);
  const runtime = new AssistantRuntime({
    fetch: async () => stream("Source answer"),
  });
  t.after(() => runtime.close());
  const source = await runtime.run(params);
  const user = source.entries.find((entry) => entry.role === "user");
  const targetFile = path.join(
    path.dirname(params.sessionFile),
    "fork-user",
    "session.jsonl",
  );
  const child = await runtime.fork({
    sessionFile: params.sessionFile,
    entryId: user.id,
    targetFile,
  });
  assert.deepEqual(
    child.entries
      .filter((entry) => ["user", "assistant"].includes(entry.role))
      .map((entry) => entry.content),
    ["hello"],
  );
  assert.deepEqual(
    (await runtime.load({ sessionFile: targetFile })).entries,
    child.entries,
  );
  const saved = await readFile(targetFile, "utf8");
  await assert.rejects(
    runtime.fork({
      sessionFile: params.sessionFile,
      entryId: user.id,
      targetFile,
    }),
    /EEXIST/,
  );
  assert.equal(await readFile(targetFile, "utf8"), saved);
  assert.ok(!saved.includes(token));
  const before = await readFile(params.sessionFile, "utf8");
  await assert.rejects(
    runtime.fork({
      sessionFile: params.sessionFile,
      entryId: "missing",
      targetFile: path.join(
        path.dirname(params.sessionFile),
        "invalid",
        "session.jsonl",
      ),
    }),
    /conversation message/,
  );
  await assert.rejects(
    runtime.fork({
      sessionFile: params.sessionFile,
      entryId: user.id,
      targetFile: params.sessionFile,
    }),
    /destination/,
  );
  assert.equal(await readFile(params.sessionFile, "utf8"), before);
});

test("run end reports the SDK context usage and compaction keeps an unknown token count", async (t) => {
  const params = {
    ...(await sandbox(t)),
    model: { ...model, contextWindow: 8192, maxOutputTokens: 128 },
    prompt: "long context ".repeat(500),
  };
  const events = [];
  const bodies = [];
  const runtime = new AssistantRuntime({
    emit: (e) => events.push(e),
    fetch: async (_url, input) => {
      bodies.push(JSON.parse(input.body));
      return stream("answer or summary");
    },
  });
  t.after(() => runtime.close());
  const first = await runtime.run(params);
  assert.equal(first.contextUsage.contextWindow, 8192);
  assert.equal(first.contextUsage.modelId, params.model.id);
  assert.ok(first.contextUsage.tokens > 0);
  assert.deepEqual(
    events.findLast((e) => e.type === "run_end").data.contextUsage,
    first.contextUsage,
  );
  await runtime.run({ ...params, runId: "second", operationId: "second" });
  await runtime.run({ ...params, runId: "third", operationId: "third" });
  const compact = await runtime.run(
    {
      ...params,
      runId: "compact",
      operationId: "compact",
      instructions: "PRIVATE-STANDING-INSTRUCTION-ONLY",
    },
    true,
  );
  assert.doesNotMatch(
    JSON.stringify(bodies.at(-1)),
    /PRIVATE-STANDING-INSTRUCTION-ONLY/,
  );
  assert.doesNotMatch(
    await readFile(params.sessionFile, "utf8"),
    /PRIVATE-STANDING-INSTRUCTION-ONLY/,
  );
  assert.equal(compact.contextUsage.contextWindow, 8192);
  assert.equal(compact.contextUsage.tokens, null);
  assert.deepEqual(
    events.findLast((e) => e.type === "run_end").data.contextUsage,
    compact.contextUsage,
  );
});

test("a context too small to compact is refused locally without an unknown bill", async (t) => {
  const params = await sandbox(t);
  let calls = 0;
  const runtime = new AssistantRuntime({
    fetch: async () => {
      calls++;
      return stream("short answer");
    },
  });
  t.after(() => runtime.close());
  await runtime.run(params);
  const result = await runtime.run(
    { ...params, runId: "compact", operationId: "compact" },
    true,
  );
  assert.equal(result.status, "error");
  assert.equal(result.errorCode, "assistant_compact_unavailable");
  assert.equal(result.errorKind, "not_sent");
  assert.equal(calls, 1);
});

test("request provenance survives reload with frozen prices and scrubbed gateway header IDs", async (t) => {
  for (const apex of ["ofox.ai", "ofox.io"]) {
    const params = {
      ...(await sandbox(t)),
      apex,
      model: {
        ...model,
        pricingKnown: true,
        cost: { input: 2, output: 3, cacheRead: 0, cacheWrite: 0 },
      },
    };
    const runtime = new AssistantRuntime({
      fetch: async () =>
        stream("receipt", {
          headers: {
            "content-type": "text/event-stream",
            "x-ofox-request-id": "req_gateway_123",
            "x-request-id": "lower_priority",
          },
        }),
    });
    t.after(() => runtime.close());
    const result = await runtime.run(params);
    const request = result.entries.find((entry) => entry.role === "request");
    assert.ok(request, "one separate receipt, without a second usage row");
    assert.equal(request.request.requestId, "req_gateway_123");
    assert.equal(request.request.requestIdSource, "x-ofox-request-id");
    assert.equal(request.request.httpStatus, 200);
    assert.equal(request.request.kind, "chat");
    assert.equal(request.request.apex, apex);
    assert.equal(request.request.modelId, model.id);
    assert.equal(request.request.status, "completed");
    assert.equal(request.request.usage.input, 10);
    assert.equal(request.request.priceSnapshot.cost.input, 2);
    assert.equal(request.request.billedCostUsd, null);
    assert.deepEqual((await runtime.load(params)).entries, result.entries);
    assert.equal(
      result.entries.filter((entry) => entry.role === "assistant").length,
      1,
    );
    assert.ok(!(await readFile(params.sessionFile, "utf8")).includes(token));
  }
});

test("refused and malformed streams retain response identity without inventing a completion ID", async (t) => {
  for (const [status, id] of [
    [429, "req_refused"],
    [200, token],
    [200, "bad id with spaces"],
  ]) {
    const params = await sandbox(t);
    const runtime = new AssistantRuntime({
      fetch: async () =>
        new Response(
          status === 429
            ? JSON.stringify({ error: { message: "limited" } })
            : "broken stream",
          {
            status,
            headers: {
              "content-type": "text/event-stream",
              "x-request-id": id,
            },
          },
        ),
    });
    t.after(() => runtime.close());
    const result = await runtime.run(params);
    const receipt = result.entries.find(
      (entry) => entry.role === "request",
    )?.request;
    assert.ok(receipt);
    assert.equal(receipt.httpStatus, status);
    assert.equal(receipt.status, status === 429 ? "rejected" : "unknown");
    assert.equal(receipt.requestId, status === 429 ? "req_refused" : null);
    assert.equal(receipt.billedCostUsd, null);
    assert.ok(!(await readFile(params.sessionFile, "utf8")).includes(token));
  }
});

test("a completed reply without usage retains its receipt while its cost stays unknown", async (t) => {
  const params = {
    ...(await sandbox(t)),
    model: { ...model, pricingKnown: true, cost: { input: 2, output: 3 } },
  };
  const runtime = new AssistantRuntime({
    fetch: async () =>
      stream("Completed without counters", { omitUsage: true }),
  });
  t.after(() => runtime.close());
  const result = await runtime.run(params);
  const receipt = result.entries.find(
    (entry) => entry.role === "request",
  ).request;
  assert.equal(receipt.status, "completed");
  assert.equal(
    receipt.requestId,
    null,
    "the body's completion ID is not a gateway correlation header",
  );
  assert.equal(receipt.estimatedCostUsd, null);
  assert.equal(receipt.billedCostUsd, null);
  assert.equal(result.usageCoverage, "incomplete");
});

test("system prompt states the local date, English weekday, IANA time zone and app language", async (t) => {
  const original = process.env.TZ;
  process.env.TZ = "Asia/Shanghai";
  t.after(() =>
    original === undefined
      ? delete process.env.TZ
      : (process.env.TZ = original),
  );
  for (const uiLanguage of ["zh", "en", "ja"])
    for (const now of ["2026-11-05T02:00:00Z", "2026-11-04T18:00:00Z"]) {
      const params = {
        ...(await sandbox(t)),
        uiLanguage,
        now,
      };
      let prompt;
      const runtime = new AssistantRuntime({
        fetch: async (_url, options) => {
          prompt = JSON.parse(options.body).messages[0].content;
          return stream("Answer");
        },
      });
      await runtime.run(params);
      assert.match(
        prompt,
        /Current date: 2026-11-05 \(Thursday\)\. Time zone: Asia\/Shanghai\./,
      );
      assert.ok(prompt.includes(`App language: ${uiLanguage}.`));
      await runtime.close();
    }
});

test("retries only the failed model HTTP request twice and preserves every receipt", async (t) => {
  for (const status of [429, 502, 503]) {
    const params = await sandbox(t);
    const events = [],
      delays = [];
    let requests = 0;
    const runtime = new AssistantRuntime({
      emit: (e) => events.push(e),
      retrySleep: async (ms) => delays.push(ms),
      fetch: async () => {
        requests++;
        return requests < 3
          ? Response.json(
              { error: { message: `busy ${token}` } },
              { status, headers: { "x-request-id": `attempt-${requests}` } },
            )
          : stream("One answer");
      },
    });
    const result = await runtime.run(params);
    assert.equal(result.status, "completed", String(status));
    assert.equal(requests, 3);
    assert.deepEqual(delays, [1000, 3000]);
    assert.deepEqual(
      events.filter((e) => e.type === "retrying").map((e) => e.data),
      [{ attempt: 1 }, { attempt: 2 }],
    );
    assert.equal(
      result.entries.filter((e) => e.role === "assistant").length,
      1,
    );
    const raw = await readFile(params.sessionFile, "utf8");
    assert.ok(!raw.includes(token));
    const receipts = raw
      .trim()
      .split("\n")
      .map(JSON.parse)
      .filter((e) => e.customType === "ofox-desktop-request")
      .map((e) => e.data);
    assert.deepEqual(
      receipts.map((e) => e.httpStatus),
      [status, status, 200],
    );
    assert.deepEqual(
      receipts.map((e) => e.status),
      [
        status === 429 ? "rejected" : "unknown",
        status === 429 ? "rejected" : "unknown",
        "completed",
      ],
    );
    assert.ok(!JSON.stringify(events).includes(token));
    await runtime.close();
  }
});

test("honours Retry-After up to 10 seconds on 429 and stops after two retries", async (t) => {
  for (const [header, expected] of [
    ["2", 2000],
    ["60", 10000],
    ["invalid", 1000],
  ]) {
    const params = await sandbox(t),
      delays = [],
      events = [];
    let requests = 0;
    const runtime = new AssistantRuntime({
      retrySleep: async (ms) => delays.push(ms),
      emit: (e) => events.push(e),
      fetch: async () => {
        requests++;
        return Response.json(
          { error: { message: "busy" } },
          { status: 429, headers: { "Retry-After": header } },
        );
      },
    });
    assert.equal((await runtime.run(params)).status, "error");
    assert.equal(requests, 3);
    assert.deepEqual(delays, [
      expected,
      header === "invalid" ? 3000 : expected,
    ]);
    assert.equal(events.filter((e) => e.type === "retrying").length, 2);
    await runtime.close();
  }
});

test("retries reliable connection failures but never timeouts, aborts, ambiguous networks or other HTTP errors", async (t) => {
  const cases = [
    [
      () => {
        throw networkError(systemError("ECONNREFUSED", "connect"));
      },
      3,
    ],
    [
      () => {
        throw networkError(systemError("ENOTFOUND", "getaddrinfo"));
      },
      3,
    ],
    [
      () => {
        throw networkError(systemError("UND_ERR_CONNECT_TIMEOUT", "connect"));
      },
      1,
    ],
    [
      () => {
        throw networkError(systemError("ECONNRESET", "read"));
      },
      1,
    ],
    [
      () => {
        throw Object.assign(new Error("timeout"), { name: "TimeoutError" });
      },
      1,
    ],
    [
      () => {
        throw Object.assign(new Error("cancelled"), {
          name: "AbortError",
          cause: systemError("ECONNREFUSED", "connect"),
        });
      },
      1,
    ],
    ...[400, 408, 409, 500, 504].map((status) => [
      () => Response.json({ error: { message: "failed" } }, { status }),
      1,
    ]),
  ];
  for (const [reply, count] of cases) {
    const params = await sandbox(t);
    let requests = 0;
    const runtime = new AssistantRuntime({
      retrySleep: async () => {},
      fetch: async () => {
        requests++;
        return reply();
      },
    });
    assert.equal((await runtime.run(params)).status, "error");
    assert.equal(requests, count);
    await runtime.close();
  }
});

test("partial text, reasoning and tool-call streams are never retried", async (t) => {
  for (const delta of [
    { content: "partial answer" },
    { reasoning_content: "partial thought" },
    {
      tool_calls: [
        {
          index: 0,
          id: "call",
          type: "function",
          function: { name: "probe", arguments: "{" },
        },
      ],
    },
  ]) {
    const params = await sandbox(t);
    let requests = 0;
    const runtime = new AssistantRuntime({
      retrySleep: async () => assert.fail("no partial retry"),
      fetch: async () => {
        requests++;
        return new Response(
          new ReadableStream({
            start(controller) {
              controller.enqueue(
                new TextEncoder().encode(
                  `data: ${JSON.stringify({ choices: [{ index: 0, delta, finish_reason: null }] })}\n\n`,
                ),
              );
              setTimeout(
                () =>
                  controller.error(
                    networkError(systemError("ECONNREFUSED", "connect")),
                  ),
                10,
              );
            },
          }),
          { headers: { "content-type": "text/event-stream" } },
        );
      },
    });
    assert.equal((await runtime.run(params)).status, "error");
    assert.equal(requests, 1);
    await runtime.close();
  }
});

test("cancelling during retry wait stops immediately without another HTTP request", async (t) => {
  const params = await sandbox(t);
  let requests = 0,
    retrySeen;
  const ready = new Promise((resolve) => (retrySeen = resolve));
  const runtime = new AssistantRuntime({
    emit: (e) => {
      if (e.type === "retrying") retrySeen();
    },
    fetch: async () => {
      requests++;
      return Response.json(
        { error: { message: "busy" } },
        { status: 429, headers: { "Retry-After": "60" } },
      );
    },
  });
  const running = runtime.run(params);
  await Promise.race([
    ready,
    running.then(() => assert.fail("expected retry wait")),
  ]);
  const start = Date.now();
  await runtime.stop(params);
  const result = await running;
  assert.equal(result.status, "stopped");
  assert.ok(Date.now() - start < 1000);
  assert.equal(requests, 1);
  await runtime.close();
});

test("request retries work for all supported protocols on both apexes without changing the request", async (t) => {
  for (const apex of ["ofox.ai", "ofox.io"])
    for (const [name, input] of Object.entries(await thinkingModels())) {
      const params = { ...(await sandbox(t)), apex, model: input };
      const bodies = [],
        events = [];
      const runtime = new AssistantRuntime({
        retrySleep: async () => {},
        emit: (e) => events.push(e),
        fetch: async (url, init) => {
          assert.equal(new URL(url).hostname, `api.${apex}`);
          assert.equal(
            new Headers(init.headers).get("authorization"),
            `Bearer ${token}`,
          );
          bodies.push(init.body);
          return bodies.length < 3
            ? Response.json({ error: { message: "busy" } }, { status: 503 })
            : thinkingReply(name);
        },
      });
      assert.equal(
        (await runtime.run(params)).status,
        "completed",
        `${name}/${apex}`,
      );
      assert.equal(bodies.length, 3);
      assert.equal(bodies[0], bodies[1]);
      assert.equal(bodies[1], bodies[2]);
      assert.equal(events.filter((e) => e.type === "retrying").length, 2);
      await runtime.close();
    }
});

test("retrying a model request after a tool or media operation never replays that side effect", async (t) => {
  for (const name of [
    probeToolName,
    "generate_image",
    "generate_video",
    "generate_speech",
  ]) {
    const params = {
      ...(await sandbox(t)),
      media: {
        image: { model: "image", models: ["image"], maxImages: 4 },
        video: {
          model: "video",
          models: ["video"],
          minDuration: 4,
          maxDuration: 30,
        },
        speech: { model: "speech", models: ["speech"], voices: ["voice"] },
      },
    };
    let requests = 0;
    const calls = [];
    const Runtime = name === probeToolName ? ProbeRuntime : AssistantRuntime;
    const args =
      name === probeToolName
        ? { command: "one operation" }
        : name === "generate_speech"
          ? { text: "Hello" }
          : { prompt: "One fox" };
    const runtime = new Runtime({
      retrySleep: async () => {},
      hostCall: async (call) => {
        calls.push(call);
        return { status: "completed", model: "media", files: [] };
      },
      fetch: async () => {
        requests++;
        return requests === 1
          ? toolStream(name, args)
          : requests < 4
            ? Response.json({ error: { message: "busy" } }, { status: 503 })
            : stream("Done once");
      },
    });
    assert.equal((await runtime.run(params)).status, "completed", name);
    assert.equal(requests, 4, name);
    assert.equal(calls.length, 1, name);
    await runtime.close();
  }
});

test("the SDK abort signal cancels a retry wait even without a run cancellation", async (t) => {
  const params = await sandbox(t);
  let requests = 0,
    retrySeen;
  const ready = new Promise((resolve) => (retrySeen = resolve));
  const runtime = new AssistantRuntime({
    emit: (e) => {
      if (e.type === "retrying") retrySeen();
    },
    fetch: async () => {
      requests++;
      return Response.json(
        { error: { message: "busy" } },
        { status: 429, headers: { "Retry-After": "60" } },
      );
    },
  });
  const running = runtime.run(params);
  await ready;
  const context = runtime.runs.get(params.runId);
  const start = Date.now();
  await context.session.abort();
  await running;
  assert.equal(context.abort.signal.aborted, false);
  assert.ok(Date.now() - start < 1000);
  assert.equal(requests, 1);
  await runtime.close();
});
