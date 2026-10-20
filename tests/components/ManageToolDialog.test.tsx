import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { useState } from "react";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import ManageToolDialog from "@/components/console/ManageToolDialog";
import { manageToolApi, type CompatibilityResult } from "@/lib/api/manageTool";
import * as modelFetch from "@/lib/api/model-fetch";
import type { FetchedModel } from "@/lib/api/model-fetch";
import * as bindToolsModule from "@/lib/bindTools";
import { ofoxBindApi, type UnbindReport } from "@/lib/api/ofoxBind";
import { toast } from "sonner";
import { BOUND_TOOLS_STORAGE_KEY } from "@/config/toolMeta";
import { settingsApi } from "@/lib/api";
import {
  modelSelectionApi,
  type ModelSelection,
  type ToolModelSnapshot,
} from "@/lib/api/modelSelection";
import { queryClient } from "@/lib/query/queryClient";
import { emitTauriEvent } from "../msw/tauriMocks";
import * as uninstallPreviewModule from "@/hooks/useUninstallPreview";

let selections: Record<string, ModelSelection> = {};
let observed: Record<string, string> = {};
function snapshot(
  toolId: string,
  selection: ModelSelection,
): ToolModelSnapshot {
  return {
    toolId,
    writeTarget: toolId === "chatgpt" ? "codex" : toolId,
    revision: `revision-${toolId}`,
    binding: {
      status: "configured",
      message: null,
      missingFiles: [],
      modifiedFields: [],
      envOverrides: [],
    },
    managedSelection: selection,
    observedSelection: {
      model:
        observed[toolId] ??
        (selection.kind === "single" ? selection.model : null),
      source: "config",
    },
    scope: [],
  };
}
function saveReport(
  toolId: string,
  selection: ModelSelection,
  skipped: { id: string; reason: string }[] = [],
) {
  return {
    snapshot: snapshot(toolId, selection),
    saved: selection.kind === "models" ? selection.models : [selection.model],
    skipped,
  };
}

const account = vi.hoisted(() => ({
  apex: "ofox.io" as "ofox.ai" | "ofox.io",
  role: "owner" as "owner" | "admin" | "member",
}));

vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({
    apex: account.apex,
    pinned: false,
    isLoading: false,
    refetch: vi.fn(),
  }),
}));

vi.mock("@/hooks/useOfoxAuth", () => ({
  useOfoxAuth: () => ({
    status: { user: { role: account.role } },
    isLoading: false,
  }),
}));

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
beforeEach(() => {
  localStorage.clear();
  queryClient.clear();
  selections = {};
  observed = {};
  vi.spyOn(modelSelectionApi, "snapshot").mockImplementation(
    async (tools = []) => ({
      tools: await Promise.all(
        tools.map(async (toolId) => {
          const item = snapshot(
            toolId,
            selections[toolId] ?? { kind: "single", model: "" },
          );
          item.binding = await ofoxBindApi
            .status(toolId)
            .catch(() => ({ ...item.binding, status: "unknown" as const }));
          return item;
        }),
      ),
    }),
  );
  vi.spyOn(modelSelectionApi, "save").mockImplementation(
    async (tool, selection) => saveReport(tool, selection),
  );
  vi.spyOn(ofoxBindApi, "status").mockResolvedValue({
    status: "configured",
    message: null,
    missingFiles: [],
    modifiedFields: [],
    envOverrides: [],
  });
});

