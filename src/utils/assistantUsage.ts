import type { TFunction } from "i18next";
import type { AssistantMessage, AssistantUsage } from "@/lib/api/assistant";
import { fmtCost, fmtTokens } from "@/lib/format";

/**
 * Tokens and cost of a conversation or of one reply. The cost is shown only
 * once its runs are saved (`settled`), and as unavailable whenever a request's
 * cost is unknown: never as $0 or as a partial sum. A cost that includes web
 * searches says how many search charges it holds (one search can bill two).
 */
export function usageSummary(
  usage: AssistantUsage | undefined,
  settled: boolean,
  t: TFunction,
): string | null {
  if (!usage) return null;
  const input = usage.inputTokens ?? 0;
  const output = usage.outputTokens ?? 0;
  const counted = input > 0 || output > 0;
  const parts = counted
    ? [
        t("assistant.usageTokens", {
          input: fmtTokens(input),
          output: fmtTokens(output),
        }),
      ]
    : [];
  if (settled) {
    if (usage.usageCoverage === "incomplete")
      parts.push(t("assistant.costUnavailable"));
    else if (typeof usage.estimatedCostUsd === "string") {
      const cost = fmtCost(Number(usage.estimatedCostUsd));
      const searches = usage.searchCount ?? 0;
      parts.push(
        searches > 0
          ? t("assistant.usageCostWithSearches", { cost, count: searches })
          : t("assistant.usageCost", { cost }),
      );
    }
    // Requests were made but a model had no price.
    else if (counted || (usage.modelRequests ?? 0) > 0)
      parts.push(t("assistant.costUnavailable"));
  }
  return parts.length ? parts.join(" · ") : null;
}

export type UsageLineKey = "input" | "cacheRead" | "cacheWrite" | "output";

export interface UsageBreakdown {
  /** Lines with tokens, in ofox-agent's order; `usd` null when unknown. */
  lines: { key: UsageLineKey; tokens: number; usd: number | null }[];
  search: { count: number; usd: number | null } | null;
  /**
   * With generations: the chat model's part (tokens and searches), apart
   * from what each image, video or speech cost. Null without generations.
   */
  chat: number | null | undefined;
  generations: {
    key: string;
    kind: string;
    modelId: string | null;
    usd: number | null;
  }[];
  /** The reply's estimated total; null when any request's cost is unknown. */
  total: number | null;
}

/**
 * A reply's cost line by line, like ofox-agent: each request's tokens priced
 * at the price frozen when it was sent. Still an estimate, not the charge.
 */
export function usageBreakdown(usage: AssistantUsage): UsageBreakdown {
  const total =
    usage.usageCoverage !== "incomplete" &&
    typeof usage.estimatedCostUsd === "string"
      ? Number(usage.estimatedCostUsd)
      : null;
  const tokens: Record<UsageLineKey, number> = {
    input: usage.inputTokens ?? 0,
    cacheRead: usage.cacheReadTokens ?? 0,
    cacheWrite: usage.cacheWriteTokens ?? 0,
    output: usage.outputTokens ?? 0,
  };
  const generated = (usage.requests ?? []).filter(
    (request) => typeof request.toolCallId === "string",
  );
  // The chat model's own cost: the reply's total when nothing was generated.
  const chat = generated.length
    ? typeof usage.chatCostUsd === "string"
      ? Number(usage.chatCostUsd)
      : null
    : total;
  const priced = (key: UsageLineKey): number | null => {
    if (chat === null) return null;
    const amounts = (usage.requests ?? [])
      .map((request) => {
        const cost = (request.usage as { cost?: Record<string, unknown> })
          ?.cost;
        return typeof cost?.[key] === "number" ? cost[key] : null;
      })
      .filter((amount): amount is number => amount !== null);
    return amounts.length
      ? amounts.reduce((sum, amount) => sum + amount, 0)
      : null;
  };
  const searches = usage.searchCount ?? 0;
  return {
    lines: (["input", "cacheRead", "cacheWrite", "output"] as const)
      .filter((key) => tokens[key] > 0)
      .map((key) => ({ key, tokens: tokens[key], usd: priced(key) })),
    search:
      searches > 0
        ? {
            count: searches,
            usd:
              chat !== null && typeof usage.searchCostUsd === "string"
                ? Number(usage.searchCostUsd)
                : null,
          }
        : null,
    chat: generated.length ? chat : undefined,
    generations: generated.map((request) => ({
      key: request.toolCallId!,
      kind: request.kind,
      modelId: request.modelId ?? null,
      usd:
        typeof request.estimatedCostUsd === "string"
          ? Number(request.estimatedCostUsd)
          : null,
    })),
    total,
  };
}

/** Small amounts keep their digits, or a reply of a few mils shows as $0.00. */
export function fmtUsd(usd: number | null | undefined): string {
  if (usd == null || !Number.isFinite(usd)) return "—";
  if (usd === 0) return "$0.00";
  if (usd < 0.001) return `$${usd.toFixed(6)}`;
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  return `$${usd.toFixed(2)}`;
}

/**
 * How long a reply took, in ms: from when its run began to when it ended.
 * Null for a reply that has no end saved (older ones, or one still going).
 */
export function replyDuration(
  message: Pick<AssistantMessage, "createdAt" | "completedAt">,
): number | null {
  const end = message.completedAt;
  return typeof end === "number" && end >= message.createdAt
    ? end - message.createdAt
    : null;
}

/**
 * A duration in the language's own units: a decimal for the first ten
 * seconds, whole seconds up to a minute, then minutes and seconds. Under a
 * second is "<1": a reply never took no time.
 */
export function fmtDuration(ms: number, locale: string): string {
  const unit = (value: number, name: "second" | "minute") =>
    new Intl.NumberFormat(locale, {
      style: "unit",
      unit: name,
      unitDisplay: "short",
      maximumFractionDigits: 1,
    }).format(value);
  const seconds = ms / 1000;
  if (seconds < 1) return `<${unit(1, "second")}`;
  if (seconds < 10) return unit(Math.round(seconds * 10) / 10, "second");
  const whole = Math.round(seconds);
  if (whole < 60) return unit(whole, "second");
  const minutes = Math.floor(whole / 60);
  const rest = whole % 60;
  return rest === 0
    ? unit(minutes, "minute")
    : `${unit(minutes, "minute")} ${unit(rest, "second")}`;
}
