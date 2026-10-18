import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ListFold } from "@/components/ui/list";
import type { SessionSubagent } from "@/lib/api/sessions";
import { fmtAgo } from "@/lib/format";
/**
 * The fold under a session with the subagents it started: Codex threads drawn
 * as sessions of their own, then those its agent keeps inside it (Magpie
 * sessions client.js `subagentsBox`, MIT).
 */
export function SessionSubagents({
  count,
  nested,
  open,
  onOpenChange,
  children,
}: {
  count: number;
  /** Drawn inside another session's fold, which sets it in already. */
  nested: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <div className={nested ? "pb-1.5 pl-6 pr-3" : "pb-1.5 pl-[30px] pr-3"}>
      <ListFold open={open} onOpenChange={onOpenChange} className="text-note">
        {t("sessionCenter.subagents", { count })}
      </ListFold>
      {open && (
        <div className="mt-0.5 flex flex-col divide-y divide-line-2 overflow-hidden rounded-[9px] border border-line-2 bg-card">
          {children}
        </div>
      )}
    </div>
  );
}
/** One subagent its agent keeps inside the session: what it was asked, the
 * agent it ran as; a click opens what was said in it (Magpie `subRow`). */
export function SubagentRow({
  subagent: s,
  depth,
  onOpen,
}: {
  subagent: SessionSubagent;
  depth: number;
  onOpen: () => void;
}) {
  const { i18n } = useTranslation();
  return (
    <button
      type="button"
      onClick={onOpen}
      className="flex min-h-[38px] w-full flex-col items-start justify-center py-1.5 pr-3 text-left hover:bg-pill"
      style={{ paddingLeft: `calc(11px + ${depth} * 16px)` }}
    >
      <span className="block w-full truncate text-ui text-ink">
        {s.title || s.kind || s.id}
      </span>
      <span className="block w-full truncate text-note text-ink-faint">
        {[
          s.title ? s.kind : "",
          fmtAgo(s.lastActiveAt ?? s.startedAt, i18n.language),
          s.id.slice(0, 8),
        ]
          .filter(Boolean)
          .join(" · ")}
      </span>
    </button>
  );
}
