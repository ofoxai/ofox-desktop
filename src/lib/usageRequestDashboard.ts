import type { RequestLog } from "@/types/usage";
import type { UsageEstimate, UsageRequestFilters } from "@/types/usageQuery";
import type {
  RequestDashboardGroup,
  RequestDashboardTotals,
  RequestChartDimension,
  RequestDimension,
  RequestMetric,
} from "@/types/usageRequestDashboard";

export const REQUEST_METRICS: RequestMetric[] = ["tokens", "cost", "requests"];
export const REQUEST_DIMENSIONS: RequestChartDimension[] = ["model", "agent"];
export const REQUEST_COLORS = [
  "#4f46e5",
  "#0ea5e9",
  "#10b981",
  "#f59e0b",
  "#8b5cf6",
  "#ec4899",
  "#64748b",
];

export function requestTokenTotal(row: RequestLog) {
  return (
    row.inputTokens +
    row.outputTokens +
    row.cacheReadTokens +
    row.cacheCreationTokens
  );
}

/** Presentation-only combination of already-priced amounts, never a new price. */
export function requestOtherEstimate(
  parts: RequestDashboardTotals[],
): UsageEstimate {
  const known = parts.filter((p) => p.estimate.knownCostUsd != null);
  const uncertain = parts.some(
    (p) =>
      p.estimate.availability === "unknown" ||
      p.estimate.availability === "partial",
  );
  return {
    knownCostUsd:
      parts.length && !known.length
        ? null
        : String(
            known.reduce((sum, p) => sum + Number(p.estimate.knownCostUsd), 0),
          ),
    availability: uncertain ? (known.length ? "partial" : "unknown") : "known",
    unpricedModelIds: [
      ...new Set(parts.flatMap((p) => p.estimate.unpricedModelIds)),
    ],
    unpricedRecordCount: parts.some(
      (p) => p.estimate.unpricedRecordCount == null,
    )
      ? null
      : parts.reduce(
          (sum, p) => sum + (p.estimate.unpricedRecordCount ?? 0),
          0,
        ),
    additionalUnitsUnsupported: parts.some(
      (p) => p.estimate.additionalUnitsUnsupported,
    ),
  };
}
export function requestMetricValue(
  totals: RequestDashboardTotals | undefined,
  metric: RequestMetric,
): number | null {
  if (!totals) return 0;
  if (metric === "tokens") return totals.totalTokensIncludingCache;
  if (metric === "requests") return totals.requests;
  return totals.estimate.knownCostUsd == null
    ? null
    : Number(totals.estimate.knownCostUsd);
}
export function requestGroupSelected(
  dimension: RequestDimension,
  group: RequestDashboardGroup,
  selection: UsageRequestFilters["selection"],
) {
  if (!selection) return false;
  return (
    (dimension === "model" || dimension === "modelProvider"
      ? selection.modelId === group.modelId
      : true) &&
    (dimension === "agent" ? selection.toolId === group.toolId : true) &&
    (dimension === "provider" || dimension === "modelProvider"
      ? selection.providerKey === group.providerKey
      : true)
  );
}
export function toggleRequestGroup(
  dimension: RequestDimension,
  group: RequestDashboardGroup,
  selection: UsageRequestFilters["selection"],
) {
  const clear = requestGroupSelected(dimension, group, selection);
  const next = { ...selection };
  if (dimension === "model" || dimension === "modelProvider")
    next.modelId = clear ? undefined : group.modelId;
  if (dimension === "agent") next.toolId = clear ? undefined : group.toolId;
  if (dimension === "provider" || dimension === "modelProvider")
    next.providerKey = clear ? undefined : group.providerKey;
  return Object.values(next).some((value) => value != null) ? next : undefined;
}
