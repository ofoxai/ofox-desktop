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
/// 端点在 ofox.ai gateway 上不是匿名公开的——至少 `/gemini/...` 路径会返回
/// 401 If you don't provide an API key。因此我们尽可能附带当前 OfoxAI
/// OAuth 会话的 access_token。拿不到时（未登录 / 刷新失败）就匿名发出去，
/// 让 gateway 的 401 透传给前端，由现有的错误提示文案处理。
#[tauri::command(rename_all = "camelCase")]
pub async fn fetch_ofox_models(
    protocol: String,
    ofox_state: State<'_, OfoxAuthState>,
) -> Result<Vec<FetchedModel>, String> {
    // 用一个独立作用域释放读锁，避免在等待 HTTP 时一直占着 manager。
    let access_token: Option<String> = {
        let manager = ofox_state.0.read().await;
        manager.get_valid_access_token().await.ok()
    };
    model_fetch::fetch_ofox_models(&protocol, access_token.as_deref()).await
}
