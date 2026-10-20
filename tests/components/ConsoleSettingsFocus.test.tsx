import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import ConsolePage from "@/components/console/ConsolePage";
import { ThemeProvider } from "@/components/theme-provider";

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: unknown) => void>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(null),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: unknown) => void) => {
    mocks.listeners.set(name, handler);
    return () => mocks.listeners.delete(name);
  }),
  emit: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@/lib/api", () => ({
  settingsApi: {
    getAutoLaunchStatus: vi.fn().mockResolvedValue(false),
    get: vi.fn().mockResolvedValue({}),
    openExternal: vi.fn().mockResolvedValue(undefined),
  },
}));
vi.mock("@/hooks/useOfoxAuth", () => ({
  useOfoxAuth: () => ({ status: { user: null }, refetch: vi.fn() }),
}));
vi.mock("@/hooks/useOfoxAccount", () => ({ useOfoxAccount: vi.fn() }));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.ai" }),
}));
vi.mock("@/lib/updater", () => ({
  getCurrentVersion: vi.fn().mockResolvedValue("1.3.6"),
}));
vi.mock("@/contexts/UpdateContext", () => ({
  useUpdate: () => ({
    hasUpdate: false,
    shouldPrompt: false,
    markPrompted: vi.fn(),
  }),
}));
vi.mock("@/components/console/SettingsHealthBanner", () => ({
  SettingsHealthBanner: () => null,
}));
vi.mock("@/components/console/ToolsPanel", () => ({ ToolsPanel: () => null }));
vi.mock("@/components/sessions/SessionsPanel", () => ({
  SessionsPanel: () => null,
}));
vi.mock("@/components/library/LibraryPage", () => ({
  LibraryPage: () => null,
}));
vi.mock("@/components/usage-overview/UsagePage", () => ({
  UsagePage: () => null,
}));
vi.mock("@/components/tools/OptionalToolsCatalog", async () => {
  const { forwardRef } = await import("react");
  return { OptionalToolsCatalog: forwardRef(() => null) };
});
vi.mock("@/components/ConfirmDialog", () => ({ ConfirmDialog: () => null }));
vi.mock("@/components/UserAvatar", () => ({ UserAvatar: () => null }));
vi.mock("@/components/OfoxApexSwitch", () => ({ OfoxApexSwitch: () => null }));

beforeEach(async () => {
  mocks.listeners.clear();
  localStorage.clear();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  window.matchMedia = vi.fn().mockReturnValue({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  });
});

function fixture(active = true, mounted = true) {
  return (
    <ThemeProvider defaultTheme="light">
      <button>其他窗口操作</button>
      <div hidden={!active}>
        {mounted && <ConsolePage active={active} boundTools={[]} localOnly />}
      </div>
    </ThemeProvider>
  );
}

it.each(["工具", "会话", "资源库", "用量"])(
  "returns Escape focus to the settings opener from %s after changing theme and tabbing",
  async (page) => {
    render(fixture());
    await userEvent.click(screen.getByRole("tab", { name: page }));
    const opener = screen.getByRole("button", { name: "设置" });
    await userEvent.click(opener);
    await userEvent.click(await screen.findByRole("radio", { name: "深色" }));
    expect(document.documentElement).toHaveClass("dark");
    const autoLaunch = screen.getByRole("switch", { name: "开机自动启动" });
    await waitFor(() => expect(autoLaunch).toBeEnabled());
    await userEvent.tab();
    expect(autoLaunch).toHaveFocus();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(opener).toHaveFocus());
  },
);

it("returns close-button focus to the explicit opener even when a click did not focus it", async () => {
  render(fixture());
  const previous = screen.getByRole("button", { name: "其他窗口操作" });
  const opener = screen.getByRole("button", { name: "设置" });
  previous.focus();
  // WebKit pointer activation does not guarantee that a button gets focus.
  fireEvent.click(opener);
  await userEvent.click(await screen.findByRole("button", { name: "关闭" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await waitFor(() => expect(opener).toHaveFocus());
});

it("uses the persistent header button when a tray event opens settings", async () => {
  render(fixture());
  const opener = screen.getByRole("button", { name: "设置" });
  await waitFor(() =>
    expect(mocks.listeners.has("ofox-open-settings")).toBe(true),
  );
  act(() =>
    mocks.listeners.get("ofox-open-settings")!({
      event: "ofox-open-settings",
      payload: null,
    }),
  );
  await screen.findByRole("dialog");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(opener).toHaveFocus());
});

it("does not move focus into an inactive retained console when settings is dismissed", async () => {
  const view = render(fixture());
  const outside = screen.getByRole("button", { name: "其他窗口操作" });
  await userEvent.click(screen.getByRole("button", { name: "设置" }));
  await screen.findByRole("dialog");
  view.rerender(fixture(false));
  outside.focus();
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await act(async () => new Promise((resolve) => setTimeout(resolve, 20)));
  expect(outside).toHaveFocus();
});

it("does not restore a disconnected opener after the console unmounts", async () => {
  const view = render(fixture());
  const outside = screen.getByRole("button", { name: "其他窗口操作" });
  const opener = screen.getByRole("button", { name: "设置" });
  await userEvent.click(opener);
  await screen.findByRole("dialog");
  view.rerender(fixture(true, false));
  outside.focus();
  expect(opener).not.toBeInTheDocument();
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await act(async () => new Promise((resolve) => setTimeout(resolve, 20)));
  expect(outside).toHaveFocus();
});

it("ignores tray events while the retained console is inactive", async () => {
  render(fixture(false));
  const outside = screen.getByRole("button", { name: "其他窗口操作" });
  outside.focus();
  await waitFor(() =>
    expect(mocks.listeners.has("ofox-open-settings")).toBe(true),
  );
  act(() =>
    mocks.listeners.get("ofox-open-settings")!({
      event: "ofox-open-settings",
      payload: null,
    }),
  );
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(outside).toHaveFocus();
});
