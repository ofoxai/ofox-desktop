import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { queryClient } from "@/lib/query/queryClient";
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
  reconcileKeys: vi.fn(),
  getUserInfo: vi.fn(),
  workBuddyModels: vi.fn(),
  chatGptModel: vi.fn(),
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
  ofoxGetUserInfo: mocks.getUserInfo,
  ofoxReconcileToolKeys: mocks.reconcileKeys,
  isOfoxBillingManager: () => false,
}));
vi.mock("@/lib/api/manageTool", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/manageTool")>()),
  manageToolApi: {
    getActiveModel: vi.fn().mockResolvedValue(""),
    getWorkBuddyManagedModels: mocks.workBuddyModels,
    getChatGptModel: mocks.chatGptModel,
  },
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
  default: ({ open }: { open: boolean }) =>
    open ? <div role="dialog">偏好设置</div> : null,
}));
vi.mock("@/components/ConfirmDialog", () => ({ ConfirmDialog: () => null }));
vi.mock("@/components/UserAvatar", () => ({ UserAvatar: () => null }));
vi.mock("@/components/tools/ToolBadge", () => ({ ToolBadge: () => null }));
vi.mock("@/components/sessions/SessionsPanel", () => ({
  SessionsPanel: () => <div>Session panel</div>,
}));

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

/** A row's buttons by accessible name: an icon button by its label. */
function buttonNames(row: HTMLElement) {
  return within(row)
    .getAllByRole("button")
    .map((button) => button.getAttribute("aria-label") ?? button.textContent);
}

async function cacheUpdates(tools: ToolUpdateInfo[]) {
  vi.mocked(toolUpdatesApi.check).mockResolvedValue(tools);
  await checkToolUpdates();
  vi.mocked(toolUpdatesApi.check).mockClear();
}

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  queryClient.clear();
  localTools = [];
  mocks.mac = true;
  mocks.onInstallDone = undefined;
  vi.mocked(toolUpdatesApi.update).mockReset();
  mocks.toast.success.mockClear();
  mocks.toast.error.mockClear();
  mocks.toast.warning.mockClear();
  mocks.chatGptModel.mockReset();
  mocks.chatGptModel.mockResolvedValue("");
  mocks.workBuddyModels.mockReset();
  mocks.workBuddyModels.mockResolvedValue([]);
  mocks.getUserInfo.mockReset();
  mocks.getUserInfo.mockResolvedValue({ name: "Test User" });
  mocks.reconcileKeys.mockReset();
  mocks.reconcileKeys.mockResolvedValue({ switched: [], failed: [] });
  mocks.invoke.mockImplementation(
    async (command: string, args: { tools?: string[] } = {}) => {
      if (command === "ofox_get_apex_state")
        return { apex: "ofox.ai", pinned: true };
      if (command === "fetch_ofox_models") return [];
      if (command === "model_selection_snapshot")
        return {
          tools: await Promise.all(
            (args.tools ?? []).map(async (toolId) => ({
              toolId,
              writeTarget: toolId === "chatgpt" ? "codex" : toolId,
              revision: `revision-${toolId}`,
              binding: {
                status: "configured",
                message: null,
                missingFiles: [],
                modifiedFields: [],
                envOverrides: [],
              },
              managedSelection:
                toolId === "workbuddy" || toolId === "opencode"
                  ? {
                      kind: "models",
                      models:
                        toolId === "workbuddy"
                          ? await mocks.workBuddyModels()
                          : [],
                    }
                  : { kind: "single", model: "" },
              observedSelection: {
                model: toolId === "chatgpt" ? await mocks.chatGptModel() : null,
                source: "config",
              },
              supportsDefault: toolId !== "workbuddy" && toolId !== "opencode",
              scope: [],
            })),
          ),
        };
      if (command === "get_tool_versions") return localTools;
      if (command === "ofox_list_api_keys") return [];
      if (command === "model_profiles_list") return [];
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
    },
  );
  await cacheUpdates([]);
});

describe("Console title bar", () => {
  it("draws its own brand under the macOS overlay title bar", async () => {
    render(<ConsolePage boundTools={[]} />);
    expect(await screen.findByText("Ofox Desktop")).toBeVisible();
  });

  it("leaves the title to the native title bar elsewhere", async () => {
    mocks.mac = false;
    render(<ConsolePage boundTools={[]} />);
    await screen.findByText("Test User");
    expect(screen.queryByText("Ofox Desktop")).toBeNull();
  });

  it("draws on the canvas, without the orange gradient", async () => {
    const { container } = render(<ConsolePage boundTools={[]} />);
    await screen.findByText("Test User");
    expect(container.firstChild).toHaveClass("bg-canvas");
    expect(container.firstChild).not.toHaveClass("bg-gradient-to-br");
  });
});

