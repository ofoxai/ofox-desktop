import {
  AppWindow,
  ArrowDownToLine,
  BarChart3,
  Download,
  Ellipsis,
  ExternalLink,
  Loader2,
  RefreshCw,
  SlidersHorizontal,
  Terminal,
  Wrench,
} from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { TOOL_META } from "@/config/toolMeta";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ListRow } from "@/components/ui/list";
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

/** How a tool was installed, as the update check names it. */
const INSTALL_METHODS = [
  "homebrew",
  "npm",
  "pnpm",
  "native",
  "sparkle",
  "msstore",
];

interface BoundToolRowProps {
  tool: BoundToolRowData;
  keyLabel?: string;
  model?: string;
  modelControl?: ReactNode;
  hasAnalytics: boolean;
  /** How this installation was installed, when the update check knows. */
  installVia?: string | null;
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

/**
 * magpie's agent row: the tool, its version and state on the first line,
 * its key and model below; one action, 管理 and a quiet ⋯ menu at the end.
 */
export default function BoundToolRow({
  tool,
  keyLabel,
  model,
  modelControl,
  hasAnalytics,
  installVia,
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
  const actionIcon =
    action === "retry" ? (
      <RefreshCw aria-hidden className="h-3.5 w-3.5" />
    ) : action === "install" && download ? (
      <Download aria-hidden className="h-3.5 w-3.5" />
    ) : action === "open" ? (
      TOOL_META[tool.id]?.launchKind === "desktopApp" ||
      tool.installationKind === "desktopApp" ? (
        <AppWindow aria-hidden className="h-3.5 w-3.5" />
      ) : (
        <Terminal aria-hidden className="h-3.5 w-3.5" />
      )
    ) : (
      <Wrench aria-hidden className="h-3.5 w-3.5" />
    );
  const versionTitle =
    installVia && INSTALL_METHODS.includes(installVia)
      ? t("toolLifecycle.installedVia", {
          method: t(`toolLifecycle.installMethod.${installVia}`),
        })
      : undefined;
  const keyText =
    tool.id === "chatgpt" ? t("modelCompatibility.chatgptCodexMode") : keyLabel;

  return (
    <ListRow
      role="group"
      aria-label={tool.label}
      className="flex-wrap items-start gap-x-3 gap-y-2 py-2.5"
    >
      <ToolBadge
        toolId={tool.id}
        size={24}
        rounded="md"
        className="mt-px rounded-[6px]"
      />
      <div className="min-w-0 flex-1 basis-56">
        <div className="flex min-h-[26px] flex-wrap items-center gap-x-2 gap-y-1">
          <span className="truncate text-ui font-medium tracking-[-.005em] text-ink">
            {tool.label}
          </span>
          {tool.version && (
            <span
              title={versionTitle}
              className="whitespace-nowrap text-sub tabular-nums text-ink-muted"
            >
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
            <Button
              variant="pill"
              onClick={onUpdate}
              disabled={busy || updating || updateLocked}
              aria-busy={updating}
              title={updateTo ? `v${tool.version} → v${updateTo}` : undefined}
              className="h-5 gap-1 pl-1.5 pr-2"
            >
              {updating ? (
                <Loader2
                  aria-hidden
                  className="h-3 w-3 animate-spin text-ink-muted"
                />
              ) : updateExternal ? (
                <ExternalLink aria-hidden className="h-3 w-3 text-ink-muted" />
              ) : (
                <ArrowDownToLine
                  aria-hidden
                  className="h-3 w-3 text-ink-muted"
                />
              )}
              {updating
                ? t("toolUpdates.updating")
                : t("toolUpdates.updateTo", { version: updateTo })}
            </Button>
          )}
          {stateKey && (
            <span className="whitespace-nowrap rounded-full bg-warn-soft px-2 py-px text-note font-medium text-warn">
              {t(`toolLifecycle.${stateKey}`)}
            </span>
          )}
        </div>
        <div className="mt-0.5 flex min-w-0 items-start gap-1.5 text-sub text-ink-muted">
          {keyText && (
            <span className="min-w-0 truncate leading-[25px]">{keyText}</span>
          )}
          {keyText && (modelControl || model !== undefined) && (
            <span aria-hidden className="leading-[25px] text-ink-faint">
              ·
            </span>
          )}
          {modelControl ??
            (model !== undefined && (
              <span className="truncate leading-[25px]">
                {model ||
                  t(
                    tool.id === "chatgpt"
                      ? "modelCompatibility.chatgptCodexUnset"
                      : "toolLifecycle.modelUnset",
                  )}
              </span>
            ))}
        </div>
      </div>
      <div className="ml-auto flex flex-none items-center gap-1.5">
        {action && actionKey && (
          <Button
            variant={action === "install" ? "primary" : "secondary"}
            size="sm"
            onClick={() => onAction(action)}
            disabled={busy}
          >
            {busy ? (
              <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
            ) : (
              actionIcon
            )}
            {progress || t(`toolLifecycle.${actionKey}`)}
          </Button>
        )}
        <Button
          variant="secondary"
          size="sm"
          onClick={onManage}
          disabled={busy}
        >
          <SlidersHorizontal aria-hidden className="h-3.5 w-3.5" />
          {t("toolLifecycle.manage")}
        </Button>
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              variant="icon"
              aria-label={t("toolLifecycle.more")}
              title={t("toolLifecycle.more")}
              className="h-6 w-6 data-[state=open]:bg-pill data-[state=open]:text-ink"
            >
              <Ellipsis aria-hidden className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-[160px]">
            <DropdownMenuItem disabled={!hasAnalytics} onSelect={onAnalytics}>
              <BarChart3 aria-hidden className="h-3.5 w-3.5 text-ink-muted" />
              <span className="flex-1">{t("toolLifecycle.analytics")}</span>
              <span aria-hidden className="text-ink-faint">
                ↗
              </span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </ListRow>
  );
}
