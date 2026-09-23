import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import ManageToolDialog from "@/components/console/ManageToolDialog";
import { manageToolApi } from "@/lib/api/manageTool";
import * as modelFetch from "@/lib/api/model-fetch";
import type { FetchedModel } from "@/lib/api/model-fetch";

function compatibleModel(id: string): FetchedModel {
  return {
    id,
    name: `Name ${id}`,
    ownedBy: id.split("/")[0],
    pricingPrompt: "0.000001",
    supportedEndpoints: ["/v1/chat/completions"],
    supportedParameters: ["tools"],
    inputModalities: ["text"],
    outputModalities: ["text"],
  };
}

afterEach(() => vi.restoreAllMocks());
beforeAll(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  HTMLElement.prototype.scrollIntoView = vi.fn();
});

function mockSingleTool(model: FetchedModel) {
  vi.spyOn(manageToolApi, "getConfigFilePath").mockResolvedValue(
    "/Users/test/.config/tool/config.json",
  );
  vi.spyOn(manageToolApi, "getActiveModel").mockResolvedValue("");
  vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue([model]);
}

async function chooseModel(id: string) {
  await screen.findByText("/Users/test/.config/tool/config.json");
  await act(async () => {
    fireEvent.click(screen.getByRole("combobox"));
  });
  await act(async () => {
    fireEvent.click(await screen.findByRole("option", { name: id }));
  });
}

async function renderTool(id: string, label: string, version = "1.0") {
  await act(async () => {
    render(
      <ManageToolDialog
        tool={{ id, abbr: "OC", label, color: "", version }}
        onOpenChange={vi.fn()}
      />,
    );
  });
}

describe("ManageToolDialog streaming compatibility", () => {
  it("reapplies the detected transport to an already-selected OpenCode model", async () => {
    const model = compatibleModel("z-ai/glm-5.3");
    model.supportedEndpoints = ["/v1/responses", "/v1/chat/completions"];
    mockSingleTool(model);
    vi.spyOn(manageToolApi, "getActiveModel").mockResolvedValue(model.id);
    const check = vi
      .spyOn(manageToolApi, "checkCompatibility")
      .mockResolvedValue({
        app: "opencode",
        model: model.id,
        protocol: "chatCompletions",
        status: "compatible",
        source: "probe",
        reason: null,
      });
    const save = vi.spyOn(manageToolApi, "setActiveModel").mockResolvedValue();

    await renderTool("opencode", "OpenCode");
    await waitFor(() =>
      expect(check).toHaveBeenCalledWith("opencode", model.id, false),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "应用检测结果" }),
      ).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "应用检测结果" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        "opencode",
        model.id,
        undefined,
        "chatCompletions",
        false,
      ),
    );
  });

  it("saves OpenCode with the detected Chat transport", async () => {
    const model = compatibleModel("z-ai/glm-5.3");
    model.supportedEndpoints = ["/v1/responses", "/v1/chat/completions"];
    mockSingleTool(model);
    vi.spyOn(manageToolApi, "checkCompatibility").mockResolvedValue({
      app: "opencode",
      model: model.id,
      protocol: "chatCompletions",
      status: "compatible",
      source: "probe",
      reason: null,
    });
    const save = vi.spyOn(manageToolApi, "setActiveModel").mockResolvedValue();
    await renderTool("opencode", "OpenCode");
    await chooseModel(model.id);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "保存" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        "opencode",
        model.id,
        undefined,
        "chatCompletions",
        false,
      ),
    );
  });

  it("requires a manual protocol and explicit acknowledgement when inconclusive", async () => {
    const model = compatibleModel("z-ai/glm-5.3");
    mockSingleTool(model);
    vi.spyOn(manageToolApi, "checkCompatibility").mockResolvedValue({
      app: "opencode",
      model: model.id,
      protocol: "responses",
      status: "inconclusive",
      source: "probe",
      reason: "网络错误",
    });
    await renderTool("opencode", "OpenCode");
    await chooseModel(model.id);
    await screen.findByText(/暂时无法确定/);
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    fireEvent.change(
      screen.getByRole("combobox", { name: "手动选择 OpenCode 协议" }),
      {
        target: { value: "chatCompletions" },
      },
    );
    fireEvent.click(screen.getByRole("checkbox"));
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
  });

  it("blocks a fixed-protocol client when its stream is incompatible", async () => {
    const model = compatibleModel("openai/model-a");
    mockSingleTool(model);
    vi.spyOn(manageToolApi, "checkCompatibility").mockResolvedValue({
      app: "openclaw",
      model: model.id,
      protocol: "chatCompletions",
      status: "incompatible",
      source: "probe",
      reason: "流式正文不可读取",
    });
    await renderTool("openclaw", "OpenClaw");
    await chooseModel(model.id);
    await screen.findByText(/不兼容，无法保存/);
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });

  it("ignores a stale check after the user selects another model", async () => {
    const first = compatibleModel("openai/model-a");
    const second = compatibleModel("openai/model-b");
    mockSingleTool(first);
    vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue([first, second]);
    let finishFirst!: (
      value: Awaited<ReturnType<typeof manageToolApi.checkCompatibility>>,
    ) => void;
    const firstResult = new Promise<
      Awaited<ReturnType<typeof manageToolApi.checkCompatibility>>
    >((resolve) => {
      finishFirst = resolve;
    });
    const check = vi
      .spyOn(manageToolApi, "checkCompatibility")
      .mockImplementation(async (app, model) =>
        model === first.id
          ? firstResult
          : {
              app,
              model,
              protocol: "chatCompletions",
              status: "compatible",
              source: "probe",
              reason: null,
            },
      );
    await renderTool("openclaw", "OpenClaw");
    await chooseModel(first.id);
    await waitFor(() =>
      expect(check).toHaveBeenCalledWith("openclaw", first.id, false),
    );
    await chooseModel(second.id);
    await screen.findByText(new RegExp(`${second.id}: 兼容`));
    await act(async () => {
      finishFirst({
        app: "openclaw",
        model: first.id,
        protocol: "chatCompletions",
        status: "incompatible",
        source: "probe",
        reason: "旧结果",
      });
    });
    expect(screen.queryByText(/旧结果/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
  });
});

describe("ManageToolDialog WorkBuddy multi-model management", () => {
  it("selects every compatible model and saves the complete set", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("anthropic/model-b"),
      compatibleModel("google/model-c"),
    ];
    vi.spyOn(manageToolApi, "getConfigFilePath").mockResolvedValue(
      "/Users/test/.workbuddy/models.json",
    );
    vi.spyOn(manageToolApi, "getWorkBuddyManagedModels").mockResolvedValue([
      "openai/model-a",
    ]);
    vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue(models);
    vi.spyOn(manageToolApi, "checkCompatibility").mockImplementation(
      async (app, model) => ({
        app,
        model,
        protocol: "chatCompletions",
        status: "compatible",
        source: "probe",
        reason: null,
      }),
    );
    const save = vi
      .spyOn(manageToolApi, "setWorkBuddyManagedModels")
      .mockResolvedValue();

    await renderTool("workbuddy", "WorkBuddy", "5.5.6");

    expect(
      await screen.findByText("共 3 个兼容模型，共用一个 Ofox Key"),
    ).toBeVisible();
    expect(screen.getByText("已选择 1 个兼容模型")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "全选兼容" }));
    expect(screen.getByText("已选择 3 个兼容模型")).toBeVisible();
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
    });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(save).toHaveBeenCalledWith(
        models.map(modelFetch.toWorkBuddyModelSelection),
        false,
      );
    });
  });
});
