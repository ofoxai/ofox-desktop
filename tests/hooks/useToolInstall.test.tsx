import { renderHook, act, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
      emitLog("codex", progressLine({ step: 1, total: 4, name: "nvm", phase: "start" }));
      emitLog("claude", progressLine({ step: 3, total: 4, name: "镜像", phase: "done" }));
    });

    expect(result.current.progress.codex?.name).toBe("nvm");
    expect(result.current.progress.claude?.name).toBe("镜像");
  });

  it("后到的进度覆盖同一工具的前一条", async () => {
    const { result } = renderHook(() => useToolInstall(vi.fn()));
    await waitFor(() => expect(typeof result.current.progress).toBe("object"));

    act(() => {
      emitLog("codex", progressLine({ step: 1, total: 4, name: "nvm", phase: "start" }));
      emitLog("codex", progressLine({ step: 2, total: 4, name: "Node.js LTS", phase: "start" }));
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
      emitLog("codex", progressLine({ step: 4, total: 4, name: "Codex", phase: "done" }));
    });
    expect(result.current.progress.codex).toBeDefined();

    act(() => {
      emitTauriEvent("install-tool-done", { tool: "codex", code: 0 });
    });

    expect(result.current.progress.codex).toBeUndefined();
  });
});
