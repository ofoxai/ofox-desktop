import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { LibraryIssue, LibraryNextAction } from "./libraryIssues";

export function LibraryIssueNotice({
  issue,
  disabled,
  onAction,
}: {
  issue: LibraryIssue;
  disabled?: boolean;
  onAction?: (action: LibraryNextAction, trigger: HTMLElement) => void;
}) {
  const { t } = useTranslation();
  const [guide, setGuide] = useState(false);
  return (
    <div className="space-y-2 rounded-ctl border border-line-2 bg-pill/40 px-3 py-2">
      <p className="text-ctl text-ink-2">
        {t(`library.issues.${issue.messageKey}Hint`)}
      </p>
      {(onAction || issue.action.kind === "nativeGuide") && (
        <Button
          variant="secondary"
          disabled={disabled}
          aria-expanded={
            issue.action.kind === "nativeGuide" ? guide : undefined
          }
          onClick={(event) =>
            issue.action.kind === "nativeGuide"
              ? setGuide((value) => !value)
              : onAction?.(issue.action, event.currentTarget)
          }
        >
          {t(`library.issues.${issue.actionKey}`)}
        </Button>
      )}
      {guide && (
        <p className="whitespace-pre-line text-ctl text-ink-muted">
          {t("library.issues.nativeSteps")}
        </p>
      )}
    </div>
  );
}
