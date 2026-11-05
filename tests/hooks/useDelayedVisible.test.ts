import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useDelayedVisible } from "@/hooks/useDelayedVisible";

// A loading indicator that neither flashes for a quick load nor vanishes
// the moment it appears.

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

const advance = (ms: number) => act(() => void vi.advanceTimersByTime(ms));

describe("useDelayedVisible", () => {
  it("stays hidden for a load shorter than 200ms", () => {
    const { result, rerender } = renderHook(
      ({ active }) => useDelayedVisible(active),
      { initialProps: { active: true } },
    );
    advance(150);
    rerender({ active: false });
    advance(500);
    expect(result.current).toBe(false);
  });

  it("shows after 200ms", () => {
    const { result } = renderHook(() => useDelayedVisible(true));
    advance(199);
    expect(result.current).toBe(false);
    advance(1);
    expect(result.current).toBe(true);
  });

  it("once shown stays at least 600ms", () => {
    const { result, rerender } = renderHook(
      ({ active }) => useDelayedVisible(active),
      { initialProps: { active: true } },
    );
    advance(200);
    expect(result.current).toBe(true);
    advance(100);
    rerender({ active: false });
    advance(400);
    expect(result.current).toBe(true);
    advance(101);
    expect(result.current).toBe(false);
  });
});
