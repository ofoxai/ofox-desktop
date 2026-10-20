import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Sparkline } from "@/components/ui/sparkline";
import { ShareBar } from "@/components/ui/share-bar";
import {
  isOfoxBillingManager,
  type OfoxUserInfo,
  type OfoxSpending,
} from "@/lib/api/ofoxAuth";
import type { BalanceTrend } from "@/lib/api/usageOverview";
import { runsOutIn } from "@/lib/usageFormat";
import { cn } from "@/lib/utils";
export function BalanceTrendCard({
  user,
  trend,
  spending,
  recharge,
}: {
  user: OfoxUserInfo | null;
  trend?: BalanceTrend;
  spending?: OfoxSpending | null;
  recharge: () => void;
}) {
  const { t } = useTranslation();
  const amount = user?.balance?.balance;
  const remaining = runsOutIn(trend?.daysLeft);
  const points = trend?.points ?? [];
  const limit = spending?.monthlyLimitUsd;
  const used = spending?.monthlyUsedUsd;
  return (
    <div
      role="region"
      aria-label={t("usagePage.balance")}
      className={cn(
        "rounded-card border border-line bg-card p-3.5",
        amount != null && amount <= 0 && "border-danger/30 bg-danger-soft",
      )}
    >
      <div className="flex items-center gap-3">
        <div className="flex-1">
          <p className="text-sub text-ink-muted">{t("usagePage.balance")}</p>
          <b
            className={cn(
              "text-[20px] font-semibold tabular-nums",
              amount != null && amount <= 0 && "text-danger",
            )}
          >
            {amount == null ? "—" : `$${amount.toFixed(2)}`}
          </b>
        </div>
        {isOfoxBillingManager(user) && (
          <Button variant="primary" onClick={recharge}>
            {t("usagePage.recharge")}
          </Button>
        )}
      </div>
      {points.length > 0 && (
        <Sparkline
          points={points.map((p) => p.amount)}
          aria-label={t("usagePage.trend")}
          className="mt-2"
        />
      )}
      {remaining && (
        <p className="mt-2 text-note text-ink-muted">
          {t(
            "days" in remaining
              ? "usagePage.runsOutDays"
              : "usagePage.runsOutHours",
            {
              count: "days" in remaining ? remaining.days : remaining.hours,
            },
          )}
        </p>
      )}
      {!spending?.forbidden && limit != null && used != null && (
        <div className="mt-3 space-y-1">
          <p className="text-note text-ink-muted">
            {t("usagePage.monthly", {
              used: `$${used.toFixed(2)}`,
              limit: `$${limit.toFixed(2)}`,
            })}
          </p>
          <ShareBar value={limit > 0 ? used / limit : 0} />
        </div>
      )}
    </div>
  );
}
