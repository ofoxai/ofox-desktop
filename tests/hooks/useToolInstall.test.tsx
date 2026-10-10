import { renderHook, act, waitFor } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { useToolInstall } from "@/hooks/useToolInstall";
import { emitTauriEvent } from "../msw/tauriMocks";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

/** 安装器 app/progress.py 打出来的那种行。 */
const progressLine = (payload: Record<string, unknown>) =>
  JSON.stringify({ type: "ofox-install-progress", ...payload });

const emitLog = (tool: string, line: string) =>
  emitTauriEvent("install-tool-log", { tool, stream: "stdout", line });

describe("useToolInstall 的安装进度", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(0);
  });

  it("把进度行解析成按工具分组的状态", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitLog(
        "codex",
        progressLine({
          step: 2,
          total: 4,
          name: "Node.js LTS",
          phase: "waiting",
          elapsed: 18,
          timeout: 300,
        }),
      );
    });

    expect(result.current.progress.codex).toEqual({
      step: 2,
      total: 4,
      name: "Node.js LTS",
      phase: "waiting",
      elapsed: 18,
      timeout: 300,
    });
  });

  it("不同工具的进度互不覆盖", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitLog(
        "codex",
        progressLine({ step: 1, total: 4, name: "nvm", phase: "start" }),
      );
      emitLog(
        "claude",
        progressLine({ step: 3, total: 4, name: "镜像", phase: "done" }),
      );
    });

    expect(result.current.progress.codex?.name).toBe("nvm");
    expect(result.current.progress.claude?.name).toBe("镜像");
  });

  it("后到的进度覆盖同一工具的前一条", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitLog(
        "codex",
        progressLine({ step: 1, total: 4, name: "nvm", phase: "start" }),
      );
      emitLog(
        "codex",
        progressLine({
          step: 2,
          total: 4,
          name: "Node.js LTS",
          phase: "start",
        }),
      );
    });

    expect(result.current.progress.codex?.step).toBe(2);
    expect(result.current.progress.codex?.name).toBe("Node.js LTS");
  });

  it("普通日志行不产生进度状态", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitLog("codex", "  [2/4] Node.js LTS");
      emitLog("codex", "npm WARN deprecated foo@1.0.0");
    });

    expect(result.current.progress.codex).toBeUndefined();
  });

  it("非法 JSON 不会让 hook 抛错", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitLog("codex", '{"type":"ofox-install-progress", 坏掉的');
    });

    expect(result.current.progress.codex).toBeUndefined();
  });

  it("type 不匹配的 JSON 行被忽略", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitLog("codex", JSON.stringify({ type: "something-else", step: 9 }));
    });

    expect(result.current.progress.codex).toBeUndefined();
  });

  it("安装结束后清掉该工具的进度，避免下次安装闪现旧值", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitLog(
        "codex",
        progressLine({ step: 4, total: 4, name: "Codex", phase: "done" }),
      );
    });
    expect(result.current.progress.codex).toBeDefined();

    act(() => {
      emitTauriEvent("install-tool-done", { tool: "codex", code: 0 });
    });

    expect(result.current.progress.codex).toBeUndefined();
  });
});

