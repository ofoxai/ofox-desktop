import {
  AppWindow,
  ArrowDownToLine,
  Download,
  Ellipsis,
  ExternalLink,
  Loader2,
  RefreshCw,
  SlidersHorizontal,
  Terminal,
  Wrench,
  Trash2,
} from "lucide-react";
import { useId, useRef, useState, type ReactNode } from "react";
import { useUninstallPreview } from "@/hooks/useUninstallPreview";
import { useTranslation } from "react-i18next";
import { TOOL_META } from "@/config/toolMeta";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ListRow } from "@/components/ui/list";
import { cn } from "@/lib/utils";
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
  /** The faint line under the name: the key, then what the model is doing. */
  subline?: string[];
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
  onAction: (action: ToolRowAction, trigger?: HTMLElement) => void;
  onManage: (trigger: HTMLElement) => void;
  onUninstall?: () => void;
}

/**
 * The bound list's columns, which every row shares through CSS subgrid so the
 * model fields and the actions line up down the list, as magpie's `.fields`
 * do. The first and last columns carry the rows' side padding, so the hairline
 * between rows still runs edge to edge. Narrower than the window's minimum
 * (zoomed in), the model drops under the name.
 */
export const TOOL_LIST_GRID =
  "grid grid-cols-[auto_minmax(0,1fr)_minmax(180px,240px)_auto] gap-x-3 max-[720px]:grid-cols-[auto_minmax(0,1fr)_auto]";

/** Without subgrid each row keeps the columns on its own. */
const ROW_FALLBACK_COLUMNS =
  "supports-[not(grid-template-columns:subgrid)]:grid-cols-[38px_minmax(0,1fr)_240px_auto] max-[720px]:supports-[not(grid-template-columns:subgrid)]:grid-cols-[38px_minmax(0,1fr)_auto]";

/**
 * magpie's agent row, always two lines tall: the tool, its version and any
 * update or state pill on the first; the key and what its model is doing,
 * faint, on the second. The model and the actions sit in the list's shared
 * columns; account usage and secondary operations share the quiet ⋯ menu.
 */
export default function BoundToolRow({
  tool,
  subline = [],
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
  onUninstall,
}: BoundToolRowProps) {
  const { t } = useTranslation();
  const analyticsDescriptionId = useId();
  const uninstallDescriptionId = useId();
  const [menuOpen, setMenuOpen] = useState(false);
  const openingUninstall = useRef(false);
  const uninstallTarget =
    tool.id === "codex" && tool.installationKind === "desktopApp"
      ? "chatgpt"
      : tool.id;
  const hasUninstallAction =
    !!onUninstall && tool.installationStatus === "installed";
  const uninstallPreview = useUninstallPreview(
    uninstallTarget,
    menuOpen && hasUninstallAction,
  );
  const uninstallHint = uninstallPreview.available
    ? null
    : uninstallPreview.error
      ? t("toolUninstall.previewUnavailable")
      : !uninstallPreview.data
        ? t("common.loading")
        : !uninstallPreview.data.installed
          ? t("toolUninstall.notInstalled")
          : t("toolUninstall.blocked.unsupportedPlatform");
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
  const subText = subline.filter(Boolean).join(" · ");

  return (
    <ListRow
      role="group"
      aria-label={tool.label}
      className={cn(
        "col-span-full grid grid-cols-subgrid items-center gap-y-1.5 px-0",
        ROW_FALLBACK_COLUMNS,
      )}
    >
      <span data-column="badge" className="flex pl-[14px]">
        <ToolBadge
          toolId={tool.id}
          size={24}
          rounded="md"
          className="rounded-[6px]"
        />
      </span>
      <div data-column="name" className="min-w-0">
        <div data-line="name" className="flex h-5 min-w-0 items-center gap-2">
          <span
            className="min-w-[3em] truncate text-ui font-medium tracking-[-.005em] text-ink"
            title={tool.label}
          >
            {tool.label}
          </span>
          {tool.version && (
            <span
              title={versionTitle}
              className="min-w-0 shrink-[4] truncate whitespace-nowrap text-sub tabular-nums text-ink-muted"
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
              className="h-5 flex-none gap-1 pl-1.5 pr-2"
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
            <span className="inline-flex h-5 flex-none items-center whitespace-nowrap rounded-full bg-warn-soft px-2 text-note font-medium text-warn">
              {t(`toolLifecycle.${stateKey}`)}
            </span>
          )}
        </div>
        <p
          data-line="sub"
          className="mt-0.5 truncate text-sub text-ink-muted"
          title={subText || undefined}
        >
          {subText || "\u00a0"}
        </p>
      </div>
      <div
        data-column="model"
        className="min-w-0 max-[720px]:col-span-2 max-[720px]:col-start-2 max-[720px]:row-start-2 max-[720px]:max-w-[280px]"
      >
        {modelControl}
      </div>
      <span id={analyticsDescriptionId} className="sr-only">
        {t("toolLifecycle.analyticsHint")}
      </span>
      <div
        data-column="actions"
        className="flex items-center justify-end gap-1 pr-3 max-[720px]:col-start-3 max-[720px]:row-start-1"
      >
        {action && actionKey && (
          <Button
            variant={action === "install" ? "primary" : "secondary"}
            size="sm"
            className="mr-1"
            onClick={(event) => onAction(action, event.currentTarget)}
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
          variant="icon"
          onClick={(event) => onManage(event.currentTarget)}
          disabled={busy}
          aria-label={t("toolLifecycle.manage")}
          title={t("toolLifecycle.manage")}
          className="h-6 w-6"
        >
          <SlidersHorizontal aria-hidden className="h-3.5 w-3.5" />
        </Button>
        <DropdownMenu modal={false} open={menuOpen} onOpenChange={setMenuOpen}>
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
          <DropdownMenuContent
            align="end"
            className="min-w-[160px] max-w-[280px]"
            onCloseAutoFocus={(event) => {
              if (openingUninstall.current) {
                event.preventDefault();
                openingUninstall.current = false;
                onUninstall?.();
              }
            }}
          >
            <DropdownMenuItem
              disabled={!hasAnalytics}
              onSelect={onAnalytics}
              title={t("toolLifecycle.analyticsHint")}
              aria-describedby={analyticsDescriptionId}
            >
              <span className="flex-1">{t("toolLifecycle.analytics")}</span>
              <ExternalLink
                aria-hidden
                className="h-3.5 w-3.5 shrink-0 text-ink-muted"
              />
            </DropdownMenuItem>
            {hasUninstallAction && (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  disabled={busy || !uninstallPreview.available}
                  aria-label={t("toolUninstall.entry")}
                  aria-describedby={
                    uninstallHint ? uninstallDescriptionId : undefined
                  }
                  title={uninstallHint ?? undefined}
                  className="text-danger"
                  onSelect={() => {
                    openingUninstall.current = true;
                  }}
                >
                  <Trash2 aria-hidden className="h-3.5 w-3.5 shrink-0" />
                  <span className="min-w-0 flex-1">
                    <span className="block">{t("toolUninstall.entry")}</span>
                    {uninstallHint && (
                      <span
                        id={uninstallDescriptionId}
                        className="block whitespace-normal text-note text-ink-muted"
                      >
                        {uninstallHint}
                      </span>
                    )}
                  </span>
                </DropdownMenuItem>
              </>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </ListRow>
  );
}
