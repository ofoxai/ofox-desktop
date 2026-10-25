import { useTranslation } from "react-i18next";
import { fmtTokens } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import type { RequestDashboardTotals } from "@/types/usageRequestDashboard";

export function UsageRequestSummary({
  totals,
  onFailed,
}: {
  totals: RequestDashboardTotals;
  onFailed: () => void;
}) {
  const { t, i18n } = useTranslation();
  const m = totals.measurements;
  const num = (n: number) =>
    n.toLocaleString(i18n.language, { maximumFractionDigits: 1 });
  const percent = (n: number) =>
    n.toLocaleString(i18n.language, {
      style: "percent",
      minimumFractionDigits: 1,
      maximumFractionDigits: 1,
    });
  const none = t("usageWorkbench.noAvailableSamples");
  const prompt =
    totals.inputTokens + totals.cacheReadTokens + totals.cacheWriteTokens;
  const cells = [
    {
      label: t("requestDashboard.tokens"),
      value: fmtTokens(totals.totalTokensIncludingCache),
      title: num(totals.totalTokensIncludingCache),
      detail: (
        <>
          {t("requestDashboard.tokenParts", {
            input: fmtTokens(totals.inputTokens),
            output: fmtTokens(totals.outputTokens),
            read: fmtTokens(totals.cacheReadTokens),
            write: fmtTokens(totals.cacheWriteTokens),
          })}
        </>
      ),
    },
    {
      label: t("requestDashboard.requests"),
      value: num(totals.requests),
      detail: m.statusSamples ? (
        <>
          {m.failedSamples > 0 && (
            <button
              type="button"
              className="text-warn underline decoration-dotted underline-offset-2"
              onClick={onFailed}
            >
              {t("requestDashboard.failed", { count: m.failedSamples })}
            </button>
          )}
          <span className="block">
            {t("requestDashboard.successSamples", {
              rate: percent(1 - (m.failureRate ?? 0)),
              count: m.statusSamples,
              total: num(totals.requests),
            })}
          </span>
        </>
      ) : (
        none
      ),
    },
    {
      label: t("requestDashboard.cost"),
      value: formatUsageEstimate(totals.estimate),
      detail: totals.estimate.unpricedRecordCount
        ? t("requestDashboard.unpriced", {
            count: totals.estimate.unpricedRecordCount,
          })
        : t("usageWorkbench.currentEstimate"),
    },
    {
      label: t("requestDashboard.cacheHit"),
      value: prompt ? percent(totals.cacheHitRate) : "—",
      detail: (
        <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-pill">
          <div
            className="h-full rounded-full bg-emerald-500"
            style={{
              width: `${Math.min(100, Math.max(0, totals.cacheHitRate * 100))}%`,
            }}
          />
        </div>
      ),
    },
  ];
  return (
    <section
      aria-label={t("requestDashboard.summary")}
      className="grid min-w-0 grid-cols-2 overflow-hidden rounded-list border border-line bg-card min-[800px]:grid-cols-4"
    >
      {cells.map((cell, index) => (
        <div
          key={cell.label}
          className={`min-w-0 border-line-2 px-4 py-3 ${index % 2 ? "border-l" : ""} ${index > 0 ? "min-[800px]:border-l" : ""} ${index >= 2 ? "border-t min-[800px]:border-t-0" : ""}`}
        >
          <div className="text-note text-ink-muted">{cell.label}</div>
          <div
            className="mt-1 break-words text-[24px] font-semibold leading-tight tabular-nums text-ink"
            title={cell.title}
          >
            {cell.value}
          </div>
          <div className="mt-1.5 break-words text-note leading-relaxed text-ink-muted">
            {cell.detail}
          </div>
        </div>
      ))}
    </section>
  );
}
