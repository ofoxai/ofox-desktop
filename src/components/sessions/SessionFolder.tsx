import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Checkbox } from "@/components/ui/checkbox";
import { ListCard, ListHead, ListFold } from "@/components/ui/list";
import { Button } from "@/components/ui/button";
import type { ManagedSession } from "@/lib/api/sessions";
import { getBaseName } from "./utils";
import { canSelect, descendants, MAX_DEPTH, type SessionKids } from "./model";
import { SessionRow } from "./SessionRow";
import { SessionSubagents } from "./SessionSubagents";
export function SelectionCheckbox({
  sessions,
  selected,
  busy,
  label,
  onChange,
}: {
  sessions: ManagedSession[];
  selected: Set<string>;
  busy: boolean;
  label: string;
  onChange: (v: boolean) => void;
}) {
  const eligible = sessions.filter(canSelect);
  const count = eligible.filter((s) => selected.has(s.sessionKey)).length;
  return (
    <Checkbox
      aria-label={label}
      disabled={busy || !eligible.length}
      checked={
        count === 0 ? false : count === eligible.length ? true : "indeterminate"
      }
      onCheckedChange={(v) => onChange(v === true)}
    />
  );
}
export function SessionFolder({
  path,
  sessions,
  kids,
  subsOpen,
  toggleSubs,
  searching,
  open,
  toggle,
  selected,
  select,
  busy,
  expanded,
  setExpanded,
  trash,
  located,
  onLocated,
  onRead,
  onAnalyze,
}: {
  path: string;
  /** The folder's sessions on top; those under them are in `kids`. */
  sessions: ManagedSession[];
  kids: SessionKids;
  /** Sessions whose subagents are unfolded, by session key. */
  subsOpen: Set<string>;
  toggleSubs: (sessionKey: string) => void;
  /** A search unfolds every session's subagents. */
  searching: boolean;
  open: boolean;
  toggle: () => void;
  selected: Set<string>;
  select: (items: ManagedSession[], v: boolean) => void;
  busy: boolean;
  expanded: string | null;
  setExpanded: (s: string | null) => void;
  trash: (items: ManagedSession[]) => void;
  located?: { key: string; request: number | null };
  onLocated?: (request: number) => void;
  onRead?: (session: ManagedSession) => void;
  onAnalyze?: (sessionKey: string) => void;
}) {
  const { t } = useTranslation();
  const [all, setAll] = useState(false);
  const visible = sessions.slice(0, all ? undefined : 100);
  // the sessions under those on top are the folder's too
  const members = sessions.flatMap((s) => descendants(s, kids));
  const target = sessions.find((session) =>
    descendants(session, kids).some((s) => s.sessionKey === located?.key),
  );
  if (target && !visible.includes(target)) visible.push(target);
  const row = (s: ManagedSession, depth: number): ReactNode => {
    const under = depth < MAX_DEPTH ? (kids.get(s.sessionKey) ?? []) : [];
    return (
      <SessionRow
        key={s.sessionKey}
        session={s}
        expanded={expanded === s.sessionKey}
        toggle={() =>
          setExpanded(expanded === s.sessionKey ? null : s.sessionKey)
        }
        selected={selected.has(s.sessionKey)}
        select={(v) => select([s], v)}
        busy={busy}
        trash={() => trash([s])}
        focusRequest={located?.key === s.sessionKey ? located.request : null}
        onLocated={onLocated}
        onRead={onRead}
        onAnalyze={onAnalyze}
      >
        {under.length > 0 && (
          <SessionSubagents
            count={under.length}
            nested={depth > 0}
            open={subsOpen.has(s.sessionKey) || searching}
            onOpenChange={() => toggleSubs(s.sessionKey)}
          >
            {under.map((k) => row(k, depth + 1))}
          </SessionSubagents>
        )}
      </SessionRow>
    );
  };
  return (
    <ListCard>
      <ListHead>
        <SelectionCheckbox
          sessions={members}
          selected={selected}
          busy={busy}
          label={t("sessionCenter.selectFolder", {
            project: path || t("sessionCenter.noProject"),
          })}
          onChange={(v) => select(members, v)}
        />
        <ListFold
          open={open}
          onOpenChange={toggle}
          className="max-w-[45%] font-medium text-ink-2"
        >
          <span className="truncate">
            {getBaseName(path) || t("sessionCenter.noProject")}
          </span>
        </ListFold>
        <span
          className="hidden min-w-0 flex-1 truncate font-mono text-note text-ink-faint min-[560px]:block"
          title={path}
        >
          {path}
        </span>
        <span className="ml-auto text-note text-ink-faint">
          {members.length}
        </span>
      </ListHead>
      {open && (
        <>
          {visible.map((s) => row(s, 0))}
          {!all && sessions.length > visible.length && (
            <Button variant="ghost" onClick={() => setAll(true)}>
              {t("sessionCenter.showRemaining", {
                count: sessions.length - visible.length,
              })}
            </Button>
          )}
        </>
      )}
    </ListCard>
  );
}
