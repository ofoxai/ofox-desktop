import { beforeEach, expect, it, vi } from "vitest";
import {
  getUsageOverview,
  getSessionUsageAnalytics,
} from "@/lib/api/usageOverview";
import { usageApi } from "@/lib/api/usage";
import { createUsageQuery } from "@/lib/usageWorkbench";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const query = createUsageQuery({ period: "all" }, {}, 1000);
const receipt = {
  appliedQuery: query,
  queryFingerprint: "same-query",
  snapshotVersion: "same-input",
  snapshotId: "frozen-requests",
  requestFilters: {},
};
beforeEach(() => invoke.mockReset());
const readers = [
  () => getUsageOverview("all", query, "ofox.ai"),
  () => getSessionUsageAnalytics(query, "ofox.ai"),
  () => usageApi.getRequestLogs({}, 0, 20, { query, expectedApex: "ofox.ai" }),
];
it.each(readers)(
  "rejects the first otherwise valid response priced in a different captured apex",
  async (read) => {
    invoke.mockResolvedValue({
      ...receipt,
      pricingCoverage: { apex: "ofox.io" },
    });
    await expect(read()).rejects.toThrow("usage.queryMismatch");
  },
);
it.each(readers)(
  "preserves older backend receipts without a price-region field",
  async (read) => {
    invoke.mockResolvedValue(receipt);
    await expect(read()).resolves.toMatchObject(receipt);
  },
);
it("keeps the frontend expected apex out of the product IPC payload", async () => {
  invoke.mockResolvedValue({
    ...receipt,
    pricingCoverage: { apex: "ofox.ai" },
  });
  await readers[2]();
  expect(invoke).toHaveBeenCalledWith("get_request_logs", {
    filters: {},
    page: 0,
    pageSize: 20,
    query,
  });
});
