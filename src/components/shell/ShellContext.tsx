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

export type ShellPage = "tools" | "sessions" | "library" | "usage";

/** The header's tabs, in order. */
export const SHELL_PAGES: readonly ShellPage[] = [
  "tools",
  "sessions",
  "library",
  "usage",
];

/** The id of a page's panel, for the tab's `aria-controls`. */
export const shellPanelId = (page: ShellPage) => `shell-panel-${page}`;

export type RefreshHandler = () => unknown;

export interface ShellContextValue {
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
  /** Pages opened at least once. They stay mounted, hidden, after. */
  visited: ReadonlySet<ShellPage>;
  openSettings: () => void;
  /** A page offers its refresh to the header; call the result to withdraw it. */
  registerRefresh: (page: ShellPage, fn: RefreshHandler) => () => void;
  /** The page on screen has a refresh. */
  canRefresh: boolean;
  /** The page on screen is refreshing. */
  refreshing: boolean;
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

  const value = useMemo<ShellContextValue>(
    () => ({
      active,
      setActive,
      registerLeaveGuard,
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
    }),
    [
      active,
      setActive,
      registerLeaveGuard,
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
