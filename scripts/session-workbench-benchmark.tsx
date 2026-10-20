/**
 * Production renderer-only benchmark. The fixture is prepared before timing;
 * no native scans, source parsing, IPC, HTTP, disk, or fixture construction is timed.
 */
import { useLayoutEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import { SessionsPage } from "@/components/sessions/SessionsPage";
import { UsageSessions } from "@/components/usage-overview/UsageSessions";
import type { ManagedSession } from "@/lib/api/sessions";
import type {
  AnalyticsRank,
  AnalyticsSession,
  AnalyticsTokens,
  SessionAnalytics,
} from "@/types/sessionAnalytics";
import type { UsageEstimate } from "@/types/usageQuery";
import { sessionAnalyticsFixture } from "../tests/components/usage/sessionAnalyticsFixture";
import { blocked } from "./session-workbench-benchmark-native";
import "@/index.css";

const COUNT = 10_000,
  DAYS = 1460,
  PROJECTS = 100,
  MODELS = 16,
  WARMUP = 3,
  ROUNDS = 30,
  TARGET = 500;
type Page = "sessions" | "usage";
type Fixture = { sessions: ManagedSession[]; analytics: SessionAnalytics };
type Sample = { round: number; commitMs: number; paintOpportunityMs: number };
const tools = ["claude", "codex", "gemini", "opencode"];
const tokens = (
  input = 0,
  output = 0,
  cache = 0,
  write = 0,
): AnalyticsTokens => ({
  inputTokens: input,
  outputTokens: output,
  cacheReadTokens: cache,
  cacheWriteTokens: write,
  totalTokens: input + output,
  cacheHitRate: input + cache + write ? cache / (input + cache + write) : 0,
});
const estimate = (count: number): UsageEstimate => ({
  knownCostUsd: (count / 1_000_000).toFixed(8),
  availability: count ? "known" : "free",
  unpricedModelIds: [],
  unpricedRecordCount: 0,
  additionalUnitsUnsupported: false,
});
const sum = (target: AnalyticsTokens, value: AnalyticsTokens) => {
  for (const key of [
    "inputTokens",
    "outputTokens",
    "cacheReadTokens",
    "cacheWriteTokens",
    "totalTokens",
  ] as const)
    target[key] += value[key];
  target.cacheHitRate =
    target.cacheReadTokens /
    Math.max(
      1,
      target.inputTokens + target.cacheReadTokens + target.cacheWriteTokens,
    );
};

function fixture(phase: number): Fixture {
  const base = sessionAnalyticsFixture();
  const series = Array.from({ length: DAYS }, (_, day) => {
    const start = new Date(2022, 9, 17 + day),
      end = new Date(2022, 9, 18 + day);
    const date = `${start.getFullYear()}-${String(start.getMonth() + 1).padStart(2, "0")}-${String(start.getDate()).padStart(2, "0")}`;
    return {
      ...tokens(),
      date,
      startMs: start.getTime(),
      endMs: end.getTime(),
      sessionCount: 0,
      messages: 0,
      activeMs: 0,
      estimate: estimate(0),
    };
  });
  const sessions: ManagedSession[] = [],
    rows: AnalyticsSession[] = [];
  const total = tokens(),
    projects = new Map<string, AnalyticsRank>(),
    models = new Map<string, AnalyticsRank>(),
    byTool = new Map<string, AnalyticsRank>();
  let messages = 0,
    activeMs = 0;
  const add = (
    map: Map<string, AnalyticsRank>,
    id: string,
    label: string,
    path: string | null,
    usage: AnalyticsTokens,
  ) => {
    const row = map.get(id) ?? {
      ...tokens(),
      id,
      label,
      path,
      sessionCount: 0,
      estimate: estimate(0),
    };
    sum(row, usage);
    row.sessionCount++;
    row.estimate = estimate(row.totalTokens);
    map.set(id, row);
  };
  for (let index = 0; index < COUNT; index++) {
    const key = index.toString(16).padStart(64, "0"),
      providerId = tools[index % tools.length],
      projectDir = `/benchmark/project-${String(index % PROJECTS).padStart(3, "0")}`,
      model = `benchmark-model-${index % MODELS}`;
    const usage = tokens(
        1000 + (index % 400) + phase * 7,
        500 + (index % 200),
        5000,
        100,
      ),
      day = series[index % DAYS],
      count = 12 + (index % 23),
      duration = 60000 + (index % 15) * 10000;
    const title = `Session ${String(index).padStart(5, "0")} · local benchmark ${phase}`,
      stamp = day.startMs + 3_600_000 + index;
    sessions.push({
      sessionKey: key,
      sessionId: `benchmark-${index}`,
      providerId,
      projectDir,
      sourcePath: `${projectDir}/session-${index}.jsonl`,
      title,
      revision: `fixture-${phase}`,
      createdAt: stamp,
      lastActiveAt: stamp + duration,
      messageCount: count,
      sizeBytes: 8192,
      models: [model],
      capabilities: {
        canReadMessages: true,
        canExport: true,
        canResume: false,
        canTrash: true,
        resumeUsesSessionModel: false,
      },
      usage: {
        ...usage,
        models: [
          {
            ...usage,
            model,
            requests: 1,
            costUsd: usage.totalTokens / 1_000_000,
          },
        ],
        costUsd: usage.totalTokens / 1_000_000,
        unpricedModels: 0,
      },
    });
    rows.push({
      sessionKey: key,
      nativeSessionId: `benchmark-${index}`,
      managerKey: key,
      sourceAvailable: true,
      toolId: providerId,
      title,
      projectId: projectDir,
      projectLabel: projectDir.split("/").at(-1)!,
      projectPath: projectDir,
      tokens: usage,
      estimate: estimate(usage.totalTokens),
      models: [{ ...usage, model, estimate: estimate(usage.totalTokens) }],
      messages: count,
      activeMs: duration,
      reasons: [],
    });
    sum(total, usage);
    sum(day, usage);
    day.sessionCount++;
    day.messages += count;
    day.activeMs += duration;
    messages += count;
    activeMs += duration;
    add(projects, projectDir, projectDir.split("/").at(-1)!, projectDir, usage);
    add(models, model, model, null, usage);
    add(byTool, providerId, providerId, null, usage);
  }
  series.forEach((day) => {
    day.estimate = estimate(day.totalTokens);
  });
  const ordered = (map: Map<string, AnalyticsRank>) =>
    [...map.values()].sort((a, b) => b.totalTokens - a.totalTokens);
  const tokenRows = [...rows].sort(
      (a, b) => b.tokens!.totalTokens - a.tokens!.totalTokens,
    ),
    coverage = {
      state: "complete" as const,
      eligibleSessions: COUNT,
      excludedSessions: 0,
      unquantifiedSourceGaps: false,
      reasons: [],
    };
  const distributions = [
    {
      metric: "tokens" as const,
      edges: [0, 1, 1000, 2000, 4000],
      values: rows.map((row) => row.tokens!.totalTokens),
    },
    {
      metric: "messages" as const,
      edges: [0, 1, 6, 16, 31, 61, 121],
      values: rows.map((row) => row.messages!),
    },
    {
      metric: "activeMs" as const,
      edges: [0, 1, 60000, 360000, 960000, 1860000, 3660000],
      values: rows.map((row) => row.activeMs!),
    },
  ].map(({ metric, edges, values }) => ({
    metric,
    edges,
    counts: edges.map(
      (edge, i) =>
        values.filter(
          (value) =>
            value >= edge && (edges[i + 1] == null || value < edges[i + 1]),
        ).length,
    ),
    samples: COUNT,
    excludedSessions: 0,
    unquantifiedSourceGaps: false,
  }));
  const sortedTokens = tokenRows
    .map((row) => row.tokens!.totalTokens)
    .reverse();
  return {
    sessions,
    analytics: {
      ...base,
      appliedQuery: {
        version: 1,
        startMs: series[0].startMs,
        endMs: series.at(-1)!.endMs,
        toolIds: [],
        modelIds: [],
        projectIds: [],
        sourceIds: [],
      },
      queryFingerprint: "renderer-benchmark-all",
      snapshotVersion: `fixture-${phase}`,
      coverage: {
        ...base.coverage,
        state: "complete",
        reasons: [],
        detailStartMs: series[0].startMs,
        detailEndMs: series.at(-1)!.endMs,
      },
      pricingCoverage: {
        ...base.pricingCoverage,
        catalogState: "ready",
        catalogVersion: "synthetic",
        unpricedModelIds: [],
        unpricedRecordCount: 0,
      },
      totals: {
        ...total,
        sessionCount: COUNT,
        projectCount: PROJECTS,
        medianTokens: String(sortedTokens[COUNT / 2]),
        p90Tokens: sortedTokens[Math.ceil(COUNT * 0.9) - 1],
        quantileSamples: COUNT,
        quantileExcluded: 0,
        estimate: estimate(total.totalTokens),
        messages,
        activeMs,
        activeDays: DAYS,
        messageReason: null,
        activityReason: null,
      },
      series,
      byProject: ordered(projects),
      byModel: ordered(models),
      byTool: ordered(byTool),
      topSessions: tokenRows.slice(0, 100),
      topLimit: 100,
      compactTop: {
        tokens: tokenRows.slice(0, 6),
        cost: tokenRows.slice(0, 6),
        active: [...rows].sort((a, b) => b.activeMs! - a.activeMs!).slice(0, 6),
      },
      knownMetrics: { messages, activeMs, activeDays: DAYS },
      metricCoverage: coverage,
      activityByHour: {
        timezone: "local",
        coverage,
        cells: Array.from({ length: 168 }, (_, index) => ({
          weekday: Math.floor(index / 24),
          hour: index % 24,
          activeMs:
            Math.floor(activeMs / 168) + (index < activeMs % 168 ? 1 : 0),
        })),
      },
      distributions,
      options: {
        toolIds: tools,
        modelIds: [...models.keys()],
        sourceIds: [],
        projects: [...projects.values()].map(({ id, label, path }) => ({
          id,
          label,
          path,
        })),
      },
      unassociated: { ...tokens(), records: 0 },
      sources: [],
    },
  };
}

// All browser state is private to this page; even a reused preview origin cannot
// read or mutate the user's saved filters or hidden-row preferences.
const memory = new Map<string, string>();
Object.defineProperty(window, "localStorage", {
  configurable: true,
  value: {
    getItem: (key: string) => memory.get(key) ?? null,
    setItem: (key: string, value: string) =>
      memory.set(String(key), String(value)),
    removeItem: (key: string) => memory.delete(key),
    clear: () => memory.clear(),
    key: (index: number) => [...memory.keys()][index] ?? null,
    get length() {
      return memory.size;
    },
  },
});
window.fetch = async () => {
  blocked.network++;
  throw new Error("Benchmark disables network fetch");
};
await i18n.use(initReactI18next).init({
  lng: "zh",
  fallbackLng: "en",
  resources: {
    zh: { translation: zh },
    en: { translation: en },
    ja: { translation: ja },
  },
  interpolation: { escapeValue: false },
});
const fixtures = [fixture(0), fixture(1)];
const client = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: Infinity,
      retry: false,
      refetchOnMount: false,
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
    },
  },
});
client.setQueryData(["sessions", "ofox.ai"], fixtures[0].sessions);
client.setQueryData(["session-trash"], []);
const frames = () =>
  new Promise<void>((resolve) =>
    requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
  );
