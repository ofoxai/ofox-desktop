import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import { BalanceCard } from "@/components/console/BalanceCard";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import type { OfoxUserInfo } from "@/lib/api/ofoxAuth";

const mocks = vi.hoisted(() => ({
  openExternal: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.io" }),
}));
vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: mocks.openExternal },
}));
vi.mock("@/components/UserAvatar", () => ({ UserAvatar: () => null }));

function user(overrides: Partial<OfoxUserInfo> = {}): OfoxUserInfo {
  return {
    name: "Test User",
    email: "test@example.com",
    org_id: "org-1",
    avatar_url: null,
    role: "member",
    balance: { balance: 12.5 },
    ...overrides,
  };
}

function account(overrides: Partial<OfoxAccount> = {}): OfoxAccount {
  return {
    user: user(),
    retrying: false,
    retryAttempt: 0,
    loadFailed: false,
    balanceRefreshing: false,
    loadUser: vi.fn(),
    retryUser: vi.fn().mockResolvedValue(undefined),
    refreshBalance: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("zh");
  mocks.openExternal.mockClear();
});
afterEach(async () => {
  await i18n.changeLanguage("zh");
});

describe("BalanceCard", () => {
  it("has no refresh of its own: the header's refresh reloads the balance", () => {
    render(<BalanceCard account={account()} />);
    const card = screen.getByTestId("balance-card");
    expect(within(card).getByText("$12.50")).toBeVisible();
    expect(within(card).queryAllByRole("button")).toHaveLength(0);
  });

  it("lets a billing manager top up in the wallet of the current region", async () => {
    render(
      <BalanceCard account={account({ user: user({ role: "owner" }) })} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "充值" }));
    expect(mocks.openExternal).toHaveBeenCalledWith(
      "https://app.ofox.io/manage/wallet",
    );
  });

  it("shows a spent balance in red on a danger-tinted card", () => {
    render(
      <BalanceCard
        account={account({ user: user({ balance: { balance: 0 } }) })}
      />,
    );
    const card = screen.getByTestId("balance-card");
    expect(card).toHaveAttribute("data-empty", "true");
    expect(card).toHaveClass("border-danger/40", "bg-danger-soft/60");
    expect(within(card).getByText("$0.00")).toHaveClass("text-danger");
  });

  it("offers a retry once the user could not be loaded", async () => {
    const retryUser = vi.fn().mockResolvedValue(undefined);
    render(
      <BalanceCard
        account={account({ user: null, loadFailed: true, retryUser })}
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "无法加载用户信息，点击重试" }),
    );
    expect(retryUser).toHaveBeenCalledTimes(1);
  });

  it("speaks the app's language", async () => {
    await i18n.changeLanguage("en");
    render(
      <BalanceCard
        account={account({ user: null, retrying: true, retryAttempt: 2 })}
      />,
    );
    const card = screen.getByTestId("balance-card");
    expect(card).toHaveTextContent("Ofox balance");
    expect(card).toHaveTextContent("Loading your account (2/3)");
    expect(card).not.toHaveTextContent(/[一-鿿]/);
  });
});
