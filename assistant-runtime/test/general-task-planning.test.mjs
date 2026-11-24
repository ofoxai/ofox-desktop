import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { Type } from "typebox";
import { AssistantRuntime } from "../src/runtime.mjs";
import { scrubTaskPlanStructures } from "../src/task-plan.mjs";

let sequence = 0;
const plan = [
  { step: "Inspect the source", status: "in_progress" },
  { step: "Transform and verify the result", status: "pending" },
];
function response(protocol, operations = [], text = "Finished") {
  const serial = ++sequence;
  const calls = operations.map(([name, args], index) => ({
    id: `general-${serial}-${index}`,
    name,
    args,
  }));
  let rows;
  if (protocol === "responses") {
    const end = {
      id: `response-${serial}`,
      status: "completed",
      usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
    };
    rows = [{ type: "response.created", response: end }];
    if (calls.length)
      for (const [index, call] of calls.entries()) {
        const item = {
          id: `fc-${call.id}`,
          call_id: call.id,
          type: "function_call",
          name: call.name,
        };
        rows.push(
          { type: "response.output_item.added", output_index: index, item },
          {
            type: "response.output_item.done",
            output_index: index,
            item: { ...item, arguments: JSON.stringify(call.args) },
          },
        );
      }
    else {
      const item = {
        id: `msg-${serial}`,
        type: "message",
        role: "assistant",
        status: "completed",
        content: [{ type: "output_text", text, annotations: [] }],
      };
      rows.push(
        { type: "response.output_item.added", output_index: 0, item },
        {
          type: "response.content_part.added",
          item_id: item.id,
          output_index: 0,
          content_index: 0,
          part: { type: "output_text", text: "", annotations: [] },
        },
        {
          type: "response.output_text.delta",
          item_id: item.id,
          output_index: 0,
          content_index: 0,
          delta: text,
        },
        { type: "response.output_item.done", output_index: 0, item },
      );
    }
    rows.push({ type: "response.completed", response: end });
  } else if (protocol === "anthropic") {
    rows = [
      {
        type: "message_start",
        message: {
          id: `msg-${serial}`,
          type: "message",
          role: "assistant",
          model: "test/chat",
          content: [],
          usage: { input_tokens: 1, output_tokens: 0 },
        },
      },
    ];
    for (const [index, call] of calls.entries())
      rows.push(
        {
          type: "content_block_start",
          index,
          content_block: {
            type: "tool_use",
            id: call.id,
            name: call.name,
            input: {},
          },
        },
        {
          type: "content_block_delta",
          index,
          delta: {
            type: "input_json_delta",
            partial_json: JSON.stringify(call.args),
          },
        },
        { type: "content_block_stop", index },
      );
    if (!calls.length)
      rows.push(
        {
          type: "content_block_start",
          index: 0,
          content_block: { type: "text", text: "" },
        },
        {
          type: "content_block_delta",
          index: 0,
          delta: { type: "text_delta", text },
        },
        { type: "content_block_stop", index: 0 },
      );
    rows.push(
      {
        type: "message_delta",
        delta: {
          stop_reason: calls.length ? "tool_use" : "end_turn",
          stop_sequence: null,
        },
        usage: { output_tokens: 1 },
      },
      { type: "message_stop" },
    );
  } else {
    rows = [
      {
        choices: [
          {
            index: 0,
            delta: {
              role: "assistant",
              ...(calls.length
                ? {
                    tool_calls: calls.map((call, index) => ({
                      index,
                      id: call.id,
                      type: "function",
                      function: {
                        name: call.name,
                        arguments: JSON.stringify(call.args),
                      },
                    })),
                  }
                : { content: text }),
            },
          },
        ],
      },
      {
        choices: [
          {
            index: 0,
            delta: {},
            finish_reason: calls.length ? "tool_calls" : "stop",
          },
        ],
        usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
      },
    ];
  }
  return new Response(
    rows
      .map(
        (row) =>
          `${row.type ? `event: ${row.type}\n` : ""}data: ${JSON.stringify(row)}\n\n`,
      )
      .join(""),
    { headers: { "content-type": "text/event-stream" } },
  );
}

