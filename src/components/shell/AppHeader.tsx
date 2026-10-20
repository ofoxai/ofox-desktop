import { RefreshCw, Settings } from "lucide-react";
import type { Ref } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { SegmentedNav } from "@/components/ui/segmented";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";
import ofoxLogo from "@/assets/icons/ofox-logo.png";
import { SHELL_PAGES, shellPanelId, useShell } from "./ShellContext";
import { UpdatePill } from "./UpdatePill";

/**
 * magpie's `.top`: 46px, the page tabs centred in the window, the update
 * pill, refresh and settings at the right.
 *
 * On macOS the window draws its traffic lights over this header
 * (`titleBarStyle: "Overlay"`, `trafficLightPosition` 14,17 in
 * tauri.conf.json), so it leaves them 78px, shows the logo and name, and
 * drags the window. Windows and Linux keep their native title bar, which
 * already names the app, so the header there is only the tabs and actions.
 */
export function AppHeader({
  settingsButtonRef,
}: {
  settingsButtonRef?: Ref<HTMLButtonElement>;
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const mac = isMac();
  // Tauri's drag script looks only at the element pressed, so the tabs and
  // buttons, which lack the attribute, stay clickable. `no-drag` (index.css)
  // also keeps them out of a CSS `-webkit-app-region: drag` area.
  const drag = mac ? { "data-tauri-drag-region": true } : {};

  return (
    <header
      {...drag}
      className={cn(
        "relative flex h-[46px] shrink-0 items-center gap-2 pr-2.5",
        mac ? "pl-[78px]" : "pl-3.5",
      )}
    >
      {mac && (
        <div
          {...drag}
          className="flex items-center gap-1.5 text-ui font-semibold tracking-[.01em] text-ink"
        >
          <img
            {...drag}
            src={ofoxLogo}
            alt=""
            aria-hidden
            className="h-4 w-4"
          />
          <span {...drag}>Ofox Desktop</span>
        </div>
      )}

      <SegmentedNav
        aria-label={t("shell.nav")}
        items={SHELL_PAGES.map((page) => ({
          value: page,
          label: t(`shell.tabs.${page}`),
          controls: shellPanelId(page),
        }))}
        value={shell.active}
        onChange={shell.setActive}
        className="no-drag absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2"
      />

      <div className="no-drag ml-auto flex items-center gap-0.5">
        <UpdatePill />
        <Button
          variant="icon"
          aria-label={t("shell.refresh")}
          title={t("shell.refresh")}
          aria-busy={shell.refreshing || undefined}
          disabled={!shell.canRefresh}
          onClick={() => void shell.refresh()}
        >
          <RefreshCw
            aria-hidden
            className={cn(
              "h-[15px] w-[15px]",
              shell.refreshing && "animate-spin",
            )}
          />
        </Button>
        <Button
          ref={settingsButtonRef}
          variant="icon"
          aria-label={t("shell.settings")}
          title={t("shell.settings")}
          onClick={shell.openSettings}
        >
          <Settings aria-hidden className="h-[15px] w-[15px]" />
        </Button>
      </div>
    </header>
  );
}
