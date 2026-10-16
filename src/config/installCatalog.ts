import { TOOL_META, TOOL_ORDER } from "./toolMeta";
import { isLinux, isMac, isWindows } from "@/lib/platform";

type CatalogPlatform = "macos" | "linux" | "windows";
type CatalogDependency =
  | "node22"
  | "node2219"
  | "node"
  | "bun"
  | "python"
  | "platform"
  | "homebrew"
  | "shell"
  | "powershell";

interface CatalogInstallCommand {
  command: string;
  dependency?: CatalogDependency;
}

export interface InstallCatalogEntry {
  toolId: string;
  label: string;
  officialUrl: string;
  command?: string;
  platformCommands?: Partial<Record<CatalogPlatform, CatalogInstallCommand>>;
  downloadOnly?: boolean;
  platformVerified: boolean;
  dependency?: CatalogDependency;
}

function nativeInstaller(unix: string, windows: string) {
  return {
    platformCommands: {
      macos: { command: unix, dependency: "shell" },
      linux: { command: unix, dependency: "shell" },
      windows: { command: windows, dependency: "powershell" },
    },
  } satisfies Pick<InstallCatalogEntry, "platformCommands">;
}

/** Copyable manual instructions only; these strings never start an installer. */
const CORE_INSTALL_COMMANDS: Partial<
  Record<
    string,
    Pick<InstallCatalogEntry, "command" | "dependency" | "platformCommands">
  >
> = {
  claude: nativeInstaller(
    "curl -fsSL https://claude.ai/install.sh | bash",
    "irm https://claude.ai/install.ps1 | iex",
  ),
  codex: { command: "npm install -g @openai/codex@latest", dependency: "node" },
  opencode: { command: "npm install -g opencode-ai", dependency: "node" },
  openclaw: nativeInstaller(
    "curl -fsSL https://openclaw.ai/install.sh | bash",
    "iwr -useb https://openclaw.ai/install.ps1 | iex",
  ),
  hermes: nativeInstaller(
    "curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash",
    "iex (irm https://hermes-agent.nousresearch.com/install.ps1)",
  ),
  gemini: { command: "npm install -g @google/gemini-cli", dependency: "node" },
  dsh: { command: "npm install -g @deepseek-ai/dsh", dependency: "node" },
  pi: {
    command: "npm install -g --ignore-scripts @earendil-works/pi-coding-agent",
    dependency: "node2219",
  },
};

export function getCatalogInstallCommand(
  entry: InstallCatalogEntry,
): CatalogInstallCommand | undefined {
  const platform = isWindows()
    ? "windows"
    : isMac()
      ? "macos"
      : isLinux()
        ? "linux"
        : undefined;
  const scoped = platform && entry.platformCommands?.[platform];
  if (scoped) return scoped;
  if (entry.command)
    return { command: entry.command, dependency: entry.dependency };
  return undefined;
}

/** Installation choices are deliberately separate from the OFox bind list. */
export const INSTALL_CATALOG: readonly InstallCatalogEntry[] = [
  ...TOOL_ORDER.map((toolId) => ({
    toolId,
    label: TOOL_META[toolId].label,
    officialUrl: TOOL_META[toolId].downloadUrl ?? TOOL_META[toolId].projectUrl,
    downloadOnly: TOOL_META[toolId].launchKind === "desktopApp",
    ...CORE_INSTALL_COMMANDS[toolId],
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
    platformCommands: {
      macos: {
        command: "brew install block-goose-cli",
        dependency: "homebrew",
      },
      linux: {
        command:
          "curl -fsSL https://github.com/aaif-goose/goose/releases/download/stable/download_cli.sh | bash",
        dependency: "shell",
      },
      windows: {
        command:
          'Invoke-WebRequest -Uri "https://raw.githubusercontent.com/aaif-goose/goose/main/download_cli.ps1" -OutFile "download_cli.ps1"\n.\\download_cli.ps1',
        dependency: "powershell",
      },
    },
    platformVerified: true,
  },
];
