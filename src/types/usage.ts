// 使用统计相关类型定义
import type {
  UsageEstimate,
  UsageFilterOptions,
  UsageQueryV1,
  UsageReceipt,
  UsageRequestFilters,
  UsageSnapshotReceipt,
} from "@/types/usageQuery";
import type {
  UsageGranularity,
  UsageSeriesPoint,
} from "@/lib/api/usageOverview";

export interface TokenUsage {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
}

export interface RequestLog {
  requestId: string;
  providerId: string;
  providerName?: string;
  appType: string;
  model: string;
  requestModel?: string;
  costMultiplier: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  inputCostUsd: string;
  outputCostUsd: string;
  cacheReadCostUsd: string;
  cacheCreationCostUsd: string;
  totalCostUsd: string;
  /** Explicit null means no original saved amount exists for this rebuilt event. */
  recordedCostUsd?: string | null;
  isStreaming: boolean;
  latencyMs: number;
  firstTokenMs?: number;
  durationMs?: number;
  statusCode: number;
  errorMessage?: string;
  createdAt: number;
  /** Canonical native event time; absent on older backends. */
  createdAtMs?: number;
  dataSource?: string;
  observedStatusCode?: number | null;
  observedLatencyMs?: number | null;
  observedFirstTokenMs?: number | null;
  currentEstimate?: UsageEstimate;
  receipt?: Pick<
    UsageReceipt,
    "appliedQuery" | "queryFingerprint" | "snapshotVersion"
  >;
}

export interface SessionSyncResult {
  imported: number;
  skipped: number;
  filesScanned: number;
  errors: string[];
}

export interface DataSourceSummary {
  dataSource: string;
  requestCount: number;
  totalCostUsd: string;
}

export interface PaginatedLogs extends Partial<UsageReceipt> {
  data: RequestLog[];
  total: number;
  page: number;
  pageSize: number;
  snapshotId?: string;
  options?: UsageFilterOptions;
  requestFilters?: UsageRequestFilters | null;
  trend?: { granularity: UsageGranularity; series: UsageSeriesPoint[] };
  measurements?: UsageRequestMeasurements;
  summary?: {
    requests: number;
    inputTokens: number;
    outputTokens: number;
    cacheReadTokens: number;
    cacheWriteTokens: number;
    totalTokens: number;
    cacheHitRate: number;
    costUsd: number;
    estimate: UsageEstimate;
  };
}

export interface UsageRequestMeasurements {
  statusSamples: number;
  failedSamples: number;
  failureRate: number | null;
  latencySamples: number;
  avgLatencyMs: number | null;
  firstTokenSamples: number;
  avgFirstTokenMs: number | null;
  generationSpeedSamples: number;
  avgGenerationTokensPerSecond: number | null;
}

export interface RequestQueryOptions {
  query: UsageQueryV1;
  /** Frontend-only ownership guard, never sent as a product IPC argument. */
  expectedApex?: string;
  requestFilters?: UsageRequestFilters;
  snapshotId?: string;
  expectedReceipt?: UsageSnapshotReceipt;
}

export interface ModelPricing {
  modelId: string;
  displayName: string;
  inputCostPerMillion: string;
  outputCostPerMillion: string;
  cacheReadCostPerMillion: string;
  cacheCreationCostPerMillion: string;
}

export interface UsageSummary {
  totalRequests: number;
  totalCost: string;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
  successRate: number;
}

export interface DailyStats {
  date: string;
  requestCount: number;
  totalCost: string;
  totalTokens: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
}

export interface ProviderStats {
  providerId: string;
  providerName: string;
  requestCount: number;
  totalTokens: number;
  totalCost: string;
  successRate: number;
  avgLatencyMs: number;
}

export interface ModelStats {
  model: string;
  requestCount: number;
  totalTokens: number;
  totalCost: string;
  avgCostPerRequest: string;
}

export interface LogFilters {
  appType?: string;
  providerName?: string;
  model?: string;
  statusCode?: number;
  startDate?: number;
  endDate?: number;
}

export interface ProviderLimitStatus {
  providerId: string;
  dailyUsage: string;
  dailyLimit?: string;
  dailyExceeded: boolean;
  monthlyUsage: string;
  monthlyLimit?: string;
  monthlyExceeded: boolean;
}

export type UsageRangePreset = "today" | "1d" | "7d" | "14d" | "30d" | "custom";

export interface UsageRangeSelection {
  preset: UsageRangePreset;
  customStartDate?: number;
  customEndDate?: number;
}

export type AppTypeFilter = "all" | "claude" | "codex" | "gemini";

export interface StatsFilters {
  timeRange: UsageRangePreset;
  providerId?: string;
  appType?: string;
}
