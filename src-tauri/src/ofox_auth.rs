//! Ofox OAuth Authentication Module
//!
//! Implements Device Authorization Grant (RFC 8628) for Ofox AI platform.
//! Single-account design (no multi-account support needed).
//!
//! ## Flow
//! 1. `start_device_flow()` requests a device code from the authorization server
//! 2. User opens verification URL in browser and enters user_code
//! 3. `poll_for_token()` polls until user approves or denies
//! 4. On approval, stores tokens and fetches user info

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

// ==================== Constants ====================

// OAuth IDP / OpenAPI 端点的具体 URL 现在由 [`crate::ofox_apex`] 单一开关：
//   - dev (`cfg!(debug_assertions)`) → localhost:3000 / 8080（与原硬编码常量
//     一致，不影响本地联调）
//   - release → `https://app.<apex>` / `https://api.<apex>`，apex 来自
//     settings.json `ofoxApex` 字段（首次启动 ip-api 探测填充）
//
// 单一开关让我们不再担心"token 在 dev IDP 签发但被打到 prod 网关"那种
// dev/prod 混搭。`ofox_apex::*_url()` 在每次调用时重新求值，所以用户在 UI
// 切换 apex 后下一次请求立刻走新地址，不需要重启进程。
// `apikey.write` 让我们能调 `POST/GET/PATCH/DELETE /openapi/api-keys`，
// 给每个工具按 slug 单独签发一把 `sk-of-...` LLM key，写进工具真实配置文件。
// `llm.read` / `llm.invoke` 当前都没被消费——chat/completions 这条链直连
// LLM gateway，认证用 sk-of- key，不依赖 OAuth access_token 的 scope。
const OFOX_SCOPES: &str = "org.read balance.read apikey.write offline_access";

/// 线上 OAuth client（public client，无 client_secret）。在 ofox.io / ofox.ai 的
/// OAuth app 后台注册为 `is_confidential=false`，device flow RFC 8628 §3.1 标准用法。
const OFOX_CLIENT_ID_PROD: &str = "ofox_app_0f3825107a340c44320b0f21";

/// 本地 dev IDP 用的 client。仅在 `OFOX_USE_LOCAL=1` 时启用，与 `ofox_apex` 的
/// dev/prod 切换共用同一开关，保证"endpoint 与 client_id 永远在同一侧"。
const OFOX_CLIENT_ID_DEV: &str = "ofox_app_switch_desktop_dev";

/// 单一入口决定本进程当前用哪个 client_id。
fn client_id() -> &'static str {
    if crate::ofox_apex::use_local_dev_mode() {
        OFOX_CLIENT_ID_DEV
    } else {
        OFOX_CLIENT_ID_PROD
    }
}

const TOKEN_REFRESH_BUFFER_MS: i64 = 60_000;

// ==================== Data Structures ====================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OfoxAuthState {
    /// No tokens at all — user has never logged in or has explicitly logged out.
    LoggedOut,
    /// Tokens present and (as far as we know) usable.
    Active,
    /// Tokens present but the refresh_token was rejected by the server with
    /// `invalid_grant`. The user must re-authenticate. We keep the cached
    /// user info around so the UI can still show "<email> · session expired".
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfoxUserInfo {
    pub email: Option<String>,
    pub name: Option<String>,
    pub org_id: Option<String>,
    pub avatar_url: Option<String>,
    /// OAuth 颁发 token 时快照的 role 值（owner / admin / member）。来源是
    /// `/openapi/me` 的 `role` 字段——平台在 token 里就钉死了 role，每次 RT
    /// rotation 才会刷新（详见 ofox-openapi-example/.../02-api-reference.md
    /// 的 "Role 模型" 一节）。
    ///
    /// `None` = 老版 token 没带这个字段，前端按 member（最小权限）回退；这
    /// 样既兼容历史 token，也避免误把"未知"当成"管理员"。前端 helper 见
    /// `src/lib/api/ofoxAuth.ts::isOfoxBillingManager`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Latest snapshot of the account wallet. Populated by
    /// `get_user_info_from_api`, which calls `/openapi/me` and then
    /// `/openapi/orgs/me/balance` in sequence. Optional so cached/legacy
    /// stores without a balance still deserialize cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<OfoxBalance>,
    /// Best-effort summary derived from `/openapi/orgs/me/spending-limits`.
    /// `None` when:
    ///   - role is `member` (endpoint returns 403)
    ///   - no spending limit with `period: "monthly"` is configured
    ///   - the endpoint is unreachable
    /// In all of these cases the UI should render a `—` placeholder rather
    /// than a fake zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spending: Option<OfoxSpending>,
}

/// Account wallet snapshot from `/openapi/orgs/me/balance`.
///
/// All amounts are USD floating point — same convention as the ofox-chat
/// reference implementation. Fields are optional because the upstream
/// schema is still evolving and we'd rather render "—" for one field than
/// fail the whole balance fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfoxBalance {
    #[serde(default)]
    pub balance: Option<f64>,
    #[serde(default)]
    pub total: Option<f64>,
    #[serde(default)]
    pub used: Option<f64>,
    #[serde(default)]
    pub currency: Option<String>,
}

/// Distilled view of `/openapi/orgs/me/spending-limits` for the UI.
///
/// The raw response is `{ spending_limits: [{ period, used_usd, limit_usd, ... }] }`.
/// We surface only what the dashboard renders today; raw items are not
/// needed by the frontend and would be churn-prone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfoxSpending {
    /// True when the endpoint returned 403 — caller's role lacks access
    /// (member role per docs). Drives the UI's "无权限" placeholder so we
    /// don't conflate "you can't see it" with "no monthly limit configured".
    #[serde(default)]
    pub forbidden: bool,
    /// `used_usd` from the entry whose `period == "monthly"`. `None` if
    /// no monthly limit is configured or `forbidden` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_used_usd: Option<f64>,
    /// `limit_usd` from the same monthly entry (rendering hint for
    /// "X.XX / Y.YY" — currently unused, kept for forward-compat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_limit_usd: Option<f64>,
}

