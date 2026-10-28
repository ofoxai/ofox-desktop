import { useEffect, useReducer } from "react";
import { ChevronRight, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AssistantThinking } from "./useAssistant";

/** Whole seconds as both captions say them: rounded, never less than one. */
const seconds = (ms: number) => Math.max(1, Math.round(ms / 1000));

/**
 * One box for every state, so nothing moves when the reasoning text arrives
 * or the reply finishes; only a reply with reasoning text can be opened.
 */
const BOX =
  "group/reasoning mb-3 rounded-ctl bg-card-2 px-3 py-2 text-ctl text-ink-ctl";

/**
 * What a reply says about its thinking. While the reply streams, `thinking`
 * (the host's `thinking_start` and `thinking_end`) gives a live "Thinking for
 * N seconds"; afterwards, and for a saved reply, it is "Thought for N
 * seconds" from `reasoningMs`, with the reasoning text behind it when there
 * is any. A reply with no reasoning shows nothing, and one saved before
 * durations were kept keeps its plain "Reasoning" disclosure.
 */
export function ReasoningBlock({
  reasoning,
  reasoningMs,
  thinking,
}: {
  reasoning?: string | null;
  reasoningMs?: number | null;
  thinking?: AssistantThinking | null;
}) {
  const { t } = useTranslation();
  const [, tick] = useReducer((count: number) => count + 1, 0);
  const since = thinking?.since ?? null;
  const live = since !== null;
  // Once a second while a span is under way; each render reads the clock.
  useEffect(() => {
    if (!live) return;
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  }, [live]);

  const spentMs = thinking
    ? thinking.doneMs + (since !== null ? Math.max(0, Date.now() - since) : 0)
    : (reasoningMs ?? null);
  const label = live
    ? t("assistant.thinking.live", { count: seconds(spentMs ?? 0) })
    : spentMs !== null
      ? t("assistant.thinking.done", { count: seconds(spentMs) })
      : t("assistant.reasoning");

  if (!live && spentMs === null && !reasoning) return null;
  // The spinner and the disclosure chevron sit beside the words, which stay
  // where they are as either comes and goes.
  const caption = (
    <span className="flex items-center gap-1.5">
      {live && (
        <Loader2
          aria-hidden
          className="h-3 w-3 flex-none animate-spin motion-reduce:animate-none"
        />
      )}
      <span className="min-w-0">{label}</span>
      {reasoning && (
        <ChevronRight
          aria-hidden
          className="h-3 w-3 flex-none opacity-70 transition-transform duration-150 ease-soft group-open/reasoning:rotate-90 motion-reduce:transition-none"
        />
      )}
    </span>
  );
  return (
    <>
      {/* The ticking caption is no live region: say once that it began. */}
      {live && (
        <span role="status" className="sr-only">
          {t("assistant.typing")}
        </span>
      )}
      {/* The reasoning text can follow a little after the first sign of
          thinking: until then there is only the caption. */}
      {reasoning ? (
        <details data-reasoning className={BOX}>
          <summary className="cursor-pointer list-none [&::-webkit-details-marker]:hidden">
            {caption}
          </summary>
          <div className="mt-2 whitespace-pre-wrap break-words">
            {reasoning}
          </div>
        </details>
      ) : (
        <div data-reasoning className={BOX}>
          {caption}
        </div>
      )}
    </>
  );
}
