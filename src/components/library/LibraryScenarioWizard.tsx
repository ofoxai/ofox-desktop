import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Layers } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  libraryApi,
  type LibraryData,
  type LibraryKind,
  type BatchResource,
  type LibraryBatchPlan,
} from "@/lib/api/library";
import { skillsApi } from "@/lib/api/skills";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { LibraryMcpForm, mcpDraftToEdits } from "./LibraryMcpForm";
import { definitionFromEdits } from "./libraryMcpInput";
import { LibraryBatchReview } from "./LibraryBatchReview";
import {
  cloneRecommendationDraft,
  LIBRARY_RECOMMENDATIONS,
  type LibraryMcpDraft,
  type LibraryRecommendation,
} from "./libraryRecommendations";
import {
  LIBRARY_SCENARIOS,
  SCENARIO_KIND_ORDER,
  scenarioRecipes,
  type LibraryScenario,
} from "./libraryScenarios";
import { libraryError, toolLabel } from "./libraryText";

type Prepared = BatchResource & {
  recipeId: string;
  kind: LibraryKind;
  name: string;
};
type Stage = "select" | "prepare" | "tools" | "summary" | "apply" | "done";
function definitions(data: LibraryData, kind: LibraryKind) {
  return kind === "instructions"
    ? (data.instructions ?? [])
    : kind === "mcp"
      ? (data.servers ?? [])
      : (data.skills ?? []);
}
function RecipePreparation({
  recipe,
  epoch,
  onSaved,
  onBusy,
}: {
  recipe: LibraryRecommendation;
  epoch: number | null;
  onSaved: (resource: Prepared, epoch: number) => void;
  onBusy: (busy: boolean) => void;
}) {
  const { t, i18n } = useTranslation();
  const language = i18n.resolvedLanguage?.startsWith("ja")
    ? "ja"
    : i18n.resolvedLanguage?.startsWith("zh")
      ? "zh"
      : "en";
  const [draft, setDraft] = useState<LibraryMcpDraft>(() =>
    recipe.kind === "mcp"
      ? { ...cloneRecommendationDraft(recipe), identifier: recipe.id }
      : {
          name: recipe.name,
          transport: "stdio",
          command: "",
          argsText: "",
          url: "",
          environment: [],
          headers: [],
        },
  );
  const [data, setData] = useState<LibraryData>();
  const [match, setMatch] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);
  const running = useRef(false);
  const creationId = useRef(crypto.randomUUID());
  const savedReceipt = useRef<{ id: string; revision: number }>();
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    let cancelled = false;
    void (async () => {
      try {
        const current = await libraryApi.list(recipe.kind);
        if (epoch !== null && current.epoch !== epoch)
          throw new Error("library.epochChanged");
        const matches =
          recipe.kind === "mcp"
            ? await libraryApi.matchMcpRecipes(
                [
                  {
                    recipeId: recipe.id,
                    definition: definitionFromEdits(mcpDraftToEdits(draft)),
                  },
                ],
                undefined,
                current.epoch,
              )
            : await libraryApi.matchBuiltinRecipes(
                recipe.kind,
                [
                  {
                    recipeId: recipe.id,
                    ...(recipe.kind === "instructions"
                      ? { content: recipe.contentByLanguage[language] }
                      : {
                          repoOwner: recipe.repository.owner,
                          repoName: recipe.repository.name,
                          directory: recipe.repository.directory,
                        }),
                  },
                ],
                undefined,
                current.epoch,
              );
        if (!cancelled) {
          setData(current);
          setMatch(
            matches.find((item) => item.recipeId === recipe.id)
              ?.exactResourceIds ?? [],
          );
          setReady(true);
        }
      } catch (failure) {
        if (!cancelled) setError(libraryError(failure, t));
      }
    })();
    return () => {
      cancelled = true;
      alive.current = false;
    };
    // A prepared item is a frozen recipe. Editing a form never reuses its initial match.
  }, []);
  async function collect(reuseId?: string, submitted?: LibraryMcpDraft) {
    if (running.current || !data || !ready) return;
    running.current = true;
    setBusy(true);
    onBusy(true);
    setError("");
    try {
      const current = await libraryApi.list(recipe.kind);
      if (
        current.epoch !== data.epoch ||
        (epoch !== null && current.epoch !== epoch)
      )
        throw new Error("library.epochChanged");
      let id: string;
      if (savedReceipt.current) {
        // A successful save may be followed by a failed refresh. Verify that
        // receipt on retry; never repeat the mutation or generate another ID.
        id = savedReceipt.current.id;
      } else if (reuseId) {
        const before = definitions(data, recipe.kind).find(
          (item) => item.id === reuseId,
        );
        const now = definitions(current, recipe.kind).find(
          (item) => item.id === reuseId,
        );
        if (!before || !now || before.revision !== now.revision)
          throw new Error("library.revisionChanged");
        id = reuseId;
      } else if (recipe.kind === "instructions") {
        const saved = await libraryApi.saveInstruction(
          {
            id: creationId.current,
            name: t(recipe.titleKey ?? recipe.name),
            content: recipe.contentByLanguage[language],
            revision: 0,
          },
          undefined,
          current.epoch,
        );
        id = saved.id;
        savedReceipt.current = { id, revision: saved.revision };
      } else if (recipe.kind === "mcp") {
        const value = submitted ?? draft;
        id = value.identifier ?? recipe.id;
        await libraryApi.saveMcp(
          id,
          value.name,
          undefined,
          mcpDraftToEdits(value),
          current.epoch,
        );
        savedReceipt.current = { id, revision: 1 };
      } else {
        const saved = await skillsApi.installUnified(
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
          current.epoch,
        );
        id = saved.id;
        savedReceipt.current = { id, revision: 1 };
      }
      const after = await libraryApi.list(recipe.kind);
      if (after.epoch !== current.epoch)
        throw new Error("library.epochChanged");
      const saved = definitions(after, recipe.kind).find(
        (item) => item.id === id,
      );
      if (
        !saved?.revision ||
        (savedReceipt.current &&
          saved.revision !== savedReceipt.current.revision)
      )
        throw new Error("library.revisionChanged");
      if (recipe.kind === "skills" && !reuseId) {
        const verified = await libraryApi.matchBuiltinRecipes(
          "skills",
          [
            {
              recipeId: recipe.id,
              repoOwner: recipe.repository.owner,
              repoName: recipe.repository.name,
              directory: recipe.repository.directory,
            },
          ],
          undefined,
          after.epoch,
        );
        if (
          !verified.some(
            (match) =>
              match.recipeId === recipe.id &&
              match.exactResourceIds.includes(id),
          )
        )
          throw new Error("library.skillNameConflict");
      }
      if (alive.current)
        onSaved(
          {
            recipeId: recipe.id,
            kind: recipe.kind,
            name: saved.name,
            resourceId: id,
            expectedRevision: saved.revision,
          },
          after.epoch,
        );
    } catch (failure) {
      if (alive.current) setError(libraryError(failure, t));
    } finally {
      running.current = false;
      onBusy(false);
      if (alive.current) setBusy(false);
    }
  }
  return (
    <div className="space-y-3">
      <h3 className="text-ui font-semibold">
        {t(recipe.titleKey ?? recipe.name)}
      </h3>
      <p className="text-ctl text-ink-muted">{t(recipe.descriptionKey)}</p>
      {recipe.dependencyKeys.map((key) => (
        <p key={key} className="text-ctl text-ink-muted">
          {t(key)}
        </p>
      ))}
      {recipe.permissionKeys.map((key) => (
        <p key={key} className="text-note text-ink-muted">
          {t(key)}
        </p>
      ))}
      {error && (
        <p role="alert" className="text-ctl text-danger">
          {error}
        </p>
      )}
      {!ready && !error && <p role="status">{t("library.loading")}</p>}
      {ready && (
        <>
          {savedReceipt.current ? (
            <>
              <p role="status" className="text-ctl text-ink-muted">
                {t("library.scenarios.savedPending")}
              </p>
              <Button disabled={busy} onClick={() => void collect()}>
                {t("library.scenarios.verifySaved")}
              </Button>
            </>
          ) : (
            <>
              {match.map((id) => (
                <Button
                  key={id}
                  disabled={busy}
                  variant="secondary"
                  onClick={() => void collect(id)}
                >
                  {t("library.scenarios.reuse", {
                    name:
                      definitions(data!, recipe.kind).find(
                        (item) => item.id === id,
                      )?.name ?? recipe.name,
                  })}
                </Button>
              ))}
              <p className="text-note text-ink-muted">
                {t("library.scenarios.matchNote")}
              </p>
              {recipe.kind === "mcp" ? (
                <LibraryMcpForm
                  value={draft}
                  onChange={setDraft}
                  busy={busy}
                  saveOnly
                  existingIdentifiers={data?.servers?.map((item) => item.id)}
                  onSave={(value) => collect(undefined, value)}
                />
              ) : (
                <>
                  {recipe.kind === "instructions" && (
                    <pre className="max-h-44 overflow-auto whitespace-pre-wrap rounded-ctl bg-pill p-3 text-ctl">
                      {recipe.contentByLanguage[language]}
                    </pre>
                  )}
                  <Button disabled={busy} onClick={() => void collect()}>
                    {t(
                      recipe.kind === "skills"
                        ? "library.scenarios.download"
                        : "library.importDraft",
                    )}
                  </Button>
                </>
              )}
            </>
          )}
        </>
      )}
    </div>
  );
}

