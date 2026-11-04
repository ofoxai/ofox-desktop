import {
  ChartColumn,
  LibraryBig,
  MessagesSquare,
  PanelLeft,
  Settings,
  Sparkles,
  Wrench,
} from "lucide-react";
import {
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
} from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";
import {
  SHELL_GROUPS,
  SHELL_PAGES,
  shellPanelId,
  useShell,
  type ShellPage,
  type ShellSubnav,
} from "./ShellContext";
import { SidebarAccount, type SidebarAccountState } from "./SidebarAccount";
import { UpdatePill } from "./UpdatePill";

const ICONS: Record<ShellPage, ReactNode> = {
  tools: <Wrench />,
  sessions: <MessagesSquare />,
  library: <LibraryBig />,
  assistant: <Sparkles />,
  usage: <ChartColumn />,
};

/** Arrow keys over a vertical list of buttons: the index to move to. */
function step(key: string, index: number, count: number) {
  if (key === "ArrowDown") return (index + 1) % count;
  if (key === "ArrowUp") return (index - 1 + count) % count;
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  return undefined;
}

/**
 * The page navigation at every width: Ofox's account on top, the pages in
 * their groups, the open page's sub-pages under it, the update and the
 * settings at the foot. Collapsed it is an 80px bar of icons (wide enough
 * for macOS's traffic lights), each naming its page and listing its
 * sub-pages in a flyout.
 */
