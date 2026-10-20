import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { listen } from "@tauri-apps/api/event";
import { libraryApi } from "@/lib/api/library";
import { McpCheck } from "@/components/library/McpCheck";
import { DraftStore } from "@/components/library/useDraftState";
import { checkIssue } from "@/components/library/libraryIssues";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});
const server = {
  id: "s",
  revision: 2,
  name: "fixture",
  transport: "stdio",
  credentialState: "none",
  fields: [],
  targets: [],
};

it("does not guess credential paths or offer arbitrary fixes for unknown reasons", () => {
  expect(
    checkIssue({
      operationId: "o",
      state: "blocked",
      reason: "credentialMissing",
      tools: [],
    })?.action,
  ).toEqual({ kind: "edit", focusPath: [] });
  expect(
    checkIssue({
      operationId: "o",
      state: "error",
      reason: "futureBackendReason",
      tools: [],
    }),
  ).toBeUndefined();
  expect(
    checkIssue({
      operationId: "o",
      state: "ok",
      reason: "programMissing",
      tools: [],
    }),
  ).toBeUndefined();
});

it.each(["revision", "epoch"])(
  "invalidates repair actions when the checked %s changes",
  async (dimension) => {
    vi.mocked(listen).mockResolvedValue(vi.fn());
    vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
      operationId: "o",
    });
    vi.spyOn(libraryApi, "runMcpCheck").mockResolvedValue({
      operationId: "o",
      state: "blocked",
      reason: "programMissing",
      tools: [],
    });
    const store = new DraftStore();
    const action = vi.fn();
    const props = {
      store,
      server,
      epoch: 1,
      readOnly: false,
      onBusyChange: vi.fn(),
      onNextAction: action,
    };
    const view = render(<McpCheck {...props} />);
    await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
    expect(
      await screen.findByRole("button", { name: "修改命令" }),
    ).toBeEnabled();
    view.rerender(
      <McpCheck
        {...props}
        epoch={dimension === "epoch" ? 2 : 1}
        server={{ ...server, revision: dimension === "revision" ? 3 : 2 }}
      />,
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "修改命令" }),
      ).not.toBeInTheDocument(),
    );
    expect(action).not.toHaveBeenCalled();
  },
);

it("offers native instructions for a blocked package runner without writing or starting anything else", async () => {
  vi.mocked(listen).mockResolvedValue(vi.fn());
  vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
    operationId: "o",
  });
  const run = vi.spyOn(libraryApi, "runMcpCheck").mockResolvedValue({
    operationId: "o",
    state: "blocked",
    reason: "runnerBlocked",
    tools: [],
  });
  const save = vi.spyOn(libraryApi, "saveMcp");
  render(
    <McpCheck
      store={new DraftStore()}
      server={server}
      epoch={1}
      readOnly={false}
      onBusyChange={vi.fn()}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "查看原生操作步骤" }),
  );
  expect(screen.getByText(/此配置需要在对应工具中检查/)).toBeVisible();
  expect(screen.getByText(/重新打开目标工具或新建会话/)).toBeVisible();
  expect(save).not.toHaveBeenCalled();
  expect(run).toHaveBeenCalledTimes(1);
});
