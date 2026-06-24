import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { BOUND_TOOLS_STORAGE_KEY } from "@/config/toolMeta";
import { settingsApi } from "@/lib/api";

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
    const cur = await settingsApi.get();
    await settingsApi.save({ ...cur, boundTools: tools });
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
 * Keep this in sync with `commands/ofox_auth.rs::ofox_provider_for`.
 */
const OFOX_AUTO_BIND_TOOLS: ReadonlySet<string> = new Set([
  "claude",
  "codex",
  "gemini",
  "opencode",
  "openclaw",
  "hermes",
]);

/**
 * Persist the bound-tool set and wire each tool through to OfoxAI.
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
 * `ofox_bind_tool` 后端幂等——重复 bind 已绑工具开销可忽略。
 *
 * Returns the same `tools` array it was given so callers can chain.
 */
export async function bindTools(tools: string[]): Promise<string[]> {
  localStorage.setItem(BOUND_TOOLS_STORAGE_KEY, JSON.stringify(tools));
  // Mirror to backend (fire-and-forget, doesn't block bind round-trips).
  void mirrorBoundToolsToSettings(tools);
  for (const tool of tools) {
    if (!OFOX_AUTO_BIND_TOOLS.has(tool)) continue;
    try {
      await invoke("ofox_bind_tool", { app: tool });
    } catch (e) {
      console.error(`[bindTools] bind ${tool} failed`, e);
    }
  }
  // 再 emit 一次 prefs-updated，唤醒 tool_health loop —— 此时 ofox_bind_tool
  // 已完成、active provider 切到 ofox-<app>，health check 才能读到正确的
  // model。第一次 emit 在 mirrorBoundToolsToSettings 里（与 bind 并行抢跑），
  // 那次 read 可能仍指向 official seed (models=[]) → "未配置模型"。这次补
  // emit 是为了让 UI 收到的最终 pill 反映 bind 后的真实状态。
  void emit("ofox-prefs-updated");
  return tools;
}

/**
 * Inverse of {@link bindTools} for a single tool — used by the manage dialog's
 * "解除绑定" button.
 *
 * Order matters and mirrors the bind path inverted:
 *   1. Update the persisted list FIRST so the locked-on reconciliation in
 *      MainApp.tsx (which treats `boundTools ∩ PROXY_SUPPORTED` as locked-on)
 *      doesn't race against the backend turning takeover off and immediately
 *      flip it back on. With the localStorage write done first, by the time
 *      any reconciliation runs the tool is no longer in the bound set.
 *   2. Tell the backend to (a) flip takeover off — restoring the user's
 *      pre-cc-switch live config from backup, and (b) switch the active
 *      provider back to `<app>-official` for tools that have an official
 *      seed. See `unbind_tool_from_ofox_internal` for details.
 *
 * Throws if the backend call fails — caller (the dialog) shows a toast and
 * keeps itself open. localStorage has already been updated by then; that's
 * okay because the next `bindTools(...)` invocation (e.g. user re-adding the
 * tool from "+ 添加") rewrites it.
 */
export async function unbindTool(app: string): Promise<void> {
  const raw = localStorage.getItem(BOUND_TOOLS_STORAGE_KEY);
  const list: string[] = raw ? JSON.parse(raw) : [];
  const next = list.filter((id) => id !== app);
  localStorage.setItem(BOUND_TOOLS_STORAGE_KEY, JSON.stringify(next));
  void mirrorBoundToolsToSettings(next);

  if (OFOX_AUTO_BIND_TOOLS.has(app)) {
    await invoke("ofox_unbind_tool", { app });
  }
}
