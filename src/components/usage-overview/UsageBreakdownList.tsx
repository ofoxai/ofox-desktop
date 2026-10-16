import { useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Ellipsis, X } from "lucide-react";
import { ListCard, ListHead, ListRow, ListFold } from "@/components/ui/list";
import { Button } from "@/components/ui/button";
import { ShareBar } from "@/components/ui/share-bar";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  DropdownMenuItem,
} from "@/components/ui/dropdown-menu";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { TOOL_META } from "@/config/toolMeta";
import type { UsageTotals } from "@/lib/api/usageOverview";
import { fmtTokens, fmtCost } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";
interface Row
  extends Pick<
    UsageTotals,
    "inputTokens" | "outputTokens" | "totalTokens" | "estimate"
  > {
  id: string;
  label?: string;
  path?: string | null;
  costUsd?: number;
  priced?: boolean;
}
function readHidden(key: string): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(key) || "[]");
    return Array.isArray(value)
      ? value.filter((v): v is string => typeof v === "string")
      : [];
  } catch {
    return [];
  }
}
export function UsageBreakdownList({
  kind,
  rows,
  total,
  onFilter,
  onRemove,
  storageKey,
  title,
  pageSize,
  compact = false,
  selectedIds = [],
  initialLimit,
  headerControl,
}: {
  kind: "tools" | "models" | "projects";
  rows: Row[];
  total: number;
  onFilter?: (id: string) => void;
  onRemove?: (id: string) => void;
  storageKey?: string;
  title?: string;
  pageSize?: number;
  compact?: boolean;
  selectedIds?: string[];
  initialLimit?: number;
  headerControl?: ReactNode;
}) {
  const { t } = useTranslation();
  const heading = useRef<HTMLHeadingElement>(null);
  const key = storageKey ?? `ofox.usageHidden.${kind}`;
  const [hidden, setHidden] = useState(() => readHidden(key));
  const [open, setOpen] = useState(false);
  const batchSize = pageSize ?? Infinity;
  const [visibleLimit, setVisibleLimit] = useState(initialLimit ?? batchSize);
  const [hiddenLimit, setHiddenLimit] = useState(batchSize);
  const set = (ids: string[]) => {
    setHidden(ids);
    localStorage.setItem(key, JSON.stringify(ids));
  };
  const label = (r: Row) =>
    kind === "tools" ? (TOOL_META[r.id]?.label ?? r.id) : (r.label ?? r.id);
  const hiddenIds = new Set(hidden);
  const hiddenRows = rows.filter((r) => hiddenIds.has(r.id));
  const visibleRows = rows.filter((r) => !hiddenIds.has(r.id));
  const barScale = compact
    ? Math.max(1, ...visibleRows.map((row) => row.totalTokens))
    : total;
  const compactColumns =
    kind === "projects"
      ? "grid-cols-[minmax(0,min(32%,14rem))_minmax(2.5rem,1fr)_auto_4.2em]"
      : "grid-cols-[minmax(0,max-content)_minmax(2.5rem,1fr)_auto_4.2em]";
  const fallbackColumns =
    kind === "projects"
      ? "supports-[not(grid-template-columns:subgrid)]:grid-cols-[minmax(0,min(32%,14rem))_minmax(2.5rem,1fr)_auto_4.2em]"
      : "supports-[not(grid-template-columns:subgrid)]:grid-cols-[minmax(0,max-content)_minmax(2.5rem,1fr)_auto_4.2em]";
  const actions = (r: Row) => (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="icon"
          size="icon"
          className={
            compact
              ? "h-5 w-5 bg-card opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100 [@media(hover:none)]:opacity-100"
              : undefined
          }
          aria-label={t("usagePage.actions", { name: label(r) })}
        >
          <Ellipsis className="h-3.5 w-3.5" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={() => set([...hidden, r.id])}>
          {t("usagePage.hide")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
  return (
    <ListCard
      className={
        compact
          ? "flex h-full min-w-0 flex-col divide-y-0 px-3.5 pb-2.5 pt-3"
          : undefined
      }
    >
      <ListHead
        className={
          compact
            ? "mb-2 min-h-5 justify-between gap-2 border-0 p-0 [*+&]:border-0"
            : undefined
        }
      >
        <h2
          ref={heading}
          tabIndex={-1}
          className="rounded-sm font-medium text-ink-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand/50"
        >
          {title ??
            t(
              kind === "projects"
                ? "usageSessions.projectRank"
                : `usagePage.${kind}`,
            )}
        </h2>
        {headerControl}
      </ListHead>
      <div
        role="list"
        className={
          compact
            ? `grid min-w-0 ${compactColumns} gap-x-2 gap-y-px`
            : undefined
        }
      >
        {visibleRows.slice(0, visibleLimit).map((r) => {
          const costText = r.estimate
            ? formatUsageEstimate(r.estimate)
            : fmtCost(r.priced === false ? null : r.costUsd);
          const partialPrice = costText.endsWith("+");
          return (
            <ListRow
              key={r.id}
              role="listitem"
              data-selected={selectedIds.includes(r.id) || undefined}
              className={
                compact
                  ? `group relative col-span-4 grid min-h-[28px] min-w-0 grid-cols-subgrid gap-x-2 rounded-md border-0 px-1.5 py-0.5 -mx-1.5 hover:bg-pill data-[selected=true]:bg-pill/60 data-[selected=true]:ring-1 data-[selected=true]:ring-inset data-[selected=true]:ring-brand/25 [*+&]:border-0 ${fallbackColumns}`
                  : "group"
              }
            >
              {kind === "tools" && !compact && (
                <ToolBadge toolId={r.id} size={24} />
              )}
              <div
                className={
                  compact
                    ? "flex min-w-0 items-center gap-1.5"
                    : "min-w-0 flex-1 space-y-1"
                }
              >
                {kind === "tools" && compact && (
                  <ToolBadge toolId={r.id} size={18} />
                )}
                <p
                  className={
                    kind === "models"
                      ? "min-w-0 truncate font-mono text-sub"
                      : "min-w-0 truncate text-sub font-medium"
                  }
                  title={r.path ? `${label(r)}\n${r.path}` : label(r)}
                >
                  {onFilter ? (
                    <button
                      type="button"
                      className="block min-w-0 w-full truncate rounded text-left hover:text-brand-ink focus-visible:outline-brand aria-pressed:text-brand-ink"
                      title={r.path ? `${label(r)}\n${r.path}` : label(r)}
                      aria-pressed={selectedIds.includes(r.id)}
                      onClick={() => onFilter(r.id)}
                    >
                      {label(r)}
                    </button>
                  ) : (
                    label(r)
                  )}
                </p>
                {selectedIds.includes(r.id) && onRemove && (
                  <Button
                    variant="icon"
                    size="icon"
                    className="h-4 w-4 shrink-0 text-brand-ink"
                    aria-label={t("usageExperience.removeFilter", {
                      name: label(r),
                    })}
                    title={t("usageExperience.removeFilter", {
                      name: label(r),
                    })}
                    onClick={() => {
                      // The updated cohort can remove this whole row. Hand
                      // focus to the card's stable title before that happens.
                      heading.current?.focus({ preventScroll: true });
                      onRemove(r.id);
                    }}
                  >
                    <X aria-hidden className="h-3 w-3" />
                  </Button>
                )}
                {r.path && !compact && (
                  <p
                    className="truncate font-mono text-note text-ink-faint"
                    title={r.path}
                  >
                    {r.path}
                  </p>
                )}
                {!compact && (
                  <ShareBar value={total > 0 ? r.totalTokens / total : 0} />
                )}
              </div>
              {compact && (
                <div className="relative min-w-0">
                  <ShareBar
                    value={barScale > 0 ? r.totalTokens / barScale : 0}
                  />
                  <div className="absolute right-0 top-1/2 -translate-y-1/2">
                    {actions(r)}
                  </div>
                </div>
              )}
              <div
                className="min-w-0 shrink-0 text-right tabular-nums"
                title={t("usagePage.io", {
                  input: fmtTokens(r.inputTokens),
                  output: fmtTokens(r.outputTokens),
                })}
              >
                <b className="text-sub">{fmtTokens(r.totalTokens)}</b>
                <p
                  className={
                    compact
                      ? "hidden"
                      : "hidden text-note text-ink-faint xl:block"
                  }
                >
                  {t("usagePage.io", {
                    input: fmtTokens(r.inputTokens),
                    output: fmtTokens(r.outputTokens),
                  })}
                </p>
              </div>
              <span
                className={
                  compact
                    ? "flex min-w-0 items-center justify-end whitespace-nowrap text-right text-note tabular-nums text-ok"
                    : "min-w-[58px] shrink-0 whitespace-nowrap text-right text-sub text-ink"
                }
                title={
                  r.estimate?.knownCostUsd == null
                    ? costText
                    : `${costText}\n${r.estimate.knownCostUsd}`
                }
              >
                <span className="min-w-0 truncate">
                  {partialPrice ? costText.slice(0, -1) : costText}
                </span>
                {partialPrice && <span className="shrink-0">+</span>}
              </span>
              {!compact && actions(r)}
            </ListRow>
          );
        })}
      </div>
      {visibleRows.length > visibleLimit && (
        <Button
          variant="ghost"
          className="m-2"
          onClick={() => setVisibleLimit((limit) => limit + batchSize)}
        >
          {t("usageSessions.showMore", {
            count: visibleRows.length - visibleLimit,
          })}
        </Button>
      )}
      {!rows.length && (
        <p className="p-4 text-sub text-ink-muted">
          {t("usageSessions.noRank")}
        </p>
      )}
      {!!hiddenRows.length && (
        <div className="px-3 py-1.5">
          <ListFold open={open} onOpenChange={setOpen}>
            {t("usagePage.hiddenCount", { count: hiddenRows.length })}
          </ListFold>
          {open &&
            hiddenRows.slice(0, hiddenLimit).map((r) => (
              <div
                key={r.id}
                className="flex items-center justify-between py-1 text-sub text-ink-muted"
              >
                <span className="truncate">{label(r)}</span>
                <Button
                  variant="ghost"
                  aria-label={t("usagePage.showItem", { name: label(r) })}
                  onClick={() => set(hidden.filter((id) => id !== r.id))}
                >
                  {t("usagePage.show")}
                </Button>
              </div>
            ))}
          {open && hiddenRows.length > hiddenLimit && (
            <Button
              variant="ghost"
              onClick={() => setHiddenLimit((limit) => limit + batchSize)}
            >
              {t("usageSessions.moreHidden", {
                count: hiddenRows.length - hiddenLimit,
              })}
            </Button>
          )}
        </div>
      )}
    </ListCard>
  );
}
