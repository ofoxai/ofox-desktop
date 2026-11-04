import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useShellLayout, type ShellLayout } from "./useShellLayout";

export type ShellPage =
  | "tools"
  | "sessions"
  | "library"
  | "usage"
  | "assistant";

/** The sidebar's groups and their pages, in order. */
export const SHELL_GROUPS: ReadonlyArray<{
  id: "use" | "observe";
  pages: readonly ShellPage[];
}> = [
  { id: "use", pages: ["tools", "sessions", "library", "assistant"] },
  { id: "observe", pages: ["usage"] },
];

/** Every page, in the sidebar's order. */
export const SHELL_PAGES: readonly ShellPage[] = SHELL_GROUPS.flatMap(
  (group) => group.pages,
);

export interface ShellSubnavItem<T extends string = string> {
  value: T;
  label: string;
  count?: number;
  /** The id of the sub-page's panel, for the tab's `aria-controls`. */
  controls: string;
}

/** A page's sub-pages, published to the sidebar; the page keeps the state. */
export interface ShellSubnav<T extends string = string> {
  items: ReadonlyArray<ShellSubnavItem<T>>;
  value: T;
  onChange: (value: T) => void;
}

const SIDEBAR_COLLAPSED_KEY = "ofox.shell.sidebarCollapsed";

