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
  gemini: "gemini",
  opencode: "openai",
  openclaw: "openai",
  hermes: "openai",
  workbuddy: "openai",
};

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
 * surfaces `error` verbatim (already includes a Chinese hint plus the raw
 * HTTP status / body), and a 401 in `statusCode` triggers a reauth nudge.
 */
export interface PingResult {
  success: boolean;
  latencyMs: number;
  statusCode: number | null;
  error: string | null;
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
export const manageToolApi = {
  async getConfigFilePath(app: string): Promise<string> {
    return await invoke("get_tool_config_file_path", { app });
  },

  async getActiveModel(app: string): Promise<string> {
    return await invoke("get_active_ofox_model", { app });
  },

  async setActiveModel(
    app: string,
    model: string,
    modelSelection?: WorkBuddyModelSelection,
    compatibilityProtocol?: CompatibilityProtocol,
    allowUnverified?: boolean,
  ): Promise<void> {
    await invoke("set_active_ofox_model", {
      app,
      model,
      modelSelection,
      compatibilityProtocol,
      allowUnverified,
    });
  },

  async getWorkBuddyManagedModels(): Promise<string[]> {
    return await invoke("get_workbuddy_managed_models");
  },

  async setWorkBuddyManagedModels(
    modelSelections: WorkBuddyModelSelection[],
    allowUnverified?: boolean,
  ): Promise<void> {
    await invoke("set_workbuddy_managed_models", {
      modelSelections,
      allowUnverified,
    });
  },

  async checkCompatibility(
    app: string,
    model: string,
    forceRetest = false,
  ): Promise<CompatibilityResult> {
    return await invoke("check_ofox_model_compatibility", {
      app,
      model,
      forceRetest,
    });
  },

  async pingModel(app: string, model: string): Promise<PingResult> {
    return await invoke("ofox_ping_model", { app, model });
  },
};
