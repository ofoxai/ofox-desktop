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
  const { t } = useTranslation();
  const failed = [
    ...new Set(
      (sources ?? [])
        .filter((source) => source.status === "failed")
        .map((source) => TOOL_META[source.toolId]?.label ?? source.toolId),
    ),
  ];
  const priceUnavailable = ["invalid", "unsupported", "missing"].includes(
    pricing?.catalogState ?? "",
  );
  const unpriced = pricing?.unpricedModelIds.length ?? 0;
  if (!failed.length && !priceUnavailable && !unpriced) return null;
  return (
    <div
      role="status"
      className="flex flex-wrap items-center gap-x-3 gap-y-1 text-note text-warn"
    >
      {!!failed.length && (
        <span>
          {t("usageWorkbench.sourcesFailed", { tools: failed.join(" · ") })}
        </span>
      )}
      {priceUnavailable ? (
        <span>{t("usageWorkbench.priceUnavailable")}</span>
      ) : (
        !!unpriced && (
          <span>{t("usageWorkbench.pricePartial", { count: unpriced })}</span>
        )
      )}
      {!!failed.length && onRetry && (
        <Button variant="ghost" className="h-6 px-2" onClick={onRetry}>
          {t("common.retry")}
        </Button>
      )}
    </div>
  );
}
