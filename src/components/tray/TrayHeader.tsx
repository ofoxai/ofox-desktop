import { AppWindow, RefreshCw, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import ofoxLogo from "@/assets/icons/ofox-logo.png";
import { OFOX_OPEN_SETTINGS_EVENT, openMainWindow } from "./trayActions";

interface TrayHeaderProps {
  refreshing: boolean;
  onRefresh: () => void;
}

/** magpie's panel `.top`: the brand, then refresh, the main window and settings. */
export default function TrayHeader({ refreshing, onRefresh }: TrayHeaderProps) {
  const { t } = useTranslation();
  const open = (request?: string) =>
    void openMainWindow(request).catch((e) => {
      console.error("[TrayHeader] open main window failed", e);
      toast.error(
        t(request ? "uiCopy.openSettingsFailed" : "uiCopy.openMainFailed"),
      );
    });
  return (
    <header className="flex h-[46px] shrink-0 items-center gap-2 pl-3.5 pr-2.5">
      <img src={ofoxLogo} alt="" className="h-[18px] w-[18px]" />
      <span className="text-ui font-semibold text-ink">Ofox</span>
      <span className="flex-1" />
      <Button
        variant="icon"
        aria-label={t("tray.refresh")}
        title={t("tray.refresh")}
        aria-busy={refreshing || undefined}
        disabled={refreshing}
        onClick={onRefresh}
      >
        <RefreshCw
          aria-hidden
          className={cn("h-3.5 w-3.5", refreshing && "animate-spin")}
        />
      </Button>
      <Button
        variant="icon"
        aria-label={t("uiCopy.openMain")}
        title={t("uiCopy.openMain")}
        onClick={() => open()}
      >
        <AppWindow aria-hidden className="h-3.5 w-3.5" />
      </Button>
      <Button
        variant="icon"
        aria-label={t("uiCopy.openPreferences")}
        title={t("uiCopy.openPreferences")}
        onClick={() => open(OFOX_OPEN_SETTINGS_EVENT)}
      >
        <Settings aria-hidden className="h-3.5 w-3.5" />
      </Button>
    </header>
  );
}
