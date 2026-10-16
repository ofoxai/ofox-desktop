import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  sessionsApi,
  type SessionSearchHit,
  type SessionSearchProgress,
  type SessionSearchResult,
} from "@/lib/api/sessions";

type SearchState =
  | "idle"
  | "preparing"
  | "running"
  | "complete"
  | "limited"
  | "cancelled"
  | "error";
interface SearchView {
  state: SearchState;
  hits: SessionSearchHit[];
  progress?: Omit<SessionSearchProgress, "hits">;
  nextCursor?: string;
  error?: string;
  limitReasons?: string[];
}
const EMPTY: SearchView = { state: "idle", hits: [] };
const hitKey = (hit: SessionSearchHit) =>
  `${hit.sessionKey}:${hit.readRevision}:${hit.messageOrdinal}:${hit.contentHash}`;
function mergeHits(previous: SessionSearchHit[], incoming: SessionSearchHit[]) {
  const rows = new Map(previous.map((hit) => [hitKey(hit), hit]));
  for (const hit of incoming) rows.set(hitKey(hit), hit);
  return [...rows.values()];
}

/** Explicit local scans. Every reply is tied to the current criteria and operation. */
export function useSessionContentSearch(scope: string) {
  const [view, setView] = useState<SearchView>(EMPTY);
  const generation = useRef(0);
  const active = useRef<{
    operationId: string;
    requestId: string;
    stop?: () => void;
  }>();
  const running = useRef(false);
  const cancel = useCallback(() => {
    generation.current++;
    const operation = active.current;
    active.current = undefined;
    operation?.stop?.();
    if (operation)
      void sessionsApi
        .cancelSearch(operation.operationId)
        .catch(() => undefined);
    running.current = false;
    setView((old) =>
      old.state === "idle"
        ? old
        : { ...old, state: "cancelled", nextCursor: undefined },
    );
  }, []);
  useEffect(() => {
    cancel();
    setView(EMPTY);
    return cancel;
  }, [scope, cancel]);

  const scan = useCallback(
    async (
      operation: { operationId: string; requestId: string },
      epoch: number,
      cursor?: string,
    ) => {
      const current = () =>
        generation.current === epoch &&
        active.current?.operationId === operation.operationId;
      try {
        // Subscription must finish before the backend starts reading content.
        const stop = await listen<SessionSearchProgress>(
          "session-search-progress",
          ({ payload }) => {
            if (
              !current() ||
              payload.operationId !== operation.operationId ||
              payload.requestId !== operation.requestId
            )
              return;
            setView((old) => ({
              ...old,
              state: "running",
              progress: payload,
              hits: mergeHits(old.hits, payload.hits),
            }));
          },
        );
        if (!current()) {
          stop();
          return;
        }
        active.current!.stop = stop;
        const result: SessionSearchResult = await sessionsApi.runSearch(
          operation.operationId,
          cursor,
        );
        if (!current()) return;
        if (
          result.operationId !== operation.operationId ||
          result.requestId !== operation.requestId
        )
          throw new Error("session.search.invalidRequest");
        setView((old) => ({
          state: result.state,
          hits: mergeHits(old.hits, result.hits),
          progress: result,
          nextCursor: result.nextCursor ?? undefined,
          limitReasons: result.limitReasons,
        }));
      } catch (error) {
        if (current()) {
          void sessionsApi
            .cancelSearch(operation.operationId)
            .catch(() => undefined);
          setView((old) => ({
            ...old,
            state: "error",
            error: String(error),
            nextCursor: undefined,
          }));
        }
      } finally {
        if (current()) {
          active.current?.stop?.();
          if (active.current) active.current.stop = undefined;
          running.current = false;
        }
      }
    },
    [],
  );

  const start = useCallback(
    async (query: string, sessionKeys: string[], includeProcess: boolean) => {
      cancel();
      const epoch = generation.current;
      if (!query.trim() || [...query.trim()].length > 256) {
        setView({
          ...EMPTY,
          state: "error",
          error: "session.search.invalidQuery",
        });
        return;
      }
      running.current = true;
      setView({ state: "preparing", hits: [] });
      const requestId = crypto.randomUUID();
      try {
        const prepared = await sessionsApi.prepareSearch({
          requestId,
          query: query.trim(),
          sessionKeys,
          includeProcess,
        });
        if (generation.current !== epoch) {
          void sessionsApi
            .cancelSearch(prepared.operationId)
            .catch(() => undefined);
          return;
        }
        if (prepared.requestId !== requestId) {
          void sessionsApi
            .cancelSearch(prepared.operationId)
            .catch(() => undefined);
          throw new Error("session.search.invalidRequest");
        }
        active.current = { operationId: prepared.operationId, requestId };
        setView({
          state: "running",
          hits: [],
          progress: {
            operationId: prepared.operationId,
            requestId,
            scanned: 0,
            total: prepared.candidateCount,
            changed: 0,
            failed: 0,
            unsupported: 0,
          },
        });
        await scan(active.current, epoch);
      } catch (error) {
        if (generation.current === epoch) {
          running.current = false;
          setView({ state: "error", hits: [], error: String(error) });
        }
      }
    },
    [cancel, scan],
  );

  const continueScan = useCallback(async () => {
    if (running.current || !active.current || !view.nextCursor) return;
    running.current = true;
    setView((old) => ({ ...old, state: "running", nextCursor: undefined }));
    await scan(active.current, generation.current, view.nextCursor);
  }, [scan, view.nextCursor]);
  return {
    ...view,
    start,
    cancel,
    continueScan,
    busy: view.state === "preparing" || view.state === "running",
  };
}
