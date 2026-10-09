import { useEffect, useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  toolUpdatesApi,
  type ToolUpdateInfo,
  type ToolUpdateProgress,
  type ToolUpdateResult,
} from "@/lib/api/toolUpdates";

/** How one tool's update went; `error` is set only when it failed. */
export interface ToolUpdateOutcome {
  tool: string;
  status: ToolUpdateResult["status"] | "failed";
  before: string | null;
  after: string | null;
  error: string | null;
}

interface UpdateState {
  tools: ToolUpdateInfo[];
  checking: boolean;
  busy: string | null;
  batch: boolean;
  /** Tools of the running update, in order; empty when none runs. */
  queue: string[];
  error: string | null;
  logs: Record<string, string[]>;
  results: Record<string, string>;
}

// Shared across settings and console; updates survive closing either view.
let state: UpdateState = {
  tools: [],
  checking: false,
  busy: null,
  batch: false,
  queue: [],
  error: null,
  logs: {},
  results: {},
};
let checkedAt = 0;
let checking: Promise<void> | null = null;
const listeners = new Set<() => void>();
function publish(patch: Partial<UpdateState>) {
  state = { ...state, ...patch };
  listeners.forEach((listener) => listener());
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
function getSnapshot() {
  return state;
}

export function checkToolUpdates(): Promise<void> {
  if (checking) return checking;
  publish({ checking: true, error: null });
  checking = (async () => {
    try {
      const tools = await toolUpdatesApi.check();
      checkedAt = Date.now();
      publish({ tools });
    } catch (error) {
      // A failed request must not leave a stale "current" or "available" badge.
      publish({
        error: String(error),
        tools: state.tools.map((tool) => ({
          ...tool,
          update_status: "failed",
        })),
      });
    } finally {
      checking = null;
      publish({ checking: false });
    }
  })();
  return checking;
}

export async function updateTools(
  names: string[],
): Promise<ToolUpdateOutcome[]> {
  if (state.busy || state.batch || !names.length) return [];
  const eligible = [...new Set(names)].filter((name) =>
    state.tools.some(
      (tool) =>
        tool.name === name &&
        tool.update_supported &&
        (tool.update_status === "available" || tool.update_status === "broken"),
    ),
  );
  if (!eligible.length) return [];
  const outcomes: ToolUpdateOutcome[] = [];
  publish({ batch: true, queue: eligible, results: {} });
  try {
    // Finish an older version scan before starting updates so it cannot overwrite verification.
    if (checking) await checking;
    for (const tool of eligible) {
      const operationId = crypto.randomUUID();
      const before =
        state.tools.find((entry) => entry.name === tool)?.version ?? null;
      publish({ busy: tool, logs: { ...state.logs, [tool]: [] } });
      let unlisten: (() => void) | undefined;
      try {
        unlisten = await listen<ToolUpdateProgress>(
          "tool-update-progress",
          ({ payload }) => {
            if (
              payload.tool !== tool ||
              payload.operationId !== operationId ||
              !payload.detail
            )
              return;
            publish({
              logs: {
                ...state.logs,
                [tool]: [...(state.logs[tool] ?? []), payload.detail].slice(
                  -150,
                ),
              },
            });
          },
        );
        const result = await toolUpdatesApi.update(tool, operationId);
        outcomes.push({
          tool,
          status: result.status,
          before: result.before || before,
          after: result.after || null,
          error: null,
        });
        publish({ results: { ...state.results, [tool]: result.status } });
      } catch (error) {
        outcomes.push({
          tool,
          status: "failed",
          before,
          after: null,
          error: String(error),
        });
        publish({
          results: { ...state.results, [tool]: "failed" },
          logs: {
            ...state.logs,
            [tool]: [...(state.logs[tool] ?? []), String(error)].slice(-150),
          },
        });
      } finally {
        unlisten?.();
      }
    }
    await checkToolUpdates();
  } finally {
    publish({ busy: null, batch: false, queue: [] });
    window.dispatchEvent(new Event("tool-updates-complete"));
  }
  return outcomes;
}

export function useToolUpdates(enabled = true) {
  const snapshot = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  useEffect(() => {
    if (!enabled) return;
    if (Date.now() - checkedAt > 5 * 60_000 && !state.batch)
      void checkToolUpdates();
    // Tools can be upgraded in another terminal or desktop app while this
    // window is in the background. The mount cache cannot detect that change.
    const refresh = () => {
      if (!state.batch) void checkToolUpdates();
    };
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [enabled]);
  return snapshot;
}
