import type {
  RequestDashboardTotals,
  UsageRequestDashboard,
} from "@/types/usageRequestDashboard";

export const emptyRequestMeasurements = {
  statusSamples: 0,
  failedSamples: 0,
  failureRate: null,
  latencySamples: 0,
  avgLatencyMs: null,
  firstTokenSamples: 0,
  avgFirstTokenMs: null,
  generationSpeedSamples: 0,
  avgGenerationTokensPerSecond: null,
};

/** Fictional native records: small known prices and one unpriced model. */
export function requestDashboardFixture(count = 8): UsageRequestDashboard {
  const start = Date.UTC(2026, 9, 18);
  const totals = (n: number, unknown = false): RequestDashboardTotals => ({
    requests: n * 7,
    inputTokens: n * 1400,
    outputTokens: n * 700,
    cacheReadTokens: n * 35000,
    cacheWriteTokens: n * 1400,
    totalTokensIncludingCache: n * 38500,
    cacheHitRate: 35000 / 37800,
    estimate: {
      knownCostUsd: unknown ? null : (n * 0.001).toFixed(3),
      availability: unknown ? "unknown" : "known",
      unpricedModelIds: unknown ? ["fixture-model-8"] : [],
      unpricedRecordCount: unknown ? n * 7 : 0,
      additionalUnitsUnsupported: false,
    },
    measurements: { ...emptyRequestMeasurements },
  });
  const groups = Array.from({ length: count }, (_, i) => ({
    id: `fixture-model-${i + 1}`,
    label: `fixture-model-${i + 1}`,
    modelId: `fixture-model-${i + 1}`,
    providerKey: i % 2 ? "unknown" : '["codex","fixture-provider"]',
    providerLabel: i % 2 ? null : "Fixture provider",
    toolId: i % 2 ? "claude" : "codex",
    totals: totals(i + 1, i === 7),
    series: Array.from({ length: 7 }, (_, day) => ({
      start: start + day * 86400000,
      end: start + (day + 1) * 86400000,
      totals: {
        ...totals(i + 1, i === 7),
        requests: i + 1,
        inputTokens: (i + 1) * 200,
        outputTokens: (i + 1) * 100,
        cacheReadTokens: (i + 1) * 5000,
        cacheWriteTokens: (i + 1) * 200,
        totalTokensIncludingCache: (i + 1) * 5500,
        estimate: {
          ...totals(i + 1, i === 7).estimate,
          knownCostUsd: i === 7 ? null : (((i + 1) * 0.001) / 7).toFixed(8),
        },
      },
    })),
  }));
  const total = totals((count * (count + 1)) / 2);
  total.estimate = {
    ...total.estimate,
    knownCostUsd: groups
      .reduce((sum, g) => sum + Number(g.totals.estimate.knownCostUsd ?? 0), 0)
      .toFixed(3),
    availability: count >= 8 ? "partial" : "known",
    unpricedModelIds: count >= 8 ? ["fixture-model-8"] : [],
    unpricedRecordCount: count >= 8 ? 56 : 0,
  };
  return {
    granularity: "day",
    totals: total,
    series: Array.from({ length: 7 }, (_, day) => ({
      start: start + day * 86400000,
      end: start + (day + 1) * 86400000,
      totals: {
        ...total,
        requests: total.requests / 7,
        inputTokens: total.inputTokens / 7,
        outputTokens: total.outputTokens / 7,
        cacheReadTokens: total.cacheReadTokens / 7,
        cacheWriteTokens: total.cacheWriteTokens / 7,
        totalTokensIncludingCache: total.totalTokensIncludingCache / 7,
        estimate: {
          ...total.estimate,
          knownCostUsd: (Number(total.estimate.knownCostUsd) / 7).toFixed(8),
        },
      },
    })),
    dimensions: {
      model: { groups, rankings: groups },
      modelProvider: {
        groups: groups.map((g) => ({
          ...g,
          id: JSON.stringify([g.modelId, g.providerKey]),
        })),
        rankings: groups.map((g) => ({
          ...g,
          id: JSON.stringify([g.modelId, g.providerKey]),
        })),
      },
      provider: { groups: [], rankings: [] },
      agent: { groups: [], rankings: [] },
    },
    providers: [
      { key: "unknown", label: null },
      { key: '["codex","fixture-provider"]', label: "Fixture provider" },
    ],
  };
}
