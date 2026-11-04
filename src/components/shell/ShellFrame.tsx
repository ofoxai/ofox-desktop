import type { ReactNode, Ref } from "react";
import { AppSidebar } from "./AppSidebar";
import type { SidebarAccountState } from "./SidebarAccount";

/**
 * The console's frame: the sidebar at the left at every width, and beside
 * it the banner over the pages. The pages never move between parents, so
 * collapsing the sidebar keeps every one of them as it was.
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
  return (
    <div className="flex h-screen w-full bg-canvas">
      <AppSidebar account={account} settingsButtonRef={settingsButtonRef} />
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {banner}
        <main className="min-h-0 flex-1">{children}</main>
      </div>
    </div>
  );
}
