import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { toast } from "sonner";
import i18n from "@/i18n";
import { BOUND_TOOLS_STORAGE_KEY } from "@/config/toolMeta";
import { settingsApi } from "@/lib/api";
import { manageToolApi, TOOL_PROTOCOL } from "@/lib/api/manageTool";
import type { UnbindReport } from "@/lib/api/ofoxBind";
import {
  fetchOfoxModels,
  filterOfoxModelsByProtocol,
  filterOfoxModelsForWorkBuddy,
  pickCheapestPaidModel,
  pickWorkBuddyCuratedModels,
  toWorkBuddyModelSelection,
  type WorkBuddyModelSelection,
} from "@/lib/api/model-fetch";

/**
 * Codex CLI 启动时强制要求 `model` 非空——空字符串会触发 "Thread model is
 * unavailable under the current configuration"。其他工具的 CLI 都允许空
 * （走 OfoxAI 默认路由），所以这个 hardcode 兜底只在 Codex + fetch 失败的
 * 极端组合下生效，不构成全局的价格表 / 模型列表硬编码。
 *
 * 值取 bailian/qwen3-coder-plus —— 这是改造前 Codex seed 的硬编码默认，
 * 已经过 dogfooding，启动可用。日后如果 OfoxAI 下架该 model，会从 fetch
 * 列表里挑到新的，正常路径用不到这个值。
 */
const CODEX_SAFE_FALLBACK = "bailian/qwen3-coder-plus";

/**
 * Bind 后立刻给工具挑一个默认 model 写进 active provider 的 settings_config。
 *
 * 触发条件：`getActiveModel` 返回空字符串（seed 模板里 model 留空 / 用户
 * 此前没设过）。用户已选过 model 时 early-return，绝不覆盖用户选择。
 *
 * 选法："最便宜但不免费"——见 {@link pickCheapestPaidModel}。
 * 兜底：fetch 失败 / 列表为空时，**只对 Codex** 回落 hardcode，其他工具留空
 * （它们的 CLI 容忍空 model，会走 OfoxAI 默认路由）。
 *
 * 不阻塞 bind 主流程：任何错误都吞掉打 warn，让用户至少能用上工具，进
 * "管理"里手动选 model 也能恢复。
 */
async function ensureDefaultModel(tool: string): Promise<void> {
  const protocol = TOOL_PROTOCOL[tool];
  if (!protocol) return;

  let current = "";
  try {
    current = (await manageToolApi.getActiveModel(tool)).trim();
  } catch {
    // 读不到当 "" 处理——后续逻辑会按"未设置"挑默认。
  }
  if (current) return;

  let picked = "";
  try {
    const raw = await fetchOfoxModels(protocol);
    // codex CLI 强绑 responses 协议——挑默认模型时也要收窄，否则会挑到
    // bailian/qwen-max 这种只支持 chat/completions 的模型，用户一起 codex
    // 就撞 "wire_api not supported"。
    const requiredEndpoint = tool === "codex" ? "/v1/responses" : undefined;
    const candidates = filterOfoxModelsByProtocol(
      raw,
      protocol,
      requiredEndpoint,
    );
    picked = pickCheapestPaidModel(candidates);
  } catch (e) {
    console.warn(`[bindTools] fetch models for ${tool} failed`, e);
  }

  if (!picked && tool === "codex") picked = CODEX_SAFE_FALLBACK;
  if (!picked) return;

  try {
    await manageToolApi.setActiveModel(tool, picked);
  } catch (e) {
    console.warn(`[bindTools] setActiveModel(${tool}, ${picked}) failed`, e);
    toast.warning(i18n.t("modelCompatibility.defaultModelFailed", { tool }));
  }
}

/**
 * Mirror the bound-tools list into `AppSettings.boundTools` so the Rust
 * background loops (specifically `services::tool_health`) can know which
 * tools to probe without coupling to the frontend's localStorage.
 *
 * Best-effort: failures only log + swallow. The localStorage write is the
 * primary source of truth for the UI; the settings mirror is a hint for
 * the backend, and missing it just means the next health-check round
 * sees an empty list (no probes, no harm).
 *
 * Always emits `ofox-prefs-updated` afterwards so the health-check loop
 * can wake up from a long sleep when the bound set changes.
 */
