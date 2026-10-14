import { useQuery } from "@tanstack/react-query";
import { getBalanceTrend } from "@/lib/api/usageOverview";
import { queryClient } from "@/lib/query/queryClient";

/** A minute: the page and the tray read these at most this often by themselves. */
const STALE_MS = 60_000;

export const BALANCE_TREND_KEY = ["balanceTrend"] as const;

/**
 * The OFox balance of the last 14 days and when it runs out, as kept on this
 * computer (no network). Shared by the usage page and the tray.
 */
export function useBalanceTrend(enabled = true) {
  return useQuery(
    {
      queryKey: BALANCE_TREND_KEY,
      queryFn: getBalanceTrend,
      staleTime: STALE_MS,
      retry: false,
      enabled,
    },
    queryClient,
  );
}
