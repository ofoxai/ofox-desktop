import { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { MenuSelect } from "@/components/ui/menu-select";
import { Button } from "@/components/ui/button";
import type {
  UsageGranularity,
  UsageSeriesPoint,
} from "@/lib/api/usageOverview";
import { fmtTokens } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";

type Metric =
  | "tokens"
  | "output"
  | "cost"
  | "records"
  | "messages"
  | "sessions"
  | "active";
export type UsageMetric = Metric;
type TrendPoint = Pick<
  UsageSeriesPoint,
  "start" | "inputTokens" | "outputTokens" | "estimate"
> & {
  costUsd?: number;
  requests?: number;
  messages?: number | null;
  sessionCount?: number;
  activeMs?: number | null;
};
export function UsageBars({
  overview,
  analysis = false,
  unavailableMetrics = [],
  selectedMetric,
  onMetricChange,
}: {
  overview: { granularity: UsageGranularity; series: TrendPoint[] };
  analysis?: boolean;
  unavailableMetrics?: Metric[];
  selectedMetric?: Metric;
  onMetricChange?: (metric: Metric) => void;
}) {
  const { t, i18n } = useTranslation();
  const [chosenMetric, setMetric] = useState<Metric>("tokens");
  const chosen = selectedMetric ?? chosenMetric;
  const metric = unavailableMetrics.includes(chosen) ? "tokens" : chosen;
  const [selected, setSelected] = useState(0);
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const metricRefs = useRef<Partial<Record<Metric, HTMLButtonElement | null>>>(
    {},
  );
  const chartId = useId();
  const analysisMetrics = [
    "tokens",
    "output",
    "messages",
    "sessions",
    "cost",
    "active",
  ] as const;
  const chooseMetric = (value: Metric) => {
    setMetric(value);
    onMetricChange?.(value);
  };
  const label = (start: number) =>
    new Date(start).toLocaleString(
      i18n.language,
      overview.granularity === "hour"
        ? { hour: "2-digit", minute: "2-digit" }
        : { month: "short", day: "numeric" },
    );
  const value = (point: TrendPoint): number | null =>
    metric === "tokens"
      ? point.inputTokens + point.outputTokens
      : metric === "output"
        ? point.outputTokens
        : metric === "messages"
          ? (point.messages ?? null)
          : metric === "sessions"
            ? (point.sessionCount ?? null)
            : metric === "active"
              ? point.activeMs == null
                ? null
                : point.activeMs / 60000
              : metric === "records"
                ? (point.requests ?? null)
                : point.estimate
                  ? point.estimate.knownCostUsd == null
                    ? null
                    : Number(point.estimate.knownCostUsd)
                  : (point.costUsd ?? null);
  const formatted = (point: TrendPoint) =>
    metric === "cost"
      ? formatUsageEstimate(point.estimate, point.costUsd)
      : metric === "active"
        ? value(point) == null
          ? "—"
          : t("usageSessions.activeMinutes", {
              value: Number(value(point)!.toFixed(2)).toLocaleString(
                i18n.language,
              ),
            })
        : `${value(point)?.toLocaleString(i18n.language) ?? "—"} ${t(`usageWorkbench.${metric}`)}`;
  const max =
    Math.max(0, ...overview.series.map((point) => value(point) ?? 0)) || 1;
  const current =
    overview.series[Math.min(selected, overview.series.length - 1)];
  return (
    <div
      className={`min-w-0 rounded-list border border-line bg-card ${analysis ? "px-3.5 pb-2.5 pt-3" : "p-3.5"}`}
    >
      {analysis && (
        <div className="mb-2 flex min-w-0 flex-wrap items-center justify-between gap-2">
          <h2 className="text-note font-medium text-ink-muted">
            {t("usageExperience.dailyTrend")}
          </h2>
          <div
            role="tablist"
            aria-label={t("usageWorkbench.metric")}
            className="inline-flex min-w-0 flex-wrap gap-px rounded-ctl bg-pill p-0.5"
          >
            {analysisMetrics.map((value) => (
              <Button
                key={value}
                variant="ghost"
                role="tab"
                id={`${chartId}-${value}`}
                aria-controls={chartId}
                aria-selected={metric === value}
                aria-disabled={unavailableMetrics.includes(value) || undefined}
                disabled={unavailableMetrics.includes(value)}
                tabIndex={metric === value ? 0 : -1}
                ref={(node) => {
                  metricRefs.current[value] = node;
                }}
                className="h-5 px-2 text-note aria-selected:bg-card aria-selected:text-ink aria-selected:shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand/50"
                onClick={() => chooseMetric(value)}
                onKeyDown={(event) => {
                  const available = analysisMetrics.filter(
                    (item) => !unavailableMetrics.includes(item),
                  );
                  const index = available.indexOf(value);
                  const next =
                    event.key === "Home"
                      ? available[0]
                      : event.key === "End"
                        ? available.at(-1)
                        : event.key === "ArrowRight"
                          ? available[(index + 1) % available.length]
                          : event.key === "ArrowLeft"
                            ? available[
                                (index - 1 + available.length) %
                                  available.length
                              ]
                            : undefined;
                  if (next) {
                    event.preventDefault();
                    chooseMetric(next);
                    metricRefs.current[next]?.focus();
                  }
                }}
              >
                {t(
                  value === "cost" || value === "active"
                    ? `usageExperience.${value}`
                    : `usageWorkbench.${value}`,
                )}
              </Button>
            ))}
          </div>
        </div>
      )}
      {!analysis && (
        <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap items-center gap-3 text-note text-ink-muted">
            {metric === "tokens" ? (
              <>
                <span className="inline-flex items-center gap-1.5">
                  <span
                    aria-hidden
                    className="h-2.5 w-2.5 overflow-hidden rounded-sm bg-brand/75"
                  >
                    <span className="block h-full w-full bg-brand/30" />
                  </span>
                  {t("usageWorkbench.freshInput")}
                </span>
                <span className="inline-flex items-center gap-1.5">
                  <span
                    aria-hidden
                    className="h-2.5 w-2.5 overflow-hidden rounded-sm bg-brand/75"
                  >
                    <span className="block h-full w-full bg-brand/90" />
                  </span>
                  {t("usageWorkbench.outputLegend")}
                </span>
                <span>{t("usageWorkbench.cacheSeparate")}</span>
              </>
            ) : (
              t(`usageWorkbench.${metric}`)
            )}
          </div>
          <span className="text-note text-ink-faint">
            {t(`usageWorkbench.granularity.${overview.granularity}`)}
          </span>
          <MenuSelect<Metric>
            aria-label={t("usageWorkbench.metric")}
            value={metric}
            onChange={chooseMetric}
            options={(analysis
              ? ([
                  "tokens",
                  "output",
                  "messages",
                  "sessions",
                  "cost",
                  "active",
                ] as const)
              : (["tokens", "output", "cost", "records"] as const)
            ).map((value) => ({
              value,
              label: t(`usageWorkbench.${value}`),
              disabled: unavailableMetrics.includes(value),
            }))}
          />
        </div>
      )}
      {analysis && (
        <p className="mb-1.5 flex flex-wrap gap-x-3 gap-y-0.5 text-micro text-ink-faint">
          {metric === "tokens" && (
            <>
              <span>
                {t("usageWorkbench.freshInput")} ·{" "}
                {t("usageWorkbench.outputLegend")}
              </span>
              <span>{t("usageWorkbench.cacheSeparate")}</span>
            </>
          )}
          <span className="ml-auto">
            {t(`usageWorkbench.granularity.${overview.granularity}`)}
          </span>
        </p>
      )}
      <div
        id={analysis ? chartId : undefined}
        role={analysis ? "tabpanel" : undefined}
        aria-labelledby={analysis ? `${chartId}-${metric}` : undefined}
        className={`grid min-w-0 grid-cols-[44px_minmax(0,1fr)] gap-2 ${analysis ? "h-[132px]" : "h-[205px]"}`}
        aria-label={t("usagePage.chart")}
      >
        <div
          aria-hidden
          className="flex flex-col justify-between pb-5 text-right text-micro tabular-nums text-ink-faint"
        >
          <span>
            {metric === "cost" ? `$${max.toPrecision(2)}` : fmtTokens(max)}
          </span>
          <span>
            {metric === "cost"
              ? `$${(max / 2).toPrecision(2)}`
              : fmtTokens(max / 2)}
          </span>
          <span>0</span>
        </div>
        <div className="flex min-w-0 items-end gap-px border-b border-line-2 pb-5">
          {overview.series.map((point, index) => {
            const amount = value(point);
            return (
              <button
                key={point.start}
                type="button"
                ref={(node) => {
                  refs.current[index] = node;
                }}
                tabIndex={
                  index === Math.min(selected, overview.series.length - 1)
                    ? 0
                    : -1
                }
                aria-label={`${label(point.start)} · ${formatted(point)}`}
                title={`${label(point.start)} · ${formatted(point)}`}
                className="relative flex h-full min-w-0 flex-1 items-end justify-center rounded-sm px-px outline-none hover:bg-pill focus-visible:ring-2 focus-visible:ring-brand/60"
                onFocus={() => setSelected(index)}
                onClick={() => setSelected(index)}
                onKeyDown={(event) => {
                  const next =
                    event.key === "ArrowRight"
                      ? (index + 1) % overview.series.length
                      : event.key === "ArrowLeft"
                        ? (index - 1 + overview.series.length) %
                          overview.series.length
                        : event.key === "Home"
                          ? 0
                          : event.key === "End"
                            ? overview.series.length - 1
                            : undefined;
                  if (next !== undefined) {
                    event.preventDefault();
                    refs.current[next]?.focus();
                  }
                }}
              >
                {amount == null ? (
                  <span className="text-micro text-ink-faint">—</span>
                ) : (
                  <span
                    aria-hidden
                    className="flex w-full max-w-[28px] flex-col-reverse overflow-hidden rounded-t-sm bg-brand/75"
                    style={{ height: `${Math.max(0, amount / max) * 100}%` }}
                  >
                    {metric === "tokens" && amount > 0 ? (
                      <>
                        <span
                          className="bg-brand/30"
                          style={{
                            height: `${(point.inputTokens / amount) * 100}%`,
                          }}
                        />
                        <span
                          className="bg-brand/90"
                          style={{
                            height: `${(point.outputTokens / amount) * 100}%`,
                          }}
                        />
                      </>
                    ) : null}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      </div>
      <div className="mt-1 flex justify-between text-micro text-ink-faint">
        <span>{overview.series[0] ? label(overview.series[0].start) : ""}</span>
        <span>
          {overview.series.at(-1) ? label(overview.series.at(-1)!.start) : ""}
        </span>
      </div>
      {current && (
        <p
          aria-live="polite"
          className={`${analysis ? "mt-1 text-micro" : "mt-2 text-note"} tabular-nums text-ink-muted`}
        >
          {label(current.start)} · {formatted(current)}
        </p>
      )}
      <details
        className={`${analysis ? "mt-1 text-micro" : "mt-2 text-note"} text-ink-muted`}
      >
        <summary className="cursor-pointer">
          {t("usageWorkbench.chartData")}
        </summary>
        <div className="max-h-[220px] overflow-y-auto">
          <table className="mt-2 w-full text-left">
            <thead>
              <tr>
                <th>{t("usageWorkbench.date")}</th>
                <th>{t("usageWorkbench.value")}</th>
              </tr>
            </thead>
            <tbody>
              {overview.series.map((point) => (
                <tr key={point.start}>
                  <td>{new Date(point.start).toLocaleString(i18n.language)}</td>
                  <td className="tabular-nums">{formatted(point)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </div>
  );
}
