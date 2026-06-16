//! Ofox OAuth Tauri Commands
//!
//! Provides Ofox AI OAuth authentication commands exposed to the frontend.
//! Uses Device Authorization Grant (RFC 8628).

use std::str::FromStr;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

use crate::app_config::AppType;
use crate::store::AppState;

/// Ofox Auth state wrapper for Tauri managed state.
pub struct OfoxAuthState(pub Arc<RwLock<crate::ofox_auth::OfoxAuthManager>>);

/// Start the Ofox OAuth Device Code login flow.
///
/// Returns device code info (user_code, verification_uri, etc.).
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_start_login(
    state: State<'_, OfoxAuthState>,
) -> Result<crate::ofox_auth::OfoxDeviceCodeResponse, String> {
    let manager = state.0.read().await;
    manager.start_device_flow().await
}

/// Poll for device code approval.
///
/// Returns Some(user) if approved, None if still pending.
/// Errors: "slow_down", "access_denied", "expired_token"
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_poll_for_token(
    device_code: String,
    state: State<'_, OfoxAuthState>,
) -> Result<Option<crate::ofox_auth::OfoxUserInfo>, String> {
    let manager = state.0.read().await;
    manager.poll_for_token(&device_code).await
}

/// Get Ofox user info (if authenticated).
///
/// Attempts to fetch fresh data from the API (which includes the latest
/// account balance), falling back to the in-memory cache on failure so the
/// UI never has to wait for the network to render the user.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_get_user_info(
    state: State<'_, OfoxAuthState>,
) -> Result<Option<crate::ofox_auth::OfoxUserInfo>, String> {
    let manager = state.0.read().await;
    if !manager.is_authenticated() {
        return Ok(None);
    }
    match manager.refresh_user_info().await {
        Ok(user) => Ok(Some(user)),
        Err(e) => {
            log::warn!("[OfoxAuth] refresh_user_info failed, falling back to cache: {e}");
            Ok(manager.get_user_info())
        }
    }
}

/// Force a refresh of the cached Ofox user info from the API.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_refresh_user_info(
    state: State<'_, OfoxAuthState>,
) -> Result<crate::ofox_auth::OfoxUserInfo, String> {
    let manager = state.0.read().await;
    manager.refresh_user_info().await
}

/// Get the current Ofox auth snapshot — the single source of truth for any
/// UI that needs to know whether to show the profile, an expired banner, or
/// the login page. Pair this with the `ofox-auth-expired` / `ofox-auth-restored`
/// events for live updates.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_get_auth_status(
    state: State<'_, OfoxAuthState>,
) -> Result<crate::ofox_auth::OfoxAuthStatus, String> {
    let manager = state.0.read().await;
    Ok(manager.get_auth_status().await)
}

/// Check if the user is currently authenticated with Ofox.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_is_authenticated(
    state: State<'_, OfoxAuthState>,
) -> Result<bool, String> {
    let manager = state.0.read().await;
    Ok(manager.is_authenticated())
}

/// Log out from Ofox (clears tokens and persisted state).
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_logout(
    state: State<'_, OfoxAuthState>,
) -> Result<(), String> {
    let mut manager = state.0.write().await;
    manager.logout();
    Ok(())
}

/// Show the main window and emit `ofox-reauth-requested` so the renderer
/// can swap from Console back to the onboarding LoginPage. Triggered by the
/// "重新登录" button in the tray popover.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_request_reauth(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::{Emitter, Manager};

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        log::warn!("[OfoxAuth] ofox_request_reauth: main window not found");
    }

    app.emit("ofox-reauth-requested", ())
        .map_err(|e| format!("emit reauth event failed: {e}"))?;
    Ok(())
}

