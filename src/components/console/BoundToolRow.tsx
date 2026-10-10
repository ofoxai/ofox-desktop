import {
  AppWindow,
  ArrowDownToLine,
  BarChart3,
  Download,
  ExternalLink,
  Loader2,
  RefreshCw,
  SlidersHorizontal,
  Terminal,
  Wrench,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { TOOL_META } from "@/config/toolMeta";
import { ToolBadge } from "@/components/tools/ToolBadge";
import type { ToolBindingStatus } from "@/lib/api/ofoxBind";
import type { InstallationStatus } from "@/lib/api/toolUpdates";

export interface BoundToolRowData {
  id: string;
  abbr: string;
  label: string;
  color: string;
  version: string | null;
  installationKind?: "desktopApp" | "cli";
  installationStatus: InstallationStatus;
  installationError: string | null;
  binding: ToolBindingStatus;
}

export type ToolRowAction = "retry" | "install" | "restore" | "manage" | "open";

/** Installation problems take precedence over configuration problems. */
export function toolRowAction(tool: BoundToolRowData): ToolRowAction | null {
  if (tool.installationStatus === "unknown") return "retry";
  if (tool.installationStatus === "notInstalled" || tool.installationError)
    return "install";
  if (tool.binding.status === "missing") return "restore";
  if (tool.binding.status === "unknown") return "retry";
  if (tool.binding.status === "modified") return null;
  const meta = TOOL_META[tool.id];
  return meta?.cliBin || meta?.launchKind === "desktopApp" ? "open" : null;
}

interface BoundToolRowProps {
  tool: BoundToolRowData;
  keyLabel?: string;
  model?: string;
  hasAnalytics: boolean;
  /** The newer version that is out, if this installation is behind. */
  updateTo: string | null;
  /** The update leaves Ofox: the app's own updater or the vendor's page. */
  updateExternal?: boolean;
  updating?: boolean;
  /** Another update runs; this one waits its turn. */
  updateLocked?: boolean;
  canInstall: boolean;
  busy: boolean;
  progress?: string;
  onAnalytics: () => void;
  onUpdate: () => void;
  onAction: (action: ToolRowAction) => void;
  onManage: () => void;
}

export default function BoundToolRow({
  tool,
  keyLabel,
  model,
  hasAnalytics,
  updateTo,
  updateExternal = false,
  updating = false,
  updateLocked = false,
  canInstall,
  busy,
  progress,
  onAnalytics,
  onUpdate,
  onAction,
  onManage,
}: BoundToolRowProps) {
  const { t } = useTranslation();
  const action = toolRowAction(tool);
  const download = action === "install" && !canInstall;
  const stateKey =
    tool.installationStatus === "unknown"
      ? "detectionFailed"
      : tool.installationStatus === "notInstalled"
        ? "notInstalled"
        : tool.installationError
          ? "installationBroken"
          : tool.binding.status !== "configured"
            ? `binding.${tool.binding.status}`
            : tool.binding.envOverrides.length
              ? "binding.envOverride"
              : null;
  const actionKey =
    action === "install"
      ? download
        ? TOOL_META[tool.id]?.downloadUrl ||
          tool.installationKind === "desktopApp"
          ? "download"
          : "upstreamInstructions"
        : tool.installationStatus === "notInstalled"
          ? "reinstall"
          : "repairInstall"
      : action === "retry"
        ? "retryDetection"
        : action === "restore"
          ? "restoreBinding"
          : action;
  const neutralButton =
    "inline-flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1 text-[12px] font-medium text-muted-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60";
  const actionIcon =
    action === "retry" ? (
      <RefreshCw className="h-3.5 w-3.5" />
    ) : action === "install" && download ? (
      <Download className="h-3.5 w-3.5" />
    ) : action === "open" ? (
      TOOL_META[tool.id]?.launchKind === "desktopApp" ||
      tool.installationKind === "desktopApp" ? (
        <AppWindow className="h-3.5 w-3.5" />
      ) : (
        <Terminal className="h-3.5 w-3.5" />
      )
    ) : (
      <Wrench className="h-3.5 w-3.5" />
    );

  return (
    <div
      role="group"
      aria-label={tool.label}
      className="flex flex-wrap items-center gap-3 border-b border-border px-4 py-3 last:border-b-0"
    >
      <ToolBadge toolId={tool.id} size={36} rounded="xl" />
      <div className="min-w-0 flex-1 basis-48">
        <div className="flex flex-wrap items-center gap-2 text-[13px] font-medium text-foreground">
          <span>{tool.label}</span>
          {tool.version && (
            <span className="text-[11px] font-normal text-muted-foreground">
              {tool.installationKind === "desktopApp"
                ? `${t("toolLifecycle.desktopApp")} · `
                : ""}
              v{tool.version}
            </span>
          )}
          {(updating ||
            (updateTo &&
              tool.installationStatus === "installed" &&
              !tool.installationError)) && (
            <button
              type="button"
              onClick={onUpdate}
              disabled={busy || updating || updateLocked}
              aria-busy={updating}
              title={updateTo ? `v${tool.version} → v${updateTo}` : undefined}
              className="inline-flex h-5 items-center gap-1 rounded-full border border-border bg-background pl-1.5 pr-2 text-[11px] font-medium text-foreground/80 transition hover:bg-accent hover:text-foreground active:scale-95 disabled:cursor-not-allowed disabled:opacity-60"
            >
              {updating ? (
                <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />
              ) : updateExternal ? (
                <ExternalLink className="h-3 w-3 text-muted-foreground" />
              ) : (
                <ArrowDownToLine className="h-3 w-3 text-muted-foreground" />
              )}
              {updating
                ? t("toolUpdates.updating")
                : t("toolUpdates.updateTo", { version: updateTo })}
            </button>
          )}
          {stateKey && (
            <span className="rounded-md bg-orange-50 px-1.5 py-0.5 text-[11px] font-normal text-orange-700 dark:bg-orange-950/40 dark:text-orange-300">
              {t(`toolLifecycle.${stateKey}`)}
            </span>
          )}
        </div>
        {(tool.id === "chatgpt" || keyLabel || model !== undefined) && (
          <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
            <span>
              {tool.id === "chatgpt"
                ? t("modelCompatibility.chatgptCodexMode")
                : (keyLabel ?? "—")}
            </span>
            <span className="mx-1.5 opacity-50">·</span>
            <span>
              {model ||
                t(
                  tool.id === "chatgpt"
                    ? "modelCompatibility.chatgptCodexUnset"
                    : "toolLifecycle.modelUnset",
                )}
            </span>
          </div>
        )}
      </div>
      <div className="ml-auto flex max-w-full flex-wrap items-center justify-end gap-1.5">
        {hasAnalytics && (
          <button onClick={onAnalytics} className={neutralButton}>
            <BarChart3 className="h-3.5 w-3.5" />
            {t("toolLifecycle.analytics")}
          </button>
        )}
        {action && actionKey && (
          <button
            onClick={() => onAction(action)}
            disabled={busy}
            className={
              action === "open" || action === "retry"
                ? neutralButton
                : "inline-flex items-center gap-1.5 rounded-lg bg-orange-500 px-2.5 py-1 text-[12px] font-medium text-white hover:bg-orange-600 disabled:cursor-not-allowed disabled:opacity-60"
            }
          >
            {busy ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              actionIcon
            )}
            {progress || t(`toolLifecycle.${actionKey}`)}
          </button>
        )}
        <button onClick={onManage} disabled={busy} className={neutralButton}>
          <SlidersHorizontal className="h-3.5 w-3.5" />
          {t("toolLifecycle.manage")}
        </button>
      </div>
    </div>
  );
}
