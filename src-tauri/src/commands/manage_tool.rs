//! Tauri commands powering the "manage bound tool" dialog (Ofox tray/console).
//!
//! Three responsibilities:
//!   1. tell the UI where the live config file lives, so it can show the path
//!      and offer "open in Finder";
//!   2. read the active model the bound (ofox-*) provider is currently using
//!      so the dialog can pre-select it;
//!   3. write a chosen model back into that provider's `settings_config` and
//!      flush a fresh live snapshot.
//!
//! Scope: only the active provider for the given app is touched. We do not
//! switch providers, do not toggle takeover, and do not unbind here — those
//! belong to other commands (`switch_provider`, `set_takeover_for_app`,
//! `ofox_unbind_tool`).
//!
//! Layout note: model fields land in different shapes depending on the tool
//! (Anthropic env, Codex TOML, Gemini env, OpenCode dict, OpenClaw/Hermes
//! list). The read/write helpers below mirror what
//! `services/provider/live.rs` ultimately consumes.

use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::State;
use tokio::sync::RwLock;

use crate::app_config::AppType;
use crate::commands::ofox_auth::OfoxAuthState;
use crate::ofox_auth::OfoxAuthManager;
use crate::ofox_apex::gateway_base;
use crate::store::AppState;

// Gateway base URL for the connectivity probe. Mirrors the dev/prod toggle in
// `ofox_auth.rs` — auth flows go through ofox-core (:8080), but chat/messages
// flow through the gateway plugin (:8088 in dev, line-routed by Traefik in
// prod). Single source of truth lives in `crate::ofox_endpoints`; both this
// probe and the OFOX_SEEDS provider templates read from there so dev/prod
// flips happen in exactly one spot.

// ---------------------------------------------------------------------------
// 1) Config file path
// ---------------------------------------------------------------------------

/// Return the absolute path to the tool's primary live config file —
/// the same file that `services/provider/live.rs::write_live_snapshot`
/// ultimately writes to. The dialog displays this string and uses the
/// containing directory for the "在 Finder 打开" action.
///
/// For tools whose live state is split across multiple files (Codex
/// auth.json + config.toml, Gemini .env + settings.json), we return the
/// "settings/config" file — that's the one users would actually inspect.
#[tauri::command(rename_all = "camelCase")]
pub fn get_tool_config_file_path(app: String) -> Result<String, String> {
    let app_type = AppType::from_str(&app).map_err(|e| format!("无效的应用类型: {e}"))?;
    let path = match app_type {
        AppType::Claude => crate::config::get_claude_settings_path(),
        AppType::Codex => crate::codex_config::get_codex_config_path(),
        AppType::Gemini => crate::gemini_config::get_gemini_settings_path(),
        AppType::OpenCode => crate::opencode_config::get_opencode_config_path(),
        AppType::OpenClaw => crate::openclaw_config::get_openclaw_config_path(),
        AppType::Hermes => crate::hermes_config::get_hermes_config_path(),
    };
    Ok(path.to_string_lossy().to_string())
}

// ---------------------------------------------------------------------------
// 2) Read active model
// ---------------------------------------------------------------------------

/// Read the model currently configured on the app's active provider.
///
/// Returns `""` when:
///   - the provider has no model set (legitimate "use OfoxAI's default route"
///     state — the OfoxAI seeds ship with no model preselected);
///   - the underlying field doesn't exist yet (newly created provider).
///
/// Hard errors only when we cannot resolve the active provider at all
/// (unknown app, DB failure). The dialog treats `""` as "未设置" and
/// still allows the user to save a new value.
#[tauri::command(rename_all = "camelCase")]
pub fn get_active_ofox_model(
    state: State<'_, AppState>,
    app: String,
) -> Result<String, String> {
    let app_type = AppType::from_str(&app).map_err(|e| format!("无效的应用类型: {e}"))?;
    read_active_model_for(&state, &app_type)
}

