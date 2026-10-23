import { ChevronDown, Clock3, Layers3, MessagesSquare } from "lucide-react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import type { AnalyticsDistribution } from "@/types/sessionAnalytics";

function bucketRange(
  metric: AnalyticsDistribution["metric"],
  start: number,
  end: number | undefined,
  t: TFunction,
  number: Intl.NumberFormat,
) {
  // Bounds must remain exact even when their visible labels are abbreviated.
  const compact = (value: number) => {
    for (const [scale, suffix] of [
      [1e9, "B"],
      [1e6, "M"],
      [1e3, "K"],
    ] as const) {
      if (value >= scale && Number.isInteger(value / scale))
        return `${number.format(value / scale)}${suffix}`;
    }
    return number.format(value);
  };
  const bounded = (lower: string, upper: string) =>
    t("usageDistribution.exclusiveRange", { start: lower, end: upper });
  if (metric === "activeMs") {
    const duration = (ms: number) =>
      ms % 60000 === 0
        ? t("usageExperience.minutes", { value: number.format(ms / 60000) })
        : t("usageDistribution.milliseconds", { value: number.format(ms) });
    if (start === 0 && end === 1) {
      const zero = duration(0);
      return { short: zero, exact: zero };
    }
    if (start === 1 && end === 60000)
      return {
        short: t("usageDistribution.underMinute"),
        exact: t("usageExperience.underMinute"),
      };
    const exact =
      end == null
        ? `≥ ${duration(start)}`
        : t("usageDistribution.exclusiveDuration", {
            start: duration(start),
            end: duration(end),
          });
    const short =
      start % 60000 === 0 && (end == null || end % 60000 === 0)
        ? t("usageExperience.minutes", {
            value:
              end == null
                ? `${number.format(start / 60000)}+`
                : `${number.format(start / 60000)}–${number.format(end / 60000)}`,
          })
        : exact;
    return { short, exact };
  }
  if (metric === "messages") {
    const short =
      end == null
        ? `${number.format(start)}+`
        : end - 1 === start
          ? number.format(start)
          : `${number.format(start)}–${number.format(end - 1)}`;
    return { short, exact: short };
  }
  return {
    short:
      end == null
        ? `${compact(start)}+`
        : start === 0
          ? `<${compact(end)}`
          : `${compact(start)}–${compact(end)}`,
    exact:
      end == null
        ? `≥ ${compact(start)}`
        : bounded(compact(start), compact(end)),
  };
}

export function UsageDistributions({
  distributions,
  activityLabel,
}: {
  distributions?: AnalyticsDistribution[];
  activityLabel?: string;
}) {
  const { t, i18n } = useTranslation();
  if (!distributions?.length) return null;
  const number = new Intl.NumberFormat(i18n.language, {
    maximumFractionDigits: 20,
  });
  const percent = new Intl.NumberFormat(i18n.language, {
    style: "percent",
    maximumFractionDigits: 1,
  });
  return (
    <details className="group rounded-list border border-line bg-card">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-3 rounded-list px-4 py-3.5 outline-none focus-visible:ring-2 focus-visible:ring-brand/60 [&::-webkit-details-marker]:hidden">
        <span className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-1">
          <span className="text-body font-medium text-ink">
            {t("usageExperience.distribution")}
          </span>
          <span className="text-note text-ink-muted">
            {t("usageDistribution.description")}
          </span>
        </span>
        <ChevronDown
          className="h-4 w-4 shrink-0 text-ink-muted transition-transform group-open:rotate-180"
          aria-hidden
        />
      </summary>
      <div
        className="grid gap-3 border-t border-line px-4 py-4"
        style={{
          gridTemplateColumns:
            "repeat(auto-fit, minmax(min(100%, 15rem), 1fr))",
        }}
      >
        {distributions.map((distribution) => {
          const max = Math.max(1, ...distribution.counts);
          const label =
            distribution.metric === "activeMs" && activityLabel
              ? activityLabel
              : distribution.metric === "toolCallsPerPrompt"
                ? t("usageInteractions.toolCalls")
                : t(`usageExperience.${distribution.metric}`);
          const Icon =
            distribution.metric === "activeMs"
              ? Clock3
              : distribution.metric === "messages"
                ? MessagesSquare
                : Layers3;
          return (
            <section
              key={distribution.metric}
              aria-label={label}
              className="flex min-w-0 flex-col rounded-xl border border-line bg-card-2 p-4"
            >
              <header className="mb-5 flex items-center gap-2.5">
                <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-line bg-card text-ink-muted">
                  <Icon className="h-4 w-4" aria-hidden />
                </span>
                <div className="min-w-0">
                  <h3 className="text-sub font-medium text-ink">{label}</h3>
                  {distribution.samples > 0 && (
                    <p className="mt-0.5 text-micro tabular-nums text-ink-muted">
                      {t("usageDistribution.samples", {
                        count: distribution.samples,
                      })}
                    </p>
                  )}
                </div>
              </header>
              {distribution.samples > 0 ? (
                <div className="space-y-3.5" role="list">
                  {distribution.counts.map((count, index) => {
                    const range = bucketRange(
                      distribution.metric,
                      distribution.edges[index],
                      distribution.edges[index + 1],
                      t,
                      number,
                    );
                    const description = t(
                      "usageExperience.distributionBucket",
                      { range: range.exact, count },
                    );
                    const share = count / distribution.samples;
                    return (
                      <div
                        key={index}
                        title={description}
                        aria-label={description}
                        role="listitem"
                      >
                        <div className="mb-1.5 flex items-baseline justify-between gap-2 text-note tabular-nums">
                          <span className="min-w-0 text-ink-muted">
                            {range.short}
                          </span>
                          <span className="flex shrink-0 items-baseline gap-2.5">
                            <span className="font-medium text-ink">
                              {number.format(count)}
                            </span>
                            <span className="w-11 text-right text-micro text-ink-muted">
                              {share > 0 && share < 0.001
                                ? `<${percent.format(0.001)}`
                                : percent.format(share)}
                            </span>
                          </span>
                        </div>
                        <div
                          className="h-1.5 overflow-hidden rounded-pill bg-pill"
                          aria-hidden
                        >
                          <div
                            className={`h-full rounded-pill ${count === max ? "bg-brand/80" : "bg-brand/40"}`}
                            style={{ width: `${share * 100}%` }}
                          />
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : (
                <p className="py-6 text-center text-note text-ink-muted">
                  {t("usageExperience.noSamples")}
                </p>
              )}
              {(distribution.excludedSessions > 0 ||
                distribution.unquantifiedSourceGaps) && (
                <p className="mt-auto pt-4 text-micro leading-relaxed text-ink-muted">
                  {[
                    distribution.excludedSessions > 0
                      ? t("usageDistribution.excluded", {
                          count: distribution.excludedSessions,
                        })
                      : null,
                    distribution.unquantifiedSourceGaps
                      ? t("usageInteractions.sourceGaps")
                      : null,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </p>
              )}
            </section>
          );
        })}
      </div>
    </details>
  );
}
