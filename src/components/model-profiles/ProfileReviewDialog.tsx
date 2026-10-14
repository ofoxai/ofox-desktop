import { useTranslation } from "react-i18next";
import { Loader2, RefreshCw, Undo2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { ModelProfilesController } from "@/hooks/useModelProfiles";
import { explicitTarget, selectionText, targetLabel } from "./profileText";

/**
 * A profile's preflight for every target, then applying it, then what each
 * target did, with the undo of that apply while it is still possible.
 */
export function ProfileReviewDialog({
  controller,
}: {
  controller: ModelProfilesController;
}) {
  const { t } = useTranslation();
  const { review, busy, error, lastUndo } = controller;
  const sharedLabel = t("modelExperience.profiles.sharedCodex");
  const defaultRoute = t("modelPicker.defaultRoute");
  const unknown = t("modelExperience.unknown");

  return (
    <Dialog
      open={!!review}
      onOpenChange={(open) => {
        if (!open) controller.closeReview();
      }}
    >
      <DialogContent zIndex="nested" className="max-h-[85vh] sm:max-w-2xl">
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
        <div className="min-h-0 space-y-2 overflow-y-auto px-[18px] py-3.5">
          {busy && (
            <div
              role="status"
              className="flex items-center gap-2 text-ctl text-ink-2"
            >
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("modelExperience.profiles.working")}
            </div>
          )}
          {error && (
            <p role="alert" className="break-words text-ctl text-danger">
              {error}
            </p>
          )}
          {review?.preview && (
            <div className="space-y-2">
              {review.preview.items.map((item) => (
                <div
                  key={item.target}
                  className="rounded-card border border-line p-3 text-ctl"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-medium text-ink">
                      {targetLabel(item.target, sharedLabel)}
                    </span>
                    <span
                      className={
                        item.status === "blocked"
                          ? "text-danger"
                          : "text-ink-muted"
                      }
                    >
                      {t(`modelExperience.profiles.preflight.${item.status}`)}
                    </span>
                  </div>
                  <div className="mt-1 break-words text-ink-muted">
                    {selectionText(
                      item.before,
                      defaultRoute,
                      t(
                        item.target === "dsh" && (item.scope?.length ?? 0) > 1
                          ? "modelExperience.mixedSelection"
                          : "modelExperience.unknown",
                      ),
                      explicitTarget(item.target),
                    )}{" "}
                    →{" "}
                    {selectionText(
                      item.after,
                      defaultRoute,
                      unknown,
                      explicitTarget(item.target),
                    )}
                  </div>
                  {!!item.scope?.length && (
                    <p className="mt-1 break-words text-ink-muted">
                      {t("modelExperience.profiles.affectedScopes", {
                        scopes: item.scope.join(", "),
                      })}
                    </p>
                  )}
                  {item.reason && (
                    <p className="mt-1 break-words text-ink-2">
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
                  review.report.status === "applied" ? "text-ok" : "text-warn"
                }
              >
                {t(`modelExperience.profiles.result.${review.report.status}`)}
              </p>
              {review.report.items.map((item) => (
                <div
                  key={item.target}
                  className="rounded-card border border-line p-3 text-ctl"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span>{targetLabel(item.target, sharedLabel)}</span>
                    <span>
                      {t(`modelExperience.profiles.outcome.${item.outcome}`)}
                    </span>
                  </div>
                  {item.reason && (
                    <p className="mt-1 break-words text-ink-muted">
                      {t(item.reason, { defaultValue: item.reason })}
                    </p>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
        <DialogFooter>
          <Button
            variant="secondary"
            disabled={busy}
            onClick={controller.closeReview}
          >
            {t("modelExperience.close")}
          </Button>
          {review?.report && lastUndo && (
            <Button
              variant="secondary"
              disabled={busy}
              onClick={controller.undo}
            >
              <Undo2 className="h-3.5 w-3.5" />
              {t("modelExperience.profiles.undo")}
            </Button>
          )}
          {review && !review.report && (
            <>
              <Button
                variant="secondary"
                disabled={busy}
                onClick={() => controller.preview(review.profile)}
              >
                <RefreshCw className="h-3.5 w-3.5" />
                {t("modelExperience.profiles.refreshPreview")}
              </Button>
              <Button
                disabled={busy || !review.preview?.canApply}
                onClick={controller.apply}
              >
                {t("modelExperience.profiles.apply")}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
