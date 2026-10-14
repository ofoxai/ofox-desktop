import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import {
  ModelProfilesPanel,
  captureProfileTargets,
} from "@/components/model-profiles/ModelProfilesPanel";
import { modelProfilesApi, type ModelProfile } from "@/lib/api/modelProfiles";
import {
  modelSelectionApi,
  type ToolModelSnapshot,
} from "@/lib/api/modelSelection";
import { queryClient } from "@/lib/query/queryClient";
import * as modelFetch from "@/lib/api/model-fetch";
import * as apexApi from "@/lib/api/ofoxApex";
import { emitTauriEvent } from "../msw/tauriMocks";

function snapshot(toolId: string, model = "old/model"): ToolModelSnapshot {
  return {
    toolId,
    writeTarget: toolId === "chatgpt" ? "codex" : toolId,
    revision: "disk-1",
    binding: {
      status: "configured",
      message: null,
      missingFiles: [],
      modifiedFields: [],
      envOverrides: [],
    },
    managedSelection: { kind: "single", model },
    observedSelection: { model, source: "config" },
    supportsDefault: toolId !== "dsh" && toolId !== "pi",
    scope: [],
  };
}
const profile: ModelProfile = {
  id: "profile-1",
  name: "Daily",
  revision: "profile-revision-1",
  match: "different",
  targets: [
    { target: "codex", selection: { kind: "single", model: "new/model" } },
    { target: "claude", selection: { kind: "single", model: "claude/new" } },
  ],
};

beforeEach(() => {
  queryClient.clear();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  HTMLElement.prototype.scrollIntoView = vi.fn();
  vi.spyOn(modelProfilesApi, "list").mockResolvedValue([profile]);
  vi.spyOn(modelSelectionApi, "snapshot").mockResolvedValue({
    tools: [snapshot("codex"), snapshot("chatgpt"), snapshot("claude")],
  });
  vi.spyOn(modelProfilesApi, "save").mockResolvedValue(profile);
  vi.spyOn(modelProfilesApi, "delete").mockResolvedValue();
  vi.spyOn(apexApi, "ofoxGetApexState").mockResolvedValue({
    apex: "ofox.io",
    pinned: true,
    detected: true,
  });
  vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue([]);
});
afterEach(() => vi.restoreAllMocks());

