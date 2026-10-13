import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  Loader2,
  Plus,
  Save,
  Pencil,
  Trash2,
  RefreshCw,
  Undo2,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  ModelPicker,
  MultiModelPicker,
} from "@/components/model-picker/ModelPicker";
import { TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import { queryClient } from "@/lib/query/queryClient";
import {
  modelProfilesApi,
  type ModelProfile,
  type ProfileTarget,
  type ProfilePreview,
  type ProfileApplyReport,
} from "@/lib/api/modelProfiles";
import type {
  ModelSelection,
  ToolModelSnapshot,
} from "@/lib/api/modelSelection";
import {
  invalidateModelState,
  MODEL_PROFILES_KEY,
  useModelCatalog,
  useModelSelection,
} from "@/hooks/useModelSelection";

/** Canonical write targets prevent Codex / ChatGPT from applying twice. */
export function captureProfileTargets(
  snapshots: ToolModelSnapshot[],
): ProfileTarget[] {
  const targets = new Map<string, ProfileTarget>();
  for (const snapshot of snapshots) {
    if (snapshot.binding.status !== "configured") continue;
    const selection = snapshot.managedSelection;
    if (
      selection.kind === "single" &&
      !selection.model &&
      !snapshot.supportsDefault
    )
      continue;
    if (selection.kind === "models" && !selection.models.length) continue;
    if (!targets.has(snapshot.writeTarget))
      targets.set(snapshot.writeTarget, {
        target: snapshot.writeTarget,
        selection,
      });
  }
  return [...targets.values()];
}

function selectionText(
  selection: ModelSelection | null,
  defaultLabel: string,
  emptyLabel: string,
  explicit = false,
) {
  if (!selection) return emptyLabel;
  return selection.kind === "models"
    ? selection.models.join(", ")
    : selection.model || (explicit ? emptyLabel : defaultLabel);
}

function targetLabel(target: string, shared: string) {
  return target === "codex" ? shared : (TOOL_META[target]?.label ?? target);
}

interface EditorState {
  profile?: ModelProfile;
  capture: boolean;
  targets: ProfileTarget[];
}

export function ModelProfilesPanel({ boundTools }: { boundTools: string[] }) {
  const { t } = useTranslation();
  const snapshots = useModelSelection(boundTools);
  const profiles = useQuery(
    {
      queryKey: MODEL_PROFILES_KEY,
      queryFn: modelProfilesApi.list,
      retry: false,
    },
    queryClient,
  );
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [deleting, setDeleting] = useState<ModelProfile | null>(null);
  const [review, setReview] = useState<{
    profile: ModelProfile;
    preview?: ProfilePreview;
    report?: ProfileApplyReport;
  } | null>(null);
  const [lastUndo, setLastUndo] = useState<{
    token: string;
    profile: ModelProfile;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const run = async (action: () => Promise<void>) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (error) {
      setError(String(error));
      toast.error(
        t("modelExperience.operationFailed", { error: String(error) }),
      );
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };
  const loadPreview = (profile: ModelProfile) => {
    setReview({ profile });
    void run(async () =>
      setReview({
        profile,
        preview: await modelProfilesApi.preview(profile.id),
      }),
    );
  };
  const apply = () => {
    if (!review?.preview?.canApply) return;
    const current = review;
    void run(async () => {
      const report = await modelProfilesApi.apply(current.preview!.planToken);
      setReview({ profile: current.profile, report });
      setLastUndo(
        report.undoToken
          ? { token: report.undoToken, profile: current.profile }
          : null,
      );
      await invalidateModelState();
    });
  };
  const undo = () => {
    if (!lastUndo) return;
    const current = lastUndo;
    void run(async () => {
      try {
        const report = await modelProfilesApi.undo(current.token);
        setReview({ profile: current.profile, report });
      } finally {
        // Native undo tokens are consumed by an attempt, including a stale-state rejection.
        setLastUndo(null);
        await invalidateModelState();
      }
    });
  };
  const sharedLabel = t("modelExperience.profiles.sharedCodex");
  const captured = captureProfileTargets(snapshots.data?.tools ?? []);
  const mixedDsh = snapshots.data?.tools.some(
    (snapshot) =>
      snapshot.toolId === "dsh" &&
      snapshot.binding.status === "configured" &&
      snapshot.managedSelection.kind === "single" &&
      !snapshot.managedSelection.model &&
      !snapshot.supportsDefault,
  );

  return (
    <div className="mx-auto max-w-4xl space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">
            {t("modelExperience.profiles.title")}
          </h1>
          <p className="mt-1 max-w-xl text-[12px] text-muted-foreground">
            {t("modelExperience.profiles.description")}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          {lastUndo && (
            <Button size="sm" variant="outline" disabled={busy} onClick={undo}>
              <Undo2 className="mr-1.5 h-3.5 w-3.5" />
              {t("modelExperience.profiles.undo")}
            </Button>
          )}
          <Button
            size="sm"
            variant="outline"
            disabled={
              busy ||
              snapshots.isPending ||
              snapshots.isError ||
              !captured.length
            }
            onClick={() => {
              setError(null);
              setEditor({ capture: true, targets: captured });
            }}
          >
            <Save className="mr-1.5 h-3.5 w-3.5" />
            {t("modelExperience.profiles.capture")}
          </Button>
          <Button
            size="sm"
            disabled={busy}
            onClick={() => {
              setError(null);
              setEditor({ capture: false, targets: [] });
            }}
          >
            <Plus className="mr-1.5 h-3.5 w-3.5" />
            {t("modelExperience.profiles.create")}
          </Button>
        </div>
      </div>
      {mixedDsh && (
        <p className="text-[12px] text-muted-foreground">
          {t("modelExperience.profiles.mixedDsh")}
        </p>
      )}
      {snapshots.isError && (
        <div role="alert" className="text-[12px] text-destructive">
          {t("modelExperience.stateFailed")}{" "}
          <button
            type="button"
            className="underline"
            onClick={() => void snapshots.refetch()}
          >
            {t("modelExperience.retry")}
          </button>
        </div>
      )}
      {error && !review && !deleting && (
        <p role="alert" className="text-[12px] text-destructive">
          {error}
        </p>
      )}
      {profiles.isPending ? (
        <p
          role="status"
          className="py-8 text-center text-sm text-muted-foreground"
        >
          {t("modelExperience.loading")}
        </p>
      ) : profiles.isError ? (
        <div role="alert" className="rounded-xl border p-5 text-sm">
          {t("modelExperience.profiles.loadFailed")}{" "}
          <Button
            size="sm"
            variant="outline"
            onClick={() => void profiles.refetch()}
          >
            {t("modelExperience.retry")}
          </Button>
        </div>
      ) : !profiles.data?.length ? (
        <div className="rounded-xl border border-dashed p-8 text-center text-sm text-muted-foreground">
          {t("modelExperience.profiles.empty")}
        </div>
      ) : (
        profiles.data.map((profile) => (
          <article
            key={profile.id}
            className="rounded-xl border border-border bg-background/80 p-4"
          >
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="flex min-w-0 items-center gap-2">
                <h2 className="truncate text-sm font-semibold">
                  {profile.name}
                </h2>
                <span
                  className={`shrink-0 rounded-md px-2 py-0.5 text-[11px] ${profile.match === "inUse" ? "bg-emerald-500/10 text-emerald-600" : "bg-muted text-muted-foreground"}`}
                >
                  {t(`modelExperience.profiles.match.${profile.match}`)}
                </span>
              </div>
              <div className="flex gap-1">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy}
                  onClick={() => loadPreview(profile)}
                >
                  {t("modelExperience.profiles.preview")}
                </Button>
                <Button
                  size="icon"
                  variant="ghost"
                  aria-label={t("modelExperience.profiles.edit", {
                    name: profile.name,
                  })}
                  disabled={busy}
                  onClick={() => {
                    setError(null);
                    setEditor({
                      profile,
                      capture: false,
                      targets: profile.targets,
                    });
                  }}
                >
                  <Pencil className="h-3.5 w-3.5" />
                </Button>
                <Button
                  size="icon"
                  variant="ghost"
                  aria-label={t("modelExperience.profiles.delete", {
                    name: profile.name,
                  })}
                  disabled={busy}
                  onClick={() => {
                    setError(null);
                    setDeleting(profile);
                  }}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>
            <dl className="mt-3 space-y-1.5 text-[12px]">
              {profile.targets.map((item) => (
                <div key={item.target} className="flex gap-3">
                  <dt className="w-36 shrink-0 text-muted-foreground">
                    {targetLabel(item.target, sharedLabel)}
                  </dt>
                  <dd className="min-w-0 break-words">
                    {selectionText(
                      item.selection,
                      t("modelPicker.defaultRoute"),
                      t("modelExperience.unknown"),
                      item.target === "pi" || item.target === "dsh",
                    )}
                  </dd>
                </div>
              ))}
            </dl>
          </article>
        ))
      )}
      {editor && (
        <ProfileEditor
          initial={editor}
          snapshots={snapshots.data?.tools ?? []}
          boundTools={boundTools}
          onClose={() => setEditor(null)}
          onSaved={() => {
            setEditor(null);
            void invalidateModelState();
          }}
        />
      )}
      <Dialog
        open={!!deleting}
        onOpenChange={(open) => {
          if (!open && !busy) setDeleting(null);
        }}
      >
        <DialogContent className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>
              {t("modelExperience.profiles.deleteTitle")}
            </DialogTitle>
            <DialogDescription>
              {t("modelExperience.profiles.deleteConfirm", {
                name: deleting?.name,
              })}
            </DialogDescription>
          </DialogHeader>
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => setDeleting(null)}
            >
              {t("modelExperience.cancel")}
            </Button>
            <Button
              variant="destructive"
              disabled={busy}
              onClick={() => {
                if (deleting)
                  void run(async () => {
                    await modelProfilesApi.delete(
                      deleting.id,
                      deleting.revision,
                    );
                    setDeleting(null);
                    await invalidateModelState();
                  });
              }}
            >
              {t("modelExperience.profiles.deleteAction")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog
        open={!!review}
        onOpenChange={(open) => {
          if (!open && !busy) {
            setReview(null);
            setError(null);
          }
        }}
      >
        <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>{review?.profile.name}</DialogTitle>
            <DialogDescription>
              {t(
                review?.report
                  ? "modelExperience.profiles.reportDescription"
                  : "modelExperience.profiles.previewDescription",
              )}
            </DialogDescription>
          </DialogHeader>
          {busy && (
            <div role="status" className="flex items-center gap-2 text-sm">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("modelExperience.profiles.working")}
            </div>
          )}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          {review?.preview && (
            <div className="space-y-2">
              {review.preview.items.map((item) => (
                <div
                  key={item.target}
                  className="rounded-lg border p-3 text-[12px]"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-medium">
                      {targetLabel(item.target, sharedLabel)}
                    </span>
                    <span
                      className={
                        item.status === "blocked"
                          ? "text-destructive"
                          : "text-muted-foreground"
                      }
                    >
                      {t(`modelExperience.profiles.preflight.${item.status}`)}
                    </span>
                  </div>
                  <div className="mt-1 break-words text-muted-foreground">
                    {selectionText(
                      item.before,
                      t("modelPicker.defaultRoute"),
                      t(
                        item.target === "dsh" && (item.scope?.length ?? 0) > 1
                          ? "modelExperience.mixedSelection"
                          : "modelExperience.unknown",
                      ),
                      item.target === "pi" || item.target === "dsh",
                    )}{" "}
                    →{" "}
                    {selectionText(
                      item.after,
                      t("modelPicker.defaultRoute"),
                      t("modelExperience.unknown"),
                      item.target === "pi" || item.target === "dsh",
                    )}
                  </div>
                  {!!item.scope?.length && (
                    <p className="mt-1 break-words text-muted-foreground">
                      {t("modelExperience.profiles.affectedScopes", {
                        scopes: item.scope.join(", "),
                      })}
                    </p>
                  )}
                  {item.reason && (
                    <p className="mt-1 break-words">
                      {t(item.reason, { defaultValue: item.reason })}
                    </p>
                  )}
                </div>
              ))}
            </div>
          )}
          {review?.report && (
            <div role="status" className="space-y-2">
              <p
                className={
                  review.report.status === "applied"
                    ? "text-emerald-600"
                    : "text-orange-600"
                }
              >
                {t(`modelExperience.profiles.result.${review.report.status}`)}
              </p>
              {review.report.items.map((item) => (
                <div
                  key={item.target}
                  className="rounded-lg border p-3 text-[12px]"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span>{targetLabel(item.target, sharedLabel)}</span>
                    <span>
                      {t(`modelExperience.profiles.outcome.${item.outcome}`)}
                    </span>
                  </div>
                  {item.reason && (
                    <p className="mt-1 break-words text-muted-foreground">
                      {t(item.reason, { defaultValue: item.reason })}
                    </p>
                  )}
                </div>
              ))}
            </div>
          )}
          <DialogFooter>
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => {
                setReview(null);
                setError(null);
              }}
            >
              {t("modelExperience.close")}
            </Button>
            {review?.report && lastUndo && (
              <Button variant="outline" disabled={busy} onClick={undo}>
                <Undo2 className="mr-1.5 h-3.5 w-3.5" />
                {t("modelExperience.profiles.undo")}
              </Button>
            )}
            {review && !review.report && (
              <>
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() => loadPreview(review.profile)}
                >
                  <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
                  {t("modelExperience.profiles.refreshPreview")}
                </Button>
                <Button
                  disabled={busy || !review.preview?.canApply}
                  onClick={apply}
                >
                  {t("modelExperience.profiles.apply")}
                </Button>
              </>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function ProfileEditor({
  initial,
  snapshots,
  boundTools,
  onClose,
  onSaved,
}: {
  initial: EditorState;
  snapshots: ToolModelSnapshot[];
  boundTools: string[];
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(initial.profile?.name ?? "");
  const [targets, setTargets] = useState(initial.targets);
  const [portal, setPortal] = useState<HTMLDivElement | null>(null);
  const [busy, setBusy] = useState(false);
  const saveLock = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const snapshotByTarget = new Map(
    snapshots.map((snapshot) => [snapshot.writeTarget, snapshot]),
  );
  const ids = [
    ...new Set([
      ...boundTools.map((id) => (id === "chatgpt" ? "codex" : id)),
      ...initial.targets.map((item) => item.target),
    ]),
  ].sort((a, b) => TOOL_ORDER.indexOf(a) - TOOL_ORDER.indexOf(b));
  const valid =
    name.trim().length > 0 &&
    name.trim().length <= 80 &&
    targets.length > 0 &&
    targets.every((item) =>
      item.selection.kind === "models"
        ? item.selection.models.length > 0
        : !!item.selection.model.trim() ||
          (snapshotByTarget.get(item.target)?.supportsDefault ??
            (item.target !== "pi" && item.target !== "dsh")),
    );
  const save = async () => {
    if (!valid || saveLock.current) return;
    saveLock.current = true;
    setBusy(true);
    setError(null);
    try {
      await modelProfilesApi.save({
        id: initial.profile?.id,
        name: name.trim(),
        targets,
        expectedRevision: initial.profile?.revision,
      });
      toast.success(t("modelExperience.profiles.saved"));
      onSaved();
    } catch (error) {
      setError(String(error));
    } finally {
      saveLock.current = false;
      setBusy(false);
    }
  };
  const updateTarget = (target: string, selection: ModelSelection | null) =>
    setTargets((previous) =>
      selection
        ? previous.some((item) => item.target === target)
          ? previous.map((item) =>
              item.target === target ? { target, selection } : item,
            )
          : [...previous, { target, selection }]
        : previous.filter((item) => item.target !== target),
    );
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent ref={setPortal} className="max-h-[85vh] sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {t(
              initial.profile
                ? "modelExperience.profiles.editTitle"
                : initial.capture
                  ? "modelExperience.profiles.capture"
                  : "modelExperience.profiles.create",
            )}
          </DialogTitle>
          <DialogDescription>
            {t("modelExperience.profiles.editorDescription")}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 space-y-4 overflow-y-auto px-6 py-4">
          <label className="block space-y-1.5 text-sm">
            <span>{t("modelExperience.profiles.name")}</span>
            <Input
              value={name}
              onChange={(event) => setName(event.target.value)}
              disabled={busy}
              maxLength={80}
              autoFocus
            />
          </label>
          {ids.length === 0 && (
            <p className="text-sm text-muted-foreground">
              {t("modelExperience.profiles.bindFirst")}
            </p>
          )}
          <div className="space-y-3">
            {ids.map((target) => (
              <ProfileTargetEditor
                key={target}
                target={target}
                snapshot={snapshotByTarget.get(target)}
                selection={
                  targets.find((item) => item.target === target)?.selection
                }
                disabled={busy}
                portal={portal}
                onChange={(selection) => updateTarget(target, selection)}
              />
            ))}
          </div>
          {error && (
            <p role="alert" className="break-words text-sm text-destructive">
              {error}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={onClose}>
            {t("modelExperience.cancel")}
          </Button>
          <Button disabled={busy || !valid} onClick={() => void save()}>
            {busy ? t("modelExperience.saving") : t("modelExperience.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ProfileTargetEditor({
  target,
  snapshot,
  selection,
  disabled,
  portal,
  onChange,
}: {
  target: string;
  snapshot?: ToolModelSnapshot;
  selection?: ModelSelection;
  disabled: boolean;
  portal: HTMLElement | null;
  onChange: (selection: ModelSelection | null) => void;
}) {
  const { t } = useTranslation();
  const catalog = useModelCatalog(target, !!selection);
  const multi = target === "opencode" || target === "workbuddy";
  const supportsDefault =
    snapshot?.supportsDefault ??
    (target !== "pi" && target !== "dsh" && !multi);
  const select = () => {
    if (selection) onChange(null);
    else
      onChange(
        snapshot?.managedSelection ??
          (multi
            ? { kind: "models", models: [] }
            : { kind: "single", model: "" }),
      );
  };
  return (
    <div className="space-y-2 rounded-lg border p-3">
      <div className="flex items-center gap-2">
        <Checkbox
          id={`profile-target-${target}`}
          checked={!!selection}
          disabled={disabled}
          onCheckedChange={select}
        />
        <label
          htmlFor={`profile-target-${target}`}
          className="text-sm font-medium"
        >
          {targetLabel(target, t("modelExperience.profiles.sharedCodex"))}
        </label>
        {snapshot?.binding.status !== "configured" && (
          <span className="text-[11px] text-muted-foreground">
            {t("modelExperience.profiles.unavailableTarget")}
          </span>
        )}
      </div>
      {selection && (
        <div className="space-y-2 pl-6">
          {selection.kind === "models" ? (
            <>
              <MultiModelPicker
                id={`profile-model-${target}`}
                selectedIds={selection.models}
                onChange={(models) => onChange({ kind: "models", models })}
                models={catalog.models}
                loading={catalog.loading}
                onFetch={catalog.fetch}
                portalContainer={portal}
                disabled={disabled}
              />
              <p className="break-words text-[11px] text-muted-foreground">
                {selection.models.join(", ")}
              </p>
            </>
          ) : (
            <>
              <ModelPicker
                id={`profile-model-${target}`}
                value={selection.model}
                onChange={(model) => onChange({ kind: "single", model })}
                models={catalog.models}
                loading={catalog.loading}
                onFetch={catalog.fetch}
                allowDefault={supportsDefault}
                emptyLabel={
                  !supportsDefault
                    ? t("modelExperience.chooseModel")
                    : undefined
                }
                portalContainer={portal}
                disabled={disabled}
              />
              {!supportsDefault && (
                <label className="block space-y-1 text-[11px] text-muted-foreground">
                  <span>{t("modelExperience.profiles.explicitModel")}</span>
                  <Input
                    value={selection.model}
                    disabled={disabled}
                    onChange={(event) =>
                      onChange({ kind: "single", model: event.target.value })
                    }
                  />
                </label>
              )}
            </>
          )}
          {catalog.error && (
            <p role="alert" className="text-[11px] text-destructive">
              {t("modelExperience.catalogFailed")}{" "}
              <button
                type="button"
                className="underline"
                disabled={disabled}
                onClick={catalog.fetch}
              >
                {t("modelExperience.retry")}
              </button>
            </p>
          )}
        </div>
      )}
    </div>
  );
}
