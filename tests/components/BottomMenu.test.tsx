import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import BottomMenu from "@/components/tray/BottomMenu";

const mocks = vi.hoisted(() => ({
  mac: true,
  invoke: vi.fn(),
  hide: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ emit: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ hide: mocks.hide }),
}));
vi.mock("@tauri-apps/plugin-process", () => ({ exit: vi.fn() }));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));
vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isMac: () => mocks.mac,
}));

beforeEach(() => {
  mocks.invoke.mockReset().mockResolvedValue(undefined);
  mocks.hide.mockReset().mockResolvedValue(undefined);
});

describe("tray BottomMenu open-main shortcut", () => {
  it("shows and handles ⌘O on macOS", async () => {
    mocks.mac = true;
    render(<BottomMenu />);
    expect(screen.getByText("⌘O")).toBeVisible();

    fireEvent.keyDown(window, { key: "o", ctrlKey: true });
    expect(mocks.invoke).not.toHaveBeenCalled();

    fireEvent.keyDown(window, { key: "o", metaKey: true });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("show_main_window"),
    );
  });

  it("shows and handles Ctrl+O on Windows", async () => {
    mocks.mac = false;
    render(<BottomMenu />);
    expect(screen.getByText("Ctrl+O")).toBeVisible();

    fireEvent.keyDown(window, { key: "O", ctrlKey: true });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("show_main_window"),
    );
  });
});
