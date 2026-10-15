import type { UsageEstimate } from "@/types/usageQuery";
import { fmtCost } from "@/lib/format";

export function formatUsageEstimate(
  estimate?: UsageEstimate,
  legacy?: number | null,
): string {
  if (!estimate) return fmtCost(legacy);
  if (estimate.availability === "unknown" || estimate.knownCostUsd == null)
    return "—";
  const value = Number(estimate.knownCostUsd);
  if (!Number.isFinite(value)) return "—";
  const cost =
    value === 0
      ? "≈$0"
      : value > 0 && value < 0.001
        ? `≈$${value.toExponential(2)}`
        : fmtCost(value);
  return estimate.availability === "partial" ? `${cost}+` : cost;
}
