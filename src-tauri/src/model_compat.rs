//! Small, read-only streaming probes for Ofox-managed model selections.
//! Never log a key or response body and never retry a real user request.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::StreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app_config::BindableTool;
use crate::ofox_secret::Slot;
use crate::services::model_fetch::FetchedModel;

const CHAT: &str = "chatCompletions";
const RESPONSES: &str = "responses";
const ANTHROPIC: &str = "anthropic";
const GEMINI: &str = "gemini";
const CACHE_DAYS: i64 = 7;
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_STREAM_BYTES: usize = 512 * 1024;
static CACHE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityResult {
    pub app: String,
    pub model: String,
    pub protocol: Option<String>,
    /// compatible, incompatible, or inconclusive.
    pub status: String,
    /// catalog, cache, probe, or manual.
    pub source: String,
    pub reason: Option<String>,
    pub allowed_protocols: Vec<String>,
}

impl CompatibilityResult {
    fn new(
        tool: BindableTool,
        model: &str,
        protocol: Option<&str>,
        status: &str,
        source: &str,
        reason: Option<&str>,
    ) -> Self {
        Self {
            app: tool.as_str().into(),
            model: model.into(),
            protocol: protocol.map(str::to_string),
            status: status.into(),
            source: source.into(),
            reason: reason.map(str::to_string),
            allowed_protocols: Vec::new(),
        }
    }

