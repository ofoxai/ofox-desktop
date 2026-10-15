import { invoke } from "@tauri-apps/api/core";

export type CatalogAction = "install" | "update" | "uninstall";
export interface CatalogCopyState {
  toolId: string;
  supported: boolean;
  prefix: string | null;
  state: "missing" | "managed" | "needsReview" | "unavailable";
  version?: string | null;
  nodeVersion?: string | null;
  reason?: string | null;
  actions: CatalogAction[];
  launchCommand?: string | null;
}
export interface CatalogPlan {
  operationId: string;
  toolId: string;
  action: CatalogAction;
  prefix: string;
  version: string;
  nodeVersion: string;
  command: string;
  expiresAtMs: number;
}
export interface CatalogProgress {
  operationId: string;
  stage: "verify" | "execute" | "cleanup" | "inspect";
}
export interface CatalogResult {
  operationId: string;
  state: "ok" | "partial" | "cancelled" | "failed";
  code?: string | null;
  installation: CatalogCopyState;
}
type CatalogActivity = { operationId: string; phase: "start" | "settled" };
const active = new Map<string, number>();
const observers = new Set<(event: CatalogActivity) => void>();
export const isCatalogOperationRunning = () => active.size > 0;
export function observeCatalogOperations(
  observer: (event: CatalogActivity) => void,
) {
  observers.add(observer);
  return () => {
    observers.delete(observer);
  };
}
function notify(event: CatalogActivity) {
  for (const observer of [...observers]) {
    try {
      observer(event);
    } catch {
      /* A view cannot change an operation's result. */
    }
  }
}
/** Only a fixed catalog identity and an action cross IPC; paths are backend-owned. */
export const catalogLifecycleApi = {
  inspect: (toolId: string) =>
    invoke<CatalogCopyState>("get_catalog_copy_state", { toolId }),
  prepare: (toolId: string, action: CatalogAction) =>
    invoke<CatalogPlan>("prepare_catalog_operation", { toolId, action }),
  run: async (operationId: string) => {
    active.set(operationId, (active.get(operationId) ?? 0) + 1);
    notify({ operationId, phase: "start" });
    try {
      return await invoke<CatalogResult>("run_catalog_operation", {
        operationId,
      });
    } finally {
      const remaining = (active.get(operationId) ?? 1) - 1;
      if (remaining) active.set(operationId, remaining);
      else active.delete(operationId);
      notify({ operationId, phase: "settled" });
    }
  },
  cancel: (operationId: string) =>
    invoke<boolean>("cancel_catalog_operation", { operationId }),
};
