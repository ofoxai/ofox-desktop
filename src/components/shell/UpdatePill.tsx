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
 * A quiet brand row at the sidebar's foot while a newer Ofox Desktop is
 * out. It opens a menu to fetch it or skip it.
 */
export function UpdatePill({
  variant = "row",
}: {
  /** "pill": the narrow window's top bar; "row": the expanded sidebar's
   *  full-width row; "icon": the collapsed sidebar's, its text the
   *  button's name and tooltip. */
  variant?: "pill" | "row" | "icon";
}) {
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

  const label = version
    ? t("shell.update.pill", { version })
    : t("update.bannerTitle");
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        {variant === "icon" ? (
          <button
            type="button"
            aria-label={label}
            title={label}
            className="grid h-[30px] w-[30px] place-items-center rounded-full bg-brand-soft text-brand-ink shadow-[inset_0_0_0_.5px_rgb(var(--brand)/.35)] transition-[background-color,color,transform] duration-150 ease-soft hover:bg-brand hover:text-brand-on active:scale-[.96] data-[state=open]:bg-brand data-[state=open]:text-brand-on"
          >
            <ArrowUpCircle aria-hidden className="h-4 w-4" />
          </button>
        ) : variant === "pill" ? (
          <button
            type="button"
            className="mr-1.5 inline-flex h-[22px] items-center gap-1 whitespace-nowrap rounded-full bg-brand-soft pl-[7px] pr-[9px] text-[11.5px] font-semibold tracking-[.005em] text-brand-ink shadow-[inset_0_0_0_.5px_rgb(var(--brand)/.35)] transition-[background-color,color,transform] duration-150 ease-soft hover:bg-brand hover:text-brand-on active:scale-[.96] data-[state=open]:bg-brand data-[state=open]:text-brand-on"
          >
            <ArrowUpCircle aria-hidden className="h-3 w-3" />
            {label}
          </button>
        ) : (
          <button
            type="button"
            className="mb-1 flex h-[30px] w-full items-center gap-2 whitespace-nowrap rounded-[8px] bg-brand-soft px-2.5 text-[12px] font-semibold text-brand-ink transition-[background-color,color] duration-150 ease-soft hover:bg-brand hover:text-brand-on data-[state=open]:bg-brand data-[state=open]:text-brand-on"
          >
            <ArrowUpCircle aria-hidden className="h-3.5 w-3.5" />
            {label}
          </button>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align={variant === "pill" ? "end" : "start"}
        side={
          variant === "icon" ? "right" : variant === "row" ? "top" : "bottom"
        }
        className="min-w-[150px]"
      >
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