    fn with_allowed(mut self, protocols: &[&str]) -> Self {
        self.allowed_protocols = protocols
            .iter()
            .map(|protocol| (*protocol).into())
            .collect();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CacheFile {
    entries: HashMap<String, CacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    protocol: String,
    checked_at: DateTime<Utc>,
}

fn cache_path() -> std::path::PathBuf {
    crate::config::get_app_config_dir().join("model-compat-cache.json")
}

fn cache_key(tool: BindableTool, model: &FetchedModel) -> String {
    let mut endpoints = model.supported_endpoints.clone().unwrap_or_default();
    endpoints.sort();
    format!(
        "{}|{}|{}|{}",
        crate::ofox_apex::gateway_base(),
        tool.as_str(),
        model.id,
        endpoints.join(",")
    )
}

fn load_cache() -> CacheFile {
    std::fs::read(cache_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn cached_protocol(cache: &CacheFile, key: &str, now: DateTime<Utc>) -> Option<String> {
    let entry = cache.entries.get(key)?;
    ((now - entry.checked_at).num_seconds() < CACHE_DAYS * 86_400 && entry.checked_at <= now)
        .then(|| entry.protocol.clone())
}

fn save_success(key: String, protocol: &str) {
    let Ok(_guard) = CACHE_LOCK.lock() else {
        return;
    };
    let mut cache = load_cache();
    cache.entries.insert(
        key,
        CacheEntry {
            protocol: protocol.into(),
            checked_at: Utc::now(),
        },
    );
    if let Err(error) = crate::config::write_json_file(&cache_path(), &cache) {
        log::warn!("Could not save model compatibility cache: {error}");
    }
}

fn invalidate_cached_success(key: &str) -> Result<(), ()> {
    let _guard = CACHE_LOCK.lock().map_err(|_| ())?;
    let mut cache = load_cache();
    if cache.entries.remove(key).is_some() {
        crate::config::write_json_file(&cache_path(), &cache).map_err(|error| {
            log::warn!("Could not invalidate model compatibility cache: {error}");
        })?;
    }
    Ok(())
}

fn endpoint_for(protocol: &str) -> Option<&'static str> {
    match protocol {
        RESPONSES => Some("/v1/responses"),
        CHAT => Some("/v1/chat/completions"),
        ANTHROPIC => Some("/anthropic/v1/messages"),
        GEMINI => Some("/gemini/v1beta/models"),
        _ => None,
    }
}

fn protocols_for(
    tool: BindableTool,
    model: &FetchedModel,
) -> Result<Vec<&'static str>, &'static str> {
    let endpoints = model.supported_endpoints.as_deref();
    let advertised = |protocol| {
        endpoints
            .map(|items| {
                items
                    .iter()
                    .any(|item| item == endpoint_for(protocol).unwrap())
            })
            .unwrap_or(true)
    };
    let protocols = match tool {
        BindableTool::OpenCode => [RESPONSES, CHAT]
            .into_iter()
            .filter(|p| advertised(p))
            .collect(),
        BindableTool::Codex => [RESPONSES].into_iter().filter(|p| advertised(p)).collect(),
        BindableTool::OpenClaw | BindableTool::Hermes | BindableTool::WorkBuddy => {
            [CHAT].into_iter().filter(|p| advertised(p)).collect()
        }
        BindableTool::Claude => vec![ANTHROPIC],
        BindableTool::Gemini => vec![GEMINI],
    };
    if protocols.is_empty() {
        Err("模型目录未声明该客户端所需的协议端点")
    } else {
        Ok(protocols)
    }
}

fn tool_protocol(tool: BindableTool) -> &'static str {
    match tool {
        BindableTool::Claude => "anthropic",
        BindableTool::Gemini => "gemini",
        _ => "openai",
    }
}

pub async fn check(
    tool: BindableTool,
    model_id: &str,
    force_retest: bool,
    catalog_access_token: Option<&str>,
) -> CompatibilityResult {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return CompatibilityResult::new(
            tool,
            model_id,
            None,
            "incompatible",
            "catalog",
            Some("请先选择模型"),
        );
    }
    // The catalog may live on a different host from the dev inference gateway.
    // Never send a tool API key to it; follow the existing OAuth catalog path.
    let catalog = match crate::services::model_fetch::fetch_ofox_models(
        tool_protocol(tool),
        catalog_access_token,
    )
    .await
    {
        Ok(models) => models,
        Err(_) => {
            return CompatibilityResult::new(
                tool,
                model_id,
                None,
                "inconclusive",
                "catalog",
                Some("模型目录暂时不可用，请重试"),
            )
        }
    };
    let Some(model) = catalog.into_iter().find(|m| m.id == model_id) else {
        return CompatibilityResult::new(
            tool,
            model_id,
            None,
            "incompatible",
            "catalog",
            Some("模型不在当前客户端的 Ofox 目录中"),
        );
    };
    let protocols = match protocols_for(tool, &model) {
        Ok(protocols) => protocols,
        Err(reason) => {
            return CompatibilityResult::new(
                tool,
                model_id,
                None,
                "incompatible",
                "catalog",
                Some(reason),
            )
        }
    };
    let key = cache_key(tool, &model);
    if force_retest {
        // A failed explicit retest must not fall back to an older success
        // when the save command checks again without force_retest.
        if invalidate_cached_success(&key).is_err() {
            return CompatibilityResult::new(
                tool,
                model_id,
                None,
                "inconclusive",
                "cache",
                Some("无法清除旧检测结果，请重试"),
            )
            .with_allowed(&protocols);
        }
    } else {
        if let Some(protocol) = cached_protocol(&load_cache(), &key, Utc::now()) {
            if protocols.contains(&protocol.as_str()) {
                return CompatibilityResult::new(
                    tool,
                    model_id,
                    Some(&protocol),
                    "compatible",
                    "cache",
                    None,
                )
                .with_allowed(&protocols);
            }
        }
    }
    // A probe must not silently mint a new key. Binding owns key creation.
    let api_key = match crate::ofox_secret::default_store().load(Slot::ApiKey { tool }) {
        Ok(Some(key)) if !key.is_empty() => key,
        _ => {
            return CompatibilityResult::new(
                tool,
                model_id,
                None,
                "inconclusive",
                "probe",
                Some("未找到已绑定的 API Key，请重新绑定后检测"),
            )
            .with_allowed(&protocols)
        }
    };
    let mut allowed = protocols.clone();
    for &protocol in &protocols {
        match probe(protocol, model_id, &api_key).await {
            ProbeOutcome::Compatible => {
                save_success(key, protocol);
                return CompatibilityResult::new(
                    tool,
                    model_id,
                    Some(protocol),
                    "compatible",
                    "probe",
                    None,
                )
                .with_allowed(&allowed);
            }
            ProbeOutcome::Inconclusive(reason) => {
                return CompatibilityResult::new(
                    tool,
                    model_id,
                    Some(protocol),
                    "inconclusive",
                    "probe",
                    Some(reason),
                )
                .with_allowed(&allowed)
            }
            ProbeOutcome::Incompatible(reason)
                if protocol == CHAT || tool != BindableTool::OpenCode =>
            {
                return CompatibilityResult::new(
                    tool,
                    model_id,
                    Some(protocol),
                    "incompatible",
                    "probe",
                    Some(reason),
                )
                .with_allowed(&allowed);
            }
            ProbeOutcome::Incompatible(_) => {
                // A confirmed broken Responses stream may fall back to Chat,
                // but cannot later be selected manually as "unverified".
                allowed.retain(|candidate| *candidate != protocol);
            }
        }
    }
    CompatibilityResult::new(
        tool,
        model_id,
        None,
        "incompatible",
        "probe",
        Some("两个流式协议均不兼容"),
    )
    .with_allowed(&allowed)
}

enum ProbeOutcome {
    Compatible,
    Incompatible(&'static str),
    Inconclusive(&'static str),
}

fn failed_http(status: u16) -> ProbeOutcome {
    match status {
        404 => ProbeOutcome::Incompatible("模型或协议端点不存在"),
        401 | 403 => ProbeOutcome::Inconclusive("鉴权或权限不足，无法判定兼容性"),
        429 => ProbeOutcome::Inconclusive("请求被限流，请稍后重试"),
        _ => ProbeOutcome::Inconclusive("网关暂时无法完成检测，请稍后重试"),
    }
}

async fn probe(protocol: &str, model: &str, key: &str) -> ProbeOutcome {
    let base = crate::ofox_apex::gateway_base();
    let client = crate::proxy::http_client::get();
    let mut request = match protocol {
        RESPONSES => client.post(format!("{base}/v1/responses")).bearer_auth(key).json(&json!({
            "model": model, "input": "Reply with OK.", "stream": true, "max_output_tokens": 512
        })),
        CHAT => client.post(format!("{base}/v1/chat/completions")).bearer_auth(key).json(&json!({
            "model": model, "messages": [{"role": "user", "content": "Reply with OK."}], "stream": true, "max_tokens": 512
        })),
        ANTHROPIC => client.post(format!("{base}/anthropic/v1/messages")).header("x-api-key", key).header("anthropic-version", "2023-06-01").json(&json!({
            "model": model, "messages": [{"role": "user", "content": "Reply with OK."}], "stream": true, "max_tokens": 512
        })),
        GEMINI => client.post(format!("{base}/gemini/v1beta/models/{model}:streamGenerateContent?alt=sse")).header("x-goog-api-key", key).json(&json!({
            "contents": [{"parts": [{"text": "Reply with OK."}]}], "generationConfig": {"maxOutputTokens": 512}
        })),
        _ => return ProbeOutcome::Incompatible("不支持的协议"),
    };
    request = request.timeout(PROBE_TIMEOUT);
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => return ProbeOutcome::Inconclusive("网络错误或检测超时"),
    };
    if !response.status().is_success() {
        return failed_http(response.status().as_u16());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(part) = stream.next().await {
        let Ok(part) = part else {
            return ProbeOutcome::Inconclusive("流式连接中断");
        };
        if bytes.len() + part.len() > MAX_STREAM_BYTES {
            return ProbeOutcome::Inconclusive("检测流超过安全大小限制");
        }
        bytes.extend_from_slice(&part);
    }
    evaluate_stream(protocol, &String::from_utf8_lossy(&bytes))
}

fn evaluate_stream(protocol: &str, body: &str) -> ProbeOutcome {
    let mut text_seen = false;
    let mut reasoning_seen = false;
    let mut finished = false;
    let mut reasoning_ids = Vec::new();
    let mut reasoning_mismatch = false;
    let mut truncated = false;
    let mut error_event = false;
    for line in body.lines() {
        if line.trim_end_matches('\r') == "event: error" {
            error_event = true;
            continue;
        }
        let Some(data) = line.trim_end_matches('\r').strip_prefix("data:") else {
            continue;
        };
        let data = data.trim_start();
        if data == "[DONE]" {
            finished = true;
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if value.get("error").is_some()
            || matches!(
                value.get("type").and_then(Value::as_str),
                Some("error" | "response.failed" | "response.incomplete")
            )
            || error_event
        {
            if protocol == RESPONSES && is_reasoning_part_error(&value) {
                return ProbeOutcome::Incompatible("Responses 推理分片无法关联");
            }
            return ProbeOutcome::Inconclusive("流式响应包含错误或未完成事件");
        }
        match protocol {
            RESPONSES => {
                let event = value.get("type").and_then(Value::as_str).unwrap_or("");
                if event == "response.output_item.added" {
                    if value.pointer("/item/type").and_then(Value::as_str) == Some("reasoning") {
                        reasoning_seen = true;
                        if let Some(id) = value.pointer("/item/id").and_then(Value::as_str) {
                            reasoning_ids.push(id.to_string());
                        }
                    }
                } else if event.starts_with("response.reasoning_") {
                    reasoning_seen = true;
                    if let Some(id) = value.get("item_id").and_then(Value::as_str) {
                        if !reasoning_ids.iter().any(|known| known == id) {
                            reasoning_mismatch = true;
                        }
                    }
                }
                if event == "response.output_text.delta"
                    && value
                        .get("delta")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty())
                {
                    text_seen = true;
                }
                if event == "response.completed" {
                    finished = true;
                }
            }
            CHAT => {
                reasoning_seen |= value
                    .pointer("/choices/0/delta/reasoning_content")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty());
                if value
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty())
                {
                    text_seen = true;
                }
                if value
                    .pointer("/choices/0/finish_reason")
                    .is_some_and(|v| !v.is_null())
                {
                    finished = true;
                    truncated |= value
                        .pointer("/choices/0/finish_reason")
                        .and_then(Value::as_str)
                        == Some("length");
                }
            }
            ANTHROPIC => {
                if value.get("type").and_then(Value::as_str) == Some("content_block_delta")
                    && value
                        .pointer("/delta/text")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty())
                {
                    text_seen = true;
                }
                if value.get("type").and_then(Value::as_str) == Some("message_stop") {
                    finished = true;
                }
                truncated |= value.pointer("/delta/stop_reason").and_then(Value::as_str)
                    == Some("max_tokens");
            }
            GEMINI => {
                if value
                    .pointer("/candidates/0/content/parts/0/text")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty())
                {
                    text_seen = true;
                }
                if value
                    .pointer("/candidates/0/finishReason")
                    .is_some_and(|v| !v.is_null())
                {
                    finished = true;
                    truncated |= value
                        .pointer("/candidates/0/finishReason")
                        .and_then(Value::as_str)
                        == Some("MAX_TOKENS");
                }
            }
            _ => {}
        }
    }
    if reasoning_mismatch {
        ProbeOutcome::Incompatible("推理分片 ID 与起始事件不一致")
    } else if truncated {
        ProbeOutcome::Inconclusive("检测输出达到 token 上限，无法确定完整流兼容性")
    } else if !text_seen && reasoning_seen {
        ProbeOutcome::Inconclusive("检测只收到推理片段，未收到正文，请重试")
    } else if !text_seen {
        ProbeOutcome::Incompatible("流式响应缺少可读取的正文增量")
    } else if !finished {
        ProbeOutcome::Inconclusive("正文已返回但未看到正常结束事件")
    } else {
        ProbeOutcome::Compatible
    }
}

