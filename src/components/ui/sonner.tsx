import { Toaster as SonnerToaster } from "sonner";
import { useTheme } from "@/components/theme-provider";

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
      style={{ pointerEvents: "auto" }}
      toastOptions={{
        duration: 2000,
        classNames: {
          toast:
            "group rounded-md border bg-background text-foreground shadow-lg",
          title: "text-sm font-semibold",
          description: "text-sm text-muted-foreground",
          closeButton:
            "absolute right-2 top-2 rounded-full p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground",
          actionButton:
            "rounded-md bg-primary px-3 py-1 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90",
        },
      }}
    />
  );
}
