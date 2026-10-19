import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ExternalLink } from "lucide-react";
import { SegmentedControl } from "@/components/ui/segmented";
import { ListCard } from "@/components/ui/list";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { TOOL_META } from "@/config/toolMeta";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { settingsApi } from "@/lib/api";
import { getUsageOverview } from "@/lib/api/usageOverview";
import { fmtCost, fmtTokenCount } from "@/lib/format";
import { ofoxActivityUrl } from "@/lib/ofoxUrls";
import { queryClient } from "@/lib/query/queryClient";
import { cn } from "@/lib/utils";

type TrayPeriod = "today" | "7d";

/** The same quiet card the tool list sits on. */
const PANEL_CARD =
  "rounded-[10px] border-0 shadow-[0_0_0_1px_rgba(0,0,0,.05),0_1px_2px_rgba(0,0,0,.04)] dark:shadow-[inset_0_0_0_1px_rgba(255,255,255,.06)]";

/**
 * What the tools used, at a glance: today or the last seven days, the total
 * and the five heaviest tools. The full breakdown lives on the main window's
 * 用量 page and on the web.
 */
export default function TrayUsage() {
  const { t } = useTranslation();
  const { apex } = useOfoxApex();
  const [period, setPeriod] = useState<TrayPeriod>("today");
  // The usage page's own cache entry, so either view warms the other.
  const overview = useQuery(
    {
      queryKey: ["usageOverview", apex, period, null],
      queryFn: () => getUsageOverview(period, undefined, apex),
      staleTime: 60_000,
      retry: false,
    },
    queryClient,
  );
  const totals = overview.data?.totals;
  const tools = (overview.data?.byTool ?? [])
    .filter((tool) => tool.totalTokens > 0)
    .slice(0, 5);

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <SegmentedControl<TrayPeriod>
          aria-label={t("tray.usage.period")}
          value={period}
          onChange={setPeriod}
          options={[
            { value: "today", label: t("tray.usage.today") },
            { value: "7d", label: t("tray.usage.week") },
          ]}
        />
        <span className="flex-1" />
        <button
          type="button"
          onClick={() => void settingsApi.openExternal(ofoxActivityUrl(apex))}
          className="inline-flex items-center gap-1 text-note text-ink-muted hover:text-ink"
        >
          {t("tray.usage.web")}
          <ExternalLink aria-hidden className="h-3 w-3" />
        </button>
      </div>
      <ListCard
        className={cn(
          PANEL_CARD,
          "grid grid-cols-2 divide-x divide-y-0 divide-line-2",
          overview.isFetching && overview.data && "opacity-55",
        )}
      >
        {[
          {
            key: "tokens",
            label: t("tray.usage.tokens"),
            value: totals ? fmtTokenCount(totals.totalTokens) : "—",
          },
          {
            key: "cost",
            label: t("tray.usage.cost"),
            value: totals ? fmtCost(totals.costUsd) : "—",
            ok: !!totals,
          },
        ].map((cell) => (
          <div key={cell.key} className="px-3 py-2.5">
            <p className="text-note text-ink-muted">{cell.label}</p>
            <p
              className={cn(
                "mt-0.5 text-title font-semibold tabular-nums",
                cell.ok ? "text-ok" : "text-ink",
              )}
            >
              {cell.value}
            </p>
          </div>
        ))}
      </ListCard>
      {overview.isError ? (
        <p role="alert" className="px-1 text-note text-warn">
          {t("tray.usage.failed")}
        </p>
      ) : tools.length > 0 ? (
        <ListCard className={PANEL_CARD}>
          {tools.map((tool) => (
            <div
              key={tool.appType}
              className="flex h-9 items-center gap-2.5 px-3"
            >
              <ToolBadge toolId={tool.appType} size={18} />
              <span className="min-w-0 flex-1 truncate text-ui text-ink">
                {TOOL_META[tool.appType]?.label ?? tool.appType}
              </span>
              <span className="text-ctl tabular-nums text-ink-muted">
                {fmtTokenCount(tool.totalTokens)}
              </span>
              <span className="w-[4.5em] text-right text-ctl tabular-nums text-ok">
                {fmtCost(tool.costUsd)}
              </span>
            </div>
          ))}
        </ListCard>
      ) : (
        overview.data && (
          <p className="px-1 text-note text-ink-muted">
            {t("tray.usage.empty")}
          </p>
        )
      )}
    </div>
  );
}
