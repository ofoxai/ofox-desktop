import { act, renderHook } from "@testing-library/react";
import { expect, it } from "vitest";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";

it("opens usage for a stable session reference and does not let an older handoff consume a newer one", () => {
  const { result } = renderHook(useShell, {
    wrapper: ({ children }) => (
      <ShellProvider openSettings={() => undefined}>{children}</ShellProvider>
    ),
  });
  act(() => result.current.setActive("sessions"));
  act(() => result.current.locateUsage("manager-first"));
  const first = result.current.usageLocation!;
  expect(result.current.active).toBe("usage");
  expect(result.current.visited.has("sessions")).toBe(true);
  act(() => result.current.locateUsage("manager-second"));
  act(() => result.current.consumeUsageLocation(first.request));
  expect(result.current.usageLocation?.key).toBe("manager-second");
  act(() =>
    result.current.consumeUsageLocation(result.current.usageLocation!.request),
  );
  expect(result.current.usageLocation).toBeNull();
});

it("opens the selected library section without applying resources", () => {
  const { result } = renderHook(useShell, {
    wrapper: ({ children }) => (
      <ShellProvider openSettings={() => undefined}>{children}</ShellProvider>
    ),
  });
  act(() => result.current.locateLibrary("skills"));
  expect(result.current.active).toBe("library");
  expect(result.current.libraryLocation?.kind).toBe("skills");
});
