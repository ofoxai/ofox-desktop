import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { useShell, useShellRefresh } from "@/components/shell/ShellContext";
import { useInteractionActive } from "@/components/ui/interaction-scope";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { MenuSelect } from "@/components/ui/menu-select";
import { SegmentedNav } from "@/components/ui/segmented";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useUsageOverview } from "@/hooks/useUsageOverview";
import {
  canRefreshUsageNow,
  useUsageVisibility,
} from "@/hooks/useUsageVisibility";
import { settingsApi } from "@/lib/api";
import {
  getSessionUsageAnalytics,
  getUsageCapabilities,
} from "@/lib/api/usageOverview";
import { queryClient } from "@/lib/query/queryClient";
import { ofoxDashboardUrl, ofoxWalletUrl } from "@/lib/ofoxUrls";
import {
  createUsageQuery,
  legacyPeriod,
  resolveWorkbenchRange,
  type WorkbenchFilters,
  type WorkbenchRange,
} from "@/lib/usageWorkbench";
import { createUsageRefreshCoordinator } from "@/lib/usageRefresh";
import { TOOL_META } from "@/config/toolMeta";
import {
  UNKNOWN_USAGE_PROJECT_ID,
  usageProjectLabel,
} from "@/lib/usageProject";
import type { SessionAnalytics } from "@/types/sessionAnalytics";
import { UsageHeader } from "./UsageHeader";
import { BalanceTrendCard } from "./BalanceTrendCard";
import { UsageKpis } from "./UsageKpis";
import { UsageBars } from "./UsageBars";
import { UsageBreakdownList } from "./UsageBreakdownList";
import { UsageFilterMenu } from "./UsageFilterMenu";
import { UsageRequests } from "./UsageRequests";
import { UsageFreshness } from "./UsageFreshness";
import { UsageSessions } from "./UsageSessions";

