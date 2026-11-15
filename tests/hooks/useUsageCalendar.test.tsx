import { act, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useUsageCalendar } from "@/hooks/useUsageCalendar";

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});
it("advances exactly at local midnight independently of collection settings", () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 21, 23, 59, 59));
  const rollover = vi.fn();
  renderHook(() => useUsageCalendar(Date.now(), true, rollover));
  act(() => vi.advanceTimersByTime(1000));
  expect(rollover).toHaveBeenCalledTimes(1);
  expect(rollover).toHaveBeenCalledWith(new Date(2026, 9, 22).getTime());
  act(() => window.dispatchEvent(new Event("focus")));
  expect(rollover).toHaveBeenCalledTimes(1);
});
it("calibrates after an inactive window resumes, while custom dates can stay fixed", () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 21, 23, 59, 59));
  const anchor = Date.now(),
    rollover = vi.fn();
  const view = renderHook(
    ({ enabled }) => useUsageCalendar(anchor, enabled, rollover),
    { initialProps: { enabled: false } },
  );
  act(() => vi.advanceTimersByTime(2 * 86_400_000));
  act(() => window.dispatchEvent(new Event("focus")));
  expect(rollover).not.toHaveBeenCalled();
  view.rerender({ enabled: true });
  expect(rollover).toHaveBeenCalledTimes(1);
  expect(rollover).toHaveBeenCalledWith(Date.now());
});
it("recalibrates when the system timezone changes without a collection tick", () => {
  vi.useFakeTimers();
  const offset = vi
    .spyOn(Date.prototype, "getTimezoneOffset")
    .mockReturnValue(0);
  const rollover = vi.fn();
  renderHook(() => useUsageCalendar(Date.now(), true, rollover));
  offset.mockReturnValue(60);
  act(() => window.dispatchEvent(new Event("focus")));
  expect(rollover).toHaveBeenCalledTimes(1);
});

it("permits a retry after the new-day read fails instead of consuming the boundary", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 21, 23, 59, 59));
  const rollover = vi
    .fn()
    .mockResolvedValueOnce(false)
    .mockResolvedValueOnce(true);
  renderHook(() => useUsageCalendar(Date.now(), true, rollover));
  vi.setSystemTime(new Date(2026, 9, 22, 0, 0, 1));
  await act(async () => window.dispatchEvent(new Event("focus")));
  expect(rollover).toHaveBeenCalledTimes(1);
  await act(async () => window.dispatchEvent(new Event("focus")));
  expect(rollover).toHaveBeenCalledTimes(2);
});

it("retries an uncommitted boundary after a pending read is hidden and discarded", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 21, 23, 59, 59));
  const anchor = Date.now();
  let finish!: (accepted: boolean) => void;
  const rollover = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise<boolean>((resolve) => {
          finish = resolve;
        }),
    )
    .mockResolvedValue(true);
  const view = renderHook(
    ({ enabled }) => useUsageCalendar(anchor, enabled, rollover),
    { initialProps: { enabled: true } },
  );
  vi.setSystemTime(new Date(2026, 9, 22, 0, 0, 1));
  await act(async () => window.dispatchEvent(new Event("focus")));
  expect(rollover).toHaveBeenCalledTimes(1);
  view.rerender({ enabled: false });
  await act(async () => finish(false));
  view.rerender({ enabled: true });
  expect(rollover).toHaveBeenCalledTimes(2);
});
