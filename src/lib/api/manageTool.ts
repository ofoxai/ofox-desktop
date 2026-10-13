import { invoke } from "@tauri-apps/api/core";
import type { WorkBuddyModelSelection } from "@/lib/api/model-fetch";

/**
 * The wire protocol the OfoxAI gateway speaks for each tool.
 *
 * Pinned per tool because the model list endpoint differs by protocol:
 *   - anthropic → https://api.ofox.ai/anthropic/v1/models  (Claude)
 *   - openai    → https://api.ofox.ai/v1/models            (Codex / OpenCode-like)
 *   - gemini    → https://api.ofox.ai/gemini/v1beta/models (Gemini CLI)
 *
 * Mirrors `services/model_fetch.rs`'s `OFOX_*_MODELS_URL` constants. If a
 * tool isn't here, the manage dialog hides the model picker — that means
 * cc-switch doesn't bind/manage it, so there's nothing to point at.
 */
export type ToolProtocol = "openai" | "anthropic" | "gemini";

export const TOOL_PROTOCOL: Readonly<Record<string, ToolProtocol>> = {
  claude: "anthropic",
  codex: "openai",
  // ChatGPT desktop's Codex view uses the same config.toml as the CLI.
  // Chat and Work use the signed-in OpenAI account, not this API protocol.
  chatgpt: "openai",
  gemini: "gemini",
  opencode: "openai",
  openclaw: "openai",
  hermes: "openai",
  workbuddy: "openai",
  dsh: "openai",
  pi: "openai",
};

/** Route the desktop app's Codex view to the CLI's single shared config. */
export function managedToolId(app: string): string {
  return app === "chatgpt" ? "codex" : app;
}

/**
 * Connectivity probe result. Mirrors `commands/manage_tool.rs::PingResult` —
 * the backend dispatches on `app` to choose the correct OfoxAI endpoint
 * (Anthropic /messages, OpenAI /chat/completions, or Gemini :generateContent)
 * and asks for a 1-token response.
 *
 * Why 1-token: keeps cost negligible while still proving end-to-end —
 * authentication, gateway routing, upstream model availability all have to
 * line up for a 2xx to come back.
 *
 * `success === true` when the gateway returned 2xx. On failure, the dialog
 * shows `error` (a hint, the HTTP status and the gateway's own message);
 * a 402 in `statusCode` shows the out-of-credits notice with a top-up link.
 */
export interface PingResult {
  success: boolean;
  latencyMs: number;
  statusCode: number | null;
  error: string | null;
}

export interface WorkBuddyEndpointStatus {
  expectedUrl: string;
  configuredUrls: string[];
  externallyModified: boolean;
}

export type CompatibilityProtocol =
  | "responses"
  | "chatCompletions"
  | "anthropic"
  | "gemini";

export interface CompatibilityResult {
  app: string;
  model: string;
  protocol: CompatibilityProtocol | null;
  status: "compatible" | "incompatible" | "inconclusive";
  source: "catalog" | "cache" | "probe" | "manual";
  reason: string | null;
  allowedProtocols?: CompatibilityProtocol[];
}

/**
 * Manage-dialog API surface.
 *
 * Backed by `src-tauri/src/commands/manage_tool.rs`. Each method is a thin
 * `invoke` shim — kept here so the dialog and any future React Query
 * migration share one contract.
 *
 * Semantic notes:
 *   - `getActiveModel` returns `""` when the active provider has no model
 *     pinned (legitimate: OfoxAI uses default routing). Treat empty string
 *     as "未设置", not as an error.
 *   - `setActiveModel("")` removes the model field rather than writing an
 *     empty string into the live config.
 *   - `pingModel` always resolves with a `PingResult` — failures (timeouts,
 *     401, 5xx, …) are reported via the `success` field, not by rejecting
 *     the promise. The only way the promise rejects is a Tauri layer error
 *     (invalid app type, etc).
 *   - All writes target the *active* provider for the app — the dialog
 *     never switches providers; that's `switch_provider`'s job.
 */
export interface ProtocolHint {
  id: string;
  current: string;
  suggested: string;
}

export interface SaveModelsReport {
  saved: string[];
  skipped: { id: string; reason: string }[];
}

export const manageToolApi = {
  async getConfigFilePath(app: string): Promise<string> {
    return await invoke("get_tool_config_file_path", {
      app: managedToolId(app),
    });
  },

  async getActiveModel(app: string): Promise<string> {
    return await invoke("get_active_ofox_model", { app: managedToolId(app) });
  },

  /** ChatGPT 桌面 App 里最近选的模型；没有就是 Codex 配置里的。 */
  async getChatGptModel(): Promise<string> {
    return await invoke("get_chatgpt_active_model");
  },

  /** 已保存的模型列表。OpenCode 是用户选的一组，其余工具最多一个。 */
  async getActiveModels(app: string): Promise<string[]> {
    return await invoke("get_active_ofox_models", { app: managedToolId(app) });
  },

  /** OpenCode 保存一组模型；协议由后端按缓存和目录选，不发检测请求。 */
  async setActiveModels(
    app: string,
    models: string[],
  ): Promise<SaveModelsReport> {
    return await invoke("set_active_ofox_models", {
      app: managedToolId(app),
      models,
    });
  },

  /** 后台检测发现和已保存协议不一致的 OpenCode 模型；再保存一次即按检测结果改。 */
  async getOpenCodeProtocolHints(): Promise<ProtocolHint[]> {
    return await invoke("get_opencode_protocol_hints");
  },

  async setActiveModel(
    app: string,
    model: string,
    modelSelection?: WorkBuddyModelSelection,
    compatibilityProtocol?: CompatibilityProtocol,
  ): Promise<void> {
    await invoke("set_active_ofox_model", {
      app: managedToolId(app),
      model,
      modelSelection,
      compatibilityProtocol,
    });
  },

  async getWorkBuddyManagedModels(): Promise<string[]> {
    return await invoke("get_workbuddy_managed_models");
  },

  async getWorkBuddyEndpointStatus(): Promise<WorkBuddyEndpointStatus> {
    return await invoke("get_workbuddy_endpoint_status");
  },

  async setWorkBuddyManagedModels(
    modelSelections: WorkBuddyModelSelection[],
  ): Promise<void> {
    await invoke("set_workbuddy_managed_models", {
      modelSelections,
    });
  },

  async checkCompatibility(
    app: string,
    model: string,
    forceRetest = false,
  ): Promise<CompatibilityResult> {
    return await invoke("check_ofox_model_compatibility", {
      app: managedToolId(app),
      model,
      forceRetest,
    });
  },

  async pingModel(app: string, model: string): Promise<PingResult> {
    return await invoke("ofox_ping_model", { app: managedToolId(app), model });
  },
};
