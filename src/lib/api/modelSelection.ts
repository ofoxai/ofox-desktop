import { invoke } from "@tauri-apps/api/core";
import type { ToolBindingStatus } from "@/lib/api/ofoxBind";

export type ModelSelection =
  | { kind: "single"; model: string }
  | { kind: "models"; models: string[] };

export interface ToolModelSnapshot {
  toolId: string;
  writeTarget: string;
  revision: string;
  binding: ToolBindingStatus;
  managedSelection: ModelSelection;
  observedSelection: { model: string | null; source: string };
  canWrite?: boolean;
  scope: string[];
  scopeModels?: { scope: string; modelId: string }[];
  /** Reasoning effort, for a tool Ofox can set it on (Codex, Claude Code, Hermes). */
  effort?: EffortState;
}

/** `value` null is the tool's own default; `levels` weakest first. */
export interface EffortState {
  value: string | null;
  levels: string[];
  /** What the tool uses when none is set, where known. */
  defaultLevel: string | null;
}

export interface ModelSaveReport {
  snapshot: ToolModelSnapshot;
  saved: string[];
  skipped: { id: string; reason: string }[];
  writeRevision?: string | null;
}

export const modelSelectionApi = {
  snapshot: (tools?: string[]) =>
    invoke<{ tools: ToolModelSnapshot[] }>("model_selection_snapshot", {
      tools,
    }),
  save: (tool: string, selection: ModelSelection, expectedRevision: string) =>
    invoke<ModelSaveReport>("model_selection_save", {
      tool,
      selection,
      expectedRevision,
    }),
  /** `effort` null removes it, leaving the level to the tool. */
  saveEffort: (tool: string, effort: string | null, expectedRevision: string) =>
    invoke<ToolModelSnapshot>("model_effort_save", {
      tool,
      effort,
      expectedRevision,
    }),
};
