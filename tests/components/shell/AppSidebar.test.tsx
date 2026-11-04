import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { ShellFrame } from "@/components/shell/ShellFrame";
import {
  SHELL_PAGES,
  ShellProvider,
  shellPanelId,
  useShell,
  useShellSubnav,
} from "@/components/shell/ShellContext";
import type { SidebarAccountState } from "@/components/shell/SidebarAccount";

// The left sidebar: grouped pages, the open page's sub-pages under it,
// collapsible to icons with a flyout per page.

const mocks = vi.hoisted(() => ({
  mac: true,
  openSettings: vi.fn(),
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
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

beforeEach(async () => {
  localStorage.clear();
  mocks.mac = true;
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

const local: SidebarAccountState = {
  kind: "local",
  signedIn: false,
  onAction: () => undefined,
};

/** The usage page publishes two sub-pages, like the real one. */
function UsagePanel() {
  const [tab, setTab] = useState<"overview" | "requests">("overview");
  useShellSubnav("usage", {
    items: [
      { value: "overview", label: "Overview", controls: "usage-overview" },
      { value: "requests", label: "Requests", count: 7, controls: "usage-req" },
    ],
    value: tab,
    onChange: setTab,
  });
  return <p data-testid="usage-tab">{tab}</p>;
}

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
          {page === "usage" && <UsagePanel />}
        </div>
      ))}
    </>
  );
}

function frame() {
  return render(
    <ShellProvider openSettings={mocks.openSettings}>
      <ShellFrame account={local} banner={null}>
        <Pages />
      </ShellFrame>
    </ShellProvider>,
  );
}

const pageTabs = () => screen.getByRole("tablist", { name: "Pages" });

describe("AppSidebar", () => {
  it("shows the two groups with pages in order", () => {
    frame();
    const nav = screen.getByRole("navigation", { name: "Pages" });
    expect(within(nav).getByText("Use")).toBeVisible();
    expect(within(nav).getByText("Observe")).toBeVisible();
    expect(
      within(pageTabs())
        .getAllByRole("tab")
        .map((tab) => tab.textContent),
    ).toEqual(["Tools", "Sessions", "Library", "Assistant", "Usage"]);
    for (const page of SHELL_PAGES)
      expect(
        within(pageTabs()).getByRole("tab", { name: en.shell.tabs[page] }),
      ).toHaveAttribute("aria-controls", shellPanelId(page));
  });

  it("clicking a page selects its panel", async () => {
    frame();
    await userEvent.click(screen.getByRole("tab", { name: "Library" }));
    expect(screen.getByRole("tab", { name: "Library" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tabpanel", { name: "library" })).toBeVisible();
  });

  it("active page uses the brand highlight", async () => {
    frame();
    expect(screen.getByRole("tab", { name: "Tools" }).className).toContain(
      "bg-brand-soft",
    );
    expect(
      screen.getByRole("tab", { name: "Sessions" }).className,
    ).not.toContain("bg-brand-soft");
  });

  it("shows only the active page's sub-pages and switches them", async () => {
    frame();
    // Usage is not open: its sub-pages stay folded away.
    expect(screen.queryByRole("tablist", { name: "Usage" })).toBeNull();
    await userEvent.click(screen.getByRole("tab", { name: "Usage" }));
    const sub = screen.getByRole("tablist", { name: "Usage" });
    const requests = within(sub).getByRole("tab", { name: /Requests/ });
    expect(requests).toHaveAttribute("aria-controls", "usage-req");
    expect(requests).toHaveTextContent("7");
    await userEvent.click(requests);
    expect(screen.getByTestId("usage-tab")).toHaveTextContent("requests");
    expect(requests).toHaveAttribute("aria-selected", "true");
    expect(requests.className).toContain("text-brand-ink");
  });

  it("collapse hides labels, remembers it, and expand restores", async () => {
    const view = frame();
    await userEvent.click(
      screen.getByRole("button", { name: "Collapse sidebar" }),
    );
    expect(within(pageTabs()).queryByText("Library")).toBeNull();
    expect(screen.getByRole("tab", { name: "Library" })).toBeInTheDocument();
    view.unmount();
    frame();
    expect(within(pageTabs()).queryByText("Library")).toBeNull();
    await userEvent.click(
      screen.getByRole("button", { name: "Expand sidebar" }),
    );
    expect(within(pageTabs()).getByText("Library")).toBeVisible();
  });

  it("collapsed: focusing an icon opens its flyout, ArrowRight + Enter switches the sub-page, Escape closes and focus returns to the icon", async () => {
    localStorage.setItem("ofox.shell.sidebarCollapsed", "1");
    frame();
    await userEvent.click(screen.getByRole("tab", { name: "Usage" }));
    const usage = screen.getByRole("tab", { name: "Usage" });
    act(() => usage.focus());
    const flyout = screen.getByRole("group", { name: "Usage" });
    expect(within(flyout).getByText("Usage")).toBeVisible();
    await userEvent.keyboard("{ArrowRight}");
    const overview = within(flyout).getByRole("button", { name: "Overview" });
    expect(overview).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}");
    expect(
      within(flyout).getByRole("button", { name: /Requests/ }),
    ).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    expect(screen.getByTestId("usage-tab")).toHaveTextContent("requests");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("group", { name: "Usage" })).toBeNull();
    expect(usage).toHaveFocus();
  });

  it("collapsed: a page without sub-pages still names itself on hover", async () => {
    localStorage.setItem("ofox.shell.sidebarCollapsed", "1");
    frame();
    await userEvent.hover(screen.getByRole("tab", { name: "Library" }));
    expect(
      within(screen.getByRole("group", { name: "Library" })).getByText(
        "Library",
      ),
    ).toBeVisible();
  });

  it("collapsed: one divider between the two groups, sub-pages aligned when expanded", async () => {
    localStorage.setItem("ofox.shell.sidebarCollapsed", "1");
    const view = frame();
    expect(
      screen
        .getByRole("tablist", { name: "Pages" })
        .querySelectorAll("span.h-px"),
    ).toHaveLength(1);
    view.unmount();
    localStorage.clear();
    frame();
    await userEvent.click(screen.getByRole("tab", { name: "Usage" }));
    expect(screen.getByRole("tablist", { name: "Usage" }).className).toContain(
      "ml-[26px]",
    );
  });

  it("settings opens through openSettings", async () => {
    frame();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(mocks.openSettings).toHaveBeenCalled();
  });

  it("update row shows when an update exists", () => {
    frame();
    expect(screen.getByRole("button", { name: /9\.9\.9/ })).toBeInTheDocument();
  });

  it("mac leaves room for the traffic lights and drags from the top row", () => {
    frame();
    const top = screen.getByTestId("sidebar-top");
    expect(top).toHaveAttribute("data-tauri-drag-region");
  });

  it("does not drag on other platforms", () => {
    mocks.mac = false;
    frame();
    expect(screen.getByTestId("sidebar-top")).not.toHaveAttribute(
      "data-tauri-drag-region",
    );
  });
});
