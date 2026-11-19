import assert from "node:assert/strict";
import { tmpdir } from "node:os";
import test from "node:test";
import { AssistantRuntime } from "../src/runtime.mjs";
import { scrubTaskPlanStructures } from "../src/task-plan.mjs";

function fixture(params = {}) {
  const events = [];
  const runtime = new AssistantRuntime({
    hostCall: async () => assert.fail("A task plan cannot call host tools"),
    emit: (event) => events.push(event),
  });
  const context = {
    attachments: new Map(),
    calls: new Map(),
    callCount: 0,
    unknownOutcome: false,
    params: {
      sessionId: "plan-session",
      runId: "plan-run",
      operationId: "plan-operation",
      ...params,
    },
  };
  const tools = runtime.tools(context, { input: ["text"] });
  return { context, events, runtime, tools };
}

test("ordinary tool-capable chats can report task plans without local configuration", async () => {
  const { events, tools, context } = fixture();
  assert.deepEqual(
    tools.map((tool) => tool.name),
    ["updatePlan"],
  );
  const steps = [
    { step: "Gather references", status: "completed" },
    { step: "Write the report", status: "in_progress" },
    { step: "Check the sources", status: "pending" },
  ];
  const result = await tools[0].execute("plan-call", { steps });
  assert.deepEqual(result.details, { steps });
  assert.deepEqual(
    events.filter((event) => event.type === "task_plan"),
    [
      {
        sessionId: "plan-session",
        runId: "plan-run",
        operationId: "plan-operation",
        type: "task_plan",
        data: { steps },
      },
    ],
  );
  assert.equal(context.callCount, 1);
  // A replayed call is an acknowledgement, not a second progress event.
  await tools[0].execute("plan-call", { steps });
  assert.equal(events.filter((event) => event.type === "task_plan").length, 1);
  assert.equal(context.callCount, 1);
});

test("local configuration does not register a second task plan tool", () => {
  for (const platform of ["macos", "windows"]) {
    const { tools } = fixture({ localConfig: { cwd: tmpdir(), platform } });
    assert.equal(tools.filter((tool) => tool.name === "updatePlan").length, 1);
  }
});

test("models without tools receive neither planning nor local operations", () => {
  const { tools } = fixture({
    model: { tools: false },
    localConfig: { cwd: tmpdir(), platform: "macos" },
  });
  assert.deepEqual(tools, []);
});

test("planning guidance covers multi-stage work and skips simple conversations", () => {
  const plan = fixture().tools.find((tool) => tool.name === "updatePlan");
  assert.ok(plan);
  const guidance = plan.promptGuidelines.join("\n");
  assert.match(guidance, /multi-stage/);
  assert.match(guidance, /3.6/);
  assert.match(guidance, /before finishing/);
  assert.match(guidance, /greetings.*translations.*short questions/);
});

test("task plan limits and redaction apply without exposing execution capabilities", async () => {
  const { tools, events, runtime } = fixture();
  const plan = tools.find((tool) => tool.name === "updatePlan");
  assert.ok(plan);
  for (const [index, steps] of [
    [{ step: "", status: "pending" }],
    [{ step: "x".repeat(41), status: "pending" }],
    Array.from({ length: 13 }, () => ({ step: "Step", status: "pending" })),
    [
      { step: "First", status: "in_progress" },
      { step: "Second", status: "in_progress" },
    ],
    [{ step: "Unrecognized", status: "failed" }],
    [{ step: "No hidden fields", status: "pending", command: "hidden" }],
  ].entries()) {
    await assert.rejects(
      plan.execute(`invalid-${index}`, { steps }),
      /assistant_invalid_plan/,
    );
  }
  assert.equal(
    events.some((event) => event.type === "task_plan"),
    false,
  );
  runtime.secrets.add("synthetic-plan-secret");
  const result = await plan.execute("unicode", {
    steps: [{ step: "🦊".repeat(40), status: "pending" }],
  });
  assert.equal([...result.details.steps[0].step].length, 40);
  await plan.execute("redaction", {
    steps: [{ step: "Use synthetic-plan-secret", status: "pending" }],
  });
  assert.doesNotMatch(JSON.stringify(events), /synthetic-plan-secret/);
});

