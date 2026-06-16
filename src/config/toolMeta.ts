export const TOOL_META: Record<
  string,
  { abbr: string; label: string; color: string }
> = {
  claude: { abbr: "CC", label: "Claude Code", color: "bg-orange-700" },
  codex: { abbr: "Cx", label: "Codex", color: "bg-neutral-800" },
  opencode: { abbr: "OC", label: "OpenCode", color: "bg-emerald-500" },
  gemini: { abbr: "Ge", label: "Gemini", color: "bg-blue-500" },
};

// Single source of truth for which tools the Ofox UI exposes. Aider and Zed
// were dropped from the bind/manage flows because cc-switch can't proxy
// them today — leaving them in the picker would let users "bind" something
// that quietly does nothing. The backend `AppType` enum still includes
// every kind so old DB rows / migrations stay readable.
export const TOOL_ORDER = ["claude", "codex", "opencode", "gemini"];

export const BOUND_TOOLS_STORAGE_KEY = "ofox-bound-tools";

export const PROXY_SUPPORTED_TOOLS = ["claude", "codex", "gemini"];
