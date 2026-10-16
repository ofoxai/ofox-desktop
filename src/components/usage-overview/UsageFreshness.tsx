import { useTranslation } from "react-i18next";
import { TOOL_META } from "@/config/toolMeta";
import type { UsageOverview } from "@/lib/api/usageOverview";
import type { UsagePricingCoverage } from "@/types/usageQuery";

export function UsageFreshness({
  localUpdatedAt,
  accountUpdatedAt,
  sources,
  pricing,
}: {
  localUpdatedAt?: number | null;
  accountUpdatedAt?: number | null;
  sources?: UsageOverview["sources"];
  pricing?: UsagePricingCoverage;
}) {
  const { t, i18n } = useTranslation();
  const time = (at: number | null | undefined) =>
    at != null
      ? new Date(at).toLocaleString(i18n.language)
      : t("usageWorkbench.notRecorded");
  const sourceIssues =
    sources?.filter((source) =>
      ["failed", "partial", "unknown"].includes(source.status),
    ) ?? [];
  const sourceSummaries = [
    ...new Map(
      sourceIssues.map((source) => [
        `${source.toolId}:${source.status}`,
        source,
      ]),
    ).values(),
  ];
  const priceUnavailable =
    pricing &&
    ["invalid", "unsupported", "missing"].includes(pricing.catalogState ?? "");
  const pricePartial = !!pricing?.unpricedModelIds.length;
  const hasWarning =
    !!sourceIssues.length || priceUnavailable || pricing?.stale || pricePartial;
  return (
    <div className="space-y-1.5 text-note text-ink-muted">
      {hasWarning && (
        <div role="status" className="space-y-1 text-warn">
          {!!sourceIssues.length && (
            <p className="flex flex-wrap gap-x-3 gap-y-1">
              {sourceSummaries.map((source) => (
                <span key={source.sourceId}>
                  {TOOL_META[source.toolId]?.label ?? source.toolId} ·{" "}
                  {t("usageWorkbench.health." + source.status)}
                </span>
              ))}
            </p>
          )}
          {priceUnavailable ? (
            <p>{t("usageWorkbench.priceUnavailable")}</p>
          ) : pricing?.stale ? (
            <p>{t("usageWorkbench.priceStale")}</p>
          ) : null}
          {pricePartial && (
            <p>
              {t("usageWorkbench.pricePartial", {
                count: pricing!.unpricedModelIds.length,
              })}
            </p>
          )}
        </div>
      )}
      <details>
        <summary className="w-fit cursor-pointer rounded-sm text-ink-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand/50">
          {t("usageWorkbench.dataStatus")}
        </summary>
        <div className="mt-2 space-y-1.5 rounded-lg border border-line bg-card px-3 py-2">
          {localUpdatedAt !== undefined && (
            <p>
              {t("usageWorkbench.localUpdated", { time: time(localUpdatedAt) })}
            </p>
          )}
          {accountUpdatedAt !== undefined && (
            <p>
              {t("usageWorkbench.accountUpdated", {
                time: time(accountUpdatedAt),
              })}
            </p>
          )}
          {pricing && (
            <>
              <p>
                {t("usageWorkbench.priceUpdated", {
                  time: time(pricing.catalogUpdatedAtMs),
                })}
              </p>
              {pricing.catalogVersion && (
                <p
                  className="font-mono text-micro text-ink-faint break-all"
                  title={pricing.catalogVersion}
                >
                  {pricing.catalogVersion}
                </p>
              )}
            </>
          )}
          <p className="text-ink-faint">{t("usagePage.disclaimer")}</p>
          {!!sources?.length && (
            <div className="space-y-1 border-t border-line-2 pt-2">
              <p className="font-medium">{t("usageWorkbench.sourceHealth")}</p>
              <ul className="space-y-2">
                {sources.map((source) => (
                  <li key={source.sourceId} className="break-words">
                    {TOOL_META[source.toolId]?.label ?? source.toolId} ·{" "}
                    {source.sourceId} ·{" "}
                    {t("usageWorkbench.health." + source.status)} ·{" "}
                    {t("usageWorkbench.sourceSuccess", {
                      time: time(source.lastSuccessAtMs),
                    })}
                    <p>
                      {source.lastScan
                        ? t("usageWorkbench.sourceScan", {
                            files: source.lastScan.filesScanned.toLocaleString(
                              i18n.language,
                            ),
                            imported: source.lastScan.imported.toLocaleString(
                              i18n.language,
                            ),
                            skipped: source.lastScan.skipped.toLocaleString(
                              i18n.language,
                            ),
                          })
                        : t("usageWorkbench.sourceScanUnknown")}
                    </p>
                    <p>
                      {source.retainedDetailRange
                        ? t("usageWorkbench.retainedDetailRange", {
                            start: time(source.retainedDetailRange.startMs),
                            end: time(source.retainedDetailRange.endMs - 1),
                          })
                        : t("usageWorkbench.retainedDetailRangeUnknown")}
                    </p>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      </details>
    </div>
  );
}

/** Requests and overview share the same visible warnings and folded metadata. */
export function UsagePriceNotice({
  pricing,
}: {
  pricing?: UsagePricingCoverage;
}) {
  return <UsageFreshness pricing={pricing} />;
}
