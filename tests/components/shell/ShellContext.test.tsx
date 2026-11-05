import { act, render, renderHook, screen } from "@testing-library/react";
import { useState, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  SHELL_PAGES,
  ShellProvider,
  useShell,
  useShellSubnav,
} from "@/components/shell/ShellContext";

// The shell's state behind the sidebar: page order by group, the collapsed
// sidebar, and the sub-pages a page publishes.

const wrapper = ({ children }: { children: ReactNode }) => (
  <ShellProvider openSettings={() => undefined}>{children}</ShellProvider>
);

beforeEach(() => localStorage.clear());
afterEach(() => vi.restoreAllMocks());

function Publisher({ onChange }: { onChange: (v: string) => void }) {
  const [value, setValue] = useState("a");
  useShellSubnav("usage", {
    items: [
      { value: "a", label: "A", controls: "panel-a" },
      { value: "b", label: "B", count: 3, controls: "panel-b" },
    ],
    value,
    onChange: (next) => {
      setValue(next);
      onChange(next);
    },
  });
  return null;
}

function Probe() {
  const shell = useShell();
  const nav = shell.subnavs.usage;
  return (
    <div>
      <span data-testid="value">{nav ? nav.value : "none"}</span>
      <span data-testid="items">
        {nav
          ? nav.items.map((i) => `${i.label}${i.count ?? ""}`).join(",")
          : ""}
      </span>
      <button onClick={() => nav?.onChange("b")}>pick b</button>
    </div>
  );
}

describe("ShellContext", () => {
  it("SHELL_PAGES follows the groups", () => {
    expect(SHELL_PAGES).toEqual([
      "tools",
      "sessions",
      "library",
      "assistant",
      "usage",
    ]);
  });

  it("a page's subnav is visible to the shell and withdrawn on unmount", () => {
    const { rerender } = render(
      <ShellProvider openSettings={() => undefined}>
        <Publisher onChange={() => undefined} />
        <Probe />
      </ShellProvider>,
    );
    expect(screen.getByTestId("items")).toHaveTextContent("A,B3");
    expect(screen.getByTestId("value")).toHaveTextContent("a");
    rerender(
      <ShellProvider openSettings={() => undefined}>
        <Probe />
      </ShellProvider>,
    );
    expect(screen.getByTestId("value")).toHaveTextContent("none");
  });

  it("a page that remounts keeps its sub-pages in the shell", () => {
    const { rerender } = render(
      <ShellProvider openSettings={() => undefined}>
        <Publisher key="before" onChange={() => undefined} />
        <Probe />
      </ShellProvider>,
    );
    rerender(
      <ShellProvider openSettings={() => undefined}>
        <Publisher key="after" onChange={() => undefined} />
        <Probe />
      </ShellProvider>,
    );
    expect(screen.getByTestId("items")).toHaveTextContent("A,B3");
  });

  it("subnav onChange calls the page's latest callback", () => {
    const first = vi.fn();
    const second = vi.fn();
    const { rerender } = render(
      <ShellProvider openSettings={() => undefined}>
        <Publisher onChange={first} />
        <Probe />
      </ShellProvider>,
    );
    rerender(
      <ShellProvider openSettings={() => undefined}>
        <Publisher onChange={second} />
        <Probe />
      </ShellProvider>,
    );
    act(() => screen.getByText("pick b").click());
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith("b");
    expect(screen.getByTestId("value")).toHaveTextContent("b");
  });

  it("collapsed state survives a remount via localStorage", () => {
    const first = renderHook(() => useShell(), { wrapper });
    expect(first.result.current.sidebarCollapsed).toBe(false);
    act(() => first.result.current.setSidebarCollapsed(true));
    expect(first.result.current.sidebarCollapsed).toBe(true);
    first.unmount();
    const second = renderHook(() => useShell(), { wrapper });
    expect(second.result.current.sidebarCollapsed).toBe(true);
  });

  it("collapsed state works when localStorage throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const { result } = renderHook(() => useShell(), { wrapper });
    expect(result.current.sidebarCollapsed).toBe(false);
    act(() => result.current.setSidebarCollapsed(true));
    expect(result.current.sidebarCollapsed).toBe(true);
  });
});
