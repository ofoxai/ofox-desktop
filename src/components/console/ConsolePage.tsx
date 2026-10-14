import { SettingsHealthBanner } from "./SettingsHealthBanner";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { settingsApi } from "@/lib/api";
import { useOfoxAccount } from "@/hooks/useOfoxAccount";
import OfoxSettingsDialog from "./OfoxSettingsDialog";
import { ToolsPanel } from "./ToolsPanel";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { useUpdate } from "@/contexts/UpdateContext";
import { SessionsPanel } from "@/components/sessions/SessionsPanel";
import { AppHeader } from "@/components/shell/AppHeader";
import {
  ShellProvider,
  shellPanelId,
  useShell,
  type ShellPage,
} from "@/components/shell/ShellContext";
import { UsagePage } from "@/components/usage-overview/UsagePage";

interface ConsolePageProps {
  boundTools: string[];
  /** Called after the user adds new tools through 添加 so MainApp can
   *  re-read localStorage and re-fire its takeover reconciliation sweep. */
  onBoundToolsChanged?: () => void;
}

/**
 * The signed-in window: the header with the 工具 / 会话 / 用量 tabs over
 * the open page. 工具 is always mounted; 会话 and 用量 mount on their first
 * visit and stay mounted, hidden, so their state survives switching.
 */
export default function ConsolePage(props: ConsolePageProps) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const openSettings = useCallback(() => setSettingsOpen(true), []);

  // 监听 tray popover 发出的"打开偏好设置"事件，把当前 dialog 拉起来。
  // popover 是独立 webview，无法直接调用本组件的 setState，因此走 Tauri
  // event 通信。channel 名与 BottomMenu.tsx 里的 emit 必须一致。
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    (async () => {
      try {
        const off = await listen("ofox-open-settings", openSettings);
        if (cancelled) {
          off();
        } else {
          unlisten = off;
        }
      } catch (e) {
        console.error("[ConsolePage] listen ofox-open-settings failed", e);
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [openSettings]);

  return (
    <ShellProvider openSettings={openSettings}>
      <ConsoleShell {...props} />
      <OfoxSettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
    </ShellProvider>
  );
}

function ConsoleShell({ boundTools, onBoundToolsChanged }: ConsolePageProps) {
  const { t } = useTranslation();
  const shell = useShell();
  // 应用自更新的首次发现弹窗；常驻的提示在顶栏（UpdatePill）。数据源 =
  // UpdateContext（自动检查走 R2 latest.json）。
  const {
    updateInfo,
    shouldPrompt: shouldPromptUpdate,
    markPrompted,
  } = useUpdate();
  const account = useOfoxAccount();

  /** A page's panel: labelled by its tab, hidden while another is open. */
  const panel = (page: ShellPage) => ({
    role: "tabpanel",
    id: shellPanelId(page),
    "aria-label": t(`shell.tabs.${page}`),
    hidden: shell.active !== page,
  });

  return (
    <div className="flex h-screen w-full flex-col bg-canvas">
      <AppHeader />
      <div className="px-4">
        <SettingsHealthBanner />
      </div>

      <main className="min-h-0 flex-1">
        <div
          {...panel("tools")}
          className="h-full overflow-y-auto px-[max(16px,calc(50%-600px))] pb-4 pt-1"
        >
          <ToolsPanel
            boundTools={boundTools}
            account={account}
            onBoundToolsChanged={onBoundToolsChanged}
          />
        </div>

        {shell.visited.has("sessions") && (
          <div {...panel("sessions")} className="h-full px-4 pb-4 pt-1">
            <SessionsPanel className="h-full min-h-0" />
          </div>
        )}
        {shell.visited.has("usage") && (
          <div
            {...panel("usage")}
            className="h-full overflow-y-auto px-4 pb-4 pt-1"
          >
            <UsagePage account={account} boundTools={boundTools} />
          </div>
        )}
      </main>

      {/* 首次发现新版本的提示对话框。顶栏的 UpdatePill 常驻（hasUpdate &&
          !isDismissed），这里只在本会话首次发现时弹一次，弹过即由
          markPrompted 关闭，避免每次定时复查都打扰。 */}
      <ConfirmDialog
        isOpen={shouldPromptUpdate}
        variant="info"
        title={t("update.promptTitle")}
        message={t("update.promptMessage", {
          version: updateInfo?.availableVersion ?? "",
        })}
        confirmText={t("settings.goDownload")}
        cancelText={t("update.later")}
        onConfirm={() => {
          markPrompted();
          const target = updateInfo?.downloadUrl;
          if (target) {
            settingsApi.openExternal(target).catch((e) => {
              console.error("[ConsolePage] open download url failed", e);
              toast.error(t("settings.openReleaseNotesFailed"));
            });
          }
        }}
        onCancel={() => {
          // "稍后"：仅关闭弹窗，顶栏的 UpdatePill 保留。
          markPrompted();
        }}
      />
    </div>
  );
}
