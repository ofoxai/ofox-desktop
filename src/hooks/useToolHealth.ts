import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  toolHealthApi,
  OFOX_TOOL_HEALTH_UPDATED_EVENT,
  type ToolHealthMap,
} from "@/lib/api/toolHealth";

interface UseToolHealthResult {
  /** Per-tool latest probe result. Empty until the loop has run at least once. */
  snapshot: ToolHealthMap;
  /** Re-fetch from the backend cache. Useful for "Refresh" buttons. */
  refetch: () => Promise<void>;
  /** Trigger a fresh round on demand and wait for the result. */
  triggerNow: () => Promise<void>;
  /** Unix-ms of the most recently observed update. `0` until first set. */
  lastUpdatedAt: number;
}

/**
 * Subscribes to backend tool-health updates and exposes the latest snapshot.
 *
 * Lifecycle:
 *   1. mount → `getSnapshot()` to populate immediately (so pills render
 *      without waiting for the next event)
 *   2. `listen(OFOX_TOOL_HEALTH_UPDATED_EVENT)` for subsequent updates
 *   3. unmount → unsubscribe
 *
 * Notes on StrictMode (dev only): React mounts → unmounts → re-mounts. The
 * `cancelled` flag below guards against the unmounted hook updating state.
 * The backend dedupes nothing per-listener, so even if both mounts briefly
 * subscribe at the same time the only cost is one extra event-handler
 * registration which the second cleanup tears down.
 */
export function useToolHealth(): UseToolHealthResult {
  const [snapshot, setSnapshot] = useState<ToolHealthMap>({});
  const [lastUpdatedAt, setLastUpdatedAt] = useState<number>(0);

  // Used by `refetch` to know whether the call should still apply state.
  const mountedRef = useRef(true);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;

    (async () => {
      try {
        const initial = await toolHealthApi.getSnapshot();
        if (!cancelled) {
          setSnapshot(initial);
          // Don't bump lastUpdatedAt for the initial pull — only real
          // backend events should — so a UI showing "X 分钟前" stays empty
          // until the first tick has actually run.
        }
      } catch (e) {
        console.error("[useToolHealth] getSnapshot failed", e);
      }

      try {
        unlisten = await listen<ToolHealthMap>(
          OFOX_TOOL_HEALTH_UPDATED_EVENT,
          (evt) => {
            if (cancelled) return;
            setSnapshot(evt.payload ?? {});
            setLastUpdatedAt(Date.now());
          },
        );
      } catch (e) {
        console.error("[useToolHealth] listen failed", e);
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  useEffect(() => {
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refetch = async () => {
    try {
      const next = await toolHealthApi.getSnapshot();
      if (mountedRef.current) {
        setSnapshot(next);
      }
    } catch (e) {
      console.error("[useToolHealth] refetch failed", e);
    }
  };

  const triggerNow = async () => {
    try {
      const next = await toolHealthApi.triggerNow();
      if (mountedRef.current) {
        setSnapshot(next);
        setLastUpdatedAt(Date.now());
      }
    } catch (e) {
      console.error("[useToolHealth] triggerNow failed", e);
    }
  };

  return { snapshot, refetch, triggerNow, lastUpdatedAt };
}
