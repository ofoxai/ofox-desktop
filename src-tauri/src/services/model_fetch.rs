//! 模型列表获取服务
//!
//! 通过 OpenAI 兼容的 GET /v1/models 端点获取供应商可用模型列表。
//! 主要面向第三方聚合站（硅基流动、OpenRouter 等）。

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 获取到的模型信息
///
/// `pricing_prompt` 是上游 `pricing.prompt`（per-token 输入价，字符串如
/// "0.000001"；免费模型为 "0"）。前端按数值排序挑「最便宜但不免费」的
/// 默认 model，所以 Rust 端不在这里做数值解析，原样透传——上游若改成对象
/// 形态或加单位也不会让我们出错。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedModel {
    pub id: String,
    pub owned_by: Option<String>,
    pub pricing_prompt: Option<String>,
    /// 上游 `supported_endpoints`——形如 `["/v1/chat/completions", "/v1/responses"]`。
    /// 前端按需过滤（例如 Codex 只能选带 `/v1/responses` 的模型）。上游未返
    /// 该字段（老 catalog / gemini 端点）时为 `None`；调用方按"未知则不过滤"
    /// 处理，避免安全降级把合法模型也砍掉。
    pub supported_endpoints: Option<Vec<String>>,
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
    pricing: Option<ModelPricing>,
    #[serde(default)]
    supported_endpoints: Option<Vec<String>>,
}

/// 上游 model 条目里的 pricing 子对象。只挑 `prompt` 字段——其余如
/// completion / cache / web_search 现阶段用不到，让 serde 忽略以避免
/// schema 漂移破坏反序列化。
#[derive(Debug, Deserialize)]
struct ModelPricing {
    prompt: Option<String>,
}

