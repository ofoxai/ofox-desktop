import { SettingsHealthBanner } from "./SettingsHealthBanner";
import { useCallback, useEffect, useRef, useState } from "react";
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
import { LibraryPage } from "@/components/library/LibraryPage";
import {
  ShellProvider,
  shellPanelId,
  useShell,
  type ShellPage,
} from "@/components/shell/ShellContext";
import { UsagePage } from "@/components/usage-overview/UsagePage";
import {
  OptionalToolsCatalog,
  type InstallCatalogHandle,
} from "@/components/tools/OptionalToolsCatalog";
import { InteractionScope } from "@/components/ui/interaction-scope";
import { useShellRefresh } from "@/components/shell/ShellContext";
import { Button } from "@/components/ui/button";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";

interface ConsolePageProps {
  active?: boolean;
  boundTools: string[];
  localOnly?: boolean;
  localSignedIn?: boolean;
  onLoginRequested?: () => void;
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
  const active = props.active ?? true;
  const [settingsOpen, setSettingsOpen] = useState(false);
  const openSettings = useCallback(() => {
    if (active) setSettingsOpen(true);
  }, [active]);
  useEffect(() => {
    if (!active) setSettingsOpen(false);
  }, [active]);

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
    <InteractionScope active={active}>
      <ShellProvider openSettings={openSettings}>
        <ConsoleShell {...props} />
        <OfoxSettingsDialog
          open={settingsOpen}
          onOpenChange={setSettingsOpen}
        />
      </ShellProvider>
    </InteractionScope>
  );
}

function ConsoleShell({
  boundTools,
  onBoundToolsChanged,
  localOnly = false,
  localSignedIn,
  onLoginRequested,
}: ConsolePageProps) {
  const { t } = useTranslation();
  const shell = useShell();
  // 应用自更新的首次发现弹窗；常驻的提示在顶栏（UpdatePill）。数据源 =
  // UpdateContext（自动检查走 R2 latest.json）。
  const {
    updateInfo,
    shouldPrompt: shouldPromptUpdate,
    markPrompted,
  } = useUpdate();

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
          className="h-full overscroll-contain overflow-y-auto px-[max(16px,calc(50%-600px))] pb-6 pt-1"
        >
          {localOnly ? (
            <LocalToolsPanel
              signedIn={localSignedIn}
              onLoginRequested={onLoginRequested}
            />
          ) : (
            <SignedToolsPanel
              boundTools={boundTools}
              onBoundToolsChanged={onBoundToolsChanged}
            />
          )}
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
            {localOnly ? (
              <UsagePage account={LOCAL_ACCOUNT} boundTools={[]} />
            ) : (
              <SignedUsagePage boundTools={boundTools} />
            )}
          </div>
        )}
        {shell.visited.has("library") && (
          <div
            {...panel("library")}
            className="h-full overflow-y-auto px-4 pb-4 pt-1"
          >
            <InteractionScope active={shell.active === "library"}>
              <LibraryPage />
            </InteractionScope>
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

const LOCAL_ACCOUNT: OfoxAccount = {
  user: null,
  retrying: false,
  retryAttempt: 0,
  loadFailed: false,
  balanceRefreshing: false,
  loadUser: async () => null,
  retryUser: async () => undefined,
  refreshBalance: async () => undefined,
};

function SignedToolsPanel(
  props: Pick<ConsolePageProps, "boundTools" | "onBoundToolsChanged">,
) {
  const account = useOfoxAccount();
  return <ToolsPanel {...props} account={account} />;
}

function SignedUsagePage({ boundTools }: { boundTools: string[] }) {
  const account = useOfoxAccount();
  return <UsagePage boundTools={boundTools} account={account} />;
}

function LocalToolsPanel({
  signedIn,
  onLoginRequested,
}: {
  signedIn?: boolean;
  onLoginRequested?: () => void;
}) {
  const { t } = useTranslation();
  const catalog = useRef<InstallCatalogHandle>(null);
  useShellRefresh("tools", () => catalog.current?.refresh());
  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-ui font-semibold">
            {t("toolCatalog.localTitle")}
          </h1>
          <p className="text-note text-ink-muted">
            {t("toolCatalog.localDescription")}
          </p>
        </div>
        <Button variant="secondary" onClick={onLoginRequested}>
          {t(signedIn ? "toolCatalog.bindOptional" : "toolCatalog.login")}
        </Button>
      </div>
      <OptionalToolsCatalog ref={catalog} boundTools={[]} />
    </>
  );
}
