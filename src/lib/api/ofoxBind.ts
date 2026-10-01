import { invoke } from "@tauri-apps/api/core";

/** One thing the unbind could not do exactly; `code` maps to `unbind.warning.<code>`. */
export interface UnbindWarning {
  code: string;
  file?: string;
}

/**
 * What 解除绑定 restores (or would restore, for a preview). Paths only — the
 * backend never sends config values or keys.
 */
export interface UnbindReport {
  tool: string;
  dryRun: boolean;
  /** Bound by an older version without a pre-bind snapshot: best-effort cleanup only. */
  legacy: boolean;
  /** Nothing of Ofox's was found; nothing changed. */
  alreadyUnbound: boolean;
  /** Files put back byte-for-byte as they were before binding. */
  exactFiles: string[];
  restoredKeys: string[];
  removedKeys: string[];
  filesRemoved: string[];
  providerRestoredTo: string | null;
  /** Other tools sharing this config (Codex ⇄ ChatGPT) still use Ofox, so it was kept. */
  sharedKeptBy: string[];
  warnings: UnbindWarning[];
}

export const ofoxBindApi = {
  /** Dry run of unbind: reports what would be restored without changing anything. */
  unbindPreview: (app: string, stillBound: string[]) =>
    invoke<UnbindReport>("ofox_unbind_preview", { app, stillBound }),
};
