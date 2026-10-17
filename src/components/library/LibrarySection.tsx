import { Fragment, useEffect, useRef, useState } from "react";
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
  type BatchResource,
  type LibraryBatchPlan,
  type LibraryInventory,
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
import { SkillUpdate } from "./SkillUpdate";
import { McpDuplicate } from "./McpDuplicate";
import { McpCheck } from "./McpCheck";
import { ResourceReview } from "./ResourceReview";
import { InstructionOverview } from "./InstructionOverview";
import { inventoryEntryDescription, libraryError } from "./libraryText";
import { LibraryResourceRow } from "./LibraryResourceRow";
import { LibraryInventoryView } from "./LibraryInventoryView";
import { LibraryUpdateChecks } from "./LibraryUpdateChecks";
import { LibraryBatchReview } from "./LibraryBatchReview";
import type { TargetDelta } from "./LibraryTargets";
import { Checkbox } from "@/components/ui/checkbox";
import { MenuSelect } from "@/components/ui/menu-select";
import { toolLabel } from "./libraryText";
import { DraftStore, useDraftState } from "./useDraftState";
import { LibraryDiscover } from "./LibraryDiscover";
import { LibraryMcpForm, mcpDraftToEdits } from "./LibraryMcpForm";
import {
  createMcpDraft,
  cloneRecommendationDraft,
  type LibraryRecommendation,
} from "./libraryRecommendations";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { settingsApi } from "@/lib/api/settings";
import { skillsApi } from "@/lib/api/skills";

export type Resource = InstructionSet | McpDefinition | SkillDefinition;
type OpenViewBody =
  | {
      type:
        | "edit"
        | "targets"
        | "details"
        | "delete"
        | "copy"
        | "update"
        | "check";
      resource: Resource;
      targets: ResourceTarget[];
    }
  | { type: "new" | "import" | "discover" | "advancedNew" }
  | {
      type: "inventory";
      selectionHint?: {
        name: string;
        variant?: string | null;
        tools: string[];
      };
    }
  | { type: "updates"; skills: SkillDefinition[] }
  | {
      type: "recommendedSkill";
      recipe: LibraryRecommendation & { kind: "skills" };
    }
  | {
      type: "batch";
      resources: BatchResource[];
      tools: string[];
      action: "apply" | "withdraw";
      initial?: LibraryBatchPlan;
    }
  | { type: "operation"; operation: LibraryOperation };
