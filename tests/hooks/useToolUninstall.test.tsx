import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { useToolUninstall } from "@/hooks/useToolUninstall";
import { useToolInstall } from "@/hooks/useToolInstall";
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  forget: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@/lib/bindTools", () => ({
  readBoundTools: () =>
    JSON.parse(localStorage.getItem("ofox-bound-tools") || "[]"),
  forgetUninstalledTool: mocks.forget,
}));
const partial = {
  status: "stillPresent",
  unbind: {},
  key: "keptShared",
  remainingPath: "/sandbox/bin/codex",
  kept: ["~/.codex"],
};
beforeEach(() => {
  vi.resetAllMocks();
  localStorage.setItem("ofox-bound-tools", '["codex"]');
  mocks.listen.mockResolvedValue(vi.fn());
  mocks.invoke.mockResolvedValue(partial);
  mocks.forget.mockResolvedValue(undefined);
});
it("subscribes before executing, ignores other operations, and submits once with latest siblings", async () => {
  let ready!: (stop: () => void) => void;
  let handler!: (event: { payload: unknown }) => void;
  let finish!: (result: unknown) => void;
  const off = vi.fn();
  mocks.listen.mockImplementation((_event, fn) => {
    handler = fn;
    return new Promise((r) => {
      ready = r;
    });
  });
  mocks.invoke.mockImplementation(
    () =>
      new Promise((r) => {
        finish = r;
      }),
  );
  const completed = vi.fn();
  const { result } = renderHook(() => useToolUninstall(completed));
  act(() => {
    void result.current.run("codex");
    void result.current.run("codex");
  });
  expect(mocks.invoke).not.toHaveBeenCalled();
  localStorage.setItem("ofox-bound-tools", '["codex","chatgpt"]');
  await act(async () => {
    ready(off);
  });
  expect(mocks.invoke).toHaveBeenCalledTimes(1);
  const [, args] = mocks.invoke.mock.calls[0];
  expect(args.stillBound).toEqual(["codex", "chatgpt"]);
  act(() =>
    handler({
      payload: {
        operationId: "other",
        line: "unrelated",
        step: 3,
        totalSteps: 3,
      },
    }),
  );
  expect(result.current.logs).toEqual([]);
  act(() =>
    handler({
      payload: {
        operationId: args.operationId,
        line: "restoring",
        step: 1,
        totalSteps: 3,
      },
    }),
  );
  expect(result.current.progress?.step).toBe(1);
  expect(result.current.logs).toEqual(["restoring"]);
  await act(async () => {
    finish(partial);
  });
  expect(mocks.forget).toHaveBeenCalledWith("codex", partial);
  expect(completed).toHaveBeenCalledWith(partial);
  expect(result.current.result).toEqual(partial);
  expect(off).toHaveBeenCalledOnce();
});
it("does not execute if subscription fails", async () => {
  mocks.listen.mockRejectedValue(new Error("cannot subscribe"));
  const { result } = renderHook(() => useToolUninstall(vi.fn()));
  await act(async () => {
    await result.current.run("claude");
  });
  expect(mocks.invoke).not.toHaveBeenCalled();
  expect(result.current.error).toContain("cannot subscribe");
  expect(result.current.running).toBe(false);
});
it("disposes a delayed subscription after unmount and never starts removal", async () => {
  let ready!: (stop: () => void) => void;
  const off = vi.fn();
  mocks.listen.mockImplementation(
    () =>
      new Promise((r) => {
        ready = r;
      }),
  );
  const { result, unmount } = renderHook(() => useToolUninstall(vi.fn()));
  act(() => {
    void result.current.run("claude");
  });
  unmount();
  await act(async () => {
    ready(off);
  });
  expect(off).toHaveBeenCalledOnce();
  expect(mocks.invoke).not.toHaveBeenCalled();
});
it("installation also cleans listeners that resolve after unmount", async () => {
  const pending: Array<(stop: () => void) => void> = [];
  const off = vi.fn();
  mocks.listen.mockImplementation(() => new Promise((r) => pending.push(r)));
  const { unmount } = renderHook(() => useToolInstall(vi.fn()));
  unmount();
  await act(async () => {
    for (const resolve of pending) resolve(off);
  });
  await waitFor(() => expect(off).toHaveBeenCalled());
});

it("finishes binding persistence but drops callbacks after an in-flight uninstall unmounts", async () => {
  let finish!: (result: unknown) => void;
  mocks.invoke.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const completed = vi.fn();
  const { result, unmount } = renderHook(() => useToolUninstall(completed));
  act(() => {
    void result.current.run("codex");
  });
  await waitFor(() => expect(finish).toBeTypeOf("function"));
  unmount();
  await act(async () => finish(partial));
  expect(mocks.forget).toHaveBeenCalledWith("codex", partial);
  expect(completed).not.toHaveBeenCalled();
});
it("calls the latest mounted completion callback", async () => {
  let finish!: (result: unknown) => void;
  mocks.invoke.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const old = vi.fn(),
    latest = vi.fn();
  const { result, rerender } = renderHook(
    ({ completed }) => useToolUninstall(completed),
    { initialProps: { completed: old } },
  );
  act(() => {
    void result.current.run("codex");
  });
  await waitFor(() => expect(finish).toBeTypeOf("function"));
  rerender({ completed: latest });
  await act(async () => finish(partial));
  expect(old).not.toHaveBeenCalled();
  expect(latest).toHaveBeenCalledWith(partial);
});

it("rechecks ownership between listener readiness and command startup", async () => {
  let ready!: (stop: () => void) => void;
  mocks.listen.mockImplementation(
    () =>
      new Promise((resolve) => {
        ready = resolve;
      }),
  );
  const { result, unmount } = renderHook(() => useToolUninstall(vi.fn()));
  act(() => {
    void result.current.run("claude");
  });
  await act(async () => {
    ready(vi.fn());
    queueMicrotask(unmount);
  });
  expect(mocks.invoke).not.toHaveBeenCalled();
});
