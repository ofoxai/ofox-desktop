import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { PageHeader, ShellRefreshButton } from "@/components/shell/PageHeader";
import {
  ShellProvider,
  useShellRefresh,
  useShellSubnav,
} from "@/components/shell/ShellContext";

// A page's title row: name, one line on what it is for, its actions; on
// macOS it drags the window, since there is no title bar above it.

const mocks = vi.hoisted(() => ({ mac: true }));
vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isMac: () => mocks.mac,
}));

afterEach(() => {
  window.innerWidth = 1024;
});
beforeEach(async () => {
  mocks.mac = true;
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

describe("PageHeader", () => {
  it("renders an h1, description and actions", () => {
    render(
      <PageHeader
        title="Usage overview"
        description="What each tool used"
        actions={<button>Act</button>}
      />,
    );
    expect(
      screen.getByRole("heading", { level: 1, name: "Usage overview" }),
    ).toBeVisible();
    expect(screen.getByText("What each tool used")).toBeVisible();
    expect(screen.getByRole("button", { name: "Act" })).toBeVisible();
  });

  it("drags on mac, not elsewhere", () => {
    const { container, unmount } = render(<PageHeader title="Tools" />);
    expect(container.querySelector("[data-page-header]")).toHaveAttribute(
      "data-tauri-drag-region",
    );
    unmount();
    mocks.mac = false;
    const other = render(<PageHeader title="Tools" />);
    expect(
      other.container.querySelector("[data-page-header]"),
    ).not.toHaveAttribute("data-tauri-drag-region");
  });

  it("ShellRefreshButton runs the open page's refresh and shows busy", async () => {
    window.innerWidth = 1512;
    let finish = () => undefined as void;
    const refresh = vi.fn(
      () => new Promise<void>((resolve) => (finish = resolve)),
    );
    function Tools() {
      useShellRefresh("tools", refresh);
      return <ShellRefreshButton />;
    }
    render(
      <ShellProvider openSettings={() => undefined}>
        <Tools />
      </ShellProvider>,
    );
    const button = screen.getByRole("button", { name: "Refresh" });
    await userEvent.click(button);
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(button).toHaveAttribute("aria-busy", "true");
    await act(async () => finish());
    expect(button).not.toHaveAttribute("aria-busy");
  });

  it("is the old compact row in a narrow window: the sub-page tabs and the actions, no title, no refresh", () => {
    window.innerWidth = 1000;
    function Library() {
      useShellSubnav("library", {
        items: [
          { value: "mcp", label: "MCP", count: 2, controls: "p-mcp" },
          { value: "skills", label: "Skills", controls: "p-skills" },
        ],
        value: "mcp",
        onChange: () => undefined,
      });
      return (
        <PageHeader
          page="library"
          navLabel="Library"
          title="MCP servers"
          actions={
            <>
              <button>Discover</button>
              <ShellRefreshButton />
            </>
          }
        />
      );
    }
    render(
      <ShellProvider openSettings={() => undefined}>
        <Library />
      </ShellProvider>,
    );
    expect(screen.queryByRole("heading", { level: 1 })).toBeNull();
    expect(screen.getByRole("tab", { name: "MCP · 2" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("button", { name: "Discover" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Refresh" })).toBeNull();
  });
});
