import { useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { useInteractionActive } from "@/components/ui/interaction-scope";
import { Button } from "@/components/ui/button";
import { ListCard, ListRow } from "@/components/ui/list";
import {
  libraryApi,
  type McpDefinition,
  type McpCheckProgress,
  type McpCheckResult,
} from "@/lib/api/library";
import { useDraftState, type DraftStore } from "./useDraftState";
import { libraryError } from "./libraryText";
import { checkIssue, type LibraryNextAction } from "./libraryIssues";
import { LibraryIssueNotice } from "./LibraryIssueNotice";
const STAGES = [
  "preflight",
  "connect",
  "negotiate",
  "list",
  "cleanup",
] as const;
export function McpCheck({
  store,
  server,
  epoch,
  readOnly,
  onBusyChange,
  onResult,
  onNextAction,
  scopeKey = "",
}: {
  scopeKey?: string;
  store: DraftStore;
  server: McpDefinition;
  epoch: number;
  readOnly: boolean;
  onBusyChange: (busy: boolean) => void;
  onResult?: (result: McpCheckResult, checkedAtMs: number) => void;
  onNextAction?: (action: LibraryNextAction, trigger: HTMLElement) => void;
}) {
  const { t } = useTranslation();
  const active = useInteractionActive();
  const [busy, setBusy] = useDraftState(store, "mcpCheckBusy", false);
  const [stage, setStage] = useDraftState<string>(
    store,
    "mcpCheckStage",
    "preflight",
  );
  const [result, setResult] = useDraftState<McpCheckResult | null>(
    store,
    "mcpCheckResult",
    null,
  );
  const [error, setError] = useDraftState(store, "mcpCheckError", "");
  const [resultProof, setResultProof] = useDraftState(
    store,
    "mcpCheckResultProof",
    "",
  );
  const [cancelling, setCancelling] = useDraftState(
    store,
    "mcpCheckCancelling",
    false,
  );
  const mounted = useRef(true);
  const running = useRef(false);
  const cancelled = useRef(false);
  const operation = useRef<string>();
  const stopListener = useRef<UnlistenFn>();
  const busyCallback = useRef(onBusyChange);
  busyCallback.current = onBusyChange;
  const resultCallback = useRef(onResult);
  resultCallback.current = onResult;
  const currentProof = `${scopeKey}:${epoch}:${server.id}:${server.revision}`;
  const proof = useRef(currentProof);
  const generation = useRef(0);
  if (proof.current !== currentProof) {
    proof.current = currentProof;
    generation.current++;
  }
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      cancelled.current = true;
      stopListener.current?.();
      stopListener.current = undefined;
      if (operation.current)
        void libraryApi
          .cancelMcpCheck(operation.current)
          .catch(() => undefined);
      if (running.current) {
        setResult({
          operationId: operation.current ?? "",
          state: "cancelled",
          tools: [],
        });
        setBusy(false);
        setCancelling(false);
        busyCallback.current(false);
      }
    };
  }, [setBusy, setCancelling, setResult]);
  useEffect(() => {
    if (resultProof && resultProof !== currentProof) {
      setResult(null);
      setError("");
    }
    if (running.current) {
      cancelled.current = true;
      if (operation.current)
        void libraryApi
          .cancelMcpCheck(operation.current)
          .catch(() => undefined);
    }
  }, [currentProof]);
  useEffect(() => {
    if (active || !running.current) return;
    generation.current++;
    cancelled.current = true;
    setCancelling(true);
    stopListener.current?.();
    stopListener.current = undefined;
    busyCallback.current(false);
    if (operation.current)
      void libraryApi.cancelMcpCheck(operation.current).catch(() => undefined);
  }, [active, setCancelling]);
  async function start() {
    if (!active || running.current || busy || readOnly) return;
    running.current = true;
    cancelled.current = false;
    setBusy(true);
    setCancelling(false);
    setError("");
    setResult(null);
    setStage("preflight");
    busyCallback.current(true);
    let unlisten: UnlistenFn | undefined;
    let id: string | undefined;
    const expectedProof = currentProof;
    const expectedGeneration = generation.current;
    const current = () =>
      mounted.current &&
      !cancelled.current &&
      proof.current === expectedProof &&
      generation.current === expectedGeneration;
    try {
      const prepared = await libraryApi.prepareMcpCheck(
        server.id,
        server.revision,
        epoch,
      );
      id = prepared.operationId;
      if (!current()) {
        await libraryApi.cancelMcpCheck(id);
        return;
      }
      operation.current = id;
      const subscribed = await listen<McpCheckProgress>(
        "library-mcp-check-progress",
        (event) => {
          if (
            current() &&
            event.payload.operationId === id &&
            STAGES.includes(event.payload.stage)
          )
            setStage(event.payload.stage);
        },
      );
      let stopped = false;
      unlisten = () => {
        if (!stopped) {
          stopped = true;
          subscribed();
        }
      };
      stopListener.current = unlisten;
      if (!current()) {
        await libraryApi.cancelMcpCheck(id);
        return;
      }
      const checked = await libraryApi.runMcpCheck(id);
      if (!current()) return;
      if (
        checked.operationId !== id ||
        ![
          "ok",
          "blocked",
          "authRequired",
          "forbidden",
          "timeout",
          "cancelled",
          "error",
        ].includes(checked.state) ||
        (checked.reason != null && typeof checked.reason !== "string") ||
        !Array.isArray(checked.tools) ||
        checked.tools.some(
          (tool) =>
            !tool ||
            typeof tool.name !== "string" ||
            !tool.name ||
            (tool.description != null && typeof tool.description !== "string"),
        )
      )
        throw new Error("library.failed");
      setResult(
        cancelled.current
          ? { operationId: id, state: "cancelled", tools: [] }
          : checked,
      );
      setResultProof(expectedProof);
      if (!cancelled.current && checked.state !== "cancelled")
        resultCallback.current?.(checked, Date.now());
    } catch (failure) {
      if (id) void libraryApi.cancelMcpCheck(id).catch(() => undefined);
      if (current()) setError(libraryError(failure, t));
    } finally {
      unlisten?.();
      stopListener.current = undefined;
      if (mounted.current) {
        if (cancelled.current)
          setResult({ operationId: id ?? "", state: "cancelled", tools: [] });
        operation.current = undefined;
        running.current = false;
        setBusy(false);
        setCancelling(false);
        busyCallback.current(false);
      }
    }
  }
  function cancel() {
    if (cancelled.current) return;
    cancelled.current = true;
    setCancelling(true);
    // Closing the dialog remains available while the native request cleans up.
    busyCallback.current(false);
    if (operation.current)
      void libraryApi.cancelMcpCheck(operation.current).catch(() => {
        if (mounted.current) setError(t("library.failed"));
      });
  }
  return (
    <div className="space-y-3 text-ctl">
      <p className="text-ink-muted">{t("library.mcpCheckDescription")}</p>
      <div className="flex flex-wrap gap-2">
        <Button disabled={busy || readOnly} onClick={() => void start()}>
          {t("library.mcpCheckStart")}
        </Button>
        {busy && (
          <Button variant="secondary" disabled={cancelling} onClick={cancel}>
            {t(
              cancelling
                ? "library.mcpCheckCancelling"
                : "library.mcpCheckCancel",
            )}
          </Button>
        )}
      </div>
      {busy && (
        <p role="status">
          {t(`library.mcpCheckStage.${stage}`, {
            defaultValue: t("library.loading"),
          })}
        </p>
      )}
      {error && (
        <p role="alert" className="text-danger">
          {error}
        </p>
      )}
      {result &&
        (result.state === "cancelled" || resultProof === currentProof) && (
          <div className="space-y-2">
            <p role="status">
              {t(`library.mcpCheckState.${result.state}`, {
                defaultValue: t("library.failed"),
              })}
            </p>
            {result.reason && (
              <p className="text-note text-ink-muted">
                {t(`library.mcpCheckReason.${result.reason}`, {
                  defaultValue: t("library.failed"),
                })}
              </p>
            )}
            {resultProof === currentProof && checkIssue(result) && (
              <LibraryIssueNotice
                issue={checkIssue(result)!}
                disabled={readOnly || busy}
                onAction={onNextAction}
              />
            )}
            {result.state === "blocked" && (
              <p className="text-ctl text-ink-muted">
                {t("library.workbench2.nativeCheck")}
              </p>
            )}
            {result.state === "ok" && !result.tools.length && (
              <p className="text-note text-ink-muted">
                {t("library.mcpCheckToolCount", { count: result.tools.length })}
              </p>
            )}
            {!!result.tools.length && (
              <details open className="space-y-2">
                <summary className="w-fit cursor-pointer rounded text-note text-ink-2 outline-none focus-visible:ring-2 focus-visible:ring-brand">
                  {t("library.mcpCheckToolCount", {
                    count: result.tools.length,
                  })}
                </summary>
                <ListCard>
                  {result.tools.map((tool, index) => (
                    <ListRow
                      key={`${index}-${tool.name}`}
                      className="block space-y-1"
                    >
                      <p className="break-all font-medium">{tool.name}</p>
                      {tool.description && (
                        <p className="whitespace-pre-wrap break-words text-note text-ink-muted">
                          {tool.description}
                        </p>
                      )}
                    </ListRow>
                  ))}
                </ListCard>
              </details>
            )}
          </div>
        )}
    </div>
  );
}
