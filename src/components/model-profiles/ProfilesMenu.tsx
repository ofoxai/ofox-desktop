import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { MenuChevron } from "@/components/ui/menu-select";
import { useModelProfiles } from "@/hooks/useModelProfiles";
import { ProfileDialogs } from "./ModelProfilesPanel";
import { ProfilesDialog } from "./ProfilesDialog";

/**
 * 方案 ▾ in the tools toolbar: the saved model profiles, the one in use
 * ticked, each opening its review; saving the current combination, undoing
 * the last apply, and 管理方案… for the full list.
 */
export function ProfilesMenu({ boundTools }: { boundTools: string[] }) {
  const { t } = useTranslation();
  // The saved profiles load the first time the menu opens, never before.
  const [opened, setOpened] = useState(false);
  const profiles = useModelProfiles(boundTools, { enabled: opened });
  const [managing, setManaging] = useState(false);
  // A picked item's dialog opens once the menu has closed, so the menu's
  // focus return never pulls focus out of the dialog.
  const afterClose = useRef<(() => void) | null>(null);
  const later = (action: () => void) => () => {
    afterClose.current = action;
  };
  const list = profiles.profiles;

  return (
    <>
      <DropdownMenu
        modal={false}
        onOpenChange={(open) => {
          if (open) setOpened(true);
        }}
      >
        <DropdownMenuTrigger asChild>
          <Button variant="secondary" className="group">
            {t("console.profiles.menu")}
            <MenuChevron />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          className="w-[240px]"
          onCloseAutoFocus={(event) => {
            const action = afterClose.current;
            if (!action) return;
            afterClose.current = null;
            event.preventDefault();
            action();
          }}
        >
          {list.isPending ? (
            <p role="status" className="px-2 py-1.5 text-sub text-ink-muted">
              {t("console.profiles.loading")}
            </p>
          ) : list.isError ? (
            <DropdownMenuItem
              onSelect={(event) => {
                event.preventDefault();
                void list.refetch();
              }}
            >
              {t("modelExperience.profiles.loadFailed")} ·{" "}
              {t("modelExperience.retry")}
            </DropdownMenuItem>
          ) : !list.data?.length ? (
            <p className="px-2 py-1.5 text-sub text-ink-muted">
              {t("console.profiles.empty")}
            </p>
          ) : (
            list.data.map((profile) => (
              <DropdownMenuItem
                key={profile.id}
                disabled={profiles.busy}
                onSelect={later(() => profiles.preview(profile))}
              >
                <span className="flex w-3.5 flex-none justify-center text-brand">
                  {profile.match === "inUse" && (
                    <Check aria-hidden strokeWidth={2.4} className="h-3 w-3" />
                  )}
                </span>
                <span className="min-w-0 flex-1 truncate">{profile.name}</span>
                {profile.match === "inUse" && (
                  <span className="sr-only">
                    {t("modelExperience.profiles.match.inUse")}
                  </span>
                )}
                <span className="flex-none text-note text-ink-faint">
                  {t("console.profiles.toolCount", {
                    count: profile.targets.length,
                  })}
                </span>
              </DropdownMenuItem>
            ))
          )}
          <DropdownMenuSeparator />
          <DropdownMenuItem
            disabled={profiles.busy || !profiles.canCapture}
            onSelect={later(profiles.startCapture)}
          >
            {t("console.profiles.capture")}
          </DropdownMenuItem>
          {profiles.lastUndo && (
            <DropdownMenuItem
              disabled={profiles.busy}
              onSelect={later(profiles.undo)}
            >
              {t("modelExperience.profiles.undo")}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={later(() => setManaging(true))}>
            {t("console.profiles.manage")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <ProfilesDialog
        open={managing}
        onOpenChange={setManaging}
        controller={profiles}
      />
      <ProfileDialogs controller={profiles} />
    </>
  );
}
