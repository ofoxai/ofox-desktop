import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import ConsolePage from "@/components/console/ConsolePage";
import { checkToolUpdates } from "@/hooks/useToolUpdates";
import { toolUpdatesApi, type ToolUpdateInfo } from "@/lib/api/toolUpdates";

const mocks = vi.hoisted(() => ({
  mac: true,
  toast: Object.assign(() => undefined, {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
    warning: vi.fn(),
  }),
  invoke: vi.fn(),
  install: vi.fn(),
  openExternal: vi.fn().mockResolvedValue(undefined),
  onInstallDone: undefined as
    | ((toolId: string, code: number) => void)
    | undefined,
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isMac: () => mocks.mac,
}));
vi.mock("sonner", () => ({ toast: mocks.toast }));
vi.mock("@/lib/api/toolUpdates", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/toolUpdates")>()),
  toolUpdatesApi: { check: vi.fn(), update: vi.fn() },
}));
vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: mocks.openExternal },
}));
vi.mock("@/lib/api/proxy", () => ({
  proxyApi: { getProxyTakeoverStatus: vi.fn().mockResolvedValue({}) },
}));
vi.mock("@/lib/api/ofoxAuth", () => ({
  ofoxGetUserInfo: vi.fn().mockResolvedValue({ name: "Test User" }),
  isOfoxBillingManager: () => false,
}));
vi.mock("@/lib/api/manageTool", () => ({
  manageToolApi: { getActiveModel: vi.fn().mockResolvedValue("") },
}));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.ai" }),
}));
vi.mock("@/hooks/useToolLaunch", () => ({
  useToolLaunch: () => ({ launching: new Set(), launch: vi.fn() }),
}));
vi.mock("@/hooks/useToolInstall", () => ({
  useToolInstall: (onDone: (toolId: string, code: number) => void) => {
    mocks.onInstallDone = onDone;
    return {
      installing: new Set(),
      install: mocks.install,
      error: null,
      clearError: vi.fn(),
      progress: {},
    };
  },
}));
vi.mock("@/contexts/UpdateContext", () => ({
  useUpdate: () => ({
    hasUpdate: false,
    isDismissed: false,
    shouldPrompt: false,
    markPrompted: vi.fn(),
    dismissUpdate: vi.fn(),
  }),
}));
vi.mock("@/components/console/AddToolsDialog", () => ({ default: () => null }));
vi.mock("@/components/console/ManageToolDialog", () => ({
  default: ({ tool }: { tool: { label: string } | null }) =>
    tool ? <div role="dialog">管理：{tool.label}</div> : null,
}));
vi.mock("@/components/console/OfoxSettingsDialog", () => ({
  default: () => null,
}));
vi.mock("@/components/ConfirmDialog", () => ({ ConfirmDialog: () => null }));
vi.mock("@/components/UserAvatar", () => ({ UserAvatar: () => null }));
vi.mock("@/components/tools/ToolBadge", () => ({ ToolBadge: () => null }));

interface LocalTool {
  name: string;
  version: string | null;
  error: string | null;
  installationKind?: "cli" | "desktopApp";
  installationStatus?: "installed" | "notInstalled" | "unknown";
}

let localTools: LocalTool[];

function localTool(name: string, version: string | null): LocalTool {
  return { name, version, error: null, installationKind: "cli" };
}

function availableTool(
  name: string,
  version: string,
  latestVersion: string,
): ToolUpdateInfo {
  return {
    ...localTool(name, version),
    installationKind: "cli",
    latest_version: latestVersion,
    installationStatus: "installed",
    update_status: "available",
    update_source: "npm",
    update_supported: true,
    update_reason: null,
    executable_path: `/node/bin/${name}`,
  };
}

async function cacheUpdates(tools: ToolUpdateInfo[]) {
  vi.mocked(toolUpdatesApi.check).mockResolvedValue(tools);
  await checkToolUpdates();
  vi.mocked(toolUpdatesApi.check).mockClear();
}

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  localTools = [];
  mocks.mac = true;
  mocks.onInstallDone = undefined;
  vi.mocked(toolUpdatesApi.update).mockReset();
  mocks.toast.success.mockClear();
  mocks.toast.error.mockClear();
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "get_tool_versions") return localTools;
    if (command === "ofox_list_api_keys") return [];
    if (command === "get_tool_install_capabilities")
      return ["claude", "codex", "chatgpt"];
    if (command === "get_tool_binding_status")
      return {
        status: "configured",
        message: null,
        missingFiles: [],
        modifiedFields: [],
        envOverrides: [],
      };
    throw new Error(`Unexpected command: ${command}`);
  });
  await cacheUpdates([]);
});

