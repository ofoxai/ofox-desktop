import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Download, MoreHorizontal, Pencil, Plus } from "lucide-react";
import {
  libraryApi,
  type LibraryOperation,
  type McpDefinition,
} from "@/lib/api/library";
import { settingsApi } from "@/lib/api/settings";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { SearchInput } from "@/components/ui/search-input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useInteractionActive } from "@/components/ui/interaction-scope";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { cn } from "@/lib/utils";
import { LibraryMcpForm } from "./LibraryMcpForm";
import { LibraryDiscover } from "./LibraryDiscover";
import { McpCheck } from "./McpCheck";
import { DraftStore } from "./useDraftState";
import {
  cloneRecommendationDraft,
  createMcpDraft,
  type LibraryMcpDraft,
} from "./libraryRecommendations";
import { createMcpIdentifier, mcpDraftToEdits } from "./libraryMcpInput";
import { libraryError, toolLabel } from "./libraryText";
import { useLibraryInventory } from "./useLibraryInventory";
import { MCP_TABLE_TOOLS, mcpCell, type McpCell } from "./mcpTableCells";

type DialogState =
  | { type: "add"; draft: LibraryMcpDraft; tools: string[] }
  | { type: "edit"; row: McpDefinition; draft: LibraryMcpDraft }
  | { type: "delete"; row: McpDefinition }
  | { type: "check"; row: McpDefinition; store: DraftStore }
  | { type: "discover" };

/** Preview, then write only when every target can be written as planned. */
async function commit(previewing: Promise<LibraryOperation>) {
  const operation = await previewing;
  const blocked = operation.targets.find(
    (target) => target.status !== "pending",
  );
  if (blocked) {
    await libraryApi
      .cancel(operation.operationId)
      .catch((reason) =>
        console.warn("[library] could not cancel a blocked preview", reason),
      );
    throw new Error(blocked.reason ?? "library.mcpNameConflict");
  }
  const result = await libraryApi.apply(operation.operationId);
  // Every target written is done, even when tidying up after it needs a look.
  const failed = result.targets.find((target) => target.status !== "applied");
  if (failed) throw new Error(failed.reason ?? "library.failed");
}
async function fresh(id: string) {
  const data = await libraryApi.list("mcp");
  const row = data.servers?.find((server) => server.id === id);
  if (!row) throw new Error("library.resourceMissing");
  return { row, epoch: data.epoch };
}
const desiredTools = (row: McpDefinition) =>
  row.targets.filter((target) => target.desired).map((target) => target.toolId);
/** Write the server into exactly `tools`, taking over `adopt` first. */
async function assign(id: string, tools: string[], adopt: string[] = []) {
  if (adopt.length) {
    const { row, epoch } = await fresh(id);
    const taken = [...new Set([...desiredTools(row), ...adopt])];
    await commit(
      libraryApi.previewMcpAdoption(id, taken, adopt, row.revision, epoch),
    );
  }
  const { row, epoch } = await fresh(id);
  await commit(libraryApi.preview("mcp", id, tools, row.revision, epoch));
}

