import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";
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
  type McpComparisonSubject,
  type McpRecipeMatch,
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
import { libraryError } from "./libraryText";
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
  LIBRARY_RECOMMENDATIONS,
  createMcpDraft,
  cloneRecommendationDraft,
  type LibraryRecommendation,
} from "./libraryRecommendations";
import { useInteractionActive } from "@/components/ui/interaction-scope";
import { useLibraryInventory } from "./useLibraryInventory";
import { LibraryNativeSources } from "./LibraryNativeSources";
import { LibrarySourceDetail } from "./LibrarySourceDetail";
import { LibraryMultiSaveResult } from "./LibraryMultiSaveResult";
import { LibraryMcpCompare } from "./LibraryMcpCompare";
import { createMcpIdentifier } from "./libraryMcpInput";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { MoreHorizontal, SlidersHorizontal, X } from "lucide-react";
import {
  Popover,
  PopoverTrigger,
  PopoverContent,
} from "@/components/ui/popover";
import { LibraryInlineDetails } from "./LibraryInlineDetails";
import {
  useLibraryInline,
  LibraryUnsavedDialog,
  type LibraryLeaveGuard,
} from "./useLibraryInline";
import { settingsApi } from "@/lib/api/settings";
import { skillsApi } from "@/lib/api/skills";

