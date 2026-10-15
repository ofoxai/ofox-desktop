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
  series: AnalyticsDay[];
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
}
