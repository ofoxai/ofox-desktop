import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import * as tauri from "@tauri-apps/api/core";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import OnboardingApp from "@/OnboardingApp";
import { bindTools } from "@/lib/bindTools";
import { TOOL_ORDER } from "@/config/toolMeta";

vi.mock("@/lib/bindTools", () => ({ bindTools: vi.fn() }));
vi.mock("@/components/onboarding/LoginPage", () => ({ default: () => null }));
vi.mock("@/hooks/useToolInstall", () => ({
  useToolInstall: () => ({
    installing: new Set(),
    install: vi.fn(),
    error: null,
    progress: {},
  }),
}));
vi.mock("@/hooks/useToolInstallCapabilities", () => ({
  useToolInstallCapabilities: () => [],
}));

beforeAll(() => i18n.addResourceBundle("zh", "translation", zh, true, true));
beforeEach(() => {
  vi.mocked(bindTools).mockReset();
  vi.spyOn(tauri, "invoke").mockResolvedValue(
    TOOL_ORDER.map((name) => ({
      name,
      version: ["hermes", "dsh"].includes(name) ? "1.0" : null,
      installationStatus: ["hermes", "dsh"].includes(name)
        ? "installed"
        : "notInstalled",
      error: null,
    })),
  );
});
afterEach(() => vi.restoreAllMocks());

async function picker(onEnterLocal = vi.fn()) {
  render(
    <OnboardingApp
      initialStep="toolDiscovery"
      onComplete={vi.fn()}
      onEnterLocal={onEnterLocal}
    />,
  );
  const button = await screen.findByRole("button", {
    name: "开始绑定（2）→",
  });
  await waitFor(() => expect(button).toBeEnabled());
  return button;
}

describe("Onboarding binding outcomes", () => {
  it("keeps the picker after every binding fails and permits an explicit retry", async () => {
    vi.mocked(bindTools)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce(["hermes", "dsh"]);
    fireEvent.click(await picker());
    await waitFor(() => expect(bindTools).toHaveBeenCalledOnce());
    expect(screen.queryByText("全部设置完成")).not.toBeInTheDocument();
    const retry = screen.getByRole("button", { name: "开始绑定（2）→" });
    await waitFor(() => expect(retry).toBeEnabled());
    fireEvent.click(retry);
    expect(await screen.findByText("全部设置完成")).toBeVisible();
    expect(screen.getByText("已为 2 个工具绑定 Ofox。")).toBeVisible();
  });

  it("does not count retained or partial bindings as completion of this selection", async () => {
    vi.mocked(bindTools).mockResolvedValue(["pi", "hermes"]);
    const enterLocal = vi.fn();
    fireEvent.click(await picker(enterLocal));
    await waitFor(() => expect(bindTools).toHaveBeenCalledOnce());
    expect(screen.queryByText("全部设置完成")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "开始绑定（2）→" }),
    ).toBeVisible();
    fireEvent.click(
      screen.getByRole("button", { name: "暂不绑定，进入本机管理" }),
    );
    expect(enterLocal).toHaveBeenCalledOnce();
  });

  it("disables navigation and duplicate submissions while binding is pending", async () => {
    let finish!: (tools: string[]) => void;
    vi.mocked(bindTools).mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    fireEvent.click(await picker());
    expect(screen.getByRole("button", { name: "正在绑定…" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "暂不绑定，进入本机管理" }),
    ).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "正在绑定…" }));
    expect(bindTools).toHaveBeenCalledOnce();
    await act(async () => finish(["hermes", "dsh"]));
    expect(await screen.findByText("全部设置完成")).toBeVisible();
  });
});
