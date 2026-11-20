import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { AssistantRuntime } from "../src/runtime.mjs";
import { withPlanningOpening } from "./fixtures/planning-opening.mjs";
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
const call = (name, args, ...additional) =>
  sse([
    {
      choices: [
        {
          index: 0,
          delta: {
            role: "assistant",
            tool_calls: [[name, args], ...additional].map(
              ([tool, input], index) => ({
                index,
                id: `local-${++id}`,
                type: "function",
                function: { name: tool, arguments: JSON.stringify(input) },
              }),
            ),
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
  content = "Finished.",
) =>
  sse([
    {
      choices: [{ index: 0, delta: { role: "assistant", content } }],
    },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage,
    },
  ]);
async function fixture(t, turns, host, extra = {}, opening) {
  const root = await mkdtemp(path.join(os.tmpdir(), "ofox-local-runtime-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const requests = [],
    events = [],
    calls = [];
  const scriptedFetch = async (_url, options) => {
    requests.push(JSON.parse(options.body));
    return turns[requests.length - 1]?.() ?? reply();
  };
  const runtime = new AssistantRuntime({
    retrySleep: async () => {},
    fetch: opening
      ? withPlanningOpening(scriptedFetch, opening)
      : scriptedFetch,
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
      task: { tool: "codex", access: "configure" },
      skillPath: fileURLToPath(
        new URL("../skills/local-config/SKILL.md", import.meta.url),
      ),
      definition: { limits },
    },
    ...extra,
  };
  return {
    requests,
    events,
    calls,
    params,
    run: () => runtime.run(params),
    steer: (input) =>
      runtime.steer({
        sessionId: params.sessionId,
        runId: params.runId,
        operationId: params.operationId,
        ...input,
      }),
  };
}
function plannedFixture(t, turns, host, extra) {
  return fixture(t, turns, host, extra, "plan");
}

function singleStepFixture(t, turns, host, extra) {
  return fixture(t, turns, host, extra, "single");
}

const providerFailure = () =>
  new Response(JSON.stringify({ error: { message: "Insufficient balance" } }), {
    status: 402,
    headers: { "content-type": "application/json" },
  });
test("the Fizzy question answers in one request with a previously enabled local capability", async (t) => {
  const answer = "如果你指官方在线版，选 1. Hosted (app.fizzy.do)。";
  const prompt =
    "Are you using the hosted or self-hosted version?\n1. Hosted (app.fizzy.do)\n2. Self-hosted\nEnter a number between 1 and 2:\n\n哪个是global？";
  const shell = process.platform === "win32" ? "powershell" : "bash";
  const f = await fixture(
    t,
    [
      () =>
        f.requests[0].tools?.some(({ function: tool }) => tool.name === shell)
          ? call(shell, {
              command:
                'ls -la; echo ---; grep -rl "app.fizzy.do" . 2>/dev/null | head -20',
            })
          : reply(undefined, answer),
    ],
    () => {
      throw new Error("assistant_command_outcome_unknown");
    },
    { prompt },
  );
  delete f.params.localConfig.task;
  const result = await f.run();
  assert.deepEqual(f.calls, []);
  assert.equal(f.requests.length, 1);
  assert.deepEqual(
    f.requests[0].tools.map(({ function: tool }) => tool.name).sort(),
    ["beginSingleStep", "updatePlan"],
  );
  assert.doesNotMatch(
    f.requests[0].messages[0].content,
    /local-config|Use host tools to inspect configuration|local configuration tools are unavailable this turn/i,
  );
  assert.match(
    f.requests[0].messages[0].content,
    /Mention local diagnostic capabilities only if the user asks/i,
  );
  assert.equal(
    f.events.some((event) => event.type === "task_plan"),
    false,
  );
  assert.equal(result.status, "completed");
  assert.equal(
    result.entries.findLast((entry) => entry.role === "assistant")?.content,
    answer,
  );
});

test("a local tool with an unknown outcome gets one text-only reply while retaining its failure", async (t) => {
  const explanation = "检查命令已停止，无法确认结果。";
  const f = await plannedFixture(
    t,
    [
      () =>
        call("diagnose_in_folder", { tool: "codex" }, [
          "tool_status",
          { tool: "codex" },
        ]),
      () => reply(undefined, explanation),
    ],
    () => {
      throw new Error("assistant_command_outcome_unknown");
    },
    { prompt: "检查我这台机器的 Codex 当前模型" },
  );
  f.params.localConfig.task = { tool: "codex", access: "diagnose" };
  f.params.model.reasoning = true;
  f.params.model.maxOutputTokens = 4096;
  f.params.thinkingLevel = "high";
  const result = await f.run();
  assert.deepEqual(
    f.calls.map((c) => c.name),
    ["diagnose_in_folder"],
  );
  assert.ok(
    f.events.some(
      (e) =>
        e.type === "tool_end" &&
        e.data.name === "diagnose_in_folder" &&
        e.data.success === false &&
        e.data.message.includes("assistant_command_outcome_unknown"),
    ),
  );
  assert.equal(f.requests.length, 2);
  assert.equal(f.requests[0].reasoning_effort, "high");
  assert.equal(f.requests[1].reasoning_effort, undefined);
  assert.equal(f.requests[1].max_tokens, 1024);
  assert.equal(f.requests[1].tools?.length ?? 0, 0);
  assert.equal(result.status, "error");
  assert.match(result.errorMessage, /assistant_command_outcome_unknown/);
  assert.equal(
    result.entries.findLast((entry) => entry.role === "assistant")?.content,
    explanation,
  );
});

test(
  "live input cannot inherit a local configuration scope",
  { timeout: 5000 },
  async (t) => {
    const entered = Promise.withResolvers();
    const release = Promise.withResolvers();
    const f = await fixture(
      t,
      [
        async () => {
          entered.resolve();
          await release.promise;
          return reply();
        },
      ],
      () => assert.fail("Unexpected local call"),
    );
    const running = f.run();
    try {
      await Promise.race([
        entered.promise,
        running.then(() =>
          assert.fail("Run ended before reaching the gateway"),
        ),
      ]);
      for (const behavior of ["steer", "followUp"])
        await assert.rejects(
          f.steer({ id: behavior, behavior, text: "哪个是global?" }),
          /assistant_local_task_in_progress/,
        );
    } finally {
      release.resolve();
      await running;
    }
    assert.equal(f.requests.length, 1);
  },
);

test("a provider failure after a change keeps its own error and is not retried by the verification gate", async (t) => {
  const f = await plannedFixture(
    t,
    [
      () => call("diagnose_in_folder", { tool: "codex" }),
      () => call("rebind_tool", { tool: "codex" }),
      providerFailure,
      reply,
    ],
    (name) =>
      name === "rebind_tool"
        ? { changed: true }
        : { verified: false, exists: true },
  );
  const result = await f.run();
  assert.equal(result.status, "error");
  assert.notEqual(result.errorCode, "assistant_verification_required");
  assert.equal(f.requests.length, 3);
  assert.equal(result.localTaskState.changed, true);
});
test("a repair completes after verification in the host-selected folder", async (t) => {
  const f = await plannedFixture(
    t,
    [
      () => call("repair_launcher", { tool: "codex" }),
      () => call("verify_in_folder", { tool: "codex" }),
      reply,
    ],
    (name, args) =>
      name === "repair_launcher"
        ? { changed: true }
        : { folder: args.folder, verified: true },
  );
  const result = await f.run();
  assert.equal(result.status, "completed");
  assert.equal(f.requests.length, 3);
  assert.equal(
    f.calls.find((c) => c.name === "verify_in_folder").args.folder,
    f.params.localConfig.cwd,
  );
  assert.equal(result.localTaskState, null);
});
test("pending verification from an earlier run still has to be done", async (t) => {
  const f = await fixture(t, [reply, reply, reply, reply], () => ({}), {});
  f.params.localConfig.taskState = { changed: true, targets: [] };
  const result = await f.run();
  assert.equal(result.errorCode, "assistant_verification_required");
  assert.equal(f.requests.length, 3);
});
test("real Pi run stops the fifth repeated operation and emits warning before it", async (t) => {
  const f = await plannedFixture(
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
  const f = await plannedFixture(
    t,
    [
      () => call("tool_status", { tool: "codex" }),
      () => call("diagnose_in_folder", { tool: "codex" }),
      () => call("repair_launcher", { tool: "codex" }),
    ],
    () => ({ installed: true }),
  );
  // Inject a small budget through the trusted capability definition.
  const run = f.run; // fixture uses a shared definition intentionally only in this test.
  const old = limits.maxToolCalls;
  limits.maxToolCalls = 4; // one planning call, two operations, one text-only closeout
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
    const f = await plannedFixture(
      t,
      [
        () => call("diagnose_in_folder", { tool: "codex" }),
        () => call("rebind_tool", { tool: "codex" }),
        reply,
        () => call("verify_in_folder", { tool: "codex" }),
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
    assert.equal(
      f.calls.find((c) => c.name === "verify_in_folder").args.folder,
      f.calls.find((c) => c.name === "diagnose_in_folder").args.folder,
    );
  });
test("two unsuccessful completion continuations remain unverified, regardless of reply text", async (t) => {
  const f = await plannedFixture(
    t,
    [
      () => call("diagnose_in_folder", { tool: "codex" }),
      () => call("rebind_tool", { tool: "codex" }),
      reply,
      reply,
      reply,
    ],
    (name) =>
      name === "rebind_tool"
        ? { changed: true }
        : { verified: false, exists: true },
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
  const f = await singleStepFixture(
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
  const f = await plannedFixture(
    t,
    [
      () => call("diagnose_in_folder", { tool: "codex" }),
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

for (const apex of ["ofox.ai", "ofox.io"])
  for (const platform of ["macos", "windows"])
    test(`a Codex diagnosis uses only scoped reads and finishes without verification (${apex}, ${platform})`, async (t) => {
      const f = await singleStepFixture(
        t,
        [() => call("diagnose_in_folder", { tool: "codex" }), reply],
        (_name, args) => ({
          folder: args.folder,
          verified: true,
          model: "test/codex",
        }),
        { apex, prompt: "检查我这台机器的 Codex 当前模型" },
      );
      f.params.localConfig.platform = platform;
      f.params.localConfig.task.access = "diagnose";
      f.params.localConfig.taskState = { changed: true, targets: [] };
      const result = await f.run();
      assert.equal(result.status, "completed");
      assert.equal(result.errorCode, undefined);
      assert.equal(
        f.requests.length,
        2,
        "no extra model request to verify tools",
      );
      assert.deepEqual(
        f.calls.map((c) => c.name),
        ["diagnose_in_folder"],
      );
      assert.deepEqual(f.calls[0].args, {
        tool: "codex",
        folder: f.params.localConfig.cwd,
      });
      assert.deepEqual(
        f.requests[0].tools.map(({ function: tool }) => tool.name).sort(),
        ["beginSingleStep", "diagnose_in_folder", "tool_status", "updatePlan"],
      );
      assert.equal(
        Object.hasOwn(result, "localTaskState"),
        false,
        "a diagnosis must leave previous verification state alone",
      );
      assert.equal(
        f.events.some((e) => e.data?.warning === "verification_required"),
        false,
      );
    });

for (const platform of ["macos", "windows"])
  test(`a provider-invented shell call cannot run during configuration (${platform})`, async (t) => {
    const shell = platform === "windows" ? "powershell" : "bash";
    const f = await plannedFixture(
      t,
      [
        () => call("tool_status", { tool: "codex" }),
        () => call(shell, { command: "repair" }),
        reply,
      ],
      () => ({ installed: true }),
    );
    f.params.localConfig.platform = platform;
    await f.run();
    assert.deepEqual(
      f.calls.map((entry) => entry.name),
      ["tool_status"],
    );
    assert.equal(
      f.requests[0].tools.some(({ function: tool }) => tool.name === shell),
      false,
    );
  });

test("a follow-up in the same conversation can create a PPTX without local tool scope", async (t) => {
  const report = {
    name: "transformer.pptx",
    format: "pptx",
    content: [{ command: "add", parent: "/", type: "slide" }],
  };
  const f = await singleStepFixture(
    t,
    [
      () => call("tool_status", { tool: "codex" }),
      reply,
      () => call("create_document", report),
      reply,
    ],
    (name) =>
      name === "tool_status"
        ? { installed: true }
        : {
            status: "completed",
            file: {
              id: "transformer-deck",
              name: report.name,
              mimeType:
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
              bytes: 512,
              slides: 1,
            },
            issues: [],
          },
  );
  f.params.localConfig.task.access = "diagnose";
  const diagnosis = await f.run();
  assert.equal(diagnosis.status, "completed");

  delete f.params.localConfig.task;
  f.params.runId = "presentation-run";
  f.params.operationId = "presentation-operation";
  f.params.prompt = "做一个 Transformer 介绍 PPT";
  f.params.documents = { formats: ["pptx"] };
  const presentation = await f.run();
  assert.equal(presentation.status, "completed");
  assert.deepEqual(
    f.calls.map((entry) => entry.name),
    ["tool_status", "create_document"],
  );
  assert.deepEqual(f.calls[1].args, report);
  assert.deepEqual(
    f.requests[2].tools.map(({ function: tool }) => tool.name).sort(),
    [
      "beginSingleStep",
      "create_document",
      "get_presentation_style",
      "updatePlan",
    ],
  );
  assert.match(
    JSON.stringify(
      f.requests[3].messages.filter((entry) => entry.role === "tool"),
    ),
    /transformer-deck/,
  );
});

test("an admitted Codex diagnosis ignores a stale selected PPTX chip", async (t) => {
  const opening = [
    { step: "Inspect the Codex configuration", status: "in_progress" },
    { step: "Report the active model", status: "pending" },
  ];
  const completed = opening.map(({ step }) => ({ step, status: "completed" }));
  const f = await fixture(
    t,
    [
      () => call("updatePlan", { steps: opening }),
      () => call("diagnose_in_folder", { tool: "codex" }),
      () => call("updatePlan", { steps: completed }),
      reply,
    ],
    () => ({ installed: true }),
    {
      prompt: "检查本机 Codex 当前模型",
      documents: { formats: ["pptx"], selectedStyle: "auto" },
    },
  );
  f.params.localConfig.task.access = "diagnose";
  const result = await f.run();
  assert.equal(f.requests[0].tool_choice, undefined);
  assert.equal(result.status, "completed");
  assert.deepEqual(
    f.calls.map((entry) => entry.name),
    ["diagnose_in_folder"],
  );
  assert.deepEqual(
    f.events
      .filter((event) => event.type === "task_plan")
      .map((event) => event.data.steps),
    [opening, completed],
  );
  const lastTool = f.requests
    .at(-1)
    .messages.findLast((message) => message.role === "tool");
  assert.deepEqual(JSON.parse(lastTool.content).steps, completed);
});

test("a selected PPTX checklist stays open until the Host keeps the final file", async (t) => {
  const deck = {
    name: "deck.pptx",
    format: "pptx",
    content: [{ command: "add", parent: "/", type: "slide" }],
    preview: true,
  };
  const keep = {
    name: deck.name,
    format: deck.format,
    fromPreview: "preview-1",
  };
  const plan = {
    steps: [
      { step: "Prepare slides", status: "completed" },
      { step: "Deliver deck", status: "completed" },
    ],
  };
  const file = {
    id: "kept-deck",
    name: deck.name,
    mimeType:
      "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    bytes: 512,
  };
  let keepAttempts = 0;
  const f = await fixture(
    t,
    [
      () => call("updatePlan", plan),
      () => call("create_document", deck, ["updatePlan", plan]),
      () => call("create_document", keep, ["updatePlan", plan]),
      () => call("create_document", keep, ["updatePlan", plan]),
      () => call("create_document", keep, ["updatePlan", plan]),
      reply,
      () => call("updatePlan", plan),
      reply,
    ],
    (name, args) => {
      assert.equal(name, "create_document");
      if (args.preview) {
        return {
          status: "completed",
          preview: true,
          previewId: "preview-1",
          file,
        };
      }
      if (++keepAttempts === 1)
        return {
          status: "error",
          error: { message: "Preview could not be kept" },
        };
      if (keepAttempts === 2)
        return {
          status: "completed",
          file: { ...file, mimeType: "text/plain" },
        };
      return { status: "completed", file };
    },
    {
      localConfig: undefined,
      prompt: "Make a PowerPoint presentation",
      documents: { formats: ["pptx"], selectedStyle: "auto", python: true },
    },
  );
  assert.equal((await f.run()).status, "completed");
  assert.equal(keepAttempts, 3);
  const statuses = ["pending", "pending", "pending", "pending", "completed"];
  const plans = f.events.filter((event) => event.type === "task_plan");
  assert.deepEqual(
    plans.map((event) => event.data.steps.at(-1).status),
    statuses,
  );
  const messages = f.requests.at(-1).messages;
  const planCalls = new Set(
    messages
      .flatMap((message) => message.tool_calls ?? [])
      .filter(({ function: tool }) => tool.name === "updatePlan")
      .map(({ id }) => id),
  );
  const results = messages
    .filter((message) => planCalls.has(message.tool_call_id))
    .map((message) => JSON.parse(message.content));
  assert.deepEqual(
    results.map((result) => result.steps.at(-1).status),
    statuses,
  );
  f.params.runId = "next-deck-run";
  f.params.operationId = "next-deck-operation";
  f.params.prompt = "Make a second PowerPoint presentation";
  assert.equal((await f.run()).status, "completed");
  const next = f.events.findLast((event) => event.type === "task_plan");
  assert.equal(next.runId, "next-deck-run");
  assert.equal(next.data.steps.at(-1).status, "pending");
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
  test(`${name} counts toward the limit at its ceiling and lets the task finish, with one receipt`, async (t) => {
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
    assert.equal(result.status, "completed");
    assert.equal(result.errorCode, undefined);
    assert.equal(f.requests.length, 1);
    assert.equal(result.entries.filter((e) => e.role === "request").length, 1);
    // Never less than all 15 tokens at the input rate: unpriced cached
    // tokens count as input, an unreported request at its ceiling.
    const spent = f.events
      .filter((e) => e.type === "task_status" && "cost" in e.data)
      .at(-1).data.cost;
    assert.ok(spent > 14.9 / 1e6, `${spent}`);
  });

test("unknown retry receipts stay unknown while the task goes on to its tools and finishes", async (t) => {
  for (const status of [502, 503]) {
    const refused = () =>
      Response.json({ error: { message: "upstream unavailable" } }, { status });
    const f = await singleStepFixture(
      t,
      [refused, refused, () => call("tool_status", { tool: "codex" }), reply],
      () => ({ installed: true }),
    );
    const result = await f.run();
    assert.equal(f.requests.length, 4);
    assert.equal(f.calls.length, 1, "an unknown cost does not block the task");
    assert.equal(result.status, "completed");
    assert.equal(result.errorCode, undefined);
    assert.deepEqual(
      f.events.filter((e) => e.type === "retrying").map((e) => e.data.attempt),
      [1, 2],
    );
    assert.deepEqual(
      result.entries
        .filter((e) => e.role === "request")
        .map((e) => e.request.status),
      ["completed", "unknown", "unknown", "completed", "completed"],
    );
    const success = result.entries.find(
      (e) => e.role === "assistant" && e.usage?.output === 5,
    );
    assert.equal(success.usage.input, 10);
    assert.equal(success.usage.output, 5);
    assert.ok(success.usage.cost.total > 0);
    assert.equal(result.usageCoverage, "incomplete");
  }
});

test("rejected and reliably unsent retry receipts do not block local tools or their follow-up model request", async (t) => {
  for (const refused of [
    () =>
      Response.json({ error: { message: "rate limited" } }, { status: 429 }),
    () => {
      throw new TypeError("fetch failed", {
        cause: Object.assign(new Error("connection refused"), {
          code: "ECONNREFUSED",
          syscall: "connect",
        }),
      });
    },
  ]) {
    const f = await singleStepFixture(
      t,
      [refused, refused, () => call("tool_status", { tool: "codex" }), reply],
      () => ({ installed: true }),
    );
    const result = await f.run();
    assert.equal(result.status, "completed");
    assert.equal(f.requests.length, 4);
    assert.equal(f.calls.length, 1);
    assert.equal(result.usageCoverage, "complete");
    assert.equal(result.errorCode, undefined);
  }
});

test("an unknown OFox retry cost does not block use_skill", async (t) => {
  const skills = [
    {
      name: "sample",
      description: "Sample",
      dir: "/skills/sample",
      mode: "auto",
      hasScripts: false,
      reviewed: false,
    },
  ];
  const f = await singleStepFixture(
    t,
    [
      () =>
        Response.json({ error: { message: "unavailable" } }, { status: 503 }),
      () => call("use_skill", { name: "sample" }),
      reply,
    ],
    () => ({
      name: "sample",
      body: "loaded",
      dir: "/skills/sample",
      files: [],
      truncated: false,
    }),
    { skills },
  );
  const result = await f.run();
  assert.equal(f.requests.length, 3);
  assert.equal(f.calls.length, 1);
  assert.equal(result.status, "completed");
  assert.equal(result.usageCoverage, "incomplete");
});
