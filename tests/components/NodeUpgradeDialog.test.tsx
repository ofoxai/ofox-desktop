import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import NodeUpgradeDialog from "@/components/tools/NodeUpgradeDialog";
import type { NodeRequirement } from "@/lib/api/nodeRequirement";

const requirement = (overrides: Partial<NodeRequirement>): NodeRequirement => ({
  status: "tooOld",
  required: ">=24.16.0 <25 || >=26.1.0",
  current: "24.15.0",
  manager: "fnm",
  canUpgrade: true,
  needsAdmin: false,
  reinstall: [],
  manual: "fnm install 24 && fnm default 24",
  ...overrides,
});

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});

describe("NodeUpgradeDialog", () => {
  it("explains the requirement, the command and the tools it reinstalls", () => {
    const onConfirm = vi.fn();
    render(
      <NodeUpgradeDialog
        prompt={{
          toolId: "openclaw",
          requirement: requirement({ reinstall: ["claude", "gemini"] }),
        }}
        onConfirm={onConfirm}
        onCancel={vi.fn()}
      />,
    );
    expect(screen.getByText("OpenClaw 需要更新的 Node.js")).toBeVisible();
    expect(
      screen.getByText(
        "OpenClaw 最新版要求 Node.js >=24.16.0 <25 || >=26.1.0，你电脑上当前是 24.15.0（由 fnm 管理）。",
      ),
    ).toBeVisible();
    expect(screen.getByText("fnm install 24 && fnm default 24")).toBeVisible();
    expect(
      screen.getByText(
        "切换 Node.js 版本后，Claude Code、Gemini 需要重新安装，Ofox 会一并装好。",
      ),
    ).toBeVisible();
    fireEvent.click(
      screen.getByRole("button", { name: "升级 Node.js 并安装" }),
    );
    expect(onConfirm).toHaveBeenCalled();
  });

  it("says when the upgrade asks for administrator approval", () => {
    render(
      <NodeUpgradeDialog
        prompt={{
          toolId: "gemini",
          requirement: requirement({
            manager: "nodejs",
            needsAdmin: true,
            manual: "winget install -e --id OpenJS.NodeJS.LTS",
          }),
        }}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(
      screen.getByText("需要管理员授权，系统会弹出授权窗口。"),
    ).toBeVisible();
  });

  it("only explains how to upgrade by hand when Ofox cannot do it", () => {
    const onCancel = vi.fn();
    render(
      <NodeUpgradeDialog
        prompt={{
          toolId: "openclaw",
          requirement: requirement({
            manager: "unknown",
            canUpgrade: false,
            manual: "从 https://nodejs.org 下载安装",
          }),
        }}
        onConfirm={vi.fn()}
        onCancel={onCancel}
      />,
    );
    expect(
      screen.queryByRole("button", { name: "升级 Node.js 并安装" }),
    ).toBeNull();
    expect(screen.getByText("从 https://nodejs.org 下载安装")).toBeVisible();
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
