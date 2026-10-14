import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation();
  const title =
    variant === "expired" ? t("uiCopy.expired") : t("uiCopy.loggedOut");
  const detail =
    variant === "expired" ? t("uiCopy.expiredHint") : t("uiCopy.loggedOutHint");
  const action = variant === "expired" ? t("uiCopy.reauth") : t("uiCopy.login");

  return (
    <div className="mx-3.5 mb-1 flex items-center gap-2.5 rounded-lg border border-warn/25 bg-warn-soft px-2.5 py-2 ">
      <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-warn text-[13px] font-bold text-brand-on">
        !
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[11px] font-semibold text-warn ">{title}</div>
        <div className="text-[10px] leading-tight text-warn ">{detail}</div>
      </div>
      <button
        type="button"
        onClick={onReauth}
        className="shrink-0 rounded-md bg-brand px-2.5 py-0.5 text-[11px] font-medium text-brand-on hover:bg-brand"
      >
        {action}
      </button>
    </div>
  );
}
