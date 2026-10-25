import type { UsageGranularity } from "@/lib/api/usageOverview";
import type { UsageRequestMeasurements } from "@/types/usage";
import type { UsageEstimate } from "@/types/usageQuery";

export type RequestMetric = "tokens" | "cost" | "requests";
export type RequestDimension = "model" | "modelProvider" | "provider" | "agent";
export type RequestChartDimension = Extract<
  RequestDimension,
  "model" | "agent"
>;

/** Request-only projection: shared UsageTotals.totalTokens keeps its meaning. */
export interface RequestDashboardTotals {
  requests: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokensIncludingCache: number;
  cacheHitRate: number;
  estimate: UsageEstimate;
  measurements: UsageRequestMeasurements;
}

export interface RequestDashboardPoint {
  start: number;
  end: number;
  totals: RequestDashboardTotals;
}

export interface RequestDashboardGroup {
  id: string;
  label: string | null;
  modelId?: string | null;
  toolId?: string | null;
  providerKey?: string | null;
  providerLabel?: string | null;
  totals: RequestDashboardTotals;
  /** Sparse buckets; the dashboard's series supplies the complete calendar. */
  series: RequestDashboardPoint[];
}

export interface UsageRequestDashboard {
  granularity: UsageGranularity;
  totals: RequestDashboardTotals;
  /** Full-period context with the current rank selection, before day focus. */
  series: RequestDashboardPoint[];
  dimensions: Record<
    RequestDimension,
    {
      groups: RequestDashboardGroup[];
      /** Retains the other conditions, ignoring only this dimension's rank focus. */
      rankings: RequestDashboardGroup[];
    }
  >;
  providers: { key: string; label: string | null }[];
}