/// Shape of one entry in the upstream `spending_limits` array. Only the
/// fields we actually need are deserialized.
#[derive(Debug, Deserialize)]
struct SpendingLimitEntry {
    period: Option<String>,
    used_usd: Option<f64>,
    limit_usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SpendingLimitsResponse {
    #[serde(default)]
    spending_limits: Vec<SpendingLimitEntry>,
}

/// Aggregate auth snapshot exposed to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct OfoxAuthStatus {
    pub state: OfoxAuthState,
    pub user: Option<OfoxUserInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfoxDeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

/// 当前 metadata 文件的 schema 版本。
///
/// `1` = 首版本，token 已搬进 OS 钥匙串，文件里只有 `expires_at` / `scope` /
/// `user` / `expired` 等非敏感元数据。0/缺省 = 仍包含明文 token 的旧版，触发
/// 一次性迁移（见 [`OfoxAuthManager::migrate_legacy_tokens`]）。
const AUTH_METADATA_SCHEMA_VERSION: u32 = 1;

/// 落盘的非敏感元数据。
///
/// 真正的 access_token / refresh_token 落在系统钥匙串里（[`ofox_secret`]），
/// 这里只保留"重启后还想恢复 UI 状态用得到"的字段。
///
/// 一个反直觉的约束：当钥匙串里其实**没有** token（用户从未登录、或刚被清
/// 掉）时，调用方不应该信任 `user` 字段——否则会出现"未登录但 UI 显示用户
/// 邮箱"的尴尬中间态。守这条规则的逻辑在 [`OfoxAuthManager::load_from_disk_sync`]
/// 里：只有读到至少一根 token 时才把 `user` 灌进内存。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OfoxAuthMetadata {
    /// schema 版本号；缺省按 `0` 处理（即旧明文版）以便老文件平滑迁移。
    #[serde(default)]
    schema_version: u32,
    pub expires_at: i64,
    pub scope: String,
    pub user: Option<OfoxUserInfo>,
    /// Optional, present only for stores written by versions that track expired sessions.
    /// Older files on disk simply lack this field and are treated as `Active`.
    #[serde(default)]
    pub expired: bool,
}

/// 兼容反序列化：v0 / pre-schema 的磁盘文件里 token 还在明文 json 里。
/// 只用在启动迁移路径，迁移完成后立即被新 schema 覆盖。
///
/// `serde(default)` 让缺字段也能解析，所以无论老文件少哪个字段都能解析成功，
/// 我们靠 `access_token` / `refresh_token` 是否非空判断是否需要迁移。
#[derive(Debug, Clone, Deserialize)]
struct LegacyOfoxTokenStore {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub user: Option<OfoxUserInfo>,
    #[serde(default)]
    pub expired: bool,
}

#[derive(Debug, Clone)]
struct PendingDeviceCode {
    user_code: String,
    expires_at_ms: i64,
}

struct CachedAccessToken {
    token: String,
    expires_at_ms: i64,
}

impl CachedAccessToken {
    fn is_expiring_soon(&self) -> bool {
        let now = chrono::Utc::now().timestamp_millis();
        self.expires_at_ms - now < TOKEN_REFRESH_BUFFER_MS
    }
}

/// OAuth token response from the Ofox token endpoint
#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
}

/// Error response from the token endpoint (device code polling)
#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

pub struct OfoxAuthManager {
    storage_path: PathBuf,
    access_token: Arc<RwLock<Option<CachedAccessToken>>>,
    refresh_token: Arc<RwLock<Option<String>>>,
    user_info: Arc<RwLock<Option<OfoxUserInfo>>>,
    /// Authoritative auth state for the UI. Transitions:
    ///   LoggedOut → Active     (successful device flow)
    ///   Active    → Expired    (refresh_token rejected with `invalid_grant`)
    ///   any       → LoggedOut  (explicit logout)
    ///   Expired   → Active     (successful re-login)
    auth_state: Arc<RwLock<OfoxAuthState>>,
    pending_device_codes: Arc<RwLock<HashMap<String, PendingDeviceCode>>>,
    http_client: Client,
    /// Handle used to emit `ofox-auth-expired` / `ofox-auth-restored` events.
    /// Set via [`OfoxAuthManager::attach_app_handle`] after the Tauri app is built.
    app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,
    /// OAuth token 落到 OS 钥匙串的后端。`Arc<dyn …>` 是为了测试时能注入
    /// `InMemoryStore` 跳过真实 Keychain（CI 环境没钥匙串守护进程）。
    secret_store: Arc<dyn crate::ofox_secret::SecretStore>,
}

impl OfoxAuthManager {
    pub fn new(app_config_dir: PathBuf) -> Self {
        Self::new_with_secret_store(
            app_config_dir,
            Arc::new(crate::ofox_secret::KeyringStore::new()),
        )
    }

    /// 测试入口：注入自定义 SecretStore（通常是 `InMemoryStore`），其余字段
    /// 跟生产路径一致。不要从生产代码调用——`OfoxAuthManager::new` 是单一入口。
    pub fn new_with_secret_store(
        app_config_dir: PathBuf,
        secret_store: Arc<dyn crate::ofox_secret::SecretStore>,
    ) -> Self {
        let storage_path = app_config_dir.join("ofox_auth.json");

        let manager = Self {
            storage_path,
            access_token: Arc::new(RwLock::new(None)),
            refresh_token: Arc::new(RwLock::new(None)),
            user_info: Arc::new(RwLock::new(None)),
            auth_state: Arc::new(RwLock::new(OfoxAuthState::LoggedOut)),
            pending_device_codes: Arc::new(RwLock::new(HashMap::new())),
            http_client: Client::new(),
            app_handle: Arc::new(RwLock::new(None)),
            secret_store,
        };

        if let Err(e) = manager.load_from_disk_sync() {
            log::warn!("[OfoxAuth] Failed to load stored auth: {e}");
        }

        manager
    }

