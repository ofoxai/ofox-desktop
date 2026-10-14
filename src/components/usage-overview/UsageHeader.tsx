import { useTranslation } from "react-i18next";
import { SegmentedControl } from "@/components/ui/segmented";
import type { UsagePeriod } from "@/lib/api/usageOverview";
import { fmtCost } from "@/lib/format";
export function UsageHeader({
  period,
  onChange,
  cost,
}: {
  period: UsagePeriod;
  onChange: (p: UsagePeriod) => void;
  cost?: number;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-wrap items-center gap-3">
      <SegmentedControl<UsagePeriod>
        aria-label={t("usagePage.period")}
        value={period}
        onChange={onChange}
        options={(["today", "7d", "30d", "all"] as const).map((value) => ({
          value,
          label: t(`usagePage.periods.${value}`),
        }))}
      />
      <span className="flex-1" />
      <span className="text-[15px] font-semibold tabular-nums">
        {fmtCost(cost)}
      </span>
      <span className="text-note text-ink-muted">
        {t("usagePage.estimate")}
      </span>
    </div>
  );
}
