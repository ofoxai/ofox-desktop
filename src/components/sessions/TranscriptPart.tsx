import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { SessionMessage } from "@/types";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { SessionMarkdown } from "./SessionMarkdown";
import { cn } from "@/lib/utils";
import { highlightText } from "./utils";
export function TranscriptPart({
  message,
  reveal = false,
  highlight = "",
}: {
  message: SessionMessage;
  reveal?: boolean;
  highlight?: string;
}) {
  const { t, i18n } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const details = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    if (reveal && details.current) details.current.open = true;
    if (reveal) setExpanded(true);
  }, [reveal]);
  const long = message.content.length > 20000;
  const secondary = [
    "thinking",
    "tool_call",
    "tool",
    "command_output",
    "command",
    "system",
  ].includes(message.role);
  const label =
    message.name ||
    t(`sessionCenter.roles.${message.role}`, { defaultValue: message.role });
  const body = (
    <>
      <div
        className={cn(
          "relative overflow-hidden",
          long && !expanded && "max-h-[176px]",
        )}
      >
        {secondary ? (
          <pre className="overflow-x-auto whitespace-pre-wrap break-words rounded-lg bg-canvas p-3 font-mono text-[12px] leading-relaxed">
            {highlightText(message.content, highlight)}
          </pre>
        ) : (
          <SessionMarkdown content={message.content} highlight={highlight} />
        )}
        {long && !expanded && (
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-9 bg-gradient-to-t from-card to-transparent" />
        )}
      </div>
      {long && (
        <Button variant="ghost" onClick={() => setExpanded(!expanded)}>
          {t(
            expanded
              ? "sessionCenter.collapseText"
              : "sessionCenter.expandText",
          )}
        </Button>
      )}
    </>
  );
  return (
    <article className="border-b border-line-2 px-4 py-3 last:border-b-0">
      <div className="mb-1 flex items-center gap-2 text-note text-ink-muted">
        <span
          className={cn(
            "rounded-pill px-2 py-px",
            message.role === "user"
              ? "bg-brand-soft text-brand-ink"
              : message.role === "assistant"
                ? "bg-ok-soft text-ok"
                : "bg-pill",
          )}
        >
          {label}
        </span>
        {message.ts != null &&
          Number.isFinite(message.ts) &&
          !Number.isNaN(new Date(message.ts).getTime()) && (
            <time dateTime={new Date(message.ts).toISOString()}>
              {new Date(message.ts).toLocaleString(i18n.language)}
            </time>
          )}
        <CopyButton
          className="ml-auto"
          text={message.content}
          aria-label={t("sessionExperience.copyMessage")}
        />
      </div>
      {secondary ? (
        <details ref={details}>
          <summary className="cursor-pointer truncate text-sub text-ink-muted">
            {label} · {message.content.slice(0, 90)}
          </summary>
          {body}
        </details>
      ) : (
        <>{body}</>
      )}
    </article>
  );
}
