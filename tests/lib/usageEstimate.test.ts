import { expect, it } from "vitest";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import type { UsageEstimate } from "@/types/usageQuery";
const estimate = (
  availability: UsageEstimate["availability"],
  knownCostUsd: string | null,
): UsageEstimate => ({
  availability,
  knownCostUsd,
  unpricedModelIds: [],
  unpricedRecordCount: 0,
  additionalUnitsUnsupported: false,
});
it("preserves unknown and partial estimates separately from explicit free usage", () => {
  expect(formatUsageEstimate(estimate("unknown", null))).toBe("—");
  expect(formatUsageEstimate(estimate("free", "0"))).toBe("≈$0");
  expect(formatUsageEstimate(estimate("partial", "0.2"))).toBe("≈$0.200+");
  expect(formatUsageEstimate(estimate("known", "0.000000001"))).toBe(
    "≈$1.00e-9",
  );
});
