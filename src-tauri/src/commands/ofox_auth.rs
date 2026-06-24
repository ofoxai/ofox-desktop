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
/// inside `settings_config` where the LLM API key should be written.
///
/// The settings shape differs by app — Claude keeps tokens in `env.*`, Codex
/// in `auth.*`, OpenCode under `options.apiKey`, OpenClaw at top-level
/// `apiKey`, Hermes at top-level `api_key` — so the bind command needs an
/// app-keyed table rather than a single helper. Kept inline so it stays next
/// to the seeds it mirrors (see `database/dao/providers_seed.rs::OFOX_SEEDS`).
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
        // options.apiKey — matches `ofox-opencode` seed at
        // providers_seed.rs:278-280
        AppType::OpenCode => Some(("ofox-opencode", &["options", "apiKey"])),
        // top-level apiKey — matches `ofox-openclaw` seed at
        // providers_seed.rs:291-293
        AppType::OpenClaw => Some(("ofox-openclaw", &["apiKey"])),
        // top-level api_key — matches `ofox-hermes` seed at
        // providers_seed.rs:304-306
        AppType::Hermes => Some(("ofox-hermes", &["api_key"])),
    }
}

// 注：`write_token_at_path` 在 bind 直写改造（commit 4）后被
// `ProxyService::write_token_into_settings` 取代——后者直接服务于"读 seed
// 模板 + 注入 token + 写盘"路径，DB provider 行不再被 bind 注入污染。

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
    // 只校验"该工具是否在 ofox 路径里有 token 注入字段"——具体路径由
    // `ProxyService::ofox_write_direct_to_live` 内部处理。Gemini 走的是
    // Google OAuth flow，没有 LLM auth token 字段，目前 ofox_provider_for
    // 对 Gemini 返 None，bind 在此提前拒绝。
    let provider_id = ofox_provider_for(&app_type)
        .map(|t| t.0)
        .ok_or_else(|| format!("{} 暂不支持自动绑定到 OfoxAI", app_type.as_str()))?;

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
            app_type.as_str()
        );
        let manager = ofox_manager.read().await;
        manager
            .get_valid_access_token()
            .await
            .map_err(|e| format!("获取 OfoxAI 访问令牌失败: {e}"))?
    } else {
        crate::ofox_api_keys::fetch_or_create_api_key(
            app_type,
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

    // 2) 备份工具原配置（DB live_backups）。失败立刻中止——没备份就 bind 会
    //    让 unbind 找不到东西恢复。Claude/Codex/Gemini 备份整文件；
    //    OpenCode/OpenClaw/Hermes 备份 "ofox-* provider 子节快照"（bind 前不
    //    存在就存哨兵）。详见 services/proxy.rs ofox_backup_live_config 注释。
    proxy_service
        .ofox_backup_live_config(&app_type)
        .await
        .map_err(|e| format!("备份 {} Live 配置失败: {e}", app_type.as_str()))?;

    // 3) 直接写工具真实配置文件——baseURL=ofox gateway、Token=真实 sk-of- 明文
    //    （绕开 takeover 的 PROXY_MANAGED 占位语义；proxy server 不为 ofox 启动）。
    //    内部读 ofox-<app> seed 模板 + 注入 token + 写盘。
    if let Err(e) = proxy_service
        .ofox_write_direct_to_live(&app_type, &token)
        .await
    {
        // 写盘失败时尽力恢复备份，免得磁盘卡在"半改"状态。恢复也失败就把
        // 两端的错都报给用户。
        let restore_hint = match proxy_service.ofox_restore_from_backup(&app_type).await {
            Ok(()) => "（已自动从备份恢复）",
            Err(re) => {
                log::error!(
                    "[ofox_bind] {} write failed AND restore failed: {re}",
                    app_type.as_str()
                );
                "（恢复备份也失败，请手动查日志）"
            }
        };
        return Err(format!(
            "写入 {} 工具配置失败: {e}{restore_hint}",
            app_type.as_str()
        ));
    }

    // 4) 切 current provider 到 ofox-<app>，DB is_current 跟随。
    //    DB 里 ofox-<app> provider 的 settings_config **不再**被 bind 流程
    //    污染——保持纯 seed 状态。前端不依赖那份字段；proxy 转发层在新路径
    //    下也不会触发去读它。
    crate::settings::set_current_provider(&app_type, Some(provider_id))
        .map_err(|e| format!("设置 {} 当前供应商失败: {e}", app_type.as_str()))?;
    db.set_current_provider(app_type.as_str(), provider_id)
        .map_err(|e| format!("更新 {} 数据库 is_current 失败: {e}", app_type.as_str()))?;

    // 5) bind 成功后更新 last_used_at——只有走 fetch_or_create_api_key 路径
    //    才有元数据条目；OFOX_USE_OAUTH_TOKEN_AS_KEY 兜底路径下 `mark_key_used`
    //    内部会发现没有元数据直接 noop。失败只 warn，不影响 bind 已经完成。
    crate::ofox_api_keys::mark_key_used(app_type);

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

/// Mirror of [`bind_tool_to_ofox_internal`] — undo the OfoxAI bind for `app`
/// and restore the official provider as active.
///
/// Steps (the inverse order of bind, so we never leave a window where the
/// live config still has ofox endpoints but a non-ofox active provider):
///   1. 从 DB live_backups 读快照 → 写回工具真实配置 → 删 backup
///      (`ofox_restore_from_backup`)。Claude/Codex/Gemini 是整文件还原；
///      OpenCode/OpenClaw/Hermes 是 "ofox-* provider 子节" 还原（或删除，
///      若 bind 前不存在）。
///   2. 切 active provider 到 `<app>-official`（仅 claude/codex/gemini 有
///      official seed；其它工具仅做步骤 1）。
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
    proxy_service: &crate::services::proxy::ProxyService,
    app: &str,
) -> Result<(), String> {
    let app_type = AppType::from_str(app).map_err(|e| format!("无效的应用类型: {e}"))?;
    let app_str = app_type.as_str();

    // 1) 从 backup 恢复磁盘 + 删 backup。失败立刻中止——没恢复前不能切官方
    //    provider，否则用户的 active 是 official 但磁盘还指 ofox。
    proxy_service
        .ofox_restore_from_backup(&app_type)
        .await
        .map_err(|e| format!("恢复 {app_str} 工具配置失败: {e}"))?;

    // 2) 切 active provider 到 official seed（如有）。
    //    `<app>-official` id 跟 `database/dao/providers_seed.rs::OFFICIAL_SEEDS`
    //    保持一致；目前只 claude/codex/gemini 有。其它工具无 official 概念，
    //    步骤 1 已经把磁盘还原好，不再额外切。
    let official_id: Option<&str> = match app_type {
        AppType::Claude => Some("claude-official"),
        AppType::Codex => Some("codex-official"),
        AppType::Gemini => Some("gemini-official"),
        AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => None,
    };

    if let Some(id) = official_id {
        // 两端持久化保持一致：settings.json 与 DB is_current。
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
