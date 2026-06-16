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
use std::time::{Duration, Instant};
use tauri::State;

use crate::app_config::AppType;
use crate::commands::ofox_auth::OfoxAuthState;
use crate::store::AppState;

// Gateway base URL for the connectivity probe. Mirrors the dev/prod toggle in
// `ofox_auth.rs` — auth flows go through ofox-core (:8080), but chat/messages
// flow through the gateway plugin (:8088 in dev, line-routed by Traefik in
// prod). Using prod here while the rest of the app speaks to localhost would
// hit a Redis whose OAuth namespace doesn't have the dev token, surfacing as
// the 401 `Invalid or expired token` we used to see.
// TODO: 发布前改回线上地址
const OFOX_GATEWAY_BASE_URL: &str = "http://localhost:8088";

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

    Ok(read_model_from_settings(&app_type, &provider.settings_config))
}

// ---------------------------------------------------------------------------
// 3) Write active model
// ---------------------------------------------------------------------------

/// Write `model` into the active provider's settings_config at the path
/// appropriate for `app`, then trigger a takeover refresh so the live
/// config file (`~/.claude/settings.json`, `~/.codex/config.toml`, …)
/// picks up the change immediately.
///
/// Empty `model` removes the field — keeps the on-disk config clean and
/// signals "fall back to OfoxAI's default routing".
#[tauri::command(rename_all = "camelCase")]
pub async fn set_active_ofox_model(
    state: State<'_, AppState>,
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

    // Re-apply takeover so the new model lands in the live config file.
    // `set_takeover_for_app(_, true)` is idempotent — if takeover was already
    // on (the normal case for a bound tool), it just rewrites the snapshot.
    state
        .proxy_service
        .set_takeover_for_app(app_str, true)
        .await
        .map_err(|e| format!("刷新 {app_str} live 配置失败: {e}"))?;

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
        AppType::OpenClaw | AppType::Hermes => settings
            .get("models")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
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
        AppType::OpenClaw | AppType::Hermes => {
            let obj = settings
                .as_object_mut()
                .ok_or_else(|| "settings_config 不是对象".to_string())?;
            if model.is_empty() {
                obj.insert("models".into(), serde_json::json!([]));
            } else {
                obj.insert("models".into(), serde_json::json!([model]));
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
    let app_type = AppType::from_str(&app).map_err(|e| format!("无效的应用类型: {e}"))?;
    let app_str = app_type.as_str();

    let model = model.trim().to_string();
    if model.is_empty() {
        return Ok(PingResult {
            success: false,
            latency_ms: 0,
            status_code: None,
            error: Some("请先选择一个模型再测试连通性".to_string()),
        });
    }

    // Pull the API key the proxy would use for this app. We read the active
    // provider's settings_config rather than the static OfoxAI seed — that
    // way users who edited the seed (or bound to a non-Ofox provider) still
    // get a meaningful probe.
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

    let api_key = extract_api_key(&app_type, &provider.settings_config);

    // For OfoxAI-bound providers, the value sitting in `settings_config` is a
    // snapshot of the OAuth access_token taken at bind/refresh time. Access
    // tokens have a short TTL (~1 h on the dev IDP, default `expires_in` on
    // prod) and stop being honored by the gateway as soon as the matching
    // `oauth:at:<token>` Redis entry expires. Without this branch, the probe
    // happily sends a stale snapshot and the user sees a confusing 401
    // `Invalid or expired token` even though their session in `auth.json` is
    // still active (the in-memory manager will refresh on demand).
    //
    // Ask the manager for a *currently-valid* token instead — it'll perform
    // a refresh-grant round-trip if the cached one is past its expiry — and
    // override `api_key` with the result. We only do this for `ofox-*`
    // providers so a user who hand-bound a non-Ofox provider with a real
    // `sk-` key keeps using that key untouched.
    let is_ofox_provider = provider_id.starts_with("ofox-");
    let api_key = if is_ofox_provider {
        let manager = ofox_state.0.read().await;
        match manager.get_valid_access_token().await {
            Ok(t) => t,
            Err(e) => {
                return Ok(PingResult {
                    success: false,
                    latency_ms: 0,
                    status_code: Some(401),
                    error: Some(format!("获取 OfoxAI 访问令牌失败：{e}")),
                });
            }
        }
    } else {
        match api_key {
            Some(k) => k,
            // Gemini's OfoxAI seed used to have no api-key field; the branch
            // is moot for `ofox-gemini` now (covered above) but kept for any
            // stray non-ofox Gemini provider that still hits this code path.
            None if matches!(app_type, AppType::Gemini) => {
                let manager = ofox_state.0.read().await;
                match manager.get_valid_access_token().await {
                    Ok(t) => t,
                    Err(e) => {
                        return Ok(PingResult {
                            success: false,
                            latency_ms: 0,
                            status_code: Some(401),
                            error: Some(format!("获取 OfoxAI 访问令牌失败：{e}")),
                        });
                    }
                }
            }
            None => {
                // Suppress the unused-state warning even on the error path.
                let _ = &ofox_state;
                return Ok(PingResult {
                    success: false,
                    latency_ms: 0,
                    status_code: None,
                    error: Some(format!(
                        "{app_str} 当前供应商未配置 API Key，无法测试连通性"
                    )),
                });
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
            let url = format!("{OFOX_GATEWAY_BASE_URL}/anthropic/v1/messages");
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
            let url = format!("{OFOX_GATEWAY_BASE_URL}/v1/chat/completions");
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
                "{OFOX_GATEWAY_BASE_URL}/gemini/v1beta/models/{}:generateContent",
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
            return Ok(PingResult {
                success: false,
                latency_ms,
                status_code: None,
                error: Some(format!("网络请求失败：{e}")),
            });
        }
    };

    let status = response.status();
    if status.is_success() {
        // We don't need to inspect the body — a 2xx from /messages or
        // /chat/completions or /generateContent means OfoxAI accepted the
        // request, routed to the upstream model, and got a response. That's
        // the only signal the user cares about.
        return Ok(PingResult {
            success: true,
            latency_ms,
            status_code: Some(status.as_u16()),
            error: None,
        });
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

    Ok(PingResult {
        success: false,
        latency_ms,
        status_code: Some(status.as_u16()),
        error: Some(if hint.is_empty() {
            format!("HTTP {status}: {trimmed}")
        } else {
            format!("{hint}（HTTP {status}）{trimmed}")
        }),
    })
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
