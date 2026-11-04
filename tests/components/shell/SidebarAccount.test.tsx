import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { ShellProvider } from "@/components/shell/ShellContext";
import {
  SidebarAccount,
  type SidebarAccountState,
} from "@/components/shell/SidebarAccount";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";

// The account at the top of the sidebar: who is signed in and what is left
// in the wallet, with top-up, refresh and settings behind it.

const mocks = vi.hoisted(() => ({
  openExternal: vi.fn().mockResolvedValue(undefined),
  openSettings: vi.fn(),
  apex: "ofox.io",
}));
vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: mocks.openExternal },
}));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: mocks.apex }),
}));

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

function account(over: Partial<OfoxAccount> = {}): OfoxAccount {
  return {
    user: {
      name: "Yun",
      email: "yun@example.com",
      avatar_url: null,
      balance: { balance: 923.8 },
      role: "owner",
    } as unknown as OfoxAccount["user"],
    retrying: false,
    retryAttempt: 0,
    loadFailed: false,
    balanceRefreshing: false,
    loadUser: vi.fn(),
    retryUser: vi.fn().mockResolvedValue(undefined),
    refreshBalance: vi.fn().mockResolvedValue(undefined),
    ...over,
  };
}

function show(state: SidebarAccountState, collapsed = false) {
  return render(
    <ShellProvider openSettings={mocks.openSettings}>
      <SidebarAccount state={state} collapsed={collapsed} />
    </ShellProvider>,
  );
}

describe("SidebarAccount", () => {
  it("signed: name and balance", () => {
    show({ kind: "signed", account: account() });
    expect(screen.getByText("Yun")).toBeVisible();
    expect(screen.getByText(/\$923\.80/)).toBeVisible();
  });

  it("balance ≤ 0 is shown in danger", () => {
    const a = account();
    (a.user as { balance: { balance: number } }).balance.balance = 0;
    show({ kind: "signed", account: a });
    expect(screen.getByText(/\$0\.00/).className).toContain("text-danger");
  });

  it("load failure offers a retry that calls retryUser", async () => {
    const a = account({ user: null, loadFailed: true });
    show({ kind: "signed", account: a });
    await userEvent.click(
      screen.getByRole("button", { name: en.console.balance.loadFailed }),
    );
    expect(a.retryUser).toHaveBeenCalled();
  });

  it("menu: top-up only for a billing manager and opens the wallet URL of the current apex", async () => {
    show({ kind: "signed", account: account() });
    await userEvent.click(screen.getByRole("button", { name: /Yun/ }));
    await userEvent.click(
      await screen.findByRole("menuitem", { name: en.console.balance.topUp }),
    );
    expect(mocks.openExternal).toHaveBeenCalledWith(
      expect.stringContaining("ofox.io"),
    );
  });

  it("menu: no top-up for a member who cannot pay", async () => {
    const a = account();
    (a.user as { role: string }).role = "member";
    show({ kind: "signed", account: a });
    await userEvent.click(screen.getByRole("button", { name: /Yun/ }));
    await screen.findByRole("menuitem", { name: en.shell.account.refresh });
    expect(
      screen.queryByRole("menuitem", { name: en.console.balance.topUp }),
    ).toBeNull();
  });

  it("menu: refresh balance and account settings", async () => {
    const a = account();
    show({ kind: "signed", account: a });
    await userEvent.click(screen.getByRole("button", { name: /Yun/ }));
    await userEvent.click(
      await screen.findByRole("menuitem", { name: en.shell.account.refresh }),
    );
    expect(a.refreshBalance).toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: /Yun/ }));
    await userEvent.click(
      await screen.findByRole("menuitem", { name: en.shell.account.settings }),
    );
    expect(mocks.openSettings).toHaveBeenCalled();
  });

  it("local signed out shows the sign-in button", async () => {
    const onAction = vi.fn();
    show({ kind: "local", signedIn: false, onAction });
    await userEvent.click(
      screen.getByRole("button", { name: en.toolCatalog.login }),
    );
    expect(onAction).toHaveBeenCalled();
  });

  it("local signed in shows bind tools", () => {
    show({ kind: "local", signedIn: true, onAction: () => undefined });
    expect(
      screen.getByRole("button", { name: en.toolCatalog.bindOptional }),
    ).toBeVisible();
  });

  it("collapsed shows only the avatar", () => {
    show({ kind: "signed", account: account() }, true);
    expect(screen.queryByText(/\$923\.80/)).toBeNull();
    expect(screen.getByRole("button", { name: /Yun/ })).toBeVisible();
  });
});
