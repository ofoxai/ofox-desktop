import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { ResourcePackDialog } from "@/components/library/ResourcePackDialog";
import { libraryApi } from "@/lib/api/library";

beforeEach(() => {
  vi.restoreAllMocks();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle(
    "zh",
    "translation",
    {
      library: {
        pack: {
          menu: "资源库更多操作",
          importAction: "导入资源包",
          exportAction: "导出资源包",
          importTitle: "导入资源包",
          exportTitle: "导出资源包",
          importDescription: "预览后加入我的资源，不会自动应用到工具。",
          exportDescription: "导出全部资源库内容。",
          cancelledImport: "未选择资源包",
          cancelledExport: "已取消导出",
        },
      },
    },
    true,
    true,
  );
});

describe("resource pack menu", () => {
  it("places both actions in a menu and goes straight to a frozen import preview", async () => {
    const picked = vi
      .spyOn(libraryApi, "pickPack")
      .mockResolvedValue("/sandbox/resources.zip");
    const preview = vi.spyOn(libraryApi, "previewPack").mockResolvedValue({
      fingerprint: "frozen",
      revision: 3,
      epoch: 8,
      items: [
        {
          kind: "mcp",
          id: "one",
          name: "Example",
          conflict: true,
          omittedFields: 1,
        },
      ],
    });
    const restore = vi.spyOn(libraryApi, "restorePack").mockResolvedValue({
      imported: 0,
      items: [{ kind: "mcp", name: "Example", status: "skipped" }],
    });
    const changed = vi.fn().mockResolvedValue(undefined);
    render(<ResourcePackDialog onChanged={changed} />);
    expect(
      screen.queryByRole("button", { name: "资源包导入／导出" }),
    ).toBeNull();
    const trigger = screen.getByRole("button", { name: "资源库更多操作" });
    await userEvent.click(trigger);
    expect(screen.getAllByRole("menuitem")).toHaveLength(2);
    await userEvent.click(screen.getByRole("menuitem", { name: "导入资源包" }));
    await screen.findByText("Example");
    expect(picked).toHaveBeenCalledTimes(1);
    expect(preview).toHaveBeenCalledTimes(1);
    expect(restore).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toContainElement(
      document.activeElement as HTMLElement,
    );
    await userEvent.click(screen.getByRole("button", { name: "确认恢复草稿" }));
    expect(restore).toHaveBeenCalledWith(
      "/sandbox/resources.zip",
      expect.objectContaining({ fingerprint: "frozen", revision: 3, epoch: 8 }),
      "skip",
    );
    expect(changed).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "关闭" }));
    await waitFor(() => expect(trigger).toHaveFocus());
  });
  it("exports directly after menu selection and restores focus from its result", async () => {
    const exported = vi.spyOn(libraryApi, "exportPack").mockResolvedValue(true);
    const picked = vi.spyOn(libraryApi, "pickPack");
    render(<ResourcePackDialog onChanged={vi.fn()} />);
    const trigger = screen.getByRole("button", { name: "资源库更多操作" });
    trigger.focus();
    await userEvent.keyboard("{Enter}");
    await userEvent.keyboard("{ArrowDown}{Enter}");
    await screen.findByText("资源包已保存");
    expect(exported).toHaveBeenCalledTimes(1);
    expect(picked).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toHaveAccessibleName("导出资源包");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(trigger).toHaveFocus());
  });
  it("keeps cancellation distinct from export failure and never calls an import", async () => {
    vi.spyOn(libraryApi, "exportPack").mockResolvedValue(false);
    render(<ResourcePackDialog onChanged={vi.fn()} />);
    await userEvent.click(
      screen.getByRole("button", { name: "资源库更多操作" }),
    );
    await userEvent.click(screen.getByRole("menuitem", { name: "导出资源包" }));
    expect(await screen.findByText("已取消导出")).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
  });
  it("reports cancelled import without calling preview or restoring resources", async () => {
    vi.spyOn(libraryApi, "pickPack").mockResolvedValue(null);
    const preview = vi.spyOn(libraryApi, "previewPack");
    const restore = vi.spyOn(libraryApi, "restorePack");
    render(<ResourcePackDialog onChanged={vi.fn()} />);
    await userEvent.click(
      screen.getByRole("button", { name: "资源库更多操作" }),
    );
    await userEvent.click(screen.getByRole("menuitem", { name: "导入资源包" }));
    expect(await screen.findByText("未选择资源包")).toBeVisible();
    expect(preview).not.toHaveBeenCalled();
    expect(restore).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
  });
  it("shows a translated export error, permits retry and never claims cancellation as failure", async () => {
    const exported = vi
      .spyOn(libraryApi, "exportPack")
      .mockRejectedValueOnce(
        new Error("library.packWriteFailed private-sentinel"),
      )
      .mockResolvedValueOnce(true);
    render(<ResourcePackDialog onChanged={vi.fn()} />);
    await userEvent.click(
      screen.getByRole("button", { name: "资源库更多操作" }),
    );
    await userEvent.click(screen.getByRole("menuitem", { name: "导出资源包" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "无法保存资源包",
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent("private-sentinel");
    await userEvent.click(screen.getByRole("button", { name: "导出资源包" }));
    expect(await screen.findByText("资源包已保存")).toBeVisible();
    expect(exported).toHaveBeenCalledTimes(2);
  });
  it("keeps one export running and blocks closing until it finishes", async () => {
    let finish!: (saved: boolean) => void;
    const exported = vi.spyOn(libraryApi, "exportPack").mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    render(<ResourcePackDialog onChanged={vi.fn()} />);
    await userEvent.click(
      screen.getByRole("button", { name: "资源库更多操作" }),
    );
    await userEvent.click(screen.getByRole("menuitem", { name: "导出资源包" }));
    expect(await screen.findByRole("button", { name: "关闭" })).toBeDisabled();
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(exported).toHaveBeenCalledTimes(1);
    await act(async () => finish(true));
    expect(screen.getByRole("button", { name: "关闭" })).toBeEnabled();
  });
});
