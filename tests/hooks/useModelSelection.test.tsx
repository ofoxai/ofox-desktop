import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as events from "@tauri-apps/api/event";
import {
  useModelSelection,
  filterModelsForNativeTool,
} from "@/hooks/useModelSelection";
import type { FetchedModel } from "@/lib/api/model-fetch";
import {
  modelSelectionApi,
  type ToolModelSnapshot,
} from "@/lib/api/modelSelection";
import { queryClient } from "@/lib/query/queryClient";
import { emitTauriEvent } from "../msw/tauriMocks";

const snapshot: ToolModelSnapshot = {
  toolId: "codex",
  writeTarget: "codex",
  revision: "one",
  binding: {
    status: "configured",
    message: null,
    missingFiles: [],
    modifiedFields: [],
    envOverrides: [],
  },
  managedSelection: { kind: "single", model: "model" },
  observedSelection: { model: "model", source: "config" },
  supportsDefault: true,
  scope: [],
};

beforeEach(() => {
  queryClient.clear();
  vi.spyOn(modelSelectionApi, "snapshot").mockResolvedValue({
    tools: [snapshot],
  });
});
afterEach(() => vi.restoreAllMocks());

describe("shared model state", () => {
  it("deduplicates native listeners across mounted consumers and refreshes their shared snapshot", async () => {
    const listener = vi.spyOn(events, "listen");
    const first = renderHook(() => useModelSelection(["codex"]));
    const second = renderHook(() => useModelSelection(["codex"]));
    await waitFor(() => expect(first.result.current.data).toBeDefined());
    expect(
      listener.mock.calls.filter(
        ([name]) => name === "model-selection-changed",
      ),
    ).toHaveLength(1);
    vi.mocked(modelSelectionApi.snapshot).mockResolvedValue({
      tools: [{ ...snapshot, revision: "two" }],
    });
    act(() => emitTauriEvent("model-selection-changed", { tools: ["codex"] }));
    await waitFor(() =>
      expect(second.result.current.data?.tools[0].revision).toBe("two"),
    );
    first.unmount();
    vi.mocked(modelSelectionApi.snapshot).mockResolvedValue({
      tools: [{ ...snapshot, revision: "three" }],
    });
    act(() => emitTauriEvent("model-selection-changed", { tools: ["codex"] }));
    await waitFor(() =>
      expect(second.result.current.data?.tools[0].revision).toBe("three"),
    );
    second.unmount();
    await act(async () => {});
    const calls = vi.mocked(modelSelectionApi.snapshot).mock.calls.length;
    emitTauriEvent("model-selection-changed", { tools: ["codex"] });
    expect(modelSelectionApi.snapshot).toHaveBeenCalledTimes(calls);
  });

  it("retains separate Codex and ChatGPT observations in a single read", async () => {
    renderHook(() => useModelSelection(["chatgpt", "codex", "chatgpt"]));
    await waitFor(() =>
      expect(modelSelectionApi.snapshot).toHaveBeenCalledWith([
        "chatgpt",
        "codex",
      ]),
    );
  });
  it.each([
    "ofox-prefs-updated",
    "ofox-auth-restored",
    "ofox-auth-expired",
    "ofox-apex-changed",
  ])("refreshes snapshots after lifecycle event %s", async (name) => {
    const reader = renderHook(() => useModelSelection(["codex"]));
    await waitFor(() =>
      expect(reader.result.current.data?.tools[0].revision).toBe("one"),
    );
    vi.mocked(modelSelectionApi.snapshot).mockResolvedValue({
      tools: [{ ...snapshot, revision: "new-state" }],
    });
    act(() =>
      emitTauriEvent(name, name === "ofox-apex-changed" ? "ofox.io" : {}),
    );
    await waitFor(() =>
      expect(reader.result.current.data?.tools[0].revision).toBe("new-state"),
    );
  });
  it("reads an externally changed CLI model when the window regains focus", async () => {
    const reader = renderHook(() => useModelSelection(["codex"]));
    await waitFor(() =>
      expect(reader.result.current.data?.tools[0].revision).toBe("one"),
    );
    vi.mocked(modelSelectionApi.snapshot).mockResolvedValue({
      tools: [{ ...snapshot, revision: "cli-change" }],
    });
    act(() => window.dispatchEvent(new Event("focus")));
    await waitFor(() =>
      expect(reader.result.current.data?.tools[0].revision).toBe("cli-change"),
    );
  });
  it("retains native Anthropic-only models and uncertain metadata while omitting image generators", () => {
    const model = {
      id: "native/model",
      name: null,
      ownedBy: null,
      pricingPrompt: null,
      supportedEndpoints: ["/anthropic/v1/messages"],
      supportedParameters: ["tools"],
      inputModalities: ["text"],
      outputModalities: ["text"],
    } satisfies FetchedModel;
    const unknown = {
      ...model,
      id: "unknown",
      supportedEndpoints: null,
      supportedParameters: null,
      outputModalities: null,
    };
    expect(
      filterModelsForNativeTool([
        model,
        unknown,
        { ...model, id: "image", outputModalities: ["image"] },
      ]).map((item) => item.id),
    ).toEqual(["native/model", "unknown"]);
  });
});
