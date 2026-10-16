import { useTranslation } from "react-i18next";
import type { AnalyticsDistribution } from "@/types/sessionAnalytics";
import { fmtTokens } from "@/lib/format";
import { formatActivity } from "@/lib/usageInsights";

export function UsageDistributions({
  distributions,
}: {
  distributions?: AnalyticsDistribution[];
}) {
  const { t } = useTranslation();
  if (!distributions) return null;
  return (
    <details className="rounded-list border border-line bg-card p-4">
      <summary className="cursor-pointer text-sub font-medium text-ink-muted">
        {t("usageExperience.distribution")}
      </summary>
      <div className="mt-4 grid gap-6 min-[1100px]:grid-cols-3">
        {distributions.map((distribution) => {
          const max = Math.max(1, ...distribution.counts);
          const format = (value: number) =>
            distribution.metric === "activeMs"
              ? formatActivity(value, t)
              : fmtTokens(value);
          return (
            <section
              key={distribution.metric}
              aria-label={t(`usageExperience.${distribution.metric}`)}
            >
              <h3 className="text-sub font-medium">
                {t(`usageExperience.${distribution.metric}`)}
              </h3>
              <p className="mb-3 text-note text-ink-faint">
                {t("usageExperience.distributionSamples", {
                  count: distribution.samples,
                  excluded: distribution.excludedSessions,
                })}
              </p>
              {distribution.samples ? (
                distribution.counts.map((count, index) => {
                  const start = distribution.edges[index];
                  const end = distribution.edges[index + 1];
                  const range =
                    start === 0 && end === 1
                      ? format(0)
                      : distribution.metric === "activeMs" &&
                          start === 1 &&
                          end === 60000
                        ? t("usageExperience.underMinute")
                        : end == null
                          ? `≥ ${format(start)}`
                          : `${format(start)}–<${format(end)}`;
                  const label = t("usageExperience.distributionBucket", {
                    range,
                    count,
                  });
                  return (
                    <div
                      key={index}
                      className="mb-2 grid grid-cols-[100px_minmax(0,1fr)_30px] items-center gap-2 text-note"
                      title={label}
                      aria-label={label}
                    >
                      <span className="truncate text-ink-muted" title={range}>
                        {range}
                      </span>
                      <span
                        className="h-2 overflow-hidden rounded-pill bg-pill"
                        aria-hidden
                      >
                        <span
                          className="block h-full rounded-pill bg-brand/75"
                          style={{ width: `${(count / max) * 100}%` }}
                        />
                      </span>
                      <b className="text-right tabular-nums">{count}</b>
                    </div>
                  );
                })
              ) : (
                <p className="text-note text-ink-muted">
                  {t("usageExperience.noSamples")}
                </p>
              )}
              {distribution.unquantifiedSourceGaps && (
                <p className="mt-2 text-note text-ink-faint">
                  {t("usageExperience.unknownGaps")}
                </p>
              )}
            </section>
          );
        })}
      </div>
    </details>
  );
}
