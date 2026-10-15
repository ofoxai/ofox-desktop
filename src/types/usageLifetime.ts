import type { UsageEstimate, UsagePricingCoverage } from "./usageQuery";

export type LifetimeCoverage = "complete" | "partial" | "unknown";
export interface LifetimeBuckets {
  inputTokens: number | null;
  outputTokens: number | null;
  cacheReadTokens: number | null;
  cacheWriteTokens: number | null;
  /** Fresh input + output. Cache remains separate. */
  totalTokens: number | null;
  /** A native cumulative counter, not attributable request details. */
  apiCallCount: number | null;
  amountKind: "known" | "lowerBound" | "unknown";
}

export interface LifetimeModelSummary {
  modelId: string | null;
  category: "main" | "voice" | "auxiliary" | "unattributed";
  buckets: LifetimeBuckets;
  estimate: UsageEstimate;
}

/** Native cumulative facts have no event-time query or manufactured ledger rows. */
export interface UsageLifetimeSummary {
  version: 1;
  toolId: "hermes";
  sourceId: "hermes-lifetime";
  scope: "lifetime";
  snapshotVersion: string | null;
  observedAtMs: number;
  lastSuccessAtMs: number | null;
  sourceState: "ready" | "missing" | "error";
  stale: boolean;
  reasons: string[];
  sessionRecords: number | null;
  totals: LifetimeBuckets;
  byModel: LifetimeModelSummary[];
  counterCoverage: LifetimeCoverage;
  modelCoverage: LifetimeCoverage;
  estimate: UsageEstimate;
  pricingCoverage: UsagePricingCoverage;
}