describe("ManageToolDialog binding lifecycle", () => {
  const emptyReport: UnbindReport = {
    tool: "codex",
    dryRun: true,
    legacy: false,
    alreadyUnbound: true,
    exactFiles: [],
    restoredKeys: [],
    removedKeys: [],
    keptKeys: [],
    filesRemoved: [],
    providerRestoredTo: null,
    sharedKeptBy: [],
    warnings: [],
  };

  async function renderLifecycleTool(
    onChanged = vi.fn(),
    onOpenChange = vi.fn(),
  ) {
    mockSingleTool(compatibleModel("test/model"));
    selections["codex"] =
      selections["chatgpt"] =
      selections["gemini"] =
      selections["openclaw"] =
      selections["hermes"] =
        { kind: "single", model: "test/model" };
    await act(async () => {
      render(
        <ManageToolDialog
          tool={{
            id: "codex",
            abbr: "Cx",
            label: "Codex",
            color: "",
            version: null,
            installationStatus: "notInstalled",
          }}
          onChanged={onChanged}
          onOpenChange={onOpenChange}
        />,
      );
    });
    await screen.findByText("/Users/test/.config/tool/config.json");
  }

  it("blocks ordinary saving and connectivity checks when the configuration is missing", async () => {
    vi.mocked(ofoxBindApi.status).mockResolvedValue({
      status: "missing",
      message: null,
      missingFiles: ["~/.codex/config.toml"],
      modifiedFields: [],
      envOverrides: [],
    });
    const save = vi.spyOn(modelSelectionApi, "save");
    const ping = vi.spyOn(manageToolApi, "pingModel");
    const compatibility = vi.spyOn(manageToolApi, "checkCompatibility");
    await renderLifecycleTool();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "测试连通性" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "解除绑定" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "恢复绑定" })).toBeEnabled();
    expect(save).not.toHaveBeenCalled();
    expect(ping).not.toHaveBeenCalled();
    expect(compatibility).not.toHaveBeenCalled();
  });

  it("explicitly restores only the managed tool and retains shared binding parties", async () => {
    localStorage.setItem(
      BOUND_TOOLS_STORAGE_KEY,
      JSON.stringify(["codex", "chatgpt", "claude"]),
    );
    vi.mocked(ofoxBindApi.status).mockResolvedValue({
      status: "missing",
      message: null,
      missingFiles: [],
      modifiedFields: [],
      envOverrides: [],
    });
    const restore = vi.spyOn(ofoxBindApi, "restore").mockResolvedValue();
    const onChanged = vi.fn();
    const onOpenChange = vi.fn();
    await renderLifecycleTool(onChanged, onOpenChange);
    expect(restore).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "恢复绑定" }));
    await waitFor(() =>
      expect(restore).toHaveBeenCalledWith("codex", ["chatgpt", "claude"]),
    );
    expect(onChanged).toHaveBeenCalledOnce();
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(JSON.parse(localStorage.getItem(BOUND_TOOLS_STORAGE_KEY)!)).toEqual([
      "codex",
      "chatgpt",
      "claude",
    ]);
  });

  it("keeps the binding and dialog available after a failed restore", async () => {
    vi.mocked(ofoxBindApi.status).mockResolvedValue({
      status: "missing",
      message: null,
      missingFiles: [],
      modifiedFields: [],
      envOverrides: [],
    });
    vi.spyOn(ofoxBindApi, "restore").mockRejectedValue(
      new Error("write failed"),
    );
    const error = vi.spyOn(toast, "error").mockImplementation(() => "");
    const onChanged = vi.fn();
    const onOpenChange = vi.fn();
    await renderLifecycleTool(onChanged, onOpenChange);
    fireEvent.click(screen.getByRole("button", { name: "恢复绑定" }));
    await waitFor(() =>
      expect(error).toHaveBeenCalledWith("恢复绑定失败：Error: write failed"),
    );
    expect(onChanged).not.toHaveBeenCalled();
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "恢复绑定" })).toBeEnabled();
  });

  it("does not offer restoration or model writes over a user-modified configuration", async () => {
    vi.mocked(ofoxBindApi.status).mockResolvedValue({
      status: "modified",
      message: "Configuration conflict",
      missingFiles: [],
      modifiedFields: ["~/.codex/config.toml · model", "current-provider"],
      envOverrides: [],
    });
    const restore = vi.spyOn(ofoxBindApi, "restore");
    await renderLifecycleTool();
    expect(screen.getByText(/绑定配置已被修改/)).toBeVisible();
    expect(screen.getByText("~/.codex/config.toml · model")).toBeVisible();
    expect(screen.getByText("当前服务商已切换为其它服务商")).toBeVisible();
    expect(
      screen.queryByText("Configuration conflict"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "恢复绑定" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "解除绑定" })).toBeEnabled();
    expect(restore).not.toHaveBeenCalled();
  });

  it("names the environment variables that override the binding and how to remove them", async () => {
    vi.mocked(ofoxBindApi.status).mockResolvedValue({
      status: "configured",
      message: null,
      missingFiles: [],
      modifiedFields: [],
      envOverrides: [
        { name: "GEMINI_API_KEY", scope: "machine", location: null },
        { name: "GEMINI_MODEL", scope: "file", location: "~/.zshrc:3" },
      ],
    });
    await renderLifecycleTool();
    expect(
      screen.getByText(
        /这些环境变量会盖过 Ofox 写入的配置，Codex 可能不会走 Ofox/,
      ),
    ).toBeVisible();
    expect(screen.getByText("GEMINI_API_KEY")).toBeVisible();
    expect(screen.getByText("Windows 系统环境变量")).toBeVisible();
    expect(screen.getByText("~/.zshrc:3")).toBeVisible();
    expect(screen.getByText(/系统属性 → 高级 → 环境变量/)).toBeVisible();
    expect(screen.getByText(/删除上面配置文件里的这些行/)).toBeVisible();
  });

  it("allows unbinding even when the configuration status cannot be read", async () => {
    vi.mocked(ofoxBindApi.status).mockRejectedValue(
      new Error("permission denied"),
    );
    vi.spyOn(ofoxBindApi, "unbindPreview").mockRejectedValue(
      new Error("preview unavailable"),
    );
    const unbind = vi
      .spyOn(bindToolsModule, "unbindTool")
      .mockResolvedValue({ ...emptyReport, dryRun: false });
    await renderLifecycleTool();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "解除绑定" }));
    const confirm = await screen.findByRole("button", {
      name: "确认解除绑定？",
    });
    fireEvent.click(confirm);
    await waitFor(() => expect(unbind).toHaveBeenCalledWith("codex"));
  });

  it("shows missing-file warnings before confirming unbind", async () => {
    vi.mocked(ofoxBindApi.status).mockResolvedValue({
      status: "missing",
      message: null,
      missingFiles: [],
      modifiedFields: [],
      envOverrides: [],
    });
    vi.spyOn(ofoxBindApi, "unbindPreview").mockResolvedValue({
      ...emptyReport,
      warnings: [
        { code: "configAlreadyMissing", file: "~/.codex/config.toml" },
      ],
    });
    await renderLifecycleTool();
    fireEvent.click(screen.getByRole("button", { name: "解除绑定" }));
    expect(await screen.findByText(/配置已删除，不再恢复/)).toBeVisible();
    expect(screen.getByText("~/.codex/config.toml")).toBeVisible();
  });
});
beforeAll(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  HTMLElement.prototype.scrollIntoView = vi.fn();
});

