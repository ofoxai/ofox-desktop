//! 模型列表获取服务
//!
//! 通过 OpenAI 兼容的 GET /v1/models 端点获取供应商可用模型列表。
//! 主要面向第三方聚合站（硅基流动、OpenRouter 等）。

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 获取到的模型信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedModel {
    pub id: String,
    pub owned_by: Option<String>,
}

/// OpenAI 兼容的 /v1/models 响应格式
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Option<Vec<ModelEntry>>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
    owned_by: Option<String>,
}

/// Gemini 原生的 models 响应格式
#[derive(Debug, Deserialize)]
struct GeminiModelsResponse {
    models: Option<Vec<GeminiModelEntry>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiModelEntry {
    /// 格式: "models/provider/model-name"
    name: String,
    owned_by: Option<String>,
}

const FETCH_TIMEOUT_SECS: u64 = 15;

/// 过滤非聊天模型（embedding、图片生成、TTS 等）
fn is_chat_model(id: &str) -> bool {
    let lower = id.to_lowercase();
    // 排除 embedding 模型（如 text-embedding-v4、text-embedding-3-large）
    if lower.contains("embedding") {
        return false;
    }
    // 排除图片生成模型（如 gpt-image-1.5、gemini-2.5-flash-image）
    if lower.contains("gpt-image") || lower.ends_with("-image") || lower.contains("-image-") {
        return false;
    }
    // 排除语音/审核模型
    if lower.contains("tts")
        || lower.contains("dall-e")
        || lower.contains("whisper")
        || lower.contains("moderation")
    {
        return false;
    }
    true
}

/// Ofox 各协议的模型列表端点
const OFOX_OPENAI_MODELS_URL: &str = "https://api.ofox.ai/v1/models";
const OFOX_ANTHROPIC_MODELS_URL: &str = "https://api.ofox.ai/anthropic/v1/models";
const OFOX_GEMINI_MODELS_URL: &str = "https://api.ofox.ai/gemini/v1beta/models";

/// 从 Ofox 获取可用模型列表（公开接口，无需 API Key）
///
/// 根据 protocol 参数选择对应的端点：
/// - "openai": GET /v1/models（OpenAI 兼容格式）
/// - "anthropic": GET /anthropic/v1/models（data[] 格式，同 OpenAI）
/// - "gemini": GET /gemini/v1beta/models（models[] 格式，需适配）
pub async fn fetch_ofox_models(protocol: &str) -> Result<Vec<FetchedModel>, String> {
    let client = crate::proxy::http_client::get();

    match protocol {
        "openai" | "anthropic" => {
            let url = if protocol == "openai" {
                OFOX_OPENAI_MODELS_URL
            } else {
                OFOX_ANTHROPIC_MODELS_URL
            };

            let response = client
                .get(url)
                .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
                .send()
                .await
                .map_err(|e| format!("Request failed: {e}"))?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(format!("HTTP {status}: {body}"));
            }

            let resp: ModelsResponse = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse response: {e}"))?;

            let mut models: Vec<FetchedModel> = resp
                .data
                .unwrap_or_default()
                .into_iter()
                .filter(|m| is_chat_model(&m.id))
                .map(|m| FetchedModel {
                    id: m.id,
                    owned_by: m.owned_by,
                })
                .collect();

            models.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(models)
        }
        "gemini" => {
            let response = client
                .get(OFOX_GEMINI_MODELS_URL)
                .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
                .send()
                .await
                .map_err(|e| format!("Request failed: {e}"))?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(format!("HTTP {status}: {body}"));
            }

            let resp: GeminiModelsResponse = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse response: {e}"))?;

            let mut models: Vec<FetchedModel> = resp
                .models
                .unwrap_or_default()
                .into_iter()
                .map(|m| {
                    // Gemini name 格式为 "models/provider/model-name"，去掉 "models/" 前缀
                    let id = m
                        .name
                        .strip_prefix("models/")
                        .unwrap_or(&m.name)
                        .to_string();
                    FetchedModel {
                        id,
                        owned_by: m.owned_by,
                    }
                })
                .filter(|m| is_chat_model(&m.id))
                .collect();

            models.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(models)
        }
        _ => Err(format!("Unsupported protocol: {protocol}")),
    }
}

