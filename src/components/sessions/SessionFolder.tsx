import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Checkbox } from "@/components/ui/checkbox";
import { ListCard, ListHead, ListFold } from "@/components/ui/list";
import { Button } from "@/components/ui/button";
import type { ManagedSession } from "@/lib/api/sessions";
import { getBaseName } from "./utils";
import { canSelect } from "./model";
import { SessionRow } from "./SessionRow";
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
  sessions: ManagedSession[];
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
  const target = sessions.find(
    (session) => session.sessionKey === located?.key,
  );
  if (target && !visible.includes(target)) visible.push(target);
  return (
    <ListCard>
      <ListHead>
        <SelectionCheckbox
          sessions={sessions}
          selected={selected}
          busy={busy}
          label={t("sessionCenter.selectFolder", {
            project: path || t("sessionCenter.noProject"),
          })}
          onChange={(v) => select(sessions, v)}
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
          {sessions.length}
        </span>
      </ListHead>
      {open && (
        <>
          {visible.map((s) => (
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
              focusRequest={
                located?.key === s.sessionKey ? located.request : null
              }
              onLocated={onLocated}
              onRead={onRead}
              onAnalyze={onAnalyze}
            />
          ))}
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
