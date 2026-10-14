import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { BalanceCard } from "@/components/console/BalanceCard";
import { useShellRefresh } from "@/components/shell/ShellContext";
import { Button } from "@/components/ui/button";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { settingsApi } from "@/lib/api";
import { ofoxDashboardUrl } from "@/lib/ofoxUrls";

interface UsagePageProps {
  account: OfoxAccount;
}

/**
 * The 用量 page. For now only the balance and a link to the web dashboard;
 * the trend, KPIs and breakdowns come with the usage overview. The header's
 * refresh refreshes the balance.
 */
export function UsagePage({ account }: UsagePageProps) {
  const { t } = useTranslation();
  const { apex } = useOfoxApex();
  useShellRefresh("usage", account.refreshBalance);

  const openDashboard = () => {
    settingsApi.openExternal(ofoxDashboardUrl(apex)).catch((e) => {
      console.error("[UsagePage] open dashboard failed", e);
      toast.error(t("shell.openFailed"));
    });
  };

  return (
    <div className="flex flex-col gap-3">
      <BalanceCard account={account} />
      <Button
        variant="link"
        className="h-auto self-start px-0 text-note"
        onClick={openDashboard}
      >
        {t("shell.usage.webDetails")}
      </Button>
    </div>
  );
}
