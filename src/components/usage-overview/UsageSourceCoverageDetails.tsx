import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { UsageSourceCoverage } from "@/types/usageQuery";

/** Source-file coverage is distinct from the message/activity sample cohort. */
export function UsageSourceCoverageDetails({
  coverage,
  onSavedHistory,
}: {
  coverage: UsageSourceCoverage;
  onSavedHistory?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <p>{t("usageExperience.nativeSources", { ...coverage })}</p>
      {coverage.undated > 0 && (
        <p>
          {t("usageExperience.undatedSources", { count: coverage.undated })}
        </p>
      )}
      {coverage.legacyHistoryAvailable && onSavedHistory && (
        <Button variant="ghost" onClick={onSavedHistory}>
          {t("usageExperience.savedHistory")}
        </Button>
      )}
    </div>
  );
}

export function hasSourceCoverageGaps(coverage?: UsageSourceCoverage) {
  return (
    !!coverage &&
    (coverage.partial > 0 || coverage.failed > 0 || coverage.undated > 0)
  );
}
