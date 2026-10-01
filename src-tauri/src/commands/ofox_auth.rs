//! Ofox OAuth Tauri Commands
//!
//! Provides Ofox AI OAuth authentication commands exposed to the frontend.
//! Uses Device Authorization Grant (RFC 8628).

use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

use crate::services::ofox_bind::report::UnbindReport;
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
pub async fn ofox_is_authenticated(state: State<'_, OfoxAuthState>) -> Result<bool, String> {
    let manager = state.0.read().await;
    Ok(manager.is_authenticated())
}

/// Log out from Ofox (clears tokens and persisted state).
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_logout(state: State<'_, OfoxAuthState>) -> Result<(), String> {
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

/// 取要写到工具配置里的 LLM 凭据（keychain 命中直接用，否则向 Ofox 签发）。
async fn acquire_bind_token(
    key_tool: crate::app_config::BindableTool,
    ofox_manager: &Arc<RwLock<crate::ofox_auth::OfoxAuthManager>>,
) -> Result<String, String> {
    // 1) 取要写到工具配置里的 LLM 凭据。
    //
    // **主路径**：调 `ofox_api_keys::fetch_or_create_api_key(_, CachedOk, _)`
    // ——keychain 命中直接拿；未命中调 `POST /openapi/api-keys` 签发新 key，
    // 落 keychain + settings 元数据后返回 key 明文。这是端点 GA 后的稳定路径。
    //
    // **逃生口**：`OFOX_USE_OAUTH_TOKEN_AS_KEY=1` 时退回老行为，把 OAuth
    // access_token 当 LLM key 用。仅供端点切流期临时验证，**下一个 commit 删**
    // ——access_token ~1h 过期会让工具 401，不是稳态。
    let token = if std::env::var("OFOX_USE_OAUTH_TOKEN_AS_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        log::warn!(
            "[ofox_bind] OFOX_USE_OAUTH_TOKEN_AS_KEY set — using OAuth access_token as LLM key \
             for {}. Transitional escape hatch—remove after /openapi/api-keys verified in prod.",
            key_tool.as_str()
        );
        let manager = ofox_manager.read().await;
        manager
            .get_valid_access_token()
            .await
            .map_err(|e| format!("获取 OfoxAI 访问令牌失败: {e}"))?
    } else {
        crate::ofox_api_keys::fetch_or_create_api_key(
            key_tool,
            crate::ofox_api_keys::FetchMode::CachedOk,
            ofox_manager,
        )
        .await
        .map_err(|e| match e {
            crate::ofox_api_keys::ApiKeyError::Unauthorized(msg) => {
                format!("OFox 授权失效：{msg}。请退出登录后重新走 device flow。")
            }
            crate::ofox_api_keys::ApiKeyError::RemoteRejected(msg) => {
                format!("OFox API key 服务端拒绝：{msg}")
            }
            crate::ofox_api_keys::ApiKeyError::Storage(msg) => {
                format!("本地 keychain / settings 读写失败：{msg}")
            }
        })?
    };
    Ok(token)
}

/// Internal implementation of "bind this tool to OfoxAI", reusable from
/// non-Tauri-command contexts (e.g. the startup self-heal path in `lib.rs`).
///
/// The Tauri command wrapper [`ofox_bind_tool`] just forwards `State<...>`
/// references into this function — keep them in sync.
pub async fn bind_tool_to_ofox_internal(
    db: &crate::database::Database,
    ofox_manager: &Arc<RwLock<crate::ofox_auth::OfoxAuthManager>>,
    app: &str,
    model_selections: Option<Vec<crate::workbuddy_config::WorkBuddyModelSelection>>,
) -> Result<(), String> {
    // 直接改配置文件的工具：第一次绑定时记下受管文件原样，只改接入字段，解绑时
    // 精确还原（见 services::ofox_bind）。Codex 和 ChatGPT 共用 ~/.codex。
    if let Some((tool, holder)) = crate::services::ofox_bind::tool_for(app) {
        let token = acquire_bind_token(tool.app().into(), ofox_manager).await?;
        crate::services::ofox_bind::bind(db, tool, holder, &token).await?;
        // 只有走 fetch_or_create_api_key 才有元数据；没有时内部直接 noop。
        crate::ofox_api_keys::mark_key_used(tool.app());
        return Ok(());
    }
    if !app.trim().eq_ignore_ascii_case("workbuddy") {
        return Err(format!("无效的应用类型: {}", app.trim()));
    }
    let selections =
        model_selections.ok_or_else(|| "绑定 WorkBuddy 前必须选择兼容模型".to_string())?;
    let key_tool = crate::app_config::BindableTool::WorkBuddy;
    let token = acquire_bind_token(key_tool, ofox_manager).await?;
    crate::workbuddy_config::sync_selected_models(db, &token, &selections).await?;
    crate::ofox_api_keys::mark_key_used(key_tool);
    Ok(())
}

