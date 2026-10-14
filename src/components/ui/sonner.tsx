import type { CSSProperties } from "react";
import { Toaster as SonnerToaster } from "sonner";
import { useTheme } from "@/components/theme-provider";

// Sonner paints toasts from these variables (its rich colours included); point
// them at the design tokens so a toast is a frosted pop like the menus, and
// success / warning / error use ok / warn / danger rather than Sonner's own
// greens, ambers and blues. The tokens follow the theme on <html>.
const TOKEN_COLORS = {
  "--normal-bg": "var(--pop-bg)",
  "--normal-border": "rgb(var(--line))",
  "--normal-text": "hsl(var(--foreground))",
  "--info-bg": "var(--pop-bg)",
  "--info-border": "rgb(var(--line))",
  "--info-text": "hsl(var(--foreground))",
  "--success-bg": "rgb(var(--ok-soft))",
  "--success-border": "rgb(var(--ok) / 0.3)",
  "--success-text": "rgb(var(--ok))",
  "--warning-bg": "rgb(var(--warn-soft))",
  "--warning-border": "rgb(var(--warn) / 0.3)",
  "--warning-text": "rgb(var(--warn))",
  "--error-bg": "rgb(var(--danger-soft))",
  "--error-border": "rgb(var(--danger) / 0.3)",
  "--error-text": "rgb(var(--danger))",
  "--border-radius": "11px",
} as CSSProperties;

export function Toaster() {
  const { theme } = useTheme();

  // 将应用主题映射到 Sonner 的主题
  // 如果是 "system"，Sonner 会自己处理
  const sonnerTheme = theme === "system" ? "system" : theme;

  return (
    <SonnerToaster
      position="top-center"
      richColors
      theme={sonnerTheme}
      // Radix Dialog 打开时会给 <body> 加 `pointer-events: none`，只有 Dialog
      // 内部恢复指针事件。Toaster 渲染在 Dialog 外的 body 层，会被这条规则
      // 屏蔽——导致在弹窗内触发的 toast 连关闭按钮/action 都点不动。强制把
      // Toaster 容器的 pointer-events 设回 auto，让所有 toast 始终可交互。
      style={{ pointerEvents: "auto", ...TOKEN_COLORS }}
      toastOptions={{
        duration: 2000,
        classNames: {
          toast: "group font-sans text-ctl !shadow-pop backdrop-blur-xl",
          title: "text-ctl font-semibold",
          description: "text-ctl opacity-80",
          closeButton:
            "absolute right-2 top-2 rounded-full p-1 text-ink-muted transition-colors hover:bg-pill hover:text-ink",
          actionButton:
            "!h-6 !rounded-ctl !bg-brand !px-2.5 !text-sub !font-medium !text-brand-on transition-[filter] hover:brightness-[1.08]",
        },
      }}
    />
  );
}
