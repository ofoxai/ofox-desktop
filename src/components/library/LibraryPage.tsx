import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { libraryApi, type LibraryKind } from "@/lib/api/library";
import { useShell, useShellRefresh } from "@/components/shell/ShellContext";
import { SegmentedNav } from "@/components/ui/segmented";
import {
  InteractionScope,
  useInteractionActive,
} from "@/components/ui/interaction-scope";
import { Button } from "@/components/ui/button";
import { Compass } from "lucide-react";
import { ResourcePackDialog } from "./ResourcePackDialog";
import { LibrarySection } from "./LibrarySection";
import { LibraryScenarioWizard } from "./LibraryScenarioWizard";
import { libraryScroller, type LibraryLeaveGuard } from "./useLibraryInline";

const KINDS: LibraryKind[] = ["instructions", "mcp", "skills"];
export function LibraryPage({
  location,
  onLocated,
}: {
  location?: { kind: "mcp" | "skills"; request: number } | null;
  onLocated?: (request: number) => void;
}) {
  const { t } = useTranslation();
  const active = useInteractionActive();
  const shell = useShell();
  const [tab, setTab] = useState<LibraryKind>("instructions");
  const [visited, setVisited] = useState<Set<LibraryKind>>(
    () => new Set(["instructions"]),
  );
  const root = useRef<HTMLDivElement>(null);
  const header = useRef<HTMLDivElement>(null);
  const positions = useRef(new Map<LibraryKind, number>());
  useLayoutEffect(() => {
    if (!active) return;
    const scroll = libraryScroller(root.current);
    if (!scroll) return;
    scroll.scrollTop = positions.current.get(tab) ?? 0;
    const remember = () => {
      positions.current.set(tab, scroll.scrollTop);
    };
    scroll.addEventListener("scroll", remember, { passive: true });
    return () => scroll.removeEventListener("scroll", remember);
  }, [tab, active]);
  useLayoutEffect(() => {
    const measure = () =>
      root.current?.style.setProperty(
        "--library-scroll-offset",
        `${(header.current?.getBoundingClientRect().height ?? 64) + 12}px`,
      );
    measure();
    if (typeof ResizeObserver === "undefined" || !header.current) return;
    const observer = new ResizeObserver(measure);
    observer.observe(header.current);
    return () => observer.disconnect();
  }, []);
  function changeTab(kind: LibraryKind) {
    const scroll = libraryScroller(root.current);
    if (scroll) positions.current.set(tab, scroll.scrollTop);
    setTab(kind);
    setVisited((previous) => new Set(previous).add(kind));
  }
  const leaveActions = useRef(new Map<LibraryKind, LibraryLeaveGuard>());
  const historyActions = useRef(new Map<LibraryKind, () => void>());
  const scenarioTrigger = useRef<HTMLButtonElement>(null);
  const [scenarioRequest, setScenarioRequest] = useState(0);
  const registerLeave = useCallback(
    (kind: LibraryKind, guard: LibraryLeaveGuard) => {
      leaveActions.current.set(kind, guard);
      return () => {
        if (leaveActions.current.get(kind) === guard)
          leaveActions.current.delete(kind);
      };
    },
    [],
  );
  const registerHistory = useCallback(
    (kind: LibraryKind, action: () => void) => {
      historyActions.current.set(kind, action);
      return () => {
        if (historyActions.current.get(kind) === action)
          historyActions.current.delete(kind);
      };
    },
    [],
  );
  const guarded = (proceed: () => void) => {
    const guard = leaveActions.current.get(tab);
    if (guard) guard(proceed);
    else proceed();
  };
  useEffect(
    () =>
      shell.registerLeaveGuard("library", (proceed) => {
        const guard = leaveActions.current.get(tab);
        if (guard) guard(proceed);
        else proceed();
      }),
    [shell.registerLeaveGuard, tab],
  );
  useEffect(() => {
    if (!location || !active) return;
    guarded(() => {
      changeTab(location.kind);
      onLocated?.(location.request);
    });
  }, [location, active, onLocated]);
  const discoverActions = useRef(
    new Map<LibraryKind, (element: HTMLElement) => void>(),
  );
  const registerDiscover = useCallback(
    (kind: LibraryKind, open: (element: HTMLElement) => void) => {
      discoverActions.current.set(kind, open);
      return () => {
        if (discoverActions.current.get(kind) === open)
          discoverActions.current.delete(kind);
      };
    },
    [],
  );
  const actions = useRef(new Map<LibraryKind, () => Promise<unknown>>());
  const summary = useQuery({
    queryKey: ["library", "summary"],
    queryFn: libraryApi.summary,
    retry: false,
  });
  const register = useCallback(
    (kind: LibraryKind, fn: () => Promise<unknown>) => {
      actions.current.set(kind, fn);
      return () => {
        if (actions.current.get(kind) === fn) actions.current.delete(kind);
      };
    },
    [],
  );
  useShellRefresh("library", async () => {
    await Promise.all([summary.refetch(), actions.current.get(tab)?.()]);
  });
  useEffect(() => {
    if (visited.size > 1) void summary.refetch();
  }, [visited.size, summary.refetch]);
  return (
    <div ref={root} className="min-w-0 space-y-3">
      <div
        ref={header}
        data-library-header
        className="sticky top-0 z-20 -mx-1 flex flex-wrap items-center justify-between gap-3 border-b border-line-2 bg-background/95 px-1 py-2 backdrop-blur"
      >
        <SegmentedNav
          aria-label={t("library.nav")}
          value={tab}
          onChange={(kind) => {
            guarded(() => {
              changeTab(kind);
            });
          }}
          items={KINDS.map((kind) => ({
            value: kind,
            label: `${t(`library.tabs.${kind}`)}${summary.data ? ` · ${summary.data[kind]}` : ""}`,
            controls: `library-panel-${kind}`,
          }))}
        />
        <div className="flex flex-wrap items-center gap-2">
          <LibraryScenarioWizard
            request={scenarioRequest}
            returnFocusRef={scenarioTrigger}
            onChanged={async () => {
              await Promise.all([
                summary.refetch(),
                ...Array.from(actions.current.values(), (action) => action()),
              ]);
            }}
          />
          <Button
            variant="ghost"
            onClick={(event) =>
              discoverActions.current.get(tab)?.(event.currentTarget)
            }
          >
            <Compass className="mr-1 h-3.5 w-3.5" />
            {t(
              tab === "instructions"
                ? "library.interaction.templates"
                : "library.interaction.discover",
            )}
          </Button>
          <ResourcePackDialog
            menuRef={scenarioTrigger}
            beforeAction={guarded}
            additionalActions={[
              {
                label: t("library.interaction.scenarioAction"),
                onSelect: () =>
                  guarded(() => setScenarioRequest((value) => value + 1)),
              },
              {
                label: t("library.interaction.historyAction"),
                onSelect: () => historyActions.current.get(tab)?.(),
              },
            ]}
            onChanged={async () => {
              await Promise.all([
                summary.refetch(),
                ...Array.from(actions.current.values(), (action) => action()),
              ]);
            }}
          />
        </div>
      </div>
      {KINDS.filter((kind) => visited.has(kind)).map((kind) => (
        <div
          key={kind}
          id={`library-panel-${kind}`}
          role="tabpanel"
          aria-label={t(`library.tabs.${kind}`)}
          hidden={kind !== tab}
        >
          <InteractionScope active={active && kind === tab}>
            <LibrarySection
              kind={kind}
              registerRefresh={register}
              registerDiscover={registerDiscover}
              registerLeave={registerLeave}
              registerHistory={registerHistory}
              onChanged={() => void summary.refetch()}
            />
          </InteractionScope>
        </div>
      ))}
    </div>
  );
}
