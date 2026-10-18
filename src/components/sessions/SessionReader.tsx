import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import {
  sessionsApi,
  type ManagedSession,
  type SessionSearchHit,
  type SessionSubagent,
} from "@/lib/api/sessions";
import { Button } from "@/components/ui/button";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { SessionTranscript } from "./SessionTranscript";
import { useSessionExport, useSessionResume } from "@/hooks/useSessionActions";
import { ResumeFeedback } from "./ResumeFeedback";
import { SessionUsageSummary } from "./SessionUsageSummary";
import { CopyButton } from "@/components/ui/copy-button";

export function SessionReader({
  session,
  subagent,
  expected,
  onBack,
  onAnalyze,
  onResearch,
}: {
  session: ManagedSession;
  /** One of the session's subagents, read in its place: no actions of its own. */
  subagent?: SessionSubagent;
  expected?: SessionSearchHit;
  onBack: () => void;
  onAnalyze?: (sessionKey: string) => void;
  onResearch?: () => void;
}) {
  const { t } = useTranslation();
  const back = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    back.current?.focus();
  }, [session.sessionKey, subagent?.id]);
  const actions = !subagent;
  const usage = session.usage;
  const resume = useSessionResume(session.sessionKey);
  const exporting = useSessionExport(session.sessionKey);
  const command = useQuery({
    queryKey: ["session-command", session.sessionKey],
    queryFn: () => sessionsApi.resume(session.sessionKey, false),
    enabled: session.capabilities.canResume,
    retry: false,
  });
  return (
    <section
      aria-label={t("sessionExperience.reader")}
      className="flex h-full min-h-0 flex-col gap-3"
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.defaultPrevented) {
          event.preventDefault();
          onBack();
        }
      }}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Button ref={back} variant="ghost" onClick={onBack}>
          <ArrowLeft aria-hidden className="h-3.5 w-3.5" />
          {t("sessionExperience.back")}
        </Button>
        <span className="flex-1" />
        {actions && session.capabilities.canResume && (
          <CopyButton
            text={command.data?.command ?? ""}
            disabled={!command.data}
            aria-label={t("sessionExperience.copyResume")}
          />
        )}
        {actions && session.capabilities.canExport && (
          <Button
            variant="ghost"
            disabled={exporting.busy}
            onClick={() => void exporting.exportFile()}
          >
            {t("sessionCenter.export")}
          </Button>
        )}
        {actions && session.capabilities.canResume && (
          <Button
            variant="secondary"
            disabled={resume.busy}
            onClick={() => void resume.launch()}
          >
            {t("sessionExperience.resumeTerminal")}
          </Button>
        )}
        {actions && onAnalyze && (
          <Button
            variant="secondary"
            onClick={() => onAnalyze(session.sessionKey)}
          >
            {t("sessionExperience.analyze")}
          </Button>
        )}
      </div>
      <ResumeFeedback command={resume.command} feedback={resume.feedback} />
      {command.isError && (
        <p role="alert" className="text-sub text-danger">
          {String(command.error)}{" "}
          <Button variant="ghost" onClick={() => void command.refetch()}>
            {t("common.retry")}
          </Button>
        </p>
      )}
      {actions && !session.capabilities.canResume && (
        <p className="text-note text-ink-muted">
          {t(
            session.capabilities.resumeUnavailableReason ===
              "session.resume.archived"
              ? "sessionExperience.resumeArchived"
              : session.capabilities.resumeUnavailableReason ===
                  "session.resume.directoryMissing"
                ? "sessionExperience.resumeDirectoryMissing"
                : "sessionExperience.resumeUnavailable",
          )}
        </p>
      )}
      <div className="flex items-start gap-3 px-1">
        <ToolBadge toolId={session.providerId} size={28} />
        <div className="min-w-0 flex-1">
          <h2 className="break-words text-lg font-semibold text-ink">
            {subagent
              ? subagent.title || subagent.kind || subagent.id
              : session.title || t("sessionCenter.untitled")}
          </h2>
          <p
            className="truncate text-sub text-ink-muted"
            title={session.projectDir || undefined}
          >
            {subagent
              ? [
                  subagent.title ? subagent.kind : "",
                  session.title || t("sessionCenter.untitled"),
                ]
                  .filter(Boolean)
                  .join(" · ")
              : session.projectDir || t("sessionCenter.noProject")}
          </p>
        </div>
        {actions && (
          <SessionUsageSummary
            usage={usage}
            display={session.usageDisplay}
            stacked
          />
        )}
      </div>
      <SessionTranscript
        key={`${session.sessionKey}/${subagent?.id ?? ""}`}
        sessionKey={session.sessionKey}
        subagentId={subagent?.id}
        revision={session.revision ?? session.lastActiveAt}
        expected={expected}
        onResearch={onResearch}
      />
    </section>
  );
}
