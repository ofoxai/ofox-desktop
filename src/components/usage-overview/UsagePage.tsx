import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowRight, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useShell, useShellRefresh } from "@/components/shell/ShellContext";
import { useInteractionActive } from "@/components/ui/interaction-scope";
import { Button } from "@/components/ui/button";
import { DatePicker } from "@/components/ui/date-picker";
import { MenuSelect } from "@/components/ui/menu-select";
import { SegmentedControl, SegmentedNav } from "@/components/ui/segmented";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useUsageOverview } from "@/hooks/useUsageOverview";
import {
  canRefreshUsageNow,
  useUsageVisibility,
} from "@/hooks/useUsageVisibility";
import { settingsApi } from "@/lib/api";
import {
  getUsageOverview,
  getSessionUsageAnalytics,
  getUsageCapabilities,
  type UsageOverview,
} from "@/lib/api/usageOverview";
import { queryClient } from "@/lib/query/queryClient";
import { ofoxDashboardUrl, ofoxWalletUrl } from "@/lib/ofoxUrls";
import {
  createUsageQuery,
  usageIsUnavailable,
  legacyPeriod,
  resolveWorkbenchRange,
  type WorkbenchFilters,
  type WorkbenchRange,
  type WorkbenchPeriod,
} from "@/lib/usageWorkbench";
import { createUsageRefreshCoordinator } from "@/lib/usageRefresh";
import { inToolOrder, TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import {
  UNKNOWN_USAGE_PROJECT_ID,
  usageProjectLabel,
} from "@/lib/usageProject";
import type { SessionAnalytics } from "@/types/sessionAnalytics";
import type { UsageFilterOptions, UsageQueryV1 } from "@/types/usageQuery";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import { formatCalendarDate } from "@/lib/calendarDate";
import { UsageFilterToolbar, type UsageFilterChip } from "./UsageFilterToolbar";
import { BalanceTrendCard } from "./BalanceTrendCard";
import { UsageKpis } from "./UsageKpis";
import { UsageBars } from "./UsageBars";
import { UsageBreakdownList } from "./UsageBreakdownList";
import { UsageFilterMenu } from "./UsageFilterMenu";
import { UsageRequests } from "./UsageRequests";
import { UsageSourceAlert } from "./UsageSourceAlert";
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
  const [savedHistory, setSavedHistory] = useState<{
    apex: string;
    query: UsageQueryV1;
    filterScope: string;
  }>();
  useEffect(() => setSavedHistory(undefined), [apex]);
  const [projectsCleared, setProjectsCleared] = useState(false);
  const [sessionTarget, setSessionTarget] = useState<{
    key: string;
    request: number;
  } | null>(null);
  const { usageLocation, consumeUsageLocation } = shell;
  useEffect(() => {
    if (!usageLocation || shell.active !== "usage") return;
    setSessionTarget(usageLocation);
    setTab("sessions");
    localStorage.setItem("ofox.usageTab", "sessions");
    consumeUsageLocation(usageLocation.request);
  }, [usageLocation, consumeUsageLocation, shell.active]);
  const clearSessionTarget = useCallback((request: number) => {
    setSessionTarget((current) =>
      current?.request === request ? null : current,
    );
  }, []);
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
  const sharedProjectSupported = modern && !!capabilities.data?.projectFilter;
  const selectedProjects = sharedProjectSupported
    ? (filters.projectIds ?? EMPTY)
    : analysisProjects;
  const chooseProjects = (projectIds: string[]) => {
    if (sharedProjectSupported) setFilters((old) => ({ ...old, projectIds }));
    else setAnalysisProjects(projectIds);
    setProjectsCleared(false);
  };
  const effectiveTab =
    ((tab === "requests" && !requestSupported) ||
      (tab === "sessions" && !analysisSupported)) &&
    !capabilities.isPending
      ? "overview"
      : tab;
  const filterScope = JSON.stringify([range, filters, analysisProjects]);
  const historyScope =
    effectiveTab === "requests" &&
    savedHistory?.apex === apex &&
    savedHistory.filterScope === filterScope
      ? savedHistory
      : undefined;
  const displayedQuery = historyScope?.query ?? query;
  const openSavedHistory = (from: UsageQueryV1) => {
    setSavedHistory({ apex, query: from, filterScope });
    setTab("requests");
    localStorage.setItem("ofox.usageTab", "requests");
  };
  const analysisQuery = useMemo(
    () => ({ ...query, projectIds: selectedProjects }),
    [query, selectedProjects],
  );
  const analysis = useQuery(
    {
      queryKey: ["sessionUsageAnalytics", apex, analysisQuery],
      queryFn: () => getSessionUsageAnalytics(analysisQuery, apex),
      enabled: visible && effectiveTab === "sessions" && analysisSupported,
      retry: false,
      staleTime: Infinity,
      refetchOnWindowFocus: false,
    },
    queryClient,
  );
  const [lastAnalysis, setLastAnalysis] = useState<{
    apex: string;
    data: SessionAnalytics;
  }>();
  const analysisDisplay =
    analysis.data ??
    (lastAnalysis?.apex === apex &&
    lastAnalysis.data.pricingCoverage.apex === apex
      ? lastAnalysis.data
      : undefined);
  useEffect(() => {
    if (
      capabilities.isFetched &&
      !modern &&
      (range.period === "90d" || range.period === "custom")
    )
      setRange({ period: "30d" });
  }, [capabilities.isFetched, modern, range.period]);
  const [options, setOptions] = useState<UsageFilterOptions>({
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
  const [lastOverview, setLastOverview] = useState<{
    apex: string;
    data: UsageOverview;
  }>();
  const data =
    state.overview.data ??
    (lastOverview?.apex === apex &&
    (!lastOverview.data.pricingCoverage ||
      lastOverview.data.pricingCoverage.apex === apex)
      ? lastOverview.data
      : undefined);
  useEffect(() => {
    if (state.overview.data)
      setLastOverview({ apex, data: state.overview.data });
  }, [state.overview.data, apex]);
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
    if (analysis.data) setLastAnalysis({ apex, data: analysis.data });
    if (analysis.data && effectiveTab === "sessions")
      updateOptions(analysis.data.options);
  }, [analysis.data, effectiveTab, updateOptions, apex]);
  const [customOpen, setCustomOpen] = useState(false);
  const [customStart, setCustomStart] = useState("");
  const [customEnd, setCustomEnd] = useState("");
  const [rangeError, setRangeError] = useState(false);
  const [refreshPhase, setRefreshPhase] = useState<
    "sync" | "query" | "account" | null
  >(null);
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
    if (includeAccount) setRefreshError(null);
    setRefreshPhase("sync");
    let publishedQuery: UsageQueryV1 | undefined;
    const stage = async () => {
      if (!activeRef.current || refreshOwnerRef.current !== owner) return;
      if (owner.tab === "sessions" && owner.analysisSupported) {
        setRefreshPhase("query");
        const nextAnchor = Date.now();
        const nextQuery = {
          ...owner.query,
          ...resolveWorkbenchRange(owner.range, nextAnchor),
        };
        // Stage the read before changing the visible query. Only its owner may
        // publish it, including when a custom range would reuse the same key.
        const next = await getSessionUsageAnalytics(nextQuery, owner.apex);
        if (!activeRef.current || refreshOwnerRef.current !== owner) return;
        queryClient.setQueryData(
          ["sessionUsageAnalytics", owner.apex, nextQuery],
          next,
        );
        publishedQuery = nextQuery;
        setAnchor(nextAnchor);
      } else if (owner.tab === "overview" && modern) {
        setRefreshPhase("query");
        const nextAnchor = Date.now();
        const nextQuery = {
          ...owner.query,
          ...resolveWorkbenchRange(owner.range, nextAnchor),
        };
        const next = await getUsageOverview(
          legacyPeriod(owner.range.period),
          nextQuery,
          owner.apex,
        );
        if (!activeRef.current || refreshOwnerRef.current !== owner) return;
        queryClient.setQueryData(
          [
            "usageOverview",
            owner.apex,
            legacyPeriod(owner.range.period),
            nextQuery,
          ],
          next,
        );
        publishedQuery = nextQuery;
        setAnchor(nextAnchor);
      }
      if (includeAccount && owner.tab === "overview" && account.user)
        setRefreshPhase("account");
    };
    try {
      // Publish local success independently of account network completion.
      if (includeAccount) await state.refresh(stage);
      else {
        await state.refreshLocal();
        await stage();
      }
    } catch (error) {
      if (
        includeAccount &&
        activeRef.current &&
        (refreshOwnerRef.current === owner ||
          (publishedQuery &&
            owner.apex === refreshOwnerRef.current.apex &&
            owner.tab === refreshOwnerRef.current.tab &&
            JSON.stringify(publishedQuery) ===
              JSON.stringify(refreshOwnerRef.current.query)))
      )
        setRefreshError(String(error));
      throw error;
    } finally {
      setRefreshPhase(null);
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
  const retry = () => void manualRefresh().catch(() => undefined);
  const chooseRange = (next: WorkbenchRange) => {
    setRange(next);
    setAnchor(Date.now());
    localStorage.setItem("ofox.usagePeriod", next.period);
    setCustomOpen(false);
    setRangeError(false);
  };
  const toggleCustomRange = () => {
    if (!customOpen) {
      const current = resolveWorkbenchRange(range, anchor);
      setCustomStart(formatCalendarDate(new Date(current.startMs ?? anchor)));
      setCustomEnd(
        formatCalendarDate(
          new Date(current.endMs - (range.period === "custom" ? 1 : 0)),
        ),
      );
      setRangeError(false);
    }
    setCustomOpen((value) => !value);
  };
  const open = (url: string) =>
    void settingsApi
      .openExternal(url)
      .catch(() => toast.error(t("shell.openFailed")));
  // ChatGPT's Codex mode writes Codex sessions, so its usage is Codex's.
  const boundUsageTools = boundTools.map((id) =>
    id === "chatgpt" ? "codex" : id,
  );
  const unsupported =
    data?.unsupportedTools.filter((id) => boundUsageTools.includes(id)) ?? [];
  // Usage filters describe local history, not the current Ofox bindings.
  // Keep supported tools available even if the first statistics read fails.
  // ChatGPT's Codex mode is accounted for under Codex.
  const toolIds = inToolOrder([
    ...TOOL_ORDER.filter((id) => id !== "chatgpt"),
    ...options.toolIds,
    ...(filters.toolIds ?? []),
  ]);
  const modelIds = [
    ...new Set([...options.modelIds, ...(filters.modelIds ?? [])]),
  ];
  const projects = sharedProjectSupported
    ? (options.projects ?? [])
    : (analysis.data?.options.projects ?? []);
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
  for (const id of selectedProjects)
    if (!projectOptions.some((option) => option.value === id)) {
      const label = usageProjectLabel(id, null, null, projectNames);
      projectOptions.push({ value: id, label, title: label });
    }
  const filterChips: UsageFilterChip[] = [
    ...(filters.toolIds ?? []).map((id) => ({
      key: `tool:${id}`,
      name: t("usageWorkbench.toolsFilter"),
      value: TOOL_META[id]?.label ?? id,
      toolId: id,
      remove: () =>
        setFilters((old) => ({
          ...old,
          toolIds: old.toolIds?.filter((value) => value !== id),
        })),
    })),
    ...(filters.modelIds ?? []).map((id) => ({
      key: `model:${id}`,
      name: t("usageWorkbench.modelsFilter"),
      value: id,
      remove: () =>
        setFilters((old) => ({
          ...old,
          modelIds: old.modelIds?.filter((value) => value !== id),
        })),
    })),
    ...selectedProjects.map((id) => ({
      key: `project:${id}`,
      name: t("usageWorkbench.projectsFilter"),
      value: projectOptions.find((option) => option.value === id)?.label ?? id,
      title: projectOptions.find((option) => option.value === id)?.title,
      remove: () =>
        chooseProjects(selectedProjects.filter((value) => value !== id)),
    })),
  ];
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const describePrevious = (previous: UsageQueryV1) =>
    [
      t("usageWorkbench.rangeSummary", {
        start:
          previous.startMs == null
            ? t("usageWorkbench.beginning")
            : new Date(previous.startMs).toLocaleDateString(i18n.language),
        end: new Date(previous.endMs - 1).toLocaleString(i18n.language),
        timezone,
      }),
      `${t("usageWorkbench.toolsFilter")}: ${previous.toolIds.length ? previous.toolIds.map((id) => TOOL_META[id]?.label ?? id).join(", ") : t("usageFilters.allTools")}`,
      `${t("usageWorkbench.modelsFilter")}: ${previous.modelIds.length ? previous.modelIds.join(", ") : t("usageFilters.allModels")}`,
      `${t("usageWorkbench.projectsFilter")}: ${previous.projectIds.length ? previous.projectIds.map((id) => usageProjectLabel(id, null, null, projectNames)).join(", ") : t("usageFilters.allProjects")}`,
    ].join(" · ");
  const queryFetching =
    effectiveTab === "sessions"
      ? analysis.isFetching
      : effectiveTab === "overview"
        ? state.overview.isFetching
        : false;
  const refreshing = !!refreshPhase || queryFetching;
  const resultQuery =
    (effectiveTab === "sessions"
      ? analysisDisplay?.appliedQuery
      : effectiveTab === "overview"
        ? data?.appliedQuery
        : undefined) ?? displayedQuery;
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        <SegmentedNav
          className="shrink-0 [&>button]:px-2 [&>button]:text-ctl"
          aria-label={t("usageWorkbench.tabs")}
          value={effectiveTab}
          onChange={(value) => {
            if (
              !sharedProjectSupported &&
              value !== "sessions" &&
              analysisProjects.length
            ) {
              setAnalysisProjects([]);
              setProjectsCleared(true);
            }
            setTab(value);
            setSavedHistory(undefined);
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
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <SegmentedControl<WorkbenchPeriod>
            aria-label={t("usagePage.period")}
            value={range.period}
            className="[&>button]:px-2"
            options={(modern
              ? (["today", "7d", "30d", "90d", "all", "custom"] as const)
              : (["today", "7d", "30d", "all"] as const)
            ).map((value) => ({
              value,
              label: t(`usagePage.periods.${value}`),
            }))}
            onChange={(period) =>
              period === "custom"
                ? toggleCustomRange()
                : chooseRange({ period })
            }
            onReselect={(period) => {
              if (period === "custom") toggleCustomRange();
            }}
          />
          {effectiveTab === "overview" && (
            <span
              className="inline-flex items-center gap-1.5"
              title={t("usagePage.estimate")}
            >
              <span className="text-sub font-semibold tabular-nums">
                {usageIsUnavailable(data)
                  ? "—"
                  : formatUsageEstimate(
                      data?.totals.estimate,
                      data?.totals.costUsd,
                    )}
              </span>
            </span>
          )}
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-1.5">
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
            variant="ghost"
            className="h-[26px] w-[26px] p-0 text-ink-muted"
            aria-label={t("usageWorkbench.refresh")}
            aria-busy={refreshing}
            disabled={refreshing}
            title={
              refreshError
                ? t("usagePage.refreshFailed")
                : t("usageWorkbench.refresh")
            }
            onClick={() => void manualRefresh().catch(() => undefined)}
          >
            <RefreshCw
              aria-hidden
              className={`h-3.5 w-3.5 ${refreshing ? "animate-spin motion-reduce:animate-none" : ""}`}
            />
          </Button>
        </div>
      </div>
      {customOpen && (
        <form
          aria-label={t("usagePage.periods.custom")}
          className="flex w-fit max-w-full flex-wrap items-end gap-3 rounded-card border border-line bg-card p-3 shadow-sm"
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
          <div className="w-[156px]">
            <DatePicker
              label={t("usageWorkbench.startDate")}
              value={customStart}
              onChange={(value) => {
                setCustomStart(value);
                setRangeError(false);
              }}
              rangeStart={customStart}
              rangeEnd={customEnd}
              invalid={rangeError}
              describedBy={rangeError ? "usage-custom-date-error" : undefined}
            />
          </div>
          <ArrowRight aria-hidden className="mb-2 h-3.5 w-3.5 text-ink-faint" />
          <div className="w-[156px]">
            <DatePicker
              label={t("usageWorkbench.endDate")}
              value={customEnd}
              onChange={(value) => {
                setCustomEnd(value);
                setRangeError(false);
              }}
              rangeStart={customStart}
              rangeEnd={customEnd}
              invalid={rangeError}
              describedBy={rangeError ? "usage-custom-date-error" : undefined}
            />
          </div>
          <div className="flex gap-1.5">
            <Button type="submit" className="h-8 px-3.5">
              {t("usageWorkbench.applyRange")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              className="h-8"
              onClick={() => setCustomOpen(false)}
            >
              {t("common.cancel")}
            </Button>
          </div>
          {rangeError && (
            <p
              id="usage-custom-date-error"
              role="alert"
              className="w-full text-note text-warn"
            >
              {t("usageWorkbench.invalidRange")}
            </p>
          )}
        </form>
      )}
      <UsageFilterToolbar
        tools={toolIds.map((value) => ({
          value,
          label: TOOL_META[value]?.label ?? value,
        }))}
        selectedTools={filters.toolIds ?? []}
        onToolsChange={(toolIds) => setFilters((old) => ({ ...old, toolIds }))}
        disabled={!modern}
        chips={filterChips}
        onReset={() => {
          setFilters({});
          setAnalysisProjects([]);
        }}
      >
        <UsageFilterMenu
          label={t("usageWorkbench.modelsFilter")}
          allLabel={t("usageFilters.allModels")}
          pageSize={effectiveTab === "sessions" ? 50 : undefined}
          options={modelIds.map((value) => ({ value, label: value }))}
          selected={filters.modelIds ?? []}
          onChange={(modelIds) => setFilters((old) => ({ ...old, modelIds }))}
          disabled={!modern}
        />
        {sharedProjectSupported ||
        (effectiveTab === "sessions" && analysisProjectSupported) ? (
          <UsageFilterMenu
            label={t(
              sharedProjectSupported
                ? "usageWorkbench.projectsFilter"
                : "usageSessions.projectFilter",
            )}
            allLabel={t("usageFilters.allProjects")}
            pageSize={50}
            options={projectOptions}
            selected={selectedProjects}
            onChange={chooseProjects}
          />
        ) : (
          <span
            className="text-note text-ink-faint"
            title={t("usageWorkbench.projectUnavailable")}
          >
            {t("usageWorkbench.projectUnavailable")}
          </span>
        )}
      </UsageFilterToolbar>
      {!sharedProjectSupported &&
        effectiveTab === "sessions" &&
        analysisProjectSupported && (
          <p className="text-note text-ink-muted">
            {t("usageSessions.projectScope")}
          </p>
        )}
      {projectsCleared && effectiveTab !== "sessions" && (
        <p role="status" className="text-note text-ink-muted">
          {t("usageSessions.projectsCleared")}
        </p>
      )}
      {refreshError && (
        <p role="alert" className="text-note text-warn">
          {t("usagePage.refreshFailed")}
        </p>
      )}
      {refreshing && effectiveTab !== "requests" && (
        <p
          role="status"
          className="flex items-center gap-2 text-note text-ink-2"
        >
          <RefreshCw
            aria-hidden
            className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none"
          />
          {t(`usageWorkbench.refreshPhase.${refreshPhase ?? "query"}`)}
        </p>
      )}
      <p className="text-note text-ink-muted">
        {t("usageWorkbench.rangeSummary", {
          start:
            resultQuery.startMs == null
              ? t("usageWorkbench.beginning")
              : new Date(resultQuery.startMs).toLocaleDateString(i18n.language),
          end: new Date(resultQuery.endMs - 1).toLocaleString(i18n.language),
          timezone,
        })}
      </p>
      {capabilities.isFetched && !modern && (
        <p role="status" className="text-note text-ink-muted">
          {t("usageWorkbench.oldBackend")}
        </p>
      )}
      <section
        id="usage-overview-panel"
        role="tabpanel"
        aria-label={t("usageWorkbench.overview")}
        hidden={effectiveTab !== "overview"}
        className="space-y-3"
        aria-busy={state.overview.isFetching || !!refreshPhase}
      >
        {!state.overview.data && data && (
          <p
            role="status"
            className="rounded-lg border border-brand/25 bg-brand/5 px-3 py-2 text-note text-ink-2"
          >
            {t(
              state.overview.isError
                ? "usageFilters.previous"
                : "usageFilters.updating",
            )}
            {data.appliedQuery && (
              <span className="mt-1 block break-words text-ink-faint">
                {describePrevious(data.appliedQuery)}
              </span>
            )}
          </p>
        )}
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
        {state.overview.isPending && !data ? (
          <p role="status" className="p-6 text-center text-ink-muted">
            {t("common.loading")}
          </p>
        ) : !data ||
          (!usageIsUnavailable(data) &&
            ![
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
            <UsageKpis
              totals={data.totals}
              attribution={data.attribution}
              unavailable={usageIsUnavailable(data)}
            />
            {usageIsUnavailable(data) ? (
              <p role="status" className="text-note text-ink-muted">
                {t("usagePage.unavailable")}
              </p>
            ) : (
              <>
                <UsageBars overview={data} />
                <div className="grid gap-3 md:grid-cols-2">
                  <UsageBreakdownList
                    kind="tools"
                    compact
                    initialLimit={6}
                    pageSize={50}
                    rows={data.byTool.map((row) => ({
                      ...row,
                      id: row.appType,
                    }))}
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
                    compact
                    initialLimit={6}
                    pageSize={50}
                    rows={data.byModel.map((row) => ({
                      ...row,
                      id: row.model,
                    }))}
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
                  {data.byProject && (
                    <section
                      className="min-w-0 md:col-span-2"
                      aria-label={t("usageSessions.projectRank")}
                    >
                      <UsageBreakdownList
                        kind="projects"
                        compact
                        initialLimit={6}
                        rows={data.byProject.map((row) => ({
                          ...row,
                          label: usageProjectLabel(
                            row.id,
                            row.label,
                            row.path,
                            projectNames,
                          ),
                        }))}
                        total={data.totals.totalTokens}
                        pageSize={50}
                        onFilter={
                          sharedProjectSupported
                            ? (id) =>
                                chooseProjects(
                                  selectedProjects.length === 1 &&
                                    selectedProjects[0] === id
                                    ? []
                                    : [id],
                                )
                            : undefined
                        }
                      />
                    </section>
                  )}
                </div>
              </>
            )}
          </>
        )}
        <UsageSourceAlert
          sources={data?.sources}
          pricing={data?.pricingCoverage}
          onRetry={retry}
        />
        {data?.coverage?.state === "unsupported" && (
          <p className="text-note text-ink-muted">
            {t("usageWorkbench.queryUnavailable")}
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
            query={displayedQuery}
            recordBasis={historyScope ? "savedHistory" : "effective"}
            onEffectiveRecords={() => setSavedHistory(undefined)}
            onSavedHistory={openSavedHistory}
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
          aria-busy={analysis.isFetching || !!refreshPhase}
        >
          {!analysis.data && analysisDisplay && (
            <p
              role="status"
              className="rounded-lg border border-brand/25 bg-brand/5 px-3 py-2 text-note text-ink-2"
            >
              {t(
                analysis.isError
                  ? "usageFilters.previous"
                  : "usageFilters.updating",
              )}
              <span className="mt-1 block break-words text-ink-faint">
                {describePrevious(analysisDisplay.appliedQuery)}
              </span>
            </p>
          )}
          {analysis.isPending && !analysisDisplay ? (
            <p role="status" className="p-6 text-center text-ink-muted">
              {t("common.loading")}
            </p>
          ) : !analysisDisplay ||
            analysisDisplay.coverage.state === "unsupported" ? (
            <p className="rounded-list border border-line bg-card p-8 text-center text-sub text-ink-muted">
              {t("usagePage.unavailable")}
            </p>
          ) : null}
          {analysisDisplay && (
            <UsageSessions
              key={apex}
              data={analysisDisplay}
              showData={analysisDisplay.coverage.state !== "unsupported"}
              active={visible && effectiveTab === "sessions"}
              onRetry={retry}
              onProject={
                sharedProjectSupported || analysisProjectSupported
                  ? (id) => {
                      chooseProjects(
                        selectedProjects.length === 1 &&
                          selectedProjects[0] === id
                          ? []
                          : [id],
                      );
                    }
                  : undefined
              }
              onModel={(id) =>
                setFilters((old) => ({
                  ...old,
                  modelIds: old.modelIds?.includes(id) ? [] : [id],
                }))
              }
              onRemoveProject={
                sharedProjectSupported || analysisProjectSupported
                  ? (id) =>
                      chooseProjects(
                        selectedProjects.filter((value) => value !== id),
                      )
                  : undefined
              }
              onRemoveModel={(id) =>
                setFilters((old) => ({
                  ...old,
                  modelIds: old.modelIds?.filter((value) => value !== id),
                }))
              }
              onRemoveTool={(id) =>
                setFilters((old) => ({
                  ...old,
                  toolIds: old.toolIds?.filter((value) => value !== id),
                }))
              }
              onManage={shell.locateSession}
              onTool={(id) =>
                setFilters((old) => ({
                  ...old,
                  toolIds: old.toolIds?.includes(id) ? [] : [id],
                }))
              }
              location={analysis.data ? sessionTarget : null}
              onLocated={clearSessionTarget}
              onExpandScope={() => {
                setFilters({});
                setAnalysisProjects([]);
                chooseRange({ period: "all" });
              }}
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
