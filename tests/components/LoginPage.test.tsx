import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import LoginPage from "@/components/onboarding/LoginPage";

const auth = vi.hoisted(() => ({
  status: vi.fn(),
  retry: vi.fn(),
}));

vi.mock("@/lib/api/ofoxAuth", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/ofoxAuth")>()),
  ofoxGetAuthStatus: auth.status,
  ofoxRetryKeychain: auth.retry,
}));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.ai" }),
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

const user = { email: "u@example.com", name: "U" };

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  auth.status.mockReset();
  auth.retry.mockReset();
});

describe("LoginPage when the keychain prompt was denied", () => {
  it("explains it and restores the session once allowed", async () => {
    auth.status.mockResolvedValue({
      state: "loggedout",
      user: null,
      keychain_denied: true,
    });
    auth.retry.mockResolvedValue({
      state: "active",
      user,
      keychain_denied: false,
    });
    const onLoginSuccess = vi.fn();
    render(<LoginPage onLoginSuccess={onLoginSuccess} />);

    expect(
      await screen.findByText(/读取钥匙串里的登录信息时被拒绝了/),
    ).toBeVisible();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "重新授权" }));
    });
    expect(auth.retry).toHaveBeenCalled();
    expect(onLoginSuccess).toHaveBeenCalledWith(user);
  });

  it("shows nothing extra after an ordinary logout", async () => {
    auth.status.mockResolvedValue({ state: "loggedout", user: null });
    render(<LoginPage onLoginSuccess={vi.fn()} />);
    await act(async () => undefined);
    expect(screen.queryByRole("button", { name: "重新授权" })).toBeNull();
  });
});
