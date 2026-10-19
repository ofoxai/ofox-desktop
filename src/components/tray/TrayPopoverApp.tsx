import { useTranslation } from "react-i18next";
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import TrayHeader from "./TrayHeader";
import TrayAccount from "./TrayAccount";
import TrayUsage from "./TrayUsage";
import TrayProfiles from "./TrayProfiles";
import ToolStatusList from "./ToolStatusList";
import BottomMenu from "./BottomMenu";
import PopoverFrame from "./PopoverFrame";
import BalanceWarningBanner from "./BalanceWarningBanner";
import UpdateBanner from "./UpdateBanner";
import { SegmentedControl } from "@/components/ui/segmented";
import { useOfoxAuth } from "@/hooks/useOfoxAuth";
import { useUpdate } from "@/contexts/UpdateContext";
import { settingsApi } from "@/lib/api";
import { queryClient } from "@/lib/query/queryClient";
import { BOUND_TOOLS_STORAGE_KEY, TOOL_META } from "@/config/toolMeta";
import { isMac } from "@/lib/platform";

const DEFAULT_LOW_BALANCE_THRESHOLD = 10;
const TAB_STORAGE_KEY = "ofox-tray-tab";
type TrayTab = "tools" | "usage";

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

function readTab(): TrayTab {
  try {
    return localStorage.getItem(TAB_STORAGE_KEY) === "usage"
      ? "usage"
      : "tools";
  } catch {
    return "tools";
  }
}

function saveTab(tab: TrayTab) {
  try {
    localStorage.setItem(TAB_STORAGE_KEY, tab);
  } catch {
    // A tab that isn't remembered opens on 工具 next time; nothing else depends on it.
  }
}

/**
 * Tray popover root, laid out as magpie's panel: the brand and its actions,
 * the account with its balance on one line, then 工具 / 用量 tabs over one
 * scrolling card, and a one-line footer.
 *
 * Layout invariant: the header, account line and tabs keep their places in
 * every auth state; only what is on the account line changes.
 */
export default function TrayPopoverApp() {
  const { t } = useTranslation();
  const { status, refetch, isActive } = useOfoxAuth();
  const { hasUpdate, isDismissed } = useUpdate();
  const [refreshing, setRefreshing] = useState(false);
  const [tab, setTab] = useState<TrayTab>(readTab);

  // 已绑定工具列表来自 localStorage（与主窗口共享同一份）。低余额阈值/开关
  // 走 settingsApi.get()。两路独立读，后者失败不影响前者。
  const [boundTools, setBoundTools] = useState<string[]>(() =>
    readBoundToolsFromStorage(),
  );
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
        typeof s?.lowBalanceThresholdUsd === "number"
          ? s.lowBalanceThresholdUsd
          : DEFAULT_LOW_BALANCE_THRESHOLD,
      );
      // 缺省视为开启 —— 用户没在设置页明确关掉的话就允许提示。
      setLowBalanceEnabled(s?.lowBalanceEnabled !== false);
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

  /**
   * The header's refresh: a fresh wallet read (`ofoxGetUserInfo` reaches
   * `/openapi/orgs/me/balance`), the bound list, and the models and usage
   * the tabs show.
   */
  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      reloadBoundTools();
      const { ofoxGetUserInfo } = await import("@/lib/api/ofoxAuth");
      await Promise.all([
        isActive ? ofoxGetUserInfo() : null,
        queryClient.invalidateQueries({ queryKey: ["modelSelection"] }),
        queryClient.invalidateQueries({ queryKey: ["usageOverview"] }),
      ]);
      await refetch();
    } catch (e) {
      console.error("[TrayPopover] refresh failed", e);
      toast.error(t("uiCopy.balanceFailed"));
    } finally {
      setRefreshing(false);
    }
  }, [isActive, refetch, reloadBoundTools, t]);

  useEffect(() => {
    // 设置透明背景：只有 macOS 的弹窗窗口是透明的（见 PopoverFrame）
    if (!isMac()) return;
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

  const handleLogin = useCallback(async () => {
    try {
      // The device flow runs in the main window; it needs room for the code.
      await invoke("ofox_request_reauth");
    } catch (e) {
      console.error("[TrayPopover] ofox_request_reauth failed", e);
    }
  }, []);

  const balanceUsd = status?.user?.balance?.balance ?? null;
  const lowBalance =
    isActive &&
    typeof balanceUsd === "number" &&
    lowBalanceEnabled &&
    balanceUsd < lowBalanceThreshold;
  const knownTools = boundTools.filter((id) => TOOL_META[id]);

  return (
    <PopoverFrame>
      <TrayHeader refreshing={refreshing} onRefresh={() => void refresh()} />
      <TrayAccount
        status={status}
        low={lowBalance}
        onLogin={() => void handleLogin()}
      />
      {hasUpdate && !isDismissed && (
        <div className="shrink-0 pb-2">
          <UpdateBanner />
        </div>
      )}
      {lowBalance && (
        <div className="shrink-0 pb-2">
          <BalanceWarningBanner
            threshold={lowBalanceThreshold}
            balanceUsd={balanceUsd as number}
          />
        </div>
      )}
      <SegmentedControl<TrayTab>
        aria-label={t("tray.tabs.label")}
        value={tab}
        onChange={(next) => {
          setTab(next);
          saveTab(next);
        }}
        options={[
          {
            value: "tools",
            label: (
              <>
                {t("tray.tabs.tools")}
                {knownTools.length > 0 && (
                  <em className="ml-1 not-italic tabular-nums text-ink-faint">
                    {knownTools.length}
                  </em>
                )}
              </>
            ),
          },
          { value: "usage", label: t("tray.tabs.usage") },
        ]}
        className="mx-3 mb-2 flex shrink-0 self-stretch [&>button]:flex-1"
      />
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-2 pt-[2px]">
        {tab === "tools" ? (
          <ToolStatusList boundTools={boundTools} disabled={!isActive} />
        ) : (
          <TrayUsage />
        )}
      </div>
      <BottomMenu
        lead={
          isActive && knownTools.length > 0 ? (
            <TrayProfiles boundTools={knownTools} />
          ) : null
        }
      />
    </PopoverFrame>
  );
}