function mockSingleTool(model: FetchedModel) {
  vi.spyOn(manageToolApi, "getConfigFilePath").mockResolvedValue(
    "/Users/test/.config/tool/config.json",
  );
  selections["codex"] =
    selections["chatgpt"] =
    selections["gemini"] =
    selections["openclaw"] =
    selections["hermes"] =
      { kind: "single", model: "" };
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

describe("ManageToolDialog compact sections", () => {
  it("keeps model, config and connection actions in named sections while folding static notes", async () => {
    mockSingleTool(compatibleModel("gemini/test"));
    selections.gemini = { kind: "single", model: "gemini/test" };
    await renderTool("gemini", "Gemini CLI");
    await screen.findByText("/Users/test/.config/tool/config.json");

    const model = screen.getByRole("region", { name: "模型" });
    expect(within(model).getByRole("combobox")).toBeEnabled();
    const files = screen.getByRole("region", { name: "配置文件" });
    expect(
      within(files).getByText("/Users/test/.config/tool/config.json"),
    ).toBeVisible();
    expect(
      within(files).getByRole("button", { name: /Finder|文件夹/ }),
    ).toBeEnabled();
    const connection = screen.getByRole("region", { name: "连通性" });
    expect(
      within(connection).getByRole("button", { name: "测试连通性" }),
    ).toBeEnabled();

    const summary = screen.getByText("配置说明");
    const notes = summary.closest("details")!;
    expect(notes).not.toHaveAttribute("open");
    const rememberHint = screen.getByText(
      zh.modelCompatibility.geminiRememberHint,
    );
    expect(rememberHint).not.toBeVisible();
    summary.focus();
    expect(summary).toHaveFocus();
    fireEvent.click(summary);
    expect(notes).toHaveAttribute("open");
    expect(rememberHint).toBeVisible();
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "解除绑定" })).toBeEnabled();
  });

  it.each([
    ["claude", "Claude Code", "claudeRememberHint"],
    ["pi", "Pi", "piRememberHint"],
  ] as const)(
    "tells %s users how to save a default model inside the agent",
    async (id, label, key) => {
      mockSingleTool(compatibleModel("anthropic/test"));
      selections[id] = { kind: "single", model: "anthropic/test" };
      await renderTool(id, label);
      const summary = await screen.findByText("配置说明");
      fireEvent.click(summary);
      expect(screen.getByText(zh.modelCompatibility[key])).toBeVisible();
    },
  );

  it("keeps real model-load and environment warnings outside the collapsed notes", async () => {
    mockSingleTool(compatibleModel("gemini/test"));
    const ping = vi.spyOn(manageToolApi, "pingModel");
    vi.mocked(modelFetch.fetchOfoxModels).mockRejectedValue(
      new Error("catalog unavailable"),
    );
    vi.mocked(ofoxBindApi.status).mockResolvedValue({
      status: "configured",
      message: null,
      missingFiles: [],
      modifiedFields: [],
      envOverrides: [
        { name: "GEMINI_API_KEY", scope: "process", location: null },
      ],
    });
    selections.gemini = { kind: "single", model: "gemini/test" };
    await renderTool("gemini", "Gemini CLI");

    const notes = screen.getByText("配置说明").closest("details")!;
    expect(notes).not.toHaveAttribute("open");
    const retry = await screen.findByRole("button", { name: "重试" });
    expect(retry).toBeVisible();
    expect(notes).not.toContainElement(retry);
    const variable = screen.getByText("GEMINI_API_KEY");
    expect(variable).toBeVisible();
    expect(notes).not.toContainElement(variable);
    expect(screen.getByRole("button", { name: "解除绑定" })).toBeEnabled();
    expect(modelSelectionApi.save).not.toHaveBeenCalled();
    expect(ping).not.toHaveBeenCalled();
  });
});

describe("ManageToolDialog keyboard return focus", () => {
  function FocusHarness({ onUninstall }: { onUninstall?: () => void }) {
    const [open, setOpen] = useState(false);
    const [returnFocus, setReturnFocus] = useState<HTMLElement | null>(null);
    return (
      <>
        <button
          onClick={(event) => {
            setReturnFocus(event.currentTarget);
            setOpen(true);
          }}
        >
          Manage Gemini
        </button>
        <ManageToolDialog
          tool={
            open
              ? {
                  id: "gemini",
                  abbr: "G",
                  label: "Gemini CLI",
                  color: "",
                  version: "1.0",
                }
              : null
          }
          returnFocus={returnFocus}
          onOpenChange={setOpen}
          onUninstall={onUninstall}
        />
      </>
    );
  }

  async function openManage(onUninstall?: () => void) {
    mockSingleTool(compatibleModel("gemini/test"));
    selections.gemini = { kind: "single", model: "gemini/test" };
    render(<FocusHarness onUninstall={onUninstall} />);
    const origin = screen.getByRole("button", { name: "Manage Gemini" });
    vi.spyOn(origin, "getClientRects").mockReturnValue([
      { width: 100, height: 30 },
    ] as unknown as DOMRectList);
    origin.focus();
    fireEvent.click(origin);
    await screen.findByText("/Users/test/.config/tool/config.json");
    return origin;
  }

  it.each(["Escape", "cancel"])(
    "returns %s dismissal to the invoking tool row",
    async (kind) => {
      const origin = await openManage();
      if (kind === "Escape") {
        fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
      } else {
        fireEvent.click(screen.getByRole("button", { name: "取消" }));
      }
      await waitFor(() =>
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
      );
      await waitFor(() => expect(origin).toHaveFocus());
    },
  );

  it.each(["hidden", "disabled", "detached"])(
    "does not restore an unavailable %s origin",
    async (state) => {
      const origin = await openManage();
      const focus = vi.spyOn(origin, "focus");
      if (state === "hidden") origin.hidden = true;
      if (state === "disabled") origin.setAttribute("disabled", "");
      if (state === "detached")
        vi.spyOn(origin, "isConnected", "get").mockReturnValue(false);
      fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
      await waitFor(() =>
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
      );
      await act(async () => new Promise((resolve) => setTimeout(resolve, 20)));
      expect(focus).not.toHaveBeenCalled();
    },
  );

  it("keeps uninstall handoff ahead of restoring the tool-row focus", async () => {
    vi.spyOn(uninstallPreviewModule, "useUninstallPreview").mockReturnValue({
      available: true,
      data: undefined,
      error: undefined,
      retry: vi.fn(),
    });
    const onUninstall = vi.fn();
    const origin = await openManage(onUninstall);
    const focus = vi.spyOn(origin, "focus");
    fireEvent.click(screen.getByRole("button", { name: "卸载…" }));
    await waitFor(() => expect(onUninstall).toHaveBeenCalledOnce());
    expect(focus).not.toHaveBeenCalled();
  });
});

