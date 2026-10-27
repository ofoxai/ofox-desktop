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
// 内置助手使用 llm.invoke；旧 token 必须重新授权，刷新不能扩大已有 scope。
const OFOX_LEGACY_SCOPES: &str = "org.read balance.read apikey.write offline_access";
const OFOX_SCOPES: &str = "org.read balance.read apikey.write offline_access llm.invoke";

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
    /// `/openapi/me` 的用户 id（形如 `user_abc`）。老版本缓存里没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
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
    ///
    /// In all of these cases the UI should render a `—` placeholder rather
    /// than a fake zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spending: Option<OfoxSpending>,
}

impl OfoxUserInfo {
    /// 这个账号的计费身份：组织 id 优先（余额接口给的比 `org_id` 可靠），
    /// 再加上用户 id。
    pub fn key_owner(&self) -> crate::settings::KeyOwner {
        crate::settings::KeyOwner {
            org_id: self
                .balance
                .as_ref()
                .and_then(|balance| balance.organization_id.clone())
                .or_else(|| self.org_id.clone()),
            user_id: self.id.clone(),
        }
    }
}

/// Account wallet snapshot from `/openapi/orgs/me/balance`.
///
/// All amounts are USD floating point — same convention as the ofox-chat
/// reference implementation. Fields are optional because the upstream
/// schema is still evolving and we'd rather render "—" for one field than
/// fail the whole balance fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfoxBalance {
    /// 余额所属的组织。API key 按组织计费，换账号时用它判断 key 还能不能用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// `limit_usd` from the same monthly entry (rendered as "X.XX / Y.YY").
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
    /// 读取钥匙串里的登录信息时，用户在系统弹窗里点了「拒绝」：显示为未登录，
    /// 但可以重新授权（`retry_keychain`）恢复，不必重新登录。
    pub keychain_denied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfoxDeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
    #[serde(default)]
    pub assistant_unavailable: bool,
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
    expires_at_ms: i64,
    scope: String,
    expected_owner: Option<String>,
    generation: u64,
    apex: String,
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
    granted_scope: std::sync::RwLock<String>,
    auth_generation: std::sync::atomic::AtomicU64,
    user_info: Arc<RwLock<Option<OfoxUserInfo>>>,
    /// Authoritative auth state for the UI. Transitions:
    ///   LoggedOut → Active     (successful device flow)
    ///   Active    → Expired    (refresh_token rejected with `invalid_grant`)
    ///   any       → LoggedOut  (explicit logout)
    ///   Expired   → Active     (successful re-login)
    auth_state: Arc<RwLock<OfoxAuthState>>,
    /// 刷新单飞锁。token 静默刷新循环、低余额循环、UI 触发的 refresh_user_info
    /// 都可能在同一秒并发进 `refresh_access_token`——OFox IDP 的 refresh_token
    /// 一次性轮转,并发双刷会让后到者吃 `invalid_grant` 把好端端的会话标成
    /// Expired。持锁 + 锁内 double-check 保证同一时刻只有一个真实刷新请求在飞。
    refresh_lock: tokio::sync::Mutex<()>,
    pending_device_codes: Arc<RwLock<HashMap<String, PendingDeviceCode>>>,
    http_client: Client,
    /// Handle used to emit `ofox-auth-expired` / `ofox-auth-restored` events.
    /// Set via [`OfoxAuthManager::attach_app_handle`] after the Tauri app is built.
    app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,
    /// OAuth token 落到 OS 钥匙串的后端。`Arc<dyn …>` 是为了测试时能注入
    /// `InMemoryStore` 跳过真实 Keychain（CI 环境没钥匙串守护进程）。
    secret_store: Arc<dyn crate::ofox_secret::SecretStore>,
    /// 上次读取钥匙串时被用户拒绝（见 [`OfoxAuthStatus::keychain_denied`]）。
    keychain_denied: Arc<std::sync::atomic::AtomicBool>,
}

impl OfoxAuthManager {
    pub fn new(app_config_dir: PathBuf) -> Self {
        // release 走钥匙串，debug 走明文文件（避免反复弹授权框）——选择逻辑集中
        // 在 ofox_secret::default_store 里，见那里的说明。
        Self::new_with_secret_store(app_config_dir, crate::ofox_secret::default_store())
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
            granted_scope: std::sync::RwLock::new(String::new()),
            auth_generation: std::sync::atomic::AtomicU64::new(0),
            user_info: Arc::new(RwLock::new(None)),
            auth_state: Arc::new(RwLock::new(OfoxAuthState::LoggedOut)),
            refresh_lock: tokio::sync::Mutex::new(()),
            pending_device_codes: Arc::new(RwLock::new(HashMap::new())),
            http_client: Client::new(),
            app_handle: Arc::new(RwLock::new(None)),
            secret_store,
            keychain_denied: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        manager.load_stored_session();
        manager
    }

    /// 读回上次的登录；记下是不是因为钥匙串被拒绝才读不到。
    fn load_stored_session(&self) {
        let denied = match self.load_from_disk_sync() {
            Ok(()) => false,
            Err(e) => {
                log::warn!("[OfoxAuth] Failed to load stored auth: {e}");
                crate::ofox_secret::is_keychain_denied(&e)
            }
        };
        self.keychain_denied
            .store(denied, std::sync::atomic::Ordering::SeqCst);
    }

