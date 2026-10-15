import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  libraryApi,
  type InstructionSet,
  type LibraryKind,
  type LibraryOperation,
  type McpDefinition,
  type ResourceTarget,
  type SkillDefinition,
} from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ListCard, ListFold, ListHead, ListRow } from "@/components/ui/list";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  InstructionEditor,
  McpEditor,
  SkillImporter,
  SkillDetails,
} from "./ResourceEditors";
import { ResourceReview } from "./ResourceReview";
import { libraryError } from "./libraryText";
import { DraftStore } from "./useDraftState";

export type Resource = InstructionSet | McpDefinition | SkillDefinition;
type OpenViewBody =
  | {
      type: "edit" | "targets" | "details" | "delete";
      resource: Resource;
      targets: ResourceTarget[];
    }
  | { type: "new" | "import" }
  | { type: "operation"; operation: LibraryOperation };
type OpenView = OpenViewBody & { epoch: number };
export function LibrarySection({
  kind,
  registerRefresh,
  onChanged,
}: {
  kind: LibraryKind;
  registerRefresh: (
    kind: LibraryKind,
    fn: () => Promise<unknown>,
  ) => () => void;
  onChanged: () => void;
}) {
  const { t } = useTranslation();
  const query = useQuery({
    queryKey: ["library", kind],
    queryFn: () => libraryApi.list(kind),
    retry: false,
  });
  const [search, setSearch] = useState("");
  const [view, setView] = useState<OpenView | null>(null);
  const [draftStore, setDraftStore] = useState(() => new DraftStore());
  const [busy, setBusy] = useState(false);
  const running = useRef(false);
  const mounted = useRef(true);
  const trigger = useRef<HTMLElement | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [history, setHistory] = useState(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(
    () =>
      registerRefresh(kind, async () => {
        const result = await query.refetch();
        if (result.error) throw result.error;
      }),
    [kind, query.refetch, registerRefresh],
  );
  async function run(action: () => Promise<unknown>, close = false) {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
      if (!mounted.current) return;
      if (close) setView(null);
      await query.refetch();
      onChanged();
    } catch (e) {
      if (mounted.current) setError(libraryError(e, t));
    } finally {
      running.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  const open = (next: OpenViewBody, event: React.MouseEvent<HTMLElement>) => {
    trigger.current = event.currentTarget;
    setError("");
    setNotice("");
    setDraftStore(new DraftStore());
    setView({ ...next, epoch: query.data?.epoch ?? -1 });
  };
  const close = () => {
    if (!running.current) {
      setView(null);
      setError("");
    }
  };
  const data = query.data;
  const writable =
    Number.isSafeInteger(data?.epoch) && (data?.epoch ?? -1) >= 0;
  const rows: Resource[] =
    data?.instructions ?? data?.servers ?? data?.skills ?? [];
  const filtered = rows.filter((row) =>
    `${row.name} ${row.id}`
      .toLocaleLowerCase()
      .includes(search.trim().toLocaleLowerCase()),
  );
  const targets = (row: Resource) =>
    "targets" in row
      ? row.targets
      : (data?.targets ?? []).map((target) => ({
          ...target,
          desired: target.desiredResourceId === row.id,
          status:
            target.desiredResourceId === row.id
              ? target.status
              : target.lastAppliedResourceId === row.id
                ? "needsReview"
                : target.readableResourceId === row.id
                  ? "shared"
                  : "unassigned",
        }));
  const resource = view && "resource" in view ? view.resource : undefined;
  return (
    <div className="space-y-3">
      <p className="text-ctl text-ink-muted">
        {t(`library.description.${kind}`)}
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          aria-label={t("library.search")}
          placeholder={t("library.search")}
          className="h-7 max-w-xs"
        />
        <Button
          ref={addButton}
          variant="secondary"
          disabled={busy || !writable}
          onClick={(event) => open({ type: "new" }, event)}
        >
          {t(kind === "skills" ? "library.addSkill" : "library.add")}
        </Button>
        {kind !== "skills" && (
          <Button
            variant="secondary"
            disabled={busy || !writable}
            onClick={(event) =>
              kind === "mcp"
                ? void run(async () => {
                    const result = await libraryApi.importMcp();
                    setNotice(
                      t("library.importReport", {
                        created: result.sources.reduce(
                          (sum, source) => sum + (source.imported ?? 0),
                          0,
                        ),
                        failed: result.sources.filter((source) => source.error)
                          .length,
                      }),
                    );
                  })
                : open({ type: "import" }, event)
            }
          >
            {t(
              kind === "mcp"
                ? "library.importNative"
                : "library.importMarkdown",
            )}
          </Button>
        )}
      </div>
      {data && !writable && (
        <p role="status" className="text-ctl text-warn">
          {t("library.legacyReadOnly")}
        </p>
      )}
      {query.isPending && (
        <p role="status" className="text-ctl text-ink-muted">
          {t("library.loading")}
        </p>
      )}
      {query.isError && (
        <div role="alert" className="space-y-2 text-ctl text-danger">
          <p>{t("library.loadFailed")}</p>
          <Button variant="secondary" onClick={() => void query.refetch()}>
            {t("library.retry")}
          </Button>
        </div>
      )}
      {!view && error && (
        <p role="alert" className="text-ctl text-danger">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="text-ctl text-ink-muted">
          {notice}
        </p>
      )}
      {data && (
        <ListCard>
          <ListHead>
            {t(`library.tabs.${kind}`)} · {rows.length}
          </ListHead>
          {!query.isPending && !filtered.length && (
            <ListRow className="text-ctl text-ink-muted">
              {t(search ? "library.noMatch" : "library.empty")}
            </ListRow>
          )}
          {filtered.map((row) => (
            <ListRow key={row.id} className="flex-wrap">
              <div className="min-w-0 flex-1">
                <div className="break-words text-ui font-medium">
                  {row.name}
                </div>
                <p className="break-all text-note text-ink-muted">
                  {"directory" in row
                    ? row.directory
                    : t("library.revision", { revision: row.revision ?? "—" })}
                </p>
                <p className="text-note text-ink-muted">
                  {t("library.targetSummary", {
                    desired: targets(row).filter((target) => target.desired)
                      .length,
                    applied: targets(row).filter((target) =>
                      kind === "instructions"
                        ? target.desired && target.status === "applied"
                        : target.applied,
                    ).length,
                  })}
                </p>
              </div>
              <div className="flex flex-wrap items-center gap-1">
                {kind !== "skills" && (
                  <Button
                    variant="secondary"
                    disabled={busy || !writable}
                    aria-label={t("library.editNamed", { name: row.name })}
                    onClick={(event) =>
                      open(
                        { type: "edit", resource: row, targets: targets(row) },
                        event,
                      )
                    }
                  >
                    {t("library.edit")}
                  </Button>
                )}
                {kind === "instructions" && (
                  <Button
                    variant="secondary"
                    disabled={busy || !writable}
                    onClick={(event) =>
                      open(
                        {
                          type: "edit",
                          resource: {
                            ...(row as InstructionSet),
                            id: crypto.randomUUID(),
                            name: t("library.copyName", { name: row.name }),
                            revision: 0,
                          },
                          targets: [],
                        },
                        event,
                      )
                    }
                  >
                    {t("library.copy")}
                  </Button>
                )}
                {kind === "skills" && (
                  <Button
                    variant="secondary"
                    disabled={busy}
                    aria-label={t("library.detailsNamed", { name: row.name })}
                    onClick={(event) =>
                      open(
                        {
                          type: "details",
                          resource: row,
                          targets: targets(row),
                        },
                        event,
                      )
                    }
                  >
                    {t("library.details")}
                  </Button>
                )}
                <Button
                  variant="secondary"
                  disabled={busy || !writable}
                  aria-label={t("library.assignNamed", { name: row.name })}
                  onClick={(event) =>
                    open(
                      { type: "targets", resource: row, targets: targets(row) },
                      event,
                    )
                  }
                >
                  {t("library.assign")}
                </Button>
                <Button
                  variant="secondary"
                  disabled={busy || !writable}
                  aria-label={t("library.deleteNamed", { name: row.name })}
                  onClick={(event) =>
                    open(
                      { type: "delete", resource: row, targets: targets(row) },
                      event,
                    )
                  }
                >
                  {t("library.remove")}
                </Button>
              </div>
            </ListRow>
          ))}
        </ListCard>
      )}
      <ListFold
        open={history}
        onOpenChange={setHistory}
        note={data?.operations.length ?? 0}
      >
        {t("library.history")}
      </ListFold>
      {history && (
        <ListCard>
          {data?.operations.map((operation) => (
            <ListRow key={operation.operationId}>
              <span className="min-w-0 flex-1 break-all text-note">
                {operation.resourceId} ·{" "}
                {t(`library.status.${operation.status}`, {
                  defaultValue: t("library.unknown"),
                })}
              </span>
              <Button
                variant="secondary"
                onClick={(event) =>
                  open({ type: "operation", operation }, event)
                }
              >
                {t("library.details")}
              </Button>
            </ListRow>
          ))}
        </ListCard>
      )}
      <Dialog
        open={!!view}
        onOpenChange={(open) => {
          if (!open) close();
        }}
      >
        <DialogContent
          className="sm:max-w-3xl"
          onEscapeKeyDown={(event) => {
            if (busy) event.preventDefault();
          }}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const element = trigger.current?.isConnected
              ? trigger.current
              : addButton.current;
            element?.focus();
          }}
        >
          <DialogHeader>
            <DialogTitle>
              {resource?.name ?? t(`library.tabs.${kind}`)}
            </DialogTitle>
            <DialogDescription>
              {t(
                view?.type === "targets"
                  ? "library.assignDescription"
                  : "library.draftDescription",
              )}
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 space-y-3 overflow-y-auto px-[18px] py-3">
            {error && (
              <p role="alert" className="text-ctl text-danger">
                {error}
              </p>
            )}
            {(view?.type === "new" ||
              view?.type === "edit" ||
              view?.type === "import") &&
              kind === "instructions" && (
                <InstructionEditor
                  store={draftStore}
                  initial={resource as InstructionSet | undefined}
                  importMode={view.type === "import"}
                  busy={busy || !writable}
                  onSave={(set, imported) =>
                    run(
                      () =>
                        imported
                          ? libraryApi.importInstruction(set.name, set.content)
                          : libraryApi.saveInstruction(
                              set,
                              set.revision || undefined,
                              view.epoch,
                            ),
                      true,
                    )
                  }
                />
              )}
            {(view?.type === "new" || view?.type === "edit") &&
              kind === "mcp" && (
                <McpEditor
                  store={draftStore}
                  initial={resource as McpDefinition | undefined}
                  busy={busy || !writable}
                  onSave={(id, name, revision, edits) =>
                    run(
                      () =>
                        libraryApi.saveMcp(
                          id,
                          name,
                          revision,
                          edits,
                          view.epoch,
                        ),
                      true,
                    )
                  }
                />
              )}
            {view?.type === "new" && kind === "skills" && (
              <SkillImporter
                store={draftStore}
                busy={busy || !writable}
                onImport={(action) => run(action, true)}
              />
            )}
            {view?.type === "details" && kind === "skills" && (
              <SkillDetails
                store={draftStore}
                skill={view.resource as SkillDefinition}
              />
            )}
            {view?.type === "targets" && (
              <ResourceReview
                store={draftStore}
                externalBusy={busy}
                readOnly={!writable}
                expectedEpoch={view.epoch}
                kind={kind}
                resource={view.resource}
                targets={view.targets}
                onBusyChange={(value) => {
                  running.current = value;
                  setBusy(value);
                }}
                onChanged={async () => {
                  await query.refetch();
                  onChanged();
                }}
              />
            )}
            {view?.type === "operation" && (
              <ResourceReview
                store={draftStore}
                externalBusy={busy}
                readOnly={!writable}
                expectedEpoch={view.epoch}
                kind={kind}
                operation={view.operation}
                onBusyChange={(value) => {
                  running.current = value;
                  setBusy(value);
                }}
                onChanged={async () => {
                  await query.refetch();
                  onChanged();
                }}
              />
            )}
            {view?.type === "delete" && (
              <>
                <p className="text-ctl text-ink-muted">
                  {t("library.removeDescription")}
                </p>
                <Button
                  disabled={
                    busy || !writable || view.resource.revision === undefined
                  }
                  onClick={() =>
                    void run(
                      () =>
                        kind === "instructions"
                          ? libraryApi.deleteInstruction(
                              view.resource.id,
                              view.resource.revision!,
                              view.epoch,
                            )
                          : kind === "mcp"
                            ? libraryApi.deleteMcp(
                                view.resource.id,
                                view.resource.revision!,
                                view.epoch,
                              )
                            : libraryApi.deleteSkill(
                                view.resource.id,
                                view.resource.revision!,
                                view.epoch,
                              ),
                      true,
                    )
                  }
                >
                  {t("library.confirmRemove")}
                </Button>
              </>
            )}
          </div>
          <DialogFooter>
            <Button variant="secondary" disabled={busy} onClick={close}>
              {t("library.close")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
