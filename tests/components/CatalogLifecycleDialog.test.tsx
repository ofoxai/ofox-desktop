import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { listen } from "@tauri-apps/api/event";
import { CatalogLifecycleDialog } from "@/components/tools/CatalogLifecycleDialog";
import {
  catalogLifecycleApi,
  type CatalogCopyState,
} from "@/lib/api/catalogLifecycle";
import { INSTALL_CATALOG } from "@/config/installCatalog";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
const missing: CatalogCopyState = {
  toolId: "copilot",
  supported: true,
  prefix: "/sandbox/managed/copilot",
  state: "missing",
  actions: ["install"],
  nodeVersion: "22.12.0",
};
const plan = {
  operationId: "job-1",
  toolId: "copilot",
  action: "install" as const,
  prefix: missing.prefix!,
  version: "1.0.98",
  nodeVersion: "22.12.0",
  command: "fixture-command",
  expiresAtMs: Date.now() + 60000,
};
const entry = INSTALL_CATALOG.find((row) => row.toolId === "copilot")!;
beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  vi.spyOn(catalogLifecycleApi, "inspect").mockResolvedValue(missing);
  vi.spyOn(catalogLifecycleApi, "prepare").mockResolvedValue(plan);
  vi.spyOn(catalogLifecycleApi, "cancel").mockResolvedValue(true);
  vi.spyOn(catalogLifecycleApi, "run").mockResolvedValue({
    operationId: "job-1",
    state: "ok",
    installation: {
      ...missing,
      state: "managed",
      version: "1.0.98",
      actions: ["update", "uninstall"],
    },
  });
  localStorage.setItem("ofox-bound-tools", '["codex"]');
});
it("previews the designated copy and subscribes before installing without changing bindings", async () => {
  let finishSubscribe!: (stop: () => void) => void;
  vi.mocked(listen).mockImplementation(
    () =>
      new Promise((resolve) => {
        finishSubscribe = resolve;
      }),
  );
  render(
    <CatalogLifecycleDialog
      entry={entry}
      onBusyChange={vi.fn()}
      onChanged={vi.fn().mockResolvedValue(undefined)}
    />,
  );
  await screen.findByText("/sandbox/managed/copilot");
  expect(catalogLifecycleApi.prepare).not.toHaveBeenCalled();
  expect(catalogLifecycleApi.run).not.toHaveBeenCalled();
  await userEvent.click(
    screen.getByRole("button", { name: "预览安装 OFox 副本" }),
  );
  await screen.findByText("fixture-command");
  expect(catalogLifecycleApi.prepare).toHaveBeenCalledWith(
    "copilot",
    "install",
  );
  await userEvent.click(screen.getByRole("button", { name: "开始安装" }));
  await waitFor(() => expect(listen).toHaveBeenCalled());
  expect(catalogLifecycleApi.run).not.toHaveBeenCalled();
  const stop = vi.fn();
  await act(async () => finishSubscribe(stop));
  await screen.findByText("操作完成");
  expect(catalogLifecycleApi.run).toHaveBeenCalledTimes(1);
  expect(catalogLifecycleApi.run).toHaveBeenCalledWith("job-1");
  expect(stop).toHaveBeenCalledTimes(1);
  expect(localStorage.getItem("ofox-bound-tools")).toBe('["codex"]');
});

