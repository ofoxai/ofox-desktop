import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { AppHeader } from "@/components/shell/AppHeader";
import {
  ShellProvider,
  useShellRefresh,
  type ShellPage,
} from "@/components/shell/ShellContext";

const mocks = vi.hoisted(() => ({
  mac: true,
  update: {
    hasUpdate: false,
    isDismissed: false,
    updateInfo: null as null | {
      availableVersion: string;
      downloadUrl?: string | null;
    },
    dismissUpdate: vi.fn(),
  },
  openExternal: vi.fn().mockResolvedValue(undefined),
  toastError: vi.fn(),
}));

vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isMac: () => mocks.mac,
}));
vi.mock("@/contexts/UpdateContext", () => ({
  useUpdate: () => mocks.update,
}));
vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: mocks.openExternal },
}));
vi.mock("sonner", () => ({ toast: { error: mocks.toastError } }));

function Page({
  page,
  onRefresh,
}: {
  page: ShellPage;
  onRefresh: () => unknown;
}) {
  useShellRefresh(page, onRefresh);
  return null;
}

function renderHeader({
  refreshers = {},
  openSettings = vi.fn(),
}: {
  refreshers?: Partial<Record<ShellPage, () => unknown>>;
  openSettings?: () => void;
} = {}) {
  return render(
    <ShellProvider openSettings={openSettings}>
      <AppHeader />
      {Object.entries(refreshers).map(([page, fn]) => (
        <Page key={page} page={page as ShellPage} onRefresh={fn} />
      ))}
    </ShellProvider>,
  );
}

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  mocks.mac = true;
  mocks.update.hasUpdate = false;
  mocks.update.isDismissed = false;
  mocks.update.updateInfo = null;
});

