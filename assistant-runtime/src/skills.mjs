import { Type } from "typebox";

const validName = (name) =>
  typeof name === "string" &&
  name.length > 0 &&
  name.length <= 128 &&
  !name.includes("..") &&
  !/[\s/\\<>:"|?*\u0000-\u001f]/u.test(name);
const cut = (text, max) => [...text].slice(0, max).join("");

// Only host results replace this index. Tool arguments never supply metadata.
export function replaceSkillIndex(context, { skills, revision = 0 }) {
  if (
    !Number.isSafeInteger(revision) ||
    revision < 0 ||
    revision <= (context.skillRevision ?? -1)
  )
    return false;
  const index = new Map();
  const duplicates = new Set();
  for (const skill of Array.isArray(skills) ? skills : []) {
    if (
      !skill ||
      !validName(skill.name) ||
      skill.broken ||
      !["auto", "manual"].includes(skill.mode) ||
      typeof skill.description !== "string" ||
      typeof skill.dir !== "string"
    )
      continue;
    if (index.has(skill.name)) duplicates.add(skill.name);
    index.set(skill.name, { ...skill });
  }
  for (const name of duplicates) index.delete(name);
  context.skillIndex = index;
  context.skillRevision = revision;
  return true;
}
function index(context) {
  if (!context.skillIndex)
    replaceSkillIndex(context, { skills: context.params.skills });
  return context.skillIndex;
}
export function skillPrompt(context) {
  if (context.params.model?.tools === false) return "";
  const skills = [...index(context).values()].filter((s) => s.mode === "auto");
  if (!skills.length) return "";
  const lines = [
    "Available skills",
    "Call use_skill when a skill is useful; use its dir with the existing read and command tools for references and scripts.",
  ];
  let used = lines.join("\n").length;
  let listed = 0;
  for (const skill of skills) {
    const line = `- ${skill.name}: ${cut(skill.description.replace(/\s+/gu, " ").trim(), 200)}`;
    const remaining = skills.length - listed - 1;
    const tail = remaining
      ? `\n(${remaining} more skills not listed; the user can pick them with /)`
      : "";
    if (used + 1 + [...line].length + tail.length > 8000) break;
    lines.push(line);
    used += 1 + [...line].length;
    listed++;
  }
  if (listed < skills.length)
    lines.push(
      `(${skills.length - listed} more skills not listed; the user can pick them with /)`,
    );
  return lines.join("\n");
}
export function skillTools(runtime, context) {
  index(context);
  return [
    runtime.wrapTool(
      context,
      "use_skill",
      "Load an automatic skill's instructions and relative file list. Use the returned dir with the existing read and command tools to read references or run scripts; skill files cannot be edited with generic tools. " +
        (context.params.localConfig
          ? "Scripts use normal command approvals; commands are not OS-sandboxed."
          : "Local tools are unavailable: scripts cannot run here."),
      Type.Object(
        { name: Type.String({ minLength: 1, maxLength: 128 }) },
        { additionalProperties: false },
      ),
      async ({ name }, signal, id) => {
        const skill = index(context).get(name);
        if (!validName(name) || !skill || skill.mode !== "auto")
          throw new Error(
            "not_available: only an available automatic skill can be loaded",
          );
        const result = await runtime.hostTool(
          context,
          "use_skill",
          { name },
          signal,
          id,
        );
        return {
          ...result,
          body: cut(result.body, 60000),
          files: result.files.slice(0, 200),
          truncated:
            result.truncated ||
            [...result.body].length > 60000 ||
            result.files.length > 200,
          scriptsUnavailable: !context.params.localConfig,
        };
      },
    ),
    ...[
      [
        "find_skills",
        "Find skills from the catalog and skills.sh. Results are read-only; only their current resultId can be requested for installation.",
        Type.Object(
          { query: Type.String({ minLength: 2, maxLength: 512 }) },
          { additionalProperties: false },
        ),
      ],
      [
        "install_skill",
        "Ask the user to install a result from the latest search. Every installation requires clicking Install, including full auto. Never choose a different source for an ambiguous result.",
        Type.Object(
          { resultId: Type.String({ minLength: 1, maxLength: 200 }) },
          { additionalProperties: false },
        ),
      ],
      [
        "save_skill",
        "Ask the user to save a reusable skill. Always requires clicking Save. Existing skills can only be replaced when they were created by this assistant and remain unchanged.",
        Type.Object(
          {
            name: Type.String({
              pattern: "^[a-z0-9]+(?:-[a-z0-9]+)*$",
              maxLength: 64,
            }),
            description: Type.String({ minLength: 1, maxLength: 500 }),
            body: Type.String({ maxLength: 60000 }),
          },
          { additionalProperties: false },
        ),
      ],
    ].map(([name, description, schema]) =>
      runtime.wrapTool(
        context,
        name,
        description,
        schema,
        async (args, signal, id) => {
          const result = await runtime.hostTool(
            context,
            name,
            args,
            signal,
            id,
          );
          if (result.refresh) {
            const changed = replaceSkillIndex(context, result.refresh);
            result.usable = Boolean(
              changed && context.skillIndex.get(result.name)?.mode === "auto",
            );
          }
          const { refresh, ...visible } = result;
          return visible;
        },
      ),
    ),
  ];
}