async function fixture(t, replies, protocol = "completions", extra = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), "ofox-general-plan-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const requests = [],
    events = [],
    calls = [];
  const runtime = new AssistantRuntime({
    emit: (event) => events.push(event),
    fetch: async (_url, init) => {
      requests.push(JSON.parse(init.body));
      const configured = replies[requests.length - 1];
      const reply =
        typeof configured === "function"
          ? configured(requests.at(-1))
          : configured;
      assert.ok(reply, "The model must not retry indefinitely");
      await reply.wait?.();
      return response(protocol, reply.operations, reply.text);
    },
    hostCall: async ({ name, args }) => {
      calls.push({ name, args });
      return { checked: true };
    },
  });
  t.after(() => runtime.close());
  const original = runtime.tools.bind(runtime);
  runtime.tools = (context, model) => [
    ...original(context, model),
    ...(context.params.model.tools === false
      ? []
      : [
          runtime.wrapTool(
            context,
            "inspect_fixture",
            "Inspect this task's synthetic source data.",
            Type.Object({ item: Type.String() }),
            (args, signal, id) =>
              runtime.hostTool(context, "inspect_fixture", args, signal, id),
          ),
        ]),
  ];
  const params = {
    sessionId: "general-session",
    runId: "general-run",
    operationId: "general-operation",
    sessionFile: path.join(root, "session.jsonl"),
    apiKey: "general-test-key-not-real",
    apex: "ofox.ai",
    prompt: "Analyze and verify this source.",
    model: {
      id: "test/chat",
      supportedEndpoints: [protocol === "completions" ? "openai" : protocol],
      contextWindow: 32000,
      maxOutputTokens: 1024,
    },
    ...extra,
  };
  return {
    runtime,
    requests,
    events,
    calls,
    params,
    run: () => runtime.run(params),
  };
}
const names = (request) =>
  (request.tools ?? []).map((tool) => tool.function?.name ?? tool.name).sort();

test("simple answers stay one request while unplanned execution tools are withheld", async (t) => {
  for (const protocol of ["completions", "responses", "anthropic"])
    await t.test(protocol, async (t) => {
      const f = await fixture(t, [{ text: "Hosted is option 1." }], protocol);
      assert.equal((await f.run()).status, "completed");
      assert.equal(f.requests.length, 1);
      assert.deepEqual(names(f.requests[0]), ["beginSingleStep", "updatePlan"]);
      assert.equal(
        f.events.some((event) => event.type === "task_plan"),
        false,
      );
      assert.deepEqual(f.calls, []);
    });
});

test("an ignored admission cannot execute or silently complete the requested operation", async (t) => {
  const f = await fixture(t, [
    { operations: [["inspect_fixture", { item: "private-fixture" }]] },
    { text: "I inspected it." },
  ]);
  const result = await f.run();
  assert.deepEqual(f.calls, []);
  assert.equal(result.status, "error");
  assert.equal(result.errorCode, "assistant_plan_required");
});

test("a non-PPT task plans before execution and publishes real progress across providers", async (t) => {
  for (const protocol of ["completions", "responses", "anthropic"])
    await t.test(protocol, async (t) => {
      const progress = plan.map(({ step }, index) => ({
        step,
        status: index ? "in_progress" : "completed",
      }));
      const complete = plan.map(({ step }) => ({ step, status: "completed" }));
      const f = await fixture(
        t,
        [
          { operations: [["updatePlan", { steps: plan }]] },
          { operations: [["inspect_fixture", { item: "source" }]] },
          { operations: [["updatePlan", { steps: progress }]] },
          { operations: [["inspect_fixture", { item: "transformed-result" }]] },
          { operations: [["updatePlan", { steps: complete }]] },
          { text: "Verified result delivered." },
        ],
        protocol,
      );
      assert.equal((await f.run()).status, "completed");
      assert.deepEqual(
        f.calls.map(({ args }) => args.item),
        ["source", "transformed-result"],
      );
      assert.deepEqual(
        f.events
          .filter((event) => event.type === "task_plan")
          .map((event) => event.data.steps),
        [plan, progress, complete],
      );
      assert.ok(names(f.requests[1]).includes("inspect_fixture"));
    });
});

