import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import LoginPage from "@/components/onboarding/LoginPage";

const mocks = vi.hoisted(() => ({
  start: vi.fn(),
  status: vi.fn(),
  open: vi.fn(),
  poll: vi.fn(),
}));
vi.mock("@/lib/api/ofoxAuth", () => ({
  ofoxStartLogin: mocks.start,
  ofoxGetAuthStatus: mocks.status,
  ofoxPollForToken: mocks.poll,
  ofoxRetryKeychain: vi.fn(),
}));
vi.mock("@/lib/api", () => ({ settingsApi: { openExternal: mocks.open } }));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.ai" }),
}));
beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  mocks.status.mockResolvedValue({ state: "loggedout" });
});

it("clears an obsolete authorization check when a new code is generated", async () => {
  mocks.start.mockResolvedValue({
    device_code: "fixture",
    user_code: "fixture",
    interval: 5,
    expires_in: 60,
    verification_uri: "https://fixture.invalid",
  });
  mocks.open.mockResolvedValue(undefined);
  render(<LoginPage onLoginSuccess={vi.fn()} />);
  fireEvent.click(
    screen.getByRole("button", { name: i18n.t("uiCopy.loginArrow") }),
  );
  const check = await screen.findByRole("button", {
    name: i18n.t("uiCopy.authorized"),
  });
  await act(async () => fireEvent.click(check));
  const regenerate = screen.getByRole("button", {
    name: i18n.t("uiCopy.regenerate"),
  });
  let resolve!: (value: unknown) => void;
  mocks.status.mockReturnValue(
    new Promise((r) => {
      resolve = r;
    }),
  );
  fireEvent.click(check);
  await act(async () => fireEvent.click(regenerate));
  await act(async () => resolve({ state: "loggedout" }));
  expect(
    screen.getByRole("button", { name: i18n.t("uiCopy.authorized") }),
  ).toBeEnabled();
});

it("does not open the browser or start polling after the user enters local management", async () => {
  let resolve!: (value: unknown) => void;
  mocks.start.mockReturnValue(
    new Promise((r) => {
      resolve = r;
    }),
  );
  const success = vi.fn();
  const enter = vi.fn();
  render(<LoginPage onLoginSuccess={success} onEnterLocal={enter} />);
  fireEvent.click(
    screen.getByRole("button", { name: i18n.t("uiCopy.loginArrow") }),
  );
  fireEvent.click(
    screen.getByRole("button", { name: i18n.t("toolCatalog.enterLocal") }),
  );
  await act(async () =>
    resolve({
      device_code: "fixture",
      user_code: "fixture",
      interval: 5,
      expires_in: 60,
      verification_uri: "https://fixture.invalid",
    }),
  );
  expect(enter).toHaveBeenCalledOnce();
  expect(mocks.open).not.toHaveBeenCalled();
  expect(mocks.poll).not.toHaveBeenCalled();
  expect(success).not.toHaveBeenCalled();
});
