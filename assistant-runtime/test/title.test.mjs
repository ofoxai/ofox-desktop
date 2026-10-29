import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { AssistantRuntime } from "../src/runtime.mjs";

const model = {
  id: "test/chat",
  supportedEndpoints: ["openai"],
  contextWindow: 32000,
  maxOutputTokens: 4096,
  pricingKnown: true,
  cost: { input: 2, output: 10 },
};
const token = "short-lived-title-credential-never-persist-987654321";

function stream(text) {
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
      usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
    },
  ];
  return new Response(
    chunks
      .map(
        (chunk) =>
          `data: ${JSON.stringify({ id: "t", object: "chat.completion.chunk", created: 1, model: model.id, ...chunk })}\n\n`,
      )
      .join("") + "data: [DONE]\n\n",
    { headers: { "content-type": "text/event-stream" } },
  );
}

function networkError(cause) {
  return new TypeError("fetch failed", { cause });
}
function systemError(code, syscall) {
  return Object.assign(new Error(`${syscall} ${code}`), { code, syscall });
}

async function sandbox(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "ofox-title-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return {
    sessionId: "session-1",
    runId: "run-1",
    operationId: "operation-1",
    sessionFile: path.join(directory, "session.jsonl"),
    model,
    apiKey: token,
    apex: "ofox.io",
    prompt: "Plan a three-day trip to Kyoto",
  };
}

/** A conversation with one answered turn, and a runtime whose model requests
 * `reply` answers (and records). */
async function answered(t, reply) {
  const params = await sandbox(t);
  const requests = [];
  let turnDone = false;
  const runtime = new AssistantRuntime({
    fetch: async (url, init) => {
      if (!turnDone)
        return stream("Day one: Fushimi Inari. Day two: Arashiyama.");
      requests.push({ url: String(url), init, body: JSON.parse(init.body) });
      return reply(init);
    },
    titleTimeoutMs: 300,
  });
  t.after(() => runtime.close());
  const turn = await runtime.run(params);
  assert.equal(turn.status, "completed");
  turnDone = true;
  const title = (extra = {}) =>
    runtime.title({
      ...params,
      userText: params.prompt,
      replyText: "Day one: Fushimi Inari. Day two: Arashiyama.",
      ...extra,
    });
  return { params, runtime, requests, turn, title };
}

const usageEntries = (entries) =>
  entries.filter((entry) => entry.role === "usage");

test("a title is one short request about the first turn, and its cost is kept under that turn", async (t) => {
  const { params, runtime, requests, turn, title } = await answered(t, () =>
    stream("Kyoto Trip Plan"),
  );
  const reply = "r".repeat(1500) + "BEYOND-THE-LIMIT";
  const result = await title({ replyText: reply });
  assert.equal(result.title, "Kyoto Trip Plan");
  assert.equal(requests.length, 1);
  const [request] = requests;
  assert.equal(request.body.max_tokens, 32);
  assert.equal(request.body.tools, undefined);
  assert.equal(request.body.reasoning_effort, undefined);
  assert.equal(request.body.messages[0].role, "system");
  assert.match(request.body.messages[0].content, /title only/);
  assert.match(request.body.messages[0].content, /at most 6 words/);
  const asked = JSON.stringify(request.body.messages[1]);
  assert.match(asked, /Plan a three-day trip to Kyoto/);
  assert.ok(asked.includes("r".repeat(1500)));
  assert.equal(asked.includes("BEYOND-THE-LIMIT"), false);
  // The key goes only in the Authorization header.
  assert.equal(
    new Headers(request.init.headers).get("authorization"),
    `Bearer ${token}`,
  );
  assert.equal(JSON.stringify(request.body).includes(token), false);
  assert.equal(request.url.includes(token), false);

  const added = usageEntries(result.entries).filter(
    (entry) => entry.type === "title",
  );
  assert.equal(added.length, 1);
  assert.equal(added[0].runId, params.runId);
  assert.equal(added[0].operationId, params.operationId);
  assert.equal(added[0].usageCoverage, "complete");
  assert.equal(added[0].pricingKnown, true);
  assert.equal(added[0].usage.input, 10);
  assert.equal(added[0].usage.output, 5);
  assert.equal(added[0].priceSnapshot.modelId, model.id);
  // Everything the turn had is still there, plus the title's cost.
  assert.equal(result.entries.length, turn.entries.length + 1);
  assert.deepEqual(
    (await new AssistantRuntime().load(params)).entries,
    result.entries,
  );
  assert.equal(
    (await readFile(params.sessionFile, "utf8")).includes(token),
    false,
  );

  // The title's record is no context: the next turn never sends it.
  let next;
  runtime.providerFetch = async (_url, init) => {
    next = JSON.parse(init.body);
    return stream("Day three: Nara.");
  };
  const resumed = await runtime.run({
    ...params,
    runId: "run-2",
    operationId: "operation-2",
    prompt: "And day three?",
  });
  assert.equal(resumed.status, "completed");
  assert.equal(JSON.stringify(next).includes("Kyoto Trip Plan"), false);
  assert.equal(JSON.stringify(next).includes("title only"), false);
  assert.match(JSON.stringify(next.messages), /Fushimi Inari/);
});

