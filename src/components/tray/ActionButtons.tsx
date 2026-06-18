import { settingsApi } from "@/lib/api";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useOfoxAuth } from "@/hooks/useOfoxAuth";
import { isOfoxBillingManager } from "@/lib/api/ofoxAuth";
import {
  ofoxActivityUrl,
  ofoxWalletUrl,
} from "@/lib/ofoxUrls";

/**
 * Tray popover 顶部的两个 CTA：「充值」+「用量详情」。
 *
 * 「充值」按 OFox role 显隐（owner / admin 才看到）——member 只读，看到充值按钮
 * 点过去会被 OFox 控制台直接拦截，索性 UI 层就不展示。member 时整行只剩一个
 * 「用量详情」并撑满宽度，免得右边出现一片空白。
 *
 * 两个按钮的目标 URL 都跟随当前 apex（`useOfoxApex`），保证国内/海外切换后
 * tray 立刻指向对的域。
 */
export default function ActionButtons() {
  const { apex } = useOfoxApex();
  const { status } = useOfoxAuth();
  const canRecharge = isOfoxBillingManager(status?.user);

  return (
    <div className="flex gap-2 px-3.5 pb-3">
      {canRecharge && (
        <button
          onClick={() => settingsApi.openExternal(ofoxWalletUrl(apex))}
          className="flex-1 rounded-lg bg-orange-500 py-1.5 text-[12px] font-medium text-white hover:bg-orange-600"
        >
          充值
        </button>
      )}
      <button
        onClick={() => settingsApi.openExternal(ofoxActivityUrl(apex))}
        className="flex-1 rounded-lg border border-border py-1.5 text-[12px] font-medium text-foreground hover:bg-accent"
      >
        用量详情
      </button>
    </div>
  );
}
