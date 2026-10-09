import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { OFOX_APEX_CHANGED_EVENT, ofoxGetApexState } from "@/lib/api/ofoxApex";
import { DEFAULT_OFOX_APEX, type OfoxApex } from "@/lib/ofoxUrls";

/**
 * Subscribes to OFox apex (region) state.
 *
 * Behavior:
 *   1. On mount, fetches `ofox_get_apex_state` so URL composers don't flicker
 *      between default and resolved value; `pinned` says whether the user
 *      chose the region by hand (otherwise every launch re-probes the network).
 *   2. Listens for `ofox-apex-changed` events emitted by the backend after
 *      a successful `ofox_set_apex` (manual switch) or first-launch ip-api
 *      probe — pulling fresh state with one round-trip; the event's payload
 *      is the new apex string.
 *   3. Exposes `refetch()` for code that needs to force a re-read (rare —
 *      the event covers most cases).
 *
 * Until the first read resolves the hook returns `DEFAULT_OFOX_APEX`. This
 * matches the backend `current_apex()` fallback so any preset URLs computed
 * during the first paint don't disagree with the eventual resolved value.
 */
export function useOfoxApex() {
  const [apex, setApex] = useState<OfoxApex>(DEFAULT_OFOX_APEX);
  const [pinned, setPinned] = useState(false);
  const [isLoading, setIsLoading] = useState(true);

  const refetch = useCallback(async () => {
    try {
      const next = await ofoxGetApexState();
      setApex(next.apex);
      setPinned(next.pinned);
    } catch (e) {
      // ofox_get_apex_state 不应该失败（它内部不做 IO，只是读 settings_store）。
      // 真出错了维持当前 state，下次事件会再矫正。
      console.error("[useOfoxApex] ofoxGetApexState failed", e);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void refetch();
  }, [refetch]);

  useEffect(() => {
    const unlisten = listen<string>(OFOX_APEX_CHANGED_EVENT, (evt) => {
      // 后端 emit 直接附带新值——比再走一次 invoke 节省 IPC，且避免事件与
      // 主动 refetch 的竞态（事件先到 → setApex；refetch 后到 → 拿到的是
      // 同一个值，no-op）。
      const next = evt.payload;
      if (next === "ofox.ai" || next === "ofox.io") {
        setApex(next);
      }
      // 事件里没有 pinned：再读一次拿权威值（也兜住未知 payload）。
      void refetch();
    });
    return () => {
      void unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [refetch]);

  return { apex, pinned, isLoading, refetch };
}
