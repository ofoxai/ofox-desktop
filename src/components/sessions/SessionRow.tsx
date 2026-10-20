import { useEffect, useRef, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import type { ManagedSession } from "@/lib/api/sessions";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { SessionDetails } from "./SessionDetails";
import { SessionUsageSummary } from "./SessionUsageSummary";
import { canSelect, sessionSubline } from "./model";
import { useSessionResume } from "@/hooks/useSessionActions";
import { ResumeFeedback } from "./ResumeFeedback";
export function SessionRow({
  session: s,
  expanded,
  toggle,
  selected,
  select,
  busy,
  trash,
  focusRequest,
  onLocated,
  onRead,
  onAnalyze,
  children,
}: {
  session: ManagedSession;
  expanded: boolean;
  toggle: () => void;
  selected: boolean;
  select: (v: boolean) => void;
  busy: boolean;
  trash: () => void;
  focusRequest?: number | null;
  onLocated?: (request: number) => void;
  onRead?: (session: ManagedSession) => void;
  onAnalyze?: (sessionKey: string) => void;
  /** What is drawn under the row: its subagents. */
  children?: ReactNode;
}) {
  const { t, i18n } = useTranslation();
  const titleButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (focusRequest == null) return;
    titleButton.current?.scrollIntoView({ block: "center" });
    titleButton.current?.focus({ preventScroll: true });
    onLocated?.(focusRequest);
  }, [focusRequest, onLocated]);
  const resume = useSessionResume(s.sessionKey);
  const title = s.title || t("sessionCenter.untitled");
  return (
    <div>
      <div
        className="flex min-h-[44px] items-center gap-2 px-3 py-1.5 hover:bg-pill"
        onClick={(e) => {
          if (e.target === e.currentTarget) toggle();
        }}
      >
        <Checkbox
          checked={selected}
          disabled={busy || !canSelect(s)}
          onCheckedChange={(v) => select(v === true)}
          aria-label={t("sessionCenter.selectSession", { title })}
        />
        <ToolBadge toolId={s.providerId} size={22} />
        <button
          ref={titleButton}
          className="min-w-0 flex-1 text-left"
          aria-expanded={expanded}
          onClick={toggle}
        >
          <span className="block truncate text-ui font-medium text-ink">
            {title}
          </span>
          <span className="block truncate text-note text-ink-faint">
            {sessionSubline(s, i18n.language, t)}
          </span>
        </button>
        <SessionUsageSummary usage={s.usage} display={s.usageDisplay} />
        {s.capabilities.canResume && (
          <Button
            variant="secondary"
            disabled={resume.busy || busy}
            onClick={() => void resume.launch()}
          >
            {t("sessionExperience.resumeTerminal")}
          </Button>
        )}
        <Button
          variant="dangerGhost"
          size="icon"
          disabled={busy || !canSelect(s)}
          onClick={trash}
          aria-label={t("sessionCenter.moveToTrash")}
        >
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      </div>
      <ResumeFeedback command={resume.command} feedback={resume.feedback} />
      {expanded && (
        <SessionDetails session={s} onRead={onRead} onAnalyze={onAnalyze} />
      )}
      {children}
    </div>
  );
}
