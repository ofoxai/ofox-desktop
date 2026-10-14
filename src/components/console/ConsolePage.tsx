import { useEffect, useState } from "react";
import {
  Settings,
  RefreshCw,
  Loader2,
  ArrowUpCircle,
  ArrowDownToLine,
  ChevronDown,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { settingsApi } from "@/lib/api";
import { isMac } from "@/lib/platform";
import { isOfoxBillingManager } from "@/lib/api/ofoxAuth";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useOfoxAccount } from "@/hooks/useOfoxAccount";
import { useBoundTools, type BoundTool } from "@/hooks/useBoundTools";
import {
  ofoxAnalyticsUrl,
  ofoxAvatarUrl,
  ofoxDashboardUrl,
  ofoxMarketingUrl,
  ofoxWalletUrl,
} from "@/lib/ofoxUrls";
import { TOOL_META } from "@/config/toolMeta";
// NOTE: ConsolePage previously rendered a read-only "已锁定" badge for tools
// in `boundTools ∩ PROXY_SUPPORTED_TOOLS`, sourced from
// `useLockedTakeoverTools`. The badge was retired once the manage-tool dialog
// landed: every bound tool now exposes the same "管理" entry (model edit +
// 解除绑定), and 解除绑定 is the explicit way to turn takeover off. The hook
// itself is still imported by ProxyToggle / ProxyPanel to hide the on/off
// toggle row for those tools.
import AddToolsDialog from "./AddToolsDialog";
import ManageToolDialog, { type ManageToolTarget } from "./ManageToolDialog";
import OfoxSettingsDialog from "./OfoxSettingsDialog";
// OfoxApexSwitch 不再在 ConsolePage 顶栏渲染——已迁入 OfoxSettingsDialog 的
// "区域" SectionCard。useOfoxApex() hook 仍保留：ofoxAnalyticsUrl(apex, ...)
// 还要它来拼"数据统计"按钮的目标 URL。
import { UserAvatar } from "@/components/UserAvatar";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { useUpdate } from "@/contexts/UpdateContext";
import { QuickModelPicker } from "@/components/model-picker/QuickModelPicker";
import { useModelSelection } from "@/hooks/useModelSelection";
import { ModelProfilesPanel } from "@/components/model-profiles/ModelProfilesPanel";
import { SessionsPanel } from "@/components/sessions/SessionsPanel";
import { useToolLaunch } from "@/hooks/useToolLaunch";
import { checkToolUpdates } from "@/hooks/useToolUpdates";
import { batchable, useInlineToolUpdates } from "@/hooks/useInlineToolUpdates";
import { useToolInstall } from "@/hooks/useToolInstall";
import { useToolInstallCapabilities } from "@/hooks/useToolInstallCapabilities";
import { launchErrorMessage } from "@/lib/launchErrors";
import BoundToolRow, { type ToolRowAction } from "./BoundToolRow";
import ofoxLogo from "@/assets/icons/ofox-logo.png";
import NodeUpgradeDialog from "@/components/tools/NodeUpgradeDialog";

interface ConsolePageProps {
  boundTools: string[];
  /** Called after the user adds new tools through "+ 添加" so MainApp can
   *  re-read localStorage and re-fire its takeover reconciliation sweep. */
  onBoundToolsChanged?: () => void;
}

