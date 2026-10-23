import type {
  UsageEstimate,
  UsageFilterOptions,
  UsageReceipt,
  UsageSourceHealth,
} from "@/types/usageQuery";

export interface AnalyticsTokens {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  cacheHitRate: number | null;
}
export interface AnalyticsKpis extends AnalyticsTokens {
  sessionCount: number;
  projectCount: number;
  medianTokens: string | null;
  p90Tokens: number | null;
  quantileSamples: number;
  quantileExcluded: number;
  estimate: UsageEstimate;
  messages: number | null;
  activeMs: number | null;
  activeDays: number | null;
  messageReason: string | null;
  activityReason: string | null;
}
export interface AnalyticsDay extends AnalyticsTokens {
  date: string;
  startMs: number;
  endMs: number;
  sessionCount: number;
  estimate: UsageEstimate;
  messages: number | null;
  activeMs: number | null;
  knownMessages?: number | null;
  knownActiveMs?: number | null;
}
export interface AnalyticsRank extends AnalyticsTokens {
  id: string;
  label: string;
  path: string | null;
  sessionCount: number;
  estimate: UsageEstimate;
}
export interface AnalyticsModel extends AnalyticsTokens {
  model: string;
  estimate: UsageEstimate;
}
export interface AnalyticsSession {
  sessionKey: string;
  nativeSessionId: string;
  toolId: string;
  title: string;
  projectId: string | null;
  projectLabel: string | null;
  projectPath: string | null;
  managerKey: string | null;
  sourceAvailable: boolean;
  tokens: AnalyticsTokens | null;
  estimate: UsageEstimate;
  models: AnalyticsModel[];
  messages: number | null;
  activeMs: number | null;
  reasons: string[];
}
export interface SessionAnalytics extends UsageReceipt {
  totals: AnalyticsKpis;
  /** Natural-day calendar data, independent of the trend's bucket size. */
  series: AnalyticsDay[];
  /** Receipt-bound trend; absent on older backends that only expose days. */
  trend?: { granularity: "hour" | "day"; series: AnalyticsDay[] };
  byProject: AnalyticsRank[];
  byModel: AnalyticsRank[];
  byTool: AnalyticsRank[];
  topSessions: AnalyticsSession[];
  topLimit: number;
  options: UsageFilterOptions & {
    projects: { id: string; label: string; path: string | null }[];
  };
  unassociated: AnalyticsTokens & { records: number };
  sources: UsageSourceHealth[];
  compactTop?: Record<AnalyticsSort, AnalyticsSession[]>;
  knownMetrics?: {
    messages: number;
    activeMs: number;
    activeDays: number;
  } | null;
  metricCoverage?: AnalyticsMetricCoverage;
  messageCoverage?: AnalyticsMetricCoverage;
  activityCoverage?: AnalyticsMetricCoverage;
  knownMessages?: number | null;
  knownActiveMs?: number | null;
  knownActiveDays?: number | null;
  activityByHour?: {
    timezone: string;
    utcOffsetSecondsAtEnd?: number;
    cells: { weekday: number; hour: number; activeMs: number }[];
    coverage: AnalyticsMetricCoverage;
  };
  distributions?: AnalyticsDistribution[];
  interactions?: AnalyticsInteractions;
  analyticsSnapshotId?: string | null;
  pagingUnavailableReason?: string | null;
}

export type AnalyticsSort = "tokens" | "cost" | "active";
export interface AnalyticsMetricCoverage {
  state: "complete" | "partial" | "unavailable";
  eligibleSessions: number;
  excludedSessions: number;
  unquantifiedSourceGaps: boolean;
  reasons: string[];
}
export interface AnalyticsDistribution {
  metric: "tokens" | "messages" | "activeMs" | "toolCallsPerPrompt";
  edges: number[];
  counts: number[];
  samples: number;
  excludedSessions: number;
  unquantifiedSourceGaps: boolean;
}
export interface AnalyticsPage
  extends Pick<
    UsageReceipt,
    "appliedQuery" | "queryFingerprint" | "snapshotVersion"
  > {
  analyticsSnapshotId: string;
  data: AnalyticsSession[];
  total: number;
  page: number;
  pageSize: number;
  appliedView: {
    sortBy: AnalyticsSort;
    search: string | null;
    managerKey?: string | null;
  };
  viewFingerprint: string;
}

export interface AnalyticsToolStat {
  name: string;
  category: string;
  calls: number;
  sessions: number;
}
export interface AnalyticsInteractions {
  coverage: AnalyticsMetricCoverage;
  tools: {
    calls: number;
    sessions: number;
    top: AnalyticsToolStat[];
    categories: AnalyticsToolStat[];
    weeks: { start: string; calls: Record<string, number> }[];
  };
  skills: {
    calls: number;
    count: number;
    top: {
      name: string;
      calls: number;
      sessions: number;
      last: string;
      agents: Record<string, number>;
      projects: { name: string; calls: number }[];
    }[];
  };
  autonomy: AnalyticsDistribution;
}
