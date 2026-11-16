import { useEffect, useLayoutEffect, useReducer, useRef } from "react";
import {
  Check,
  CircleDashed,
  Clock3,
  Hourglass,
  Loader2,
  RotateCw,
  Sparkles,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AssistantActivity } from "./useAssistant";
import type { LocalPlanStep } from "./AssistantLocalConfig";
import { LocalTaskProgress } from "./AssistantLocalConfig";

type OperationRecord = {
  id: string;
  activity: AssistantActivity;
  status: "running" | "completed" | "failed";
};

type ShownOperation = Omit<OperationRecord, "status"> & {
  status: OperationRecord["status"] | "retried";
};

/**
 * Summarize only the retained calls, preserving their latest observed order.
 * A failed step the model went on to do again reads as retried: the run is
 * correcting itself, and a red failure there invites stopping it.
 */
function recentOperationGroups(records: OperationRecord[]) {
  const calls = new Map<string, OperationRecord>();
  for (const record of records) {
    calls.delete(record.id);
    calls.set(record.id, record);
  }
  const ordered = [...calls.values()];
  const shown: ShownOperation[] = ordered.map((record, index) =>
    record.status === "failed" &&
    ordered.slice(index + 1).some((later) => later.activity === record.activity)
      ? { ...record, status: "retried" }
      : record,
  );
  const groups = new Map<string, ShownOperation & { count: number }>();
  for (const record of shown) {
    const key = `${record.activity}:${record.status}`;
    const count = (groups.get(key)?.count ?? 0) + 1;
    groups.delete(key);
    groups.set(key, { ...record, id: key, count });
  }
  return [...groups.values()];
}

/** Live, observed work only: no estimated percentage or invented task steps. */
export function AssistantRunProgress({
  startedAt,
  lastEventAt,
  activity,
  activities = [],
  intent,
  plan,
  warning,
  approvals = 0,
  retryAttempt,
  thinking = false,
  onFinish,
}: {
  startedAt: number;
  lastEventAt?: number;
  activity: AssistantActivity | null;
  activities?: OperationRecord[];
  intent: string | null;
  plan: LocalPlanStep[];
  warning: string | null;
  approvals?: number;
  retryAttempt?: 1 | 2 | null;
  thinking?: boolean;
  onFinish?: () => void;
}) {
  const { t } = useTranslation();
  const surface = useRef<HTMLElement>(null);
  useLayoutEffect(
    () => () => {
      if (surface.current?.contains(document.activeElement)) onFinish?.();
    },
    [onFinish],
  );
  const [, tick] = useReducer((n: number) => n + 1, 0);
  useEffect(() => {
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  }, []);
  const elapsed = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
  const duration = `${Math.floor(elapsed / 60)
    .toString()
    .padStart(2, "0")}:${(elapsed % 60).toString().padStart(2, "0")}`;
  const waiting = approvals > 0;
  const quiet = !waiting && Date.now() - (lastEventAt ?? startedAt) >= 45_000;
  const title = waiting
    ? t("assistant.progress.waiting")
    : retryAttempt
      ? t("assistant.retrying", { attempt: retryAttempt })
      : activity
        ? t(`assistant.activity.${activity}`)
        : thinking
          ? t("assistant.progress.thinking")
          : t("assistant.progress.working");
  const hint = waiting
    ? t("assistant.progress.approvalHint")
    : quiet
      ? t("assistant.progress.quietHint")
      : activity === "makingDocument"
        ? t("assistant.progress.documentHint")
        : t("assistant.progress.liveHint");
  const Icon = waiting ? Hourglass : activity ? Loader2 : Sparkles;
  // A model checklist is independent of tool responses. Keep actual running
  // calls and failures visible alongside it; returned calls are not task checks.
  const operationGroups = recentOperationGroups(activities);
  const visibleActivities =
    plan.length > 0
      ? operationGroups.filter((item) => item.status !== "completed")
      : operationGroups;
  return (
    <section
      ref={surface}
      aria-label={t("assistant.progress.label")}
      className="mb-4 overflow-hidden rounded-list border border-line bg-card"
    >
      <div className="flex items-start gap-3 px-4 pb-3 pt-4">
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-ctl bg-brand-soft text-brand-ink">
          <Icon
            aria-hidden
            className={`h-4 w-4 ${!waiting && activity ? "animate-spin motion-reduce:animate-none" : ""}`}
          />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
            <p
              role="status"
              className="break-words text-ui font-medium text-ink"
            >
              {title}
            </p>
            <span className="flex shrink-0 items-center gap-1 text-note tabular-nums text-ink-muted">
              <Clock3 aria-hidden className="h-3 w-3" />
              {t("assistant.progress.elapsed", { duration })}
            </span>
          </div>
          <p className="mt-1 text-note leading-relaxed text-ink-muted">
            {hint}
          </p>
        </div>
      </div>
      {visibleActivities.length > 0 && (
        <div className="border-t border-line-2 px-4 py-3">
          <p className="mb-2 text-note font-medium text-ink-2">
            {t("assistant.progress.recent")}
          </p>
          <p className="mb-2 text-note leading-relaxed text-ink-muted">
            {t("assistant.progress.operationHint")}
          </p>
          <ul
            role="list"
            aria-label={t("assistant.progress.recent")}
            className="space-y-2"
          >
            {visibleActivities.map((item) => {
              const active = item.status === "running";
              const ActivityIcon =
                item.status === "failed"
                  ? X
                  : item.status === "retried"
                    ? RotateCw
                    : waiting
                      ? CircleDashed
                      : Loader2;
              const state = waiting && active ? "waiting" : item.status;
              const stateLabel = t(`assistant.progress.states.${state}`);
              return (
                <li
                  key={item.id}
                  className="flex min-w-0 items-start gap-2 text-note text-ink-2"
                >
                  {item.status === "completed" ? (
                    <span
                      aria-hidden
                      className="ml-1 mr-1 mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-ink-muted"
                    />
                  ) : (
                    <ActivityIcon
                      aria-hidden
                      className={`mt-0.5 h-3.5 w-3.5 shrink-0 ${active && !waiting ? "animate-spin motion-reduce:animate-none" : ""}`}
                    />
                  )}
                  {item.status !== "completed" && (
                    <span className="sr-only">{stateLabel}: </span>
                  )}
                  <span className="min-w-0 break-words">
                    {t(`assistant.progress.activities.${item.activity}`)}
                  </span>
                  {item.count > 1 && (
                    <span className="text-ink-muted">
                      {t("assistant.progress.recentCalls", {
                        count: item.count,
                      })}
                    </span>
                  )}
                  <span
                    aria-hidden={item.status === "completed" ? undefined : true}
                    className={`ml-auto shrink-0 ${item.status === "failed" ? "text-danger" : "text-ink-muted"}`}
                  >
                    {item.status === "completed" ? (
                      <Check
                        role="img"
                        aria-hidden={false}
                        aria-label={stateLabel}
                        className="h-3.5 w-3.5"
                      />
                    ) : (
                      stateLabel
                    )}
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      )}
      {(plan.length > 0 || warning) && (
        <div className="px-4 pb-3">
          <LocalTaskProgress
            intent={null}
            warning={warning}
            plan={plan}
            paused={waiting}
            showIntent={false}
          />
        </div>
      )}
      {intent && (
        <details className="border-t border-line-2 px-4 py-2 text-note text-ink-muted">
          <summary className="w-fit cursor-pointer rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand">
            {t("assistant.progress.details")}
          </summary>
          <p className="mt-2 whitespace-pre-wrap break-words">{intent}</p>
        </details>
      )}
    </section>
  );
}
