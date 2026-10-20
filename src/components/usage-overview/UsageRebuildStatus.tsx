import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  cancelUsageRebuild,
  getUsageRebuildProgress,
} from "@/lib/api/usageOverview";
import { queryClient } from "@/lib/query/queryClient";
import type { UsageRebuildProgress } from "@/types/usageQuery";

const progressKey = ["usageNativeRebuildProgress"];
const terminal = (state: UsageRebuildProgress["state"]) =>
  ["cancelled", "completed", "failed"].includes(state);

export function UsageRebuildStatus({
  active,
  supported,
  onDismiss,
}: {
  active: boolean;
  supported: boolean;
  onDismiss: () => void;
}) {
  const { t } = useTranslation();
  const progress = useQuery(
    {
      queryKey: progressKey,
      queryFn: getUsageRebuildProgress,
      enabled: active && supported,
      retry: false,
      staleTime: 0,
      gcTime: 0,
      refetchOnWindowFocus: false,
      refetchInterval: (query) =>
        active && supported && !query.state.error ? 1000 : false,
    },
    queryClient,
  );
  const current = progress.data;
  const owner = useMemo(() => ({}), [active, supported, current?.operationId]);
  const currentOwner = useRef<object>();
  useLayoutEffect(() => {
    currentOwner.current = owner;
    return () => {
      currentOwner.current = undefined;
    };
  }, [owner]);
  const observed = useRef<string>();
  const refreshed = useRef<string>();
  const cancelFocused = useRef(false);
  const dismissButton = useRef<HTMLButtonElement>(null);
  const retryButton = useRef<HTMLButtonElement>(null);
  const [dismissed, setDismissed] = useState<string>();
  const [cancelling, setCancelling] = useState<string>();
  const [cancelError, setCancelError] = useState<string>();
  useLayoutEffect(() => {
    if (!active || !supported || !progress.isError || !cancelFocused.current)
      return;
    retryButton.current?.focus();
    cancelFocused.current = false;
  }, [active, supported, progress.isError]);
  useLayoutEffect(() => {
    if (!active || !supported || !current) return;
    if (!terminal(current.state)) observed.current = current.operationId;
    if (terminal(current.state) && cancelFocused.current) {
      dismissButton.current?.focus();
      cancelFocused.current = false;
    }
  }, [active, supported, current]);
  useEffect(() => {
    if (
      current?.state !== "completed" ||
      observed.current !== current.operationId ||
      refreshed.current === current.operationId
    )
      return;
    refreshed.current = current.operationId;
    for (const queryKey of [
      ["usageOverview"],
      ["sessionUsageAnalytics"],
      ["usage", "workbench-logs"],
    ]) {
      void queryClient.invalidateQueries({ queryKey });
    }
  }, [current]);
  const cancel = async () => {
    if (
      !current ||
      current.state !== "running" ||
      cancelling === current.operationId
    )
      return;
    const operationId = current.operationId;
    setCancelling(operationId);
    setCancelError(undefined);
    try {
      const result = await cancelUsageRebuild(operationId);
      if (currentOwner.current !== owner) return;
      await queryClient.cancelQueries({ queryKey: progressKey, exact: true });
      queryClient.setQueryData<UsageRebuildProgress | null>(
        progressKey,
        (latest) =>
          latest?.operationId === operationId && !terminal(latest.state)
            ? result
            : latest,
      );
    } catch {
      if (currentOwner.current === owner) setCancelError(operationId);
    } finally {
      if (currentOwner.current === owner) setCancelling(undefined);
    }
  };
  if (!active || !supported) return null;
  if (progress.isError)
    return (
      <div
        role="status"
        className="flex flex-wrap items-center gap-2 rounded-lg border border-line bg-card px-3 py-2 text-note text-ink-muted"
      >
        <span>{t("usageExperience.rebuildUnavailable")}</span>
        <Button
          ref={retryButton}
          variant="ghost"
          onClick={() => void progress.refetch()}
        >
          {t("common.retry")}
        </Button>
      </div>
    );
  if (
    !current ||
    current.operationId === dismissed ||
    current.state === "completed"
  )
    return null;
  const isTerminal = terminal(current.state);
  return (
    <div
      role="status"
      className="flex min-w-0 flex-wrap items-center justify-between gap-2 rounded-lg border border-line bg-card px-3 py-2 text-note text-ink-muted"
    >
      <div className="min-w-0">
        <p>
          {t(
            `usageExperience.rebuild${current.state === "running" ? "Running" : current.state === "cancelling" ? "Cancelling" : current.state === "cancelled" ? "Cancelled" : "Failed"}`,
            {
              completed: current.completedSources,
              total: current.totalSources,
            },
          )}
        </p>
        {cancelError === current.operationId && (
          <p role="alert" className="text-warn">
            {t("usageExperience.rebuildCancelFailed")}
          </p>
        )}
      </div>
      {isTerminal ? (
        <Button
          ref={dismissButton}
          variant="ghost"
          onClick={() => {
            setDismissed(current.operationId);
            onDismiss();
          }}
        >
          {t("usageWorkbench.close")}
        </Button>
      ) : (
        <Button
          variant="secondary"
          aria-disabled={
            current.state === "cancelling" || cancelling === current.operationId
          }
          onFocus={() => {
            cancelFocused.current = true;
          }}
          onBlur={() => {
            cancelFocused.current = false;
          }}
          onClick={() => void cancel()}
        >
          {t("usageExperience.rebuildCancel")}
        </Button>
      )}
    </div>
  );
}
