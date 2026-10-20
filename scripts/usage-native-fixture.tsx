/** Synthetic UI-only responses. No live sources, credentials, IPC or paid calls. */
import { Profiler, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import { UsagePage } from "@/components/usage-overview/UsagePage";
import { Button } from "@/components/ui/button";
import { MenuSelect } from "@/components/ui/menu-select";
import { TooltipProvider } from "@/components/ui/tooltip";
import { queryClient } from "@/lib/query/queryClient";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import type { UsageQueryV1, UsageRebuildProgress } from "@/types/usageQuery";
import type { RequestLog, PaginatedLogs } from "@/types/usage";
import { sessionAnalyticsFixture } from "../tests/components/usage/sessionAnalyticsFixture";
import "@/index.css";

type Mode = "complete" | "partial" | "unavailable" | "zero" | "legacy";
const params = new URLSearchParams(location.search);
let mode: Mode = [
  "complete",
  "partial",
  "unavailable",
  "zero",
  "legacy",
].includes(params.get("state") ?? "")
  ? (params.get("state") as Mode)
  : "partial";
let apex = params.get("apex") === "ofox.io" ? "ofox.io" : "ofox.ai";
let operation = 1;
let renderRequest: { serial: number; command: string; startMs: number } | null =
  null;
let measuredRender = 0;
const renderSamples: {
  command: string;
  requestToFrameMs: number;
}[] = [];
function recordFixtureRender() {
  const request = renderRequest;
  // Only a fresh-navigation first load is measured. A later query may retain
  // old model DOM, which is not evidence for that new response's commit.
  if (!request || measuredRender !== 0) return;
  requestAnimationFrame(() => {
    if (
      measuredRender !== 0 ||
      !document
        .getElementById("fixture-usage-surface")
        ?.textContent?.includes("fictional-model")
    )
      return;
    measuredRender = request.serial;
    renderSamples.push({
      command: request.command,
      requestToFrameMs: performance.now() - request.startMs,
    });
    const output = document.getElementById("fixture-render-metrics");
    if (output)
      output.textContent = JSON.stringify({
        scope:
          "fresh-navigation first model-marker frame only; synthetic mock IPC; excludes native scan, backend latency and subsequent refreshes",
        samples: renderSamples,
      });
  });
}
let rebuild: UsageRebuildProgress | null =
  params.get("rebuild") === "running"
    ? {
        operationId: "fixture-rebuild-1",
        state: "running",
        completedSources: 2,
        totalSources: 7,
        errorCode: null,
      }
    : null;
const initialLanguage = ["zh", "en", "ja"].includes(params.get("lang") ?? "")
  ? params.get("lang")!
  : "zh";
await i18n.use(initReactI18next).init({
  lng: initialLanguage,
  resources: {
    zh: { translation: zh },
    en: { translation: en },
    ja: { translation: ja },
  },
  interpolation: { escapeValue: false },
});
document.documentElement.classList.add("mac");
document.documentElement.classList.toggle(
  "dark",
  params.get("theme") === "dark",
);
localStorage.setItem("ofox.usagePeriod", "7d");
localStorage.setItem("ofox.usageRefreshSeconds", "0");
localStorage.setItem(
  "ofox.usageTab",
  ["overview", "requests", "sessions"].includes(params.get("tab") ?? "")
    ? params.get("tab")!
    : "sessions",
);

function analysis(query: UsageQueryV1) {
  const data = sessionAnalyticsFixture();
  const unknown = mode === "unavailable";
  const zero = unknown || mode === "zero";
  const partial = mode === "partial";
  const lastDay = new Date(query.endMs - 1);
  lastDay.setHours(0, 0, 0, 0);
  const dayMs = lastDay.getTime() - 6 * 86400000;
  const tokens = (scale: number) => ({
    inputTokens: zero ? 0 : 2000 * scale,
    outputTokens: zero ? 0 : 500 * scale,
    cacheReadTokens: zero ? 0 : 9000 * scale,
    cacheWriteTokens: 0,
    totalTokens: zero ? 0 : 2500 * scale,
    cacheHitRate: zero ? 0 : 9 / 11,
  });
  const estimate = (main: number) => ({
    knownCostUsd: (main / 200000).toFixed(6),
    availability: partial
      ? ("partial" as const)
      : main
        ? ("known" as const)
        : ("free" as const),
    unpricedModelIds: partial ? ["fictional-unpriced-model"] : [],
    unpricedRecordCount: partial ? null : 0,
    additionalUnitsUnsupported: false,
  });
  const coverage = {
    state: unknown
      ? ("unavailable" as const)
      : partial
        ? ("partial" as const)
        : ("complete" as const),
    eligibleSessions: unknown ? 0 : 3,
    excludedSessions: partial || unknown ? 1 : 0,
    unquantifiedSourceGaps: partial || unknown,
    reasons: partial || unknown ? ["sourceReadIncomplete"] : [],
  };
  Object.assign(data, {
    appliedQuery: query,
    queryFingerprint: `fixture-query:${JSON.stringify(query)}`,
    snapshotVersion: `fixture-generation:${mode}:${apex}`,
    analyticsSnapshotId: `fixture-analysis:${mode}:${apex}`,
    methodVersion: mode === "legacy" ? undefined : "magpie-native-v1",
    inputSnapshotVersion: `fictional-input:${mode}`,
    sourceCoverage:
      mode === "legacy"
        ? undefined
        : {
            observed: 7,
            read: unknown ? 0 : partial ? 5 : 7,
            partial: partial ? 1 : 0,
            failed: unknown ? 7 : partial ? 1 : 0,
            undated: partial ? 1 : 0,
            legacyHistoryAvailable: true,
            usageAvailability: unknown
              ? "unavailable"
              : partial
                ? "partial"
                : "available",
            knownUsageRecords: unknown ? 0 : 7,
          },
    messageCoverage: coverage,
    activityCoverage: coverage,
    metricCoverage: coverage,
    knownMessages: unknown ? null : 84,
    knownActiveMs: unknown ? null : 3360000,
    knownActiveDays: unknown ? null : 7,
  });
  data.coverage = {
    ...data.coverage,
    state: partial || unknown ? "partial" : "complete",
    reasons: coverage.reasons,
    detailStartMs: dayMs,
    detailEndMs: query.endMs,
  };
  data.pricingCoverage = {
    ...data.pricingCoverage,
    apex,
    catalogVersion: "fictional-test-prices-v1",
    catalogUpdatedAtMs: dayMs,
    catalogState: "ready",
    unpricedModelIds: partial ? ["fictional-unpriced-model"] : [],
    unpricedRecordCount: partial ? null : 0,
  };
  data.totals = {
    ...data.totals,
    ...tokens(28),
    sessionCount: unknown ? 0 : 3,
    projectCount: unknown ? 0 : 1,
    medianTokens: zero ? "0" : "20000",
    p90Tokens: zero ? 0 : 40000,
    quantileSamples: unknown ? 0 : 3,
    quantileExcluded: partial || unknown ? 1 : 0,
    estimate: estimate(tokens(28).totalTokens),
    messages: partial || unknown ? null : 84,
    activeMs: partial || unknown ? null : 3360000,
    activeDays: partial || unknown ? null : 7,
    messageReason: partial || unknown ? "sourceReadIncomplete" : null,
    activityReason: partial || unknown ? "sourceReadIncomplete" : null,
  };
  data.series = Array.from({ length: 7 }, (_, index) => ({
    ...tokens(index + 1),
    date: new Date(dayMs + index * 86400000).toLocaleDateString("en-CA"),
    startMs: dayMs + index * 86400000,
    endMs: dayMs + (index + 1) * 86400000,
    sessionCount: unknown ? 0 : 1,
    estimate: estimate(tokens(index + 1).totalTokens),
    messages: unknown || partial ? null : (index + 1) * 3,
    activeMs: unknown || partial ? null : (index + 1) * 120000,
    knownMessages: unknown ? null : (index + 1) * 3,
    knownActiveMs: unknown ? null : (index + 1) * 120000,
  }));
  data.byProject = unknown
    ? []
    : [
        {
          ...tokens(28),
          id: "fictional-project",
          label:
            "示例项目 — A very long fictional project name — サンプルプロジェクト",
          path: "/fixture/projects/example",
          sessionCount: 3,
          estimate: data.totals.estimate,
        },
      ];
  data.byModel = unknown
    ? []
    : [
        {
          ...tokens(28),
          id: "fictional-model",
          label: "fictional-model",
          path: null,
          sessionCount: 3,
          estimate: data.totals.estimate,
        },
      ];
  data.byTool = unknown
    ? []
    : [
        {
          ...tokens(28),
          id: "codex",
          label: "Codex",
          path: null,
          sessionCount: 3,
          estimate: data.totals.estimate,
        },
      ];
  data.topSessions = unknown
    ? []
    : [4, 8, 16].map((scale, index) => ({
        ...data.topSessions[0],
        sessionKey: `fictional-thread-${index}`,
        nativeSessionId: `fictional-thread-${index}`,
        managerKey: null,
        title: index
          ? `Fictional session ${index + 1}`
          : "修复示例：非常长的会话标题 · A deliberately long fictional title · 検証用セッション",
        projectId: "fictional-project",
        projectLabel: data.byProject[0].label,
        projectPath: "/fixture/projects/example",
        tokens: tokens(scale),
        estimate: estimate(tokens(scale).totalTokens),
        models: [
          {
            ...tokens(scale),
            model: "fictional-model",
            estimate: estimate(tokens(scale).totalTokens),
          },
        ],
        messages: scale * 3,
        activeMs: scale * 120000,
        reasons: partial ? ["activityTimeUnavailable"] : [],
      }));
  data.topLimit = 6;
  data.compactTop = {
    tokens: [...data.topSessions].reverse(),
    cost: [...data.topSessions].reverse(),
    active: [...data.topSessions].reverse(),
  };
  data.options = {
    toolIds: ["codex"],
    modelIds: ["fictional-model"],
    sourceIds: ["codex_session"],
    projects: [
      {
        id: "fictional-project",
        label: "Fictional project",
        path: "/fixture/projects/example",
      },
    ],
  };
  data.unassociated = { ...tokens(0), records: 0 };
  data.sources = [
    {
      sourceId: "codex_session",
      toolId: "codex",
      status: unknown ? "failed" : partial ? "partial" : "ok",
      lastAttemptAtMs: query.endMs - 1000,
      lastSuccessAtMs: unknown ? null : query.endMs - 1000,
      errorCode: unknown ? "fixture-source-unreadable" : null,
      lastScan: { filesScanned: 7, imported: unknown ? 0 : 7, skipped: 0 },
      retainedDetailRange: unknown
        ? null
        : { startMs: dayMs, endMs: query.endMs },
    },
  ];
  data.activityByHour = {
    timezone: "local",
    coverage,
    cells: Array.from({ length: 168 }, (_, index) => ({
      weekday: Math.floor(index / 24),
      hour: index % 24,
      activeMs: index === 36 ? 3360000 : 0,
    })),
  };
  data.distributions = ["tokens", "messages", "activeMs"].map((metric) => ({
    metric: metric as "tokens" | "messages" | "activeMs",
    edges: [0, 1, 100, 10000],
    counts: [zero && !unknown ? 3 : 0, 0, 0, zero ? 0 : 3],
    samples: unknown ? 0 : 3,
    excludedSessions: coverage.excludedSessions,
    unquantifiedSourceGaps: coverage.unquantifiedSourceGaps,
  }));
  return data;
}

const snapshots = new Map<string, PaginatedLogs>();
function records(
  data: ReturnType<typeof analysis>,
  history: boolean,
): RequestLog[] {
  if (mode === "unavailable" && !history) return [];
  return data.series.map((day, index) => ({
    requestId: history
      ? `fictional-legacy-${index}`
      : `codex_native_v1:fictional-${index}`,
    providerId: "_codex_session",
    appType: "codex",
    model: "fictional-model",
    costMultiplier: "1",
    inputTokens: day.inputTokens,
    outputTokens: day.outputTokens,
    cacheReadTokens: day.cacheReadTokens,
    cacheCreationTokens: day.cacheWriteTokens,
    inputCostUsd: "0",
    outputCostUsd: "0",
    cacheReadCostUsd: "0",
    cacheCreationCostUsd: "0",
    totalCostUsd: history ? "0.012340" : "0",
    recordedCostUsd: history ? "0.012340" : null,
    isStreaming: true,
    latencyMs: 0,
    statusCode: 0,
    createdAt: Math.floor(day.startMs / 1000),
    createdAtMs: day.startMs + 123,
    dataSource: "codex_session",
    observedStatusCode: null,
    observedLatencyMs: null,
    observedFirstTokenMs: null,
    currentEstimate: day.estimate,
    receipt: {
      queryFingerprint: data.queryFingerprint,
      snapshotVersion: data.snapshotVersion,
      appliedQuery: data.appliedQuery,
    },
  }));
}
const internals = window as unknown as Record<string, unknown>;
internals.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener: () => undefined,
};
internals.__TAURI_INTERNALS__ = {
  metadata: {
    currentWindow: { label: "fixture" },
    currentWebview: { label: "fixture" },
  },
  transformCallback: () => 1,
  invoke: async (command: string, args: Record<string, any> = {}) => {
    if (
      [
        "get_session_usage_analytics",
        "get_usage_overview",
        "get_request_logs",
      ].includes(command)
    )
      renderRequest = {
        serial: (renderRequest?.serial ?? 0) + 1,
        command,
        startMs: performance.now(),
      };
    if (["plugin:event|listen", "plugin:event|unlisten"].includes(command))
      return 1;
    if (command === "plugin:window|is_visible") return true;
    if (command === "plugin:window|is_minimized") return false;
    if (command === "ofox_get_apex_state")
      return { apex, pinned: true, detected: true };
    if (command === "get_usage_capabilities")
      return {
        queryVersion: 1,
        requestSnapshots: true,
        csv: false,
        projectFilter: true,
        sessionAnalytics: true,
        sessionProjectFilter: true,
        localRefresh: true,
        nativeRebuild: mode !== "legacy",
        sources: [],
      };
    if (command === "get_usage_rebuild_progress")
      return structuredClone(rebuild);
    if (command === "cancel_usage_rebuild") {
      if (!rebuild || rebuild.operationId !== args.operationId)
        throw new Error("fixture-stale-operation");
      rebuild = { ...rebuild, state: "cancelled" };
      return structuredClone(rebuild);
    }
    if (["sync_session_usage", "refresh_local_usage"].includes(command))
      return { imported: 7, skipped: 0, filesScanned: 7, errors: [] };
    if (command === "get_session_usage_analytics") return analysis(args.query);
    if (command === "get_usage_overview") {
      const data = analysis(args.query);
      const total = {
        ...data.totals,
        requests: data.sourceCoverage?.knownUsageRecords ?? 7,
        cacheHitRate: data.totals.cacheHitRate ?? 0,
        costUsd: Number(data.totals.estimate.knownCostUsd),
      };
      return {
        ...data,
        period: args.period,
        start: args.query.startMs,
        end: args.query.endMs,
        granularity: "day",
        totals: total,
        series: data.series.map((day) => ({
          ...day,
          start: day.startMs,
          requests: mode === "unavailable" ? 0 : 1,
          costUsd: Number(day.estimate.knownCostUsd),
        })),
        byTool: data.byTool.map((row) => ({ ...row, appType: row.id })),
        byModel: data.byModel.map((row) => ({
          ...row,
          model: row.id,
          appTypes: ["codex"],
          priced: true,
        })),
        attribution: {
          sessionCount: data.totals.sessionCount,
          projectCount: data.totals.projectCount,
          unassociatedRecords: 0,
          partial: mode === "partial",
        },
        unsupportedTools: [],
        lastSyncedAt: args.query.endMs - 1000,
      };
    }
    if (command === "get_request_logs") {
      if (args.snapshotId && snapshots.has(args.snapshotId))
        return structuredClone(snapshots.get(args.snapshotId));
      const data = analysis(args.query);
      const history = args.requestFilters?.recordBasis === "savedHistory";
      const rows = records(data, history);
      const result: PaginatedLogs = {
        ...data,
        data: rows,
        total: rows.length,
        page: 0,
        pageSize: args.pageSize,
        snapshotId: `fixture-requests:${mode}:${apex}:${history}`,
        requestFilters: args.requestFilters ?? {},
        summary: {
          ...data.totals,
          requests: rows.length,
          cacheHitRate: data.totals.cacheHitRate ?? 0,
          costUsd: Number(data.totals.estimate.knownCostUsd),
        },
        trend: {
          granularity: "day",
          series: data.series.map((day) => ({
            ...day,
            start: day.startMs,
            requests: mode === "unavailable" ? 0 : 1,
            costUsd: Number(day.estimate.knownCostUsd),
          })),
        },
        measurements: {
          statusSamples: 0,
          failedSamples: 0,
          failureRate: null,
          latencySamples: 0,
          avgLatencyMs: null,
          firstTokenSamples: 0,
          avgFirstTokenMs: null,
          generationSpeedSamples: 0,
          avgGenerationTokensPerSecond: null,
        },
      };
      if (history && result.sourceCoverage)
        result.sourceCoverage = {
          ...result.sourceCoverage,
          usageAvailability: "available",
          knownUsageRecords: rows.length,
        };
      snapshots.set(result.snapshotId!, structuredClone(result));
      return result;
    }
    if (command === "get_request_detail")
      return structuredClone(
        snapshots
          .get(args.snapshotId)
          ?.data.find((row) => row.requestId === args.requestId) ?? null,
      );
    if (command === "get_session_analytics_page") {
      const query = JSON.parse(
        args.expectedReceipt.queryFingerprint.replace(/^fixture-query:/, ""),
      );
      const data = analysis(query);
      return {
        ...data,
        ...args.expectedReceipt,
        analyticsSnapshotId: args.analyticsSnapshotId,
        data: data.topSessions,
        total: data.topSessions.length,
        page: args.page,
        pageSize: args.pageSize,
        appliedView: {
          sortBy: args.sortBy,
          search: args.search?.trim().toLowerCase() ?? null,
          managerKey: args.managerKey ?? null,
        },
        viewFingerprint: "fictional-view",
      };
    }
    throw new Error(`fixture blocked: ${command}`);
  },
};
window.fetch = async () => {
  throw new Error("fixture blocks external fetch");
};
window.open = () => null;
const account: OfoxAccount = {
  user: null,
  retrying: false,
  retryAttempt: 0,
  loadFailed: false,
  balanceRefreshing: false,
  loadUser: async () => null,
  retryUser: async () => undefined,
  refreshBalance: async () => undefined,
};
function Page() {
  const shell = useShell();
  useEffect(() => shell.setActive("usage"), [shell.setActive]);
  return <UsagePage account={account} />;
}
function Fixture() {
  const [generation, setGeneration] = useState(0);
  const [language, setLanguage] = useState(initialLanguage);
  const [chosen, setChosen] = useState(mode);
  const change = () => {
    queryClient.clear();
    setGeneration((value) => value + 1);
  };
  return (
    <TooltipProvider>
      <QueryClientProvider client={queryClient}>
        <main className="mx-auto space-y-3 p-4" style={{ maxWidth: 1200 }}>
          <aside
            className="flex flex-wrap items-center gap-2 rounded-card border border-line bg-card p-3 text-note text-ink-muted"
            aria-label="Synthetic fixture controls"
          >
            <b>UI sandbox · synthetic records</b>
            {["zh", "en", "ja"].map((value) => (
              <Button
                key={value}
                variant={value === language ? "primary" : "secondary"}
                onClick={() => {
                  setLanguage(value);
                  void i18n.changeLanguage(value);
                }}
              >
                {value}
              </Button>
            ))}
            <Button
              variant="secondary"
              onClick={() => document.documentElement.classList.toggle("dark")}
            >
              Light / Dark
            </Button>
            <MenuSelect
              aria-label="Synthetic coverage state"
              value={chosen}
              onChange={(value) => {
                mode = value;
                setChosen(value);
                change();
              }}
              options={(
                [
                  "complete",
                  "partial",
                  "unavailable",
                  "zero",
                  "legacy",
                ] as const
              ).map((value) => ({ value, label: value }))}
            />
            <Button
              variant="secondary"
              onClick={() => {
                apex = apex === "ofox.ai" ? "ofox.io" : "ofox.ai";
                change();
              }}
            >
              {apex}
            </Button>
            <Button
              variant="secondary"
              onClick={() => {
                rebuild = {
                  operationId: `fixture-rebuild-${++operation}`,
                  state: "running",
                  completedSources: 2,
                  totalSources: 7,
                  errorCode: null,
                };
                void queryClient.invalidateQueries({
                  queryKey: ["usageNativeRebuildProgress"],
                });
              }}
            >
              Start rebuild
            </Button>
            <Button
              variant="secondary"
              onClick={() => {
                if (rebuild)
                  rebuild = {
                    ...rebuild,
                    state: "completed",
                    completedSources: 7,
                  };
                void queryClient.invalidateQueries({
                  queryKey: ["usageNativeRebuildProgress"],
                });
              }}
            >
              Finish rebuild
            </Button>
            <span>
              No real files, IPC, HTTP or paid calls. Raw audit opens from the
              expanded data status.
            </span>
          </aside>
          <output id="fixture-render-metrics" hidden />
          <div className="rounded-list border border-line bg-canvas p-3">
            <header className="mb-4 flex items-center justify-between border-b border-line pb-3">
              <b>Ofox Desktop</b>
              <span>{i18n.t("shell.tabs.usage")}</span>
            </header>
            <div id="fixture-usage-surface">
              <Profiler id="usage-fixture" onRender={recordFixtureRender}>
                <ShellProvider key={generation} openSettings={() => undefined}>
                  <Page />
                </ShellProvider>
              </Profiler>
            </div>
          </div>
        </main>
      </QueryClientProvider>
    </TooltipProvider>
  );
}
createRoot(document.getElementById("root")!).render(<Fixture />);
