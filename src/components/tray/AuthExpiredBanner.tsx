interface AuthExpiredBannerProps {
  /** Called when the user clicks "重新登录"; should kick off device flow. */
  onReauth: () => void;
  /** Differentiates "expired" vs "logged out" copy; layout is shared. */
  variant: "expired" | "loggedOut";
}

/**
 * The orange "session expired" call-to-action that lives at the top of the
 * popover whenever the auth state is not Active. Visually matches the
 * existing low-balance banner so the layout doesn't shift between states.
 */
export default function AuthExpiredBanner({
  onReauth,
  variant,
}: AuthExpiredBannerProps) {
  const title = variant === "expired" ? "登录已过期" : "未登录";
  const detail =
    variant === "expired"
      ? "余额与用量需重新登录后才会更新"
      : "登录后查看账户余额与用量";
  const action = variant === "expired" ? "重新登录" : "登录";

  return (
    <div className="mx-3.5 mb-1 flex items-center gap-2.5 rounded-lg border border-[#e8d5a3] bg-gradient-to-l from-[#f5dfa0] to-[#faf3e0] px-2.5 py-2 dark:border-amber-500/30 dark:from-amber-600/25 dark:to-amber-500/5">
      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-amber-400 text-[13px] font-bold text-white">
        !
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[11px] font-semibold text-amber-800 dark:text-amber-300">
          {title}
        </div>
        <div className="text-[10px] leading-tight text-amber-700/80 dark:text-amber-400/70">
          {detail}
        </div>
      </div>
      <button
        type="button"
        onClick={onReauth}
        className="shrink-0 rounded-md bg-orange-500 px-2.5 py-0.5 text-[11px] font-medium text-white hover:bg-orange-600"
      >
        {action}
      </button>
    </div>
  );
}
