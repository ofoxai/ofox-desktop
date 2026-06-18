import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import {
  BOUND_TOOLS_STORAGE_KEY,
  PROXY_SUPPORTED_TOOLS,
} from "@/config/toolMeta";
import { proxyApi } from "@/lib/api/proxy";
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
 * Tools the backend can fully bind to OfoxAI in one call (sets the
 * ofox-* provider as current AND injects the OAuth access_token AND turns
 * on takeover). Gemini is intentionally out — its OfoxAI seed has no
 * token field today (Gemini CLI uses its own Google OAuth flow), so we
 * fall back to just enabling takeover.
 *
 * Keep this in sync with `commands/ofox_auth.rs::ofox_provider_for`.
 */
const OFOX_AUTO_BIND_TOOLS: ReadonlySet<string> = new Set(["claude", "codex"]);

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
 * Per-tool behavior:
 *   - claude / codex → `ofox_bind_tool` does everything: switches active
 *     provider to ofox-<tool>, writes the OAuth access_token into the
 *     provider's settings_config, enables proxy takeover.
 *   - gemini → only `setProxyTakeoverForApp` (no token field yet).
 *   - others (opencode/...) → not in PROXY_SUPPORTED_TOOLS, no-op.
 *
 * Both Rust paths are idempotent — re-binding an already-bound tool is
 * cheap.
 *
 * Returns the same `tools` array it was given so callers can chain.
 */
export async function bindTools(tools: string[]): Promise<string[]> {
  localStorage.setItem(BOUND_TOOLS_STORAGE_KEY, JSON.stringify(tools));
  // Mirror to backend (fire-and-forget, doesn't block bind round-trips).
  void mirrorBoundToolsToSettings(tools);
  for (const tool of tools) {
    if (!PROXY_SUPPORTED_TOOLS.includes(tool)) continue;
    try {
      if (OFOX_AUTO_BIND_TOOLS.has(tool)) {
        await invoke("ofox_bind_tool", { app: tool });
      } else {
        await proxyApi.setProxyTakeoverForApp(tool, true);
      }
    } catch (e) {
      console.error(`[bindTools] bind ${tool} failed`, e);
    }
  }
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

  if (PROXY_SUPPORTED_TOOLS.includes(app)) {
    await invoke("ofox_unbind_tool", { app });
  }
}
