import {
  useEffect,
  useId,
  useLayoutEffect,
  useReducer,
  useRef,
  useState,
} from "react";
import {
  Check,
  CircleDashed,
  CircleCheck,
  ChevronRight,
  Hourglass,
  Loader2,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AssistantActivity } from "./useAssistant";
import type { LocalPlanStep } from "./AssistantLocalConfig";
import { LocalTaskProgress } from "./AssistantLocalConfig";
import type { AssistantMessage, AssistantMcpStep } from "@/lib/api/assistant";
import { cn } from "@/lib/utils";
import { McpSteps } from "./McpSteps";

type OperationRecord = {
  id: string;
  activity: AssistantActivity;
  status: "running" | "completed" | "failed";
};

/**
 * Summarize only the retained calls, preserving their latest observed order.
 * A matching activity category is not evidence that a failed operation was
 * retried: it may have targeted an entirely different file or resource.
 */
function recentOperationGroups(records: OperationRecord[]) {
  const calls = new Map<string, OperationRecord>();
  for (const record of records) {
    calls.delete(record.id);
    calls.set(record.id, record);
  }
  const ordered = [...calls.values()];
  const groups = new Map<string, OperationRecord & { count: number }>();
  for (const record of ordered) {
    const key = `${record.activity}:${record.status}`;
    const count = (groups.get(key)?.count ?? 0) + 1;
    groups.delete(key);
    groups.set(key, { ...record, id: key, count });
  }
  return [...groups.values()];
}