export function LibraryScenarioWizard({
  onChanged,
  request,
  returnFocusRef,
}: {
  onChanged: () => Promise<unknown>;
  request?: number;
  returnFocusRef?: import("react").RefObject<HTMLButtonElement>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (request) {
      reset();
      setOpen(true);
    }
  }, [request]);
  const [scenario, setScenario] = useState<LibraryScenario>();
  const [selected, setSelected] = useState<string[]>([]);
  const [stage, setStage] = useState<Stage>("select");
  const [prepared, setPrepared] = useState<Prepared[]>([]);
  const [epoch, setEpoch] = useState<number | null>(null);
  const [tools, setTools] = useState<string[]>([]);
  const [available, setAvailable] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [kindIndex, setKindIndex] = useState(0);
  const [review, setReview] = useState<{
    kind: LibraryKind;
    resources: BatchResource[];
    epoch: number;
  }>();
  const [results, setResults] = useState<LibraryBatchPlan[]>([]);
  const [canContinue, setCanContinue] = useState(false);
  const running = useRef(false);
  const generation = useRef(0);
  const recipes = scenarioRecipes(selected);
  const current = recipes.find(
    (recipe) => !prepared.some((item) => item.recipeId === recipe.id),
  );
  const kinds = SCENARIO_KIND_ORDER.filter((kind) =>
    prepared.some((item) => item.kind === kind),
  );
  function reset() {
    generation.current++;
    setScenario(undefined);
    setSelected([]);
    setStage("select");
    setPrepared([]);
    setTools([]);
    setAvailable([]);
    setEpoch(null);
    setError("");
    setKindIndex(0);
    setReview(undefined);
    setResults([]);
    setCanContinue(false);
  }
  async function beginReview(index: number) {
    if (running.current || epoch === null) return;
    const expectedGeneration = generation.current;
    running.current = true;
    setBusy(true);
    setError("");
    try {
      const kind = kinds[index];
      if (!kind) {
        setStage("done");
        return;
      }
      const fresh = await libraryApi.list(kind);
      if (fresh.epoch !== epoch) throw new Error("library.epochChanged");
      const resources = prepared.filter((item) => item.kind === kind);
      for (const item of resources) {
        if (
          !definitions(fresh, kind).some(
            (value) =>
              value.id === item.resourceId &&
              value.revision === item.expectedRevision,
          )
        )
          throw new Error("library.revisionChanged");
      }
      if (generation.current === expectedGeneration) {
        setKindIndex(index);
        setReview({ kind, resources, epoch: fresh.epoch });
        setCanContinue(false);
        setStage("apply");
      }
    } catch (failure) {
      if (generation.current === expectedGeneration)
        setError(libraryError(failure, t));
    } finally {
      running.current = false;
      setBusy(false);
    }
  }
  async function chooseTools() {
    if (running.current || epoch === null) return;
    const expectedGeneration = generation.current;
    running.current = true;
    setBusy(true);
    setError("");
    try {
      const snapshots = await Promise.all(
        kinds.map((kind) => libraryApi.list(kind)),
      );
      if (snapshots.some((data) => data.epoch !== epoch))
        throw new Error("library.epochChanged");
      const supported = [
        "claude",
        "codex",
        "gemini",
        "opencode",
        "hermes",
      ].filter((tool) =>
        snapshots.every((data) =>
          data.targets?.some(
            (target) => target.toolId === tool && target.available,
          ),
        ),
      );
      if (generation.current === expectedGeneration) {
        setAvailable(supported);
        setStage("tools");
      }
    } catch (failure) {
      if (generation.current === expectedGeneration)
        setError(libraryError(failure, t));
    } finally {
      running.current = false;
      setBusy(false);
    }
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!busy) {
          if (value) reset();
          setOpen(value);
        }
      }}
    >
      {request === undefined && (
        <DialogTrigger asChild>
          <Button variant="ghost">
            <Layers className="mr-1 h-3.5 w-3.5" />
            {t("library.scenarios.title")}
          </Button>
        </DialogTrigger>
      )}
      <DialogContent
        className="w-[calc(100vw-32px)] max-w-3xl"
        onCloseAutoFocus={
          returnFocusRef
            ? (event) => {
                event.preventDefault();
                returnFocusRef.current?.focus();
              }
            : undefined
        }
        onEscapeKeyDown={(event) => {
          if (busy) event.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle>
            {t("library.scenarios.title")}
            {scenario ? ` · ${t(`library.scenarios.${scenario.id}`)}` : ""}
          </DialogTitle>
          <DialogDescription>
            {t("library.scenarios.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 space-y-4 overflow-y-auto p-4">
          {error && (
            <p role="alert" className="text-ctl text-danger">
              {error}
            </p>
          )}
          {stage === "select" && (
            <>
              <div className="grid gap-2 sm:grid-cols-3">
                {LIBRARY_SCENARIOS.map((item) => (
                  <Button
                    key={item.id}
                    variant={scenario?.id === item.id ? "primary" : "secondary"}
                    onClick={() => {
                      setScenario(item);
                      setSelected(
                        item.resources
                          .filter((resource) => !resource.optional)
                          .map((resource) => resource.recipeId),
                      );
                    }}
                  >
                    {t(`library.scenarios.${item.id}`)}
                  </Button>
                ))}
              </div>
              {scenario?.resources.map((item) => {
                const recipe = LIBRARY_RECOMMENDATIONS.find(
                  (recipe) => recipe.id === item.recipeId,
                )!;
                return (
                  <label
                    key={item.recipeId}
                    className="flex items-start gap-3 rounded-ctl border border-line-2 p-3"
                  >
                    <Checkbox
                      aria-label={t(recipe.titleKey ?? recipe.name)}
                      checked={selected.includes(item.recipeId)}
                      onCheckedChange={(value) =>
                        setSelected((previous) =>
                          value
                            ? [...previous, item.recipeId]
                            : previous.filter((id) => id !== item.recipeId),
                        )
                      }
                    />
                    <span className="min-w-0">
                      <span className="text-ctl font-medium">
                        {t(recipe.titleKey ?? recipe.name)}
                        {item.optional
                          ? ` · ${t("library.scenarios.optional")}`
                          : ""}
                      </span>
                      <span className="mt-1 block text-note text-ink-muted">
                        {t(recipe.descriptionKey)}
                      </span>
                    </span>
                  </label>
                );
              })}
              <Button
                disabled={!selected.length}
                onClick={() => setStage("prepare")}
              >
                {t("library.scenarios.prepare")}
              </Button>
            </>
          )}
          {stage === "prepare" && (
            <>
              <p className="text-note text-ink-muted">
                {t("library.scenarios.prepared", {
                  count: prepared.length,
                  total: recipes.length,
                })}
              </p>
              {current ? (
                <RecipePreparation
                  key={current.id}
                  recipe={current}
                  epoch={epoch}
                  onBusy={setBusy}
                  onSaved={(item, currentEpoch) => {
                    setEpoch(currentEpoch);
                    setPrepared((previous) => [...previous, item]);
                    void onChanged().catch(() => setError(t("library.failed")));
                  }}
                />
              ) : (
                <Button disabled={busy} onClick={() => void chooseTools()}>
                  {t("library.scenarios.chooseTools")}
                </Button>
              )}
            </>
          )}
          {stage === "tools" && (
            <>
              <p className="text-ctl text-ink-muted">
                {t("library.scenarios.toolsHint")}
              </p>
              <div className="flex flex-wrap gap-2">
                {available.map((tool) => (
                  <Button
                    key={tool}
                    variant={tools.includes(tool) ? "primary" : "secondary"}
                    aria-pressed={tools.includes(tool)}
                    onClick={() =>
                      setTools((previous) =>
                        previous.includes(tool)
                          ? previous.filter((value) => value !== tool)
                          : [...previous, tool],
                      )
                    }
                  >
                    <span aria-hidden>
                      <ToolBadge toolId={tool} size={20} variant="plain" />
                    </span>
                    <span className="ml-2">{toolLabel(tool, t)}</span>
                  </Button>
                ))}
              </div>
              {!available.length && (
                <p role="status">{t("library.nativeCliRequired")}</p>
              )}
              <Button
                disabled={!tools.length}
                onClick={() => setStage("summary")}
              >
                {t("library.scenarios.summary")}
              </Button>
            </>
          )}
          {stage === "summary" && (
            <>
              <p className="text-ctl">
                {tools.map((tool) => toolLabel(tool, t)).join(" / ")}
              </p>
              {kinds.map((kind) => (
                <section
                  key={kind}
                  className="rounded-ctl border border-line-2 p-3"
                >
                  <h3 className="text-ctl font-semibold">
                    {t(`library.tabs.${kind}`)}
                  </h3>
                  {prepared
                    .filter((item) => item.kind === kind)
                    .map((item) => (
                      <p
                        key={item.resourceId}
                        className="mt-1 break-words text-ctl text-ink-2"
                      >
                        {item.name}
                      </p>
                    ))}
                </section>
              ))}
              <p className="text-ctl text-ink-muted">
                {t("library.scenarios.confirmSeparately")}
              </p>
              <Button disabled={busy} onClick={() => void beginReview(0)}>
                {t("library.scenarios.startReview")}
              </Button>
            </>
          )}
          {stage === "apply" && review && (
            <>
              <h3 className="text-ui font-semibold">
                {t(`library.tabs.${review.kind}`)}
              </h3>
              <LibraryBatchReview
                key={`${review.kind}:${kindIndex}`}
                {...review}
                tools={tools}
                action="apply"
                onBusyChange={setBusy}
                onChanged={async (plan) => {
                  setResults((previous) => [
                    ...previous.filter((item) => item.kind !== plan.kind),
                    plan,
                  ]);
                  setCanContinue(plan.status === "applied");
                  await onChanged();
                }}
              />
              <Button
                disabled={busy || !canContinue}
                onClick={() => void beginReview(kindIndex + 1)}
              >
                {t(
                  kindIndex + 1 < kinds.length
                    ? "library.scenarios.nextType"
                    : "library.scenarios.showResult",
                )}
              </Button>
            </>
          )}
          {stage === "done" && (
            <p className="text-ui font-medium">{t("library.scenarios.done")}</p>
          )}
          {results.length > 0 && (
            <div className="space-y-2 rounded-ctl bg-pill/50 p-3">
              {results.map((result) => (
                <p key={result.kind} className="text-ctl">
                  {t(`library.tabs.${result.kind}`)} ·{" "}
                  {t(`library.status.${result.status}`, {
                    defaultValue: t("library.unknown"),
                  })}
                </p>
              ))}
              <p className="text-note text-ink-muted">
                {t("library.scenarios.resultHint")}
              </p>
            </div>
          )}
        </div>
        <DialogFooter>
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => setOpen(false)}
          >
            {t(
              stage === "select" || stage === "done"
                ? "common.close"
                : "library.scenarios.stop",
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
