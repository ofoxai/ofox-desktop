import { useTranslation } from "react-i18next";
import { Plus, Save, Pencil, Trash2, Undo2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  useModelProfiles,
  type ModelProfilesController,
} from "@/hooks/useModelProfiles";
import { ProfileDeleteDialog } from "./ProfileDeleteDialog";
import { ProfileEditorDialog } from "./ProfileEditorDialog";
import { ProfileReviewDialog } from "./ProfileReviewDialog";
import { explicitTarget, selectionText, targetLabel } from "./profileText";

export { captureProfileTargets } from "@/hooks/useModelProfiles";

type ModelProfilesPanelProps =
  /** Shown by whoever owns the controller, who also renders its dialogs. */
  | { controller: ModelProfilesController; boundTools?: undefined }
  /** On its own: it loads the profiles and renders its dialogs itself. */
  | { boundTools: string[]; controller?: undefined };

export function ModelProfilesPanel(props: ModelProfilesPanelProps) {
  return props.controller ? (
    <ProfilesList controller={props.controller} />
  ) : (
    <StandaloneProfilesPanel boundTools={props.boundTools} />
  );
}

function StandaloneProfilesPanel({ boundTools }: { boundTools: string[] }) {
  const controller = useModelProfiles(boundTools);
  return (
    <>
      <ProfilesList controller={controller} />
      <ProfileDialogs controller={controller} />
    </>
  );
}

/** The editor, review and delete dialogs of one controller. */
export function ProfileDialogs({
  controller,
}: {
  controller: ModelProfilesController;
}) {
  return (
    <>
      <ProfileEditorDialog controller={controller} />
      <ProfileDeleteDialog controller={controller} />
      <ProfileReviewDialog controller={controller} />
    </>
  );
}

function ProfilesList({ controller }: { controller: ModelProfilesController }) {
  const { t } = useTranslation();
  const { snapshots, profiles, busy, error, lastUndo } = controller;
  const sharedLabel = t("modelExperience.profiles.sharedCodex");

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
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={controller.undo}
            >
              <Undo2 className="mr-1.5 h-3.5 w-3.5" />
              {t("modelExperience.profiles.undo")}
            </Button>
          )}
          <Button
            size="sm"
            variant="outline"
            disabled={busy || !controller.canCapture}
            onClick={controller.startCapture}
          >
            <Save className="mr-1.5 h-3.5 w-3.5" />
            {t("modelExperience.profiles.capture")}
          </Button>
          <Button size="sm" disabled={busy} onClick={controller.startCreate}>
            <Plus className="mr-1.5 h-3.5 w-3.5" />
            {t("modelExperience.profiles.create")}
          </Button>
        </div>
      </div>
      {controller.mixedDsh && (
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
      {error && !controller.review && !controller.deleting && (
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
                  onClick={() => controller.preview(profile)}
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
                  onClick={() => controller.startEdit(profile)}
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
                  onClick={() => controller.askDelete(profile)}
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
                      explicitTarget(item.target),
                    )}
                  </dd>
                </div>
              ))}
            </dl>
          </article>
        ))
      )}
    </div>
  );
}
