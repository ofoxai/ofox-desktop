import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { KpiStrip } from "@/components/ui/kpi-strip";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { fmtTokens } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import type {
  AnalyticsSession,
  SessionAnalytics,
} from "@/types/sessionAnalytics";
import { UsageBars, type UsageMetric } from "./UsageBars";
import { usageProjectLabel } from "@/lib/usageProject";
import { UsageFreshness } from "./UsageFreshness";
import { UsageBreakdownList } from "./UsageBreakdownList";
import type { UsageQueryV1 } from "@/types/usageQuery";

export function UsageSessions({
  data,
  active,
  onProject,
  onModel,
  onManage,
  showData = true,
}: {
  data: SessionAnalytics;
  active: boolean;
  onProject?: (id: string) => void;
  onModel?: (id: string) => void;
  onManage?: (key: string) => void;
  showData?: boolean;
}) {
  const { t, i18n } = useTranslation();
  const panel = useRef<HTMLElement>(null);
  const returnFocus = useRef<HTMLElement>();
  const returnKey = useRef<string>();
  const rowButtons = useRef(new Map<string, HTMLButtonElement>());
  const handingOff = useRef(false);
  const [selected, setSelected] = useState<{
    row: AnalyticsSession;
    version: string;
    query: UsageQueryV1;
  }>();
  useEffect(() => {
    if (!active) setSelected(undefined);
  }, [active]);
  const [metric, setMetric] = useState<UsageMetric>("tokens");
  const [detailModelLimit, setDetailModelLimit] = useState(50);
  const project = (
    id: string | null,
    label?: string | null,
    path?: string | null,
  ) =>
    usageProjectLabel(id, label, path, {
      unknown: t("usageSessions.unknownProject"),
      unparsed: t("usageSessions.projectUnparsed"),
    });
  const firstProject = data.byProject[0];
  const v = data.totals;
  const duration = (ms: number | null) =>
    ms == null
      ? "—"
      : t("usageSessions.activeMinutes", {
          value: Number((ms / 60000).toFixed(2)).toLocaleString(i18n.language),
        });
  const quantile = (value: string | number | null) => {
    if (value == null) return "—";
    const [whole, decimal] = String(value).split(".");
    if (!/^\d+$/.test(whole)) return "—";
    const separator =
      new Intl.NumberFormat(i18n.language)
        .formatToParts(1.1)
        .find((p) => p.type === "decimal")?.value ?? ".";
    return (
      BigInt(whole).toLocaleString(i18n.language) +
      (decimal ? separator + decimal : "")
    );
  };

  return (
    <section
      ref={panel}
      tabIndex={-1}
      aria-label={t("usageSessions.title")}
      className="space-y-3 outline-none"
    >
      {showData && (
        <>
          <KpiStrip
            className="!grid-cols-2 md:!grid-cols-3"
            items={[
              {
                label: t("usageSessions.sessions"),
                value: v.sessionCount.toLocaleString(i18n.language),
                sub: t("usageSessions.quantiles", {
                  median: quantile(v.medianTokens),
                  p90: quantile(v.p90Tokens),
                }),
              },
              {
                label: "Tokens",
                value: fmtTokens(v.totalTokens),
                sub: t("usagePage.io", {
                  input: fmtTokens(v.inputTokens),
                  output: fmtTokens(v.outputTokens),
                }),
              },
              {
                label: t("usageWorkbench.currentEstimate"),
                value: formatUsageEstimate(v.estimate),
                sub: t("usagePage.estimate"),
              },
              {
                label: t("usagePage.cacheRead"),
                value: fmtTokens(v.cacheReadTokens),
                sub: t("usagePage.hitRate", {
                  rate:
                    v.cacheHitRate == null
                      ? "—"
                      : `${Math.round(v.cacheHitRate * 100)}%`,
                }),
              },
              {
                label: t("usageSessions.activity"),
                value: duration(v.activeMs),
                sub: t("usageSessions.activeDays", {
                  value: v.activeDays ?? "—",
                }),
              },
              {
                label: t("usageSessions.projects"),
                value: v.projectCount.toLocaleString(i18n.language),
                sub: firstProject
                  ? project(
                      firstProject.id,
                      firstProject.label,
                      firstProject.path,
                    )
                  : "—",
              },
            ]}
          />
          <p className="text-note text-ink-muted">
            {t("usageSessions.samples", {
              count: v.quantileSamples,
              excluded: v.quantileExcluded,
            })}
          </p>
          {(v.messageReason === "modelAttributionUnavailable" ||
            v.activityReason === "modelAttributionUnavailable") && (
            <p className="text-note text-ink-muted">
              {t("usageSessions.modelUnavailable")}
            </p>
          )}
          {(v.messages == null || v.activeMs == null) &&
            v.messageReason !== "modelAttributionUnavailable" &&
            v.activityReason !== "modelAttributionUnavailable" && (
              <p className="text-note text-ink-muted">
                {t(
                  v.messageReason === "sourceAttributionUnavailable" ||
                    v.activityReason === "sourceAttributionUnavailable"
                    ? "usageSessions.sourceUnavailableMetrics"
                    : "usageSessions.messageCoveragePartial",
                )}
              </p>
            )}
          <UsageBars
            selectedMetric={metric}
            onMetricChange={setMetric}
            analysis
            overview={{
              granularity: "day",
              series: data.series.map((day) => ({
                ...day,
                start: day.startMs,
              })),
            }}
            unavailableMetrics={[
              ...(v.messages == null ? ["messages" as const] : []),
              ...(v.activeMs == null ? ["active" as const] : []),
            ]}
          />
          <p className="text-note text-ink-faint">
            {t("usageSessions.activityExplanation")}
          </p>
          <div className="grid gap-3 md:grid-cols-2">
            <UsageBreakdownList
              kind="projects"
              rows={data.byProject.map((row) => ({
                ...row,
                label: project(row.id, row.label, row.path),
              }))}
              total={v.totalTokens}
              onFilter={onProject}
              pageSize={50}
              storageKey="ofox.usageAnalysisHidden.projects"
            />
            <UsageBreakdownList
              kind="models"
              rows={data.byModel}
              total={v.totalTokens}
              onFilter={onModel}
              pageSize={50}
              storageKey="ofox.usageAnalysisHidden.models"
              title={t("usageSessions.modelRank")}
            />
          </div>
          <div className="overflow-hidden rounded-list border border-line bg-card">
            <h2 className="border-b border-line-2 px-3 py-2 text-sub font-medium">
              {t("usageSessions.top", { count: data.topLimit })}
            </h2>
            {data.topSessions.length ? (
              data.topSessions.map((row) => (
                <button
                  key={row.sessionKey}
                  type="button"
                  aria-label={t("usageSessions.view", { title: row.title })}
                  ref={(node) => {
                    if (node) rowButtons.current.set(row.sessionKey, node);
                    else rowButtons.current.delete(row.sessionKey);
                  }}
                  onClick={(event) => {
                    handingOff.current = false;
                    returnFocus.current = event.currentTarget;
                    returnKey.current = row.sessionKey;
                    setDetailModelLimit(50);
                    setSelected({
                      row,
                      version: data.snapshotVersion,
                      query: data.appliedQuery,
                    });
                  }}
                  className="flex w-full min-w-0 items-center gap-3 border-b border-line-2 px-3 py-3 text-left last:border-b-0 hover:bg-pill focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand/50"
                >
                  <ToolBadge toolId={row.toolId} size={24} />
                  <span className="min-w-0 flex-1">
                    <span
                      className="block truncate text-sub font-medium"
                      title={row.title}
                    >
                      {row.title}
                    </span>
                    <span
                      className="block truncate text-note text-ink-muted"
                      title={project(
                        row.projectId,
                        row.projectPath ?? row.projectLabel,
                      )}
                    >
                      {project(
                        row.projectId,
                        row.projectLabel,
                        row.projectPath,
                      )}{" "}
                      ·{" "}
                      {row.models
                        .slice(0, 2)
                        .map((m) => m.model)
                        .join(", ") || "—"}
                      {row.models.length > 2
                        ? ` +${row.models.length - 2}`
                        : ""}
                    </span>
                  </span>
                  <span className="shrink-0 text-sub tabular-nums">
                    {row.tokens == null
                      ? "—"
                      : fmtTokens(row.tokens.totalTokens)}
                  </span>
                  <span className="shrink-0 whitespace-nowrap text-sub tabular-nums text-ink-muted">
                    {formatUsageEstimate(row.estimate)}
                  </span>
                </button>
              ))
            ) : (
              <p className="p-5 text-sub text-ink-muted">
                {t("usageSessions.empty")}
              </p>
            )}
          </div>
          {!!data.unassociated.records && (
            <p className="text-note text-ink-muted">
              {t("usageSessions.unassociated", {
                count: data.unassociated.records,
                tokens: fmtTokens(data.unassociated.totalTokens),
              })}
            </p>
          )}
          {data.coverage.state !== "complete" && (
            <p className="text-note text-ink-muted">
              {t(
                data.coverage.state === "unsupported"
                  ? "usageWorkbench.queryUnavailable"
                  : "usageWorkbench.partial",
              )}
            </p>
          )}
          <UsageFreshness
            sources={data.sources}
            pricing={data.pricingCoverage}
          />
        </>
      )}
      <Dialog
        open={active && !!selected}
        onOpenChange={(open) => {
          if (!open) setSelected(undefined);
        }}
      >
        <DialogContent
          aria-describedby={undefined}
          className="max-w-2xl"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            if (handingOff.current || !active) return;
            if (returnFocus.current?.isConnected) returnFocus.current.focus();
            else if (
              returnKey.current &&
              rowButtons.current.has(returnKey.current)
            )
              rowButtons.current.get(returnKey.current)?.focus();
            else panel.current?.focus();
          }}
        >
          <DialogHeader>
            <div className="flex items-center justify-between gap-3">
              <DialogTitle>{t("usageSessions.detail")}</DialogTitle>
              <Button variant="ghost" onClick={() => setSelected(undefined)}>
                {t("usageWorkbench.close")}
              </Button>
            </div>
          </DialogHeader>
          {selected && (
            <div className="max-h-[70vh] space-y-4 overflow-y-auto p-4 text-sub">
              <p className="text-note text-ink-muted">
                {t("usageWorkbench.rangeSummary", {
                  start:
                    selected.query.startMs == null
                      ? t("usageWorkbench.beginning")
                      : new Date(selected.query.startMs).toLocaleDateString(
                          i18n.language,
                        ),
                  end: new Date(selected.query.endMs - 1).toLocaleString(
                    i18n.language,
                  ),
                  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
                })}
              </p>
              <div className="flex min-w-0 items-center gap-2">
                <ToolBadge toolId={selected.row.toolId} size={24} />
                <span className="break-all font-mono text-note text-ink-muted">
                  {selected.row.nativeSessionId}
                </span>
              </div>
              <h3 className="break-words font-medium">{selected.row.title}</h3>
              <p className="break-all font-mono text-note text-ink-muted">
                {project(
                  selected.row.projectId,
                  selected.row.projectPath ?? selected.row.projectLabel,
                )}
              </p>
              <dl className="grid grid-cols-2 gap-4">
                {[
                  [
                    "Tokens",
                    selected.row.tokens
                      ? fmtTokens(selected.row.tokens.totalTokens)
                      : "—",
                  ],
                  [
                    t("usageWorkbench.currentEstimate"),
                    formatUsageEstimate(selected.row.estimate),
                  ],
                  [
                    t("usageSessions.messages"),
                    selected.row.messages?.toLocaleString(i18n.language) ?? "—",
                  ],
                  [
                    t("usageSessions.activity"),
                    duration(selected.row.activeMs),
                  ],
                  [
                    t("usagePage.cacheRead"),
                    selected.row.tokens
                      ? fmtTokens(selected.row.tokens.cacheReadTokens)
                      : "—",
                  ],
                  [
                    t("usageSessions.cacheWrite"),
                    selected.row.tokens
                      ? fmtTokens(selected.row.tokens.cacheWriteTokens)
                      : "—",
                  ],
                ].map(([label, value]) => (
                  <div key={label}>
                    <dt className="text-note text-ink-muted">{label}</dt>
                    <dd className="tabular-nums">{value}</dd>
                  </div>
                ))}
              </dl>
              {selected.row.models.slice(0, detailModelLimit).map((model) => (
                <div
                  key={model.model}
                  className="flex min-w-0 items-center justify-between gap-3"
                >
                  <span className="truncate" title={model.model}>
                    {model.model}
                  </span>
                  <span className="shrink-0 tabular-nums">
                    {fmtTokens(model.totalTokens)} ·{" "}
                    {formatUsageEstimate(model.estimate)}
                  </span>
                </div>
              ))}
              {selected.row.models.length > detailModelLimit && (
                <Button
                  variant="ghost"
                  onClick={() => setDetailModelLimit((limit) => limit + 50)}
                >
                  {t("usageSessions.showMore", {
                    count: selected.row.models.length - detailModelLimit,
                  })}
                </Button>
              )}
              {!selected.row.sourceAvailable && (
                <p className="text-note text-ink-muted">
                  {t("usageSessions.sourceUnavailable")}
                </p>
              )}
              {!!selected.row.reasons.length && (
                <p className="text-note text-ink-muted">
                  {t("usageWorkbench.partial")}
                </p>
              )}
              <p className="text-note text-ink-faint">
                {t("usageSessions.frozenDetail")}
              </p>
              <Button
                variant="secondary"
                disabled={
                  !onManage ||
                  !selected.row.sourceAvailable ||
                  !selected.row.managerKey
                }
                onClick={() => {
                  if (selected.row.sourceAvailable && selected.row.managerKey) {
                    handingOff.current = true;
                    setSelected(undefined);
                    onManage?.(selected.row.managerKey);
                  }
                }}
              >
                {t("usageSessions.openManager")}
              </Button>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </section>
  );
}
