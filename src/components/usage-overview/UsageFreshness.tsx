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
  localUpdatedAt: number | null;
  accountUpdatedAt?: number | null;
  sources?: UsageOverview["sources"];
  pricing?: UsagePricingCoverage;
}) {
  const { t, i18n } = useTranslation();
  const time = (at: number | null | undefined) =>
    at != null
      ? new Date(at).toLocaleString(i18n.language)
      : t("usageWorkbench.notRecorded");
  return (
    <div className="space-y-1 text-note text-ink-muted">
      <p>{t("usageWorkbench.localUpdated", { time: time(localUpdatedAt) })}</p>
      {accountUpdatedAt !== undefined && (
        <p>
          {t("usageWorkbench.accountUpdated", { time: time(accountUpdatedAt) })}
        </p>
      )}
      <UsagePriceNotice pricing={pricing} />
      {!!sources?.length && (
        <details>
          <summary className="cursor-pointer">
            {t("usageWorkbench.sourceHealth")}
          </summary>
          <ul className="mt-1 space-y-1">
            {sources.map((source) => (
              <li key={source.sourceId} className="break-words">
                {TOOL_META[source.toolId]?.label ?? source.toolId} ·{" "}
                {source.sourceId} ·{" "}
                {t(`usageWorkbench.health.${source.status}`)} ·{" "}
                {t("usageWorkbench.sourceSuccess", {
                  time: time(source.lastSuccessAtMs),
                })}
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

export function UsagePriceNotice({
  pricing,
}: {
  pricing?: UsagePricingCoverage;
}) {
  const { t, i18n } = useTranslation();
  const time = (at: number | null) =>
    at == null
      ? t("usageWorkbench.notRecorded")
      : new Date(at).toLocaleString(i18n.language);
  return (
    <div className="space-y-1 text-note text-ink-muted">
      {pricing && (
        <>
          <p>
            {t("usageWorkbench.priceUpdated", {
              time: time(pricing.catalogUpdatedAtMs),
            })}
            {pricing.catalogVersion ? ` · ${pricing.catalogVersion}` : ""}
          </p>
          {pricing.catalogState === "invalid" ||
          pricing.catalogState === "unsupported" ||
          pricing.catalogState === "missing" ? (
            <p>{t("usageWorkbench.priceUnavailable")}</p>
          ) : pricing.stale ? (
            <p>{t("usageWorkbench.priceStale")}</p>
          ) : null}
          {!!pricing.unpricedModelIds.length && (
            <p>
              {t("usageWorkbench.pricePartial", {
                count: pricing.unpricedModelIds.length,
              })}
            </p>
          )}
        </>
      )}
    </div>
  );
}
