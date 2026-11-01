import { useId } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { KpiStrip } from "@/components/ui/kpi-strip";
import { assistantApi } from "@/lib/api/assistant";
import { fmtCost, fmtTokens } from "@/lib/format";
import { queryClient } from "@/lib/query/queryClient";

export const ASSISTANT_USAGE_KEY = ["assistantUsage"] as const;

/**
 * What the Ofox assistant's saved conversations used, the trash included.
 * `identity` keys the read to the signed-in account; it is read again each
 * time the card is shown, since replies change it on another page. A cost
 * is never shown as a partial sum: unknown whenever a request's is.
 */
export function AssistantUsageCard({
  identity,
  enabled,
}: {
  identity: string;
  enabled: boolean;
}) {
  const { t } = useTranslation();
  const titleId = useId();
  const usage = useQuery(
    {
      queryKey: [...ASSISTANT_USAGE_KEY, identity],
      queryFn: assistantApi.usageSummary,
      staleTime: 0,
      gcTime: 0,
      retry: false,
      refetchOnWindowFocus: false,
      enabled,
    },
    queryClient,
  );
  const data = usage.data;
  const costKnown =
    data?.usageCoverage !== "incomplete" &&
    typeof data?.estimatedCostUsd === "string";
  const searches = data?.searchCount ?? 0;
  return (
    <section aria-labelledby={titleId} className="space-y-2">
      <p id={titleId} className="pt-1 text-note font-medium text-ink-muted">
        {t("assistant.title")}
      </p>
      <KpiStrip
        className="!grid-cols-2 sm:!grid-cols-4"
        items={[
          {
            label: t("usagePage.assistant.input"),
            value: data ? fmtTokens(data.inputTokens ?? 0) : "—",
          },
          {
            label: t("usagePage.assistant.output"),
            value: data ? fmtTokens(data.outputTokens ?? 0) : "—",
          },
          {
            label: t("usagePage.assistant.cost"),
            value:
              data && costKnown ? fmtCost(Number(data.estimatedCostUsd)) : "—",
            sub:
              data && !costKnown
                ? t("usagePage.assistant.costUnavailable")
                : undefined,
            title: t("usagePage.estimate"),
          },
          {
            label: t("usagePage.assistant.searches"),
            value: data ? String(searches) : "—",
            sub:
              searches > 0 && typeof data?.searchCostUsd === "string"
                ? fmtCost(Number(data.searchCostUsd))
                : undefined,
          },
        ]}
      />
      <p className="text-note text-ink-muted">
        {usage.isError && !data
          ? t("usagePage.assistant.unavailable")
          : t("usagePage.assistant.includesTrash")}
      </p>
    </section>
  );
}
