import { Type } from "typebox";
import { validatePlan } from "./local-task.mjs";
import { scrubSecrets } from "./security.mjs";

const parameters = () =>
  Type.Object({
    steps: Type.Array(
      Type.Object({
        step: Type.String({ maxLength: 40 }),
        status: Type.Union([
          Type.Literal("pending"),
          Type.Literal("in_progress"),
          Type.Literal("completed"),
        ]),
      }),
      { maxItems: 12 },
    ),
    intent: Type.Optional(Type.String({ maxLength: 200 })),
  });
const singleStepParameters = () => Type.Object({});

export function singleStepResult() {
  const details = { mode: "single_step", remainingOperations: 1 };
  return {
    content: [{ type: "text", text: JSON.stringify(details) }],
    details,
  };
}

function successfulSingleStep(source) {
  const valid = (details) =>
    details &&
    typeof details === "object" &&
    !Array.isArray(details) &&
    Object.keys(details).length === 2 &&
    details.mode === "single_step" &&
    details.remainingOperations === 1;
  if (
    source.isError ||
    source.terminate ||
    source.error ||
    !valid(source.details)
  )
    return false;
  if (
    !Array.isArray(source.content) ||
    source.content.length !== 1 ||
    source.content[0]?.type !== "text"
  )
    return false;
  try {
    return valid(JSON.parse(source.content[0].text));
  } catch {
    return false;
  }
}

/** Validate raw input first; only labels are credential-bearing content. */
export function scrubTaskPlan(steps, secrets) {
  return validatePlan(steps).map(({ step, status }) => {
    let label = step;
    // Plan labels are bounded presentation metadata. Captured short MCP
    // passwords belong here too; ordinary conversation redaction is unchanged.
    for (const secret of secrets ?? [])
      if (typeof secret === "string" && secret.length > 0)
        label = label.split(secret).join("[REDACTED]");
    return { step: [...label].slice(0, 40).join(""), status };
  });
}

export function taskPlanResult(steps, secrets) {
  const safe = { steps: scrubTaskPlan(steps, secrets) };
  return {
    content: [{ type: "text", text: JSON.stringify(safe) }],
    details: safe,
  };
}

/**
 * Preserve only validated, typed updatePlan structures through Pi's message,
 * event and request boundaries. Everything else keeps ordinary redaction.
 * Unknown fields never inherit the enum exemption; results are reconstructed.
 */
export function scrubTaskPlanStructures(value, secrets) {
  const call = (source) => {
    const safe = scrubSecrets(source, secrets);
    if (source?.type === "toolCall" && source.name === "beginSingleStep")
      return { ...safe, name: "beginSingleStep", arguments: {} };
    if (source?.type !== "toolCall" || source.name !== "updatePlan")
      return safe;
    try {
      return {
        ...safe,
        name: "updatePlan",
        arguments: { steps: scrubTaskPlan(source.arguments?.steps, secrets) },
      };
    } catch {
      // Redaction can shorten an over-limit label. Never let invalid raw input
      // become a newly valid call after it crosses a message boundary.
      return { ...safe, name: "updatePlan", arguments: { steps: null } };
    }
  };
  const definitions = (tools) =>
    tools.map((tool) =>
      tool.name === "updatePlan" || tool.name === "beginSingleStep"
        ? {
            ...scrubSecrets(tool, secrets),
            name: tool.name,
            parameters:
              tool.name === "updatePlan"
                ? parameters()
                : singleStepParameters(),
          }
        : scrubSecrets(tool, secrets),
    );
  const message = (source) => {
    const safe = scrubSecrets(source, secrets);
    if (source?.role === "assistant" && Array.isArray(source.content))
      return { ...safe, content: source.content.map(call) };
    if (source?.role === "system" && Array.isArray(source.toolsAdded))
      return { ...safe, toolsAdded: definitions(source.toolsAdded) };
    if (
      (source?.role === "toolResult" || source?.type === "tool_result") &&
      source.toolName === "beginSingleStep" &&
      successfulSingleStep(source)
    )
      return { ...safe, ...singleStepResult(), toolName: "beginSingleStep" };
    if (
      (source?.role === "toolResult" || source?.type === "tool_result") &&
      source.toolName === "updatePlan" &&
      !source.isError &&
      !source.terminate &&
      !source.error
    ) {
      try {
        return {
          ...safe,
          ...taskPlanResult(source.details?.steps, secrets),
          toolName: "updatePlan",
        };
      } catch {
        return safe;
      }
    }
    return safe;
  };
  if (Array.isArray(value)) return value.map(message);
  const safe = message(value);
  if (
    !value ||
    typeof value !== "object" ||
    value.role ||
    value.type === "tool_result"
  )
    return safe;
  // Only the SDK's known message containers are traversed, never arbitrary
  // arguments, resource results, or nested user-controlled metadata.
  if (Array.isArray(value.messages))
    return {
      ...safe,
      messages: value.messages.map(message),
      ...(Array.isArray(value.tools)
        ? { tools: definitions(value.tools) }
        : {}),
    };
  if (typeof value.type === "string") {
    const result = { ...safe };
    for (const key of ["partial", "message", "error"])
      if (value[key]?.role === "assistant") result[key] = message(value[key]);
    if (value.type === "toolcall_end") result.toolCall = call(value.toolCall);
    return result;
  }
  return safe;
}

