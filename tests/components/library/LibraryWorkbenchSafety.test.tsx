import { StrictMode, useState } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import {
  libraryApi,
  type LibraryInventory,
  type LibraryBatchPlan,
} from "@/lib/api/library";
import { LibraryInventoryView } from "@/components/library/LibraryInventoryView";
import { LibraryBatchReview } from "@/components/library/LibraryBatchReview";
import { LibraryResourceRow } from "@/components/library/LibraryResourceRow";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { LIBRARY_ADD_COPY } from "@/components/library/libraryAddCopy";

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle(
    "zh",
    "translation",
    { library: LIBRARY_ADD_COPY.zh },
    true,
    true,
  );
});
const inventory: LibraryInventory = {
  ticketId: "ticket",
  kind: "mcp",
  expiresAtMs: Date.now() + 60000,
  sources: [],
  entries: [
    {
      entryId: "entry",
      kind: "mcp",
      name: "Native item",
      sources: [],
      matchedResourceIds: [],
      conflict: false,
    },
  ],
};
const plan: LibraryBatchPlan = {
  operationId: "batch",
  kind: "mcp",
  status: "previewed",
  resources: [{ resourceId: "server" }],
  groups: [
    {
      groupId: "group",
      tools: ["codex"],
      readers: ["codex"],
      path: "/fixture/config.toml",
      status: "previewed",
      resourceIds: ["server"],
      beforeManaged: "before",
      afterManaged: "after",
    },
  ],
};

it("keeps one read-only inventory scan through StrictMode replay and releases its ticket on close", async () => {
  let finish!: (data: LibraryInventory) => void;
  const scan = vi.spyOn(libraryApi, "inventory").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const discard = vi
    .spyOn(libraryApi, "discardInventory")
    .mockResolvedValue(true);
  const busy = vi.fn();
  const view = render(
    <StrictMode>
      <LibraryInventoryView
        kind="mcp"
        epoch={1}
        onChanged={vi.fn()}
        onBusyChange={busy}
      />
    </StrictMode>,
  );
  expect(scan).toHaveBeenCalledTimes(1);
  await act(async () => finish(inventory));
  expect(await screen.findByText("Native item")).toBeVisible();
  view.unmount();
  expect(busy).toHaveBeenLastCalledWith(false);
  expect(discard).toHaveBeenCalledWith("ticket");
});

it("unlocks the parent immediately and discards an inventory ticket arriving after unmount", async () => {
  let finish!: (data: LibraryInventory) => void;
  vi.spyOn(libraryApi, "inventory").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const discard = vi
    .spyOn(libraryApi, "discardInventory")
    .mockResolvedValue(true);
  const busy = vi.fn();
  const view = render(
    <LibraryInventoryView
      kind="mcp"
      epoch={1}
      onChanged={vi.fn()}
      onBusyChange={busy}
    />,
  );
  view.unmount();
  expect(busy).toHaveBeenLastCalledWith(false);
  await act(async () => finish(inventory));
  expect(discard).toHaveBeenCalledWith("ticket");
});

it("deduplicates batch cancellation and refreshes the parent with its result", async () => {
  let finish!: (data: LibraryBatchPlan) => void;
  const cancel = vi.spyOn(libraryApi, "cancelBatch").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const changed = vi.fn().mockResolvedValue(undefined);
  render(
    <LibraryBatchReview
      kind="mcp"
      resources={[]}
      tools={[]}
      action="apply"
      epoch={1}
      initial={plan}
      onBusyChange={vi.fn()}
      onChanged={changed}
    />,
  );
  const stop = screen.getByRole("button", { name: "停止剩余项" });
  await userEvent.click(stop);
  await userEvent.click(stop);
  expect(cancel).toHaveBeenCalledTimes(1);
  const cancelled = { ...plan, status: "cancelled" };
  await act(async () => finish(cancelled));
  expect(changed).toHaveBeenCalledWith(cancelled);
});

it("returns focus to a stable batch result after the confirmation button disappears", async () => {
  vi.spyOn(libraryApi, "runBatch").mockResolvedValue({
    ...plan,
    status: "applied",
    groups: [{ ...plan.groups[0], status: "applied" }],
  });
  render(
    <LibraryBatchReview
      kind="mcp"
      resources={[]}
      tools={[]}
      action="apply"
      epoch={1}
      initial={plan}
      onBusyChange={vi.fn()}
      onChanged={vi.fn().mockResolvedValue(undefined)}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "确认应用" }));
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "确认应用" }),
    ).not.toBeInTheDocument(),
  );
  expect(screen.getByRole("status")).toHaveFocus();
});

it("preserves batch preparation under StrictMode and clears parent busy on unmount", async () => {
  let finish!: (data: LibraryBatchPlan) => void;
  const prepare = vi.spyOn(libraryApi, "previewBatch").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const busy = vi.fn();
  const view = render(
    <StrictMode>
      <LibraryBatchReview
        kind="mcp"
        resources={[]}
        tools={[]}
        action="apply"
        epoch={1}
        onBusyChange={busy}
        onChanged={vi.fn().mockResolvedValue(undefined)}
      />
    </StrictMode>,
  );
  expect(prepare).toHaveBeenCalledTimes(1);
  await act(async () => finish(plan));
  expect(await screen.findByText("codex", { exact: false })).toBeVisible();
  view.unmount();
  expect(busy).toHaveBeenLastCalledWith(false);
});

