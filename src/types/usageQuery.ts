/** Local history queries are account-independent and use [startMs, endMs). */
export interface UsageQueryV1 {
  version: 1;
  startMs: number | null;
  endMs: number;
  toolIds: string[];
  modelIds: string[];
  projectIds: string[];
  sourceIds: string[];
}

export interface UsageCoverage {
  state: "complete" | "partial" | "unsupported";
  reasons: string[];
  timeBasis: "eventTime" | "legacyLocalDate" | "mixed";
  detailStartMs: number | null;
  detailEndMs: number | null;
  aggregateOnly: boolean;
  excludedRollupDays: number;
  sourceIds: string[];
}

export interface UsageEstimate {
  knownCostUsd: string | null;
  availability: "known" | "free" | "partial" | "unknown";
  unpricedModelIds: string[];
  unpricedRecordCount: number | null;
  additionalUnitsUnsupported: boolean;
}

export interface UsagePricingCoverage {
  policy: "current-ofox-base-v1";
  apex: string;
  catalogVersion: string | null;
  catalogUpdatedAtMs: number | null;
  stale: boolean;
  unpricedModelIds: string[];
  unpricedRecordCount: number | null;
  additionalUnitsUnsupported: boolean;
  catalogState?: "missing" | "ready" | "invalid" | "unsupported";
  errorCode?: string | null;
}

export interface UsageReceipt {
  appliedQuery: UsageQueryV1;
  queryFingerprint: string;
  snapshotVersion: string;
  coverage: UsageCoverage;
  pricingCoverage: UsagePricingCoverage;
}

export type UsageSnapshotReceipt = Pick<
  UsageReceipt,
  "queryFingerprint" | "snapshotVersion"
>;

export interface UsageCapabilities {
  queryVersion: 1;
  requestSnapshots: boolean;
  csv: boolean;
  projectFilter: boolean;
  sessionAnalytics?: boolean;
  sessionProjectFilter?: boolean;
  localRefresh?: boolean;
  sources: {
    toolId: string;
    sourceIds: string[];
    usageSupported: boolean;
    lifetimeUsageSupported?: boolean;
    sessionAnalyticsSupported: boolean;
    projectFilterSupported: boolean;
    reason: string | null;
  }[];
}

export interface UsageSourceHealth {
  sourceId: string;
  toolId: string;
  status: "unknown" | "ok" | "partial" | "failed" | "unsupported";
  lastAttemptAtMs: number | null;
  lastSuccessAtMs: number | null;
  errorCode: string | null;
}

export interface UsageFilterOptions {
  toolIds: string[];
  modelIds: string[];
  sourceIds: string[];
}

export interface UsageRequestFilters {
  sourceIds?: string[];
  statusCode?: number | null;
  statusKind?: "recorded" | "unrecorded" | null;
  search?: string | null;
}

export interface UsageCsvExport {
  fileName: string;
  csv: string;
  recordCount: number;
  coverage: UsageCoverage;
  queryFingerprint: string;
  snapshotVersion: string;
  savedPath?: string | null;
}
