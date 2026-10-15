import { useTranslation } from "react-i18next";
import { SegmentedControl } from "@/components/ui/segmented";
import type { WorkbenchPeriod } from "@/lib/usageWorkbench";
import type { UsageEstimate } from "@/types/usageQuery";
import { formatUsageEstimate } from "@/lib/usageEstimate";
export function UsageHeader({
  period,
  onChange,
  cost,
  estimate,
  modern = false,
}: {
  period: WorkbenchPeriod;
  onChange: (p: WorkbenchPeriod) => void;
  cost?: number;
  estimate?: UsageEstimate;
  modern?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-wrap items-center gap-3">
      <SegmentedControl<WorkbenchPeriod>
        aria-label={t("usagePage.period")}
        value={period}
        onChange={onChange}
        options={(modern
          ? (["today", "7d", "30d", "90d", "all", "custom"] as const)
          : (["today", "7d", "30d", "all"] as const)
        ).map((value) => ({
          value,
          label: t(`usagePage.periods.${value}`),
        }))}
      />
      <span className="flex-1" />
      <span className="text-[15px] font-semibold tabular-nums">
        {formatUsageEstimate(estimate, cost)}
      </span>
      <span className="text-note text-ink-muted">
        {t("usagePage.estimate")}
      </span>
    </div>
  );
}
