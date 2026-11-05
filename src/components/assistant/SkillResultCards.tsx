import { useTranslation } from "react-i18next";
import type {
  AssistantSkillResultGroup,
  AssistantSkillApproval,
} from "@/lib/api/assistant";
import { Button } from "@/components/ui/button";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";

export function SkillResultCards({
  groups,
}: {
  groups: AssistantSkillResultGroup[];
}) {
  const { t } = useTranslation();
  return (
    <div
      className="my-2 space-y-2"
      aria-label={t("assistant.skillTools.results")}
    >
      {groups.map((group) => (
        <ul
          key={group.toolCallId}
          className="grid min-w-0 gap-2 sm:grid-cols-2"
        >
          {group.results.slice(0, 8).map((result) => (
            <li
              key={result.resultId}
              className="min-w-0 space-y-1 rounded-list border border-line bg-card-2 p-3 text-ctl"
            >
              <p className="break-words font-medium text-ink">{result.name}</p>
              {result.installed === true && (
                <p className="text-note font-medium text-ink">
                  {t("assistant.skillTools.installed")}
                </p>
              )}
              <p className="break-all text-note text-ink-muted">
                {result.source}
              </p>
              {result.description && (
                <p className="break-words text-ink-2">{result.description}</p>
              )}
              <p className="text-note text-ink-muted">
                {t(
                  result.reviewed
                    ? "library.assistantTarget.reviewed"
                    : "library.assistantTarget.unreviewed",
                )}
                {result.installs != null && (
                  <>
                    {" "}
                    ·{" "}
                    {t("assistant.skillTools.installs", {
                      count: result.installs,
                    })}
                  </>
                )}
              </p>
            </li>
          ))}
        </ul>
      ))}
      <p className="text-note text-ink-muted">
        {t("assistant.skillTools.originHint")}
      </p>
    </div>
  );
}
export function SkillApprovalCard({
  details,
  install,
  busy,
  onRespond,
}: {
  details: AssistantSkillApproval;
  install: boolean;
  busy: boolean;
  onRespond: (choice: "once" | "deny") => void;
}) {
  const { t } = useTranslation();
  return (
    <div
      role="group"
      aria-label={t(
        install
          ? "assistant.skillTools.installTitle"
          : "assistant.skillTools.saveTitle",
      )}
      className="mt-2 min-w-0 space-y-2 rounded-list border border-line bg-card-2 p-3"
    >
      <p className="break-words font-medium text-ink">
        {t(
          install
            ? "assistant.skillTools.installTitle"
            : "assistant.skillTools.saveTitle",
        )}{" "}
        · {details.name}
      </p>
      {install && (
        <div className="space-y-1 text-note text-ink-2">
          <p className="break-all">{details.source}</p>
          <p className="break-all">{details.directory || "/"}</p>
          <p className="break-all font-mono">{details.commitSha}</p>
          <p>
            {t(
              details.reviewed
                ? "library.assistantTarget.reviewed"
                : "library.assistantTarget.unreviewed",
            )}
            {details.installs != null && (
              <>
                {" "}
                ·{" "}
                {t("assistant.skillTools.installs", {
                  count: details.installs,
                })}
              </>
            )}
          </p>
          <p>{t("assistant.skillTools.originHint")}</p>
        </div>
      )}
      {details.description && (
        <p className="break-words text-ctl text-ink-2">{details.description}</p>
      )}
      {!install && (
        <div
          tabIndex={0}
          aria-label={t("assistant.skillTools.preview")}
          className="max-h-80 overflow-auto rounded-ctl border border-line p-2"
        >
          <SessionMarkdown content={details.body ?? ""} />
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          variant="secondary"
          disabled={busy}
          onClick={() => onRespond("once")}
        >
          {t(
            install
              ? "assistant.skillTools.install"
              : "assistant.skillTools.save",
          )}
        </Button>
        <Button
          variant="ghost"
          disabled={busy}
          onClick={() => onRespond("deny")}
        >
          {t("common.cancel")}
        </Button>
      </div>
    </div>
  );
}
