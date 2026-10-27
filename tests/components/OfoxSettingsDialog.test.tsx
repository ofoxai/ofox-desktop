import { ThemeProvider } from "@/components/theme-provider";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import OfoxSettingsDialog from "@/components/console/OfoxSettingsDialog";

const mocks = vi.hoisted(() => ({
  openExternal: vi.fn(),
  toastError: vi.fn(),
  toastInfo: vi.fn(),
  checkUpdates: vi.fn(),
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
    checkUpdates: mocks.checkUpdates,
  },
}));
vi.mock("@/lib/updater", () => ({
  getCurrentVersion: vi.fn().mockResolvedValue("1.3.6"),
}));
vi.mock("@/components/OfoxApexSwitch", () => ({ OfoxApexSwitch: () => null }));
vi.mock("@/components/UserAvatar", () => ({ UserAvatar: () => null }));
vi.mock("sonner", () => ({
  toast: { error: mocks.toastError, success: vi.fn(), info: mocks.toastInfo },
}));

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle("en", "translation", en, true, true);
  i18n.addResourceBundle("ja", "translation", ja, true, true);
  await i18n.changeLanguage("zh");
  window.matchMedia = vi.fn().mockReturnValue({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  });
  mocks.openExternal.mockReset();
  mocks.toastError.mockReset();
  mocks.toastInfo.mockReset();
  mocks.checkUpdates.mockReset();
  mocks.openExternal.mockResolvedValue(undefined);
});

describe("OfoxSettingsDialog about", () => {
  it.each(["en", "ja"])(
    "uses the active language after switching the open dialog: %s",
    async (locale) => {
      vi.spyOn(console, "error").mockImplementation(() => {});
      mocks.checkUpdates.mockRejectedValueOnce("updateRequiresMacOS:14.0");
      render(
        <ThemeProvider>
          <OfoxSettingsDialog open onOpenChange={vi.fn()} />
        </ThemeProvider>,
      );
      await screen.findByText("Ofox Desktop v1.3.6");
      await act(async () => {
        await i18n.changeLanguage(locale);
      });
      const resource = locale === "en" ? en : ja;
      await userEvent.click(
        screen.getByRole("button", { name: resource.uiCopy.checkUpdates }),
      );
      await waitFor(() =>
        expect(mocks.toastError).toHaveBeenCalledWith(
          resource.update.requiresMacOS.replace("{{version}}", "14.0"),
        ),
      );
      expect(mocks.openExternal).not.toHaveBeenCalled();
    },
  );

  it.each(["updateRequiresMacOS:14.0", "updateCompatibilityUnknown"])(
    "explains incompatible or unverifiable updates: %s",
    async (error) => {
      vi.spyOn(console, "error").mockImplementation(() => {});
      mocks.checkUpdates.mockRejectedValueOnce(error);
      render(
        <ThemeProvider>
          <OfoxSettingsDialog open onOpenChange={vi.fn()} />
        </ThemeProvider>,
      );
      await userEvent.click(
        await screen.findByRole("button", { name: zh.uiCopy.checkUpdates }),
      );
      const expected = error.startsWith("updateRequires")
        ? zh.update.requiresMacOS.replace("{{version}}", "14.0")
        : zh.update.compatibilityUnknown;
      await waitFor(() =>
        expect(mocks.toastError).toHaveBeenCalledWith(expected),
      );
      expect(mocks.toastInfo).not.toHaveBeenCalled();
      expect(mocks.openExternal).not.toHaveBeenCalled();
    },
  );

  it("links to the Ofox website of the current region", async () => {
    render(
      <ThemeProvider>
        <OfoxSettingsDialog open onOpenChange={vi.fn()} />
      </ThemeProvider>,
    );
    await screen.findByText("Ofox Desktop v1.3.6");

    await userEvent.click(screen.getByRole("button", { name: "Ofox 网站 ↗" }));

    expect(mocks.openExternal).toHaveBeenCalledWith("https://ofox.io");
  });

  it("says so when the website cannot be opened", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    mocks.openExternal.mockRejectedValueOnce(new Error("denied"));
    render(
      <ThemeProvider>
        <OfoxSettingsDialog open onOpenChange={vi.fn()} />
      </ThemeProvider>,
    );

    await userEvent.click(
      await screen.findByRole("button", { name: "Ofox 网站 ↗" }),
    );

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith("无法打开链接"),
    );
  });
});

it("switches appearance through the ThemeProvider", async () => {
  const { ThemeProvider } = await import("@/components/theme-provider");
  render(
    <ThemeProvider>
      <OfoxSettingsDialog open onOpenChange={vi.fn()} />
    </ThemeProvider>,
  );
  await userEvent.click(screen.getByRole("radio", { name: "深色" }));
  await waitFor(() => expect(document.documentElement).toHaveClass("dark"));
  expect(localStorage.getItem("ofox-switch-theme")).toBe("dark");
});
