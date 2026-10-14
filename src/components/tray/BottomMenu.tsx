import { useTranslation } from "react-i18next";
import { useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { exit } from "@tauri-apps/plugin-process";
import { toast } from "sonner";
import { isMac } from "@/lib/platform";

/**
 * Tray popover 跟主窗口通信用的事件名。主窗口在 ConsolePage 内监听，
 * 收到后打开 OfoxSettingsDialog。命名沿用项目其它 ofox-* 风格。
 */
const OFOX_OPEN_SETTINGS_EVENT = "ofox-open-settings";

export default function BottomMenu() {
  const { t } = useTranslation();
  const hideSelf = useCallback(async () => {
    try {
      await getCurrentWindow().hide();
    } catch (e) {
      // 失焦自动隐藏兜底，问题不大
      console.error("[BottomMenu] hide popover failed", e);
    }
  }, []);

  const handleOpenMain = useCallback(async () => {
    try {
      await invoke("show_main_window");
      await hideSelf();
    } catch (e) {
      console.error("[BottomMenu] show_main_window failed", e);
      toast.error(t("uiCopy.openMainFailed"));
    }
  }, [hideSelf]);

  // ⌘O on macOS, Ctrl+O elsewhere, as the menu item shows.
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

  const handleOpenSettings = useCallback(async () => {
    try {
      await invoke("show_main_window");
      await emit(OFOX_OPEN_SETTINGS_EVENT);
      await hideSelf();
    } catch (e) {
      console.error("[BottomMenu] open settings failed", e);
      toast.error(t("uiCopy.openSettingsFailed"));
    }
  }, [hideSelf]);

  const handleQuit = useCallback(async () => {
    try {
      await exit(0);
    } catch (e) {
      console.error("[BottomMenu] exit failed", e);
    }
  }, []);

  const items: Array<{
    label: string;
    shortcut?: string;
    onClick: () => void | Promise<void>;
  }> = [
    {
      label: t("uiCopy.openMain"),
      shortcut: isMac() ? "⌘O" : "Ctrl+O",
      onClick: handleOpenMain,
    },
    {
      label: t("uiCopy.openPreferences"),
      onClick: handleOpenSettings,
    },
  ];

  return (
    <div className="border-t border-line">
      {items.map((item) => (
        <button
          key={item.label}
          onClick={() => void item.onClick()}
          className="flex w-full items-center justify-between px-3.5 py-2 text-[12px] text-ink hover:bg-accent/50"
        >
          <span>{item.label}</span>
          {item.shortcut && (
            <span className="text-[11px] text-ink-muted">{item.shortcut}</span>
          )}
        </button>
      ))}
      <button
        onClick={() => void handleQuit()}
        className="flex w-full items-center px-3.5 py-2 text-[12px] text-ink-muted hover:bg-accent/50"
      >
        {t("uiCopy.quit")}
      </button>
    </div>
  );
}
