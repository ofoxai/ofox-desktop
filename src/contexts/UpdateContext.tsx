import React, {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
  useRef,
} from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { settingsApi } from "@/lib/api";

/** 展示用的更新信息，从后端 `UpdateCheckResult` 映射而来。 */
export interface UpdateInfo {
  currentVersion: string;
  availableVersion: string;
  notes?: string;
  pubDate?: string;
  /** 当前平台安装包直链；选不到时为通用下载页。 */
  downloadUrl?: string | null;
}

/** 托盘点"跳过"后广播给主窗口（反之亦然）的事件名，沿用项目 ofox-* 风格。 */
const UPDATE_DISMISSED_EVENT = "ofox-update-dismissed";

interface UpdateContextValue {
  // 更新状态
  hasUpdate: boolean;
  updateInfo: UpdateInfo | null;
  isChecking: boolean;
  error: string | null;

  // 提示状态
  isDismissed: boolean;
  dismissUpdate: () => void;

  // 首次发现弹窗：仅在尚未弹过、未跳过时为 true，弹一次后由 markPrompted 关闭
  shouldPrompt: boolean;
  markPrompted: () => void;

  // 操作方法
  checkUpdate: () => Promise<boolean>;
  resetDismiss: () => void;
}

const UpdateContext = createContext<UpdateContextValue | undefined>(undefined);

