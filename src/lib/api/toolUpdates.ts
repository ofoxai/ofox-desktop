import { invoke } from "@tauri-apps/api/core";

export type InstallationStatus = "installed" | "notInstalled" | "unknown";

export interface ToolInstallationInfo {
  name: string;
  version: string | null;
  error: string | null;
  installationKind?: "desktopApp" | "cli";
  installationStatus?: InstallationStatus;
}

/** Older responses without installation evidence must never imply uninstall. */
export function getInstallationStatus(
  info: ToolInstallationInfo | undefined,
): InstallationStatus {
  if (info?.installationStatus) return info.installationStatus;
  return info?.version && !info.error ? "installed" : "unknown";
}

export interface ToolUpdateInfo extends ToolInstallationInfo {
  latest_version: string | null;
  installationKind: "desktopApp" | "cli";
  installationStatus: InstallationStatus;
  update_status:
    | "unchecked"
    | "notInstalled"
    | "broken"
    | "failed"
    | "available"
    | "current"
    | "unknown"
    | "appManaged"
    | "unsupported";
  /** homebrew | npm | native | pnpm (CLIs); sparkle | msstore (ChatGPT desktop). */
  update_source: string | null;
  update_supported: boolean;
  update_reason: string | null;
  executable_path: string | null;
}

export interface ToolUpdateResult {
  status: "updated" | "unchanged" | "current" | "repaired";
  before: string;
  after: string;
}

export interface ToolUpdateProgress {
  tool: string;
  operationId: string;
  stage: string;
  detail: string;
}

export const toolUpdatesApi = {
  check: () =>
    invoke<ToolUpdateInfo[]>("get_tool_versions", { includeLatest: true }),
  update: (tool: string, operationId: string) =>
    invoke<ToolUpdateResult>("update_tool", { tool, operationId }),
  /** Desktop apps only: whether upgrading would have to close the app first. */
  isAppRunning: (tool: string) =>
    invoke<boolean>("is_tool_app_running", { tool }),
  /** Opens a desktop app so its built-in updater (e.g. Sparkle) can run. */
  openApp: (tool: string) => invoke<void>("launch_tool", { toolId: tool }),
};