describe("Console navigation", () => {
  it("offers 工具, 会话 and 用量, and no 模型方案 tab", async () => {
    render(<ConsolePage boundTools={[]} />);
    await screen.findByText("Test User");
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "工具",
      "会话",
      "用量",
    ]);
  });

  it("keeps tools mounted and retains the missing-tools expansion across Sessions", async () => {
    localTools = [
      localTool("claude", "1.0"),
      { ...localTool("workbuddy", null), installationStatus: "notInstalled" },
    ];
    render(<ConsolePage boundTools={["claude", "workbuddy"]} />);
    const fold = await screen.findByRole("button", { name: /未安装工具（1）/ });
    fireEvent.click(fold);
    const row = screen.getByRole("group", { name: "WorkBuddy" });
    fireEvent.click(screen.getByRole("tab", { name: "会话" }));
    expect(row).toBeInTheDocument();
    expect(row).not.toBeVisible();
    expect(screen.getByText("Session panel")).toBeVisible();
    fireEvent.click(screen.getByRole("tab", { name: "工具" }));
    expect(screen.getByRole("group", { name: "WorkBuddy" })).toBe(row);
    expect(fold).toHaveAttribute("aria-expanded", "true");
  });

  it("mounts 用量 on its first visit and keeps it, hidden, after", async () => {
    render(<ConsolePage boundTools={[]} />);
    await screen.findByText("Test User");
    const tools = screen.getByRole("tabpanel", { name: "工具" });
    expect(document.getElementById("shell-panel-usage")).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: "用量" }));
    const usage = screen.getByRole("tabpanel", { name: "用量" });
    expect(usage).toBeVisible();
    expect(tools).toBeInTheDocument();
    expect(tools).not.toBeVisible();

    fireEvent.click(screen.getByRole("tab", { name: "工具" }));
    expect(screen.getByRole("tabpanel", { name: "工具" })).toBe(tools);
    expect(tools).toBeVisible();
    expect(usage).toBeInTheDocument();
    expect(usage).not.toBeVisible();
  });
});

describe("Console model profiles", () => {
  it("reaches the model profiles from 方案 in the tools toolbar, loading them only when it opens", async () => {
    localTools = [localTool("claude", "2.1.293")];
    render(<ConsolePage boundTools={["claude"]} />);
    await screen.findByRole("group", { name: "Claude Code" });
    const profilesCalls = () =>
      mocks.invoke.mock.calls.filter(
        ([command]) => command === "model_profiles_list",
      );
    expect(profilesCalls()).toHaveLength(0);

    await userEvent.click(screen.getByRole("button", { name: "方案" }));

    expect(
      await screen.findByRole("menuitem", { name: "管理方案…" }),
    ).toBeVisible();
    expect(await screen.findByText("还没有方案")).toBeVisible();
    expect(profilesCalls()).toHaveLength(1);
  });
});