describe("ManageToolDialog streaming compatibility", () => {
  it("does not overwrite a draft or advance its write revision after an external model change", async () => {
    const model = compatibleModel("openai/new");
    model.supportedEndpoints = ["/v1/responses"];
    mockSingleTool(model);
    selections.codex = { kind: "single", model: "openai/old" };
    await renderTool("codex", "Codex");
    await chooseModel(model.id);
    vi.mocked(modelSelectionApi.snapshot).mockResolvedValue({
      tools: [
        {
          ...snapshot("codex", { kind: "single", model: "external/model" }),
          revision: "changed-revision",
        },
      ],
    });
    act(() => emitTauriEvent("model-selection-changed", { tools: ["codex"] }));
    await waitFor(() =>
      expect(modelSelectionApi.snapshot).toHaveBeenCalledTimes(2),
    );
    expect(screen.getByRole("combobox")).toHaveTextContent("openai/new");
    vi.mocked(modelSelectionApi.save).mockRejectedValue(
      new Error("stale revision"),
    );
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(modelSelectionApi.save).toHaveBeenCalledWith(
        "codex",
        { kind: "single", model: "openai/new" },
        "revision-codex",
      ),
    );
    expect(screen.getByRole("dialog")).toBeVisible();
  });
  it("does not check Codex or ChatGPT models before saving", async () => {
    const model = compatibleModel("deepseek/deepseek-v4.1-flash");
    model.supportedEndpoints = ["/v1/responses"];
    mockSingleTool(model);
    const check = vi.spyOn(manageToolApi, "checkCompatibility");
    const save = vi.spyOn(modelSelectionApi, "save");

    await renderTool("chatgpt", "ChatGPT");
    await chooseModel(model.id);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "保存" })).toBeEnabled(),
    );
    expect(check).not.toHaveBeenCalled();
    expect(screen.queryByText(/流式兼容性/)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        "chatgpt",
        { kind: "single", model: model.id },
        "revision-chatgpt",
      ),
    );
  });

  it("edits the shared default while displaying ChatGPT's observed model separately", async () => {
    mockSingleTool(compatibleModel("openai/gpt-6-luna"));
    // The Codex config still says the old default; the ChatGPT app moved on.
    selections["codex"] =
      selections["chatgpt"] =
      selections["gemini"] =
      selections["openclaw"] =
      selections["hermes"] =
        { kind: "single", model: "gpt-5.6-luna" };
    observed.chatgpt = "gpt-6-luna";

    await renderTool("chatgpt", "ChatGPT");

    expect(
      await screen.findByText(/当前观察到的模型：gpt-6-luna/),
    ).toBeVisible();
    expect(screen.getByRole("combobox")).toHaveTextContent("gpt-5.6-luna");
  });

  it("still reads the Codex config model for Codex itself", async () => {
    mockSingleTool(compatibleModel("openai/gpt-6-luna"));
    selections["codex"] =
      selections["chatgpt"] =
      selections["gemini"] =
      selections["openclaw"] =
      selections["hermes"] =
        { kind: "single", model: "gpt-5.6-luna" };
    observed.chatgpt = "gpt-6-luna";

    await renderTool("codex", "Codex");

    expect(await screen.findByText("gpt-5.6-luna")).toBeVisible();
    expect(modelSelectionApi.snapshot).toHaveBeenCalledWith(["codex"]);
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
    expect(
      await screen.findByText(
        "openai/model-a 不能在 OpenClaw 中使用（流式正文不可读取），请换一个模型。",
      ),
    ).toBeVisible();
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
    await waitFor(() =>
      expect(check).toHaveBeenCalledWith("openclaw", second.id, false),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "保存" })).toBeEnabled(),
    );
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

describe("ManageToolDialog re-saving an unchanged model", () => {
  it("re-saves the loaded model of a fixed-protocol client", async () => {
    const model = compatibleModel("google/gemini-3.8-flash");
    mockSingleTool(model);
    selections["codex"] =
      selections["chatgpt"] =
      selections["gemini"] =
      selections["openclaw"] =
      selections["hermes"] =
        { kind: "single", model: model.id };
    const check = vi.spyOn(manageToolApi, "checkCompatibility");
    const save = vi.spyOn(modelSelectionApi, "save");

    await renderTool("gemini", "Gemini");
    await waitFor(() =>
      expect(screen.getByRole("combobox")).toHaveTextContent(model.id),
    );
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        "gemini",
        { kind: "single", model: model.id },
        "revision-gemini",
      ),
    );
    expect(check).not.toHaveBeenCalled();
  });

  it("keeps save disabled when the current model failed to load", async () => {
    const model = compatibleModel("google/gemini-3.8-flash");
    mockSingleTool(model);
    vi.mocked(modelSelectionApi.snapshot).mockRejectedValue(
      new Error("read failed"),
    );
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const save = vi.spyOn(modelSelectionApi, "save");

    await renderTool("gemini", "Gemini");
    await screen.findByText("/Users/test/.config/tool/config.json");

    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    expect(save).not.toHaveBeenCalled();
  });
});

