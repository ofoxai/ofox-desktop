import type { UsageLifetimeSummary } from "@/types/usageLifetime";

export function lifetimeFixture(): UsageLifetimeSummary {
  const estimate = {
    knownCostUsd: "0.31",
    availability: "known" as const,
    unpricedModelIds: [],
    unpricedRecordCount: null,
    additionalUnitsUnsupported: false,
  };
  const buckets = {
    inputTokens: 40,
    outputTokens: 10,
    cacheReadTokens: 100,
    cacheWriteTokens: 20,
    totalTokens: 50,
    apiCallCount: 3,
    amountKind: "known" as const,
  };
  return {
    version: 1,
    toolId: "hermes",
    sourceId: "hermes-lifetime",
    scope: "lifetime",
    snapshotVersion: "native-snapshot-1",
    observedAtMs: 1791993600000,
    lastSuccessAtMs: 1791993500000,
    sourceState: "ready",
    stale: false,
    reasons: ["historicalModelAttributionUnverified"],
    sessionRecords: 2,
    totals: buckets,
    byModel: [{ modelId: "native-model", category: "main", buckets, estimate }],
    counterCoverage: "complete",
    modelCoverage: "partial",
    estimate,
    pricingCoverage: {
      policy: "current-ofox-base-v1",
      apex: "ofox.ai",
      catalogVersion: "catalog-1",
      catalogUpdatedAtMs: 1791993400000,
      stale: false,
      unpricedModelIds: [],
      unpricedRecordCount: null,
      additionalUnitsUnsupported: false,
      catalogState: "ready",
    },
  };
}
