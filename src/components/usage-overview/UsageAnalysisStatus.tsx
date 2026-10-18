import { useTranslation } from "react-i18next";
import type { SessionAnalytics } from "@/types/sessionAnalytics";
import { usageIsUnavailable } from "@/lib/usageWorkbench";
import { UsageFreshness } from "./UsageFreshness";
import {
  hasSourceCoverageGaps,
  UsageSourceCoverageDetails,
} from "./UsageSourceCoverageDetails";

export function UsageAnalysisStatus({
  data,
  compact = false,
  onSavedHistory,
}: {
  data: SessionAnalytics;
  compact?: boolean;
  onSavedHistory?: () => void;
}) {
  const { t, i18n } = useTranslation();
  const coverage = data.metricCoverage;
  const messages = data.messageCoverage ?? coverage;
  const activity = data.activityCoverage ?? coverage;
  const sources = data.sourceCoverage;
  const modern = data.methodVersion === "magpie-native-v1";
  const unavailable =
    data.coverage.state === "unsupported" || usageIsUnavailable(data);
  const messagesUnavailable = messages?.state === "unavailable";
  const activityUnavailable = activity?.state === "unavailable";
  const sourcePartial =
    data.coverage.state !== "complete" || hasSourceCoverageGaps(sources);
  const partial =
    sourcePartial ||
    messages?.state === "partial" ||
    activity?.state === "partial" ||
    messagesUnavailable ||
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
    ...new Set([
      ...data.coverage.reasons,
      ...(messages?.reasons ?? []),
      ...(activity?.reasons ?? []),
    ]),
  ];
  return (
    <details
      className={`rounded-lg border border-line bg-card px-3 text-note text-ink-muted ${compact ? "py-1.5" : "py-2"}`}
    >
      <summary className="cursor-pointer">
        <span
          className={`mr-2 inline-block h-1.5 w-1.5 rounded-full ${partial || unavailable ? "bg-warn" : "bg-ok"}`}
          aria-hidden
        />
        <span>
          {t(
            !modern
              ? "usageExperience.legacyMethod"
              : unavailable
                ? "usageExperience.unavailable"
                : sourcePartial
                  ? "usageExperience.nativePartial"
                  : "usageExperience.nativeUpdated",
          )}
        </span>
        {(activityUnavailable || messagesUnavailable) && (
          <span className="ml-3">
            {t(
              activityUnavailable && messagesUnavailable
                ? "usageExperience.activityUnavailable"
                : activityUnavailable
                  ? "usageExperience.toolActivityUnavailable"
                  : "usageExperience.messagesUnavailable",
            )}
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
        {sources && (
          <UsageSourceCoverageDetails
            coverage={sources}
            onSavedHistory={modern ? onSavedHistory : undefined}
          />
        )}
        {(data.messageCoverage || data.activityCoverage) && !unavailable ? (
          <>
            {messages && (
              <p>
                {t("usageExperience.messageSamples", {
                  eligible: messages.eligibleSessions,
                  excluded: messages.excludedSessions,
                })}
              </p>
            )}
            {activity && (
              <p>
                {t("usageExperience.activitySamples", {
                  eligible: activity.eligibleSessions,
                  excluded: activity.excludedSessions,
                })}
              </p>
            )}
          </>
        ) : (
          coverage &&
          !unavailable && (
            <p>
              {t("usageExperience.metricSamples", {
                eligible: coverage.eligibleSessions,
                excluded: coverage.excludedSessions,
              })}
            </p>
          )
        )}
        {(messages?.unquantifiedSourceGaps ||
          activity?.unquantifiedSourceGaps) && (
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