    /// 用户点「重新授权」：再读一次钥匙串（系统会再次弹窗），返回读完后的状态。
    pub async fn retry_keychain(&self) -> OfoxAuthStatus {
        self.load_stored_session();
        if self.is_authenticated() {
            // 和登录成功走同一条路：老用户回主界面，新用户去选工具（MainApp）。
            self.emit_state_event("ofox-auth-restored").await;
        }
        self.get_auth_status().await
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
        self.start_device_flow_for(false).await
    }
    pub async fn start_device_flow_for(
        &self,
        assistant_only: bool,
    ) -> Result<OfoxDeviceCodeResponse, String> {
        self.start_device_flow_with(&crate::ofox_apex::device_auth_url(), assistant_only)
            .await
    }
    #[cfg(test)]
    async fn start_device_flow_at(&self, url: &str) -> Result<OfoxDeviceCodeResponse, String> {
        self.start_device_flow_with(url, false).await
    }
    async fn start_device_flow_with(
        &self,
        url: &str,
        assistant_only: bool,
    ) -> Result<OfoxDeviceCodeResponse, String> {
        let expected_owner = if assistant_only {
            let owner = self
                .current_key_owner()
                .await
                .ok_or("assistant_sign_in_required")?;
            Some(crate::services::assistant::store::namespace(
                owner.user_id.as_deref().unwrap_or(""),
                owner.org_id.as_deref().unwrap_or(""),
            )?)
        } else {
            None
        };
        let generation = self
            .auth_generation
            .load(std::sync::atomic::Ordering::SeqCst);
        let apex = crate::ofox_apex::current_apex().to_string();
        let mut scope = OFOX_SCOPES;
        let mut resp = loop {
            let response = self
                .http_client
                .post(url)
                .form(&[("client_id", client_id()), ("scope", scope)])
                .send()
                .await
                .map_err(|_| "Device authorization request failed")?;
            if response.status().is_success() {
                break response
                    .json::<OfoxDeviceCodeResponse>()
                    .await
                    .map_err(|_| "Invalid device authorization response")?;
            }
            let status = response.status();
            let error = response.json::<OAuthErrorResponse>().await.ok();
            if !assistant_only
                && scope == OFOX_SCOPES
                && status == reqwest::StatusCode::BAD_REQUEST
                && error.as_ref().is_some_and(|e| e.error == "invalid_scope")
            {
                scope = OFOX_LEGACY_SCOPES;
                continue;
            }
            return Err(error
                .map(|e| e.error)
                .unwrap_or_else(|| format!("Device authorization failed: {status}")));
        };
        if generation
            != self
                .auth_generation
                .load(std::sync::atomic::Ordering::SeqCst)
            || apex != crate::ofox_apex::current_apex()
        {
            return Err("assistant_identity_changed".into());
        }
        resp.assistant_unavailable = scope == OFOX_LEGACY_SCOPES;
        let expires_at_ms = chrono::Utc::now()
            .timestamp_millis()
            .saturating_add((resp.expires_in.min(86_400) as i64) * 1000);
        let mut pending = self.pending_device_codes.write().await;
        let now = chrono::Utc::now().timestamp_millis();
        pending.retain(|_, item| item.expires_at_ms > now);
        pending.insert(
            resp.device_code.clone(),
            PendingDeviceCode {
                expires_at_ms,
                scope: scope.into(),
                expected_owner,
                generation,
                apex,
            },
        );
        Ok(resp)
    }
    pub async fn cancel_device_flow(&self, device_code: &str) {
        self.pending_device_codes.write().await.remove(device_code);
    }

    fn save_additional_grant(
        &self,
        token: &OAuthTokenResponse,
        expires: i64,
        scope: &str,
        user: &OfoxUserInfo,
    ) -> Result<(), String> {
        use crate::ofox_secret::Slot;
        // Snapshot before changing either store. A failed/cancelled upgrade never
        // clears the working in-memory account or intentionally signs it out.
        let old_at = self
            .secret_store
            .load(Slot::AccessToken)
            .map_err(|_| "assistant_authorization_persist_failed")?;
        let old_rt = self
            .secret_store
            .load(Slot::RefreshToken)
            .map_err(|_| "assistant_authorization_persist_failed")?;
        let metadata = match std::fs::read(&self.storage_path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err("assistant_authorization_persist_failed".into()),
        };
        let old_scope = self
            .granted_scope
            .read()
            .map_err(|_| "scope_lock_failed")?
            .clone();
        if self
            .save_to_disk(
                &token.access_token,
                token.refresh_token.as_deref(),
                expires,
                scope,
                Some(user),
                false,
            )
            .is_ok()
        {
            return Ok(());
        }
        let mut restored = true;
        for (slot, value) in [(Slot::RefreshToken, old_rt), (Slot::AccessToken, old_at)] {
            restored &= match value {
                Some(value) => self.secret_store.save(slot, &value),
                None => self.secret_store.clear(slot),
            }
            .is_ok();
        }
        restored &= match metadata {
            Some(bytes) => crate::config::atomic_write(&self.storage_path, &bytes).is_ok(),
            None => match std::fs::remove_file(&self.storage_path) {
                Ok(()) => true,
                Err(e) => e.kind() == std::io::ErrorKind::NotFound,
            },
        };
        if let Ok(mut grant) = self.granted_scope.write() {
            *grant = old_scope;
        }
        Err(if restored {
            "assistant_authorization_persist_failed"
        } else {
            "assistant_authorization_persist_rollback_failed"
        }
        .into())
    }