export default function ConsolePage({
  boundTools,
  onBoundToolsChanged,
}: ConsolePageProps) {
  const { apex } = useOfoxApex();
  const { t } = useTranslation();
  const [activeView, setActiveView] = useState<
    "tools" | "profiles" | "sessions"
  >("tools");
  const [visited, setVisited] = useState({ profiles: false, sessions: false });
  const modelState = useModelSelection(boundTools);
  const snapshotByTool = new Map(
    modelState.data?.tools.map((snapshot) => [snapshot.toolId, snapshot]),
  );
  // "打开"按钮：CLI 拉到独立终端，桌面客户端由系统直接启动。
  const { launching: launchingTools, launch: launchTool } = useToolLaunch(
    (_toolId, error) => {
      toast.error(launchErrorMessage(error, t));
    },
  );
  // 应用自更新：底部栏提示按钮 + 首次发现弹窗。数据源 = UpdateContext
  // （自动检查走 R2 latest.json）。
  const {
    hasUpdate,
    isDismissed,
    updateInfo,
    shouldPrompt: shouldPromptUpdate,
    markPrompted,
    dismissUpdate,
  } = useUpdate();
  const account = useOfoxAccount();
  const { user } = account;
  const {
    tools,
    loading,
    listRefreshing,
    apiKeyIdByTool,
    apiKeyLabelByTool,
    refreshList,
    reload,
  } = useBoundTools(boundTools, account);
  const [missingExpanded, setMissingExpanded] = useState(false);
  const installableTools = useToolInstallCapabilities();
  const inlineUpdates = useInlineToolUpdates();
  const toolUpdates = inlineUpdates.state;
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false);
  // null = manage dialog is closed. Holds the snapshot of the tool row that
  // was clicked, so the dialog stays consistent even if `tools` reloads
  // mid-edit (e.g. monthly token refetch).
  const [manageTool, setManageTool] = useState<ManageToolTarget | null>(null);

  // 监听 tray popover 发出的"打开偏好设置"事件，把当前 dialog 拉起来。
  // popover 是独立 webview，无法直接调用本组件的 setState，因此走 Tauri
  // event 通信。channel 名与 BottomMenu.tsx 里的 emit 必须一致。
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    (async () => {
      try {
        const off = await listen("ofox-open-settings", () => {
          setSettingsDialogOpen(true);
        });
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
  }, []);

  // 「修复」= 用安装器重装这个工具。装完重新检测，卡片自己从「需修复」翻回正常。
  const {
    installing: repairing,
    install: repair,
    error: repairError,
    clearError: clearRepairError,
    progress: repairProgress,
    nodePrompt: repairNodePrompt,
    confirmNodeUpgrade: confirmRepairNodeUpgrade,
    dismissNodePrompt: dismissRepairNodePrompt,
  } = useToolInstall((toolId, code) => {
    if (code === 0) {
      void reload();
      void checkToolUpdates();
    } else {
      console.warn(`[ConsolePage] repair ${toolId} exit=${code}`);
    }
  });

  const installedTools = tools.filter(
    (tool) => tool.installationStatus !== "notInstalled",
  );
  const missingTools = tools.filter(
    (tool) => tool.installationStatus === "notInstalled",
  );
  const attentionCount = installedTools.filter(
    (tool) =>
      tool.installationStatus === "unknown" ||
      tool.installationError ||
      tool.binding.status !== "configured" ||
      tool.binding.envOverrides.length > 0,
  ).length;

  // magpie's "Update all" line: with two or more listed tools behind, one
  // line above the list says which and updates them one after another.
  const behindTools = installedTools.filter(
    (tool) => !tool.installationError && inlineUpdates.updateFor(tool),
  );
  const batchTools = behindTools.filter((tool) =>
    batchable(inlineUpdates.updateFor(tool)!),
  );
  const batchRunning = toolUpdates.batch && toolUpdates.queue.length > 1;
  const updatingNow = toolUpdates.busy ?? toolUpdates.queue[0];
  const showUpdatesBar = batchRunning || behindTools.length >= 2;

  const installTarget = (tool: BoundTool) =>
    tool.id === "codex" && tool.installationKind === "desktopApp"
      ? "chatgpt"
      : tool.id;
  const renderTool = (tool: BoundTool) => {
    const target = installTarget(tool);
    const canInstall = installableTools.includes(target);
    const update = inlineUpdates.updateFor(tool);
    const busy =
      repairing.has(target) ||
      launchingTools.has(tool.id) ||
      toolUpdates.busy === tool.id;
    return (
      <BoundToolRow
        key={tool.id}
        tool={tool}
        keyLabel={apiKeyLabelByTool[tool.id]}
        modelControl={
          <QuickModelPicker
            toolId={tool.id}
            snapshot={snapshotByTool.get(tool.id)}
            loading={modelState.isPending}
            error={modelState.isError}
            disabled={
              busy ||
              tool.installationStatus !== "installed" ||
              !!tool.installationError ||
              (tool.id === "codex" && tool.installationKind === "desktopApp")
            }
            onRetry={() => void modelState.refetch()}
          />
        }
        hasAnalytics={!!apiKeyIdByTool[tool.id]}
        updateTo={update?.latest_version ?? null}
        updateExternal={
          !!update &&
          (update.update_source === "sparkle" || !update.update_supported)
        }
        updating={toolUpdates.busy === tool.id}
        updateLocked={toolUpdates.batch}
        canInstall={canInstall}
        busy={busy}
        progress={repairProgress[target]?.name}
        onAnalytics={() => {
          void settingsApi
            .openExternal(ofoxAnalyticsUrl(apex, apiKeyIdByTool[tool.id]))
            .catch(() => toast.error(t("toolLifecycle.analyticsFailed")));
        }}
        onUpdate={() => void inlineUpdates.updateOne(tool)}
        onManage={() => setManageTool(tool)}
        onAction={(action: ToolRowAction) => {
          if (action === "retry") {
            void reload();
          } else if (action === "install") {
            if (canInstall) {
              void repair(target);
            } else {
              const meta = TOOL_META[target];
              const url = meta?.downloadUrl ?? meta?.projectUrl;
              if (url)
                void settingsApi
                  .openExternal(url)
                  .catch(() => toast.error(t("toolLifecycle.downloadFailed")));
            }
          } else if (action === "open") {
            void launchTool(tool.id);
          } else {
            setManageTool(tool);
          }
        }}
      />
    );
  };

  // 只有明确查到余额且不大于 0 才算没钱；还没查到（—）不报警。
  const balance = user?.balance?.balance;
  const balanceEmpty = typeof balance === "number" && balance <= 0;

  return (
    <div className="flex h-screen w-full flex-col bg-gradient-to-br from-orange-50/50 via-white to-orange-50/30 dark:from-neutral-950 dark:via-neutral-900 dark:to-neutral-950">
      {/* Title bar drag region — full 40px so it covers the macOS traffic-light
          row (titleBarStyle: "Overlay") with comfortable margin on either side.
          The previous 28px (h-7) was tall enough technically but felt fiddly:
          users would aim for the visible "Ofox" text and miss. Windows and
          Linux draw a native title bar with the same title, so skip it there. */}
      {isMac() && (
        <div className="relative h-10 shrink-0" data-tauri-drag-region="true">
          <div
            className="flex h-full items-center justify-center gap-1.5"
            data-tauri-drag-region="true"
          >
            <img
              src={ofoxLogo}
              alt=""
              aria-hidden
              className="h-4 w-4"
              data-tauri-drag-region="true"
            />
            <span
              className="text-[13px] font-medium text-muted-foreground"
              data-tauri-drag-region="true"
            >
              Ofox Desktop
            </span>
          </div>
          {/* Apex (region) switcher 已迁入"设置"弹窗的"区域"卡——顶栏只保留
              logo + 标题，更干净；apex 切换是低频操作，藏一层更合适。 */}
        </div>
      )}

      <div
        role="tablist"
        aria-label={t("modelExperience.navigation")}
        className="flex shrink-0 gap-1 border-b border-border px-5 pb-2"
      >
        {(["tools", "profiles", "sessions"] as const).map((view) => (
          <button
            key={view}
            id={`console-tab-${view}`}
            type="button"
            role="tab"
            aria-selected={activeView === view}
            tabIndex={activeView === view ? 0 : -1}
            aria-controls={`console-panel-${view}`}
            onKeyDown={(event) => {
              const tabs = Array.from(
                event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>(
                  '[role="tab"]',
                ) ?? [],
              );
              const index = tabs.indexOf(event.currentTarget);
              const next =
                event.key === "ArrowRight"
                  ? (index + 1) % tabs.length
                  : event.key === "ArrowLeft"
                    ? (index - 1 + tabs.length) % tabs.length
                    : event.key === "Home"
                      ? 0
                      : event.key === "End"
                        ? tabs.length - 1
                        : null;
              if (next !== null) {
                event.preventDefault();
                tabs[next]?.focus();
              }
            }}
            onClick={() => {
              setActiveView(view);
              if (view !== "tools")
                setVisited((previous) => ({ ...previous, [view]: true }));
            }}
            className={`rounded-lg px-3 py-1.5 text-[12px] font-medium ${activeView === view ? "bg-orange-500 text-white" : "text-muted-foreground hover:bg-accent hover:text-foreground"}`}
          >
            {t(`modelExperience.nav.${view}`)}
          </button>
        ))}
      </div>

      {/* Content */}
      <div
        role="tabpanel"
        id="console-panel-tools"
        aria-labelledby="console-tab-tools"
        hidden={activeView !== "tools"}
        className={`min-h-0 flex-1 overflow-y-auto px-5 pb-4 pt-4 ${activeView !== "tools" ? "hidden" : ""}`}
      >
        {/* Profile + Balance Card */}
        <div
          data-testid="balance-card"
          data-empty={balanceEmpty ? "true" : undefined}
          className={`mb-4 rounded-xl border px-5 py-4 ${
            balanceEmpty
              ? "border-red-500/40 bg-red-500/5"
              : "border-border bg-background/80"
          }`}
        >
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <UserAvatar
                avatarUrl={ofoxAvatarUrl(user?.avatar_url)}
                name={user?.name}
                email={user?.email}
                className="h-10 w-10"
                fallbackTextClassName="text-lg"
              />
              <div>
                <div className="text-[14px] font-semibold text-foreground">
                  {user?.name ?? (account.retrying ? "加载中…" : "用户")}
                </div>
                <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
                  {user?.email ? (
                    <span className="truncate">{user.email}</span>
                  ) : account.retrying ? (
                    <span className="inline-flex items-center gap-1">
                      <Loader2 className="h-3 w-3 animate-spin" />
                      正在加载用户信息
                      {account.retryAttempt > 0
                        ? `（${account.retryAttempt}/3）`
                        : "…"}
                    </span>
                  ) : account.loadFailed ? (
                    <button
                      type="button"
                      onClick={() => void account.retryUser()}
                      className="inline-flex items-center gap-1 text-orange-500 hover:text-orange-600 hover:underline"
                      title="无法加载用户信息，点击重试"
                    >
                      <RefreshCw className="h-3 w-3" />
                      无法加载用户信息，点击重试
                    </button>
                  ) : null}
                </div>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <div className="text-right">
                <div className="flex items-center justify-end gap-1.5 text-[11px] text-muted-foreground">
                  <span>OFOX 账户余额</span>
                  <button
                    onClick={() => void account.refreshBalance()}
                    disabled={account.balanceRefreshing}
                    title="刷新余额"
                    className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
                  >
                    <RefreshCw
                      className={`h-3 w-3 ${
                        account.balanceRefreshing ? "animate-spin" : ""
                      }`}
                    />
                  </button>
                </div>
                <div
                  className={`text-2xl font-bold ${
                    balanceEmpty ? "text-red-600" : "text-foreground"
                  }`}
                >
                  {formatBalance(user?.balance?.balance)}
                </div>
              </div>
              {isOfoxBillingManager(user) && (
                <button
                  onClick={() => settingsApi.openExternal(ofoxWalletUrl(apex))}
                  className={`rounded-lg px-4 py-1.5 text-[13px] font-medium text-white ${
                    balanceEmpty
                      ? "bg-orange-600 shadow-md ring-2 ring-orange-500/40 hover:bg-orange-700"
                      : "bg-orange-500 hover:bg-orange-600"
                  }`}
                >
                  充值
                </button>
              )}
            </div>
          </div>
        </div>

        {/* Tool List */}
        <div className="rounded-xl border border-border bg-background/80">
          <div className="flex items-center justify-between border-b border-border px-4 py-3">
            <div className="flex items-center gap-2">
              <span className="text-[14px] font-semibold text-foreground">
                绑定的工具
              </span>
              <span className="rounded-md bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                {tools.length}
              </span>
              {attentionCount > 0 && (
                <span className="rounded-md bg-red-50 px-2 py-0.5 text-[11px] font-medium text-red-500 dark:bg-red-950/50">
                  {t("toolLifecycle.needsAttention", { count: attentionCount })}
                </span>
              )}
            </div>
            <div className="flex items-center gap-2">
              <button
                onClick={refreshList}
                disabled={listRefreshing}
                title="刷新工具检测与状态"
                className="flex items-center gap-1.5 rounded-md border border-border px-3 py-1 text-[12px] text-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
              >
                <RefreshCw
                  className={`h-3.5 w-3.5 ${listRefreshing ? "animate-spin" : ""}`}
                />
                刷新
              </button>
              <button
                onClick={() => setAddDialogOpen(true)}
                className="rounded-md border border-border px-3 py-1 text-[12px] text-foreground hover:bg-accent"
              >
                + 添加
              </button>
            </div>
          </div>

          {loading && tools.length === 0 ? (
            // 首次加载——列表本身还没渲染过，给个占位条避免空白
            <div className="flex items-center justify-center py-8 text-sm text-muted-foreground">
              正在加载...
            </div>
          ) : tools.length === 0 ? (
            <div className="flex items-center justify-center py-8 text-sm text-muted-foreground">
              暂无绑定的工具
            </div>
          ) : (
            // 列表始终保持渲染，刷新中叠遮罩——避免刷新时整段消失再回来
            // 的视觉跳变，尤其是用户已经选好滚动位置时。
            <div className="relative">
              {repairError && (
                <div
                  role="alert"
                  className="mb-3 flex items-start justify-between gap-3 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-300"
                >
                  <span>
                    {t("console.repairFailed", {
                      tool:
                        TOOL_META[repairError.toolId]?.label ??
                        repairError.toolId,
                      message: repairError.message,
                    })}
                  </span>
                  <button
                    onClick={clearRepairError}
                    className="shrink-0 font-medium underline-offset-2 hover:underline"
                  >
                    {t("console.repairDismiss")}
                  </button>
                </div>
              )}
              {showUpdatesBar && (
                <div
                  role="status"
                  className="flex min-h-[40px] items-center gap-3 border-b border-border px-4 py-2 text-[12px] text-muted-foreground"
                >
                  <span className="min-w-0 flex-1 truncate">
                    {batchRunning
                      ? t("toolUpdates.batchProgress", {
                          tool: TOOL_META[updatingNow]?.label ?? updatingNow,
                          index: toolUpdates.queue.indexOf(updatingNow) + 1,
                          count: toolUpdates.queue.length,
                        })
                      : t("toolUpdates.manyAvailable", {
                          count: behindTools.length,
                          names: behindTools
                            .map((tool) => tool.label)
                            .join(", "),
                        })}
                  </span>
                  {batchRunning ? (
                    <Loader2
                      className="h-3.5 w-3.5 shrink-0 animate-spin"
                      aria-hidden
                    />
                  ) : (
                    batchTools.length >= 2 && (
                      <button
                        type="button"
                        disabled={toolUpdates.batch}
                        onClick={() => void inlineUpdates.updateAll(batchTools)}
                        title={batchTools
                          .map((tool) => {
                            const update = inlineUpdates.updateFor(tool);
                            return `${tool.label}: ${update?.version} → ${update?.latest_version}`;
                          })
                          .join("\n")}
                        className="inline-flex h-6 shrink-0 items-center gap-1 rounded-full border border-border bg-background pl-2 pr-2.5 text-[12px] font-medium text-foreground/80 transition hover:bg-accent hover:text-foreground active:scale-95 disabled:cursor-not-allowed disabled:opacity-60"
                      >
                        <ArrowDownToLine className="h-3.5 w-3.5 text-muted-foreground" />
                        {t("toolUpdates.updateAll")}
                      </button>
                    )
                  )}
                </div>
              )}
              {installedTools.length > 0 ? (
                installedTools.map(renderTool)
              ) : (
                <div className="px-4 py-6 text-center text-sm text-muted-foreground">
                  {t("toolLifecycle.noInstalledBoundTools")}
                </div>
              )}
              {missingTools.length > 0 && (
                <div className="border-t border-border">
                  <button
                    type="button"
                    aria-expanded={missingExpanded}
                    aria-controls="missing-bound-tools"
                    onClick={() => setMissingExpanded((expanded) => !expanded)}
                    className="flex w-full flex-wrap items-center gap-2 px-4 py-3 text-left text-[12px] text-muted-foreground hover:bg-accent"
                  >
                    <ChevronDown
                      className={`h-4 w-4 transition-transform ${missingExpanded ? "rotate-180" : ""}`}
                    />
                    <span className="font-medium text-foreground">
                      {t("toolLifecycle.missingTools", {
                        count: missingTools.length,
                      })}
                    </span>
                    <span>{t("toolLifecycle.bindingRetained")}</span>
                  </button>
                  {missingExpanded && (
                    <div id="missing-bound-tools">
                      {missingTools.map(renderTool)}
                    </div>
                  )}
                </div>
              )}
              {(loading || listRefreshing) && (
                <div
                  aria-busy="true"
                  className="pointer-events-none absolute inset-0 flex items-center justify-center bg-background/60 backdrop-blur-[1px]"
                >
                  <span className="inline-flex items-center gap-2 rounded-md bg-background/80 px-3 py-1.5 text-[12px] text-muted-foreground shadow-sm">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    刷新中…
                  </span>
                </div>
              )}
            </div>
          )}
        </div>
      </div>

      {visited.profiles && (
        <div
          role="tabpanel"
          id="console-panel-profiles"
          aria-labelledby="console-tab-profiles"
          hidden={activeView !== "profiles"}
          className={`min-h-0 flex-1 overflow-y-auto p-5 ${activeView !== "profiles" ? "hidden" : ""}`}
        >
          <ModelProfilesPanel boundTools={boundTools} />
        </div>
      )}
      {visited.sessions && (
        <div
          role="tabpanel"
          id="console-panel-sessions"
          aria-labelledby="console-tab-sessions"
          hidden={activeView !== "sessions"}
          className={`min-h-0 flex-1 ${activeView !== "sessions" ? "hidden" : ""}`}
        >
          <SessionsPanel className="h-full min-h-0" />
        </div>
      )}

      {/* Bottom Bar */}
      <div className="flex shrink-0 items-center justify-between border-t border-border px-5 py-2.5">
        <div className="flex items-center gap-4">
          <button
            onClick={() => setSettingsDialogOpen(true)}
            className="flex items-center gap-1.5 text-[12px] text-muted-foreground hover:text-foreground"
          >
            <Settings className="h-3.5 w-3.5" />
            设置
          </button>
          {hasUpdate && !isDismissed && (
            <div className="flex items-center gap-1">
              <button
                onClick={() => {
                  const target = updateInfo?.downloadUrl;
                  if (target) {
                    settingsApi.openExternal(target).catch((e) => {
                      console.error(
                        "[ConsolePage] open download url failed",
                        e,
                      );
                      toast.error(t("settings.openReleaseNotesFailed"));
                    });
                  }
                }}
                title={t("settings.updateAvailable", {
                  version: updateInfo?.availableVersion ?? "",
                })}
                className="flex items-center gap-1.5 text-[12px] font-medium text-emerald-600 hover:text-emerald-700 dark:text-emerald-400 dark:hover:text-emerald-300"
              >
                <ArrowUpCircle className="h-3.5 w-3.5" />
                {t("update.bannerTitle", { defaultValue: "发现新版本" })}
                {updateInfo?.availableVersion
                  ? ` v${updateInfo.availableVersion}`
                  : ""}
              </button>
              <button
                onClick={() => dismissUpdate()}
                title={t("update.skipThisVersion", {
                  defaultValue: "跳过此版本",
                })}
                className="rounded px-1 text-[11px] text-muted-foreground hover:text-foreground"
              >
                {t("update.skipThisVersion", { defaultValue: "跳过此版本" })}
              </button>
            </div>
          )}
        </div>
        <div className="flex items-center gap-4">
          <button
            onClick={() => settingsApi.openExternal(ofoxDashboardUrl(apex))}
            className="text-[12px] text-orange-500 hover:text-orange-600 hover:underline"
          >
            查看详细用量 ↗
          </button>
          <button
            onClick={() => settingsApi.openExternal(ofoxMarketingUrl(apex))}
            className="text-[12px] text-orange-500 hover:text-orange-600 hover:underline"
          >
            Ofox 网站 ↗
          </button>
        </div>
      </div>

      <AddToolsDialog
        open={addDialogOpen}
        onOpenChange={setAddDialogOpen}
        alreadyBound={boundTools}
        onManageBound={(id) => {
          const tool = tools.find((entry) => entry.id === id);
          if (tool) {
            setAddDialogOpen(false);
            setManageTool(tool);
          }
        }}
        onAdded={() => {
          // localStorage is already updated by bindTools(); notify the
          // parent so MainApp's `boundTools` state re-reads and the
          // reconciliation useEffect re-fires for the newly-bound tools.
          // ConsolePage itself doesn't need a manual reload: when
          // MainApp updates `boundTools`, our `loadData` reruns via the
          // `[boundTools]` dep already on the useEffect at line 211.
          onBoundToolsChanged?.();
        }}
      />

      <ManageToolDialog
        tool={manageTool}
        onOpenChange={(open) => {
          if (!open) setManageTool(null);
        }}
        onChanged={() => {
          onBoundToolsChanged?.();
          // Recovery does not change the bound set; reload its disk status too.
          // Generation checks reject scans captured before an unbind prop update.
          void reload();
        }}
      />

      {inlineUpdates.confirmDialog}

      <NodeUpgradeDialog
        prompt={repairNodePrompt}
        onConfirm={() => void confirmRepairNodeUpgrade()}
        onCancel={dismissRepairNodePrompt}
      />

      <OfoxSettingsDialog
        open={settingsDialogOpen}
        onOpenChange={setSettingsDialogOpen}
      />

      {/* 首次发现新版本的提示对话框。底部栏的提示按钮常驻（hasUpdate &&
          !isDismissed），这里只在本会话首次发现时弹一次，弹过即由
          markPrompted 关闭，避免每次定时复查都打扰。 */}
      <ConfirmDialog
        isOpen={shouldPromptUpdate}
        variant="info"
        title={t("update.promptTitle", { defaultValue: "发现新版本" })}
        message={t("update.promptMessage", {
          version: updateInfo?.availableVersion ?? "",
          defaultValue: "Ofox Desktop {{version}} 已发布，前往下载页获取更新。",
        })}
        confirmText={t("settings.goDownload", { defaultValue: "前往下载" })}
        cancelText={t("update.later", { defaultValue: "稍后" })}
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
          // "稍后"：仅关闭弹窗，底部栏提示按钮保留。
          markPrompted();
        }}
      />
    </div>
  );
}

// ==================== Helpers ====================

/**
 * Render the wallet figure. `null`/`undefined` collapses to `—` so the
 * layout doesn't shift between "loading" and "no balance returned".
 */
function formatBalance(value: number | null | undefined): string {
  if (typeof value !== "number" || Number.isNaN(value)) return "—";
  return `$${value.toFixed(2)}`;
}