/// Plain-reference helper extracted from [`get_active_ofox_model`] so
/// internal callers (notably `set_active_ofox_model` and `ofox_ping_model`)
/// can resolve "what model is the active provider pointing at" without going
/// through a Tauri command frame.
///
/// Returns `Ok("")` when the active provider exists but has no model field
/// configured (callers treat empty as "unset"); only surfaces `Err` for hard
/// DB failures or when there is no active provider.
fn read_active_model_for(
    state: &AppState,
    app_type: &AppType,
) -> Result<String, String> {
    let app_str = app_type.as_str();

    let provider_id = state
        .db
        .get_current_provider(app_str)
        .map_err(|e| format!("读取当前供应商失败: {e}"))?
        .ok_or_else(|| format!("{app_str} 暂无活跃供应商"))?;

    let provider = state
        .db
        .get_provider_by_id(&provider_id, app_str)
        .map_err(|e| format!("读取 {provider_id} 失败: {e}"))?
        .ok_or_else(|| format!("供应商 {provider_id} 不存在"))?;

    Ok(read_model_from_settings(app_type, &provider.settings_config))
}

// ---------------------------------------------------------------------------
// 3) Write active model
// ---------------------------------------------------------------------------

/// Write `model` into the active provider's settings_config at the path
/// appropriate for `app`, then refresh the on-disk live config so the tool
/// picks up the change immediately.
///
/// Empty `model` removes the field — keeps the on-disk config clean and
/// signals "fall back to OfoxAI's default routing".
///
/// For ofox-* providers this goes through the **bind 直写** path
/// ([`ProxyService::ofox_write_direct_to_live`])：拿 keychain 里的 sk-of-、
/// 把 DB 里的 settings_config（含新 model）+ token 合成完整磁盘 config 写盘。
/// 不调老的 `refresh_takeover_for_app`——那条会写 `PROXY_MANAGED` 占位符把
/// 真 sk-of- 覆盖掉，并触发 backup 删除（破坏 unbind 可恢复性）。
///
/// 非 ofox provider 走老路径——保留 takeover 自托管的兼容形态。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_active_ofox_model(
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
    app: String,
    model: String,
) -> Result<(), String> {
    let app_type = AppType::from_str(&app).map_err(|e| format!("无效的应用类型: {e}"))?;
    let app_str = app_type.as_str();

    let provider_id = state
        .db
        .get_current_provider(app_str)
        .map_err(|e| format!("读取当前供应商失败: {e}"))?
        .ok_or_else(|| format!("{app_str} 暂无活跃供应商"))?;

    let mut provider = state
        .db
        .get_provider_by_id(&provider_id, app_str)
        .map_err(|e| format!("读取 {provider_id} 失败: {e}"))?
        .ok_or_else(|| format!("供应商 {provider_id} 不存在"))?;

    write_model_into_settings(&app_type, &mut provider.settings_config, model.trim())?;

    state
        .db
        .update_provider_settings_config(app_str, &provider_id, &provider.settings_config)
        .map_err(|e| format!("更新 {provider_id} settings_config 失败: {e}"))?;

    if provider_id.starts_with("ofox-") {
        // ofox 直写路径：拿 keychain 里的 sk-of-（CachedOk——已经 bind 过的工具
        // 一定命中；命中不了说明 keychain 被清/迁移，这种异常态 fetch 会
        // 调 /openapi/api-keys 重新签发）+ 新 model 合成完整磁盘 config 写盘。
        let api_key = crate::ofox_api_keys::fetch_or_create_api_key(
            app_type,
            crate::ofox_api_keys::FetchMode::CachedOk,
            &ofox_state.0,
        )
        .await
        .map_err(|e| format!("获取 {app_str} OfoxAI API key 失败: {e}"))?;

        state
            .proxy_service
            .ofox_write_direct_to_live(&app_type, &api_key)
            .await
            .map_err(|e| format!("刷新 {app_str} live 配置失败: {e}"))?;
    } else {
        // 非 ofox provider 走老 takeover 路径——保留兼容形态。
        state
            .proxy_service
            .refresh_takeover_for_app(app_str)
            .await
            .map_err(|e| format!("刷新 {app_str} live 配置失败: {e}"))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers — model field shape per app
// ---------------------------------------------------------------------------

/// Read the model from a settings_config JSON, returning `""` for any
/// "absent / wrong shape" case rather than erroring — see [`get_active_ofox_model`]
/// for the rationale.
fn read_model_from_settings(app: &AppType, settings: &serde_json::Value) -> String {
    match app {
        AppType::Claude => settings
            .get("env")
            .and_then(|v| v.get("ANTHROPIC_MODEL"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        AppType::Gemini => settings
            .get("env")
            .and_then(|v| v.get("GEMINI_MODEL"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        AppType::Codex => {
            // Codex stores its config as a TOML *string* inside the
            // `config` key. Parse and read the top-level `model`.
            let cfg_text = settings.get("config").and_then(|v| v.as_str()).unwrap_or("");
            if cfg_text.is_empty() {
                return String::new();
            }
            toml::from_str::<toml::Value>(cfg_text)
                .ok()
                .and_then(|t| t.get("model").and_then(|v| v.as_str()).map(str::to_string))
                .unwrap_or_default()
        }
        AppType::OpenCode => settings
            .get("models")
            .and_then(|v| v.as_object())
            .and_then(|m| m.keys().next().cloned())
            .unwrap_or_default(),
        AppType::OpenClaw => settings
            .get("models")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        AppType::Hermes => settings
            .get("models")
            .and_then(|v| v.as_object())
            .and_then(|m| m.keys().next().cloned())
            .unwrap_or_default(),
    }
}

/// Write `model` into the right slot for `app`. Empty `model` removes the
/// field (keeps the stored JSON tidy and lets OfoxAI fall back to its
/// default-route behavior). Mutates `settings` in place.
fn write_model_into_settings(
    app: &AppType,
    settings: &mut serde_json::Value,
    model: &str,
) -> Result<(), String> {
    match app {
        AppType::Claude => set_env_string(settings, "ANTHROPIC_MODEL", model),
        AppType::Gemini => set_env_string(settings, "GEMINI_MODEL", model),
        AppType::Codex => set_codex_model(settings, model),
        AppType::OpenCode => {
            // Single-select: replace the entire `models` dict. Keys are model
            // names; the value object is empty (matches the OfoxAI seed).
            let obj = settings
                .as_object_mut()
                .ok_or_else(|| "settings_config 不是对象".to_string())?;
            if model.is_empty() {
                obj.insert("models".into(), serde_json::json!({}));
            } else {
                obj.insert(
                    "models".into(),
                    serde_json::json!({ model: serde_json::json!({}) }),
                );
            }
            Ok(())
        }
        AppType::OpenClaw => {
            // OpenClaw schema：`models: [{id: string, name: string, ...}]`
            // —— 对象数组，CLI runtime schema 要求 `name` **必填** string
            // （Rust `OpenClawModelEntry` 里 name 是 Option，但 CLI 端校验更严，
            // 缺 name 会报 "Invalid input: expected string, received undefined"
            // 后整个 config 失效）。name 没有更好来源时直接复用 id。
            let obj = settings
                .as_object_mut()
                .ok_or_else(|| "settings_config 不是对象".to_string())?;
            if model.is_empty() {
                obj.insert("models".into(), serde_json::json!([]));
            } else {
                obj.insert(
                    "models".into(),
                    serde_json::json!([{ "id": model, "name": model }]),
                );
            }
            Ok(())
        }
        AppType::Hermes => {
            // Hermes schema：`models: { <id>: { context_length?, ... } }`
            // —— dict，key 是 model id。
            let obj = settings
                .as_object_mut()
                .ok_or_else(|| "settings_config 不是对象".to_string())?;
            if model.is_empty() {
                obj.insert("models".into(), serde_json::json!({}));
            } else {
                obj.insert(
                    "models".into(),
                    serde_json::json!({ model: serde_json::json!({}) }),
                );
            }
            Ok(())
        }
    }
}

/// Set or remove `settings.env.<key>`. Creates the `env` object if missing.
fn set_env_string(
    settings: &mut serde_json::Value,
    key: &str,
    value: &str,
) -> Result<(), String> {
    if !settings.is_object() {
        *settings = serde_json::json!({});
    }
    let root = settings
        .as_object_mut()
        .expect("ensured object above");
    if !root.get("env").map(|v| v.is_object()).unwrap_or(false) {
        root.insert("env".into(), serde_json::json!({}));
    }
    let env = root
        .get_mut("env")
        .and_then(|v| v.as_object_mut())
        .expect("ensured object above");
    if value.is_empty() {
        env.remove(key);
    } else {
        env.insert(key.into(), serde_json::Value::String(value.into()));
    }
    Ok(())
}

/// Mutate the top-level `model` field inside the Codex `config` TOML
/// string, preserving every other key (`model_provider`, `wire_api`,
/// `[model_providers.*]`, …). Uses `toml_edit` so formatting and comments
/// survive untouched, matching what `codex_config.rs` does elsewhere.
fn set_codex_model(settings: &mut serde_json::Value, model: &str) -> Result<(), String> {
    use toml_edit::DocumentMut;

    let root = settings
        .as_object_mut()
        .ok_or_else(|| "settings_config 不是对象".to_string())?;
    let cfg_text = root
        .get("config")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut doc = if cfg_text.is_empty() {
        DocumentMut::new()
    } else {
        cfg_text
            .parse::<DocumentMut>()
            .map_err(|e| format!("解析 Codex config TOML 失败: {e}"))?
    };

    if model.is_empty() {
        doc.as_table_mut().remove("model");
    } else {
        doc["model"] = toml_edit::value(model);
    }

    root.insert("config".into(), serde_json::Value::String(doc.to_string()));
    Ok(())
}

// ---------------------------------------------------------------------------
// 4) Connectivity probe
// ---------------------------------------------------------------------------

/// Result of a ping — minimal shape that the dialog can render directly.
///
/// `success` is the only field the happy path needs; `latencyMs` powers the
/// "✓ 连通成功（123 ms）" subtitle, and `error` carries the human-readable
/// reason when `success = false`. Status code is surfaced separately so the
/// front-end can special-case 401 ("session expired, re-login") without
/// pattern-matching on Chinese error text.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub success: bool,
    pub latency_ms: u64,
    pub status_code: Option<u16>,
    pub error: Option<String>,
}

const PING_TIMEOUT_SECS: u64 = 20;

/// Send a 1-token completion request through the OfoxAI gateway to verify
/// the bound tool's protocol + chosen model are actually reachable end to
/// end.
///
/// `app` selects the wire protocol (Anthropic / OpenAI / Gemini). The bearer
/// is whatever token the active provider's `settings_config` exposes — the
/// same value the proxy itself injects on real requests, so a successful
/// ping proves the very same auth path the user's tool will use. We do NOT
/// fall back to the OAuth access_token: those are different scopes
/// (`/openapi/...` vs `/v1/...`) and mixing them is what produced the
/// `invalid_api_key` 401 we saw before this change.
///
/// We deliberately ask for the smallest possible response (max_tokens=1,
/// prompt = "hi") to keep cost negligible. 4xx other than 401 (typically
/// 404 unknown model) and 5xx are passed through with the body trimmed.
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_ping_model(
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
    app: String,
    model: String,
) -> Result<PingResult, String> {
    Ok(ofox_ping_model_internal(&state, &ofox_state.0, &app, &model).await)
}

/// Implementation of [`ofox_ping_model`] that takes plain references — kept
/// as a separate helper because Tauri commands can't construct
/// `tauri::State<'_>` from places that don't have the managed-state context.
/// Always returns `Ok`-shaped result; failures land in the `success: false`
/// variant of [`PingResult`].
async fn ofox_ping_model_internal(
    state: &AppState,
    manager_arc: &Arc<RwLock<OfoxAuthManager>>,
    app: &str,
    model: &str,
) -> PingResult {
    let app_type = match AppType::from_str(app) {
        Ok(t) => t,
        Err(e) => {
            return PingResult {
                success: false,
                latency_ms: 0,
                status_code: None,
                error: Some(format!("无效的应用类型: {e}")),
            };
        }
    };
    let app_str = app_type.as_str();

    let model = model.trim().to_string();
    if model.is_empty() {
        return PingResult {
            success: false,
            latency_ms: 0,
            status_code: None,
            error: Some("请先选择一个模型再测试连通性".to_string()),
        };
    }

    // Pull the API key the proxy would use for this app. We read the active
    // provider's settings_config rather than the static OfoxAI seed — that
    // way users who edited the seed (or bound to a non-Ofox provider) still
    // get a meaningful probe.
    let provider_id = match state.db.get_current_provider(app_str) {
        Ok(Some(id)) => id,
        Ok(None) => {
            return PingResult {
                success: false,
                latency_ms: 0,
                status_code: None,
                error: Some(format!("{app_str} 暂无活跃供应商")),
            };
        }
        Err(e) => {
            return PingResult {
                success: false,
                latency_ms: 0,
                status_code: None,
                error: Some(format!("读取当前供应商失败: {e}")),
            };
        }
    };
    let provider = match state.db.get_provider_by_id(&provider_id, app_str) {
        Ok(Some(p)) => p,
        Ok(None) => {
            return PingResult {
                success: false,
                latency_ms: 0,
                status_code: None,
                error: Some(format!("供应商 {provider_id} 不存在")),
            };
        }
        Err(e) => {
            return PingResult {
                success: false,
                latency_ms: 0,
                status_code: None,
                error: Some(format!("读取 {provider_id} 失败: {e}")),
            };
        }
    };

    let api_key = extract_api_key(&app_type, &provider.settings_config);

    // 对 `ofox-*` provider：用 keychain 里 cc-switch 创建的 sk-of- LLM API key
    // 探活，**不要**用 OAuth access_token。
    //
    // 历史 context：takeover 时代 `settings_config` 里存的是 OAuth access_token
    // 快照（proxy server 拿着它代为转发到 gateway），probe 时为了保证 token 没
    // 过期会走 `get_valid_access_token` 重新拉一份。**但 bind 直写改造后**，
    // 工具配置文件里写的是 sk-of- 明文，gateway 走 LLM API key 鉴权链；如果
    // probe 仍然用 access_token，会被 gateway 的 OAuth scope check 拦下报
    // `Token is missing the required scope "llm.invoke"`——access_token 只有
    // `org.read / balance.read / apikey.write / offline_access`，没有 LLM 调用
    // scope，**永远**过不了这道检查。
    //
    // 新路径：直接 `fetch_or_create_api_key(_, CachedOk, _)` 从 keychain 拿
    // bind 时落地的 sk-of-。CachedOk 模式下 keychain 命中立即返回不调端点；
    // 命中失败（理论上不该发生：能到 probe 阶段说明已 bind 过）会重新签发。
    let is_ofox_provider = provider_id.starts_with("ofox-");
    let api_key = if is_ofox_provider {
        match crate::ofox_api_keys::fetch_or_create_api_key(
            app_type,
            crate::ofox_api_keys::FetchMode::CachedOk,
            manager_arc,
        )
        .await
        {
            Ok(k) => k,
            Err(e) => {
                return PingResult {
                    success: false,
                    latency_ms: 0,
                    status_code: Some(401),
                    error: Some(format!("获取 OfoxAI API key 失败：{e}")),
                };
            }
        }
    } else {
        match api_key {
            Some(k) => k,
            // Gemini's OfoxAI seed used to have no api-key field; the branch
            // is moot for `ofox-gemini` now (covered above) but kept for any
            // stray non-ofox Gemini provider that still hits this code path.
            None if matches!(app_type, AppType::Gemini) => {
                let manager = manager_arc.read().await;
                match manager.get_valid_access_token().await {
                    Ok(t) => t,
                    Err(e) => {
                        return PingResult {
                            success: false,
                            latency_ms: 0,
                            status_code: Some(401),
                            error: Some(format!("获取 OfoxAI 访问令牌失败：{e}")),
                        };
                    }
                }
            }
            None => {
                return PingResult {
                    success: false,
                    latency_ms: 0,
                    status_code: None,
                    error: Some(format!(
                        "{app_str} 当前供应商未配置 API Key，无法测试连通性"
                    )),
                };
            }
        }
    };

    let client = crate::proxy::http_client::get();
    let started = Instant::now();

    // Build the protocol-specific request. Each branch follows the same
    // shape: pick URL + body, fire, read body once. Keeping the branches
    // self-contained beats trying to abstract over three subtly different
    // wire formats.
    let response_result = match app_type {
        AppType::Claude => {
            // Anthropic: POST /anthropic/v1/messages, max_tokens=1.
            // Auth header is x-api-key per Anthropic's spec — Bearer also
            // works on the OfoxAI gateway today, but x-api-key is what the
            // claude provider actually injects (see proxy/providers/claude.rs).
            let url = format!("{}/anthropic/v1/messages", gateway_base());
            let body = serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "hi"}],
            });
            client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .timeout(Duration::from_secs(PING_TIMEOUT_SECS))
                .json(&body)
                .send()
                .await
        }
        AppType::Codex | AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => {
            // OpenAI-compatible: POST /v1/chat/completions, max_tokens=1.
            let url = format!("{}/v1/chat/completions", gateway_base());
            let body = serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "hi"}],
            });
            client
                .post(&url)
                .bearer_auth(&api_key)
                .timeout(Duration::from_secs(PING_TIMEOUT_SECS))
                .json(&body)
                .send()
                .await
        }
        AppType::Gemini => {
            // Gemini: POST /gemini/v1beta/models/{model}:generateContent.
            // Auth header is x-goog-api-key (matches what the proxy's
            // gemini adapter injects; see proxy/providers/gemini.rs:248).
            let url = format!(
                "{}/gemini/v1beta/models/{}:generateContent",
                gateway_base(),
                model
            );
            let body = serde_json::json!({
                "contents": [{"parts": [{"text": "hi"}]}],
                "generationConfig": {"maxOutputTokens": 1},
            });
            client
                .post(&url)
                .header("x-goog-api-key", &api_key)
                .timeout(Duration::from_secs(PING_TIMEOUT_SECS))
                .json(&body)
                .send()
                .await
        }
    };

    let latency_ms = started.elapsed().as_millis() as u64;

    let response = match response_result {
        Ok(r) => r,
        Err(e) => {
            // Network-level failure (DNS, TLS, connect refused, timeout, …).
            // No status_code to report.
            return PingResult {
                success: false,
                latency_ms,
                status_code: None,
                error: Some(format!("网络请求失败：{e}")),
            };
        }
    };

    let status = response.status();
    if status.is_success() {
        // We don't need to inspect the body — a 2xx from /messages or
        // /chat/completions or /generateContent means OfoxAI accepted the
        // request, routed to the upstream model, and got a response. That's
        // the only signal the user cares about.
        return PingResult {
            success: true,
            latency_ms,
            status_code: Some(status.as_u16()),
            error: None,
        };
    }

    // Failure path: read and trim the body so error messages stay glanceable.
    let body = response.text().await.unwrap_or_default();
    let trimmed = if body.len() > 400 {
        format!("{}…", &body[..400])
    } else {
        body
    };
    let hint = match status.as_u16() {
        401 => "API Key 无效或已过期，请尝试解除绑定后重新绑定该工具。",
        403 => "请求被拒绝，请检查账户权限或额度。",
        404 => "未找到该模型，可能已下线或拼写有误。",
        429 => "请求过于频繁，请稍后再试。",
        500..=599 => "OfoxAI 网关返回 5xx，请稍后再试。",
        _ => "",
    };

    PingResult {
        success: false,
        latency_ms,
        status_code: Some(status.as_u16()),
        error: Some(if hint.is_empty() {
            format!("HTTP {status}: {trimmed}")
        } else {
            format!("{hint}（HTTP {status}）{trimmed}")
        }),
    }
}

