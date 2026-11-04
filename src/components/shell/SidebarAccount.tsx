import { Loader2, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { UserAvatar } from "@/components/UserAvatar";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { settingsApi } from "@/lib/api";
import { isOfoxBillingManager } from "@/lib/api/ofoxAuth";
import { ofoxAvatarUrl, ofoxWalletUrl } from "@/lib/ofoxUrls";
import { cn } from "@/lib/utils";
import ofoxLogo from "@/assets/icons/ofox-logo.png";
import { useShell } from "./ShellContext";

export type SidebarAccountState =
  | { kind: "signed"; account: OfoxAccount }
  | {
      kind: "local";
      /** Signed in but tools not bound yet: offer binding, not sign-in. */
      signedIn: boolean;
      onAction: () => void;
    };

/**
 * The top of the sidebar, Ofox's own: who is signed in and what is left in
 * the wallet. It opens a menu to top up (billing managers only), refresh
 * the balance or open the settings. Without a bound account it is the one
 * way on: sign in, or bind tools.
 */
export function SidebarAccount({
  state,
  collapsed,
}: {
  state: SidebarAccountState;
  collapsed: boolean;
}) {
  const { t } = useTranslation();
  if (state.kind === "local") {
    const label = t(
      state.signedIn ? "toolCatalog.bindOptional" : "toolCatalog.login",
    );
    return collapsed ? (
      <Button
        variant="icon"
        aria-label={label}
        title={label}
        onClick={state.onAction}
        className="no-drag mx-auto h-9 w-9 rounded-full bg-brand-soft hover:bg-brand-soft"
      >
        <img src={ofoxLogo} alt="" aria-hidden className="h-4 w-4" />
      </Button>
    ) : (
      <Button
        variant="primary"
        onClick={state.onAction}
        className="no-drag w-full"
      >
        {label}
      </Button>
    );
  }
  return <SignedAccount account={state.account} collapsed={collapsed} />;
}

function SignedAccount({
  account,
  collapsed,
}: {
  account: OfoxAccount;
  collapsed: boolean;
}) {
  const { t } = useTranslation();
  const { apex } = useOfoxApex();
  const shell = useShell();
  const { user } = account;
  const balance = user?.balance?.balance;
  const spent = typeof balance === "number" && balance <= 0;
  const name =
    user?.name ??
    t(
      account.retrying
        ? "console.balance.loadingName"
        : "console.balance.userFallback",
    );
  const amount =
    typeof balance === "number" && !Number.isNaN(balance)
      ? `$${balance.toFixed(2)}`
      : "—";
  const avatar = user ? (
    <UserAvatar
      avatarUrl={ofoxAvatarUrl(user.avatar_url)}
      name={user.name}
      email={user.email}
      className="h-7 w-7 flex-none"
      fallbackTextClassName="text-ctl"
    />
  ) : (
    <span className="grid h-7 w-7 flex-none place-items-center rounded-full bg-brand-soft">
      <img src={ofoxLogo} alt="" aria-hidden className="h-4 w-4" />
    </span>
  );

  const menu = (
    <DropdownMenuContent
      align="start"
      side={collapsed ? "right" : "bottom"}
      className="min-w-[170px]"
    >
      {isOfoxBillingManager(user) && (
        <DropdownMenuItem
          onSelect={() =>
            void settingsApi.openExternal(ofoxWalletUrl(apex)).catch((e) => {
              console.error("[SidebarAccount] open wallet failed", e);
            })
          }
        >
          {t("console.balance.topUp")}
        </DropdownMenuItem>
      )}
      <DropdownMenuItem onSelect={() => void account.refreshBalance()}>
        {t("shell.account.refresh")}
      </DropdownMenuItem>
      <DropdownMenuItem onSelect={shell.openSettings}>
        {t("shell.account.settings")}
      </DropdownMenuItem>
    </DropdownMenuContent>
  );

  if (collapsed)
    return (
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={`${name} · ${t("shell.account.balance", { amount })}`}
            title={`${name} · ${t("shell.account.balance", { amount })}`}
            className="no-drag mx-auto grid h-9 w-9 place-items-center rounded-full hover:bg-pill"
          >
            {avatar}
          </button>
        </DropdownMenuTrigger>
        {menu}
      </DropdownMenu>
    );

  return (
    <div
      className={cn(
        "no-drag flex items-center rounded-[10px] border bg-card",
        spent ? "border-danger/40" : "border-line",
      )}
    >
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="flex min-w-0 flex-1 items-center gap-2.5 rounded-[10px] px-2.5 py-2 text-left transition-colors hover:bg-pill/50"
          >
            {avatar}
            <span className="min-w-0 flex-1 leading-tight">
              <span className="block truncate text-[12.5px] font-semibold text-ink">
                {name}
              </span>
              {!account.loadFailed && (
                <span
                  className={cn(
                    "mt-0.5 flex items-center gap-1 truncate text-[11.5px] tabular-nums",
                    spent ? "text-danger" : "text-brand-ink",
                  )}
                >
                  {account.retrying ? (
                    <>
                      <Loader2 aria-hidden className="h-3 w-3 animate-spin" />
                      {account.retryAttempt > 0
                        ? t("console.balance.loadingUserAttempt", {
                            attempt: account.retryAttempt,
                          })
                        : t("console.balance.loadingUser")}
                    </>
                  ) : (
                    t("shell.account.balance", { amount })
                  )}
                </span>
              )}
            </span>
          </button>
        </DropdownMenuTrigger>
        {menu}
      </DropdownMenu>
      {account.loadFailed && (
        <button
          type="button"
          aria-label={t("console.balance.loadFailed")}
          title={t("console.balance.loadFailed")}
          onClick={() => void account.retryUser()}
          className="mr-2 grid h-6 w-6 flex-none place-items-center rounded-[6px] text-brand-ink hover:bg-pill"
        >
          <RefreshCw aria-hidden className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}
