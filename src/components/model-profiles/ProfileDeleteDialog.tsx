import { useTranslation } from "react-i18next";
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

/** Deletes a saved profile; what it applied to the tools stays. */
export function ProfileDeleteDialog({
  controller,
}: {
  controller: ModelProfilesController;
}) {
  const { t } = useTranslation();
  const { deleting, busy, error } = controller;
  return (
    <Dialog
      open={!!deleting}
      onOpenChange={(open) => {
        if (!open) controller.cancelDelete();
      }}
    >
      <DialogContent zIndex="nested" className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>{t("modelExperience.profiles.deleteTitle")}</DialogTitle>
          <DialogDescription>
            {t("modelExperience.profiles.deleteConfirm", {
              name: deleting?.name,
            })}
          </DialogDescription>
        </DialogHeader>
        {error && (
          <p role="alert" className="px-[18px] pt-3 text-ctl text-danger">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button
            variant="secondary"
            disabled={busy}
            onClick={controller.cancelDelete}
          >
            {t("modelExperience.cancel")}
          </Button>
          <Button
            variant="danger"
            disabled={busy}
            onClick={controller.confirmDelete}
          >
            {t("modelExperience.profiles.deleteAction")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
