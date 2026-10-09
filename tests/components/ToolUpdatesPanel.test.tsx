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
import { ToolUpdatesPanel } from "@/components/settings/ToolUpdatesPanel";
import { checkToolUpdates, updateTools } from "@/hooks/useToolUpdates";
import { toolUpdatesApi, type ToolUpdateInfo } from "@/lib/api/toolUpdates";
import { settingsApi } from "@/lib/api";
import { emitTauriEvent } from "../msw/tauriMocks";

vi.mock("@/lib/api/toolUpdates", () => ({
  toolUpdatesApi: {
    check: vi.fn(),
    update: vi.fn(),
    isAppRunning: vi.fn(),
    openApp: vi.fn(),
  },
}));

function tool(
  name: string,
  extra: Partial<ToolUpdateInfo> = {},
): ToolUpdateInfo {
  return {
    name,
    version: "1.0.0",
    latest_version: "1.1.0",
    error: null,
    installationKind: "cli",
    installationStatus: "installed",
    update_status: "available",
    update_source: "npm",
    update_supported: true,
    update_reason: null,
    executable_path: "/node/bin/cli",
    ...extra,
  };
}

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  vi.mocked(toolUpdatesApi.check).mockResolvedValue([]);
  await checkToolUpdates();
});

describe("Tool updates", () => {
  it("offers repair for a broken installation without presenting the registry version as installed", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      tool("opencode", {
        version: null,
        update_status: "broken",
        update_source: "pnpm",
      }),
    ]);
    vi.mocked(toolUpdatesApi.update).mockResolvedValue({
      status: "repaired",
      before: "",
      after: "1.1.0",
    });
    await checkToolUpdates();
    const { container } = render(<ToolUpdatesPanel />);
    expect(
      within(
        container.querySelector('[data-tool="opencode"]') as HTMLElement,
      ).getByText("当前：—"),
    ).toBeVisible();
    expect(screen.getByText("远端最新：1.1.0")).toBeVisible();
    expect(
      screen.getByText("已检测到安装，但无法运行；版本尚未验证"),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "修复安装" }));
    await waitFor(() =>
      expect(toolUpdatesApi.update).toHaveBeenCalledWith(
        "opencode",
        expect.any(String),
      ),
    );
    await waitFor(() => expect(screen.getByText(/已恢复运行/)).toBeVisible());
  });
  it("puts updates first and keeps a visible action for unsupported sources", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      tool("claude", { update_status: "current" }),
      tool("opencode", { update_supported: false }),
    ]);
    await checkToolUpdates();
    const open = vi.spyOn(settingsApi, "openExternal").mockResolvedValue();
    const { container } = render(<ToolUpdatesPanel />);
    expect(container.querySelector("[data-tool]")).toHaveAttribute(
      "data-tool",
      "opencode",
    );
    expect(screen.getByText("发现新版")).toBeVisible();
    expect(screen.getByText("1 个工具有更新")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "查看更新方式" }));
    expect(open).toHaveBeenCalledWith("https://github.com/anomalyco/opencode");
  });
  it("distinguishes failed checks and app-managed updates from up-to-date", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      tool("gemini", { update_status: "failed", latest_version: null }),
      tool("codex", { update_status: "current" }),
      tool("workbuddy", {
        installationKind: "desktopApp",
        update_status: "appManaged",
        latest_version: null,
        update_supported: false,
      }),
    ]);
    await checkToolUpdates();
    const open = vi.spyOn(settingsApi, "openExternal").mockResolvedValue();
    render(<ToolUpdatesPanel />);
    expect(screen.getByText("检查失败，请重试")).toBeVisible();
    expect(screen.getAllByText("当前版本不低于最新稳定版")).toHaveLength(1);
    expect(
      screen.queryByRole("button", { name: "升级" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "官方下载页" }));
    expect(open).toHaveBeenCalledWith("https://www.workbuddy.cn/work/");
  });

  it("serializes a batch, continues after failure, ignores unrelated logs and refreshes", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      tool("claude"),
      tool("gemini"),
    ]);
    await checkToolUpdates();
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    vi.mocked(toolUpdatesApi.update).mockImplementation(
      async (name, operationId) => {
        if (name === "claude") {
          emitTauriEvent("tool-update-progress", {
            tool: name,
            operationId: "other",
            stage: "log",
            detail: "unrelated-log",
          });
          emitTauriEvent("tool-update-progress", {
            tool: name,
            operationId,
            stage: "log",
            detail: "anchored-update",
          });
          await pending;
          throw new Error("mock network failure");
        }
        return { status: "updated", before: "1.0.0", after: "1.1.0" };
      },
    );
    render(<ToolUpdatesPanel />);
    fireEvent.click(screen.getByRole("button", { name: /^全部升级/ }));
    await waitFor(() => expect(toolUpdatesApi.update).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("button", { name: /^全部升级/ })).toBeDisabled();
    await act(() => updateTools(["claude"]));
    expect(toolUpdatesApi.update).toHaveBeenCalledTimes(1);
    expect(screen.queryByText(/unrelated-log/)).not.toBeInTheDocument();
    await act(async () => {
      release();
    });
    await waitFor(() =>
      expect(screen.getByText("升级完成，已验证生效版本")).toBeVisible(),
    );
    expect(screen.getByText("升级失败，请查看日志后重试")).toBeVisible();
    expect(toolUpdatesApi.update).toHaveBeenCalledTimes(2);
    expect(toolUpdatesApi.check).toHaveBeenCalledTimes(3);
  });

  it("shows unchanged version as a diagnostic outcome", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([tool("opencode")]);
    vi.mocked(toolUpdatesApi.update).mockResolvedValue({
      status: "unchanged",
      before: "1.0.0",
      after: "1.0.0",
    });
    await checkToolUpdates();
    render(<ToolUpdatesPanel />);
    fireEvent.click(screen.getByRole("button", { name: "升级" }));
    expect(
      await screen.findByText("尚未升级至目标版本，请检查日志和安装路径"),
    ).toBeVisible();
  });

  it("clears stale update availability after a failed refresh", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([tool("codex")]);
    await checkToolUpdates();
    render(<ToolUpdatesPanel />);
    vi.mocked(toolUpdatesApi.check).mockRejectedValue(new Error("offline"));
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
    await waitFor(() =>
      expect(screen.queryByText("有可用更新")).not.toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: /^全部升级/ })).toBeDisabled();
  });
});

