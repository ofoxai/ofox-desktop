/** README-only production components with fictional in-memory data. */
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  libraryApi,
  type LibraryData,
  type LibraryKind,
  type LibraryBatchPlan,
} from "@/lib/api/library";
import { LibraryPage } from "@/components/library/LibraryPage";
import { LibraryDiscover } from "@/components/library/LibraryDiscover";
import { LibraryBatchReview } from "@/components/library/LibraryBatchReview";
import { SessionsPage } from "@/components/sessions/SessionsPage";
import { UsagePage } from "@/components/usage-overview/UsagePage";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useShell } from "@/components/shell/ShellContext";
import { account } from "./readme-showcase-data";
import type { ManagedSession } from "@/lib/api/sessions";
import type { UsageQueryV1 } from "@/types/usageQuery";
import type { PaginatedLogs, RequestLog } from "@/types/usage";
import { sessionAnalyticsFixture } from "../tests/components/usage/sessionAnalyticsFixture";

const mode = "complete" as
  | "complete"
  | "partial"
  | "unavailable"
  | "zero"
  | "legacy";
const apex = "ofox.ai";
const toolIds = [
  "claude",
  "codex",
  "opencode",
  "openclaw",
  "hermes",
  "gemini",
  "workbuddy",
  "dsh",
  "pi",
];
const targets = toolIds.map((toolId) => ({
  toolId,
  available: true,
  discoveryStatus: "installed",
  path: `/demo/${toolId}/config`,
  targetGroupId: toolId,
  status: "unassigned",
  desired: false,
  nativeState: "ready",
}));
const mcp = (
  id: string,
  name: string,
  summary: string,
  transport: string,
  enabled: string[],
) => ({
  id,
  name,
  revision: 1,
  transport,
  credentialState: "none",
  displaySummary: summary,
  iconId: id,
  fields: [],
  targets: targets.map((target) => ({
    ...target,
    desired: enabled.includes(target.toolId),
    applied: enabled.includes(target.toolId),
    status: enabled.includes(target.toolId) ? "applied" : "unassigned",
  })),
});
const db: Record<LibraryKind, LibraryData> = {
  instructions: {
    epoch: 1,
    instructions: [
      {
        id: "team",
        name: "Aurora engineering guidelines",
        content:
          "# Engineering guidelines\n\nRead the code, keep changes focused, and verify the result.\n\n- Preserve user configuration.\n- Use the project test suite.\n- Keep interfaces accessible.",
        revision: 1,
      },
      {
        id: "review",
        name: "Code review",
        content: "Check correctness, privacy, and test coverage.",
        revision: 1,
      },
      {
        id: "writing",
        name: "Technical writing",
        content: "Use clear language and explain why a change matters.",
        revision: 1,
      },
    ],
    targets: targets.map((target) => ({
      ...target,
      status: "applied",
      desiredResourceId: "team",
      readableResourceId: "team",
      managedContentLines: 8,
      userContentLines: 12,
    })),
    operations: [],
  },
  mcp: {
    epoch: 1,
    targets,
    servers: [
      mcp("context7", "Context7", "mcp.context7.com/mcp", "http", [
        "claude",
        "codex",
        "opencode",
        "gemini",
      ]),
      mcp(
        "chrome-devtools",
        "Chrome DevTools",
        "npx · chrome-devtools-mcp",
        "stdio",
        ["claude", "codex", "opencode"],
      ),
      mcp("playwright", "Playwright", "npx · @playwright/mcp", "stdio", [
        "claude",
        "codex",
      ]),
      mcp("deepwiki", "DeepWiki", "mcp.deepwiki.com/mcp", "http", [
        "codex",
        "opencode",
        "hermes",
      ]),
      mcp(
        "filesystem",
        "Filesystem",
        "npx · server-filesystem · /demo/aurora",
        "stdio",
        ["claude", "codex", "opencode", "hermes"],
      ),
    ],
    operations: [],
  },
  skills: {
    epoch: 1,
    skills: [
      {
        id: "frontend-design",
        name: "frontend-design",
        description:
          "Build thoughtful interfaces with clear hierarchy and accessible interaction.",
        directory: "frontend-design",
        revision: 1,
        artifactHash: "demo-frontend",
        sourceState: "ready",
        sourceKind: "repository",
        repository: "anthropics/skills",
        commitSha: "a".repeat(40),
        targets: targets
          .slice(0, 4)
          .map((target) => ({ ...target, status: "applied", desired: true })),
      },
      {
        id: "code-review",
        name: "code-review",
        description:
          "Review correctness, maintainability, and regression risks.",
        directory: "code-review",
        revision: 1,
        artifactHash: "demo-review",
        sourceState: "ready",
        sourceKind: "localOrArchive",
        targets,
      },
    ],
    skillTargets: toolIds.map((id) => ({
      id,
      label: id,
      detected: true,
      readsShared: id === "codex",
      scopeKnown: true,
    })),
    operations: [],
  },
};
const preview: LibraryBatchPlan = {
  operationId: "readme-preview",
  kind: "mcp",
  status: "previewed",
  resources: [
    { resourceId: "context7", name: "Context7", expectedRevision: 1 },
    {
      resourceId: "chrome-devtools",
      name: "Chrome DevTools",
      expectedRevision: 1,
    },
  ],
  groups: ["claude", "codex", "opencode"].map((toolId) => ({
    groupId: toolId,
    tools: [toolId],
    readers: [toolId],
    path: `/demo/${toolId}/config`,
    status: "pending",
    resourceIds: ["context7", "chrome-devtools"],
    changedTools: [toolId],
    changes: [
      {
        resourceId: "context7",
        name: "Context7",
        action: "apply",
        tools: [toolId],
        changedFields: ["mcpServers.context7"],
        status: "pending",
      },
    ],
  })),
};

