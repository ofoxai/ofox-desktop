import { invoke } from "@tauri-apps/api/core";
import type { UsageLifetimeSummary } from "@/types/usageLifetime";

export async function getUsageSourceSummary(
  toolId: "hermes",
  apex: string,
): Promise<UsageLifetimeSummary> {
  const result: unknown = await invoke("get_usage_source_summary", { toolId });
  if (!validSummary(result, toolId, apex))
    throw new Error("usage.lifetime.invalidResponse");
  return result;
}

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function strings(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((v) => typeof v === "string");
}
function count(value: unknown): boolean {
  return value === null || (Number.isSafeInteger(value) && Number(value) >= 0);
}
function time(value: unknown): boolean {
  return value === null || Number.isSafeInteger(value);
}
function member(value: unknown, values: readonly string[]): boolean {
  return typeof value === "string" && values.includes(value);
}
function buckets(value: unknown): boolean {
  return (
    record(value) &&
    [
      "inputTokens",
      "outputTokens",
      "cacheReadTokens",
      "cacheWriteTokens",
      "totalTokens",
      "apiCallCount",
    ].every((key) => count(value[key])) &&
    member(value.amountKind, ["known", "lowerBound", "unknown"])
  );
}
function estimate(value: unknown): boolean {
  return (
    record(value) &&
    (value.knownCostUsd === null ||
      (typeof value.knownCostUsd === "string" &&
        /^\d+(\.\d+)?$/.test(value.knownCostUsd))) &&
    member(value.availability, ["known", "free", "partial", "unknown"]) &&
    strings(value.unpricedModelIds) &&
    count(value.unpricedRecordCount) &&
    typeof value.additionalUnitsUnsupported === "boolean"
  );
}
function validSummary(
  value: unknown,
  toolId: "hermes",
  apex: string,
): value is UsageLifetimeSummary {
  if (!record(value) || !record(value.pricingCoverage)) return false;
  const price = value.pricingCoverage;
  return (
    value.version === 1 &&
    value.toolId === toolId &&
    value.sourceId === "hermes-lifetime" &&
    value.scope === "lifetime" &&
    (value.snapshotVersion === null ||
      (typeof value.snapshotVersion === "string" &&
        value.snapshotVersion.length > 0)) &&
    Number.isSafeInteger(value.observedAtMs) &&
    time(value.lastSuccessAtMs) &&
    member(value.sourceState, ["ready", "missing", "error"]) &&
    typeof value.stale === "boolean" &&
    strings(value.reasons) &&
    count(value.sessionRecords) &&
    buckets(value.totals) &&
    Array.isArray(value.byModel) &&
    value.byModel.every(
      (row) =>
        record(row) &&
        (row.modelId === null || typeof row.modelId === "string") &&
        member(row.category, ["main", "voice", "auxiliary", "unattributed"]) &&
        buckets(row.buckets) &&
        estimate(row.estimate),
    ) &&
    member(value.counterCoverage, ["complete", "partial", "unknown"]) &&
    member(value.modelCoverage, ["complete", "partial", "unknown"]) &&
    estimate(value.estimate) &&
    price.policy === "current-ofox-base-v1" &&
    price.apex === apex &&
    (price.catalogVersion === null ||
      typeof price.catalogVersion === "string") &&
    time(price.catalogUpdatedAtMs) &&
    typeof price.stale === "boolean" &&
    strings(price.unpricedModelIds) &&
    count(price.unpricedRecordCount) &&
    typeof price.additionalUnitsUnsupported === "boolean" &&
    (price.catalogState === undefined ||
      member(price.catalogState, [
        "missing",
        "ready",
        "invalid",
        "unsupported",
      ]))
  );
}