describe("AppHeader navigation", () => {
  it("offers tools, sessions, library and usage in order with tools open first", () => {
    renderHeader();
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual([
      "工具",
      "会话",
      "资源库",
      "用量",
    ]);
    expect(screen.getByRole("tab", { name: "工具" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "用量" })).toHaveAttribute(
      "aria-controls",
      "shell-panel-usage",
    );
  });

  it("centres the tabs in a 46px header", () => {
    const { container } = renderHeader();
    expect(container.querySelector("header")).toHaveClass("h-[46px]");
    expect(screen.getByRole("tablist", { name: "页面" })).toHaveClass(
      "absolute",
      "left-1/2",
      "-translate-x-1/2",
    );
  });

  it("opens the page whose tab is clicked", async () => {
    renderHeader();
    await userEvent.click(screen.getByRole("tab", { name: "会话" }));
    expect(screen.getByRole("tab", { name: "会话" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });
});

describe("AppHeader title area", () => {
  it("leaves room for the traffic lights and shows the brand on macOS, as a drag region", () => {
    const { container } = renderHeader();
    const header = container.querySelector("header")!;
    expect(header).toHaveAttribute("data-tauri-drag-region");
    expect(header).toHaveClass("pl-[78px]");
    expect(screen.getByText("Ofox Desktop")).toBeVisible();
  });

  it("keeps the tabs and the buttons out of the macOS drag region", () => {
    mocks.update.hasUpdate = true;
    mocks.update.updateInfo = { availableVersion: "1.4.0" };
    const { container } = renderHeader();
    const header = container.querySelector("header")!;
    const tablist = screen.getByRole("tablist", { name: "页面" });
    const actions = screen.getByRole("button", { name: "设置" }).parentElement!;

    expect(tablist).toHaveClass("no-drag");
    expect(actions).toHaveClass("no-drag");
    expect(actions).toContainElement(
      screen.getByRole("button", { name: "新版本 v1.4.0" }),
    );
    for (const control of header.querySelectorAll("button")) {
      expect(control).not.toHaveAttribute("data-tauri-drag-region");
    }
    expect(tablist).not.toHaveAttribute("data-tauri-drag-region");
    expect(actions).not.toHaveAttribute("data-tauri-drag-region");
  });

  it("leaves the brand and dragging to the native title bar elsewhere", () => {
    mocks.mac = false;
    const { container } = renderHeader();
    const header = container.querySelector("header")!;
    expect(header).not.toHaveAttribute("data-tauri-drag-region");
    expect(header).not.toHaveClass("pl-[78px]");
    expect(screen.queryByText("Ofox Desktop")).toBeNull();
  });
});

describe("AppHeader refresh", () => {
  it("runs the open page's refresh and spins until it settles", async () => {
    let finish!: () => void;
    const tools = vi.fn(
      () => new Promise<void>((resolve) => (finish = resolve)),
    );
    renderHeader({ refreshers: { tools } });
    const refresh = screen.getByRole("button", { name: "刷新" });

    await userEvent.click(refresh);

    expect(tools).toHaveBeenCalledTimes(1);
    expect(refresh).toHaveAttribute("aria-busy", "true");
    expect(refresh.querySelector("svg")).toHaveClass("animate-spin");
    await userEvent.click(refresh);
    expect(tools).toHaveBeenCalledTimes(1);

    await act(async () => finish());
    expect(refresh).not.toHaveAttribute("aria-busy");
    expect(refresh.querySelector("svg")).not.toHaveClass("animate-spin");
  });

  it("refreshes the page that is open, not the others", async () => {
    const tools = vi.fn();
    const usage = vi.fn();
    renderHeader({ refreshers: { tools, usage } });

    await userEvent.click(screen.getByRole("tab", { name: "用量" }));
    await userEvent.click(screen.getByRole("button", { name: "刷新" }));

    expect(usage).toHaveBeenCalledTimes(1);
    expect(tools).not.toHaveBeenCalled();
  });

  it("cannot refresh a page that has nothing to refresh", async () => {
    renderHeader({ refreshers: { tools: vi.fn() } });
    expect(screen.getByRole("button", { name: "刷新" })).toBeEnabled();
    await userEvent.click(screen.getByRole("tab", { name: "会话" }));
    expect(screen.getByRole("button", { name: "刷新" })).toBeDisabled();
  });

  it("stops spinning and logs when a refresh fails", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const failure = new Error("offline");
    renderHeader({ refreshers: { tools: () => Promise.reject(failure) } });
    const refresh = screen.getByRole("button", { name: "刷新" });

    await userEvent.click(refresh);

    await waitFor(() => expect(refresh).not.toHaveAttribute("aria-busy"));
    expect(error).toHaveBeenCalledWith(
      "[Shell] refresh of tools failed",
      failure,
    );
    error.mockRestore();
  });
});

describe("AppHeader settings", () => {
  it("opens the settings", async () => {
    const openSettings = vi.fn();
    renderHeader({ openSettings });
    await userEvent.click(screen.getByRole("button", { name: "设置" }));
    expect(openSettings).toHaveBeenCalledTimes(1);
  });
});

describe("UpdatePill", () => {
  it("is absent without a new version or once it is skipped", () => {
    renderHeader();
    expect(screen.queryByRole("button", { name: /新版本/ })).toBeNull();
    mocks.update.hasUpdate = true;
    mocks.update.isDismissed = true;
    mocks.update.updateInfo = { availableVersion: "1.4.0" };
    renderHeader();
    expect(screen.queryByRole("button", { name: /新版本/ })).toBeNull();
  });

  it("names the new version and offers 立即更新 and 跳过此版本", async () => {
    mocks.update.hasUpdate = true;
    mocks.update.updateInfo = {
      availableVersion: "1.4.0",
      downloadUrl: "https://example.test/Ofox.dmg",
    };
    renderHeader();
    const pill = screen.getByRole("button", { name: "新版本 v1.4.0" });
    expect(pill).toHaveClass("bg-brand-soft");

    await userEvent.click(pill);
    await userEvent.click(screen.getByRole("menuitem", { name: "立即更新" }));
    expect(mocks.openExternal).toHaveBeenCalledWith(
      "https://example.test/Ofox.dmg",
    );

    await userEvent.click(pill);
    await userEvent.click(screen.getByRole("menuitem", { name: "跳过此版本" }));
    expect(mocks.update.dismissUpdate).toHaveBeenCalledTimes(1);
  });

  it("says so when the download page cannot be opened", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    mocks.openExternal.mockRejectedValueOnce(new Error("denied"));
    mocks.update.hasUpdate = true;
    mocks.update.updateInfo = {
      availableVersion: "1.4.0",
      downloadUrl: "https://example.test/Ofox.dmg",
    };
    renderHeader();

    await userEvent.click(
      screen.getByRole("button", { name: "新版本 v1.4.0" }),
    );
    await userEvent.click(screen.getByRole("menuitem", { name: "立即更新" }));

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith("无法打开下载页面"),
    );
  });
});