export function AppSidebar({
  account,
  settingsButtonRef,
  navRef,
}: {
  account: SidebarAccountState;
  settingsButtonRef?: Ref<HTMLButtonElement>;
  navRef?: Ref<HTMLElement>;
}) {
  const { t } = useTranslation();
  const shell = useShell();
  const collapsed = shell.sidebarCollapsed;
  const mac = isMac();
  const drag = mac ? { "data-tauri-drag-region": true } : {};
  const pageButtons = useRef<(HTMLButtonElement | null)[]>([]);
  const toggle = (
    <Button
      variant="icon"
      aria-label={t(
        collapsed ? "shell.sidebar.expand" : "shell.sidebar.collapse",
      )}
      title={t(collapsed ? "shell.sidebar.expand" : "shell.sidebar.collapse")}
      onClick={() => shell.setSidebarCollapsed(!collapsed)}
      className="no-drag h-7 w-7"
    >
      <PanelLeft aria-hidden className="h-4 w-4" />
    </Button>
  );
  const tabStop = Math.max(0, SHELL_PAGES.indexOf(shell.active));
  const onPageKey = (event: KeyboardEvent, index: number) => {
    const next = step(event.key, index, SHELL_PAGES.length);
    if (next === undefined) return;
    event.preventDefault();
    pageButtons.current[next]?.focus();
    shell.setActive(SHELL_PAGES[next]);
  };

  return (
    <nav
      ref={navRef}
      aria-label={t("shell.nav")}
      className={cn(
        "relative z-30 flex shrink-0 flex-col border-r border-line-2 bg-canvas pb-3",
        collapsed ? "w-20 items-center px-2" : "w-[216px] px-2.5",
      )}
    >
      <div
        data-testid="sidebar-top"
        {...drag}
        className={cn(
          "flex w-full shrink-0 items-center",
          mac ? "h-[52px]" : "h-11",
          collapsed ? "justify-center" : "justify-end",
          mac && collapsed && "items-end pb-1",
        )}
      >
        {toggle}
      </div>

      <div className={cn("w-full", collapsed ? "mb-2" : "mb-1")}>
        <SidebarAccount state={account} collapsed={collapsed} />
      </div>

      <div
        role="tablist"
        aria-label={t("shell.nav")}
        aria-orientation="vertical"
        className="no-drag flex w-full flex-col"
      >
        {SHELL_GROUPS.map((group, groupIndex) => (
          <div key={group.id} className="flex flex-col gap-0.5">
            {collapsed ? (
              groupIndex > 0 && (
                <span
                  aria-hidden
                  className="mx-auto my-2 block h-px w-6 bg-line"
                />
              )
            ) : (
              <span className="px-2.5 pb-1.5 pt-3.5 text-[11px] text-ink-muted">
                {t(`shell.groups.${group.id}`)}
              </span>
            )}
            {group.pages.map((page) => {
              const index = SHELL_PAGES.indexOf(page);
              const on = shell.active === page;
              const nav = shell.subnavs[page];
              const button = (
                <button
                  type="button"
                  role="tab"
                  aria-selected={on}
                  aria-controls={shellPanelId(page)}
                  aria-label={collapsed ? t(`shell.tabs.${page}`) : undefined}
                  tabIndex={index === tabStop ? 0 : -1}
                  ref={(node) => {
                    pageButtons.current[index] = node;
                  }}
                  onClick={() => {
                    if (!on) shell.setActive(page);
                  }}
                  onKeyDown={(event) => onPageKey(event, index)}
                  className={cn(
                    "flex items-center rounded-[8px] transition-colors duration-150 ease-soft [&_svg]:h-4 [&_svg]:w-4 [&_svg]:flex-none",
                    collapsed
                      ? "mx-auto h-9 w-9 justify-center"
                      : "h-8 w-full gap-2.5 px-2.5 text-[13.5px]",
                    on
                      ? "bg-brand-soft font-semibold text-brand-ink [&_svg]:text-brand"
                      : "text-ink-2 hover:bg-pill/70 hover:text-ink",
                  )}
                >
                  <span aria-hidden className="contents">
                    {ICONS[page]}
                  </span>
                  {!collapsed && (
                    <span className="truncate">{t(`shell.tabs.${page}`)}</span>
                  )}
                </button>
              );
              return (
                <div key={page}>
                  {collapsed ? (
                    <Flyout
                      label={t(`shell.tabs.${page}`)}
                      nav={nav}
                      onPick={(value) => {
                        if (!on) shell.setActive(page);
                        nav?.onChange(value);
                      }}
                    >
                      {button}
                    </Flyout>
                  ) : (
                    button
                  )}
                  {!collapsed && on && nav && (
                    <SubNav label={t(`shell.tabs.${page}`)} nav={nav} />
                  )}
                </div>
              );
            })}
          </div>
        ))}
      </div>

      <div
        className={cn(
          "no-drag mt-auto flex w-full flex-col gap-0.5",
          collapsed && "items-center",
        )}
      >
        <UpdatePill variant={collapsed ? "icon" : "row"} />
        <button
          ref={settingsButtonRef}
          type="button"
          aria-label={t("shell.settings")}
          title={collapsed ? t("shell.settings") : undefined}
          onClick={shell.openSettings}
          className={cn(
            "flex items-center rounded-[8px] text-ink-2 transition-colors hover:bg-pill/70 hover:text-ink [&_svg]:h-4 [&_svg]:w-4",
            collapsed
              ? "h-9 w-9 justify-center"
              : "h-8 w-full gap-2.5 px-2.5 text-[13.5px]",
          )}
        >
          <Settings aria-hidden />
          {!collapsed && t("shell.settings")}
        </button>
      </div>
    </nav>
  );
}