    /// Wire up the AppHandle so we can emit lifecycle events to the frontend.
    /// Called from `lib.rs` setup once Tauri has built the app.
    pub async fn attach_app_handle(&self, handle: tauri::AppHandle) {
        let mut slot = self.app_handle.write().await;
        *slot = Some(handle);
    }

    // ==================== Device Code Flow ====================

    /// Start the Device Authorization flow.
    ///
    /// Returns device code info including user_code and verification_uri.
    pub async fn start_device_flow(&self) -> Result<OfoxDeviceCodeResponse, String> {
        let response = self
            .http_client
            .post(crate::ofox_apex::device_auth_url())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("client_id", client_id()),
                ("scope", OFOX_SCOPES),
            ])
            .send()
            .await
            .map_err(|e| format!("Device authorization request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("Device authorization failed: {status} - {text}"));
        }

        let resp: OfoxDeviceCodeResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse device authorization response: {e}"))?;

        // Store pending device code
        let expires_at_ms =
            chrono::Utc::now().timestamp_millis() + (resp.expires_in as i64) * 1000;
        {
            let mut pending = self.pending_device_codes.write().await;
            // Clean expired entries
            let now_ms = chrono::Utc::now().timestamp_millis();
            pending.retain(|_, entry| entry.expires_at_ms > now_ms);
            pending.insert(
                resp.device_code.clone(),
                PendingDeviceCode {
                    user_code: resp.user_code.clone(),
                    expires_at_ms,
                },
            );
        }

        log::info!(
            "[OfoxAuth] Device flow started, user_code: {}",
            resp.user_code
        );
        Ok(resp)
    }

    /// Poll the token endpoint for device code approval.
    ///
    /// Returns:
    /// - `Ok(Some(user))` — user approved, login complete
    /// - `Ok(None)` — authorization_pending, keep polling
    /// - `Err("slow_down")` — poll too fast, increase interval
    /// - `Err("access_denied")` — user denied
    /// - `Err("expired_token")` — device code expired
    pub async fn poll_for_token(
        &self,
        device_code: &str,
    ) -> Result<Option<OfoxUserInfo>, String> {
        // Verify device code is pending
        {
            let pending = self.pending_device_codes.read().await;
            let entry = pending
                .get(device_code)
                .ok_or_else(|| "Unknown device code. Please restart login.".to_string())?;

            if entry.expires_at_ms <= chrono::Utc::now().timestamp_millis() {
                drop(pending);
                let mut pending = self.pending_device_codes.write().await;
                pending.remove(device_code);
                return Err("expired_token".to_string());
            }
        }

        let response = self
            .http_client
            .post(crate::ofox_apex::token_url())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
                ("client_id", client_id()),
            ])
            .send()
            .await
            .map_err(|e| format!("Token poll request failed: {e}"))?;

        let status = response.status();

        if status.is_success() {
            // User approved — parse tokens
            let token_resp: OAuthTokenResponse = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse token response: {e}"))?;

            // Remove from pending
            {
                let mut pending = self.pending_device_codes.write().await;
                pending.remove(device_code);
            }

            let expires_at_ms = compute_expires_at_ms(token_resp.expires_in);

            // Store tokens
            {
                let mut at = self.access_token.write().await;
                *at = Some(CachedAccessToken {
                    token: token_resp.access_token.clone(),
                    expires_at_ms,
                });
            }
            {
                let mut rt = self.refresh_token.write().await;
                *rt = token_resp.refresh_token.clone();
            }

            // Fetch user info. Failure here is **non-fatal** — the tokens
            // we just stored are valid, the OAuth handshake is complete, and
            // the only thing that didn't load is the profile/balance bundle.
            // Treat that as a transient network glitch: keep an empty user
            // placeholder, still transition to Active, and let
            // `ofox_get_user_info` (which already has cache fallback in
            // `commands/ofox_auth.rs`) fill the gap on the next call.
            //
            // Pre-fix: a `?` here propagated the error all the way up to
            // the LoginPage's poll loop, which silently swallowed it. Tokens
            // were on disk but the UI was stuck — `set_state(Active)` and
            // `emit("ofox-auth-restored")` below never ran, and the device
            // code (already consumed by the upstream) couldn't be polled
            // again. Net effect: the user got stuck on the spinner forever.
            let user = match self
                .get_user_info_from_api(&token_resp.access_token)
                .await
            {
                Ok(u) => u,
                Err(e) => {
                    log::warn!(
                        "[OfoxAuth] device login: tokens stored OK, but \
                         /openapi/me failed ({e}). Continuing with empty \
                         profile; will be filled on next refresh."
                    );
                    OfoxUserInfo {
                        email: None,
                        name: None,
                        org_id: None,
                        avatar_url: None,
                        role: None,
                        balance: None,
                        spending: None,
                    }
                }
            };
            {
                let mut ui = self.user_info.write().await;
                *ui = Some(user.clone());
            }

            // Transition to Active and emit a restored event so any UI sitting on
            // an expired banner can drop it immediately.
            let prev_state = self.set_state(OfoxAuthState::Active).await;
            if prev_state != OfoxAuthState::Active {
                self.emit_state_event("ofox-auth-restored").await;
            }

            // Persist
            self.save_to_disk(
                &token_resp.access_token,
                token_resp.refresh_token.as_deref(),
                expires_at_ms,
                token_resp.scope.as_deref().unwrap_or(OFOX_SCOPES),
                Some(&user),
                false,
            )?;

            // Notify the token-refresh listener (lib.rs) so it pushes the
            // freshly-issued access_token into every `ofox-*` provider's
            // settings_config. Without this, a re-login after logout (or a
            // first login after an old install) leaves bound providers
            // pinned to the previous token — which the gateway either
            // can't resolve (we just revoked it on logout) or no longer
            // has the right scopes for. Mirrors the emit in
            // `refresh_access_token` (line ~516) — keep both paths
            // emitting the same event so downstream wiring stays single-
            // sourced.
            if let Some(handle) = self.app_handle.read().await.as_ref() {
                use tauri::Emitter;
                let _ = handle.emit("ofox-auth-token-refreshed", ());
            }

            log::info!("[OfoxAuth] Device flow login successful for {:?}", user.email);
            return Ok(Some(user));
        }

        // Error response — check error code
        let err_resp: OAuthErrorResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse error response: {e}"))?;

        match err_resp.error.as_str() {
            "authorization_pending" => Ok(None),
            "slow_down" => {
                log::debug!("[OfoxAuth] poll: slow_down");
                Err("slow_down".to_string())
            }
            "access_denied" => {
                let mut pending = self.pending_device_codes.write().await;
                pending.remove(device_code);
                Err("access_denied".to_string())
            }
            "expired_token" => {
                let mut pending = self.pending_device_codes.write().await;
                pending.remove(device_code);
                Err("expired_token".to_string())
            }
            other => {
                log::warn!(
                    "[OfoxAuth] poll: unknown error '{other}', description: {}",
                    err_resp.error_description.clone().unwrap_or_default()
                );
                Err(format!(
                    "{other}: {}",
                    err_resp.error_description.unwrap_or_default()
                ))
            }
        }
    }

    // ==================== Token Refresh ====================

    /// Refresh the access token using the stored refresh token.
    ///
    /// On `invalid_grant` (refresh_token rejected by the server), transitions
    /// to `Expired` and emits `ofox-auth-expired` for the UI.
    pub async fn refresh_access_token(&self) -> Result<(), String> {
        let refresh_token = {
            let rt = self.refresh_token.read().await;
            rt.clone()
                .ok_or_else(|| "No refresh token available".to_string())?
        };

        let response = self
            .http_client
            .post(crate::ofox_apex::token_url())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("client_id", client_id()),
            ])
            .send()
            .await
            .map_err(|e| format!("Refresh token request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            // Attempt to parse the OAuth error envelope so we can react to
            // `invalid_grant` specifically (refresh_token revoked/expired).
            let is_invalid_grant = serde_json::from_str::<OAuthErrorResponse>(&text)
                .map(|e| e.error == "invalid_grant")
                .unwrap_or(false);
            if is_invalid_grant {
                log::warn!("[OfoxAuth] refresh_token rejected as invalid_grant, marking session expired");
                self.mark_expired().await;
                return Err("invalid_grant".to_string());
            }
            return Err(format!("Token refresh failed: {status} - {text}"));
        }

        let token_resp: OAuthTokenResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse refresh response: {e}"))?;

        let expires_at_ms = compute_expires_at_ms(token_resp.expires_in);

        // Atomic update: both AT and RT together
        {
            let mut at = self.access_token.write().await;
            *at = Some(CachedAccessToken {
                token: token_resp.access_token.clone(),
                expires_at_ms,
            });
        }
        if let Some(new_rt) = &token_resp.refresh_token {
            let mut rt = self.refresh_token.write().await;
            *rt = Some(new_rt.clone());
        }

        let current_rt = self.refresh_token.read().await.clone();
        let current_user = self.user_info.read().await.clone();
        self.save_to_disk(
            &token_resp.access_token,
            current_rt.as_deref(),
            expires_at_ms,
            token_resp.scope.as_deref().unwrap_or(OFOX_SCOPES),
            current_user.as_ref(),
            false,
        )?;

        log::info!("[OfoxAuth] Access token refreshed successfully");

        // Tell anyone holding a stale copy of the token (e.g. bound tools'
        // settings_config) to re-pull. We deliberately do NOT include the
        // token in the payload — the renderer should never see it. A Rust
        // listener attached at startup (see `lib.rs`) pulls the fresh value
        // via `get_valid_access_token` and writes it into all ofox-* provider
        // rows. Failure to emit is non-fatal: the next manual bind would
        // resync anyway.
        if let Some(handle) = self.app_handle.read().await.as_ref() {
            use tauri::Emitter;
            let _ = handle.emit("ofox-auth-token-refreshed", ());
        }
        Ok(())
    }

    /// Get a valid access token, refreshing if necessary.
    ///
    /// If the session is `Expired` we don't try to refresh again — the
    /// refresh_token has already been rejected once, retries would only
    /// hammer the server and risk additional rate-limiting.
    pub async fn get_valid_access_token(&self) -> Result<String, String> {
        // Hard short-circuit on Expired: caller must trigger a new login.
        if *self.auth_state.read().await == OfoxAuthState::Expired {
            return Err("session_expired".to_string());
        }

        {
            let at = self.access_token.read().await;
            if let Some(cached) = at.as_ref() {
                if !cached.is_expiring_soon() {
                    return Ok(cached.token.clone());
                }
            }
        }

        let has_refresh = {
            let rt = self.refresh_token.read().await;
            rt.is_some()
        };

        if has_refresh {
            self.refresh_access_token().await?;
            let at = self.access_token.read().await;
            if let Some(cached) = at.as_ref() {
                return Ok(cached.token.clone());
            }
        }

        {
            let at = self.access_token.read().await;
            if let Some(cached) = at.as_ref() {
                return Ok(cached.token.clone());
            }
        }

        Err("Not authenticated".to_string())
    }

    // ==================== User Info ====================

    async fn get_user_info_from_api(&self, access_token: &str) -> Result<OfoxUserInfo, String> {
        let response = self
            .http_client
            .get(crate::ofox_apex::user_info_url())
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("User info request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            // 401/403 from the resource server means our token isn't being
            // honored — typically a server-side revocation. Surface as expired
            // so the UI prompts for re-auth instead of silently retrying.
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                log::warn!(
                    "[OfoxAuth] /openapi/me returned {status}, marking session expired: {text}"
                );
                self.mark_expired().await;
                return Err("session_expired".to_string());
            }
            return Err(format!("User info request failed: {status} - {text}"));
        }

        let mut user: OfoxUserInfo = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse user info: {e}"))?;

        // Best-effort balance fetch — a balance failure must not bring the
        // entire login flow down (a fresh org may not have wallet data yet,
        // or `balance.read` scope may have been denied).
        match self.fetch_balance(access_token).await {
            Ok(balance) => user.balance = Some(balance),
            Err(e) => {
                log::warn!("[OfoxAuth] balance fetch failed (non-fatal): {e}");
            }
        }

        // NOTE: spending-limits fetch is intentionally NOT chained in here.
        // The endpoint tracks *configured caps*, not month-to-date usage,
        // so it can't satisfy the "本月消耗" card. `fetch_spending` is kept
        // around for future quota-alert features but only called on demand.

        Ok(user)
    }

    /// Fetch the account wallet from `/openapi/orgs/me/balance`. Mirrors the
    /// ofox-chat backend, which proxies this same endpoint.
    async fn fetch_balance(&self, access_token: &str) -> Result<OfoxBalance, String> {
        let response = self
            .http_client
            .get(crate::ofox_apex::balance_url())
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Balance request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(format!("balance endpoint returned {status}: {text}"));
        }

        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read balance body: {e}"))?;
        log::debug!("[OfoxAuth] /openapi/orgs/me/balance response: {body}");

        serde_json::from_str::<OfoxBalance>(&body)
            .map_err(|e| format!("Failed to parse balance: {e}"))
    }

    /// Fetch the org's spending limits and reduce them to the single
    /// monthly figure the UI cares about.
    ///
    /// **Currently unused.** Kept for future quota-alert features (e.g.
    /// "you've hit 80% of your monthly cap"). NOT a source for "本月消耗" —
    /// that needs a real usage endpoint, which the OAuth API doesn't expose.
    ///
    /// Returns:
    ///   - `Ok(Some(spending))` when the endpoint responded; populates
    ///     `forbidden=true` for member role's 403, or `monthly_used_usd`
    ///     when a `period == "monthly"` entry exists.
    ///   - `Ok(None)` when the response succeeded but no monthly entry is
    ///     configured.
    ///   - `Err(_)` for transport / parse failures — caller logs and moves on.
    #[allow(dead_code)]
    async fn fetch_spending(
        &self,
        access_token: &str,
    ) -> Result<Option<OfoxSpending>, String> {
        let response = self
            .http_client
            .get(crate::ofox_apex::spending_limits_url())
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Spending request failed: {e}"))?;

        let status = response.status();
        if status == reqwest::StatusCode::FORBIDDEN {
            // Documented: `member` role gets 403 insufficient_role on this
            // endpoint. That's a normal state — not an error. We still
            // surface it (with `forbidden: true`) so the UI can distinguish
            // "no permission" from "no monthly limit configured".
            log::debug!("[OfoxAuth] spending-limits returned 403 (member role)");
            return Ok(Some(OfoxSpending {
                forbidden: true,
                monthly_used_usd: None,
                monthly_limit_usd: None,
            }));
        }
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(format!("spending endpoint returned {status}: {text}"));
        }

        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read spending body: {e}"))?;
        log::debug!("[OfoxAuth] /openapi/orgs/me/spending-limits response: {body}");

        let parsed: SpendingLimitsResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse spending: {e}"))?;

        // Find the monthly entry. The endpoint can return multiple periods
        // (`daily`, `weekly`, `monthly`); we only surface monthly here.
        let monthly = parsed
            .spending_limits
            .into_iter()
            .find(|e| e.period.as_deref() == Some("monthly"));

        Ok(monthly.map(|entry| OfoxSpending {
            forbidden: false,
            monthly_used_usd: entry.used_usd,
            monthly_limit_usd: entry.limit_usd,
        }))
    }

    pub fn get_user_info(&self) -> Option<OfoxUserInfo> {
        self.user_info.try_read().ok().and_then(|ui| ui.clone())
    }

    /// Snapshot the current auth state and cached user info — the data the
    /// frontend needs to decide between "show profile", "show expired banner",
    /// and "show login page".
    pub async fn get_auth_status(&self) -> OfoxAuthStatus {
        OfoxAuthStatus {
            state: *self.auth_state.read().await,
            user: self.user_info.read().await.clone(),
        }
    }

    /// Fetch fresh user info from the API, refreshing the access token if needed,
    /// then update the in-memory cache and persist to disk.
    pub async fn refresh_user_info(&self) -> Result<OfoxUserInfo, String> {
        let access_token = self.get_valid_access_token().await?;
        let user = self.get_user_info_from_api(&access_token).await?;

        {
            let mut ui = self.user_info.write().await;
            *ui = Some(user.clone());
        }

        // Persist alongside current tokens
        let current_at = {
            let at = self.access_token.read().await;
            at.as_ref().map(|c| (c.token.clone(), c.expires_at_ms))
        };
        let current_rt = self.refresh_token.read().await.clone();
        if let Some((at_token, expires_at_ms)) = current_at {
            self.save_to_disk(
                &at_token,
                current_rt.as_deref(),
                expires_at_ms,
                OFOX_SCOPES,
                Some(&user),
                false,
            )?;
        }

        Ok(user)
    }

    // ==================== Auth State Transitions ====================

    /// Transition to a new state, returning the previous one (for change detection).
    async fn set_state(&self, next: OfoxAuthState) -> OfoxAuthState {
        let mut slot = self.auth_state.write().await;
        let prev = *slot;
        *slot = next;
        prev
    }

    /// Emit a Tauri event reflecting the current auth state. Best-effort; if
    /// the app handle hasn't been attached yet (e.g. during early bootstrap)
    /// we just log and move on.
    async fn emit_state_event(&self, event_name: &str) {
        let handle = self.app_handle.read().await.clone();
        let Some(handle) = handle else {
            log::debug!("[OfoxAuth] emit_state_event({event_name}): no AppHandle yet");
            return;
        };
        let status = self.get_auth_status().await;
        // tauri's Emitter trait is needed for .emit on AppHandle
        use tauri::Emitter;
        if let Err(e) = handle.emit(event_name, &status) {
            log::warn!("[OfoxAuth] Failed to emit {event_name}: {e}");
        }
    }

    /// Mark the current session as expired (refresh_token no longer accepted).
    /// Idempotent: if already Expired, it's a no-op aside from logging.
    /// Keeps user info + the (now useless) refresh_token on disk so the UI
    /// can still display "<email> · session expired" until re-login.
    async fn mark_expired(&self) {
        let prev = self.set_state(OfoxAuthState::Expired).await;
        if prev == OfoxAuthState::Expired {
            return;
        }

        // Drop the cached access token; keep refresh_token bytes (they're
        // dead but we don't need to scrub them) and user info.
        {
            let mut at = self.access_token.write().await;
            *at = None;
        }

        // Persist the expired flag so a restart doesn't try to refresh again.
        let current_rt = self.refresh_token.read().await.clone();
        let current_user = self.user_info.read().await.clone();
        if let Err(e) = self.save_to_disk(
            "",
            current_rt.as_deref(),
            0,
            OFOX_SCOPES,
            current_user.as_ref(),
            true,
        ) {
            log::warn!("[OfoxAuth] Failed to persist expired state: {e}");
        }

        log::info!("[OfoxAuth] Session marked as expired; awaiting re-login");
        self.emit_state_event("ofox-auth-expired").await;
    }

    // ==================== Status & Lifecycle ====================

    /// "Has a usable session right now" — Active only. Expired and LoggedOut
    /// both fail the predicate so callers must drive the user through re-auth.
    pub fn is_authenticated(&self) -> bool {
        self.auth_state
            .try_read()
            .map(|s| *s == OfoxAuthState::Active)
            .unwrap_or(false)
    }

    pub fn logout(&mut self) {
        if let Ok(mut at) = self.access_token.try_write() {
            *at = None;
        }
        if let Ok(mut rt) = self.refresh_token.try_write() {
            *rt = None;
        }
        if let Ok(mut ui) = self.user_info.try_write() {
            *ui = None;
        }
        if let Ok(mut s) = self.auth_state.try_write() {
            *s = OfoxAuthState::LoggedOut;
        }

        // 同步删 keychain 两个条目。失败只记 warn——logout 必须能完成，否则
        // 用户卡在"看着登出按钮但点不动"的中间态比"keychain 里残留死 token"
        // 危险得多。`clear` 内部已经吞了 NoEntry。
        if let Err(e) = self
            .secret_store
            .clear(crate::ofox_secret::Slot::AccessToken)
        {
            log::warn!("[OfoxAuth] Failed to clear access_token from keychain: {e}");
        }
        if let Err(e) = self
            .secret_store
            .clear(crate::ofox_secret::Slot::RefreshToken)
        {
            log::warn!("[OfoxAuth] Failed to clear refresh_token from keychain: {e}");
        }

        if self.storage_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.storage_path) {
                log::warn!("[OfoxAuth] Failed to delete auth file: {e}");
            }
        }

        log::info!("[OfoxAuth] Logged out successfully");
    }

    // ==================== Persistence ====================

    /// 持久化一次 auth 状态——token 字段写 OS 钥匙串，其余元数据写 json 文件。
    ///
    /// 写入顺序：**先 RT 再 AT**。理由：
    ///   - RT 是长期凭据，写失败会让用户失去自愈刷新能力——必须先写它确保
    ///     至少能支撑下一次登录恢复；
    ///   - AT 即便短暂落不下也无所谓，下次启动 manager 用 RT 重刷一份即可；
    ///   - 反过来如果先写 AT 成功、写 RT 失败，用户会看到"短期能跑、一小时
    ///     后突然就要重登"的奇怪表现。
    ///
    /// 任一钥匙串调用失败即 fail-fast 返回 Err。json 写入用 atomic_write。
    /// 调用方（`mark_expired` 之外的三处）会把这个 Err 一路冒到用户层，UI 上
    /// 体现为登录失败、可以重试。
    fn save_to_disk(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: i64,
        scope: &str,
        user: Option<&OfoxUserInfo>,
        expired: bool,
    ) -> Result<(), String> {
        use crate::ofox_secret::Slot;

        // 1. 先处理 refresh_token：有值就写、没值就清。
        match refresh_token {
            Some(rt) if !rt.is_empty() => self.secret_store.save(Slot::RefreshToken, rt)?,
            _ => self.secret_store.clear(Slot::RefreshToken)?,
        }

        // 2. 再处理 access_token：同理。`mark_expired` 走这条空 AT 路径清掉
        //    钥匙串里那根失效的 access_token，避免别人误读到。
        if access_token.is_empty() {
            self.secret_store.clear(Slot::AccessToken)?;
        } else {
            self.secret_store.save(Slot::AccessToken, access_token)?;
        }

        // 3. 写元数据 json（不含任何 token 字段）。
        let metadata = OfoxAuthMetadata {
            schema_version: AUTH_METADATA_SCHEMA_VERSION,
            expires_at,
            scope: scope.to_string(),
            user: user.cloned(),
            expired,
        };

        let data = serde_json::to_string_pretty(&metadata)
            .map_err(|e| format!("Failed to serialize auth metadata: {e}"))?;

        crate::config::atomic_write(&self.storage_path, data.as_bytes())
            .map_err(|e| format!("Failed to write auth file: {e}"))?;

        log::debug!("[OfoxAuth] Auth state persisted (keychain + metadata)");
        Ok(())
    }

    fn load_from_disk_sync(&self) -> Result<(), String> {
        if !self.storage_path.exists() {
            // 没有 json 文件——通常是全新安装或刚 logout。也兜底清一下钥匙串
            // 残留（理论上 logout 已经清过；这里再补一刀防御历史 bug 残留）。
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.storage_path)
            .map_err(|e| format!("Failed to read auth file: {e}"))?;

        // 解析成 Value 以便先看 schema_version、再决定走新解析还是迁移路径。
        // 用 Value 中转一次比起 try-OfoxAuthMetadata-then-try-Legacy 错误信息
        // 更清晰，且 metadata 文件本来就小（< 2KB），性能不是顾虑。
        let raw: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse auth file: {e}"))?;

        let schema_version = raw
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // 旧 schema（含明文 token）走一次性迁移；新 schema 直接读。
        if schema_version < AUTH_METADATA_SCHEMA_VERSION {
            return self.migrate_legacy_tokens(&raw);
        }

        let metadata: OfoxAuthMetadata = serde_json::from_value(raw)
            .map_err(|e| format!("Failed to deserialize auth metadata: {e}"))?;

        self.hydrate_from_keychain(&metadata)
    }

    /// 从钥匙串读 token + 把 metadata 灌到内存字段。所有"新 schema 路径"的
    /// 终点都在这里，无论是首次 load、刷新后重读，还是迁移成功后再 hydrate。
    fn hydrate_from_keychain(&self, metadata: &OfoxAuthMetadata) -> Result<(), String> {
        use crate::ofox_secret::Slot;

        let stored_at = self.secret_store.load(Slot::AccessToken)?;
        let stored_rt = self.secret_store.load(Slot::RefreshToken)?;
        let has_any_token = stored_at.is_some() || stored_rt.is_some();

        // 状态推导：expired flag 优先（哪怕钥匙串里 RT 还在，用户已被 IDP
        // 拒过一次就不该再无声重试）。否则按"钥匙串里有没有 token"判断。
        let initial_state = if metadata.expired {
            OfoxAuthState::Expired
        } else if has_any_token {
            OfoxAuthState::Active
        } else {
            OfoxAuthState::LoggedOut
        };

        if let Ok(mut at) = self.access_token.try_write() {
            *at = stored_at.map(|token| CachedAccessToken {
                token,
                expires_at_ms: metadata.expires_at,
            });
        }
        if let Ok(mut rt) = self.refresh_token.try_write() {
            *rt = stored_rt;
        }
        if let Ok(mut ui) = self.user_info.try_write() {
            // 只在确实有 token 时才信任 metadata 里的 user——避免"未登录但
            // UI 显示历史邮箱"的尴尬。若两根 token 都没了，user 也清空。
            *ui = if has_any_token { metadata.user.clone() } else { None };
        }
        if let Ok(mut s) = self.auth_state.try_write() {
            *s = initial_state;
        }

        log::info!("[OfoxAuth] Loaded auth state (state={initial_state:?})");
        Ok(())
    }

    /// 一次性迁移：磁盘上还是含明文 token 的旧 schema。
    ///
    /// 流程：
    ///   1. 用 `LegacyOfoxTokenStore` 把旧字段抠出来
    ///   2. 钥匙串里如果已经有 token（说明之前迁过一半），跳过写入避免覆盖
    ///      更新过的 token；否则按"先 RT 再 AT"顺序写入
    ///   3. 全部成功才用新 schema 重写 json（自动丢弃 token 字段）
    ///   4. 失败保留旧 json 原状，下次启动重试，期间 hydrate 直接读旧字段
    ///      让用户先用着
    fn migrate_legacy_tokens(&self, raw: &serde_json::Value) -> Result<(), String> {
        use crate::ofox_secret::Slot;

        let legacy: LegacyOfoxTokenStore = serde_json::from_value(raw.clone())
            .map_err(|e| format!("Failed to deserialize legacy auth file: {e}"))?;

        let has_legacy_at = !legacy.access_token.is_empty();
        let has_legacy_rt = legacy
            .refresh_token
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false);

        log::info!(
            "[OfoxAuth] Migrating legacy auth file → keychain (at={has_legacy_at}, rt={has_legacy_rt})"
        );

        // 钥匙串现存值（若有）的"幂等保护"：之前如果已经迁过一次但 json 重写
        // 失败，下次启动钥匙串里就已经有最新 token——再覆盖一遍会把刷新过的
        // 新 token 写回成旧的。检测到非空就跳过。
        let keychain_has_rt = self.secret_store.load(Slot::RefreshToken)?.is_some();
        let keychain_has_at = self.secret_store.load(Slot::AccessToken)?.is_some();

        if !keychain_has_rt {
            if let Some(rt) = legacy.refresh_token.as_deref().filter(|s| !s.is_empty()) {
                self.secret_store.save(Slot::RefreshToken, rt)?;
            }
        }
        if !keychain_has_at && has_legacy_at {
            self.secret_store
                .save(Slot::AccessToken, &legacy.access_token)?;
        }

        // 用新 schema 重写元数据 json，把 token 字段顺势丢掉。这一步成功后
        // 旧 token 才真正从磁盘消失——前面写钥匙串失败时不会走到这里。
        let metadata = OfoxAuthMetadata {
            schema_version: AUTH_METADATA_SCHEMA_VERSION,
            expires_at: legacy.expires_at,
            scope: legacy.scope.clone(),
            user: legacy.user.clone(),
            expired: legacy.expired,
        };

        let data = serde_json::to_string_pretty(&metadata)
            .map_err(|e| format!("Failed to serialize migrated metadata: {e}"))?;
        crate::config::atomic_write(&self.storage_path, data.as_bytes())
            .map_err(|e| format!("Failed to rewrite auth file post-migration: {e}"))?;

        log::info!("[OfoxAuth] Legacy auth file migrated to keychain + new schema");
        self.hydrate_from_keychain(&metadata)
    }
}