describe("useToolInstall 的失败反馈", () => {
  beforeAll(() => {
    i18n.addResourceBundle("zh", "translation", zh, true, true);
  });

  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(1);
  });

  const emitErr = (tool: string, line: string) =>
    emitTauriEvent("install-tool-log", { tool, stream: "stderr", line });

  it("安装脚本非零退出时，把它打印的错误原因暴露出来", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitLog("openclaw", "  OpenClaw macOS 环境初始化");
      emitErr(
        "openclaw",
        "  \u001b[31m\u001b[1m错误: 磁盘可用空间不足（2.3GB），至少需要 5GB。\u001b[0m",
      );
      emitTauriEvent("install-tool-done", { tool: "openclaw", code: 1 });
    });

    expect(result.current.error).toEqual({
      toolId: "openclaw",
      message: "磁盘可用空间不足（2.3GB），至少需要 5GB。",
    });
  });

  it("步骤失败时优先给出安装器的提示", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitLog("hermes", "       \u001b[31m\u001b[1m✕ Hermes 失败\u001b[0m");
      emitLog(
        "hermes",
        "       \u001b[33m提示: 安装命令超时；终端仍保持打开，请检查其中的日志。\u001b[0m",
      );
      emitLog("hermes", "  ℹ 已保存进度，下次运行将从断点继续。");
      emitTauriEvent("install-tool-done", { tool: "hermes", code: 1 });
    });

    expect(result.current.error?.message).toBe(
      "安装命令超时；终端仍保持打开，请检查其中的日志。",
    );
  });

  it("没有可识别的原因时至少给出退出码", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitTauriEvent("install-tool-done", { tool: "gemini", code: 2 });
    });

    expect(result.current.error).toEqual({
      toolId: "gemini",
      message: "安装未完成（退出码 2），请重试",
    });
  });

  it("安装成功不产生错误，重新安装时清掉上次的原因", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitErr("codex", "  错误: 网络不可用");
      emitTauriEvent("install-tool-done", { tool: "codex", code: 0 });
    });
    expect(result.current.error).toBeNull();

    act(() => {
      emitErr("codex", "  错误: 网络不可用");
    });
    await act(async () => {
      await result.current.install("codex");
    });
    act(() => {
      emitTauriEvent("install-tool-done", { tool: "codex", code: 3 });
    });

    expect(result.current.error?.message).toBe(
      "安装未完成（退出码 3），请重试",
    );
  });
});

describe("安装前的 Node.js 版本检查", () => {
  const tooOld = {
    status: "tooOld",
    required: ">=24.16.0 <25 || >=26.1.0",
    current: "24.15.0",
    manager: "fnm",
    canUpgrade: true,
    needsAdmin: false,
    reinstall: ["gemini"],
    manual: "fnm install 24 && fnm default 24",
  };
  const installCalls = () =>
    invokeMock.mock.calls.filter(([command]) => command === "install_tool");

  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("Node 不够新时先停下来问，不直接安装", async () => {
    invokeMock.mockImplementation(async (command: string) =>
      command === "check_tool_node_requirement" ? tooOld : 0,
    );
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await act(async () => result.current.install("openclaw"));

    expect(installCalls()).toHaveLength(0);
    expect(result.current.nodePrompt).toEqual({
      toolId: "openclaw",
      requirement: tooOld,
    });
    expect(result.current.installing.has("openclaw")).toBe(false);
  });

  it("用户同意后带着升级 Node 的要求安装", async () => {
    invokeMock.mockImplementation(async (command: string) =>
      command === "check_tool_node_requirement" ? tooOld : 0,
    );
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await act(async () => result.current.install("openclaw"));
    await act(async () => result.current.confirmNodeUpgrade());

    expect(installCalls()).toEqual([
      [
        "install_tool",
        { toolId: "openclaw", skipEnv: false, upgradeNode: true },
      ],
    ]);
    expect(result.current.nodePrompt).toBeNull();
  });

  it("用户取消就不安装", async () => {
    invokeMock.mockImplementation(async (command: string) =>
      command === "check_tool_node_requirement" ? tooOld : 0,
    );
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await act(async () => result.current.install("openclaw"));
    act(() => result.current.dismissNodePrompt());

    expect(result.current.nodePrompt).toBeNull();
    expect(installCalls()).toHaveLength(0);
  });

  it("版本够用、或检查本身失败时照常安装", async () => {
    invokeMock.mockImplementation(async (command: string) =>
      command === "check_tool_node_requirement" ? { status: "ok" } : 0,
    );
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await act(async () => result.current.install("gemini"));
    expect(installCalls()).toEqual([
      [
        "install_tool",
        { toolId: "gemini", skipEnv: false, upgradeNode: false },
      ],
    ]);

    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "check_tool_node_requirement") throw new Error("offline");
      return 0;
    });
    await act(async () => result.current.install("codex"));
    expect(installCalls()).toEqual([
      ["install_tool", { toolId: "codex", skipEnv: false, upgradeNode: false }],
    ]);
  });
});