    /// Poll the token endpoint for device code approval.
    ///
    /// Returns:
    /// - `Ok(Some(user))` — user approved, login complete
    /// - `Ok(None)` — authorization_pending, keep polling
    /// - `Err("slow_down")` — poll too fast, increase interval
    /// - `Err("access_denied")` — user denied
    /// - `Err("expired_token")` — device code expired
    pub async fn poll_for_token(&self, device_code: &str) -> Result<Option<OfoxUserInfo>, String> {
        self.poll_for_token_at(
            device_code,
            &crate::ofox_apex::token_url(),
            &crate::ofox_apex::user_info_url(),
            true,
        )
        .await
    }
    async fn poll_for_token_at(
        &self,
        device_code: &str,
        token_url: &str,
        profile_url: &str,
        fetch_balance: bool,
    ) -> Result<Option<OfoxUserInfo>, String> {
        let entry = self
            .pending_device_codes
            .read()
            .await
            .get(device_code)
            .cloned()
            .ok_or("authorization_cancelled")?;
        if entry.expires_at_ms <= chrono::Utc::now().timestamp_millis() {
            self.cancel_device_flow(device_code).await;
            return Err("expired_token".into());
        }
        if entry.generation
            != self
                .auth_generation
                .load(std::sync::atomic::Ordering::SeqCst)
            || entry.apex != crate::ofox_apex::current_apex()
        {
            return Err("assistant_identity_changed".into());
        }
        let response = self
            .http_client
            .post(token_url)
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

            let expires_at_ms = compute_expires_at_ms(token_resp.expires_in);
            // Probe the proposed credential without expiring or changing the old
            // account. An additional grant must verify the same user AND org.
            let profile = self
                .fetch_user_profile(
                    &token_resp.access_token,
                    profile_url,
                    entry.expected_owner.is_none(),
                )
                .await;
            let mut user = match profile {
                Ok(user) => user,
                Err(error) if entry.expected_owner.is_some() => return Err(error),
                Err(_) => OfoxUserInfo {
                    id: None,
                    email: None,
                    name: None,
                    org_id: None,
                    avatar_url: None,
                    role: None,
                    balance: None,
                    spending: None,
                },
            };
            if let Some(expected) = &entry.expected_owner {
                let owner = user.key_owner();
                if crate::services::assistant::store::namespace(
                    owner.user_id.as_deref().unwrap_or(""),
                    owner.org_id.as_deref().unwrap_or(""),
                )? != *expected
                {
                    return Err("assistant_authorization_identity_mismatch".into());
                }
                user.balance = self
                    .user_info
                    .read()
                    .await
                    .as_ref()
                    .and_then(|u| u.balance.clone());
            } else if fetch_balance {
                if let Ok(balance) = self.fetch_balance(&token_resp.access_token).await {
                    user.balance = Some(balance);
                }
            }
            let _refresh = self.refresh_lock.lock().await;
            let mut pending = self.pending_device_codes.write().await;
            if !pending.contains_key(device_code) {
                return Err("authorization_cancelled".into());
            }
            if entry.generation
                != self
                    .auth_generation
                    .load(std::sync::atomic::Ordering::SeqCst)
                || entry.apex != crate::ofox_apex::current_apex()
            {
                return Err("assistant_identity_changed".into());
            }
            if let Some(expected) = &entry.expected_owner {
                let current = self
                    .current_key_owner()
                    .await
                    .ok_or("assistant_identity_changed")?;
                if crate::services::assistant::store::namespace(
                    current.user_id.as_deref().unwrap_or(""),
                    current.org_id.as_deref().unwrap_or(""),
                )? != *expected
                {
                    return Err("assistant_identity_changed".into());
                }
            }
            let scope = token_resp.scope.as_deref().unwrap_or(&entry.scope);
            if entry.expected_owner.is_some()
                && !scope.split_whitespace().any(|s| s == "llm.invoke")
            {
                return Err("assistant_reauthorization_required".into());
            }
            if entry.expected_owner.is_some() {
                self.save_additional_grant(&token_resp, expires_at_ms, scope, &user)?;
            } else {
                self.save_to_disk(
                    &token_resp.access_token,
                    token_resp.refresh_token.as_deref(),
                    expires_at_ms,
                    scope,
                    Some(&user),
                    false,
                )?;
            }
            *self.access_token.write().await = Some(CachedAccessToken {
                token: token_resp.access_token.clone(),
                expires_at_ms,
            });
            *self.refresh_token.write().await = token_resp.refresh_token.clone();
            *self.user_info.write().await = Some(user.clone());
            pending.remove(device_code);
            self.auth_generation
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let previous = self.set_state(OfoxAuthState::Active).await;
            if previous != OfoxAuthState::Active {
                self.emit_state_event("ofox-auth-restored").await;
            }
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

            log::info!(
                "[OfoxAuth] Device flow login successful for {:?}",
                user.email
            );
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
        self.refresh_access_token_at(&crate::ofox_apex::token_url())
            .await
    }

