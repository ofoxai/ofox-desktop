import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  ModelPicker,
  MultiModelPicker,
} from "@/components/model-picker/ModelPicker";
import { TOOL_ORDER } from "@/config/toolMeta";
import { modelProfilesApi } from "@/lib/api/modelProfiles";
import type {
  ModelSelection,
  ToolModelSnapshot,
} from "@/lib/api/modelSelection";
import { useModelCatalog } from "@/hooks/useModelSelection";
import type {
  ModelProfilesController,
  ProfileEditorState,
} from "@/hooks/useModelProfiles";
import { explicitTarget, targetLabel } from "./profileText";

/** The controller's editor, while one is open. */
export function ProfileEditorDialog({
  controller,
}: {
  controller: ModelProfilesController;
}) {
  if (!controller.editor) return null;
  return (
    <ProfileEditor
      initial={controller.editor}
      snapshots={controller.snapshots.data?.tools ?? []}
      boundTools={controller.boundTools}
      onClose={controller.closeEditor}
      onSaved={controller.editorSaved}
    />
  );
}

function ProfileEditor({
  initial,
  snapshots,
  boundTools,
  onClose,
  onSaved,
}: {
  initial: ProfileEditorState;
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
        : !!item.selection.model.trim(),
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
      <DialogContent
        ref={setPortal}
        zIndex="nested"
        className="max-h-[85vh] sm:max-w-2xl"
      >
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
        <div className="min-h-0 space-y-4 overflow-y-auto px-[18px] py-4">
          <label className="block space-y-1.5 text-ui">
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
            <p className="text-ui text-ink-muted">
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
            <p role="alert" className="break-words text-ctl text-danger">
              {error}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
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
    <div className="space-y-2 rounded-card border border-line p-3">
      <div className="flex items-center gap-2">
        <Checkbox
          id={`profile-target-${target}`}
          checked={!!selection}
          disabled={disabled}
          onCheckedChange={select}
        />
        <label
          htmlFor={`profile-target-${target}`}
          className="text-ui font-medium text-ink"
        >
          {targetLabel(target, t("modelExperience.profiles.sharedCodex"))}
        </label>
        {snapshot?.binding.status !== "configured" && (
          <span className="text-note text-ink-muted">
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
              <p className="break-words text-note text-ink-muted">
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
                portalContainer={portal}
                disabled={disabled}
              />
              {explicitTarget(target) && (
                <label className="block space-y-1 text-note text-ink-muted">
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
            <p role="alert" className="text-note text-danger">
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
