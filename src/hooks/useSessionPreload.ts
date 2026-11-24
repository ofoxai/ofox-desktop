import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { sessionListOptions } from "@/lib/query/sessionQueries";
import { useOfoxApex } from "@/hooks/useOfoxApex";

/** Warm only the list: keep the reader, search index and page unmounted. */
export function useSessionPreload(active: boolean) {
  const client = useQueryClient();
  const { apex, isLoading } = useOfoxApex();
  useEffect(() => {
    if (!active || isLoading) return;
    const preload = () => {
      void client.prefetchQuery({
        ...sessionListOptions(apex),
        staleTime: 30_000,
        // A first visit may be much later than the default five-minute GC.
        // SessionsPage still refreshes stale data in the background on mount.
        gcTime: Infinity,
      });
    };
    if (typeof window.requestIdleCallback === "function") {
      const idle = window.requestIdleCallback(preload, { timeout: 1500 });
      return () => window.cancelIdleCallback(idle);
    }
    const timer = window.setTimeout(preload, 200);
    return () => window.clearTimeout(timer);
  }, [active, apex, isLoading, client]);
}
