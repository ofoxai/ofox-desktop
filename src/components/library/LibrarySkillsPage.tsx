import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { homeDir } from "@tauri-apps/api/path";
import {
  ArrowLeft,
  FileText,
  MoreHorizontal,
  Search,
  Trash2,
} from "lucide-react";
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
import {
  InteractionScope,
  useInteractionActive,
} from "@/components/ui/interaction-scope";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { cn } from "@/lib/utils";
import type { SkillSyncMethod } from "@/types";
import { LibrarySkillDiscover } from "./LibrarySkillDiscover";
import { libraryScroller } from "./useLibraryInline";
import {
  canResolveSkill,
  foundSkillAgents,
  foundSkills,
  nativeSkillDifferences,
  type FoundSkill,
} from "./foundSkills";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
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
import { collectedRepositorySkill } from "./skillIdentity";
import {
  distributeSkill,
  type CollectedSkillVersion,
  type SkillDistributionResult,
} from "./skillDistribution";

type FoundEntry = LibraryInventory["entries"][number];
type Candidate = {
  key: string;
  name: string;
  description?: string;
  /** Already in the library: shown, not installable. */
  existing: boolean;
  skill?: DiscoverableSkill;
  folder?: { path: string; leaf: string };
  archive?: { path: string; directory?: string };
  /** Retained after a partial distribution so Retry never downloads it again. */
  collectedId?: string;
  collectedVersion?: CollectedSkillVersion;
  collectionStatus?: "collected" | "skipped" | "available";
  results?: SkillDistributionResult[];
  issue?: string;
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
  | { type: "deleteFound"; entry: FoundEntry }
  | {
      type: "conflict";
      row: SkillDefinition;
      tool: string;
      epoch: number;
      routing: string;
    };
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
async function expandHome(path: string) {
  if (!path.startsWith("~")) return path;
  return (await homeDir()).replace(/[\\/]+$/, "") + path.slice(1);
}
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
    <span
      aria-hidden
      className="mt-0.5 grid h-[26px] w-[26px] shrink-0 place-items-center self-start rounded-[7px] bg-pill text-ink-2"
    >
      <FileText className="h-[15px] w-[15px]" />
    </span>
  );
}