const EMPTY: string[] = [];
type Tab = "overview" | "requests" | "sessions";
export function UsagePage({
  account,
  boundTools = EMPTY,
}: {
  account: OfoxAccount;
  boundTools?: string[];
}) {
  const { t, i18n } = useTranslation();
  const { apex } = useOfoxApex();
  const shell = useShell();
  const interactionActive = useInteractionActive();
  const visible = useUsageVisibility(
    interactionActive && shell.active === "usage",
  );
  const [tab, setTab] = useState<Tab>(() => {
    const saved = localStorage.getItem("ofox.usageTab");
    return saved === "requests" || saved === "sessions" ? saved : "overview";
  });
  const [range, setRange] = useState<WorkbenchRange>(() => {
    const value = localStorage.getItem("ofox.usagePeriod");
    return {
      period: ["today", "7d", "30d", "90d", "all"].includes(value ?? "")
        ? (value as WorkbenchRange["period"])
        : "30d",
    };
  });
  const [filters, setFilters] = useState<WorkbenchFilters>({});
  const [analysisProjects, setAnalysisProjects] = useState<string[]>([]);
  const [projectsCleared, setProjectsCleared] = useState(false);
  const [anchor, setAnchor] = useState(Date.now);
  const query = useMemo(
    () => createUsageQuery(range, filters, anchor),
    [range, filters, anchor],
  );
  const capabilities = useQuery(
    {
      queryKey: ["usageCapabilities"],
      queryFn: getUsageCapabilities,
      retry: false,
      staleTime: Infinity,
      enabled: visible,
      refetchOnWindowFocus: false,
    },
    queryClient,
  );
  const modern = capabilities.data?.queryVersion === 1;
  const requestSupported = modern && !!capabilities.data?.requestSnapshots;
  const analysisSupported = modern && !!capabilities.data?.sessionAnalytics;
  const analysisProjectSupported =
    analysisSupported && !!capabilities.data?.sessionProjectFilter;
  const effectiveTab =
    ((tab === "requests" && !requestSupported) ||
      (tab === "sessions" && !analysisSupported)) &&
    !capabilities.isPending
      ? "overview"
      : tab;
  const analysisQuery = useMemo(
    () => ({ ...query, projectIds: analysisProjects }),
    [query, analysisProjects],
  );
  const analysis = useQuery(
    {
      queryKey: ["sessionUsageAnalytics", apex, analysisQuery],
      queryFn: () => getSessionUsageAnalytics(analysisQuery),
      enabled: visible && effectiveTab === "sessions" && analysisSupported,
      retry: false,
      staleTime: Infinity,
      refetchOnWindowFocus: false,
    },
    queryClient,
  );
  const [lastAnalysis, setLastAnalysis] = useState<SessionAnalytics>();
  const analysisDisplay = analysis.data ?? lastAnalysis;
  useEffect(() => {
    if (
      capabilities.isFetched &&
      !modern &&
      (range.period === "90d" || range.period === "custom")
    )
      setRange({ period: "30d" });
  }, [capabilities.isFetched, modern, range.period]);
  const [options, setOptions] = useState({
    toolIds: EMPTY,
    modelIds: EMPTY,
    sourceIds: EMPTY,
  });
  const [optionsLoaded, setOptionsLoaded] = useState(false);
  const updateOptions = useCallback((next: typeof options) => {
    setOptions(next);
    setOptionsLoaded(true);
  }, []);
  const state = useUsageOverview(
    legacyPeriod(range.period),
    account.user,
    apex,
    {
      query: modern ? query : undefined,
      enabled:
        visible &&
        capabilities.isFetched &&
        (effectiveTab === "overview" ||
          (effectiveTab === "requests" && !optionsLoaded)),
      accountEnabled: visible && effectiveTab === "overview",
      localRefreshSupported: modern && !!capabilities.data?.localRefresh,
    },
  );
  const data = state.overview.data;
  useEffect(() => {
    if (data) {
      updateOptions(
        data.options ?? {
          toolIds: data.byTool.map((item) => item.appType),
          modelIds: data.byModel.map((item) => item.model),
          sourceIds: [],
        },
      );
    }
  }, [data, updateOptions]);
  useEffect(() => {
    if (analysis.data) setLastAnalysis(analysis.data);
    if (analysis.data && effectiveTab === "sessions")
      updateOptions(analysis.data.options);
  }, [analysis.data, effectiveTab, updateOptions]);
  const [customOpen, setCustomOpen] = useState(false);
  const [customStart, setCustomStart] = useState("");
  const [customEnd, setCustomEnd] = useState("");
  const [rangeError, setRangeError] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const [interval, setIntervalValue] = useState(() => {
    const saved = Number(
      localStorage.getItem("ofox.usageRefreshSeconds") ?? 30,
    );
    return [0, 5, 10, 30, 60].includes(saved) ? saved : 30;
  });
  const activeRef = useRef(visible);
  activeRef.current = visible;
  const refreshOwner = useMemo(
    () => ({
      query: effectiveTab === "sessions" ? analysisQuery : query,
      apex,
      tab: effectiveTab,
      range,
      analysisSupported,
    }),
    [
      query,
      analysisQuery,
      apex,
      effectiveTab,
      range,
      visible,
      analysisSupported,
    ],
  );
  const refreshOwnerRef = useRef(refreshOwner);
  refreshOwnerRef.current = refreshOwner;
  const manualRequested = useRef(false);
  const runRef = useRef<() => Promise<void>>(async () => undefined);
  runRef.current = async () => {
    const owner = refreshOwnerRef.current;
    const includeAccount = manualRequested.current;
    manualRequested.current = false;
    if (
      !activeRef.current ||
      !(await canRefreshUsageNow()) ||
      !activeRef.current
    )
      return;
    setRefreshError(null);
    try {
      await (includeAccount ? state.refresh() : state.refreshLocal());
      if (!activeRef.current || refreshOwnerRef.current !== owner) return;
      if (owner.tab === "sessions" && owner.analysisSupported) {
        const nextAnchor = Date.now();
        const nextQuery = {
          ...owner.query,
          ...resolveWorkbenchRange(owner.range, nextAnchor),
        };
        // Stage the read before changing the visible query. Only its owner may
        // publish it, including when a custom range would reuse the same key.
        const next = await getSessionUsageAnalytics(nextQuery);
        if (!activeRef.current || refreshOwnerRef.current !== owner) return;
        queryClient.setQueryData(
          ["sessionUsageAnalytics", owner.apex, nextQuery],
          next,
        );
        setAnchor(nextAnchor);
      } else if (effectiveTab === "overview") setAnchor(Date.now());
    } catch (error) {
      if (activeRef.current && refreshOwnerRef.current === owner)
        setRefreshError(String(error));
      throw error;
    }
  };
  const coordinator =
    useRef<ReturnType<typeof createUsageRefreshCoordinator>>();
  useEffect(() => {
    const owner = createUsageRefreshCoordinator(() => runRef.current());
    owner.setEnabled(activeRef.current);
    coordinator.current = owner;
    return () => {
      owner.dispose();
      if (coordinator.current === owner) coordinator.current = undefined;
    };
  }, []);
  useEffect(() => {
    coordinator.current?.setEnabled(visible);
    if (!visible) manualRequested.current = false;
  }, [visible]);
  useEffect(() => {
    if (!visible || !interval) return;
    const timer = window.setInterval(
      () =>
        void coordinator.current?.request("automatic").catch(() => undefined),
      interval * 1000,
    );
    return () => window.clearInterval(timer);
  }, [visible, interval]);
  const manualRefresh = () => {
    manualRequested.current = true;
    return coordinator.current?.request("manual") ?? Promise.resolve();
  };
  useShellRefresh("usage", manualRefresh);
  const chooseRange = (next: WorkbenchRange) => {
    setRange(next);
    setAnchor(Date.now());
    localStorage.setItem("ofox.usagePeriod", next.period);
    setCustomOpen(false);
    setRangeError(false);
  };
  const open = (url: string) =>
    void settingsApi
      .openExternal(url)
      .catch(() => toast.error(t("shell.openFailed")));
  const unsupported =
    data?.unsupportedTools.filter((id) => boundTools.includes(id)) ?? [];
  const errors = (
    effectiveTab === "sessions"
      ? [analysis.error]
      : effectiveTab === "overview"
        ? [
            state.overview.error,
            state.balance.error,
            state.trend.error,
            state.spending.error,
          ]
        : []
  ).filter(Boolean);
  const toolIds = [
    ...new Set([...options.toolIds, ...(filters.toolIds ?? [])]),
  ];
  const modelIds = [
    ...new Set([...options.modelIds, ...(filters.modelIds ?? [])]),
  ];
  const projects = analysis.data?.options.projects ?? [];
  const projectLabels = new Map<string, number>();
  for (const project of projects)
    projectLabels.set(
      project.label,
      (projectLabels.get(project.label) ?? 0) + 1,
    );
  const projectNames = {
    unknown: t("usageSessions.unknownProject"),
    unparsed: t("usageSessions.projectUnparsed"),
  };
  const projectOptions = projects.map((project) => {
    const label = usageProjectLabel(
      project.id,
      project.label,
      project.path,
      projectNames,
    );
    return {
      value: project.id,
      label:
        project.id !== UNKNOWN_USAGE_PROJECT_ID &&
        (projectLabels.get(project.label) ?? 0) > 1 &&
        project.path
          ? `${label} · ${project.path}`
          : label,
      title:
        project.id === UNKNOWN_USAGE_PROJECT_ID
          ? label
          : (project.path ?? label),
    };
  });
  for (const id of analysisProjects)
    if (!projectOptions.some((option) => option.value === id)) {
      const label = usageProjectLabel(id, null, null, projectNames);
      projectOptions.push({ value: id, label, title: label });
    }
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <SegmentedNav
          aria-label={t("usageWorkbench.tabs")}
          value={effectiveTab}
          onChange={(value) => {
            if (value !== "sessions" && analysisProjects.length) {
              setAnalysisProjects([]);
              setProjectsCleared(true);
            }
            setTab(value);
            localStorage.setItem("ofox.usageTab", value);
          }}
          items={[
            {
              value: "overview",
              label: t("usageWorkbench.overview"),
              controls: "usage-overview-panel",
            },
            ...(requestSupported
              ? [
                  {
                    value: "requests" as const,
                    label: t("usageWorkbench.requests"),
                    controls: "usage-requests-panel",
                  },
                ]
              : []),
            ...(analysisSupported
              ? [
                  {
                    value: "sessions" as const,
                    label: t("usageSessions.title"),
                    controls: "usage-sessions-panel",
                  },
                ]
              : []),
          ]}
        />
        <div className="flex items-center gap-2">
          <MenuSelect
            aria-label={t("usageWorkbench.refreshEvery")}
            value={String(interval)}
            onChange={(value) => {
              setIntervalValue(Number(value));
              localStorage.setItem("ofox.usageRefreshSeconds", value);
            }}
            options={[0, 5, 10, 30, 60].map((value) => ({
              value: String(value),
              label: value
                ? t("usageWorkbench.seconds", { count: value })
                : t("usageWorkbench.refreshOff"),
            }))}
          />
          <Button
            variant="secondary"
            onClick={() => void manualRefresh().catch(() => undefined)}
          >
            {t("usageWorkbench.refresh")}
          </Button>
        </div>
      </div>
      <UsageHeader
        period={range.period}
        modern={modern}
        onChange={(period) =>
          period === "custom"
            ? setCustomOpen((value) => !value)
            : chooseRange({ period })
        }
        cost={effectiveTab === "overview" ? data?.totals.costUsd : undefined}
        estimate={
          effectiveTab === "overview" ? data?.totals.estimate : undefined
        }
      />
      {customOpen && (
        <form
          className="flex flex-wrap items-end gap-2 rounded-card border border-line bg-card p-3"
          onSubmit={(event) => {
            event.preventDefault();
            const next: WorkbenchRange = {
              period: "custom",
              startDate: customStart,
              endDate: customEnd,
            };
            try {
              resolveWorkbenchRange(next);
              chooseRange(next);
            } catch {
              setRangeError(true);
            }
          }}
        >
          <label className="grid gap-1 text-note text-ink-muted">
            {t("usageWorkbench.startDate")}
            <Input
              type="date"
              value={customStart}
              onChange={(event) => setCustomStart(event.target.value)}
            />
          </label>
          <label className="grid gap-1 text-note text-ink-muted">
            {t("usageWorkbench.endDate")}
            <Input
              type="date"
              value={customEnd}
              onChange={(event) => setCustomEnd(event.target.value)}
            />
          </label>
          <Button type="submit">{t("usageWorkbench.applyRange")}</Button>
          {rangeError && (
            <p role="alert" className="w-full text-note text-warn">
              {t("usageWorkbench.invalidRange")}
            </p>
          )}
        </form>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <UsageFilterMenu
          label={t("usageWorkbench.toolsFilter")}
          options={toolIds.map((value) => ({
            value,
            label: TOOL_META[value]?.label ?? value,
          }))}
          selected={filters.toolIds ?? []}
          onChange={(toolIds) => setFilters((old) => ({ ...old, toolIds }))}
          disabled={!modern}
        />
        <UsageFilterMenu
          label={t("usageWorkbench.modelsFilter")}
          pageSize={effectiveTab === "sessions" ? 50 : undefined}
          options={modelIds.map((value) => ({ value, label: value }))}
          selected={filters.modelIds ?? []}
          onChange={(modelIds) => setFilters((old) => ({ ...old, modelIds }))}
          disabled={!modern}
        />
        {effectiveTab === "sessions" && analysisProjectSupported ? (
          <UsageFilterMenu
            label={t("usageSessions.projectFilter")}
            pageSize={50}
            options={projectOptions}
            selected={analysisProjects}
            onChange={(ids) => {
              setAnalysisProjects(ids);
              setProjectsCleared(false);
            }}
          />
        ) : (
          <span
            className="text-note text-ink-faint"
            title={t("usageWorkbench.projectUnavailable")}
          >
            {t("usageWorkbench.projectUnavailable")}
          </span>
        )}
        {!!(
          (filters.toolIds?.length ?? 0) +
          (filters.modelIds?.length ?? 0) +
          analysisProjects.length
        ) && (
          <Button
            variant="ghost"
            onClick={() => {
              setFilters({});
              setAnalysisProjects([]);
            }}
          >
            {t("usageWorkbench.reset")}
          </Button>
        )}
      </div>
      {effectiveTab === "sessions" && analysisProjectSupported && (
        <p className="text-note text-ink-muted">
          {t("usageSessions.projectScope")}
        </p>
      )}
      {projectsCleared && effectiveTab !== "sessions" && (
        <p role="status" className="text-note text-ink-muted">
          {t("usageSessions.projectsCleared")}
        </p>
      )}
      <p className="text-note text-ink-muted">
        {t("usageWorkbench.rangeSummary", {
          start:
            query.startMs == null
              ? t("usageWorkbench.beginning")
              : new Date(query.startMs).toLocaleDateString(i18n.language),
          end: new Date(query.endMs - 1).toLocaleString(i18n.language),
          timezone,
        })}
      </p>
      {capabilities.isFetched && !modern && (
        <p role="status" className="text-note text-ink-muted">
          {t("usageWorkbench.oldBackend")}
        </p>
      )}
      {(errors.length > 0 || refreshError) && (
        <div
          role="alert"
          className="rounded-card border border-warn/30 bg-warn-soft px-3 py-2 text-sub text-warn"
        >
          {t("usagePage.refreshFailed")}
          <details>
            <summary>{t("usagePage.errorDetails")}</summary>
            <pre className="whitespace-pre-wrap break-words text-note">
              {refreshError || errors.map(String).join("\n")}
            </pre>
          </details>
          <Button
            variant="ghost"
            onClick={() => void manualRefresh().catch(() => undefined)}
          >
            {t("common.retry")}
          </Button>
        </div>
      )}
      <section
        id="usage-overview-panel"
        role="tabpanel"
        aria-label={t("usageWorkbench.overview")}
        hidden={effectiveTab !== "overview"}
        className="space-y-3"
      >
        <p className="text-note font-medium text-ink-muted">
          {t("usageWorkbench.currentAccount")}
        </p>
        <BalanceTrendCard
          user={state.user}
          trend={state.trend.data}
          spending={state.spending.data}
          recharge={() => open(ofoxWalletUrl(apex))}
        />
        <p className="pt-1 text-note font-medium text-ink-muted">
          {t("usageWorkbench.localHistory")}
        </p>
        {state.overview.isPending ? (
          <p role="status" className="p-6 text-center text-ink-muted">
            {t("common.loading")}
          </p>
        ) : !data ||
          (![
            data.totals.requests,
            data.totals.inputTokens,
            data.totals.outputTokens,
            data.totals.cacheReadTokens,
            data.totals.cacheWriteTokens,
            data.totals.costUsd,
            Number(data.totals.estimate?.knownCostUsd),
          ].some((value) => value > 0) &&
            data.coverage?.state !== "partial" &&
            !data.coverage?.aggregateOnly) ? (
          <div className="rounded-list border border-line bg-card p-8 text-center text-sub text-ink-muted">
            {t(
              state.overview.isError || data?.coverage?.state === "unsupported"
                ? "usagePage.unavailable"
                : "usagePage.empty",
            )}
          </div>
        ) : (
          <>
            <UsageKpis totals={data.totals} />
            <UsageBars overview={data} />
            <div className="grid gap-3 md:grid-cols-2">
              <UsageBreakdownList
                kind="tools"
                rows={data.byTool.map((row) => ({ ...row, id: row.appType }))}
                total={data.totals.totalTokens}
                onFilter={
                  modern
                    ? (id) =>
                        setFilters((old) => ({
                          ...old,
                          toolIds: old.toolIds?.includes(id) ? [] : [id],
                        }))
                    : undefined
                }
              />
              <UsageBreakdownList
                kind="models"
                rows={data.byModel.map((row) => ({ ...row, id: row.model }))}
                total={data.totals.totalTokens}
                onFilter={
                  modern
                    ? (id) =>
                        setFilters((old) => ({
                          ...old,
                          modelIds: old.modelIds?.includes(id) ? [] : [id],
                        }))
                    : undefined
                }
              />
            </div>
          </>
        )}
        <UsageFreshness
          localUpdatedAt={data?.lastSyncedAt ?? null}
          accountUpdatedAt={
            account.user ? state.balance.dataUpdatedAt || null : undefined
          }
          sources={data?.sources}
          pricing={data?.pricingCoverage}
        />
        {data?.coverage?.state === "unsupported" && (
          <p className="text-note text-ink-muted">
            {t("usageWorkbench.queryUnavailable")}
          </p>
        )}
        {data?.coverage?.state === "partial" && (
          <p className="text-note text-ink-muted">
            {t("usageWorkbench.partial")}
          </p>
        )}
        {data?.coverage?.timeBasis !== undefined &&
          data.coverage.timeBasis !== "eventTime" && (
            <p className="text-note text-ink-muted">
              {t("usageWorkbench.legacyDates")}
            </p>
          )}
        {data?.pricingCoverage?.additionalUnitsUnsupported && (
          <p className="text-note text-ink-muted">
            {t("usageWorkbench.unsupportedUnits")}
          </p>
        )}
      </section>
      {requestSupported && (
        <div
          id="usage-requests-panel"
          role="tabpanel"
          aria-label={t("usageWorkbench.requests")}
          hidden={effectiveTab !== "requests"}
        >
          <UsageRequests
            query={query}
            onOptions={updateOptions}
            apex={apex}
            active={visible && effectiveTab === "requests"}
            sourceIds={options.sourceIds}
            updateVersion={state.sourceVersion}
            canExport={!!capabilities.data?.csv}
            onLatest={() => setAnchor(Date.now())}
          />
        </div>
      )}
      {analysisSupported && (
        <div
          id="usage-sessions-panel"
          role="tabpanel"
          aria-label={t("usageSessions.title")}
          hidden={effectiveTab !== "sessions"}
        >
          {analysis.isPending ? (
            <p role="status" className="p-6 text-center text-ink-muted">
              {t("common.loading")}
            </p>
          ) : !analysis.data ||
            analysis.data.coverage.state === "unsupported" ? (
            <p className="rounded-list border border-line bg-card p-8 text-center text-sub text-ink-muted">
              {t("usagePage.unavailable")}
            </p>
          ) : null}
          {analysisDisplay && (
            <UsageSessions
              key={apex}
              data={analysisDisplay}
              showData={
                !!analysis.data &&
                analysis.data.coverage.state !== "unsupported"
              }
              active={visible && effectiveTab === "sessions"}
              onProject={
                analysisProjectSupported
                  ? (id) => {
                      setAnalysisProjects((old) =>
                        old.length === 1 && old[0] === id ? [] : [id],
                      );
                      setProjectsCleared(false);
                    }
                  : undefined
              }
              onModel={(id) =>
                setFilters((old) => ({
                  ...old,
                  modelIds: old.modelIds?.includes(id) ? [] : [id],
                }))
              }
              onManage={shell.locateSession}
            />
          )}
        </div>
      )}
      {!!unsupported.length && (
        <p className="text-note text-ink-muted">
          {t("usagePage.unsupported", {
            tools: unsupported
              .map((id) => TOOL_META[id]?.label ?? id)
              .join(", "),
          })}
        </p>
      )}
      <p className="text-note text-ink-faint">{t("usagePage.disclaimer")}</p>
      <Button
        variant="link"
        className="h-auto self-start px-0 text-note"
        onClick={() => open(ofoxDashboardUrl(apex))}
      >
        {t("shell.usage.webDetails")}
      </Button>
    </div>
  );
}
