import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { QueryClientProvider } from "@tanstack/react-query";
import { queryClient } from "@/lib/query/queryClient";
import TrayPopoverApp from "@/components/tray/TrayPopoverApp";
import { BOUND_TOOLS_STORAGE_KEY } from "@/config/toolMeta";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  emit: vi.fn(),
  hide: vi.fn(),
  auth: "active" as "active" | "expired" | "loggedout",
  paused: [] as string[],
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async () => () => undefined,
  emit: mocks.emit,
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ hide: mocks.hide }),
}));
vi.mock("@tauri-apps/plugin-process", () => ({ exit: vi.fn() }));
vi.mock("@/contexts/UpdateContext", () => ({
  useUpdate: () => ({ hasUpdate: false, isDismissed: false }),
}));

const binding = {
  status: "configured",
  message: null,
  missingFiles: [],
  modifiedFields: [],
  envOverrides: [],
};
const selections: Record<string, unknown> = {
  opencode: { kind: "models", models: ["a/1", "a/2", "a/3"] },
  openclaw: { kind: "single", model: "deepseek/deepseek-v4.1-flash" },
  hermes: { kind: "single", model: "anthropic/claude-sonnet-5-5" },
};

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  queryClient.clear();
  localStorage.clear();
  mocks.auth = "active";
  mocks.paused = [];
  mocks.emit.mockReset().mockResolvedValue(undefined);
  mocks.hide.mockReset().mockResolvedValue(undefined);
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation(
    async (command: string, args: { tools?: string[] } = {}) => {
      if (command === "ofox_get_auth_status")
        return {
          state: mocks.auth,
          user:
            mocks.auth === "loggedout"
              ? null
              : {
                  email: "demo@example.com",
                  name: "Demo",
                  org_id: null,
                  avatar_url: null,
                  balance: { balance: 2243.54 },
                },
        };
      if (command === "get_settings") return {};
      if (command === "ofox_get_apex_state")
        return { apex: "ofox.ai", pinned: true, detected: true };
      if (command === "fetch_ofox_models") return [];
      if (command === "model_selection_snapshot")
        return {
          tools: (args.tools ?? []).map((toolId) => ({
            toolId,
            writeTarget: toolId,
            revision: `r-${toolId}`,
            binding: mocks.paused.includes(toolId)
              ? { ...binding, status: "paused" }
              : binding,
            managedSelection: selections[toolId],
            observedSelection: { model: null, source: "config" },
            scope: [],
          })),
        };
      return null;
    },
  );
});

function bind(...tools: string[]) {
  localStorage.setItem(BOUND_TOOLS_STORAGE_KEY, JSON.stringify(tools));
}

function renderTray() {
  return render(
    <QueryClientProvider client={queryClient}>
      <TrayPopoverApp />
    </QueryClientProvider>,
  );
}

it("puts the account and its balance on one line", async () => {
  bind("openclaw");
  renderTray();
  const account = await screen.findByTestId("tray-account");
  await waitFor(() => expect(account).toHaveTextContent("$2243.54"));
  expect(account).toHaveTextContent("demo@example.com");
});

it("says each bound tool's model in words", async () => {
  bind("opencode", "openclaw", "hermes");
  renderTray();
  const openclaw = await screen.findByRole("button", { name: /OpenClaw/ });
  await waitFor(() =>
    expect(openclaw).toHaveTextContent("deepseek/deepseek-v4.1-flash"),
  );
  expect(screen.getByRole("button", { name: /OpenCode/ })).toHaveTextContent(
    "3 个模型",
  );
});

it("opens one row at a time, its model field inside", async () => {
  bind("openclaw", "hermes");
  renderTray();
  const openclaw = await screen.findByRole("button", { name: /OpenClaw/ });
  await waitFor(() =>
    expect(openclaw).toHaveTextContent("deepseek/deepseek-v4.1-flash"),
  );
  fireEvent.click(openclaw);
  expect(openclaw).toHaveAttribute("aria-expanded", "true");
  const panel = document.getElementById(
    openclaw.getAttribute("aria-controls")!,
  )!;
  expect(within(panel).getByRole("combobox")).toHaveClass("w-full");
  const hermes = screen.getByRole("button", { name: /Hermes/ });
  fireEvent.click(hermes);
  expect(hermes).toHaveAttribute("aria-expanded", "true");
  expect(openclaw).toHaveAttribute("aria-expanded", "false");
});

it("says a paused tool's key is paused, not that it can't be read", async () => {
  bind("hermes");
  mocks.paused = ["hermes"];
  renderTray();
  const hermes = await screen.findByRole("button", { name: /Hermes/ });
  await waitFor(() => expect(hermes).toHaveTextContent("Key 已暂停"));
  expect(screen.queryByText("无法读取模型状态")).toBeNull();
});

it("offers one-click binding when nothing is bound", async () => {
  renderTray();
  fireEvent.click(await screen.findByRole("button", { name: "一键绑定工具" }));
  await waitFor(() =>
    expect(mocks.emit).toHaveBeenCalledWith("ofox-start-binding"),
  );
  expect(mocks.invoke).toHaveBeenCalledWith("show_main_window");
  expect(mocks.hide).toHaveBeenCalled();
});

it("asks a signed-out user to log in from the account line", async () => {
  mocks.auth = "loggedout";
  renderTray();
  const account = await screen.findByTestId("tray-account");
  fireEvent.click(await within(account).findByRole("button", { name: "登录" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("ofox_request_reauth"),
  );
});

it("remembers the tab it was left on", async () => {
  bind("openclaw");
  const first = renderTray();
  fireEvent.click(await screen.findByRole("radio", { name: "用量" }));
  expect(screen.getByRole("radio", { name: "用量" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  first.unmount();
  renderTray();
  expect(await screen.findByRole("radio", { name: "用量" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await act(async () => undefined);
});
