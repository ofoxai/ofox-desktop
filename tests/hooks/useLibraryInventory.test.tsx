import { useLayoutEffect } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { libraryApi, type LibraryInventory } from "@/lib/api/library";
import { useLibraryInventory } from "@/components/library/useLibraryInventory";

afterEach(() => vi.restoreAllMocks());
it("reports pending in the first committed frame after requesting a rescan", async () => {
  let finish!: (value: LibraryInventory) => void;
  vi.spyOn(libraryApi, "inventory").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  vi.spyOn(libraryApi, "discardInventory").mockResolvedValue(true);
  const frames: boolean[] = [];
  const { result } = renderHook(() => {
    const native = useLibraryInventory("skills", 1, "stable", true);
    useLayoutEffect(() => {
      frames.push(native.loading);
    });
    return native;
  });
  act(() =>
    finish({
      ticketId: "first",
      kind: "skills",
      expiresAtMs: Date.now() + 600000,
      sources: [],
      entries: [],
    }),
  );
  await waitFor(() => expect(result.current.inventory?.ticketId).toBe("first"));
  await waitFor(() => expect(result.current.loading).toBe(false));
  const before = frames.length;
  act(() => result.current.rescan());
  expect(frames.slice(before).length).toBeGreaterThan(0);
  expect(frames.slice(before).every(Boolean)).toBe(true);
});

it("leaves an expired ticket retryable when its consumer has not requested renewal", async () => {
  vi.useFakeTimers();
  try {
    let finish!: (value: LibraryInventory) => void;
    const scan = vi.spyOn(libraryApi, "inventory").mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    vi.spyOn(libraryApi, "discardInventory").mockResolvedValue(true);
    const frames: boolean[] = [];
    const { result } = renderHook(() => {
      const native = useLibraryInventory("mcp", 1, "stable", true);
      useLayoutEffect(() => {
        frames.push(native.loading);
      });
      return native;
    });
    await act(async () =>
      finish({
        ticketId: "short",
        kind: "mcp",
        expiresAtMs: Date.now() + 100,
        sources: [],
        entries: [],
      }),
    );
    expect(result.current.loading).toBe(false);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(101);
    });
    expect(result.current.expired).toBe(true);
    expect(result.current.loading).toBe(false);
    expect(scan).toHaveBeenCalledTimes(1);
    const before = frames.length;
    act(() => result.current.rescan());
    expect(frames.slice(before).every(Boolean)).toBe(true);
    expect(result.current.loading).toBe(true);
    expect(scan).toHaveBeenCalledTimes(2);
  } finally {
    vi.useRealTimers();
  }
});
