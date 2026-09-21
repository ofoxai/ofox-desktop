import { RefreshCw } from "lucide-react";
import type { OfoxAuthStateValue } from "@/lib/api/ofoxAuth";

interface AccountBalanceProps {
  state: OfoxAuthStateValue;
  /** Balance in USD (floating point). null while the value is unknown. */
  balanceUsd?: number | null;
  /** Triggered by the inline refresh affordance. Disabled when not active. */
  onRefresh?: () => void;
  /** Drives the spinner animation while the parent is refetching. */
  refreshing?: boolean;
}

/**
 * The hero "balance" line under the user header.
 *
 * Three render modes:
 *   - `active`   + balance number  →  "$8.40"
 *   - `active`   + null balance    →  skeleton (data still loading)
 *   - non-active                   →  "$--    需重新登录后更新"
 *
 * Per-day / per-month deltas are intentionally suppressed until the backend
 * surfaces them on /openapi/me — showing fabricated numbers next to the real
 * balance would erode trust faster than missing data does.
 */
export default function AccountBalance({
  state,
  balanceUsd,
  onRefresh,
  refreshing = false,
}: AccountBalanceProps) {
  const isActive = state === "active";
  const hasNumber = isActive && typeof balanceUsd === "number";
  const displayValue = hasNumber
    ? formatUsd(balanceUsd as number)
    : isActive
      ? "—"
      : "$--";

  const helperLine = (() => {
    if (!isActive) {
      return state === "expired" ? "需重新登录后更新" : "请登录后查看余额";
    }
    return hasNumber ? "实时余额" : "正在同步…";
  })();

  return (
    <div className="px-3.5 pt-3 pb-2">
      <div className="flex items-center gap-1.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        <span>OFOX 账户余额</span>
        {onRefresh && (
          <button
            type="button"
            onClick={onRefresh}
            disabled={!isActive || refreshing}
            title="刷新余额"
            className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
          >
            <RefreshCw
              className={`h-3 w-3 ${refreshing ? "animate-spin" : ""}`}
            />
          </button>
        )}
      </div>
      <div
        className={`mt-0.5 text-2xl font-bold ${
          isActive ? "text-foreground" : "text-muted-foreground"
        }`}
      >
        {displayValue}
      </div>
      <div className="mt-0.5 text-[11px] text-muted-foreground">
        {helperLine}
      </div>
    </div>
  );
}

/** USD formatter that keeps two decimals and the leading $ sign. */
function formatUsd(value: number): string {
  const sign = value < 0 ? "-" : "";
  const abs = Math.abs(value);
  return `${sign}$${abs.toFixed(2)}`;
}