test("one real operation needs no visible checklist and cannot renew its allowance", async (t) => {
  const f = await fixture(t, [
    { operations: [["beginSingleStep", {}]] },
    { operations: [["inspect_fixture", { item: "one" }]] },
    {
      operations: [
        ["beginSingleStep", {}],
        ["inspect_fixture", { item: "blocked" }],
      ],
    },
    { text: "The second call was blocked." },
  ]);
  assert.equal((await f.run()).status, "error");
  assert.deepEqual(
    f.calls.map(({ args }) => args.item),
    ["one"],
  );
  assert.equal(
    f.events.some((event) => event.type === "task_plan"),
    false,
  );
  assert.equal(
    f.events.some(
      (event) =>
        event.type === "tool_start" && event.data.name === "beginSingleStep",
    ),
    false,
  );
  assert.deepEqual(names(f.requests[2]), ["updatePlan"]);
});

test("a second operation promotes a single-step task into a visible multi-stage plan", async (t) => {
  const promoted = [
    { step: "Inspect the source", status: "completed" },
    { step: "Verify another source", status: "in_progress" },
  ];
  const f = await fixture(t, [
    { operations: [["beginSingleStep", {}]] },
    { operations: [["inspect_fixture", { item: "first" }]] },
    { operations: [["inspect_fixture", { item: "not-yet" }]] },
    { operations: [["updatePlan", { steps: promoted }]] },
    { operations: [["inspect_fixture", { item: "after-plan" }]] },
    {
      operations: [
        [
          "updatePlan",
          {
            steps: promoted.map(({ step }) => ({ step, status: "completed" })),
          },
        ],
      ],
    },
    { text: "Both sources verified." },
  ]);
  assert.equal((await f.run()).status, "completed");
  assert.deepEqual(
    f.calls.map(({ args }) => args.item),
    ["first", "after-plan"],
  );
  assert.equal(
    f.events.filter((event) => event.type === "task_plan").length,
    2,
  );
});

test("same-response preparation cannot execute work and parallel calls share one slot", async (t) => {
  for (const admission of [
    ["beginSingleStep", {}],
    ["updatePlan", { steps: plan }],
  ])
    await t.test(admission[0], async (t) => {
      const f = await fixture(t, [
        {
          operations: [
            admission,
            ["inspect_fixture", { item: "same-response" }],
          ],
        },
        {
          operations: [
            ["inspect_fixture", { item: "next-response" }],
            ["inspect_fixture", { item: "second-slot" }],
          ],
        },
        { text: "Finished." },
      ]);
      const result = await f.run();
      assert.deepEqual(
        f.calls.map(({ args }) => args.item),
        admission[0] === "beginSingleStep"
          ? []
          : ["next-response", "second-slot"],
      );
      assert.equal(
        result.status,
        admission[0] === "beginSingleStep" ? "error" : "completed",
      );
    });
});

test("invalid empty plans cannot clear the checklist or reset a used single-step slot", async (t) => {
  const f = await fixture(t, [
    { operations: [["beginSingleStep", {}]] },
    { operations: [["inspect_fixture", { item: "one" }]] },
    { operations: [["updatePlan", { steps: [] }]] },
    { operations: [["inspect_fixture", { item: "blocked" }]] },
    { text: "Stopped." },
  ]);
  assert.equal((await f.run()).status, "error");
  assert.deepEqual(
    f.calls.map(({ args }) => args.item),
    ["one"],
  );
  assert.equal(
    f.events.some((event) => event.type === "task_plan"),
    false,
  );
});

