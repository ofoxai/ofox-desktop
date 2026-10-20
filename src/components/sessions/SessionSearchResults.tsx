import { useTranslation } from "react-i18next";
import type { ManagedSession, SessionSearchHit } from "@/lib/api/sessions";
import type { useSessionContentSearch } from "@/hooks/useSessionContentSearch";
import { Button } from "@/components/ui/button";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { highlightText } from "./utils";

export function SessionSearchResults({
  search,
  sessions,
  needle,
  onOpen,
  onStart,
}: {
  search: ReturnType<typeof useSessionContentSearch>;
  sessions: ManagedSession[];
  needle: string;
  onOpen: (session: ManagedSession, hit: SessionSearchHit) => void;
  onStart: () => void;
}) {
  const { t } = useTranslation();
  const byKey = new Map(
    sessions.map((session) => [session.sessionKey, session]),
  );
  const error = search.error ?? "";
  const limits: Record<string, string> = {
    sessionLimit: "limitSession",
    lineLimit: "limitLine",
    batchLimit: "limitBatch",
    databaseLimit: "limitDatabase",
    deadline: "limitTime",
    hitLimit: "limitHits",
  };
  const errorKey = /invalidQuery/.test(error)
    ? "searchInvalid"
    : /expired/.test(error)
      ? "searchExpired"
      : /changed/.test(error)
        ? "changed"
        : /unknown command|not found|does not exist|not implemented/i.test(
              error,
            )
          ? "searchUnavailable"
          : "searchFailed";
  return (
    <div className="space-y-3">
      {search.progress && (
        <div
          role="status"
          aria-live="polite"
          className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sub text-ink-muted"
        >
          <span>{t("sessionExperience.searchProgress", search.progress)}</span>
          <span>
            {t("sessionExperience.searchResults", {
              count: search.hits.length,
            })}
          </span>
          {search.progress.changed +
            search.progress.failed +
            search.progress.unsupported >
            0 && (
            <span>{t("sessionExperience.searchIssues", search.progress)}</span>
          )}
        </div>
      )}
      {search.state === "limited" && (
        <div role="status" className="space-y-1 text-sub text-ink-muted">
          <p>{t("sessionExperience.searchPartial")}</p>
          {!!search.limitReasons?.length && (
            <ul className="list-disc pl-4 text-note">
              {search.limitReasons
                .filter((reason) => limits[reason])
                .map((reason) => (
                  <li key={reason}>
                    {t(`sessionExperience.${limits[reason]}`)}
                  </li>
                ))}
            </ul>
          )}
        </div>
      )}
      {search.state === "cancelled" && (
        <p role="status" className="text-sub text-ink-muted">
          {t("sessionExperience.searchCancelled")}
        </p>
      )}
      {search.state === "complete" && (
        <p role="status" className="text-note text-ink-muted">
          {t("sessionExperience.searchComplete")}
        </p>
      )}
      {search.state === "error" && (
        <div
          role="alert"
          className="flex items-center gap-2 text-sub text-danger"
        >
          {t(`sessionExperience.${errorKey}`)}
          <Button variant="ghost" onClick={onStart}>
            {t("common.retry")}
          </Button>
        </div>
      )}
      {!search.busy &&
        search.state !== "idle" &&
        search.state !== "error" &&
        !search.hits.length && (
          <p className="py-8 text-center text-sub text-ink-muted">
            {t("sessionExperience.searchNoResults")}
          </p>
        )}
      <div
        className="divide-y divide-line-2 overflow-hidden rounded-list border border-line bg-card"
        hidden={!search.hits.length}
      >
        {search.hits.map((hit) => {
          const session = byKey.get(hit.sessionKey);
          return (
            <button
              key={`${hit.sessionKey}:${hit.readRevision}:${hit.messageOrdinal}:${hit.contentHash}`}
              type="button"
              disabled={!session}
              onClick={() => session && onOpen(session, hit)}
              className="flex w-full gap-3 px-4 py-3 text-left hover:bg-pill disabled:opacity-50"
            >
              <ToolBadge toolId={session?.providerId ?? ""} size={24} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-ui font-medium text-ink">
                  {session?.title || t("sessionCenter.untitled")}
                </span>
                <span className="mb-1 block truncate text-note text-ink-muted">
                  {session?.projectDir || t("sessionCenter.noProject")} ·{" "}
                  {hit.name || t(`sessionCenter.roles.${hit.role}`)}
                </span>
                <span className="block whitespace-pre-wrap break-words text-sub leading-relaxed text-ink-2">
                  {highlightText(hit.snippet, needle.trim())}
                </span>
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
