import { useTranslation } from "react-i18next";
import { settingsApi } from "@/lib/api";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useOfoxAuth } from "@/hooks/useOfoxAuth";
import { useBalanceTrend, accountIdentity } from "@/hooks/useUsageOverview";
import { isOfoxBillingManager } from "@/lib/api/ofoxAuth";
import { ofoxWalletUrl } from "@/lib/ofoxUrls";
import { runsOutIn } from "@/lib/usageFormat";

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
 * 这里只负责渲染。第二行说按当前速度多久用完，来自余额走势
 * （`ofox_get_balance_trend`）；还没有足够的余额记录算出速度时不说。
 *
 * 充值按钮的可见性按 role 控制——member 看到余额不足时也能从警告里得知
 * 风险，但没有充值权限，所以按钮收起；owner / admin 可以一键跳到
 * `/manage/wallet`（跟随当前 apex）。
 */
export default function BalanceWarningBanner({
  threshold,
  balanceUsd,
}: BalanceWarningBannerProps) {
  const { t } = useTranslation();
  const { apex } = useOfoxApex();
  const { status } = useOfoxAuth();
  const { data: trend } = useBalanceTrend(
    !!status?.user,
    accountIdentity(status?.user, apex),
  );
  const canRecharge = isOfoxBillingManager(status?.user);
  const runsOut = runsOutIn(trend?.daysLeft);

  void balanceUsd; // 当前未在 UI 文案里直接展示，但留作签名稳定，将来可加"剩余 $1.23"

  return (
    // mb-2 把 banner 和下方 ActionButtons（充值/用量详情）拉开距离——之前两者
    // 直接贴在一起，banner 的金色描边压在按钮顶边上视觉很挤。margin 加在
    // banner 而不是 ActionButtons 上，是因为 ActionButtons 在没 banner 时
    // 紧贴 AccountBalance 是 intended look。
    <div className="mx-3.5 mb-2 flex items-center gap-2.5 rounded-lg border border-warn/25 bg-warn-soft px-2.5 py-2 ">
      {/* 圆形感叹号图标 */}
      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-warn text-[13px] font-bold text-brand-on">
        !
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[11px] font-semibold text-warn ">
          {t("balanceWarning.title", { threshold: `$${threshold}` })}
        </div>
        {runsOut && (
          <div className="text-[10px] leading-tight text-warn ">
            {"days" in runsOut
              ? t("balanceWarning.runsOutDays", { days: runsOut.days })
              : t("balanceWarning.runsOutSoon")}
          </div>
        )}
      </div>
      {canRecharge && (
        <button
          onClick={() => settingsApi.openExternal(ofoxWalletUrl(apex))}
          className="shrink-0 rounded-md bg-brand px-2.5 py-0.5 text-[11px] font-medium text-brand-on hover:bg-brand"
        >
          {t("balanceWarning.recharge")}
        </button>
      )}
    </div>
  );
}
