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
  supportsDefault: boolean;
  canWrite?: boolean;
  scope: string[];
  scopeModels?: { scope: string; modelId: string }[];
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
};
