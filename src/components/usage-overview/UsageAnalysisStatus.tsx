import { useTranslation } from "react-i18next";
import type { SessionAnalytics } from "@/types/sessionAnalytics";
import { UsageFreshness } from "./UsageFreshness";

export function UsageAnalysisStatus({ data }: { data: SessionAnalytics }) {
  const { t, i18n } = useTranslation();
  const coverage = data.metricCoverage;
  const unavailable = data.coverage.state === "unsupported";
  const activityUnavailable = coverage?.state === "unavailable";
  const partial =
    data.coverage.state !== "complete" ||
    coverage?.state === "partial" ||
    activityUnavailable ||
    data.pricingCoverage.stale ||
    !!data.pricingCoverage.unpricedModelIds.length ||
    ["missing", "invalid", "unsupported"].includes(
      data.pricingCoverage.catalogState ?? "",
    );
  const attemptedAt = Math.max(
    0,
    ...data.sources.map((source) => source.lastAttemptAtMs ?? 0),
  );
  const reasons = [
    ...new Set([...data.coverage.reasons, ...(coverage?.reasons ?? [])]),
  ];
  return (
    <details className="rounded-lg border border-line bg-card px-3 py-2 text-note text-ink-muted">
      <summary className="cursor-pointer">
        <span
          className={`mr-2 inline-block h-1.5 w-1.5 rounded-full ${partial || unavailable ? "bg-warn" : "bg-ok"}`}
          aria-hidden
        />
        {t(
          `usageExperience.${unavailable ? "unavailable" : partial ? "partial" : "complete"}`,
        )}
        {activityUnavailable && (
          <span className="ml-3">
            {t("usageExperience.activityUnavailable")}
          </span>
        )}
        {coverage && !unavailable && (
          <span className="ml-3">
            {t("usageExperience.coverage", {
              eligible: coverage.eligibleSessions,
              excluded: coverage.excludedSessions,
            })}
          </span>
        )}
        <span className="ml-3 text-ink-faint">
          {t("usageExperience.details")}
        </span>
        {attemptedAt > 0 && (
          <span className="ml-3 text-ink-faint">
            {t("usageExperience.lastAttempt", {
              time: new Date(attemptedAt).toLocaleString(i18n.language),
            })}
          </span>
        )}
      </summary>
      <div className="mt-3 space-y-2 border-t border-line-2 pt-3">
        {coverage?.unquantifiedSourceGaps && (
          <p>{t("usageExperience.unknownGaps")}</p>
        )}
        {!!data.unassociated.records && (
          <p>
            {t("usageSessions.unassociated", {
              count: data.unassociated.records,
              tokens: data.unassociated.totalTokens,
            })}
          </p>
        )}
        {!!reasons.length && (
          <div>
            <p className="font-medium">{t("usageExperience.partialReasons")}</p>
            <ul className="list-inside list-disc">
              {reasons.map((reason) => (
                <li key={reason}>
                  {t(`usageExperience.reasons.${reason}`, {
                    defaultValue: t("usageExperience.unknownReason"),
                  })}
                </li>
              ))}
            </ul>
          </div>
        )}
        <UsageFreshness sources={data.sources} pricing={data.pricingCoverage} />
      </div>
    </details>
  );
}
