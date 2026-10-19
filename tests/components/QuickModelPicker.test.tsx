import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { toast } from "sonner";
import {
  QuickModelPicker,
  modelNotes,
} from "@/components/model-picker/QuickModelPicker";
import { queryClient } from "@/lib/query/queryClient";
import {
  modelSelectionApi,
  type ToolModelSnapshot,
  type ModelSelection,
} from "@/lib/api/modelSelection";
import { manageToolApi } from "@/lib/api/manageTool";
import * as modelFetch from "@/lib/api/model-fetch";
import * as apexApi from "@/lib/api/ofoxApex";

function snapshot(
  toolId = "codex",
  managedSelection: ModelSelection = { kind: "single", model: "openai/old" },
): ToolModelSnapshot {
  return {
    toolId,
    writeTarget: toolId === "chatgpt" ? "codex" : toolId,
    revision: "disk-revision-1",
    binding: {
      status: "configured",
      message: null,
      missingFiles: [],
      modifiedFields: [],
      envOverrides: [],
    },
    managedSelection,
    observedSelection: { model: "openai/old", source: "config" },
    supportsDefault: managedSelection.kind === "single",
    scope: [],
  };
}
const models = ["openai/old", "openai/new", "anthropic/claude"].map((id) => ({
  id,
  name: id,
  ownedBy: id.split("/")[0],
  pricingPrompt: null,
  supportedEndpoints: ["/v1/responses", "/v1/chat/completions"],
  supportedParameters: ["tools"],
  inputModalities: ["text"],
  outputModalities: ["text"],
}));

beforeEach(() => {
  queryClient.clear();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  HTMLElement.prototype.scrollIntoView = vi.fn();
  vi.spyOn(apexApi, "ofoxGetApexState").mockResolvedValue({
    apex: "ofox.io",
    pinned: true,
    detected: true,
  });
  vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue(models);
  vi.spyOn(modelSelectionApi, "save").mockImplementation(
    async (tool, selection) => ({
      snapshot: snapshot(tool, selection),
      saved: selection.kind === "models" ? selection.models : [selection.model],
      skipped: [],
    }),
  );
  vi.spyOn(toast, "success").mockImplementation(() => "");
  vi.spyOn(toast, "error").mockImplementation(() => "");
});
afterEach(() => vi.restoreAllMocks());

async function selectSingle(model = "openai/new") {
  await waitFor(() => expect(modelFetch.fetchOfoxModels).toHaveBeenCalled());
  fireEvent.click(screen.getByRole("combobox"));
  fireEvent.click(await screen.findByRole("option", { name: model }));
}

