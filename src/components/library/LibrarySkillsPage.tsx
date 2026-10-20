import { useEffect, useRef, useState, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { homeDir } from "@tauri-apps/api/path";
import { MoreHorizontal, Search, Trash2 } from "lucide-react";
import {
  libraryApi,
  type InventoryImport,
  type LibraryInventory,
  type LibraryOperation,
  type SkillDefinition,
  type SkillDraftChange,
  type SkillUpdateCheck,
} from "@/lib/api/library";
import { skillsApi, type DiscoverableSkill } from "@/lib/api/skills";
import { settingsApi } from "@/lib/api/settings";
import { useSettingsQuery } from "@/lib/query/queries";
import { isWindows } from "@/lib/platform";
import { TOOL_LOGOS } from "@/config/toolLogos";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { SearchInput } from "@/components/ui/search-input";
import { SegmentedControl } from "@/components/ui/segmented";
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
import type { SkillSyncMethod } from "@/types";
import { LibraryDiscover } from "./LibraryDiscover";
import { LibraryResourceIcon } from "./LibraryResourceIcon";
import { SkillDetails } from "./ResourceEditors";
import { SkillUpdate } from "./SkillUpdate";
import { DraftStore } from "./useDraftState";
import type { LibraryRecommendation } from "./libraryRecommendations";
import { libraryError, toolLabel } from "./libraryText";
import { useLibraryInventory } from "./useLibraryInventory";
import {
  desiredSkillTools,
  skillAll,
  skillCell,
  skillColumns,
  skillToggle,
  type SkillCell,
} from "./skillCells";
import { parseSkillSource } from "./skillSource";

type FoundEntry = LibraryInventory["entries"][number];
type Candidate = {
  key: string;
  name: string;
  description?: string;
  /** Already in the library: shown, not installable. */
  existing: boolean;
  skill?: DiscoverableSkill;
  folder?: { path: string; leaf: string };
};
type Found = {
  source: string;
  candidates: Candidate[];
  selected: string[];
  agents: string[];
};
type DialogState =
  | { type: "remove"; row: SkillDefinition }
  | { type: "details"; row: SkillDefinition; store: DraftStore }
  | { type: "update"; row: SkillDefinition; store: DraftStore }
  | { type: "deleteFound"; entry: FoundEntry };
type Sort = "source" | "az" | "za";

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
  const data = await libraryApi.list("skills");
  const row = data.skills?.find((skill) => skill.id === id);
  if (!row) throw new Error("library.resourceMissing");
  return { row, epoch: data.epoch };
}
/** Give the skill to exactly `tools`, taking over `adopt` in place first. */
async function assign(id: string, tools: string[], adopt: string[] = []) {
  if (adopt.length) {
    const { row, epoch } = await fresh(id);
    if (row.revision === undefined) throw new Error("library.revisionChanged");
    const taken = [...new Set([...desiredSkillTools(row), ...adopt])];
    await commit(
      libraryApi.previewSkillAdoption(id, taken, adopt, row.revision, epoch),
    );
  }
  const { row, epoch } = await fresh(id);
  await commit(libraryApi.preview("skills", id, tools, row.revision, epoch));
}
/** Give a freshly added skill to the agents picked, skipping those that can't take it. */
async function giveTo(id: string, agents: string[]) {
  const { row } = await fresh(id);
  const cells = agents.map((tool) => ({ tool, cell: skillCell(row, tool) }));
  const adopt = cells
    .filter(({ cell }) => cell.state === "adoptable")
    .map(({ tool }) => tool);
  const add = cells
    .filter(({ cell }) => cell.state === "off")
    .map(({ tool }) => tool);
  if (!adopt.length && !add.length) return;
  await assign(
    id,
    [...new Set([...desiredSkillTools(row), ...add, ...adopt])],
    adopt,
  );
}
async function expandHome(path: string) {
  if (!path.startsWith("~")) return path;
  return (await homeDir()).replace(/[\\/]+$/, "") + path.slice(1);
}
const leafOf = (directory: string) =>
  directory.split(/[\\/]/).filter(Boolean).pop() ?? directory;
const repositoryOf = (row: SkillDefinition) =>
  row.repository && row.sourceKind !== "localOrArchive"
    ? row.repository
    : undefined;
const sameText = (a?: string, b?: string) =>
  (a ?? "").toLocaleLowerCase() === (b ?? "").toLocaleLowerCase();

/** A section title in magpie's small capitals. */
function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <h2 className="text-note font-semibold uppercase tracking-[.06em] text-ink-muted">
      {children}
    </h2>
  );
}
function SkillGlyph() {
  return (
    <LibraryResourceIcon
      kind="skills"
      className="mt-0.5 h-[26px] w-[26px] self-start rounded-[7px] [&_svg]:h-[15px] [&_svg]:w-[15px]"
    />
  );
}