export function UpdateProvider({ children }: { children: React.ReactNode }) {
  const DISMISSED_VERSION_KEY = "ccswitch:update:dismissedVersion";
  const LEGACY_DISMISSED_KEY = "dismissedUpdateVersion"; // 兼容旧键

  const [hasUpdate, setHasUpdate] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [isChecking, setIsChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isDismissed, setIsDismissed] = useState(false);
  // 本会话是否已经弹过首次提示对话框。用 ref 而非 state——它只控制
  // shouldPrompt 的计算，且要在多次定时复查间保持稳定。
  const [hasPrompted, setHasPrompted] = useState(false);

  // 读取已跳过的版本号（含旧键迁移），返回当前生效的 dismissed 版本。
  const readDismissedVersion = useCallback((): string | null => {
    let dismissedVersion = localStorage.getItem(DISMISSED_VERSION_KEY);
    if (!dismissedVersion) {
      const legacy = localStorage.getItem(LEGACY_DISMISSED_KEY);
      if (legacy) {
        localStorage.setItem(DISMISSED_VERSION_KEY, legacy);
        localStorage.removeItem(LEGACY_DISMISSED_KEY);
        dismissedVersion = legacy;
      }
    }
    return dismissedVersion;
  }, []);

  // updateInfo 变化时重算 isDismissed（覆盖跨窗口 localStorage 已被改的情况）。
  useEffect(() => {
    const current = updateInfo?.availableVersion;
    if (!current) return;
    setIsDismissed(readDismissedVersion() === current);
  }, [updateInfo?.availableVersion, readDismissedVersion]);

  const isCheckingRef = useRef(false);

  const checkUpdate = useCallback(async () => {
    if (isCheckingRef.current) return false;
    isCheckingRef.current = true;
    setIsChecking(true);
    setError(null);

    try {
      // Ofox 自托管链路：拉 R2 的 latest.json 比对版本。本应用不做自动安装，
      // 发现新版只引导用户去下载页手动安装。
      const result = await settingsApi.checkUpdates();

      if (result.hasUpdate) {
        const info: UpdateInfo = {
          currentVersion: result.currentVersion,
          availableVersion: result.latestVersion,
          notes: result.notes ?? undefined,
          pubDate: result.pubDate ?? undefined,
          downloadUrl: result.downloadUrl,
        };
        setHasUpdate(true);
        setUpdateInfo(info);
        setIsDismissed(readDismissedVersion() === result.latestVersion);
        return true; // 有更新
      } else {
        setHasUpdate(false);
        setUpdateInfo(null);
        setIsDismissed(false);
        return false; // 已是最新
      }
    } catch (err) {
      console.error("检查更新失败:", err);
      setError(err instanceof Error ? err.message : "检查更新失败");
      setHasUpdate(false);
      throw err; // 抛出错误让调用方处理（手动检查时弹 toast）
    } finally {
      setIsChecking(false);
      isCheckingRef.current = false;
    }
  }, [readDismissedVersion]);

  const dismissUpdate = useCallback(() => {
    const version = updateInfo?.availableVersion;
    setIsDismissed(true);
    if (version) {
      localStorage.setItem(DISMISSED_VERSION_KEY, version);
      localStorage.removeItem(LEGACY_DISMISSED_KEY);
      // 广播给另一个窗口（主窗口 / 托盘弹窗各持一份 context 实例）。
      void emit(UPDATE_DISMISSED_EVENT, { version });
    }
  }, [updateInfo?.availableVersion]);

  const resetDismiss = useCallback(() => {
    setIsDismissed(false);
    localStorage.removeItem(DISMISSED_VERSION_KEY);
    localStorage.removeItem(LEGACY_DISMISSED_KEY);
  }, []);

  const markPrompted = useCallback(() => {
    setHasPrompted(true);
  }, []);

  // 监听另一个窗口的"跳过"广播，同步本窗口的 isDismissed（仅当跳过的版本
  // 正是当前提示的版本）。
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    (async () => {
      try {
        const off = await listen<{ version?: string }>(
          UPDATE_DISMISSED_EVENT,
          (evt) => {
            const v = evt.payload?.version;
            if (v && v === updateInfo?.availableVersion) {
              setIsDismissed(true);
            }
          },
        );
        if (cancelled) off();
        else unlisten = off;
      } catch (e) {
        console.error("[Update] listen dismissed event failed", e);
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [updateInfo?.availableVersion]);

  // 主动检查。两个 webview（主窗口 / 托盘弹窗）各持一份 context 实例，React
  // state 不跨 webview 共享，所以各自都要查一次才能拿到 hasUpdate（localStorage
  // 共享、但只存"跳过的版本号"，不存检查结果）。失败静默处理（吞掉错误），不
  // 弹 toast，避免离线/网络抖动时打扰用户。
  //   - 主窗口：启动延迟 5s 查一次 + 每 24h 定时复查（长生命周期）。
  //   - 托盘弹窗：mount 即查一次 + 窗口 focus 时复查（短生命周期、按需打开，
  //     不需要定时器；每次重开都会重新 mount 触发检查）。
  useEffect(() => {
    const isTrayPopover = window.location.hash === "#/tray-popover";

    const silentCheck = () => {
      checkUpdate().catch(() => {
        /* 自动检查失败静默处理 */
      });
    };

    if (isTrayPopover) {
      // 托盘：立即查 + focus 复查。
      silentCheck();
      const onFocus = () => silentCheck();
      window.addEventListener("focus", onFocus);
      return () => window.removeEventListener("focus", onFocus);
    }

    // 主窗口：启动延迟 + 24h 定时。
    const STARTUP_DELAY_MS = 5000;
    const INTERVAL_MS = 24 * 60 * 60 * 1000;
    const startupTimer = setTimeout(silentCheck, STARTUP_DELAY_MS);
    const intervalTimer = setInterval(silentCheck, INTERVAL_MS);

    return () => {
      clearTimeout(startupTimer);
      clearInterval(intervalTimer);
    };
  }, [checkUpdate]);

  const shouldPrompt = hasUpdate && !isDismissed && !hasPrompted;

  const value: UpdateContextValue = {
    hasUpdate,
    updateInfo,
    isChecking,
    error,
    isDismissed,
    dismissUpdate,
    shouldPrompt,
    markPrompted,
    checkUpdate,
    resetDismiss,
  };

  return (
    <UpdateContext.Provider value={value}>{children}</UpdateContext.Provider>
  );
}

export function useUpdate() {
  const context = useContext(UpdateContext);
  if (!context) {
    throw new Error("useUpdate must be used within UpdateProvider");
  }
  return context;
}
