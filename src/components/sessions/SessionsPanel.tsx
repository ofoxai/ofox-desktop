import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import {
  Copy,
  Download,
  FolderOpen,
  Play,
  RefreshCw,
  Trash2,
  Undo2,
} from "lucide-react";
import { toast } from "sonner";
import {
  sessionsApi,
  type ManagedSession,
  type SessionTrashEntry,
} from "@/lib/api/sessions";
import { useSessionSearch } from "@/hooks/useSessionSearch";
import { isMac } from "@/lib/platform";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { cn } from "@/lib/utils";
import { SessionMessageItem } from "./SessionMessageItem";
import { formatSessionTitle, formatTimestamp, getSessionKey } from "./utils";

const PROVIDERS = [
  "all",
  "claude",
  "codex",
  "gemini",
  "opencode",
  "openclaw",
  "hermes",
  "pi",
  "dsh",
];
const PAGE_SIZE = 100;
const EMPTY_SESSIONS: ManagedSession[] = [];
type Confirmation =
  | { kind: "trash"; sessions: ManagedSession[] }
  | { kind: "purge"; entry: SessionTrashEntry };
type Row =
  | { kind: "project"; project: string; sessions: ManagedSession[] }
  | { kind: "session"; session: ManagedSession }
  | { kind: "more"; project: string };

export function groupSessions(sessions: ManagedSession[]) {
  const groups = new Map<string, ManagedSession[]>();
  for (const session of sessions) {
    const project = session.projectDir || "";
    const group = groups.get(project) ?? [];
    group.push(session);
    groups.set(project, group);
  }
  return groups;
}