/// 获取供应商的可用模型列表
///
/// 使用 OpenAI 兼容的 GET /v1/models 端点。
pub async fn fetch_models(
    base_url: &str,
    api_key: &str,
    is_full_url: bool,
) -> Result<Vec<FetchedModel>, String> {
    if api_key.is_empty() {
        return Err("API Key is required to fetch models".to_string());
    }

    let models_url = build_models_url(base_url, is_full_url)?;
    let client = crate::proxy::http_client::get();

    let response = client
        .get(&models_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {body}"));
    }

    let resp: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    let mut models: Vec<FetchedModel> = resp
        .data
        .unwrap_or_default()
        .into_iter()
        .map(|m| FetchedModel {
            id: m.id,
            owned_by: m.owned_by,
        })
        .collect();

    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

/// 构造 /v1/models 的完整 URL
fn build_models_url(base_url: &str, is_full_url: bool) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');

    if trimmed.is_empty() {
        return Err("Base URL is empty".to_string());
    }

    if is_full_url {
        // 尝试从完整端点 URL 推导 API 根路径
        // 例如: https://proxy.example.com/v1/chat/completions → https://proxy.example.com/v1/models
        if let Some(idx) = trimmed.find("/v1/") {
            return Ok(format!("{}/v1/models", &trimmed[..idx]));
        }
        // 如果没有 /v1/ 路径，直接去掉最后一段路径
        if let Some(idx) = trimmed.rfind('/') {
            let root = &trimmed[..idx];
            if root.contains("://") && root.len() > root.find("://").unwrap() + 3 {
                return Ok(format!("{root}/v1/models"));
            }
        }
        return Err("Cannot derive models endpoint from full URL".to_string());
    }

    // 常规情况: base_url 是 API 根路径
    // 如果已经包含 /v1 路径，直接追加 /models
    if trimmed.ends_with("/v1") {
        return Ok(format!("{trimmed}/models"));
    }

    Ok(format!("{trimmed}/v1/models"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_models_url_basic() {
        assert_eq!(
            build_models_url("https://api.siliconflow.cn", false).unwrap(),
            "https://api.siliconflow.cn/v1/models"
        );
    }

    #[test]
    fn test_build_models_url_trailing_slash() {
        assert_eq!(
            build_models_url("https://api.example.com/", false).unwrap(),
            "https://api.example.com/v1/models"
        );
    }

    #[test]
    fn test_build_models_url_with_v1() {
        assert_eq!(
            build_models_url("https://api.example.com/v1", false).unwrap(),
            "https://api.example.com/v1/models"
        );
    }

    #[test]
    fn test_build_models_url_full_url() {
        assert_eq!(
            build_models_url("https://proxy.example.com/v1/chat/completions", true).unwrap(),
            "https://proxy.example.com/v1/models"
        );
    }

    #[test]
    fn test_build_models_url_empty() {
        assert!(build_models_url("", false).is_err());
    }

    #[test]
    fn test_parse_response() {
        let json = r#"{"object":"list","data":[{"id":"gpt-4","object":"model","owned_by":"openai"},{"id":"claude-3-sonnet","object":"model","owned_by":"anthropic"}]}"#;
        let resp: ModelsResponse = serde_json::from_str(json).unwrap();
        let data = resp.data.unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].id, "gpt-4");
        assert_eq!(data[0].owned_by.as_deref(), Some("openai"));
        assert_eq!(data[1].id, "claude-3-sonnet");
    }

    #[test]
    fn test_parse_response_no_owned_by() {
        let json = r#"{"object":"list","data":[{"id":"my-model","object":"model"}]}"#;
        let resp: ModelsResponse = serde_json::from_str(json).unwrap();
        let data = resp.data.unwrap();
        assert_eq!(data[0].id, "my-model");
        assert!(data[0].owned_by.is_none());
    }

    #[test]
    fn test_parse_response_empty_data() {
        let json = r#"{"object":"list","data":[]}"#;
        let resp: ModelsResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data.unwrap().is_empty());
    }

    #[test]
    fn test_parse_gemini_response() {
        let json = r#"{"models":[{"name":"models/google/gemini-3.1-pro","displayName":"Gemini 3.1 Pro","ownedBy":"google"}]}"#;
        let resp: GeminiModelsResponse = serde_json::from_str(json).unwrap();
        let data = resp.models.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].name, "models/google/gemini-3.1-pro");
        assert_eq!(data[0].owned_by.as_deref(), Some("google"));
    }

    #[test]
    fn test_gemini_model_name_strip_prefix() {
        let name = "models/google/gemini-3.1-pro";
        let id = name.strip_prefix("models/").unwrap_or(name);
        assert_eq!(id, "google/gemini-3.1-pro");
    }
}