type OpenView = OpenViewBody & { epoch: number };
export function LibrarySection({
  kind,
  registerRefresh,
  onChanged,
  registerDiscover,
}: {
  kind: LibraryKind;
  registerRefresh: (
    kind: LibraryKind,
    fn: () => Promise<unknown>,
  ) => () => void;
  onChanged: () => void;
  registerDiscover: (
    kind: LibraryKind,
    open: (trigger: HTMLElement) => void,
  ) => () => void;
}) {
  const { t, i18n } = useTranslation();
  const query = useQuery({
    queryKey: ["library", kind],
    queryFn: () => libraryApi.list(kind),
    retry: false,
  });
  const [search, setSearch] = useState("");
  const [view, setView] = useState<OpenView | null>(null);
  const [draftStore, setDraftStore] = useState(() => new DraftStore());
  const [mcpDraft, setMcpDraft] = useDraftState(
    draftStore,
    "friendlyMcp",
    createMcpDraft,
  );
  const [newMcpId] = useDraftState(draftStore, "newMcpId", () =>
    crypto.randomUUID(),
  );
  const [busy, setBusy] = useState(false);
  const running = useRef(false);
  const mounted = useRef(true);
  const trigger = useRef<HTMLElement | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [history, setHistory] = useState(false);
  const [agentFilter, setAgentFilter] = useState("all");
  const [statusFilter, setStatusFilter] = useState("all");
  const [sort, setSort] = useState("name");
  const [selecting, setSelecting] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [pending, setPending] = useState<
    Record<string, TargetDelta & { revision: number }>
  >({});
  const [batchTools, setBatchTools] = useState<string[]>([]);
  const [batches, setBatches] = useState<LibraryBatchPlan[]>([]);
  const [nativeSnapshot, setNativeSnapshot] = useState<{
    epoch: number;
    inventory: LibraryInventory;
  }>();
  const [checks, setChecks] = useState<
    Record<string, { state: string; checkedAtMs: number; toolsCount: number }>
  >({});
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
    if (running.current) return false;
    running.current = true;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
      if (!mounted.current) return false;
      if (close) setView(null);
      await query.refetch();
      onChanged();
      return true;
    } catch (e) {
      if (mounted.current) setError(libraryError(e, t));
      return false;
    } finally {
      running.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  const open = (next: OpenViewBody, event: { currentTarget: HTMLElement }) => {
    trigger.current = event.currentTarget;
    setError("");
    setNotice("");
    setDraftStore(new DraftStore());
    setView({ ...next, epoch: query.data?.epoch ?? -1 });
  };
  useEffect(
    () =>
      registerDiscover(kind, (element) =>
        open({ type: "discover" }, { currentTarget: element }),
      ),
    [kind, registerDiscover, query.data?.epoch],
  );
  const close = () => {
    if (!running.current) {
      if (view?.type === "update") {
        const checked = draftStore.values.get("skillUpdateCheck") as
          | import("@/lib/api/library").SkillUpdateCheck
          | undefined;
        if (checked?.candidate)
          void libraryApi
            .discardSkillUpdate(checked.candidate.ticketId)
            .catch(() => undefined);
      }
      setView(null);
      setError("");
    }
  };
  const data = query.data;
  const writable =
    Number.isSafeInteger(data?.epoch) && (data?.epoch ?? -1) >= 0;
  const rows: Resource[] =
    data?.instructions ?? data?.servers ?? data?.skills ?? [];
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
  const sourceName = (row: Resource) =>
    "repository" in row && row.repository
      ? row.repository
      : "sourceKind" in row && row.sourceKind === "localOrArchive"
        ? t("library.workbench.localSource")
        : t("library.workbench.unknownSource");
  const filtered = rows
    .filter((row) => {
      const rowTargets = targets(row);
      const text = [
        row.name,
        "description" in row ? row.description : "",
        "repository" in row ? row.repository : "",
      ]
        .join(" ")
        .toLocaleLowerCase();
      return (
        text.includes(search.trim().toLocaleLowerCase()) &&
        (agentFilter === "all" ||
          rowTargets.some(
            (target) =>
              target.toolId === agentFilter &&
              (target.desired || target.readable || target.status === "shared"),
          )) &&
        (statusFilter === "all" ||
          (statusFilter === "applied"
            ? rowTargets.some(
                (target) => target.status === "applied" && target.desired,
              )
            : statusFilter === "draft"
              ? !rowTargets.some((target) => target.desired)
              : rowTargets.some((target) =>
                  ["needsReview", "conflict", "unreadable", "failed"].includes(
                    target.status,
                  ),
                )))
      );
    })
    .sort((a, b) =>
      sort === "reverse"
        ? b.name.localeCompare(a.name)
        : sort === "source"
          ? ("repository" in a ? (a.repository ?? "") : "").localeCompare(
              "repository" in b ? (b.repository ?? "") : "",
            ) || a.name.localeCompare(b.name)
          : a.name.localeCompare(b.name),
    );
  const toggleTarget = (row: Resource, target: ResourceTarget) =>
    setPending((previous) => {
      const change = previous[row.id] ?? {
        revision: row.revision ?? 0,
        addTools: [],
        withdrawTools: [],
      };
      const add = new Set(change.addTools),
        withdraw = new Set(change.withdrawTools);
      const desired =
        add.has(target.toolId) ||
        (!!target.desired && !withdraw.has(target.toolId));
      add.delete(target.toolId);
      withdraw.delete(target.toolId);
      if (desired && target.desired) withdraw.add(target.toolId);
      if (!desired && !target.desired) add.add(target.toolId);
      const next = { ...previous };
      if (add.size || withdraw.size)
        next[row.id] = {
          revision: change.revision,
          addTools: [...add],
          withdrawTools: [...withdraw],
        };
      else delete next[row.id];
      return next;
    });
  const pendingResources = Object.entries(pending).map(
    ([resourceId, delta]) => ({
      resourceId,
      expectedRevision: delta.revision,
      addTools: delta.addTools,
      withdrawTools: delta.withdrawTools,
    }),
  );
  const selectedRows = rows.filter((row) => selected.has(row.id));
  const resource = view && "resource" in view ? view.resource : undefined;
  async function nextAfterSave(resourceId: string, intent: "draft" | "apply") {
    if (intent === "draft") {
      setView(null);
      return;
    }
    const next = await libraryApi.list(kind);
    const row = (next.instructions ?? next.servers ?? next.skills ?? []).find(
      (row) => row.id === resourceId,
    );
    if (!row) throw new Error("library.resourceMissing");
    const nextTargets =
      "targets" in row
        ? row.targets
        : (next.targets ?? []).map((target) => ({
            ...target,
            desired: target.desiredResourceId === row.id,
          }));
    setDraftStore(new DraftStore());
    setView({
      type: "targets",
      resource: row,
      targets: nextTargets,
      epoch: next.epoch,
    });
  }
  function chooseRecommendation(recipe: LibraryRecommendation) {
    const next = new DraftStore();
    if (recipe.kind === "mcp") {
      next.values.set("friendlyMcp", cloneRecommendationDraft(recipe));
      setDraftStore(next);
      setView({ type: "new", epoch: data?.epoch ?? -1 });
    } else if (recipe.kind === "instructions") {
      const language =
        i18n.resolvedLanguage?.split("-")[0] ?? i18n.language.split("-")[0];
      setDraftStore(next);
      setView({
        type: "edit",
        resource: {
          id: crypto.randomUUID(),
          name: recipe.titleKey ? t(recipe.titleKey) : recipe.name,
          content:
            recipe.contentByLanguage[
              language === "ja" ? "ja" : language === "en" ? "en" : "zh"
            ],
          revision: 0,
        },
        targets: [],
        epoch: data?.epoch ?? -1,
      });
    } else
      setView({ type: "recommendedSkill", recipe, epoch: data?.epoch ?? -1 });
  }
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          aria-label={t("library.search")}
          placeholder={t("library.search")}
          className="h-7 min-w-[160px] max-w-xs flex-1"
        />
        <MenuSelect
          aria-label={t("library.workbench.agentFilter")}
          value={agentFilter}
          onChange={setAgentFilter}
          options={[
            { value: "all", label: t("library.workbench.allAgents") },
            ...["claude", "codex", "gemini", "opencode", "hermes"].map(
              (value) => ({ value, label: toolLabel(value, t) }),
            ),
          ]}
        />
        <MenuSelect
          aria-label={t("library.workbench.statusFilter")}
          value={statusFilter}
          onChange={setStatusFilter}
          options={["all", "applied", "draft", "attention"].map((value) => ({
            value,
            label: t(`library.workbench.filter.${value}`),
          }))}
        />
        <MenuSelect
          aria-label={t("library.workbench.sort")}
          value={sort}
          onChange={setSort}
          options={[
            { value: "name", label: "A → Z" },
            { value: "reverse", label: "Z → A" },
            ...(kind === "skills"
              ? [{ value: "source", label: t("library.workbench.bySource") }]
              : []),
          ]}
        />
        <Button
          variant="ghost"
          onClick={() => {
            setSelecting(!selecting);
            setSelected(new Set());
          }}
        >
          {t(
            selecting
              ? "library.workbench.finishSelection"
              : "library.workbench.select",
          )}
        </Button>
        <Button
          ref={addButton}
          variant="secondary"
          disabled={busy || !writable}
          onClick={(event) => open({ type: "new" }, event)}
        >
          {t(kind === "skills" ? "library.addSkill" : "library.add")}
        </Button>
        <Button
          variant="secondary"
          disabled={busy || !writable}
          onClick={(event) => open({ type: "inventory" }, event)}
        >
          {t("library.workbench.importNative")}
        </Button>
        {kind === "instructions" && (
          <Button
            variant="ghost"
            disabled={busy || !writable}
            onClick={(event) => open({ type: "import" }, event)}
          >
            {t("library.importMarkdown")}
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
            {t("library.workbench.inLibrary")} · {rows.length}
          </ListHead>
          {!query.isPending && !filtered.length && (
            <ListRow className="text-ctl text-ink-muted">
              <div className="space-y-2">
                <p>{t(search ? "library.noMatch" : "library.empty")}</p>
                {!search && (
                  <Button
                    variant="secondary"
                    onClick={(event) => open({ type: "inventory" }, event)}
                  >
                    {t("library.workbench.importNative")}
                  </Button>
                )}
                {!search && (
                  <>
                    <Button
                      variant="ghost"
                      onClick={(event) => open({ type: "discover" }, event)}
                    >
                      {t("library.discover.title")}
                    </Button>
                    <Button
                      variant="ghost"
                      onClick={(event) => open({ type: "new" }, event)}
                    >
                      {t("library.workbench.addManual")}
                    </Button>
                  </>
                )}
              </div>
            </ListRow>
          )}
          {selecting && (
            <ListRow>
              <Checkbox
                aria-label={t("library.workbench.selectAll")}
                checked={
                  filtered.length > 0 &&
                  filtered.every((row) => selected.has(row.id))
                    ? true
                    : filtered.some((row) => selected.has(row.id))
                      ? "indeterminate"
                      : false
                }
                onCheckedChange={(checked) =>
                  setSelected((previous) => {
                    const next = new Set(previous);
                    for (const row of filtered) {
                      if (checked === true) next.add(row.id);
                      else next.delete(row.id);
                    }
                    return next;
                  })
                }
              />
              <span className="text-ctl text-ink-muted">
                {t("library.workbench.selected", {
                  count: selectedRows.length,
                })}
              </span>
            </ListRow>
          )}
          {filtered.map((row, index) => (
            <Fragment key={row.id}>
              {kind === "skills" &&
                sort === "source" &&
                (index === 0 ||
                  sourceName(row) !== sourceName(filtered[index - 1])) && (
                  <div className="border-b border-line bg-pill/40 px-4 py-2">
                    <h3 className="text-ctl font-medium text-ink-muted">
                      {sourceName(row)}
                    </h3>
                  </div>
                )}
              <LibraryResourceRow
                key={row.id}
                kind={kind}
                row={row}
                targets={targets(row)}
                busy={busy}
                writable={writable}
                selecting={selecting}
                selected={selected.has(row.id)}
                onSelected={(checked) =>
                  setSelected((previous) => {
                    const next = new Set(previous);
                    if (checked) next.add(row.id);
                    else next.delete(row.id);
                    return next;
                  })
                }
                delta={pending[row.id]}
                check={checks[`${data?.epoch}:${row.id}:${row.revision}`]}
                onToggle={(target) => toggleTarget(row, target)}
                onAction={(type, element) => {
                  if (type === "export") {
                    void run(() => libraryApi.exportSelected(kind, [row.id]));
                    return;
                  }
                  if (type === "copy" && kind === "instructions")
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
                      { currentTarget: element },
                    );
                  else
                    open(
                      { type, resource: row, targets: targets(row) },
                      { currentTarget: element },
                    );
                }}
              />
            </Fragment>
          ))}
        </ListCard>
      )}
      {kind !== "instructions" && data && (
        <section
          aria-label={t("library.workbench.inTools")}
          className="space-y-2"
        >
          <div className="flex flex-wrap items-center justify-between gap-2 px-1 pt-2">
            <h3 className="text-ctl font-medium">
              {t("library.workbench.inTools")}
            </h3>
            <Button
              variant="ghost"
              disabled={busy}
              onClick={(event) => open({ type: "inventory" }, event)}
            >
              {t(
                nativeSnapshot && nativeSnapshot.epoch === data.epoch
                  ? "library.workbench.rescan"
                  : "library.workbench.scanNative",
              )}
            </Button>
          </div>
          {nativeSnapshot && nativeSnapshot.epoch === data.epoch ? (
            <>
              <ListCard>
                {nativeSnapshot.inventory.entries
                  .filter((entry) => !entry.matchedResourceIds.length)
                  .map((entry) => (
                    <ListRow key={entry.entryId} className="flex-wrap gap-3">
                      <div className="min-w-0 flex-1">
                        <p
                          className="truncate text-ui font-medium"
                          title={entry.name}
                        >
                          {entry.name}
                        </p>
                        <p
                          className="truncate text-note text-ink-muted"
                          title={inventoryEntryDescription(
                            nativeSnapshot.inventory,
                            entry,
                            t,
                          )}
                        >
                          {inventoryEntryDescription(
                            nativeSnapshot.inventory,
                            entry,
                            t,
                          )}
                        </p>
                      </div>
                      <div className="flex flex-wrap gap-1">
                        {[
                          ...new Set(
                            entry.sources.map((source) => source.toolId),
                          ),
                        ].map((tool) => (
                          <span key={tool} title={toolLabel(tool, t)}>
                            <ToolBadge
                              toolId={tool}
                              size={22}
                              variant="plain"
                            />
                          </span>
                        ))}
                      </div>
                      <Button
                        variant="secondary"
                        disabled={busy}
                        aria-label={t("library.workbench.importNamed", {
                          name: entry.name,
                        })}
                        onClick={(event) =>
                          open(
                            {
                              type: "inventory",
                              selectionHint: {
                                name: entry.name,
                                variant: entry.variant,
                                tools: entry.sources.map(
                                  (source) => source.toolId,
                                ),
                              },
                            },
                            event,
                          )
                        }
                      >
                        {t("library.importDraft")}
                      </Button>
                    </ListRow>
                  ))}
                {!nativeSnapshot.inventory.entries.some(
                  (entry) => !entry.matchedResourceIds.length,
                ) && (
                  <ListRow className="text-ctl text-ink-muted">
                    {t("library.workbench.noNativeResources")}
                  </ListRow>
                )}
              </ListCard>
              <p className="px-1 text-note text-ink-muted">
                {t("library.workbench.scanSnapshot")}
              </p>
              {nativeSnapshot.inventory.sources.some(
                (source) => !["ready", "missing"].includes(source.state),
              ) && (
                <p role="status" className="px-1 text-note text-warn">
                  {t("library.workbench.scanIncomplete")}
                </p>
              )}
            </>
          ) : (
            <p className="px-1 text-note text-ink-muted">
              {t("library.workbench.nativeDraftHint")}
            </p>
          )}
        </section>
      )}
      {kind === "instructions" && data && (
        <InstructionOverview
          data={data}
          busy={busy || !writable}
          onManage={(set, event) =>
            open(
              { type: "targets", resource: set, targets: targets(set) },
              event,
            )
          }
        />
      )}
      {pendingResources.length > 0 && (
        <div
          role="region"
          aria-label={t("library.workbench.pendingChanges")}
          className="sticky bottom-2 z-10 flex flex-wrap items-center justify-between gap-2 rounded-card border border-brand/30 bg-card p-3 shadow-sm"
        >
          <span className="text-ctl font-medium">
            {t("library.workbench.pendingCount", {
              count: pendingResources.length,
            })}
          </span>
          <div className="flex gap-2">
            <Button
              variant="ghost"
              disabled={busy}
              onClick={() => {
                setPending({});
                addButton.current?.focus();
              }}
            >
              {t("library.workbench.discard")}
            </Button>
            <Button
              disabled={busy || !writable}
              onClick={(event) =>
                open(
                  {
                    type: "batch",
                    resources: pendingResources,
                    tools: [],
                    action: "apply",
                  },
                  event,
                )
              }
            >
              {t("library.workbench.previewChanges")}
            </Button>
          </div>
        </div>
      )}
      {selecting && selectedRows.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 rounded-card border border-line bg-card p-3">
          <span className="text-ctl">
            {t("library.workbench.selected", { count: selectedRows.length })}
          </span>
          {["claude", "codex", "gemini", "opencode", "hermes"].map((tool) => (
            <Button
              key={tool}
              variant="ghost"
              aria-pressed={batchTools.includes(tool)}
              aria-disabled={
                busy ||
                selectedRows.some(
                  (row) =>
                    !targets(row).some(
                      (target) =>
                        target.toolId === tool &&
                        (target.available || target.desired),
                    ),
                )
              }
              title={
                selectedRows.some(
                  (row) =>
                    !targets(row).some(
                      (target) =>
                        target.toolId === tool &&
                        (target.available || target.desired),
                    ),
                )
                  ? t("library.nativeCliRequired")
                  : toolLabel(tool, t)
              }
              onClick={() => {
                if (
                  busy ||
                  selectedRows.some(
                    (row) =>
                      !targets(row).some(
                        (target) =>
                          target.toolId === tool &&
                          (target.available || target.desired),
                      ),
                  )
                )
                  return;
                setBatchTools((previous) =>
                  previous.includes(tool)
                    ? previous.filter((value) => value !== tool)
                    : [...previous, tool],
                );
              }}
            >
              {toolLabel(tool, t)}
            </Button>
          ))}
          {(["apply", "withdraw"] as const).map((action) => (
            <Button
              key={action}
              variant="secondary"
              disabled={
                busy ||
                !batchTools.length ||
                batchTools.some((tool) =>
                  selectedRows.some(
                    (row) =>
                      !targets(row).some(
                        (target) =>
                          target.toolId === tool &&
                          (target.available || target.desired),
                      ),
                  ),
                ) ||
                selectedRows.some((row) => row.revision === undefined)
              }
              onClick={(event) =>
                open(
                  {
                    type: "batch",
                    resources: selectedRows.map((row) => ({
                      resourceId: row.id,
                      expectedRevision: row.revision!,
                    })),
                    tools: batchTools,
                    action,
                  },
                  event,
                )
              }
            >
              {t(`library.workbench.${action}Selected`)}
            </Button>
          ))}
          <Button
            variant="ghost"
            disabled={busy}
            onClick={() =>
              void run(() =>
                libraryApi.exportSelected(
                  kind,
                  selectedRows.map((row) => row.id),
                ),
              )
            }
          >
            {t("library.workbench.exportSelected")}
          </Button>
          {kind === "skills" && (
            <Button
              variant="ghost"
              disabled={busy}
              onClick={(event) =>
                open(
                  {
                    type: "updates",
                    skills: selectedRows as SkillDefinition[],
                  },
                  event,
                )
              }
            >
              {t("library.skillUpdate.check")}
            </Button>
          )}
        </div>
      )}
      <ListFold
        open={history}
        onOpenChange={(value) => {
          setHistory(value);
          if (value)
            void libraryApi
              .batches(kind)
              .then(setBatches)
              .catch(() => undefined);
        }}
        note={data?.operations.length ?? 0}
      >
        {t("library.history")}
      </ListFold>
      {history && (
        <ListCard>
          {batches.map((batch) => (
            <ListRow key={batch.operationId}>
              <span className="flex-1 text-ctl">
                {t("library.workbench.batchHistory", {
                  count: batch.resources.length,
                })}{" "}
                ·{" "}
                {t(`library.status.${batch.status}`, {
                  defaultValue: t("library.unknown"),
                })}
              </span>
              <Button
                variant="secondary"
                onClick={(event) =>
                  open(
                    {
                      type: "batch",
                      resources: [],
                      tools: [],
                      action: "apply",
                      initial: batch,
                    },
                    event,
                  )
                }
              >
                {t("library.details")}
              </Button>
            </ListRow>
          ))}
          {data?.operations
            .filter(
              (operation) =>
                !operation.operationId.startsWith("batch-child:") &&
                !batches.some(
                  (batch) => batch.operationId === operation.operationId,
                ),
            )
            .map((operation) => (
              <ListRow key={operation.operationId}>
                <span className="min-w-0 flex-1 break-all text-note">
                  {rows.find((row) => row.id === operation.resourceId)?.name ??
                    t("library.workbench.removedResource")}{" "}
                  ·{" "}
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
              {view?.type === "discover"
                ? t("library.discover.title")
                : view?.type === "inventory"
                  ? t("library.workbench.importNative")
                  : (resource?.name ?? t(`library.tabs.${kind}`))}
            </DialogTitle>
            <DialogDescription>
              {t(
                view?.type === "check"
                  ? "library.mcpCheckTitleDescription"
                  : view?.type === "targets"
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
            {view?.type === "discover" && (
              <LibraryDiscover
                kind={kind}
                busy={busy}
                onChoose={chooseRecommendation}
                onOpenSource={(url) =>
                  void settingsApi
                    .openExternal(url)
                    .catch(() => setError(t("library.failed")))
                }
              />
            )}
            {view?.type === "recommendedSkill" && (
              <div className="space-y-3">
                <p className="text-ui font-medium">{view.recipe.name}</p>
                <p className="text-ctl text-ink-muted">
                  {t("library.discover.pinNote")}
                </p>
                <p className="text-ctl">
                  {view.recipe.dependencyKeys.map((key) => t(key)).join(" ")}
                </p>
                <p className="text-note text-ink-muted">
                  {view.recipe.permissionKeys.map((key) => t(key)).join(" ")}
                </p>
                {(["draft", "apply"] as const).map((intent) => (
                  <Button
                    key={intent}
                    variant={intent === "draft" ? "secondary" : "primary"}
                    disabled={busy || !writable}
                    onClick={() =>
                      void run(async () => {
                        const recipe = view.recipe;
                        const installed = await skillsApi.installUnified(
                          {
                            key: `${recipe.repository.owner}/${recipe.repository.name}:${recipe.repository.directory}`,
                            name: recipe.name,
                            description: t(recipe.descriptionKey),
                            directory: recipe.repository.directory,
                            repoOwner: recipe.repository.owner,
                            repoName: recipe.repository.name,
                            repoBranch: recipe.repository.branch,
                          },
                          "claude",
                        );
                        await nextAfterSave(installed.id, intent);
                      })
                    }
                  >
                    {t(
                      intent === "draft"
                        ? "library.importDraft"
                        : "library.workbench.downloadAndChoose",
                    )}
                  </Button>
                ))}
              </div>
            )}
            {view?.type === "updates" && (
              <LibraryUpdateChecks
                skills={view.skills}
                epoch={view.epoch}
                onBusyChange={(value) => {
                  running.current = value;
                  setBusy(value);
                }}
                onOpen={(skill) => {
                  setDraftStore(new DraftStore());
                  setView({
                    type: "update",
                    resource: skill,
                    targets: targets(skill),
                    epoch: view.epoch,
                  });
                }}
              />
            )}
            {view?.type === "inventory" && (
              <LibraryInventoryView
                kind={kind}
                epoch={view.epoch}
                selectionHint={view.selectionHint}
                onScanned={(inventory) =>
                  setNativeSnapshot({ epoch: view.epoch, inventory })
                }
                onChanged={async () => {
                  setNativeSnapshot(undefined);
                  await query.refetch();
                  onChanged();
                }}
                onBusyChange={(value) => {
                  running.current = value;
                  setBusy(value);
                }}
              />
            )}
            {view?.type === "batch" && (
              <LibraryBatchReview
                kind={kind}
                resources={view.resources}
                tools={view.tools}
                action={view.action}
                initial={view.initial}
                epoch={view.epoch}
                onBusyChange={(value) => {
                  running.current = value;
                  setBusy(value);
                }}
                onChanged={async (plan) => {
                  await query.refetch();
                  onChanged();
                  if (
                    ["applied", "partial", "cancelled", "needsReview"].includes(
                      plan.status,
                    )
                  ) {
                    setPending((previous) => {
                      const next = { ...previous };
                      for (const submitted of view.resources) {
                        const delta = next[submitted.resourceId];
                        if (!delta) continue;
                        const requested = new Set([
                          ...view.tools,
                          ...(submitted.addTools ?? []),
                          ...(submitted.withdrawTools ?? []),
                        ]);
                        const completed = new Set(
                          plan.status === "applied"
                            ? requested
                            : plan.groups
                                .filter(
                                  (group) =>
                                    group.status === "applied" &&
                                    group.resourceIds.includes(
                                      submitted.resourceId,
                                    ),
                                )
                                .flatMap(
                                  (group) => group.changedTools ?? group.tools,
                                )
                                .concat(
                                  (plan.metadataChanges ?? [])
                                    .filter(
                                      (change) =>
                                        change.resourceId ===
                                          submitted.resourceId &&
                                        change.status === "applied",
                                    )
                                    .flatMap((change) => change.tools ?? []),
                                ),
                        );
                        const addTools = delta.addTools.filter(
                          (tool) =>
                            !requested.has(tool) || !completed.has(tool),
                        );
                        const withdrawTools = delta.withdrawTools.filter(
                          (tool) =>
                            !requested.has(tool) || !completed.has(tool),
                        );
                        if (addTools.length || withdrawTools.length)
                          next[submitted.resourceId] = {
                            ...delta,
                            addTools,
                            withdrawTools,
                          };
                        else delete next[submitted.resourceId];
                      }
                      return next;
                    });
                    if (plan.status === "applied") {
                      const completed = new Set(
                        view.resources.map((resource) => resource.resourceId),
                      );
                      setSelected(
                        (previous) =>
                          new Set(
                            [...previous].filter((id) => !completed.has(id)),
                          ),
                      );
                      setBatchTools([]);
                    }
                  }
                }}
              />
            )}
            {(view?.type === "new" ||
              view?.type === "edit" ||
              view?.type === "import") &&
              kind === "instructions" && (
                <InstructionEditor
                  store={draftStore}
                  initial={resource as InstructionSet | undefined}
                  importMode={view.type === "import"}
                  onDiscard={close}
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
            {(view?.type === "new" ||
              view?.type === "edit" ||
              view?.type === "import") &&
              kind === "instructions" && (
                <Button
                  variant="secondary"
                  disabled={busy || !writable}
                  onClick={() =>
                    void run(async () => {
                      const draft = draftStore.values.get(
                        "instructionDraft",
                      ) as InstructionSet | undefined;
                      if (!draft?.name.trim())
                        throw new Error("library.invalidInstructions");
                      const saved =
                        view.type === "import"
                          ? await libraryApi.importInstruction(
                              draft.name,
                              draft.content,
                            )
                          : await libraryApi.saveInstruction(
                              draft,
                              draft.revision || undefined,
                              view.epoch,
                            );
                      await nextAfterSave(saved.id, "apply");
                    })
                  }
                >
                  {t("library.workbench.saveAndChoose")}
                </Button>
              )}
            {view?.type === "copy" && kind === "mcp" && (
              <McpDuplicate
                store={draftStore}
                initial={view.resource as McpDefinition}
                busy={busy || !writable}
                onSave={(id, name) =>
                  run(
                    () =>
                      libraryApi.duplicateMcp(
                        view.resource.id,
                        view.resource.revision!,
                        view.epoch,
                        id,
                        name,
                      ),
                    true,
                  )
                }
              />
            )}
            {view?.type === "check" && kind === "mcp" && (
              <McpCheck
                store={draftStore}
                server={view.resource as McpDefinition}
                epoch={view.epoch}
                readOnly={!writable}
                onResult={(result, checkedAtMs) =>
                  setChecks((previous) => ({
                    ...previous,
                    [`${view.epoch}:${view.resource.id}:${view.resource.revision}`]:
                      {
                        state: result.state,
                        checkedAtMs,
                        toolsCount: result.tools.length,
                      },
                  }))
                }
                onBusyChange={(value) => {
                  running.current = value;
                  setBusy(value);
                }}
              />
            )}
            {view?.type === "new" && kind === "mcp" && (
              <LibraryMcpForm
                value={mcpDraft}
                onChange={setMcpDraft}
                busy={busy || !writable}
                onSave={(draft, intent) =>
                  run(async () => {
                    await libraryApi.saveMcp(
                      newMcpId,
                      draft.name,
                      undefined,
                      mcpDraftToEdits(draft),
                      view.epoch,
                    );
                    await nextAfterSave(newMcpId, intent);
                  })
                }
                onAdvanced={(draft) => {
                  let edits: import("@/lib/api/library").FieldEdit[];
                  try {
                    edits = mcpDraftToEdits(draft);
                  } catch {
                    edits = [
                      { path: ["type"], action: "set", value: draft.transport },
                      ...(draft.transport === "stdio"
                        ? [
                            {
                              path: ["command"],
                              action: "set" as const,
                              value: draft.command,
                            },
                            {
                              path: ["args"],
                              action: "set" as const,
                              value: draft.argsText
                                .split(/\r?\n/)
                                .filter(Boolean),
                            },
                          ]
                        : [
                            {
                              path: ["url"],
                              action: "set" as const,
                              value: draft.url,
                            },
                          ]),
                      ...draft.environment
                        .filter((pair) => pair.name)
                        .map((pair) => ({
                          path: ["env", pair.name],
                          action: "set" as const,
                          value: pair.value,
                        })),
                      ...draft.headers
                        .filter((pair) => pair.name)
                        .map((pair) => ({
                          path: ["headers", pair.name],
                          action: "set" as const,
                          value: pair.value,
                        })),
                      ...(draft.advancedFields ?? []).map((field) => ({
                        ...field,
                        action: "set" as const,
                      })),
                    ];
                  }
                  draftStore.values.set("mcpId", newMcpId);
                  draftStore.values.set("mcpName", draft.name);
                  draftStore.values.set(
                    "mcpFields",
                    edits.map((edit, index) => ({
                      ...edit,
                      input: JSON.stringify(edit.value),
                      key: String(index),
                    })),
                  );
                  setView({ ...view, type: "advancedNew" });
                }}
              />
            )}
            {(view?.type === "advancedNew" || view?.type === "edit") &&
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
            {view?.type === "update" && kind === "skills" && (
              <SkillUpdate
                store={draftStore}
                skill={view.resource as SkillDefinition}
                epoch={view.epoch}
                readOnly={!writable}
                externalBusy={busy}
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
            {view?.type === "details" && kind === "skills" && (
              <SkillDetails
                store={draftStore}
                skill={view.resource as SkillDefinition}
                epoch={view.epoch}
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