/** Observed work only: a checklist and a quiet activity disclosure. */
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
  replyStarted = false,
  onFinish,
  status = "streaming",
  completedAt,
  pptxDelivered = false,
  previewPages,
  mcpSteps = [],
  errorCode,
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
  replyStarted?: boolean;
  onFinish?: () => void;
  status?: AssistantMessage["status"];
  completedAt?: number | null;
  pptxDelivered?: boolean;
  previewPages?: number;
  mcpSteps?: AssistantMcpStep[];
  errorCode?: string | null;
}) {
  const { t } = useTranslation();
  const live = status === "streaming";
  const surface = useRef<HTMLElement>(null);
  const summary = useRef<HTMLButtonElement>(null);
  const focused = useRef(false);
  const previousLive = useRef(live);
  const previousExecution = useRef(false);
  const detailId = useId();
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [summaryOpen, setSummaryOpen] = useState(false);
  const [, tick] = useReducer((n: number) => n + 1, 0);
  const operationGroups = recentOperationGroups(activities);
  const failures = operationGroups.filter((item) => item.status === "failed");
  const operationCount = operationGroups.reduce(
    (count, group) => count + group.count,
    0,
  );
  const hasExecution =
    plan.length > 0 ||
    (live &&
      Boolean(
        previewPages ||
        pptxDelivered ||
        activity === "preparingDocuments" ||
        approvals ||
        warning ||
        failures.length ||
        intent ||
        mcpSteps.length ||
        operationCount > 1,
      ));
  useEffect(() => {
    if (!live) return;
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  }, [live, hasExecution]);
  useLayoutEffect(() => {
    if (
      (previousLive.current && !live) ||
      (previousExecution.current && !hasExecution)
    ) {
      setDetailsOpen(false);
      setSummaryOpen(false);
      if (focused.current) {
        if (summary.current) summary.current.focus();
        else onFinish?.();
      }
    }
    previousLive.current = live;
    previousExecution.current = hasExecution;
  }, [live, hasExecution, onFinish]);
  useLayoutEffect(
    () => () => {
      if (surface.current?.contains(document.activeElement)) onFinish?.();
    },
    [onFinish],
  );

  const end = live ? Date.now() : completedAt;
  const elapsed =
    typeof end === "number" &&
    Number.isFinite(end) &&
    Number.isFinite(startedAt) &&
    end >= startedAt
      ? Math.floor((end - startedAt) / 1000)
      : null;
  const duration =
    elapsed === null
      ? null
      : elapsed >= 3600
        ? t("assistant.progress.durationHours", {
            hours: Math.floor(elapsed / 3600),
            minutes: Math.floor((elapsed % 3600) / 60),
          })
        : elapsed >= 60
          ? t("assistant.progress.durationMinutes", {
              minutes: Math.floor(elapsed / 60),
              seconds: elapsed % 60,
            })
          : t("assistant.progress.durationSeconds", { seconds: elapsed });
  const waiting = live && approvals > 0;
  const quiet =
    live && !waiting && Date.now() - (lastEventAt ?? startedAt) >= 45_000;
  const activeMcp = [...mcpSteps]
    .reverse()
    .find((step) => step.status === "running");
  const title = waiting
    ? t("assistant.progress.waiting")
    : retryAttempt
      ? t("assistant.retrying", { attempt: retryAttempt })
      : activity
        ? t(`assistant.activity.${activity}`)
        : activeMcp
          ? t("assistant.progress.mcpActivity", {
              name: activeMcp.server ?? activeMcp.tool,
            })
          : thinking
            ? t(
                hasExecution
                  ? "assistant.progress.thinking"
                  : "assistant.typing",
              )
            : t("assistant.progress.working");
  const mcpFailures = mcpSteps.filter(
    (step) => step.status === "failed" || step.status === "unavailable",
  );
  const detailOperations = operationGroups.filter(
    (item) => item.status !== "failed",
  );
  const detailMcp = mcpSteps.filter(
    (step) => step.status !== "failed" && step.status !== "unavailable",
  );
  const hasDetails = Boolean(
    intent || detailOperations.length || detailMcp.length,
  );
  const completed = plan.filter((step) => step.status === "completed").length;
  const terminalState =
    errorCode === "assistant_interrupted" ||
    (!completedAt && status === "stopped")
      ? "interrupted"
      : status === "stopped" || status === "cancelled"
        ? "stopped"
        : status === "error" || status === "failed"
          ? "failed"
          : "ended";
  const showDelivery =
    terminalState === "ended" && pptxDelivered && completed < plan.length;
  const terminalTitle = showDelivery
    ? t("assistant.progress.pptxDelivered")
    : t(`assistant.progress.${terminalState}`);
  const TerminalIcon =
    terminalState === "failed"
      ? X
      : (showDelivery || completed === plan.length) && terminalState === "ended"
        ? CircleCheck
        : CircleDashed;
  const count = t("assistant.progress.stepCount", {
    completed,
    total: plan.length,
  });
  const operations = (items: ReturnType<typeof recentOperationGroups>) => (
    <ul
      role="list"
      aria-label={t("assistant.progress.recent")}
      className="space-y-2"
    >
      {items.map((item) => {
        const active = live && item.status === "running";
        const state = waiting && active ? "waiting" : item.status;
        const stateLabel = t(`assistant.progress.states.${state}`);
        const Icon =
          item.status === "completed"
            ? Check
            : item.status === "failed"
              ? X
              : waiting || !live
                ? CircleDashed
                : Loader2;
        return (
          <li
            key={item.id}
            className="flex min-w-0 items-start gap-2 text-note text-ink-2"
          >
            <Icon
              role="img"
              aria-label={stateLabel}
              className={cn(
                "mt-0.5 h-3.5 w-3.5 shrink-0",
                item.status === "failed" && "text-danger",
                active && !waiting && "animate-spin motion-reduce:animate-none",
              )}
            />
            {item.status !== "completed" && (
              <span className="sr-only">{stateLabel}: </span>
            )}
            <span className="min-w-0 break-words">
              {t(`assistant.progress.activities.${item.activity}`)}
            </span>
            {item.count > 1 && (
              <span className="text-ink-muted">
                {t("assistant.progress.recentCalls", { count: item.count })}
              </span>
            )}
            {item.status !== "completed" && (
              <span
                aria-hidden
                className={cn(
                  "ml-auto shrink-0 text-ink-muted",
                  item.status === "failed" && "text-danger",
                )}
              >
                {stateLabel}
              </span>
            )}
          </li>
        );
      })}
    </ul>
  );
  const clock = (hasExecution || quiet) && duration && (
    <span className="shrink-0 tabular-nums text-ink-muted">
      {t("assistant.progress.elapsed", { duration })}
    </span>
  );
  const activityLine = (
    <>
      {waiting ? (
        <Hourglass aria-hidden className="h-4 w-4 shrink-0" />
      ) : (
        <Loader2
          aria-hidden
          className="h-4 w-4 shrink-0 animate-spin text-brand-ink motion-reduce:animate-none"
        />
      )}
      <span role="status" className="min-w-0 break-words">
        {title}
      </span>
      {clock && (
        <>
          <span aria-hidden>·</span>
          {clock}
        </>
      )}
      {hasExecution && hasDetails && (
        <ChevronRight
          aria-hidden
          className={cn(
            "h-3.5 w-3.5 shrink-0 transition-transform motion-reduce:transition-none",
            detailsOpen && "rotate-90",
          )}
        />
      )}
    </>
  );
  if (!hasExecution) {
    if (!live || (replyStarted && !activity && !retryAttempt && !thinking))
      return null;
    return (
      <div className="my-2 min-w-0 space-y-2 text-note text-ink-muted">
        <div className="flex flex-wrap items-center gap-2">{activityLine}</div>
        {quiet && <p>{t("assistant.progress.quietHint")}</p>}
      </div>
    );
  }
  return (
    <section
      ref={surface}
      aria-label={t("assistant.progress.label")}
      onFocusCapture={() => {
        focused.current = true;
      }}
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget))
          focused.current = false;
      }}
      className="my-3 min-w-0 space-y-3"
    >
      {live ? (
        <>
          {(pptxDelivered || previewPages) && (
            <p
              role="status"
              className="flex items-center gap-2 text-ctl text-ink"
            >
              <CircleCheck aria-hidden className="h-4 w-4" />
              {pptxDelivered && (
                <span>{t("assistant.progress.pptxDelivered")}</span>
              )}
              {pptxDelivered && previewPages && <span aria-hidden>·</span>}
              {previewPages && (
                <span>
                  {t("assistant.progress.slidesPreviewed", {
                    count: previewPages,
                  })}
                </span>
              )}
            </p>
          )}
          {plan.length > 0 &&
            (pptxDelivered || previewPages ? (
              <details className="text-note text-ink-muted">
                <summary className="cursor-pointer">
                  {t("assistant.progress.reportedPlan")}
                </summary>
                <LocalTaskProgress
                  intent={null}
                  warning={null}
                  plan={plan}
                  paused
                  showIntent={false}
                  collapsible={false}
                />
              </details>
            ) : (
              <LocalTaskProgress
                intent={null}
                warning={null}
                plan={plan}
                paused={waiting}
                showIntent={false}
              />
            ))}
          {hasDetails ? (
            <button
              type="button"
              aria-label={t("assistant.progress.details")}
              aria-expanded={detailsOpen}
              aria-controls={detailId}
              onClick={() => setDetailsOpen((open) => !open)}
              className="flex max-w-full flex-wrap items-center gap-x-2 gap-y-1 rounded-ctl py-1 text-left text-ctl text-ink-muted outline-none focus-visible:ring-2 focus-visible:ring-brand/60"
            >
              {activityLine}
            </button>
          ) : (
            <div className="flex max-w-full flex-wrap items-center gap-x-2 gap-y-1 py-1 text-ctl text-ink-muted">
              {activityLine}
            </div>
          )}
          {waiting && (
            <p className="text-note text-ink-muted">
              {t("assistant.progress.approvalHint")}
            </p>
          )}
          {quiet && (
            <p className="text-note text-ink-muted">
              {t("assistant.progress.quietHint")}
            </p>
          )}
          {warning && (
            <p role="status" className="break-words text-note text-warn">
              {t(`assistant.local.warnings.${warning}`)}
            </p>
          )}
          {failures.length > 0 && operations(failures)}
          {mcpFailures.length > 0 && <McpSteps steps={mcpFailures} />}
          {hasDetails && (
            <div
              id={detailId}
              hidden={!detailsOpen}
              className="space-y-3 border-l border-line pl-3 text-note text-ink-muted"
            >
              {detailOperations.length > 0 && operations(detailOperations)}
              {detailMcp.length > 0 && (
                <McpSteps
                  steps={detailMcp}
                  showCostHint={mcpFailures.length === 0}
                />
              )}
              {intent && (
                <p className="whitespace-pre-wrap break-words">{intent}</p>
              )}
            </div>
          )}
        </>
      ) : (
        <>
          <button
            ref={summary}
            type="button"
            aria-expanded={summaryOpen}
            aria-controls={detailId}
            onClick={() => setSummaryOpen((open) => !open)}
            className="flex max-w-full flex-wrap items-center gap-x-2 gap-y-1 rounded-ctl py-1 text-left text-ctl text-ink-muted outline-none focus-visible:ring-2 focus-visible:ring-brand/60"
          >
            <TerminalIcon aria-hidden className="h-4 w-4 shrink-0" />
            <span>{terminalTitle}</span>
            {!showDelivery && (
              <>
                <span aria-hidden>·</span>
                <span>{count}</span>
              </>
            )}
            {clock && (
              <>
                <span aria-hidden>·</span>
                {clock}
              </>
            )}
            <ChevronRight
              aria-hidden
              className={cn(
                "h-3.5 w-3.5 shrink-0 transition-transform motion-reduce:transition-none",
                summaryOpen && "rotate-90",
              )}
            />
          </button>
          <div id={detailId} hidden={!summaryOpen}>
            {showDelivery && (
              <p className="mb-1 text-note text-ink-muted">
                {t("assistant.progress.reportedPlan")}
              </p>
            )}
            <LocalTaskProgress
              intent={null}
              warning={null}
              plan={plan}
              paused
              terminal
              collapsible={false}
              showIntent={false}
            />
            {detailMcp.length > 0 && (
              <McpSteps
                steps={detailMcp}
                showCostHint={mcpFailures.length === 0}
              />
            )}
          </div>
          {mcpFailures.length > 0 && <McpSteps steps={mcpFailures} />}
        </>
      )}
    </section>
  );
}
