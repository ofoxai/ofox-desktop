import { useRef, useState } from "react";
import { Loader2, Plus } from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { ListCard } from "@/components/ui/list";
import { ProfilesMenu } from "@/components/model-profiles/ProfilesMenu";
import {
  QuickModelPicker,
  modelNotes,
} from "@/components/model-picker/QuickModelPicker";
import NodeUpgradeDialog from "@/components/tools/NodeUpgradeDialog";
import { useShellRefresh } from "@/components/shell/ShellContext";
import { TOOL_META } from "@/config/toolMeta";
import { useBoundTools, type BoundTool } from "@/hooks/useBoundTools";
import { useInlineToolUpdates } from "@/hooks/useInlineToolUpdates";
import {
  useModelSelection,
  invalidateModelState,
} from "@/hooks/useModelSelection";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useToolInstall } from "@/hooks/useToolInstall";
import { useToolInstallCapabilities } from "@/hooks/useToolInstallCapabilities";
import { useToolLaunch } from "@/hooks/useToolLaunch";
import { checkToolUpdates } from "@/hooks/useToolUpdates";
import { settingsApi } from "@/lib/api";
import { launchErrorMessage } from "@/lib/launchErrors";
import { ofoxAnalyticsUrl } from "@/lib/ofoxUrls";
// NOTE: The tools list previously rendered a read-only "已锁定" badge for tools
// in `boundTools ∩ PROXY_SUPPORTED_TOOLS`, sourced from
// `useLockedTakeoverTools`. The badge was retired once the manage-tool dialog
// landed: every bound tool now exposes the same "管理" entry (model edit +
// 解除绑定), and 解除绑定 is the explicit way to turn takeover off. The hook
// itself is still imported by ProxyToggle / ProxyPanel to hide the on/off
// toggle row for those tools.
import AddToolsDialog from "./AddToolsDialog";
import { BalanceCard } from "./BalanceCard";
import BoundToolRow, { type ToolRowAction } from "./BoundToolRow";
import ManageToolDialog, { type ManageToolTarget } from "./ManageToolDialog";
import { UninstallToolDialog } from "./UninstallToolDialog";
import { MissingToolsFold } from "./MissingToolsFold";
import { UpdateAllLine } from "./UpdateAllLine";
import {
  OptionalToolsCatalog,
  type InstallCatalogHandle,
} from "@/components/tools/OptionalToolsCatalog";

interface ToolsPanelProps {
  boundTools: string[];
  account: OfoxAccount;
  /** Called after the user adds new tools through 添加 so MainApp can
   *  re-read localStorage and re-fire its takeover reconciliation sweep. */
  onBoundToolsChanged?: () => void;
}

/**
 * The 工具 page, as magpie's Agents page: the balance, a toolbar, the
 * update-all line, the bound tools and, folded under them, the ones no
 * longer installed.
 */