async function mirrorBoundToolsToSettings(tools: string[]): Promise<void> {
  try {
    await settingsApi.saveBoundTools(tools);
    await emit("ofox-prefs-updated");
  } catch (e) {
    // Swallow — local UI behavior is already correct via localStorage.
    console.warn("[bindTools] mirrorBoundToolsToSettings failed", e);
  }
}

/**
 * 走"ofox 直写"路径 bind 的工具：调 `ofox_bind_tool` —— 后端拿 sk-of-
 * API key（keychain 命中 / 否则调 `/openapi/api-keys` 签发）→ 直接把真
 * Token 写进工具配置文件、baseURL 指 ofox gateway，**绕开** proxy takeover。
 *
 * 6 工具全部走直写：Claude / Codex / Gemini / OpenCode / OpenClaw / Hermes。
 * Gemini 走 `env.GEMINI_API_KEY` + `GOOGLE_GEMINI_BASE_URL`，CLI 支持这条
 * 第三方走法（其 Google OAuth 是另一条独立分支，跟我们无关）。
 *
 * `chatgpt` 也在集合里：ChatGPT App 的 Codex 模式与 codex CLI 共用
 * `~/.codex/config.toml`。后端把两者记为同一份配置的两个绑定方——只绑 ChatGPT
 * 也会配置 `~/.codex`，两个都解绑后才还原。Chat/Work 使用 OpenAI 登录态，不受影响。
 *
 * Keep this in sync with `commands/ofox_auth.rs::ofox_provider_for`.
 */
const OFOX_AUTO_BIND_TOOLS: ReadonlySet<string> = new Set([
  "claude",
  "codex",
  "chatgpt",
  "gemini",
  "opencode",
  "openclaw",
  "hermes",
  "workbuddy",
]);

async function selectWorkBuddyDefaults(): Promise<WorkBuddyModelSelection[]> {
  const all = await fetchOfoxModels("openai");
  const compatible = filterOfoxModelsForWorkBuddy(all);
  if (compatible.length === 0) {
    throw new Error("暂无同时支持文本、工具调用和 chat/completions 的模型");
  }
  const existingIds = await manageToolApi
    .getWorkBuddyManagedModels()
    .catch(() => []);
  const existing = existingIds
    .map((id) => compatible.find((model) => model.id === id))
    .filter((model): model is (typeof compatible)[number] => !!model);
  const selected =
    existing.length > 0 ? existing : pickWorkBuddyCuratedModels(compatible);
  return selected.map(toWorkBuddyModelSelection);
}

/**
 * Bind new tools and merge successful additions into the existing bound set.
 *
 * Single source of truth for "what does binding a tool *do*?" — both the
 * Onboarding flow (first-time bind) and the in-Console "+ 添加" dialog
 * (incremental bind) go through here. Keeping them aligned matters because
 * MainApp.tsx's reconciliation useEffect treats `boundTools ∩ PROXY_SUPPORTED`
 * as locked-on; if a path skips the bind call, the user sees the "已锁定"
 * badge but the proxy is silently off (or worse, pointing at the official
 * provider which has no base_url and 400s at request time).
 *
 * Per-tool behavior：所有 6 个 ofox-supported 工具走 `ofox_bind_tool` —— 切换
 * active provider 到 ofox-<tool>、从 keychain 取或经 /openapi/api-keys 签发
 * sk-of- API key、备份 + 字段级 patch 写盘。**无 proxy takeover**。集合外的工具
 * （例如未来加新工具但还没走通 ofox 路径）跳过。
 *
 * 历史 PROXY_SUPPORTED_TOOLS gate 已经移除——它原本是用来过滤"走 proxy
 * takeover 的工具"，但 ofox 改造后 bind 完全绕开 proxy，把 OpenCode/OpenClaw/
 * Hermes 这些 multi-provider 容器型工具也错挡在外。现在统一用
 * OFOX_AUTO_BIND_TOOLS 作为唯一 gate。
 *
 * Existing bindings are never rewritten here: their files may have been
 * deliberately deleted, and restoring them requires an explicit action.
 * Returns the complete retained binding set, including successful additions.
 */
