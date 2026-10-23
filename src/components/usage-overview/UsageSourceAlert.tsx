import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { TOOL_META } from "@/config/toolMeta";
import type { UsageOverview } from "@/lib/api/usageOverview";
import type { UsagePricingCoverage } from "@/types/usageQuery";

/** One warning line, shown only when a tool failed to read or prices are missing. */
export function UsageSourceAlert({
  sources,
  pricing,
  onRetry,
}: {
  sources?: UsageOverview["sources"];
  pricing?: UsagePricingCoverage;
  onRetry?: () => void;
}) {
  const { t, i18n } = useTranslation();
  const failed = [
    ...new Set(
      (sources ?? [])
        .filter((source) => source.status === "failed")
        .map((source) => TOOL_META[source.toolId]?.label ?? source.toolId),
    ),
  ];
  const priceUnavailable =
    ["invalid", "unsupported", "missing"].includes(
      pricing?.catalogState ?? "",
    ) && !pricing?.officialCatalogVersion;
  const unpriced = pricing?.unpricedModelIds.length ?? 0;
  const official = pricing?.officialModelIds?.length ?? 0;
  if (!failed.length && !priceUnavailable && !unpriced && !official)
    return null;
  return (
    <div
      role="status"
      className="flex flex-wrap items-center gap-x-3 gap-y-1 text-note text-ink-muted"
    >
      {!!failed.length && (
        <span className="text-warn">
          {t("usageWorkbench.sourcesFailed", { tools: failed.join(" · ") })}
        </span>
      )}
      {priceUnavailable ? (
        <span className="text-warn">
          {t("usageWorkbench.priceUnavailable")}
        </span>
      ) : (
        !!unpriced && (
          <span className="text-warn">
            {t("usageWorkbench.pricePartial", { count: unpriced })}
          </span>
        )
      )}
      {!!official && (
        <span
          title={[
            t("usageWorkbench.officialPriceSource"),
            ...(pricing?.officialCatalogUpdatedAtMs
              ? [
                  new Date(pricing.officialCatalogUpdatedAtMs).toLocaleString(
                    i18n.language,
                  ),
                ]
              : []),
            pricing?.officialModelIds?.join(" · "),
          ]
            .filter(Boolean)
            .join("\n")}
        >
          {t("usageWorkbench.officialFallback", { count: official })}
        </span>
      )}
      {!!failed.length && onRetry && (
        <Button variant="ghost" className="h-6 px-2" onClick={onRetry}>
          {t("common.retry")}
        </Button>
      )}
    </div>
  );
}
