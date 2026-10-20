import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Check } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { MenuChevron } from "@/components/ui/menu-select";
import { ProfileReviewDialog } from "@/components/model-profiles/ProfileReviewDialog";
import { useModelProfiles } from "@/hooks/useModelProfiles";

/**
 * magpie's footer 方案: the profile in use named, as a pop-up button shows
 * what is picked; the others a click away, each applied only after the same
 * review the main window shows. No profiles, no button: saving one belongs
 * to the main window.
 */
export default function TrayProfiles({ boundTools }: { boundTools: string[] }) {
  const { t } = useTranslation();
  const profiles = useModelProfiles(boundTools);
  // The review opens once the menu has closed, so focus lands in the dialog.
  const afterClose = useRef<(() => void) | null>(null);
  const list = profiles.profiles.data ?? [];
  if (!list.length) return null;
  const inUse = list.find((profile) => profile.match === "inUse");

  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            disabled={profiles.busy}
            className="group inline-flex h-6 max-w-full items-center gap-1 rounded-[6px] px-[7px] text-ctl text-ink-ctl transition-colors hover:bg-pill hover:text-ink data-[state=open]:bg-pill"
          >
            <span className="min-w-0 truncate">
              {inUse
                ? t("tray.profileInUse", { name: inUse.name })
                : t("console.profiles.menu")}
            </span>
            <MenuChevron />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          side="top"
          align="start"
          className="w-[240px]"
          onCloseAutoFocus={(event) => {
            const action = afterClose.current;
            if (!action) return;
            afterClose.current = null;
            event.preventDefault();
            action();
          }}
        >
          {list.map((profile) => (
            <DropdownMenuItem
              key={profile.id}
              disabled={profiles.busy}
              onSelect={() => {
                afterClose.current = () => profiles.preview(profile);
              }}
            >
              <span className="flex w-3.5 flex-none justify-center text-brand">
                {profile.match === "inUse" && (
                  <Check aria-hidden strokeWidth={2.4} className="h-3 w-3" />
                )}
              </span>
              <span className="min-w-0 flex-1 truncate">{profile.name}</span>
              <span className="flex-none text-note text-ink-faint">
                {t("console.profiles.toolCount", {
                  count: profile.targets.length,
                })}
              </span>
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      <ProfileReviewDialog controller={profiles} />
    </>
  );
}
