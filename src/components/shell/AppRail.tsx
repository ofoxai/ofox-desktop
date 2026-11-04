import {
  ChartColumn,
  LibraryBig,
  MessagesSquare,
  RefreshCw,
  Settings,
  Sparkles,
  Wrench,
} from "lucide-react";
import type { Ref } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { RailNav } from "@/components/ui/segmented";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";
import ofoxLogo from "@/assets/icons/ofox-logo.png";
import {
  SHELL_PAGES,
  shellPanelId,
  useShell,
  type ShellPage,
} from "./ShellContext";
import { UpdatePill } from "./UpdatePill";

const ICONS: Record<ShellPage, JSX.Element> = {
  tools: <Wrench />,
  sessions: <MessagesSquare />,
  library: <LibraryBig />,
  usage: <ChartColumn />,
  assistant: <Sparkles />,
};

/**
 * The page navigation of a wide window: an 80px rail at the left, the page
 * tabs down it and the update, refresh and settings at its foot.
 *
 * On macOS the traffic lights stay where the top bar had them (14,17): the
 * rail is wide enough for them, and its top strip, holding nothing else,
 * drags the window.
 */
export function AppRail({
  settingsButtonRef,
  navRef,
}: {
  settingsButtonRef?: Ref<HTMLButtonElement>;
  navRef?: Ref<HTMLElement>;
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const mac = isMac();
  const drag = mac ? { "data-tauri-drag-region": true } : {};
  return (
    <nav
      ref={navRef}
      aria-label={t("shell.nav")}
      className="flex w-20 shrink-0 flex-col items-center bg-canvas pb-3"
    >
      <div {...drag} className={cn("w-full shrink-0", mac ? "h-10" : "h-3")} />
      <img
        {...drag}
        src={ofoxLogo}
        alt=""
        aria-hidden
        className="mb-3 h-6 w-6 shrink-0"
      />
      <RailNav
        aria-label={t("shell.nav")}
        items={SHELL_PAGES.map((page) => ({
          value: page,
          label: t(`shell.tabs.${page}`),
          controls: shellPanelId(page),
        }))}
        icons={ICONS}
        value={shell.active}
        onChange={shell.setActive}
        className="no-drag"
      />
      <div className="no-drag mt-auto flex flex-col items-center gap-1">
        <UpdatePill compact />
        <Button
          variant="icon"
          aria-label={t("shell.refresh")}
          title={t("shell.refresh")}
          aria-busy={shell.refreshing || undefined}
          disabled={!shell.canRefresh}
          onClick={() => void shell.refresh()}
          className="h-[30px] w-[30px]"
        >
          <RefreshCw
            aria-hidden
            className={cn("h-4 w-4", shell.refreshing && "animate-spin")}
          />
        </Button>
        <Button
          ref={settingsButtonRef}
          variant="icon"
          aria-label={t("shell.settings")}
          title={t("shell.settings")}
          onClick={shell.openSettings}
          className="h-[30px] w-[30px]"
        >
          <Settings aria-hidden className="h-4 w-4" />
        </Button>
      </div>
    </nav>
  );
}
