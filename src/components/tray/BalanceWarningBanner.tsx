import { settingsApi } from "@/lib/api";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useOfoxAuth } from "@/hooks/useOfoxAuth";
import { isOfoxBillingManager } from "@/lib/api/ofoxAuth";
import { ofoxWalletUrl } from "@/lib/ofoxUrls";

interface BalanceWarningBannerProps {
  /** 用户在偏好里设置的低余额阈值，单位 USD。 */
  threshold: number;
  /** 当前实时余额，单位 USD。父组件已确保 < threshold 时才渲染本组件。 */
  balanceUsd: number;
}

/**
 * 余额低于阈值时显示的警告条。
 *
 * 显示条件由父组件 `TrayPopoverApp` 控制（`active` + 阈值 + 开关），
 * 这里只负责渲染。
 *
 * 充值按钮的可见性按 role 控制——member 看到余额不足时也能从警告里得知
 * 风险，但没有充值权限，所以按钮收起；owner / admin 可以一键跳到
 * `/manage/wallet`（跟随当前 apex）。
 */
export default function BalanceWarningBanner({
  threshold,
  balanceUsd,
}: BalanceWarningBannerProps) {
  const { apex } = useOfoxApex();
  const { status } = useOfoxAuth();
  const canRecharge = isOfoxBillingManager(status?.user);

  // 阈值显示规则：整数（10）显 $10，带小数（5.5）显 $5.5
  const thresholdLabel = Number.isInteger(threshold)
    ? `$${threshold}`
    : `$${threshold}`;

  void balanceUsd; // 当前未在 UI 文案里直接展示，但留作签名稳定，将来可加"剩余 $1.23"

  return (
    <div className="mx-3.5 flex items-center gap-2.5 rounded-lg border border-[#e8d5a3] bg-gradient-to-l from-[#f5dfa0] to-[#faf3e0] px-2.5 py-2 dark:border-yellow-500/30 dark:from-yellow-600/25 dark:to-yellow-500/5">
      {/* 圆形感叹号图标 */}
      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-amber-400 text-[13px] font-bold text-white">
        !
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[11px] font-semibold text-amber-800 dark:text-amber-300">
          余额不足 {thresholdLabel}
        </div>
        <div className="text-[10px] leading-tight text-amber-700/80 dark:text-amber-400/70">
          按当前速率可用不足 1 天，建议尽快充值
        </div>
      </div>
      {canRecharge && (
        <button
          onClick={() => settingsApi.openExternal(ofoxWalletUrl(apex))}
          className="shrink-0 rounded-md bg-orange-500 px-2.5 py-0.5 text-[11px] font-medium text-white hover:bg-orange-600"
        >
          充值
        </button>
      )}
    </div>
  );
}
