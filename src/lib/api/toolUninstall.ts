import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { UnbindReport } from "@/lib/api/ofoxBind";

/** How the program was installed; `unknown` means it has to be removed by hand. */
export type UninstallSource =
  | "homebrew"
  | "npm"
  | "pnpm"
  | "bun"
  | "native"
  | "app"
  | "unknown";

/** What happens to the tool's Ofox key: revoked, kept for another tool sharing it, or none recorded. */
export type UninstallKeyPlan = "revoke" | "keepShared" | "none";

/** Why the uninstall cannot be started from here. */
export type UninstallBlockedReason =
  | "unknownSource"
  | "appRunning"
  | "busy"
  | "unsupportedPlatform";

/** Dry run: nothing was changed to produce it. */
export interface UninstallPreview {
  tool: string;
  installed: boolean;
  source: UninstallSource;
  /** The command that will run; null when files are removed or an app is trashed instead. */
  command: string | null;
  /** Paths or packages that go away. */
  removes: string[];
  /** The agent's own data, which is never touched. */
  keeps: string[];
  /** What to run in a terminal when it cannot be done here. */
  manualCommand: string | null;
  bound: boolean;
  /** Other tools sharing this tool's configuration and key (Codex and ChatGPT). */
  sharedWith: string[];
  key: UninstallKeyPlan;
  blockedReason: UninstallBlockedReason | null;
}

/** `queued`: the server could not be reached; the key is deleted at the next sign-in. */
export type UninstallKeyOutcome = "revoked" | "queued" | "keptShared" | "none";

export interface UninstallResult {
  unbind: UnbindReport | null;
  key: UninstallKeyOutcome;
  /** `stillPresent`: the program was found again after the removal; see `remainingPath`. */
  status: "removed" | "stillPresent";
  remainingPath: string | null;
  kept: string[];
}

/** `step` of `totalSteps`: 1 restore the configuration, 2 revoke the key, 3 remove the program. */
export interface UninstallProgress {
  operationId: string;
  line: string;
  step: number;
  totalSteps: number;
}

/**
 * `stillBound` is the renderer's own list of bound tools (`ofox-bound-tools`,
 * the real one). The backend only keeps a best-effort mirror, so it merges
 * this list in to see that Codex and ChatGPT share a key and a configuration.
 * Passing every bound tool, this one included, is fine.
 */
export const toolUninstallApi = {
  preview: (tool: string, stillBound: string[] = []) =>
    invoke<UninstallPreview>("uninstall_tool_preview", { tool, stillBound }),
  /**
   * Unbinds the tool, revokes its Ofox key and removes the program by how it was
   * installed. A failed unbind rejects before anything else is done.
   */
  uninstall: (tool: string, operationId: string, stillBound: string[] = []) =>
    invoke<UninstallResult>("uninstall_tool", {
      tool,
      operationId,
      stillBound,
    }),
  onProgress: (
    operationId: string,
    handler: (progress: UninstallProgress) => void,
  ): Promise<UnlistenFn> =>
    listen<UninstallProgress>("tool-uninstall-progress", (event) => {
      if (event.payload.operationId === operationId) handler(event.payload);
    }),
};
