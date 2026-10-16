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
import { InstructionOverview } from "@/components/library/InstructionOverview";
import { InstructionEditor } from "@/components/library/ResourceEditors";
import { McpCheck } from "@/components/library/McpCheck";
import { DraftStore } from "@/components/library/useDraftState";
import {
  libraryApi,
  type LibraryData,
  type McpCheckResult,
} from "@/lib/api/library";
import { listen } from "@tauri-apps/api/event";
import { LIBRARY_ADD_COPY } from "@/components/library/libraryAddCopy";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@/components/MarkdownEditor", () => ({
  default: ({
    value,
    onChange,
  }: {
    value: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label="Markdown"
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

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

const initial = {
  id: "rules",
  name: "Team rules",
  content: "# Team rules\n\nKeep tests meaningful.",
  revision: 2,
};

describe("compact instruction experience", () => {
  it("puts full paths and revisions in technical details while retaining the actual/desired distinction", async () => {
    const data: LibraryData = {
      epoch: 1,
      instructions: [initial, { ...initial, id: "other", name: "Other rules" }],
      targets: [
        {
          toolId: "codex",
          path: "/fixture/deep/config/AGENTS.md",
          targetGroupId: "physical",
          available: true,
          discoveryStatus: "installed",
          status: "pending",
          desiredResourceId: "rules",
          readableResourceId: "other",
          nativeState: "managed",
          userContentLines: 12,
          managedContentLines: 4,
          lastAppliedRevision: 1,
        },
      ],
      operations: [],
    };
    render(<InstructionOverview data={data} busy={false} onManage={vi.fn()} />);
    expect(screen.getByRole("img", { name: "Codex" })).toBeVisible();
    expect(
      screen.getByText("/fixture/deep/config/AGENTS.md"),
    ).not.toBeVisible();
    expect(screen.getByText(/实际可读：Other rules/)).toBeVisible();
    await userEvent.click(screen.getByText("技术详情"));
    expect(screen.getByText("/fixture/deep/config/AGENTS.md")).toBeVisible();
  });

  it("previews safe Markdown and restores the original draft on discard", async () => {
    const discard = vi.fn();
    render(
      <InstructionEditor
        store={new DraftStore()}
        initial={initial}
        busy={false}
        onSave={vi.fn()}
        onDiscard={discard}
      />,
    );
    fireEvent.change(screen.getByLabelText("Markdown"), {
      target: { value: "# Changed\n\n<script>alert('unsafe')</script>" },
    });
    await userEvent.click(screen.getByRole("radio", { name: "预览" }));
    expect(screen.getByRole("heading", { name: "Changed" })).toBeVisible();
    expect(document.querySelector("script")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "放弃修改" }));
    expect(discard).toHaveBeenCalledOnce();
    await userEvent.click(screen.getByRole("radio", { name: "编辑" }));
    expect(screen.getByLabelText("Markdown")).toHaveValue(initial.content);
  });

  it("saves with Ctrl/Cmd+S and blocks repeated submissions while saving", async () => {
    let finish!: () => void;
    const save = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    render(
      <InstructionEditor
        store={new DraftStore()}
        initial={initial}
        busy={false}
        onSave={save}
      />,
    );
    const content = screen.getByLabelText("Markdown");
    fireEvent.keyDown(content, { key: "s", ctrlKey: true });
    fireEvent.keyDown(content, { key: "s", metaKey: true });
    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith(initial, false);
    await act(async () => finish());
  });

  it("does not mark a rejected parent save as the baseline", async () => {
    const save = vi.fn().mockResolvedValue(false);
    render(
      <InstructionEditor
        store={new DraftStore()}
        initial={initial}
        busy={false}
        onSave={save}
      />,
    );
    fireEvent.change(screen.getByLabelText("Markdown"), {
      target: { value: "Unsaved local changes" },
    });
    await userEvent.click(screen.getByRole("button", { name: "保存草稿" }));
    expect(screen.getByRole("button", { name: "放弃修改" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "放弃修改" }));
    expect(screen.getByLabelText("Markdown")).toHaveValue(initial.content);
  });
});

