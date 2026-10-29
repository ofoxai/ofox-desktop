import { ChevronLeft, ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { TurnVersions } from "@/utils/assistantVersions";

/**
 * `‹ 2/3 ›` for a turn that has more than one version. A step that can't be
 * taken (an end, a reply running, a switch under way) is `aria-disabled`, not
 * `disabled`: the button keeps the keyboard focus the press gave it, and the
 * position it announces (a polite live region) is heard when it changes.
 */
export function VersionSwitcher({
  versions,
  disabled,
  onSwitch,
}: {
  versions: TurnVersions;
  disabled: boolean;
  /** Shows the version whose question has this message id. */
  onSwitch: (messageId: string) => void;
}) {
  const { t } = useTranslation();
  const step = (
    label: string,
    messageId: string | undefined,
    icon: "<" | ">",
  ) => {
    const Icon = icon === "<" ? ChevronLeft : ChevronRight;
    const blocked = disabled || !messageId;
    return (
      <Button
        variant="icon"
        className="h-6 w-6 aria-disabled:cursor-default aria-disabled:opacity-40 aria-disabled:hover:bg-transparent aria-disabled:hover:text-ink-muted aria-disabled:active:scale-100"
        aria-label={label}
        title={label}
        aria-disabled={blocked || undefined}
        onClick={() => {
          if (!blocked) onSwitch(messageId);
        }}
      >
        <Icon aria-hidden className="h-3.5 w-3.5" />
      </Button>
    );
  };
  return (
    <div
      role="group"
      aria-label={t("assistant.versions")}
      className="flex items-center text-note text-ink-ctl"
    >
      {step(t("assistant.previousVersion"), versions.previousId, "<")}
      <span aria-live="polite" aria-atomic="true" className="tabular-nums">
        {t("assistant.versionPosition", {
          index: versions.index,
          total: versions.count,
        })}
      </span>
      {step(t("assistant.nextVersion"), versions.nextId, ">")}
    </div>
  );
}