it("hands focus from More to the opened dialog without refocusing More", async () => {
  function Fixture() {
    const [open, setOpen] = useState(false);
    return (
      <>
        <LibraryResourceRow
          kind="instructions"
          row={{ id: "rules", name: "Shared", content: "Rules", revision: 1 }}
          targets={[]}
          busy={false}
          writable
          selecting={false}
          selected={false}
          onSelected={vi.fn()}
          onToggle={vi.fn()}
          onAction={() => setOpen(true)}
        />
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogContent aria-describedby={undefined}>
            <DialogTitle>Edit rules</DialogTitle>
            <Input aria-label="Editor field" autoFocus />
          </DialogContent>
        </Dialog>
      </>
    );
  }
  render(<Fixture />);
  const more = screen.getByRole("button", { name: "Shared 的更多操作" });
  await userEvent.click(more);
  const focus = vi.spyOn(more, "focus");
  await userEvent.click(screen.getByRole("menuitem", { name: "编辑 Shared" }));
  await screen.findByRole("dialog");
  expect(screen.getByLabelText("Editor field")).toHaveFocus();
  expect(focus).not.toHaveBeenCalled();
});

it("does not confuse applied configuration with an MCP connection result", () => {
  const base = {
    kind: "mcp" as const,
    row: {
      id: "server",
      name: "A very long MCP resource name",
      revision: 2,
      transport: "http",
      credentialState: "none",
      fields: [],
      targets: [],
    },
    targets: [],
    busy: false,
    writable: true,
    selecting: false,
    selected: false,
    onSelected: vi.fn(),
    onToggle: vi.fn(),
    onAction: vi.fn(),
  };
  const view = render(<LibraryResourceRow {...base} />);
  expect(screen.queryByText(/上次检查/)).not.toBeInTheDocument();
  view.rerender(
    <LibraryResourceRow
      {...base}
      check={{ state: "blocked", checkedAtMs: 1000, toolsCount: 0 }}
    />,
  );
  expect(screen.getByText(/上次检查：请在对应工具中检查/)).toBeVisible();
  expect(screen.queryByText(/连接成功/)).not.toBeInTheDocument();
});

it("requires a fresh scan after consuming an import ticket", async () => {
  vi.spyOn(libraryApi, "inventory").mockResolvedValue(inventory);
  vi.spyOn(libraryApi, "discardInventory").mockResolvedValue(true);
  const imported = vi.spyOn(libraryApi, "importInventory").mockResolvedValue({
    imported: 1,
    items: [{ entryId: "entry", name: "Native item", status: "imported" }],
  });
  render(
    <LibraryInventoryView
      kind="mcp"
      epoch={1}
      onChanged={vi.fn().mockResolvedValue(undefined)}
      onBusyChange={vi.fn()}
    />,
  );
  await screen.findByText("Native item");
  await userEvent.click(
    screen.getByRole("checkbox", { name: "选择 Native item" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "导入选中项（1）" }),
  );
  await waitFor(() =>
    expect(screen.queryByText("Native item")).not.toBeInTheDocument(),
  );
  expect(imported).toHaveBeenCalledTimes(1);
  expect(
    screen.getByRole("button", { name: "导入选中项（0）" }),
  ).toBeDisabled();
});
it("shows the names, intent and field paths of MCP changes before confirmation", () => {
  const preview = {
    ...plan,
    resources: [{ resourceId: "server", name: "Context7" }],
    groups: [
      {
        ...plan.groups[0],
        changes: [
          {
            resourceId: "server",
            name: "Context7",
            action: "apply" as const,
            changedFields: ["url", "headers.Authorization"],
          },
        ],
      },
    ],
  };
  render(
    <LibraryBatchReview
      kind="mcp"
      epoch={1}
      resources={[]}
      tools={[]}
      action="apply"
      initial={preview}
      onBusyChange={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  expect(screen.getByText("Context7")).toBeVisible();
  expect(screen.getByText("更新或应用")).toBeVisible();
  expect(screen.getByText(/headers.Authorization/)).toBeVisible();
});
it("shows the actual planned skill transfer method before confirmation", () => {
  const preview = {
    ...plan,
    kind: "skills" as const,
    resources: [{ resourceId: "server", name: "frontend-design" }],
    groups: [
      {
        ...plan.groups[0],
        changes: [
          {
            resourceId: "server",
            name: "frontend-design",
            action: "apply" as const,
            changedFields: [],
            method: "symlink",
          },
        ],
      },
    ],
  };
  render(
    <LibraryBatchReview
      kind="skills"
      epoch={1}
      resources={[]}
      tools={[]}
      action="apply"
      initial={preview}
      onBusyChange={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  expect(screen.getByText("frontend-design")).toBeVisible();
  expect(screen.getByText("符号链接")).toBeVisible();
});
it("reviews metadata-only withdrawals without inventing a physical write group", () => {
  const preview = {
    ...plan,
    groups: [],
    metadataChanges: [
      {
        resourceId: "server",
        name: "Pending resource",
        action: "withdraw" as const,
        tools: ["codex"],
        changedFields: [],
        method: null,
        status: "pending",
      },
    ],
  };
  render(
    <LibraryBatchReview
      kind="mcp"
      epoch={1}
      resources={[]}
      tools={[]}
      action="withdraw"
      initial={preview}
      onBusyChange={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
  expect(screen.getByText("Pending resource")).toBeVisible();
  expect(screen.getByText("仅调整分配记录，不改动文件")).toBeVisible();
  expect(screen.getByText("Codex")).toBeVisible();
  expect(screen.getByText("撤回分配")).toBeVisible();
  expect(screen.queryByText("/fixture/config.toml")).toBeNull();
});