test("a title request that fails gives no title, never throws, and keeps what it may have cost", async (t) => {
  for (const [name, reply, recorded] of [
    [
      "refused",
      () => Response.json({ error: { message: "bad" } }, { status: 400 }),
      false,
    ],
    [
      "never sent",
      () => {
        throw networkError(systemError("ECONNREFUSED", "connect"));
      },
      false,
    ],
    [
      "server error",
      () => Response.json({ error: { message: "upstream" } }, { status: 500 }),
      true,
    ],
    [
      "reset after sending",
      () => {
        throw networkError(systemError("ECONNRESET", "read"));
      },
      true,
    ],
  ]) {
    const { params, turn, title } = await answered(t, reply);
    const before = await readFile(params.sessionFile, "utf8");
    const result = await title();
    assert.equal(result.title, null, name);
    if (!recorded) {
      // Nothing was generated: native history is untouched.
      assert.equal(result.entries, undefined, name);
      assert.equal(await readFile(params.sessionFile, "utf8"), before, name);
      continue;
    }
    assert.equal(result.entries.length, turn.entries.length + 1, name);
    const added = usageEntries(result.entries).filter(
      (entry) => entry.type === "title",
    );
    assert.equal(added.length, 1, name);
    assert.equal(added[0].usageCoverage, "incomplete", name);
    assert.equal(added[0].pricingKnown, false, name);
    assert.equal(added[0].usage, null, name);
    assert.equal(result.usageCoverage, "incomplete", name);
  }
});

test("a title request that does not answer in time is given up", async (t) => {
  const { title } = await answered(
    t,
    (init) =>
      new Promise((_resolve, reject) => {
        init.signal?.addEventListener("abort", () =>
          reject(init.signal.reason ?? new Error("aborted")),
        );
      }),
  );
  const started = Date.now();
  const result = await title();
  assert.ok(Date.now() - started < 5000, "the timeout ends the request");
  assert.equal(result.title, null);
  // It was sent: it may have been billed.
  const added = usageEntries(result.entries).filter(
    (entry) => entry.type === "title",
  );
  assert.equal(added.length, 1);
  assert.equal(added[0].usageCoverage, "incomplete");
});

test("a title thinks only at the level the host chose", async (t) => {
  const nano = {
    ...model,
    id: "openai/gpt-5-nano",
    reasoning: true,
    thinkingLevelMap: {
      high: "high",
      low: "low",
      max: null,
      medium: "medium",
      minimal: "minimal",
      off: null,
      xhigh: null,
    },
  };
  const { requests, title } = await answered(t, () => stream("Kyoto"));
  await title({ model: nano, thinkingLevel: "minimal" });
  assert.equal(requests.at(-1).body.reasoning_effort, "minimal");
  assert.equal(requests.at(-1).body.model, "openai/gpt-5-nano");
  await title({ model: { ...model, id: "openai/gpt-5-nano" } });
  assert.equal(requests.at(-1).body.reasoning_effort, undefined);
});

test("the title instruction is in the language the user wrote in", async (t) => {
  const { requests, title } = await answered(t, () => stream("标题"));
  const instruction = async (userText) => {
    await title({ userText });
    return requests.at(-1).body.messages[0].content;
  };
  assert.match(await instruction("帮我规划京都三日游"), /12 个字/);
  assert.match(await instruction("京都の旅行プランを作って"), /12文字/);
  assert.match(await instruction("Plan a trip to Kyoto"), /6 words/);
});

test("a title needs a saved conversation and trusted parameters", async (t) => {
  const { params, title } = await answered(t, () => stream("Kyoto"));
  await assert.rejects(
    title({
      sessionFile: path.join(path.dirname(params.sessionFile), "gone.jsonl"),
    }),
  );
  await assert.rejects(title({ apiKey: "" }), /apiKey/);
  await assert.rejects(title({ userText: undefined }), /title input/);
  await assert.rejects(title({ sessionFile: "relative.jsonl" }), /absolute/);
});
