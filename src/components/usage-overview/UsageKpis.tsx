import { useTranslation } from "react-i18next";
import { KpiStrip } from "@/components/ui/kpi-strip";
import type { UsageTotals } from "@/lib/api/usageOverview";
import { fmtTokens, fmtCost } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import type { UsageAttribution } from "@/types/usageQuery";
export function UsageKpis({
  totals: v,
  attribution,
  unavailable = false,
}: {
  totals: UsageTotals;
  attribution?: UsageAttribution;
  unavailable?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <KpiStrip
      className={
        attribution
          ? "!grid-cols-2 sm:!grid-cols-3"
          : "!grid-cols-2 sm:!grid-cols-4"
      }
      items={[
        {
          label: "Tokens",
          value: unavailable ? "—" : fmtTokens(v.totalTokens),
          sub: t("usagePage.io", {
            input: unavailable ? "—" : fmtTokens(v.inputTokens),
            output: unavailable ? "—" : fmtTokens(v.outputTokens),
          }),
        },
        {
          label: t("usagePage.cacheRead"),
          value: unavailable ? "—" : fmtTokens(v.cacheReadTokens),
          sub: t("usagePage.hitRate", {
            rate:
              unavailable ||
              v.inputTokens + v.cacheReadTokens + v.cacheWriteTokens === 0
                ? "—"
                : `${Math.round(v.cacheHitRate * 100)}%`,
          }),
          title: t("usagePage.cacheWrite", {
            value: unavailable ? "—" : fmtTokens(v.cacheWriteTokens),
          }),
        },
        {
          label: t("usagePage.cost"),
          value: unavailable
            ? "—"
            : v.estimate
              ? formatUsageEstimate(v.estimate)
              : fmtCost(v.costUsd),
          title: t("usagePage.estimate"),
        },
        {
          label: t("usageWorkbench.records"),
          value: unavailable ? "—" : fmtTokens(v.requests),
        },
        ...(attribution
          ? [
              {
                label: t("usageWorkbench.identifiedSessions"),
                value: unavailable ? "—" : fmtTokens(attribution.sessionCount),
                sub: attribution.partial
                  ? t("usageWorkbench.attributionPartial")
                  : undefined,
              },
              {
                label: t("usageWorkbench.identifiedProjects"),
                value: unavailable ? "—" : fmtTokens(attribution.projectCount),
                sub: attribution.partial
                  ? t("usageWorkbench.attributionPartial")
                  : undefined,
              },
            ]
          : []),
      ]}
    />
  );
}
