import { useTranslation } from "react-i18next";
import { KpiStrip } from "@/components/ui/kpi-strip";
import type { UsageTotals } from "@/lib/api/usageOverview";
import { fmtTokens, fmtCost } from "@/lib/format";
export function UsageKpis({ totals: v }: { totals: UsageTotals }) {
  const { t } = useTranslation();
  return (
    <KpiStrip
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
            rate: `${Math.round(v.cacheHitRate * 100)}%`,
          }),
          title: t("usagePage.cacheWrite", {
            value: fmtTokens(v.cacheWriteTokens),
          }),
        },
        {
          label: t("usagePage.cost"),
          value: fmtCost(v.costUsd),
          sub: t("usagePage.estimate"),
        },
        { label: t("usagePage.requests"), value: fmtTokens(v.requests) },
      ]}
    />
  );
}
