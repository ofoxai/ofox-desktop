import { beforeEach, describe, expect, it, vi } from "vitest";
import { toolUninstallApi } from "@/lib/api/toolUninstall";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: tauri.listen }));

beforeEach(() => {
  tauri.invoke.mockReset();
  tauri.listen.mockReset();
});

describe("toolUninstallApi", () => {
  it("asks for a preview and starts an uninstall under the backend command names", async () => {
    tauri.invoke.mockResolvedValue({});
    await toolUninstallApi.preview("codex", ["chatgpt"]);
    await toolUninstallApi.uninstall("codex", "op-1", ["chatgpt"]);
    expect(tauri.invoke).toHaveBeenNthCalledWith(1, "uninstall_tool_preview", {
      tool: "codex",
      stillBound: ["chatgpt"],
    });
    expect(tauri.invoke).toHaveBeenNthCalledWith(2, "uninstall_tool", {
      tool: "codex",
      operationId: "op-1",
      stillBound: ["chatgpt"],
    });
  });

  it("sends an empty list when the caller has none to report", async () => {
    tauri.invoke.mockResolvedValue({});
    await toolUninstallApi.preview("claude");
    await toolUninstallApi.uninstall("claude", "op-2");
    expect(tauri.invoke).toHaveBeenNthCalledWith(1, "uninstall_tool_preview", {
      tool: "claude",
      stillBound: [],
    });
    expect(tauri.invoke).toHaveBeenNthCalledWith(2, "uninstall_tool", {
      tool: "claude",
      operationId: "op-2",
      stillBound: [],
    });
  });

  it("hands over only the progress of its own operation", async () => {
    const unlisten = vi.fn();
    let deliver: (event: { payload: unknown }) => void = () => {};
    tauri.listen.mockImplementation(async (_name, handler) => {
      deliver = handler;
      return unlisten;
    });
    const seen: string[] = [];
    const stop = await toolUninstallApi.onProgress("op-1", (progress) =>
      seen.push(progress.line),
    );

    deliver({
      payload: { operationId: "op-2", line: "other", step: 3, totalSteps: 3 },
    });
    deliver({
      payload: { operationId: "op-1", line: "mine", step: 3, totalSteps: 3 },
    });

    expect(tauri.listen).toHaveBeenCalledWith(
      "tool-uninstall-progress",
      expect.any(Function),
    );
    expect(seen).toEqual(["mine"]);
    expect(stop).toBe(unlisten);
  });
});
