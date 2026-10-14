import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { SessionMessage } from "@/types";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
export function TranscriptPart({ message }: { message: SessionMessage }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const long =
    message.content.length > 700 || message.content.split("\n").length > 9;
  const secondary = [
    "thinking",
    "tool_call",
    "tool",
    "command_output",
    "system",
  ].includes(message.role);
  const label = message.name || t(`sessionCenter.roles.${message.role}`);
  const body = (
    <>
      <div
        className={cn(
          "relative overflow-hidden",
          long && !expanded && "max-h-[176px]",
        )}
      >
        <pre className="whitespace-pre-wrap break-words rounded-lg border border-line-2 bg-card p-2 font-mono text-[11.5px] leading-normal">
          {message.content}
        </pre>
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
    <div className="p-2">
      {secondary ? (
        <details>
          <summary className="cursor-pointer truncate text-sub text-ink-muted">
            {label} · {message.content.slice(0, 90)}
          </summary>
          {body}
        </details>
      ) : (
        <>
          <span
            className={cn(
              "mb-1 inline-block rounded-pill px-2 py-px text-note",
              message.role === "user"
                ? "bg-brand-soft text-brand-ink"
                : message.role === "assistant"
                  ? "bg-ok-soft text-ok"
                  : "bg-pill font-mono text-ink-muted",
            )}
          >
            {label}
          </span>
          {body}
        </>
      )}
    </div>
  );
}
