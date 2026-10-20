import { useTranslation } from "react-i18next";
import { UserAvatar } from "@/components/UserAvatar";
import { Button } from "@/components/ui/button";
import { settingsApi } from "@/lib/api";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { isOfoxBillingManager, type OfoxAuthStatus } from "@/lib/api/ofoxAuth";
import { ofoxAvatarUrl, ofoxWalletUrl } from "@/lib/ofoxUrls";
import { cn } from "@/lib/utils";

interface TrayAccountProps {
  status: OfoxAuthStatus | null;
  /** Below the low-balance threshold: the figure turns to the warning colour. */
  low: boolean;
  onLogin: () => void;
}

/**
 * Who is signed in and what is left, on one line: the balance is what Ofox
 * is about, so it stays in view whatever tab is open. Signed out or expired,
 * the line is the way back in.
 */
export default function TrayAccount({
  status,
  low,
  onLogin,
}: TrayAccountProps) {
  const { t } = useTranslation();
  const { apex } = useOfoxApex();
  const user = status?.user ?? null;
  const state = status?.state ?? "loggedout";
  const balance = user?.balance?.balance;

  if (state !== "active")
    return (
      <div
        data-testid="tray-account"
        className="flex min-h-8 shrink-0 items-center gap-2 px-3.5 pb-2"
      >
        <span className="min-w-0 flex-1 truncate text-sub text-ink-muted">
          {state === "expired"
            ? user?.email
              ? t("uiCopy.expiredEmail", { value1: user.email })
              : t("uiCopy.expired")
            : t("uiCopy.loginHint")}
        </span>
        <Button variant="primary" size="sm" onClick={onLogin}>
          {t(state === "expired" ? "uiCopy.reauth" : "uiCopy.login")}
        </Button>
      </div>
    );

  return (
    <div
      data-testid="tray-account"
      className="flex min-h-8 shrink-0 items-center gap-2 px-3.5 pb-2"
    >
      <UserAvatar
        avatarUrl={ofoxAvatarUrl(user?.avatar_url)}
        name={user?.name}
        email={user?.email}
        className="h-5 w-5"
        fallbackTextClassName="text-[10px]"
      />
      <span
        className="min-w-0 flex-1 truncate text-sub text-ink-muted"
        title={user?.email ?? undefined}
      >
        {user?.email || user?.name}
      </span>
      {isOfoxBillingManager(user) && (
        <button
          type="button"
          onClick={() => void settingsApi.openExternal(ofoxWalletUrl(apex))}
          className="flex-none text-note text-brand-ink hover:underline"
        >
          {t("uiCopy.topUp")}
        </button>
      )}
      <span
        title={t("uiCopy.accountBalance")}
        className={cn(
          "flex-none text-ui font-semibold tabular-nums",
          low ? "text-warn" : "text-ink",
        )}
      >
        {typeof balance === "number" ? formatUsd(balance) : "—"}
      </span>
    </div>
  );
}

function formatUsd(value: number): string {
  return `${value < 0 ? "-" : ""}$${Math.abs(value).toFixed(2)}`;
}
