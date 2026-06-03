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
const OFOX_SCOPES: &str = "org.read balance.read offline_access";
// TODO: 发布前替换为正式 client_id
const OFOX_CLIENT_ID: &str = "ofox_app_switch_desktop_dev";
const TOKEN_REFRESH_BUFFER_MS: i64 = 60_000;

// ==================== Data Structures ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfoxUserInfo {
    pub email: Option<String>,
    pub name: Option<String>,
    pub org_id: Option<String>,
    pub avatar_url: Option<String>,
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
    pending_device_codes: Arc<RwLock<HashMap<String, PendingDeviceCode>>>,
    http_client: Client,
}

impl OfoxAuthManager {
    pub fn new(app_config_dir: PathBuf) -> Self {
        let storage_path = app_config_dir.join("ofox_auth.json");

        let manager = Self {
            storage_path,
            access_token: Arc::new(RwLock::new(None)),
            refresh_token: Arc::new(RwLock::new(None)),
            user_info: Arc::new(RwLock::new(None)),
            pending_device_codes: Arc::new(RwLock::new(HashMap::new())),
            http_client: Client::new(),
        };

        if let Err(e) = manager.load_from_disk_sync() {
            log::warn!("[OfoxAuth] Failed to load stored auth: {e}");
        }

        manager
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

            // Persist
            self.save_to_disk(
                &token_resp.access_token,
                token_resp.refresh_token.as_deref(),
                expires_at_ms,
                token_resp.scope.as_deref().unwrap_or(OFOX_SCOPES),
                Some(&user),
            )?;

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
            "slow_down" => Err("slow_down".to_string()),
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
            other => Err(format!(
                "{other}: {}",
                err_resp.error_description.unwrap_or_default()
            )),
        }
    }

    // ==================== Token Refresh ====================

    /// Refresh the access token using the stored refresh token.
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

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
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
        )?;

        log::info!("[OfoxAuth] Access token refreshed successfully");
        Ok(())
    }

    /// Get a valid access token, refreshing if necessary.
    pub async fn get_valid_access_token(&self) -> Result<String, String> {
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

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("User info request failed: {status} - {text}"));
        }

        let user: OfoxUserInfo = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse user info: {e}"))?;

        Ok(user)
    }

    pub fn get_user_info(&self) -> Option<OfoxUserInfo> {
        self.user_info.try_read().ok().and_then(|ui| ui.clone())
    }

    // ==================== Status & Lifecycle ====================

    pub fn is_authenticated(&self) -> bool {
        if let Ok(rt) = self.refresh_token.try_read() {
            if rt.is_some() {
                return true;
            }
        }
        if let Ok(at) = self.access_token.try_read() {
            if at.is_some() {
                return true;
            }
        }
        false
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
    ) -> Result<(), String> {
        let store = OfoxTokenStore {
            access_token: access_token.to_string(),
            refresh_token: refresh_token.map(|s| s.to_string()),
            expires_at,
            scope: scope.to_string(),
            user: user.cloned(),
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

        if let Ok(mut at) = self.access_token.try_write() {
            *at = Some(CachedAccessToken {
                token: store.access_token,
                expires_at_ms: store.expires_at,
            });
        }
        if let Ok(mut rt) = self.refresh_token.try_write() {
            *rt = store.refresh_token;
        }
        if let Ok(mut ui) = self.user_info.try_write() {
            *ui = store.user;
        }

        log::info!("[OfoxAuth] Loaded auth state from disk");
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