describe("QuickModelPicker", () => {
  it("saves a single selection immediately with the captured disk revision", async () => {
    const legacy = vi.spyOn(manageToolApi, "setActiveModel");
    render(
      <QuickModelPicker
        toolId="codex"
        snapshot={snapshot()}
        onRetry={vi.fn()}
      />,
    );
    await selectSingle();
    await waitFor(() =>
      expect(modelSelectionApi.save).toHaveBeenCalledWith(
        "codex",
        { kind: "single", model: "openai/new" },
        "disk-revision-1",
      ),
    );
    expect(legacy).not.toHaveBeenCalled();
    expect(toast.success).toHaveBeenCalledWith("模型已更新");
  });

  it("keeps the old selection and reports a stale-revision failure", async () => {
    vi.mocked(modelSelectionApi.save).mockRejectedValue(
      new Error("configuration changed"),
    );
    render(
      <QuickModelPicker
        toolId="codex"
        snapshot={snapshot()}
        onRetry={vi.fn()}
      />,
    );
    await selectSingle();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "configuration changed",
    );
    expect(screen.getByRole("combobox")).toHaveTextContent("openai/old");
    expect(toast.success).not.toHaveBeenCalled();
  });

  it("shows ChatGPT's observed model separately and only writes the shared default", async () => {
    const state = snapshot("chatgpt");
    state.observedSelection = { model: "app/current", source: "chatgpt" };
    render(
      <QuickModelPicker toolId="chatgpt" snapshot={state} onRetry={vi.fn()} />,
    );
    expect(screen.getByText("当前观察到的模型：app/current")).toBeVisible();
    expect(screen.getByRole("combobox")).toHaveTextContent("openai/old");
    await selectSingle();
    await waitFor(() =>
      expect(toast.success).toHaveBeenCalledWith(
        expect.stringContaining("共用配置的默认模型已更新"),
      ),
    );
    expect(screen.getByText("当前观察到的模型：app/current")).toBeVisible();
  });

  it.each(["opencode", "workbuddy"])(
    "keeps %s as an ordered multi-model set and never calls the single setter",
    async (tool) => {
      const legacy = vi.spyOn(manageToolApi, "setActiveModel");
      render(
        <QuickModelPicker
          toolId={tool}
          snapshot={snapshot(tool, {
            kind: "models",
            models: ["openai/new", "openai/old"],
          })}
          onRetry={vi.fn()}
        />,
      );
      fireEvent.click(screen.getByRole("button", { name: "2 个模型" }));
      const dialog = screen.getByRole("dialog");
      await waitFor(() =>
        expect(modelFetch.fetchOfoxModels).toHaveBeenCalled(),
      );
      fireEvent.click(within(dialog).getByRole("combobox"));
      fireEvent.click(
        await screen.findByRole("option", { name: /anthropic\/claude/ }),
      );
      fireEvent.click(within(dialog).getByRole("button", { name: "保存" }));
      await waitFor(() =>
        expect(modelSelectionApi.save).toHaveBeenCalledWith(
          tool,
          {
            kind: "models",
            models: ["openai/new", "openai/old", "anthropic/claude"],
          },
          "disk-revision-1",
        ),
      );
      expect(legacy).not.toHaveBeenCalled();
    },
  );

  it("shows OpenCode's observed route without reducing its managed model set", () => {
    const state = snapshot("opencode", {
      kind: "models",
      models: ["openai/old", "openai/new"],
    });
    state.observedSelection = {
      model: "external/provider-model",
      source: "tui",
    };
    render(
      <QuickModelPicker toolId="opencode" snapshot={state} onRetry={vi.fn()} />,
    );
    expect(screen.getByRole("button", { name: "2 个模型" })).toBeVisible();
    expect(
      screen.getByText("当前观察到的模型：external/provider-model"),
    ).toBeVisible();
    expect(modelSelectionApi.save).not.toHaveBeenCalled();
  });

  it("keeps an open multi-model editor on its original revision when the outer snapshot refreshes", async () => {
    const state = snapshot("opencode", {
      kind: "models",
      models: ["openai/old"],
    });
    const { rerender } = render(
      <QuickModelPicker toolId="opencode" snapshot={state} onRetry={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "1 个模型" }));
    rerender(
      <QuickModelPicker
        toolId="opencode"
        snapshot={{ ...state, revision: "disk-revision-2" }}
        onRetry={vi.fn()}
      />,
    );
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "保存" }),
    );
    await waitFor(() =>
      expect(modelSelectionApi.save).toHaveBeenCalledWith(
        "opencode",
        { kind: "models", models: ["openai/old"] },
        "disk-revision-1",
      ),
    );
  });

  it("does not describe mixed dsh scopes as a default route", () => {
    const state = snapshot("dsh", { kind: "single", model: "" });
    state.supportsDefault = false;
    state.scopeModels = [
      { scope: "one", modelId: "a" },
      { scope: "two", modelId: "b" },
    ];
    render(
      <QuickModelPicker toolId="dsh" snapshot={state} onRetry={vi.fn()} />,
    );
    expect(screen.getByRole("combobox")).toHaveTextContent("多个配置");
    expect(
      screen.getByText("多个配置使用不同模型，请选择一个明确的模型"),
    ).toBeVisible();
    expect(screen.queryByText("OfoxAI 默认路由")).toBeNull();
  });

  it("draws the single-model trigger as the app's 26px picker, in ink once a model is set", () => {
    render(
      <QuickModelPicker
        toolId="codex"
        snapshot={snapshot()}
        onRetry={vi.fn()}
      />,
    );
    const trigger = screen.getByRole("combobox");
    expect(trigger).toHaveAttribute("aria-haspopup", "dialog");
    expect(trigger).toHaveClass(
      "h-[26px]",
      "rounded-ctl",
      "border-line",
      "bg-card",
      "text-ctl",
      "text-ink",
    );
    expect(trigger.querySelector("svg.lucide-chevron-down")).toHaveClass(
      "group-data-[state=open]:rotate-180",
    );
  });

  it("shows the default route in the control colour while no model is set", () => {
    render(
      <QuickModelPicker
        toolId="codex"
        snapshot={snapshot("codex", { kind: "single", model: "" })}
        onRetry={vi.fn()}
      />,
    );
    const trigger = screen.getByRole("combobox");
    expect(trigger).toHaveTextContent("OfoxAI 默认路由");
    expect(trigger).toHaveClass("text-ink-ctl");
    expect(trigger).not.toHaveClass("text-ink");
  });

  it("draws the model-set button in the same 26px picker look", () => {
    render(
      <QuickModelPicker
        toolId="opencode"
        snapshot={snapshot("opencode", {
          kind: "models",
          models: ["openai/new", "openai/old"],
        })}
        onRetry={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "2 个模型" })).toHaveClass(
      "h-[26px]",
      "rounded-ctl",
      "border-line",
      "bg-card",
      "text-ctl",
    );
  });

  it("requires a readable state and a writable binding before opening a picker", () => {
    const state = snapshot();
    state.binding.status = "modified";
    render(
      <QuickModelPicker toolId="codex" snapshot={state} onRetry={vi.fn()} />,
    );
    expect(screen.getByRole("combobox")).toBeDisabled();
    expect(modelSelectionApi.save).not.toHaveBeenCalled();
  });
  it("respects backend capability even when the binding is configured", () => {
    const state = snapshot();
    state.canWrite = false;
    render(
      <QuickModelPicker toolId="codex" snapshot={state} onRetry={vi.fn()} />,
    );
    expect(screen.getByRole("combobox")).toBeDisabled();
  });
});

