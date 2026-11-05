import { SettingsHealthBanner } from "./SettingsHealthBanner";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type Ref,
} from "react";
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
import { ShellFrame } from "@/components/shell/ShellFrame";
import {
  PAGE_BODY,
  PAGE_BODY_WIDE,
  PageHeader,
  ShellRefreshButton,
} from "@/components/shell/PageHeader";
import { cn } from "@/lib/utils";
import type { SidebarAccountState } from "@/components/shell/SidebarAccount";
import { LibraryPage } from "@/components/library/LibraryPage";
import {
  ShellProvider,
  shellPanelId,
  useShell,
  type ShellPage,
} from "@/components/shell/ShellContext";
import { UsagePage } from "@/components/usage-overview/UsagePage";
import { AccountAssistantPanel } from "@/components/assistant/AccountAssistantPanel";
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
 * The signed-in window: the header with the 工具 / 会话 / 资源库 / 用量 / 助手
 * tabs over the open page. 工具 is always mounted; the others mount on their
 * first visit and stay mounted, hidden, so their state survives switching.
 */
export default function ConsolePage(props: ConsolePageProps) {
  const active = props.active ?? true;
  const settingsButton = useRef<HTMLButtonElement>(null);
  const activeRef = useRef(active);
  useLayoutEffect(() => {
    activeRef.current = active;
  }, [active]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const openSettings = useCallback(() => {
    if (active) setSettingsOpen(true);
  }, [active]);
  useEffect(() => {
    if (!active) setSettingsOpen(false);
  }, [active]);
  const restoreSettingsFocus = useCallback(() => {
    const opener = settingsButton.current;
    if (
      activeRef.current &&
      opener?.isConnected &&
      !opener.closest('[hidden], [inert], [aria-hidden="true"]')
    ) {
      opener.focus();
    }
  }, []);

  // 监听 tray popover 发出的"打开偏好设置"事件，把当前 dialog 拉起来。
  // popover 是独立 webview，无法直接调用本组件的 setState，因此走 Tauri
  // event 通信。channel 名与 BottomMenu.tsx 里的 emit 必须一致。
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    (async () => {
      try {
        const off = await listen("ofox-open-settings", () => openSettings());
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
        <ConsoleShell {...props} settingsButtonRef={settingsButton} />
        <OfoxSettingsDialog
          open={settingsOpen}
          onOpenChange={setSettingsOpen}
          onReturnFocus={restoreSettingsFocus}
        />
      </ShellProvider>
    </InteractionScope>
  );
}

type ShellProps = ConsolePageProps & {
  settingsButtonRef: Ref<HTMLButtonElement>;
};

function ConsoleShell({
  boundTools,
  onBoundToolsChanged,
  localOnly = false,
  localSignedIn,
  onLoginRequested,
  settingsButtonRef,
}: ShellProps) {
  const { t } = useTranslation();
  // One shell in both modes, so switching local ↔ signed-in keeps every
  // page mounted. The account query never fetches by itself, so local mode
  // loads nothing; only the signed-in pages are handed the account.
  const signedAccount = useOfoxAccount();
  const account = localOnly ? undefined : signedAccount;
  const sidebarAccount: SidebarAccountState = account
    ? { kind: "signed", account }
    : {
        kind: "local",
        signedIn: Boolean(localSignedIn),
        onAction: () => onLoginRequested?.(),
      };
  const shell = useShell();
  // A narrow window keeps the top bar and the pages as they were; a wide
  // one has the sidebar and titled pages.
  const wide = shell.layout === "wide";
  // 应用自更新的首次发现弹窗；常驻的提示在侧栏底部（UpdatePill）。数据源 =
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
    <ShellFrame
      account={sidebarAccount}
      settingsButtonRef={settingsButtonRef}
      banner={
        <div className="px-4">
          <SettingsHealthBanner />
        </div>
      }
    >
      <div
        {...panel("tools")}
        className={cn(
          "h-full overscroll-contain overflow-y-auto pb-6",
          !wide && "pt-1",
        )}
      >
        {localOnly || !account ? (
          <LocalToolsPanel
            signedIn={localSignedIn}
            onLoginRequested={onLoginRequested}
          />
        ) : (
          <ToolsPanel
            boundTools={boundTools}
            onBoundToolsChanged={onBoundToolsChanged}
            account={account}
          />
        )}
      </div>

      {shell.visited.has("sessions") && (
        // The panel stays a plain block so its hidden attribute hides it;
        // a display class here (flex) would win over [hidden].
        <div
          {...panel("sessions")}
          className={cn("h-full", !wide && "px-4 pb-4 pt-1")}
        >
          <div className="flex h-full flex-col">
            {wide && (
              <PageHeader
                wide
                title={t("pages.sessions.title")}
                description={t("pages.sessions.description")}
                actions={<ShellRefreshButton />}
              />
            )}
            <SessionsPanel
              className={
                wide
                  ? cn(PAGE_BODY_WIDE, "min-h-0 flex-1 pb-4 pt-0")
                  : "h-full min-h-0"
              }
              onAnalyze={shell.locateUsage}
              onConfigure={(section) =>
                section === "tools"
                  ? shell.setActive("tools")
                  : shell.locateLibrary(section)
              }
            />
          </div>
        </div>
      )}
      {shell.visited.has("usage") && (
        <div
          {...panel("usage")}
          className={cn("h-full overflow-y-auto pb-4", !wide && "pt-1")}
        >
          {localOnly || !account ? (
            <UsagePage account={LOCAL_ACCOUNT} boundTools={[]} />
          ) : (
            <UsagePage boundTools={boundTools} account={account} />
          )}
        </div>
      )}
      {shell.visited.has("library") && (
        <div {...panel("library")} className="h-full overflow-y-auto pb-4">
          <InteractionScope active={shell.active === "library"}>
            <LibraryPage
              location={shell.libraryLocation}
              onLocated={shell.consumeLibraryLocation}
            />
          </InteractionScope>
        </div>
      )}
      {shell.visited.has("assistant") && (
        <div
          {...panel("assistant")}
          // Beside the sidebar of a wide window the assistant fills the space.
          className={wide ? "h-full" : "h-full px-4 pb-4 pt-1"}
        >
          <InteractionScope active={shell.active === "assistant"}>
            <AccountAssistantPanel
              enabled={Boolean(localSignedIn)}
              onLoginRequested={onLoginRequested}
            />
          </InteractionScope>
        </div>
      )}

      {/* 首次发现新版本的提示对话框。侧栏的 UpdatePill 常驻（hasUpdate &&
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
          // "稍后"：仅关闭弹窗，侧栏的 UpdatePill 保留。
          markPrompted();
        }}
      />
    </ShellFrame>
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

function LocalToolsPanel({
  signedIn,
  onLoginRequested,
}: {
  signedIn?: boolean;
  onLoginRequested?: () => void;
}) {
  const { t } = useTranslation();
  const catalog = useRef<InstallCatalogHandle>(null);
  const wide = useShell().layout === "wide";
  useShellRefresh("tools", () => catalog.current?.refresh());
  // The page's one way on, so the brand button: sign in, then bind.
  const next = (
    <Button variant="primary" onClick={onLoginRequested}>
      {t(signedIn ? "toolCatalog.bindOptional" : "toolCatalog.login")}
    </Button>
  );
  if (!wide)
    return (
      <div className={PAGE_BODY}>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h1 className="text-ui font-semibold">
              {t("toolCatalog.localTitle")}
            </h1>
            <p className="text-note text-ink-muted">
              {t("toolCatalog.localDescription")}
            </p>
          </div>
          {next}
        </div>
        <OptionalToolsCatalog ref={catalog} boundTools={[]} />
      </div>
    );
  return (
    <>
      <PageHeader
        title={t("toolCatalog.localTitle")}
        description={t("toolCatalog.localDescription")}
        actions={
          <>
            {next}
            <ShellRefreshButton />
          </>
        }
      />
      <div className={PAGE_BODY}>
        <OptionalToolsCatalog ref={catalog} boundTools={[]} />
      </div>
    </>
  );
}
