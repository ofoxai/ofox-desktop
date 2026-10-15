import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
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

describe("independent tool installation catalog", () => {
  it("lists a verified OFox copy among installed tools while preserving external-copy evidence", async () => {
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
    await userEvent.click(
      screen.getByRole("button", { name: /其他已安装工具/ }),
    );
    await waitFor(() =>
      expect(screen.getByText(/OFox 管理副本.*1.0.98/)).toBeVisible(),
    );
    expect(screen.getByText(/外部副本.*external-1/)).toBeVisible();
    expect(screen.getByText("/external/copilot")).toBeVisible();
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
    await userEvent.click(
      screen.getByRole("button", { name: /其他已安装工具/ }),
    );
    await waitFor(() =>
      expect(screen.getByText("GitHub Copilot CLI")).toBeVisible(),
    );
    expect(screen.getByText(/OFox 管理副本.*1.0.98/)).toBeVisible();
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
    fireEvent.click(screen.getByRole("button", { name: /其他已安装工具/ }));
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
    fireEvent.click(screen.getByRole("button", { name: /其他已安装工具/ }));
    expect(await screen.findByText("Crush")).toBeVisible();
    expect(screen.getByText(/版本检测未完成/)).toBeVisible();
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
    fireEvent.click(screen.getByRole("button", { name: /其他已安装工具/ }));
    expect(screen.queryByText("暂无其他已安装工具")).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("加载");
  });
  it("starts both areas collapsed, even on an empty machine", async () => {
    mocks.inventory.mockResolvedValue([]);
    render(<OptionalToolsCatalog boundTools={[]} />);
    const folds = screen.getAllByRole("button", { expanded: false });
    expect(folds).toHaveLength(2);
    expect(screen.queryByText("GitHub Copilot CLI")).toBeNull();
  });

  it("keeps installed tools out of the candidates and bound tools out of the extra inventory", async () => {
    render(<OptionalToolsCatalog boundTools={["codex"]} />);
    await waitFor(() => expect(mocks.inventory).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: /其他已安装工具/ }));
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
