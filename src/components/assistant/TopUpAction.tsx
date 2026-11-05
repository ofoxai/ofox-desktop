import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Wallet } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { useOfoxAuth } from "@/hooks/useOfoxAuth";
import { isOfoxBillingManager } from "@/lib/api/ofoxAuth";
import { settingsApi } from "@/lib/api/settings";
import { ofoxWalletUrl } from "@/lib/ofoxUrls";

/** The failures a top-up fixes: a reply's, and a generation's. */
const BALANCE_CODES = new Set([
  "assistant_insufficient_balance",
  "assistant_media_insufficient_credits",
]);

export function needsTopUp(code?: string | null): boolean {
  return Boolean(code && BALANCE_CODES.has(code));
}

/**
 * Beside a failure for want of balance: an owner or admin gets 「去充值」,
 * which opens the wallet of the account's region; a member, who cannot top
 * up, is told to ask an admin. Asking again stays with the failure's card.
 */
export function TopUpAction() {
  const { t } = useTranslation();
  const { apex, isLoading: apexLoading } = useOfoxApex();
  const { status, isLoading } = useOfoxAuth();
  if (isLoading) return null;
  if (!isOfoxBillingManager(status?.user))
    return (
      <p className="text-note text-ink-muted">
        {t("connectivity.askAdminToTopUp")}
      </p>
    );
  return (
    <Button
      variant="secondary"
      // Until the region is read the wallet could be the other region's.
      disabled={apexLoading}
      onClick={() =>
        void settingsApi
          .openExternal(ofoxWalletUrl(apex))
          .catch(() => toast.error(t("notifications.openLinkFailed")))
      }
    >
      <Wallet aria-hidden className="h-3.5 w-3.5" />
      {t("connectivity.topUp")}
    </Button>
  );
}
