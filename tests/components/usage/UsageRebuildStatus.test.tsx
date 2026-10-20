import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageRebuildStatus } from "@/components/usage-overview/UsageRebuildStatus";
import { queryClient } from "@/lib/query/queryClient";
import type { UsageRebuildProgress } from "@/types/usageQuery";

const mocks = vi.hoisted(() => ({ read: vi.fn(), cancel: vi.fn() }));
vi.mock("@/lib/api/usageOverview", () => ({
  getUsageRebuildProgress: mocks.read,
  cancelUsageRebuild: mocks.cancel,
}));
const running: UsageRebuildProgress = {
  operationId: "run-a",
  state: "running",
  completedSources: 1,
  totalSources: 9,
  errorCode: null,
};
beforeEach(() => {
  queryClient.clear();
  mocks.read.mockReset().mockResolvedValue(running);
  mocks.cancel.mockReset();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});

it("does not call an older backend or poll while the usage page is hidden", async () => {
  const { rerender } = render(
    <UsageRebuildStatus active supported={false} onDismiss={vi.fn()} />,
  );
  expect(mocks.read).not.toHaveBeenCalled();
  rerender(<UsageRebuildStatus active={false} supported onDismiss={vi.fn()} />);
  expect(mocks.read).not.toHaveBeenCalled();
  rerender(<UsageRebuildStatus active supported onDismiss={vi.fn()} />);
  expect(
    await screen.findByText("正在重建 Codex 本机统计：1 / 9 个来源"),
  ).toBeVisible();
});

it("cannot replace a new rebuild with a delayed cancel receipt from an older operation", async () => {
  let finish!: (value: UsageRebuildProgress) => void;
  mocks.cancel.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  render(<UsageRebuildStatus active supported onDismiss={vi.fn()} />);
  await userEvent.click(
    await screen.findByRole("button", { name: "取消重建" }),
  );
  await act(async () => {
    queryClient.setQueryData(["usageNativeRebuildProgress"], {
      ...running,
      operationId: "run-b",
      completedSources: 2,
    });
  });
  expect(
    await screen.findByText("正在重建 Codex 本机统计：2 / 9 个来源"),
  ).toBeVisible();
  await act(async () => {
    finish({ ...running, state: "cancelling" });
  });
  expect(
    screen.getByText("正在重建 Codex 本机统计：2 / 9 个来源"),
  ).toBeVisible();
  expect(screen.queryByText("正在停止后续重建，保留原有统计…")).toBeNull();
  expect(
    queryClient.getQueryData(["usageNativeRebuildProgress"]),
  ).toMatchObject({ operationId: "run-b" });
});

it("retains keyboard focus through cancellation and dismisses without touching account sync", async () => {
  mocks.cancel.mockResolvedValue({ ...running, state: "cancelled" });
  const dismiss = vi.fn();
  render(<UsageRebuildStatus active supported onDismiss={dismiss} />);
  const cancel = await screen.findByRole("button", { name: "取消重建" });
  cancel.focus();
  await userEvent.keyboard("{Enter}");
  const close = await screen.findByRole("button", { name: "关闭" });
  await waitFor(() => expect(close).toHaveFocus());
  await userEvent.keyboard("{Enter}");
  expect(dismiss).toHaveBeenCalledOnce();
  expect(mocks.cancel).toHaveBeenCalledWith("run-a");
  expect(screen.queryByText("重建已取消，原有统计已保留。")).toBeNull();
});

it("hides a completed rebuild and keeps it hidden when an earlier cancel reply arrives", async () => {
  let finish!: (value: UsageRebuildProgress) => void;
  mocks.cancel.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  render(<UsageRebuildStatus active supported onDismiss={vi.fn()} />);
  await userEvent.click(
    await screen.findByRole("button", { name: "取消重建" }),
  );
  await act(async () => {
    queryClient.setQueryData(["usageNativeRebuildProgress"], {
      ...running,
      state: "completed",
    });
  });
  await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  await act(async () => {
    finish({ ...running, state: "cancelling" });
  });
  expect(screen.queryByRole("status")).toBeNull();
});

it("returns focus to retry when a progress read failure removes the focused cancel action", async () => {
  render(<UsageRebuildStatus active supported onDismiss={vi.fn()} />);
  const cancel = await screen.findByRole("button", { name: "取消重建" });
  cancel.focus();
  mocks.read.mockRejectedValue(new Error("unavailable"));
  await act(async () => {
    await queryClient.invalidateQueries({
      queryKey: ["usageNativeRebuildProgress"],
    });
  });
  const retry = await screen.findByRole("button", { name: "重试" });
  expect(retry).toHaveFocus();
});

it("does not steal focus from another control when a background progress read fails", async () => {
  render(
    <>
      <button>other control</button>
      <UsageRebuildStatus active supported onDismiss={vi.fn()} />
    </>,
  );
  await screen.findByRole("button", { name: "取消重建" });
  const other = screen.getByRole("button", { name: "other control" });
  other.focus();
  mocks.read.mockRejectedValue(new Error("unavailable"));
  await act(async () => {
    await queryClient.invalidateQueries({
      queryKey: ["usageNativeRebuildProgress"],
    });
  });
  await screen.findByRole("button", { name: "重试" });
  expect(other).toHaveFocus();
});
