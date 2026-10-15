import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { OptionalToolsCatalog } from "@/components/tools/OptionalToolsCatalog";
import { catalogLifecycleApi } from "@/lib/api/catalogLifecycle";
const mocks = vi.hoisted(() => ({
  inventory: vi.fn(),
  invoke: vi.fn(),
  cancel: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("@/lib/api/installCatalog", () => ({
  getInstallInventory: mocks.inventory,
}));
vi.mock("@/lib/api", () => ({ settingsApi: { openExternal: vi.fn() } }));
vi.mock("@/lib/clipboard", () => ({ copyText: vi.fn() }));
beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  const managed = {
    toolId: "copilot",
    supported: true,
    prefix: "/private/prefix",
    state: "managed" as const,
    version: "1.0.98",
    nodeVersion: "22.12.0",
    actions: ["uninstall" as const],
  };
  vi.spyOn(catalogLifecycleApi, "inspect").mockResolvedValue(managed);
  vi.spyOn(catalogLifecycleApi, "prepare").mockResolvedValue({
    operationId: "running-catalog-job",
    toolId: "copilot",
    action: "uninstall",
    prefix: "/private/prefix",
    version: "1.0.98",
    nodeVersion: "22.12.0",
    command: "fixed verified command",
    expiresAtMs: Date.now() + 60000,
  });
  vi.spyOn(catalogLifecycleApi, "cancel").mockResolvedValue(true);
  mocks.inventory
    .mockResolvedValueOnce([
      {
        toolId: "copilot",
        status: "notInstalled",
        managedCopy: {
          state: "managed",
          version: "1.0.98",
          path: "/private/prefix/bin/copilot",
        },
      },
    ])
    .mockResolvedValue([
      {
        toolId: "copilot",
        status: "notInstalled",
        managedCopy: { state: "needsReview", path: "/private/prefix" },
      },
    ]);
});
it("invalidates verified evidence during a job and refreshes after cancellation and dialog closure", async () => {
  let finish!: (value: unknown) => void;
  mocks.invoke.mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  render(<OptionalToolsCatalog boundTools={[]} />);
  await userEvent.click(screen.getByRole("button", { name: /其他已安装工具/ }));
  await waitFor(() =>
    expect(screen.getByText(/OFox 管理副本.*1.0.98/)).toBeVisible(),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "管理 GitHub Copilot CLI 安装" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "预览卸载此副本" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "卸载此副本并保留数据" }),
  );
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("run_catalog_operation", {
      operationId: "running-catalog-job",
    }),
  );
  await userEvent.click(screen.getByRole("button", { name: "取消操作" }));
  await userEvent.click(screen.getByRole("button", { name: "关闭" }));
  try {
    expect(screen.queryByText(/OFox 管理副本.*1.0.98/)).not.toBeInTheDocument();
  } finally {
    await act(async () =>
      finish({
        operationId: "running-catalog-job",
        state: "partial",
        installation: {
          toolId: "copilot",
          supported: true,
          prefix: "/private/prefix",
          state: "needsReview",
          actions: [],
        },
      }),
    );
  }
  await waitFor(() => expect(mocks.inventory).toHaveBeenCalledTimes(2));
  fireEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
  expect(await screen.findByText("OFox 副本需要核对")).toBeVisible();
});

it("keeps a remounted directory unverified until the outstanding operation settles", async () => {
  const verified = [
    {
      toolId: "copilot",
      status: "notInstalled",
      managedCopy: {
        state: "managed",
        version: "1.0.98",
        path: "/private/prefix/bin/copilot",
      },
    },
  ];
  const unknown = [
    {
      toolId: "copilot",
      status: "notInstalled",
      managedCopy: { state: "needsReview", path: "/private/prefix" },
    },
  ];
  mocks.inventory
    .mockReset()
    .mockResolvedValueOnce(verified)
    .mockResolvedValueOnce(verified)
    .mockResolvedValue(unknown);
  let finish!: (value: unknown) => void;
  mocks.invoke.mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  const first = render(<OptionalToolsCatalog boundTools={[]} />);
  await userEvent.click(screen.getByRole("button", { name: /其他已安装工具/ }));
  await screen.findByText(/OFox 管理副本.*1.0.98/);
  let pending!: ReturnType<typeof catalogLifecycleApi.run>;
  act(() => {
    pending = catalogLifecycleApi.run("remounted-operation");
  });
  first.unmount();
  render(<OptionalToolsCatalog boundTools={[]} />);
  await userEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
  try {
    expect(await screen.findByText("OFox 副本需要核对")).toBeVisible();
    expect(screen.queryByText(/OFox 管理副本.*1.0.98/)).not.toBeInTheDocument();
  } finally {
    await act(async () => {
      finish({ operationId: "remounted-operation", state: "partial" });
      await pending;
    });
  }
  await waitFor(() => expect(mocks.inventory).toHaveBeenCalledTimes(3));
  expect(screen.getByText("OFox 副本需要核对")).toBeVisible();
});

