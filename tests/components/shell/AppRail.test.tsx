import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { ShellFrame } from "@/components/shell/ShellFrame";
import {
  SHELL_PAGES,
  ShellProvider,
  shellPanelId,
  useShell,
} from "@/components/shell/ShellContext";

// The page navigation: in the top bar while the window is narrow, in a
// left rail once it is wide, with the pages themselves untouched.

const mocks = vi.hoisted(() => ({
  mac: true,
  update: {
    hasUpdate: true,
    isDismissed: false,
    updateInfo: { availableVersion: "9.9.9", downloadUrl: null },
    dismissUpdate: vi.fn(),
  },
}));
vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isMac: () => mocks.mac,
}));
vi.mock("@/contexts/UpdateContext", () => ({
  useUpdate: () => mocks.update,
}));
vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: vi.fn().mockResolvedValue(undefined) },
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));

const jsdomWidth = window.innerWidth;
const resizeTo = (width: number) =>
  act(() => {
    window.innerWidth = width;
    window.dispatchEvent(new Event("resize"));
  });
beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
afterEach(() => {
  window.innerWidth = jsdomWidth;
});

function Pages() {
  const shell = useShell();
  return (
    <>
      {SHELL_PAGES.map((page) => (
        <div
          key={page}
          role="tabpanel"
          id={shellPanelId(page)}
          aria-label={page}
          hidden={shell.active !== page}
        >
          {page === "tools" && <input aria-label="Tool search" />}
        </div>
      ))}
    </>
  );
}
function frame(width: number) {
  window.innerWidth = width;
  return render(
    <ShellProvider openSettings={() => undefined}>
      <ShellFrame banner={null}>
        <Pages />
      </ShellFrame>
    </ShellProvider>,
  );
}

describe("the page navigation", () => {
  it("puts the page tabs in the rail when the window is wide", () => {
    frame(1512);
    const tabs = screen.getByRole("tablist", { name: "Pages" });
    expect(tabs).toHaveAttribute("aria-orientation", "vertical");
    expect(screen.queryByRole("banner")).toBeNull();
    const rail = screen.getByRole("navigation", { name: "Pages" });
    expect(rail).toContainElement(tabs);
    for (const page of SHELL_PAGES)
      expect(
        within(tabs).getByRole("tab", { name: en.shell.tabs[page] }),
      ).toHaveAttribute("aria-controls", shellPanelId(page));
    // Refresh, settings and the update stay within reach.
    expect(
      within(rail).getByRole("button", { name: "Settings" }),
    ).toBeVisible();
    expect(within(rail).getByRole("button", { name: "Refresh" })).toBeVisible();
    expect(
      within(rail).getByRole("button", { name: /9\.9\.9/ }),
    ).toBeInTheDocument();
  });

  it("keeps the top bar when the window is narrow", () => {
    frame(1000);
    const tabs = screen.getByRole("tablist", { name: "Pages" });
    expect(tabs).toHaveAttribute("aria-orientation", "horizontal");
    expect(screen.getByRole("banner")).toContainElement(tabs);
    expect(screen.queryByRole("navigation", { name: "Pages" })).toBeNull();
  });

  it("keeps a page as it was when the layout switches", async () => {
    frame(1000);
    const input = screen.getByRole("textbox", { name: "Tool search" });
    await userEvent.type(input, "codex");
    resizeTo(1512);
    resizeTo(1000);
    const after = screen.getByRole("textbox", { name: "Tool search" });
    expect(after).toBe(input);
    expect(after).toHaveValue("codex");
  });

  it("moves the focus from a top-bar tab to the same page's rail tab", async () => {
    frame(1000);
    await userEvent.click(screen.getByRole("tab", { name: "Library" }));
    expect(screen.getByRole("tab", { name: "Library" })).toHaveFocus();
    resizeTo(1512);
    const tab = screen.getByRole("tab", { name: "Library" });
    expect(screen.getByRole("navigation", { name: "Pages" })).toContainElement(
      tab,
    );
    expect(tab).toHaveFocus();
  });

  it("arrows through the rail with up and down", async () => {
    frame(1512);
    const tools = screen.getByRole("tab", { name: "Tools" });
    tools.focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(screen.getByRole("tab", { name: "Sessions" })).toHaveFocus();
    expect(screen.getByRole("tab", { name: "Sessions" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await userEvent.keyboard("{ArrowUp}");
    expect(tools).toHaveFocus();
  });
});
