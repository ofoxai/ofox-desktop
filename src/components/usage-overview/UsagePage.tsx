import { useState } from "react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { useShellRefresh } from "@/components/shell/ShellContext";
import { Button } from "@/components/ui/button";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useUsageOverview } from "@/hooks/useUsageOverview";
import { settingsApi } from "@/lib/api";
import { ofoxDashboardUrl, ofoxWalletUrl } from "@/lib/ofoxUrls";
import type { UsagePeriod } from "@/lib/api/usageOverview";
import { TOOL_META } from "@/config/toolMeta";
import { UsageHeader } from "./UsageHeader";
import { BalanceTrendCard } from "./BalanceTrendCard";
import { UsageKpis } from "./UsageKpis";
import { UsageBars } from "./UsageBars";
import { UsageBreakdownList } from "./UsageBreakdownList";
const EMPTY: string[] = [];
export function UsagePage({
  account,
  boundTools = EMPTY,
}: {
  account: OfoxAccount;
  boundTools?: string[];
}) {
  const { t } = useTranslation();
  const { apex } = useOfoxApex();
  const [period, setPeriod] = useState<UsagePeriod>(() => {
    const value = localStorage.getItem("ofox.usagePeriod");
    return ["today", "7d", "30d", "all"].includes(value ?? "")
      ? (value as UsagePeriod)
      : "7d";
  });
  const state = useUsageOverview(period, account.user, apex);
  const data = state.overview.data;
  const [refreshError, setRefreshError] = useState<string | null>(null);
  useShellRefresh("usage", async () => {
    setRefreshError(null);
    try {
      await state.refresh();
    } catch (e) {
      setRefreshError(String(e));
      toast.error(t("usagePage.refreshFailed"));
    }
  });
  const open = (url: string) =>
    void settingsApi
      .openExternal(url)
      .catch(() => toast.error(t("shell.openFailed")));
  const unsupported =
    data?.unsupportedTools.filter((id) => boundTools.includes(id)) ?? [];
  const errors = [
    state.overview.error,
    state.balance.error,
    state.trend.error,
    state.spending.error,
  ].filter(Boolean);
  const empty = !data || !data.totals.requests;
  return (
    <div className="flex flex-col gap-3">
      <UsageHeader
        period={period}
        onChange={(v) => {
          setPeriod(v);
          localStorage.setItem("ofox.usagePeriod", v);
        }}
        cost={data?.totals.costUsd}
      />
      <BalanceTrendCard
        user={state.user}
        trend={state.trend.data}
        spending={state.spending.data}
        recharge={() => open(ofoxWalletUrl(apex))}
      />
      {(errors.length > 0 || refreshError) && (
        <div
          role="alert"
          className="rounded-card border border-warn/30 bg-warn-soft px-3 py-2 text-sub text-warn"
        >
          {t("usagePage.refreshFailed")}
          <details>
            <summary>{t("usagePage.errorDetails")}</summary>
            <pre className="whitespace-pre-wrap break-words text-note">
              {refreshError || errors.map(String).join("\n")}
            </pre>
          </details>
          <Button
            variant="ghost"
            onClick={() => {
              setRefreshError(null);
              void state.refresh().catch((e) => setRefreshError(String(e)));
            }}
          >
            {t("common.retry")}
          </Button>
        </div>
      )}
      {state.overview.isPending ? (
        <p role="status" className="p-6 text-center text-ink-muted">
          {t("common.loading")}
        </p>
      ) : empty ? (
        <div className="rounded-list border border-line bg-card p-8 text-center text-sub text-ink-muted">
          {t(
            state.overview.isError
              ? "usagePage.unavailable"
              : "usagePage.empty",
          )}
        </div>
      ) : (
        <>
          <UsageKpis totals={data.totals} />
          <UsageBars overview={data} />
          <UsageBreakdownList
            kind="tools"
            rows={data.byTool.map((r) => ({ ...r, id: r.appType }))}
            total={data.totals.totalTokens}
          />
          <UsageBreakdownList
            kind="models"
            rows={data.byModel.map((r) => ({ ...r, id: r.model }))}
            total={data.totals.totalTokens}
          />
        </>
      )}
      {!!unsupported.length && (
        <p className="text-note text-ink-muted">
          {t("usagePage.unsupported", {
            tools: unsupported
              .map((id) => TOOL_META[id]?.label ?? id)
              .join(", "),
          })}
        </p>
      )}
      <p className="text-note text-ink-faint">{t("usagePage.disclaimer")}</p>
      <Button
        variant="link"
        className="h-auto self-start px-0 text-note"
        onClick={() => open(ofoxDashboardUrl(apex))}
      >
        {t("shell.usage.webDetails")}
      </Button>
    </div>
  );
}