/// Identifier of the OfoxAI seed provider for a given app, plus the path
/// inside `settings_config` where the OAuth access_token should be written.
///
/// The settings shape differs by app — Claude keeps tokens in `env.*`, Codex
/// in `auth.*`, etc. — so the bind command needs an app-keyed table rather
/// than a single helper. Kept inline so it stays next to the seeds it mirrors
/// (see `database/dao/providers_seed.rs::OFOX_SEEDS`).
fn ofox_provider_for(app: &AppType) -> Option<(&'static str, &'static [&'static str])> {
    match app {
        // env.ANTHROPIC_AUTH_TOKEN — matches `ofox-claude` seed at
        // providers_seed.rs:111
        AppType::Claude => Some(("ofox-claude", &["env", "ANTHROPIC_AUTH_TOKEN"])),
        // auth.OPENAI_API_KEY — matches `ofox-codex` seed at
        // providers_seed.rs:122
        AppType::Codex => Some(("ofox-codex", &["auth", "OPENAI_API_KEY"])),
        // `ofox-gemini` seed has no token field today — Gemini CLI uses a
        // Google OAuth flow rather than a shared bearer. Returning None
        // surfaces a clear error rather than silently doing nothing.
        AppType::Gemini => None,
        _ => None,
    }
}

/// Write `token` at `path` inside `settings`, creating intermediate objects
/// as needed. `path` must be non-empty. Always ends by replacing the leaf
/// value with the provided string.
fn write_token_at_path(
    settings: &mut serde_json::Value,
    path: &[&str],
    token: &str,
) -> Result<(), String> {
    if path.is_empty() {
        return Err("token path is empty".into());
    }
    if !settings.is_object() {
        *settings = serde_json::json!({});
    }
    let (last, ancestors) = path.split_last().expect("non-empty per check above");
    let mut cursor = settings;
    for key in ancestors {
        let obj = cursor
            .as_object_mut()
            .ok_or_else(|| format!("settings_config 路径 {key} 处不是对象"))?;
        // Insert an empty object if the key is missing or holds a non-object
        // (e.g. accidentally serialized as a string by a hand edit).
        if !obj.get(*key).map(|v| v.is_object()).unwrap_or(false) {
            obj.insert((*key).to_string(), serde_json::json!({}));
        }
        cursor = obj.get_mut(*key).expect("just inserted");
    }
    let leaf = cursor
        .as_object_mut()
        .ok_or_else(|| format!("settings_config 路径 {last} 的父级不是对象"))?;
    leaf.insert((*last).to_string(), serde_json::json!(token));
    Ok(())
}

/// Internal implementation of "bind this tool to OfoxAI", reusable from
/// non-Tauri-command contexts (e.g. the startup self-heal path in `lib.rs`).
///
/// The Tauri command wrapper [`ofox_bind_tool`] just forwards `State<...>`
/// references into this function — keep them in sync.
pub async fn bind_tool_to_ofox_internal(
    db: &crate::database::Database,
    proxy_service: &crate::services::proxy::ProxyService,
    ofox_manager: &Arc<RwLock<crate::ofox_auth::OfoxAuthManager>>,
    app: &str,
) -> Result<(), String> {
    let app_type = AppType::from_str(app).map_err(|e| format!("无效的应用类型: {e}"))?;
    let (provider_id, token_path) = ofox_provider_for(&app_type)
        .ok_or_else(|| format!("{} 暂不支持自动绑定到 OfoxAI", app_type.as_str()))?;

    // Pull a fresh token first — if auth is broken, fail before mutating any
    // DB state so the user can re-login without leaving a half-bound entry.
    let token = {
        let manager = ofox_manager.read().await;
        manager
            .get_valid_access_token()
            .await
            .map_err(|e| format!("获取 OfoxAI 访问令牌失败: {e}"))?
    };

    // Read the seed provider, mutate, write back.
    let mut provider = db
        .get_provider_by_id(provider_id, app_type.as_str())
        .map_err(|e| format!("读取 {provider_id} 失败: {e}"))?
        .ok_or_else(|| format!("供应商 {provider_id} 不存在（应由启动种子建立）"))?;

    write_token_at_path(&mut provider.settings_config, token_path, &token)?;

    db.update_provider_settings_config(
        app_type.as_str(),
        provider_id,
        &provider.settings_config,
    )
    .map_err(|e| format!("写入 {provider_id} settings_config 失败: {e}"))?;

    // Switch active provider BEFORE flipping takeover so the takeover sync
    // path sees the right current provider. `settings::set_current_provider`
    // is the persisted source of truth (see `get_effective_current_provider`).
    crate::settings::set_current_provider(&app_type, Some(provider_id))
        .map_err(|e| format!("设置 {} 当前供应商失败: {e}", app_type.as_str()))?;
    // Also mirror to DB's is_current column so anything reading `is_current`
    // (e.g. tray menu, provider list ordering) stays in sync.
    db.set_current_provider(app_type.as_str(), provider_id)
        .map_err(|e| format!("更新 {} 数据库 is_current 失败: {e}", app_type.as_str()))?;

    // Finally, ensure proxy takeover is on for this app. Idempotent.
    proxy_service
        .set_takeover_for_app(app_type.as_str(), true)
        .await?;

    Ok(())
}

/// Bind an AI tool (claude/codex/...) to OfoxAI.
///
/// The user-visible effect: after this returns, the tool's requests are
/// intercepted by cc-switch's local proxy and forwarded to OfoxAI's gateway
/// (`https://api.ofox.ai/...`) with the user's OAuth access_token attached.
///
/// Concretely:
///   1. Look up the matching `ofox-<app>` seed provider in the DB.
///   2. Pull a fresh, valid access_token from OfoxAuth (refreshes if needed).
///   3. Write the token into that provider's `settings_config` at the right
///      path for the app (env.ANTHROPIC_AUTH_TOKEN, auth.OPENAI_API_KEY, …).
///   4. Persist + set it as the current provider for the app.
///   5. Enable proxy takeover for the app (idempotent on the Rust side).
///
/// This is the missing step that the previous in-Console "+ 添加" path lacked:
/// it would toggle takeover ON, but leave the current provider pointing at
/// `claude-official` (settings_config `{"env":{}}`), which has no base_url
/// and produces "Claude Provider 缺少 base_url 配置" at request time.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_bind_tool(
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
    app: String,
) -> Result<(), String> {
    bind_tool_to_ofox_internal(
        &state.db,
        &state.proxy_service,
        &ofox_state.0,
        &app,
    )
    .await
}

