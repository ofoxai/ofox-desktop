//! 模型列表获取命令
//!
//! 提供 Tauri 命令，供前端在供应商表单中获取可用模型列表。

use tauri::State;

use crate::commands::ofox_auth::OfoxAuthState;
use crate::services::model_fetch::{self, FetchedModel};

/// 获取供应商的可用模型列表
///
/// 使用 OpenAI 兼容的 GET /v1/models 端点。
/// 主要面向第三方聚合站（硅基流动、OpenRouter 等）。
#[tauri::command(rename_all = "camelCase")]
pub async fn fetch_models_for_config(
    base_url: String,
    api_key: String,
    is_full_url: Option<bool>,
) -> Result<Vec<FetchedModel>, String> {
    model_fetch::fetch_models(&base_url, &api_key, is_full_url.unwrap_or(false)).await
}

/// 从 Ofox 获取可用模型列表
///
/// protocol: "openai" | "anthropic" | "gemini"
///
/// Public catalog requests start anonymously; on 401/403 the service retries
/// with the OAuth token. An explicit refresh bypasses the recent cache.
#[tauri::command(rename_all = "camelCase")]
pub async fn fetch_ofox_models(
    protocol: String,
    force_refresh: Option<bool>,
    ofox_state: State<'_, OfoxAuthState>,
) -> Result<Vec<FetchedModel>, String> {
    // 用一个独立作用域释放读锁，避免在等待 HTTP 时一直占着 manager。
    let access_token: Option<String> = {
        let manager = ofox_state.0.read().await;
        manager.get_valid_access_token().await.ok()
    };
    if force_refresh.unwrap_or(false) {
        model_fetch::refresh_ofox_models(&protocol, access_token.as_deref()).await
    } else {
        model_fetch::fetch_ofox_models(&protocol, access_token.as_deref()).await
    }
}
