import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { InstallCatalogEntry } from "@/config/installCatalog";
import {
  catalogLifecycleApi,
  type CatalogAction,
  type CatalogCopyState,
  type CatalogPlan,
  type CatalogProgress,
  type CatalogResult,
} from "@/lib/api/catalogLifecycle";
import { useToolJob } from "@/hooks/useToolJob";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
const ACTIONS = ["install", "update", "uninstall"] as const;
const STAGES = ["verify", "execute", "cleanup", "inspect"] as const;
export function CatalogLifecycleDialog({
  entry,
  onBusyChange,
  onChanged,
}: {
  entry: InstallCatalogEntry;
  onBusyChange: (busy: boolean) => void;
  onChanged: (operationId?: string) => Promise<void>;
}) {
  const { t } = useTranslation();
  const { subscribe } = useToolJob();
  const [state, setState] = useState<CatalogCopyState | null>(null);
  const [plan, setPlan] = useState<CatalogPlan | null>(null);
  const [result, setResult] = useState<CatalogResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [refreshError, setRefreshError] = useState(false);
  const [refreshBusy, setRefreshBusy] = useState(false);
  const [stage, setStage] = useState<CatalogProgress["stage"]>("verify");
  const mounted = useRef(true);
  const generation = useRef(0);
  const running = useRef(false);
  const refreshing = useRef(false);
  const cancelled = useRef(false);
  const operation = useRef<string>();
  const stop = useRef<() => void>();
  const busyCallback = useRef(onBusyChange);
  busyCallback.current = onBusyChange;
  const alive = (epoch: number) =>
    mounted.current && generation.current === epoch;
  useEffect(() => {
    const epoch = ++generation.current;
    mounted.current = true;
    running.current = false;
    refreshing.current = false;
    cancelled.current = false;
    operation.current = undefined;
    stop.current = undefined;
    setState(null);
    setPlan(null);
    setResult(null);
    setError("");
    setBusy(false);
    setLoading(true);
    setRefreshError(false);
    setRefreshBusy(false);
    void catalogLifecycleApi
      .inspect(entry.toolId)
      .then((found) => {
        if (!alive(epoch)) return;
        if (found.toolId !== entry.toolId || !Array.isArray(found.actions))
          throw new Error();
        setState(found);
      })
      .catch(() => {
        if (alive(epoch)) setError("catalogLifecycle.unavailable");
      })
      .finally(() => {
        if (alive(epoch)) setLoading(false);
      });
    return () => {
      mounted.current = false;
      generation.current++;
      cancelled.current = true;
      stop.current?.();
      stop.current = undefined;
      const id = operation.current;
      operation.current = undefined;
      if (id) void catalogLifecycleApi.cancel(id).catch(() => undefined);
      busyCallback.current(false);
    };
  }, [entry.toolId]);
  async function refreshDirectory(operationId?: string) {
    if (refreshing.current) return;
    const epoch = generation.current;
    refreshing.current = true;
    setRefreshBusy(true);
    setRefreshError(false);
    try {
      await onChanged(operationId);
    } catch {
      if (alive(epoch)) setRefreshError(true);
    } finally {
      if (alive(epoch)) {
        refreshing.current = false;
        setRefreshBusy(false);
      }
    }
  }
  async function preview(action: CatalogAction) {
    if (running.current || busy) return;
    const epoch = generation.current;
    running.current = true;
    cancelled.current = false;
    setBusy(true);
    setError("");
    setResult(null);
    setPlan(null);
    setRefreshError(false);
    let id: string | undefined;
    try {
      const previous = operation.current;
      if (previous) await catalogLifecycleApi.cancel(previous);
      if (!alive(epoch) || cancelled.current) return;
      operation.current = undefined;
      const prepared = await catalogLifecycleApi.prepare(entry.toolId, action);
      if (typeof prepared.operationId === "string" && prepared.operationId)
        id = prepared.operationId;
      if (!alive(epoch) || cancelled.current) {
        if (id) await catalogLifecycleApi.cancel(id);
        return;
      }
      if (
        prepared.toolId !== entry.toolId ||
        prepared.action !== action ||
        !id ||
        !prepared.prefix ||
        !prepared.command ||
        !Number.isFinite(prepared.expiresAtMs) ||
        prepared.expiresAtMs <= Date.now()
      )
        throw new Error();
      operation.current = id;
      setPlan(prepared);
    } catch {
      if (id) void catalogLifecycleApi.cancel(id).catch(() => undefined);
      if (alive(epoch)) setError("catalogLifecycle.prepareFailed");
    } finally {
      if (alive(epoch)) {
        setBusy(false);
        running.current = false;
      }
    }
  }
  async function run() {
    if (!plan || running.current || busy || cancelled.current) return;
    const epoch = generation.current;
    const id = plan.operationId;
    running.current = true;
    setBusy(true);
    busyCallback.current(true);
    setError("");
    setStage("verify");
    let off: (() => void) | undefined;
    try {
      off = await subscribe<CatalogProgress>(
        "catalog-operation-progress",
        (progress) => {
          if (
            alive(epoch) &&
            !cancelled.current &&
            progress.operationId === id &&
            STAGES.includes(progress.stage)
          )
            setStage(progress.stage);
        },
      );
      if (!alive(epoch) || cancelled.current) {
        await catalogLifecycleApi.cancel(id);
        return;
      }
      stop.current = off;
      const finished = await catalogLifecycleApi.run(id);
      if (!alive(epoch)) return;
      if (
        finished.operationId !== id ||
        !["ok", "partial", "cancelled", "failed"].includes(finished.state) ||
        finished.installation?.toolId !== entry.toolId ||
        !Array.isArray(finished.installation.actions)
      )
        throw new Error();
      setResult(finished);
      setState(finished.installation);
      setPlan(null);
      operation.current = undefined;
      await refreshDirectory(id);
    } catch {
      void catalogLifecycleApi.cancel(id).catch(() => undefined);
      if (alive(epoch)) {
        setError("catalogLifecycle.executeFailed");
        setPlan(null);
        operation.current = undefined;
        setState((previous) =>
          previous
            ? {
                ...previous,
                state: "needsReview",
                actions: [],
                reason: "operationUnknown",
              }
            : null,
        );
      }
    } finally {
      off?.();
      if (alive(epoch)) {
        stop.current = undefined;
        running.current = false;
        setBusy(false);
        busyCallback.current(false);
      }
    }
  }
  function cancel() {
    if (cancelled.current) return;
    const epoch = generation.current;
    cancelled.current = true;
    setStage("cleanup");
    stop.current?.();
    stop.current = undefined;
    busyCallback.current(false);
    if (operation.current)
      void catalogLifecycleApi.cancel(operation.current).catch(() => {
        if (alive(epoch)) setError("catalogLifecycle.executeFailed");
      });
  }
  return (
    <div className="space-y-3 text-ctl">
      <p className="text-note text-ink-muted">
        {t("catalogLifecycle.description")}
      </p>
      {loading && <p role="status">{t("common.loading")}</p>}
      {error && (
        <p role="alert" className="text-warn">
          {t(error)}
        </p>
      )}
      {state && (
        <>
          <p className="break-all text-note text-ink-muted">{state.prefix}</p>
          <p>
            {t(`catalogLifecycle.copyState.${state.state}`, {
              defaultValue: t("catalogLifecycle.unavailable"),
            })}
            {state.version ? ` · ${state.version}` : ""}
          </p>
          {state.nodeVersion && (
            <p className="text-note text-ink-muted">
              Node.js {state.nodeVersion}
            </p>
          )}
          {(state.reason || !state.supported) && (
            <p className="text-note text-warn">
              {t("catalogLifecycle.manualRequired")}
            </p>
          )}
          {!plan && !busy && (
            <div className="flex flex-wrap gap-2">
              {state.actions
                .filter((action) => ACTIONS.includes(action))
                .map((action) => (
                  <Button
                    key={action}
                    variant="secondary"
                    onClick={() => void preview(action)}
                  >
                    {t(`catalogLifecycle.preview.${action}`)}
                  </Button>
                ))}
            </div>
          )}
          {state.launchCommand && (
            <div className="flex min-w-0 items-start gap-2">
              <code className="min-w-0 flex-1 whitespace-pre-wrap break-all rounded-ctl bg-canvas p-2 text-note">
                {state.launchCommand}
              </code>
              <CopyButton
                text={state.launchCommand}
                aria-label={t("catalogLifecycle.copyLaunch")}
              />
            </div>
          )}
        </>
      )}
      {plan && (
        <div className="space-y-2 rounded-card border border-line p-3">
          <p>
            {t("catalogLifecycle.frozenVersion", {
              version: plan.version,
              node: plan.nodeVersion,
            })}
          </p>
          <code className="block whitespace-pre-wrap break-all rounded-ctl bg-canvas p-2 text-note">
            {plan.command}
          </code>
          <p className="text-note text-ink-muted">
            {t("catalogLifecycle.preservesData")}
          </p>
          <Button disabled={busy} onClick={() => void run()}>
            {t(`catalogLifecycle.start.${plan.action}`)}
          </Button>
        </div>
      )}
      {busy && (
        <div className="flex items-center gap-2">
          <p role="status" className="text-note text-ink-muted">
            {t(`catalogLifecycle.stages.${stage}`)}
          </p>
          <Button
            variant="secondary"
            disabled={cancelled.current}
            onClick={cancel}
          >
            {t("catalogLifecycle.cancel")}
          </Button>
        </div>
      )}
      {refreshError && (
        <div role="alert" className="text-note text-warn">
          <p>{t("catalogLifecycle.refreshFailed")}</p>
          <Button
            variant="secondary"
            disabled={refreshBusy}
            onClick={() => void refreshDirectory()}
          >
            {t("common.retry")}
          </Button>
        </div>
      )}
      {result && (
        <p
          role="status"
          className={result.state === "ok" ? "text-ink" : "text-warn"}
        >
          {t(`catalogLifecycle.result.${result.state}`)}
        </p>
      )}
    </div>
  );
}
