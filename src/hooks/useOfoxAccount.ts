import { useCallback, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { ofoxGetUserInfo, type OfoxUserInfo } from "@/lib/api/ofoxAuth";
import { queryClient } from "@/lib/query/queryClient";

export const OFOX_USER_KEY = ["ofoxUser"] as const;

/** Waits between the retries of an empty `/openapi/me`: about 12s in all. */
const RETRY_DELAYS_MS = [1000, 3000, 8000];

/**
 * True when the user object came back from the backend with at least one
 * identity field populated. False for `null` AND for the all-null
 * placeholder the backend returns when `/openapi/me` was unreachable
 * during the login flow (see `ofox_auth.rs::poll_for_token`).
 *
 * We treat email or name presence as the signal — `org_id` alone wouldn't
 * give the header anything useful to render.
 */
export function isMeaningfulUser(u: OfoxUserInfo | null | undefined): boolean {
  if (!u) return false;
  return Boolean(u.email || u.name);
}

function storeUser(user: OfoxUserInfo | null) {
  queryClient.setQueryData<OfoxUserInfo | null>(OFOX_USER_KEY, user);
}

export interface OfoxAccount {
  user: OfoxUserInfo | null;
  /** True while the retry below is probing for a real user. */
  retrying: boolean;
  /** 1..3 while retrying, for the "（1/3）" hint; 0 otherwise. */
  retryAttempt: number;
  /** The retry ran out without a real user: offer a manual retry. */
  loadFailed: boolean;
  balanceRefreshing: boolean;
  /** Fetch the user once. A failed call keeps the previous user. */
  loadUser: () => Promise<OfoxUserInfo | null>;
  /** Retry an empty user three times with backoff. */
  retryUser: () => Promise<void>;
  /** Refetch the user for its wallet and toast the new balance. */
  refreshBalance: () => Promise<void>;
}

/**
 * The signed-in OFox user and wallet, shared by every page through the
 * `["ofoxUser"]` query. The query never fetches by itself: the tools scan,
 * the balance refresh and the retry below fetch it, as the console always
 * did. It is dropped when the last consumer unmounts (gcTime 0), so the next
 * account to sign in never sees the previous one's name or balance.
 */
export function useOfoxAccount(): OfoxAccount {
  const { t } = useTranslation();
  const { data: user = null } = useQuery<OfoxUserInfo | null>(
    {
      queryKey: OFOX_USER_KEY,
      queryFn: ofoxGetUserInfo,
      enabled: false,
      staleTime: Infinity,
      gcTime: 0,
    },
    queryClient,
  );
  // The backend's `poll_for_token` returns a placeholder user (all-null
  // fields) when /openapi/me was unreachable at login time — that prevents
  // the LoginPage spinner trap, but it leaves the Console showing "用户"
  // with an empty email. The retry probes a few more times before giving up
  // and surfacing a manual "重新加载".
  const [retrying, setRetrying] = useState(false);
  const [retryAttempt, setRetryAttempt] = useState(0);
  const [loadFailed, setLoadFailed] = useState(false);
  const [balanceRefreshing, setBalanceRefreshing] = useState(false);

  const loadUser = useCallback(async () => {
    const fresh = await ofoxGetUserInfo().catch(() => null);
    if (fresh) storeUser(fresh);
    return fresh;
  }, []);

  /**
   * 1s/3s/8s — total ~12s. Picked over the original 2s/5s/15s because 22s
   * with no UI progress feels broken; 12s is short enough that the user is
   * still expecting a result and long enough that genuine backend recovery
   * (a few hundred ms after a glitch) lands inside. Bails as soon as a
   * meaningful user comes back.
   */
  const retryUser = useCallback(async () => {
    setLoadFailed(false);
    setRetrying(true);
    setRetryAttempt(0);
    try {
      for (let i = 0; i < RETRY_DELAYS_MS.length; i++) {
        setRetryAttempt(i + 1);
        await new Promise((r) => setTimeout(r, RETRY_DELAYS_MS[i]));
        const fresh = await ofoxGetUserInfo().catch(() => null);
        if (isMeaningfulUser(fresh)) {
          storeUser(fresh);
          return;
        }
      }
      setLoadFailed(true);
    } finally {
      setRetrying(false);
      setRetryAttempt(0);
    }
  }, []);

  /**
   * Refetch *only* the user/wallet snapshot, not the tools. The toast tells
   * the user the round-trip happened even when the number didn't move.
   */
  const refreshBalance = useCallback(async () => {
    setBalanceRefreshing(true);
    try {
      const fresh = await ofoxGetUserInfo();
      storeUser(fresh);
      const value = fresh?.balance?.balance;
      if (typeof value === "number") {
        toast.success(
          t("console.balanceUpdated", { amount: `$${value.toFixed(2)}` }),
        );
      } else {
        // Auth was fine but the balance endpoint returned no number.
        toast.warning(t("console.balanceNoData"));
      }
    } catch (e) {
      console.error("[useOfoxAccount] balance refresh failed", e);
      toast.error(t("console.balanceRefreshFailed"));
    } finally {
      setBalanceRefreshing(false);
    }
  }, [t]);

  return {
    user,
    retrying,
    retryAttempt,
    loadFailed,
    balanceRefreshing,
    loadUser,
    retryUser,
    refreshBalance,
  };
}
