//! Ofox OAuth Tauri Commands
//!
//! Provides Ofox AI OAuth authentication commands exposed to the frontend.
//! Uses Device Authorization Grant (RFC 8628).

use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

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

/// Get cached Ofox user info (if authenticated).
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_get_user_info(
    state: State<'_, OfoxAuthState>,
) -> Result<Option<crate::ofox_auth::OfoxUserInfo>, String> {
    let manager = state.0.read().await;
    Ok(manager.get_user_info())
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
