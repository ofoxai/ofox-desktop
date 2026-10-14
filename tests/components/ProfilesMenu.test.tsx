import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { ProfilesMenu } from "@/components/model-profiles/ProfilesMenu";
import { modelProfilesApi, type ModelProfile } from "@/lib/api/modelProfiles";
import {
  modelSelectionApi,
  type ToolModelSnapshot,
} from "@/lib/api/modelSelection";
import { queryClient } from "@/lib/query/queryClient";
import * as modelFetch from "@/lib/api/model-fetch";
import * as apexApi from "@/lib/api/ofoxApex";

function snapshot(toolId: string, model = "old/model"): ToolModelSnapshot {
  return {
    toolId,
    writeTarget: toolId,
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
    supportsDefault: true,
    scope: [],
  };
}

const daily: ModelProfile = {
  id: "profile-1",
  name: "Daily",
  revision: "r1",
  match: "different",
  targets: [
    { target: "codex", selection: { kind: "single", model: "new/model" } },
    { target: "claude", selection: { kind: "single", model: "claude/new" } },
  ],
};
const team: ModelProfile = {
  id: "profile-2",
  name: "Team",
  revision: "r2",
  match: "inUse",
  targets: [
    { target: "codex", selection: { kind: "single", model: "old/model" } },
  ],
};

beforeEach(() => {
  queryClient.clear();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  HTMLElement.prototype.scrollIntoView = vi.fn();
  vi.spyOn(modelProfilesApi, "list").mockResolvedValue([daily, team]);
  vi.spyOn(modelSelectionApi, "snapshot").mockResolvedValue({
    tools: [snapshot("codex"), snapshot("claude")],
  });
  vi.spyOn(modelProfilesApi, "preview").mockResolvedValue({
    planToken: "plan-1",
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
    undoToken: "undo-1",
  });
  vi.spyOn(apexApi, "ofoxGetApexState").mockResolvedValue({
    apex: "ofox.io",
    pinned: true,
    detected: true,
  });
  vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue([]);
});
afterEach(() => vi.restoreAllMocks());

async function openMenu() {
  await userEvent.click(screen.getByRole("button", { name: "方案" }));
  return screen.findByRole("menu");
}

describe("ProfilesMenu", () => {
  it("loads the saved profiles only once the menu is opened", async () => {
    render(<ProfilesMenu boundTools={["codex", "claude"]} />);
    await waitFor(() => expect(modelSelectionApi.snapshot).toHaveBeenCalled());
    expect(modelProfilesApi.list).not.toHaveBeenCalled();

    await openMenu();

    expect(
      await screen.findByRole("menuitem", { name: /^Daily/ }),
    ).toBeVisible();
    expect(modelProfilesApi.list).toHaveBeenCalledTimes(1);
  });

  it("ticks the profile in use and says how many tools each one sets", async () => {
    render(<ProfilesMenu boundTools={["codex", "claude"]} />);
    await openMenu();

    const inUse = await screen.findByRole("menuitem", { name: /^Team/ });
    const other = screen.getByRole("menuitem", { name: /^Daily/ });
    expect(inUse).toHaveAccessibleName(/使用中/);
    expect(other).not.toHaveAccessibleName(/使用中/);
    expect(within(inUse).getByText("1 个工具")).toBeVisible();
    expect(within(other).getByText("2 个工具")).toBeVisible();
  });

  it("opens a profile's review once the menu has closed, applies it and then offers the undo", async () => {
    render(<ProfilesMenu boundTools={["codex", "claude"]} />);
    await openMenu();
    await userEvent.click(
      await screen.findByRole("menuitem", { name: /^Daily/ }),
    );

    const review = await screen.findByRole("dialog", { name: "Daily" });
    expect(screen.queryByRole("menu")).toBeNull();
    await waitFor(() =>
      expect(review).toContainElement(document.activeElement as HTMLElement),
    );
    expect(modelProfilesApi.preview).toHaveBeenCalledWith("profile-1");
    const apply = within(review).getByRole("button", { name: "应用方案" });
    await waitFor(() => expect(apply).toBeEnabled());
    await userEvent.click(apply);
    expect(await within(review).findByText("应用完成")).toBeVisible();
    expect(modelProfilesApi.apply).toHaveBeenCalledWith("plan-1");

    await userEvent.click(within(review).getByRole("button", { name: "关闭" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await openMenu();
    expect(
      screen.getByRole("menuitem", { name: "撤销上次应用" }),
    ).toBeVisible();
  });

  it("lets mouse users close profile management and return to tools", async () => {
    render(<ProfilesMenu boundTools={["codex"]} />);
    await openMenu();
    await userEvent.click(
      await screen.findByRole("menuitem", { name: "管理方案…" }),
    );
    const dialog = await screen.findByRole("dialog", { name: "模型方案" });
    await userEvent.click(within(dialog).getByRole("button", { name: "关闭" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("button", { name: "方案" })).toBeVisible();
  });

  it("offers no undo before anything was applied", async () => {
    render(<ProfilesMenu boundTools={["codex", "claude"]} />);
    await openMenu();
    await screen.findByRole("menuitem", { name: /^Daily/ });
    expect(screen.queryByRole("menuitem", { name: "撤销上次应用" })).toBeNull();
  });

  it("saves the current combination from the menu", async () => {
    render(<ProfilesMenu boundTools={["codex", "claude"]} />);
    await openMenu();
    const capture = await screen.findByRole("menuitem", {
      name: "保存当前组合…",
    });
    await waitFor(() => expect(capture).not.toHaveAttribute("data-disabled"));
    await userEvent.click(capture);

    const editor = await screen.findByRole("dialog", { name: "保存当前组合" });
    expect(
      within(editor).getByRole("textbox", { name: "方案名称" }),
    ).toBeVisible();
  });

  it("opens 管理方案… as a dialog of the profiles, whose review opens over it", async () => {
    render(<ProfilesMenu boundTools={["codex", "claude"]} />);
    await openMenu();
    await userEvent.click(
      await screen.findByRole("menuitem", { name: "管理方案…" }),
    );

    const manage = await screen.findByRole("dialog", { name: "模型方案" });
    const card = within(manage).getByRole("group", { name: "Daily" });
    expect(
      within(manage).getByRole("button", { name: "新建方案" }),
    ).toBeVisible();
    await userEvent.click(
      within(card).getByRole("button", { name: "预览应用" }),
    );
    expect(await screen.findByRole("dialog", { name: "Daily" })).toBeVisible();
    expect(modelProfilesApi.preview).toHaveBeenCalledWith("profile-1");
  });
});
