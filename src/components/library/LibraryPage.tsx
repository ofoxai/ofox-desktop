import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { libraryApi, type LibraryKind } from "@/lib/api/library";
import { useShellRefresh } from "@/components/shell/ShellContext";
import { SegmentedNav } from "@/components/ui/segmented";
import {
  InteractionScope,
  useInteractionActive,
} from "@/components/ui/interaction-scope";
import { ResourcePackDialog } from "./ResourcePackDialog";
import { LibrarySection } from "./LibrarySection";

const KINDS: LibraryKind[] = ["instructions", "mcp", "skills"];
export function LibraryPage() {
  const { t } = useTranslation();
  const active = useInteractionActive();
  const [tab, setTab] = useState<LibraryKind>("instructions");
  const [visited, setVisited] = useState<Set<LibraryKind>>(
    () => new Set(["instructions"]),
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
    <div className="mx-auto max-w-[1200px] space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <SegmentedNav
          aria-label={t("library.nav")}
          value={tab}
          onChange={(kind) => {
            setTab(kind);
            setVisited((previous) => new Set(previous).add(kind));
          }}
          items={KINDS.map((kind) => ({
            value: kind,
            label: `${t(`library.tabs.${kind}`)}${summary.data ? ` · ${summary.data[kind]}` : ""}`,
            controls: `library-panel-${kind}`,
          }))}
        />
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-note text-ink-muted">{t("library.globalScope")}</p>
          <ResourcePackDialog
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
              onChanged={() => void summary.refetch()}
            />
          </InteractionScope>
        </div>
      ))}
    </div>
  );
}
