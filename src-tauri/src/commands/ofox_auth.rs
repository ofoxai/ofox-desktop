//! Ofox OAuth Tauri Commands
//!
//! Provides Ofox AI OAuth authentication commands exposed to the frontend.
//! Uses Device Authorization Grant (RFC 8628).

use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

use crate::services::balance_history::{self, BalanceTrend};
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
/// UI never has to wait for the network to render the user. A fresh balance
/// is also kept as a point of the balance history.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_get_user_info(
    state: State<'_, OfoxAuthState>,
    app_state: State<'_, AppState>,
) -> Result<Option<crate::ofox_auth::OfoxUserInfo>, String> {
    let manager = state.0.read().await;
    if !manager.is_authenticated() {
        return Ok(None);
    }
    match manager.refresh_user_info().await {
        Ok(user) => {
            balance_history::record(&app_state.db, &user, now_ms());
            Ok(Some(user))
        }
        Err(e) => {
            log::warn!("[OfoxAuth] refresh_user_info failed, falling back to cache: {e}");
            Ok(manager.get_user_info())
        }
    }
}

/// Force a refresh of the cached Ofox user info from the API, and keep the
/// fresh balance as a point of the balance history.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_refresh_user_info(
    state: State<'_, OfoxAuthState>,
    app_state: State<'_, AppState>,
) -> Result<crate::ofox_auth::OfoxUserInfo, String> {
    let manager = state.0.read().await;
    let user = manager.refresh_user_info().await?;
    balance_history::record(&app_state.db, &user, now_ms());
    Ok(user)
}

/// The balance history of the signed-in account's org and what it says: the
/// points of the last 14 days, the burn rate, and when the balance runs out.
/// Reads what the refreshes kept; it does not call OFox. Empty when signed out.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_get_balance_trend(
    state: State<'_, OfoxAuthState>,
    app_state: State<'_, AppState>,
) -> Result<BalanceTrend, String> {
    let owner = state.0.read().await.current_key_owner().await;
    let Some(org_id) = owner.and_then(|owner| owner.org_id) else {
        return Ok(BalanceTrend::default());
    };
    balance_history::trend_for(&app_state.db, &org_id, now_ms()).map_err(|e| e.to_string())
}

/// The org's monthly spending limit (`forbidden` for a member role), for the
/// usage page. `None` when signed out or when no monthly limit is configured.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_get_spending(
    state: State<'_, OfoxAuthState>,
) -> Result<Option<crate::ofox_auth::OfoxSpending>, String> {
    let manager = state.0.read().await;
    if !manager.is_authenticated() {
        return Ok(None);
    }
    manager.get_spending().await
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
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

/// 用户点「重新授权」：再读一次钥匙串里的登录信息（系统会再次询问），
/// 允许后不必重新登录。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_retry_keychain(
    state: State<'_, OfoxAuthState>,
) -> Result<crate::ofox_auth::OfoxAuthStatus, String> {
    let manager = state.0.read().await;
    Ok(manager.retry_keychain().await)
}

/// Check if the user is currently authenticated with Ofox.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_is_authenticated(state: State<'_, OfoxAuthState>) -> Result<bool, String> {
    let manager = state.0.read().await;
    Ok(manager.is_authenticated())
}

/// Log out from Ofox (clears tokens and persisted state).
///
/// 先趁登录还有效，把本账号在这台电脑上用的 API key 停掉（服务器上吊销），
/// 工具保持绑定但暂停使用；登录后由 [`ofox_reconcile_tool_keys`] 换上当前账号
/// 的新 key。切换地区也会退出登录，但走的是 `manager.logout()`，不停 key。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_logout(
    app_handle: tauri::AppHandle,
    state: State<'_, OfoxAuthState>,
) -> Result<crate::services::account_keys::LogoutReport, String> {
    let report = crate::services::account_keys::stop_for_logout(&state.0).await;
    state.0.write().await.logout();
    crate::commands::model_selection::notify(&app_handle, report.stopped.clone());
    Ok(report)
}