/// Mirror of [`bind_tool_to_ofox_internal`] — undo the OfoxAI takeover for
/// `app` and restore the official provider as active.
///
/// Steps (the inverse order of bind, so we never leave a window where the
/// live config still has Ofox endpoints but a non-Ofox active provider):
///   1. Disable proxy takeover for the app — restores the user's previous
///      live config from backup, stops the proxy if no other tool needs it.
///   2. Switch the active provider to `<app>-official` for the three apps
///      that ship an official seed (claude/codex/gemini). For tools without
///      an official seed (opencode/openclaw/hermes), step 1 alone is the
///      meaningful action — they have no built-in fallback to switch to.
///
/// We deliberately leave the OfoxAI seed's `settings_config` (and any token
/// previously written into it) alone. Re-binding later will overwrite the
/// token with a fresh one, and keeping the old value avoids a second token
/// request on the unbind path.
///
/// Note: the front-end is responsible for updating its own `ofox-bound-tools`
/// localStorage list — that store is purely UI state and the backend has no
/// view into it.
pub async fn unbind_tool_from_ofox_internal(
    db: &crate::database::Database,
    proxy_service: &crate::services::proxy::ProxyService,
    app: &str,
) -> Result<(), String> {
    let app_type = AppType::from_str(app).map_err(|e| format!("无效的应用类型: {e}"))?;
    let app_str = app_type.as_str();

    // 1) Turn off takeover first. This restores the live config from backup
    //    so that, even if step 2 fails, the user's tool is no longer pointed
    //    at api.ofox.ai with a stale token.
    proxy_service
        .set_takeover_for_app(app_str, false)
        .await
        .map_err(|e| format!("关闭 {app_str} 代理接管失败: {e}"))?;

    // 2) Switch active provider back to the official seed, if one exists.
    //    The id convention `<app>-official` matches `OFFICIAL_SEEDS` in
    //    `database/dao/providers_seed.rs`; only claude/codex/gemini have
    //    official seeds today.
    let official_id: Option<&str> = match app_type {
        AppType::Claude => Some("claude-official"),
        AppType::Codex => Some("codex-official"),
        AppType::Gemini => Some("gemini-official"),
        AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => None,
    };

    if let Some(id) = official_id {
        // Persist via the same two-tier write that bind uses, so anything
        // reading either source-of-truth (settings.json file or DB) sees
        // the change consistently.
        crate::settings::set_current_provider(&app_type, Some(id))
            .map_err(|e| format!("设置 {app_str} 当前供应商失败: {e}"))?;
        db.set_current_provider(app_str, id)
            .map_err(|e| format!("更新 {app_str} 数据库 is_current 失败: {e}"))?;
    }

    Ok(())
}

/// Tauri command wrapper — see [`unbind_tool_from_ofox_internal`].
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_unbind_tool(
    state: State<'_, AppState>,
    app: String,
) -> Result<(), String> {
    unbind_tool_from_ofox_internal(&state.db, &state.proxy_service, &app).await
}
