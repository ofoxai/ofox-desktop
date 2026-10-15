import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import {
  getBalanceTrend,
  getSpending,
  getUsageOverview,
  type UsagePeriod,
} from "@/lib/api/usageOverview";
import { ofoxRefreshUserInfo, type OfoxUserInfo } from "@/lib/api/ofoxAuth";
import { usageApi } from "@/lib/api/usage";
import { queryClient } from "@/lib/query/queryClient";
import { OFOX_USER_KEY } from "@/hooks/useOfoxAccount";
import type { UsageQueryV1 } from "@/types/usageQuery";
const STALE_MS = 60_000;
export const BALANCE_TREND_KEY = ["balanceTrend"] as const;
export function accountIdentity(
  user: OfoxUserInfo | null | undefined,
  apex: string,
) {
  return `${apex}:${user?.balance?.organization_id ?? user?.org_id ?? ""}:${user?.id ?? user?.email ?? user?.name ?? ""}`;
}
export function useBalanceTrend(enabled = true, identity = "") {
  return useQuery(
    {
      queryKey: [...BALANCE_TREND_KEY, identity],
      queryFn: getBalanceTrend,
      staleTime: STALE_MS,
      gcTime: 0,
      retry: false,
      enabled,
    },
    queryClient,
  );
}
export function useUsageOverview(
  period: UsagePeriod,
  user: OfoxUserInfo | null,
  apex: string,
  options: {
    query?: UsageQueryV1;
    enabled?: boolean;
    accountEnabled?: boolean;
  } = {},
) {
  const identity = accountIdentity(user, apex);
  const [sourceVersion, setSourceVersion] = useState(0);
  const enabled = options.enabled ?? true;
  const accountEnabled = options.accountEnabled ?? enabled;
  const overview = useQuery(
    {
      queryKey: ["usageOverview", apex, period, options.query ?? null],
      queryFn: () => getUsageOverview(period, options.query),
      staleTime: STALE_MS,
      gcTime: 0,
      retry: false,
      enabled,
    },
    queryClient,
  );
  const balance = useQuery(
    {
      queryKey: ["usageUser", identity],
      // `previous` is an ownership snapshot; preserve its reference on refetch.
      structuralSharing: false,
      queryFn: async () => {
        const previous = queryClient.getQueryData<OfoxUserInfo | null>(
          OFOX_USER_KEY,
        );
        const fresh = await ofoxRefreshUserInfo();
        return { fresh, previous };
      },
      staleTime: STALE_MS,
      gcTime: 0,
      retry: false,
      enabled: !!user && accountEnabled,
    },
    queryClient,
  );
  const trend = useBalanceTrend(!!user && accountEnabled, identity);
  const spending = useQuery(
    {
      queryKey: ["spending", identity],
      queryFn: getSpending,
      staleTime: STALE_MS,
      gcTime: 0,
      retry: false,
      enabled: !!user && accountEnabled,
    },
    queryClient,
  );
  // Only publish a fresh reply to the same still-observed account.
  useEffect(() => {
    const fresh = balance.data?.fresh;
    if (!fresh || accountIdentity(fresh, apex) !== identity) return;
    const current = queryClient.getQueryData<OfoxUserInfo | null>(
      OFOX_USER_KEY,
    );
    if (
      current &&
      current === balance.data?.previous &&
      accountIdentity(current, apex) === identity
    )
      queryClient.setQueryData(OFOX_USER_KEY, fresh);
    void queryClient.invalidateQueries({
      queryKey: [...BALANCE_TREND_KEY, identity],
    });
  }, [balance.data, identity, apex]);
  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen("usage-synced", () => {
      if (disposed) return;
      setSourceVersion((version) => version + 1);
      void queryClient.invalidateQueries({
        queryKey: ["usageOverview", apex],
      });
    })
      .then((off) => {
        if (disposed) off();
        else stop = off;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, [apex]);
  const refreshLocal = async () => {
    await usageApi.syncSessionUsage();
    setSourceVersion((version) => version + 1);
    if (enabled) await overview.refetch({ throwOnError: true });
  };
  const refresh = async () => {
    // Keep every failed source visible while still refreshing the other sources.
    const results = await Promise.allSettled([
      refreshLocal(),
      ...(user && accountEnabled
        ? [
            balance.refetch({ throwOnError: true }),
            spending.refetch({ throwOnError: true }),
          ]
        : []),
    ]);
    const next = await Promise.allSettled([
      ...(user && accountEnabled
        ? [trend.refetch({ throwOnError: true })]
        : []),
    ]);
    const failed = [...results, ...next].filter(
      (r): r is PromiseRejectedResult => r.status === "rejected",
    );
    if (failed.length)
      throw new Error(failed.map((r) => String(r.reason)).join("\n"));
  };
  return {
    overview,
    balance,
    trend,
    spending,
    user,
    refresh,
    refreshLocal,
    sourceVersion,
  };
}
