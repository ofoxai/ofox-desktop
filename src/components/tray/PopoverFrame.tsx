import type { ReactNode } from "react";
import { isMac } from "@/lib/platform";

/**
 * Tray popover 的外框。
 *
 * - macOS：透明窗口里浮着一张半透明圆角卡片（窗口阴影已在 Rust 侧关闭）。
 * - Windows / Linux：窗口本身就是面板，不透明、铺满。系统会给无边框窗口加
 *   边框和阴影，透明边距会被描成一圈框；Windows 11 还会给窗口本身加圆角。
 */
export default function PopoverFrame({ children }: { children: ReactNode }) {
  if (!isMac()) {
    return (
      <div className="flex h-screen w-full flex-col overflow-hidden bg-popover">
        {children}
      </div>
    );
  }
  return (
    <div className="h-screen w-full bg-transparent p-4">
      <div
        className="flex h-full flex-col overflow-hidden rounded-xl border border-border/50 shadow-[0_4px_12px_rgba(0,0,0,0.12)] backdrop-blur-xl"
        style={{ backgroundColor: "hsl(var(--popover) / 0.92)" }}
      >
        {children}
      </div>
    </div>
  );
}
