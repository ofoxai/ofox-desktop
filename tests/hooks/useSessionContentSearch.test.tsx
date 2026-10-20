import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emitTauriEvent } from "../msw/tauriMocks";
import {
  sessionsApi,
  type SessionSearchHit,
  type SessionSearchResult,
} from "@/lib/api/sessions";
import { useSessionContentSearch } from "@/hooks/useSessionContentSearch";

const hit = (ordinal = 0): SessionSearchHit => ({
  sessionKey: "key",
  readRevision: "revision",
  messageOrdinal: ordinal,
  contentHash: `hash-${ordinal}`,
  role: "user",
  snippet: "needle",
});
const payload = (
  operationId: string,
  requestId: string,
  hits: SessionSearchHit[] = [],
) => ({
  operationId,
  requestId,
  hits,
  scanned: 1,
  total: 2,
  changed: 0,
  failed: 0,
  unsupported: 0,
});
beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(sessionsApi, "cancelSearch").mockResolvedValue(true);
});

describe("explicit local session content search", () => {
  it("subscribes before executing and rejects other operation or request events", async () => {
    let finish!: (result: SessionSearchResult) => void;
    vi.spyOn(sessionsApi, "prepareSearch").mockImplementation(
      async ({ requestId }) => ({
        operationId: "current",
        requestId,
        candidateCount: 2,
      }),
    );
    const run = vi
      .spyOn(sessionsApi, "runSearch")
      .mockImplementation(async () => {
        const requestId = vi.mocked(sessionsApi.prepareSearch).mock.calls[0][0]
          .requestId;
        emitTauriEvent(
          "session-search-progress",
          payload("old", requestId, [hit(9)]),
        );
        emitTauriEvent(
          "session-search-progress",
          payload("current", "other-request", [hit(8)]),
        );
        emitTauriEvent(
          "session-search-progress",
          payload("current", requestId, [hit()]),
        );
        return await new Promise<SessionSearchResult>((resolve) => {
          finish = resolve;
        });
      });
    const { result } = renderHook(() => useSessionContentSearch("scope"));
    act(() => {
      void result.current.start("needle", ["key"], false);
    });
    await waitFor(() => expect(run).toHaveBeenCalledOnce());
    await waitFor(() => expect(result.current.hits).toEqual([hit()]));
    const requestId = vi.mocked(sessionsApi.prepareSearch).mock.calls[0][0]
      .requestId;
    await act(async () =>
      finish({
        ...payload("current", requestId, [hit()]),
        state: "complete",
        limitReasons: [],
      }),
    );
    expect(result.current.hits).toEqual([hit()]);
    expect(result.current.state).toBe("complete");
  });

  it("cancels a late prepared operation after filters change without starting a scan", async () => {
    let finish!: (result: {
      operationId: string;
      requestId: string;
      candidateCount: number;
    }) => void;
    const prepare = vi.spyOn(sessionsApi, "prepareSearch").mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const run = vi.spyOn(sessionsApi, "runSearch");
    const { result, rerender } = renderHook(
      ({ scope }) => useSessionContentSearch(scope),
      { initialProps: { scope: "old" } },
    );
    act(() => {
      void result.current.start("needle", ["key"], false);
    });
    rerender({ scope: "new" });
    await act(async () =>
      finish({
        operationId: "abandoned",
        requestId: prepare.mock.calls[0][0].requestId,
        candidateCount: 1,
      }),
    );
    expect(sessionsApi.cancelSearch).toHaveBeenCalledWith("abandoned");
    expect(run).not.toHaveBeenCalled();
    expect(result.current.state).toBe("idle");
    expect(result.current.hits).toEqual([]);
  });

  it("retains prior batches, deduplicates streamed hits and prevents duplicate continuation", async () => {
    const prepare = vi
      .spyOn(sessionsApi, "prepareSearch")
      .mockImplementation(async ({ requestId }) => ({
        operationId: "current",
        requestId,
        candidateCount: 2,
      }));
    let finish!: (result: SessionSearchResult) => void;
    const run = vi
      .spyOn(sessionsApi, "runSearch")
      .mockImplementationOnce(async () => ({
        ...payload("current", prepare.mock.calls[0][0].requestId, [hit()]),
        state: "limited",
        nextCursor: "cursor",
        limitReasons: ["deadline"],
      }))
      .mockImplementationOnce(async () => {
        emitTauriEvent(
          "session-search-progress",
          payload("current", prepare.mock.calls[0][0].requestId, [hit(1)]),
        );
        return await new Promise<SessionSearchResult>((resolve) => {
          finish = resolve;
        });
      });
    const { result } = renderHook(() => useSessionContentSearch("scope"));
    await act(async () => result.current.start("needle", ["key"], true));
    expect(result.current.state).toBe("limited");
    act(() => {
      void result.current.continueScan();
      void result.current.continueScan();
    });
    await waitFor(() => expect(run).toHaveBeenCalledTimes(2));
    await act(async () =>
      finish({
        ...payload("current", prepare.mock.calls[0][0].requestId, [hit(1)]),
        state: "complete",
        limitReasons: [],
      }),
    );
    expect(result.current.hits).toEqual([hit(), hit(1)]);
    expect(run).toHaveBeenLastCalledWith("current", "cursor");
  });

  it("ignores late results after cancel and keeps already delivered excerpts", async () => {
    const prepare = vi
      .spyOn(sessionsApi, "prepareSearch")
      .mockImplementation(async ({ requestId }) => ({
        operationId: "current",
        requestId,
        candidateCount: 2,
      }));
    let finish!: (result: SessionSearchResult) => void;
    vi.spyOn(sessionsApi, "runSearch").mockImplementation(async () => {
      emitTauriEvent(
        "session-search-progress",
        payload("current", prepare.mock.calls[0][0].requestId, [hit()]),
      );
      return await new Promise<SessionSearchResult>((resolve) => {
        finish = resolve;
      });
    });
    const { result } = renderHook(() => useSessionContentSearch("scope"));
    act(() => {
      void result.current.start("needle", ["key"], false);
    });
    await waitFor(() => expect(result.current.hits).toHaveLength(1));
    act(() => result.current.cancel());
    await act(async () =>
      finish({
        ...payload("current", prepare.mock.calls[0][0].requestId, [hit(1)]),
        state: "complete",
        limitReasons: [],
      }),
    );
    expect(result.current.state).toBe("cancelled");
    expect(result.current.hits).toEqual([hit()]);
  });

  it("ends a mismatched terminal reply with an error instead of remaining busy forever", async () => {
    vi.spyOn(sessionsApi, "prepareSearch").mockImplementation(
      async ({ requestId }) => ({
        operationId: "current",
        requestId,
        candidateCount: 2,
      }),
    );
    vi.spyOn(sessionsApi, "runSearch").mockResolvedValue({
      ...payload("other-operation", "other-request", [hit()]),
      state: "complete",
      limitReasons: [],
    });
    const { result } = renderHook(() => useSessionContentSearch("scope"));
    await act(async () => result.current.start("needle", ["key"], false));
    expect(result.current.state).toBe("error");
    expect(result.current.busy).toBe(false);
    expect(result.current.hits).toEqual([]);
  });
});