export function LibrarySkillsPage({
  managementRequest,
  registerRefresh,
  registerDiscover,
  onChanged,
}: {
  managementRequest?: number;
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
    routingSettings?.openclawConfigDir,
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
  const [completedInstall, setCompletedInstall] = useState<Candidate[]>([]);
  const [filter, setFilter] = useState("");
  const [sort, setSort] = useState<Sort>("source");
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [checking, setChecking] = useState(false);
  const [savingSync, setSavingSync] = useState(false);
  const [discoverOpen, setDiscoverOpen] = useState(false);
  const [discoverVisited, setDiscoverVisited] = useState(false);
  const page = useRef<HTMLElement>(null);
  const discoverTrigger = useRef<HTMLElement | null>(null);
  const listScroll = useRef(0);
  const pendingFocus = useRef<"discover" | "back" | "install" | null>(null);
  const conflictOpener = useRef<HTMLElement | null>(null);
  const installCard = useRef<HTMLDivElement>(null);
  const sourceInput = useRef<HTMLInputElement>(null);
  const discoverSection = useRef<HTMLElement>(null);
  const discoverSearch = useRef<HTMLInputElement>(null);
  const mounted = useRef(true);
  function focusDiscovery() {
    discoverSection.current?.scrollIntoView?.({ block: "start" });
    discoverSearch.current?.focus({ preventScroll: true });
  }
  useLayoutEffect(() => {
    if (managementRequest == null) return;
    pendingFocus.current = "install";
    setDiscoverOpen(false);
  }, [managementRequest]);
  useLayoutEffect(() => {
    const intent = pendingFocus.current;
    // An explicit location can reveal management in a second commit.
    if (active && intent === "install" && discoverOpen) return;
    pendingFocus.current = null;
    if (!active || !intent) return;
    if (intent === "discover") focusDiscovery();
    else if (intent === "back") {
      const scroll = libraryScroller(page.current);
      if (scroll) scroll.scrollTop = listScroll.current;
      const trigger = discoverTrigger.current;
      if (trigger?.isConnected) trigger.focus({ preventScroll: true });
      else sourceInput.current?.focus({ preventScroll: true });
    } else {
      installCard.current?.scrollIntoView?.({ block: "start" });
      sourceInput.current?.focus({ preventScroll: true });
    }
  }, [active, discoverOpen, found, managementRequest]);
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
      registerDiscover("skills", (trigger) => {
        discoverTrigger.current = trigger;
        if (discoverOpen) {
          focusDiscovery();
          return;
        }
        listScroll.current = libraryScroller(page.current)?.scrollTop ?? 0;
        pendingFocus.current = "discover";
        setDiscoverVisited(true);
        setDiscoverOpen(true);
      }),
    [registerDiscover, discoverOpen],
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
  const inLibrary = (_directory: string, key?: string) =>
    key !== undefined && rows.some((row) => collectedRepositorySkill(row, key));
  const findErrorText = (reason: unknown) =>
    /skillDocumentMissing/.test(String(reason))
      ? t("library.skillsPage.noSkillDoc")
      : libraryError(reason, t);
  const candidateFeedback = (candidate: Candidate) => (
    <>
      {candidate.collectionStatus && (
        <span role="status" className="block text-note text-ink-muted">
          {candidate.collectionStatus === "available"
            ? t("library.skillsPage.alreadyInLibrary")
            : t(`library.skillsPage.collection.${candidate.collectionStatus}`)}
        </span>
      )}
      {candidate.issue && (
        <span role="alert" className="block text-note text-danger">
          {candidate.issue}
        </span>
      )}
      {candidate.results?.map((result) => (
        <span
          key={result.tool}
          role="status"
          className={cn(
            "block text-note",
            ["failed", "conflict"].includes(result.status)
              ? "text-danger"
              : "text-ink-muted",
          )}
        >
          {t(`library.skillsPage.distribution.${result.status}`, {
            tool: labelFor(result.tool),
          })}
          {result.reason && ` · ${libraryError(result.reason, t)}`}
        </span>
      ))}
    </>
  );

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
    setCompletedInstall([]);
    try {
      if (parsed.kind !== "repository") {
        setFound({
          source: parsed.path,
          candidates: [
            {
              key: parsed.path,
              name: parsed.leaf,
              description: parsed.path,
              existing: inLibrary(parsed.leaf),
              ...(parsed.kind === "archive"
                ? { archive: { path: parsed.path } }
                : { folder: parsed }),
            },
          ],
          selected: inLibrary(parsed.leaf) ? [] : [parsed.path],
          agents: installAgents,
        });
        return;
      }
      const lookup = {
        owner: parsed.owner,
        name: parsed.name,
        branch: parsed.branch ?? "",
        enabled: true,
      };
      const discovered = await skillsApi.discoverRepository(lookup);
      const branch = lookup.branch;
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
    setCompletedInstall([]);
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
    chooseDiscovered(skill);
  }
  function chooseDiscovered(skill: DiscoverableSkill) {
    pendingFocus.current = "install";
    setDiscoverOpen(false);
    setCompletedInstall([]);
    const { directory, repoOwner: owner, repoName: name } = skill;
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
  }
  const install = (current: Found) =>
    run("install", async () => {
      setCompletedInstall([]);
      const selected = current.candidates.filter(
        (candidate) =>
          current.selected.includes(candidate.key) && !candidate.existing,
      );
      const expanded = new Map<string, Candidate[]>();
      for (const candidate of selected) {
        if (!candidate.archive || candidate.collectedId) continue;
        try {
          const result = await skillsApi.collectZip(
            await expandHome(candidate.archive.path),
            data!.epoch,
            candidate.archive.directory
              ? [candidate.archive.directory]
              : undefined,
          );
          expanded.set(
            candidate.key,
            result.map((entry) => ({
              key: `${candidate.archive!.path}#${entry.directory}`,
              name: entry.name,
              description: entry.directory,
              existing: false,
              archive: {
                path: candidate.archive!.path,
                directory: entry.directory,
              },
              collectedId: entry.collected?.id,
              collectedVersion: entry.collected,
              collectionStatus:
                entry.status === "collected" || entry.status === "skipped"
                  ? entry.status
                  : undefined,
              issue: entry.reason ? libraryError(entry.reason, t) : undefined,
            })),
          );
        } catch (reason) {
          expanded.set(candidate.key, [
            { ...candidate, issue: findErrorText(reason) },
          ]);
        }
      }
      const picked = selected.flatMap(
        (candidate) => expanded.get(candidate.key) ?? [candidate],
      );
      const updated = new Map<string, Candidate>();
      let added = [...expanded.values()]
        .flat()
        .filter((entry) => entry.collectionStatus === "collected").length;
      for (const candidate of picked) {
        const next: Candidate = { ...candidate, issue: undefined };
        try {
          if (next.collectedId) {
            // A previous attempt already committed the download to the library.
          } else if (candidate.archive) {
            throw new Error(candidate.issue ?? "library.skillWriteFailed");
          } else if (candidate.folder) {
            const collected = await libraryApi.importSkillDirectory(
              await expandHome(candidate.folder.path),
              candidate.folder.leaf,
              candidate.folder.leaf,
              data!.epoch,
            );
            next.collectedId = collected.id;
            next.collectedVersion = collected;
            next.collectionStatus = "available";
            added++;
          } else if (candidate.skill) {
            const installed = await skillsApi.collect(
              candidate.skill,
              data!.epoch,
            );
            next.collectedId = installed.id;
            next.collectedVersion = installed;
            next.collectionStatus = "available";
            added++;
          }
          if (!next.collectedId) throw new Error("library.resourceMissing");
          if (!next.collectedVersion)
            throw new Error("library.revisionChanged");
          const done = new Set(
            next.results
              ?.filter((result) =>
                ["applied", "skipped"].includes(result.status),
              )
              .map((result) => result.tool),
          );
          const results = await distributeSkill(
            next.collectedId,
            current.agents.filter((tool) => !done.has(tool)),
            next.collectedVersion,
          );
          next.results = [
            ...(next.results ?? []).filter((result) => done.has(result.tool)),
            ...results,
          ];
        } catch (reason) {
          next.issue = candidate.issue ?? findErrorText(reason);
        }
        updated.set(candidate.key, next);
      }
      if (mounted.current) {
        const candidates = current.candidates.flatMap((candidate) =>
          (expanded.get(candidate.key) ?? [candidate]).map(
            (entry) => updated.get(entry.key) ?? entry,
          ),
        );
        const failed = candidates.filter(
          (candidate) =>
            candidate.issue ||
            candidate.results?.some((result) =>
              ["failed", "conflict"].includes(result.status),
            ),
        );
        if (failed.length) {
          setFound({
            ...current,
            candidates,
            selected: failed.map((candidate) => candidate.key),
          });
        } else {
          setCompletedInstall(
            candidates.filter((candidate) => candidate.collectedId),
          );
          setFound(null);
          setSource("");
        }
      }
      return added
        ? t("library.skillsPage.installed", { count: added })
        : undefined;
    });

  const toggle = (row: SkillDefinition, tool: string, cell: SkillCell) =>
    run(`${row.id}:${tool}`, async () => {
      if (["off", "pending"].includes(cell.state)) {
        if (row.revision === undefined)
          throw new Error("library.revisionChanged");
        const results = await distributeSkill(row.id, [tool], {
          epoch: data!.epoch,
          revision: row.revision,
          artifactHash: row.artifactHash,
        });
        const failure = results.find((result) =>
          ["failed", "conflict"].includes(result.status),
        );
        if (failure) throw new Error(failure.reason ?? "library.failed");
        return;
      }
      const { tools, adopt } = skillToggle(row, tool, cell);
      await assign(row.id, tools, adopt);
    });
  const toggleAll = (row: SkillDefinition) =>
    run(`${row.id}:all`, async () => {
      const all = skillAll(row, columns);
      if (all.allOn) {
        await assign(row.id, all.tools, all.adopt);
        return;
      }
      if (row.revision === undefined)
        throw new Error("library.revisionChanged");
      const results = await distributeSkill(
        row.id,
        columns.filter((tool) =>
          ["off", "pending", "adoptable"].includes(skillCell(row, tool).state),
        ),
        {
          epoch: data!.epoch,
          revision: row.revision,
          artifactHash: row.artifactHash,
        },
      );
      const failure = results.find((result) =>
        ["failed", "conflict"].includes(result.status),
      );
      if (failure) throw new Error(failure.reason ?? "library.failed");
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
  const entries = foundSkills(native.inventory?.entries ?? [], rows);
  const importable = entries.map(({ entry }) => entry);
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
  const importEntries = (key: string, chosen: FoundEntry[]) =>
    run(key, async () => {
      const inventory = native.inventory;
      if (!inventory || !chosen.length)
        return t("library.skillsPage.importNone");
      const result = await libraryApi.importInventory(
        inventory.ticketId,
        chosen.map((entry) => entry.entryId),
        data!.epoch,
      );
      return importNotice(result);
    }).then((succeeded) => {
      if (succeeded && mounted.current && active) sourceInput.current?.focus();
      return succeeded;
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
    if (
      /library\.(skillNameConflict|skillEntryChanged)/.test(cell.reason ?? "")
    )
      return t("library.skillsPage.resolveHint", { agents: label });
    if (cell.reason && ["attention", "unsupported"].includes(cell.state))
      return libraryError(cell.reason, t);
    return t(`library.skillsPage.cell.${cell.state}`, { tool: label });
  };
  const openRepository = (repository: string) =>
    void settingsApi
      .openExternal(`https://github.com/${repository}`)
      .catch((reason) => setError(libraryError(reason, t)));

  const openConflict = (row: SkillDefinition, tool: string) => {
    conflictOpener.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    setDialog({
      type: "conflict",
      row,
      tool,
      epoch: data!.epoch,
      routing: routingKey,
    });
  };
  const resolveConflict = (useLibrary: boolean) => {
    if (dialog?.type !== "conflict") return;
    const current = dialog;
    void run(`resolve:${current.row.id}:${current.tool}`, async () => {
      if (current.routing !== routingKey)
        throw new Error("library.inventoryChanged");
      if (current.row.revision === undefined)
        throw new Error("library.revisionChanged");
      if (useLibrary)
        await commit(
          libraryApi.previewSkillReplacement(
            current.row.id,
            current.tool,
            current.row.revision,
            current.epoch,
          ),
        );
      else
        await libraryApi.keepSkillNative(
          current.row.id,
          current.tool,
          current.row.revision,
          current.epoch,
        );
      return t(
        useLibrary
          ? "library.skillsPage.usedLibrary"
          : "library.skillsPage.keptNative",
        { tool: labelFor(current.tool) },
      );
    }).then(() => {
      if (mounted.current) setDialog(null);
    });
  };
  function differenceBadge(
    agents: string[],
    onClick?: () => void,
    name?: string,
    nativeOnly = false,
  ) {
    if (!agents.length) return null;
    const tools = agents.map(labelFor).join("、");
    const text = t("library.skillsPage.differsIn", { agents: tools });
    const style =
      "inline-flex max-w-full shrink-0 items-center gap-1.5 rounded-[7px] bg-warn-soft px-2 py-1 text-note text-warn";
    return (
      <TooltipProvider delayDuration={150}>
        <Tooltip>
          <TooltipTrigger asChild>
            {onClick ? (
              <button
                type="button"
                className={cn(style, "hover:brightness-95 disabled:opacity-50")}
                aria-label={`${name} · ${text}`}
                disabled={!writable || !!busy}
                onClick={onClick}
              >
                <span
                  aria-hidden
                  className="h-1.5 w-1.5 shrink-0 rounded-full bg-current"
                />
                {text}
              </button>
            ) : (
              <span tabIndex={0} className={style}>
                <span
                  aria-hidden
                  className="h-1.5 w-1.5 shrink-0 rounded-full bg-current"
                />
                {text}
              </span>
            )}
          </TooltipTrigger>
          <TooltipContent>
            {t(
              onClick
                ? "library.skillsPage.resolveHint"
                : nativeOnly
                  ? "library.skillsPage.nativeDifferenceHint"
                  : "library.skillsPage.differsHint",
              { agents: tools },
            )}
          </TooltipContent>
        </Tooltip>
      </TooltipProvider>
    );
  }
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
          const resolvable = canResolveSkill(row, tool);
          const blocked = !cell.toggleable && !resolvable;
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
                if (resolvable) openConflict(row, tool);
                else if (!blocked) void toggle(row, tool, cell);
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
    const differences = [
      ...new Set([
        ...columns.filter((tool) => canResolveSkill(row, tool)),
        ...nativeSkillDifferences(native.inventory?.entries ?? [], row),
      ]),
    ];
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
        <div className="flex max-w-[58%] flex-wrap items-center justify-end gap-2">
          {renderChips(row)}
          {differences.map((tool) => (
            <span key={tool}>
              {differenceBadge(
                [tool],
                canResolveSkill(row, tool)
                  ? () => openConflict(row, tool)
                  : undefined,
                row.name,
                true,
              )}
            </span>
          ))}
        </div>
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
  function renderFound({ entry, others }: FoundSkill) {
    const locations = entry.locations ?? [];
    const agents = foundSkillAgents(entry);
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
        <div className="flex max-w-[52%] flex-wrap items-center justify-end gap-2">
          {agents.map((tool) => (
            <span key={tool} title={labelFor(tool)}>
              <ToolBadge toolId={tool} size={16} variant="plain" rounded="md" />
            </span>
          ))}
          {differenceBadge(others)}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <Button
            variant="secondary"
            size="sm"
            aria-label={t("library.skillsPage.importOne", { name: entry.name })}
            disabled={!writable || !!busy || !native.inventory}
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

  const affected =
    dialog?.type === "conflict"
      ? (
          dialog.row.targets.find((target) => target.toolId === dialog.tool)
            ?.readers ?? []
        ).filter((tool) => tool !== dialog.tool)
      : [];
  return (
    <section
      ref={page}
      aria-label={t("library.tabs.skills")}
      className="space-y-5 pb-4"
    >
      <div hidden={discoverOpen} className="space-y-5">
        <div className="space-y-2.5">
          <p className="text-ctl text-ink-muted">
            {t("library.skillsPage.intro")}
          </p>
          <div
            ref={installCard}
            className="rounded-list border border-line bg-card"
            style={{ scrollMarginTop: "var(--library-scroll-offset, 96px)" }}
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
                          {candidateFeedback(candidate)}
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
                        : t("library.skillsPage.install", {
                            count: pickedCount,
                          })}
                    </Button>
                  </div>
                </div>
              </div>
            )}
            {completedInstall.length > 0 && (
              <section
                aria-label={t("library.skillsPage.installResults")}
                className="border-t border-line-2 px-3 py-2"
              >
                <div className="flex items-center justify-between gap-2">
                  <h3 className="text-ctl font-medium text-ink">
                    {t("library.skillsPage.installResults")}
                  </h3>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setCompletedInstall([]);
                      sourceInput.current?.focus();
                    }}
                  >
                    {t("common.close")}
                  </Button>
                </div>
                <ul className="max-h-72 space-y-2 overflow-y-auto py-1">
                  {completedInstall.map((candidate) => (
                    <li key={candidate.key}>
                      <span className="text-ctl font-medium text-ink">
                        {candidate.name}
                      </span>
                      {candidate.description && (
                        <span className="block break-all text-note text-ink-muted">
                          {candidate.description}
                        </span>
                      )}
                      {candidateFeedback(candidate)}
                    </li>
                  ))}
                </ul>
              </section>
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
                    !writable ||
                    !!busy ||
                    !native.inventory ||
                    !importable.length
                  }
                  onClick={() => void importEntries("importAll", importable)}
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
      </div>
      {discoverVisited && (
        <section
          ref={discoverSection}
          hidden={!discoverOpen}
          aria-label={t("library.skillsPage.discover")}
          className="space-y-3"
          style={{ scrollMarginTop: "var(--library-scroll-offset, 96px)" }}
        >
          <div className="flex flex-wrap items-center gap-3">
            <Button
              variant="ghost"
              onClick={() => {
                pendingFocus.current = "back";
                setDiscoverOpen(false);
              }}
            >
              <ArrowLeft aria-hidden className="mr-1 h-3.5 w-3.5" />
              {t("library.skillsPage.backToSkills")}
            </Button>
            <SectionLabel>{t("library.skillsPage.discover")}</SectionLabel>
          </div>
          <InteractionScope active={active && discoverOpen}>
            <LibrarySkillDiscover
              rows={rows}
              enabled={active && discoverOpen}
              busy={!!busy || !writable}
              searchInputRef={discoverSearch}
              onChoose={chooseDiscovered}
              onChooseRecipe={chooseRecipe}
              onOpenSource={(url) =>
                void settingsApi
                  .openExternal(url)
                  .catch((reason) => setError(libraryError(reason, t)))
              }
            />
          </InteractionScope>
        </section>
      )}

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
        open={dialog?.type === "conflict"}
        onOpenChange={(open) => {
          if (!open) closeDialog();
        }}
      >
        <DialogContent
          className="sm:max-w-lg"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            if (!active) return;
            const opener = conflictOpener.current;
            if (
              opener?.isConnected &&
              !(opener instanceof HTMLButtonElement && opener.disabled)
            )
              opener.focus();
            else sourceInput.current?.focus();
          }}
          onEscapeKeyDown={(event) => {
            if (busy) event.preventDefault();
          }}
        >
          <DialogHeader>
            <DialogTitle>
              {dialog?.type === "conflict" &&
                t("library.skillsPage.resolveTitle", {
                  name: dialog.row.name,
                  tool: labelFor(dialog.tool),
                })}
            </DialogTitle>
            <DialogDescription>
              {t("library.skillsPage.resolveBody")}
              {affected.length > 0 && (
                <span className="mt-2 block">
                  {t("library.skillsPage.affectedAgents", {
                    agents: affected.map(labelFor).join("、"),
                  })}
                </span>
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="flex-wrap">
            <Button variant="secondary" disabled={!!busy} onClick={closeDialog}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="secondary"
              disabled={!!busy}
              onClick={() => resolveConflict(false)}
            >
              {dialog?.type === "conflict" &&
                t("library.skillsPage.keepNative", {
                  tool: labelFor(dialog.tool),
                })}
            </Button>
            <Button
              disabled={
                !!busy ||
                (dialog?.type === "conflict" &&
                  dialog.tool === "claude-desktop")
              }
              onClick={() => resolveConflict(true)}
            >
              {t("library.skillsPage.useLibrary")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

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
