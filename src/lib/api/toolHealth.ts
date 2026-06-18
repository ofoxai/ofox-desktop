import { invoke } from "@tauri-apps/api/core";

/**
 * Tool-health probe API.
 *
 * The backend runs a periodic loop (`services::tool_health::run_health_check_round`)
 * that probes every bound tool with a `max_tokens=1` completion and stores the
 * result in `AppState.tool_health_cache`. Two surface points:
 *
 *   - `getSnapshot()` — UI mounts read this so pills render before the next
 *     event arrives.
 *   - `triggerNow()` — manual "check now" hook (also used in DevTools for
 *     quick validation).
 *
 * Subscribers should also `listen(OFOX_TOOL_HEALTH_UPDATED_EVENT)` to react
 * to background updates.
 */

export type ToolHealthStatus = "ok" | "fail" | "skipped";

export interface ToolHealthSnapshot {
  toolId: string;
  status: ToolHealthStatus;
  /** Null when status is "skipped" — no HTTP request was made. */
  latencyMs: number | null;
  /** Unix-ms. */
  checkedAt: number;
  statusCode: number | null;
  error: string | null;
}

export type ToolHealthMap = Record<string, ToolHealthSnapshot>;

/** Tauri event broadcast every time the cache is updated (including clears). */
export const OFOX_TOOL_HEALTH_UPDATED_EVENT = "ofox-tool-health-updated";

export const toolHealthApi = {
  async getSnapshot(): Promise<ToolHealthMap> {
    return await invoke("get_tool_health_snapshot");
  },

  async triggerNow(): Promise<ToolHealthMap> {
    return await invoke("trigger_tool_health_check_now");
  },
};