describe("ManageToolDialog model picker", () => {
  it("loads the model list when the dialog opens, with no refresh button", async () => {
    const model = compatibleModel("openai/gpt-6-luna");
    mockSingleTool(model);
    const fetch = vi.spyOn(modelFetch, "fetchOfoxModels");

    await renderTool("openclaw", "OpenClaw");
    await waitFor(() => expect(fetch).toHaveBeenCalledWith("openai", false));
    expect(
      screen.queryByRole("button", { name: "刷新模型列表" }),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("combobox"));
    expect(await screen.findByRole("option", { name: model.id })).toBeVisible();
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it("says the list failed to load and retries without the cache", async () => {
    const model = compatibleModel("openai/gpt-6-luna");
    mockSingleTool(model);
    const fetch = vi
      .spyOn(modelFetch, "fetchOfoxModels")
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue([model]);
    const toastError = vi.spyOn(toast, "error");

    await renderTool("opencode", "OpenCode");
    expect(await screen.findByText("模型列表加载失败")).toBeVisible();
    expect(toastError).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() => expect(fetch).toHaveBeenCalledWith("openai", true));
    await waitFor(() =>
      expect(screen.queryByText("模型列表加载失败")).not.toBeInTheDocument(),
    );
  });

  it("explains that ChatGPT desktop shares only its Codex mode with the CLI", async () => {
    const model = compatibleModel("openai/gpt-6-luna");
    model.supportedEndpoints = ["/v1/responses"];
    mockSingleTool(model);
    observed.chatgpt = model.id;

    await renderTool("chatgpt", "ChatGPT");
    await screen.findByText(new RegExp(`当前观察到的模型：${model.id}`));
    expect(screen.getByText(/Codex CLI 共用配置/)).toBeInTheDocument();
    expect(
      screen.getByText(/Chat 和 Work 使用 OpenAI 账号/),
    ).toBeInTheDocument();
    expect(modelSelectionApi.snapshot).toHaveBeenCalledWith(["chatgpt"]);
    expect(screen.getByText(/保存的是 Codex 的默认模型/)).toBeInTheDocument();
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
    selections.workbuddy = { kind: "models", models: current };
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

    expect(await screen.findByText("当前地址")).toBeVisible();
    expect(
      screen.getByText("https://api.ofox.ai/v1/chat/completions"),
    ).toBeVisible();
    expect(screen.getByText(/与当前地区不一致/)).toBeVisible();
  });

  it("saves every catalog-compatible model without automatic probes", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("anthropic/model-b"),
      compatibleModel("google/model-c"),
    ];
    mockWorkBuddy(models);
    const check = vi.spyOn(manageToolApi, "checkCompatibility");
    const save = vi.spyOn(modelSelectionApi, "save");

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
        "workbuddy",
        { kind: "models", models: models.map((model) => model.id) },
        "revision-workbuddy",
      );
    });
    expect(check).not.toHaveBeenCalled();
  });

  it("clears the whole WorkBuddy selection", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("anthropic/model-b"),
    ];
    mockWorkBuddy(models);

    await renderTool("workbuddy", "WorkBuddy", "5.5.6");
    await screen.findByText("共 2 个兼容模型，共用一个 Ofox Key");
    fireEvent.click(screen.getByRole("button", { name: "全选兼容" }));
    expect(screen.getByText("已选择 2 个兼容模型")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "取消全选" }));
    expect(screen.getByText("请选择至少一个兼容模型")).toBeVisible();
    expect(screen.getByRole("button", { name: "取消全选" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "精选模型" })).toBeNull();
  });

  it("re-saves an unchanged WorkBuddy selection", async () => {
    const model = compatibleModel("openai/model-a");
    mockWorkBuddy([model]);
    const save = vi.spyOn(modelSelectionApi, "save");

    await renderTool("workbuddy", "WorkBuddy");
    await screen.findByText("共 1 个兼容模型，共用一个 Ofox Key");
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        "workbuddy",
        { kind: "models", models: [model.id] },
        "revision-workbuddy",
      ),
    );
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

  it("defers conflicting writes until a manual WorkBuddy stream check finishes", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("openai/model-b"),
    ];
    mockWorkBuddy(models);
    let finishCheck!: (result: CompatibilityResult) => void;
    vi.spyOn(manageToolApi, "checkCompatibility").mockImplementation(
      () =>
        new Promise((resolve) => {
          finishCheck = resolve;
        }),
    );
    const save = vi.spyOn(modelSelectionApi, "save");

    await renderTool("workbuddy", "WorkBuddy");
    await screen.findByText("共 2 个兼容模型，共用一个 Ofox Key");
    fireEvent.click(screen.getByRole("button", { name: "全选兼容" }));
    fireEvent.click(screen.getByRole("button", { name: "检测选中模型" }));

    expect(
      screen.getByRole("button", { name: "正在验证流式兼容性…" }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "解除绑定" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(save).not.toHaveBeenCalled();
    await act(async () =>
      finishCheck({
        app: "workbuddy",
        model: models[0].id,
        protocol: "chatCompletions",
        status: "compatible",
        source: "probe",
        reason: null,
      }),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "保存" })).toBeEnabled(),
    );
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

  it("picks the model to check from the app's own menu inside the dialog, the chosen one ticked", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("openai/model-b"),
    ];
    mockWorkBuddy(
      models,
      models.map((model) => model.id),
    );

    await renderTool("workbuddy", "WorkBuddy");
    await screen.findByText("共 2 个兼容模型，共用一个 Ofox Key");
    const picker = screen.getByRole("combobox", {
      name: "选择待检测的 WorkBuddy 模型",
    });
    expect(picker).toHaveAttribute("aria-haspopup", "listbox");
    expect(picker).toHaveTextContent("openai/model-a");

    fireEvent.click(picker);
    const listbox = await screen.findByRole("listbox");
    expect(listbox.closest('[role="dialog"]')).toContainElement(picker);
    expect(
      within(listbox).getByRole("option", { name: "openai/model-a" }),
    ).toHaveAttribute("data-checked", "true");
    expect(
      within(listbox).getByRole("option", { name: "openai/model-b" }),
    ).not.toHaveAttribute("data-checked");
  });

  it("keeps save disabled when the catalog lacks a selected model", async () => {
    const models = [compatibleModel("openai/model-a")];
    mockWorkBuddy(models, ["missing/model"]);
    const save = vi.spyOn(modelSelectionApi, "save");

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
    const save = vi.spyOn(modelSelectionApi, "save");

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
      await screen.findByText("模型列表加载失败，加载后才能保存。"),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    expect(
      screen.queryByRole("button", { name: "刷新模型列表" }),
    ).not.toBeInTheDocument();

    vi.mocked(modelFetch.fetchOfoxModels).mockResolvedValue([
      compatibleModel("openai/model-a"),
    ]);
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "保存" })).toBeEnabled(),
    );
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

