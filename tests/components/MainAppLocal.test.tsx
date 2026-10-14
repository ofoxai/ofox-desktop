import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import MainApp from "@/MainApp";
import { useState } from "react";

const mocks = vi.hoisted(() => ({
  status: vi.fn(),
  listeners: new Map<string, () => void>(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, callback: () => void) => {
    mocks.listeners.set(name, callback);
    return () => mocks.listeners.delete(name);
  }),
}));
vi.mock("@/lib/api/ofoxAuth", () => ({
  ofoxGetAuthStatus: mocks.status,
  OFOX_AUTH_EXPIRED_EVENT: "expired",
  OFOX_AUTH_RESTORED_EVENT: "restored",
}));
vi.mock("@/OnboardingApp", () => ({
  default: ({
    initialStep,
    onEnterLocal,
    onComplete,
  }: {
    initialStep: string;
    onEnterLocal?: () => void;
    onComplete?: () => void;
  }) => (
    <div>
      <span>{initialStep}</span>
      <button onClick={onEnterLocal}>本机管理</button>
      <button onClick={onComplete}>完成登录</button>
    </div>
  ),
}));
vi.mock("@/components/console/ConsolePage", () => ({
  default: ({
    localOnly,
    boundTools,
    onLoginRequested,
  }: {
    localOnly?: boolean;
    boundTools: string[];
    onLoginRequested?: () => void;
  }) => {
    const [page, setPage] = useState("工具已打开");
    return (
      <div>
        {localOnly ? "本机界面" : "账户界面"}
        <span data-testid="bound">{boundTools.join(",")}</span>
        <span>{page}</span>
        <button onClick={() => setPage("会话已打开")}>打开会话</button>
        <button onClick={onLoginRequested}>登录账户</button>
      </div>
    );
  },
}));

beforeEach(() => {
  localStorage.clear();
  mocks.listeners.clear();
  mocks.status.mockResolvedValue({ state: "loggedout" });
});

describe("MainApp local management", () => {
  it("requires authentication after a logout request even when bindings remain", async () => {
    mocks.status.mockResolvedValue({ state: "active" });
    localStorage.setItem("ofox-bound-tools", '["codex"]');
    render(<MainApp />);
    await screen.findByText("账户界面");
    await waitFor(() => expect(mocks.listeners.has("ofox-reauth-requested")).toBe(true));
    act(() => mocks.listeners.get("ofox-reauth-requested")?.());
    fireEvent.click(screen.getByRole("button", { name: "本机管理" }));
    fireEvent.click(screen.getByRole("button", { name: "登录账户" }));
    expect(screen.getByText("login")).toBeVisible();
    expect(localStorage.getItem("ofox-bound-tools")).toBe('["codex"]');
  });
  it("keeps the current local page through an explicit login", async () => {
    render(<MainApp />);
    fireEvent.click(await screen.findByRole("button", { name: "本机管理" }));
    fireEvent.click(screen.getByRole("button", { name: "打开会话" }));
    fireEvent.click(screen.getByRole("button", { name: "登录账户" }));
    fireEvent.click(screen.getByRole("button", { name: "完成登录" }));
    expect(await screen.findByText("会话已打开")).toBeVisible();
  });
  it.each(["loggedout", "expired"])(
    "allows %s users to enter without binding or clearing their stored bindings",
    async (state) => {
      mocks.status.mockResolvedValue({ state });
      localStorage.setItem("ofox-bound-tools", '["codex"]');
      render(<MainApp />);
      fireEvent.click(await screen.findByRole("button", { name: "本机管理" }));
      expect(await screen.findByText("本机界面")).toBeVisible();
      expect(localStorage.getItem("ofox-bound-tools")).toBe('["codex"]');
    },
  );

  it("allows an authenticated user with no bindings to skip discovery", async () => {
    mocks.status.mockResolvedValue({ state: "active" });
    render(<MainApp />);
    expect(await screen.findByText("toolDiscovery")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "本机管理" }));
    expect(await screen.findByText("本机界面")).toBeVisible();
    expect(localStorage.getItem("ofox-bound-tools")).toBeNull();
  });

  it("keeps the local page when authentication is restored or expires", async () => {
    render(<MainApp />);
    fireEvent.click(await screen.findByRole("button", { name: "本机管理" }));
    await waitFor(() => expect(mocks.listeners.has("restored")).toBe(true));
    act(() => mocks.listeners.get("restored")?.());
    expect(screen.getByText("本机界面")).toBeVisible();
    act(() => mocks.listeners.get("expired")?.());
    expect(screen.getByText("本机界面")).toBeVisible();
  });

  it("does not let the initial auth probe replace a newer local choice", async () => {
    let resolve!: (value: { state: string }) => void;
    mocks.status.mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );
    render(<MainApp />);
    await waitFor(() => expect(mocks.listeners.has("expired")).toBe(true));
    act(() => mocks.listeners.get("expired")?.());
    fireEvent.click(await screen.findByRole("button", { name: "本机管理" }));
    await act(async () => resolve({ state: "active" }));
    expect(screen.getByText("本机界面")).toBeVisible();
  });
});
