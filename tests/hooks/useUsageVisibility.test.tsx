import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useUsageVisibility } from "@/hooks/useUsageVisibility";
const mocks = vi.hoisted(() => ({
  visible: vi.fn(),
  minimized: vi.fn(),
  focus: undefined as (() => void) | undefined,
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isVisible: mocks.visible,
    isMinimized: mocks.minimized,
    onFocusChanged: async (callback: () => void) => {
      mocks.focus = callback;
      return () => undefined;
    },
    onResized: async () => () => undefined,
  }),
}));
beforeEach(() => {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    value: {},
    configurable: true,
  });
  mocks.focus = undefined;
  mocks.visible.mockReset().mockResolvedValue(true);
  mocks.minimized.mockReset().mockResolvedValue(false);
});
afterEach(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});
it("does not start visible-page work before the native window state is known", async () => {
  let release!: (value: boolean) => void;
  mocks.visible.mockImplementation(
    () =>
      new Promise((resolve) => {
        release = resolve;
      }),
  );
  const { result } = renderHook(() => useUsageVisibility(true));
  expect(result.current).toBe(false);
  await waitFor(() => expect(release).toBeTypeOf("function"));
  await act(async () => release(true));
  await waitFor(() => expect(result.current).toBe(true));
});
it("pauses for native minimization even while the document says visible", async () => {
  const { result, rerender } = renderHook(
    ({ active }) => useUsageVisibility(active),
    { initialProps: { active: true } },
  );
  await waitFor(() => expect(result.current).toBe(true));
  expect(document.hidden).toBe(false);
  mocks.minimized.mockResolvedValue(true);
  await act(async () => mocks.focus?.());
  await waitFor(() => expect(result.current).toBe(false));
  mocks.minimized.mockResolvedValue(false);
  mocks.visible.mockResolvedValue(false);
  await act(async () => mocks.focus?.());
  expect(result.current).toBe(false);
  rerender({ active: false });
  expect(result.current).toBe(false);
});

it("rechecks the actual native state at refresh time even before an event arrives", async () => {
  const { canRefreshUsageNow } = await import("@/hooks/useUsageVisibility");
  mocks.minimized.mockResolvedValue(true);
  expect(await canRefreshUsageNow()).toBe(false);
  mocks.minimized.mockResolvedValue(false);
  expect(await canRefreshUsageNow()).toBe(true);
  mocks.visible.mockRejectedValue(new Error("native unavailable"));
  expect(await canRefreshUsageNow()).toBe(false);
});
