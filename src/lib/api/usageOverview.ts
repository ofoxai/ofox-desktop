import { invoke } from "@tauri-apps/api/core";

import type { OfoxSpending } from "./ofoxAuth";
import type {
  UsageCapabilities,
  UsageEstimate,
  UsageQueryV1,
  UsageReceipt,
  UsageSourceHealth,
  UsageFilterOptions,
  UsageAttribution,
  UsageRebuildProgress,
} from "@/types/usageQuery";
import type {
  AnalyticsPage,
  AnalyticsSort,
  SessionAnalytics,
} from "@/types/sessionAnalytics";
import {
  assertSnapshotReceipt,
  assertUsageReceipt,
  assertUsageApex,
} from "@/lib/usageWorkbench";
import type { UsageSnapshotReceipt } from "@/types/usageQuery";

/**
 * Usage overview and balance trend (Rust: `commands/usage_overview.rs`,
 * `services/usage_overview.rs`, `services/balance_history.rs`).
 *
 * Every timestamp here is epoch milliseconds. Costs are USD estimates (tokens
 * priced at OFox's prices), not the bill.
 */

export type UsagePeriod = "today" | "7d" | "30d" | "all";

/** Bucket size of `series`: hours for today, days, or weeks for a long `all`. */
export type UsageGranularity = "hour" | "day" | "week";

/**
 * A sum of calls. `totalTokens` is input + output (cache traffic is counted
 * apart); `cacheHitRate` (0..1) is cache reads over input + cache reads +
 * cache writes, 0 when there were none.
 */
export interface UsageTotals {
  requests: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  cacheHitRate: number;
  costUsd: number;
  estimate?: UsageEstimate;
}

/** One bucket of the series; `start` is when it begins (local midnight / hour). */
export interface UsageSeriesPoint {
  start: number;
  requests: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  costUsd: number;
  estimate?: UsageEstimate;
}

/** One tool (`appType`: claude, codex, gemini, opencode, ...). */
export interface ToolUsage extends UsageTotals {
  appType: string;
}

/**
 * One model. `priced` is false when no call of it got a price, so its cost is
 * unknown (show "—", not $0). `appTypes` are the tools that used it, heaviest
 * first.
 */
export interface ModelUsage extends UsageTotals {
  model: string;
  appTypes: string[];
  priced: boolean;
}

export interface ProjectUsage extends UsageTotals {
  id: string;
  label: string;
  path: string | null;
}

export interface UsageOverview extends Partial<UsageReceipt> {
  period: UsagePeriod;
  /** `null` for `all`. */
  start: number | null;
  /** Now. */
  end: number;
  granularity: UsageGranularity;
  totals: UsageTotals;
  series: UsageSeriesPoint[];
  /** Largest first (by input + output tokens). */
  byTool: ToolUsage[];
  /** Largest first (by input + output tokens). */
  byModel: ModelUsage[];
  /** When the last usage sync finished; `null` before the first of this run. */
  lastSyncedAt: number | null;
  /** Bound tools whose usage is not read yet (e.g. openclaw, hermes, pi, dsh). */
  unsupportedTools: string[];
  options?: UsageFilterOptions;
  sources?: UsageSourceHealth[];
  byProject?: ProjectUsage[];
  attribution?: UsageAttribution;
}

/** One reading of the OFox balance: `amount` USD was left at `at`. */
export interface BalancePoint {
  at: number;
  amount: number;
}

/**
 * The balance of the last 14 days (`points` is empty with fewer than two) and
 * the pace it is being spent at. `perDay`, `runsOutAt` and `daysLeft` are
 * `null` without enough points (3 points over at least an hour since the last
 * top-up); `perDay` is 0 for a balance that is not being spent, and only a
 * balance that is being spent and runs out within a year has `runsOutAt` and
 * `daysLeft`.
 */
export interface BalanceTrend {
  points: BalancePoint[];
  perDay: number | null;
  runsOutAt: number | null;
  daysLeft: number | null;
}

/** Query mode reads a frozen local projection; the legacy period call syncs first. */
export async function getUsageOverview(
  period: UsagePeriod,
  query?: UsageQueryV1,
  expectedApex?: string,
): Promise<UsageOverview> {
  const response = await invoke<UsageOverview>("get_usage_overview", {
    period,
    ...(query ? { query } : {}),
  });
  if (query) assertUsageReceipt(response, query);
  assertUsageApex(response, expectedApex);
  return response;
}

export function getUsageCapabilities(): Promise<UsageCapabilities> {
  return invoke("get_usage_capabilities");
}

function rebuildProgress(value: unknown): UsageRebuildProgress {
  const progress = value as UsageRebuildProgress | null;
  if (
    !progress ||
    typeof progress.operationId !== "string" ||
    !progress.operationId ||
    !["running", "cancelling", "cancelled", "completed", "failed"].includes(
      progress.state,
    ) ||
    !Number.isSafeInteger(progress.completedSources) ||
    progress.completedSources < 0 ||
    !Number.isSafeInteger(progress.totalSources) ||
    progress.totalSources < progress.completedSources
  )
    throw new Error("usage.rebuild.invalidReceipt");
  return progress;
}

export async function getUsageRebuildProgress(): Promise<UsageRebuildProgress | null> {
  const value = await invoke<unknown>("get_usage_rebuild_progress");
  return value == null ? null : rebuildProgress(value);
}

export async function cancelUsageRebuild(
  operationId: string,
): Promise<UsageRebuildProgress> {
  const value = rebuildProgress(
    await invoke<unknown>("cancel_usage_rebuild", { operationId }),
  );
  if (value.operationId !== operationId)
    throw new Error("usage.rebuild.invalidReceipt");
  return value;
}

export async function getSessionUsageAnalytics(
  query: UsageQueryV1,
  expectedApex?: string,
): Promise<SessionAnalytics> {
  const response = await invoke<SessionAnalytics>(
    "get_session_usage_analytics",
    {
      query,
    },
  );
  assertUsageReceipt(response, query);
  assertUsageApex(response, expectedApex);
  return response;
}

export async function getSessionAnalyticsPage(options: {
  analyticsSnapshotId: string;
  expectedReceipt: UsageSnapshotReceipt;
  sortBy: AnalyticsSort;
  page: number;
  pageSize: number;
  search?: string;
  managerKey?: string;
}): Promise<AnalyticsPage> {
  const response = await invoke<AnalyticsPage>("get_session_analytics_page", {
    ...options,
    expectedReceipt: {
      queryFingerprint: options.expectedReceipt.queryFingerprint,
      snapshotVersion: options.expectedReceipt.snapshotVersion,
    },
  });
  assertSnapshotReceipt(response, options.expectedReceipt);
  if (
    response.analyticsSnapshotId !== options.analyticsSnapshotId ||
    response.page !== options.page ||
    response.pageSize !== options.pageSize ||
    response.appliedView.sortBy !== options.sortBy ||
    (response.appliedView.search ?? "") !==
      (options.search?.trim().toLowerCase() ?? "") ||
    (response.appliedView.managerKey ?? null) !== (options.managerKey ?? null)
  )
    throw new Error("usage.queryMismatch");
  return response;
}

/** The signed-in account's balance history and run-out estimate; empty when signed out. */
export async function getBalanceTrend(): Promise<BalanceTrend> {
  return invoke<BalanceTrend>("ofox_get_balance_trend");
}

/**
 * The org's monthly spending limit; `null` when signed out or no monthly limit
 * is set, `forbidden: true` for a member role.
 */
export async function getSpending(): Promise<OfoxSpending | null> {
  return invoke<OfoxSpending | null>("ofox_get_spending");
}