export function LibraryMcpTable({
  registerRefresh,
  registerDiscover,
  onChanged,
}: {
  registerRefresh: (kind: "mcp", fn: () => Promise<unknown>) => () => void;
  registerDiscover: (
    kind: "mcp",
    open: (trigger: HTMLElement) => void,
  ) => () => void;
  onChanged: () => void;
}) {
  const { t } = useTranslation();
  const active = useInteractionActive();
  // Observe the existing settings cache; this does not read or write settings.
  const routingSettings = useQuery({
    queryKey: ["settings"],
    queryFn: settingsApi.get,
    enabled: false,
  }).data;
  const routingKey = JSON.stringify([
    routingSettings?.claudeConfigDir,
    routingSettings?.codexConfigDir,
    routingSettings?.geminiConfigDir,
    routingSettings?.opencodeConfigDir,
    routingSettings?.hermesConfigDir,
  ]);
  const query = useQuery({
    queryKey: ["library", "mcp", routingKey],
    queryFn: () => libraryApi.list("mcp"),
    retry: false,
  });
  const data = query.data;
  const rows = data?.servers ?? [];
  const writable =
    Number.isSafeInteger(data?.epoch) && (data?.epoch ?? -1) >= 0;
  const definitionsKey = rows
    .map(
      (row) =>
        `${row.id}:${row.revision}:${row.targets.map((target) => `${target.toolId}:${target.status}:${target.available}`).join(",")}`,
    )
    .sort()
    .join("|");
  const native = useLibraryInventory(
    "mcp",
    data?.epoch,
    definitionsKey + routingKey,
    active,
  );
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [checking, setChecking] = useState(false);
  const mounted = useRef(true);
  useEffect(() => {
    // StrictMode runs this twice in development; mark the second mount too.
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(
    () =>
      registerRefresh("mcp", async () => {
        await query.refetch();
        native.rescan();
      }),
    [registerRefresh, query.refetch, native.rescan],
  );
  useEffect(
    () => registerDiscover("mcp", () => setDialog({ type: "discover" })),
    [registerDiscover],
  );
  async function run(key: string, action: () => Promise<string | void>) {
    if (busy) return false;
    setBusy(key);
    setError("");
    setNotice("");
    try {
      const done = await action();
      if (mounted.current && done) setNotice(done);
      return true;
    } catch (reason) {
      if (mounted.current) setError(libraryError(reason, t));
      return false;
    } finally {
      await query.refetch();
      native.rescan();
      onChanged();
      if (mounted.current) setBusy(undefined);
    }
  }
  const toggle = (row: McpDefinition, tool: string, cell: McpCell) =>
    run(`${row.id}:${tool}`, async () => {
      const current = desiredTools(row);
      if (cell.state === "off") await assign(row.id, [...current, tool]);
      else if (cell.state === "present")
        await assign(
          row.id,
          current.filter((item) => item !== tool),
          [tool],
        );
      else
        await assign(
          row.id,
          current.filter((item) => item !== tool),
        );
    });
  const entries = native.inventory?.entries ?? [];
  const importable = entries.filter(
    (entry) => !entry.matchedResourceIds.length && !entry.conflict,
  );
  const clashing = entries.filter(
    (entry) => !entry.matchedResourceIds.length && entry.conflict,
  );
  const importFromTools = () =>
    run("import", async () => {
      const inventory = native.inventory;
      if (!inventory || !importable.length)
        return t("library.mcpTable.importNone");
      const result = await libraryApi.importInventory(
        inventory.ticketId,
        importable.map((entry) => entry.entryId),
        data!.epoch,
      );
      return [
        t("library.mcpTable.imported", { count: result.imported }),
        clashing.length
          ? t("library.mcpTable.importSkipped", { count: clashing.length })
          : "",
      ]
        .filter(Boolean)
        .join(" ");
    });
  const existingIds = rows.map((row) => row.id);
  const needle = search.trim().toLowerCase();
  const filtered = rows.filter(
    (row) =>
      !needle ||
      [row.name, row.id, row.commandLine ?? "", row.displaySummary ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(needle),
  );
  const cells = new Map(
    rows.map((row) => [
      row.id,
      Object.fromEntries(
        MCP_TABLE_TOOLS.map((tool) => [
          tool,
          mcpCell(row, tool, native.inventory),
        ]),
      ) as Record<string, McpCell>,
    ]),
  );
  const cellText = (cell: McpCell, tool: string) =>
    cell.reason && ["unsupported", "conflict"].includes(cell.state)
      ? libraryError(cell.reason, t)
      : t(`library.mcpTable.cell.${cell.state}`, {
          tool: toolLabel(tool, t),
        });
  const formTargets = MCP_TABLE_TOOLS.flatMap((tool) =>
    (data?.targets ?? []).filter((target) => target.toolId === tool),
  );
  return (
    <section aria-label={t("library.tabs.mcp")} className="space-y-3">
      <div className="flex flex-wrap items-center gap-3">
        <SearchInput
          className="w-full max-w-sm"
          value={search}
          onChange={setSearch}
          placeholder={t("library.mcpTable.search")}
          aria-label={t("library.mcpTable.search")}
        />
        <span className="text-sub text-ink-muted">
          {t("library.mcpTable.count", { count: rows.length })}
        </span>
        <div className="ml-auto flex items-center gap-2">
          <Button
            variant="ghost"
            disabled={!writable || !!busy || !native.inventory}
            onClick={() => void importFromTools()}
          >
            <Download className="mr-1 h-3.5 w-3.5" />
            {t("library.mcpTable.import")}
            {importable.length ? ` · ${importable.length}` : ""}
          </Button>
          <Button
            disabled={!writable || !!busy}
            onClick={() =>
              setDialog({ type: "add", draft: createMcpDraft(), tools: [] })
            }
          >
            <Plus className="mr-1 h-3.5 w-3.5" />
            {t("library.mcpTable.add")}
          </Button>
        </div>
      </div>
      {data && !writable && (
        <p role="status" className="text-ctl text-warn">
          {t("library.legacyReadOnly")}
        </p>
      )}
      {error && (
        <p role="alert" className="text-ctl text-danger">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="text-ctl text-ink-muted">
          {notice}
        </p>
      )}
      {query.isError ? (
        <p role="alert" className="text-ctl text-danger">
          {libraryError(query.error, t)}
        </p>
      ) : !rows.length && !query.isLoading ? (
        <p className="rounded-list border border-dashed border-line px-4 py-10 text-center text-ctl text-ink-muted">
          {t("library.mcpTable.empty")}
        </p>
      ) : (
        <div className="overflow-x-auto rounded-list border border-line bg-card">
          <table className="w-full min-w-[640px] border-collapse text-left">
            <thead className="bg-pill/60">
              <tr className="border-b border-line-2">
                <th className="px-4 py-2 text-sub font-medium text-ink-muted">
                  {t("library.mcpTable.name")}
                </th>
                {MCP_TABLE_TOOLS.map((tool) => (
                  <th
                    key={tool}
                    scope="col"
                    className="w-14 px-1 py-2 text-center"
                    title={toolLabel(tool, t)}
                  >
                    <span className="sr-only">{toolLabel(tool, t)}</span>
                    <span
                      aria-hidden
                      className="flex flex-col items-center gap-0.5"
                    >
                      <ToolBadge toolId={tool} size={18} variant="plain" />
                      <span className="text-micro tabular-nums text-ink-muted">
                        {
                          rows.filter(
                            (row) => cells.get(row.id)?.[tool].checked,
                          ).length
                        }
                      </span>
                    </span>
                  </th>
                ))}
                <th className="w-20 px-2">
                  <span className="sr-only">
                    {t("library.mcpTable.actions")}
                  </span>
                </th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((row) => (
                <tr
                  key={row.id}
                  data-testid={`mcp-row-${row.id}`}
                  className="border-b border-line-2 last:border-b-0"
                >
                  <td className="max-w-0 px-4 py-2.5">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="truncate text-ui font-medium text-ink">
                        {row.name}
                      </span>
                      <span className="shrink-0 rounded-full border border-line px-1.5 font-mono text-micro text-ink-muted">
                        {row.transport}
                      </span>
                    </div>
                    <p
                      className="truncate font-mono text-note text-ink-muted [font-variant-ligatures:none]"
                      title={row.commandLine ?? undefined}
                    >
                      {row.commandLine ||
                        row.displaySummary ||
                        row.serviceHost ||
                        ""}
                    </p>
                    {row.appOwned && (
                      <p className="text-note text-ink-faint">
                        {t("library.mcpAppOwned")}
                      </p>
                    )}
                  </td>
                  {MCP_TABLE_TOOLS.map((tool) => {
                    const cell = cells.get(row.id)![tool];
                    const label = `${row.name} · ${toolLabel(tool, t)} · ${cellText(cell, tool)}`;
                    return (
                      <td
                        key={tool}
                        className="px-1 py-2.5 text-center"
                        title={cellText(cell, tool)}
                      >
                        <span className="relative inline-grid place-items-center">
                          <Checkbox
                            aria-label={label}
                            data-state-name={cell.state}
                            className={cn(
                              "h-4 w-4",
                              cell.state === "present" &&
                                "data-[state=checked]:border-brand/70 data-[state=checked]:bg-brand/70",
                            )}
                            checked={cell.checked}
                            aria-busy={busy === `${row.id}:${tool}`}
                            disabled={!writable || !!busy || !cell.toggleable}
                            onCheckedChange={() => void toggle(row, tool, cell)}
                          />
                          {["conflict", "pending", "presentOff"].includes(
                            cell.state,
                          ) && (
                            <span
                              aria-hidden
                              className={cn(
                                "absolute -right-2.5 -top-2 text-micro font-semibold",
                                cell.state === "conflict"
                                  ? "text-warn"
                                  : "text-ink-muted",
                              )}
                            >
                              {cell.state === "conflict"
                                ? "!"
                                : cell.state === "pending"
                                  ? "±"
                                  : "◌"}
                            </span>
                          )}
                        </span>
                      </td>
                    );
                  })}
                  <td className="px-2 py-2.5">
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        variant="icon"
                        size="icon"
                        aria-label={t("library.mcpTable.edit", {
                          name: row.name,
                        })}
                        disabled={!writable || !!busy}
                        onClick={() =>
                          setDialog({
                            type: "edit",
                            row,
                            draft: createMcpDraft({
                              name: row.name,
                              transport:
                                row.transport as LibraryMcpDraft["transport"],
                            }),
                          })
                        }
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </Button>
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            variant="icon"
                            size="icon"
                            aria-label={t("library.mcpTable.more", {
                              name: row.name,
                            })}
                            disabled={!writable || !!busy}
                          >
                            <MoreHorizontal className="h-4 w-4" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem
                            onSelect={() =>
                              void run(`${row.id}:copy`, async () => {
                                const name = t("library.copyName", {
                                  name: row.name,
                                });
                                await libraryApi.duplicateMcp(
                                  row.id,
                                  row.revision,
                                  data!.epoch,
                                  createMcpIdentifier(name, existingIds),
                                  name,
                                );
                              })
                            }
                          >
                            {t("library.mcpTable.copy")}
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onSelect={() =>
                              setDialog({
                                type: "check",
                                row,
                                store: new DraftStore(),
                              })
                            }
                          >
                            {t("library.mcpTable.check")}
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            className="text-danger"
                            onSelect={() => setDialog({ type: "delete", row })}
                          >
                            {t("library.mcpTable.delete")}
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {native.loading && (
        <p className="text-note text-ink-faint">
          {t("library.workbench2.scanning")}
        </p>
      )}
      <Dialog
        open={dialog?.type === "add" || dialog?.type === "edit"}
        onOpenChange={(open) => {
          if (!open && !busy) setDialog(null);
        }}
      >
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>
              {dialog?.type === "edit"
                ? t("library.mcpTable.edit", { name: dialog.row.name })
                : t("library.mcpTable.add")}
            </DialogTitle>
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto px-[18px] py-4">
            {dialog?.type === "add" && (
              <LibraryMcpForm
                immediate
                value={dialog.draft}
                onChange={(draft) => setDialog({ ...dialog, draft })}
                identifier={
                  dialog.draft.identifier ||
                  createMcpIdentifier(dialog.draft.name, existingIds)
                }
                onIdentifierChange={(identifier) =>
                  setDialog({
                    ...dialog,
                    draft: { ...dialog.draft, identifier },
                  })
                }
                existingIdentifiers={existingIds}
                targets={formTargets}
                selectedTools={dialog.tools}
                onToolsChange={(tools) => setDialog({ ...dialog, tools })}
                busy={!!busy}
                onSave={(draft, intent) =>
                  run("add", async () => {
                    const id =
                      draft.identifier ||
                      createMcpIdentifier(draft.name, existingIds);
                    await libraryApi.saveMcp(
                      id,
                      draft.name,
                      undefined,
                      mcpDraftToEdits(draft),
                      data!.epoch,
                    );
                    if (intent === "apply" && dialog.tools.length)
                      await assign(id, dialog.tools);
                    setDialog(null);
                    return t("library.mcpTable.added", { name: draft.name });
                  })
                }
                onImportMany={(drafts, intent) =>
                  run("add", async () => {
                    for (const draft of drafts) {
                      const id = draft.identifier!;
                      await libraryApi.saveMcp(
                        id,
                        draft.name,
                        undefined,
                        mcpDraftToEdits(draft),
                        (await libraryApi.list("mcp")).epoch,
                      );
                      if (intent === "apply" && dialog.tools.length)
                        await assign(id, dialog.tools);
                    }
                    setDialog(null);
                    return t("library.mcpTable.imported", {
                      count: drafts.length,
                    });
                  })
                }
              />
            )}
            {dialog?.type === "edit" && (
              <LibraryMcpForm
                saveOnly
                immediate
                initial={dialog.row}
                value={dialog.draft}
                onChange={(draft) => setDialog({ ...dialog, draft })}
                identifier={dialog.row.id}
                existingIdentifiers={existingIds}
                busy={!!busy}
                onSave={() => undefined}
                onSaveEdits={(edits) =>
                  run("edit", async () => {
                    await libraryApi.saveMcp(
                      dialog.row.id,
                      dialog.draft.name,
                      dialog.row.revision,
                      edits,
                      data!.epoch,
                    );
                    // Tools that already have it get the new version at once.
                    const tools = desiredTools(dialog.row);
                    if (tools.length) await assign(dialog.row.id, tools);
                    setDialog(null);
                  })
                }
              />
            )}
          </div>
        </DialogContent>
      </Dialog>
      <Dialog
        open={dialog?.type === "delete"}
        onOpenChange={(open) => {
          if (!open && !busy) setDialog(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>
              {dialog?.type === "delete" &&
                t("library.mcpTable.deleteTitle", { name: dialog.row.name })}
            </DialogTitle>
            <DialogDescription>
              {t("library.mcpTable.deleteBody")}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="secondary"
              disabled={!!busy}
              onClick={() => setDialog(null)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              disabled={!!busy}
              onClick={() =>
                dialog?.type === "delete" &&
                void run("delete", async () => {
                  const id = dialog.row.id;
                  if (desiredTools(dialog.row).length) await assign(id, []);
                  const { row, epoch } = await fresh(id);
                  await libraryApi.deleteMcp(id, row.revision, epoch);
                  setDialog(null);
                })
              }
            >
              {t("library.mcpTable.delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog
        open={dialog?.type === "check"}
        onOpenChange={(open) => {
          if (!open && !checking) setDialog(null);
        }}
      >
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>
              {dialog?.type === "check" &&
                t("library.mcpTable.checkTitle", { name: dialog.row.name })}
            </DialogTitle>
          </DialogHeader>
          {dialog?.type === "check" && (
            <div className="min-h-0 flex-1 overflow-y-auto px-[18px] py-4">
              <McpCheck
                store={dialog.store}
                scopeKey={routingKey}
                server={dialog.row}
                epoch={data!.epoch}
                readOnly={!writable}
                onBusyChange={setChecking}
              />
            </div>
          )}
        </DialogContent>
      </Dialog>
      <Dialog
        open={dialog?.type === "discover"}
        onOpenChange={(open) => {
          if (!open) setDialog(null);
        }}
      >
        <DialogContent className="sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>{t("library.mcpTable.discover")}</DialogTitle>
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto px-[18px] py-4">
            <LibraryDiscover
              kind="mcp"
              busy={!!busy}
              onChoose={(recipe) => {
                if (recipe.kind !== "mcp") return;
                setDialog({
                  type: "add",
                  draft: cloneRecommendationDraft(recipe),
                  tools: [],
                });
              }}
            />
          </div>
        </DialogContent>
      </Dialog>
    </section>
  );
}
