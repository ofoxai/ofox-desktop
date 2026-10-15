import { useTranslation } from "react-i18next";
import { KpiStrip } from "@/components/ui/kpi-strip";
import type { UsageTotals } from "@/lib/api/usageOverview";
import { fmtTokens, fmtCost } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";
export function UsageKpis({ totals: v }: { totals: UsageTotals }) {
  const { t } = useTranslation();
  return (
    <KpiStrip
      className="!grid-cols-2 sm:!grid-cols-4"
      items={[
        {
          label: "Tokens",
          value: fmtTokens(v.totalTokens),
          sub: t("usagePage.io", {
            input: fmtTokens(v.inputTokens),
            output: fmtTokens(v.outputTokens),
          }),
        },
        {
          label: t("usagePage.cacheRead"),
          value: fmtTokens(v.cacheReadTokens),
          sub: t("usagePage.hitRate", {
            rate:
              v.inputTokens + v.cacheReadTokens + v.cacheWriteTokens === 0
                ? "—"
                : `${Math.round(v.cacheHitRate * 100)}%`,
          }),
          title: t("usagePage.cacheWrite", {
            value: fmtTokens(v.cacheWriteTokens),
          }),
        },
        {
          label: t("usagePage.cost"),
          value: v.estimate
            ? formatUsageEstimate(v.estimate)
            : fmtCost(v.costUsd),
          sub: t("usagePage.estimate"),
        },
        { label: t("usageWorkbench.records"), value: fmtTokens(v.requests) },
      ]}
    />
  );
}
