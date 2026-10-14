import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { SettingsHealthBanner } from "@/components/console/SettingsHealthBanner";
import { emitTauriEvent } from "../msw/tauriMocks";
const mocks = vi.hoisted(() => ({ get: vi.fn(), reset: vi.fn() }));
vi.mock("@/lib/api/settings", () => ({
  getSettingsHealth: mocks.get,
  resetUnreadableSettings: mocks.reset,
  SETTINGS_HEALTH_EVENT: "settings-health-changed",
}));
const unhealthy = {
  state: "unreadable",
  path: "/sandbox/settings.json",
  badCopy: "/sandbox/settings.bad.json",
  detail: "invalid JSON",
};
beforeEach(() => {
  vi.resetAllMocks();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  mocks.get.mockResolvedValue(unhealthy);
  mocks.reset.mockResolvedValue({ ...unhealthy, state: "ok" });
});
it("queries on mount and requires explicit confirmation before resetting", async () => {
  render(<SettingsHealthBanner />);
  expect(await screen.findByText("设置无法读取，已暂停写入")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "恢复默认配置…" }));
  expect(mocks.reset).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  expect(mocks.reset).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "恢复默认配置…" }));
  fireEvent.click(screen.getByRole("button", { name: "确认恢复默认配置" }));
  await waitFor(() => expect(mocks.reset).toHaveBeenCalledOnce());
  await waitFor(() =>
    expect(screen.queryByText("设置无法读取，已暂停写入")).toBeNull(),
  );
});
it("follows health events and prevents an older initial query from hiding recovery", async () => {
  let finish!: (result: unknown) => void;
  mocks.get.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  render(<SettingsHealthBanner />);
  await waitFor(() => expect(finish).toBeTypeOf("function"));
  act(() =>
    emitTauriEvent("settings-health-changed", {
      ...unhealthy,
      state: "recovered",
    }),
  );
  await act(async () => finish({ ...unhealthy, state: "ok" }));
  expect(screen.getByText("设置已从备份恢复")).toBeVisible();
});

it("uses danger for unreadable settings and allows dismissing a recovery until the next event", async () => {
  render(<SettingsHealthBanner />);
  expect(await screen.findByRole("alert")).toHaveClass("bg-danger-soft");
  expect(screen.queryByRole("button", { name: "关闭" })).toBeNull();
  act(() => emitTauriEvent("settings-health-changed", { ...unhealthy, state: "recovered" }));
  expect(screen.getByRole("alert")).toHaveClass("bg-warn-soft");
  fireEvent.click(screen.getByRole("button", { name: "关闭" }));
  expect(screen.queryByRole("alert")).toBeNull();
  act(() => emitTauriEvent("settings-health-changed", unhealthy));
  expect(screen.getByRole("alert")).toHaveClass("bg-danger-soft");
});
