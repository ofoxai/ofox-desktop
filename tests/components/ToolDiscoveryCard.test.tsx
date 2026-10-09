import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ToolDiscoveryCard } from "@/components/onboarding/ToolDiscoveryCard";
import type { InstallProgress } from "@/hooks/useToolInstall";
import { settingsApi } from "@/lib/api";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { beforeAll } from "vitest";

beforeAll(() => i18n.addResourceBundle("zh", "translation", zh, true, true));

const baseProps = {
  toolId: "codex",
  label: "Codex",
  version: null,
  autoSelectTick: 0,
  onClick: vi.fn(),
};

const progress = (over: Partial<InstallProgress> = {}): InstallProgress => ({
  step: 2,
  total: 4,
  name: "Node.js LTS",
  phase: "start",
  ...over,
});

describe("ToolDiscoveryCard 的安装进度展示", () => {
  it("有进度时显示第几步和步骤名，而不是笼统的“安装中…”", () => {
    render(
      <ToolDiscoveryCard
        {...baseProps}
        status="installing"
        progress={progress()}
      />,
    );

    expect(screen.getByText(/2\/4/)).toBeTruthy();
    expect(screen.getByText(/Node\.js LTS/)).toBeTruthy();
    expect(screen.queryByText("安装中…")).toBeNull();
  });

  it("waiting 阶段附带已用秒数，让用户看出它在动", () => {
    render(
      <ToolDiscoveryCard
        {...baseProps}
        status="installing"
        progress={progress({ phase: "waiting", elapsed: 18, timeout: 300 })}
      />,
    );

    expect(screen.getByText(/18s/)).toBeTruthy();
  });

  it("没有进度时回退到“安装中…”", () => {
    render(<ToolDiscoveryCard {...baseProps} status="installing" />);

    expect(screen.getByText("安装中…")).toBeTruthy();
  });

  it("非 installing 态忽略进度，仍显示版本号", () => {
    render(
      <ToolDiscoveryCard
        {...baseProps}
        status="selected"
        version="1.2.3"
        progress={progress()}
      />,
    );

    expect(screen.getByText("1.2.3")).toBeTruthy();
    expect(screen.queryByText(/2\/4/)).toBeNull();
  });

  it("通过系统浏览器打开对应工具的官方 GitHub 仓库", async () => {
    const openExternal = vi
      .spyOn(settingsApi, "openExternal")
      .mockResolvedValueOnce();

    render(<ToolDiscoveryCard {...baseProps} status="selected" />);
    fireEvent.click(screen.getByRole("button", { name: "查看 Codex GitHub" }));

    await waitFor(() => {
      expect(openExternal).toHaveBeenCalledWith(
        "https://github.com/openai/codex",
      );
    });
    openExternal.mockRestore();
  });

  it("WorkBuddy 未安装时提供官方下载入口", async () => {
    const openExternal = vi
      .spyOn(settingsApi, "openExternal")
      .mockResolvedValueOnce();

    render(
      <ToolDiscoveryCard
        {...baseProps}
        toolId="workbuddy"
        label="WorkBuddy"
        status="missing"
      />,
    );
    expect(screen.getByAltText("WorkBuddy")).toBeInTheDocument();
    expect(screen.queryByText("WB")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "下载" }));

    await waitFor(() => {
      expect(openExternal).toHaveBeenCalledWith(
        "https://www.workbuddy.cn/work/",
      );
    });
    openExternal.mockRestore();
  });

  it("shows retained missing bindings with a management action instead of reinstalling from Add", () => {
    const manage = vi.fn();
    const install = vi.fn();
    render(
      <ToolDiscoveryCard
        {...baseProps}
        status="boundMissing"
        onManageBound={manage}
        onInstall={install}
      />,
    );
    expect(screen.getByText("已绑定 · 未安装")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "安装" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "在主页管理" }));
    expect(manage).toHaveBeenCalledOnce();
    expect(install).not.toHaveBeenCalled();
  });

  it("offers a retry rather than installation when detection is uncertain", () => {
    const retry = vi.fn();
    const install = vi.fn();
    render(
      <ToolDiscoveryCard
        {...baseProps}
        status="detectionFailed"
        onRetry={retry}
        onInstall={install}
      />,
    );
    expect(screen.getByText("检测失败")).toBeVisible();
    expect(screen.queryByText("未安装")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "安装" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "重新检测" }));
    expect(retry).toHaveBeenCalledOnce();
    expect(install).not.toHaveBeenCalled();
  });

  it("does not imply uninstall for a bound tool without a detected version", () => {
    render(<ToolDiscoveryCard {...baseProps} status="bound" />);
    expect(screen.getByText("绑定已保留")).toBeVisible();
    expect(screen.queryByText("未安装")).not.toBeInTheDocument();
  });
});
