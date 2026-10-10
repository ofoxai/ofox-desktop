import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import SetupCompletePage from "@/components/onboarding/SetupCompletePage";

const platform = vi.hoisted(() => ({ mac: true }));

vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isMac: () => platform.mac,
}));

describe("SetupCompletePage tray hint", () => {
  it("points to the menu bar on macOS", () => {
    platform.mac = true;
    render(<SetupCompletePage boundCount={2} onOpenConsole={() => {}} />);
    expect(screen.getByText(/菜单栏的/)).toBeVisible();
  });

  it("points to the taskbar notification area elsewhere", () => {
    platform.mac = false;
    render(<SetupCompletePage boundCount={2} onOpenConsole={() => {}} />);
    expect(screen.getByText(/任务栏通知区域的/)).toBeVisible();
    expect(screen.queryByText(/菜单栏/)).toBeNull();
  });
});
