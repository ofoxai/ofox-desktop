import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  sessionsApi,
  type ManagedSession,
  type SessionTrashEntry,
  type SessionSearchHit,
} from "@/lib/api/sessions";
import { useSessionSearch } from "@/hooks/useSessionSearch";
import { useSessionContentSearch } from "@/hooks/useSessionContentSearch";
import { useShell, useShellRefresh } from "@/components/shell/ShellContext";
import { Button } from "@/components/ui/button";
import { ListHead } from "@/components/ui/list";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { cn } from "@/lib/utils";
import { groupByFolder, canSelect } from "./model";
import { SessionsHeader } from "./SessionsHeader";
import { SessionFolder, SelectionCheckbox } from "./SessionFolder";
import { SessionsTrash } from "./SessionsTrash";
import { SessionReader } from "./SessionReader";
import { SessionSearchResults } from "./SessionSearchResults";
import { MenuSelect } from "@/components/ui/menu-select";
import { Switch } from "@/components/ui/switch";
import { getBaseName } from "./utils";
const EMPTY: ManagedSession[] = [];
// Retained for consumers of the old panel's pure grouping helper.
export const groupSessions = (sessions: ManagedSession[]) =>
  new Map(groupByFolder(sessions).map((g) => [g.path, g.sessions]));
type Confirmation =
  | { kind: "trash"; items: ManagedSession[] }
  | { kind: "purge"; items: SessionTrashEntry[] };