fn is_reasoning_part_error(value: &Value) -> bool {
    let message = value
        .pointer("/error/message")
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    message.contains("reasoning part") && message.contains("not found")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_reasoning_id_mismatch_is_rejected() {
        let stream = "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\",\"id\":\"signed-id\"}}\n\ndata: {\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"unsigned-id\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"OK\"}\n\ndata: {\"type\":\"response.completed\"}\n\n";
        assert!(matches!(
            evaluate_stream(RESPONSES, stream),
            ProbeOutcome::Incompatible(_)
        ));
    }

    #[test]
    fn known_glm_reasoning_error_falls_back_but_unknown_error_does_not() {
        let known = "event: error\ndata: {\"error\":{\"message\":\"reasoning part rs_123:0 not found\"}}\n\n";
        let unknown = "event: error\ndata: {\"error\":{\"message\":\"upstream unavailable\"}}\n\n";
        assert!(matches!(
            evaluate_stream(RESPONSES, known),
            ProbeOutcome::Incompatible(_)
        ));
        assert!(matches!(
            evaluate_stream(RESPONSES, unknown),
            ProbeOutcome::Inconclusive(_)
        ));
    }

    #[test]
    fn reasoning_only_stream_is_not_mislabelled_incompatible() {
        let responses = "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\",\"id\":\"rs_1\"}}\n\ndata: {\"type\":\"response.completed\"}\n\n";
        let chat = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\ndata: [DONE]\n\n";
        assert!(matches!(
            evaluate_stream(RESPONSES, responses),
            ProbeOutcome::Inconclusive(_)
        ));
        assert!(matches!(
            evaluate_stream(CHAT, chat),
            ProbeOutcome::Inconclusive(_)
        ));
    }

    #[test]
    fn normal_streams_are_accepted() {
        let responses = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"OK\"}\n\ndata: {\"type\":\"response.completed\"}\n\n";
        let chat = "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\ndata: [DONE]\n\n";
        assert!(matches!(
            evaluate_stream(RESPONSES, responses),
            ProbeOutcome::Compatible
        ));
        assert!(matches!(
            evaluate_stream(CHAT, chat),
            ProbeOutcome::Compatible
        ));
        let anthropic = "data:{\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"OK\"}}\n\ndata:{\"type\":\"message_stop\"}\n\n";
        let gemini = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"OK\"}]},\"finishReason\":\"STOP\"}]}\n\n";
        assert!(matches!(
            evaluate_stream(ANTHROPIC, anthropic),
            ProbeOutcome::Compatible
        ));
        assert!(matches!(
            evaluate_stream(GEMINI, gemini),
            ProbeOutcome::Compatible
        ));
    }

    #[test]
    fn chat_message_without_delta_is_rejected() {
        let stream = "data: {\"choices\":[{\"message\":{\"content\":\"OK\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        assert!(matches!(
            evaluate_stream(CHAT, stream),
            ProbeOutcome::Incompatible(_)
        ));
    }

    #[test]
    fn fixed_protocols_reject_mismatched_catalog_entries() {
        let model = FetchedModel {
            id: "x".into(),
            name: None,
            owned_by: None,
            pricing_prompt: None,
            supported_endpoints: Some(vec!["/v1/chat/completions".into()]),
            supported_parameters: None,
            input_modalities: None,
            output_modalities: None,
        };
        assert!(protocols_for(BindableTool::Codex, &model).is_err());
        assert_eq!(
            protocols_for(BindableTool::OpenCode, &model).unwrap(),
            vec![CHAT]
        );
        for tool in [
            BindableTool::OpenClaw,
            BindableTool::Hermes,
            BindableTool::WorkBuddy,
        ] {
            assert_eq!(protocols_for(tool, &model).unwrap(), vec![CHAT]);
        }
        assert_eq!(
            protocols_for(BindableTool::Claude, &model).unwrap(),
            vec![ANTHROPIC]
        );
        assert_eq!(
            protocols_for(BindableTool::Gemini, &model).unwrap(),
            vec![GEMINI]
        );
    }

    #[test]
    fn cache_expires_after_seven_days() {
        let now = Utc::now();
        let mut cache = CacheFile::default();
        cache.entries.insert(
            "x".into(),
            CacheEntry {
                protocol: CHAT.into(),
                checked_at: now - chrono::Duration::days(8),
            },
        );
        assert!(cached_protocol(&cache, "x", now).is_none());
    }

    #[test]
    fn transient_http_and_truncated_streams_are_not_declared_incompatible() {
        for status in [400, 401, 429, 500] {
            assert!(matches!(failed_http(status), ProbeOutcome::Inconclusive(_)));
        }
        let chat = "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"},\"finish_reason\":\"length\"}]}\n\n";
        assert!(matches!(
            evaluate_stream(CHAT, chat),
            ProbeOutcome::Inconclusive(_)
        ));
    }

    #[test]
    fn changing_catalog_endpoints_invalidates_cache_key() {
        let mut model = FetchedModel {
            id: "x".into(),
            name: None,
            owned_by: None,
            pricing_prompt: None,
            supported_endpoints: Some(vec!["/v1/responses".into()]),
            supported_parameters: None,
            input_modalities: None,
            output_modalities: None,
        };
        let previous = cache_key(BindableTool::OpenCode, &model);
        model
            .supported_endpoints
            .as_mut()
            .unwrap()
            .push("/v1/chat/completions".into());
        assert_ne!(previous, cache_key(BindableTool::OpenCode, &model));
    }
}
