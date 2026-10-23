// Native clients and physical targets are deliberately separate.
export const resourceClientIds = Object.freeze([
  "claude",
  "codex",
  "chatgpt",
  "opencode",
  "openclaw",
  "hermes",
  "gemini",
  "workbuddy",
  "dsh",
  "pi",
]);
export const resourceTargetId = (id) => (id === "chatgpt" ? "codex" : id);
export function acceptanceResourceKinds(kinds) {
  const selected = kinds ?? ["instructions", "mcp", "skills"];
  if (
    !Array.isArray(selected) ||
    new Set(selected).size !== selected.length ||
    !selected.includes("mcp") ||
    !selected.includes("skills") ||
    selected.some((kind) => !["instructions", "mcp", "skills"].includes(kind))
  )
    throw new Error("invalidResourceKinds");
  return [...selected].sort();
}