it("keeps a completed operation successful when refreshing the directory fails", async () => {
  vi.mocked(listen).mockResolvedValue(vi.fn());
  const changed = vi
    .fn()
    .mockRejectedValue(new Error("fixture refresh failure"));
  render(
    <CatalogLifecycleDialog
      entry={entry}
      onBusyChange={vi.fn()}
      onChanged={changed}
    />,
  );
  await screen.findByText(missing.prefix!);
  await userEvent.click(
    screen.getByRole("button", { name: "预览安装 OFox 副本" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "开始安装" }),
  );
  await screen.findByText("操作完成");
  await waitFor(() => expect(changed).toHaveBeenCalledTimes(1));
  expect(catalogLifecycleApi.cancel).not.toHaveBeenCalled();
  expect(
    screen.queryByText("操作未能完成。请重新检测实际安装状态后再处理。"),
  ).not.toBeInTheDocument();
  expect(
    await screen.findByText("刷新安装目录失败；操作结果已保留。"),
  ).toBeVisible();
});
it("releases a prepared operation whose returned scope does not match", async () => {
  vi.mocked(catalogLifecycleApi.prepare).mockResolvedValue({
    ...plan,
    toolId: "cline",
  });
  render(
    <CatalogLifecycleDialog
      entry={entry}
      onBusyChange={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  await screen.findByText(missing.prefix!);
  await userEvent.click(
    screen.getByRole("button", { name: "预览安装 OFox 副本" }),
  );
  await screen.findByRole("alert");
  expect(catalogLifecycleApi.cancel).toHaveBeenCalledWith("job-1");
  expect(catalogLifecycleApi.run).not.toHaveBeenCalled();
});
it("does not accept an old inspection after the component changes tools", async () => {
  let finish!: (state: CatalogCopyState) => void;
  vi.mocked(catalogLifecycleApi.inspect)
    .mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    )
    .mockResolvedValueOnce({
      ...missing,
      toolId: "cline",
      prefix: "/sandbox/cline",
    });
  const other = INSTALL_CATALOG.find((row) => row.toolId === "cline")!;
  const rendered = render(
    <CatalogLifecycleDialog
      entry={entry}
      onBusyChange={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  rendered.rerender(
    <CatalogLifecycleDialog
      entry={other}
      onBusyChange={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  await screen.findByText("/sandbox/cline");
  await act(async () => finish(missing));
  expect(screen.queryByText(missing.prefix!)).not.toBeInTheDocument();
  expect(screen.getByText("/sandbox/cline")).toBeVisible();
});
it("does not accept a previous tool's completed job after its lifecycle has ended", async () => {
  let finish!: (
    result: Awaited<ReturnType<typeof catalogLifecycleApi.run>>,
  ) => void;
  vi.mocked(listen).mockResolvedValue(vi.fn());
  vi.mocked(catalogLifecycleApi.run).mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  const changed = vi.fn().mockResolvedValue(undefined);
  const rendered = render(
    <CatalogLifecycleDialog
      entry={entry}
      onBusyChange={vi.fn()}
      onChanged={changed}
    />,
  );
  await screen.findByText(missing.prefix!);
  await userEvent.click(
    screen.getByRole("button", { name: "预览安装 OFox 副本" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "开始安装" }),
  );
  await waitFor(() => expect(catalogLifecycleApi.run).toHaveBeenCalled());
  vi.mocked(catalogLifecycleApi.inspect).mockResolvedValueOnce({
    ...missing,
    toolId: "cline",
    prefix: "/sandbox/cline",
  });
  rendered.rerender(
    <CatalogLifecycleDialog
      entry={INSTALL_CATALOG.find((row) => row.toolId === "cline")!}
      onBusyChange={vi.fn()}
      onChanged={changed}
    />,
  );
  await screen.findByText("/sandbox/cline");
  await act(async () =>
    finish({
      operationId: "job-1",
      state: "ok",
      installation: {
        ...missing,
        state: "managed",
        actions: ["update", "uninstall"],
      },
    }),
  );
  expect(screen.queryByText("操作完成")).not.toBeInTheDocument();
  expect(screen.getByText("/sandbox/cline")).toBeVisible();
  expect(changed).not.toHaveBeenCalled();
});
it("cancels a late prepared operation after unmount without subscribing or executing", async () => {
  let finish!: (prepared: typeof plan) => void;
  vi.mocked(catalogLifecycleApi.prepare).mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  const rendered = render(
    <CatalogLifecycleDialog
      entry={entry}
      onBusyChange={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  await screen.findByText(missing.prefix!);
  await userEvent.click(
    screen.getByRole("button", { name: "预览安装 OFox 副本" }),
  );
  rendered.unmount();
  await act(async () => finish(plan));
  expect(catalogLifecycleApi.cancel).toHaveBeenCalledWith("job-1");
  expect(listen).not.toHaveBeenCalled();
  expect(catalogLifecycleApi.run).not.toHaveBeenCalled();
});
it("closes a late subscription exactly once after unmount and never starts the job", async () => {
  let finish!: (stop: () => void) => void;
  vi.mocked(listen).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const rendered = render(
    <CatalogLifecycleDialog
      entry={entry}
      onBusyChange={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  await screen.findByText(missing.prefix!);
  await userEvent.click(
    screen.getByRole("button", { name: "预览安装 OFox 副本" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "开始安装" }),
  );
  await waitFor(() => expect(listen).toHaveBeenCalled());
  rendered.unmount();
  const stop = vi.fn();
  await act(async () => finish(stop));
  expect(stop).toHaveBeenCalledTimes(1);
  expect(catalogLifecycleApi.cancel).toHaveBeenCalledWith("job-1");
  expect(catalogLifecycleApi.run).not.toHaveBeenCalled();
});
it("ignores an old cancellation failure after switching to another tool", async () => {
  let rejectCancel!: (error: Error) => void;
  vi.mocked(listen).mockResolvedValue(vi.fn());
  vi.mocked(catalogLifecycleApi.run).mockReturnValue(
    new Promise(() => undefined),
  );
  vi.mocked(catalogLifecycleApi.cancel).mockReturnValue(
    new Promise((_resolve, reject) => {
      rejectCancel = reject;
    }),
  );
  const rendered = render(
    <CatalogLifecycleDialog
      entry={entry}
      onBusyChange={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  await screen.findByText(missing.prefix!);
  await userEvent.click(
    screen.getByRole("button", { name: "预览安装 OFox 副本" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "开始安装" }),
  );
  await waitFor(() => expect(catalogLifecycleApi.run).toHaveBeenCalled());
  await userEvent.click(screen.getByRole("button", { name: "取消操作" }));
  const oldFailure = rejectCancel;
  vi.mocked(catalogLifecycleApi.cancel).mockResolvedValue(true);
  vi.mocked(catalogLifecycleApi.inspect).mockResolvedValueOnce({
    ...missing,
    toolId: "cline",
    prefix: "/sandbox/cline",
  });
  rendered.rerender(
    <CatalogLifecycleDialog
      entry={INSTALL_CATALOG.find((row) => row.toolId === "cline")!}
      onBusyChange={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  await screen.findByText("/sandbox/cline");
  await act(async () => oldFailure(new Error("old cancel failure")));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it.each([
  ["update", "预览更新此副本"],
  ["uninstall", "预览卸载此副本"],
] as const)(
  "starts a fresh %s preview at verification after cancellation",
  async (action, label) => {
    vi.mocked(catalogLifecycleApi.inspect).mockResolvedValue({
      ...missing,
      state: "managed",
      version: "1.0.98",
      actions: ["update", "uninstall"],
    });
    let finish!: (
      prepared: Awaited<ReturnType<typeof catalogLifecycleApi.prepare>>,
    ) => void;
    vi.mocked(catalogLifecycleApi.prepare).mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    render(
      <CatalogLifecycleDialog
        entry={entry}
        onBusyChange={vi.fn()}
        onChanged={vi.fn()}
      />,
    );
    await userEvent.click(await screen.findByRole("button", { name: label }));
    await userEvent.click(screen.getByRole("button", { name: "取消操作" }));
    expect(screen.getByRole("status")).toHaveTextContent("正在回收进程");
    await act(async () => finish({ ...plan, action }));
    vi.mocked(catalogLifecycleApi.prepare).mockReturnValueOnce(
      new Promise(() => undefined),
    );
    await userEvent.click(await screen.findByRole("button", { name: label }));
    expect(screen.getByRole("status")).toHaveTextContent("核对来源");
    await userEvent.click(screen.getByRole("button", { name: "取消操作" }));
    expect(screen.getByRole("button", { name: "取消操作" })).toBeDisabled();
    expect(catalogLifecycleApi.run).not.toHaveBeenCalled();
  },
);
