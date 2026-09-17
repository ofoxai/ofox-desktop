import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ToolDiscoveryCard } from "@/components/onboarding/ToolDiscoveryCard";
import type { InstallProgress } from "@/hooks/useToolInstall";

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
      <ToolDiscoveryCard {...baseProps} status="installing" progress={progress()} />,
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
});
