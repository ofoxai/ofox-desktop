import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { queryClient } from "@/lib/query/queryClient";
import {
  modelProfilesApi,
  type ModelProfile,
  type ProfileApplyReport,
  type ProfilePreview,
  type ProfileTarget,
} from "@/lib/api/modelProfiles";
import type { ToolModelSnapshot } from "@/lib/api/modelSelection";
import {
  invalidateModelState,
  MODEL_PROFILES_KEY,
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
    if (selection.kind === "single" && !selection.model) continue;
    if (selection.kind === "models" && !selection.models.length) continue;
    if (!targets.has(snapshot.writeTarget))
      targets.set(snapshot.writeTarget, {
        target: snapshot.writeTarget,
        selection,
      });
  }
  return [...targets.values()];
}

/** The profile being created, captured from the tools, or edited. */
export interface ProfileEditorState {
  profile?: ModelProfile;
  capture: boolean;
  targets: ProfileTarget[];
}

/** A profile's preflight, then the report of applying or undoing it. */
export interface ProfileReview {
  profile: ModelProfile;
  preview?: ProfilePreview;
  report?: ProfileApplyReport;
}

export interface ModelProfilesOptions {
  /** Load the saved profiles. The 方案 menu waits until it is first opened. */
  enabled?: boolean;
}

/**
 * The model profiles and everything done with them: the saved list, the
 * tools' current models, the open editor, review and delete, one operation
 * at a time, and the undo token of the last apply.
 */
export function useModelProfiles(
  boundTools: string[],
  { enabled = true }: ModelProfilesOptions = {},
) {
  const { t } = useTranslation();
  const snapshots = useModelSelection(boundTools);
  const profiles = useQuery(
    {
      queryKey: MODEL_PROFILES_KEY,
      queryFn: modelProfilesApi.list,
      retry: false,
      enabled,
    },
    queryClient,
  );
  const [editor, setEditor] = useState<ProfileEditorState | null>(null);
  const [deleting, setDeleting] = useState<ModelProfile | null>(null);
  const [review, setReview] = useState<ProfileReview | null>(null);
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

  /** Open the review of a profile and run its preflight. */
  const preview = (profile: ModelProfile) => {
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

  const closeReview = () => {
    if (busyRef.current) return;
    setReview(null);
    setError(null);
  };

  const openEditor = (state: ProfileEditorState) => {
    setError(null);
    setEditor(state);
  };

  const askDelete = (profile: ModelProfile) => {
    setError(null);
    setDeleting(profile);
  };

  const cancelDelete = () => {
    if (!busyRef.current) setDeleting(null);
  };

  const confirmDelete = () => {
    const profile = deleting;
    if (!profile) return;
    void run(async () => {
      await modelProfilesApi.delete(profile.id, profile.revision);
      setDeleting(null);
      await invalidateModelState();
    });
  };

  const tools = snapshots.data?.tools ?? [];
  const captured = captureProfileTargets(tools);
  const mixedDsh = tools.some(
    (snapshot) =>
      snapshot.toolId === "dsh" &&
      snapshot.binding.status === "configured" &&
      snapshot.managedSelection.kind === "single" &&
      !snapshot.managedSelection.model,
  );

  return {
    boundTools,
    snapshots,
    profiles,
    busy,
    error,
    /** The tools' models as a profile would save them now. */
    captured,
    canCapture:
      !snapshots.isPending && !snapshots.isError && captured.length > 0,
    mixedDsh,
    editor,
    startCapture: () => openEditor({ capture: true, targets: captured }),
    startCreate: () => openEditor({ capture: false, targets: [] }),
    startEdit: (profile: ModelProfile) =>
      openEditor({ profile, capture: false, targets: profile.targets }),
    closeEditor: () => setEditor(null),
    editorSaved: () => {
      setEditor(null);
      void invalidateModelState();
    },
    deleting,
    askDelete,
    cancelDelete,
    confirmDelete,
    review,
    preview,
    closeReview,
    apply,
    lastUndo,
    undo,
  };
}

export type ModelProfilesController = ReturnType<typeof useModelProfiles>;
