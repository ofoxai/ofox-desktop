import { useTranslation } from "react-i18next";
import { CopyButton } from "@/components/ui/copy-button";
export function ResumeFeedback({
  command,
  feedback,
}: {
  command?: string;
  feedback?: string;
}) {
  const { t } = useTranslation();
  return feedback ? (
    <div role="status" className="space-y-1 px-3 pb-2 text-note text-ink-muted">
      <p>{feedback}</p>
      {command && (
        <div className="flex items-center gap-2">
          <code className="min-w-0 flex-1 break-all rounded-ctl bg-pill px-2 py-1">
            {command}
          </code>
          <CopyButton
            text={command}
            aria-label={t("sessionExperience.copyResume")}
          />
        </div>
      )}
    </div>
  ) : null;
}
