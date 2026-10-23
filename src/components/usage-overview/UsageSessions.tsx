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
import { usageIsUnavailable } from "@/lib/usageWorkbench";
import type {
  AnalyticsSession,
  SessionAnalytics,
} from "@/types/sessionAnalytics";
import { UsageBars, type UsageMetric } from "./UsageBars";
import { usageProjectLabel } from "@/lib/usageProject";
import { UsageSourceAlert } from "./UsageSourceAlert";
import { UsageSessionRanking } from "./UsageSessionRanking";
import { UsageTimeInsights } from "./UsageTimeInsights";
import { UsageDistributions } from "./UsageDistributions";
import {
  UsageSessionShape,
  UsageSkillInsights,
  UsageToolInsights,
} from "./UsageInteractions";
import { SegmentedControl } from "@/components/ui/segmented";
import {
  formatActivity,
  isPartialActivity,
  usageCohortIsUnavailable,
} from "@/lib/usageInsights";
import { getSessionAnalyticsPage } from "@/lib/api/usageOverview";
import { UsageBreakdownList } from "./UsageBreakdownList";
import type { UsageQueryV1 } from "@/types/usageQuery";

export function UsageSessions({
  data,
  active,
  onProject,
  onModel,
  onTool,
  onRemoveProject,
  onRemoveModel,
  onRemoveTool,
  onManage,
  showData = true,
  location,
  onLocated,
  onExpandScope,
  onRetry,
}: {
  data: SessionAnalytics;
  active: boolean;
  onProject?: (id: string) => void;
  onModel?: (id: string) => void;
  onTool?: (id: string) => void;
  onRemoveProject?: (id: string) => void;
  onRemoveModel?: (id: string) => void;
  onRemoveTool?: (id: string) => void;
  onManage?: (key: string) => void;
  showData?: boolean;
  location?: { key: string; request: number } | null;
  onLocated?: (request: number) => void;
  onExpandScope?: () => void;
  onRetry?: () => void;
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
  const [locationState, setLocationState] = useState<
    "loading" | "missing" | "unsupported" | null
  >(null);
  useEffect(() => {
    if (!active || !showData || !location) return;
    if (!data.analyticsSnapshotId) {
      setLocationState("unsupported");
      return;
    }
    let stale = false;
    setLocationState("loading");
    void getSessionAnalyticsPage({
      analyticsSnapshotId: data.analyticsSnapshotId,
      expectedReceipt: data,
      managerKey: location.key,
      sortBy: "tokens",
      page: 0,
      pageSize: 1,
    })
      .then((page) => {
        if (stale) return;
        const row = page.data[0];
        if (!row) {
          setLocationState("missing");
          return;
        }
        handingOff.current = false;
        returnFocus.current = undefined;
        returnKey.current = row.sessionKey;
        setSelected({
          row,
          version: data.snapshotVersion,
          query: data.appliedQuery,
        });
        setLocationState(null);
        onLocated?.(location.request);
      })
      .catch(() => {
        if (!stale) setLocationState("unsupported");
      });
    return () => {
      stale = true;
    };
  }, [active, showData, location, data, onLocated]);
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
  const duration = (ms: number | null) => formatActivity(ms, t);
  const quantile = (value: string | number | null) =>
    value == null ? "—" : fmtTokens(Number(value));
  const [rankKind, setRankKind] = useState<"projects" | "tools">("projects");
  const known =
    data.metricCoverage?.state === "unavailable" ? null : data.knownMetrics;
  const messageCoverage = data.messageCoverage ?? data.metricCoverage;
  const activityCoverage = data.activityCoverage ?? data.metricCoverage;
  const activeMs =
    activityCoverage?.state === "unavailable"
      ? null
      : (v.activeMs ?? data.knownActiveMs ?? known?.activeMs ?? null);
  const messages =
    messageCoverage?.state === "unavailable"
      ? null
      : (v.messages ?? data.knownMessages ?? known?.messages ?? null);
  const activeDays =
    activityCoverage?.state === "unavailable"
      ? null
      : (v.activeDays ?? data.knownActiveDays ?? known?.activeDays ?? null);
  const noUsage = usageIsUnavailable(data);
  const noCohort = usageCohortIsUnavailable(data);
  const nativeMethod = data.methodVersion === "magpie-native-v1";
  const activityLabel = t(
    nativeMethod ? "usageExperience.nativeActivity" : "usageSessions.activity",
  );

  return (
    <section
      ref={panel}
      tabIndex={-1}
      aria-label={t("usageSessions.title")}
      className="min-w-0 space-y-2 outline-none"
    >
      {showData && (
        <>
          {location && locationState && (
            <div
              role="status"
              className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-line bg-card px-3 py-2 text-sub text-ink-muted"
            >
              <span>
                {t(
                  locationState === "loading"
                    ? "common.loading"
                    : locationState === "missing"
                      ? "usageExperience.locationMissing"
                      : "usageExperience.rankUnavailable",
                )}
              </span>
              {locationState === "missing" && onExpandScope && (
                <Button variant="secondary" onClick={onExpandScope}>
                  {t("usageExperience.expandScope")}
                </Button>
              )}
              <Button
                variant="ghost"
                onClick={() => onLocated?.(location.request)}
              >
                {t("usageWorkbench.close")}
              </Button>
            </div>
          )}
          <UsageSourceAlert
            sources={data.sources}
            pricing={data.pricingCoverage}
            onRetry={onRetry}
          />
          <KpiStrip
            className="!grid-cols-2 min-[420px]:!grid-cols-3 [&>div]:px-3 [&>div]:pb-2 [&>div]:pt-2.5 [&_b]:text-[24px]"
            items={[
              {
                label: t("usageSessions.sessions"),
                value: noCohort
                  ? "—"
                  : v.sessionCount.toLocaleString(i18n.language),
                sub: t("usageSessions.quantiles", {
                  median: noUsage ? "—" : quantile(v.medianTokens),
                  p90: noUsage ? "—" : quantile(v.p90Tokens),
                }),
                title: noUsage
                  ? "—"
                  : `${v.medianTokens ?? "—"} / ${v.p90Tokens ?? "—"}`,
              },
              {
                label: "Tokens",
                value: noUsage ? "—" : fmtTokens(v.totalTokens),
                sub: t("usagePage.io", {
                  input: noUsage ? "—" : fmtTokens(v.inputTokens),
                  output: noUsage ? "—" : fmtTokens(v.outputTokens),
                }),
              },
              {
                label: t("usageWorkbench.currentEstimate"),
                value: noUsage ? "—" : formatUsageEstimate(v.estimate),
                sub: t("usagePage.estimate"),
              },
              {
                label: t("usagePage.cacheRead"),
                value: noUsage ? "—" : fmtTokens(v.cacheReadTokens),
                sub: t("usagePage.hitRate", {
                  rate:
                    noUsage || v.cacheHitRate == null
                      ? "—"
                      : `${Math.round(v.cacheHitRate * 100)}%`,
                }),
              },
              {
                label: activityLabel,
                value: duration(activeMs),
                title: t(
                  nativeMethod
                    ? "usageExperience.nativeActivityHelp"
                    : "usageSessions.activityExplanation",
                ),
                sub: t("usageSessions.activeDays", {
                  value: activeDays ?? "—",
                }),
              },
              {
                label: t("usageSessions.projects"),
                value: noCohort
                  ? "—"
                  : v.projectCount.toLocaleString(i18n.language),
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
            {t("usageExperience.aux", {
              messages: messages?.toLocaleString(i18n.language) ?? "—",
              cache: noUsage ? "—" : fmtTokens(v.cacheWriteTokens),
            })}
          </p>
          <UsageBars
            activityLabel={activityLabel}
            selectedMetric={metric}
            onMetricChange={setMetric}
            analysis
            overview={{
              granularity: data.trend?.granularity ?? "day",
              series: (data.trend?.series ?? data.series).map((day) => ({
                ...day,
                start: day.startMs,
                messages: day.messages ?? day.knownMessages ?? null,
                activeMs: day.activeMs ?? day.knownActiveMs ?? null,
              })),
            }}
            unavailableMetrics={[
              ...(noUsage
                ? ["tokens" as const, "output" as const, "cost" as const]
                : []),
              ...(noCohort ? ["sessions" as const] : []),
              ...(messages == null ? ["messages" as const] : []),
              ...(activeMs == null ? ["active" as const] : []),
            ]}
          />
          <div className="grid min-w-0 gap-2 min-[720px]:grid-cols-2">
            <UsageBreakdownList
              headerControl={
                <SegmentedControl
                  aria-label={t("usageExperience.projectTools")}
                  value={rankKind}
                  onChange={setRankKind}
                  options={[
                    { value: "projects", label: t("usageExperience.projects") },
                    { value: "tools", label: t("usageExperience.tools") },
                  ]}
                />
              }
              key={rankKind}
              kind={rankKind}
              rows={
                rankKind === "projects"
                  ? data.byProject.map((row) => ({
                      ...row,
                      label: project(row.id, row.label, row.path),
                    }))
                  : data.byTool
              }
              total={v.totalTokens}
              unavailable={noUsage}
              onFilter={rankKind === "projects" ? onProject : onTool}
              onRemove={
                rankKind === "projects" ? onRemoveProject : onRemoveTool
              }
              selectedIds={
                rankKind === "projects"
                  ? data.appliedQuery.projectIds
                  : data.appliedQuery.toolIds
              }
              pageSize={50}
              initialLimit={6}
              storageKey={`ofox.usageAnalysisHidden.${rankKind}`}
              compact
            />
            <UsageBreakdownList
              kind="models"
              rows={data.byModel}
              total={v.totalTokens}
              unavailable={noUsage}
              onFilter={onModel}
              onRemove={onRemoveModel}
              selectedIds={data.appliedQuery.modelIds}
              pageSize={50}
              initialLimit={6}
              storageKey="ofox.usageAnalysisHidden.models"
              title={t("usageSessions.modelRank")}
              compact
            />
          </div>
          <div className="grid min-w-0 items-stretch gap-2 min-[720px]:grid-cols-2">
            <UsageTimeInsights data={data} />
            <UsageSessionRanking
              data={data}
              active={active}
              onSelect={(row, event, context) => {
                handingOff.current = false;
                returnFocus.current = event.currentTarget;
                returnKey.current = row.sessionKey;
                setDetailModelLimit(50);
                setSelected({
                  row,
                  version: context.snapshotVersion,
                  query: context.appliedQuery,
                });
              }}
            />
          </div>
          <div className="grid min-w-0 items-stretch gap-2 min-[720px]:grid-cols-2">
            <UsageSessionShape
              distributions={data.distributions}
              interactions={data.interactions}
            />
            <UsageSkillInsights data={data.interactions} />
          </div>
          <UsageToolInsights data={data.interactions} />
          <UsageDistributions
            distributions={data.distributions}
            activityLabel={activityLabel}
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
                    activityLabel,
                    `${duration(selected.row.activeMs)}${isPartialActivity(selected.row) ? "＋" : ""}`,
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
