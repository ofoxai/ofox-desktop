import { Type } from "typebox";

const LOCAL_TARGETS = new Set([
  "claude",
  "codex",
  "gemini",
  "opencode",
  "openclaw",
  "hermes",
  "workbuddy",
  "dsh",
  "pi",
]);

export function activeLocalConfig(config) {
  if (
    !LOCAL_TARGETS.has(config?.task?.tool) ||
    !["diagnose", "configure"].includes(config.task.access) ||
    typeof config.cwd !== "string" ||
    !config.cwd ||
    !config.definition?.limits
  )
    return null;
  return config;
}

const textResult = (value) => ({
  content: [
    {
      type: "text",
      text: typeof value === "string" ? value : JSON.stringify(value),
    },
  ],
  details: value,
});

export function selectedSkillRead(runtime, context) {
  return runtime.wrapTool(
    context,
    "read",
    "Read a reference within the user-selected skill. Use its absolute path.",
    Type.Object(
      { path: Type.String({ minLength: 1 }) },
      { additionalProperties: false },
    ),
    async ({ path }, signal, id) =>
      textResult(await runtime.hostTool(context, "read", { path }, signal, id)),
  );
}

export function localTools(runtime, context) {
  const { cwd, task } = context.localConfig;
  // The Host chose the target and folder for this turn. Model arguments
  // cannot widen either, and the Host independently enforces both.
  const direct = (name, description, { folder = false } = {}) =>
    runtime.wrapTool(
      context,
      name,
      description,
      Type.Object(
        {
          tool: Type.Literal(task.tool),
          intent: Type.Optional(Type.String({ maxLength: 200 })),
        },
        { additionalProperties: false },
      ),
      async ({ tool }, signal, id) => {
        if (tool !== task.tool) throw new Error("assistant_operation_denied");
        return textResult(
          await runtime.hostTool(
            context,
            name,
            { tool, ...(folder ? { folder: cwd } : {}) },
            signal,
            id,
          ),
        );
      },
    );

  const tools = [
    direct(
      "tool_status",
      "Inspect this AI tool's local installation and binding without modifying it.",
    ),
    direct(
      "diagnose_in_folder",
      "Diagnose this AI tool's executable resolution and configuration in the authorized folder.",
      { folder: true },
    ),
  ];
  if (task.access === "diagnose") return tools;

  tools.push(
    direct(
      "verify_in_folder",
      "Verify this AI tool after changing its configuration in the authorized folder.",
      { folder: true },
    ),
    direct("repair_launcher", "Repair this AI tool's managed launcher."),
    direct("rebind_tool", "Repair this AI tool's managed OFox binding."),
  );
  return tools;
}