    /// URL-agnostic 内层——拆出来仅为单测能注入本地 wiremock 地址(与
    /// `ofox_api_keys::call_create_endpoint_at` 同一先例)。生产路径走
    /// [`Self::refresh_access_token`]。
    async fn refresh_access_token_at(&self, token_url: &str) -> Result<(), String> {
        // 单飞:并发调用在这排队,同一时刻只有一个真实刷新请求在飞。
        let _guard = self.refresh_lock.lock().await;

        // Double-check(拿到锁后世界可能已经变了):
        //   - 等锁期间别的任务吃了真·invalid_grant → 状态已 Expired,别再拿
        //     死 RT 去撞 IDP;
        //   - 等锁期间别的任务刚刷完 → 缓存 AT 已新鲜,直接复用,不发第二次
        //     请求(第二次请求会消费掉一次性轮转的 RT,把会话搞死)。
        if *self.auth_state.read().await == OfoxAuthState::Expired {
            return Err("session_expired".to_string());
        }
        {
            let at = self.access_token.read().await;
            if let Some(cached) = at.as_ref() {
                if !cached.is_expiring_soon() {
                    return Ok(());
                }
            }
        }

        let refresh_token = {
            let rt = self.refresh_token.read().await;
            rt.clone()
                .ok_or_else(|| "No refresh token available".to_string())?
        };

        let response = self
            .http_client
            .post(token_url)
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
                // 兜底:请求期间 RT 可能被其他写入路径轮转过(典型是
                // `poll_for_token` 并发完成了一次重新登录)。此时被拒的是旧
                // RT,会话本身是好的——不能标 Expired。单飞锁挡刷新↔刷新,
                // 这里挡刷新↔登录。
                let rt_rotated = {
                    let rt = self.refresh_token.read().await;
                    rt.as_deref() != Some(refresh_token.as_str())
                };
                if rt_rotated {
                    log::warn!(
                        "[OfoxAuth] refresh got invalid_grant but the refresh_token \
                         was rotated concurrently (likely a parallel re-login); \
                         keeping session alive"
                    );
                    return Ok(());
                }
                log::warn!(
                    "[OfoxAuth] refresh_token rejected as invalid_grant, marking session expired"
                );
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
        // A refresh never grants newly requested scopes. Servers omitting
        // `scope` retain the previous grant (RFC 6749 section 6).
        let previous_scope = self
            .granted_scope
            .read()
            .map_err(|_| "scope_lock_failed")?
            .clone();
        let refresh_scope = if let Some(reported) = token_resp.scope.as_deref() {
            previous_scope
                .split_whitespace()
                .filter(|scope| reported.split_whitespace().any(|value| value == *scope))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            previous_scope
        };
        self.save_to_disk(
            &token_resp.access_token,
            current_rt.as_deref(),
            expires_at_ms,
            &refresh_scope,
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

    async fn fetch_user_profile(
        &self,
        access_token: &str,
        url: &str,
        expire_current: bool,
    ) -> Result<OfoxUserInfo, String> {
        let generation = self
            .auth_generation
            .load(std::sync::atomic::Ordering::SeqCst);
        let response = self
            .http_client
            .get(url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("User info request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            // 401/403 from the resource server means our token isn't being
            // honored — typically a server-side revocation. Surface as expired
            // so the UI prompts for re-auth instead of silently retrying.
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                if expire_current {
                    let _lock = self.refresh_lock.lock().await;
                    if generation
                        == self
                            .auth_generation
                            .load(std::sync::atomic::Ordering::SeqCst)
                        && self
                            .access_token
                            .read()
                            .await
                            .as_ref()
                            .is_some_and(|cached| cached.token == access_token)
                    {
                        log::warn!("[OfoxAuth] current profile request returned {status}");
                        self.mark_expired().await;
                    }
                }
                return Err("session_expired".to_string());
            }
            return Err(format!("User info request failed: {status}"));
        }

        let user: OfoxUserInfo = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse user info: {e}"))?;

        Ok(user)
    }

    async fn get_user_info_from_api_at(
        &self,
        access_token: &str,
        url: &str,
        with_balance: bool,
    ) -> Result<OfoxUserInfo, String> {
        let mut user = self.fetch_user_profile(access_token, url, true).await?;
        // Best-effort balance fetch — a balance failure must not bring the
        // entire login flow down (a fresh org may not have wallet data yet,
        // or `balance.read` scope may have been denied).
        if with_balance {
            match self.fetch_balance(access_token).await {
                Ok(balance) => user.balance = Some(balance),
                Err(e) => {
                    log::warn!("[OfoxAuth] balance fetch failed (non-fatal): {e}");
                }
            }
        }
        // NOTE: spending-limits fetch is intentionally NOT chained in here.
        // The endpoint tracks *configured caps*, not month-to-date usage,
        // so it can't satisfy the "本月消耗" card. `get_spending` is called on
        // demand by the usage page.

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

    /// The org's monthly spending limit, for the usage page's "本月已用 a / b".
    ///
    /// `Ok(None)` when no monthly limit is configured; `forbidden: true` when the
    /// caller's role may not read it. This is NOT a source for month-to-date
    /// usage: the OAuth API exposes configured caps, not a usage total.
    pub async fn get_spending(&self) -> Result<Option<OfoxSpending>, String> {
        let access_token = self.get_valid_access_token().await?;
        self.fetch_spending(&access_token).await
    }

    /// Fetch the org's spending limits from `/openapi/orgs/me/spending-limits`
    /// and reduce them to the single monthly figure the UI cares about.
    ///
    /// Returns:
    ///   - `Ok(Some(spending))` when the endpoint responded; populates
    ///     `forbidden=true` for member role's 403, or the monthly figures
    ///     when a `period == "monthly"` entry exists.
    ///   - `Ok(None)` when the response succeeded but no monthly entry is
    ///     configured.
    ///   - `Err(_)` for transport / parse failures — caller logs and moves on.
    async fn fetch_spending(&self, access_token: &str) -> Result<Option<OfoxSpending>, String> {
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

        spending_from_limits_body(&body)
    }

    pub fn get_user_info(&self) -> Option<OfoxUserInfo> {
        self.user_info.try_read().ok().and_then(|ui| ui.clone())
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.granted_scope
            .read()
            .is_ok_and(|granted| granted.split_whitespace().any(|value| value == scope))
    }

    /// 当前登录的是哪个账号（API key 按它的组织计费）。没登录或还没取到
    /// 用户信息时为 `None`。
    pub async fn current_key_owner(&self) -> Option<crate::settings::KeyOwner> {
        if *self.auth_state.read().await != OfoxAuthState::Active {
            return None;
        }
        self.user_info
            .read()
            .await
            .as_ref()
            .map(OfoxUserInfo::key_owner)
            .filter(|owner| owner.org_id.is_some() || owner.user_id.is_some())
    }

    #[cfg(test)]
    pub(crate) async fn set_signed_in_for_test(&self, user: OfoxUserInfo) {
        *self.user_info.write().await = Some(user);
        *self.auth_state.write().await = OfoxAuthState::Active;
    }

    /// Snapshot the current auth state and cached user info — the data the
    /// frontend needs to decide between "show profile", "show expired banner",
    /// and "show login page".
    pub async fn get_auth_status(&self) -> OfoxAuthStatus {
        OfoxAuthStatus {
            state: *self.auth_state.read().await,
            user: self.user_info.read().await.clone(),
            keychain_denied: self
                .keychain_denied
                .load(std::sync::atomic::Ordering::SeqCst),
        }
    }

    /// Fetch fresh user info from the API, refreshing the access token if needed,
    /// then update the in-memory cache and persist to disk.
    pub async fn refresh_user_info(&self) -> Result<OfoxUserInfo, String> {
        self.refresh_user_info_at(&crate::ofox_apex::user_info_url(), true)
            .await
    }
    async fn refresh_user_info_at(
        &self,
        url: &str,
        with_balance: bool,
    ) -> Result<OfoxUserInfo, String> {
        let access_token = self.get_valid_access_token().await?;
        let generation = self
            .auth_generation
            .load(std::sync::atomic::Ordering::SeqCst);
        let user = self
            .get_user_info_from_api_at(&access_token, url, with_balance)
            .await?;

        let _lock = self.refresh_lock.lock().await;
        if generation
            != self
                .auth_generation
                .load(std::sync::atomic::Ordering::SeqCst)
            || self
                .access_token
                .read()
                .await
                .as_ref()
                .is_none_or(|cached| cached.token != access_token)
        {
            return Err("auth_session_changed".into());
        }
        let previous_owner = self
            .user_info
            .read()
            .await
            .as_ref()
            .map(OfoxUserInfo::key_owner);
        // Persist alongside current tokens
        let current_at = {
            let at = self.access_token.read().await;
            at.as_ref().map(|c| (c.token.clone(), c.expires_at_ms))
        };
        let current_rt = self.refresh_token.read().await.clone();
        let current_scope = self
            .granted_scope
            .read()
            .map_err(|_| "scope_lock_failed")?
            .clone();
        if let Some((at_token, expires_at_ms)) = current_at {
            self.save_to_disk(
                &at_token,
                current_rt.as_deref(),
                expires_at_ms,
                &current_scope,
                Some(&user),
                false,
            )?;
        }

        if previous_owner != Some(user.key_owner()) {
            self.auth_generation
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        *self.user_info.write().await = Some(user.clone());
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
        let current_scope = self
            .granted_scope
            .read()
            .map(|s| s.clone())
            .unwrap_or_default();
        if let Err(e) = self.save_to_disk(
            "",
            current_rt.as_deref(),
            0,
            &current_scope,
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
        self.auth_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut pending) = self.pending_device_codes.try_write() {
            pending.clear();
        }
        if let Ok(mut scope) = self.granted_scope.write() {
            scope.clear();
        }
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

        *self
            .granted_scope
            .write()
            .map_err(|_| "scope_lock_failed")? = scope.to_string();

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
        *self
            .granted_scope
            .write()
            .map_err(|_| "scope_lock_failed")? = if has_any_token {
            metadata.scope.clone()
        } else {
            String::new()
        };

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
            *ui = if has_any_token {
                metadata.user.clone()
            } else {
                None
            };
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

/// Reduce a `/openapi/orgs/me/spending-limits` body to its `monthly` entry.
///
/// The endpoint can return multiple periods (`daily`, `weekly`, `monthly`); only
/// monthly is surfaced. `Ok(None)` when no monthly limit is configured.
fn spending_from_limits_body(body: &str) -> Result<Option<OfoxSpending>, String> {
    let parsed: SpendingLimitsResponse =
        serde_json::from_str(body).map_err(|e| format!("Failed to parse spending: {e}"))?;
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

fn compute_expires_at_ms(expires_in: Option<i64>) -> i64 {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let secs = expires_in.unwrap_or(3600);
    now_ms + secs * 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile_fixture() -> OfoxUserInfo {
        serde_json::from_value(
            serde_json::json!({"id":"test-user","org_id":"test-org","email":"test@example.test"}),
        )
        .unwrap()
    }
    async fn additional_manager(temp: &tempfile::TempDir) -> OfoxAuthManager {
        let manager = seeded_manager(temp).await;
        let user = profile_fixture();
        *manager.user_info.write().await = Some(user.clone());
        manager
            .save_to_disk(
                "old-at",
                Some("rt-1"),
                chrono::Utc::now().timestamp_millis() + 60_000,
                OFOX_LEGACY_SCOPES,
                Some(&user),
                false,
            )
            .unwrap();
        manager
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn refreshing_an_old_consent_cannot_add_server_returned_scopes() {
        let _sandbox = crate::test_sandbox::sandbox();
        use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":"rotated-at","refresh_token":"rotated-rt","expires_in":3600,"scope":"org.read offline_access llm.invoke"}))).expect(1).mount(&server).await;
        let tmp = tempfile::tempdir().unwrap();
        let manager = seeded_manager(&tmp).await;
        *manager.granted_scope.write().unwrap() = "org.read offline_access".into();
        manager
            .refresh_access_token_at(&server.uri())
            .await
            .unwrap();
        assert!(!manager.has_scope("llm.invoke"));
        assert!(manager.has_scope("org.read"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn a_late_profile_response_cannot_expire_or_overwrite_an_upgraded_session() {
        let _sandbox = crate::test_sandbox::sandbox();
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        for status in [401, 200] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/old-profile"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_delay(std::time::Duration::from_millis(80))
                        .set_body_json(serde_json::json!({"id":"test-user","org_id":"old-org"})),
                )
                .mount(&server)
                .await;
            let tmp = tempfile::tempdir().unwrap();
            let manager = Arc::new(additional_manager(&tmp).await);
            manager
                .access_token
                .write()
                .await
                .as_mut()
                .unwrap()
                .expires_at_ms = chrono::Utc::now().timestamp_millis() + 3_600_000;
            let worker = manager.clone();
            let url = format!("{}/old-profile", server.uri());
            let refresh =
                tokio::spawn(async move { worker.refresh_user_info_at(&url, false).await });
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while server.received_requests().await.unwrap().is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            {
                let _lock = manager.refresh_lock.lock().await;
                manager
                    .save_to_disk(
                        "new-at",
                        Some("new-rt"),
                        chrono::Utc::now().timestamp_millis() + 3_600_000,
                        OFOX_SCOPES,
                        Some(&profile_fixture()),
                        false,
                    )
                    .unwrap();
                *manager.access_token.write().await = Some(CachedAccessToken {
                    token: "new-at".into(),
                    expires_at_ms: chrono::Utc::now().timestamp_millis() + 3_600_000,
                });
                *manager.refresh_token.write().await = Some("new-rt".into());
                manager
                    .auth_generation
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            let _ = refresh.await.unwrap();
            assert!(
                manager.is_authenticated(),
                "late status {status} expired the new grant"
            );
            assert_eq!(
                manager
                    .access_token
                    .read()
                    .await
                    .as_ref()
                    .map(|c| c.token.as_str()),
                Some("new-at")
            );
            assert_eq!(
                manager.get_user_info().unwrap().org_id.as_deref(),
                Some("test-org")
            );
            assert!(manager.has_scope("llm.invoke"));
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn device_login_network_or_other_errors_never_downgrade_scope() {
        let _sandbox = crate::test_sandbox::sandbox();
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        for (status, error) in [
            (500, "invalid_scope"),
            (400, "invalid_client"),
            (403, "invalid_scope"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/device"))
                .respond_with(
                    ResponseTemplate::new(status).set_body_json(serde_json::json!({"error":error})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let temp = tempfile::tempdir().unwrap();
            let manager = additional_manager(&temp).await;
            assert!(manager
                .start_device_flow_at(&format!("{}/device", server.uri()))
                .await
                .is_err());
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
            assert!(!manager.has_scope("llm.invoke"));
            assert!(manager.is_authenticated());
        }
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn additional_grant_denial_mismatch_and_cancel_keep_the_original_account() {
        let _sandbox = crate::test_sandbox::sandbox();
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        for scenario in ["denied", "mismatch", "cancel", "success"] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).and(path("/device")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"device_code":"additional","user_code":"ABCD","verification_uri":"https://example.test/verify","expires_in":600,"interval":5}))).mount(&server).await;
            Mock::given(method("POST")).and(path("/token")).respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(30)).set_body_json(serde_json::json!({"access_token":"new-at","refresh_token":"new-rt","expires_in":3600}))).mount(&server).await;
            let response = if scenario == "denied" {
                ResponseTemplate::new(401)
            } else {
                ResponseTemplate::new(200).set_body_json(if scenario == "mismatch" {
                    serde_json::json!({"id":"other-user","org_id":"test-org"})
                } else {
                    serde_json::to_value(profile_fixture()).unwrap()
                })
            };
            Mock::given(method("GET"))
                .and(path("/me"))
                .respond_with(response)
                .mount(&server)
                .await;
            let temp = tempfile::tempdir().unwrap();
            let manager = Arc::new(additional_manager(&temp).await);
            manager
                .start_device_flow_with(&format!("{}/device", server.uri()), true)
                .await
                .unwrap();
            let worker = manager.clone();
            let base = server.uri();
            let poll = tokio::spawn(async move {
                worker
                    .poll_for_token_at(
                        "additional",
                        &format!("{base}/token"),
                        &format!("{base}/me"),
                        false,
                    )
                    .await
            });
            if scenario == "cancel" {
                // Wait for the request to start, then revoke its pending receipt.
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    while server.received_requests().await.unwrap().len() < 2 {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                manager.cancel_device_flow("additional").await;
            }
            let result = poll.await.unwrap();
            assert_eq!(result.is_ok(), scenario == "success", "{scenario}");
            assert!(manager.is_authenticated());
            assert_eq!(
                manager.get_user_info().unwrap().id.as_deref(),
                Some("test-user")
            );
            assert_eq!(
                manager.access_token.read().await.as_ref().unwrap().token,
                if scenario == "success" {
                    "new-at"
                } else {
                    "old-at"
                }
            );
            assert_eq!(
                manager
                    .secret_store
                    .load(crate::ofox_secret::Slot::RefreshToken)
                    .unwrap()
                    .as_deref(),
                Some(if scenario == "success" {
                    "new-rt"
                } else {
                    "rt-1"
                })
            );
            assert_eq!(manager.has_scope("llm.invoke"), scenario == "success");
        }
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn a_fallback_grant_without_scope_cannot_gain_assistant_permission() {
        let _sandbox = crate::test_sandbox::sandbox();
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"access_token":"legacy-at","refresh_token":"legacy-rt"}),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::to_value(profile_fixture()).unwrap()),
            )
            .mount(&server)
            .await;
        let temp = tempfile::tempdir().unwrap();
        let manager = additional_manager(&temp).await;
        manager.pending_device_codes.write().await.insert(
            "legacy".into(),
            PendingDeviceCode {
                expires_at_ms: chrono::Utc::now().timestamp_millis() + 60_000,
                scope: OFOX_LEGACY_SCOPES.into(),
                expected_owner: None,
                generation: 0,
                apex: crate::ofox_apex::current_apex().into(),
            },
        );
        manager
            .poll_for_token_at(
                "legacy",
                &format!("{}/token", server.uri()),
                &format!("{}/me", server.uri()),
                false,
            )
            .await
            .unwrap();
        assert!(!manager.has_scope("llm.invoke"));
        assert!(manager.is_authenticated());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn device_login_falls_back_once_only_for_explicit_invalid_scope() {
        let _sandbox = crate::test_sandbox::sandbox();
        use wiremock::{
            matchers::{body_string_contains, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/device"))
            .and(body_string_contains("llm.invoke"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"error":"invalid_scope"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/device")).and(body_string_contains("scope=org.read+balance.read+apikey.write+offline_access"))
            .and(wiremock::matchers::body_string_contains("client_id="))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"device_code":"fallback-device","user_code":"ABCD","verification_uri":"https://example.test/verify","expires_in":600,"interval":5}))).mount(&server).await;
        let temp = tempfile::tempdir().unwrap();
        let manager = OfoxAuthManager::new_with_secret_store(
            temp.path().into(),
            Arc::new(crate::ofox_secret::InMemoryStore::new()),
        );
        assert!(manager
            .start_device_flow_at(&format!("{}/device", server.uri()))
            .await
            .is_ok());
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    use crate::ofox_secret::SecretStore;

    #[tokio::test]
    async fn expiring_and_restarting_an_old_grant_cannot_add_llm_invoke() {
        let temp = tempfile::tempdir().unwrap();
        let secrets = Arc::new(crate::ofox_secret::InMemoryStore::new());
        let manager = OfoxAuthManager::new_with_secret_store(temp.path().into(), secrets.clone());
        manager
            .save_to_disk(
                "test-access",
                Some("test-refresh"),
                1,
                "org.read offline_access",
                None,
                false,
            )
            .unwrap();
        assert!(!manager.has_scope("llm.invoke"));
        manager.mark_expired().await;
        assert!(!manager.has_scope("llm.invoke"));
        let restored = OfoxAuthManager::new_with_secret_store(temp.path().into(), secrets);
        assert!(!restored.has_scope("llm.invoke"));
    }

    #[test]
    fn test_compute_expires_at_ms() {
        let result = compute_expires_at_ms(Some(3600));
        let now = chrono::Utc::now().timestamp_millis();
        assert!(result > now + 3500 * 1000);
        assert!(result < now + 3700 * 1000);
    }

    #[test]
    fn spending_serializes_in_camel_case() {
        let spending = OfoxSpending {
            forbidden: false,
            monthly_used_usd: Some(12.5),
            monthly_limit_usd: Some(100.0),
        };

        assert_eq!(
            serde_json::to_value(&spending).unwrap(),
            serde_json::json!({
                "forbidden": false,
                "monthlyUsedUsd": 12.5,
                "monthlyLimitUsd": 100.0,
            })
        );
    }

    #[test]
    fn spending_is_the_monthly_entry_of_the_limits() {
        let body = r#"{"spending_limits":[
            {"period":"daily","used_usd":1.0,"limit_usd":5.0},
            {"period":"monthly","used_usd":12.5,"limit_usd":100.0}]}"#;

        assert_eq!(
            spending_from_limits_body(body).unwrap(),
            Some(OfoxSpending {
                forbidden: false,
                monthly_used_usd: Some(12.5),
                monthly_limit_usd: Some(100.0),
            })
        );
    }

    #[test]
    fn no_monthly_limit_is_no_spending() {
        let body = r#"{"spending_limits":[{"period":"daily","used_usd":1.0,"limit_usd":5.0}]}"#;

        assert_eq!(spending_from_limits_body(body).unwrap(), None);
        assert_eq!(spending_from_limits_body(r#"{}"#).unwrap(), None);
    }

    #[test]
    fn a_body_that_is_not_limits_is_an_error() {
        assert!(spending_from_limits_body("<html>").is_err());
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
    async fn api_keys_belong_to_the_billing_org_of_the_signed_in_user() {
        // Bodies exactly as documented for /openapi/me and /openapi/orgs/me/balance.
        let mut user: OfoxUserInfo = serde_json::from_str(
            r#"{"id":"user_abc","email":"alice@example.com","name":"Alice","avatar_url":null,"role":"owner"}"#,
        )
        .unwrap();
        user.balance = Some(
            serde_json::from_str(
                r#"{"organization_id":"org_xyz","balance":75.0,"total":100.0,"used":25.0,"currency":"USD"}"#,
            )
            .unwrap(),
        );

        let temp = tempfile::tempdir().unwrap();
        let manager = OfoxAuthManager::new_with_secret_store(
            temp.path().to_path_buf(),
            Arc::new(crate::ofox_secret::InMemoryStore::new()),
        );
        assert_eq!(manager.current_key_owner().await, None, "signed out");

        manager.set_signed_in_for_test(user).await;
        assert_eq!(
            manager.current_key_owner().await,
            Some(crate::settings::KeyOwner {
                org_id: Some("org_xyz".into()),
                user_id: Some("user_abc".into()),
            })
        );
    }

    /// 造一个"临期 AT + 可用 RT + Active"的 manager,复现刷新窗口内的状态。
    async fn seeded_manager(temp: &tempfile::TempDir) -> OfoxAuthManager {
        let manager = OfoxAuthManager::new_with_secret_store(
            temp.path().to_path_buf(),
            Arc::new(crate::ofox_secret::InMemoryStore::new()),
        );
        {
            let mut at = manager.access_token.write().await;
            *at = Some(CachedAccessToken {
                token: "old-at".into(),
                // 10s 后过期——落在 TOKEN_REFRESH_BUFFER_MS(60s)窗口内
                expires_at_ms: chrono::Utc::now().timestamp_millis() + 10_000,
            });
        }
        {
            let mut rt = manager.refresh_token.write().await;
            *rt = Some("rt-1".into());
        }
        manager.set_state(OfoxAuthState::Active).await;
        manager
    }

    #[tokio::test]
    async fn test_concurrent_refresh_single_flights() {
        // 复现 12:03:13 事故:两个 30min 定时器同秒并发进刷新。单飞锁下
        // 只有一个真实请求打到 IDP(`.expect(1)`),后到者复用新 AT,会话
        // 保持 Active,RT 换成轮转后的新值。
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let temp = tempfile::tempdir().unwrap();
        let manager = seeded_manager(&temp).await;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "new-at",
                "refresh_token": "rt-2",
                "expires_in": 3600,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/api/oauth/token", server.uri());
        let (r1, r2) = tokio::join!(
            manager.refresh_access_token_at(&url),
            manager.refresh_access_token_at(&url),
        );
        assert!(r1.is_ok(), "first refresh failed: {r1:?}");
        assert!(r2.is_ok(), "second refresh failed: {r2:?}");

        assert!(manager.is_authenticated(), "session must stay Active");
        assert_eq!(
            manager.refresh_token.read().await.as_deref(),
            Some("rt-2"),
            "RT should be the rotated one"
        );
        assert_eq!(
            manager.access_token.read().await.as_ref().unwrap().token,
            "new-at"
        );
        // MockServer drop 校验 expect(1)——多于一次请求会 panic
    }

    #[tokio::test]
    async fn test_invalid_grant_marks_session_expired() {
        // 真失效路径不被单飞/兜底逻辑误伤:RT 未被并发轮转、IDP 判
        // invalid_grant → 状态必须转 Expired,让 UI 引导重登。
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let temp = tempfile::tempdir().unwrap();
        let manager = seeded_manager(&temp).await;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/oauth/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/api/oauth/token", server.uri());
        let err = manager.refresh_access_token_at(&url).await.unwrap_err();
        assert_eq!(err, "invalid_grant");

        let status = manager.get_auth_status().await;
        assert_eq!(status.state, OfoxAuthState::Expired);
        // Expired 后 get_valid_access_token 硬短路,不再撞 IDP
        assert_eq!(
            manager.get_valid_access_token().await.unwrap_err(),
            "session_expired"
        );
    }

    /// A keychain whose prompt the user denied, until they allow it.
    struct DeniedKeychain {
        allowed: std::sync::atomic::AtomicBool,
        tokens: crate::ofox_secret::InMemoryStore,
    }

    impl crate::ofox_secret::SecretStore for DeniedKeychain {
        fn load(&self, slot: crate::ofox_secret::Slot) -> Result<Option<String>, String> {
            if self.allowed.load(std::sync::atomic::Ordering::SeqCst) {
                self.tokens.load(slot)
            } else {
                Err(format!(
                    "{}: keychain access denied",
                    crate::ofox_secret::KEYCHAIN_DENIED
                ))
            }
        }
        fn save(&self, slot: crate::ofox_secret::Slot, value: &str) -> Result<(), String> {
            self.tokens.save(slot, value)
        }
        fn clear(&self, slot: crate::ofox_secret::Slot) -> Result<(), String> {
            self.tokens.clear(slot)
        }
    }

    #[tokio::test]
    async fn a_denied_keychain_is_reported_and_allowing_it_restores_the_session() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("ofox_auth.json"),
            serde_json::json!({
                "schema_version": 1,
                "expires_at": 0,
                "scope": "",
                "user": null,
                "expired": false,
            })
            .to_string(),
        )
        .unwrap();
        let keychain = Arc::new(DeniedKeychain {
            allowed: std::sync::atomic::AtomicBool::new(false),
            tokens: crate::ofox_secret::InMemoryStore::new(),
        });
        keychain
            .tokens
            .save(crate::ofox_secret::Slot::RefreshToken, "rt")
            .unwrap();
        let manager = OfoxAuthManager::new_with_secret_store(
            temp.path().to_path_buf(),
            Arc::clone(&keychain) as Arc<dyn crate::ofox_secret::SecretStore>,
        );

        let denied = manager.get_auth_status().await;
        assert_eq!(denied.state, OfoxAuthState::LoggedOut);
        assert!(denied.keychain_denied);

        keychain
            .allowed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let restored = manager.retry_keychain().await;
        assert_eq!(restored.state, OfoxAuthState::Active);
        assert!(!restored.keychain_denied);
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
        let manager =
            OfoxAuthManager::new_with_secret_store(temp.path().to_path_buf(), Arc::clone(&store));

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

        let _manager =
            OfoxAuthManager::new_with_secret_store(temp.path().to_path_buf(), Arc::clone(&store));

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
