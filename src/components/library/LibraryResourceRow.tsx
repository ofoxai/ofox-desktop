import { useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  FileText,
  MoreHorizontal,
  Plug,
  Sparkles,
} from "lucide-react";
import type { LibraryKind, ResourceTarget } from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ListRow } from "@/components/ui/list";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { LibraryTargets, type TargetDelta } from "./LibraryTargets";
import { LibraryResourceIcon } from "./LibraryResourceIcon";
import type { Resource } from "./LibrarySection";
export function LibraryResourceRow({
  kind,
  row,
  targets,
  busy,
  writable,
  selecting,
  selected,
  onSelected,
  delta,
  check,
  observedTools = [],
  located = false,
  recipeId,
  onToggle,
  onAction,
  expanded = false,
  onExpand,
}: {
  kind: LibraryKind;
  row: Resource;
  targets: ResourceTarget[];
  busy: boolean;
  writable: boolean;
  selecting: boolean;
  selected: boolean;
  onSelected: (selected: boolean) => void;
  delta?: TargetDelta;
  check?: { state: string; checkedAtMs: number; toolsCount: number };
  observedTools?: string[];
  located?: boolean;
  recipeId?: string;
  expanded?: boolean;
  onExpand?: (trigger: HTMLElement) => void;
  onToggle: (target: ResourceTarget) => void;
  onAction: (
    action:
      | "edit"
      | "targets"
      | "details"
      | "delete"
      | "copy"
      | "update"
      | "check"
      | "export",
    trigger: HTMLElement,
  ) => void;
}) {
  const { t, i18n } = useTranslation();
  const menu = useRef<HTMLButtonElement>(null);
  const pendingAction = useRef<{
    action: Parameters<typeof onAction>[0];
    trigger: HTMLElement;
  }>();
  const latestAction = useRef(onAction);
  latestAction.current = onAction;
  const Icon =
    kind === "instructions" ? FileText : kind === "mcp" ? Plug : Sparkles;
  const summary =
    "content" in row
      ? row.content
          .split(/\r?\n/)
          .map((line) => line.replace(/^\s*[#>*-]+\s*/, "").trim())
          .find(Boolean)
      : "description" in row
        ? row.description
        : "transport" in row
          ? `${row.displaySummary || row.serviceHost || (row.transport === "stdio" ? t("library.workbench.localConnection") : t("library.workbench.remoteConnection"))}${row.credentialState === "needsMigration" ? ` · ${t("library.mcpNeedsValues")}` : ""}`
          : "";
  const checkTime =
    check && Number.isFinite(check.checkedAtMs)
      ? new Intl.DateTimeFormat(i18n.language, {
          month: "numeric",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        }).format(check.checkedAtMs)
      : undefined;
  const summaryText = [
    summary || t("library.workbench.noSummary"),
    check && checkTime
      ? t("library.rowCheck.last", {
          state: t(
            check.state === "blocked"
              ? "library.workbench2.nativeCheck"
              : `library.mcpCheckState.${check.state}`,
            {
              defaultValue: t("library.unknown"),
            },
          ),
          time: checkTime,
        })
      : "",
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <ListRow
      id={`library-resource-${row.id}`}
      data-testid={`library-resource-${row.id}`}
      data-located={located}
      style={{ scrollMarginTop: "var(--library-scroll-offset, 96px)" }}
      tabIndex={-1}
      className={`scroll-mt-24 flex-wrap gap-x-3 gap-y-2 py-3 outline-none ${located ? "bg-brand/5 ring-1 ring-inset ring-brand/40" : ""}`}
    >
      {selecting && (
        <Checkbox
          checked={selected}
          disabled={busy}
          aria-label={t("library.workbench.selectNamed", { name: row.name })}
          onCheckedChange={(checked) => onSelected(checked === true)}
        />
      )}
      <span
        aria-hidden
        className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-pill text-ink-muted"
      >
        {kind !== "instructions" ? (
          <LibraryResourceIcon
            recipeId={"iconId" in row ? row.iconId || recipeId : recipeId}
            kind={kind}
            className="h-9 w-9"
          />
        ) : (
          <Icon className="h-5 w-5" />
        )}
      </span>
      <div className="min-w-0 flex-1 basis-[180px]">
        <button
          type="button"
          className="block max-w-full truncate text-left text-ui font-medium outline-none hover:text-brand focus-visible:ring-2 focus-visible:ring-brand"
          title={row.name}
          aria-expanded={onExpand ? expanded : undefined}
          aria-controls={onExpand ? `library-inline-${row.id}` : undefined}
          aria-label={
            onExpand
              ? t(
                  expanded
                    ? "library.interaction.collapseNamed"
                    : "library.interaction.expandNamed",
                  { name: row.name },
                )
              : t(
                  kind === "skills"
                    ? "library.detailsNamed"
                    : "library.editNamed",
                  { name: row.name },
                )
          }
          onClick={(event) =>
            onExpand
              ? onExpand(event.currentTarget)
              : onAction(
                  kind === "skills" ? "details" : "edit",
                  event.currentTarget,
                )
          }
          disabled={busy || (kind !== "skills" && !writable)}
        >
          {row.name}
          {onExpand && (
            <ChevronDown
              aria-hidden
              className={`ml-1 inline h-3 w-3 transition-transform ${expanded ? "rotate-180" : ""}`}
            />
          )}
        </button>
        <p className="truncate text-note text-ink-muted" title={summaryText}>
          {summaryText}
        </p>
      </div>
      <div className="order-3 flex w-full flex-wrap items-center gap-2 pl-12 min-[1000px]:order-none min-[1000px]:w-auto min-[1000px]:pl-0">
        <LibraryTargets
          name={row.name}
          targets={targets}
          observedTools={observedTools}
          delta={delta}
          busy={busy || !writable}
          onToggle={onToggle}
        />
      </div>
      <div className="flex shrink-0 items-center gap-1">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              ref={menu}
              size="icon"
              variant="ghost"
              aria-label={t("library.workbench.moreNamed", { name: row.name })}
              disabled={busy}
            >
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            onCloseAutoFocus={(event) => {
              event.preventDefault();
              const pending = pendingAction.current;
              pendingAction.current = undefined;
              if (pending)
                queueMicrotask(() =>
                  latestAction.current(pending.action, pending.trigger),
                );
              else menu.current?.focus();
            }}
          >
            {(kind === "skills"
              ? ["details", "targets", "update", "export", "delete"]
              : kind === "mcp"
                ? ["edit", "targets", "copy", "check", "export", "delete"]
                : ["edit", "targets", "copy", "export", "delete"]
            ).map((action) => (
              <DropdownMenuItem
                key={action}
                aria-label={t(
                  action === "check"
                    ? "library.mcpCheckNamed"
                    : action === "update"
                      ? "library.updateNamed"
                      : action === "delete"
                        ? "library.deleteNamed"
                        : action === "targets"
                          ? "library.assignNamed"
                          : action === "copy" && kind === "mcp"
                            ? "library.copyMcpNamed"
                            : action === "edit"
                              ? "library.editNamed"
                              : action === "details"
                                ? "library.detailsNamed"
                                : action === "export"
                                  ? "library.workbench.exportResource"
                                  : `library.${action}`,
                  { name: row.name },
                )}
                disabled={
                  (action !== "details" && !writable) ||
                  (action === "update" && row.revision === undefined)
                }
                onSelect={() => {
                  if (!menu.current) return;
                  if (action === "export") onAction("export", menu.current);
                  else
                    pendingAction.current = {
                      action: action as Parameters<typeof onAction>[0],
                      trigger: menu.current,
                    };
                }}
                className={action === "delete" ? "text-danger" : undefined}
              >
                {t(
                  action === "check"
                    ? "library.mcpCheck"
                    : action === "update"
                      ? "library.skillUpdate.check"
                      : action === "copy" && kind === "mcp"
                        ? "library.copyTemplate"
                        : action === "delete"
                          ? "library.remove"
                          : action === "targets"
                            ? "library.assign"
                            : action === "export"
                              ? "library.workbench.exportResource"
                              : `library.${action}`,
                )}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </ListRow>
  );
}
