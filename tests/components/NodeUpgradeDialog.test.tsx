import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import NodeUpgradeDialog from "@/components/tools/NodeUpgradeDialog";
import type { NodeRequirement } from "@/lib/api/nodeRequirement";

const openExternal = vi.fn().mockResolvedValue(undefined);

vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: (url: string) => openExternal(url) },
}));

const requirement = (overrides: Partial<NodeRequirement>): NodeRequirement => ({
  status: "tooOld",
  required: ">=24.16.0 <25 || >=26.1.0",
  current: "24.15.0",
  manager: "fnm",
  canUpgrade: true,
  needsAdmin: false,
  reinstall: [],
  manual: "fnm install 24 && fnm default 24",
  minimum: "24.16.0",
  command: "fnm install 24 && fnm default 24 && fnm use 24",
  ...overrides,
});

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  openExternal.mockClear();
});

describe("NodeUpgradeDialog", () => {
  it("compares the current Node.js with the minimum the tool needs", () => {
    render(
      <NodeUpgradeDialog
        prompt={{ toolId: "openclaw", requirement: requirement({}) }}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(screen.getByText("安装 OpenClaw 前需要升级 Node.js")).toBeVisible();
    expect(screen.getByText("24.15.0")).toBeVisible();
    expect(screen.getByText("由 fnm 管理")).toBeVisible();
    expect(screen.getByText("24.16.0")).toBeVisible();
  });

  it("puts every global tool back at its version, and says why", () => {
    const onConfirm = vi.fn();
    render(
      <NodeUpgradeDialog
        prompt={{
          toolId: "openclaw",
          requirement: requirement({
            reinstall: [
              {
                name: "@anthropic-ai/claude-code",
                version: "2.1.296",
                tool: "claude",
              },
              { name: "@augmentcode/auggie", version: "0.28.0", tool: null },
              { name: "pnpm", version: "10.34.5", tool: null },
            ],
          }),
        }}
        onConfirm={onConfirm}
        onCancel={vi.fn()}
      />,
    );
    const steps = screen.getAllByRole("listitem");
    expect(steps.map((step) => step.querySelector("p")?.textContent)).toEqual([
      "用 fnm 升级 Node.js",
      "把原来的 3 个全局工具装到新版本（版本不变）",
      "安装 OpenClaw",
    ]);
    expect(steps[1]).toHaveTextContent(
      "Claude Code、@augmentcode/auggie、pnpm",
    );
    expect(
      screen.getByText(
        "fnm 给每个 Node.js 版本单独存放全局安装的工具，不装回去的话，换版本后这些命令就找不到了。它们的配置和登录不受影响。",
      ),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "升级并安装" }));
    expect(onConfirm).toHaveBeenCalled();
  });

  it("skips the reinstall step when nothing else depends on this Node.js", () => {
    render(
      <NodeUpgradeDialog
        prompt={{ toolId: "openclaw", requirement: requirement({}) }}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });

  it("keeps the full command folded away until asked for", () => {
    render(
      <NodeUpgradeDialog
        prompt={{ toolId: "openclaw", requirement: requirement({}) }}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(
      screen.getByText("fnm install 24 && fnm default 24 && fnm use 24"),
    ).not.toBeVisible();
    fireEvent.click(screen.getByText("查看要运行的命令"));
    expect(
      screen.getByText("fnm install 24 && fnm default 24 && fnm use 24"),
    ).toBeVisible();
  });

  it("says when the upgrade asks for administrator approval", () => {
    render(
      <NodeUpgradeDialog
        prompt={{
          toolId: "gemini",
          requirement: requirement({
            manager: "nodejs",
            needsAdmin: true,
            command: "winget install -e --id OpenJS.NodeJS.LTS",
          }),
        }}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(screen.getByText("用 winget 升级 Node.js")).toBeVisible();
    expect(
      screen.getByText("需要管理员授权，系统会弹出授权窗口。"),
    ).toBeVisible();
  });

  it("falls back to the raw range when the minimum is unknown", () => {
    render(
      <NodeUpgradeDialog
        prompt={{
          toolId: "openclaw",
          requirement: requirement({ minimum: null }),
        }}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(
      screen.getAllByText(/>=24\.16\.0 <25 \|\| >=26\.1\.0/)[0],
    ).toBeVisible();
  });

  it("points to the Node.js website when Ofox cannot upgrade it", () => {
    const onCancel = vi.fn();
    render(
      <NodeUpgradeDialog
        prompt={{
          toolId: "openclaw",
          requirement: requirement({
            manager: "asdf",
            canUpgrade: false,
            command: null,
          }),
        }}
        onConfirm={vi.fn()}
        onCancel={onCancel}
      />,
    );
    expect(screen.queryByRole("button", { name: "升级并安装" })).toBeNull();
    expect(screen.queryByRole("listitem")).toBeNull();
    expect(
      screen.getByText(
        "Ofox 没法替你升级由 asdf 管理的 Node.js。请从 Node.js 官网下载安装最新的长期支持版，装好后再回来安装 OpenClaw。",
      ),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "打开 Node.js 官网" }));
    expect(openExternal).toHaveBeenCalledWith(
      "https://nodejs.org/zh-cn/download",
    );
    fireEvent.click(screen.getByRole("button", { name: "知道了" }));
    expect(onCancel).toHaveBeenCalled();
  });

  it("renders nothing without a prompt", () => {
    render(
      <NodeUpgradeDialog
        prompt={null}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