/** The open page's sub-pages, indented under it. */
export function SubNav({ label, nav }: { label: string; nav: ShellSubnav }) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const current = nav.items.findIndex((item) => item.value === nav.value);
  return (
    <div
      role="tablist"
      aria-label={label}
      aria-orientation="vertical"
      className="mb-1 ml-[26px] mt-0.5 flex flex-col gap-px"
    >
      {nav.items.map((item, index) => {
        const on = item.value === nav.value;
        return (
          <button
            key={item.value}
            type="button"
            role="tab"
            aria-selected={on}
            aria-controls={item.controls}
            tabIndex={index === Math.max(0, current) ? 0 : -1}
            ref={(node) => {
              refs.current[index] = node;
            }}
            onClick={() => {
              if (!on) nav.onChange(item.value);
            }}
            onKeyDown={(event) => {
              const next = step(event.key, index, nav.items.length);
              if (next === undefined) return;
              event.preventDefault();
              refs.current[next]?.focus();
              nav.onChange(nav.items[next].value);
            }}
            className={cn(
              "flex h-7 items-center justify-between rounded-[7px] px-2.5 text-[13px] transition-colors",
              on
                ? "font-semibold text-brand-ink"
                : "text-ink-2 hover:bg-pill/70 hover:text-ink",
            )}
          >
            <span className="truncate">{item.label}</span>
            {item.count !== undefined && (
              <span className="text-[11px] font-normal tabular-nums text-ink-muted">
                {item.count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

/**
 * A collapsed page icon's flyout: the page's name, then its sub-pages. It
 * opens on hover and on keyboard focus; ArrowRight enters it, the arrows
 * move, Escape or ArrowLeft go back to the icon.
 */
function Flyout({
  label,
  nav,
  onPick,
  children,
}: {
  label: string;
  nav: ShellSubnav | undefined;
  onPick: (value: string) => void;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const items = useRef<(HTMLButtonElement | null)[]>([]);
  const closing = useRef<number>();
  const show = () => {
    window.clearTimeout(closing.current);
    setOpen(true);
  };
  const hide = () => {
    window.clearTimeout(closing.current);
    closing.current = window.setTimeout(() => setOpen(false), 120);
  };
  items.current.length = nav?.items.length ?? 0;
  // Going back to the icon focuses it; that focus must not reopen the flyout.
  const returning = useRef(false);
  const back = () => {
    setOpen(false);
    returning.current = true;
    root.current?.querySelector<HTMLElement>('[role="tab"]')?.focus();
    returning.current = false;
  };
  return (
    <div
      ref={root}
      className="relative"
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={() => {
        if (!returning.current) show();
      }}
      onBlur={(event) => {
        if (!root.current?.contains(event.relatedTarget as Node | null))
          setOpen(false);
      }}
      onKeyDown={(event) => {
        const inside = items.current.indexOf(
          document.activeElement as HTMLButtonElement,
        );
        if (
          event.key === "Escape" ||
          (event.key === "ArrowLeft" && inside >= 0)
        ) {
          event.preventDefault();
          event.stopPropagation();
          back();
        } else if (event.key === "ArrowRight" && inside < 0) {
          event.preventDefault();
          if (open) items.current[0]?.focus();
          else {
            setOpen(true);
            window.requestAnimationFrame(() => items.current[0]?.focus());
          }
        } else if (inside >= 0) {
          const next = step(event.key, inside, items.current.length);
          if (next === undefined) return;
          event.preventDefault();
          event.stopPropagation();
          items.current[next]?.focus();
        }
      }}
    >
      {children}
      {open && (
        <div
          role="group"
          aria-label={label}
          className="absolute left-full top-0 z-40 ml-2 min-w-[150px] rounded-pop bg-pop-bg p-1 shadow-pop backdrop-blur-xl"
        >
          <div className="px-2.5 py-1.5 text-[12.5px] font-semibold text-ink">
            {label}
          </div>
          {nav?.items.map((item, index) => {
            const on = item.value === nav.value;
            return (
              <button
                key={item.value}
                type="button"
                tabIndex={-1}
                ref={(node) => {
                  items.current[index] = node;
                }}
                onClick={() => {
                  onPick(item.value);
                }}
                className={cn(
                  "flex h-7 w-full items-center justify-between gap-4 rounded-[6px] px-2.5 text-left text-[13px] hover:bg-pill",
                  on ? "font-semibold text-brand-ink" : "text-ink-2",
                )}
              >
                <span>{item.label}</span>
                {item.count !== undefined && (
                  <span className="text-[11px] font-normal tabular-nums text-ink-muted">
                    {item.count}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