test("plan updates retain the shared tool budget and unknown-outcome gate", async () => {
  const { context, events, tools } = fixture();
  const plan = tools.find((tool) => tool.name === "updatePlan");
  assert.ok(plan);
  const args = { steps: [{ step: "Review the outcome", status: "pending" }] };
  context.unknownOutcome = true;
  const blocked = await plan.execute("unknown", args);
  assert.equal(blocked.details.code, "unknown_outcome");
  assert.equal(blocked.terminate, true);
  assert.equal(
    events.some((event) => event.type === "task_plan"),
    false,
  );
  context.unknownOutcome = false;
  context.callCount = 20;
  await assert.rejects(
    plan.execute("budget", args),
    /Maximum tool-call count reached/,
  );
});

test("typed plan states survive matching credentials while labels stay redacted and bounded", async () => {
  const { runtime, tools, events } = fixture();
  for (const secret of ["completed", "in_progress", "abcdefgh"])
    runtime.secrets.add(secret);
  const plan = tools.find((tool) => tool.name === "updatePlan");
  const result = await plan.execute("states", {
    steps: [
      { step: "completed evidence", status: "completed" },
      { step: "in_progress analysis", status: "in_progress" },
      { step: "abcdefgh".repeat(5), status: "pending" },
    ],
    intent: "abcdefgh",
    unused: "abcdefgh",
  });
  const expected = ["completed", "in_progress", "pending"];
  assert.deepEqual(
    result.details.steps.map((step) => step.status),
    expected,
  );
  assert.deepEqual(JSON.parse(result.content[0].text), result.details);
  const shown = events.find((event) => event.type === "task_plan").data;
  assert.deepEqual(
    shown.steps.map((step) => step.status),
    expected,
  );
  for (const step of shown.steps) {
    assert.ok([...step.step].length <= 40);
    assert.doesNotMatch(step.step, /completed|in_progress|abcdefgh/);
  }
  const start = events.find((event) => event.type === "tool_start");
  assert.deepEqual(start.data.args, {});
  await assert.rejects(
    plan.execute("overlong", {
      steps: [{ step: "abcdefgh".repeat(5) + "x", status: "pending" }],
    }),
    /assistant_invalid_plan/,
  );
});

test("typed plan redaction never exempts unrelated tools, text, or unknown fields", () => {
  const secrets = ["completed", "abcdefgh"];
  const source = {
    role: "toolResult",
    toolName: "unrelated",
    content: [{ type: "text", text: "completed abcdefgh" }],
    details: {
      nested: {
        type: "toolCall",
        name: "updatePlan",
        arguments: {
          steps: [{ step: "abcdefgh", status: "completed" }],
        },
      },
    },
  };
  assert.doesNotMatch(
    JSON.stringify(scrubTaskPlanStructures(source, secrets)),
    /completed|abcdefgh/,
  );
  const call = scrubTaskPlanStructures(
    {
      role: "assistant",
      content: [
        {
          type: "toolCall",
          name: "updatePlan",
          id: "plan",
          note: "completed abcdefgh",
          arguments: {
            steps: [{ step: "abcdefgh", status: "completed" }],
            unknown: "abcdefgh",
          },
        },
      ],
    },
    secrets,
  ).content[0];
  assert.deepEqual(Object.keys(call.arguments), ["steps"]);
  assert.equal(call.arguments.steps[0].status, "completed");
  assert.equal(call.note, "[REDACTED] [REDACTED]");
  const invalid = scrubTaskPlanStructures(
    {
      role: "assistant",
      content: [
        {
          type: "toolCall",
          name: "updatePlan",
          arguments: {
            steps: [{ step: "Okay", status: "completed", unknown: "abcdefgh" }],
          },
        },
      ],
    },
    secrets,
  ).content[0];
  assert.deepEqual(invalid.arguments, { steps: null });
});
