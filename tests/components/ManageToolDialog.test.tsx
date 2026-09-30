import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
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
    fireEvent.click(
      screen.getByRole("combobox", { name: "手动选择 OpenCode 协议" }),
    );
    fireEvent.click(
      await screen.findByRole("option", { name: "Chat Completions" }),
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

describe("ManageToolDialog model picker", () => {
  it("bypasses the catalog cache only for an explicit refresh", async () => {
    const model = compatibleModel("openai/gpt-6-luna");
    mockSingleTool(model);
    const fetch = vi.spyOn(modelFetch, "fetchOfoxModels");

    await renderTool("opencode", "OpenCode");
    fireEvent.click(screen.getByRole("combobox"));
    await waitFor(() => expect(fetch).toHaveBeenCalledWith("openai", false));

    fireEvent.click(screen.getByRole("button", { name: "刷新模型列表" }));
    await waitFor(() => expect(fetch).toHaveBeenCalledWith("openai", true));
  });

  it("explains that ChatGPT desktop shares only its Codex mode with the CLI", async () => {
    const model = compatibleModel("openai/gpt-6-luna");
    model.supportedEndpoints = ["/v1/responses"];
    mockSingleTool(model);
    vi.spyOn(manageToolApi, "getActiveModel").mockResolvedValue(model.id);

    await renderTool("chatgpt", "ChatGPT");
    await screen.findByText(model.id);
    expect(screen.getByText(/Codex CLI 共用配置/)).toBeInTheDocument();
    expect(
      screen.getByText(/Chat 和 Work 使用 OpenAI 账号/),
    ).toBeInTheDocument();
    expect(manageToolApi.getActiveModel).toHaveBeenCalledWith("chatgpt");
    expect(screen.getByRole("combobox")).toBeInTheDocument();
  });

  it("keeps the dropdown inside the dialog and renders large catalogs in batches", async () => {
    const models = Array.from({ length: 200 }, (_, index) =>
      compatibleModel(`openai/model-${String(index).padStart(3, "0")}`),
    );
    mockSingleTool(models[0]);
    vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue(models);
    await renderTool("openclaw", "OpenClaw");
    await screen.findByText("/Users/test/.config/tool/config.json");
    fireEvent.click(screen.getByRole("combobox"));

    const list = await screen.findByRole("listbox");
    expect(list.closest('[role="dialog"]')).not.toBeNull();
    expect(screen.getAllByRole("option")).toHaveLength(80);
    expect(
      screen.queryByRole("option", { name: "openai/model-199" }),
    ).toBeNull();

    Object.defineProperties(list, {
      clientHeight: { configurable: true, value: 300 },
      scrollHeight: { configurable: true, value: 600 },
      scrollTop: { configurable: true, value: 300, writable: true },
    });
    fireEvent.scroll(list);
    expect(screen.getAllByRole("option")).toHaveLength(160);

    fireEvent.change(screen.getByPlaceholderText("搜索模型…"), {
      target: { value: "model-199" },
    });
    expect(
      screen.getByRole("option", { name: "openai/model-199" }),
    ).toBeVisible();
    expect(screen.getAllByRole("option")).toHaveLength(1);
  });
});

describe("ManageToolDialog WorkBuddy multi-model management", () => {
  function mockWorkBuddy(models: FetchedModel[], current = [models[0].id]) {
    vi.spyOn(manageToolApi, "getConfigFilePath").mockResolvedValue(
      "/Users/test/.workbuddy/models.json",
    );
    vi.spyOn(manageToolApi, "getWorkBuddyManagedModels").mockResolvedValue(
      current,
    );
    vi.spyOn(manageToolApi, "getWorkBuddyEndpointStatus").mockResolvedValue({
      expectedUrl: "https://api.ofox.io/v1/chat/completions",
      configuredUrls: ["https://api.ofox.io/v1/chat/completions"],
      externallyModified: false,
    });
    vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue(models);
  }

  it("shows the actual WorkBuddy endpoint and flags a stale region", async () => {
    mockWorkBuddy([compatibleModel("openai/model-a")]);
    vi.spyOn(manageToolApi, "getWorkBuddyEndpointStatus").mockResolvedValue({
      expectedUrl: "https://api.ofox.io/v1/chat/completions",
      configuredUrls: ["https://api.ofox.ai/v1/chat/completions"],
      externallyModified: false,
    });

    await renderTool("workbuddy", "WorkBuddy");

    expect(await screen.findByText("WorkBuddy 当前配置地址")).toBeVisible();
    expect(
      screen.getByText("https://api.ofox.ai/v1/chat/completions"),
    ).toBeVisible();
    expect(screen.getByText(/WorkBuddy 配置与当前地区不一致/)).toBeVisible();
  });

  it("saves every catalog-compatible model without automatic probes", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("anthropic/model-b"),
      compatibleModel("google/model-c"),
    ];
    mockWorkBuddy(models);
    const check = vi.spyOn(manageToolApi, "checkCompatibility");
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
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
    expect(check).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(save).toHaveBeenCalledWith(
        models.map(modelFetch.toWorkBuddyModelSelection),
      );
    });
    expect(check).not.toHaveBeenCalled();
  });

  it("manually checks only the chosen model without blocking save", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("openai/model-b"),
    ];
    mockWorkBuddy(models);
    const check = vi
      .spyOn(manageToolApi, "checkCompatibility")
      .mockResolvedValue({
        app: "workbuddy",
        model: models[1].id,
        protocol: "chatCompletions",
        status: "incompatible",
        source: "probe",
        reason: "流式正文不可读取",
      });

    await renderTool("workbuddy", "WorkBuddy");
    await screen.findByText("共 2 个兼容模型，共用一个 Ofox Key");
    fireEvent.click(screen.getByRole("button", { name: "全选兼容" }));
    fireEvent.click(
      screen.getByRole("combobox", { name: "选择待检测的 WorkBuddy 模型" }),
    );
    fireEvent.click(await screen.findByRole("option", { name: models[1].id }));
    fireEvent.click(screen.getByRole("button", { name: "检测选中模型" }));

    await screen.findByText(/model-b: 流式检测不兼容（仍可保存）/);
    expect(check).toHaveBeenCalledTimes(1);
    expect(check).toHaveBeenCalledWith("workbuddy", models[1].id, true);
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
  });

  it("can save while a manual WorkBuddy stream check is still running", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("openai/model-b"),
    ];
    mockWorkBuddy(models);
    vi.spyOn(manageToolApi, "checkCompatibility").mockImplementation(
      () => new Promise(() => {}),
    );
    const save = vi
      .spyOn(manageToolApi, "setWorkBuddyManagedModels")
      .mockResolvedValue();

    await renderTool("workbuddy", "WorkBuddy");
    await screen.findByText("共 2 个兼容模型，共用一个 Ofox Key");
    fireEvent.click(screen.getByRole("button", { name: "全选兼容" }));
    fireEvent.click(screen.getByRole("button", { name: "检测选中模型" }));

    expect(
      screen.getByRole("button", { name: "正在验证流式兼容性…" }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  });

  it("shows a successful manual stream check for its selected model", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("openai/model-b"),
    ];
    mockWorkBuddy(models);
    const check = vi
      .spyOn(manageToolApi, "checkCompatibility")
      .mockResolvedValue({
        app: "workbuddy",
        model: models[1].id,
        protocol: "chatCompletions",
        status: "compatible",
        source: "probe",
        reason: null,
      });

    await renderTool("workbuddy", "WorkBuddy");
    await screen.findByText("共 2 个兼容模型，共用一个 Ofox Key");
    fireEvent.click(screen.getByRole("button", { name: "全选兼容" }));
    fireEvent.click(
      screen.getByRole("combobox", { name: "选择待检测的 WorkBuddy 模型" }),
    );
    fireEvent.click(await screen.findByRole("option", { name: models[1].id }));
    fireEvent.click(screen.getByRole("button", { name: "检测选中模型" }));

    expect(await screen.findByText("openai/model-b: 兼容")).toBeVisible();
    expect(check).toHaveBeenCalledTimes(1);
    expect(check).toHaveBeenCalledWith("workbuddy", models[1].id, true);
  });

  it("keeps save disabled when the catalog lacks a selected model", async () => {
    const models = [compatibleModel("openai/model-a")];
    mockWorkBuddy(models, ["missing/model"]);
    const save = vi.spyOn(manageToolApi, "setWorkBuddyManagedModels");

    await renderTool("workbuddy", "WorkBuddy");
    await screen.findByText("共 1 个兼容模型，共用一个 Ofox Key");
    fireEvent.click(screen.getByRole("combobox", { name: "模型" }));
    fireEvent.click(
      within(await screen.findByRole("listbox")).getByRole("option", {
        name: /openai\/model-a/,
      }),
    );
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    expect(save).not.toHaveBeenCalled();
  });

  it("does not save an empty WorkBuddy selection", async () => {
    const model = compatibleModel("openai/model-a");
    mockWorkBuddy([model]);
    const save = vi.spyOn(manageToolApi, "setWorkBuddyManagedModels");

    await renderTool("workbuddy", "WorkBuddy");
    await screen.findByText("共 1 个兼容模型，共用一个 Ofox Key");
    fireEvent.click(screen.getByRole("combobox", { name: "模型" }));
    fireEvent.click(
      within(await screen.findByRole("listbox")).getByRole("option", {
        name: /openai\/model-a/,
      }),
    );

    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    expect(save).not.toHaveBeenCalled();
  });

  it("explains why save is unavailable when the catalog cannot load", async () => {
    mockWorkBuddy([compatibleModel("openai/model-a")]);
    vi.spyOn(modelFetch, "fetchOfoxModels").mockRejectedValue(
      new Error("catalog unavailable"),
    );

    await renderTool("workbuddy", "WorkBuddy");
    expect(
      await screen.findByText("模型目录加载失败，请刷新后重试。"),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });

  it("reports a manual timeout while keeping catalog-based save available", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("openai/model-b"),
    ];
    mockWorkBuddy(models);
    vi.spyOn(manageToolApi, "checkCompatibility").mockResolvedValue({
      app: "workbuddy",
      model: models[0].id,
      protocol: "chatCompletions",
      status: "inconclusive",
      source: "probe",
      reason: "网络错误或检测超时",
    });

    await renderTool("workbuddy", "WorkBuddy");
    await screen.findByText("共 2 个兼容模型，共用一个 Ofox Key");
    fireEvent.click(screen.getByRole("button", { name: "全选兼容" }));
    fireEvent.click(screen.getByRole("button", { name: "检测选中模型" }));

    expect(await screen.findByText(/网络错误或检测超时/)).toBeVisible();
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
  });
});