describe("ManageToolDialog connectivity", () => {
  async function pingWithEmptyBalance() {
    const model = compatibleModel("deepseek/deepseek-v4.1-flash");
    mockSingleTool(model);
    selections["codex"] =
      selections["chatgpt"] =
      selections["gemini"] =
      selections["openclaw"] =
      selections["hermes"] =
        { kind: "single", model: model.id };
    vi.spyOn(manageToolApi, "pingModel").mockResolvedValue({
      success: false,
      latencyMs: 120,
      statusCode: 402,
      error: "OFOX 账户余额不足，充值后即可使用。",
    });
    await renderTool("openclaw", "OpenClaw");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "测试连通性" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "测试连通性" }));
    await screen.findByText("余额不足");
  }

  it("tells an owner the balance ran out and opens the wallet for their region", async () => {
    account.role = "owner";
    account.apex = "ofox.io";
    const open = vi.spyOn(settingsApi, "openExternal").mockResolvedValue();
    await pingWithEmptyBalance();

    expect(screen.queryByText("连通失败")).not.toBeInTheDocument();
    expect(screen.queryByText(/HTTP 402|insufficient_credits/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "去充值" }));
    await waitFor(() =>
      expect(open).toHaveBeenCalledWith("https://app.ofox.io/manage/wallet"),
    );
  });

  it("asks a member to contact an admin instead of offering a top-up", async () => {
    account.role = "member";
    await pingWithEmptyBalance();

    expect(
      screen.queryByRole("button", { name: "去充值" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/请联系团队管理员充值/)).toBeVisible();
  });
});

