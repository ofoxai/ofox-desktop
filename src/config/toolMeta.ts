export const TOOL_META: Record<
  string,
  { abbr: string; label: string; color: string }
> = {
  claude: { abbr: "CC", label: "Claude Code", color: "bg-orange-700" },
  codex: { abbr: "Cu", label: "Cursor", color: "bg-neutral-800" },
  opencode: { abbr: "OC", label: "OpenCode", color: "bg-emerald-500" },
  gemini: { abbr: "Ge", label: "Gemini", color: "bg-blue-500" },
  aider: { abbr: "Ai", label: "Aider", color: "bg-purple-400" },
  zed: { abbr: "Z", label: "Zed", color: "bg-indigo-500" },
};

export const TOOL_ORDER = [
  "claude",
  "codex",
  "opencode",
  "gemini",
  "aider",
  "zed",
];

export const BOUND_TOOLS_STORAGE_KEY = "ofox-bound-tools";

export const PROXY_SUPPORTED_TOOLS = ["claude", "codex", "gemini"];