test("repeated ignored admission is bounded and cannot call the Host", async (t) => {
  const f = await fixture(
    t,
    Array.from({ length: 3 }, () => ({
      operations: [["inspect_fixture", { item: "ignored" }]],
    })),
  );
  const result = await f.run();
  assert.equal(f.requests.length, 3);
  assert.equal(result.status, "error");
  assert.deepEqual(f.calls, []);
});

test("a failed tool keeps unfinished plan states instead of manufacturing completed checks", async (t) => {
  const f = await fixture(t, [
    { operations: [["updatePlan", { steps: plan }]] },
    { operations: [["inspect_fixture", { item: "unknown" }]] },
  ]);
  f.runtime.hostCall = async () => {
    throw new Error("assistant_command_outcome_unknown");
  };
  assert.equal((await f.run()).status, "error");
  assert.deepEqual(
    f.events
      .filter((event) => event.type === "task_plan")
      .map((event) => event.data.steps),
    [plan],
  );
});

test("a simple follow-up in the same saved conversation starts without the old checklist", async (t) => {
  const f = await fixture(t, [
    { operations: [["updatePlan", { steps: plan }]] },
    { operations: [["inspect_fixture", { item: "source" }]] },
    { text: "Work remains unverified." },
    { text: "Hosted is option 1." },
  ]);
  assert.equal((await f.run()).status, "completed");
  const previous = f.events.length;
  assert.equal(
    (
      await f.runtime.run({
        ...f.params,
        runId: "follow-up",
        operationId: "follow-up",
        prompt: "Which option is hosted?",
      })
    ).status,
    "completed",
  );
  assert.deepEqual(names(f.requests[3]), ["beginSingleStep", "updatePlan"]);
  assert.equal(
    f.events.slice(previous).some((event) => event.type === "task_plan"),
    false,
  );
});

test("tool-free models receive no execution preparation tools", async (t) => {
  const f = await fixture(t, [{ text: "Direct answer." }], "completions", {
    model: {
      id: "test/chat",
      supportedEndpoints: ["openai"],
      tools: false,
      contextWindow: 32000,
      maxOutputTokens: 1024,
    },
  });
  assert.equal((await f.run()).status, "completed");
  assert.deepEqual(names(f.requests[0]), []);
});

test("applied steering resets preparation without inheriting the previous task's tool access", async (t) => {
  let entered, release;
  const waiting = new Promise((resolve) => {
    entered = resolve;
  });
  const blocked = new Promise((resolve) => {
    release = resolve;
  });
  t.after(() => release());
  const f = await fixture(t, [
    { operations: [["updatePlan", { steps: plan }]] },
    { operations: [["inspect_fixture", { item: "source" }]] },
    {
      text: "The old work is unfinished.",
      wait: () => {
        entered();
        return blocked;
      },
    },
    { text: "Hosted is option 1." },
  ]);
  const running = f.run();
  await waiting;
  await f.runtime.steer({
    ...f.params,
    id: "steered-input",
    behavior: "steer",
    text: "Which option is hosted?",
  });
  release();
  assert.equal((await running).status, "completed");
  assert.deepEqual(names(f.requests[3]), ["beginSingleStep", "updatePlan"]);
  assert.deepEqual(
    f.events
      .filter((event) => event.type === "steer_applied")
      .map((event) => event.data.id),
    ["steered-input"],
  );
  assert.deepEqual(
    f.calls.map(({ args }) => args.item),
    ["source"],
  );
  assert.equal(
    f.events.filter((event) => event.type === "task_plan").length,
    1,
  );
});

