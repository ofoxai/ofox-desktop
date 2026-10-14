import { Loader2, RefreshCw } from "lucide-react";
import { UserAvatar } from "@/components/UserAvatar";
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

/** The signed-in user and the OFox wallet balance, red once it is spent. */
export function BalanceCard({ account, className }: BalanceCardProps) {
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
        "rounded-xl border px-5 py-4",
        balanceEmpty
          ? "border-red-500/40 bg-red-500/5"
          : "border-border bg-background/80",
        className,
      )}
    >
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <UserAvatar
            avatarUrl={ofoxAvatarUrl(user?.avatar_url)}
            name={user?.name}
            email={user?.email}
            className="h-10 w-10"
            fallbackTextClassName="text-lg"
          />
          <div>
            <div className="text-[14px] font-semibold text-foreground">
              {user?.name ?? (account.retrying ? "加载中…" : "用户")}
            </div>
            <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
              {user?.email ? (
                <span className="truncate">{user.email}</span>
              ) : account.retrying ? (
                <span className="inline-flex items-center gap-1">
                  <Loader2 className="h-3 w-3 animate-spin" />
                  正在加载用户信息
                  {account.retryAttempt > 0
                    ? `（${account.retryAttempt}/3）`
                    : "…"}
                </span>
              ) : account.loadFailed ? (
                <button
                  type="button"
                  onClick={() => void account.retryUser()}
                  className="inline-flex items-center gap-1 text-orange-500 hover:text-orange-600 hover:underline"
                  title="无法加载用户信息，点击重试"
                >
                  <RefreshCw className="h-3 w-3" />
                  无法加载用户信息，点击重试
                </button>
              ) : null}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-3">
          <div className="text-right">
            <div className="flex items-center justify-end gap-1.5 text-[11px] text-muted-foreground">
              <span>OFOX 账户余额</span>
              <button
                onClick={() => void account.refreshBalance()}
                disabled={account.balanceRefreshing}
                title="刷新余额"
                className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
              >
                <RefreshCw
                  className={`h-3 w-3 ${
                    account.balanceRefreshing ? "animate-spin" : ""
                  }`}
                />
              </button>
            </div>
            <div
              className={`text-2xl font-bold ${
                balanceEmpty ? "text-red-600" : "text-foreground"
              }`}
            >
              {formatBalance(user?.balance?.balance)}
            </div>
          </div>
          {isOfoxBillingManager(user) && (
            <button
              onClick={() => settingsApi.openExternal(ofoxWalletUrl(apex))}
              className={`rounded-lg px-4 py-1.5 text-[13px] font-medium text-white ${
                balanceEmpty
                  ? "bg-orange-600 shadow-md ring-2 ring-orange-500/40 hover:bg-orange-700"
                  : "bg-orange-500 hover:bg-orange-600"
              }`}
            >
              充值
            </button>
          )}
        </div>
      </div>
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
