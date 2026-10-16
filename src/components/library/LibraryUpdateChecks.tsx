import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  libraryApi,
  type SkillDefinition,
  type SkillUpdateCheck,
} from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { ListCard, ListRow } from "@/components/ui/list";
export interface LibraryUpdateChecksProps {
  skills: SkillDefinition[];
  epoch: number;
  onOpen: (skill: SkillDefinition) => void;
  onBusyChange: (busy: boolean) => void;
}
type CheckState =
  | "queued"
  | "checking"
  | "updateAvailable"
  | "upToDate"
  | "notTracked"
  | "sourceMissing"
  | "targetModified"
  | "sourceModified"
  | "failed"
  | "cancelled";
interface CheckRow {
  skill: SkillDefinition;
  state: CheckState;
}
export function LibraryUpdateChecks({
  skills,
  epoch,
  onOpen,
  onBusyChange,
}: LibraryUpdateChecksProps) {
  const { t } = useTranslation();
  const [rows, setRows] = useState<CheckRow[]>(() =>
    skills.map((skill) => ({ skill, state: "queued" })),
  );
  const [working, setWorking] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [cleanupFailed, setCleanupFailed] = useState(false);
  const running = useRef(false);
  const alive = useRef(true);
  const cancelled = useRef(false);
  const busyReported = useRef(false);
  const callbacks = useRef({ onOpen, onBusyChange });
  callbacks.current = { onOpen, onBusyChange };
  const tickets = useRef(new Set<string>());
  const discarding = useRef(new Map<string, Promise<boolean>>());
  const scope = JSON.stringify([
    epoch,
    skills.map((skill) => [skill.id, skill.revision, skill.artifactHash]),
  ]);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  function notifyBusy(value: boolean) {
    if (busyReported.current === value) return;
    busyReported.current = value;
    callbacks.current.onBusyChange(value);
  }
  function discard(ticketId: string): Promise<boolean> {
    const existing = discarding.current.get(ticketId);
    if (existing) return existing;
    const pending = libraryApi
      .discardSkillUpdate(ticketId)
      .then(
        () => {
          tickets.current.delete(ticketId);
          return true;
        },
        () => false,
      )
      .finally(() => {
        discarding.current.delete(ticketId);
      });
    discarding.current.set(ticketId, pending);
    return pending;
  }
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      cancelled.current = true;
      notifyBusy(false);
      for (const ticket of tickets.current) void discard(ticket);
    };
  }, []);
  function stop() {
    cancelled.current = true;
    setStopping(true);
    notifyBusy(false);
    setRows((previous) =>
      previous.map((row) =>
        ["queued", "checking"].includes(row.state)
          ? { ...row, state: "cancelled" }
          : row,
      ),
    );
    for (const ticket of tickets.current) void discard(ticket);
  }
  function update(id: string, state: CheckState, expectedScope: string) {
    if (
      alive.current &&
      !cancelled.current &&
      currentScope.current === expectedScope
    )
      setRows((previous) =>
        previous.map((row) => (row.skill.id === id ? { ...row, state } : row)),
      );
  }
  function checkedState(
    check: SkillUpdateCheck,
    skill: SkillDefinition,
  ): CheckState {
    if (
      !check ||
      check.resourceId !== skill.id ||
      check.revision !== skill.revision ||
      check.epoch !== epoch ||
      !["updateAvailable", "upToDate", "notTracked", "sourceMissing"].includes(
        check.state,
      )
    )
      throw new Error();
    if (check.targetConflicts?.length) return "targetModified";
    if (check.sourceVerified === false) return "sourceModified";
    return check.state;
  }
  async function start() {
    if (running.current || !skills.length) return;
    running.current = true;
    cancelled.current = false;
    setWorking(true);
    setStopping(false);
    setCleanupFailed(false);
    setRows(skills.map((skill) => ({ skill, state: "queued" })));
    notifyBusy(true);
    const expectedScope = scope;
    try {
      for (const skill of skills) {
        if (
          cancelled.current ||
          !alive.current ||
          currentScope.current !== expectedScope
        )
          break;
        if (skill.revision === undefined) {
          update(skill.id, "notTracked", expectedScope);
          continue;
        }
        update(skill.id, "checking", expectedScope);
        let ticket: string | undefined;
        let state: CheckState = "failed";
        let released = true;
        try {
          const check = await libraryApi.checkSkillUpdate(
            skill.id,
            skill.revision,
            epoch,
          );
          if (
            typeof check?.candidate?.ticketId === "string" &&
            check.candidate.ticketId
          ) {
            ticket = check.candidate.ticketId;
            tickets.current.add(ticket);
          }
          state = checkedState(check, skill);
        } catch {
          state = "failed";
        } finally {
          // A detail visit performs a fresh check. Do not retain one ticket per row.
          if (ticket) released = await discard(ticket);
        }
        update(skill.id, state, expectedScope);
        if (!released) {
          if (alive.current) setCleanupFailed(true);
          break;
        }
      }
    } finally {
      running.current = false;
      notifyBusy(false);
      if (alive.current) {
        setWorking(false);
        setStopping(false);
      }
    }
  }
  const completed = rows.filter(
    (row) => !["queued", "checking", "cancelled"].includes(row.state),
  ).length;
  return (
    <div className="space-y-3">
      <p className="text-ctl text-ink-muted">
        {t("library.updateChecks.description")}
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="secondary"
          disabled={working || !skills.length}
          onClick={() => void start()}
        >
          {t("library.updateChecks.start")}
        </Button>
        {working && (
          <Button
            type="button"
            variant="ghost"
            disabled={stopping}
            onClick={stop}
          >
            {t(
              stopping
                ? "library.updateChecks.stopping"
                : "library.updateChecks.stop",
            )}
          </Button>
        )}
        <p role="status" className="text-note text-ink-muted">
          {t("library.updateChecks.progress", {
            completed,
            total: rows.length,
          })}
        </p>
      </div>
      {cleanupFailed && (
        <p role="alert" className="text-ctl text-warn">
          {t("library.updateChecks.cleanupFailed")}
        </p>
      )}
      <ListCard>
        {rows.map(({ skill, state }) => (
          <ListRow key={skill.id} className="flex-wrap gap-y-2">
            <div className="min-w-0 flex-1 basis-48">
              <p className="truncate text-ui font-medium" title={skill.name}>
                {skill.name}
              </p>
              <p className="text-note text-ink-muted">
                {t(`library.updateChecks.state.${state}`)}
              </p>
            </div>
            {!["queued", "checking", "cancelled"].includes(state) && (
              <Button
                type="button"
                variant="ghost"
                disabled={working}
                onClick={() => callbacks.current.onOpen(skill)}
              >
                {t("library.updateChecks.open")}
              </Button>
            )}
          </ListRow>
        ))}
      </ListCard>
    </div>
  );
}