test("single-step protocol identities and schema survive matching credentials", () => {
  const value = {
    messages: [
      {
        role: "system",
        toolsAdded: [
          { name: "beginSingleStep", parameters: { invalid: true } },
        ],
      },
      {
        role: "assistant",
        content: [
          {
            type: "toolCall",
            name: "beginSingleStep",
            id: "one",
            arguments: { ignored: "private-value" },
          },
        ],
      },
      {
        role: "toolResult",
        toolName: "beginSingleStep",
        content: [
          {
            type: "text",
            text: JSON.stringify({
              mode: "single_step",
              remainingOperations: 1,
            }),
          },
        ],
        details: { mode: "single_step", remainingOperations: 1 },
      },
    ],
  };
  const safe = scrubTaskPlanStructures(value, [
    "begin",
    "single_step",
    "private-value",
  ]);
  assert.equal(safe.messages[0].toolsAdded[0].name, "beginSingleStep");
  assert.equal(safe.messages[0].toolsAdded[0].parameters.type, "object");
  assert.deepEqual(safe.messages[1].content[0].arguments, {});
  assert.equal(safe.messages[1].content[0].name, "beginSingleStep");
  assert.deepEqual(JSON.parse(safe.messages[2].content[0].text), {
    mode: "single_step",
    remainingOperations: 1,
  });
  assert.doesNotMatch(JSON.stringify(safe), /private-value/);
});

test("beginSingleStep refusals and unknown outcomes remain failures through typed scrubbing", () => {
  for (const details of [
    { code: "assistant_tool_limit" },
    { code: "unknown_outcome" },
  ]) {
    const source = {
      role: "toolResult",
      toolName: "beginSingleStep",
      terminate: true,
      content: [{ type: "text", text: `${details.code} private-refusal` }],
      details,
    };
    const safe = scrubTaskPlanStructures(source, ["private-refusal"]);
    assert.deepEqual(safe.details, details);
    assert.match(safe.content[0].text, /REDACTED/);
    assert.doesNotMatch(JSON.stringify(safe), /single_step|private-refusal/);
    assert.equal(safe.terminate, true);
  }
  const malformed = {
    type: "tool_result",
    toolName: "beginSingleStep",
    terminate: true,
    content: [{ type: "text", text: "operation refused" }],
    details: { mode: "single_step", remainingOperations: 1 },
  };
  assert.deepEqual(scrubTaskPlanStructures(malformed, []), malformed);
});

