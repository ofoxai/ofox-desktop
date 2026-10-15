import { beforeEach, expect, it, vi } from "vitest";
import { getUsageSourceSummary } from "@/lib/api/usageLifetime";
import { lifetimeFixture } from "../components/usage/lifetimeFixture";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
beforeEach(() => invoke.mockReset());

it("requests cumulative facts without time, model or project filters", async () => {
  const data = lifetimeFixture();
  invoke.mockResolvedValue(data);
  await expect(getUsageSourceSummary("hermes", "ofox.ai")).resolves.toEqual(
    data,
  );
  expect(invoke).toHaveBeenCalledWith("get_usage_source_summary", {
    toolId: "hermes",
  });
});

it.each([
  { version: 2 },
  { toolId: "codex" },
  { scope: "eventTime" },
  { sourceId: "session_log" },
  {
    pricingCoverage: { ...lifetimeFixture().pricingCoverage, apex: "ofox.io" },
  },
  { byModel: null },
  { totals: { ...lifetimeFixture().totals, totalTokens: -1 } },
  { totals: { ...lifetimeFixture().totals, inputTokens: 2 ** 53 } },
  { totals: { ...lifetimeFixture().totals, outputTokens: "10" } },
  { counterCoverage: "ready" },
])("rejects incompatible or unsafe lifetime receipts: %j", async (change) => {
  invoke.mockResolvedValue({ ...lifetimeFixture(), ...change });
  await expect(getUsageSourceSummary("hermes", "ofox.ai")).rejects.toThrow(
    "usage.lifetime.invalidResponse",
  );
});

it("preserves unknown buckets and missing source separately from zero", async () => {
  const data = lifetimeFixture();
  data.sourceState = "missing";
  data.stale = true;
  data.totals.cacheReadTokens = null;
  data.totals.amountKind = "lowerBound";
  invoke.mockResolvedValue(data);
  const result = await getUsageSourceSummary("hermes", "ofox.ai");
  expect(result.totals.cacheReadTokens).toBeNull();
  expect(result.stale).toBe(true);
});

it("checks each model bucket instead of only validating aggregate totals", async () => {
  const data = lifetimeFixture();
  data.byModel[0].buckets.totalTokens = Number.POSITIVE_INFINITY;
  invoke.mockResolvedValue(data);
  await expect(getUsageSourceSummary("hermes", "ofox.ai")).rejects.toThrow(
    "usage.lifetime.invalidResponse",
  );
});
