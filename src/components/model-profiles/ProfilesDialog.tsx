import { useTranslation } from "react-i18next";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { ModelProfilesController } from "@/hooks/useModelProfiles";
import { ModelProfilesPanel } from "./ModelProfilesPanel";

interface ProfilesDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  controller: ModelProfilesController;
}

/**
 * 管理方案…: every saved profile, to preview, edit or delete, and new ones.
 * The controller's owner renders its editor, review and delete dialogs,
 * which open over this one.
 */
export function ProfilesDialog({
  open,
  onOpenChange,
  controller,
}: ProfilesDialogProps) {
  const { t } = useTranslation();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] max-w-[720px]">
        <DialogHeader>
          <DialogTitle>{t("modelExperience.profiles.title")}</DialogTitle>
          <DialogDescription>
            {t("modelExperience.profiles.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 overflow-y-auto px-[18px] py-4">
          <ModelProfilesPanel controller={controller} />
        </div>
      </DialogContent>
    </Dialog>
  );
}