export async function bindTools(tools: string[]): Promise<string[]> {
  const existing = new Set(readBoundTools());
  const succeeded: string[] = [];
  const failed: string[] = [];
  for (const tool of new Set(tools)) {
    if (existing.has(tool)) continue;
    if (!OFOX_AUTO_BIND_TOOLS.has(tool)) {
      // 非 ofox 接管类工具——本流程不操作其后端状态，视为成功记下来。
      succeeded.push(tool);
      continue;
    }
    try {
      const modelSelections =
        tool === "workbuddy" ? await selectWorkBuddyDefaults() : undefined;
      await invoke("ofox_bind_tool", { app: tool, modelSelections });
      // bind 之后立即挑默认 model：必须在 ofox_bind_tool 完成后才跑，
      // 因为 setActiveModel 依赖 active provider 已切到 ofox-<tool>。
      // 单条失败不影响 bind 结果——ensureDefaultModel 内部自吞异常。
      if (tool !== "workbuddy") await ensureDefaultModel(tool);
      succeeded.push(tool);
    } catch (e) {
      console.error(`[bindTools] bind ${tool} failed`, e);
      failed.push(tool);
    }
  }

  if (failed.length > 0) {
    // 让用户知道哪些工具没绑上——之前是 silent failure，用户只能从"工具
    // 没出现在列表里"反推。常见原因是 token 过期：toast 文案不点破具体
    // 错因（后端错误对最终用户不友好），主流程仍然继续，已成功的工具会
    // 被正常持久化。
    toast.error(
      i18n.t("toolLifecycle.bindFailed", { tools: failed.join("、") }),
    );
  }

  const retained = Array.from(new Set([...readBoundTools(), ...succeeded]));
  localStorage.setItem(BOUND_TOOLS_STORAGE_KEY, JSON.stringify(retained));
  // Mirror to backend after the bind round-trip so the settings.json 镜像
  // 也只看到已成功的工具——避免 health loop 去探一个根本没 bind 的 app。
  void mirrorBoundToolsToSettings(retained);

  // 再 emit 一次 prefs-updated，唤醒 tool_health loop —— ofox_bind_tool 已
  // 完成、active provider 切到 ofox-<app>，health check 才能读到正确的
  // model。mirrorBoundToolsToSettings 内部也会 emit 一次（两次 emit 由订阅
  // 方节流去重）。
  void emit("ofox-prefs-updated");
  return retained;
}

/** Tools currently marked bound in this renderer (localStorage). */
export function readBoundTools(): string[] {
  const raw = localStorage.getItem(BOUND_TOOLS_STORAGE_KEY);
  return raw ? (JSON.parse(raw) as string[]) : [];
}

/**
 * Inverse of {@link bindTools} for a single tool — used by the manage dialog's
 * "解除绑定" button.
 *
 * The backend restores the tool's pre-Ofox connection setup (provider, base URL,
 * key, model) and keeps everything else (MCP, skills, plugins). `stillBound`
 * lets it keep `~/.codex` while the other of Codex / ChatGPT is still bound.
 *
 * The bound list changes only after the backend succeeds: on failure the tool
 * stays bound, the dialog stays open and the user can retry.
 */
export async function unbindTool(app: string): Promise<UnbindReport | null> {
  const stillBound = readBoundTools().filter((id) => id !== app);

  const report = OFOX_AUTO_BIND_TOOLS.has(app)
    ? await invoke<UnbindReport>("ofox_unbind_tool", { app, stillBound })
    : null;

  if (
    report?.warnings?.some((warning) => warning.code === "recordCleanupFailed")
  ) {
    throw new Error(i18n.t("unbind.warning.recordCleanupFailed"));
  }

  localStorage.setItem(BOUND_TOOLS_STORAGE_KEY, JSON.stringify(stillBound));
  void mirrorBoundToolsToSettings(stillBound);
  return report;
}
