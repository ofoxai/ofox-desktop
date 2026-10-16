import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { sessionsApi, type ManagedSession } from "@/lib/api/sessions";
import { fmtCost, fmtTokens, fmtBytes } from "@/lib/format";
import { CopyButton } from "@/components/ui/copy-button";
import { Button } from "@/components/ui/button";
import { SessionTranscript } from "./SessionTranscript";
import { useSessionExport } from "@/hooks/useSessionActions";
export function SessionDetails({
  session: s,
  onRead,
  onAnalyze,
}: {
  session: ManagedSession;
  onRead?: (session: ManagedSession) => void;
  onAnalyze?: (sessionKey: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const [show, setShow] = useState(false);
  const { busy: exporting, exportFile } = useSessionExport(s.sessionKey);
  const resume = useQuery({
    queryKey: ["session-command", s.sessionKey],
    queryFn: () => sessionsApi.resume(s.sessionKey, false),
    enabled: s.capabilities.canResume,
    retry: false,
  });
  const stamp = (n?: number) =>
    n == null ? "—" : new Date(n).toLocaleString(i18n.language);
  return (
    <div className="space-y-2 border-t border-line-2 py-3 pl-[38px] pr-3 text-[11.5px] text-ink-muted select-text">
      <div>
        {t("sessionCenter.time")}: {stamp(s.createdAt)} –{" "}
        {stamp(s.lastActiveAt)}
      </div>
      <div className="break-all">
        {t("sessionCenter.folder")}:{" "}
        {s.projectDir || t("sessionCenter.noProject")}
      </div>
      <div className="flex items-center gap-1 break-all">
        <span>
          {t("sessionCenter.id")}: {s.sessionId}
        </span>
        <CopyButton text={s.sessionId} aria-label={t("sessionCenter.copyId")} />
      </div>
      {resume.data && (
        <div className="flex items-center gap-1">
          <code className="min-w-0 break-all rounded-pill bg-pill px-2 py-1">
            {resume.data.command}
          </code>
          <CopyButton
            text={resume.data.command}
            aria-label={t("sessionCenter.copyResume")}
          />
        </div>
      )}
      {resume.isError && <p role="alert">{String(resume.error)}</p>}
      {!s.capabilities.canResume && (
        <p className="text-note">
          {t(
            s.capabilities.resumeUnavailableReason === "session.resume.archived"
              ? "sessionExperience.resumeArchived"
              : s.capabilities.resumeUnavailableReason ===
                  "session.resume.directoryMissing"
                ? "sessionExperience.resumeDirectoryMissing"
                : "sessionExperience.resumeUnavailable",
          )}
        </p>
      )}
      {s.usage?.models.map((m) => (
        <div
          key={m.model}
          className="grid grid-cols-[minmax(90px,1fr)_2fr_auto] gap-2 border-b border-line-2 pb-1"
        >
          <code className="break-all">{m.model}</code>
          <span>
            {t("sessionCenter.tokenDetails", {
              input: fmtTokens(m.inputTokens),
              output: fmtTokens(m.outputTokens),
              cache: fmtTokens(m.cacheReadTokens),
              write: fmtTokens(m.cacheWriteTokens),
            })}
          </span>
          <span className="text-ok">{fmtCost(m.costUsd)}</span>
        </div>
      ))}
      {!!s.files?.length && (
        <details>
          <summary className="cursor-pointer break-all">
            {t("sessionCenter.files")}: {s.files[0].path}
            {s.files.length > 1 &&
              ` · ${t("sessionCenter.moreCount", { count: s.files.length - 1 })}`}
          </summary>
          {s.files.map((f) => (
            <div key={f.path} className="flex items-center gap-1 break-all">
              <code>
                {f.path} · {fmtBytes(f.sizeBytes)}
              </code>
              <CopyButton text={f.path} />
            </div>
          ))}
        </details>
      )}
      <div className="flex gap-2">
        {s.capabilities.canReadMessages && (
          <Button
            variant="ghost"
            aria-expanded={show}
            onClick={() => (onRead ? onRead(s) : setShow(!show))}
          >
            {t(
              show ? "sessionCenter.hideContent" : "sessionCenter.viewContent",
            )}
          </Button>
        )}
        {s.capabilities.canExport && (
          <Button
            variant="ghost"
            disabled={exporting}
            onClick={() => void exportFile()}
          >
            {t("sessionCenter.export")}
          </Button>
        )}
        {onAnalyze && (
          <Button variant="ghost" onClick={() => onAnalyze(s.sessionKey)}>
            {t("sessionExperience.analyze")}
          </Button>
        )}
      </div>
      {show && (
        <SessionTranscript
          sessionKey={s.sessionKey}
          revision={s.revision ?? s.lastActiveAt}
        />
      )}
    </div>
  );
}