fn compute_expires_at_ms(expires_in: Option<i64>) -> i64 {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let secs = expires_in.unwrap_or(3600);
    now_ms + secs * 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ofox_secret::SecretStore;

    #[test]
    fn test_compute_expires_at_ms() {
        let result = compute_expires_at_ms(Some(3600));
        let now = chrono::Utc::now().timestamp_millis();
        assert!(result > now + 3500 * 1000);
        assert!(result < now + 3700 * 1000);
    }

    #[tokio::test]
    async fn test_manager_initial_state_not_authenticated() {
        // 用 InMemoryStore 跑——CI 上没真实 keychain 守护进程，调 KeyringStore
        // 会失败。线程注入符合 `new_with_secret_store` 的设计意图。
        let temp = tempfile::tempdir().unwrap();
        let manager = OfoxAuthManager::new_with_secret_store(
            temp.path().to_path_buf(),
            Arc::new(crate::ofox_secret::InMemoryStore::new()),
        );
        assert!(!manager.is_authenticated());
        assert!(manager.get_user_info().is_none());
    }

    #[tokio::test]
    async fn test_legacy_json_migrates_tokens_to_keychain() {
        // 模拟 v0 schema：旧 ofox_auth.json 含明文 token。启动后 manager 应
        // 把 token 搬进 keychain（InMemoryStore），并把 json 改写为新 schema
        // 不含 token 字段。
        let temp = tempfile::tempdir().unwrap();
        let auth_path = temp.path().join("ofox_auth.json");
        let legacy = serde_json::json!({
            "access_token": "legacy-at",
            "refresh_token": "legacy-rt",
            "expires_at": 1_700_000_000_000_i64,
            "scope": "org.read balance.read offline_access",
            "user": null,
            "expired": false,
        });
        std::fs::write(&auth_path, serde_json::to_string_pretty(&legacy).unwrap()).unwrap();

        let store: Arc<dyn crate::ofox_secret::SecretStore> =
            Arc::new(crate::ofox_secret::InMemoryStore::new());
        let manager = OfoxAuthManager::new_with_secret_store(
            temp.path().to_path_buf(),
            Arc::clone(&store),
        );

        // 钥匙串里应该出现两根 token
        assert_eq!(
            store
                .load(crate::ofox_secret::Slot::AccessToken)
                .unwrap()
                .as_deref(),
            Some("legacy-at")
        );
        assert_eq!(
            store
                .load(crate::ofox_secret::Slot::RefreshToken)
                .unwrap()
                .as_deref(),
            Some("legacy-rt")
        );

        // 新 json 应该只剩元数据，schema_version=1
        let migrated = std::fs::read_to_string(&auth_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&migrated).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert!(v.get("access_token").is_none());
        assert!(v.get("refresh_token").is_none());

        // Manager 状态应为 Active（迁移完成 + 至少一根 token）
        assert!(manager.is_authenticated());
    }

    #[tokio::test]
    async fn test_migration_skips_when_keychain_already_has_tokens() {
        // 防御场景：上次迁移写完 keychain 但 json 重写失败 → 下次启动如果再
        // 写一次 keychain 会用旧 token 覆盖期间被 refresh 过的新 token。
        let temp = tempfile::tempdir().unwrap();
        let auth_path = temp.path().join("ofox_auth.json");
        let legacy = serde_json::json!({
            "access_token": "stale-at-from-disk",
            "refresh_token": "stale-rt-from-disk",
            "expires_at": 0,
            "scope": "",
            "user": null,
            "expired": false,
        });
        std::fs::write(&auth_path, serde_json::to_string_pretty(&legacy).unwrap()).unwrap();

        let store = crate::ofox_secret::InMemoryStore::new();
        store
            .save(crate::ofox_secret::Slot::AccessToken, "fresh-at")
            .unwrap();
        store
            .save(crate::ofox_secret::Slot::RefreshToken, "fresh-rt")
            .unwrap();
        let store: Arc<dyn crate::ofox_secret::SecretStore> = Arc::new(store);

        let _manager = OfoxAuthManager::new_with_secret_store(
            temp.path().to_path_buf(),
            Arc::clone(&store),
        );

        // 钥匙串里的 fresh token 不被磁盘旧值覆盖
        assert_eq!(
            store
                .load(crate::ofox_secret::Slot::AccessToken)
                .unwrap()
                .as_deref(),
            Some("fresh-at")
        );
        assert_eq!(
            store
                .load(crate::ofox_secret::Slot::RefreshToken)
                .unwrap()
                .as_deref(),
            Some("fresh-rt")
        );
    }
}
