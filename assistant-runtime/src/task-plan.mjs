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
      tool.name === "updatePlan"
        ? {
            ...scrubSecrets(tool, secrets),
            name: "updatePlan",
            parameters: parameters(),
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
      source.toolName === "updatePlan" &&
      !source.isError
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
        runtime.notify(context, "task_plan", { steps });
        return taskPlanResult(steps, [...runtime.secrets]);
      },
    ),
    promptGuidelines: [
      "Use updatePlan for meaningful multi-stage tasks, usually 3–6 concise steps in the user's language. Skip planning for greetings, simple translations, and short questions.",
      "Create the plan before substantive execution. Batch updates when stages change and before finishing; avoid unchanged updates. Keep at most one step in progress, and mark a step completed only after doing its work. A tool returning does not by itself complete a task step.",
    ],
  };
}