function ProjectCheckbox({
  sessions,
  checked,
  busy,
  label,
  onChange,
}: {
  sessions: ManagedSession[];
  checked: Set<string>;
  busy: boolean;
  label: string;
  onChange: (select: boolean) => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  const eligible = sessions.filter(
    (session) => session.capabilities?.canTrash && session.revision,
  );
  const count = eligible.filter((session) =>
    checked.has(session.sessionKey),
  ).length;
  useEffect(() => {
    if (ref.current)
      ref.current.indeterminate = count > 0 && count < eligible.length;
  }, [count, eligible.length]);
  return (
    <input
      ref={ref}
      type="checkbox"
      aria-label={label}
      disabled={busy || !eligible.length}
      checked={eligible.length > 0 && count === eligible.length}
      onChange={(event) => onChange(event.target.checked)}
    />
  );
}

export function SessionsPanel({
  initialProvider = "all",
  className,
}: {
  initialProvider?: string;
  className?: string;
}) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const [provider, setProvider] = useState(initialProvider);
  const [search, setSearch] = useState("");
  const [view, setView] = useState<"sessions" | "trash">("sessions");
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [pages, setPages] = useState<Record<string, number>>({});
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const [busy, setBusy] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const detailRef = useRef<HTMLDivElement>(null);
  const list = useQuery({
    queryKey: ["sessions"],
    queryFn: sessionsApi.list,
    staleTime: 30_000,
  });
  const trash = useQuery({
    queryKey: ["sessionTrash"],
    queryFn: sessionsApi.listTrash,
    enabled: view === "trash",
  });
  const sessions = list.data ?? EMPTY_SESSIONS;
  const { search: searchMetadata } = useSessionSearch({
    sessions,
    providerFilter: provider,
  });
  const filtered = useMemo(() => {
    const keys = new Set(searchMetadata(search).map(getSessionKey));
    return sessions.filter((session) => keys.has(getSessionKey(session)));
  }, [sessions, searchMetadata, search]);
  const selected = filtered.find(
    (session) => session.sessionKey === selectedKey,
  );
  useEffect(() => {
    if (!filtered.some((session) => session.sessionKey === selectedKey))
      setSelectedKey(filtered[0]?.sessionKey ?? null);
    const visible = new Set(filtered.map((session) => session.sessionKey));
    setChecked((current) => {
      const next = new Set([...current].filter((key) => visible.has(key)));
      return next.size === current.size ? current : next;
    });
  }, [filtered, selectedKey]);
  const messages = useQuery({
    queryKey: [
      "sessionMessages",
      selected?.sessionKey,
      selected?.revision,
      selected?.lastActiveAt,
    ],
    queryFn: () => sessionsApi.getMessagesByKey(selected!.sessionKey),
    enabled: Boolean(
      selected?.capabilities?.canReadMessages && selected.sourcePath,
    ),
  });
  useEffect(() => {
    if (detailRef.current) detailRef.current.scrollTop = 0;
  }, [selectedKey]);
  const rows = useMemo(() => {
    const result: Row[] = [];
    for (const [project, group] of groupSessions(filtered)) {
      result.push({ kind: "project", project, sessions: group });
      if (collapsed.has(project)) continue;
      const limit = (pages[project] ?? 1) * PAGE_SIZE;
      result.push(
        ...group
          .slice(0, limit)
          .map((session): Row => ({ kind: "session", session })),
      );
      if (group.length > limit) result.push({ kind: "more", project });
    }
    return result;
  }, [filtered, collapsed, pages]);
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => listRef.current,
    estimateSize: (index) => (rows[index]?.kind === "session" ? 68 : 44),
    overscan: 8,
  });
  const messageVirtual = useVirtualizer({
    count: messages.data?.length ?? 0,
    getScrollElement: () => detailRef.current,
    estimateSize: () => 120,
    overscan: 5,
    gap: 12,
  });
  const toggle = (key: string) =>
    setChecked((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const refresh = async () => {
    await Promise.all([
      client.invalidateQueries({ queryKey: ["sessions"] }),
      client.invalidateQueries({ queryKey: ["sessionTrash"] }),
      client.invalidateQueries({ queryKey: ["sessionMessages"] }),
    ]);
  };
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    try {
      await action();
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };
  const exportSession = () =>
    selected &&
    run(async () => {
      const preview = await sessionsApi.exportMarkdown(selected.sessionKey);
      const destination = await save({
        defaultPath: preview.filename,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (!destination) return;
      await sessionsApi.exportMarkdown(selected.sessionKey, destination);
      toast.success(t("sessionCenter.exported"));
    });
  const resume = (launch: boolean) =>
    selected &&
    run(async () => {
      let result;
      try {
        result = await sessionsApi.resume(selected.sessionKey, launch);
      } catch (error) {
        if (!launch) throw error;
        toast.error(String(error));
        result = await sessionsApi.resume(selected.sessionKey, false);
      }
      if (!result.launched) {
        await navigator.clipboard.writeText(result.command);
        toast.success(t("sessionCenter.commandCopied"));
      }
    });
  const confirm = () => {
    const action = confirmation;
    setConfirmation(null);
    if (!action) return;
    void run(async () => {
      if (action.kind === "purge") await sessionsApi.purge(action.entry.key);
      else {
        const results = await sessionsApi.trash(
          action.sessions.map((session) => ({
            sessionKey: session.sessionKey,
            revision: session.revision!,
          })),
        );
        const successful = new Set(
          results
            .filter((result) => result.success)
            .map((result) => result.sessionKey),
        );
        client.setQueryData<ManagedSession[]>(["sessions"], (data) =>
          data?.filter((session) => !successful.has(session.sessionKey)),
        );
        setChecked(
          (current) =>
            new Set([...current].filter((key) => !successful.has(key))),
        );
        const errors = results.filter((result) => !result.success);
        if (errors.length)
          toast.error(errors.map((result) => result.error).join("\n"));
      }
      await refresh();
    });
  };
  const trashable = filtered.filter(
    (session) =>
      checked.has(session.sessionKey) &&
      session.capabilities?.canTrash &&
      session.revision,
  );
  return (
    <TooltipProvider>
      <section
        className={cn("flex min-h-0 flex-col gap-3", className)}
        aria-label={t("sessionCenter.title")}
      >
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant={view === "sessions" ? "default" : "outline"}
            onClick={() => setView("sessions")}
          >
            {t("sessionCenter.title")}
          </Button>
          <Button
            variant={view === "trash" ? "default" : "outline"}
            onClick={() => setView("trash")}
          >
            <Trash2 className="mr-1 size-4" />
            {t("sessionCenter.trash")}
          </Button>
          <Button
            variant="ghost"
            size="icon"
            disabled={busy || list.isFetching || trash.isFetching}
            aria-label={t("sessionCenter.refresh")}
            onClick={() => void refresh()}
          >
            <RefreshCw className="size-4" />
          </Button>
          {view === "sessions" && (
            <>
              <select
                className="h-9 rounded-md border bg-background px-2 text-sm"
                aria-label={t("sessionCenter.provider")}
                value={provider}
                onChange={(event) => setProvider(event.target.value)}
              >
                {PROVIDERS.map((id) => (
                  <option key={id} value={id}>
                    {id === "all" ? t("sessionCenter.allTools") : id}
                  </option>
                ))}
              </select>
              <Input
                className="min-w-48 flex-1"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder={t("sessionCenter.searchMetadata")}
                aria-label={t("sessionCenter.searchMetadata")}
              />
              <Button
                variant="outline"
                disabled={busy || !trashable.length}
                onClick={() =>
                  setConfirmation({ kind: "trash", sessions: trashable })
                }
              >
                {t("sessionCenter.trashSelected", { count: trashable.length })}
              </Button>
            </>
          )}
        </div>
        {(list.error || trash.error) && (
          <p role="alert" className="text-sm text-destructive">
            {String(
              view === "sessions" ? (list.error ?? "") : (trash.error ?? ""),
            )}
          </p>
        )}
        {view === "trash" ? (
          <div className="min-h-0 flex-1 overflow-auto space-y-3">
            <p className="text-sm text-muted-foreground">
              {t("sessionCenter.trashHint")}
            </p>
            {!trash.isLoading && !trash.data?.length && (
              <p>{t("sessionCenter.emptyTrash")}</p>
            )}
            {trash.data?.map((entry) => (
              <div
                key={entry.key}
                className="flex flex-wrap items-center gap-3 rounded-lg border p-3"
              >
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium">
                    {entry.title || entry.sessionId}
                  </p>
                  <p className="truncate text-xs text-muted-foreground">
                    {entry.providerId} · {entry.projectDir} ·{" "}
                    {formatTimestamp(entry.deletedAt)}
                  </p>
                  {entry.state !== "trashed" && (
                    <p role="alert" className="text-sm text-destructive">
                      {t("sessionCenter.recoveryRequired")} {entry.error}
                    </p>
                  )}
                </div>
                <Button
                  variant="outline"
                  disabled={busy || entry.state !== "trashed"}
                  onClick={() =>
                    void run(async () => {
                      await sessionsApi.restore(entry.key);
                      await refresh();
                    })
                  }
                >
                  <Undo2 className="mr-1 size-4" />
                  {t("sessionCenter.restore")}
                </Button>
                <Button
                  variant="destructive"
                  disabled={busy || entry.state !== "trashed"}
                  onClick={() => setConfirmation({ kind: "purge", entry })}
                >
                  {t("sessionCenter.purge")}
                </Button>
              </div>
            ))}
          </div>
        ) : (
          <div className="grid min-h-0 flex-1 grid-cols-[minmax(220px,1fr)_minmax(0,2fr)] gap-3">
            <div
              ref={listRef}
              className="min-h-0 overflow-auto rounded-lg border"
              aria-label={t("sessionCenter.sessionList")}
            >
              {!list.isLoading && !filtered.length && (
                <p className="p-4 text-sm text-muted-foreground">
                  {t("sessionCenter.empty")}
                </p>
              )}
              <div
                className="relative w-full"
                style={{ height: virtual.getTotalSize() }}
              >
                {virtual.getVirtualItems().map((item) => {
                  const row = rows[item.index];
                  return (
                    <div
                      key={item.key}
                      ref={virtual.measureElement}
                      data-index={item.index}
                      className="absolute left-0 top-0 w-full"
                      style={{ transform: `translateY(${item.start}px)` }}
                    >
                      {row.kind === "project" ? (
                        <div className="flex items-center gap-2 bg-muted/50 p-2 text-xs">
                          <ProjectCheckbox
                            sessions={row.sessions}
                            checked={checked}
                            busy={busy}
                            label={t("sessionCenter.selectProject", {
                              project:
                                row.project || t("sessionCenter.noProject"),
                            })}
                            onChange={(select) =>
                              setChecked((current) => {
                                const next = new Set(current);
                                for (const session of row.sessions.filter(
                                  (session) =>
                                    session.capabilities?.canTrash &&
                                    session.revision,
                                )) {
                                  if (select) next.add(session.sessionKey);
                                  else next.delete(session.sessionKey);
                                }
                                return next;
                              })
                            }
                          />
                          <button
                            className="flex min-w-0 flex-1 items-center gap-1 text-left"
                            onClick={() =>
                              setCollapsed((current) => {
                                const next = new Set(current);
                                if (next.has(row.project))
                                  next.delete(row.project);
                                else next.add(row.project);
                                return next;
                              })
                            }
                          >
                            <FolderOpen className="size-3 shrink-0" />
                            <span className="truncate" title={row.project}>
                              {row.project || t("sessionCenter.noProject")}
                            </span>
                            <span>{row.sessions.length}</span>
                          </button>
                        </div>
                      ) : row.kind === "more" ? (
                        <Button
                          variant="ghost"
                          className="w-full"
                          onClick={() =>
                            setPages((current) => ({
                              ...current,
                              [row.project]: (current[row.project] ?? 1) + 1,
                            }))
                          }
                        >
                          {t("sessionCenter.showMore")}
                        </Button>
                      ) : (
                        <div
                          className={cn(
                            "flex items-center gap-2 border-b px-2",
                            selectedKey === row.session.sessionKey &&
                              "bg-primary/10",
                          )}
                        >
                          <input
                            type="checkbox"
                            aria-label={t("sessionCenter.selectSession", {
                              title: formatSessionTitle(row.session),
                            })}
                            checked={checked.has(row.session.sessionKey)}
                            disabled={
                              busy ||
                              !row.session.capabilities?.canTrash ||
                              !row.session.revision
                            }
                            onChange={() => toggle(row.session.sessionKey)}
                          />
                          <button
                            className="min-w-0 flex-1 py-3 text-left"
                            onClick={() =>
                              setSelectedKey(row.session.sessionKey)
                            }
                          >
                            <p className="truncate text-sm font-medium">
                              {formatSessionTitle(row.session)}
                            </p>
                            <p className="truncate text-xs text-muted-foreground">
                              {row.session.providerId} ·{" "}
                              {formatTimestamp(
                                row.session.lastActiveAt ??
                                  row.session.createdAt,
                              )}
                            </p>
                          </button>
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
            <div className="flex min-h-0 min-w-0 flex-col rounded-lg border">
              {selected && (
                <div className="space-y-2 border-b p-3">
                  <h3 className="truncate font-semibold">
                    {formatSessionTitle(selected)}
                  </h3>
                  <p
                    className="truncate text-xs text-muted-foreground"
                    title={selected.projectDir ?? undefined}
                  >
                    {selected.projectDir}
                  </p>
                  {Boolean(selected.models?.length) && (
                    <p className="text-xs text-muted-foreground">
                      {selected.models!.join(" · ")}
                    </p>
                  )}
                  {Boolean(selected.subagentCount) && (
                    <p className="text-xs text-muted-foreground">
                      {t("sessionCenter.subagents", {
                        count: selected.subagentCount,
                      })}
                    </p>
                  )}
                  {!selected.capabilities?.canTrash &&
                    selected.providerId !== "dsh" && (
                      <p className="text-xs text-muted-foreground">
                        {t("sessionCenter.fileTrashUnavailable")}
                      </p>
                    )}
                  <div className="flex flex-wrap gap-2">
                    {selected.capabilities?.canExport && (
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={busy}
                        onClick={exportSession}
                      >
                        <Download className="mr-1 size-3" />
                        {t("sessionCenter.export")}
                      </Button>
                    )}
                    {selected.capabilities?.canResume && (
                      <>
                        <Button
                          size="sm"
                          variant="outline"
                          disabled={busy}
                          onClick={() => resume(isMac())}
                        >
                          <Play className="mr-1 size-3" />
                          {t(
                            isMac()
                              ? "sessionCenter.resume"
                              : "sessionCenter.copyResume",
                          )}
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          aria-label={t("sessionCenter.copyResume")}
                          disabled={busy}
                          onClick={() => resume(false)}
                        >
                          <Copy className="size-3" />
                        </Button>
                      </>
                    )}
                    {selected.capabilities?.canTrash && selected.revision && (
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={busy}
                        onClick={() =>
                          setConfirmation({
                            kind: "trash",
                            sessions: [selected],
                          })
                        }
                      >
                        <Trash2 className="mr-1 size-3" />
                        {t("sessionCenter.moveToTrash")}
                      </Button>
                    )}
                  </div>
                  {selected.capabilities?.resumeUsesSessionModel && (
                    <p className="text-xs text-muted-foreground">
                      {t("sessionCenter.piResumeModel")}
                    </p>
                  )}
                </div>
              )}
              <div ref={detailRef} className="min-h-0 flex-1 overflow-auto p-3">
                {selected && !selected.capabilities?.canReadMessages && (
                  <p className="text-sm text-muted-foreground">
                    {t("sessionCenter.metadataOnly")}
                  </p>
                )}
                {messages.error && (
                  <p role="alert" className="text-sm text-destructive">
                    {String(messages.error)}
                  </p>
                )}
                {selected?.capabilities?.canReadMessages && (
                  <div
                    className="relative"
                    style={{ height: messageVirtual.getTotalSize() }}
                  >
                    {messageVirtual.getVirtualItems().map((item) => (
                      <div
                        key={item.key}
                        data-index={item.index}
                        ref={messageVirtual.measureElement}
                        className="absolute top-0 left-0 w-full"
                        style={{ transform: `translateY(${item.start}px)` }}
                      >
                        <SessionMessageItem
                          message={messages.data![item.index]}
                          isActive={false}
                          onCopy={(content) =>
                            void navigator.clipboard
                              .writeText(content)
                              .catch((error) => toast.error(String(error)))
                          }
                        />
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>
        )}
        <ConfirmDialog
          isOpen={Boolean(confirmation)}
          title={t(
            confirmation?.kind === "purge"
              ? "sessionCenter.purge"
              : "sessionCenter.moveToTrash",
          )}
          message={
            confirmation?.kind === "purge"
              ? t("sessionCenter.purgeConfirm", {
                  title:
                    confirmation.entry.title || confirmation.entry.sessionId,
                })
              : t("sessionCenter.trashConfirm", {
                  count: confirmation?.sessions.length ?? 0,
                })
          }
          onConfirm={confirm}
          onCancel={() => setConfirmation(null)}
        />
      </section>
    </TooltipProvider>
  );
}
