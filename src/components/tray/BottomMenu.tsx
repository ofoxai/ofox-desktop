import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { exit } from "@tauri-apps/plugin-process";
import { toast } from "sonner";

import { toolHealthApi } from "@/lib/api/toolHealth";

/**
 * Tray popover 跟主窗口通信用的事件名。主窗口在 ConsolePage 内监听，
 * 收到后打开 OfoxSettingsDialog。命名沿用项目其它 ofox-* 风格。
 */
const OFOX_OPEN_SETTINGS_EVENT = "ofox-open-settings";

export default function BottomMenu() {
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
      toast.error("打开主窗口失败");
    }
  }, [hideSelf]);

  const handleRescanTools = useCallback(async () => {
    try {
      await toolHealthApi.triggerNow();
      toast.success("工具检测完成");
    } catch (e) {
      console.error("[BottomMenu] triggerNow failed", e);
      toast.error("工具检测失败");
    }
  }, []);

  const handleOpenSettings = useCallback(async () => {
    try {
      await invoke("show_main_window");
      await emit(OFOX_OPEN_SETTINGS_EVENT);
      await hideSelf();
    } catch (e) {
      console.error("[BottomMenu] open settings failed", e);
      toast.error("打开偏好设置失败");
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
      label: "打开 Ofox 主窗口",
      shortcut: "⌘O",
      onClick: handleOpenMain,
    },
    {
      // 文案改为"重新检测"，与背后实际行为（运行 max_tokens=1 的健康探针）匹配。
      // 真正的"工具发现"是 onboarding 的多步流程，popover 一键无法完成。
      label: "重新检测工具",
      onClick: handleRescanTools,
    },
    {
      label: "偏好设置...",
      onClick: handleOpenSettings,
    },
  ];

  return (
    <div className="border-t border-border">
      {items.map((item) => (
        <button
          key={item.label}
          onClick={() => void item.onClick()}
          className="flex w-full items-center justify-between px-3.5 py-2 text-[12px] text-foreground hover:bg-accent/50"
        >
          <span>{item.label}</span>
          {item.shortcut && (
            <span className="text-[11px] text-muted-foreground">
              {item.shortcut}
            </span>
          )}
        </button>
      ))}
      <button
        onClick={() => void handleQuit()}
        className="flex w-full items-center px-3.5 py-2 text-[12px] text-muted-foreground hover:bg-accent/50"
      >
        退出 Ofox
      </button>
    </div>
  );
}
