import { useTranslation } from "react-i18next";
import { Plus, Save, Pencil, Trash2, Undo2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ListCard, ListRow } from "@/components/ui/list";
import type { ModelProfile } from "@/lib/api/modelProfiles";
import { cn } from "@/lib/utils";
import {
  useModelProfiles,
  type ModelProfilesController,
} from "@/hooks/useModelProfiles";
import { ProfileDeleteDialog } from "./ProfileDeleteDialog";
import { ProfileEditorDialog } from "./ProfileEditorDialog";
import { ProfileReviewDialog } from "./ProfileReviewDialog";
import { selectionText, targetLabel } from "./profileText";

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

/** A profile's match as a soft chip: in use is green, partial amber. */
const MATCH_TONE: Record<ModelProfile["match"], string> = {
  inUse: "bg-ok-soft text-ok",
  partial: "bg-warn-soft text-warn",
  different: "bg-pill text-ink-muted",
  unavailable: "bg-pill text-ink-faint",
};

function ProfilesList({ controller }: { controller: ModelProfilesController }) {
  const { t } = useTranslation();
  const { snapshots, profiles, busy, error, lastUndo } = controller;
  const sharedLabel = t("modelExperience.profiles.sharedCodex");

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-end gap-2">
        {lastUndo && (
          <Button variant="secondary" disabled={busy} onClick={controller.undo}>
            <Undo2 aria-hidden className="h-3.5 w-3.5" />
            {t("modelExperience.profiles.undo")}
          </Button>
        )}
        <Button
          variant="secondary"
          disabled={busy || !controller.canCapture}
          onClick={controller.startCapture}
        >
          <Save aria-hidden className="h-3.5 w-3.5" />
          {t("modelExperience.profiles.capture")}
        </Button>
        <Button disabled={busy} onClick={controller.startCreate}>
          <Plus aria-hidden className="h-3.5 w-3.5" />
          {t("modelExperience.profiles.create")}
        </Button>
      </div>
      {controller.mixedDsh && (
        <p className="text-ctl text-ink-muted">
          {t("modelExperience.profiles.mixedDsh")}
        </p>
      )}
      {snapshots.isError && (
        <div role="alert" className="text-ctl text-danger">
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
        <p role="alert" className="break-words text-ctl text-danger">
          {error}
        </p>
      )}
      {profiles.isPending ? (
        <p role="status" className="py-8 text-center text-ui text-ink-muted">
          {t("modelExperience.loading")}
        </p>
      ) : profiles.isError ? (
        <div
          role="alert"
          className="flex items-center justify-between gap-3 rounded-list border border-line bg-card py-3 pl-[14px] pr-3 text-ui text-ink-2"
        >
          {t("modelExperience.profiles.loadFailed")}
          <Button variant="secondary" onClick={() => void profiles.refetch()}>
            {t("modelExperience.retry")}
          </Button>
        </div>
      ) : !profiles.data?.length ? (
        <div className="rounded-list border border-dashed border-line px-6 py-8 text-center text-ui text-ink-muted">
          {t("modelExperience.profiles.empty")}
        </div>
      ) : (
        profiles.data.map((profile) => (
          <ListCard key={profile.id} role="group" aria-label={profile.name}>
            <ListRow>
              <div className="flex min-w-0 flex-1 items-center gap-2">
                <h3 className="truncate text-ui font-medium text-ink">
                  {profile.name}
                </h3>
                <span
                  className={cn(
                    "flex-none rounded-full px-2 py-px text-note font-medium",
                    MATCH_TONE[profile.match],
                  )}
                >
                  {t(`modelExperience.profiles.match.${profile.match}`)}
                </span>
              </div>
              <div className="flex flex-none items-center gap-1">
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={busy}
                  onClick={() => controller.preview(profile)}
                >
                  {t("modelExperience.profiles.preview")}
                </Button>
                <Button
                  variant="icon"
                  aria-label={t("modelExperience.profiles.edit", {
                    name: profile.name,
                  })}
                  disabled={busy}
                  onClick={() => controller.startEdit(profile)}
                >
                  <Pencil aria-hidden className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="dangerGhost"
                  size="icon"
                  aria-label={t("modelExperience.profiles.delete", {
                    name: profile.name,
                  })}
                  disabled={busy}
                  onClick={() => controller.askDelete(profile)}
                >
                  <Trash2 aria-hidden className="h-3.5 w-3.5" />
                </Button>
              </div>
            </ListRow>
            <dl className="space-y-1 py-2.5 pl-[14px] pr-3 text-ctl">
              {profile.targets.map((item) => (
                <div key={item.target} className="flex gap-3">
                  <dt className="w-36 flex-none text-ink-muted">
                    {targetLabel(item.target, sharedLabel)}
                  </dt>
                  <dd className="min-w-0 break-words text-ink-2">
                    {selectionText(item.selection, t("modelExperience.unset"))}
                  </dd>
                </div>
              ))}
            </dl>
          </ListCard>
        ))
      )}
    </div>
  );
}
