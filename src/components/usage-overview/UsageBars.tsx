import { useTranslation } from "react-i18next";
import { StackedBars } from "@/components/ui/stacked-bars";
import type { UsageOverview } from "@/lib/api/usageOverview";
import { fmtTokens } from "@/lib/format";
export function UsageBars({ overview }: { overview: UsageOverview }) {
  const { t, i18n } = useTranslation();
  const label = (start: string | number) =>
    new Date(Number(start)).toLocaleString(
      i18n.language,
      overview.granularity === "hour"
        ? { hour: "2-digit", minute: "2-digit" }
        : { month: "short", day: "numeric" },
    );
  return (
    <div className="rounded-list border border-line bg-card p-3.5">
      <p className="mb-2 text-sub text-ink-muted">
        {t("usagePage.chartLegend")}
      </p>
      <StackedBars
        series={overview.series.map((p) => ({
          start: p.start,
          a: p.inputTokens,
          b: p.outputTokens,
        }))}
        aria-label={t("usagePage.chart")}
        formatLabel={label}
        formatTitle={(p) =>
          `${label(p.start)} · ${t("usagePage.io", { input: fmtTokens(p.a), output: fmtTokens(p.b) })}`
        }
      />
    </div>
  );
}
