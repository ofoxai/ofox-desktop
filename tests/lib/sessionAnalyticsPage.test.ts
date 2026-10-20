import { beforeEach, expect, it, vi } from "vitest";
import { getSessionAnalyticsPage } from "@/lib/api/usageOverview";
import { sessionAnalyticsFixture } from "../components/usage/sessionAnalyticsFixture";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
beforeEach(() => invoke.mockReset());
const fixture = () => ({
  ...sessionAnalyticsFixture(),
  analyticsSnapshotId: "snapshot-one",
  data: [],
  total: 0,
  page: 0,
  pageSize: 25,
  appliedView: { sortBy: "cost", search: "mixed case", managerKey: "manager" },
  viewFingerprint: "view",
});
const options = {
  analyticsSnapshotId: "snapshot-one",
  expectedReceipt: sessionAnalyticsFixture(),
  sortBy: "cost" as const,
  page: 0,
  pageSize: 25,
  search: " Mixed Case ",
  managerKey: "manager",
};
it("accepts the backend's normalized case-insensitive list search without changing the scope receipt", async () => {
  invoke.mockResolvedValue(fixture());
  await expect(getSessionAnalyticsPage(options)).resolves.toMatchObject({
    total: 0,
  });
});
it("rejects a different frozen snapshot even when its query and data receipts match", async () => {
  invoke.mockResolvedValue({
    ...fixture(),
    analyticsSnapshotId: "snapshot-other",
    appliedView: { sortBy: "cost", search: null, managerKey: "manager" },
  });
  await expect(
    getSessionAnalyticsPage({ ...options, search: undefined }),
  ).rejects.toThrow();
});
it("rejects silently ignored session targeting", async () => {
  invoke.mockResolvedValue({
    ...fixture(),
    appliedView: { sortBy: "cost", search: null, managerKey: null },
  });
  await expect(
    getSessionAnalyticsPage({ ...options, search: undefined }),
  ).rejects.toThrow("usage.queryMismatch");
});
it("sends only the two receipt identifiers instead of serializing the entire analytics response", async () => {
  invoke.mockResolvedValue({
    ...fixture(),
    appliedView: { sortBy: "cost", search: null, managerKey: "manager" },
  });
  await getSessionAnalyticsPage({ ...options, search: undefined });
  expect(invoke.mock.calls[0][1].expectedReceipt).toEqual({
    queryFingerprint: "analysis-query",
    snapshotVersion: "analysis-v1",
  });
});
