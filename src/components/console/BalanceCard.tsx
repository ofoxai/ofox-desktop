import { Loader2, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { UserAvatar } from "@/components/UserAvatar";
import { Button } from "@/components/ui/button";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { settingsApi } from "@/lib/api";
import { isOfoxBillingManager } from "@/lib/api/ofoxAuth";
import { ofoxAvatarUrl, ofoxWalletUrl } from "@/lib/ofoxUrls";
import { cn } from "@/lib/utils";

interface BalanceCardProps {
  account: OfoxAccount;
  className?: string;
}

/**
 * The signed-in user and the OFox wallet balance, red once it is spent.
 * The header's refresh reloads it; the card has no refresh of its own.
 */
export function BalanceCard({ account, className }: BalanceCardProps) {
  const { t } = useTranslation();
  const { apex } = useOfoxApex();
  const { user } = account;
  // 只有明确查到余额且不大于 0 才算没钱；还没查到（—）不报警。
  const balance = user?.balance?.balance;
  const balanceEmpty = typeof balance === "number" && balance <= 0;

  return (
    <div
      data-testid="balance-card"
      data-empty={balanceEmpty ? "true" : undefined}
      className={cn(
        "flex min-w-0 items-center gap-3 rounded-card border px-3.5 py-2.5",
        balanceEmpty
          ? "border-danger/40 bg-danger-soft/60"
          : "border-line bg-card",
        className,
      )}
    >
      <UserAvatar
        avatarUrl={ofoxAvatarUrl(user?.avatar_url)}
        name={user?.name}
        email={user?.email}
        className="h-8 w-8"
        fallbackTextClassName="text-ui"
      />
      <div className="min-w-0 flex-1">
        <div className="truncate text-ui font-semibold text-ink">
          {user?.name ??
            t(
              account.retrying
                ? "console.balance.loadingName"
                : "console.balance.userFallback",
            )}
        </div>
        <div className="mt-px flex min-w-0 items-center text-sub text-ink-muted">
          {user?.email ? (
            <span className="truncate">{user.email}</span>
          ) : account.retrying ? (
            <span className="inline-flex items-center gap-1">
              <Loader2 aria-hidden className="h-3 w-3 animate-spin" />
              {account.retryAttempt > 0
                ? t("console.balance.loadingUserAttempt", {
                    attempt: account.retryAttempt,
                  })
                : t("console.balance.loadingUser")}
            </span>
          ) : account.loadFailed ? (
            <button
              type="button"
              onClick={() => void account.retryUser()}
              className="inline-flex items-center gap-1 text-brand-ink underline-offset-2 hover:underline"
            >
              <RefreshCw aria-hidden className="h-3 w-3" />
              {t("console.balance.loadFailed")}
            </button>
          ) : null}
        </div>
      </div>
      <div className="flex-none text-right">
        <div className="text-note text-ink-muted">
          {t("console.balance.label")}
        </div>
        <div
          className={cn(
            "text-kpi font-semibold leading-tight tabular-nums",
            balanceEmpty ? "text-danger" : "text-ink",
          )}
        >
          {formatBalance(balance)}
        </div>
      </div>
      {isOfoxBillingManager(user) && (
        <Button
          className="ml-1 flex-none"
          onClick={() => void settingsApi.openExternal(ofoxWalletUrl(apex))}
        >
          {t("console.balance.topUp")}
        </Button>
      )}
    </div>
  );
}

/**
 * Render the wallet figure. `null`/`undefined` collapses to `—` so the
 * layout doesn't shift between "loading" and "no balance returned".
 */
function formatBalance(value: number | null | undefined): string {
  if (typeof value !== "number" || Number.isNaN(value)) return "—";
  return `$${value.toFixed(2)}`;
}