describe("Console title bar", () => {
  it("draws its own title strip under the macOS overlay title bar", async () => {
    render(<ConsolePage boundTools={[]} />);
    expect(await screen.findByText("Ofox Desktop")).toBeVisible();
  });

  it("leaves the title to the native title bar elsewhere", async () => {
    mocks.mac = false;
    render(<ConsolePage boundTools={[]} />);
    await screen.findByText("Test User");
    expect(screen.queryByText("Ofox Desktop")).toBeNull();
  });
});

describe("Console tool update badges", () => {
  it("updates a tool in place from its row", async () => {
    const available = availableTool("codex", "0.160.0", "0.161.0");
    localTools = [localTool("codex", available.version)];
    await cacheUpdates([available]);
    vi.mocked(toolUpdatesApi.update).mockResolvedValue({
      status: "updated",
      before: "0.160.0",
      after: "0.161.0",
    });
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      { ...available, version: "0.161.0", update_status: "current" },
    ]);
    render(<ConsolePage boundTools={["codex"]} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "升级到 v0.161.0" }),
    );

    await waitFor(() =>
      expect(toolUpdatesApi.update).toHaveBeenCalledWith(
        "codex",
        expect.any(String),
      ),
    );
    await waitFor(() =>
      expect(mocks.toast.success).toHaveBeenCalledWith(
        "Codex 已升级：v0.160.0 → v0.161.0",
      ),
    );
  });

  it("offers one Update all for two tools behind and runs them in order", async () => {
    const available = [
      availableTool("claude", "2.1.292", "2.1.293"),
      availableTool("codex", "0.160.0", "0.161.0"),
    ];
    localTools = available.map(({ name, version }) => localTool(name, version));
    await cacheUpdates(available);
    vi.mocked(toolUpdatesApi.update).mockImplementation(async (tool) => ({
      status: "updated",
      before: tool === "claude" ? "2.1.292" : "0.160.0",
      after: tool === "claude" ? "2.1.293" : "0.161.0",
    }));
    vi.mocked(toolUpdatesApi.check).mockResolvedValue(
      available.map((tool) => ({
        ...tool,
        version: tool.latest_version,
        update_status: "current",
      })),
    );
    render(<ConsolePage boundTools={["claude", "codex"]} />);

    expect(
      await screen.findByText("2 个工具有新版本：Claude Code, Codex"),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "全部升级" }));

    await waitFor(() => expect(toolUpdatesApi.update).toHaveBeenCalledTimes(2));
    expect(
      vi.mocked(toolUpdatesApi.update).mock.calls.map(([tool]) => tool),
    ).toEqual(["claude", "codex"]);
    await waitFor(() =>
      expect(mocks.toast.success).toHaveBeenCalledWith(
        "已升级 2 个工具：Claude Code 2.1.292 → 2.1.293, Codex 0.160.0 → 0.161.0",
      ),
    );
  });

  it("leaves a single tool behind to its row without an Update all line", async () => {
    const available = availableTool("codex", "0.160.0", "0.161.0");
    localTools = [
      localTool("claude", "2.1.293"),
      localTool("codex", "0.160.0"),
    ];
    await cacheUpdates([available]);
    render(<ConsolePage boundTools={["claude", "codex"]} />);

    expect(
      await screen.findByRole("button", { name: "升级到 v0.161.0" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "全部升级" }),
    ).not.toBeInTheDocument();
  });

  it("rechecks updates when refreshing externally upgraded Claude and Codex", async () => {
    const available = [
      availableTool("claude", "2.1.292", "2.1.293"),
      availableTool("codex", "0.160.0", "0.161.0"),
    ];
    localTools = available.map(({ name, version }) => localTool(name, version));
    await cacheUpdates(available);
    render(<ConsolePage boundTools={["claude", "codex"]} />);
    expect(await screen.findByText("v2.1.292")).toBeVisible();
    expect(screen.getAllByRole("button", { name: /^升级到 v/ })).toHaveLength(
      2,
    );

    localTools = [
      localTool("claude", "2.1.293"),
      localTool("codex", "0.161.0"),
    ];
    vi.mocked(toolUpdatesApi.check).mockResolvedValue(
      available.map((tool) => ({
        ...tool,
        version: tool.latest_version,
        update_status: "current",
      })),
    );
    fireEvent.click(screen.getByRole("button", { name: "刷新" }));

    await waitFor(() => expect(toolUpdatesApi.check).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("v2.1.293")).toBeVisible();
    expect(screen.getByText("v0.161.0")).toBeVisible();
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: /^升级到 v/ }),
      ).not.toBeInTheDocument(),
    );
  });

  it("keeps an update badge when the refreshed installation still trails latest", async () => {
    const available = availableTool("claude", "2.1.292", "2.1.294");
    localTools = [localTool("claude", available.version)];
    await cacheUpdates([available]);
    render(<ConsolePage boundTools={["claude"]} />);
    expect(await screen.findByText("v2.1.292")).toBeVisible();

    localTools = [localTool("claude", "2.1.293")];
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      { ...available, version: "2.1.293" },
    ]);
    fireEvent.click(screen.getByRole("button", { name: "刷新" }));

    await waitFor(() => expect(toolUpdatesApi.check).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("v2.1.293")).toBeVisible();
    expect(
      await screen.findByRole("button", { name: /^升级到 v/ }),
    ).toBeVisible();
  });

  it("does not apply an old available result to a new local version while checking", async () => {
    const available = availableTool("codex", "0.160.0", "0.161.0");
    localTools = [localTool("codex", available.version)];
    await cacheUpdates([available]);
    render(<ConsolePage boundTools={["codex"]} />);
    expect(await screen.findByText("v0.160.0")).toBeVisible();
    expect(screen.getByRole("button", { name: /^升级到 v/ })).toBeVisible();

    let finishCheck!: (tools: ToolUpdateInfo[]) => void;
    vi.mocked(toolUpdatesApi.check).mockReturnValue(
      new Promise((resolve) => {
        finishCheck = resolve;
      }),
    );
    localTools = [localTool("codex", "0.161.0")];
    fireEvent.click(screen.getByRole("button", { name: "刷新" }));

    expect(await screen.findByText("v0.161.0")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: /^升级到 v/ }),
    ).not.toBeInTheDocument();
    await act(async () => {
      finishCheck([
        { ...available, version: "0.161.0", update_status: "current" },
      ]);
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "刷新" })).toBeEnabled(),
    );
  });

  it("does not use a desktop update result for a CLI with the same version", async () => {
    localTools = [localTool("codex", "0.161.0")];
    await cacheUpdates([
      {
        ...availableTool("codex", "0.161.0", "0.162.0"),
        installationKind: "desktopApp",
      },
    ]);
    render(<ConsolePage boundTools={["codex"]} />);
    expect(await screen.findByText("v0.161.0")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: /^升级到 v/ }),
    ).not.toBeInTheDocument();
  });

  it("rechecks updates after a successful installer repair", async () => {
    const available = availableTool("codex", "0.160.0", "0.161.0");
    localTools = [localTool("codex", available.version)];
    await cacheUpdates([available]);
    render(<ConsolePage boundTools={["codex"]} />);
    expect(await screen.findByText("v0.160.0")).toBeVisible();

    localTools = [localTool("codex", "0.161.0")];
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      { ...available, version: "0.161.0", update_status: "current" },
    ]);
    await act(async () => mocks.onInstallDone?.("codex", 0));

    await waitFor(() => expect(toolUpdatesApi.check).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("v0.161.0")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: /^升级到 v/ }),
    ).not.toBeInTheDocument();
  });

  it("rechecks a fresh cached result when returning to the window", async () => {
    const available = availableTool("codex", "0.160.0", "0.161.0");
    localTools = [localTool("codex", available.version)];
    await cacheUpdates([available]);
    render(<ConsolePage boundTools={["codex"]} />);
    expect(await screen.findByText("v0.160.0")).toBeVisible();
    expect(toolUpdatesApi.check).not.toHaveBeenCalled();

    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      { ...available, version: "0.161.0", update_status: "current" },
    ]);
    fireEvent.focus(window);

    await waitFor(() => expect(toolUpdatesApi.check).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: /^升级到 v/ }),
      ).not.toBeInTheDocument(),
    );
  });
});