test("preparation lists current capabilities after chronological add, remove and replacement", async (t) => {
  const f = await fixture(t, [{ text: "A direct answer." }]);
  const { SessionManager } = await import("@earendil-works/pi-coding-agent");
  const manager = SessionManager.open(
    f.params.sessionFile,
    path.dirname(f.params.sessionFile),
    path.dirname(f.params.sessionFile),
  );
  const schema = { type: "object", properties: {} };
  manager.appendMessage({
    role: "system",
    content: "Earlier capabilities.",
    timestamp: 1,
    toolsAdded: [
      {
        name: "retired_capability",
        description: "Retired capability",
        parameters: schema,
      },
      {
        name: "inspect_fixture",
        description: "Old description",
        parameters: schema,
      },
    ],
  });
  manager.appendMessage({
    role: "user",
    content: "Earlier question.",
    timestamp: 2,
  });
  manager.appendMessage({
    role: "assistant",
    content: [{ type: "text", text: "Earlier answer." }],
    api: "openai-completions",
    provider: "ofox",
    model: "test/chat",
    stopReason: "stop",
    timestamp: 3,
    usage: {
      input: 1,
      output: 1,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 2,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
  });
  manager.appendMessage({
    role: "system",
    content: "Removed capabilities.",
    timestamp: 4,
    toolsRemoved: [{ name: "retired_capability" }, { name: "inspect_fixture" }],
  });
  manager.appendMessage({
    role: "system",
    content: "Replacement capability.",
    timestamp: 5,
    toolsAdded: [
      {
        name: "inspect_fixture",
        description: "Current replacement description",
        parameters: schema,
      },
    ],
  });
  assert.equal((await f.run()).status, "completed");
  const system = f.requests[0].messages.find(
    (message) => message.role === "system",
  ).content;
  const preparation = system.slice(
    system.indexOf("Task execution preparation:"),
  );
  assert.ok(
    preparation.includes("Inspect this task's synthetic source data."),
    preparation,
  );
  assert.doesNotMatch(preparation, /retired_capability|Old description/);
});

test("a blocked second operation automatically requests a real plan before retrying across providers", async (t) => {
  for (const protocol of ["completions", "responses", "anthropic"])
    await t.test(protocol, async (t) => {
      const choice =
        protocol === "completions"
          ? { type: "function", function: { name: "updatePlan" } }
          : {
              type: protocol === "responses" ? "function" : "tool",
              name: "updatePlan",
            };
      const promoted = [
        { step: "Create the corrected file", status: "completed" },
        { step: "Read and verify the corrected file", status: "in_progress" },
      ];
      const f = await fixture(
        t,
        [
          { operations: [["beginSingleStep", {}]] },
          { operations: [["inspect_fixture", { item: "created-file" }]] },
          { operations: [["inspect_fixture", { item: "blocked-read" }]] },
          (body) =>
            JSON.stringify(body.tool_choice) === JSON.stringify(choice)
              ? { operations: [["updatePlan", { steps: promoted }]] }
              : {
                  text: "The file was created but its readback was not allowed.",
                },
          { operations: [["inspect_fixture", { item: "verified-read" }]] },
          {
            operations: [
              [
                "updatePlan",
                {
                  steps: promoted.map(({ step }) => ({
                    step,
                    status: "completed",
                  })),
                },
              ],
            ],
          },
          { text: "The corrected file was verified." },
        ],
        protocol,
        {
          thinkingLevel: "high",
          model: {
            id:
              protocol === "anthropic"
                ? "anthropic/claude-haiku-5.5"
                : "deepseek/deepseek-v4.1-flash-1017",
            supportedEndpoints: [
              protocol === "completions" ? "openai" : protocol,
            ],
            contextWindow: 32000,
            maxOutputTokens: 4096,
            reasoning: true,
            thinkingLevelMap: {
              off: "none",
              low: "low",
              medium: "medium",
              high: "high",
            },
            ...(protocol === "anthropic"
              ? {
                  anthropicCompat: {
                    forceAdaptiveThinking: true,
                    supportsTemperature: false,
                  },
                }
              : {}),
          },
        },
      );
      const result = await f.run();
      assert.equal(result.status, "completed", result.errorMessage);
      assert.deepEqual(
        f.calls.map(({ args }) => args.item),
        ["created-file", "verified-read"],
      );
      assert.equal(
        f.requests[2].tool_choice,
        undefined,
        "One completed operation may still finish directly",
      );
      assert.deepEqual(f.requests[3].tool_choice, choice);
      assert.deepEqual(names(f.requests[3]), ["updatePlan"]);
      assert.equal(
        f.requests[4].tool_choice,
        undefined,
        "The accepted plan ends forced planning before retrying work",
      );
      if (protocol === "responses") {
        assert.equal(f.requests[3].reasoning.effort, "none");
        assert.equal(f.requests[4].reasoning.effort, "high");
      } else {
        assert.deepEqual(f.requests[3].thinking, { type: "disabled" });
        if (protocol === "completions") {
          assert.equal(f.requests[4].thinking, undefined);
          assert.equal(f.requests[4].reasoning_effort, "high");
        } else assert.equal(f.requests[4].thinking.type, "adaptive");
      }
      assert.deepEqual(
        f.events
          .filter((event) => event.type === "task_plan")
          .map((event) => event.data.steps),
        [promoted, promoted.map(({ step }) => ({ step, status: "completed" }))],
      );
    });
});

test("a genuine single operation can finish directly without forced planning across providers", async (t) => {
  for (const protocol of ["completions", "responses", "anthropic"])
    await t.test(protocol, async (t) => {
      const f = await fixture(
        t,
        [
          { operations: [["beginSingleStep", {}]] },
          { operations: [["inspect_fixture", { item: "one" }]] },
          { text: "The one requested operation is complete." },
        ],
        protocol,
      );
      assert.equal((await f.run()).status, "completed");
      assert.equal(f.requests.length, 3);
      assert.equal(f.requests[2].tool_choice, undefined);
      assert.equal(
        f.events.some((event) => event.type === "task_plan"),
        false,
      );
      assert.equal(f.calls.length, 1);
    });
});

test("invalid forced promotion is bounded to two planning requests without losing usage or the saved work", async (t) => {
  for (const protocol of ["completions", "responses", "anthropic"])
    await t.test(protocol, async (t) => {
      const f = await fixture(
        t,
        [
          { operations: [["beginSingleStep", {}]] },
          { operations: [["inspect_fixture", { item: "saved-work" }]] },
          { operations: [["inspect_fixture", { item: "not-executed" }]] },
          ...Array.from({ length: 25 }, () => ({
            operations: [["updatePlan", { steps: [] }]],
          })),
        ],
        protocol,
      );
      const result = await f.run();
      assert.equal(result.status, "error");
      assert.equal(result.errorCode, "assistant_plan_required");
      assert.equal(
        f.requests.filter((request) => request.tool_choice).length,
        2,
      );
      assert.equal(
        f.requests.length,
        5,
        "Only the three pre-promotion requests and two forced requests reach the provider",
      );
      assert.deepEqual(
        f.calls.map(({ args }) => args.item),
        ["saved-work"],
      );
      assert.equal(
        f.events.some((event) => event.type === "task_plan"),
        false,
      );
      assert.equal(
        result.entries.filter((entry) => entry.role === "request").length,
        5,
      );
    });
});

test("the global twenty-call budget terminates further execution and model requests while preserving the unfinished plan", async (t) => {
  const f = await fixture(t, [
    { operations: [["updatePlan", { steps: plan }]] },
    ...Array.from({ length: 25 }, (_, index) => ({
      operations: [["inspect_fixture", { item: `operation-${index}` }]],
    })),
  ]);
  const result = await f.run();
  assert.equal(result.status, "error");
  assert.equal(result.errorCode, "assistant_tool_limit");
  assert.equal(
    f.calls.length,
    19,
    "The opening plan shares the twenty-call budget",
  );
  assert.equal(
    f.requests.length,
    21,
    "The refused twenty-first call terminates before a further model request",
  );
  assert.deepEqual(
    f.events
      .filter((event) => event.type === "task_plan")
      .map((event) => event.data.steps),
    [plan],
  );
});

test("an explicit presentation also stops after two invalid forced plans without a third request", async (t) => {
  const f = await fixture(
    t,
    Array.from({ length: 25 }, () => ({
      operations: [["updatePlan", { steps: [] }]],
    })),
    "completions",
    { documents: { formats: ["pptx"], selectedStyle: "auto" } },
  );
  const result = await f.run();
  assert.equal(result.status, "error");
  assert.equal(result.errorCode, "assistant_invalid_plan");
  assert.equal(f.requests.length, 2);
  assert.equal(
    result.entries.filter((entry) => entry.role === "request").length,
    2,
  );
  assert.deepEqual(f.calls, []);
});

test("applied steering gives the new task its own two-attempt forced planning limit", async (t) => {
  let entered, release;
  const waiting = new Promise((resolve) => {
    entered = resolve;
  });
  const blocked = new Promise((resolve) => {
    release = resolve;
  });
  t.after(() => release());
  const f = await fixture(
    t,
    [
      { operations: [["updatePlan", { steps: plan }]] },
      { operations: [["inspect_fixture", { item: "old-source" }]] },
      {
        text: "The old work is unfinished.",
        wait: () => {
          entered();
          return blocked;
        },
      },
      { operations: [["beginSingleStep", {}]] },
      { operations: [["inspect_fixture", { item: "new-source" }]] },
      { operations: [["inspect_fixture", { item: "blocked-second" }]] },
      { operations: [["updatePlan", { steps: [] }]] },
      { operations: [["updatePlan", { steps: plan }]] },
      { operations: [["inspect_fixture", { item: "new-verified" }]] },
      {
        operations: [
          [
            "updatePlan",
            { steps: plan.map(({ step }) => ({ step, status: "completed" })) },
          ],
        ],
      },
      { text: "The new task is complete." },
    ],
    "completions",
    { documents: { formats: ["pptx"], selectedStyle: "auto" } },
  );
  const running = f.run();
  await waiting;
  await f.runtime.steer({
    ...f.params,
    id: "reset-forced-planning",
    behavior: "steer",
    text: "Inspect and verify the new source.",
  });
  release();
  const result = await running;
  assert.equal(result.status, "completed", result.errorMessage);
  assert.equal(f.requests.filter((request) => request.tool_choice).length, 3);
  assert.deepEqual(
    f.calls.map(({ args }) => args.item),
    ["old-source", "new-source", "new-verified"],
  );
});

test("queued steering cannot clear an exhausted global tool budget or send a twenty-second request", async (t) => {
  let entered, release;
  const waiting = new Promise((resolve) => {
    entered = resolve;
  });
  const blocked = new Promise((resolve) => {
    release = resolve;
  });
  t.after(() => release());
  const f = await fixture(t, [
    { operations: [["updatePlan", { steps: plan }]] },
    ...Array.from({ length: 19 }, (_, index) => ({
      operations: [["inspect_fixture", { item: `operation-${index}` }]],
    })),
    {
      operations: [["inspect_fixture", { item: "over-budget" }]],
      wait: () => {
        entered();
        return blocked;
      },
    },
    { text: "This extra request must never reach the provider." },
  ]);
  const running = f.run();
  await waiting;
  await f.runtime.steer({
    ...f.params,
    id: "steered-after-budget",
    behavior: "steer",
    text: "Which option is hosted?",
  });
  release();
  const result = await running;
  assert.equal(result.status, "error");
  assert.equal(result.errorCode, "assistant_tool_limit");
  assert.equal(f.calls.length, 19);
  assert.equal(f.requests.length, 21);
  assert.equal(
    f.events.filter((event) => event.type === "steer_applied").length,
    1,
  );
});

test("terminated planning results never acquire a successful typed receipt", async () => {
  const runtime = new AssistantRuntime({});
  const context = {
    params: { sessionId: "limit", runId: "limit", operationId: "limit" },
    calls: new Map(),
    callCount: 0,
    unknownOutcome: false,
  };
  try {
    for (const name of ["updatePlan", "beginSingleStep"])
      for (const isError of [true, undefined]) {
        const failure = {
          terminate: true,
          ...(isError ? { isError } : {}),
          content: [{ type: "text", text: "assistant_tool_limit" }],
          details:
            name === "updatePlan"
              ? { steps: plan }
              : { mode: "single_step", remainingOperations: 1 },
        };
        const safe = scrubTaskPlanStructures(
          { role: "toolResult", toolName: name, ...failure },
          [],
        );
        assert.deepEqual(safe.content, failure.content);
        const wrapped = runtime.wrapTool(
          context,
          name,
          "Synthetic refused preparation",
          Type.Object({}),
          async () => failure,
        );
        const result = await wrapped.execute(
          `${name}-${isError}`,
          name === "updatePlan" ? { steps: plan } : {},
        );
        assert.equal(result.terminate, true);
        assert.deepEqual(result.content, failure.content);
      }
  } finally {
    await runtime.close();
  }
});
