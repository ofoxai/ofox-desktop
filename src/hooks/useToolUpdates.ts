import { useEffect, useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  toolUpdatesApi,
  type ToolUpdateInfo,
  type ToolUpdateProgress,
} from "@/lib/api/toolUpdates";

interface UpdateState {
  tools: ToolUpdateInfo[];
  checking: boolean;
  busy: string | null;
  batch: boolean;
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

export async function updateTools(names: string[]) {
  if (state.busy || state.batch || !names.length) return;
  const eligible = [...new Set(names)].filter((name) =>
    state.tools.some(
      (tool) =>
        tool.name === name &&
        tool.update_supported &&
        tool.update_status === "available",
    ),
  );
  if (!eligible.length) return;
  publish({ batch: true, results: {} });
  try {
    // Finish an older version scan before starting updates so it cannot overwrite verification.
    if (checking) await checking;
    for (const tool of eligible) {
      const operationId = crypto.randomUUID();
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
        publish({ results: { ...state.results, [tool]: result.status } });
      } catch (error) {
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
    publish({ busy: null, batch: false });
    window.dispatchEvent(new Event("tool-updates-complete"));
  }
}

export function useToolUpdates(enabled = true) {
  const snapshot = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  useEffect(() => {
    if (enabled && Date.now() - checkedAt > 5 * 60_000 && !state.batch)
      void checkToolUpdates();
  }, [enabled]);
  return snapshot;
}