describe("Console header", () => {
  it("has the one refresh, in the header, and no footer links", async () => {
    render(<ConsolePage boundTools={[]} />);
    await screen.findByText("Test User");
    const refresh = screen.getAllByRole("button", { name: "刷新" });
    expect(refresh).toHaveLength(1);
    expect(refresh[0].closest("header")).not.toBeNull();
    expect(screen.queryByText("查看详细用量 ↗")).toBeNull();
    expect(screen.queryByText("Ofox 网站 ↗")).toBeNull();
  });

  it("opens the settings from the header", async () => {
    render(<ConsolePage boundTools={[]} />);
    await screen.findByText("Test User");
    fireEvent.click(screen.getByRole("button", { name: "设置" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("偏好设置");
  });

  it("refreshes the balance, not the tools, while 用量 is open", async () => {
    mocks.getUserInfo.mockResolvedValue({
      name: "Test User",
      balance: { balance: 12.5 },
    });
    localTools = [localTool("claude", "2.1.293")];
    render(<ConsolePage boundTools={["claude"]} />);
    await screen.findByRole("group", { name: "Claude Code" });
    await waitFor(() => expect(mocks.getUserInfo).toHaveBeenCalled());
    const detections = mocks.invoke.mock.calls.filter(
      ([command]) => command === "get_tool_versions",
    ).length;

    fireEvent.click(screen.getByRole("tab", { name: "用量" }));
    fireEvent.click(screen.getByRole("button", { name: "刷新" }));

    await waitFor(() =>
      expect(mocks.toast.success).toHaveBeenCalledWith("余额已更新：$12.50"),
    );
    expect(
      mocks.invoke.mock.calls.filter(
        ([command]) => command === "get_tool_versions",
      ),
    ).toHaveLength(detections);
    expect(toolUpdatesApi.check).not.toHaveBeenCalled();
  });
});

describe("Console usage page", () => {
  it("shows the balance and links to the detailed usage on the web", async () => {
    mocks.getUserInfo.mockResolvedValue({
      name: "Test User",
      balance: { balance: 12.5 },
    });
    render(<ConsolePage boundTools={[]} />);
    await screen.findAllByText("$12.50");

    fireEvent.click(screen.getByRole("tab", { name: "用量" }));
    const usage = screen.getByRole("tabpanel", { name: "用量" });
    expect(within(usage).getByTestId("balance-card")).toHaveTextContent(
      "$12.50",
    );
    fireEvent.click(
      within(usage).getByRole("button", { name: "在网页查看详细用量 ↗" }),
    );
    expect(mocks.openExternal).toHaveBeenCalledWith(
      "https://app.ofox.ai/dashboard",
    );
  });
});

describe("Console balance card", () => {
  it("highlights the card when the balance is zero", async () => {
    mocks.getUserInfo.mockResolvedValue({
      name: "Test User",
      balance: { balance: 0 },
    });
    render(<ConsolePage boundTools={[]} />);

    expect(await screen.findByText("$0.00")).toBeVisible();
    expect(screen.getByTestId("balance-card")).toHaveAttribute(
      "data-empty",
      "true",
    );
  });

  it("does not highlight a positive or unknown balance", async () => {
    mocks.getUserInfo.mockResolvedValue({
      name: "Test User",
      balance: { balance: 12.5 },
    });
    const { unmount } = render(<ConsolePage boundTools={[]} />);
    expect(await screen.findByText("$12.50")).toBeVisible();
    expect(screen.getByTestId("balance-card")).not.toHaveAttribute(
      "data-empty",
    );
    unmount();

    mocks.getUserInfo.mockResolvedValue({ name: "Test User" });
    render(<ConsolePage boundTools={[]} />);
    expect(await screen.findByText("—")).toBeVisible();
    expect(screen.getByTestId("balance-card")).not.toHaveAttribute(
      "data-empty",
    );
  });
});

describe("Console WorkBuddy model label", () => {
  it("shows how many models are selected instead of a single current model", async () => {
    mocks.workBuddyModels.mockResolvedValue(["a/one", "b/two", "c/three"]);
    localTools = [localTool("workbuddy", "5.7.7")];
    render(<ConsolePage boundTools={["workbuddy"]} />);

    expect(await screen.findByText("3 个模型")).toBeVisible();
    expect(screen.queryByText("a/one")).toBeNull();
  });

  it("keeps a one-model OpenCode or WorkBuddy set as a multiple-selection entry", async () => {
    mocks.workBuddyModels.mockResolvedValue(["a/one"]);
    localTools = [localTool("workbuddy", "5.7.7")];
    render(<ConsolePage boundTools={["workbuddy"]} />);

    expect(await screen.findByText("1 个模型")).toBeVisible();
  });
});

describe("Console ChatGPT row model", () => {
  it("shows the model picked inside the ChatGPT app, not the Codex config model", async () => {
    mocks.chatGptModel.mockResolvedValue("gpt-5.6-sol");
    localTools = [localTool("chatgpt", "26.1007.30912")];
    render(<ConsolePage boundTools={["chatgpt"]} />);

    expect(await screen.findByText(/gpt-5\.6-sol/)).toBeVisible();
  });
});

describe("Console ChatGPT update badge", () => {
  it("shows no upgrade button when the app manages its own update", async () => {
    const chatgpt: ToolUpdateInfo = {
      ...availableTool("chatgpt", "26.1007.30912", "26.1007.41230"),
      installationKind: "desktopApp",
      update_status: "appManaged",
      update_source: "sparkle",
      update_supported: false,
      update_reason:
        "Public Sparkle feed; ChatGPT may roll this release out gradually",
    };
    localTools = [
      {
        ...localTool("chatgpt", "26.1007.30912"),
        installationKind: "desktopApp",
      },
    ];
    await cacheUpdates([chatgpt]);
    render(<ConsolePage boundTools={["chatgpt"]} />);

    expect(await screen.findByText(/v26\.1007\.30912/)).toBeVisible();
    expect(
      screen.queryByRole("button", { name: /v26\.1007\.41230/ }),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: /升级到/ })).toBeNull();
  });
});

describe("Console after switching accounts", () => {
  it("moves bound tools to this account's key and reloads their key labels", async () => {
    mocks.reconcileKeys.mockResolvedValueOnce({
      switched: ["claude"],
      failed: [],
    });
    render(<ConsolePage boundTools={["claude"]} />);

    await waitFor(() =>
      expect(mocks.toast.success).toHaveBeenCalledWith(
        "Claude Code 已改用当前账号的额度。",
      ),
    );
    await waitFor(() =>
      expect(
        mocks.invoke.mock.calls.filter(
          ([command]) => command === "ofox_list_api_keys",
        ),
      ).toHaveLength(2),
    );
  });

  it("says which tool still uses the previous account's key", async () => {
    mocks.reconcileKeys.mockResolvedValueOnce({
      switched: [],
      failed: [{ tool: "codex", message: "接入配置已被修改，没有自动更换。" }],
    });
    render(<ConsolePage boundTools={["codex"]} />);

    await waitFor(() =>
      expect(mocks.toast.warning).toHaveBeenCalledWith(
        "Codex 还在用之前账号的 API Key：接入配置已被修改，没有自动更换。",
        expect.anything(),
      ),
    );
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
      expect(mocks.toast.success).toHaveBeenCalledWith("已刷新"),
    );
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

describe("Console tool rows", () => {
  it("opens the tool's usage on the web from its ⋯ menu, with its key on the sub-line", async () => {
    localTools = [localTool("claude", "2.1.293")];
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(
      async (command: string, ...args: unknown[]) => {
        if (command === "ofox_list_api_keys")
          return [{ tool: "claude", keyId: "key-1", name: "Claude on mac" }];
        return original(command, ...args);
      },
    );
    render(<ConsolePage boundTools={["claude"]} />);
    const row = await screen.findByRole("group", { name: "Claude Code" });
    expect(await within(row).findByText("Claude on mac")).toBeVisible();

    await userEvent.click(
      within(row).getByRole("button", { name: "更多操作" }),
    );
    await userEvent.click(
      await screen.findByRole("menuitem", { name: "数据统计" }),
    );

    expect(mocks.openExternal).toHaveBeenCalledWith(
      "https://app.ofox.ai/analytics?apiKeys=key-1",
    );
  });

  it("keeps 数据统计 in the ⋯ menu but unavailable while the tool has no key", async () => {
    localTools = [localTool("claude", "2.1.293")];
    render(<ConsolePage boundTools={["claude"]} />);
    const row = await screen.findByRole("group", { name: "Claude Code" });

    await userEvent.click(
      within(row).getByRole("button", { name: "更多操作" }),
    );

    expect(
      await screen.findByRole("menuitem", { name: "数据统计" }),
    ).toHaveAttribute("data-disabled");
  });

  it("names how the version was installed when it is known", async () => {
    localTools = [
      localTool("claude", "2.1.293"),
      localTool("codex", "0.161.0"),
    ];
    await cacheUpdates([
      {
        ...availableTool("claude", "2.1.293", "2.1.293"),
        update_status: "current",
      },
      {
        ...availableTool("codex", "0.161.0", "0.161.0"),
        update_status: "current",
        update_source: null,
      },
    ]);
    render(<ConsolePage boundTools={["claude", "codex"]} />);

    const claude = await screen.findByRole("group", { name: "Claude Code" });
    expect(within(claude).getByText("v2.1.293")).toHaveAttribute(
      "title",
      "通过 npm 安装",
    );
    const codex = screen.getByRole("group", { name: "Codex" });
    expect(within(codex).getByText("v0.161.0")).not.toHaveAttribute("title");
  });

  it("styles repair as the primary action and the others as secondary", async () => {
    localTools = [
      localTool("claude", "2.1.293"),
      {
        ...localTool("codex", null),
        installationStatus: "installed",
        error: "version process failed",
      },
    ];
    render(<ConsolePage boundTools={["claude", "codex"]} />);
    const codex = await screen.findByRole("group", { name: "Codex" });
    const claude = screen.getByRole("group", { name: "Claude Code" });

    expect(within(codex).getByRole("button", { name: "修复安装" })).toHaveClass(
      "bg-brand",
    );
    expect(
      within(claude).getByRole("button", { name: "打开" }),
    ).not.toHaveClass("bg-brand");
    expect(
      within(claude).getByRole("button", { name: "管理" }),
    ).not.toHaveClass("bg-brand");
  });

  it("puts the model picker on the sub-line as a pill", async () => {
    localTools = [localTool("claude", "2.1.293")];
    render(<ConsolePage boundTools={["claude"]} />);
    const row = await screen.findByRole("group", { name: "Claude Code" });
    expect(await within(row).findByRole("combobox")).toHaveClass("bg-pill");
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
    expect(buttonNames(row)).toEqual([
      "0 个模型",
      "前往下载",
      "管理",
      "更多操作",
    ]);
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
    expect(buttonNames(row)).toEqual(["重新检测", "管理", "更多操作"]);
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
    expect(buttonNames(row)).toEqual(["修复安装", "管理", "更多操作"]);
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
    expect(buttonNames(row)).toEqual(["恢复绑定", "管理", "更多操作"]);
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