/// 登录后、进入主页时调用：已绑定的工具如果用的不是当前账号的 key（换了账号，
/// 或退出时停用了），换成当前账号新签发的 key。没有要换的时不访问网络。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_reconcile_tool_keys(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
) -> Result<crate::services::account_keys::KeyReconcileReport, String> {
    let report = crate::services::account_keys::reconcile(&state.db, &ofox_state.0).await;
    crate::commands::model_selection::notify(&app_handle, report.switched.clone());
    Ok(report)
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
/// 取要写进工具配置的 key。失败原因里可能有服务端回显的 key，抹掉再往外传。
async fn acquire_bind_token(
    key_tool: crate::app_config::BindableTool,
    ofox_manager: &Arc<RwLock<crate::ofox_auth::OfoxAuthManager>>,
) -> Result<String, String> {
    fetch_bind_token(key_tool, ofox_manager)
        .await
        .map_err(crate::redact::scrub_error)
}

async fn fetch_bind_token(
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
    if let Some(tool) = crate::app_config::BindableTool::native(app) {
        let access = ofox_manager
            .read()
            .await
            .get_valid_access_token()
            .await
            .ok();
        let catalog =
            crate::services::model_fetch::fetch_ofox_catalog("openai", access.as_deref(), false)
                .await?;
        let model = catalog
            .models
            .iter()
            .filter(|m| crate::native_tools::model_api(tool, m).is_ok())
            .filter(|m| {
                m.pricing_prompt
                    .as_deref()
                    .and_then(|p| p.parse::<f64>().ok())
                    .is_some_and(|p| p > 0.0)
            })
            .min_by(|a, b| {
                a.pricing_prompt
                    .as_deref()
                    .and_then(|p| p.parse::<f64>().ok())
                    .unwrap_or(f64::MAX)
                    .total_cmp(
                        &b.pricing_prompt
                            .as_deref()
                            .and_then(|p| p.parse::<f64>().ok())
                            .unwrap_or(f64::MAX),
                    )
            })
            .or_else(|| {
                catalog
                    .models
                    .iter()
                    .find(|m| crate::native_tools::model_api(tool, m).is_ok())
            })
            .ok_or_else(|| "当前目录没有 Pi/dsh 可用的模型".to_string())?;
        crate::native_tools::validate_bind(tool, model, &catalog.models)?;
        let key = acquire_bind_token(tool, ofox_manager).await?;
        crate::native_tools::bind_with_catalog(db, tool, &key, model, &catalog.models).await?;
        crate::ofox_api_keys::mark_key_used(tool);
        return Ok(());
    }
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

/// Startup compatibility repair must not create configuration that the user
/// deleted, or claim a connection that they changed to another provider.
pub(crate) async fn bind_existing_tool_to_ofox_internal(
    db: &crate::database::Database,
    ofox_manager: &Arc<RwLock<crate::ofox_auth::OfoxAuthManager>>,
    app: &str,
) -> Result<(), String> {
    if let Some(tool) = crate::app_config::BindableTool::native(app) {
        let status = crate::native_tools::binding_status(db, tool).await;
        if status.status != crate::services::ofox_bind::status::BindingStatus::Configured {
            return Err("原生工具配置已修改或缺失，未自动修复".into());
        }
        return Ok(());
    }
    let (tool, holder) =
        crate::services::ofox_bind::tool_for(app).ok_or_else(|| "无效的应用类型".to_string())?;
    let token = acquire_bind_token(tool.app().into(), ofox_manager).await?;
    crate::services::ofox_bind::bind_existing(db, tool, holder, &token).await?;
    crate::ofox_api_keys::mark_key_used(tool.app());
    Ok(())
}

/// Bind an AI tool (claude/codex/...) to OfoxAI.
///
/// 取（或签发）该工具的 Ofox API key，把工具配置文件里的接入方式改成 Ofox 网关，
/// 并把当前服务商切到 `ofox-<app>`。第一次绑定时记下绑定前的样子，解绑时精确还原。
/// 详见 [`bind_tool_to_ofox_internal`]。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_bind_tool(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
    app: String,
    model_selection: Option<crate::workbuddy_config::WorkBuddyModelSelection>,
    model_selections: Option<Vec<crate::workbuddy_config::WorkBuddyModelSelection>>,
) -> Result<(), String> {
    let settings = crate::settings::get_settings();
    if native_bind_needs_apex_probe(&app, &settings) {
        // New native configurations must use the completed region decision.
        // The existing probe retains the documented fallback when offline and
        // reconciles managed URLs on the next successful startup detection.
        let client = crate::proxy::http_client::get();
        crate::ofox_apex::probe_apex_on_startup(&app_handle, &client).await;
    }
    // `modelSelection` remains accepted for compatibility with older renderer
    // builds while the multi-model UI sends `modelSelections`.
    let selections = model_selections.or_else(|| model_selection.map(|selection| vec![selection]));
    bind_tool_to_ofox_internal(&state.db, &ofox_state.0, &app, selections)
        .await
        .map_err(crate::redact::scrub_error)?;
    crate::commands::model_selection::notify(
        &app_handle,
        vec![crate::model_selection::canonical_tool(&app)?],
    );
    Ok(())
}

fn native_bind_needs_apex_probe(app: &str, settings: &crate::settings::AppSettings) -> bool {
    crate::app_config::BindableTool::native(app).is_some()
        && !settings.ofox_apex_resolved.unwrap_or(false)
        && !settings.ofox_apex_pinned.unwrap_or(false)
}

#[cfg(test)]
mod native_bind_apex_tests {
    use super::native_bind_needs_apex_probe;

    #[test]
    fn native_first_bind_waits_for_auto_region_but_respects_resolved_or_pinned_apex() {
        for tool in ["pi", "dsh"] {
            let mut settings = crate::settings::AppSettings::default();
            assert!(native_bind_needs_apex_probe(tool, &settings));
            for apex in ["ofox.ai", "ofox.io"] {
                settings.ofox_apex = Some(apex.into());
                settings.ofox_apex_resolved = Some(false);
                settings.ofox_apex_pinned = Some(false);
                assert!(native_bind_needs_apex_probe(tool, &settings));
                settings.ofox_apex_pinned = Some(true);
                assert!(!native_bind_needs_apex_probe(tool, &settings));
                settings.ofox_apex_pinned = Some(false);
                settings.ofox_apex_resolved = Some(true);
                assert!(!native_bind_needs_apex_probe(tool, &settings));
            }
        }
        assert!(!native_bind_needs_apex_probe(
            "codex",
            &crate::settings::AppSettings::default(),
        ));
    }
}

/// Explicit recovery of deleted connection fields. Re-check after acquiring the
/// key so concurrent model edits, unbinds, and external configuration changes
/// cannot turn recovery into an overwrite.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_restore_tool_binding(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
    app: String,
    still_bound: Option<Vec<String>>,
) -> Result<(), String> {
    use crate::services::ofox_bind::status::BindingStatus;
    if let Some(tool) = crate::app_config::BindableTool::native(&app) {
        if crate::native_tools::binding_status(&state.db, tool)
            .await
            .status
            != BindingStatus::Missing
        {
            return Err("配置未缺失或已修改，无需恢复".into());
        }
        let key = acquire_bind_token(tool, &ofox_state.0).await?;
        crate::native_tools::restore_missing(&state.db, tool, &key)
            .await
            .map_err(crate::redact::scrub_error)?;
        crate::commands::model_selection::notify(&app_handle, vec![tool.as_str().into()]);
        return Ok(());
    }

    if app.trim().eq_ignore_ascii_case("workbuddy") {
        let health = crate::workbuddy_config::binding_status(&state.db).await;
        if health.status != BindingStatus::Missing {
            return Err(health
                .message
                .unwrap_or_else(|| "配置未缺失，无需恢复。".into()));
        }
        let key =
            acquire_bind_token(crate::app_config::BindableTool::WorkBuddy, &ofox_state.0).await?;
        crate::workbuddy_config::restore_missing_binding(&state.db, &key)
            .await
            .map_err(crate::redact::scrub_error)?;
        crate::commands::model_selection::notify(&app_handle, vec!["workbuddy".into()]);
        return Ok(());
    }
    let (tool, holder) =
        crate::services::ofox_bind::tool_for(&app).ok_or_else(|| "无效的应用类型".to_string())?;
    let cached_key = crate::ofox_secret::default_store()
        .load(crate::ofox_secret::Slot::ApiKey {
            tool: tool.app().into(),
        })
        .map_err(|_| "读取已保存的 OFox API Key 失败，已停止恢复。".to_string())?;
    let health =
        crate::services::ofox_bind::status::binding_status(&state.db, tool, cached_key.as_deref())
            .await;
    if health.status != BindingStatus::Missing {
        return Err(health
            .message
            .unwrap_or_else(|| "配置未缺失，无需恢复。".into()));
    }
    let key = acquire_bind_token(tool.app().into(), &ofox_state.0).await?;
    crate::services::ofox_bind::restore_missing_binding(
        &state.db,
        tool,
        holder,
        &still_bound.unwrap_or_default(),
        &key,
    )
    .await
    .map_err(crate::redact::scrub_error)?;
    crate::commands::model_selection::notify(&app_handle, vec![tool.app().as_str().into()]);
    Ok(())
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
    if let Some(tool) = crate::app_config::BindableTool::native(app) {
        return crate::native_tools::unbind(db, tool, dry_run).await;
    }
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
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    app: String,
    still_bound: Option<Vec<String>>,
) -> Result<UnbindReport, String> {
    let still_bound = still_bound.unwrap_or_default();
    let report = unbind_tool_from_ofox_internal(&state.db, &app, &still_bound, false)
        .await
        .map_err(crate::redact::scrub_error)?;
    crate::commands::model_selection::notify(
        &app_handle,
        vec![crate::model_selection::canonical_tool(&app)?],
    );
    Ok(report)
}

/// 解绑预览：返回解绑会还原/删除哪些内容，不做任何改动。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_unbind_preview(
    state: State<'_, AppState>,
    app: String,
    still_bound: Option<Vec<String>>,
) -> Result<UnbindReport, String> {
    let still_bound = still_bound.unwrap_or_default();
    unbind_tool_from_ofox_internal(&state.db, &app, &still_bound, true)
        .await
        .map_err(crate::redact::scrub_error)
}
