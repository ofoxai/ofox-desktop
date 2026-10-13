import { invoke } from "@tauri-apps/api/core";
import type { ModelSelection } from "@/lib/api/modelSelection";

export interface ProfileTarget {
  target: string;
  selection: ModelSelection;
}
export interface ModelProfile {
  id: string;
  name: string;
  revision: string;
  targets: ProfileTarget[];
  match: "inUse" | "partial" | "different" | "unavailable";
}
export interface ProfilePreview {
  planToken: string;
  canApply: boolean;
  items: {
    target: string;
    before: ModelSelection | null;
    after: ModelSelection | null;
    status: "ready" | "unchanged" | "skipped" | "blocked";
    scope?: string[];
    reason?: string | null;
  }[];
}
export interface ProfileApplyReport {
  status: "applied" | "partial" | "failed";
  items: {
    target: string;
    outcome: "applied" | "unchanged" | "skipped" | "failed";
    reason?: string | null;
  }[];
  undoToken?: string | null;
}
export const modelProfilesApi = {
  list: () => invoke<ModelProfile[]>("model_profiles_list"),
  save: (input: {
    id?: string;
    name: string;
    targets: ProfileTarget[];
    expectedRevision?: string;
  }) => invoke<ModelProfile>("model_profiles_save", input),
  delete: (id: string, expectedRevision: string) =>
    invoke<void>("model_profiles_delete", { id, expectedRevision }),
  preview: (id: string) =>
    invoke<ProfilePreview>("model_profiles_preview", { id }),
  apply: (planToken: string) =>
    invoke<ProfileApplyReport>("model_profiles_apply", { planToken }),
  undo: (undoToken: string) =>
    invoke<ProfileApplyReport>("model_profiles_undo", { undoToken }),
};
