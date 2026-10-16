import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";
import { sessionsApi, type SessionSearchHit } from "@/lib/api/sessions";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { cn } from "@/lib/utils";
import { TranscriptPart } from "./TranscriptPart";

export function SessionTranscript({
  sessionKey,
  revision,
  expected,
  onResearch,
}: {
  sessionKey: string;
  revision?: string | number | null;
  expected?: SessionSearchHit;
  onResearch?: () => void;
}) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  const frame = useRef<number>();
  const [find, setFind] = useState("");
  const [matchIndex, setMatchIndex] = useState(0);
  const [focusedOrdinal, setFocusedOrdinal] = useState<number>();
  const [showQuestions, setShowQuestions] = useState(false);
  const query = useQuery({
    queryKey: [
      "session-messages",
      sessionKey,
      revision,
      expected?.readRevision,
      expected?.messageOrdinal,
      expected?.contentHash,
    ],
    queryFn: () =>
      sessionsApi
        .getTranscript(sessionKey, expected)
        .then((result) => result.messages),
    retry: false,
    staleTime: 0,
    refetchOnMount: "always",
    gcTime: 0,
  });
  // Every explicit open must verify the native source again, even on a cache hit.
  const verified = query.isFetchedAfterMount && !query.isError;
  const rows = verified ? query.data : undefined;
  const matches = useMemo(
    () =>
      !find.trim()
        ? []
        : (rows ?? []).flatMap((message, index) =>
            message.content
              .toLocaleLowerCase()
              .includes(find.trim().toLocaleLowerCase())
              ? [index]
              : [],
          ),
    [rows, find],
  );
  const questions = useMemo(
    () =>
      (rows ?? []).flatMap((message, index) =>
        message.role === "user"
          ? [
              {
                index,
                title: message.content.replace(/\s+/g, " ").slice(0, 100),
              },
            ]
          : [],
      ),
    [rows],
  );
  const virtual = useVirtualizer({
    count: rows?.length ?? 0,
    getScrollElement: () => ref.current,
    estimateSize: () => 200,
    overscan: 4,
  });
  const jump = useCallback(
    (ordinal: number, focus = true) => {
      setFocusedOrdinal(ordinal);
      virtual.scrollToIndex?.(ordinal, { align: "start" });
      window.cancelAnimationFrame(frame.current ?? 0);
      if (!focus) return;
      frame.current = window.requestAnimationFrame(() => {
        ref.current
          ?.querySelector<HTMLElement>(`[data-message-ordinal="${ordinal}"]`)
          ?.focus({ preventScroll: true });
      });
    },
    [virtual],
  );
  const expectedIdentity = expected
    ? `${expected.readRevision}:${expected.messageOrdinal}:${expected.contentHash}`
    : "";
  const didLocate = useRef("");
  useEffect(() => {
    if (!rows || !expected || didLocate.current === expectedIdentity) return;
    didLocate.current = expectedIdentity;
    jump(expected.messageOrdinal);
  }, [rows, expected, expectedIdentity, jump]);
  useEffect(() => {
    setMatchIndex(0);
    if (matches.length) jump(matches[0], false);
  }, [matches, jump]);
  useEffect(() => () => window.cancelAnimationFrame(frame.current ?? 0), []);
  const next = (step: number) => {
    if (!matches.length) return;
    const index = (matchIndex + step + matches.length) % matches.length;
    setMatchIndex(index);
    jump(matches[index], false);
  };
  const error = String(query.error);
  const changed = !!expected && error.includes("session.search.changed");
  const unsupported =
    /unknown command|not found|does not exist|not implemented/i.test(error);
  const limited =
    /session\.search\.(sessionLimit|lineLimit|batchLimit|databaseLimit|deadline)/.test(
      error,
    );
  const errorLabel = changed
    ? "sessionExperience.changed"
    : unsupported
      ? "sessionExperience.readerUnavailable"
      : limited
        ? "sessionExperience.readerReadLimit"
        : "sessionExperience.readerReadFailed";
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-list border border-line bg-card">
      <div className="flex flex-wrap items-center gap-2 border-b border-line-2 px-3 py-2">
        <SearchInput
          aria-label={t("sessionExperience.findInSession")}
          placeholder={t("sessionExperience.findInSession")}
          value={find}
          onChange={setFind}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              next(event.shiftKey ? -1 : 1);
            }
          }}
          className="w-[240px]"
        />
        {find && (
          <>
            <span role="status" className="text-note text-ink-muted">
              {matches.length
                ? t("sessionExperience.matches", {
                    current: matchIndex + 1,
                    count: matches.length,
                  })
                : t("sessionExperience.noMatches")}
            </span>
            <Button
              variant="ghost"
              disabled={!matches.length}
              aria-label={t("sessionExperience.previousMatch")}
              onClick={() => next(-1)}
            >
              ↑
            </Button>
            <Button
              variant="ghost"
              disabled={!matches.length}
              aria-label={t("sessionExperience.nextMatch")}
              onClick={() => next(1)}
            >
              ↓
            </Button>
          </>
        )}
        <Button
          className="ml-auto"
          variant="ghost"
          aria-expanded={showQuestions}
          onClick={() => setShowQuestions(!showQuestions)}
        >
          {t("sessionExperience.questions")} · {questions.length}
        </Button>
      </div>
      {showQuestions && (
        <nav
          aria-label={t("sessionExperience.questions")}
          className="max-h-36 space-y-1 overflow-auto border-b border-line-2 p-2"
        >
          {questions.map((question, index) => (
            <button
              key={question.index}
              className="block w-full truncate rounded-ctl px-2 py-1 text-left text-sub text-ink-muted hover:bg-pill"
              aria-label={t("sessionExperience.questionNumber", {
                number: index + 1,
              })}
              onClick={() => jump(question.index)}
            >
              {index + 1}. {question.title}
            </button>
          ))}
        </nav>
      )}
      <div
        ref={ref}
        className="min-h-0 flex-1 overflow-auto"
        style={{ minHeight: 180 }}
      >
        {(!query.isFetchedAfterMount || query.isPending) && !query.isError && (
          <p role="status" className="p-4">
            {t("common.loading")}
          </p>
        )}
        {query.isError && (
          <div role="alert" className="p-4 text-sub text-danger">
            {t(errorLabel)}{" "}
            <Button
              variant="ghost"
              onClick={() =>
                changed && onResearch ? onResearch() : void query.refetch()
              }
            >
              {t(changed ? "sessionExperience.research" : "common.retry")}
            </Button>
          </div>
        )}
        {verified && !rows?.length && (
          <p className="p-4 text-ink-muted">
            {t("sessionCenter.emptyContent")}
          </p>
        )}
        <div style={{ height: virtual.getTotalSize(), position: "relative" }}>
          {virtual.getVirtualItems().map((item) => (
            <div
              key={item.key}
              data-index={item.index}
              data-message-ordinal={item.index}
              tabIndex={-1}
              ref={virtual.measureElement}
              className={cn(
                "outline-none",
                item.index === focusedOrdinal &&
                  "rounded-ctl ring-2 ring-inset ring-brand/40",
              )}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${item.start}px)`,
              }}
            >
              <TranscriptPart
                message={rows![item.index]}
                reveal={item.index === focusedOrdinal}
                highlight={find.trim()}
              />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
