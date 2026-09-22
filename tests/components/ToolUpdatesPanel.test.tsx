import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
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
  toolUpdatesApi: { check: vi.fn(), update: vi.fn() },
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