/** Presentation only: no local capability, host callback or inferred effects. */
export function taskPlanTool(runtime, context) {
  return {
    ...runtime.wrapTool(
      context,
      "updatePlan",
      "Report a concise task plan, at most 12 steps and one in progress. This only updates the visible checklist; it does not execute or authorize operations.",
      parameters(),
      async (args) => {
        const steps = validatePlan(args.steps);
        const delivery = steps.at(-1);
        if (
          context.pptx?.expected &&
          !context.pptx.delivered &&
          delivery?.status === "completed"
        )
          delivery.status = "pending";
        if (
          (context.planning?.enabled || context.pptx?.needsInitialPlan) &&
          (steps.length < 2 ||
            (context.planning?.mode !== "planned" &&
              steps.every((step) => step.status === "completed")))
        )
          throw new Error(
            "assistant_invalid_plan: Provide at least two meaningful steps with unfinished work before execution",
          );
        if (context.planning?.enabled) context.planning.mode = "planned";
        runtime.notify(context, "task_plan", { steps });
        if (context.pptx && steps.length) context.pptx.needsInitialPlan = false;
        return taskPlanResult(steps, [...runtime.secrets]);
      },
    ),
    promptGuidelines: [
      "Use updatePlan only for meaningful multi-stage tasks, usually 3–6 concise steps in the user's language. Skip planning for greetings, simple translations, short questions, and single-step requests.",
      "For a task requiring multiple operations, publish at least two meaningful steps before execution. For exactly one operation, call beginSingleStep with no arguments instead of inventing a checklist. Simple answers need neither tool. Task preparation only exposes the capabilities already enabled for this request and never grants permissions.",
      "Create the plan before substantive execution. Batch updates when stages change and before finishing; avoid unchanged updates. Keep at most one step in progress, and mark a step completed only after doing its work. A tool returning does not by itself complete a task step.",
      ...(context.params.documents?.formats?.includes("pptx")
        ? [
            "End a PPTX checklist with a delivery step. For a PPTX task, mark delivery complete only after a successful create_document result confirms the final PPTX file was kept as an attachment. This includes delivery with fromPreview. Treat a preview or a promise as progress toward delivery.",
          ]
        : []),
    ],
  };
}

export function singleStepTool(runtime, context) {
  return runtime.wrapTool(
    context,
    "beginSingleStep",
    "Prepare exactly one operation when the user's task is genuinely single-step. Call with no arguments, then perform the enabled operation in the next response. Do not use for multi-stage work; call updatePlan first instead. Simple conversational answers need no preparation tool. This grants no permissions and cannot renew an operation allowance.",
    singleStepParameters(),
    async () => {
      const planning = context.planning;
      if (!planning?.enabled || planning.mode !== "undecided")
        throw new Error(
          "assistant_plan_required: Single-step preparation cannot be renewed; publish a multi-stage plan before further operations",
        );
      if (context.pptx?.needsInitialPlan)
        throw new Error(
          "assistant_plan_required: This presentation needs a multi-stage plan",
        );
      planning.mode = "single";
      return singleStepResult();
    },
  );
}
