export default function BalanceWarningBanner() {
  return (
    <div className="mx-3.5 flex items-center gap-2.5 rounded-lg border border-[#e8d5a3] bg-gradient-to-l from-[#f5dfa0] to-[#faf3e0] px-2.5 py-2 dark:border-yellow-500/30 dark:from-yellow-600/25 dark:to-yellow-500/5">
      {/* 圆形感叹号图标 */}
      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-amber-400 text-[13px] font-bold text-white">
        !
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[11px] font-semibold text-amber-800 dark:text-amber-300">
          余额不足 $10
        </div>
        <div className="text-[10px] leading-tight text-amber-700/80 dark:text-amber-400/70">
          按当前速率可用不足 1 天，建议尽快充值
        </div>
      </div>
      <button className="shrink-0 rounded-md bg-orange-500 px-2.5 py-0.5 text-[11px] font-medium text-white hover:bg-orange-600">
        充值
      </button>
    </div>
  );
}
