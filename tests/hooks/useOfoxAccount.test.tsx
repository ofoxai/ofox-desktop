import { StrictMode, useEffect, type ReactNode } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { queryClient } from "@/lib/query/queryClient";
import { OFOX_USER_KEY, useOfoxAccount } from "@/hooks/useOfoxAccount";

const mocks = vi.hoisted(() => ({
  getUserInfo: vi.fn(),
  toast: { success: vi.fn(), warning: vi.fn(), error: vi.fn() },
}));

vi.mock("@/lib/api/ofoxAuth", () => ({ ofoxGetUserInfo: mocks.getUserInfo }));
vi.mock("sonner", () => ({ toast: mocks.toast }));

const ADA = { name: "Ada", email: "ada@example.com" };

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle("en", "translation", en, true, true);
  queryClient.clear();
  mocks.getUserInfo.mockReset();
  Object.values(mocks.toast).forEach((mock) => mock.mockClear());
});

afterEach(async () => {
  vi.useRealTimers();
  await i18n.changeLanguage("zh");
});

describe("useOfoxAccount", () => {
  it("shares the loaded user with every consumer through the ofoxUser query", async () => {
    mocks.getUserInfo.mockResolvedValue(ADA);
    const tools = renderHook(() => useOfoxAccount());
    const usage = renderHook(() => useOfoxAccount());

    await act(() => tools.result.current.loadUser());

    await waitFor(() => expect(usage.result.current.user).toEqual(ADA));
    expect(queryClient.getQueryData(OFOX_USER_KEY)).toEqual(ADA);
  });

  it("keeps the last user when a reload fails", async () => {
    mocks.getUserInfo.mockResolvedValueOnce(ADA);
    const { result } = renderHook(() => useOfoxAccount());
    await act(() => result.current.loadUser());
    await waitFor(() => expect(result.current.user).toEqual(ADA));

    mocks.getUserInfo.mockRejectedValueOnce(new Error("offline"));
    let loaded: unknown = "unset";
    await act(async () => {
      loaded = await result.current.loadUser();
    });

    expect(loaded).toBeNull();
    expect(queryClient.getQueryData(OFOX_USER_KEY)).toEqual(ADA);
    expect(result.current.user).toEqual(ADA);
  });

  it("forgets the user once nothing shows it, so the next sign-in starts blank", async () => {
    mocks.getUserInfo.mockResolvedValue(ADA);
    const page = renderHook(() => useOfoxAccount());
    await act(() => page.result.current.loadUser());

    page.unmount();

    await waitFor(() =>
      expect(queryClient.getQueryData(OFOX_USER_KEY)).toBeUndefined(),
    );
  });

  it.each(["loadUser", "refreshBalance"] as const)(
    "drops a %s reply that lands after the console closed, so the next account never sees it",
    async (call) => {
      let reply!: (user: unknown) => void;
      mocks.getUserInfo.mockReturnValueOnce(
        new Promise((resolve) => (reply = resolve)),
      );
      const previous = renderHook(() => useOfoxAccount());
      const pending = previous.result.current[call]();

      previous.unmount();
      await waitFor(() =>
        expect(
          queryClient.getQueryCache().find({ queryKey: OFOX_USER_KEY }),
        ).toBeUndefined(),
      );
      await act(async () => {
        reply({ ...ADA, balance: { balance: 7 } });
        await pending;
      });

      expect(queryClient.getQueryData(OFOX_USER_KEY)).toBeUndefined();
      const next = renderHook(() => useOfoxAccount());
      expect(next.result.current.user).toBeNull();
    },
  );

  it("does not toast the previous account balance after unmount", async () => {
    let reply!: (user: unknown) => void;
    mocks.getUserInfo.mockReturnValueOnce(
      new Promise((resolve) => {
        reply = resolve;
      }),
    );
    const previous = renderHook(() => useOfoxAccount());
    const pending = previous.result.current.refreshBalance();
    previous.unmount();
    await act(async () => {
      reply({ ...ADA, balance: { balance: 9 } });
      await pending;
    });
    expect(mocks.toast.success).not.toHaveBeenCalled();
    expect(mocks.toast.warning).not.toHaveBeenCalled();
  });

  it("drops the abandoned effect request during StrictMode replay", async () => {
    let reply!: (user: unknown) => void;
    mocks.getUserInfo
      .mockReturnValueOnce(
        new Promise((resolve) => {
          reply = resolve;
        }),
      )
      .mockResolvedValue({ ...ADA, name: "Current" });
    renderHook(
      () => {
        const account = useOfoxAccount();
        useEffect(() => {
          void account.loadUser();
        }, [account.loadUser]);
        return account;
      },
      {
        wrapper: ({ children }: { children: ReactNode }) => (
          <StrictMode>{children}</StrictMode>
        ),
      },
    );
    await waitFor(() =>
      expect(queryClient.getQueryData(OFOX_USER_KEY)).toMatchObject({
        name: "Current",
      }),
    );
    await act(async () => {
      reply(ADA);
    });
    expect(queryClient.getQueryData(OFOX_USER_KEY)).toMatchObject({
      name: "Current",
    });
  });

  it("stops retrying once the console closed and stores nothing", async () => {
    vi.useFakeTimers();
    mocks.getUserInfo.mockResolvedValue(ADA);
    const previous = renderHook(() => useOfoxAccount());
    act(() => void previous.result.current.retryUser());

    previous.unmount();
    await act(() => vi.advanceTimersByTimeAsync(1000 + 3000 + 8000));

    expect(mocks.getUserInfo).not.toHaveBeenCalled();
    expect(queryClient.getQueryData(OFOX_USER_KEY)).toBeUndefined();
  });

  it("retries an empty user after 1s, 3s and 8s, then offers a manual retry", async () => {
    vi.useFakeTimers();
    mocks.getUserInfo.mockResolvedValue({ name: null, email: null });
    const { result } = renderHook(() => useOfoxAccount());

    act(() => void result.current.retryUser());
    expect(result.current.retrying).toBe(true);
    expect(result.current.retryAttempt).toBe(1);

    await act(() => vi.advanceTimersByTimeAsync(1000));
    expect(mocks.getUserInfo).toHaveBeenCalledTimes(1);
    expect(result.current.retryAttempt).toBe(2);
    await act(() => vi.advanceTimersByTimeAsync(3000));
    expect(mocks.getUserInfo).toHaveBeenCalledTimes(2);
    await act(() => vi.advanceTimersByTimeAsync(8000));
    expect(mocks.getUserInfo).toHaveBeenCalledTimes(3);

    expect(result.current.retrying).toBe(false);
    expect(result.current.loadFailed).toBe(true);
    expect(result.current.user).toBeNull();
  });

  it("stops retrying as soon as a real user comes back", async () => {
    vi.useFakeTimers();
    mocks.getUserInfo
      .mockResolvedValueOnce({ name: null, email: null })
      .mockResolvedValueOnce(ADA);
    const { result } = renderHook(() => useOfoxAccount());

    act(() => void result.current.retryUser());
    await act(() => vi.advanceTimersByTimeAsync(1000 + 3000));

    expect(result.current.user).toEqual(ADA);
    expect(result.current.retrying).toBe(false);
    expect(result.current.loadFailed).toBe(false);
    await act(() => vi.advanceTimersByTimeAsync(8000));
    expect(mocks.getUserInfo).toHaveBeenCalledTimes(2);
  });

  it("refreshes the balance on request and says what it is now", async () => {
    mocks.getUserInfo.mockResolvedValue({ ...ADA, balance: { balance: 3.5 } });
    const { result } = renderHook(() => useOfoxAccount());

    await act(() => result.current.refreshBalance());

    expect(result.current.user?.balance?.balance).toBe(3.5);
    expect(result.current.balanceRefreshing).toBe(false);
    expect(mocks.toast.success).toHaveBeenCalledWith("余额已更新：$3.50");
  });

  it("says the new balance in the app's language", async () => {
    await i18n.changeLanguage("en");
    mocks.getUserInfo.mockResolvedValue({ ...ADA, balance: { balance: 3.5 } });
    const { result } = renderHook(() => useOfoxAccount());

    await act(() => result.current.refreshBalance());

    expect(mocks.toast.success).toHaveBeenCalledWith("Balance updated: $3.50");
  });
});
