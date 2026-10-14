import { TOOL_META, TOOL_ORDER } from "./toolMeta";

export interface InstallCatalogEntry {
  toolId: string;
  label: string;
  officialUrl: string;
  command?: string;
  platformVerified: boolean;
  dependency?: "node22" | "node" | "bun" | "python" | "platform";
}

/** Installation choices are deliberately separate from the OFox bind list. */
export const INSTALL_CATALOG: readonly InstallCatalogEntry[] = [
  ...TOOL_ORDER.map((toolId) => ({
    toolId,
    label: TOOL_META[toolId].label,
    officialUrl: TOOL_META[toolId].downloadUrl ?? TOOL_META[toolId].projectUrl,
    platformVerified: true,
  })),
  {
    toolId: "copilot",
    label: "GitHub Copilot CLI",
    officialUrl:
      "https://docs.github.com/en/copilot/get-started/cli-quickstart",
    command: "npm install -g @github/copilot",
    platformVerified: true,
    dependency: "node22",
  },
  {
    toolId: "cline",
    label: "Cline CLI",
    officialUrl: "https://github.com/cline/cline/blob/main/apps/cli/README.md",
    command: "npm install -g cline",
    platformVerified: true,
    dependency: "node",
  },
  {
    toolId: "crush",
    label: "Crush",
    officialUrl: "https://github.com/charmbracelet/crush",
    command: "npm install -g @charmland/crush",
    platformVerified: true,
    dependency: "node",
  },
  {
    toolId: "omp",
    label: "oh-my-pi (omp)",
    officialUrl: "https://github.com/can1357/oh-my-pi",
    command: "bun install -g @oh-my-pi/pi-coding-agent",
    platformVerified: true,
    dependency: "bun",
  },
  {
    toolId: "codebuddy",
    label: "CodeBuddy Code CLI",
    officialUrl: "https://www.codebuddy.ai/docs/cli/installation",
    command: "npm install -g @tencent-ai/codebuddy-code",
    platformVerified: true,
    dependency: "node",
  },
  {
    toolId: "deveco-code",
    label: "DevEco Code",
    officialUrl: "https://developer.huawei.com/consumer/cn/deveco-studio/",
    command: "npm install -g @deveco/deveco-code",
    platformVerified: false,
    dependency: "platform",
  },
  {
    toolId: "deveco-cli",
    label: "DevEco CLI",
    officialUrl: "https://developer.huawei.com/consumer/cn/deveco-studio/",
    command: "npm install -g @deveco/deveco-cli",
    platformVerified: false,
    dependency: "platform",
  },
  {
    toolId: "aider",
    label: "Aider",
    officialUrl: "https://aider.chat/docs/install.html",
    command: "python -m pip install aider-install\naider-install",
    platformVerified: true,
    dependency: "python",
  },
  {
    toolId: "goose-cli",
    label: "Goose CLI",
    officialUrl: "https://github.com/aaif-goose/goose",
    platformVerified: true,
  },
];