export function LibrarySkillsPage({
  registerRefresh,
  registerDiscover,
  onChanged,
}: {
  registerRefresh: (kind: "skills", fn: () => Promise<unknown>) => () => void;
  registerDiscover: (
    kind: "skills",
    open: (trigger: HTMLElement) => void,
  ) => () => void;
  onChanged: () => void;
}) {
  const { t } = useTranslation();
  const active = useInteractionActive();
  const queryClient = useQueryClient();
  // Observe the existing settings cache for the folders that move skill targets.
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
  const settings = useSettingsQuery().data;
  const query = useQuery({
    queryKey: ["library", "skills", routingKey],
    queryFn: () => libraryApi.list("skills"),
    retry: false,
  });
  const data = query.data;
  const rows = data?.skills ?? [];
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
    "skills",
    data?.epoch,
    definitionsKey + routingKey,
    active,
  );
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [source, setSource] = useState("");
  const [finding, setFinding] = useState(false);
  const [findError, setFindError] = useState("");
  const [found, setFound] = useState<Found | null>(null);
  const [filter, setFilter] = useState("");
  const [sort, setSort] = useState<Sort>("source");
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [checking, setChecking] = useState(false);
  const [savingSync, setSavingSync] = useState(false);
  const installCard = useRef<HTMLDivElement>(null);
  const sourceInput = useRef<HTMLInputElement>(null);
  const discoverSection = useRef<HTMLElement>(null);
  const discoverSearch = useRef<HTMLInputElement>(null);
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
      registerRefresh("skills", async () => {
        await query.refetch();
        native.rescan();
      }),
    [registerRefresh, query.refetch, native.rescan],
  );
  useEffect(
    () =>
      registerDiscover("skills", () => {
        discoverSection.current?.scrollIntoView?.({ block: "start" });
        discoverSearch.current?.focus({ preventScroll: true });
      }),
    [registerDiscover],
  );
  useEffect(() => {
    if (native.expired) native.rescan();
  }, [native.expired, native.rescan]);
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

  const skillTargets = data?.skillTargets ?? [];
  const columns = skillColumns(data?.skillTargets, rows);
  const installAgents = skillColumns(data?.skillTargets, []);
  const labelFor = (tool: string) =>
    skillTargets.find((target) => target.id === tool)?.label ??
    TOOL_LOGOS[tool]?.label ??
    toolLabel(tool, t);
  const inLibrary = (directory: string, key?: string) =>
    rows.some(
      (row) =>
        (key !== undefined && row.id === key) ||
        sameText(row.directory, leafOf(directory)),
    );
  const findErrorText = (reason: unknown) =>
    /skillDocumentMissing/.test(String(reason))
      ? t("library.skillsPage.noSkillDoc")
      : libraryError(reason, t);

  async function find() {
    if (finding || busy) return;
    const parsed = parseSkillSource(source);
    setFindError("");
    if (!parsed) {
      setFindError(t("library.skillsPage.badSource"));
      return;
    }
    setFinding(true);
    setFound(null);
    try {
      if (parsed.kind === "folder") {
        setFound({
          source: parsed.path,
          candidates: [
            {
              key: parsed.path,
              name: parsed.leaf,
              description: parsed.path,
              existing: inLibrary(parsed.leaf),
              folder: parsed,
            },
          ],
          selected: inLibrary(parsed.leaf) ? [] : [parsed.path],
          agents: installAgents,
        });
        return;
      }
      // Looking a repository up never changes the saved ones: it is added
      // (or switched on, or to the branch asked for) only for the lookup.
      const saved = (await skillsApi.getRepos()).find(
        (repo) =>
          sameText(repo.owner, parsed.owner) &&
          sameText(repo.name, parsed.name),
      );
      const branch = parsed.branch ?? saved?.branch ?? "";
      const lookup = {
        owner: saved?.owner ?? parsed.owner,
        name: saved?.name ?? parsed.name,
        branch,
        enabled: true,
      };
      const touched = !saved || !saved.enabled || saved.branch !== branch;
      if (touched) await skillsApi.addRepo(lookup);
      let discovered;
      try {
        discovered = await skillsApi.discoverAvailable();
      } finally {
        if (touched)
          await (saved
            ? skillsApi.addRepo(saved)
            : skillsApi.removeRepo(lookup.owner, lookup.name));
      }
      const available = discovered.filter(
        (skill) =>
          sameText(skill.repoOwner, lookup.owner) &&
          sameText(skill.repoName, lookup.name) &&
          (!branch || skill.repoBranch === branch) &&
          (!parsed.path ||
            skill.directory === parsed.path ||
            skill.directory.startsWith(`${parsed.path}/`)),
      );
      if (!mounted.current) return;
      if (!available.length) {
        setFindError(t("library.skillsPage.noSkills"));
        return;
      }
      const candidates = available
        .map((skill) => ({
          key: skill.key,
          name: skill.name,
          description: skill.description,
          existing: inLibrary(skill.directory, skill.key),
          skill,
        }))
        .sort((a, b) => a.name.localeCompare(b.name));
      setFound({
        source: `${parsed.owner}/${parsed.name}`,
        candidates,
        selected: candidates
          .filter((candidate) => !candidate.existing)
          .map((candidate) => candidate.key),
        agents: installAgents,
      });
    } catch (reason) {
      if (mounted.current) setFindError(findErrorText(reason));
    } finally {
      if (mounted.current) setFinding(false);
    }
  }
  function chooseRecipe(recipe: LibraryRecommendation) {
    if (recipe.kind !== "skills") return;
    const { owner, name, branch, directory } = recipe.repository;
    const skill: DiscoverableSkill = {
      key: `${owner}/${name}:${directory}`,
      name: recipe.name,
      description: t(recipe.descriptionKey),
      directory,
      repoOwner: owner,
      repoName: name,
      repoBranch: branch,
    };
    const existing = inLibrary(directory, skill.key);
    setSource(`${owner}/${name}`);
    setFindError("");
    setFound({
      source: `${owner}/${name}`,
      candidates: [
        {
          key: skill.key,
          name: skill.name,
          description: skill.description,
          existing,
          skill,
        },
      ],
      selected: existing ? [] : [skill.key],
      agents: installAgents,
    });
    installCard.current?.scrollIntoView?.({ block: "center" });
  }
  const install = (current: Found) =>
    run("install", async () => {
      const picked = current.candidates.filter(
        (candidate) =>
          current.selected.includes(candidate.key) && !candidate.existing,
      );
      const added: { id: string; name: string }[] = [];
      const problems: string[] = [];
      for (const candidate of picked) {
        try {
          if (candidate.folder) {
            const { id } = await libraryApi.importSkillDirectory(
              await expandHome(candidate.folder.path),
              candidate.folder.leaf,
              candidate.folder.leaf,
            );
            added.push({ id, name: candidate.name });
          } else if (candidate.skill) {
            const installed = await skillsApi.installUnified(
              candidate.skill,
              "claude",
              data!.epoch,
            );
            added.push({ id: installed.id, name: candidate.name });
          }
        } catch (reason) {
          problems.push(
            t("library.skillsPage.skipped", {
              name: candidate.name,
              error: findErrorText(reason),
            }),
          );
        }
      }
      for (const { id, name } of added) {
        if (!current.agents.length) break;
        try {
          await giveTo(id, current.agents);
        } catch (reason) {
          problems.push(
            t("library.skillsPage.notGiven", {
              name,
              error: libraryError(reason, t),
            }),
          );
        }
      }
      if (mounted.current) {
        if (problems.length) setError(problems.join(" "));
        if (added.length) {
          setFound(null);
          setSource("");
        }
      }
      return added.length
        ? t("library.skillsPage.installed", { count: added.length })
        : undefined;
    });

  const toggle = (row: SkillDefinition, tool: string, cell: SkillCell) =>
    run(`${row.id}:${tool}`, async () => {
      const { tools, adopt } = skillToggle(row, tool, cell);
      await assign(row.id, tools, adopt);
    });
  const toggleAll = (row: SkillDefinition) =>
    run(`${row.id}:all`, async () => {
      const { tools, adopt } = skillAll(row, columns);
      await assign(row.id, tools, adopt);
    });
  const [synced, setSynced] = useState<boolean>();
  const resync = async (change: SkillDraftChange) => {
    try {
      if (change.state === "updated" && change.nativeApplyRequired) {
        const { row } = await fresh(change.resourceId);
        const tools = desiredSkillTools(row);
        if (tools.length) await assign(row.id, tools);
        if (mounted.current) setSynced(true);
      }
    } catch (reason) {
      if (mounted.current) setSynced(false);
      throw reason;
    } finally {
      await query.refetch();
      native.rescan();
      onChanged();
    }
  };
  async function saveSync(method: SkillSyncMethod) {
    if (savingSync) return;
    setSavingSync(true);
    setError("");
    setNotice("");
    try {
      const { webdavSync: _ignored, ...current } = await settingsApi.get();
      await settingsApi.save({ ...current, skillSyncMethod: method });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      if (mounted.current)
        setNotice(
          t("library.skillsPage.syncSaved", {
            method: t(`library.skillsPage.${method}`),
          }),
        );
    } catch (reason) {
      if (mounted.current) setError(libraryError(reason, t));
    } finally {
      if (mounted.current) setSavingSync(false);
    }
  }
  function closeDialog() {
    if (busy || checking) return;
    if (dialog?.type === "update") {
      const checked = dialog.store.values.get("skillUpdateCheck") as
        | SkillUpdateCheck
        | undefined;
      if (checked?.candidate)
        void libraryApi
          .discardSkillUpdate(checked.candidate.ticketId)
          .catch(() => undefined);
    }
    setDialog(null);
  }

  // Found in the agents, not in the library.
  const entries = (native.inventory?.entries ?? [])
    .filter((entry) => !entry.matchedResourceIds.length)
    .sort((a, b) => a.name.localeCompare(b.name));
  const importable = entries.filter((entry) => !entry.conflict);
  const importNotice = (result: InventoryImport) => {
    const notes = [
      result.imported === 1 &&
      result.items.some((item) => item.status === "imported")
        ? t("library.skillsPage.importedOne", {
            name: result.items.find((item) => item.status === "imported")!.name,
          })
        : t("library.skillsPage.imported", { count: result.imported }),
    ];
    for (const item of result.items) {
      // "existing": the library already has these very files, which is fine.
      if (item.status === "stale" || item.status === "failed")
        notes.push(
          t("library.skillsPage.skipped", {
            name: item.name,
            error: libraryError(item.reason, t),
          }),
        );
      for (const missed of item.notAdopted ?? [])
        notes.push(
          t("library.skillsPage.notAdopted", {
            agents: labelFor(missed.targetId),
            reason: libraryError(missed.reason, t),
          }),
        );
    }
    return notes.join(" ");
  };
  const importEntries = (key: string, chosen: FoundEntry[], skipped = 0) =>
    run(key, async () => {
      const inventory = native.inventory;
      if (!inventory || !chosen.length)
        return t("library.skillsPage.importNone");
      const result = await libraryApi.importInventory(
        inventory.ticketId,
        chosen.map((entry) => entry.entryId),
        data!.epoch,
      );
      return [
        importNotice(result),
        skipped
          ? t("library.skillsPage.importSkipped", { count: skipped })
          : "",
      ]
        .filter(Boolean)
        .join(" ");
    });

  // The library's skills, filtered and in the order picked.
  const needle = filter.trim().toLocaleLowerCase();
  const visible = rows
    .filter(
      (row) =>
        !needle ||
        [row.name, row.description ?? "", row.directory, row.repository ?? ""]
          .join(" ")
          .toLocaleLowerCase()
          .includes(needle),
    )
    .sort((a, b) =>
      sort === "za"
        ? b.name.localeCompare(a.name)
        : a.name.localeCompare(b.name),
    );
  const groups =
    sort === "source"
      ? [
          ...[...new Set(visible.map(repositoryOf).filter(Boolean))]
            .sort((a, b) => a!.localeCompare(b!))
            .map((repository) => ({
              key: repository!,
              repository,
              rows: visible.filter((row) => repositoryOf(row) === repository),
            })),
          {
            key: "local",
            repository: undefined,
            rows: visible.filter((row) => !repositoryOf(row)),
          },
        ].filter((group) => group.rows.length)
      : [{ key: "all", repository: undefined, rows: visible }];

  const cellText = (cell: SkillCell, tool: string) => {
    const label = labelFor(tool);
    if (cell.reason && ["attention", "unsupported"].includes(cell.state))
      return libraryError(cell.reason, t);
    return t(`library.skillsPage.cell.${cell.state}`, { tool: label });
  };
  const openRepository = (repository: string) =>
    void settingsApi
      .openExternal(`https://github.com/${repository}`)
      .catch((reason) => setError(libraryError(reason, t)));

  function renderChips(row: SkillDefinition) {
    const all = skillAll(row, columns);
    const allText = t(
      all.allOn ? "library.skillsPage.allOff" : "library.skillsPage.allOn",
      { count: all.count },
    );
    return (
      <div className="flex shrink-0 items-center gap-[3px]">
        <button
          type="button"
          aria-label={`${row.name} · ${allText}`}
          title={allText}
          aria-pressed={all.allOn}
          aria-busy={busy === `${row.id}:all`}
          disabled={!writable || !!busy || !all.count}
          onClick={() => void toggleAll(row)}
          className={cn(
            "mr-1.5 h-6 rounded-[6px] border border-line-2 bg-card px-[7px] text-note font-medium text-ink-muted transition-colors hover:bg-pill disabled:pointer-events-none disabled:opacity-45",
            all.allOn &&
              "border-line text-ink shadow-[0_1px_1px_rgba(0,0,0,.05)]",
          )}
        >
          {t("library.skillsPage.all")}
        </button>
        {columns.map((tool) => {
          const cell = skillCell(row, tool);
          const text = cellText(cell, tool);
          const label = `${row.name} · ${labelFor(tool)} · ${text}`;
          const blocked = !cell.toggleable;
          return (
            <button
              key={tool}
              type="button"
              aria-label={label}
              title={text}
              aria-pressed={cell.on}
              aria-disabled={blocked || undefined}
              aria-busy={busy === `${row.id}:${tool}`}
              data-state-name={cell.state}
              disabled={!writable || !!busy}
              onClick={() => {
                if (!blocked) void toggle(row, tool, cell);
              }}
              className={cn(
                "relative grid h-6 w-6 shrink-0 place-items-center rounded-[6px] border border-line-2 bg-card transition-[background-color,border-color,transform] duration-150 ease-soft disabled:opacity-60",
                !blocked && "hover:bg-pill active:scale-[.92]",
                blocked && "cursor-default",
                cell.on && "border-line shadow-[0_1px_2px_rgba(0,0,0,.08)]",
                cell.state === "shared" && "border-dashed",
                cell.state === "adoptable" && "border-dashed",
                ["unavailable", "unsupported"].includes(cell.state) &&
                  "opacity-40",
              )}
            >
              <ToolBadge
                toolId={tool}
                size={16}
                variant="plain"
                rounded="md"
                className={cn(
                  "transition-[filter,opacity] duration-150",
                  !cell.on && "opacity-35 grayscale dark:opacity-25",
                  ["shared", "adoptable"].includes(cell.state) && "opacity-70",
                )}
              />
              {["attention", "pending"].includes(cell.state) && (
                <span
                  aria-hidden
                  className={cn(
                    "absolute -right-1 -top-1.5 text-micro font-semibold leading-none",
                    cell.state === "attention" ? "text-warn" : "text-ink-muted",
                  )}
                >
                  {cell.state === "attention" ? "!" : "±"}
                </span>
              )}
            </button>
          );
        })}
      </div>
    );
  }
  function renderRow(row: SkillDefinition) {
    const repository = repositoryOf(row);
    return (
      <div
        key={row.id}
        data-testid={`skill-row-${row.id}`}
        className="flex items-center gap-3 border-b border-line-2 px-3 py-2.5 last:border-b-0"
      >
        <SkillGlyph />
        <div className="min-w-0 flex-1">
          <p className="truncate text-ui font-medium text-ink">{row.name}</p>
          {row.description && (
            <p
              className="truncate text-note text-ink-muted"
              title={row.description}
            >
              {row.description}
            </p>
          )}
          {/* Grouped by source, the group's title already says where it is from. */}
          {sort !== "source" && (
            <p className="mt-0.5 truncate text-micro text-ink-faint">
              {repository ? (
                <button
                  type="button"
                  title={t("library.skillsPage.openRepository", {
                    repo: repository,
                  })}
                  onClick={() => openRepository(repository)}
                  className="font-mono hover:text-ink hover:underline hover:underline-offset-2"
                >
                  {repository}
                </button>
              ) : (
                t("library.skillsPage.local")
              )}
            </p>
          )}
        </div>
        {renderChips(row)}
        <div className="flex shrink-0 items-center">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="icon"
                size="icon"
                aria-label={t("library.skillsPage.more", { name: row.name })}
                disabled={!!busy}
              >
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem
                onSelect={() =>
                  setDialog({ type: "details", row, store: new DraftStore() })
                }
              >
                {t("library.skillsPage.details")}
              </DropdownMenuItem>
              <DropdownMenuItem
                onSelect={() =>
                  setDialog({ type: "update", row, store: new DraftStore() })
                }
              >
                {t("library.skillsPage.checkUpdate")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Button
            variant="dangerGhost"
            size="icon"
            aria-label={t("library.skillsPage.remove", { name: row.name })}
            title={t("library.skillsPage.remove", { name: row.name })}
            disabled={!writable || !!busy || row.revision === undefined}
            onClick={() => setDialog({ type: "remove", row })}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    );
  }
  function renderFound(entry: FoundEntry) {
    const locations = entry.locations ?? [];
    const agents = [
      ...new Set([
        ...locations.map((location) => location.targetId),
        ...entry.sources.map((source) => source.toolId),
      ]),
    ].filter((tool) => tool !== "shared");
    return (
      <div
        key={entry.entryId}
        data-testid={`found-row-${entry.entryId}`}
        className="flex items-center gap-3 border-b border-line-2 px-3 py-2.5 last:border-b-0"
      >
        <SkillGlyph />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <p className="truncate text-ui font-medium text-ink">
              {entry.name}
            </p>
            {entry.conflict && (
              <span className="shrink-0 rounded-full bg-warn-soft px-1.5 text-micro text-warn">
                {t("library.skillsPage.conflict")}
              </span>
            )}
          </div>
          {entry.summary && (
            <p
              className="truncate text-note text-ink-muted"
              title={entry.summary}
            >
              {entry.summary}
            </p>
          )}
          {locations.length > 0 && (
            <div
              className="mt-0.5"
              title={locations
                .map((location) => location.displayPath)
                .join("\n")}
            >
              {locations.slice(0, 2).map((location, index) => (
                <p
                  key={`${location.targetId}:${location.displayPath}`}
                  className="truncate font-mono text-micro leading-4 text-ink-faint [font-variant-ligatures:none]"
                >
                  {location.targetId === "shared"
                    ? t("library.skillsPage.sharedIn", {
                        path: location.displayPath,
                      })
                    : location.displayPath}
                  {index === 1 && locations.length > 2 && (
                    <span className="ml-2 font-sans">
                      {t("library.skillsPage.moreLocations", {
                        count: locations.length - 2,
                      })}
                    </span>
                  )}
                </p>
              ))}
            </div>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {agents.map((tool) => (
            <span key={tool} title={labelFor(tool)}>
              <ToolBadge toolId={tool} size={16} variant="plain" rounded="md" />
            </span>
          ))}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <Button
            variant="secondary"
            size="sm"
            aria-label={t("library.skillsPage.importOne", { name: entry.name })}
            title={
              entry.conflict ? t("library.skillsPage.conflict") : undefined
            }
            disabled={
              !writable || !!busy || !native.inventory || entry.conflict
            }
            onClick={() =>
              void importEntries(`import:${entry.entryId}`, [entry])
            }
          >
            {t("library.skillsPage.import")}
          </Button>
          <Button
            variant="dangerGhost"
            size="icon"
            aria-label={t("library.skillsPage.deleteFound", {
              name: entry.name,
            })}
            title={t("library.skillsPage.deleteFound", { name: entry.name })}
            disabled={!writable || !!busy || !native.inventory}
            onClick={() => setDialog({ type: "deleteFound", entry })}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    );
  }

  const method = settings?.skillSyncMethod ?? "auto";
  const shownMethod =
    method === "copy" || (method === "auto" && isWindows())
      ? "copy"
      : "symlink";
  const pickedCount = found
    ? found.candidates.filter(
        (candidate) =>
          found.selected.includes(candidate.key) && !candidate.existing,
      ).length
    : 0;

  return (
    <section aria-label={t("library.tabs.skills")} className="space-y-5 pb-4">
      <div className="space-y-2.5">
        <p className="text-ctl text-ink-muted">
          {t("library.skillsPage.intro")}
        </p>
        <div
          ref={installCard}
          className="rounded-list border border-line bg-card"
        >
          <form
            className="flex items-center gap-2 py-1.5 pl-3 pr-1.5"
            onSubmit={(event) => {
              event.preventDefault();
              void find();
            }}
          >
            <Search
              aria-hidden
              className="h-3.5 w-3.5 flex-none text-ink-faint"
            />
            <input
              ref={sourceInput}
              value={source}
              onChange={(event) => {
                setSource(event.target.value);
                setFindError("");
              }}
              placeholder={t("library.skillsPage.sourcePlaceholder")}
              aria-label={t("library.skillsPage.source")}
              disabled={!writable || finding}
              className="h-7 min-w-0 flex-1 bg-transparent text-ui text-ink outline-none placeholder:text-ink-faint disabled:opacity-60"
            />
            <Button
              type="submit"
              variant="secondary"
              disabled={!writable || finding || !!busy || !source.trim()}
            >
              {finding
                ? t("library.skillsPage.finding")
                : t("library.skillsPage.find")}
            </Button>
          </form>
          {findError && (
            <p
              role="alert"
              className="border-t border-line-2 px-3 py-2 text-ctl text-danger"
            >
              {findError}
            </p>
          )}
          {found && (
            <div className="border-t border-line-2">
              <div className="flex flex-wrap items-center gap-2 px-3 pt-2.5">
                <p className="text-ctl font-medium text-ink">
                  {t("library.skillsPage.foundIn", {
                    source: found.source,
                    count: found.candidates.length,
                  })}
                </p>
                {found.candidates.length > 1 && (
                  <div className="ml-auto flex items-center gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() =>
                        setFound({
                          ...found,
                          selected: found.candidates
                            .filter((candidate) => !candidate.existing)
                            .map((candidate) => candidate.key),
                        })
                      }
                    >
                      {t("library.skillsPage.selectAll")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setFound({ ...found, selected: [] })}
                    >
                      {t("library.skillsPage.selectNone")}
                    </Button>
                  </div>
                )}
              </div>
              <ul className="max-h-72 overflow-y-auto px-1.5 py-1.5">
                {found.candidates.map((candidate) => (
                  <li key={candidate.key}>
                    <label
                      className={cn(
                        "flex cursor-pointer items-start gap-2.5 rounded-ctl px-1.5 py-1.5 hover:bg-pill",
                        candidate.existing &&
                          "cursor-default opacity-55 hover:bg-transparent",
                      )}
                    >
                      <Checkbox
                        className="mt-0.5"
                        aria-label={t("library.skillsPage.pick", {
                          name: candidate.name,
                        })}
                        checked={
                          candidate.existing ||
                          found.selected.includes(candidate.key)
                        }
                        disabled={candidate.existing || busy === "install"}
                        onCheckedChange={(checked) =>
                          setFound({
                            ...found,
                            selected: checked
                              ? [...found.selected, candidate.key]
                              : found.selected.filter(
                                  (key) => key !== candidate.key,
                                ),
                          })
                        }
                      />
                      <span className="min-w-0 flex-1">
                        <span className="flex items-center gap-2">
                          <span className="truncate text-ctl font-medium text-ink">
                            {candidate.name}
                          </span>
                          {candidate.existing && (
                            <span className="shrink-0 text-micro text-ink-faint">
                              {t("library.skillsPage.alreadyInLibrary")}
                            </span>
                          )}
                        </span>
                        {candidate.description && (
                          <span
                            className="block truncate text-note text-ink-muted"
                            title={candidate.description}
                          >
                            {candidate.description}
                          </span>
                        )}
                      </span>
                    </label>
                  </li>
                ))}
              </ul>
              <div className="flex flex-wrap items-center gap-2 border-t border-line-2 px-3 py-2">
                {installAgents.length > 0 && (
                  <div className="flex flex-wrap items-center gap-1">
                    <span className="mr-1 text-ctl text-ink-muted">
                      {t("library.skillsPage.for")}
                    </span>
                    {installAgents.map((tool) => {
                      const on = found.agents.includes(tool);
                      return (
                        <button
                          key={tool}
                          type="button"
                          aria-pressed={on}
                          aria-label={t("library.skillsPage.giveTo", {
                            tool: labelFor(tool),
                          })}
                          title={labelFor(tool)}
                          disabled={busy === "install"}
                          onClick={() =>
                            setFound({
                              ...found,
                              agents: on
                                ? found.agents.filter((item) => item !== tool)
                                : [...found.agents, tool],
                            })
                          }
                          className={cn(
                            "inline-flex h-6 items-center gap-1 rounded-[6px] border border-line-2 pl-1 pr-2 text-note text-ink-muted transition-colors hover:bg-pill",
                            on &&
                              "border-line bg-brand-soft text-ink shadow-[0_1px_1px_rgba(0,0,0,.05)]",
                          )}
                        >
                          <ToolBadge
                            toolId={tool}
                            size={14}
                            variant="plain"
                            rounded="md"
                            className={cn(!on && "opacity-40 grayscale")}
                          />
                          {labelFor(tool)}
                        </button>
                      );
                    })}
                  </div>
                )}
                <div className="ml-auto flex items-center gap-2">
                  <Button
                    variant="ghost"
                    disabled={busy === "install"}
                    onClick={() => setFound(null)}
                  >
                    {t("common.cancel")}
                  </Button>
                  <Button
                    disabled={!writable || !!busy || !pickedCount}
                    onClick={() => void install(found)}
                  >
                    {busy === "install"
                      ? t("library.skillsPage.installing")
                      : t("library.skillsPage.install", { count: pickedCount })}
                  </Button>
                </div>
              </div>
            </div>
          )}
        </div>
        {data && !writable && (
          <p role="status" className="text-ctl text-warn">
            {t("library.legacyReadOnly")}
          </p>
        )}
        {query.isError && (
          <p role="alert" className="text-ctl text-danger">
            {libraryError(query.error, t)}
          </p>
        )}
      </div>

      {rows.length > 0 && (
        <section
          aria-label={t("library.skillsPage.inLibrary")}
          className="space-y-2"
        >
          <div className="flex flex-wrap items-center gap-2">
            <SectionLabel>
              {t("library.skillsPage.inLibrary")}
              <span className="ml-1.5 font-normal tabular-nums text-ink-faint">
                {rows.length}
              </span>
            </SectionLabel>
            <div className="ml-auto flex items-center gap-2">
              {(rows.length > 8 || filter) && (
                <SearchInput
                  value={filter}
                  onChange={setFilter}
                  placeholder={t("library.skillsPage.filter", {
                    count: rows.length,
                  })}
                />
              )}
              <SegmentedControl
                aria-label={t("library.skillsPage.sort")}
                value={sort}
                onChange={setSort}
                options={[
                  {
                    value: "source",
                    label: t("library.skillsPage.sortSource"),
                  },
                  { value: "az", label: t("library.skillsPage.sortAz") },
                  { value: "za", label: t("library.skillsPage.sortZa") },
                ]}
              />
            </div>
          </div>
          {!visible.length && (
            <p className="rounded-list border border-dashed border-line px-4 py-6 text-center text-ctl text-ink-muted">
              {t("library.skillsPage.noMatch", { query: filter.trim() })}
            </p>
          )}
          {groups.map((group) => (
            <div key={group.key} className="space-y-1.5">
              {sort === "source" && (
                <h3 className="flex items-center gap-1.5 px-1 text-sub text-ink-muted">
                  {group.repository ? (
                    <span className="font-mono text-ink-2 [font-variant-ligatures:none]">
                      {group.repository}
                    </span>
                  ) : (
                    <span className="text-ink-2">
                      {t("library.skillsPage.local")}
                    </span>
                  )}
                  <span aria-hidden>·</span>
                  <span>
                    {t("library.skillsPage.groupCount", {
                      count: group.rows.length,
                    })}
                  </span>
                </h3>
              )}
              <div className="rounded-list border border-line bg-card">
                {group.rows.map(renderRow)}
              </div>
            </div>
          ))}
        </section>
      )}
      {!rows.length && query.isSuccess && (
        <p className="rounded-list border border-dashed border-line px-4 py-8 text-center text-ctl text-ink-muted">
          {t("library.skillsPage.empty")}
        </p>
      )}

      <section
        aria-label={t("library.skillsPage.syncTitle")}
        className="space-y-1.5"
      >
        <div className="flex flex-wrap items-center gap-3">
          <SectionLabel>{t("library.skillsPage.syncTitle")}</SectionLabel>
          <SegmentedControl
            aria-label={t("library.skillsPage.syncTitle")}
            value={shownMethod}
            disabled={!settings || savingSync}
            onChange={(value) => void saveSync(value)}
            options={[
              { value: "symlink", label: t("library.skillsPage.symlink") },
              { value: "copy", label: t("library.skillsPage.copy") },
            ]}
          />
        </div>
        <p className="text-note text-ink-muted">
          {t(
            shownMethod === "copy"
              ? "library.skillsPage.copyNote"
              : "library.skillsPage.symlinkNote",
          )}
        </p>
      </section>

      {!!native.error && !native.loading && (
        <p role="alert" className="text-ctl text-danger">
          {libraryError(native.error, t)}{" "}
          <Button variant="link" onClick={native.rescan}>
            {t("library.retry")}
          </Button>
        </p>
      )}
      {(entries.length > 0 || native.loading) && (
        <section
          aria-label={t("library.skillsPage.inAgents")}
          className="space-y-2"
        >
          <div className="flex flex-wrap items-center gap-2">
            <div className="min-w-0">
              <SectionLabel>{t("library.skillsPage.inAgents")}</SectionLabel>
              <p className="text-note text-ink-muted">
                {native.loading
                  ? t("library.skillsPage.scanning")
                  : t("library.skillsPage.inAgentsNote")}
              </p>
            </div>
            {entries.length > 0 && (
              <Button
                variant="secondary"
                className="ml-auto"
                disabled={
                  !writable || !!busy || !native.inventory || !importable.length
                }
                onClick={() =>
                  void importEntries(
                    "importAll",
                    importable,
                    entries.length - importable.length,
                  )
                }
              >
                {t("library.skillsPage.importAll")}
              </Button>
            )}
          </div>
          {entries.length > 0 && (
            <div className="rounded-list border border-line bg-card">
              {entries.map(renderFound)}
            </div>
          )}
        </section>
      )}

      <section
        ref={discoverSection}
        aria-label={t("library.skillsPage.discover")}
        className="space-y-2"
        style={{ scrollMarginTop: "var(--library-scroll-offset, 96px)" }}
      >
        <SectionLabel>{t("library.skillsPage.discover")}</SectionLabel>
        <LibraryDiscover
          kind="skills"
          busy={!!busy || !writable}
          searchInputRef={discoverSearch}
          onChoose={chooseRecipe}
          onOpenSource={(url) =>
            void settingsApi
              .openExternal(url)
              .catch((reason) => setError(libraryError(reason, t)))
          }
        />
      </section>

      {(error || notice) && (
        <div className="pointer-events-none sticky bottom-3 z-10 flex justify-center">
          <p
            role={error ? "alert" : "status"}
            className={cn(
              "pointer-events-auto max-w-[min(640px,100%)] rounded-ctl border border-line bg-card px-3 py-1.5 text-ctl shadow-[0_4px_16px_rgba(0,0,0,.10)]",
              error ? "text-danger" : "text-ink-2",
            )}
          >
            {[error, notice].filter(Boolean).join(" ")}
          </p>
        </div>
      )}

      <Dialog
        open={dialog?.type === "remove" || dialog?.type === "deleteFound"}
        onOpenChange={(open) => {
          if (!open) closeDialog();
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>
              {dialog?.type === "remove" &&
                t("library.skillsPage.removeTitle", { name: dialog.row.name })}
              {dialog?.type === "deleteFound" &&
                t("library.skillsPage.deleteFoundTitle", {
                  name: dialog.entry.name,
                })}
            </DialogTitle>
            <DialogDescription>
              {dialog?.type === "remove"
                ? t("library.skillsPage.removeBody")
                : t("library.skillsPage.deleteFoundBody")}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="secondary" disabled={!!busy} onClick={closeDialog}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              disabled={!!busy}
              onClick={() => {
                if (dialog?.type === "remove") {
                  const { row } = dialog;
                  // Close either way, so a failure shows on the page.
                  void run("remove", async () => {
                    // Anything Ofox still holds (assigned, written, or left
                    // behind by a failed write) is withdrawn first.
                    const latest = (await fresh(row.id)).row;
                    if (
                      latest.targets.some(
                        (target) =>
                          target.desired ||
                          target.lastAppliedRevision != null ||
                          !["unassigned", "shared"].includes(target.status),
                      )
                    )
                      await assign(row.id, []);
                    const current = await fresh(row.id);
                    await libraryApi.deleteSkill(
                      row.id,
                      current.row.revision!,
                      current.epoch,
                    );
                    return t("library.skillsPage.removed", { name: row.name });
                  }).then(() => setDialog(null));
                } else if (dialog?.type === "deleteFound") {
                  const { entry } = dialog;
                  void run("deleteFound", async () => {
                    const inventory = native.inventory;
                    if (!inventory) throw new Error("library.failed");
                    const result = await libraryApi.removeFoundSkill(
                      inventory.ticketId,
                      entry.entryId,
                      [],
                      data!.epoch,
                    );
                    return t("library.skillsPage.deletedFound", {
                      name: entry.name,
                      count: result.removed.length,
                    });
                  }).then(() => setDialog(null));
                }
              }}
            >
              {dialog?.type === "deleteFound"
                ? t("library.skillsPage.deleteFoundConfirm")
                : t("library.skillsPage.removeConfirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog
        open={dialog?.type === "details" || dialog?.type === "update"}
        onOpenChange={(open) => {
          if (!open) closeDialog();
        }}
      >
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>
              {dialog?.type === "details" && dialog.row.name}
              {dialog?.type === "update" &&
                t("library.skillsPage.updateTitle", { name: dialog.row.name })}
            </DialogTitle>
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto px-[18px] py-4 text-ctl">
            {dialog?.type === "details" && data && (
              <SkillDetails
                store={dialog.store}
                skill={dialog.row}
                epoch={data.epoch}
              />
            )}
            {dialog?.type === "update" && data && (
              <SkillUpdate
                store={dialog.store}
                skill={dialog.row}
                epoch={data.epoch}
                readOnly={!writable}
                externalBusy={!!busy}
                onBusyChange={setChecking}
                onChanged={resync}
                updatedNote={
                  synced === undefined
                    ? undefined
                    : t(
                        synced
                          ? "library.skillsPage.updateSynced"
                          : "library.skillsPage.updateNotSynced",
                      )
                }
              />
            )}
          </div>
        </DialogContent>
      </Dialog>
    </section>
  );
}