import { definitionFromEdits } from "./libraryMcpInput";
/** Opaque IDs are ticket-local. A description is only useful when unique in both scans. */
function inventorySourceIdentity(source: LibraryInventory["sources"][number]) {
  if (!source.locationKind) return undefined;
  if (
    ["file", "directory"].includes(source.locationKind) &&
    !source.displayPath
  )
    return undefined;
  return JSON.stringify([
    source.toolId,
    source.label,
    source.locationKind,
    source.displayPath ?? null,
  ]);
}
function recipeDefinition(
  draft: import("./libraryRecommendations").LibraryMcpDraft,
) {
  return definitionFromEdits([
    { path: ["type"], action: "set", value: draft.transport },
    ...(draft.transport === "stdio"
      ? [
          { path: ["command"], action: "set" as const, value: draft.command },
          {
            path: ["args"],
            action: "set" as const,
            value:
              draft.args ??
              (draft.argsText ? draft.argsText.split(/\r?\n/) : []),
          },
        ]
      : [{ path: ["url"], action: "set" as const, value: draft.url }]),
    ...draft.environment.map((pair) => ({
      path: ["env", pair.name],
      action: "set" as const,
      value: pair.value,
    })),
    ...draft.headers.map((pair) => ({
      path: ["headers", pair.name],
      action: "set" as const,
      value: pair.value,
    })),
    ...(draft.advancedFields ?? []).map((field) => ({
      ...field,
      action: "set" as const,
    })),
  ]);
}
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
      focusPath?: string[];
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
  | { type: "source"; ticketId: string; entryId: string; sourceRef: string }
  | {
      type: "compare";
      subjects: { name: string; subject: McpComparisonSubject }[];
    }
  | {
      type: "multiResult";
      saved: { id: string; name: string }[];
      remaining: import("./libraryRecommendations").LibraryMcpDraft[];
      intent: "draft" | "apply";
      tools: string[];
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
type OpenView = OpenViewBody & {
  epoch: number;
  contextKey?: string;
  invalidated?: boolean;
};
export function LibrarySection({
  kind,
  registerRefresh,
  onChanged,
  registerDiscover,
  registerLeave,
  registerHistory,
}: {
  kind: LibraryKind;
  registerRefresh: (
    kind: LibraryKind,
    fn: () => Promise<unknown>,
  ) => () => void;
  registerLeave?: (kind: LibraryKind, guard: LibraryLeaveGuard) => () => void;
  registerHistory?: (kind: LibraryKind, open: () => void) => () => void;
  onChanged: () => void;
  registerDiscover: (
    kind: LibraryKind,
    open: (trigger: HTMLElement) => void,
  ) => () => void;
}) {
  const { t, i18n } = useTranslation();
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
    queryKey: ["library", kind, routingKey],
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
  const [newMcpId, setNewMcpId] = useDraftState(
    draftStore,
    "newMcpId",
    () => "",
  );
  const [formTools, setFormTools] = useDraftState<string[]>(
    draftStore,
    "formTools",
    () => [],
  );
  const [busy, setBusy] = useState(false);
  const running = useRef(false);
  const mounted = useRef(true);
  const trigger = useRef<HTMLElement | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const discoverRef = useRef<HTMLElement>(null);
  const discoverSearch = useRef<HTMLInputElement>(null);
  const historyRef = useRef<HTMLDivElement>(null);
  const [collected, setCollected] = useState<{
    id: string;
    epoch: number;
    routing: string;
  }>();
  const toolbarMenu = useRef<HTMLButtonElement>(null);
  const locateRequest = useRef(0);
  const toolbarAction = useRef<"inventory" | "import">();
  const [error, setError] = useState("");
  const [sourceLocateError, setSourceLocateError] = useState<{
    reason: "unsupported" | "sourceUnconfirmed";
    ticketId?: string;
    sourceIdentity?: string;
  }>();
  const [locateIntent, setLocateIntent] = useState<{
    id: string;
    target?: "library";
    request: number;
    context: string;
  }>();
  const focusedLocateRequest = useRef(0);
  const [notice, setNotice] = useState("");
  const [history, setHistory] = useState(false);
  const [agentFilter, setAgentFilter] = useState("all");
  const [statusFilter, setStatusFilter] = useState("all");
  const [sourceFilter, setSourceFilter] = useState("all");
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(
    () => new Set(),
  );
  const [located, setLocated] = useState<string>();
  const [nativeMatchVerified, setNativeMatchVerified] = useState(false);
  const [recipeMatches, setRecipeMatches] = useState<McpRecipeMatch[]>([]);
  const [sort, setSort] = useState("name");
  const [selecting, setSelecting] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [pending, setPending] = useState<
    Record<string, TargetDelta & { revision: number }>
  >({});
  const [batchTools, setBatchTools] = useState<string[]>([]);
  const [batches, setBatches] = useState<LibraryBatchPlan[]>([]);
  const [checks, setChecks] = useState<
    Record<
      string,
      {
        state: string;
        checkedAtMs: number;
        toolsCount: number;
        result: import("@/lib/api/library").McpCheckResult;
      }
    >
  >({});
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      locateRequest.current++;
    };
  }, []);
  useEffect(
    () =>
      registerRefresh(kind, async () => {
        const result = await query.refetch();
        if (result.error) throw result.error;
        native.rescan();
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
      if (close) {
        setView(null);
        inline.resetEditor();
      }
      await query.refetch();
      native.rescan();
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
  const open = (next: OpenViewBody, event: { currentTarget: HTMLElement }) =>
    inline.guard(() => {
      if (next.type === "discover") {
        jumpDiscover();
        return;
      }
      locateRequest.current++;
      trigger.current = event.currentTarget;
      setError("");
      setNotice("");
      const store = new DraftStore();
      inline.resetEditor();
      if (next.type === "edit" && "resource" in next && kind === "mcp")
        store.values.set(
          "friendlyMcp",
          createMcpDraft({
            name: next.resource.name,
            transport: (next.resource as McpDefinition).transport as
              | "stdio"
              | "http"
              | "sse",
          }),
        );
      setDraftStore(store);
      setView({
        ...next,
        epoch: query.data?.epoch ?? -1,
        contextKey: definitionsKey,
      });
    });
  function jumpDiscover() {
    discoverRef.current?.scrollIntoView?.({
      block: "start",
      behavior: window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
        ? "auto"
        : "smooth",
    });
    discoverSearch.current?.focus({ preventScroll: true });
  }
  useEffect(
    () => registerDiscover(kind, () => inline.guard(jumpDiscover)),
    [kind, registerDiscover],
  );
  useEffect(
    () =>
      registerHistory?.(kind, () => {
        void libraryApi
          .batches(kind)
          .then(setBatches)
          .catch(() => undefined);
        setHistory(true);
        requestAnimationFrame(() =>
          historyRef.current?.scrollIntoView?.({ block: "center" }),
        );
      }),
    [kind, registerHistory],
  );
  const close = () =>
    inline.guard(() => {
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
        inline.resetEditor();
        setError("");
      }
    });
  const data = query.data;
  const writable =
    Number.isSafeInteger(data?.epoch) && (data?.epoch ?? -1) >= 0;
  const rows: Resource[] =
    data?.instructions ?? data?.servers ?? data?.skills ?? [];
  const active = useInteractionActive();
  const inline = useLibraryInline(rows, data?.epoch, routingKey, active);
  useEffect(
    () => registerLeave?.(kind, inline.guard),
    [kind, registerLeave, inline.guard],
  );

  const definitionsKey =
    rows
      .map(
        (row) =>
          `${row.id}:${row.revision}:${"targets" in row ? row.targets.map((target) => `${target.toolId}:${target.path}:${target.available}`).join(",") : ""}`,
      )
      .sort()
      .join("|") +
    routingKey +
    JSON.stringify(
      data?.targets?.map((target) => [
        target.toolId,
        target.path,
        target.available,
      ]),
    );
  const native = useLibraryInventory(kind, data?.epoch, definitionsKey, active);
  useLayoutEffect(() => {
    if (!collected) return;
    if (
      !active ||
      collected.epoch !== data?.epoch ||
      collected.routing !== routingKey
    ) {
      setCollected(undefined);
      return;
    }
    const row = rows.find((row) => row.id === collected.id);
    if (!row) return;
    locateResource(row.id);
    setNotice(t("library.interaction.savedLocated", { name: row.name }));
    setCollected(undefined);
  }, [collected, rows, active, data?.epoch, routingKey]);
  const observedSources = (id: string) =>
    (native.inventory?.entries ?? [])
      .filter((entry) => entry.matchedResourceIds.includes(id))
      .flatMap((entry) => entry.sources);
  const observed = (id: string) => [
    ...new Set(observedSources(id).map((source) => source.toolId)),
  ];
  const switchedOff = (id: string) => {
    const sources = observedSources(id);
    return observed(id).filter((tool) =>
      sources.every((source) => source.toolId !== tool || source.disabled),
    );
  };
  const recipeGeneration = useRef(0);
  useEffect(() => {
    const generation = ++recipeGeneration.current;
    setRecipeMatches([]);
    setNativeMatchVerified(false);
    if (!writable) return;
    const language =
      i18n.resolvedLanguage?.split("-")[0] ?? i18n.language.split("-")[0];
    const mcpRecipes = LIBRARY_RECOMMENDATIONS.filter(
      (recipe) => recipe.kind === "mcp",
    ).map((recipe) => ({
      recipeId: recipe.id,
      name: recipe.name,
      definition: recipeDefinition(recipe.initialDraft),
    }));
    const builtins = LIBRARY_RECOMMENDATIONS.filter(
      (recipe): recipe is Exclude<LibraryRecommendation, { kind: "mcp" }> =>
        recipe.kind === kind && recipe.kind !== "mcp",
    ).map((recipe) =>
      recipe.kind === "instructions"
        ? {
            recipeId: recipe.id,
            content:
              recipe.contentByLanguage[
                language === "ja" ? "ja" : language === "en" ? "en" : "zh"
              ],
          }
        : {
            recipeId: recipe.id,
            repoOwner: recipe.repository.owner,
            repoName: recipe.repository.name,
            directory: recipe.repository.directory,
          },
    );
    const match = (ticket: string | undefined) =>
      kind === "mcp"
        ? libraryApi.matchMcpRecipes(mcpRecipes, ticket, data!.epoch)
        : libraryApi.matchBuiltinRecipes(kind, builtins, ticket, data!.epoch);
    const ticket = native.inventory?.ticketId;
    void match(ticket)
      .then((value) => {
        if (generation === recipeGeneration.current && Array.isArray(value)) {
          setRecipeMatches(value);
          setNativeMatchVerified(!!ticket);
        }
      })
      .catch(async () => {
        if (!ticket) return;
        try {
          const value = await match(undefined);
          if (generation === recipeGeneration.current && Array.isArray(value))
            setRecipeMatches(value);
        } catch {
          /* Optional matching is unavailable on older backends. */
        }
      });
    return () => {
      recipeGeneration.current++;
    };
  }, [
    kind,
    data?.epoch,
    definitionsKey,
    native.inventory?.ticketId,
    i18n.language,
  ]);
  useEffect(() => {
    setPending((previous) =>
      Object.fromEntries(
        Object.entries(previous).filter(([id, delta]) =>
          rows.some((row) => row.id === id && row.revision === delta.revision),
        ),
      ),
    );
  }, [definitionsKey, data?.epoch]);
  const lastEpoch = useRef(data?.epoch);
  const lastRouting = useRef(routingKey);
  useEffect(() => {
    if (
      (lastEpoch.current !== undefined && lastEpoch.current !== data?.epoch) ||
      lastRouting.current !== routingKey
    ) {
      setPending({});
      setSelected(new Set());
      setChecks({});
      setView((previous) =>
        previous ? { ...previous, invalidated: true } : previous,
      );
    }
    lastEpoch.current = data?.epoch;
    lastRouting.current = routingKey;
  }, [data?.epoch, routingKey]);
  const viewStale =
    !!view &&
    (view.invalidated ||
      view.epoch !== data?.epoch ||
      (view.contextKey !== undefined && view.contextKey !== definitionsKey) ||
      ("resource" in view &&
        view.resource.revision !== undefined &&
        view.resource.revision > 0 &&
        !rows.some(
          (row) =>
            row.id === view.resource.id &&
            row.revision === view.resource.revision,
        )));
  const sourceValid =
    view?.type !== "source" ||
    (view.epoch === data?.epoch &&
      view.contextKey === definitionsKey &&
      view.ticketId === native.inventory?.ticketId);
  const compareValid =
    view?.type !== "compare" ||
    (view.epoch === data?.epoch &&
      view.contextKey === definitionsKey &&
      view.subjects.every(
        (item) =>
          item.subject.kind !== "native" ||
          item.subject.ticketId === native.inventory?.ticketId,
      ));
  const locationContext = `${definitionsKey}:${native.inventory?.ticketId}:${data?.epoch}:${active}`;
  const latestLocationContext = useRef(locationContext);
  latestLocationContext.current = locationContext;
  useEffect(() => {
    if (
      !active ||
      !sourceLocateError ||
      sourceLocateError.reason !== "sourceUnconfirmed" ||
      !sourceLocateError.sourceIdentity ||
      !native.inventory ||
      native.inventory.ticketId === sourceLocateError.ticketId
    )
      return;
    const refreshedSources = native.inventory.sources.filter(
      (source) =>
        inventorySourceIdentity(source) === sourceLocateError.sourceIdentity,
    );
    if (
      refreshedSources.length === 1 &&
      ["ready", "missing"].includes(refreshedSources[0].state)
    )
      setSourceLocateError(undefined);
  }, [active, native.inventory, sourceLocateError]);
  useLayoutEffect(() => {
    if (!locateIntent) return;
    if (
      !active ||
      !mounted.current ||
      locateIntent.request !== locateRequest.current ||
      locateIntent.context !== latestLocationContext.current
    ) {
      // A paused or superseded intent must never resume on a later matching render.
      if (locateIntent.request === locateRequest.current)
        locateRequest.current++;
      return;
    }
    if (locateIntent.request === focusedLocateRequest.current) return;
    // All filter resets and variant expansion have committed before consuming focus.
    const element = document.getElementById(
      locateIntent.target === "library"
        ? `library-owned-${kind}`
        : `library-resource-${locateIntent.id}`,
    );
    if (!element) return;
    focusedLocateRequest.current = locateIntent.request;
    element.scrollIntoView?.({
      block: locateIntent.target === "library" ? "start" : "center",
      behavior: "smooth",
    });
    element.focus({ preventScroll: true });
  }, [locateIntent, locationContext, active, kind, sourceFilter]);
  async function locateNativeResource(
    entry: import("@/lib/api/library").LibraryInventory["entries"][number],
    id: string,
  ) {
    const source = native.inventory?.sources.find(
      (source) =>
        source.sourceRef &&
        entry.sources.some(
          (reference) =>
            reference.sourceId === source.sourceId &&
            reference.toolId === source.toolId,
        ),
    );
    if (!source?.sourceRef || !native.inventory || data?.epoch === undefined) {
      setSourceLocateError({
        reason: "unsupported",
        ticketId: native.inventory?.ticketId,
      });
      return;
    }
    const context = locationContext;
    const request = ++locateRequest.current;
    try {
      const detail = await libraryApi.inventorySource(
        native.inventory.ticketId,
        entry.entryId,
        source.sourceRef,
        data.epoch,
      );
      if (
        context !== latestLocationContext.current ||
        request !== locateRequest.current ||
        !mounted.current
      )
        return;
      if (!detail.matchedResourceIds.includes(id))
        throw new Error("library.inventoryChanged");
      if (!locateResource(id)) throw new Error("library.inventoryChanged");
      setSourceLocateError(undefined);
    } catch {
      if (
        context === latestLocationContext.current &&
        request === locateRequest.current &&
        mounted.current
      ) {
        const identity = inventorySourceIdentity(source);
        const uniquelyIdentified =
          identity &&
          native.inventory!.sources.filter(
            (item) => inventorySourceIdentity(item) === identity,
          ).length === 1;
        setSourceLocateError({
          reason: "sourceUnconfirmed",
          ticketId: native.inventory!.ticketId,
          sourceIdentity: uniquelyIdentified ? identity : undefined,
        });
        native.rescan();
      }
    }
  }
  function locateResource(id: string) {
    if (!active || !mounted.current) return false;
    const row = rows.find((row) => row.id === id);
    if (!row) return false;
    const request = ++locateRequest.current;
    setView(null);
    setSearch("");
    setAgentFilter("all");
    setStatusFilter("all");
    setSourceFilter("all");
    setLocated(id);
    setExpandedGroups((previous) =>
      new Set(previous).add(row.name.toLocaleLowerCase()),
    );
    setLocateIntent({ id, request, context: locationContext });
    return true;
  }
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
        "serviceHost" in row ? row.serviceHost : "",
        "displaySummary" in row ? row.displaySummary : "",
        "description" in row ? row.description : "",
        "repository" in row ? row.repository : "",
        ...targets(row)
          .filter((target) => target.desired || target.readable)
          .map((target) => toolLabel(target.toolId, t)),
        ...(native.inventory?.entries ?? [])
          .filter((entry) => entry.matchedResourceIds.includes(row.id))
          .map((entry) =>
            [
              entry.summary,
              ...entry.sources.map((source) => toolLabel(source.toolId, t)),
            ].join(" "),
          ),
        ...recipeMatches
          .filter((match) => match.exactResourceIds.includes(row.id))
          .map((match) => {
            const recipe = LIBRARY_RECOMMENDATIONS.find(
              (recipe) => recipe.id === match.recipeId,
            );
            return recipe ? t(recipe.descriptionKey) : "";
          }),
      ]
        .join(" ")
        .toLocaleLowerCase();
      return (
        sourceFilter !== "native" &&
        text.includes(search.trim().toLocaleLowerCase()) &&
        (agentFilter === "all" ||
          observed(row.id).includes(agentFilter) ||
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
  const rowGroups = new Map<string, Resource[]>();
  for (const row of filtered) {
    const key = row.name.toLocaleLowerCase();
    rowGroups.set(key, [...(rowGroups.get(key) ?? []), row]);
  }
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
  const formTargets =
    data?.targets ??
    (rows.length && "targets" in rows[0] ? rows[0].targets : []);
  const identifier =
    newMcpId ||
    createMcpIdentifier(
      mcpDraft.name,
      rows.map((row) => row.id),
    );
  const resource = view && "resource" in view ? view.resource : undefined;
  async function nextAfterSave(resourceId: string, intent: "draft" | "apply") {
    inline.resetEditor();
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
    if (kind === "mcp" && formTools.length)
      setView({
        type: "batch",
        resources: [{ resourceId: row.id, expectedRevision: row.revision! }],
        tools: formTools,
        action: "apply",
        epoch: next.epoch,
      });
    else
      setView({
        type: "targets",
        resource: row,
        targets: nextTargets,
        epoch: next.epoch,
      });
  }
  function chooseRecommendation(recipe: LibraryRecommendation) {
    inline.resetEditor();
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
  async function collectNative(
    entry: LibraryInventory["entries"][number],
    element: HTMLElement,
  ) {
    const inventory = native.inventory;
    if (!inventory || native.expired || data?.epoch === undefined) {
      setError(t("library.interaction.scanNeeded"));
      native.rescan();
      return;
    }
    if (entry.conflict) {
      const peers = inventory.entries.filter(
        (item) =>
          item.name.toLocaleLowerCase() === entry.name.toLocaleLowerCase(),
      );
      const existing = rows.filter(
        (row) =>
          row.name.toLocaleLowerCase() === entry.name.toLocaleLowerCase(),
      );
      setError(t("library.interaction.importConflict"));
      if (kind === "mcp" && peers.length + existing.length > 1)
        open(
          {
            type: "compare",
            subjects: [
              ...peers.map((item) => ({
                name: item.name,
                subject: {
                  kind: "native" as const,
                  ticketId: inventory.ticketId,
                  entryId: item.entryId,
                },
              })),
              ...existing.map((row) => ({
                name: row.name,
                subject: {
                  kind: "resource" as const,
                  resourceId: row.id,
                  expectedRevision: row.revision!,
                },
              })),
            ],
          },
          { currentTarget: element },
        );
      return;
    }
    const epoch = data.epoch;
    await run(async () => {
      const result = await libraryApi.importInventory(
        inventory.ticketId,
        [entry.entryId],
        epoch,
      );
      const item = result.items.find(
        (item) =>
          item.entryId === entry.entryId &&
          ["imported", "existing"].includes(item.status),
      );
      if (!item?.resourceId) throw new Error("library.inventoryChanged");
      if (mounted.current)
        setCollected({ id: item.resourceId, epoch, routing: routingKey });
    });
  }
  function nativeSources(resource?: { id: string; name: string }) {
    return (
      <LibraryNativeSources
        inventory={native.inventory}
        loading={native.loading}
        failed={!!native.error || native.expired}
        busy={busy}
        onRescan={native.rescan}
        onImport={(entry, element) =>
          inline.guard(() => void collectNative(entry, element))
        }
        onSource={(entry, sourceRef, element) =>
          open(
            {
              type: "source",
              ticketId: native.inventory!.ticketId,
              entryId: entry.entryId,
              sourceRef,
            },
            { currentTarget: element },
          )
        }
        onLocate={(entry, id) => void locateNativeResource(entry, id)}
        onCompare={(entries, element) =>
          open(
            {
              type: "compare",
              subjects: entries.map((entry) => ({
                name: `${entry.name} · ${entry.variant ?? entry.sources.map((source) => toolLabel(source.toolId, t)).join(", ")}`,
                subject: {
                  kind: "native",
                  ticketId: native.inventory!.ticketId,
                  entryId: entry.entryId,
                },
              })),
            },
            { currentTarget: element },
          )
        }
        resource={resource}
        search={resource ? "" : search}
        tool={resource ? "all" : agentFilter}
        status={resource ? "all" : statusFilter}
        onShowLibrary={() =>
          inline.guard(() => {
            const context = latestLocationContext.current;
            const request = ++locateRequest.current;
            setSourceFilter("library");
            setLocateIntent({
              id: "",
              target: "library",
              request,
              context,
            });
          })
        }
      />
    );
  }
  const discoverContent = (
    <LibraryDiscover
      kind={kind}
      searchInputRef={discoverSearch}
      busy={busy}
      matches={Object.fromEntries(
        recipeMatches
          .filter(
            (match) =>
              match.exactResourceIds.length ||
              match.nativeEntryIds.length ||
              match.similarNativeEntryIds.length ||
              nativeMatchVerified,
          )
          .map((match) => [
            match.recipeId,
            match.exactResourceIds.length
              ? {
                  state: "exactLibrary",
                  resourceId: match.exactResourceIds[0],
                }
              : match.nativeEntryIds.length ||
                  match.similarNativeEntryIds.length
                ? {
                    state: "nativeOther",
                    entryId: [
                      ...match.nativeEntryIds,
                      ...match.similarNativeEntryIds,
                    ][0],
                  }
                : { state: "none" },
          ]),
      )}
      onLocateExisting={(match) =>
        inline.guard(() => {
          if (match.state === "exactLibrary") locateResource(match.resourceId);
          else {
            const entry = native.inventory?.entries.find(
              (entry) => entry.entryId === match.entryId,
            );
            const source =
              entry &&
              native.inventory?.sources.find(
                (source) =>
                  source.sourceRef &&
                  entry.sources.some(
                    (reference) => reference.sourceId === source.sourceId,
                  ),
              );
            if (entry && source?.sourceRef)
              setView({
                type: "source",
                ticketId: native.inventory!.ticketId,
                entryId: entry.entryId,
                sourceRef: source.sourceRef,
                epoch: data!.epoch,
                contextKey: definitionsKey,
              });
            else {
              setView(null);
              setSourceFilter("native");
              setSearch(entry?.name ?? "");
            }
          }
        })
      }
      onChoose={(recipe, element) =>
        inline.guard(() => {
          trigger.current = element;
          chooseRecommendation(recipe);
        })
      }
      onOpenSource={(url) =>
        void settingsApi
          .openExternal(url)
          .catch(() => setError(t("library.failed")))
      }
    />
  );
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={search}
          onChange={(event) => {
            locateRequest.current++;
            setLocated(undefined);
            setSearch(event.target.value);
          }}
          aria-label={t("library.interaction.searchExisting")}
          placeholder={t("library.interaction.searchExisting")}
          className="h-8 min-w-[180px] max-w-md flex-1"
        />
        <MenuSelect
          aria-label={t("library.workbench.agentFilter")}
          value={agentFilter}
          onChange={(value) => {
            locateRequest.current++;
            setAgentFilter(value);
          }}
          options={[
            { value: "all", label: t("library.workbench.allAgents") },
            ...["claude", "codex", "gemini", "opencode", "hermes"].map(
              (value) => ({ value, label: toolLabel(value, t) }),
            ),
          ]}
        />
        <Popover>
          <PopoverTrigger asChild>
            <Button variant="secondary">
              <SlidersHorizontal aria-hidden className="mr-1 h-3.5 w-3.5" />
              {t("library.interaction.filter")}
              {sourceFilter !== "all" || statusFilter !== "all"
                ? ` · ${Number(sourceFilter !== "all") + Number(statusFilter !== "all")}`
                : ""}
            </Button>
          </PopoverTrigger>
          <PopoverContent className="flex flex-col gap-3 p-3" align="start">
            <MenuSelect
              aria-label={t("library.workbench2.sourceFilter")}
              value={sourceFilter}
              onChange={(value) => {
                locateRequest.current++;
                setSourceFilter(value);
              }}
              options={[
                { value: "all", label: t("library.workbench2.allSources") },
                {
                  value: "library",
                  label: t("library.workbench2.libraryOnly"),
                },
                { value: "native", label: t("library.workbench2.nativeOnly") },
              ]}
            />
            <MenuSelect
              aria-label={t("library.workbench.statusFilter")}
              value={statusFilter}
              onChange={(value) => {
                locateRequest.current++;
                setStatusFilter(value);
              }}
              options={["all", "applied", "draft", "attention"].map(
                (value) => ({
                  value,
                  label: t(`library.workbench.filter.${value}`),
                }),
              )}
            />
          </PopoverContent>
        </Popover>
        <Button
          ref={addButton}
          variant="secondary"
          disabled={busy || !writable}
          onClick={(event) => open({ type: "new" }, event)}
        >
          {t(
            kind === "mcp"
              ? "library.workbench2.addMcp"
              : kind === "skills"
                ? "library.addSkill"
                : "library.add",
          )}
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              ref={toolbarMenu}
              variant="ghost"
              size="icon"
              aria-label={t("library.workbench2.more")}
            >
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            onCloseAutoFocus={(event) => {
              event.preventDefault();
              const action = toolbarAction.current;
              toolbarAction.current = undefined;
              if (action && toolbarMenu.current) {
                const element = toolbarMenu.current;
                queueMicrotask(() =>
                  open({ type: action }, { currentTarget: element }),
                );
              } else toolbarMenu.current?.focus();
            }}
          >
            <DropdownMenuItem
              disabled={busy || !writable}
              onSelect={() => {
                toolbarAction.current = "inventory";
              }}
            >
              {t("library.workbench.importNative")}
            </DropdownMenuItem>
            {kind === "instructions" && (
              <DropdownMenuItem
                disabled={busy || !writable}
                onSelect={() => {
                  toolbarAction.current = "import";
                }}
              >
                {t("library.importMarkdown")}
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {(agentFilter !== "all" ||
        sourceFilter !== "all" ||
        statusFilter !== "all") && (
        <div
          role="region"
          aria-label={t("library.interaction.filtersCount", {
            count:
              Number(agentFilter !== "all") +
              Number(sourceFilter !== "all") +
              Number(statusFilter !== "all"),
          })}
          className="flex flex-wrap gap-2"
        >
          {[
            {
              value: agentFilter,
              label: toolLabel(agentFilter, t),
              clear: () => setAgentFilter("all"),
            },
            {
              value: sourceFilter,
              label: t(
                sourceFilter === "native"
                  ? "library.workbench2.nativeOnly"
                  : "library.workbench2.libraryOnly",
              ),
              clear: () => setSourceFilter("all"),
            },
            {
              value: statusFilter,
              label: t(`library.workbench.filter.${statusFilter}`),
              clear: () => setStatusFilter("all"),
            },
          ]
            .filter((item) => item.value !== "all")
            .map((item, index) => (
              <Button
                key={index}
                size="sm"
                variant="secondary"
                aria-label={t("library.interaction.removeFilter", {
                  name: item.label,
                })}
                onClick={item.clear}
              >
                {item.label}
                <X aria-hidden className="ml-1 h-3 w-3" />
              </Button>
            ))}
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setAgentFilter("all");
              setSourceFilter("all");
              setStatusFilter("all");
            }}
          >
            {t("library.interaction.clearFilters")}
          </Button>
        </div>
      )}
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
      {!view && sourceLocateError && (
        <p role="alert" className="text-ctl text-warn">
          {t(`library.workbench2.${sourceLocateError.reason}`)}
        </p>
      )}
      {notice && (
        <p role="status" className="text-ctl text-ink-muted">
          {notice}
        </p>
      )}
      {data && sourceFilter !== "native" && (
        <ListCard
          id={`library-owned-${kind}`}
          tabIndex={-1}
          className="outline-none"
        >
          <ListHead>
            <span>
              {t("library.workbench.inLibrary")} · {filtered.length}
            </span>
            <span className="flex-1" />
            <MenuSelect
              aria-label={t("library.workbench.sort")}
              value={sort}
              onChange={(value) => {
                locateRequest.current++;
                setSort(value);
              }}
              options={[
                { value: "name", label: "A → Z" },
                { value: "reverse", label: "Z → A" },
                ...(kind === "skills"
                  ? [
                      {
                        value: "source",
                        label: t("library.workbench.bySource"),
                      },
                    ]
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
              {kind === "mcp" &&
                rowGroups.get(row.name.toLocaleLowerCase())!.length > 1 &&
                rowGroups.get(row.name.toLocaleLowerCase())![0].id ===
                  row.id && (
                  <ListRow className="justify-between">
                    <Button
                      variant="ghost"
                      aria-expanded={
                        expandedGroups.has(row.name.toLocaleLowerCase()) ||
                        !!search
                      }
                      onClick={() =>
                        setExpandedGroups((previous) => {
                          const next = new Set(previous);
                          const key = row.name.toLocaleLowerCase();
                          if (next.has(key)) next.delete(key);
                          else next.add(key);
                          return next;
                        })
                      }
                    >
                      {t("library.workbench2.variants", {
                        name: row.name,
                        count: rowGroups.get(row.name.toLocaleLowerCase())!
                          .length,
                      })}
                    </Button>
                    <Button
                      variant="ghost"
                      onClick={(event) =>
                        open(
                          {
                            type: "compare",
                            subjects: rowGroups
                              .get(row.name.toLocaleLowerCase())!
                              .map((item) => ({
                                name: `${item.name} · ${item.id}`,
                                subject: {
                                  kind: "resource",
                                  resourceId: item.id,
                                  expectedRevision: item.revision!,
                                },
                              })),
                          },
                          event,
                        )
                      }
                    >
                      {t("library.workbench2.compare")}
                    </Button>
                  </ListRow>
                )}
              {(kind !== "mcp" ||
                !!search ||
                expandedGroups.has(row.name.toLocaleLowerCase()) ||
                rowGroups.get(row.name.toLocaleLowerCase())!.length === 1) && (
                <Fragment>
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
                    expanded={inline.opened?.row.id === row.id}
                    onExpand={(element) => inline.toggle(row, element)}
                    located={located === row.id}
                    observedTools={observed(row.id)}
                    disabledTools={switchedOff(row.id)}
                    recipeId={
                      recipeMatches.find((match) =>
                        match.exactResourceIds.includes(row.id),
                      )?.recipeId
                    }
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
                    check={
                      checks[
                        `${data?.epoch}:${routingKey}:${row.id}:${row.revision}`
                      ]
                    }
                    onToggle={(target) => toggleTarget(row, target)}
                    onAction={(type, element) => {
                      if (type === "export") {
                        void run(() =>
                          libraryApi.exportSelected(kind, [row.id]),
                        );
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
                  {inline.opened?.row.id === row.id && (
                    <LibraryInlineDetails
                      key={`${inline.opened.epoch}:${inline.opened.routingKey}:${inline.opened.row.id}:${inline.opened.row.revision}`}
                      kind={kind}
                      routingKey={routingKey}
                      onCollapse={() => {
                        const button = document
                          .getElementById(`library-resource-${row.id}`)
                          ?.querySelector<HTMLButtonElement>(
                            "button[aria-expanded]",
                          );
                        if (button) inline.toggle(row, button);
                      }}
                      sources={nativeSources(row)}
                      row={inline.opened.row}
                      epoch={inline.opened.epoch}
                      store={inline.opened.store}
                      stale={inline.stale}
                      busy={busy || !writable}
                      targets={targets(row)}
                      onEditor={view ? undefined : inline.registerEditor}
                      onBusy={setBusy}
                      onSave={(set) =>
                        run(() =>
                          libraryApi.saveInstruction(
                            set,
                            set.revision || undefined,
                            inline.opened!.epoch,
                          ),
                        )
                      }
                      onCheck={(result, checkedAtMs) =>
                        setChecks((previous) => ({
                          ...previous,
                          [`${data?.epoch}:${routingKey}:${row.id}:${row.revision}`]:
                            {
                              state: result.state,
                              toolsCount: result.tools.length,
                              checkedAtMs,
                              result,
                            },
                        }))
                      }
                      onEdit={(element) =>
                        open(
                          {
                            type: "edit",
                            resource: row,
                            targets: targets(row),
                          },
                          { currentTarget: element },
                        )
                      }
                      onNextAction={(action, element) => {
                        if (action.kind === "edit")
                          open(
                            {
                              type: "edit",
                              resource: row,
                              targets: targets(row),
                              focusPath: action.focusPath,
                            },
                            { currentTarget: element },
                          );
                      }}
                    />
                  )}
                </Fragment>
              )}
            </Fragment>
          ))}
        </ListCard>
      )}
      {data && sourceFilter !== "library" && nativeSources()}
      {kind === "instructions" && data && (
        <InstructionOverview
          data={data}
          busy={busy || !writable || viewStale}
          onManage={(set, event) =>
            open(
              { type: "targets", resource: set, targets: targets(set) },
              event,
            )
          }
        />
      )}
      <section
        ref={discoverRef}
        style={{ scrollMarginTop: "var(--library-scroll-offset, 96px)" }}
        role="region"
        aria-label={t(
          kind === "mcp"
            ? "library.interaction.discoverMcp"
            : kind === "skills"
              ? "library.interaction.discoverSkills"
              : "library.interaction.templates",
        )}
        className="scroll-mt-20 space-y-3 pt-5"
      >
        <h3 className="px-1 text-ctl font-medium text-ink-muted">
          {t(
            kind === "mcp"
              ? "library.interaction.discoverMcp"
              : kind === "skills"
                ? "library.interaction.discoverSkills"
                : "library.interaction.templates",
          )}
        </h3>
        {discoverContent}
      </section>
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
      <div
        ref={historyRef}
        className={!history && registerHistory ? "hidden" : undefined}
      >
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
      </div>
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
      <LibraryUnsavedDialog state={inline} />
      <Dialog
        open={!!view}
        onOpenChange={(open) => {
          if (!open) close();
        }}
      >
        <DialogContent
          data-library-editor-dialog
          className="sm:max-w-3xl"
          onEscapeKeyDown={(event) => {
            if (busy) event.preventDefault();
          }}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            if (
              locateIntent?.request === locateRequest.current &&
              locateIntent.context === latestLocationContext.current &&
              active
            )
              return;
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
                  : view?.type === "source"
                    ? t("library.workbench2.sourceDetails")
                    : view?.type === "compare"
                      ? t("library.workbench2.compare")
                      : kind === "mcp" &&
                          (view?.type === "new" || view?.type === "advancedNew")
                        ? t("library.workbench2.addMcp")
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
            {viewStale && (
              <p role="alert" className="text-ctl text-warn">
                {t("library.workbench2.staleEdit")}
              </p>
            )}
            {error && (
              <p role="alert" className="text-ctl text-danger">
                {error}
              </p>
            )}
            {view?.type === "multiResult" && (
              <LibraryMultiSaveResult
                result={view}
                busy={busy || !writable || view.epoch !== data?.epoch}
                targets={formTargets}
                existingIds={rows.map((row) => row.id)}
                onRetry={(draft, index) =>
                  run(async () => {
                    await libraryApi.saveMcp(
                      draft.identifier!,
                      draft.name,
                      undefined,
                      mcpDraftToEdits(draft),
                      view.epoch,
                    );
                    setView({
                      ...view,
                      saved: [
                        ...view.saved,
                        { id: draft.identifier!, name: draft.name },
                      ],
                      remaining: view.remaining.filter(
                        (_, item) => item !== index,
                      ),
                    });
                  })
                }
                onPreview={() =>
                  void run(async () => {
                    const current = await libraryApi.list("mcp");
                    setView({
                      type: "batch",
                      resources: (current.servers ?? [])
                        .filter((row) =>
                          view.saved.some((item) => item.id === row.id),
                        )
                        .map((row) => ({
                          resourceId: row.id,
                          expectedRevision: row.revision,
                        })),
                      tools: view.tools,
                      action: "apply",
                      epoch: current.epoch,
                    });
                  })
                }
              />
            )}
            {(!sourceValid || !compareValid) && (
              <p role="alert" className="text-ctl text-warn">
                {t("library.workbench2.sourceUnconfirmed")}
              </p>
            )}
            {sourceValid && view?.type === "source" && (
              <LibrarySourceDetail
                key={`${view.ticketId}:${view.entryId}:${view.sourceRef}`}
                ticketId={view.ticketId}
                entryId={view.entryId}
                sourceRef={view.sourceRef}
                epoch={view.epoch}
                onBack={close}
                onLocate={locateResource}
              />
            )}
            {compareValid && view?.type === "compare" && (
              <LibraryMcpCompare
                key={JSON.stringify(view.subjects)}
                subjects={view.subjects}
                epoch={view.epoch}
              />
            )}
            {view?.type === "recommendedSkill" && (
              <div className="space-y-3">
                <p className="text-ui font-medium">{view.recipe.name}</p>
                <p className="text-ctl text-ink-muted">
                  {t("library.discover2.selectedSkillNote")}
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
                          view.epoch,
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
                onChanged={async () => {
                  await query.refetch();
                  native.rescan();
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
                  native.rescan();
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
                  onDiscard={() => {
                    inline.resetEditor();
                    setView(null);
                  }}
                  onController={inline.registerEditor}
                  busy={busy || !writable || viewStale}
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
                onController={inline.registerEditor}
                store={draftStore}
                initial={view.resource as McpDefinition}
                busy={busy || !writable || viewStale}
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
                scopeKey={routingKey}
                server={view.resource as McpDefinition}
                epoch={view.epoch}
                readOnly={!writable || viewStale}
                onNextAction={(action, element) => {
                  if (action.kind !== "edit" || viewStale || !writable) return;
                  const latest = rows.find(
                    (row) => row.id === view.resource.id,
                  );
                  if (
                    !latest ||
                    latest.revision !== view.resource.revision ||
                    data?.epoch !== view.epoch
                  )
                    return;
                  open(
                    {
                      type: "edit",
                      resource: latest,
                      targets: view.targets,
                      focusPath: action.focusPath,
                    },
                    {
                      currentTarget: trigger.current?.isConnected
                        ? trigger.current
                        : element,
                    },
                  );
                }}
                onResult={(result, checkedAtMs) =>
                  setChecks((previous) => ({
                    ...previous,
                    [`${view.epoch}:${routingKey}:${view.resource.id}:${view.resource.revision}`]:
                      {
                        state: result.state,
                        checkedAtMs,
                        toolsCount: result.tools.length,
                        result,
                      },
                  }))
                }
                onBusyChange={(value) => {
                  running.current = value;
                  setBusy(value);
                }}
              />
            )}
            {(view?.type === "new" || view?.type === "edit") &&
              kind === "mcp" && (
                <LibraryMcpForm
                  onController={inline.registerEditor}
                  value={mcpDraft}
                  initialFocusPath={
                    view.type === "edit" ? view.focusPath : undefined
                  }
                  initial={
                    view.type === "edit"
                      ? (view.resource as McpDefinition)
                      : undefined
                  }
                  identifier={
                    view.type === "edit" ? view.resource.id : identifier
                  }
                  onIdentifierChange={setNewMcpId}
                  existingIdentifiers={rows.map((row) => row.id)}
                  targets={view.type === "edit" ? view.targets : formTargets}
                  selectedTools={formTools}
                  onToolsChange={setFormTools}
                  onChange={setMcpDraft}
                  onDiscover={() =>
                    inline.guard(() => {
                      setView(null);
                      inline.resetEditor();
                      requestAnimationFrame(jumpDiscover);
                    })
                  }
                  onSaveEdits={(edits, intent) =>
                    run(async () => {
                      if (view.type !== "edit") return;
                      await libraryApi.saveMcp(
                        view.resource.id,
                        mcpDraft.name,
                        view.resource.revision,
                        edits,
                        view.epoch,
                      );
                      await nextAfterSave(view.resource.id, intent);
                    })
                  }
                  onImportMany={(drafts, intent) =>
                    run(async () => {
                      const saved: { id: string; name: string }[] = [];
                      const remaining: typeof drafts = [];
                      for (const draft of drafts) {
                        const id = draft.identifier!;
                        try {
                          await libraryApi.saveMcp(
                            id,
                            draft.name,
                            undefined,
                            mcpDraftToEdits(draft),
                            view.epoch,
                          );
                          saved.push({ id, name: draft.name });
                        } catch {
                          remaining.push(draft);
                        }
                      }
                      setView({
                        type: "multiResult",
                        saved,
                        remaining,
                        intent,
                        tools: formTools,
                        epoch: view.epoch,
                      });
                    })
                  }
                  busy={
                    busy ||
                    !writable ||
                    view.epoch !== data?.epoch ||
                    (view.type === "edit" &&
                      !rows.some(
                        (row) =>
                          row.id === view.resource.id &&
                          row.revision === view.resource.revision,
                      ))
                  }
                  onSave={(draft, intent) =>
                    run(async () => {
                      setNewMcpId(identifier);
                      await libraryApi.saveMcp(
                        identifier,
                        draft.name,
                        undefined,
                        mcpDraftToEdits(draft),
                        view.epoch,
                      );
                      await nextAfterSave(
                        draft.identifier ?? identifier,
                        intent,
                      );
                    })
                  }
                  onAdvanced={
                    view.type === "edit"
                      ? undefined
                      : (draft, dirty) => {
                          draftStore.values.set("mcpInheritedDirty", dirty);
                          let edits: import("@/lib/api/library").FieldEdit[];
                          try {
                            edits = mcpDraftToEdits(draft);
                          } catch {
                            edits = [
                              {
                                path: ["type"],
                                action: "set",
                                value: draft.transport,
                              },
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
                          draftStore.values.set("mcpId", identifier);
                          setNewMcpId(identifier);
                          draftStore.values.set("mcpName", draft.name);
                          draftStore.values.set(
                            "mcpFields",
                            edits.map((edit, index) => ({
                              ...edit,
                              input: JSON.stringify(edit.value),
                              key: String(index),
                            })),
                          );
                          inline.resetEditor();
                          setView({ ...view, type: "advancedNew" });
                        }
                  }
                />
              )}
            {view?.type === "advancedNew" && kind === "mcp" && (
              <McpEditor
                onController={inline.registerEditor}
                store={draftStore}
                initial={resource as McpDefinition | undefined}
                busy={busy || !writable || viewStale}
                onSave={(id, name, revision, edits) =>
                  run(
                    () =>
                      libraryApi.saveMcp(id, name, revision, edits, view.epoch),
                    true,
                  )
                }
              />
            )}
            {view?.type === "new" && kind === "skills" && (
              <SkillImporter
                onController={inline.registerEditor}
                store={draftStore}
                busy={busy || !writable || viewStale}
                onImport={(action) => run(action, true)}
              />
            )}
            {view?.type === "update" && kind === "skills" && (
              <SkillUpdate
                store={draftStore}
                skill={view.resource as SkillDefinition}
                epoch={view.epoch}
                readOnly={!writable || viewStale}
                externalBusy={busy}
                onBusyChange={(value) => {
                  running.current = value;
                  setBusy(value);
                }}
                onChanged={async () => {
                  await query.refetch();
                  native.rescan();
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
                readOnly={!writable || viewStale}
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
                  native.rescan();
                  onChanged();
                }}
              />
            )}
            {view?.type === "operation" && (
              <ResourceReview
                store={draftStore}
                externalBusy={busy}
                readOnly={!writable || viewStale}
                expectedEpoch={view.epoch}
                kind={kind}
                operation={view.operation}
                onBusyChange={(value) => {
                  running.current = value;
                  setBusy(value);
                }}
                onChanged={async () => {
                  await query.refetch();
                  native.rescan();
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
