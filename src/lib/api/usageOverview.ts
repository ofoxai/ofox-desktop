import { invoke } from "@tauri-apps/api/core";

import type { OfoxSpending } from "./ofoxAuth";

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

export interface UsageOverview {
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

/** Everything the usage page shows for `period`; syncs the session sources first. */
export async function getUsageOverview(
  period: UsagePeriod,
): Promise<UsageOverview> {
  return invoke<UsageOverview>("get_usage_overview", { period });
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
