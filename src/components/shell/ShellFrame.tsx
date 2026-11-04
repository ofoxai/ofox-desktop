import { useLayoutEffect, useRef, type ReactNode, type Ref } from "react";
import { cn } from "@/lib/utils";
import { AppHeader } from "./AppHeader";
import { AppSidebar } from "./AppSidebar";
import { shellPanelId, useShell } from "./ShellContext";
import type { SidebarAccountState } from "./SidebarAccount";

/**
 * The console's frame: a narrow window keeps the top bar with the page
 * tabs; a wide one (full screen, maximised) has the sidebar at the left.
 * The pages sit at the same place in both, so switching keeps every one of
 * them as it was; a page tab that had the focus hands it to the same
 * page's tab in the other form. `--page-x` is the pages' side margin.
 */
export function ShellFrame({
  account,
  settingsButtonRef,
  banner,
  children,
}: {
  account: SidebarAccountState;
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
    <div
      className={cn(
        "flex h-screen w-full bg-canvas",
        wide ? "[--page-x:28px]" : "flex-col [--page-x:16px]",
      )}
    >
      {wide ? (
        <AppSidebar
          account={account}
          settingsButtonRef={settingsButtonRef}
          navRef={nav}
        />
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
