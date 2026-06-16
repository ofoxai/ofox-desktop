import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  OFOX_AUTH_EXPIRED_EVENT,
  OFOX_AUTH_RESTORED_EVENT,
  ofoxGetAuthStatus,
  type OfoxAuthStatus,
} from "@/lib/api/ofoxAuth";

/**
 * Subscribes to Ofox auth state.
 *
 * Behavior:
 *   1. On mount, fetches `ofox_get_auth_status` so the UI doesn't render a
 *      "loading" flash longer than one tick.
 *   2. Listens for the backend's `ofox-auth-expired` / `ofox-auth-restored`
 *      events and folds the payload directly into local state — no extra
 *      round-trip.
 *   3. Exposes `refetch()` for callers that just performed a state-mutating
 *      action (login complete, manual logout) and want an immediate re-read.
 *
 * Each window (main + tray popover) maintains its own copy via this hook.
 * They stay in sync because the backend emits events globally.
 */
export function useOfoxAuth() {
  const [status, setStatus] = useState<OfoxAuthStatus | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  const refetch = useCallback(async () => {
    try {
      const next = await ofoxGetAuthStatus();
      setStatus(next);
    } catch (e) {
      console.error("[useOfoxAuth] ofoxGetAuthStatus failed", e);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void refetch();
  }, [refetch]);

  useEffect(() => {
    let active = true;
    const unlisteners: Array<() => void> = [];

    const setup = async () => {
      try {
        const offExpired = await listen<OfoxAuthStatus>(
          OFOX_AUTH_EXPIRED_EVENT,
          (event) => {
            if (!active) return;
            setStatus(event.payload);
          },
        );
        const offRestored = await listen<OfoxAuthStatus>(
          OFOX_AUTH_RESTORED_EVENT,
          (event) => {
            if (!active) return;
            setStatus(event.payload);
          },
        );
        if (!active) {
          offExpired();
          offRestored();
          return;
        }
        unlisteners.push(offExpired, offRestored);
      } catch (e) {
        console.error("[useOfoxAuth] failed to subscribe auth events", e);
      }
    };

    void setup();

    return () => {
      active = false;
      unlisteners.forEach((off) => off());
    };
  }, []);

  return {
    status,
    isLoading,
    refetch,
    /** True only for the `active` state — gate sensitive UI on this. */
    isActive: status?.state === "active",
    isExpired: status?.state === "expired",
    isLoggedOut: status?.state === "loggedout",
  };
}
