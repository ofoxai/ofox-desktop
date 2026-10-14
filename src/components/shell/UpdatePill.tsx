import { ArrowUpCircle } from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { useUpdate } from "@/contexts/UpdateContext";
import { settingsApi } from "@/lib/api";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

/**
 * magpie's `.update`: a quiet brand pill before the header's icons while a
 * newer Ofox Desktop is out. It opens a menu to fetch it or skip it.
 */
export function UpdatePill() {
  const { t } = useTranslation();
  const { hasUpdate, isDismissed, updateInfo, dismissUpdate } = useUpdate();
  if (!hasUpdate || isDismissed) return null;

  const version = updateInfo?.availableVersion;
  const downloadUrl = updateInfo?.downloadUrl;
  const openDownload = () => {
    if (!downloadUrl) return;
    settingsApi.openExternal(downloadUrl).catch((e) => {
      console.error("[UpdatePill] open download url failed", e);
      toast.error(t("shell.update.openFailed"));
    });
  };

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="mr-1.5 inline-flex h-[22px] items-center gap-1 whitespace-nowrap rounded-full bg-brand-soft pl-[7px] pr-[9px] text-[11.5px] font-semibold tracking-[.005em] text-brand-ink shadow-[inset_0_0_0_.5px_rgb(var(--brand)/.35)] transition-[background-color,color,transform] duration-150 ease-soft hover:bg-brand hover:text-brand-on active:scale-[.96] data-[state=open]:bg-brand data-[state=open]:text-brand-on"
        >
          <ArrowUpCircle aria-hidden className="h-3 w-3" />
          {version
            ? t("shell.update.pill", { version })
            : t("update.bannerTitle")}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-[150px]">
        <DropdownMenuItem disabled={!downloadUrl} onSelect={openDownload}>
          {t("shell.update.now")}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => dismissUpdate()}>
          {t("update.skipThisVersion")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