/// Read the API key (or auth token) that the proxy would inject when
/// forwarding a request for `app`, looking only at the supplied
/// `settings_config`. Returns `None` when the field is missing or empty.
///
/// Mirrors the field paths used by:
///   - `commands/ofox_auth.rs::ofox_provider_for` for Claude / Codex
///   - `database/dao/providers_seed.rs::OFOX_SEEDS` for OpenCode / OpenClaw
///     / Hermes
///   - `proxy/providers/gemini.rs::extract_key_raw` for Gemini
fn extract_api_key(app: &AppType, settings: &serde_json::Value) -> Option<String> {
    let trim_nonempty = |s: &str| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    };
    match app {
        AppType::Claude => settings
            .pointer("/env/ANTHROPIC_AUTH_TOKEN")
            .and_then(|v| v.as_str())
            .and_then(trim_nonempty),
        AppType::Codex => settings
            .pointer("/auth/OPENAI_API_KEY")
            .and_then(|v| v.as_str())
            .and_then(trim_nonempty),
        AppType::OpenCode => settings
            .pointer("/options/apiKey")
            .and_then(|v| v.as_str())
            .and_then(trim_nonempty),
        AppType::OpenClaw => settings
            .get("apiKey")
            .and_then(|v| v.as_str())
            .and_then(trim_nonempty),
        AppType::Hermes => settings
            .get("api_key")
            .and_then(|v| v.as_str())
            .and_then(trim_nonempty),
        AppType::Gemini => settings
            .pointer("/env/GEMINI_API_KEY")
            .and_then(|v| v.as_str())
            .and_then(trim_nonempty),
    }
}