describe("QuickModelPicker as a tool row's field", () => {
  it("fills its column with one chip and puts no note under it", () => {
    const state = snapshot("opencode", {
      kind: "models",
      models: ["openai/old", "openai/new"],
    });
    state.observedSelection = { model: "external/x", source: "tui" };
    const { container } = render(
      <QuickModelPicker
        toolId="opencode"
        snapshot={state}
        onRetry={vi.fn()}
        variant="field"
      />,
    );
    const field = screen.getByRole("button", { name: "2 个模型" });
    expect(field).toHaveClass("h-[25px]", "w-full", "bg-pill");
    expect(field.querySelector("svg.lucide-chevron-down")).not.toBeNull();
    expect(container).not.toHaveTextContent("external/x");
  });

  it("stretches the single-model picker across the column", () => {
    render(
      <QuickModelPicker
        toolId="codex"
        snapshot={snapshot()}
        onRetry={vi.fn()}
        variant="field"
      />,
    );
    expect(screen.getByRole("combobox")).toHaveClass("w-full", "bg-pill");
  });

  it("holds the column with a placeholder of the same shape while loading", () => {
    render(
      <QuickModelPicker
        toolId="codex"
        loading
        onRetry={vi.fn()}
        variant="field"
      />,
    );
    expect(
      screen.getByRole("status", { name: "正在读取模型状态…" }),
    ).toHaveClass("h-[25px]", "w-full");
  });

  it("retries an unreadable state from inside the field", () => {
    const onRetry = vi.fn();
    render(
      <QuickModelPicker
        toolId="codex"
        error
        onRetry={onRetry}
        variant="field"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "无法读取模型状态" }));
    expect(onRetry).toHaveBeenCalled();
  });

  it("turns the chevron into a spinner while a change saves", async () => {
    vi.mocked(modelSelectionApi.save).mockReturnValue(new Promise(() => {}));
    render(
      <QuickModelPicker
        toolId="codex"
        snapshot={snapshot()}
        onRetry={vi.fn()}
        variant="field"
      />,
    );
    await selectSingle();
    await waitFor(() =>
      expect(screen.getByRole("combobox")).toHaveAttribute("aria-busy", "true"),
    );
    expect(screen.queryByText("正在保存…")).toBeNull();
  });

  it("names a failed save on the field instead of a line under it", async () => {
    vi.mocked(modelSelectionApi.save).mockRejectedValue(new Error("stale"));
    render(
      <QuickModelPicker
        toolId="codex"
        snapshot={snapshot()}
        onRetry={vi.fn()}
        variant="field"
      />,
    );
    await selectSingle();
    expect(await screen.findByRole("alert")).toHaveClass("sr-only");
    expect(screen.getByRole("combobox").title).toContain("stale");
  });
});

describe("modelNotes", () => {
  const t = i18n.t.bind(i18n);

  it("says what OpenCode runs beside its model set", () => {
    const state = snapshot("opencode", { kind: "models", models: ["a"] });
    state.observedSelection = { model: "external/x", source: "tui" };
    expect(modelNotes(state, "opencode", t)).toEqual(["实际在用 external/x"]);
  });

  it("stays quiet when a single-model tool runs what is set", () => {
    expect(modelNotes(snapshot(), "codex", t)).toEqual([]);
  });

  it("says what a single-model tool runs when it differs", () => {
    const state = snapshot();
    state.observedSelection = { model: "openai/live", source: "config" };
    expect(modelNotes(state, "codex", t)).toEqual(["实际在用 openai/live"]);
  });

  it("always says what ChatGPT runs, as it only shares the default", () => {
    expect(modelNotes(snapshot("chatgpt"), "chatgpt", t)).toEqual([
      "实际在用 openai/old",
    ]);
  });

  it("does not describe WorkBuddy's model set by what it ran last", () => {
    const state = snapshot("workbuddy", { kind: "models", models: ["a"] });
    expect(modelNotes(state, "workbuddy", t)).toEqual([]);
  });

  it("says the model is set in the app when Ofox cannot write it", () => {
    const state = snapshot();
    state.canWrite = false;
    expect(modelNotes(state, "codex", t)).toEqual(["模型在应用内切换"]);
  });

  it("has nothing to say before the state is read", () => {
    expect(modelNotes(undefined, "codex", t)).toEqual([]);
  });
});