export function SessionsPage({
  className,
  onAnalyze,
  onConfigure,
}: {
  className?: string;
  onAnalyze?: (sessionKey: string) => void;
  onConfigure?: (section: "tools" | "mcp" | "skills") => void;
}) {
  const { t } = useTranslation();
  const { active, sessionLocation, consumeSessionLocation } = useShell();
  const client = useQueryClient();
  const query = useQuery({
    queryKey: ["sessions"],
    queryFn: sessionsApi.list,
    retry: false,
  });
  const trashQuery = useQuery({
    queryKey: ["session-trash"],
    queryFn: sessionsApi.listTrash,
    retry: false,
  });
  const sessions = query.data ?? EMPTY;
  const [agent, setAgent] = useState(
    () => localStorage.getItem("ofox.sessionsAgent") || "all",
  );
  const [search, setSearch] = useState("");
  const [searchMode, setSearchMode] = useState<"metadata" | "content">(
    "metadata",
  );
  const [project, setProject] = useState("all");
  const [includeProcess, setIncludeProcess] = useState(false);
  const [reader, setReader] = useState<{
    key: string;
    expected?: SessionSearchHit;
  }>();
  const listRef = useRef<HTMLDivElement>(null);
  const returnFocus = useRef<HTMLElement | null>(null);
  const returnScroll = useRef(0);
  const [trash, setTrash] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [expanded, setExpanded] = useState<string | null>(null);
  const [opened, setOpened] = useState<Record<string, boolean>>({});
  const [located, setLocated] = useState<{
    key: string;
    request: number | null;
  }>();
  const [locationMissing, setLocationMissing] = useState(false);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const [busy, setBusy] = useState(false);
  const running = useRef(false);
  const { search: find } = useSessionSearch({
    sessions,
    providerFilter: agent,
  });
  const filtered = useMemo(
    () =>
      (
        find(searchMode === "metadata" ? search : "") as ManagedSession[]
      ).filter(
        (session) =>
          project === "all" || (session.projectDir || "") === project,
      ),
    [find, search, searchMode, project],
  );
  const contentScope = JSON.stringify([
    searchMode,
    search,
    agent,
    project,
    includeProcess,
    filtered.map((session) => [session.sessionKey, session.revision]),
  ]);
  const contentSearch = useSessionContentSearch(contentScope);
  useEffect(() => {
    if (active !== "sessions" && contentSearch.busy) contentSearch.cancel();
  }, [active, contentSearch.busy, contentSearch.cancel]);
  const startSearch = () => {
    if (searchMode !== "content" || contentSearch.busy || query.isPending)
      return;
    void contentSearch.start(
      search,
      filtered.map((session) => session.sessionKey),
      includeProcess,
    );
  };
  const openReader = (session: ManagedSession, expected?: SessionSearchHit) => {
    returnFocus.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    returnScroll.current = listRef.current?.scrollTop ?? 0;
    setReader({ key: session.sessionKey, expected });
  };
  const closeReader = () => {
    setReader(undefined);
    window.requestAnimationFrame(() => {
      if (listRef.current) listRef.current.scrollTop = returnScroll.current;
      returnFocus.current?.focus({ preventScroll: true });
    });
  };
  const readingSession = sessions.find(
    (session) => session.sessionKey === reader?.key,
  );
  const groups = useMemo(() => groupByFolder(filtered), [filtered]);
  useEffect(() => {
    if (active !== "sessions" || !sessionLocation || !query.isSuccess) return;
    const row = sessions.find(
      (session) => session.sessionKey === sessionLocation.key,
    );
    setLocationMissing(!row);
    if (row) {
      setAgent("all");
      localStorage.setItem("ofox.sessionsAgent", "all");
      setSearch("");
      setSearchMode("metadata");
      setProject("all");
      setReader(undefined);
      setTrash(false);
      setOpened((old) => ({ ...old, [row.projectDir || ""]: true }));
      setLocated(sessionLocation);
    }
    consumeSessionLocation(sessionLocation.request);
  }, [
    active,
    sessionLocation,
    query.isSuccess,
    sessions,
    consumeSessionLocation,
  ]);
  const locatedFocus = useCallback((request: number) => {
    setLocated((current) =>
      current?.request === request ? { ...current, request: null } : current,
    );
  }, []);
  useEffect(() => {
    const keys = new Set(filtered.filter(canSelect).map((s) => s.sessionKey));
    setSelected((old) => new Set([...old].filter((k) => keys.has(k))));
  }, [filtered]);
  useEffect(() => {
    let disposed = false;
    const unlisten: Array<() => void> = [];
    for (const name of ["sessions-index-updated", "usage-synced"]) {
      void listen(
        name,
        () => void client.invalidateQueries({ queryKey: ["sessions"] }),
      )
        .then((stop) => {
          if (disposed) stop();
          else unlisten.push(stop);
        })
        .catch(() => undefined);
    }
    return () => {
      disposed = true;
      unlisten.forEach((stop) => stop());
    };
  }, [client]);
  const refresh = async () => {
    try {
      const rows = await sessionsApi.refreshIndex();
      await client.invalidateQueries({ queryKey: ["session-messages"] });
      client.setQueryData(["sessions"], rows);
      await client.invalidateQueries({ queryKey: ["session-trash"] });
    } catch (e) {
      toast.error(String(e));
    }
  };
  useShellRefresh("sessions", refresh);
  const select = (items: ManagedSession[], value: boolean) =>
    setSelected((old) => {
      const next = new Set(old);
      for (const s of items.filter(canSelect)) {
        if (value) next.add(s.sessionKey);
        else next.delete(s.sessionKey);
      }
      return next;
    });
  const run = async (action: () => Promise<unknown>) => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    try {
      await action();
    } catch (e) {
      toast.error(String(e));
    } finally {
      await Promise.all([
        client.invalidateQueries({ queryKey: ["sessions"] }),
        client.invalidateQueries({ queryKey: ["session-trash"] }),
      ]);
      running.current = false;
      setBusy(false);
    }
  };
  const confirm = () =>
    void run(async () => {
      if (!confirmation) return;
      if (confirmation.kind === "trash") {
        const result = await sessionsApi.trash(
          confirmation.items
            .filter(canSelect)
            .map((s) => ({ sessionKey: s.sessionKey, revision: s.revision! })),
        );
        const errors = result
          .filter((r) => !r.success)
          .map((r) => r.error)
          .filter(Boolean);
        if (errors.length) toast.error(errors.join("\n"));
        const removed = new Set(
          result.filter((r) => r.success).map((r) => r.sessionKey),
        );
        setSelected((old) => new Set([...old].filter((k) => !removed.has(k))));
      } else {
        const results = await Promise.allSettled(
          confirmation.items.map((s) => sessionsApi.purge(s.key)),
        );
        const errors = results.filter(
          (r): r is PromiseRejectedResult => r.status === "rejected",
        );
        if (errors.length)
          toast.error(errors.map((e) => String(e.reason)).join("\n"));
      }
      setConfirmation(null);
    });
  const chosen = filtered.filter((s) => selected.has(s.sessionKey));
  const title = (s: ManagedSession | SessionTrashEntry) =>
    s.title || s.sessionId;
  return (
    <section
      className={cn("min-h-0 overflow-hidden p-4", className)}
      aria-label={t("sessionCenter.sessionList")}
    >
      {reader &&
        (readingSession ? (
          <SessionReader
            session={readingSession}
            expected={reader.expected}
            onBack={closeReader}
            onAnalyze={onAnalyze}
            onResearch={() => {
              closeReader();
              startSearch();
            }}
          />
        ) : (
          <div role="status">
            <p>{t("usageSessions.managerMissing")}</p>
            <Button variant="ghost" onClick={closeReader}>
              {t("sessionExperience.back")}
            </Button>
          </div>
        ))}
      <div
        ref={listRef}
        hidden={!!reader}
        className="h-full min-h-0 overflow-auto"
      >
        <SessionsHeader
          sessions={sessions}
          agent={agent}
          setAgent={(v) => {
            setAgent(v);
            localStorage.setItem("ofox.sessionsAgent", v);
          }}
          search={search}
          setSearch={setSearch}
          trash={trash}
          setTrash={setTrash}
          trashCount={trashQuery.data?.length ?? 0}
          searchMode={searchMode}
          setSearchMode={setSearchMode}
          onStartSearch={startSearch}
          projectPicker={
            <MenuSelect
              aria-label={t("sessionExperience.projectScope")}
              value={project}
              onChange={setProject}
              searchable
              options={[
                { value: "all", label: t("sessionExperience.allProjects") },
                ...[
                  ...new Set(
                    sessions
                      .filter(
                        (session) =>
                          agent === "all" || session.providerId === agent,
                      )
                      .map((session) => session.projectDir || ""),
                  ),
                ]
                  .sort()
                  .map((path) => ({
                    value: path,
                    label:
                      getBaseName(path) || t("sessionExperience.noProject"),
                    note: path,
                  })),
              ]}
            />
          }
        />
        {!trash && searchMode === "content" && (
          <div className="mb-4 space-y-3">
            <div className="flex flex-wrap items-center gap-3">
              <label className="flex items-center gap-2 text-sub text-ink-muted">
                <Switch
                  checked={includeProcess}
                  onCheckedChange={setIncludeProcess}
                  aria-label={t("sessionExperience.includeProcess")}
                />
                {t("sessionExperience.includeProcess")}
              </label>
              <span className="flex-1" />
              <Button
                variant="secondary"
                disabled={contentSearch.busy || !search.trim()}
                onClick={startSearch}
              >
                {t("sessionExperience.startSearch")}
              </Button>
              {contentSearch.busy && (
                <Button variant="ghost" onClick={contentSearch.cancel}>
                  {t("sessionExperience.cancelSearch")}
                </Button>
              )}
              {contentSearch.nextCursor && (
                <Button
                  variant="ghost"
                  onClick={() => void contentSearch.continueScan()}
                >
                  {t("sessionExperience.continueSearch")}
                </Button>
              )}
            </div>
            <p className="text-note text-ink-muted">
              {t("sessionExperience.searchLocal")}
            </p>
          </div>
        )}
        {locationMissing && (
          <p role="status" className="mb-3 text-sub text-ink-muted">
            {t("usageSessions.managerMissing")}
          </p>
        )}
        {(trash ? trashQuery : query).isError && (
          <div role="alert" className="mb-3 text-sub text-danger">
            {String((trash ? trashQuery : query).error)}{" "}
            <Button
              variant="ghost"
              onClick={() => void (trash ? trashQuery : query).refetch()}
            >
              {t("common.retry")}
            </Button>
          </div>
        )}
        {(trash ? trashQuery : query).isPending && (
          <p role="status">{t("common.loading")}</p>
        )}
        {!trash && searchMode === "content" ? (
          <SessionSearchResults
            search={contentSearch}
            sessions={sessions}
            needle={search}
            onOpen={openReader}
            onStart={startSearch}
          />
        ) : trash ? (
          <SessionsTrash
            items={trashQuery.data ?? []}
            busy={busy}
            restore={(s) => void run(() => sessionsApi.restore(s.key))}
            purge={(s) => setConfirmation({ kind: "purge", items: [s] })}
            clear={() =>
              setConfirmation({ kind: "purge", items: trashQuery.data ?? [] })
            }
          />
        ) : (
          <>
            <ListHead className="mb-2 px-3">
              <SelectionCheckbox
                sessions={filtered}
                selected={selected}
                busy={busy}
                label={t("sessionCenter.selectAll")}
                onChange={(v) => select(filtered, v)}
              />
              <span>
                {t(
                  selected.size
                    ? "sessionCenter.selectedCount"
                    : "sessionCenter.sessionCount",
                  { count: selected.size || filtered.length },
                )}
              </span>
              <span className="flex-1" />
              <Button
                variant="dangerGhost"
                disabled={busy || !chosen.length}
                onClick={() =>
                  setConfirmation({ kind: "trash", items: chosen })
                }
              >
                {t("sessionCenter.trashSelected", { count: chosen.length })}
              </Button>
            </ListHead>
            {filtered.some((s) => !s.capabilities.canReadMessages) && (
              <p className="mb-2 text-sub text-ink-muted">
                {t("sessionCenter.metadataOnly")}
              </p>
            )}
            {filtered.some((s) => s.capabilities.resumeUsesSessionModel) && (
              <p className="mb-2 text-sub text-ink-muted">
                {t("sessionCenter.piResumeModel")}
              </p>
            )}
            {filtered.some((s) => !s.capabilities.canTrash) && (
              <p className="mb-2 text-sub text-ink-muted">
                {t("sessionCenter.fileTrashUnavailable")}
              </p>
            )}
            {!query.isPending && !filtered.length && (
              <div className="py-12 text-center text-ink-muted">
                <p>{t("sessionCenter.empty")}</p>
                {!sessions.length && onConfigure && (
                  <>
                    <p className="my-2 text-sub">
                      {t("sessionExperience.setupHint")}
                    </p>
                    <div className="flex flex-wrap justify-center gap-2">
                      {(["tools", "mcp", "skills"] as const).map((section) => (
                        <Button
                          key={section}
                          variant="secondary"
                          onClick={() => onConfigure(section)}
                        >
                          {t(
                            `sessionExperience.${section === "tools" ? "configureTools" : section === "mcp" ? "configureMcp" : "configureSkills"}`,
                          )}
                        </Button>
                      ))}
                    </div>
                  </>
                )}
              </div>
            )}
            <div className="space-y-2">
              {groups.map((group, i) => (
                <SessionFolder
                  key={group.path}
                  {...group}
                  open={!!search || (opened[group.path] ?? i === 0)}
                  toggle={() =>
                    setOpened((old) => ({
                      ...old,
                      [group.path]: !(old[group.path] ?? i === 0),
                    }))
                  }
                  selected={selected}
                  select={select}
                  busy={busy}
                  expanded={expanded}
                  setExpanded={setExpanded}
                  located={located}
                  onLocated={locatedFocus}
                  onRead={openReader}
                  onAnalyze={onAnalyze}
                  trash={(items) => setConfirmation({ kind: "trash", items })}
                />
              ))}
            </div>
          </>
        )}
      </div>
      <ConfirmDialog
        isOpen={!!confirmation}
        title={t(
          confirmation?.kind === "trash"
            ? "sessionCenter.moveToTrash"
            : "sessionCenter.purge",
        )}
        message={
          confirmation?.kind === "trash"
            ? t("sessionCenter.trashConfirm", {
                count: confirmation.items.length,
              })
            : t("sessionCenter.purgeManyConfirm", {
                count: confirmation?.items.length ?? 0,
              })
        }
        details={
          confirmation && (
            <>
              <ul>
                {confirmation.items.slice(0, 5).map((s) => (
                  <li key={"sessionKey" in s ? s.sessionKey : s.key}>
                    {title(s)}
                  </li>
                ))}
              </ul>
              {confirmation.items.length > 5 && (
                <p>
                  {t("sessionCenter.moreCount", {
                    count: confirmation.items.length - 5,
                  })}
                </p>
              )}
            </>
          )
        }
        busy={busy}
        onCancel={() => {
          if (!running.current) setConfirmation(null);
        }}
        onConfirm={confirm}
      />
    </section>
  );
}
