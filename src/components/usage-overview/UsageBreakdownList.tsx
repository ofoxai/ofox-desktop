import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Ellipsis } from "lucide-react";
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
interface Row extends UsageTotals {
  id: string;
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
}: {
  kind: "tools" | "models";
  rows: Row[];
  total: number;
  onFilter?: (id: string) => void;
}) {
  const { t } = useTranslation();
  const key = `ofox.usageHidden.${kind}`;
  const [hidden, setHidden] = useState(() => readHidden(key));
  const [open, setOpen] = useState(false);
  const set = (ids: string[]) => {
    setHidden(ids);
    localStorage.setItem(key, JSON.stringify(ids));
  };
  const label = (r: Row) =>
    kind === "tools" ? (TOOL_META[r.id]?.label ?? r.id) : r.id;
  const hiddenRows = rows.filter((r) => hidden.includes(r.id));
  return (
    <ListCard>
      <ListHead>
        <h2 className="font-medium text-ink-2">{t(`usagePage.${kind}`)}</h2>
      </ListHead>
      {rows
        .filter((r) => !hidden.includes(r.id))
        .map((r) => (
          <ListRow key={r.id}>
            {kind === "tools" && <ToolBadge toolId={r.id} size={24} />}
            <div className="min-w-0 flex-1 space-y-1">
              <p
                className={
                  kind === "models"
                    ? "truncate font-mono text-sub"
                    : "truncate text-ui font-medium"
                }
                title={label(r)}
              >
                {onFilter ? (
                  <button
                    type="button"
                    className="block w-full truncate text-left hover:text-brand-ink focus-visible:outline-brand"
                    title={label(r)}
                    onClick={() => onFilter(r.id)}
                  >
                    {label(r)}
                  </button>
                ) : (
                  label(r)
                )}
              </p>
              <ShareBar value={total > 0 ? r.totalTokens / total : 0} />
            </div>
            <div
              className="shrink-0 text-right tabular-nums"
              title={t("usagePage.io", {
                input: fmtTokens(r.inputTokens),
                output: fmtTokens(r.outputTokens),
              })}
            >
              <b className="text-sub">{fmtTokens(r.totalTokens)}</b>
              <p className="hidden text-note text-ink-faint xl:block">
                {t("usagePage.io", {
                  input: fmtTokens(r.inputTokens),
                  output: fmtTokens(r.outputTokens),
                })}
              </p>
            </div>
            <span
              className="min-w-[58px] shrink-0 whitespace-nowrap text-right text-sub text-ink"
              title={r.estimate?.knownCostUsd ?? undefined}
            >
              {r.estimate
                ? formatUsageEstimate(r.estimate)
                : fmtCost(r.priced === false ? null : r.costUsd)}
            </span>
            <DropdownMenu modal={false}>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="icon"
                  size="icon"
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
          </ListRow>
        ))}
      {!!hiddenRows.length && (
        <div className="px-3 py-1.5">
          <ListFold open={open} onOpenChange={setOpen}>
            {t("usagePage.hiddenCount", { count: hiddenRows.length })}
          </ListFold>
          {open &&
            hiddenRows.map((r) => (
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
        </div>
      )}
    </ListCard>
  );
}