describe("Bound tool lifecycle", () => {
  it("folds confirmed missing tools while retaining management and the bound count", async () => {
    localTools = [
      localTool("claude", "2.1.293"),
      {
        ...localTool("workbuddy", null),
        installationStatus: "notInstalled",
        installationKind: "desktopApp",
      },
    ];
    render(<ConsolePage boundTools={["claude", "workbuddy"]} />);
    expect(
      await screen.findByRole("group", { name: "Claude Code" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("group", { name: "WorkBuddy" }),
    ).not.toBeInTheDocument();
    const fold = screen.getByRole("button", { name: /未安装工具（1）/ });
    expect(fold).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByText("2")).toBeVisible();
    fireEvent.click(fold);
    const row = screen.getByRole("group", { name: "WorkBuddy" });
    expect(
      within(row)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["前往下载", "管理"]);
    fireEvent.click(within(row).getByRole("button", { name: "管理" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("管理：WorkBuddy");
    expect(mocks.invoke).not.toHaveBeenCalledWith(
      "ofox_unbind_tool",
      expect.anything(),
    );
  });

  it("keeps the missing section accessible when every bound app was deleted", async () => {
    localTools = [
      { ...localTool("workbuddy", null), installationStatus: "notInstalled" },
    ];
    render(<ConsolePage boundTools={["workbuddy"]} />);
    expect(await screen.findByText("暂无已安装的绑定工具")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /未安装工具（1）/ }));
    expect(screen.getByRole("group", { name: "WorkBuddy" })).toBeVisible();
  });

  it("does not infer uninstall from missing legacy version data or IPC failure", async () => {
    localTools = [localTool("codex", null)];
    render(<ConsolePage boundTools={["codex"]} />);
    const row = await screen.findByRole("group", { name: "Codex" });
    expect(within(row).getByText("检测失败")).toBeVisible();
    expect(
      within(row)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["重新检测", "管理"]);
    expect(
      screen.queryByRole("button", { name: /未安装工具/ }),
    ).not.toBeInTheDocument();
    mocks.invoke.mockRejectedValueOnce(new Error("IPC unavailable"));
    fireEvent.click(within(row).getByRole("button", { name: "重新检测" }));
    await waitFor(() =>
      expect(within(row).getByText("检测失败")).toBeVisible(),
    );
  });

  it("keeps a found but broken executable in the main list with repair beside manage", async () => {
    localTools = [
      {
        ...localTool("codex", null),
        installationStatus: "installed",
        error: "version process failed",
      },
    ];
    render(<ConsolePage boundTools={["codex"]} />);
    const row = await screen.findByRole("group", { name: "Codex" });
    await waitFor(() =>
      expect(
        within(row).getByRole("button", { name: "修复安装" }),
      ).toBeEnabled(),
    );
    expect(within(row).getByText("安装异常")).toBeVisible();
    expect(
      within(row)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["修复安装", "管理"]);
    fireEvent.click(within(row).getByRole("button", { name: "修复安装" }));
    expect(mocks.install).toHaveBeenCalledWith("codex");
  });

  it("offers explicit configuration recovery without automatically rebinding", async () => {
    localTools = [localTool("claude", "2.1.293")];
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(
      async (command: string, ...args: unknown[]) => {
        if (command === "get_tool_binding_status")
          return {
            status: "missing",
            message: null,
            missingFiles: ["~/.claude/settings.json"],
            modifiedFields: [],
            envOverrides: [],
          };
        return original(command, ...args);
      },
    );
    render(<ConsolePage boundTools={["claude"]} />);
    const row = await screen.findByRole("group", { name: "Claude Code" });
    expect(within(row).getByText("绑定配置缺失")).toBeVisible();
    expect(
      within(row)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["恢复绑定", "管理"]);
    fireEvent.click(within(row).getByRole("button", { name: "恢复绑定" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("管理：Claude Code");
    expect(
      mocks.invoke.mock.calls.some(
        ([command]) => command === "ofox_restore_tool_binding",
      ),
    ).toBe(false);
  });

  it("flags environment variables that bypass Ofox and counts them as needing attention", async () => {
    localTools = [localTool("gemini", "0.63.0")];
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(
      async (command: string, ...args: unknown[]) => {
        if (command === "get_tool_binding_status")
          return {
            status: "configured",
            message: null,
            missingFiles: [],
            modifiedFields: [],
            envOverrides: [
              { name: "GEMINI_API_KEY", scope: "machine", location: null },
            ],
          };
        return original(command, ...args);
      },
    );
    render(<ConsolePage boundTools={["gemini"]} />);
    const row = await screen.findByRole("group", { name: "Gemini" });
    expect(within(row).getByText("环境变量冲突")).toBeVisible();
    expect(screen.getByText("1 个需处理")).toBeVisible();
  });

  it("moves a manually reinstalled app back on window focus", async () => {
    localTools = [
      { ...localTool("workbuddy", null), installationStatus: "notInstalled" },
    ];
    render(<ConsolePage boundTools={["workbuddy"]} />);
    expect(
      await screen.findByRole("button", { name: /未安装工具（1）/ }),
    ).toBeVisible();
    localTools = [
      {
        ...localTool("workbuddy", "5.6.2"),
        installationStatus: "installed",
        installationKind: "desktopApp",
      },
    ];
    // Focus re-detects at most every 30 seconds.
    const startedAt = Date.now();
    vi.spyOn(Date, "now").mockReturnValue(startedAt + 31_000);
    fireEvent.focus(window);
    const row = await screen.findByRole("group", { name: "WorkBuddy" });
    expect(within(row).getByRole("button", { name: "打开" })).toBeVisible();
    expect(
      screen.queryByRole("button", { name: /未安装工具/ }),
    ).not.toBeInTheDocument();
    expect(
      mocks.invoke.mock.calls.some(([command]) => command === "ofox_bind_tool"),
    ).toBe(false);
  });

  it("detects only the bound tools", async () => {
    localTools = [localTool("claude", "2.1.293")];
    render(<ConsolePage boundTools={["claude"]} />);
    await screen.findByRole("group", { name: "Claude Code" });
    expect(mocks.invoke).toHaveBeenCalledWith(
      "get_tool_versions",
      expect.objectContaining({ tools: ["claude"] }),
    );
  });

  it("refreshes on window focus in the background without covering the list", async () => {
    localTools = [localTool("claude", "2.1.293")];
    const original = mocks.invoke.getMockImplementation()!;
    render(<ConsolePage boundTools={["claude"]} />);
    const row = await screen.findByRole("group", { name: "Claude Code" });
    await waitFor(() =>
      expect(screen.queryByText("刷新中…")).not.toBeInTheDocument(),
    );

    let finish: (tools: LocalTool[]) => void = () => undefined;
    mocks.invoke.mockImplementation(
      async (command: string, ...args: unknown[]) =>
        command === "get_tool_versions"
          ? new Promise<LocalTool[]>((resolve) => (finish = resolve))
          : original(command, ...args),
    );
    const startedAt = Date.now();
    vi.spyOn(Date, "now").mockReturnValue(startedAt + 31_000);
    fireEvent.focus(window);

    await waitFor(() =>
      expect(
        mocks.invoke.mock.calls.filter(
          ([command]) => command === "get_tool_versions",
        ),
      ).toHaveLength(2),
    );
    expect(screen.queryByText("刷新中…")).not.toBeInTheDocument();
    expect(within(row).getByRole("button", { name: "打开" })).toBeEnabled();
    await act(async () => finish([localTool("claude", "2.1.294")]));
    expect(await within(row).findByText("v2.1.294")).toBeVisible();
  });

  it("does not re-detect when the window regains focus within 30 seconds", async () => {
    localTools = [localTool("claude", "2.1.293")];
    render(<ConsolePage boundTools={["claude"]} />);
    await screen.findByRole("group", { name: "Claude Code" });
    const detections = () =>
      mocks.invoke.mock.calls.filter(
        ([command]) => command === "get_tool_versions",
      ).length;
    const before = detections();
    fireEvent.focus(window);
    fireEvent.focus(window);
    await act(async () => undefined);
    expect(detections()).toBe(before);
  });

  it("uses upstream instructions when the host cannot automatically install a CLI", async () => {
    localTools = [
      { ...localTool("codex", null), installationStatus: "notInstalled" },
    ];
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(
      async (command: string, ...args: unknown[]) => {
        if (command === "get_tool_install_capabilities") return [];
        return original(command, ...args);
      },
    );
    render(<ConsolePage boundTools={["codex"]} />);
    fireEvent.click(
      await screen.findByRole("button", { name: /未安装工具（1）/ }),
    );
    fireEvent.click(screen.getByRole("button", { name: "安装说明" }));
    expect(mocks.openExternal).toHaveBeenCalledWith(
      "https://github.com/openai/codex",
    );
    expect(mocks.install).not.toHaveBeenCalled();
  });

  it("rejects an old scan that completes after a tool is unbound", async () => {
    localTools = [localTool("codex", "0.161.0")];
    const { rerender } = render(<ConsolePage boundTools={["codex"]} />);
    await screen.findByRole("group", { name: "Codex" });
    let finish!: (tools: LocalTool[]) => void;
    mocks.invoke.mockImplementationOnce(
      () =>
        new Promise<LocalTool[]>((resolve) => {
          finish = resolve;
        }),
    );
    fireEvent.click(screen.getByRole("button", { name: "刷新" }));
    rerender(<ConsolePage boundTools={[]} />);
    await screen.findByText("暂无绑定的工具");
    await act(async () => finish([localTool("codex", "0.161.0")]));
    expect(
      screen.queryByRole("group", { name: "Codex" }),
    ).not.toBeInTheDocument();
  });
});
