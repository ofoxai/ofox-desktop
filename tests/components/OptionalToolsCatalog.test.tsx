import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { OptionalToolsCatalog } from "@/components/tools/OptionalToolsCatalog";
import { INSTALL_CATALOG } from "@/config/installCatalog";
import { createRef } from "react";
import type { InstallCatalogHandle } from "@/components/tools/OptionalToolsCatalog";

const mocks = vi.hoisted(() => ({
  inventory: vi.fn(),
  open: vi.fn(),
  copy: vi.fn(),
  changed: vi.fn(),
}));
vi.mock("@/lib/api/installCatalog", () => ({
  getInstallInventory: mocks.inventory,
}));
vi.mock("@/lib/api", () => ({ settingsApi: { openExternal: mocks.open } }));
vi.mock("@/lib/clipboard", () => ({ copyText: mocks.copy }));
vi.mock("@/components/tools/CatalogLifecycleDialog", () => ({
  CatalogLifecycleDialog: ({
    onChanged,
  }: {
    onChanged: () => Promise<void>;
  }) => {
    mocks.changed(onChanged);
    return null;
  },
}));

beforeEach(() => {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Macintosh");
  vi.spyOn(navigator, "platform", "get").mockReturnValue("MacIntel");
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  mocks.inventory.mockResolvedValue([
    {
      toolId: "copilot",
      status: "installed",
      version: "1.0",
      path: "/fixture/copilot",
    },
    { toolId: "codex", status: "installed", version: "2.0" },
    { toolId: "crush", status: "notInstalled" },
    { toolId: "cline", status: "unknown" },
  ]);
  mocks.open.mockResolvedValue(undefined);
  mocks.copy.mockResolvedValue(true);
  localStorage.setItem("ofox-bound-tools", '["codex"]');
});

afterEach(() => vi.restoreAllMocks());

