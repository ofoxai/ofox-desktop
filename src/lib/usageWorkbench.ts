import type { UsagePeriod } from "@/lib/api/usageOverview";
import type {
  UsageQueryV1,
  UsageReceipt,
  UsageRequestFilters,
  UsageSnapshotReceipt,
} from "@/types/usageQuery";

export type WorkbenchPeriod = UsagePeriod | "90d" | "custom";

/** Only an explicit backend receipt can make zero-valued usage unavailable. */
export function usageIsUnavailable(
  receipt?: Partial<UsageReceipt> | null,
): boolean {
  return receipt?.sourceCoverage?.usageAvailability === "unavailable";
}

/** Validate the region captured before a read; legacy receipts may omit it. */
export function assertUsageApex(
  response: Partial<UsageReceipt>,
  expectedApex?: string,
) {
  if (
    expectedApex != null &&
    response.pricingCoverage?.apex != null &&
    response.pricingCoverage.apex !== expectedApex
  )
    throw new Error("usage.queryMismatch");
}
export interface WorkbenchRange {
  period: WorkbenchPeriod;
  startDate?: string;
  endDate?: string;
}
export type WorkbenchFilters = Partial<
  Pick<UsageQueryV1, "toolIds" | "modelIds" | "projectIds" | "sourceIds">
>;

function calendarDate(value: string | undefined): Date {
  const match = value?.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!match) throw new Error("usage.invalidRange");
  const [, y, m, d] = match.map(Number);
  const date = new Date(y, m - 1, d);
  if (
    date.getFullYear() !== y ||
    date.getMonth() !== m - 1 ||
    date.getDate() !== d
  )
    throw new Error("usage.invalidRange");
  return date;
}

export function resolveWorkbenchRange(
  range: WorkbenchRange,
  nowMs = Date.now(),
): Pick<UsageQueryV1, "startMs" | "endMs"> {
  if (!Number.isSafeInteger(nowMs)) throw new Error("usage.invalidRange");
  if (range.period === "custom") {
    const start = calendarDate(range.startDate);
    const end = calendarDate(range.endDate);
    if (start > end) throw new Error("usage.invalidRange");
    end.setDate(end.getDate() + 1);
    return { startMs: start.getTime(), endMs: end.getTime() };
  }
  if (range.period === "all") return { startMs: null, endMs: nowMs };
  const start = new Date(nowMs);
  start.setHours(0, 0, 0, 0);
  const count = range.period === "today" ? 1 : Number.parseInt(range.period);
  start.setDate(start.getDate() - count + 1);
  return { startMs: start.getTime(), endMs: nowMs };
}

export function createUsageQuery(
  range: WorkbenchRange,
  filters: WorkbenchFilters = {},
  nowMs = Date.now(),
): UsageQueryV1 {
  const ids = (values?: string[]) => [...new Set(values ?? [])].sort();
  return {
    version: 1,
    ...resolveWorkbenchRange(range, nowMs),
    toolIds: ids(filters.toolIds),
    modelIds: ids(filters.modelIds),
    projectIds: ids(filters.projectIds),
    sourceIds: ids(filters.sourceIds),
  };
}

export function sameUsageQuery(a: unknown, b: unknown): boolean {
  const valid = (value: unknown): value is UsageQueryV1 => {
    if (!value || typeof value !== "object") return false;
    const query = value as UsageQueryV1;
    return (
      query.version === 1 &&
      (query.startMs === null || Number.isSafeInteger(query.startMs)) &&
      Number.isSafeInteger(query.endMs) &&
      (["toolIds", "modelIds", "projectIds", "sourceIds"] as const).every(
        (key) =>
          Array.isArray(query[key]) &&
          query[key].every((id) => typeof id === "string"),
      )
    );
  };
  if (!valid(a) || !valid(b)) return false;
  return (
    a.version === b.version &&
    a.startMs === b.startMs &&
    a.endMs === b.endMs &&
    (["toolIds", "modelIds", "projectIds", "sourceIds"] as const).every(
      (key) => {
        const left = [...a[key]].sort();
        const right = [...b[key]].sort();
        return (
          left.length === right.length &&
          left.every((id, index) => id === right[index])
        );
      },
    )
  );
}

export function normalizedRequestFilters(value: UsageRequestFilters = {}) {
  return {
    recordBasis: value.recordBasis ?? "effective",
    sourceIds: [...new Set(value.sourceIds ?? [])].sort(),
    statusCode: value.statusCode ?? null,
    statusKind: value.statusKind ?? null,
    search: value.search?.trim() || null,
    providerKey: value.providerKey ?? null,
    statusClass: value.statusClass ?? null,
    selection: {
      modelId: value.selection?.modelId ?? null,
      toolId: value.selection?.toolId ?? null,
      providerKey: value.selection?.providerKey ?? null,
    },
    dayStartMs: value.dayStartMs ?? null,
  };
}

export function sameRequestFilters(
  actual: unknown,
  expected: UsageRequestFilters = {},
) {
  if (!actual || typeof actual !== "object") return false;
  const value = actual as UsageRequestFilters;
  if (
    (value.providerKey != null && typeof value.providerKey !== "string") ||
    (value.dayStartMs != null && !Number.isSafeInteger(value.dayStartMs)) ||
    (value.statusClass != null &&
      value.statusClass !== "failed" &&
      value.statusClass !== "succeeded") ||
    (value.selection != null &&
      (typeof value.selection !== "object" ||
        Array.isArray(value.selection) ||
        ["modelId", "toolId", "providerKey"].some((key) => {
          const id = (value.selection as Record<string, unknown>)[key];
          return id != null && typeof id !== "string";
        })))
  )
    return false;
  if (
    value.sourceIds != null &&
    (!Array.isArray(value.sourceIds) ||
      value.sourceIds.some((id) => typeof id !== "string"))
  )
    return false;
  if (value.search != null && typeof value.search !== "string") return false;
  if (
    value.recordBasis != null &&
    value.recordBasis !== "effective" &&
    value.recordBasis !== "savedHistory"
  )
    return false;
  return (
    JSON.stringify(normalizedRequestFilters(value)) ===
    JSON.stringify(normalizedRequestFilters(expected))
  );
}

export function assertSnapshotReceipt(
  actual: Partial<UsageSnapshotReceipt> | undefined,
  expected?: UsageSnapshotReceipt,
) {
  if (
    typeof actual?.queryFingerprint !== "string" ||
    !actual.queryFingerprint ||
    typeof actual.snapshotVersion !== "string" ||
    !actual.snapshotVersion
  )
    throw new Error("usage.queryUnsupported");
  if (
    expected &&
    (actual.queryFingerprint !== expected.queryFingerprint ||
      actual.snapshotVersion !== expected.snapshotVersion)
  )
    throw new Error("usage.queryMismatch");
}

export function legacyPeriod(period: WorkbenchPeriod): UsagePeriod {
  return period === "custom" || period === "90d" ? "all" : period;
}

export function assertUsageReceipt(
  response: Partial<UsageReceipt>,
  query: UsageQueryV1,
): asserts response is UsageReceipt {
  if (
    !response.appliedQuery ||
    typeof response.queryFingerprint !== "string" ||
    !response.queryFingerprint ||
    typeof response.snapshotVersion !== "string" ||
    !response.snapshotVersion
  )
    throw new Error("usage.queryUnsupported");
  if (!sameUsageQuery(response.appliedQuery, query))
    throw new Error("usage.queryMismatch");
}
