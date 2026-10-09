import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
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
import { http, HttpResponse } from "msw";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import AddToolsDialog from "@/components/console/AddToolsDialog";
import { bindTools } from "@/lib/bindTools";
import { TOOL_ORDER } from "@/config/toolMeta";
import { server } from "../msw/server";
import type { ToolInstallationInfo } from "@/lib/api/toolUpdates";
import * as tauri from "@tauri-apps/api/core";

vi.mock("@/lib/bindTools", () => ({ bindTools: vi.fn() }));
vi.mock("@/hooks/useToolInstall", () => ({
  useToolInstall: () => ({
    installing: new Set(),
    install: vi.fn(),
    error: null,
    progress: {},
  }),
}));
vi.mock("@/hooks/useToolInstallCapabilities", () => ({
  useToolInstallCapabilities: () => ["claude"],
}));

beforeAll(() => i18n.addResourceBundle("zh", "translation", zh, true, true));
beforeEach(() => vi.mocked(bindTools).mockReset());
afterEach(() => vi.restoreAllMocks());

function scan(overrides: Record<string, Partial<ToolInstallationInfo>>) {
  server.use(
    http.post("http://tauri.local/get_tool_versions", () =>
      HttpResponse.json(
        TOOL_ORDER.map((name) => ({
          name,
          version: null,
          error: null,
          installationStatus: "notInstalled",
          ...overrides[name],
        })),
      ),
    ),
  );
}

describe("AddToolsDialog binding lifecycle", () => {
  it("directs a retained missing binding to homepage management", async () => {
    scan({});
    const manage = vi.fn();
    const close = vi.fn();
    render(
      <AddToolsDialog
        open
        alreadyBound={["workbuddy"]}
        onAdded={vi.fn()}
        onOpenChange={close}
        onManageBound={manage}
      />,
    );
    expect(await screen.findByText("已绑定 · 未安装")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "在主页管理" }));
    expect(manage).toHaveBeenCalledWith("workbuddy");
    expect(close).toHaveBeenCalledWith(false);
    expect(bindTools).not.toHaveBeenCalled();
  });

  it("submits only newly selected tools and returns the merged retained list", async () => {
    scan({ claude: { installationStatus: "installed", version: "1.0" } });
    vi.mocked(bindTools).mockResolvedValue(["workbuddy", "claude"]);
    const added = vi.fn();
    render(
      <AddToolsDialog
        open
        alreadyBound={["workbuddy"]}
        onAdded={added}
        onOpenChange={vi.fn()}
      />,
    );
    const confirm = await screen.findByRole("button", { name: "确认（1）" });
    await waitFor(() => expect(confirm).toBeEnabled());
    fireEvent.click(confirm);
    await waitFor(() => expect(bindTools).toHaveBeenCalledWith(["claude"]));
    expect(added).toHaveBeenCalledWith(["workbuddy", "claude"]);
  });

  it("does not label an uncertain scan as uninstalled or offer installation", async () => {
    scan({
      codex: { installationStatus: "unknown", error: "shell unavailable" },
    });
    render(
      <AddToolsDialog
        open
        alreadyBound={[]}
        onAdded={vi.fn()}
        onOpenChange={vi.fn()}
      />,
    );
    await screen.findByText("检测失败");
    const card = screen
      .getByText("Codex")
      .closest("div.relative") as HTMLElement;
    expect(
      within(card).getByRole("button", { name: "重新检测" }),
    ).toBeVisible();
    expect(
      within(card).queryByRole("button", { name: "安装" }),
    ).not.toBeInTheDocument();
    expect(within(card).queryByText("未安装")).not.toBeInTheDocument();
  });

  it("uses upstream instructions when this host cannot install a missing CLI", async () => {
    scan({});
    render(
      <AddToolsDialog
        open
        alreadyBound={[]}
        onAdded={vi.fn()}
        onOpenChange={vi.fn()}
      />,
    );
    await screen.findByText("没有可添加的工具——所有已识别的工具都已绑定。");
    const card = screen
      .getByText("Codex")
      .closest("div.relative") as HTMLElement;
    expect(
      within(card).getByRole("button", { name: "安装说明" }),
    ).toBeVisible();
    expect(
      within(card).queryByRole("button", { name: "安装" }),
    ).not.toBeInTheDocument();
  });

  it("ignores a late scan from a previous opening instead of reselecting a cancelled tool", async () => {
    const current = TOOL_ORDER.map(
      (name): ToolInstallationInfo => ({
        name,
        version: ["claude", "codex"].includes(name) ? "1.0" : null,
        error: null,
        installationStatus: ["claude", "codex"].includes(name)
          ? "installed"
          : "notInstalled",
      }),
    );
    let finishFirst!: (tools: ToolInstallationInfo[]) => void;
    const firstScan = new Promise<ToolInstallationInfo[]>((resolve) => {
      finishFirst = resolve;
    });
    const invoke = vi
      .spyOn(tauri, "invoke")
      .mockReturnValueOnce(firstScan)
      .mockResolvedValueOnce(current);
    const props = {
      alreadyBound: [] as string[],
      onAdded: vi.fn(),
      onOpenChange: vi.fn(),
    };
    const view = render(<AddToolsDialog open {...props} />);
    await waitFor(() => expect(invoke).toHaveBeenCalledOnce());
    view.rerender(<AddToolsDialog open={false} {...props} />);
    view.rerender(<AddToolsDialog open {...props} />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "确认（2）" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByText("Codex").closest("button")!);
    await act(async () => finishFirst(current));
    expect(screen.getByRole("button", { name: "确认（1）" })).toBeEnabled();
    vi.mocked(bindTools).mockResolvedValue(["claude"]);
    fireEvent.click(screen.getByRole("button", { name: "确认（1）" }));
    await waitFor(() => expect(bindTools).toHaveBeenCalledWith(["claude"]));
  });
});
