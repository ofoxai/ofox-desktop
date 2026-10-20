import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UninstallToolDialog } from "@/components/console/UninstallToolDialog";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), forget: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/bindTools", () => ({
  readBoundTools: () =>
    JSON.parse(localStorage.getItem("ofox-bound-tools") || "[]"),
  forgetUninstalledTool: mocks.forget,
}));
const preview = {
  tool: "codex",
  installed: true,
  source: "npm",
  command: "npm uninstall -g @openai/codex",
  removes: ["@openai/codex"],
  keeps: ["~/.codex"],
  manualCommand: "npm uninstall -g @openai/codex",
  bound: true,
  sharedWith: ["chatgpt"],
  key: "keepShared",
  blockedReason: null,
};
beforeEach(() => {
  vi.resetAllMocks();
  localStorage.setItem("ofox-bound-tools", '["codex","chatgpt"]');
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  mocks.invoke.mockImplementation(async (command) =>
    command === "uninstall_tool_preview"
      ? preview
      : {
          unbind: {},
          key: "keptShared",
          status: "stillPresent",
          remainingPath: "/sandbox/codex",
          kept: ["~/.codex"],
        },
  );
});
it("explains shared data and key retention, then keeps partial results and a copyable command", async () => {
  const changed = vi.fn();
  render(
    <UninstallToolDialog
      tool="codex"
      onOpenChange={vi.fn()}
      onCompleted={changed}
    />,
  );
  expect(await screen.findByText(/保留：其他工具仍在使用/)).toBeVisible();
  expect(screen.getByText("~/.codex")).toBeVisible();
  expect(mocks.invoke).toHaveBeenCalledWith("uninstall_tool_preview", {
    tool: "codex",
    stillBound: ["codex", "chatgpt"],
  });
  fireEvent.click(screen.getByRole("button", { name: "卸载" }));
  expect(
    await screen.findByText("程序仍存在，请完成以下手动步骤"),
  ).toBeVisible();
  expect(screen.getByText("/sandbox/codex")).toBeVisible();
  expect(screen.getByRole("button", { name: "复制命令" })).toBeVisible();
  await waitFor(() => expect(changed).toHaveBeenCalled());
});
it("explains blocked removal without offering execution", async () => {
  mocks.invoke.mockResolvedValue({ ...preview, blockedReason: "appRunning" });
  render(
    <UninstallToolDialog
      tool="codex"
      onOpenChange={vi.fn()}
      onCompleted={vi.fn()}
    />,
  );
  expect(await screen.findByText("请先退出工具，再重试")).toBeVisible();
  expect(screen.queryByRole("button", { name: "卸载" })).toBeNull();
});

it("offers a fresh uninstall preview from a tool row only when supported", async () => {
  const { default: BoundToolRow } = await import(
    "@/components/console/BoundToolRow"
  );
  const userEvent = (await import("@testing-library/user-event")).default;
  const open = vi.fn();
  const props = {
    tool: {
      id: "codex",
      abbr: "CX",
      label: "Codex",
      color: "gray",
      version: "1",
      installationStatus: "installed" as const,
      installationError: null,
      binding: {
        status: "configured" as const,
        envOverrides: [],
        message: null,
        missingFiles: [],
        modifiedFields: [],
      },
    },
    hasAnalytics: false,
    updateTo: null,
    canInstall: true,
    busy: false,
    onAnalytics: vi.fn(),
    onUpdate: vi.fn(),
    onAction: vi.fn(),
    onManage: vi.fn(),
    onUninstall: open,
  };
  const view = render(<BoundToolRow {...props} />);
  await userEvent.click(screen.getByRole("button", { name: /更多/ }));
  await userEvent.click(await screen.findByRole("menuitem", { name: "卸载…" }));
  expect(open).toHaveBeenCalledOnce();
  view.unmount();
  mocks.invoke.mockRejectedValue("Command uninstall_tool_preview not found");
  render(<BoundToolRow {...props} />);
  await userEvent.click(screen.getByRole("button", { name: /更多/ }));
  await waitFor(() => expect(mocks.invoke).toHaveBeenCalled());
  expect(await screen.findByText("暂时无法读取卸载信息")).toBeVisible();
  expect(screen.getByRole("menuitem", { name: "卸载…" })).toHaveAttribute(
    "data-disabled",
  );
});

it("prevents escape and duplicate confirmation while removal is running", async () => {
  const close = vi.fn();
  mocks.invoke.mockImplementation((command) =>
    command === "uninstall_tool_preview"
      ? Promise.resolve(preview)
      : new Promise(() => {}),
  );
  render(
    <UninstallToolDialog
      tool="codex"
      onOpenChange={close}
      onCompleted={vi.fn()}
    />,
  );
  const confirm = await screen.findByRole("button", { name: "卸载" });
  fireEvent.click(confirm);
  fireEvent.click(confirm);
  await waitFor(() =>
    expect(
      mocks.invoke.mock.calls.filter(
        ([command]) => command === "uninstall_tool",
      ),
    ).toHaveLength(1),
  );
  fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
  expect(close).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "取消" })).toBeDisabled();
});

it("refreshes a partial result before offering a command for a remaining copy", async () => {
  let previews = 0;
  mocks.invoke.mockImplementation(async (command) =>
    command === "uninstall_tool_preview"
      ? ++previews === 1
        ? preview
        : {
            ...preview,
            bound: false,
            source: "homebrew",
            command: "brew uninstall --formula codex",
            manualCommand: "brew uninstall --formula codex",
          }
      : {
          unbind: {},
          key: "keptShared",
          status: "stillPresent",
          remainingPath: "/sandbox/homebrew/bin/codex",
          kept: ["~/.codex"],
        },
  );
  render(
    <UninstallToolDialog
      tool="codex"
      onOpenChange={vi.fn()}
      onCompleted={vi.fn()}
    />,
  );
  fireEvent.click(await screen.findByRole("button", { name: "卸载" }));
  expect(
    await screen.findByText("brew uninstall --formula codex"),
  ).toBeVisible();
  expect(screen.queryByText("npm uninstall -g @openai/codex")).toBeNull();
});

it("hides the old installer command while a partial result is being persisted", async () => {
  mocks.forget.mockImplementation(() => new Promise(() => {}));
  render(
    <UninstallToolDialog
      tool="codex"
      onOpenChange={vi.fn()}
      onCompleted={vi.fn()}
    />,
  );
  fireEvent.click(await screen.findByRole("button", { name: "卸载" }));
  await screen.findByText("程序仍存在，请完成以下手动步骤");
  expect(screen.queryByText("npm uninstall -g @openai/codex")).toBeNull();
});