describe("ModelProfilesPanel", () => {
  it("captures canonical write targets, preserves ordered sets and omits mixed dsh scopes", () => {
    const multi = snapshot("workbuddy");
    multi.managedSelection = { kind: "models", models: ["b", "a"] };
    multi.supportsDefault = false;
    const modified = snapshot("gemini");
    modified.binding.status = "modified";
    expect(
      captureProfileTargets([
        snapshot("codex"),
        snapshot("chatgpt"),
        multi,
        snapshot("dsh", ""),
        modified,
      ]),
    ).toEqual([
      { target: "codex", selection: { kind: "single", model: "old/model" } },
      {
        target: "workbuddy",
        selection: { kind: "models", models: ["b", "a"] },
      },
    ]);
  });

  it("saves the current combination once for Codex and ChatGPT without credentials or machine paths", async () => {
    render(<ModelProfilesPanel boundTools={["codex", "chatgpt", "claude"]} />);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "保存当前组合" }),
      ).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "保存当前组合" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(
      within(dialog).getByRole("textbox", { name: "方案名称" }),
      { target: { value: "Current" } },
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(modelProfilesApi.save).toHaveBeenCalledWith({
        id: undefined,
        name: "Current",
        targets: [
          {
            target: "codex",
            selection: { kind: "single", model: "old/model" },
          },
          {
            target: "claude",
            selection: { kind: "single", model: "old/model" },
          },
        ],
        expectedRevision: undefined,
      }),
    );
  });

  it("prevents an empty recipe and retains profile revision when editing", async () => {
    render(<ModelProfilesPanel boundTools={["codex"]} />);
    await screen.findByText("Daily");
    fireEvent.click(screen.getByRole("button", { name: "编辑 Daily" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(
      within(dialog).getByRole("textbox", { name: "方案名称" }),
      { target: { value: "Revised" } },
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(modelProfilesApi.save).toHaveBeenCalledWith({
        id: profile.id,
        name: "Revised",
        targets: profile.targets,
        expectedRevision: profile.revision,
      }),
    );
  });

  it("blocks applying an invalid preflight and displays the specific conflict", async () => {
    vi.spyOn(modelProfilesApi, "preview").mockResolvedValue({
      planToken: "blocked-plan",
      canApply: false,
      items: [
        {
          target: "codex",
          before: { kind: "single", model: "old/model" },
          after: { kind: "single", model: "new/model" },
          status: "blocked",
          reason: "User changed the provider",
        },
      ],
    });
    const apply = vi.spyOn(modelProfilesApi, "apply");
    render(<ModelProfilesPanel boundTools={["codex"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "预览应用" }));
    expect(await screen.findByText("User changed the provider")).toBeVisible();
    expect(screen.getByRole("button", { name: "应用方案" })).toBeDisabled();
    expect(apply).not.toHaveBeenCalled();
  });
  it("localizes native preflight reason keys and displays dsh's affected configuration scopes", async () => {
    vi.spyOn(modelProfilesApi, "preview").mockResolvedValue({
      planToken: "plan-scope",
      canApply: false,
      items: [
        {
          target: "dsh",
          before: { kind: "single", model: "" },
          after: { kind: "single", model: "new/model" },
          status: "blocked",
          reason: "modelProfiles.reasonBinding",
          scope: ["home", "work"],
        },
      ],
    });
    render(<ModelProfilesPanel boundTools={["dsh"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "预览应用" }));
    expect(
      await screen.findByText("配置不可写，请先在工具页解决配置问题。"),
    ).toBeVisible();
    expect(screen.getByText("受影响的配置：home, work")).toBeVisible();
    expect(screen.queryByText("modelProfiles.reasonBinding")).toBeNull();
  });

  it("applies the preview token, presents partial results, and undoes with the returned token", async () => {
    vi.spyOn(modelProfilesApi, "preview").mockResolvedValue({
      planToken: "plan-1",
      canApply: true,
      items: profile.targets.map((item) => ({
        target: item.target,
        before: { kind: "single" as const, model: "old/model" },
        after: item.selection,
        status: "ready" as const,
      })),
    });
    vi.spyOn(modelProfilesApi, "apply").mockResolvedValue({
      status: "partial",
      items: [
        { target: "codex", outcome: "applied" },
        {
          target: "claude",
          outcome: "failed",
          reason: "Disk became unavailable",
        },
      ],
      undoToken: "undo-1",
    });
    vi.spyOn(modelProfilesApi, "undo").mockResolvedValue({
      status: "applied",
      items: [{ target: "codex", outcome: "applied" }],
    });
    render(<ModelProfilesPanel boundTools={["codex", "claude"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "预览应用" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "应用方案" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "应用方案" }));
    expect(await screen.findByText("Disk became unavailable")).toBeVisible();
    expect(modelProfilesApi.apply).toHaveBeenCalledWith("plan-1");
    expect(screen.getByText("部分完成，请查看逐项结果")).toBeVisible();
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "撤销上次应用",
      }),
    );
    await waitFor(() =>
      expect(modelProfilesApi.undo).toHaveBeenCalledWith("undo-1"),
    );
    await waitFor(() =>
      expect(
        screen.queryAllByRole("button", { name: "撤销上次应用" }),
      ).toHaveLength(0),
    );
  });

  it("uses actual refreshed managed state for the in-use badge", async () => {
    render(<ModelProfilesPanel boundTools={["codex"]} />);
    expect(await screen.findByText("未使用")).toBeVisible();
    vi.mocked(modelProfilesApi.list).mockResolvedValue([
      { ...profile, match: "inUse" },
    ]);
    emitTauriEvent("model-selection-changed", { tools: ["codex"] });
    expect(await screen.findByText("使用中")).toBeVisible();
  });

  it("shows a saved write with unavailable undo receipt as partial without offering undo", async () => {
    vi.spyOn(modelProfilesApi, "preview").mockResolvedValue({
      planToken: "plan-no-receipt",
      canApply: true,
      items: [],
    });
    vi.spyOn(modelProfilesApi, "apply").mockResolvedValue({
      status: "partial",
      items: [
        {
          target: "codex",
          outcome: "applied",
          reason: "modelProfiles.reasonUndoUnavailable",
        },
      ],
    });
    render(<ModelProfilesPanel boundTools={["codex"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "预览应用" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "应用方案" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "应用方案" }));
    expect(
      await screen.findByText("模型已保存，但无法确认撤销状态，请检查配置。"),
    ).toBeVisible();
    expect(screen.getByText("部分完成，请查看逐项结果")).toBeVisible();
    expect(
      screen.queryAllByRole("button", { name: "撤销上次应用" }),
    ).toHaveLength(0);
  });
  it("removes a consumed undo token when changed configuration rejects undo", async () => {
    vi.spyOn(modelProfilesApi, "preview").mockResolvedValue({
      planToken: "plan-undo",
      canApply: true,
      items: [
        {
          target: "codex",
          before: { kind: "single", model: "old/model" },
          after: { kind: "single", model: "new/model" },
          status: "ready",
        },
      ],
    });
    vi.spyOn(modelProfilesApi, "apply").mockResolvedValue({
      status: "applied",
      items: [{ target: "codex", outcome: "applied" }],
      undoToken: "one-use-undo",
    });
    vi.spyOn(modelProfilesApi, "undo").mockRejectedValue(
      new Error("configuration changed after apply"),
    );
    render(<ModelProfilesPanel boundTools={["codex"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "预览应用" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "应用方案" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "应用方案" }));
    await screen.findByText("应用完成");
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "撤销上次应用",
      }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "configuration changed after apply",
    );
    expect(
      screen.queryAllByRole("button", { name: "撤销上次应用" }),
    ).toHaveLength(0);
  });

  it("lists each profile as its own card, with 新建方案 as the one primary action", async () => {
    render(<ModelProfilesPanel boundTools={["codex"]} />);
    const card = await screen.findByRole("group", { name: "Daily" });
    expect(within(card).getByText("未使用")).toBeVisible();
    expect(within(card).getByText("Codex / ChatGPT")).toBeVisible();
    expect(
      screen
        .getAllByRole("button")
        .filter((button) => button.className.includes("bg-brand"))
        .map((button) => button.textContent),
    ).toEqual(["新建方案"]);
  });

  it("deletes only the saved recipe with its revision", async () => {
    render(<ModelProfilesPanel boundTools={["codex"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "删除 Daily" }));
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "删除" }),
    );
    await waitFor(() =>
      expect(modelProfilesApi.delete).toHaveBeenCalledWith(
        profile.id,
        profile.revision,
      ),
    );
  });
});