/// Bind an AI tool (claude/codex/...) to OfoxAI.
///
/// 取（或签发）该工具的 Ofox API key，把工具配置文件里的接入方式改成 Ofox 网关，
/// 并把当前服务商切到 `ofox-<app>`。第一次绑定时记下绑定前的样子，解绑时精确还原。
/// 详见 [`bind_tool_to_ofox_internal`]。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_bind_tool(
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
    app: String,
    model_selection: Option<crate::workbuddy_config::WorkBuddyModelSelection>,
    model_selections: Option<Vec<crate::workbuddy_config::WorkBuddyModelSelection>>,
) -> Result<(), String> {
    // `modelSelection` remains accepted for compatibility with older renderer
    // builds while the multi-model UI sends `modelSelections`.
    let selections = model_selections.or_else(|| model_selection.map(|selection| vec![selection]));
    bind_tool_to_ofox_internal(&state.db, &ofox_state.0, &app, selections).await
}

/// Mirror of [`bind_tool_to_ofox_internal`] — undo the OfoxAI bind for `app`.
///
/// - 直接改配置文件的工具：交给 `services::ofox_bind::unbind`，按绑定前快照把
///   接入方式（地址、key、登录方式、模型）和当前服务商精确还原。
/// - WorkBuddy：`workbuddy_config::unbind`。
///
/// 我们**故意保留**：
///   - keychain 里的 `sk-of-...`——用户下次再 bind 直接命中、不重新调端点
///   - settings.json 里的 `ofoxApiKeys` 元数据——同上
///   - ofox-* provider 的 DB 行——是 seed，从来不被 bind 流程污染
///
/// 也**不**调 `revoke_remote`——服务端的 key 不主动撤销，留给用户在 ofox
/// console 管理（避免"我不小心点了 unbind 就把我手工配过的 ofox-cli 也搞挂了"）。
///
/// Note: 前端管自己的 `ofox-bound-tools` localStorage——后端不维护那个。
pub async fn unbind_tool_from_ofox_internal(
    db: &crate::database::Database,
    app: &str,
    still_bound: &[String],
    dry_run: bool,
) -> Result<UnbindReport, String> {
    if let Some((tool, holder)) = crate::services::ofox_bind::tool_for(app) {
        return crate::services::ofox_bind::unbind(db, tool, holder, still_bound, dry_run).await;
    }
    if !app.trim().eq_ignore_ascii_case("workbuddy") {
        return Err(format!("无效的应用类型: {}", app.trim()));
    }
    crate::workbuddy_config::unbind(db, dry_run).await
}

/// Tauri command wrapper — see [`unbind_tool_from_ofox_internal`].
///
/// `still_bound`：前端认为仍然绑定的其它工具。Codex 和 ChatGPT 共用 ~/.codex，
/// 另一方还绑定着时只解除这一方，不还原配置。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_unbind_tool(
    state: State<'_, AppState>,
    app: String,
    still_bound: Option<Vec<String>>,
) -> Result<UnbindReport, String> {
    let still_bound = still_bound.unwrap_or_default();
    unbind_tool_from_ofox_internal(&state.db, &app, &still_bound, false).await
}

/// 解绑预览：返回解绑会还原/删除哪些内容，不做任何改动。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_unbind_preview(
    state: State<'_, AppState>,
    app: String,
    still_bound: Option<Vec<String>>,
) -> Result<UnbindReport, String> {
    let still_bound = still_bound.unwrap_or_default();
    unbind_tool_from_ofox_internal(&state.db, &app, &still_bound, true).await
}