it("shares a failed settle refresh with the open dialog and performs a new query on retry", async () => {
  mocks.inventory
    .mockReset()
    .mockResolvedValueOnce([
      {
        toolId: "copilot",
        status: "notInstalled",
        managedCopy: {
          state: "managed",
          version: "1.0.98",
          path: "/private/prefix/bin/copilot",
        },
      },
    ])
    .mockRejectedValueOnce(new Error("fixture inventory failure"))
    .mockResolvedValue([{ toolId: "copilot", status: "notInstalled" }]);
  let finish!: (value: unknown) => void;
  mocks.invoke.mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  render(<OptionalToolsCatalog boundTools={[]} />);
  await userEvent.click(screen.getByRole("button", { name: /其他已安装工具/ }));
  await screen.findByText(/OFox 管理副本.*1.0.98/);
  await userEvent.click(
    screen.getByRole("button", { name: "管理 GitHub Copilot CLI 安装" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "预览卸载此副本" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "卸载此副本并保留数据" }),
  );
  await act(async () =>
    finish({
      operationId: "running-catalog-job",
      state: "ok",
      installation: {
        toolId: "copilot",
        supported: true,
        prefix: "/private/prefix",
        state: "missing",
        actions: ["install"],
      },
    }),
  );
  expect(
    await screen.findByText("刷新安装目录失败；操作结果已保留。"),
  ).toBeVisible();
  expect(screen.getByText("操作完成")).toBeVisible();
  expect(mocks.inventory).toHaveBeenCalledTimes(2);
  await userEvent.click(screen.getByRole("button", { name: "重试" }));
  await waitFor(() => expect(mocks.inventory).toHaveBeenCalledTimes(3));
  await waitFor(() =>
    expect(
      screen.queryByText("刷新安装目录失败；操作结果已保留。"),
    ).not.toBeInTheDocument(),
  );
});

it("refreshes past an older inventory request when the run IPC rejects", async () => {
  const verified = [
    {
      toolId: "copilot",
      status: "notInstalled",
      managedCopy: {
        state: "managed",
        version: "1.0.98",
        path: "/private/prefix/bin/copilot",
      },
    },
  ];
  let oldInventory!: (value: unknown) => void;
  mocks.inventory
    .mockReset()
    .mockResolvedValueOnce(verified)
    .mockReturnValueOnce(
      new Promise((resolve) => {
        oldInventory = resolve;
      }),
    )
    .mockResolvedValue([
      {
        toolId: "copilot",
        status: "notInstalled",
        managedCopy: { state: "needsReview", path: "/private/prefix" },
      },
    ]);
  let rejectRun!: (error: Error) => void;
  mocks.invoke.mockReturnValueOnce(
    new Promise((_resolve, reject) => {
      rejectRun = reject;
    }),
  );
  const ref = {
    current: null as
      | import("@/components/tools/OptionalToolsCatalog").InstallCatalogHandle
      | null,
  };
  render(<OptionalToolsCatalog ref={ref} boundTools={[]} />);
  await userEvent.click(screen.getByRole("button", { name: /其他已安装工具/ }));
  await screen.findByText(/OFox 管理副本.*1.0.98/);
  let oldQuery!: Promise<void>;
  act(() => {
    oldQuery = ref.current!.refresh();
  });
  let pending!: ReturnType<typeof catalogLifecycleApi.run>;
  act(() => {
    pending = catalogLifecycleApi.run("rejected-operation");
  });
  await act(async () => {
    rejectRun(new Error("fixture lost response"));
    await pending.catch(() => undefined);
  });
  expect(mocks.inventory).toHaveBeenCalledTimes(2);
  await act(async () => {
    oldInventory(verified);
    await oldQuery;
  });
  await waitFor(() => expect(mocks.inventory).toHaveBeenCalledTimes(3));
  await userEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
  expect(await screen.findByText("OFox 副本需要核对")).toBeVisible();
  expect(screen.queryByText(/OFox 管理副本.*1.0.98/)).not.toBeInTheDocument();
});
