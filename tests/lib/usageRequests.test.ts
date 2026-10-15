import { beforeEach, expect, it, vi } from "vitest";
import { usageApi } from "@/lib/api/usage";
import { createUsageQuery } from "@/lib/usageWorkbench";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const query = createUsageQuery({ period: "all" }, {}, 1000);
const receipt = {
  appliedQuery: query,
  queryFingerprint: "query-1",
  snapshotVersion: "version-1",
};
const filters = {
  sourceIds: ["session_log"],
  statusCode: 429,
  search: "model",
};
beforeEach(() => invoke.mockReset());
it.each([
  undefined,
  {},
  { sourceIds: ["proxy"], statusCode: 429, search: "model" },
])(
  "rejects missing or different request-specific filter receipts",
  async (requestFilters) => {
    invoke.mockResolvedValue({
      ...receipt,
      snapshotId: "snapshot",
      requestFilters,
      data: [],
      total: 99,
      page: 0,
      pageSize: 20,
    });
    await expect(
      usageApi.getRequestLogs({}, 0, 20, { query, requestFilters: filters }),
    ).rejects.toThrow(/usage.query(Unsupported|Mismatch)/);
  },
);
it("rejects a new projection masquerading as the next frozen page", async () => {
  invoke.mockResolvedValue({
    ...receipt,
    snapshotVersion: "other",
    snapshotId: "snapshot",
    requestFilters: {},
  });
  await expect(
    usageApi.getRequestLogs({}, 1, 20, {
      query,
      snapshotId: "snapshot",
      expectedReceipt: receipt,
    }),
  ).rejects.toThrow("usage.queryMismatch");
});
it("rejects the right request ID from the wrong detail snapshot", async () => {
  invoke.mockResolvedValue({
    requestId: "request",
    receipt: { ...receipt, snapshotVersion: "other" },
  });
  await expect(
    usageApi.getRequestDetail("request", "snapshot", receipt),
  ).rejects.toThrow("usage.queryMismatch");
});
it("rejects CSV from the wrong frozen projection before saving", async () => {
  invoke.mockResolvedValue({
    ...receipt,
    queryFingerprint: "other",
    csv: "wrong",
    recordCount: 1,
  });
  await expect(
    usageApi.exportRequestLogsCsv("snapshot", undefined, receipt),
  ).rejects.toThrow("usage.queryMismatch");
});
it("reports malformed shared receipts as a query mismatch", async () => {
  invoke.mockResolvedValue({
    ...receipt,
    appliedQuery: { ...query, toolIds: null },
    snapshotId: "snapshot",
    requestFilters: {},
  });
  await expect(usageApi.getRequestLogs({}, 0, 20, { query })).rejects.toThrow(
    "usage.queryMismatch",
  );
});
