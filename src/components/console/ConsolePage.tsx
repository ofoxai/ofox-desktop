import { useCallback, useEffect, useState } from "react";
import { Loader2, ArrowDownToLine, ChevronDown } from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { settingsApi } from "@/lib/api";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useOfoxAccount } from "@/hooks/useOfoxAccount";
import { useBoundTools, type BoundTool } from "@/hooks/useBoundTools";
import { ofoxAnalyticsUrl } from "@/lib/ofoxUrls";
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
import { BalanceCard } from "./BalanceCard";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { useUpdate } from "@/contexts/UpdateContext";
import { QuickModelPicker } from "@/components/model-picker/QuickModelPicker";
import { useModelSelection } from "@/hooks/useModelSelection";
import { SessionsPanel } from "@/components/sessions/SessionsPanel";
import { AppHeader } from "@/components/shell/AppHeader";
import {
  ShellProvider,
  shellPanelId,
  useShell,
  useShellRefresh,
  type ShellPage,
} from "@/components/shell/ShellContext";
import { UsagePage } from "@/components/usage-overview/UsagePage";
import { ProfilesMenu } from "@/components/model-profiles/ProfilesMenu";
import { useToolLaunch } from "@/hooks/useToolLaunch";
import { checkToolUpdates } from "@/hooks/useToolUpdates";
import { batchable, useInlineToolUpdates } from "@/hooks/useInlineToolUpdates";
import { useToolInstall } from "@/hooks/useToolInstall";
import { useToolInstallCapabilities } from "@/hooks/useToolInstallCapabilities";
import { launchErrorMessage } from "@/lib/launchErrors";
import BoundToolRow, { type ToolRowAction } from "./BoundToolRow";
import NodeUpgradeDialog from "@/components/tools/NodeUpgradeDialog";

interface ConsolePageProps {
  boundTools: string[];
  /** Called after the user adds new tools through "+ 添加" so MainApp can
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
  const { apex } = useOfoxApex();
  const { t } = useTranslation();
  const shell = useShell();
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
  // 应用自更新的首次发现弹窗；常驻的提示在顶栏（UpdatePill）。数据源 =
  // UpdateContext（自动检查走 R2 latest.json）。
  const {
    updateInfo,
    shouldPrompt: shouldPromptUpdate,
    markPrompted,
  } = useUpdate();
  const account = useOfoxAccount();
  const {
    tools,
    loading,
    listRefreshing,
    apiKeyIdByTool,
    apiKeyLabelByTool,
    refreshList,
    reload,
  } = useBoundTools(boundTools, account);
  // 顶栏的刷新在工具页 = 重新检测工具并检查更新。
  useShellRefresh("tools", refreshList);
  const [missingExpanded, setMissingExpanded] = useState(false);
  const installableTools = useToolInstallCapabilities();
  const inlineUpdates = useInlineToolUpdates();
  const toolUpdates = inlineUpdates.state;
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  // null = manage dialog is closed. Holds the snapshot of the tool row that
  // was clicked, so the dialog stays consistent even if `tools` reloads
  // mid-edit (e.g. monthly token refetch).
  const [manageTool, setManageTool] = useState<ManageToolTarget | null>(null);

  /** A page's panel: labelled by its tab, hidden while another is open. */
  const panel = (page: ShellPage) => ({
    role: "tabpanel",
    id: shellPanelId(page),
    "aria-label": t(`shell.tabs.${page}`),
    hidden: shell.active !== page,
  });

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

  return (
    <div className="flex h-screen w-full flex-col bg-canvas">
      <AppHeader />

      <main className="min-h-0 flex-1">
        <div
          {...panel("tools")}
          className="h-full overflow-y-auto px-[max(16px,calc(50%-600px))] pb-4 pt-1"
        >
          <BalanceCard account={account} className="mb-4" />

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
                    {t("toolLifecycle.needsAttention", {
                      count: attentionCount,
                    })}
                  </span>
                )}
              </div>
              <div className="flex items-center gap-2">
                <ProfilesMenu boundTools={boundTools} />
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
                          onClick={() =>
                            void inlineUpdates.updateAll(batchTools)
                          }
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
                      onClick={() =>
                        setMissingExpanded((expanded) => !expanded)
                      }
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
            <UsagePage account={account} />
          </div>
        )}
      </main>

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
          // MainApp updates `boundTools`, useBoundTools' scan reruns via
          // its `[boundTools]` dep.
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

      {/* 首次发现新版本的提示对话框。顶栏的 UpdatePill 常驻（hasUpdate &&
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
          // "稍后"：仅关闭弹窗，顶栏的 UpdatePill 保留。
          markPrompted();
        }}
      />
    </div>
  );
}
