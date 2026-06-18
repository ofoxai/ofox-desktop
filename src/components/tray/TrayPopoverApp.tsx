import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import ProfileHeader from "./ProfileHeader";
import AuthExpiredBanner from "./AuthExpiredBanner";
import AccountBalance from "./AccountBalance";
import ActionButtons from "./ActionButtons";
import StatsRow from "./StatsRow";
import ToolStatusList, { type ToolActivenessMap } from "./ToolStatusList";
import BottomMenu from "./BottomMenu";
import BalanceWarningBanner from "./BalanceWarningBanner";
import { useOfoxAuth } from "@/hooks/useOfoxAuth";
import { settingsApi } from "@/lib/api";
import { proxyApi } from "@/lib/api/proxy";
import {
  BOUND_TOOLS_STORAGE_KEY,
  PROXY_SUPPORTED_TOOLS,
} from "@/config/toolMeta";

const DEFAULT_LOW_BALANCE_THRESHOLD = 10;

/**
 * 已绑定工具的真理源 = `localStorage[BOUND_TOOLS_STORAGE_KEY]`，与 MainApp
 * 的 `readBoundToolsFromStorage` 共用同一份数据（popover 跟主窗口同 origin
 * `tauri://localhost`，localStorage 是共享的）。
 *
 * `AppSettings.boundTools` 是后端 health probe 用的镜像副本，写入是
 * `bindTools()` 里的 `void mirrorBoundToolsToSettings(...)` 异步触发，存在
 * 老数据迁移延迟、写入失败被吞掉等边缘情况，不能作为前端展示的真理源——
 * 老的 / 之前手动改过的用户会出现"主窗口已绑定 N 个工具，popover 显示 0"。
 */
function readBoundToolsFromStorage(): string[] {
  try {
    const raw = localStorage.getItem(BOUND_TOOLS_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((x): x is string => typeof x === "string")
      : [];
  } catch {
    return [];
  }
}

interface ToolInfoLite {
  name: string;
  version: string | null;
  error: string | null;
}

/**
 * 计算每个绑定工具的状态点颜色，与主窗口 ConsolePage 的"已接管 ●"
 * 完全同语义：
 *
 *   active = 接管 ON 且检测到二进制（绿）—— 主页"已接管"
 *   error  = 接管 ON 但检测不到二进制（红）—— 主页"未检测到"
 *   idle   = 其它（灰）—— 主页"暂不支持代理统计 / 未开启代理 / 未检测到"
 *
 * 不依赖 health probe 探针。健康探针只代表"上次 max_tokens=1 的请求是否
 * 跑通"，跟"已接管"是两件事，会让 popover 与主页色块不一致。
 */
async function computeActiveness(
  boundTools: string[],
): Promise<ToolActivenessMap> {
  if (boundTools.length === 0) return {};

  const [takeoverStatus, toolInfos] = await Promise.all([
    proxyApi.getProxyTakeoverStatus().catch(() => null),
    settingsApi
      .getToolVersions(boundTools, undefined, false)
      .catch(() => [] as ToolInfoLite[]),
  ]);

  const takeoverMap = (takeoverStatus ?? {}) as Record<string, boolean>;
  const detectedMap = new Map<string, boolean>();
  for (const info of toolInfos as ToolInfoLite[]) {
    detectedMap.set(info.name, !!info.version && !info.error);
  }

  const out: ToolActivenessMap = {};
  for (const id of boundTools) {
    const proxied = !!takeoverMap[id];
    const detected = !!detectedMap.get(id);
    const proxySupported = PROXY_SUPPORTED_TOOLS.includes(id);

    if (proxied && detected) {
      out[id] = "active";
    } else if (proxied && !detected) {
      out[id] = "error";
    } else if (!proxySupported && detected) {
      out[id] = "idle";
    } else if (detected) {
      out[id] = "idle";
    } else {
      out[id] = "error";
    }
  }
  return out;
}

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

  // 已绑定工具列表来自 localStorage（与主窗口共享同一份）。低余额阈值/开关
  // 走 settingsApi.get()。两路独立读，后者失败不影响前者。
  const [boundTools, setBoundTools] = useState<string[]>(() =>
    readBoundToolsFromStorage(),
  );
  const [activeness, setActiveness] = useState<ToolActivenessMap>({});
  const [lowBalanceThreshold, setLowBalanceThreshold] = useState<number>(
    DEFAULT_LOW_BALANCE_THRESHOLD,
  );
  const [lowBalanceEnabled, setLowBalanceEnabled] = useState<boolean>(true);

  const reloadBoundTools = useCallback(() => {
    setBoundTools(readBoundToolsFromStorage());
  }, []);

  const reloadSettings = useCallback(async () => {
    try {
      const s = await settingsApi.get();
      setLowBalanceThreshold(
        typeof s.lowBalanceThresholdUsd === "number"
          ? s.lowBalanceThresholdUsd
          : DEFAULT_LOW_BALANCE_THRESHOLD,
      );
      // 缺省视为开启 —— 用户没在设置页明确关掉的话就允许提示。
      setLowBalanceEnabled(s.lowBalanceEnabled !== false);
    } catch (e) {
      console.error("[TrayPopover] settings fetch failed", e);
    }
  }, []);

  useEffect(() => {
    void reloadSettings();
  }, [reloadSettings]);

  // 监听主窗口绑定/解绑工具时 emit 的 `ofox-prefs-updated`，让 popover 立刻
  // 跟着 reload。settings 也跟着重读，覆盖低余额阈值在主窗口刚改完的情况。
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    (async () => {
      try {
        const off = await listen("ofox-prefs-updated", () => {
          reloadBoundTools();
          void reloadSettings();
        });
        if (cancelled) off();
        else unlisten = off;
      } catch (e) {
        console.error("[TrayPopover] listen ofox-prefs-updated failed", e);
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [reloadBoundTools, reloadSettings]);

  // boundTools 变化时重算激活状态（takeover + detected）。focus 时也强制重算。
  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      const next = await computeActiveness(boundTools);
      if (!cancelled) setActiveness(next);
    };
    void load();

    const onFocus = () => void load();
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", onFocus);
    };
  }, [boundTools]);

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
      reloadBoundTools();
      void reloadSettings();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
    };
  }, [refetch, reloadBoundTools, reloadSettings]);

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

  const showLowBalanceBanner =
    isActive &&
    typeof balanceUsd === "number" &&
    lowBalanceEnabled &&
    balanceUsd < lowBalanceThreshold;

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
          {showLowBalanceBanner && (
            <BalanceWarningBanner
              threshold={lowBalanceThreshold}
              balanceUsd={balanceUsd as number}
            />
          )}
          <ActionButtons />
          <StatsRow boundTools={boundTools} activeness={activeness} />
        </div>
        {/* 工具状态：标题固定，列表可滚动 */}
        <ToolStatusList boundTools={boundTools} activeness={activeness} />
        {/* 固定区域：底部菜单 */}
        <div className="shrink-0">
          <BottomMenu />
        </div>
      </div>
    </div>
  );
}
