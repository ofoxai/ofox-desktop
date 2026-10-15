import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { KpiStrip } from "@/components/ui/kpi-strip";
import { getUsageSourceSummary } from "@/lib/api/usageLifetime";
import { fmtTokens } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import type { UsageCapabilities } from "@/types/usageQuery";
import type {
  LifetimeBuckets,
  UsageLifetimeSummary,
} from "@/types/usageLifetime";

export interface UsageLifetimeEntryProps {
  boundTools: string[];
  capabilities?: UsageCapabilities;
  apex: string;
  active: boolean;
}

export function UsageLifetimeEntry({
  boundTools,
  capabilities,
  apex,
  active,
}: UsageLifetimeEntryProps) {
  const supported = capabilities?.sources.some(
    (source) =>
      source.toolId === "hermes" && source.lifetimeUsageSupported === true,
  );
  if (!active || !supported || !boundTools.includes("hermes")) return null;
  // Unmounting isolates hidden pages, binding changes, and apex ABA responses.
  return <HermesLifetimeDialog key={apex} apex={apex} />;
}

function HermesLifetimeDialog({ apex }: { apex: string }) {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const [data, setData] = useState<UsageLifetimeSummary>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const [limit, setLimit] = useState(50);
  const generation = useRef(0);
  useEffect(
    () => () => {
      generation.current += 1;
    },
    [],
  );
  const read = async () => {
    const owner = ++generation.current;
    setBusy(true);
    setError(false);
    try {
      const next = await getUsageSourceSummary("hermes", apex);
      if (owner === generation.current) setData(next);
    } catch {
      if (owner === generation.current) setError(true);
    } finally {
      if (owner === generation.current) setBusy(false);
    }
  };
  const changeOpen = (next: boolean) => {
    setOpen(next);
    setLimit(50);
    if (next) void read();
    else {
      generation.current += 1;
      setBusy(false);
    }
  };
  const number = (value: number | null) =>
    value == null ? "—" : value.toLocaleString(i18n.language);
  const tokens = (value: number | null) =>
    value == null ? "—" : fmtTokens(value);
  const total = (buckets: LifetimeBuckets) =>
    (buckets.amountKind === "lowerBound" && buckets.totalTokens != null
      ? "≥"
      : "") + tokens(buckets.totalTokens);
  const lastRead =
    data?.lastSuccessAtMs == null
      ? null
      : new Date(data.lastSuccessAtMs).toLocaleString(i18n.language);
  const priceUnavailable =
    data &&
    (!data.pricingCoverage.catalogVersion ||
      ["invalid", "unsupported", "missing"].includes(
        data.pricingCoverage.catalogState ?? "",
      ));
  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogTrigger asChild>
        <Button variant="ghost" size="sm">
          {t("usageLifetime.title")}
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-3xl overflow-hidden">
        <DialogHeader>
          <DialogTitle>{t("usageLifetime.title")}</DialogTitle>
          <DialogDescription>{t("usageLifetime.scope")}</DialogDescription>
        </DialogHeader>
        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-5 pb-4">
          {error && (
            <p role="alert" className="text-note text-danger">
              {t("usageLifetime.failed")}
            </p>
          )}
          {busy && (
            <p role="status" className="text-note text-ink-muted">
              {t("usageLifetime.loading")}
            </p>
          )}
          {data && (
            <>
              {data.sourceState !== "ready" && (
                <p className="rounded-item border border-line px-3 py-2 text-note text-ink-muted">
                  {t(
                    `usageLifetime.${data.stale ? (data.sourceState === "missing" ? "staleMissing" : "staleError") : data.sourceState === "missing" ? "missing" : "unreadable"}`,
                  )}
                </p>
              )}
              {lastRead && (
                <p className="text-note text-ink-faint">
                  {t("usageLifetime.lastRead", { time: lastRead })}
                </p>
              )}
              {data.snapshotVersion != null && (
                <>
                  <div className="flex flex-wrap gap-x-4 gap-y-1 text-note text-ink-muted">
                    <span>
                      {t("usageLifetime.counterCoverage", {
                        value: t(
                          `usageLifetime.coverage.${data.counterCoverage}`,
                        ),
                      })}
                    </span>
                    <span>
                      {t("usageLifetime.modelCoverage", {
                        value: t(
                          `usageLifetime.coverage.${data.modelCoverage}`,
                        ),
                      })}
                    </span>
                    <span>
                      {t("usageLifetime.priceCoverage", {
                        value: t(
                          `usageLifetime.price.${data.estimate.availability}`,
                        ),
                      })}
                    </span>
                  </div>
                  {data.modelCoverage !== "complete" && (
                    <p className="text-note text-ink-muted">
                      {t("usageLifetime.modelPartial")}
                    </p>
                  )}
                  <KpiStrip
                    className="!grid-cols-2 sm:!grid-cols-3"
                    items={[
                      {
                        label: "Tokens",
                        value: total(data.totals),
                        sub: t("usageLifetime.mainTokens"),
                      },
                      {
                        label: t("usageWorkbench.currentEstimate"),
                        value: formatUsageEstimate(data.estimate),
                        sub: t("usagePage.estimate"),
                      },
                      {
                        label: t("usageLifetime.sessionRecords"),
                        value: number(data.sessionRecords),
                      },
                      {
                        label: t("usagePage.cacheRead"),
                        value: tokens(data.totals.cacheReadTokens),
                      },
                      {
                        label: t("usageSessions.cacheWrite"),
                        value: tokens(data.totals.cacheWriteTokens),
                      },
                      {
                        label: t("usageLifetime.apiCalls"),
                        value: number(data.totals.apiCallCount),
                      },
                    ]}
                  />
                  <p className="text-note text-ink-muted">
                    {t("usagePage.io", {
                      input: tokens(data.totals.inputTokens),
                      output: tokens(data.totals.outputTokens),
                    })}{" "}
                    · {t("usageWorkbench.cacheSeparate")}
                  </p>
                  <p className="text-note text-ink-faint">
                    {t("usageLifetime.callExplanation")}
                  </p>
                  {priceUnavailable && (
                    <p className="text-note text-ink-muted">
                      {t("usageWorkbench.priceUnavailable")}
                    </p>
                  )}
                  {data.pricingCoverage.stale && (
                    <p className="text-note text-ink-muted">
                      {t("usageWorkbench.priceStale")}
                    </p>
                  )}
                  <h3 className="text-body font-medium text-ink">
                    {t("usageLifetime.models")}
                  </h3>
                  <ul
                    aria-label={t("usageLifetime.models")}
                    className="divide-y divide-line rounded-list border border-line"
                  >
                    {data.byModel.slice(0, limit).map((row, index) => (
                      <li
                        key={`${row.category}:${row.modelId}:${index}`}
                        className="flex min-w-0 items-center justify-between gap-3 px-3 py-2"
                      >
                        <div className="min-w-0">
                          <div
                            className="truncate text-body text-ink"
                            title={row.modelId ?? t("usageLifetime.residual")}
                          >
                            {row.modelId ?? t("usageLifetime.residual")}
                          </div>
                          <p className="text-note text-ink-faint">
                            {t(`usageLifetime.category.${row.category}`)}
                          </p>
                        </div>
                        <div className="shrink-0 text-right text-note tabular-nums text-ink-muted">
                          <div>{total(row.buckets)} Tokens</div>
                          <div>{formatUsageEstimate(row.estimate)}</div>
                        </div>
                      </li>
                    ))}
                  </ul>
                  {limit < data.byModel.length && (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setLimit((value) => value + 50)}
                    >
                      {t("usageSessions.showMore", {
                        count: data.byModel.length - limit,
                      })}
                    </Button>
                  )}
                </>
              )}
            </>
          )}
        </div>
        <div className="flex justify-end gap-2 border-t border-line px-5 py-3">
          <Button
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => void read()}
          >
            {t("usageLifetime.refresh")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => changeOpen(false)}
          >
            {t("usageWorkbench.close")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
