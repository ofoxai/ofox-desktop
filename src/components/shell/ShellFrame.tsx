import { useLayoutEffect, useRef, type ReactNode, type Ref } from "react";
import { cn } from "@/lib/utils";
import { AppHeader } from "./AppHeader";
import { AppRail } from "./AppRail";
import { shellPanelId, useShell } from "./ShellContext";

/**
 * The console's frame: the page navigation (the top bar, or the left rail
 * of a wide window), the banner and the pages. The pages sit at the same
 * place in both, so switching keeps every one of them as it was; a page tab
 * that had the focus hands it to the same page's tab in the other form.
 */
export function ShellFrame({
  settingsButtonRef,
  banner,
  children,
}: {
  settingsButtonRef?: Ref<HTMLButtonElement>;
  banner: ReactNode;
  /** The pages. */
  children: ReactNode;
}) {
  const shell = useShell();
  const wide = shell.layout === "wide";
  const nav = useRef<HTMLElement>(null);
  // Read while rendering the switch, before the old navigation is removed.
  const shown = useRef({ layout: shell.layout, focused: false });
  if (shown.current.layout !== shell.layout)
    shown.current = {
      layout: shell.layout,
      focused: Boolean(nav.current?.contains(document.activeElement)),
    };
  useLayoutEffect(() => {
    if (!shown.current.focused) return;
    shown.current.focused = false;
    nav.current
      ?.querySelector<HTMLElement>(
        `[role="tab"][aria-controls="${shellPanelId(shell.active)}"]`,
      )
      ?.focus();
    // Only a switch hands the focus over.
  }, [shell.layout]);
  return (
    <div className={cn("flex h-screen w-full bg-canvas", !wide && "flex-col")}>
      {wide ? (
        <AppRail settingsButtonRef={settingsButtonRef} navRef={nav} />
      ) : (
        <AppHeader settingsButtonRef={settingsButtonRef} navRef={nav} />
      )}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {banner}
        <main className="min-h-0 flex-1">{children}</main>
      </div>
    </div>
  );
}