function chatgpt(extra: Partial<ToolUpdateInfo> = {}): ToolUpdateInfo {
  return tool("chatgpt", {
    installationKind: "desktopApp",
    version: "26.924.22138",
    latest_version: "26.928.21956",
    update_source: "sparkle",
    update_supported: false,
    update_reason:
      "Public Sparkle feed; ChatGPT may roll this release out gradually",
    executable_path: "/Users/test/Applications/ChatGPT.app",
    ...extra,
  });
}

function card(container: HTMLElement, name: string) {
  return within(
    container.querySelector(`[data-tool="${name}"]`) as HTMLElement,
  );
}

describe("ChatGPT desktop updates", () => {
  it("shows the real latest version instead of in-client guidance", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([chatgpt()]);
    await checkToolUpdates();
    const { container } = render(<ToolUpdatesPanel />);
    const chat = card(container, "chatgpt");
    expect(chat.getByText("远端最新：26.928.21956")).toBeVisible();
    expect(chat.getByText("发现新版")).toBeVisible();
    expect(chat.getByText("有可用更新")).toBeVisible();
    expect(chat.queryByText("请在客户端内检查更新")).not.toBeInTheDocument();
    expect(chat.getByText(/屏幕顶部菜单栏左侧的「ChatGPT」/)).toBeVisible();
  });

  it("opens ChatGPT for a Sparkle update instead of running an update", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([chatgpt()]);
    vi.mocked(toolUpdatesApi.openApp).mockResolvedValue();
    await checkToolUpdates();
    render(<ToolUpdatesPanel />);
    fireEvent.click(screen.getByRole("button", { name: "在 ChatGPT 中更新" }));
    await waitFor(() =>
      expect(toolUpdatesApi.openApp).toHaveBeenCalledWith("chatgpt"),
    );
    expect(toolUpdatesApi.update).not.toHaveBeenCalled();
  });

  it("leaves ChatGPT out of update all", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      tool("claude"),
      chatgpt({ update_source: "msstore", update_supported: true }),
    ]);
    vi.mocked(toolUpdatesApi.update).mockResolvedValue({
      status: "updated",
      before: "1.0.0",
      after: "1.1.0",
    });
    await checkToolUpdates();
    render(<ToolUpdatesPanel />);
    fireEvent.click(screen.getByRole("button", { name: "全部升级 (1)" }));
    await waitFor(() =>
      expect(toolUpdatesApi.update).toHaveBeenCalledWith(
        "claude",
        expect.any(String),
      ),
    );
    expect(toolUpdatesApi.update).toHaveBeenCalledTimes(1);
    expect(screen.getByText("可一键升级 2 个，需手动更新 0 个")).toBeVisible();
  });

  it("confirms before closing a running ChatGPT and upgrades after confirming", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      chatgpt({ update_source: "msstore", update_supported: true }),
    ]);
    vi.mocked(toolUpdatesApi.isAppRunning).mockResolvedValue(true);
    vi.mocked(toolUpdatesApi.update).mockResolvedValue({
      status: "updated",
      before: "26.924.22138",
      after: "26.928.21956",
    });
    await checkToolUpdates();
    render(<ToolUpdatesPanel />);
    fireEvent.click(screen.getByRole("button", { name: "升级" }));
    expect(await screen.findByText("需要先关闭 ChatGPT")).toBeVisible();
    expect(toolUpdatesApi.update).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "关闭并升级" }));
    await waitFor(() =>
      expect(toolUpdatesApi.update).toHaveBeenCalledWith(
        "chatgpt",
        expect.any(String),
      ),
    );
  });

  it("does not upgrade a running ChatGPT when the user cancels", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      chatgpt({ update_source: "msstore", update_supported: true }),
    ]);
    vi.mocked(toolUpdatesApi.isAppRunning).mockResolvedValue(true);
    await checkToolUpdates();
    render(<ToolUpdatesPanel />);
    fireEvent.click(screen.getByRole("button", { name: "升级" }));
    fireEvent.click(await screen.findByRole("button", { name: "取消" }));
    await waitFor(() =>
      expect(screen.queryByText("需要先关闭 ChatGPT")).not.toBeInTheDocument(),
    );
    expect(toolUpdatesApi.update).not.toHaveBeenCalled();
  });

  it("upgrades a closed ChatGPT without asking", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      chatgpt({ update_source: "msstore", update_supported: true }),
    ]);
    vi.mocked(toolUpdatesApi.isAppRunning).mockResolvedValue(false);
    vi.mocked(toolUpdatesApi.update).mockResolvedValue({
      status: "updated",
      before: "26.924.22138",
      after: "26.928.21956",
    });
    await checkToolUpdates();
    render(<ToolUpdatesPanel />);
    fireEvent.click(screen.getByRole("button", { name: "升级" }));
    await waitFor(() =>
      expect(toolUpdatesApi.update).toHaveBeenCalledWith(
        "chatgpt",
        expect.any(String),
      ),
    );
    expect(screen.queryByText("需要先关闭 ChatGPT")).not.toBeInTheDocument();
  });

  it("asks before upgrading when the running check fails", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      chatgpt({ update_source: "msstore", update_supported: true }),
    ]);
    vi.mocked(toolUpdatesApi.isAppRunning).mockRejectedValue(
      new Error("powershell failed"),
    );
    await checkToolUpdates();
    render(<ToolUpdatesPanel />);
    fireEvent.click(screen.getByRole("button", { name: "升级" }));
    expect(await screen.findByText("需要先关闭 ChatGPT")).toBeVisible();
    expect(toolUpdatesApi.update).not.toHaveBeenCalled();
  });

  it("offers the download page when ChatGPT is missing", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      chatgpt({
        version: null,
        latest_version: null,
        update_status: "notInstalled",
        update_source: null,
        update_reason: null,
        executable_path: null,
      }),
    ]);
    await checkToolUpdates();
    const open = vi.spyOn(settingsApi, "openExternal").mockResolvedValue();
    const { container } = render(<ToolUpdatesPanel />);
    const chat = card(container, "chatgpt");
    expect(chat.getByText("未检测到可运行的工具")).toBeVisible();
    fireEvent.click(chat.getByRole("button", { name: "官方下载页" }));
    expect(open).toHaveBeenCalledWith("https://chatgpt.com/download/");
  });

  it("keeps in-client guidance and the reason when the latest check failed", async () => {
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([
      chatgpt({
        latest_version: null,
        update_status: "appManaged",
        update_reason: "Latest-version check failed: offline",
      }),
    ]);
    await checkToolUpdates();
    const { container } = render(<ToolUpdatesPanel />);
    const chat = card(container, "chatgpt");
    expect(chat.getByText("请在客户端内检查更新")).toBeVisible();
    expect(
      chat.getByText(/Latest-version check failed: offline/),
    ).toBeInTheDocument();
  });
});
