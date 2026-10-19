import { useTranslation } from "react-i18next";
import { useCallback, useEffect, type ReactNode } from "react";
import { exit } from "@tauri-apps/plugin-process";
import { toast } from "sonner";
import { isMac } from "@/lib/platform";
import { openMainWindow } from "./trayActions";

const FOOT_BUTTON =
  "inline-flex h-6 items-center gap-1.5 rounded-[6px] px-[7px] text-ctl text-ink-ctl transition-colors hover:bg-pill hover:text-ink";

/**
 * magpie's panel `.foot`: one line under the list, what the panel is about
 * on the left (`lead`), the main window and quitting on the right.
 */
export default function BottomMenu({ lead }: { lead?: ReactNode }) {
  const { t } = useTranslation();
  const shortcut = isMac() ? "⌘O" : "Ctrl+O";

  const handleOpenMain = useCallback(async () => {
    try {
      await openMainWindow();
    } catch (e) {
      console.error("[BottomMenu] show_main_window failed", e);
      toast.error(t("uiCopy.openMainFailed"));
    }
  }, [t]);

  // ⌘O on macOS, Ctrl+O elsewhere, as the button shows.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const modifier = isMac() ? event.metaKey : event.ctrlKey;
      if (
        modifier &&
        !event.altKey &&
        !event.shiftKey &&
        event.key.toLowerCase() === "o"
      ) {
        event.preventDefault();
        void handleOpenMain();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [handleOpenMain]);

  const handleQuit = useCallback(async () => {
    try {
      await exit(0);
    } catch (e) {
      console.error("[BottomMenu] exit failed", e);
    }
  }, []);

  return (
    <footer className="flex h-9 shrink-0 items-center gap-0.5 border-t border-line-2 pl-2 pr-2">
      <div className="min-w-0 flex-1">{lead}</div>
      <button
        type="button"
        onClick={() => void handleOpenMain()}
        className={FOOT_BUTTON}
      >
        {t("tray.openOfox")}
        <kbd className="font-sans text-note text-ink-faint">{shortcut}</kbd>
      </button>
      <button
        type="button"
        onClick={() => void handleQuit()}
        className={FOOT_BUTTON}
      >
        {t("tray.quit")}
      </button>
    </footer>
  );
}