const p95 = (values: number[]) =>
  [...values].sort((a, b) => a - b)[Math.ceil(values.length * 0.95) - 1];

function Surface({ page, data }: { page: Page; data: Fixture }) {
  const shell = useShell();
  useLayoutEffect(() => shell.setActive(page), [page, shell.setActive]);
  return page === "usage" ? (
    <UsageSessions data={data.analytics} active />
  ) : (
    <SessionsPage className="h-full" />
  );
}
function Harness() {
  const [frame, setFrame] = useState({
    page: "usage" as Page,
    revision: 0,
    run: 0,
    data: fixtures[0],
  });
  const [running, setRunning] = useState(false),
    [language, setLanguage] = useState("zh"),
    [report, setReport] = useState<unknown>(null);
  const commit = useRef({ revision: 0, time: 0 }),
    revision = useRef(0),
    guard = useRef(false),
    status = useRef<HTMLParagraphElement>(null);
  useLayoutEffect(() => {
    commit.current = { revision: frame.revision, time: performance.now() };
  }, [frame]);
  const update = async (page: Page, round: number): Promise<Sample> => {
    if (document.visibilityState !== "visible")
      throw new Error("Keep this benchmark tab visible");
    const next = ++revision.current,
      data = fixtures[next % 2],
      start = performance.now();
    flushSync(() => {
      client.setQueryData(["sessions", "ofox.ai"], data.sessions);
      setFrame((old) => ({ ...old, page, revision: next, data }));
    });
    if (commit.current.revision !== next)
      throw new Error("The expected React commit did not complete");
    const commitMs = commit.current.time - start;
    await frames();
    return { round, commitMs, paintOpportunityMs: performance.now() - start };
  };
  const run = async () => {
    if (guard.current) return;
    guard.current = true;
    setRunning(true);
    setReport(null);
    blocked.ipc = 0;
    blocked.network = 0;
    memory.clear();
    const results: {
      page: Page;
      samples: Sample[];
      commitP95Ms: number;
      paintOpportunityP95Ms: number;
      pass: boolean;
    }[] = [];
    try {
      if (!import.meta.env.PROD)
        throw new Error("Use the production build, not the Vite dev server");
      await document.fonts.ready;
      for (const page of ["usage", "sessions"] as const) {
        flushSync(() =>
          setFrame((old) => ({ ...old, page, run: old.run + 1 })),
        );
        await frames();
        const samples: Sample[] = [];
        for (let round = -WARMUP; round < ROUNDS; round++) {
          if (status.current)
            status.current.textContent = `${page} · ${round < 0 ? `warmup ${round + WARMUP + 1}/${WARMUP}` : `sample ${round + 1}/${ROUNDS}`}`;
          const sample = await update(page, round + 1);
          if (round >= 0) samples.push(sample);
        }
        const commitP95Ms = p95(samples.map((sample) => sample.commitMs)),
          paintOpportunityP95Ms = p95(
            samples.map((sample) => sample.paintOpportunityMs),
          );
        results.push({
          page,
          samples,
          commitP95Ms,
          paintOpportunityP95Ms,
          pass: paintOpportunityP95Ms < TARGET,
        });
      }
      const result = {
        rendererOnly: true,
        production: import.meta.env.PROD,
        measured:
          "cached data replacement → React commit; cached data replacement → second requestAnimationFrame (paint opportunity, not compositor proof)",
        excludes: [
          "fixture construction",
          "native source scan",
          "source parsing",
          "IPC",
          "HTTP",
          "disk IO",
        ],
        fixture: {
          sessions: COUNT,
          projects: PROJECTS,
          models: MODELS,
          tools: tools.length,
          days: DAYS,
          transcriptBodies: 0,
        },
        warmup: WARMUP,
        rounds: ROUNDS,
        language: i18n.language,
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        viewport: {
          width: innerWidth,
          height: innerHeight,
          dpr: devicePixelRatio,
        },
        userAgent: navigator.userAgent,
        targetMs: TARGET,
        p95Definition: "nearest rank ceil(0.95*N)",
        blockedAttempts: { ...blocked },
        results,
        worstPageP95Ms: Math.max(
          ...results.map((result) => result.paintOpportunityP95Ms),
        ),
        pass:
          results.every((result) => result.pass) &&
          !blocked.ipc &&
          !blocked.network,
      };
      setReport(result);
      if (status.current)
        status.current.textContent = result.pass
          ? "PASS · renderer update p95 < 500 ms"
          : "FAIL · inspect the results";
    } catch (error) {
      setReport({
        error: String(error),
        partialResults: results,
        blockedAttempts: { ...blocked },
      });
      if (status.current)
        status.current.textContent = "INVALID · benchmark did not finish";
    } finally {
      guard.current = false;
      setRunning(false);
    }
  };
  return (
    <>
      <header id="benchmark-controls">
        <h1>OFox production renderer update benchmark</h1>
        <p>
          10,000 synthetic sessions · body-free metadata · no native scanning,
          IPC, account or network. Static local assets only. Run with this tab
          visible. 3 warmups + 30 samples per page. Threshold: p95 &lt; 500 ms.
        </p>
        <button
          id="run-benchmark"
          disabled={running}
          onClick={() => void run()}
        >
          Run both pages
        </button>
        <label>
          UI locale{" "}
          <select
            disabled={running}
            value={language}
            onChange={(event) => {
              const value = event.target.value;
              setLanguage(value);
              void i18n.changeLanguage(value);
            }}
          >
            <option value="zh">中文</option>
            <option value="en">English</option>
            <option value="ja">日本語</option>
          </select>
        </label>
        <button
          disabled={running}
          onClick={() => document.documentElement.classList.toggle("dark")}
        >
          Light / dark
        </button>
        <p ref={status} id="benchmark-status" role="status">
          Ready · production build required
        </p>
      </header>
      <main id="benchmark-surface" data-revision={frame.revision}>
        <Surface
          key={`${frame.page}-${frame.run}`}
          page={frame.page}
          data={frame.data}
        />
      </main>
      <pre id="benchmark-results" aria-label="Benchmark JSON results">
        {report ? JSON.stringify(report, null, 2) : "No result yet"}
      </pre>
    </>
  );
}
createRoot(document.getElementById("root")!).render(
  <QueryClientProvider client={client}>
    <ShellProvider openSettings={() => undefined}>
      <Harness />
    </ShellProvider>
  </QueryClientProvider>,
);
