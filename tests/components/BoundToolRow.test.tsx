import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import BoundToolRow from "@/components/console/BoundToolRow";
import type { UninstallPreview } from "@/lib/api/toolUninstall";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
const preview: UninstallPreview = {
  tool: "claude",
  installed: true,
  source: "npm",
  command: "npm uninstall -g @anthropic-ai/claude-code",
  removes: ["@anthropic-ai/claude-code"],
  keeps: ["~/.claude"],
  manualCommand: null,
  bound: true,
  sharedWith: [],
  key: "revoke",
  blockedReason: null,
};
function props() {
  return {
    tool: {
      id: "claude",
      abbr: "C",
      label: "Claude Code",
      color: "",
      version: "2.1.301",
      installationStatus: "installed" as const,
      installationError: null,
      binding: {
        status: "configured" as const,
        message: null,
        missingFiles: [],
        modifiedFields: [],
        envOverrides: [],
      },
    },
    hasAnalytics: true,
    updateTo: null,
    canInstall: true,
    busy: false,
    onAnalytics: vi.fn(),
    onUpdate: vi.fn(),
    onAction: vi.fn(),
    onManage: vi.fn(),
  };
}
beforeEach(async () => {
  for (const [language, bundle] of Object.entries({ zh, en, ja }))
    i18n.addResourceBundle(language, "translation", bundle, true, true);
  await i18n.changeLanguage("zh");
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue(preview);
  localStorage.clear();
});

it.each([
  ["zh", "用量明细"],
  ["en", "Usage details"],
  ["ja", "使用量明細"],
])(
  "keeps the account usage link visible and localized in %s without opening a menu",
  async (language, name) => {
    await i18n.changeLanguage(language);
    const input = props();
    render(<BoundToolRow {...input} />);
    const link = screen.getByRole("button", { name });
    expect(link).toBeVisible();
    expect(link).toHaveAttribute(
      "title",
      i18n.t("toolLifecycle.analyticsHint"),
    );
    expect(link).toHaveAccessibleDescription(
      i18n.t("toolLifecycle.analyticsHint"),
    );
    expect(
      screen.queryByRole("button", { name: i18n.t("toolLifecycle.more") }),
    ).toBeNull();
    await userEvent.click(link);
    expect(input.onAnalytics).toHaveBeenCalledOnce();
    expect(mocks.invoke).not.toHaveBeenCalled();
  },
);

it("keeps long-name row actions directly reachable by keyboard without a More detour", async () => {
  const input = props();
  input.tool.label = `Claude ${"很长的工具名称".repeat(30)}`;
  render(
    <div style={{ width: 900 }}>
      <BoundToolRow
        {...input}
        keyLabel={"key-name-".repeat(20)}
        model={"model-name-".repeat(20)}
      />
    </div>,
  );
  expect(screen.getByTitle(input.tool.label)).toBeVisible();
  await userEvent.tab();
  expect(screen.getByRole("button", { name: "打开" })).toHaveFocus();
  await userEvent.tab();
  expect(screen.getByRole("button", { name: "管理" })).toHaveFocus();
  await userEvent.tab();
  expect(screen.getByRole("button", { name: "用量明细" })).toHaveFocus();
  await userEvent.keyboard("{Enter}");
  expect(input.onAnalytics).toHaveBeenCalledOnce();
});

it("retains the disabled account link when no verified key exists", async () => {
  const input = props();
  render(<BoundToolRow {...input} hasAnalytics={false} />);
  const link = screen.getByRole("button", { name: "用量明细" });
  expect(link).toBeVisible();
  expect(link).toBeDisabled();
  await userEvent.click(link);
  expect(input.onAnalytics).not.toHaveBeenCalled();
});

it("loads uninstall preview only on opening More, retaining a disabled loading entry", async () => {
  let resolve!: (value: UninstallPreview) => void;
  mocks.invoke.mockReturnValue(
    new Promise<UninstallPreview>((done) => {
      resolve = done;
    }),
  );
  const uninstall = vi.fn();
  render(<BoundToolRow {...props()} onUninstall={uninstall} />);
  await act(async () => undefined);
  expect(mocks.invoke).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "更多操作" }));
  const pending = screen.getByRole("menuitem", { name: /卸载/ });
  expect(pending).toHaveAttribute("data-disabled");
  expect(screen.getByText(zh.common.loading)).toBeVisible();
  expect(mocks.invoke).toHaveBeenCalledWith("uninstall_tool_preview", {
    tool: "claude",
    stillBound: [],
  });
  expect(screen.queryByRole("menuitem", { name: "用量明细" })).toBeNull();
  await act(async () => resolve(preview));
  const ready = await screen.findByRole("menuitem", { name: "卸载…" });
  expect(ready).not.toHaveAttribute("data-disabled");
  await userEvent.click(ready);
  await waitFor(() => expect(uninstall).toHaveBeenCalledOnce());
  expect(
    mocks.invoke.mock.calls.every(
      ([command]) => command === "uninstall_tool_preview",
    ),
  ).toBe(true);
});

it.each(["unsupported", "error"])(
  "retains a disabled uninstall explanation for %s instead of an empty menu",
  async (state) => {
    if (state === "unsupported")
      mocks.invoke.mockResolvedValue({
        ...preview,
        blockedReason: "unsupportedPlatform",
      });
    else mocks.invoke.mockRejectedValue(new Error("read failed"));
    const uninstall = vi.fn();
    render(<BoundToolRow {...props()} onUninstall={uninstall} />);
    await userEvent.click(screen.getByRole("button", { name: "更多操作" }));
    const text =
      state === "unsupported"
        ? zh.toolUninstall.blocked.unsupportedPlatform
        : "暂时无法读取卸载信息";
    expect(await screen.findByText(text)).toBeVisible();
    expect(screen.getByRole("menuitem", { name: /卸载/ })).toHaveAttribute(
      "data-disabled",
    );
    expect(uninstall).not.toHaveBeenCalled();
  },
);

it("omits More when a missing tool has no secondary action", () => {
  const input = props();
  render(
    <BoundToolRow
      {...input}
      tool={{ ...input.tool, installationStatus: "notInstalled" }}
      onUninstall={vi.fn()}
    />,
  );
  expect(screen.getByRole("button", { name: "用量明细" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "更多操作" })).toBeNull();
});
