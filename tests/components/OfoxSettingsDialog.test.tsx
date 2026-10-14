import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import OfoxSettingsDialog from "@/components/console/OfoxSettingsDialog";

const mocks = vi.hoisted(() => ({
  openExternal: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("@/hooks/useOfoxAuth", () => ({
  useOfoxAuth: () => ({
    status: { user: { name: "Test User", email: "test@example.com" } },
    refetch: vi.fn(),
  }),
}));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.io" }),
}));
vi.mock("@/lib/api", () => ({
  settingsApi: {
    getAutoLaunchStatus: vi.fn().mockResolvedValue(false),
    get: vi.fn().mockResolvedValue({}),
    openExternal: mocks.openExternal,
  },
}));
vi.mock("@/lib/updater", () => ({
  getCurrentVersion: vi.fn().mockResolvedValue("1.3.6"),
}));
vi.mock("@/components/OfoxApexSwitch", () => ({ OfoxApexSwitch: () => null }));
vi.mock("@/components/UserAvatar", () => ({ UserAvatar: () => null }));
vi.mock("sonner", () => ({
  toast: { error: mocks.toastError, success: vi.fn(), info: vi.fn() },
}));

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  mocks.openExternal.mockReset();
  mocks.openExternal.mockResolvedValue(undefined);
});

describe("OfoxSettingsDialog about", () => {
  it("links to the Ofox website of the current region", async () => {
    render(<OfoxSettingsDialog open onOpenChange={vi.fn()} />);
    await screen.findByText("Ofox Desktop v1.3.6");

    await userEvent.click(screen.getByRole("button", { name: "Ofox 网站 ↗" }));

    expect(mocks.openExternal).toHaveBeenCalledWith("https://ofox.io");
  });

  it("says so when the website cannot be opened", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    mocks.openExternal.mockRejectedValueOnce(new Error("denied"));
    render(<OfoxSettingsDialog open onOpenChange={vi.fn()} />);

    await userEvent.click(
      await screen.findByRole("button", { name: "Ofox 网站 ↗" }),
    );

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith("无法打开链接"),
    );
  });
});
