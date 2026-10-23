import { useTranslation } from "react-i18next";
import type { SessionUsage, SessionUsageDisplay } from "@/lib/api/sessions";
import { fmtCost, fmtTokenCount, fmtTokens } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";

export function sessionUsageUnavailableReason(display?: SessionUsageDisplay) {
  const reasons = display?.reasons ?? [];
  if (reasons.includes("nativeSessionNotAssociated"))
    return "sessionCenter.usageNativeNotAssociated";
  if (reasons.includes("pricingContextChanged"))
    return "sessionCenter.usagePricingChanged";
  if (reasons.includes("ambiguousThreadHistory"))
    return "sessionCenter.usageAmbiguous";
  if (reasons.includes("conflictingThreadSources"))
    return "sessionCenter.usageConflicting";
  if (reasons.includes("threadUsageSourceChanged"))
    return "sessionCenter.usageSourceChanged";
  if (reasons.includes("threadUsageNotIndexed"))
    return "sessionCenter.usageNotIndexed";
  return "sessionCenter.usageUnavailableDetails";
}

export function sessionCostState(
  usage?: SessionUsage,
  display?: SessionUsageDisplay,
) {
  if (!usage || display?.state === "unavailable") return "unavailable";
  if (display) {
    if (
      !display.estimate ||
      display.estimate.knownCostUsd == null ||
      display.estimate.availability === "unknown"
    )
      return "unpriced";
    return display.estimate.availability === "partial" ? "partial" : "priced";
  }
  const hasKnownModel = usage.models.some((model) => model.costUsd != null);
  if (!hasKnownModel && (usage.models.length > 0 || usage.unpricedModels > 0))
    return "unpriced";
  return usage.unpricedModels > 0 ? "partial" : "priced";
}

export function SessionUsageSummary({
  usage,
  display,
  stacked = false,
}: {
  usage?: SessionUsage;
  display?: SessionUsageDisplay;
  stacked?: boolean;
}) {
  const { t } = useTranslation();
  const state = sessionCostState(usage, display);
  if (!usage || state === "unavailable")
    return (
      <span
        className="shrink-0 text-note text-ink-faint"
        title={t(sessionUsageUnavailableReason(display))}
        aria-label={t("sessionCenter.usageUnavailable")}
      >
        -
      </span>
    );
  const costHint = [
    t(
      state === "unpriced"
        ? "sessionCenter.unpricedDetails"
        : state === "partial"
          ? "sessionCenter.partialCostDetails"
          : display
            ? "sessionCenter.nativeThreadEstimate"
            : "sessionCenter.estimated",
    ),
    ...(display?.estimate?.officialModelIds?.length
      ? [t("usageWorkbench.officialPriceSource")]
      : []),
  ].join(" · ");
  // fresh input and output; what was read from the cache is said apart
  const split = [
    t("sessionCenter.tokenLine", {
      input: fmtTokenCount(usage.inputTokens),
      output: fmtTokenCount(usage.outputTokens),
    }),
    usage.cacheReadTokens > 0
      ? t("sessionCenter.cachedTokens", {
          tokens: fmtTokenCount(usage.cacheReadTokens),
        })
      : "",
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <div
      className={
        stacked
          ? "shrink-0 text-right tabular-nums"
          : "flex shrink-0 items-center gap-3 text-right tabular-nums"
      }
    >
      <div
        title={t("sessionCenter.tokenDetails", {
          input: fmtTokens(usage.inputTokens),
          output: fmtTokens(usage.outputTokens),
          cache: fmtTokens(usage.cacheReadTokens),
          write: fmtTokens(usage.cacheWriteTokens),
        })}
      >
        <b className="block text-ui text-ink">
          {fmtTokenCount(usage.totalTokens)}
        </b>
        <span className="hidden whitespace-nowrap text-note text-ink-faint min-[560px]:block">
          {split}
        </span>
      </div>
      <span
        className={`block min-w-[58px] text-sub ${state === "unpriced" ? "text-ink-faint" : "text-ok"}`}
        title={costHint}
      >
        {state === "unpriced"
          ? t("sessionCenter.unpriced")
          : display?.estimate
            ? formatUsageEstimate(display.estimate)
            : `${fmtCost(usage.costUsd)}${state === "partial" ? "+" : ""}`}
      </span>
    </div>
  );
}