function readCollapsed(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

/** The id of a page's panel, for the tab's `aria-controls`. */
export const shellPanelId = (page: ShellPage) => `shell-panel-${page}`;

export type RefreshHandler = () => unknown;

export interface ShellContextValue {
  /** The top bar, or the left rail and roomier pages of a wide window. */
  layout: ShellLayout;
  /** The page on screen. */
  active: ShellPage;
  setActive: (page: ShellPage) => void;
  registerLeaveGuard: (
    page: ShellPage,
    guard: (proceed: () => void) => void,
  ) => () => void;
  sessionLocation: { key: string; request: number } | null;
  locateSession: (key: string) => void;
  consumeSessionLocation: (request: number) => void;
  usageLocation: { key: string; request: number } | null;
  locateUsage: (key: string) => void;
  consumeUsageLocation: (request: number) => void;
  libraryLocation: { kind: "mcp" | "skills"; request: number } | null;
  locateLibrary: (kind: "mcp" | "skills") => void;
  consumeLibraryLocation: (request: number) => void;
  assistantLocation: { tool: string; request: number } | null;
  locateAssistant: (tool: string) => void;
  consumeAssistantLocation: (request: number) => void;
  /** Pages opened at least once. They stay mounted, hidden, after. */
  visited: ReadonlySet<ShellPage>;
  openSettings: () => void;
  /** A page offers its refresh to the header; call the result to withdraw it. */
  registerRefresh: (page: ShellPage, fn: RefreshHandler) => () => void;
  /** The page on screen has a refresh. */
  canRefresh: boolean;
  /** The page on screen is refreshing. */
  refreshing: boolean;
  /** Sub-pages the mounted pages publish (useShellSubnav). */
  subnavs: Partial<Record<ShellPage, ShellSubnav>>;
  /** For useShellSubnav: publish or (null) withdraw a page's sub-pages. */
  publishSubnav: (page: ShellPage, nav: ShellSubnav | null) => void;
  /** The sidebar shows icons only; remembered on this machine. */
  sidebarCollapsed: boolean;
  setSidebarCollapsed: (collapsed: boolean) => void;
  /** Refresh the page on screen; ignored while it already is refreshing. */
  refresh: () => Promise<void>;
}

const ShellContext = createContext<ShellContextValue | null>(null);

function withPage(set: ReadonlySet<ShellPage>, page: ShellPage) {
  return set.has(page) ? set : new Set(set).add(page);
}

function withoutPage(set: ReadonlySet<ShellPage>, page: ShellPage) {
  if (!set.has(page)) return set;
  const next = new Set(set);
  next.delete(page);
  return next;
}

interface ShellProviderProps {
  openSettings: () => void;
  children: ReactNode;
}

/**
 * The console's frame: which page is open, which pages have been opened,
 * and the one header refresh, which runs whatever the open page registered.
 */
export function ShellProvider({ openSettings, children }: ShellProviderProps) {
  const [active, setActiveState] = useState<ShellPage>("tools");
  const [sessionLocation, setSessionLocation] =
    useState<ShellContextValue["sessionLocation"]>(null);
  const locationSequence = useRef(0);
  const [assistantLocation, setAssistantLocation] =
    useState<ShellContextValue["assistantLocation"]>(null);
  const [usageLocation, setUsageLocation] =
    useState<ShellContextValue["usageLocation"]>(null);
  const [libraryLocation, setLibraryLocation] =
    useState<ShellContextValue["libraryLocation"]>(null);
  const [visited, setVisited] = useState<ReadonlySet<ShellPage>>(
    () => new Set<ShellPage>(["tools"]),
  );
  const handlers = useRef(new Map<ShellPage, RefreshHandler>());
  const leaveGuards = useRef(
    new Map<ShellPage, (proceed: () => void) => void>(),
  );
  const activeRef = useRef(active);
  const [registered, setRegistered] = useState<ReadonlySet<ShellPage>>(
    () => new Set(),
  );
  const running = useRef(new Set<ShellPage>());
  const [runningPages, setRunningPages] = useState<ReadonlySet<ShellPage>>(
    () => new Set(),
  );

  const navigate = useCallback((page: ShellPage, after?: () => void) => {
    const commit = () => {
      activeRef.current = page;
      setActiveState(page);
      setVisited((previous) => withPage(previous, page));
      after?.();
    };
    const guard =
      page !== activeRef.current
        ? leaveGuards.current.get(activeRef.current)
        : undefined;
    if (guard) guard(commit);
    else commit();
  }, []);
  const setActive = useCallback(
    (page: ShellPage) => navigate(page),
    [navigate],
  );
  const registerLeaveGuard = useCallback(
    (page: ShellPage, guard: (proceed: () => void) => void) => {
      leaveGuards.current.set(page, guard);
      return () => {
        if (leaveGuards.current.get(page) === guard)
          leaveGuards.current.delete(page);
      };
    },
    [],
  );
  const locateAssistant = useCallback(
    (tool: string) =>
      navigate("assistant", () =>
        setAssistantLocation({
          tool: tool === "chatgpt" ? "codex" : tool,
          request: ++locationSequence.current,
        }),
      ),
    [navigate],
  );
  const consumeAssistantLocation = useCallback(
    (request: number) =>
      setAssistantLocation((current) =>
        current?.request === request ? null : current,
      ),
    [],
  );
  const locateSession = useCallback(
    (key: string) => {
      navigate("sessions", () =>
        setSessionLocation({ key, request: ++locationSequence.current }),
      );
    },
    [navigate],
  );
  const consumeSessionLocation = useCallback((request: number) => {
    setSessionLocation((current) =>
      current?.request === request ? null : current,
    );
  }, []);
  const locateUsage = useCallback(
    (key: string) => {
      navigate("usage", () =>
        setUsageLocation({ key, request: ++locationSequence.current }),
      );
    },
    [navigate],
  );
  const consumeUsageLocation = useCallback((request: number) => {
    setUsageLocation((current) =>
      current?.request === request ? null : current,
    );
  }, []);
  const locateLibrary = useCallback(
    (kind: "mcp" | "skills") => {
      navigate("library", () =>
        setLibraryLocation({ kind, request: ++locationSequence.current }),
      );
    },
    [navigate],
  );
  const consumeLibraryLocation = useCallback((request: number) => {
    setLibraryLocation((current) =>
      current?.request === request ? null : current,
    );
  }, []);

  const registerRefresh = useCallback((page: ShellPage, fn: RefreshHandler) => {
    handlers.current.set(page, fn);
    setRegistered((previous) => withPage(previous, page));
    return () => {
      if (handlers.current.get(page) !== fn) return;
      handlers.current.delete(page);
      setRegistered((previous) => withoutPage(previous, page));
    };
  }, []);

  const refresh = useCallback(async () => {
    const page = active;
    const fn = handlers.current.get(page);
    if (!fn || running.current.has(page)) return;
    running.current.add(page);
    setRunningPages((previous) => withPage(previous, page));
    try {
      await fn();
    } catch (e) {
      // Pages report their own failures; this only keeps the trace.
      console.error(`[Shell] refresh of ${page} failed`, e);
    } finally {
      running.current.delete(page);
      setRunningPages((previous) => withoutPage(previous, page));
    }
  }, [active]);

  const [subnavs, setSubnavs] = useState<
    Partial<Record<ShellPage, ShellSubnav>>
  >({});
  const publishSubnav = useCallback(
    (page: ShellPage, nav: ShellSubnav | null) =>
      setSubnavs((previous) => {
        if (!nav && !previous[page]) return previous;
        const next = { ...previous };
        if (nav) next[page] = nav;
        else delete next[page];
        return next;
      }),
    [],
  );
  const [sidebarCollapsed, setCollapsedState] = useState(readCollapsed);
  const setSidebarCollapsed = useCallback((collapsed: boolean) => {
    setCollapsedState(collapsed);
    try {
      localStorage.setItem(SIDEBAR_COLLAPSED_KEY, collapsed ? "1" : "0");
    } catch {
      // A blocked storage only loses the memory, not the toggle.
    }
  }, []);

  const layout = useShellLayout();
  const value = useMemo<ShellContextValue>(
    () => ({
      layout,
      active,
      setActive,
      registerLeaveGuard,
      assistantLocation,
      locateAssistant,
      consumeAssistantLocation,
      sessionLocation,
      locateSession,
      consumeSessionLocation,
      usageLocation,
      locateUsage,
      consumeUsageLocation,
      libraryLocation,
      locateLibrary,
      consumeLibraryLocation,
      visited,
      openSettings,
      registerRefresh,
      canRefresh: registered.has(active),
      refreshing: runningPages.has(active),
      refresh,
      subnavs,
      publishSubnav,
      sidebarCollapsed,
      setSidebarCollapsed,
    }),
    [
      active,
      setActive,
      registerLeaveGuard,
      assistantLocation,
      locateAssistant,
      consumeAssistantLocation,
      sessionLocation,
      locateSession,
      consumeSessionLocation,
      usageLocation,
      locateUsage,
      consumeUsageLocation,
      libraryLocation,
      locateLibrary,
      consumeLibraryLocation,
      visited,
      openSettings,
      registerRefresh,
      registered,
      runningPages,
      refresh,
      layout,
      subnavs,
      publishSubnav,
      sidebarCollapsed,
      setSidebarCollapsed,
    ],
  );

  return (
    <ShellContext.Provider value={value}>{children}</ShellContext.Provider>
  );
}

export function useShell(): ShellContextValue {
  const shell = useContext(ShellContext);
  if (!shell) throw new Error("useShell must be used inside <ShellProvider>");
  return shell;
}

/** The console's frame, or null outside one (a page rendered on its own). */
export function useOptionalShell(): ShellContextValue | null {
  return useContext(ShellContext);
}

/**
 * Offer `fn` as `page`'s refresh while the calling component is mounted.
 * The latest `fn` runs, so it need not be memoised.
 */
export function useShellRefresh(page: ShellPage, fn: RefreshHandler) {
  const { registerRefresh } = useShell();
  const latest = useRef(fn);
  useLayoutEffect(() => {
    latest.current = fn;
  });
  useEffect(
    () => registerRefresh(page, () => latest.current()),
    [page, registerRefresh],
  );
}

/**
 * Publish `page`'s sub-pages to the sidebar while the caller is mounted.
 * The page keeps the state; the sidebar calls the latest `onChange`, so it
 * need not be memoised. Republished only when the value or the items' text
 * change.
 */
export function useShellSubnav<T extends string>(
  page: ShellPage,
  nav: ShellSubnav<T> | null,
) {
  const { publishSubnav } = useShell();
  const latest = useRef(nav);
  useLayoutEffect(() => {
    latest.current = nav;
  });
  const key = nav
    ? JSON.stringify([
        nav.value,
        nav.items.map((item) => [
          item.value,
          item.label,
          item.count ?? null,
          item.controls,
        ]),
      ])
    : null;
  useLayoutEffect(() => {
    const current = latest.current;
    publishSubnav(
      page,
      current
        ? {
            items: current.items,
            value: current.value,
            onChange: (value) => latest.current?.onChange(value as T),
          }
        : null,
    );
  }, [key, page, publishSubnav]);
  useEffect(() => () => publishSubnav(page, null), [page, publishSubnav]);
}
