import { useEffect, useReducer } from "react";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AssistantThinking } from "./useAssistant";

/** Whole seconds as the reply says them: never less than one. */
const doneSeconds = (ms: number) => Math.max(1, Math.round(ms / 1000));

/**
 * What a reply says about its thinking. While the reply streams, `thinking`
 * (the host's `thinking_start` and the first text after it) gives a live
 * "Thinking for N seconds"; afterwards, and for a saved reply, it is
 * "Thought for N seconds" from `reasoningMs`, with the reasoning text behind
 * it when there is any. A reply with no reasoning shows nothing, and one
 * saved before durations were kept keeps its plain "Reasoning" disclosure.
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
    ? t("assistant.thinking.live", {
        count: Math.floor((spentMs ?? 0) / 1000),
      })
    : spentMs !== null
      ? t("assistant.thinking.done", { count: doneSeconds(spentMs) })
      : t("assistant.reasoning");

  if (!live && spentMs === null && !reasoning) return null;
  const caption = (
    <>
      {live && (
        <Loader2
          aria-hidden
          className="mr-1.5 inline h-3 w-3 animate-spin align-[-1px] motion-reduce:animate-none"
        />
      )}
      {label}
    </>
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
        <details className="mb-3 rounded-ctl bg-card-2 px-3 py-2 text-ctl text-ink-muted">
          <summary className="cursor-pointer">{caption}</summary>
          <div className="mt-2 whitespace-pre-wrap break-words">
            {reasoning}
          </div>
        </details>
      ) : (
        <p className="mb-3 text-ctl text-ink-muted">{caption}</p>
      )}
    </>
  );
}
