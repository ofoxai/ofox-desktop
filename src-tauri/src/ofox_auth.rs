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

// 开发环境：指向本地服务
// ofox-app (Next.js, pm2) → localhost:3000 对应线上 app.ofox.ai
// ofox-core (Go, pm2) → localhost:8080 对应线上 api.ofox.ai
// TODO: 发布前改回线上地址
const OFOX_DEVICE_AUTH_URL: &str = "http://localhost:3000/api/oauth/device_authorization";
const OFOX_TOKEN_URL: &str = "http://localhost:3000/api/oauth/token";
const OFOX_USER_INFO_URL: &str = "http://localhost:8080/openapi/me";
const OFOX_BALANCE_URL: &str = "http://localhost:8080/openapi/orgs/me/balance";
const OFOX_SPENDING_LIMITS_URL: &str =
    "http://localhost:8080/openapi/orgs/me/spending-limits";
const OFOX_SCOPES: &str = "org.read balance.read offline_access llm.invoke";
// TODO: 发布前替换为正式 client_id
const OFOX_CLIENT_ID: &str = "ofox_app_switch_desktop_dev";
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OfoxTokenStore {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    pub scope: String,
    pub user: Option<OfoxUserInfo>,
    /// Optional, present only for stores written by versions that track expired sessions.
    /// Older files on disk simply lack this field and are treated as `Active`.
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
}

impl OfoxAuthManager {
    pub fn new(app_config_dir: PathBuf) -> Self {
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
            .post(OFOX_DEVICE_AUTH_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("client_id", OFOX_CLIENT_ID),
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
            .post(OFOX_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
                ("client_id", OFOX_CLIENT_ID),
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

            // Fetch user info
            let user = self
                .get_user_info_from_api(&token_resp.access_token)
                .await?;
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
            .post(OFOX_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("client_id", OFOX_CLIENT_ID),
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
            .get(OFOX_USER_INFO_URL)
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
            .get(OFOX_BALANCE_URL)
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
            .get(OFOX_SPENDING_LIMITS_URL)
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

        if self.storage_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.storage_path) {
                log::warn!("[OfoxAuth] Failed to delete auth file: {e}");
            }
        }

        log::info!("[OfoxAuth] Logged out successfully");
    }

    // ==================== Persistence ====================

    fn save_to_disk(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: i64,
        scope: &str,
        user: Option<&OfoxUserInfo>,
        expired: bool,
    ) -> Result<(), String> {
        let store = OfoxTokenStore {
            access_token: access_token.to_string(),
            refresh_token: refresh_token.map(|s| s.to_string()),
            expires_at,
            scope: scope.to_string(),
            user: user.cloned(),
            expired,
        };

        let data = serde_json::to_string_pretty(&store)
            .map_err(|e| format!("Failed to serialize auth store: {e}"))?;

        crate::config::atomic_write(&self.storage_path, data.as_bytes())
            .map_err(|e| format!("Failed to write auth file: {e}"))?;

        log::debug!("[OfoxAuth] Auth state persisted to disk");
        Ok(())
    }

    fn load_from_disk_sync(&self) -> Result<(), String> {
        if !self.storage_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.storage_path)
            .map_err(|e| format!("Failed to read auth file: {e}"))?;

        let store: OfoxTokenStore = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse auth file: {e}"))?;

        // If we previously persisted an `expired` flag, restore that state so
        // the UI immediately knows to prompt for re-login instead of trying
        // (and failing) to refresh again on startup.
        let initial_state = if store.expired {
            OfoxAuthState::Expired
        } else if !store.access_token.is_empty() || store.refresh_token.is_some() {
            OfoxAuthState::Active
        } else {
            OfoxAuthState::LoggedOut
        };

        if let Ok(mut at) = self.access_token.try_write() {
            *at = if store.access_token.is_empty() {
                None
            } else {
                Some(CachedAccessToken {
                    token: store.access_token,
                    expires_at_ms: store.expires_at,
                })
            };
        }
        if let Ok(mut rt) = self.refresh_token.try_write() {
            *rt = store.refresh_token;
        }
        if let Ok(mut ui) = self.user_info.try_write() {
            *ui = store.user;
        }
        if let Ok(mut s) = self.auth_state.try_write() {
            *s = initial_state;
        }

        log::info!("[OfoxAuth] Loaded auth state from disk (state={initial_state:?})");
        Ok(())
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

    #[test]
    fn test_compute_expires_at_ms() {
        let result = compute_expires_at_ms(Some(3600));
        let now = chrono::Utc::now().timestamp_millis();
        assert!(result > now + 3500 * 1000);
        assert!(result < now + 3700 * 1000);
    }

    #[tokio::test]
    async fn test_manager_initial_state_not_authenticated() {
        let temp = tempfile::tempdir().unwrap();
        let manager = OfoxAuthManager::new(temp.path().to_path_buf());
        assert!(!manager.is_authenticated());
        assert!(manager.get_user_info().is_none());
    }
}