describe("MCP check list result receipts", () => {
  const server = {
    id: "server",
    name: "Demo",
    revision: 2,
    transport: "http",
    credentialState: "none",
    fields: [],
    targets: [],
  };
  it("reports only an explicitly completed matching operation with a check timestamp", async () => {
    const onResult = vi.fn();
    vi.mocked(listen).mockResolvedValue(vi.fn());
    vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
      operationId: "check",
    });
    const result: McpCheckResult = {
      operationId: "check",
      state: "ok",
      tools: [{ name: "lookup" }],
    };
    vi.spyOn(libraryApi, "runMcpCheck").mockResolvedValue(result);
    render(
      <McpCheck
        store={new DraftStore()}
        server={server}
        epoch={3}
        readOnly={false}
        onBusyChange={vi.fn()}
        onResult={onResult}
      />,
    );
    expect(onResult).not.toHaveBeenCalled();
    expect(libraryApi.prepareMcpCheck).not.toHaveBeenCalled();
    const before = Date.now();
    await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
    await waitFor(() => expect(onResult).toHaveBeenCalledOnce());
    expect(onResult).toHaveBeenCalledWith(result, expect.any(Number));
    expect(onResult.mock.calls[0][1]).toBeGreaterThanOrEqual(before);
    expect(onResult.mock.calls[0][1]).toBeLessThanOrEqual(Date.now());
  });

  it("never reports mismatched or structurally invalid check results", async () => {
    vi.mocked(listen).mockResolvedValue(vi.fn());
    vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
      operationId: "check",
    });
    vi.spyOn(libraryApi, "cancelMcpCheck").mockResolvedValue(true);
    const run = vi.spyOn(libraryApi, "runMcpCheck");
    for (const result of [
      { operationId: "other", state: "ok", tools: [] },
      { operationId: "check", state: "invented", tools: [] },
      { operationId: "check", state: "ok", tools: [{ name: 42 }] },
    ]) {
      run.mockResolvedValue(result as unknown as McpCheckResult);
      const onResult = vi.fn();
      const view = render(
        <McpCheck
          store={new DraftStore()}
          server={server}
          epoch={3}
          readOnly={false}
          onBusyChange={vi.fn()}
          onResult={onResult}
        />,
      );
      await userEvent.click(
        within(view.container).getByRole("button", { name: "开始检查" }),
      );
      await waitFor(() =>
        expect(within(view.container).getByRole("alert")).toBeVisible(),
      );
      expect(onResult).not.toHaveBeenCalled();
      view.unmount();
    }
  });

  it("does not cache a check result after its resource revision changes", async () => {
    let finish!: (result: McpCheckResult) => void;
    const onResult = vi.fn();
    vi.mocked(listen).mockResolvedValue(vi.fn());
    vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
      operationId: "check",
    });
    vi.spyOn(libraryApi, "runMcpCheck").mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const store = new DraftStore();
    const view = render(
      <McpCheck
        store={store}
        server={server}
        epoch={3}
        readOnly={false}
        onBusyChange={vi.fn()}
        onResult={onResult}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
    await waitFor(() => expect(libraryApi.runMcpCheck).toHaveBeenCalled());
    view.rerender(
      <McpCheck
        store={store}
        server={{ ...server, revision: 3 }}
        epoch={3}
        readOnly={false}
        onBusyChange={vi.fn()}
        onResult={onResult}
      />,
    );
    await act(async () =>
      finish({
        operationId: "check",
        state: "ok",
        tools: [{ name: "stale-tool" }],
      }),
    );
    expect(onResult).not.toHaveBeenCalled();
    expect(screen.queryByText("stale-tool")).not.toBeInTheDocument();
  });
});
