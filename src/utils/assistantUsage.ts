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
