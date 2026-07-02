export const TOOL_META: Record<
  string,
  {
    abbr: string;
    label: string;
    color: string;
    /**
     * 主页"打开"按钮启动的 CLI 命令。留空表示该工具不支持一键打开
     * （例如 hermes 是 dashboard 服务而不是交互式 CLI，openclaw 走
     * gateway，不需要在终端里跑）。
     */
    cliBin?: string;
  }
> = {
  claude: {
    abbr: "CC",
    label: "Claude Code",
    color: "bg-orange-700",
    cliBin: "claude",
  },
  codex: {
    abbr: "Cx",
    label: "Codex",
    color: "bg-neutral-800",
    cliBin: "codex",
  },
  opencode: {
    abbr: "OC",
    label: "OpenCode",
    color: "bg-emerald-500",
    cliBin: "opencode",
  },
  gemini: {
    abbr: "Ge",
    label: "Gemini",
    color: "bg-blue-500",
    cliBin: "gemini",
  },
  openclaw: { abbr: "OCl", label: "OpenClaw", color: "bg-amber-600" },
  hermes: { abbr: "He", label: "Hermes", color: "bg-violet-600" },
};

// Single source of truth for which tools the Ofox UI exposes. Aider and Zed
// were dropped from the bind/manage flows because cc-switch can't proxy
// them today — leaving them in the picker would let users "bind" something
// that quietly does nothing. The backend `AppType` enum still includes
// every kind so old DB rows / migrations stay readable.
// OpenClaw / Hermes 都走 ofox 字段级 patch bind（同 OpenCode），不依赖
// proxy，所以包含进 picker。
export const TOOL_ORDER = [
  "claude",
  "codex",
  "opencode",
  "openclaw",
  "hermes",
  "gemini",
];

export const BOUND_TOOLS_STORAGE_KEY = "ofox-bound-tools";

export const PROXY_SUPPORTED_TOOLS = ["claude", "codex", "gemini"];

// 哪些工具支持从 cc-switch UI 一键安装。当前 6 个都在 scripts/installer/app/
// steps.py 的 TOOL_STEPS 注册表里——保持两边同步即可。Windows / Linux 实现
// 落地后在此处按平台收紧。
export const INSTALLABLE_TOOLS: readonly string[] = [
  "claude",
  "codex",
  "gemini",
  "opencode",
  "openclaw",
  "hermes",
];