export function installReadmePageMocks() {
  libraryApi.summary = async () => ({ instructions: 3, mcp: 5, skills: 2 });
  libraryApi.list = async (kind) => structuredClone(db[kind]);
  libraryApi.inventory = async (kind) => ({
    ticketId: `demo-${kind}`,
    kind,
    expiresAtMs: Date.now() + 600000,
    sources: [],
    entries: [],
  });
  libraryApi.discardInventory = async () => true;
  libraryApi.matchMcpRecipes = async () => [];
  libraryApi.matchBuiltinRecipes = async () => [];
  libraryApi.batches = async () => [];
  libraryApi.previewBatch = async () => structuredClone(preview);
  libraryApi.runBatch = async () => {
    throw new Error("README demo disables configuration writes");
  };
  libraryApi.cancelBatch = async () => ({
    ...structuredClone(preview),
    status: "cancelled",
  });
  libraryApi.prepareMcpCheck = async () => {
    throw new Error("README demo disables external connections");
  };
  localStorage.setItem("ofox.usagePeriod", "7d");
  localStorage.setItem("ofox.usageRefreshSeconds", "0");
  localStorage.setItem("ofox.usageTab", "overview");
}

const sampleTitles = [
  "Design the Aurora dashboard",
  "Review the API contracts",
  "Polish the onboarding flow",
  "Add accessible keyboard navigation",
  "Optimize the streaming response",
  "Write focused regression tests",
  "Plan the next release",
  "Document the workspace setup",
];
const modelIds = ["gpt-5", "claude-sonnet-4-5", "gemini-2.5-pro", "gpt-5"];
function sampleSessions(): ManagedSession[] {
  const now = Date.now();
  return sampleTitles.map((title, index) => ({
    sessionKey: `readme-session-${index}`,
    sessionId: `demo-${index}`,
    revision: "readme-v1",
    providerId: ["codex", "claude", "gemini", "opencode"][index % 4],
    projectDir: index < 5 ? "/demo/aurora" : "/demo/design-system",
    sourcePath: `/demo/sessions/session-${index}.jsonl`,
    title,
    createdAt: now - (index + 2) * 3600000,
    lastActiveAt: now - (index + 1) * 600000,
    messageCount: 24 + index * 4,
    sizeBytes: 18000 + index * 2048,
    models: [modelIds[index % 4]],
    capabilities: {
      canReadMessages: true,
      canExport: true,
      canResume: false,
      canTrash: false,
      resumeUsesSessionModel: false,
      resumeUnavailableReason: "demo",
    },
    usage: {
      inputTokens: 12000 + index * 1800,
      outputTokens: 4200 + index * 600,
      cacheReadTokens: 86000 + index * 9000,
      cacheWriteTokens: 0,
      totalTokens: 16200 + index * 2400,
      costUsd: 0.08 + index * 0.012,
      unpricedModels: 0,
      models: [
        {
          model: modelIds[index % 4],
          requests: 12 + index,
          inputTokens: 12000 + index * 1800,
          outputTokens: 4200 + index * 600,
          cacheReadTokens: 86000 + index * 9000,
          cacheWriteTokens: 0,
          costUsd: 0.08 + index * 0.012,
        },
      ],
    },
  }));
}

