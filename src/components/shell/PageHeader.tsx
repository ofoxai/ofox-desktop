import { RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";
import { useShell } from "./ShellContext";

/** A page's content column: room at the sides, centred on a large window. */
export const PAGE_BODY = "mx-auto w-full max-w-[1120px] px-7";
/** The same for pages of tables and charts, which use more width. */
export const PAGE_BODY_WIDE = "mx-auto w-full max-w-[1440px] px-7";

/**
 * A page's title row: its name, one line on what it is for, and its
 * actions at the right. It stays at the top while the page scrolls and,
 * with no title bar above it, drags the window on macOS. The top padding
 * sets the title level with the sidebar's account card.
 */
export function PageHeader({
  title,
  description,
  actions,
  wide,
  className,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  /** Use the wide content column (PAGE_BODY_WIDE). */
  wide?: boolean;
  className?: string;
}) {
  const drag = isMac() ? { "data-tauri-drag-region": true } : {};
  return (
    <header
      data-page-header=""
      {...drag}
      className={cn("sticky top-0 z-20 shrink-0 bg-canvas", className)}
    >
      <div
        {...drag}
        className={cn(
          wide ? PAGE_BODY_WIDE : PAGE_BODY,
          "flex items-start gap-4 pb-4 pt-9",
        )}
      >
        <div {...drag} className="min-w-0 flex-1">
          <h1
            {...drag}
            className="truncate text-[20px] font-semibold leading-7 tracking-[-0.01em] text-ink"
          >
            {title}
          </h1>
          {description && (
            <p {...drag} className="mt-1 text-[12px] text-ink-muted">
              {description}
            </p>
          )}
        </div>
        {actions && (
          <div className="no-drag flex min-h-7 flex-none items-center gap-2">
            {actions}
          </div>
        )}
      </div>
    </header>
  );
}

/** Refresh the page on screen (what it registered with useShellRefresh). */
export function ShellRefreshButton() {
  const { t } = useTranslation();
  const shell = useShell();
  return (
    <Button
      variant="icon"
      aria-label={t("shell.refresh")}
      title={t("shell.refresh")}
      aria-busy={shell.refreshing || undefined}
      disabled={!shell.canRefresh}
      onClick={() => void shell.refresh()}
      className="h-7 w-7"
    >
      <RefreshCw
        aria-hidden
        className={cn(
          "h-[15px] w-[15px]",
          shell.refreshing && "animate-spin motion-reduce:animate-none",
        )}
      />
    </Button>
  );
}