export function ToolsPanel({
  boundTools,
  account,
  onBoundToolsChanged,
}: ToolsPanelProps) {
  const { apex } = useOfoxApex();
  const { t } = useTranslation();
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
  const catalog = useRef<InstallCatalogHandle>(null);
  useShellRefresh("tools", async () => {
    await Promise.all([refreshList(), catalog.current?.refresh()]);
  });
  const [uninstallTool, setUninstallTool] = useState<string | null>(null);
  const [uninstallFromManage, setUninstallFromManage] = useState<string | null>(
    null,
  );
  const [missingExpanded, setMissingExpanded] = useState(false);
  const installableTools = useToolInstallCapabilities();
  const inlineUpdates = useInlineToolUpdates();
  const toolUpdates = inlineUpdates.state;
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const addToolTrigger = useRef<HTMLButtonElement>(null);
  const manageTrigger = useRef<HTMLElement | null>(null);
  // null = manage dialog is closed. Holds the snapshot of the tool row that
  // was clicked, so the dialog stays consistent even if `tools` reloads
  // mid-edit (e.g. monthly token refetch).
  const [manageTool, setManageTool] = useState<ManageToolTarget | null>(null);

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
      console.warn(`[ToolsPanel] repair ${toolId} exit=${code}`);
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
        subline={[
          tool.id === "chatgpt"
            ? t("modelCompatibility.chatgptCodexMode")
            : apiKeyLabelByTool[tool.id],
          ...modelNotes(snapshotByTool.get(tool.id), tool.id, t),
        ]}
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
            variant="field"
          />
        }
        hasAnalytics={!!apiKeyIdByTool[tool.id]}
        installVia={inlineUpdates.entryFor(tool)?.update_source}
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
        onUninstall={() => setUninstallTool(target)}
        onManage={(trigger) => {
          manageTrigger.current = trigger;
          setUninstallFromManage(target);
          setManageTool(tool);
        }}
        onAction={(action: ToolRowAction, trigger) => {
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
            manageTrigger.current = trigger ?? null;
            setUninstallFromManage(installTarget(tool));
            setManageTool(tool);
          }
        }}
      />
    );
  };

  return (
    <>
      <BalanceCard account={account} className="mb-3" />

      <div className="mb-2.5 flex min-h-[27px] items-center gap-2 px-0.5">
        <h2 className="text-ui font-semibold text-ink">
          {t("console.tools.title")}
        </h2>
        <span className="text-sub tabular-nums text-ink-muted">
          {tools.length}
        </span>
        {attentionCount > 0 && (
          <span className="whitespace-nowrap rounded-full bg-warn-soft px-2 py-px text-note font-medium text-warn">
            {t("toolLifecycle.needsAttention", { count: attentionCount })}
          </span>
        )}
        <span className="flex-1" />
        <ProfilesMenu boundTools={boundTools} />
        <Button
          ref={addToolTrigger}
          variant="secondary"
          onClick={() => setAddDialogOpen(true)}
        >
          <Plus aria-hidden className="h-3.5 w-3.5" />
          {t("console.tools.add")}
        </Button>
      </div>

      {loading && tools.length === 0 ? (
        // 首次加载——列表本身还没渲染过，给个占位条避免空白
        <ListCard>
          <p role="status" className="py-8 text-center text-ui text-ink-muted">
            {t("console.tools.loading")}
          </p>
        </ListCard>
      ) : tools.length === 0 ? (
        <ListCard>
          <p className="py-8 text-center text-ui text-ink-muted">
            {t("console.tools.empty")}
          </p>
        </ListCard>
      ) : (
        // 列表始终保持渲染，刷新中叠遮罩——避免刷新时整段消失再回来
        // 的视觉跳变，尤其是用户已经选好滚动位置时。
        <div className="relative">
          {repairError && (
            <div
              role="alert"
              className="mb-2.5 flex items-start justify-between gap-3 rounded-card border border-danger/30 bg-danger-soft px-3 py-2 text-ctl text-danger"
            >
              <span>
                {t("console.repairFailed", {
                  tool:
                    TOOL_META[repairError.toolId]?.label ?? repairError.toolId,
                  message: repairError.message,
                })}
              </span>
              <button
                type="button"
                onClick={clearRepairError}
                className="flex-none font-medium underline-offset-2 hover:underline"
              >
                {t("console.repairDismiss")}
              </button>
            </div>
          )}
          <UpdateAllLine tools={installedTools} updates={inlineUpdates} />
          <ListCard>
            {installedTools.length > 0 ? (
              installedTools.map(renderTool)
            ) : (
              <p className="px-4 py-6 text-center text-ui text-ink-muted">
                {t("toolLifecycle.noInstalledBoundTools")}
              </p>
            )}
          </ListCard>
          <MissingToolsFold
            tools={missingTools}
            open={missingExpanded}
            onOpenChange={setMissingExpanded}
            renderTool={renderTool}
          />
          {(loading || listRefreshing) && (
            <div
              aria-busy="true"
              className="pointer-events-none absolute inset-0 flex items-center justify-center rounded-list bg-canvas/60 backdrop-blur-[1px]"
            >
              <span className="inline-flex items-center gap-2 rounded-ctl bg-card px-3 py-1.5 text-ctl text-ink-muted shadow-pop">
                <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
                {t("console.tools.refreshing")}
              </span>
            </div>
          )}
        </div>
      )}

      <OptionalToolsCatalog boundTools={boundTools} ref={catalog} />
      <AddToolsDialog
        open={addDialogOpen}
        onOpenChange={setAddDialogOpen}
        alreadyBound={boundTools}
        onManageBound={(id) => {
          const tool = tools.find((entry) => entry.id === id);
          if (tool) {
            manageTrigger.current = addToolTrigger.current;
            setAddDialogOpen(false);
            setUninstallFromManage(installTarget(tool));
            setManageTool(tool);
          }
        }}
        onAdded={() => {
          // localStorage is already updated by bindTools(); notify the
          // parent so MainApp's `boundTools` state re-reads and the
          // reconciliation useEffect re-fires for the newly-bound tools.
          // The list itself doesn't need a manual reload: when MainApp
          // updates `boundTools`, useBoundTools' scan reruns via its
          // `[boundTools]` dep.
          onBoundToolsChanged?.();
        }}
      />

      <ManageToolDialog
        tool={manageTool}
        returnFocus={manageTrigger.current}
        onUninstall={() => setUninstallTool(uninstallFromManage)}
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

      <UninstallToolDialog
        tool={uninstallTool}
        onOpenChange={(open) => {
          if (!open) setUninstallTool(null);
        }}
        onCompleted={() => {
          onBoundToolsChanged?.();
          void reload();
          void checkToolUpdates();
          void invalidateModelState();
        }}
      />
      {inlineUpdates.confirmDialog}

      <NodeUpgradeDialog
        prompt={repairNodePrompt}
        onConfirm={() => void confirmRepairNodeUpgrade()}
        onCancel={dismissRepairNodePrompt}
      />
    </>
  );
}