function analysis(query: UsageQueryV1) {
  const data = sessionAnalyticsFixture();
  const unknown = mode === "unavailable";
  const zero = unknown || mode === "zero";
  const partial = mode === "partial";
  const lastDay = new Date(query.endMs - 1);
  lastDay.setHours(0, 0, 0, 0);
  const dayMs = lastDay.getTime() - 6 * 86400000;
  const tokens = (scale: number) => ({
    inputTokens: zero ? 0 : 12000 * scale,
    outputTokens: zero ? 0 : 4200 * scale,
    cacheReadTokens: zero ? 0 : 86000 * scale,
    cacheWriteTokens: 0,
    totalTokens: zero ? 0 : 16200 * scale,
    cacheHitRate: zero ? 0 : 86000 / 98000,
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
          id: "aurora-studio",
          label: "Aurora Studio",
          path: "/demo/aurora",
          sessionCount: 3,
          estimate: data.totals.estimate,
        },
      ];
  data.byModel = unknown
    ? []
    : [
        {
          ...tokens(28),
          id: "gpt-5",
          label: "gpt-5",
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
          ? [
              "Design the Aurora dashboard",
              "Review the API contracts",
              "Polish the onboarding flow",
            ][index]
          : "Design the Aurora dashboard",
        projectId: "aurora-studio",
        projectLabel: data.byProject[0].label,
        projectPath: "/demo/aurora",
        tokens: tokens(scale),
        estimate: estimate(tokens(scale).totalTokens),
        models: [
          {
            ...tokens(scale),
            model: "gpt-5",
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
    modelIds: ["gpt-5"],
    sourceIds: ["codex_session"],
    projects: [
      {
        id: "aurora-studio",
        label: "Aurora Studio",
        path: "/demo/aurora",
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
    model: "gpt-5",
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

function overviewShare(scale: number) {
  const totalTokens = 16200 * scale;
  return {
    inputTokens: 12000 * scale,
    outputTokens: 4200 * scale,
    cacheReadTokens: 86000 * scale,
    cacheWriteTokens: 0,
    totalTokens,
    cacheHitRate: 86 / 98,
    sessionCount: 1,
    costUsd: totalTokens / 200000,
    estimate: {
      knownCostUsd: (totalTokens / 200000).toFixed(6),
      availability: "known" as const,
      unpricedModelIds: [],
      unpricedRecordCount: 0,
      additionalUnitsUnsupported: false,
    },
  };
}

export const supportedReadmePageIpc = [
  "list_sessions",
  "refresh_session_index",
  "list_session_trash",
  "get_session_transcript",
  "get_usage_capabilities",
  "get_usage_rebuild_progress",
  "get_session_usage_analytics",
  "get_usage_overview",
  "get_request_logs",
  "get_request_detail",
  "get_session_analytics_page",
  "sync_session_usage",
  "refresh_local_usage",
  "ofox_get_balance_trend",
  "ofox_get_spending",
] as const;

export async function handleReadmePageIpc(
  command: string,
  args: Record<string, unknown> = {},
): Promise<unknown> {
  switch (command) {
    case "list_sessions":
    case "refresh_session_index":
      return sampleSessions();
    case "list_session_trash":
      return [];
    case "get_session_transcript":
      return {
        readRevision: "readme-v1",
        messages: [
          {
            role: "user",
            content: "Create a clear, accessible dashboard for Aurora Studio.",
            timestamp: Date.now() - 1200000,
          },
          {
            role: "assistant",
            content:
              "## Implementation plan\n\n1. Build a focused overview with clear metrics.\n2. Add keyboard navigation and responsive layouts.\n3. Verify the important flows with focused tests.\n\nThis is sample conversation data for the README showcase.",
            timestamp: Date.now() - 1100000,
          },
        ],
      };
    case "get_usage_capabilities":
      return {
        queryVersion: 1,
        requestSnapshots: true,
        csv: false,
        projectFilter: true,
        sessionAnalytics: true,
        sessionProjectFilter: true,
        localRefresh: true,
        nativeRebuild: false,
        sources: [],
      };
    case "get_usage_rebuild_progress":
      return null;
    case "sync_session_usage":
    case "refresh_local_usage":
      return { imported: 7, skipped: 0, filesScanned: 7, errors: [] };
    case "get_session_usage_analytics":
      return analysis(args.query as UsageQueryV1);
    case "get_usage_overview": {
      const query = args.query as UsageQueryV1;
      const data = analysis(query);
      return {
        ...data,
        period: args.period,
        start: query.startMs,
        end: query.endMs,
        granularity: "day",
        totals: {
          ...data.totals,
          requests: 148,
          cacheHitRate: data.totals.cacheHitRate ?? 0,
          costUsd: Number(data.totals.estimate.knownCostUsd),
        },
        series: data.series.map((day, index) => ({
          ...day,
          start: day.startMs,
          requests: [12, 19, 15, 28, 22, 31, 21][index],
          costUsd: Number(day.estimate.knownCostUsd),
        })),
        byTool: [16, 8, 4].map((scale, index) => ({
          ...data.byTool[0],
          ...overviewShare(scale),
          appType: ["codex", "claude", "opencode"][index],
          requests: [72, 49, 27][index],
        })),
        byModel: [16, 8, 4].map((scale, index) => ({
          ...data.byModel[0],
          ...overviewShare(scale),
          id: ["gpt-5", "claude-sonnet-4-5", "gemini-2.5-pro"][index],
          label: ["gpt-5", "claude-sonnet-4-5", "gemini-2.5-pro"][index],
          model: ["gpt-5", "claude-sonnet-4-5", "gemini-2.5-pro"][index],
          appTypes: [["codex"], ["claude"], ["opencode"]][index],
          priced: true,
          requests: [72, 49, 27][index],
        })),
        attribution: {
          sessionCount: 3,
          projectCount: 1,
          unassociatedRecords: 0,
          partial: false,
        },
        unsupportedTools: [],
        lastSyncedAt: query.endMs - 1000,
      };
    }
    case "ofox_get_balance_trend": {
      const now = Date.now();
      return {
        points: Array.from({ length: 14 }, (_, index) => ({
          at: now - (13 - index) * 86400000,
          amount: 60.35 - index * 1.35,
        })),
        perDay: 1.35,
        runsOutAt: now + 32 * 86400000,
        daysLeft: 32,
      };
    }
    case "ofox_get_spending":
      return { forbidden: false, monthlyUsedUsd: 17.5, monthlyLimitUsd: 100 };
    case "get_request_logs": {
      if (typeof args.snapshotId === "string" && snapshots.has(args.snapshotId))
        return structuredClone(snapshots.get(args.snapshotId));
      const data = analysis(args.query as UsageQueryV1);
      const rows = records(data, false);
      const result: PaginatedLogs = {
        ...data,
        data: rows,
        total: rows.length,
        page: 0,
        pageSize: Number(args.pageSize ?? 20),
        snapshotId: "readme-requests",
        requestFilters: {},
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
            requests: 1,
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
      snapshots.set(result.snapshotId!, result);
      return result;
    }
    case "get_request_detail":
      return (
        snapshots
          .get(String(args.snapshotId))
          ?.data.find((row) => row.requestId === args.requestId) ?? null
      );
    case "get_session_analytics_page": {
      const receipt = args.expectedReceipt as {
        queryFingerprint: string;
        snapshotVersion: string;
        appliedQuery: UsageQueryV1;
      };
      const data = analysis(receipt.appliedQuery);
      return {
        ...data,
        ...receipt,
        analyticsSnapshotId: args.analyticsSnapshotId,
        data: data.topSessions,
        total: data.topSessions.length,
        page: args.page,
        pageSize: args.pageSize,
        appliedView: {
          sortBy: args.sortBy,
          search:
            typeof args.search === "string"
              ? args.search.trim().toLowerCase()
              : null,
          managerKey: args.managerKey ?? null,
        },
        viewFingerprint: "readme-view",
      };
    }
    default:
      throw new Error(`README demo does not support ${command}`);
  }
}

const mcpLocation = { kind: "mcp" as const, request: 1 };
export function ReadmeOtherPage({
  page,
  scene,
}: {
  page: "library" | "sessions" | "usage";
  scene?: string;
  lang?: string;
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const [open, setOpen] = useState(true);
  useEffect(() => {
    if (page === "sessions") shell.locateSession("readme-session-0");
  }, [page, shell.locateSession]);
  if (page === "sessions") return <SessionsPage className="h-full pb-4" />;
  if (page === "usage")
    return (
      <div className="h-full overflow-y-auto pb-4">
        <UsagePage
          account={account}
          boundTools={["codex", "claude", "opencode"]}
        />
      </div>
    );
  return (
    <div className="h-full overflow-y-auto pb-4">
      <LibraryPage location={mcpLocation} />
      {scene === "discover" && (
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogContent
            style={{ maxWidth: 1024, width: "calc(100vw - 48px)" }}
          >
            <DialogHeader>
              <DialogTitle>{t("library.mcpTable.discover")}</DialogTitle>
            </DialogHeader>
            <div className="min-h-0 flex-1 overflow-y-auto px-[18px] py-4">
              <LibraryDiscover kind="mcp" onChoose={() => undefined} />
            </div>
          </DialogContent>
        </Dialog>
      )}
      {scene === "mcp-preview" && (
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogContent className="sm:max-w-3xl">
            <DialogHeader>
              <DialogTitle>{t("library.preview")}</DialogTitle>
            </DialogHeader>
            <div className="min-h-0 flex-1 overflow-y-auto px-[18px] py-4">
              <LibraryBatchReview
                kind="mcp"
                resources={preview.resources.map((resource) => ({
                  resourceId: resource.resourceId,
                  expectedRevision: 1,
                }))}
                tools={["claude", "codex", "opencode"]}
                action="apply"
                epoch={1}
                initial={preview}
                onBusyChange={() => undefined}
                onChanged={async () => undefined}
              />
            </div>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}
