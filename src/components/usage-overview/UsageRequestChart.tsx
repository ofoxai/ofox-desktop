import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SegmentedControl } from "@/components/ui/segmented";
import { TOOL_META } from "@/config/toolMeta";
import { fmtTokens } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import {
  REQUEST_COLORS,
  REQUEST_DIMENSIONS,
  REQUEST_METRICS,
  requestGroupSelected,
  requestMetricValue,
  requestOtherEstimate,
} from "@/lib/usageRequestDashboard";
import type { UsageRequestFilters } from "@/types/usageQuery";
import type {
  RequestDashboardGroup,
  RequestChartDimension,
  RequestMetric,
  UsageRequestDashboard,
} from "@/types/usageRequestDashboard";

export function UsageRequestChart({
  dashboard,
  selection,
  dayStartMs,
  onGroup,
  onDay,
  view,
  onViewChange,
  focusGroupId,
  focusPointStart,
  onFocusRestored,
}: {
  dashboard: UsageRequestDashboard;
  selection: UsageRequestFilters["selection"];
  dayStartMs?: number | null;
  onGroup: (
    dimension: RequestChartDimension,
    group: RequestDashboardGroup,
  ) => void;
  onDay: (start: number | undefined) => void;
  view?: { metric: RequestMetric; dimension: RequestChartDimension };
  onViewChange?: (view: {
    metric: RequestMetric;
    dimension: RequestChartDimension;
  }) => void;
  focusGroupId?: string | null;
  focusPointStart?: number | null;
  onFocusRestored?: (found: boolean) => void;
}) {
  const { t, i18n } = useTranslation();
  const id = useId();
  const [internalMetric, setMetric] = useState<RequestMetric>(() => {
    try {
      const value = localStorage.getItem(
        "ofox.usageRequests.metric",
      ) as RequestMetric;
      return REQUEST_METRICS.includes(value) ? value : "tokens";
    } catch {
      return "tokens";
    }
  });
  const [internalDimension, setDimension] =
    useState<RequestChartDimension>("model");
  const metric = view?.metric ?? internalMetric;
  const dimension = view?.dimension ?? internalDimension;
  const [hovered, setHovered] = useState<string | null>(null);
  const [pointIndex, setPointIndex] = useState<number | null>(null);
  const plotRef = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const rankRefs = useRef(new Map<string, HTMLButtonElement>());
  const colors = useRef(new Map<string, string>());
  const colorCounts = useRef<Partial<Record<RequestChartDimension, number>>>(
    {},
  );
  const [width, setWidth] = useState(900);
  useEffect(() => {
    const host = plotRef.current;
    if (!host) return;
    const measure = () => {
      const w = host.getBoundingClientRect().width;
      if (w > 0) setWidth(w);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(host);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (focusGroupId != null) {
      const target = rankRefs.current.get(focusGroupId);
      target?.focus({ preventScroll: true });
      target?.scrollIntoView({ block: "nearest", inline: "nearest" });
      onFocusRestored?.(!!target);
    } else if (focusPointStart != null) {
      const target = svgRef.current;
      const index = dashboard.series.findIndex(
        (point) => point.start === focusPointStart,
      );
      setPointIndex(index >= 0 ? index : null);
      target?.focus({ preventScroll: true });
      onFocusRestored?.(!!target);
    }
  }, [dashboard, focusGroupId, focusPointStart, onFocusRestored]);
  const name = (group: RequestDashboardGroup) =>
    dimension === "agent"
      ? (TOOL_META[group.toolId ?? group.id]?.label ?? group.label ?? group.id)
      : group.label || t("requestDashboard.unknownModel");
  const color = (key: string) => {
    const scoped = `${dimension}:${key}`;
    if (!colors.current.has(scoped)) {
      const index = colorCounts.current[dimension] ?? 0;
      colorCounts.current[dimension] = index + 1;
      colors.current.set(
        scoped,
        REQUEST_COLORS[index] ?? `hsl(${(index * 137.508) % 360} 65% 48%)`,
      );
    }
    return colors.current.get(scoped)!;
  };
  const compare = (a: RequestDashboardGroup, b: RequestDashboardGroup) =>
    (requestMetricValue(b.totals, metric) ?? -1) -
      (requestMetricValue(a.totals, metric) ?? -1) || a.id.localeCompare(b.id);
  const data = dashboard.dimensions[dimension];
  // Establish identity colours before sorting by any metric.
  useMemo(() => {
    [...data.groups, ...data.rankings]
      .map((g) => g.id)
      .sort()
      .forEach(color);
  }, [data, dimension]);
  const ranks = useMemo(
    () => [...data.rankings].sort(compare),
    [data.rankings, metric],
  );
  const ordered = useMemo(
    () => [...data.groups].sort(compare),
    [data.groups, metric],
  );
  const leaders = ordered.slice(0, 7);
  const selected = ordered.find((g) =>
    requestGroupSelected(dimension, g, selection),
  );
  if (selected && !leaders.some((g) => g.id === selected.id))
    leaders.splice(6, 1, selected);
  const values = useMemo(
    () =>
      new Map(
        data.groups.map((g) => [
          g.id,
          new Map(g.series.map((p) => [p.start, p.totals])),
        ]),
      ),
    [data],
  );
  const points = dashboard.series;
  const height = 268,
    left = 64,
    right = 12,
    top = 12,
    bottom = 32;
  const plotWidth = Math.max(1, width - left - right),
    plotHeight = height - top - bottom;
  const slot = plotWidth / Math.max(1, points.length),
    barWidth = Math.max(1, Math.min(30, slot * 0.68));
  const rawMax = Math.max(
    0,
    ...points.map((p) => requestMetricValue(p.totals, metric) ?? 0),
  );
  const power = 10 ** Math.floor(Math.log10(rawMax || 1));
  const max = ([1, 2, 5, 10].find((n) => n * power >= rawMax) ?? 10) * power;
  const y = (n: number) => top + plotHeight * (1 - n / max);
  const number = (n: number) =>
    n.toLocaleString(i18n.language, { maximumFractionDigits: 2 });
  const display = (n: number | null, compact = false) =>
    n == null
      ? "—"
      : metric === "cost"
        ? `≈$${n.toLocaleString(i18n.language, { maximumFractionDigits: n > 0 && n < 1 ? 4 : 2 })}`
        : compact
          ? fmtTokens(n)
          : number(n);
  const when = (start: number, full = false) =>
    new Date(start).toLocaleString(
      i18n.language,
      full
        ? {
            year: "numeric",
            month: "2-digit",
            day: "2-digit",
            ...(dashboard.granularity === "hour"
              ? {
                  hour: "2-digit",
                  minute: "2-digit",
                  timeZoneName: "shortOffset" as const,
                }
              : {}),
          }
        : dashboard.granularity === "hour"
          ? { hour: "2-digit", minute: "2-digit" }
          : { month: "numeric", day: "numeric" },
    );
  const activePoint =
    pointIndex == null
      ? undefined
      : points[Math.min(pointIndex, points.length - 1)];
  const remaining = (start: number, total: number) =>
    Math.max(
      0,
      total -
        leaders.reduce(
          (sum, g) =>
            sum +
            (requestMetricValue(values.get(g.id)?.get(start), metric) ?? 0),
          0,
        ),
    );
  const hasOther = ordered.length > leaders.length;
  const pickPoint = (index: number) => {
    const p = points[index];
    if (p && dashboard.granularity === "day")
      onDay(dayStartMs === p.start ? undefined : p.start);
  };
  const rankMax =
    Math.max(
      0,
      ...ranks.map((g) => requestMetricValue(g.totals, metric) ?? 0),
    ) || 1;
  const rankSum = ranks.reduce(
    (n, g) => n + (requestMetricValue(g.totals, metric) ?? 0),
    0,
  );
  const empty = !points.length || !data.groups.length;
  return (
    <section
      aria-label={t("usageWorkbench.requestTrend")}
      className="relative min-w-0 rounded-list border border-line bg-card p-3.5"
    >
      <div className="mb-3 flex min-w-0 flex-wrap items-center justify-between gap-2">
        <SegmentedControl
          aria-label={t("usageWorkbench.metric")}
          value={metric}
          onChange={(next) => {
            setMetric(next);
            onViewChange?.({ metric: next, dimension });
            setPointIndex(null);
            try {
              localStorage.setItem("ofox.usageRequests.metric", next);
            } catch {
              /* Storage can be unavailable in a WebView. */
            }
          }}
          options={REQUEST_METRICS.map((value) => ({
            value,
            label: t(`requestDashboard.${value}`),
          }))}
        />
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span className="text-note text-ink-muted">
            {t("requestDashboard.splitBy")}
          </span>
          <SegmentedControl
            aria-label={t("requestDashboard.splitBy")}
            value={dimension}
            onChange={(next) => {
              setDimension(next);
              onViewChange?.({ metric, dimension: next });
              setHovered(null);
              setPointIndex(null);
            }}
            options={REQUEST_DIMENSIONS.map((value) => ({
              value,
              label: t(`requestDashboard.${value}`),
            }))}
          />
        </div>
      </div>
      <div
        ref={plotRef}
        className="relative min-w-0"
        onPointerLeave={() => setPointIndex(null)}
      >
        {empty ||
        (metric === "cost" &&
          points.every((p) => p.totals.estimate.knownCostUsd == null)) ? (
          <div className="flex h-[268px] items-center justify-center text-sub text-ink-muted">
            {t(
              metric === "cost" && !empty
                ? "requestDashboard.noPrice"
                : "usageWorkbench.noRequests",
            )}
          </div>
        ) : (
          <svg
            ref={svgRef}
            role="img"
            aria-label={t("usageWorkbench.requestTrend")}
            aria-describedby={`${id}-readout`}
            viewBox={`0 0 ${width} ${height}`}
            className="block h-[268px] w-full outline-none focus-visible:ring-2 focus-visible:ring-brand/50"
            tabIndex={0}
            onPointerMove={(event) => {
              const rect = event.currentTarget.getBoundingClientRect();
              const x =
                ((event.clientX - rect.left) * width) / (rect.width || width);
              setPointIndex(
                Math.max(
                  0,
                  Math.min(points.length - 1, Math.floor((x - left) / slot)),
                ),
              );
            }}
            onKeyDown={(event) => {
              const last = Math.max(0, points.length - 1);
              let next: number | undefined;
              if (event.key === "ArrowRight")
                next = Math.min(last, (pointIndex ?? -1) + 1);
              if (event.key === "ArrowLeft")
                next = Math.max(0, (pointIndex ?? points.length) - 1);
              if (event.key === "Home") next = 0;
              if (event.key === "End") next = last;
              if (event.key === "Escape") {
                setPointIndex(null);
                return;
              }
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                pickPoint(pointIndex ?? 0);
              }
              if (next != null) {
                event.preventDefault();
                setPointIndex(next);
              }
            }}
          >
            {[0, 1, 2, 3, 4].map((n) => (
              <g key={n}>
                <line
                  x1={left}
                  x2={width - right}
                  y1={top + (plotHeight * n) / 4}
                  y2={top + (plotHeight * n) / 4}
                  className="stroke-line-2"
                  strokeDasharray="3 4"
                />
                <text
                  x={left - 8}
                  y={top + (plotHeight * n) / 4 + 4}
                  textAnchor="end"
                  fontSize="11"
                  className="fill-ink-muted"
                >
                  {display(max * (1 - n / 4), true)}
                </text>
              </g>
            ))}
            {points.map((point, index) => {
              const x = left + slot * (index + 0.5) - barWidth / 2;
              const total = requestMetricValue(point.totals, metric) ?? 0;
              let stacked = 0;
              const dim = dayStartMs != null && dayStartMs !== point.start;
              const tickEvery = Math.max(
                1,
                Math.ceil(
                  points.length / Math.max(2, Math.floor(plotWidth / 64)),
                ),
              );
              return (
                <g key={point.start}>
                  {(pointIndex === index || dayStartMs === point.start) && (
                    <rect
                      x={left + slot * index}
                      y={top}
                      width={slot}
                      height={plotHeight}
                      rx={3}
                      className="fill-pill"
                    />
                  )}
                  {leaders.map((group) => {
                    const value =
                      requestMetricValue(
                        values.get(group.id)?.get(point.start),
                        metric,
                      ) ?? 0;
                    const base = stacked;
                    stacked += value;
                    return value > 0 ? (
                      <g
                        key={group.id}
                        opacity={
                          dim || (hovered != null && hovered !== group.id)
                            ? 0.2
                            : 1
                        }
                      >
                        <rect
                          data-group={group.id}
                          x={x}
                          y={y(base + value)}
                          width={barWidth}
                          height={Math.max(0, y(base) - y(base + value))}
                          rx={1}
                          fill={color(group.id)}
                        />
                      </g>
                    ) : null;
                  })}
                  {remaining(point.start, total) > 0 && (
                    <rect
                      data-group="other"
                      x={x}
                      y={y(total)}
                      width={barWidth}
                      height={Math.max(0, y(stacked) - y(total))}
                      rx={1}
                      fill="#9499a3"
                      opacity={dim || hovered ? 0.2 : 1}
                    />
                  )}
                  {(index % tickEvery === 0 ||
                    (index === points.length - 1 &&
                      points.length > 1 &&
                      (index - 1) % tickEvery !== 0)) && (
                    <text
                      x={left + slot * (index + 0.5)}
                      y={height - 9}
                      textAnchor="middle"
                      fontSize="11"
                      className="fill-ink-muted"
                    >
                      {when(point.start)}
                    </text>
                  )}
                  {dashboard.granularity === "day" && (
                    <rect
                      role="button"
                      aria-label={when(point.start, true)}
                      aria-pressed={dayStartMs === point.start}
                      tabIndex={-1}
                      x={left + slot * index}
                      y={top}
                      width={slot}
                      height={plotHeight}
                      fill="transparent"
                      className="cursor-pointer"
                      onClick={() => pickPoint(index)}
                    />
                  )}
                </g>
              );
            })}
          </svg>
        )}
        {activePoint && (
          <div
            role="tooltip"
            className="pointer-events-none absolute top-3 z-10 w-[260px] max-w-[calc(100%-16px)] rounded-card border border-line bg-card p-3 text-note shadow-lg"
            style={{
              left: `${Math.max(0, Math.min(100 - (260 / Math.max(width, 260)) * 100, ((left + slot * ((pointIndex ?? 0) + 0.5)) / width) * 100))}%`,
            }}
          >
            <strong className="mb-2 block text-sub text-ink">
              {when(activePoint.start, true)}
            </strong>
            {leaders.map((group) => {
              const totals = values.get(group.id)?.get(activePoint.start);
              const value = requestMetricValue(totals, metric);
              return value != null && value > 0 ? (
                <div key={group.id} className="flex items-center gap-2 py-0.5">
                  <i
                    className="h-2 w-2 shrink-0 rounded-sm"
                    style={{ background: color(group.id) }}
                  />
                  <span className="min-w-0 flex-1 truncate">{name(group)}</span>
                  <span className="tabular-nums">
                    {metric === "cost" && totals
                      ? formatUsageEstimate(totals.estimate)
                      : display(value)}
                  </span>
                </div>
              ) : null;
            })}
            {hasOther && (
              <div className="flex justify-between gap-2 py-0.5">
                <span>{t("requestDashboard.other")}</span>
                <span>
                  {metric === "cost"
                    ? formatUsageEstimate(
                        requestOtherEstimate(
                          ordered
                            .filter(
                              (g) =>
                                !leaders.some((leader) => leader.id === g.id),
                            )
                            .flatMap((g) => {
                              const totals = values
                                .get(g.id)
                                ?.get(activePoint.start);
                              return totals ? [totals] : [];
                            }),
                        ),
                      )
                    : display(
                        remaining(
                          activePoint.start,
                          requestMetricValue(activePoint.totals, metric) ?? 0,
                        ),
                      )}
                </span>
              </div>
            )}
            <div className="mt-2 flex justify-between gap-2 border-t border-line-2 pt-2">
              <span>{t("requestDashboard.total")}</span>
              <span className="tabular-nums">
                {metric === "cost"
                  ? formatUsageEstimate(activePoint.totals.estimate)
                  : display(requestMetricValue(activePoint.totals, metric))}
              </span>
            </div>
          </div>
        )}
      </div>
      <p id={`${id}-readout`} className="sr-only" aria-live="polite">
        {activePoint
          ? `${when(activePoint.start, true)} · ${display(requestMetricValue(activePoint.totals, metric))}`
          : t("requestDashboard.keyboardChart")}
      </p>
      <div
        aria-label={t("requestDashboard.ranking")}
        className="mt-3 max-h-[360px] min-w-0 space-y-1 overflow-y-auto overscroll-contain"
      >
        {ranks.map((group) => {
          const value = requestMetricValue(group.totals, metric);
          const on = requestGroupSelected(dimension, group, selection);
          const groupName = name(group);
          const m = group.totals.measurements;
          return (
            <button
              key={group.id}
              ref={(node) => {
                if (node) rankRefs.current.set(group.id, node);
                else rankRefs.current.delete(group.id);
              }}
              type="button"
              aria-pressed={on}
              aria-label={t("requestDashboard.filterGroup", {
                dimension: t(`requestDashboard.${dimension}`),
                name: groupName,
              })}
              title={groupName}
              className="block w-full min-w-0 rounded-lg px-2 py-2 text-left hover:bg-pill aria-pressed:bg-pill focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand/50"
              onPointerEnter={() => setHovered(group.id)}
              onPointerLeave={() => setHovered(null)}
              onFocus={() => setHovered(group.id)}
              onBlur={() => setHovered(null)}
              onClick={() => {
                onGroup(dimension, group);
              }}
            >
              <div className="flex min-w-0 items-center gap-2">
                <i
                  className="h-2.5 w-2.5 shrink-0 rounded-sm"
                  style={{ background: color(group.id) }}
                />
                <span className="min-w-0 flex-1 truncate text-sub font-medium text-ink">
                  {groupName}
                </span>
                <span className="shrink-0 text-sub font-semibold tabular-nums text-ink">
                  {metric === "cost"
                    ? formatUsageEstimate(group.totals.estimate)
                    : display(value)}
                </span>
              </div>
              <div className="ml-[18px] mt-1.5 h-1.5 overflow-hidden rounded-full bg-pill">
                <div
                  className="h-full rounded-full"
                  style={{
                    background: color(group.id),
                    width: `${Math.max(0, Math.min(100, ((value ?? 0) / rankMax) * 100))}%`,
                  }}
                />
              </div>
              <div className="ml-[18px] mt-1.5 flex flex-wrap gap-x-1 text-note tabular-nums text-ink-muted">
                <span>
                  {t("requestDashboard.requestCount", {
                    count: group.totals.requests,
                  })}
                </span>
                {metric !== "tokens" && (
                  <span>
                    · {fmtTokens(group.totals.totalTokensIncludingCache)} Token
                  </span>
                )}
                {metric !== "cost" && (
                  <span>· {formatUsageEstimate(group.totals.estimate)}</span>
                )}
                <span>
                  ·{" "}
                  {t("requestDashboard.hitRate", {
                    rate: Math.round(group.totals.cacheHitRate * 100),
                  })}
                </span>
                {rankSum > 0 && (
                  <span>· {Math.round(((value ?? 0) / rankSum) * 100)}%</span>
                )}
                {m.failedSamples > 0 && (
                  <span className="text-warn">
                    · {t("requestDashboard.failed", { count: m.failedSamples })}
                  </span>
                )}
              </div>
            </button>
          );
        })}
      </div>
    </section>
  );
}
