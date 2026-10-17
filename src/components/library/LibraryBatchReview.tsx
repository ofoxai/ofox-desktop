import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  libraryApi,
  type BatchResource,
  type LibraryBatchPlan,
  type LibraryKind,
  type LibraryBatchChange,
} from "@/lib/api/library";
import { Button } from "@/components/ui/button";
import { ListCard, ListRow } from "@/components/ui/list";
import { libraryError, toolLabel } from "./libraryText";
import { batchIssue } from "./libraryIssues";
import { LibraryIssueNotice } from "./LibraryIssueNotice";
function BatchChangeSummary({
  change,
}: {
  change: Omit<LibraryBatchChange, "action"> & {
    action?: "apply" | "withdraw";
  };
}) {
  const { t } = useTranslation();
  return (
    <div className="min-w-0 rounded-ctl bg-pill/50 px-3 py-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="min-w-0 break-words text-ctl font-medium">
          {change.name}
        </span>
        {change.action && (
          <span className="text-note text-ink-muted">
            {t(`library.batchSummary.${change.action}`)}
          </span>
        )}
        {change.tools && change.tools.length > 0 && (
          <span className="text-note text-ink-muted">
            {change.tools.map((tool) => toolLabel(tool, t)).join(" / ")}
          </span>
        )}
        {change.method && (
          <span className="text-note text-ink-muted">
            {t(`library.batchSummary.method.${change.method}`, {
              defaultValue: t("library.unknown"),
            })}
          </span>
        )}
        {change.status && (
          <span className="text-note text-ink-muted">
            {t(`library.status.${change.status}`, {
              defaultValue: t("library.unknown"),
            })}
          </span>
        )}
      </div>
      {change.changedFields.length > 0 && (
        <p className="mt-1 break-all text-note text-ink-muted">
          {t("library.batchSummary.fields", {
            fields: change.changedFields.join(", "),
          })}
        </p>
      )}
    </div>
  );
}
export function LibraryBatchReview({
  kind,
  resources,
  tools,
  action,
  epoch,
  initial,
  onBusyChange,
  onChanged,
}: {
  kind: LibraryKind;
  resources: BatchResource[];
  tools: string[];
  action: "apply" | "withdraw";
  epoch: number;
  initial?: LibraryBatchPlan;
  onBusyChange: (busy: boolean) => void;
  onChanged: (plan: LibraryBatchPlan) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [plan, setPlan] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  const running = useRef(false);
  const loaded = useRef(false);
  const cancelling = useRef(false);
  const [cancelPending, setCancelPending] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  const latestPlan = useRef(plan);
  latestPlan.current = plan;
  const executionStarted = useRef(false);
  const cancellationRequests = useRef(
    new Map<string, Promise<LibraryBatchPlan>>(),
  );
  const result = useRef<HTMLParagraphElement>(null);
  const focusResult = useRef(false);
  const reportedBusy = useRef(false);
  const callbacks = useRef({ onChanged, onBusyChange });
  callbacks.current = { onChanged, onBusyChange };
  function notifyBusy(value: boolean) {
    if (reportedBusy.current === value) return;
    reportedBusy.current = value;
    callbacks.current.onBusyChange(value);
  }
  function cancelOnce(operationId: string) {
    const existing = cancellationRequests.current.get(operationId);
    if (existing) return existing;
    const request = libraryApi
      .cancelBatch(operationId)
      .then((next) => {
        if (next.operationId !== operationId) throw new Error("library.failed");
        return next;
      })
      .catch((failure: unknown) => {
        cancellationRequests.current.delete(operationId);
        throw failure;
      });
    cancellationRequests.current.set(operationId, request);
    return request;
  }
  function disposeUntouched(candidate?: LibraryBatchPlan) {
    if (
      !executionStarted.current &&
      candidate &&
      ["previewed", "blocked"].includes(candidate.status)
    )
      void cancelOnce(candidate.operationId).catch(() => undefined);
  }
  useEffect(() => {
    alive.current = true;
    if (running.current || cancelling.current) notifyBusy(true);
    return () => {
      alive.current = false;
      notifyBusy(false);
      // React StrictMode replays setup in the same turn. Only a real close
      // disposes an untouched preview; executed journals remain recoverable.
      queueMicrotask(() => {
        if (!alive.current) disposeUntouched(latestPlan.current);
      });
    };
  }, []);
  useEffect(() => {
    if (focusResult.current && !busy && !cancelPending) {
      focusResult.current = false;
      result.current?.focus();
    }
  }, [plan, busy, cancelPending]);
  async function run(fn: () => Promise<LibraryBatchPlan>, changed = false) {
    if (running.current || cancelling.current) return;
    running.current = true;
    const hadPlan = !!latestPlan.current;
    if (changed) executionStarted.current = true;
    else setPreviewing(true);
    setBusy(true);
    notifyBusy(true);
    setError("");
    try {
      const next = await fn();
      latestPlan.current = next;
      if (!changed) executionStarted.current = false;
      if (alive.current) {
        setPlan(next);
        if (hadPlan) focusResult.current = true;
        if (changed) {
          focusResult.current = true;
          await callbacks.current.onChanged(next);
        }
      } else if (!changed) disposeUntouched(next);
    } catch (e) {
      if (alive.current) setError(libraryError(e, t));
    } finally {
      running.current = false;
      if (alive.current) {
        setPreviewing(false);
        setBusy(cancelling.current);
        notifyBusy(cancelling.current);
      }
    }
  }
  async function stop() {
    if (!plan || cancelling.current || previewing) return;
    cancelling.current = true;
    setCancelPending(true);
    notifyBusy(true);
    setError("");
    try {
      const next = await cancelOnce(plan.operationId);
      latestPlan.current = next;
      if (alive.current) {
        focusResult.current = true;
        setPlan(next);
        await callbacks.current.onChanged(next);
      }
    } catch (e) {
      if (alive.current) setError(libraryError(e, t));
    } finally {
      cancelling.current = false;
      if (alive.current) {
        setCancelPending(false);
        setBusy(running.current);
        notifyBusy(running.current);
      }
    }
  }
  useEffect(() => {
    if (!loaded.current) {
      loaded.current = true;
      if (!initial)
        void run(() =>
          libraryApi.previewBatch(kind, resources, tools, action, epoch),
        );
    }
  }, []);
  return (
    <div className="space-y-3">
      <p className="text-ctl text-ink-muted">
        {t("library.workbench.batchDescription")}
      </p>
      {error && (
        <p role="alert" className="text-ctl text-danger">
          {error}
        </p>
      )}
      {busy && (
        <p role="status" className="text-ctl text-ink-muted">
          {t("library.loading")}
        </p>
      )}
      {plan && (
        <>
          <p
            ref={result}
            tabIndex={-1}
            role="status"
            className="rounded-ctl text-ctl font-medium outline-none focus-visible:ring-2 focus-visible:ring-brand"
          >
            {t(`library.status.${plan.status}`, {
              defaultValue: t("library.unknown"),
            })}
          </p>
          {batchIssue(plan.status) && (
            <LibraryIssueNotice
              issue={batchIssue(plan.status)!}
              disabled={busy || cancelPending}
              onAction={
                batchIssue(plan.status)!.action.kind === "repreview" &&
                !resources.length
                  ? undefined
                  : (next) => {
                      if (next.kind === "reconcile")
                        void run(
                          () => libraryApi.reconcileBatch(plan.operationId),
                          true,
                        );
                      if (next.kind === "repreview")
                        void run(() =>
                          libraryApi.previewBatch(
                            kind,
                            resources,
                            tools,
                            action,
                            epoch,
                          ),
                        );
                    }
              }
            />
          )}
          {plan.metadataChanges && plan.metadataChanges.length > 0 && (
            <section className="space-y-2">
              <p className="text-ctl text-ink-muted">
                {t("library.batchSummary.metadataOnly")}
              </p>
              {plan.metadataChanges.map((change, index) => (
                <BatchChangeSummary
                  key={`${change.resourceId}:${index}`}
                  change={change}
                />
              ))}
            </section>
          )}
          {plan.groups.some((group) => group.status === "applied") && (
            <p className="text-note text-ink-muted">
              {t("library.workbench2.targetNext")}
            </p>
          )}
          <ListCard>
            {plan.groups.map((group) => (
              <ListRow
                key={group.groupId}
                className="flex-col items-stretch gap-2"
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="text-ctl font-medium">
                    {group.tools.map((tool) => toolLabel(tool, t)).join(" / ")}
                  </span>
                  <span className="text-note text-ink-muted">
                    {t(`library.status.${group.status}`, {
                      defaultValue: t("library.unknown"),
                    })}
                  </span>
                </div>
                <div className="space-y-2">
                  {(
                    group.changes ??
                    group.resourceIds.map((resourceId) => ({
                      resourceId,
                      name:
                        plan.resources.find(
                          (resource) => resource.resourceId === resourceId,
                        )?.name ?? t("library.workbench.removedResource"),
                      action: undefined,
                      changedFields: [],
                      method: undefined,
                    }))
                  ).map((change, index) => (
                    <BatchChangeSummary
                      key={`${change.resourceId}:${index}`}
                      change={change}
                    />
                  ))}
                </div>
                {group.reason && (
                  <p className="text-ctl text-warn">
                    {libraryError(group.reason, t)}
                  </p>
                )}
                {group.readers.length > 1 && (
                  <p className="text-note text-ink-muted">
                    {t("library.readers", {
                      tools: group.readers
                        .map((tool) => toolLabel(tool, t))
                        .join(", "),
                    })}
                  </p>
                )}
                <details className="text-note">
                  <summary className="cursor-pointer text-ink-muted">
                    {t("library.workbench.viewChanges")}
                  </summary>
                  <p className="break-all py-2 text-ink-muted">{group.path}</p>
                  {group.beforeManaged != null && (
                    <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded-ctl bg-pill p-2">
                      {group.beforeManaged}
                    </pre>
                  )}
                  {group.afterManaged != null && (
                    <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-ctl bg-pill p-2">
                      {group.afterManaged}
                    </pre>
                  )}
                </details>
              </ListRow>
            ))}
          </ListCard>
          <div className="flex flex-wrap gap-2">
            {plan.status === "previewed" && (
              <Button
                disabled={
                  busy ||
                  cancelPending ||
                  plan.groups.some((group) =>
                    ["conflict", "failed", "needsReview"].includes(
                      group.status,
                    ),
                  )
                }
                onClick={() =>
                  void run(() => libraryApi.runBatch(plan.operationId), true)
                }
              >
                {t("library.workbench.confirmApply")}
              </Button>
            )}
            {!["applied", "cancelled"].includes(plan.status) && (
              <Button
                variant="secondary"
                disabled={cancelPending || previewing}
                onClick={() => void stop()}
              >
                {t("library.workbench.stopRemaining")}
              </Button>
            )}
            {["partial", "needsReview"].includes(plan.status) &&
              resources.length > 0 && (
                <Button
                  variant="secondary"
                  disabled={busy || cancelPending}
                  onClick={() =>
                    void run(() =>
                      libraryApi.previewBatch(
                        kind,
                        resources,
                        tools,
                        action,
                        epoch,
                      ),
                    )
                  }
                >
                  {t("library.batchLifecycle.repreview")}
                </Button>
              )}
          </div>
          {["partial", "cancelled", "needsReview"].includes(plan.status) &&
            !resources.length && (
              <p className="text-note text-ink-muted">
                {t("library.batchLifecycle.returnToSelection")}
              </p>
            )}
          <details className="text-note text-ink-muted">
            <summary>{t("library.workbench.technical")}</summary>
            <p className="break-all">{plan.operationId}</p>
          </details>
        </>
      )}
    </div>
  );
}
