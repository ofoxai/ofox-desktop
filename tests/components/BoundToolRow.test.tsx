import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import BoundToolRow, {
  TOOL_LIST_GRID,
} from "@/components/console/BoundToolRow";
import { ListCard } from "@/components/ui/list";
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
  ["zh", "配置"],
  ["en", "Configure"],
  ["ja", "設定"],
])(
  "points model-recovery guidance at the available %s configuration action",
  async (language, label) => {
    await i18n.changeLanguage(language);
    render(<BoundToolRow {...props()} />);
    const configure = screen.getByRole("button", { name: label });
    expect(
      i18n.t("modelCompatibility.defaultModelFailed", { tool: "Claude Code" }),
    ).toContain(configure.getAttribute("aria-label"));
  },
);

it.each([
  ["zh", "用量明细"],
  ["en", "Usage details"],
  ["ja", "使用量明細"],
])(
  "hides account usage until More opens and localizes its menu entry in %s",
  async (language, name) => {
    await i18n.changeLanguage(language);
    const input = props();
    render(<BoundToolRow {...input} />);
    expect(screen.queryByRole("button", { name })).toBeNull();
    expect(screen.queryByRole("menuitem", { name })).toBeNull();
    await userEvent.click(
      screen.getByRole("button", { name: i18n.t("toolLifecycle.more") }),
    );
    const link = screen.getByRole("menuitem", { name });
    expect(link).toBeVisible();
    expect(link).toHaveAttribute(
      "title",
      i18n.t("toolLifecycle.analyticsHint"),
    );
    expect(link).toHaveAccessibleDescription(
      i18n.t("toolLifecycle.analyticsHint"),
    );
    await userEvent.click(link);
    expect(input.onAnalytics).toHaveBeenCalledOnce();
    expect(mocks.invoke).not.toHaveBeenCalled();
  },
);

it("opens usage from More by keyboard and restores focus to the long-name row trigger", async () => {
  const input = props();
  input.tool.label = `Claude ${"很长的工具名称".repeat(30)}`;
  render(
    <div style={{ width: 900 }}>
      <BoundToolRow
        {...input}
        subline={["key-name-".repeat(20), "model-name-".repeat(20)]}
      />
    </div>,
  );
  expect(screen.getByTitle(input.tool.label)).toBeVisible();
  await userEvent.tab();
  expect(screen.getByRole("button", { name: "打开" })).toHaveFocus();
  await userEvent.tab();
  expect(screen.getByRole("button", { name: "配置" })).toHaveFocus();
  await userEvent.tab();
  const more = screen.getByRole("button", { name: "更多操作" });
  expect(more).toHaveFocus();
  await userEvent.keyboard("{Enter}");
  const entry = screen.getByRole("menuitem", { name: "用量明细" });
  expect(entry).toHaveFocus();
  await userEvent.keyboard("{Escape}");
  expect(more).toHaveFocus();
  await userEvent.keyboard("{Enter}{Enter}");
  expect(input.onAnalytics).toHaveBeenCalledOnce();
  await waitFor(() => expect(more).toHaveFocus());
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it("keeps a disabled usage menu entry when no verified key exists", async () => {
  const input = props();
  render(<BoundToolRow {...input} hasAnalytics={false} />);
  await userEvent.click(screen.getByRole("button", { name: "更多操作" }));
  const link = screen.getByRole("menuitem", { name: "用量明细" });
  expect(link).toBeVisible();
  expect(link).toHaveAttribute("data-disabled");
  await userEvent.keyboard("{Enter}");
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
  expect(screen.getByRole("menuitem", { name: "用量明细" })).toBeVisible();
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

it.each(["notInstalled", "unknown"] as const)(
  "retains usage without uninstall preview when installation is %s",
  async (status) => {
    const input = props();
    render(
      <BoundToolRow
        {...input}
        tool={{ ...input.tool, installationStatus: status }}
        onUninstall={vi.fn()}
      />,
    );
    expect(screen.queryByRole("button", { name: "用量明细" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "更多操作" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "用量明细" }));
    expect(input.onAnalytics).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menuitem", { name: /卸载/ })).toBeNull();
    expect(mocks.invoke).not.toHaveBeenCalled();
  },
);

function inList(row: React.ReactElement) {
  return render(<ListCard className={TOOL_LIST_GRID}>{row}</ListCard>);
}

it("lays a row out on the list's shared columns: badge, name, model, actions", () => {
  inList(
    <BoundToolRow
      {...props()}
      subline={["claude on mac"]}
      modelControl={<button type="button">gpt</button>}
    />,
  );
  const row = screen.getByRole("group", { name: "Claude Code" });
  expect(row).toHaveClass("col-span-full", "grid-cols-subgrid");
  const cells = [...row.children].filter(
    (cell) => !cell.classList.contains("sr-only"),
  );
  expect(cells.map((cell) => cell.getAttribute("data-column"))).toEqual([
    "badge",
    "name",
    "model",
    "actions",
  ]);
  expect(within(cells[2] as HTMLElement).getByText("gpt")).toBeVisible();
});

it("draws the sub-line even with nothing on it, so every row is as tall", () => {
  inList(<BoundToolRow {...props()} />);
  const sub = screen
    .getByRole("group", { name: "Claude Code" })
    .querySelector("[data-line=sub]");
  expect(sub).not.toBeNull();
  expect(sub).toHaveClass("truncate");
});

it("joins the key and model notes on the sub-line, the whole of it in its title", () => {
  inList(
    <BoundToolRow
      {...props()}
      subline={["opencode on mac", "", "实际在用 gpt-6-luna"]}
    />,
  );
  const sub = screen.getByText("opencode on mac · 实际在用 gpt-6-luna");
  expect(sub).toHaveAttribute("title", "opencode on mac · 实际在用 gpt-6-luna");
});

it("keeps 配置 a quiet icon beside 更多操作, after the row's one bordered action", () => {
  inList(<BoundToolRow {...props()} />);
  const row = screen.getByRole("group", { name: "Claude Code" });
  const configure = within(row).getByRole("button", { name: "配置" });
  expect(configure).toHaveAttribute("title", "配置");
  expect(configure.textContent).toBe("");
  expect(
    within(row)
      .getAllByRole("button")
      .map((button) => button.getAttribute("aria-label") ?? button.textContent),
  ).toEqual(["打开", "配置", "更多操作"]);
});

it("keeps the upgrade pill on the name's line, so the row keeps its height", () => {
  inList(<BoundToolRow {...props()} updateTo="2.1.400" />);
  const name = screen
    .getByRole("group", { name: "Claude Code" })
    .querySelector("[data-line=name]") as HTMLElement;
  expect(name).not.toHaveClass("flex-wrap");
  expect(
    within(name).getByRole("button", { name: /升级到 v2\.1\.400/ }),
  ).toBeVisible();
});
