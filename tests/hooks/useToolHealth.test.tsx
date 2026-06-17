import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";

import { server } from "../msw/server";
import { emitTauriEvent } from "../msw/tauriMocks";
import { useToolHealth } from "@/hooks/useToolHealth";
import { OFOX_TOOL_HEALTH_UPDATED_EVENT } from "@/lib/api/toolHealth";

const TAURI_ENDPOINT = "http://tauri.local";

const okSnapshot = {
  toolId: "claude",
  status: "ok" as const,
  latencyMs: 234,
  checkedAt: 1_700_000_000_000,
  statusCode: 200,
  error: null,
};

describe("useToolHealth", () => {
  beforeEach(() => {
    vi.useRealTimers();
  });

  it("populates initial snapshot from getSnapshot on mount", async () => {
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_tool_health_snapshot`, () =>
        HttpResponse.json({ claude: okSnapshot }),
      ),
    );

    const { result } = renderHook(() => useToolHealth());

    await waitFor(() => {
      expect(result.current.snapshot.claude).toBeDefined();
    });

    expect(result.current.snapshot.claude).toEqual(okSnapshot);
    // Initial pull should NOT bump lastUpdatedAt — only real backend events
    // do, so the UI's "X 分钟前" stays empty until first tick.
    expect(result.current.lastUpdatedAt).toBe(0);
  });

  it("updates snapshot when ofox-tool-health-updated event fires", async () => {
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_tool_health_snapshot`, () =>
        HttpResponse.json({}),
      ),
    );

    const { result } = renderHook(() => useToolHealth());

    // Wait for the listener to be installed.
    await waitFor(() => {
      // Best signal we have: the initial getSnapshot resolved (empty map).
      expect(Object.keys(result.current.snapshot)).toHaveLength(0);
    });

    act(() => {
      emitTauriEvent(OFOX_TOOL_HEALTH_UPDATED_EVENT, {
        codex: { ...okSnapshot, toolId: "codex" },
      });
    });

    await waitFor(() => {
      expect(result.current.snapshot.codex).toBeDefined();
    });
    expect(result.current.snapshot.codex.toolId).toBe("codex");
    expect(result.current.lastUpdatedAt).toBeGreaterThan(0);
  });

  it("triggerNow invokes backend and updates snapshot", async () => {
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_tool_health_snapshot`, () =>
        HttpResponse.json({}),
      ),
      http.post(`${TAURI_ENDPOINT}/trigger_tool_health_check_now`, () =>
        HttpResponse.json({ gemini: { ...okSnapshot, toolId: "gemini" } }),
      ),
    );

    const { result } = renderHook(() => useToolHealth());

    // Wait for the initial snapshot fetch to settle.
    await waitFor(() => {
      expect(Object.keys(result.current.snapshot)).toHaveLength(0);
    });

    await act(async () => {
      await result.current.triggerNow();
    });

    expect(result.current.snapshot.gemini).toBeDefined();
    expect(result.current.snapshot.gemini.toolId).toBe("gemini");
  });
});
