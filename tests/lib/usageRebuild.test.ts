import { beforeEach, expect, it, vi } from "vitest";
import {
  cancelUsageRebuild,
  getUsageRebuildProgress,
} from "@/lib/api/usageOverview";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const progress = {
  operationId: "run-a",
  state: "running",
  completedSources: 0,
  totalSources: 9,
  errorCode: null,
};
beforeEach(() => invoke.mockReset());

it("preserves no active rebuild without inventing progress", async () => {
  invoke.mockResolvedValue(null);
  await expect(getUsageRebuildProgress()).resolves.toBeNull();
});

it.each([
  { ...progress, operationId: "" },
  { ...progress, state: "idle" },
  { ...progress, completedSources: -1 },
  { ...progress, completedSources: 10 },
  { ...progress, totalSources: Number.POSITIVE_INFINITY },
])("rejects an invalid rebuild receipt before presenting it", async (value) => {
  invoke.mockResolvedValue(value);
  await expect(getUsageRebuildProgress()).rejects.toThrow(
    "usage.rebuild.invalidReceipt",
  );
});

it("rejects cancellation attributed to a different operation", async () => {
  invoke.mockResolvedValue({
    ...progress,
    operationId: "run-b",
    state: "cancelling",
  });
  await expect(cancelUsageRebuild("run-a")).rejects.toThrow(
    "usage.rebuild.invalidReceipt",
  );
  expect(invoke).toHaveBeenCalledWith("cancel_usage_rebuild", {
    operationId: "run-a",
  });
});