describe("ManageToolDialog unbind", () => {
  function report(extra: Partial<UnbindReport> = {}): UnbindReport {
    return {
      tool: "codex",
      dryRun: true,
      legacy: false,
      alreadyUnbound: false,
      exactFiles: ["~/.codex/config.toml"],
      restoredKeys: ["~/.codex/config.toml: model"],
      removedKeys: ["~/.codex/config.toml: model_provider"],
      keptKeys: [],
      filesRemoved: [],
      providerRestoredTo: "default",
      sharedKeptBy: [],
      warnings: [],
      ...extra,
    };
  }

  async function renderCodex(onOpenChange = vi.fn()) {
    vi.spyOn(manageToolApi, "getConfigFilePath").mockResolvedValue(
      "/Users/test/.codex/config.toml",
    );
    selections["codex"] =
      selections["chatgpt"] =
      selections["gemini"] =
      selections["openclaw"] =
      selections["hermes"] =
        { kind: "single", model: "" };
    vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue([]);
    await act(async () => {
      render(
        <ManageToolDialog
          tool={{
            id: "codex",
            abbr: "CX",
            label: "Codex",
            color: "",
            version: "1.0",
          }}
          onOpenChange={onOpenChange}
        />,
      );
    });
    await screen.findByText("/Users/test/.codex/config.toml");
  }

  it("previews what the first click will restore and that MCP/skills are kept", async () => {
    const preview = vi
      .spyOn(ofoxBindApi, "unbindPreview")
      .mockResolvedValue(report());
    await renderCodex();

    fireEvent.click(screen.getByRole("button", { name: "解除绑定" }));

    expect(await screen.findByText(/恢复到绑定前/)).toBeVisible();
    expect(screen.getByText("~/.codex/config.toml: model")).toBeVisible();
    expect(
      screen.getByText("~/.codex/config.toml: model_provider"),
    ).toBeVisible();
    expect(screen.getByText(/MCP、skills、插件/)).toBeVisible();
    expect(preview).toHaveBeenCalledWith("codex", []);
    expect(
      screen.getByRole("button", { name: "确认解除绑定？" }),
    ).toBeVisible();
  });

  it("says settings changed after binding are kept and lists them", async () => {
    const kept = report({
      exactFiles: [],
      keptKeys: ["~/.codex/config.toml: model_reasoning_effort"],
      warnings: [{ code: "userValueKept" }],
    });
    vi.spyOn(ofoxBindApi, "unbindPreview").mockResolvedValue(kept);
    const unbind = vi
      .spyOn(bindToolsModule, "unbindTool")
      .mockResolvedValue({ ...kept, dryRun: false });
    const success = vi.spyOn(toast, "success").mockImplementation(() => "");
    await renderCodex();

    fireEvent.click(screen.getByRole("button", { name: "解除绑定" }));

    expect(
      await screen.findByText(/Ofox 能判断是你修改过的设置会保留/),
    ).toBeVisible();
    expect(screen.queryByText(/也会恢复为绑定前的值/)).not.toBeInTheDocument();
    const listed =
      "绑定后你改过的设置保持不变：~/.codex/config.toml: model_reasoning_effort";
    expect(screen.getByText(listed)).toBeVisible();

    fireEvent.click(
      await screen.findByRole("button", { name: "确认解除绑定？" }),
    );
    await waitFor(() => expect(unbind).toHaveBeenCalledWith("codex"));
    await waitFor(() =>
      expect(success).toHaveBeenCalledWith("Codex 已解除绑定", {
        description: expect.stringContaining(listed),
      }),
    );
  });

  it("warns that an older binding cannot be fully restored", async () => {
    vi.spyOn(ofoxBindApi, "unbindPreview").mockResolvedValue(
      report({ legacy: true, exactFiles: [], restoredKeys: [] }),
    );
    await renderCodex();
    fireEvent.click(screen.getByRole("button", { name: "解除绑定" }));
    expect(await screen.findByText(/旧版本中绑定/)).toBeVisible();
  });

  it("explains that ChatGPT keeps the shared Codex config", async () => {
    vi.spyOn(ofoxBindApi, "unbindPreview").mockResolvedValue(
      report({ sharedKeptBy: ["chatgpt"], restoredKeys: [], removedKeys: [] }),
    );
    await renderCodex();
    fireEvent.click(screen.getByRole("button", { name: "解除绑定" }));
    expect(await screen.findByText(/ChatGPT 仍在使用 Ofox/)).toBeVisible();
  });

  it("confirms on the second click and summarizes the restore", async () => {
    vi.spyOn(ofoxBindApi, "unbindPreview").mockResolvedValue(report());
    const unbind = vi
      .spyOn(bindToolsModule, "unbindTool")
      .mockResolvedValue(report({ dryRun: false }));
    const success = vi.spyOn(toast, "success").mockImplementation(() => "");
    const onOpenChange = vi.fn();
    await renderCodex(onOpenChange);

    fireEvent.click(screen.getByRole("button", { name: "解除绑定" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "确认解除绑定？" }),
    );

    await waitFor(() => expect(unbind).toHaveBeenCalledWith("codex"));
    await waitFor(() =>
      expect(success).toHaveBeenCalledWith("Codex 已解除绑定", {
        description: "配置文件已恢复为绑定前的原样。",
      }),
    );
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("keeps the dialog open when unbinding fails", async () => {
    vi.spyOn(ofoxBindApi, "unbindPreview").mockResolvedValue(report());
    vi.spyOn(bindToolsModule, "unbindTool").mockRejectedValue(
      new Error("restore failed"),
    );
    const error = vi.spyOn(toast, "error").mockImplementation(() => "");
    const onOpenChange = vi.fn();
    await renderCodex(onOpenChange);

    fireEvent.click(screen.getByRole("button", { name: "解除绑定" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "确认解除绑定？" }),
    );

    await waitFor(() =>
      expect(error).toHaveBeenCalledWith("解除绑定失败：Error: restore failed"),
    );
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });
});

describe("ManageToolDialog OpenCode multi-model management", () => {
  function mockOpenCode(models: FetchedModel[], saved: string[] = []) {
    vi.spyOn(manageToolApi, "getConfigFilePath").mockResolvedValue(
      "/Users/test/.config/opencode/opencode.json",
    );
    selections.opencode = { kind: "models", models: saved };
    vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue(models);
  }

  async function pick(...ids: string[]) {
    fireEvent.click(screen.getByRole("combobox", { name: "模型" }));
    const listbox = await screen.findByRole("listbox");
    for (const id of ids) {
      fireEvent.click(
        within(listbox).getByRole("option", { name: new RegExp(id) }),
      );
    }
  }

  it("shows the saved set and lets the user save several models at once", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("anthropic/model-b"),
      compatibleModel("google/model-c"),
    ];
    mockOpenCode(models, ["openai/model-a"]);
    const save = vi
      .spyOn(modelSelectionApi, "save")
      .mockResolvedValue(
        saveReport("opencode", { kind: "models", models: [] }, []),
      );
    const single = vi.spyOn(manageToolApi, "setActiveModel");

    await renderTool("opencode", "OpenCode");
    expect(await screen.findByText("已选择 1 个兼容模型")).toBeVisible();

    await pick("anthropic/model-b");
    await waitFor(() =>
      expect(screen.getByText("已选择 2 个兼容模型")).toBeVisible(),
    );
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        "opencode",
        { kind: "models", models: ["openai/model-a", "anthropic/model-b"] },
        "revision-opencode",
      ),
    );
    expect(single).not.toHaveBeenCalled();
  });

  it("offers to apply a protocol the background check preferred, by saving the saved set again", async () => {
    mockOpenCode([compatibleModel("openai/model-a")], ["openai/model-a"]);
    const hints = vi
      .spyOn(manageToolApi, "getOpenCodeProtocolHints")
      .mockResolvedValueOnce([
        {
          id: "openai/model-a",
          current: "responses",
          suggested: "chatCompletions",
        },
      ])
      .mockResolvedValue([]);
    const save = vi
      .spyOn(modelSelectionApi, "save")
      .mockResolvedValue(
        saveReport(
          "opencode",
          { kind: "models", models: ["openai/model-a"] },
          [],
        ),
      );

    await renderTool("opencode", "OpenCode");
    expect(await screen.findByText(/openai\/model-a → Chat/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "应用建议" }));

    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        "opencode",
        { kind: "models", models: ["openai/model-a"] },
        "revision-opencode",
      ),
    );
    await waitFor(() =>
      expect(screen.queryByText(/openai\/model-a → Chat/)).toBeNull(),
    );
    expect(hints).toHaveBeenCalledTimes(2);
  });

  it("shows no protocol suggestion when the background check agrees", async () => {
    mockOpenCode([compatibleModel("openai/model-a")], ["openai/model-a"]);
    vi.spyOn(manageToolApi, "getOpenCodeProtocolHints").mockResolvedValue([]);

    await renderTool("opencode", "OpenCode");
    await screen.findByText("已选择 1 个兼容模型");

    expect(screen.queryByRole("button", { name: "应用建议" })).toBeNull();
  });

  it("offers select-all for OpenCode and saves every listed model", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("anthropic/model-b"),
      compatibleModel("google/model-c"),
    ];
    mockOpenCode(models, ["openai/model-a"]);
    const save = vi
      .spyOn(modelSelectionApi, "save")
      .mockResolvedValue(
        saveReport(
          "opencode",
          { kind: "models", models: models.map((model) => model.id) },
          [],
        ),
      );

    await renderTool("opencode", "OpenCode");
    await screen.findByText("已选择 1 个兼容模型");
    fireEvent.click(screen.getByRole("button", { name: "全选兼容" }));
    await waitFor(() =>
      expect(screen.getByText("已选择 3 个兼容模型")).toBeVisible(),
    );
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        "opencode",
        {
          kind: "models",
          models: ["openai/model-a", "anthropic/model-b", "google/model-c"],
        },
        "revision-opencode",
      ),
    );
  });

  it("warns about the models the backend skipped but still closes after saving the rest", async () => {
    mockOpenCode(
      [compatibleModel("openai/model-a"), compatibleModel("anthropic/model-b")],
      ["openai/model-a"],
    );
    vi.spyOn(modelSelectionApi, "save").mockResolvedValue(
      saveReport("opencode", { kind: "models", models: ["openai/model-a"] }, [
        { id: "anthropic/model-b", reason: "模型不在当前 Ofox 目录中" },
      ]),
    );
    const toastWarning = vi.spyOn(toast, "warning");

    await renderTool("opencode", "OpenCode");
    await screen.findByText("已选择 1 个兼容模型");
    await pick("anthropic/model-b");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "保存" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(toastWarning).toHaveBeenCalledWith(
        expect.stringContaining("anthropic/model-b"),
      ),
    );
  });

  it("does not save an empty selection", async () => {
    mockOpenCode([compatibleModel("openai/model-a")], ["openai/model-a"]);
    const save = vi
      .spyOn(modelSelectionApi, "save")
      .mockResolvedValue(
        saveReport("opencode", { kind: "models", models: [] }, []),
      );

    await renderTool("opencode", "OpenCode");
    await screen.findByText("已选择 1 个兼容模型");
    fireEvent.click(screen.getByRole("button", { name: "取消全选" }));

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "保存" })).toBeDisabled(),
    );
    expect(save).not.toHaveBeenCalled();
  });

  it("does not run a per-model check in the dialog or on save", async () => {
    mockOpenCode([compatibleModel("openai/model-a")], ["openai/model-a"]);
    const check = vi.spyOn(manageToolApi, "checkCompatibility");

    await renderTool("opencode", "OpenCode");
    await screen.findByText("已选择 1 个兼容模型");

    expect(check).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
  });

  it("reports the model the backend rejected and keeps the dialog open", async () => {
    mockOpenCode(
      [compatibleModel("openai/model-a"), compatibleModel("anthropic/model-b")],
      ["openai/model-a"],
    );
    vi.spyOn(modelSelectionApi, "save").mockRejectedValue(
      "anthropic/model-b：模型与客户端协议不兼容",
    );
    const toastError = vi.spyOn(toast, "error");

    await renderTool("opencode", "OpenCode");
    await screen.findByText("已选择 1 个兼容模型");
    await pick("anthropic/model-b");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "保存" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith(
        expect.stringContaining("anthropic/model-b"),
      ),
    );
  });

  it("checks one chosen model's stream on demand and shows the protocol it found", async () => {
    mockOpenCode(
      [compatibleModel("openai/model-a"), compatibleModel("anthropic/model-b")],
      ["openai/model-a", "anthropic/model-b"],
    );
    const check = vi
      .spyOn(manageToolApi, "checkCompatibility")
      .mockResolvedValue({
        app: "opencode",
        model: "anthropic/model-b",
        protocol: "chatCompletions",
        status: "compatible",
        source: "probe",
        reason: null,
      });

    await renderTool("opencode", "OpenCode");
    await screen.findByText("已选择 2 个兼容模型");
    expect(check).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("combobox", { name: "选择待检测的 OpenCode 模型" }),
    );
    fireEvent.click(
      await screen.findByRole("option", { name: "anthropic/model-b" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "检测选中模型" }));

    await waitFor(() =>
      expect(check).toHaveBeenCalledWith("opencode", "anthropic/model-b", true),
    );
    expect(await screen.findByText(/Chat Completions/)).toBeVisible();
  });

  it("says a model that fails the stream check may not work but can still be saved", async () => {
    mockOpenCode([compatibleModel("openai/model-a")], ["openai/model-a"]);
    vi.spyOn(manageToolApi, "checkCompatibility").mockResolvedValue({
      app: "opencode",
      model: "openai/model-a",
      protocol: null,
      status: "incompatible",
      source: "probe",
      reason: "流式输出不完整",
    });

    await renderTool("opencode", "OpenCode");
    await screen.findByText("已选择 1 个兼容模型");
    fireEvent.click(screen.getByRole("button", { name: "检测选中模型" }));

    expect(await screen.findByText(/仍可保存/)).toBeVisible();
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
  });
});
