import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import ProfileHeader from "./ProfileHeader";
import AuthExpiredBanner from "./AuthExpiredBanner";
import AccountBalance from "./AccountBalance";
import ActionButtons from "./ActionButtons";
import StatsRow from "./StatsRow";
import ToolStatusList from "./ToolStatusList";
import BottomMenu from "./BottomMenu";
import { useOfoxAuth } from "@/hooks/useOfoxAuth";

/**
 * Tray popover root.
 *
 * Layout invariant: the section below the header keeps the same vertical
 * order across all auth states; we only swap which component renders.
 * That avoids the popover height jumping when the state changes.
 */
export default function TrayPopoverApp() {
  const { status, refetch, isActive, isExpired, isLoggedOut } = useOfoxAuth();
  const [balanceRefreshing, setBalanceRefreshing] = useState(false);

  /**
   * Standalone refresh trigger for the balance pill. `useOfoxAuth.refetch()`
   * re-reads the in-memory snapshot the backend keeps. To force a network
   * round-trip to `/openapi/orgs/me/balance` we need to call
   * `ofoxGetUserInfo()` directly — it invokes `refresh_user_info` on the
   * Rust side, which pulls a fresh wallet snapshot, persists it, and then
   * the cached auth status reflects the new balance.
   */
  const refreshBalance = useCallback(async () => {
    setBalanceRefreshing(true);
    try {
      // Lazy-imported to keep the bundle tree intact; this module is a
      // sibling of the auth API.
      const { ofoxGetUserInfo } = await import("@/lib/api/ofoxAuth");
      const fresh = await ofoxGetUserInfo();
      await refetch();
      const value = fresh?.balance?.balance;
      if (typeof value === "number") {
        toast.success(`余额已更新：$${value.toFixed(2)}`);
      } else {
        toast.warning("余额已刷新，但暂无可用数据");
      }
    } catch (e) {
      console.error("[TrayPopover] balance refresh failed", e);
      toast.error("余额刷新失败，请稍后重试");
    } finally {
      setBalanceRefreshing(false);
    }
  }, [refetch]);

  useEffect(() => {
    // 设置透明背景
    document.documentElement.classList.add("tray-popover");
    return () => {
      document.documentElement.classList.remove("tray-popover");
    };
  }, []);

  // Re-fetch every time the popover comes back into focus — covers the case
  // where the user reauthenticated in the main window and reopened the tray.
  useEffect(() => {
    const onFocus = () => {
      void refetch();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
    };
  }, [refetch]);

  const handleReauth = useCallback(async () => {
    try {
      // Ask the backend to surface the main window in re-auth mode. The
      // device flow itself runs there (it needs a wider surface to display
      // the user_code + browser nudge).
      await invoke("ofox_request_reauth");
    } catch (e) {
      console.error("[TrayPopover] ofox_request_reauth failed", e);
    }
  }, []);

  // Pull straight from the cached user snapshot — the backend fetches the
  // wallet from `/openapi/orgs/me/balance` alongside `/openapi/me`.
  const balanceUsd = status?.user?.balance?.balance ?? null;

  return (
    <div className="h-screen w-full bg-transparent p-4">
      <div
        className="flex h-full flex-col overflow-hidden rounded-xl border border-border/50 shadow-[0_4px_12px_rgba(0,0,0,0.12)] backdrop-blur-xl"
        style={{ backgroundColor: "hsl(var(--popover) / 0.92)" }}
      >
        {/* 固定区域：不滚动 */}
        <div className="shrink-0">
          <ProfileHeader status={status} />
          {(isExpired || isLoggedOut) && (
            <AuthExpiredBanner
              variant={isExpired ? "expired" : "loggedOut"}
              onReauth={handleReauth}
            />
          )}
          <AccountBalance
            state={status?.state ?? "loggedout"}
            balanceUsd={isActive ? balanceUsd : null}
            onRefresh={refreshBalance}
            refreshing={balanceRefreshing}
          />
          <ActionButtons />
          <StatsRow />
        </div>
        {/* 工具状态：标题固定，列表可滚动 */}
        <ToolStatusList />
        {/* 固定区域：底部菜单 */}
        <div className="shrink-0">
          <BottomMenu />
        </div>
      </div>
    </div>
  );
}
