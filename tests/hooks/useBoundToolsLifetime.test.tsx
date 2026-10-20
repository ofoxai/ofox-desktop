import { act, renderHook } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { useBoundTools } from "@/hooks/useBoundTools";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  reconcile: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@/lib/api/ofoxAuth", () => ({
  ofoxReconcileToolKeys: mocks.reconcile,
}));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.ai" }),
}));

it("disposes a prefs subscription that arrives after local management replaces the bound panel", async () => {
  vi.useFakeTimers();
  let callback!: () => void;
  let resolve!: (off: () => void) => void;
  const off = vi.fn();
  mocks.listen.mockImplementation((_name, cb) => {
    callback = cb;
    return new Promise((r) => {
      resolve = r;
    });
  });
  mocks.invoke.mockResolvedValue([]);
  mocks.reconcile.mockResolvedValue({
    switched: [],
    alreadyCurrent: [],
    skipped: [],
    failed: [],
  });
  const account = {
    loadUser: vi.fn().mockResolvedValue({ name: "fixture" }),
    retryUser: vi.fn(),
  };
  const { unmount } = renderHook(() => useBoundTools([], account));
  await act(async () => undefined);
  unmount();
  mocks.invoke.mockClear();
  mocks.reconcile.mockClear();
  await act(async () => resolve(off));
  await act(async () => {
    callback();
    await vi.advanceTimersByTimeAsync(500);
  });
  expect(off).toHaveBeenCalledOnce();
  expect(mocks.invoke).not.toHaveBeenCalled();
  expect(mocks.reconcile).not.toHaveBeenCalled();
  vi.useRealTimers();
});