describe("independent tool installation catalog", () => {
  it.each([
    "Claude Code",
    "Codex",
    "OpenCode",
    "OpenClaw",
    "Hermes",
    "Gemini",
    "DeepSeek Harness",
    "Pi",
  ])("offers a manual command for the unbound core CLI %s", async (name) => {
    mocks.inventory.mockResolvedValue([]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    await userEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    expect(
      within(screen.getByRole("group", { name })).getByRole("button", {
        name: `复制 ${name} 安装命令`,
      }),
    ).toBeVisible();
    expect(mocks.copy).not.toHaveBeenCalled();
  });

  it("copies Pi's current official npm command and shows its Node requirement", async () => {
    mocks.inventory.mockResolvedValue([]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    await userEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    const row = within(screen.getByRole("group", { name: "Pi" }));
    expect(row.getByText(/Node\.js 22\.19/)).toBeVisible();
    await userEvent.click(
      row.getByRole("button", { name: "复制 Pi 安装命令" }),
    );
    expect(mocks.copy).toHaveBeenCalledWith(
      "npm install -g --ignore-scripts @earendil-works/pi-coding-agent",
    );
    expect(localStorage.getItem("ofox-bound-tools")).toBe('["codex"]');
  });

  it.each([
    ["Macintosh", "MacIntel", "brew install block-goose-cli"],
    [
      "X11; Linux x86_64",
      "Linux x86_64",
      "curl -fsSL https://github.com/aaif-goose/goose/releases/download/stable/download_cli.sh | bash",
    ],
    [
      "Windows NT 10.0; Win64; x64",
      "Win32",
      'Invoke-WebRequest -Uri "https://raw.githubusercontent.com/aaif-goose/goose/main/download_cli.ps1" -OutFile "download_cli.ps1"\n.\\download_cli.ps1',
    ],
  ])("copies the Goose CLI command for %s", async (ua, platform, command) => {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue(ua);
    vi.spyOn(navigator, "platform", "get").mockReturnValue(platform);
    mocks.inventory.mockResolvedValue([]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    await userEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    const row = within(screen.getByRole("group", { name: "Goose CLI" }));
    await userEvent.click(
      row.getByRole("button", { name: "复制 Goose CLI 安装命令" }),
    );
    expect(mocks.copy).toHaveBeenCalledWith(command);
    expect(row.queryByText("brew install goose")).not.toBeInTheDocument();
  });

  it("provides guidance instead of a foreign OS command when the host is unknown", async () => {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Unknown host");
    vi.spyOn(navigator, "platform", "get").mockReturnValue("");
    mocks.inventory.mockResolvedValue([]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    await userEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    const row = within(screen.getByRole("group", { name: "Goose CLI" }));
    expect(row.getByText("请按官方说明选择当前系统的安装方式")).toBeVisible();
    expect(row.queryByRole("button", { name: /复制/ })).toBeNull();
    expect(
      row.getByRole("button", { name: "查看 Goose CLI 官方说明" }),
    ).toBeEnabled();
  });

  it.each([
    ["Claude Code", "irm https://claude.ai/install.ps1 | iex"],
    ["OpenClaw", "iwr -useb https://openclaw.ai/install.ps1 | iex"],
    ["Hermes", "iex (irm https://hermes-agent.nousresearch.com/install.ps1)"],
  ])("uses the native Windows installer for %s", async (name, command) => {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Windows NT 10.0");
    vi.spyOn(navigator, "platform", "get").mockReturnValue("Win32");
    mocks.inventory.mockResolvedValue([]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    await userEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    await userEvent.click(
      within(screen.getByRole("group", { name })).getByRole("button", {
        name: `复制 ${name} 安装命令`,
      }),
    );
    expect(mocks.copy).toHaveBeenCalledWith(command);
  });

  it.each(["ChatGPT", "WorkBuddy"])(
    "explains the download-only installation for %s",
    async (name) => {
      mocks.inventory.mockResolvedValue([]);
      render(<OptionalToolsCatalog boundTools={[]} />);
      await userEvent.click(
        screen.getByRole("button", { name: /安装其他工具/ }),
      );
      const row = within(screen.getByRole("group", { name }));
      expect(row.getByText("请前往官方页面下载安装")).toBeVisible();
      expect(row.queryByRole("button", { name: /复制/ })).toBeNull();
    },
  );

  it("keeps the management label concise while naming its tool for assistive technology", async () => {
    render(<OptionalToolsCatalog boundTools={[]} />);
    const manage = await screen.findByRole("button", {
      name: "管理 GitHub Copilot CLI 安装",
    });
    expect(manage).toHaveTextContent(/^安装管理$/);
    expect(manage).toHaveAttribute(
      "title",
      zh.catalogLifecycle.installationScope,
    );
    await userEvent.click(manage);
    expect(screen.getByRole("dialog")).toHaveAccessibleName(
      "GitHub Copilot CLI · Ofox 管理副本",
    );
  });
  it("keeps installation details on one faint line, the paths in its title", async () => {
    mocks.inventory.mockResolvedValueOnce([
      {
        toolId: "copilot",
        status: "installed",
        version: "external-1",
        path: "/external/very-long/copilot",
        managedCopy: {
          state: "managed",
          version: "1.0.98",
          path: "/private/very-long/prefix/bin/copilot",
        },
      },
    ]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    const row = await screen.findByRole("group", {
      name: "GitHub Copilot CLI",
    });
    const sub = row.querySelector("[data-line=sub]") as HTMLElement;
    expect(sub).toHaveClass("truncate");
    expect(sub).toHaveTextContent(/Ofox 管理副本.*1.0.98/);
    expect(sub).toHaveTextContent(/外部副本.*external-1/);
    expect(sub.title).toContain("/external/very-long/copilot");
    expect(sub.title).toContain("/private/very-long/prefix/bin/copilot");
    expect(row.querySelector("details")).toBeNull();
    expect(localStorage.getItem("ofox-bound-tools")).toBe('["codex"]');
  });
  it("lists a verified Ofox copy among installed tools while preserving external-copy evidence", async () => {
    mocks.inventory.mockResolvedValueOnce([
      {
        toolId: "copilot",
        status: "installed",
        version: "external-1",
        path: "/external/copilot",
        managedCopy: {
          state: "managed",
          version: "1.0.98",
          path: "/private/prefix/bin/copilot",
        },
      },
    ]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    await waitFor(() =>
      expect(screen.getByText(/Ofox 管理副本.*1.0.98/)).toBeVisible(),
    );
    expect(screen.getByText(/外部副本.*external-1/).title).toContain(
      "/external/copilot",
    );
  });
  it("does not hide a private managed copy when no shell executable exists", async () => {
    mocks.inventory.mockResolvedValueOnce([
      {
        toolId: "copilot",
        status: "notInstalled",
        managedCopy: {
          state: "managed",
          version: "1.0.98",
          path: "/private/prefix/bin/copilot",
        },
      },
    ]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    await waitFor(() =>
      expect(screen.getByText("GitHub Copilot CLI")).toBeVisible(),
    );
    expect(screen.getByText(/Ofox 管理副本.*1.0.98/)).toBeVisible();
    expect(screen.queryByText("未安装")).not.toBeInTheDocument();
  });
  it("does not start a queued mutation refresh after the catalog has unmounted", async () => {
    let finish!: (rows: unknown[]) => void;
    mocks.inventory.mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    const rendered = render(<OptionalToolsCatalog boundTools={[]} />);
    fireEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    await userEvent.click(
      screen.getByRole("button", { name: "管理 GitHub Copilot CLI 安装" }),
    );
    const afterChange = mocks.changed.mock.calls.at(
      -1,
    )![0] as () => Promise<void>;
    let completion!: Promise<void>;
    act(() => {
      completion = afterChange().catch(() => undefined);
    });
    rendered.unmount();
    await act(async () => finish([]));
    await act(async () => completion);
    expect(mocks.inventory).toHaveBeenCalledTimes(1);
  });

  it("reprobes after a mutation instead of accepting an inventory request started before it", async () => {
    let finish!: (rows: unknown[]) => void;
    mocks.inventory
      .mockReturnValueOnce(
        new Promise((resolve) => {
          finish = resolve;
        }),
      )
      .mockResolvedValueOnce([
        { toolId: "copilot", status: "installed", version: "new" },
      ]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    fireEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    await userEvent.click(
      screen.getByRole("button", { name: "管理 GitHub Copilot CLI 安装" }),
    );
    const afterChange = mocks.changed.mock.calls.at(
      -1,
    )![0] as () => Promise<void>;
    let completion!: Promise<void>;
    act(() => {
      completion = afterChange();
    });
    await act(async () =>
      finish([{ toolId: "copilot", status: "installed", version: "old" }]),
    );
    await waitFor(() => expect(mocks.inventory).toHaveBeenCalledTimes(2));
    await act(async () => completion);
  });

  it("offers installation-only management for Copilot without changing bindings", async () => {
    render(<OptionalToolsCatalog boundTools={["codex"]} />);
    await screen.findByText("GitHub Copilot CLI");
    expect(
      screen.getByRole("button", { name: "管理 GitHub Copilot CLI 安装" }),
    ).toBeEnabled();
    expect(localStorage.getItem("ofox-bound-tools")).toBe('["codex"]');
  });

  it("awaits and coalesces concurrent inventory refreshes", async () => {
    let resolve!: (value: unknown[]) => void;
    mocks.inventory.mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );
    const ref = createRef<InstallCatalogHandle>();
    render(<OptionalToolsCatalog ref={ref} boundTools={[]} />);
    let first!: Promise<void>;
    let second!: Promise<void>;
    act(() => {
      first = ref.current!.refresh();
      second = ref.current!.refresh();
    });
    expect(mocks.inventory).toHaveBeenCalledOnce();
    let finished = false;
    void first.then(() => {
      finished = true;
    });
    await act(async () => undefined);
    expect(finished).toBe(false);
    await act(async () => {
      resolve([]);
      await Promise.all([first, second]);
    });
    expect(finished).toBe(true);
  });

  it("retains installation evidence but surfaces a failed version probe without raw diagnostics", async () => {
    mocks.inventory.mockResolvedValue([
      {
        toolId: "crush",
        status: "installed",
        path: "/fixture/crush",
        error: "secret-fixture-value",
      },
    ]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    expect(await screen.findByText("Crush")).toBeVisible();
    expect(
      screen.getByText("版本检测未完成").closest("[data-line=name]"),
    ).not.toBeNull();
    expect(screen.queryByText(/secret-fixture-value/)).toBeNull();
    expect(
      screen.getByRole("button", { name: /重新检测 Crush/ }),
    ).toBeEnabled();
  });
  it("names each copy action using its actual product", async () => {
    render(<OptionalToolsCatalog boundTools={[]} />);
    fireEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    expect(
      await screen.findByRole("button", { name: "复制 Crush 安装命令" }),
    ).toBeVisible();
  });

  it("does not claim an empty installation list while detection is pending", async () => {
    mocks.inventory.mockReturnValue(new Promise(() => undefined));
    render(<OptionalToolsCatalog boundTools={[]} />);
    expect(screen.queryByText("暂无其他已安装工具")).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("加载");
  });
  it("opens the installed tools and keeps the install catalog folded", async () => {
    render(<OptionalToolsCatalog boundTools={[]} />);
    expect(
      screen.getByRole("button", { name: /其他已安装工具/ }),
    ).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getByRole("button", { name: /安装其他工具/ }),
    ).toHaveAttribute("aria-expanded", "false");
    expect(await screen.findByText("GitHub Copilot CLI")).toBeVisible();
  });

  it("keeps installed tools out of the candidates and bound tools out of the extra inventory", async () => {
    render(<OptionalToolsCatalog boundTools={["codex"]} />);
    await waitFor(() => expect(mocks.inventory).toHaveBeenCalled());
    expect(await screen.findByText("GitHub Copilot CLI")).toBeVisible();
    expect(screen.queryByText("Codex")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    expect(screen.getAllByText("GitHub Copilot CLI")).toHaveLength(1);
    expect(screen.getByText("Crush")).toBeVisible();
    expect(
      screen.getByText("Cline CLI").closest("div[data-tool-id]"),
    ).toHaveTextContent("待检测");
  });

  it("copies a manual command without changing binding preferences", async () => {
    render(<OptionalToolsCatalog boundTools={["codex"]} />);
    fireEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    const row = await screen.findByText("Crush");
    await userEvent.click(
      within(row.closest("div[data-tool-id]")!).getByRole("button", {
        name: /复制/,
      }),
    );
    expect(mocks.copy).toHaveBeenCalledWith("npm install -g @charmland/crush");
    expect(localStorage.getItem("ofox-bound-tools")).toBe('["codex"]');
  });

  it("retains the manual directory when an older backend cannot detect it", async () => {
    mocks.inventory.mockRejectedValue(new Error("unknown command"));
    render(<OptionalToolsCatalog boundTools={[]} />);
    fireEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("检测");
    expect(screen.getByText("Crush")).toBeVisible();
  });

  it("lays candidate rows on the list's shared columns, each command on one line", async () => {
    mocks.inventory.mockResolvedValue([]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    await userEvent.click(screen.getByRole("button", { name: /安装其他工具/ }));
    const row = screen.getByRole("group", { name: "Pi" });
    expect(row).toHaveClass("col-span-full", "grid-cols-subgrid");
    expect(
      [...row.children].map((cell) => cell.getAttribute("data-column")),
    ).toEqual(["badge", "name", "command", "actions"]);
    expect(
      within(row).getByText(/npm install -g --ignore-scripts/),
    ).toHaveClass("whitespace-nowrap");
    expect(row.querySelector("[data-line=sub]")).toHaveTextContent(
      /Node\.js 22\.19/,
    );
  });

  it("lets an installed row's name take the command column it does not need", async () => {
    render(<OptionalToolsCatalog boundTools={[]} />);
    const row = await screen.findByRole("group", {
      name: "GitHub Copilot CLI",
    });
    const cells = [...row.children];
    expect(cells.map((cell) => cell.getAttribute("data-column"))).toEqual([
      "badge",
      "name",
      "actions",
    ]);
    expect(cells[1]).toHaveClass("col-span-2");
    expect(row.querySelector("[data-line=sub]")).toHaveTextContent(
      "已安装 · 1.0",
    );
  });

  it("catalog identities do not imply model binding or automatic installation", () => {
    expect(INSTALL_CATALOG.find((r) => r.toolId === "omp")?.command).toContain(
      "bun install",
    );
    expect(
      INSTALL_CATALOG.find((r) => r.toolId === "aider")?.command,
    ).toContain("aider-install");
    expect(
      INSTALL_CATALOG.find((r) => r.toolId === "deveco-cli")?.platformVerified,
    ).toBe(false);
    expect(
      INSTALL_CATALOG.find((r) => r.toolId === "goose-cli")?.officialUrl,
    ).toContain("aaif-goose");
  });
});
