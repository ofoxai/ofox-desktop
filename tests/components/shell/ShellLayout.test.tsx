import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ReactNode } from "react";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import { useShellLayout } from "@/components/shell/useShellLayout";

// Whether the window is wide: the rail and the roomier pages from 1280px,
// back to the top bar below 1240px, and in between it stays as it was.

const jsdomWidth = window.innerWidth;
const resizeTo = (width: number) =>
  act(() => {
    window.innerWidth = width;
    window.dispatchEvent(new Event("resize"));
  });
afterEach(() => {
  window.innerWidth = jsdomWidth;
});

describe("the shell's layout", () => {
  it("is wide from 1280px and narrow again below 1240px", () => {
    window.innerWidth = 1000;
    const { result } = renderHook(() => useShellLayout());
    const seen: string[] = [result.current];
    for (const width of [1279, 1280, 1250, 1239, 1260]) {
      resizeTo(width);
      seen.push(result.current);
    }
    expect(seen).toEqual([
      "narrow",
      "narrow",
      "wide",
      "wide",
      "narrow",
      "narrow",
    ]);
  });

  it("reads the width it opens at", () => {
    window.innerWidth = 1512;
    const { result } = renderHook(() => useShellLayout());
    expect(result.current).toBe("wide");
  });

  it("is what the shell tells its pages", () => {
    window.innerWidth = 1512;
    const wrapper = ({ children }: { children: ReactNode }) => (
      <ShellProvider openSettings={() => undefined}>{children}</ShellProvider>
    );
    const { result } = renderHook(() => useShell().layout, { wrapper });
    expect(result.current).toBe("wide");
    resizeTo(1000);
    expect(result.current).toBe("narrow");
  });
});