/// Gemini 原生的 models 响应格式
#[derive(Debug, Deserialize)]
struct GeminiModelsResponse {
    models: Option<Vec<GeminiModelEntry>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiModelEntry {
    /// 格式: "models/provider/model-name"，例如 `models/google/gemini-2.5-pro`
    name: String,
    owned_by: Option<String>,
    #[serde(default)]
    pricing: Option<ModelPricing>,
    /// Gemini catalog 端点当前不返 supported_endpoints，保留字段以便未来
    /// 上游对齐；serde(default) 让缺字段时安静降级为 None。
    #[serde(default, rename = "supportedEndpoints")]
    supported_endpoints: Option<Vec<String>>,
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

/// Ofox 各协议的模型列表端点。base 锁定到
/// [`crate::ofox_apex::models_catalog_base`]——**始终走线上**，
/// 不跟随 `OFOX_USE_LOCAL`，但仍遵守 apex 切换（ofox.ai / ofox.io）。
///
/// 为什么不沿用 [`crate::ofox_apex::gateway_base`]：本地 ofox-gateway 的
/// traefik 配置里 `ofox-api-router` 只抢 `/v1/models`，`/anthropic/v1/models`
/// 和 `/gemini/v1beta/models` 会落到 LLM 父路由 → gemini/anthropic 插件
/// default 分支 → 404。生产环境由 MSE 网关单独路由直转 ofox-core，避开了
/// traefik。dev 环境要么得自己改 4 份 configmap、要么让插件实现 list 端点，
/// 都比"列模型锁线上"重。模型 catalog 是只读跨地区共享的，没有 dev/prod
/// 数据差异，锁线上副作用最小。
fn ofox_openai_models_url() -> String {
    format!("{}/v1/models", crate::ofox_apex::models_catalog_base())
}

fn ofox_anthropic_models_url() -> String {
    format!(
        "{}/anthropic/v1/models",
        crate::ofox_apex::models_catalog_base()
    )
}

fn ofox_gemini_models_url() -> String {
    format!(
        "{}/gemini/v1beta/models",
        crate::ofox_apex::models_catalog_base()
    )
}

/// 从 Ofox 获取可用模型列表
///
/// 端点选择（对应 `https://ofox.ai/zh/docs/api/<protocol>/models`）：
/// - "openai":    GET /v1/models             —— OpenAI 兼容 data[] 格式
/// - "anthropic": GET /anthropic/v1/models   —— 同 data[] 格式
/// - "gemini":    GET /gemini/v1beta/models  —— models[] 原生格式，需把
///   `name = "models/provider/id"` 还原成 `provider/id`
///
/// `access_token` 是 OfoxAI OAuth 颁发的 access_token。三条端点在文档里都标
/// 注「无需 API Key」可匿名调用，但 dev 网关曾经短暂地对 /gemini 强制鉴权，
/// 故这里在拿得到 token 时统一带上 Bearer——线上无害（公开接口忽略），dev
/// 严格模式也能过。`None` 走匿名，由 gateway 决定是否放行。
pub async fn fetch_ofox_models(
    protocol: &str,
    access_token: Option<&str>,
) -> Result<Vec<FetchedModel>, String> {
    let client = crate::proxy::http_client::get();

    /// 把 access_token 统一拼成 Authorization 头加到请求上。
    fn with_auth(
        req: reqwest::RequestBuilder,
        access_token: Option<&str>,
    ) -> reqwest::RequestBuilder {
        match access_token {
            Some(t) if !t.is_empty() => req.header("Authorization", format!("Bearer {t}")),
            _ => req,
        }
    }

    match protocol {
        "openai" | "anthropic" => {
            let url = if protocol == "openai" {
                ofox_openai_models_url()
            } else {
                ofox_anthropic_models_url()
            };

            let response = with_auth(client.get(&url), access_token)
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
                    pricing_prompt: m.pricing.and_then(|p| p.prompt),
                    supported_endpoints: m.supported_endpoints,
                })
                .collect();

            models.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(models)
        }
        "gemini" => {
            let response = with_auth(client.get(ofox_gemini_models_url()), access_token)
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
                    // `name` 形如 `models/google/gemini-2.5-pro`，去掉 "models/" 前缀
                    // 后得到 `google/gemini-2.5-pro`，跟 openai 协议下的 vendor/id
                    // 格式一致——前端选择器统一按这个形态展示。
                    let id = m
                        .name
                        .strip_prefix("models/")
                        .unwrap_or(&m.name)
                        .to_string();
                    FetchedModel {
                        id,
                        owned_by: m.owned_by,
                        pricing_prompt: m.pricing.and_then(|p| p.prompt),
                        supported_endpoints: m.supported_endpoints,
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
            pricing_prompt: m.pricing.and_then(|p| p.prompt),
            supported_endpoints: m.supported_endpoints,
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

    #[test]
    fn test_parse_response_with_pricing() {
        // 真实 OfoxAI /v1/models 响应的精简形态——保留 pricing.prompt 是因为
        // 前端用它来挑「最便宜但不免费」的默认 model；其余字段被 serde 默认忽略。
        let json = r#"{"object":"list","data":[
            {"id":"anthropic/claude-haiku-4.5","object":"model","owned_by":"bedrock","pricing":{"prompt":"0.000001","completion":"0.000005"}},
            {"id":"anthropic/claude-opus-4.5","object":"model","owned_by":"bedrock","pricing":{"prompt":"0.000005","completion":"0.000025"}}
        ]}"#;
        let resp: ModelsResponse = serde_json::from_str(json).unwrap();
        let data = resp.data.unwrap();
        assert_eq!(data[0].pricing.as_ref().and_then(|p| p.prompt.as_deref()), Some("0.000001"));
        assert_eq!(data[1].pricing.as_ref().and_then(|p| p.prompt.as_deref()), Some("0.000005"));
    }

    #[test]
    fn test_parse_response_pricing_absent() {
        // 没有 pricing 字段时不应该报错——只是 pricing_prompt = None。
        let json = r#"{"object":"list","data":[{"id":"my-model","object":"model"}]}"#;
        let resp: ModelsResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data.unwrap()[0].pricing.is_none());
    }
}
