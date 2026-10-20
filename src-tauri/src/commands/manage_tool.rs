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
//! (Anthropic env, Codex TOML, Gemini env, OpenCode/Hermes dict, OpenClaw
//! list). The read/write helpers below mirror what
//! `services/provider/live.rs` ultimately consumes.

use std::future::Future;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::State;
use tokio::sync::RwLock;

use crate::app_config::{AppType, BindableTool};
use crate::commands::ofox_auth::OfoxAuthState;
use crate::ofox_apex::gateway_base;
use crate::ofox_api_keys::ApiKeyError;
use crate::ofox_auth::OfoxAuthManager;
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
    if let Some(tool) = BindableTool::native(&app) {
        return Ok(crate::native_tools::primary_path(tool)?
            .to_string_lossy()
            .into_owned());
    }
    if app.trim().eq_ignore_ascii_case("workbuddy") {
        return Ok(crate::workbuddy_config::models_path()
            .to_string_lossy()
            .to_string());
    }
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

/// Only safe diagnostics cross IPC; configuration values and parser excerpts stay local.
#[tauri::command(rename_all = "camelCase")]
pub async fn get_tool_binding_status(
    state: State<'_, AppState>,
    app: String,
) -> Result<crate::services::ofox_bind::status::ToolBindingStatus, String> {
    if let Some(tool) = BindableTool::native(&app) {
        return Ok(crate::native_tools::binding_status(&state.db, tool).await);
    }
    if app.trim().eq_ignore_ascii_case("workbuddy") {
        return Ok(crate::workbuddy_config::binding_status(&state.db).await);
    }
    let (tool, _) =
        crate::services::ofox_bind::tool_for(&app).ok_or_else(|| "无效的应用类型".to_string())?;
    let mut status =
        match crate::ofox_secret::default_store().load(crate::ofox_secret::Slot::ApiKey {
            tool: tool.app().into(),
        }) {
            Ok(key) => {
                let status = crate::services::ofox_bind::status::binding_status(
                    &state.db,
                    tool,
                    key.as_deref(),
                )
                .await;
                // 用户在 CLI 里换了模型：让主页显示的模型跟着变。
                if status.status == crate::services::ofox_bind::status::BindingStatus::Configured {
                    crate::services::ofox_bind::prefetch_model_catalog(tool).await;
                    crate::services::ofox_bind::sync_live_model(&state.db, tool, key.as_deref())
                        .await;
                }
                status
            }
            Err(_) => crate::services::ofox_bind::status::ToolBindingStatus::unknown(),
        };
    status.env_overrides = crate::services::env_override::find(tool.app().as_str());
    Ok(status)
}

/// 从 Desktop 启动 Claude、Gemini 前，先采纳用户在 agent 里「记住」的模型，并把旧版本
/// 写的、优先级更高的模型（Claude 的 `ANTHROPIC_MODEL`、Gemini 的 `GEMINI_MODEL`）迁走，
/// 不然刚换的模型这次启动会被盖回去。只是同步展示和配置，出错不拦启动。
pub(crate) async fn follow_live_model(state: &AppState, app: &str) {
    let Some((tool, _)) = crate::services::ofox_bind::tool_for(app) else {
        return;
    };
    let Ok(key) = crate::ofox_secret::default_store().load(crate::ofox_secret::Slot::ApiKey {
        tool: tool.app().into(),
    }) else {
        return;
    };
    let configured =
        crate::services::ofox_bind::status::binding_status(&state.db, tool, key.as_deref())
            .await
            .status
            == crate::services::ofox_bind::status::BindingStatus::Configured;
    if configured {
        crate::services::ofox_bind::prefetch_model_catalog(tool).await;
        crate::services::ofox_bind::sync_live_model(&state.db, tool, key.as_deref()).await;
    }
}

// ---------------------------------------------------------------------------
// 2) Read active model
// ---------------------------------------------------------------------------

/// Read the model currently configured on the app's active provider.
///
/// Returns `""` when:
///   - the provider has no model set yet (the OfoxAI seeds ship with none;
///     binding picks one, and the UI asks for one while it is empty);
///   - the underlying field doesn't exist yet (newly created provider).
///
/// Hard errors only when we cannot resolve the active provider at all
/// (unknown app, DB failure). The dialog treats `""` as "未设置" and
/// still allows the user to save a new value.
#[tauri::command(rename_all = "camelCase")]
pub async fn get_active_ofox_model(
    state: State<'_, AppState>,
    app: String,
) -> Result<String, String> {
    if let Some(tool) = BindableTool::native(&app) {
        return Ok(crate::native_tools::snapshot(&state.db, tool)
            .await?
            .observed_model
            .unwrap_or_default());
    }
    if app.trim().eq_ignore_ascii_case("workbuddy") {
        return crate::workbuddy_config::active_model(&state.db).await;
    }
    let app_type = AppType::from_str(&app).map_err(|e| format!("无效的应用类型: {e}"))?;
    let saved = read_active_model_for(&state, &app_type)?;
    // 用户在 agent 里选了自己配的、同样指向 Ofox 的条目：显示它实际在用的模型。
    Ok(match crate::services::ofox_bind::tool_for(&app) {
        Some((tool, _)) => crate::services::ofox_bind::display_model_or(tool, saved),
        None => saved,
    })
}

/// 主页 ChatGPT 桌面 App 那一行显示的模型：优先用户在 App 里最近选的（App 把它记在自己的
/// 状态文件里，不写 config.toml），没有就用 Codex 配置里的。
#[tauri::command]
pub async fn get_chatgpt_active_model(state: State<'_, AppState>) -> Result<String, String> {
    if let Some(model) = crate::services::ofox_bind::chatgpt_state::recent_model() {
        return Ok(model);
    }
    get_active_ofox_model(state, "codex".to_string()).await
}

/// Return every WorkBuddy model currently managed by Ofox, in display order.
#[tauri::command]
pub async fn get_workbuddy_managed_models(
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    crate::workbuddy_config::active_models(&state.db).await
}

/// Plain-reference helper extracted from [`get_active_ofox_model`] so
/// internal callers (notably `set_active_ofox_model` and `ofox_ping_model`)
/// can resolve "what model is the active provider pointing at" without going
/// through a Tauri command frame.
///
/// Returns `Ok("")` when the active provider exists but has no model field
/// configured (callers treat empty as "unset"); only surfaces `Err` for hard
/// DB failures or when there is no active provider.
fn read_active_model_for(state: &AppState, app_type: &AppType) -> Result<String, String> {
    read_active_provider_and_model_for(state, app_type).map(|(_, model)| model)
}

/// Resolve the active provider id together with the model stored in that
/// provider's configuration. The launcher uses both pieces for tools such as
/// OpenCode, whose CLI model syntax is `<provider-id>/<model-id>`.
pub(crate) fn read_active_provider_and_model_for(
    state: &AppState,
    app_type: &AppType,
) -> Result<(String, String), String> {
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

    let model = read_model_from_settings(app_type, &provider.settings_config);
    Ok((provider_id, model))
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
/// For ofox-* providers this goes through `services::ofox_bind::persist_bound_settings`：
/// 在绑定锁内核对原配置，再保存 DB 模型和工具接入字段。
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
    model_selection: Option<crate::workbuddy_config::WorkBuddyModelSelection>,
    compatibility_protocol: Option<String>,
) -> Result<(), String> {
    if let Some(tool) = BindableTool::native(&app) {
        require_configured(crate::native_tools::binding_status(&state.db, tool).await)?;
        let token = catalog_access_token(&ofox_state.0).await;
        validate_compatibility(
            tool,
            &model,
            compatibility_protocol.as_deref(),
            token.as_deref(),
        )
        .await?;
        let catalog =
            crate::services::model_fetch::fetch_ofox_catalog("openai", token.as_deref(), false)
                .await?;
        let selected = catalog
            .models
            .iter()
            .find(|m| m.id == model.trim())
            .ok_or_else(|| "模型不在当前目录中".to_string())?;
        let key = crate::ofox_secret::default_store()
            .load(crate::ofox_secret::Slot::ApiKey { tool })
            .map_err(|_| "读取已绑定的 API Key 失败".to_string())?
            .ok_or_else(|| "未找到已绑定的 API Key".to_string())?;
        crate::native_tools::set_model(&state.db, tool, &key, selected)
            .await
            .map_err(crate::redact::scrub_error)?;
        crate::ofox_api_keys::mark_key_used(tool);
        return Ok(());
    }
    if app.trim().eq_ignore_ascii_case("workbuddy") {
        require_configured(crate::workbuddy_config::binding_status(&state.db).await)?;
        let selection = model_selection
            .ok_or_else(|| "切换 WorkBuddy 模型时缺少能力信息，请重新打开窗口后重试".to_string())?;
        if selection.id.trim() != model.trim() {
            return Err("WorkBuddy 模型 ID 与能力信息不一致，请刷新后重试".to_string());
        }
        crate::workbuddy_config::validate_selections(std::slice::from_ref(&selection))?;
        let api_key = crate::ofox_api_keys::fetch_or_create_api_key(
            BindableTool::WorkBuddy,
            crate::ofox_api_keys::FetchMode::CachedOk,
            &ofox_state.0,
        )
        .await
        .map_err(|e| api_key_failure("获取 WorkBuddy OfoxAI API key 失败: ", &e))?;
        crate::workbuddy_config::update_selected_models(&state.db, &api_key, &[selection])
            .await
            .map_err(crate::redact::scrub_error)?;
        crate::ofox_api_keys::mark_key_used(BindableTool::WorkBuddy);
        return Ok(());
    }
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

    let resolved_protocol = if needs_compatibility_check(&app_type, &provider_id, &model) {
        let catalog_token = catalog_access_token(&ofox_state.0).await;
        validate_compatibility(
            app_type.into(),
            &model,
            compatibility_protocol.as_deref(),
            catalog_token.as_deref(),
        )
        .await?
    } else {
        None
    };
    let previous_settings = provider.settings_config.clone();

    write_model_into_settings(&app_type, &mut provider.settings_config, model.trim())?;
    if app_type == AppType::OpenCode && !model.trim().is_empty() {
        if let Some(protocol) = resolved_protocol.as_deref() {
            let npm = match protocol {
                "responses" => crate::opencode_config::OFOX_RESPONSES_NPM,
                "chatCompletions" => crate::opencode_config::OFOX_CHAT_NPM,
                _ => return Err("OpenCode 协议选择无效".into()),
            };
            provider.settings_config["models"][model.trim()]["provider"] =
                serde_json::json!({ "npm": npm });
        }
    }

    let api_key = if provider_id.starts_with("ofox-") {
        Some(
            crate::ofox_secret::default_store()
                .load(crate::ofox_secret::Slot::ApiKey {
                    tool: app_type.into(),
                })
                .map_err(|_| "读取已绑定的 API Key 失败".to_string())?
                .ok_or_else(|| "未找到已绑定的 API Key，请重新绑定".to_string())?,
        )
    } else {
        None
    };

    if let Some(key) = api_key.as_deref() {
        crate::services::ofox_bind::persist_bound_settings(
            &state.db,
            crate::services::ofox_bind::Tool::from_app(&app_type),
            key,
            &previous_settings,
            &provider.settings_config,
        )
        .await
        .map_err(crate::redact::scrub_error)?;
    } else {
        persist_with_rollback(
            &state.db,
            app_str,
            &provider_id,
            &previous_settings,
            &provider.settings_config,
            || async {
                state
                    .proxy_service
                    .refresh_takeover_for_app(app_str)
                    .await
                    .map_err(|e| format!("刷新 {app_str} live 配置失败: {e}"))
            },
        )
        .await?;
    }

    Ok(())
}

/// 去掉空白和重复，保持选择的顺序；一个都没有就报错。
fn clean_model_ids(models: &[String]) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = Vec::new();
    for model in models {
        let id = model.trim();
        if !id.is_empty() && !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }
    if ids.is_empty() {
        Err("请至少选择一个模型".into())
    } else {
        Ok(ids)
    }
}

/// OpenCode 条目的 `models`：用户选的这一组，按选择的顺序；每个模型按各自检测出的协议
/// 单独指定传输方式（没检测出结果就不指定，用条目默认的）。
fn write_opencode_models(
    settings: &mut serde_json::Value,
    models: &[(String, Option<String>)],
) -> Result<(), String> {
    let mut dict = serde_json::Map::new();
    for (id, protocol) in models {
        let mut model = serde_json::json!({ "name": id });
        if let Some(protocol) = protocol {
            let npm = match protocol.as_str() {
                "responses" => crate::opencode_config::OFOX_RESPONSES_NPM,
                "chatCompletions" => crate::opencode_config::OFOX_CHAT_NPM,
                _ => return Err("OpenCode 协议选择无效".into()),
            };
            model["provider"] = serde_json::json!({ "npm": npm });
        }
        dict.insert(id.clone(), model);
    }
    settings
        .as_object_mut()
        .ok_or_else(|| "settings_config 不是对象".to_string())?
        .insert("models".into(), serde_json::Value::Object(dict));
    Ok(())
}

/// 已保存的模型列表。OpenCode 是用户选的一组，其余工具只有一个。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_active_ofox_models(
    state: State<'_, AppState>,
    app: String,
) -> Result<Vec<String>, String> {
    if let Some(tool) = BindableTool::native(&app) {
        return Ok(crate::native_tools::snapshot(&state.db, tool)
            .await?
            .model_ids);
    }
    let app_type = AppType::from_str(&app).map_err(|e| format!("无效的应用类型: {e}"))?;
    if app_type != AppType::OpenCode {
        let model = read_active_model_for(&state, &app_type)?;
        return Ok(if model.is_empty() {
            vec![]
        } else {
            vec![model]
        });
    }
    let provider_id = state
        .db
        .get_current_provider(app_type.as_str())
        .map_err(|e| format!("读取当前供应商失败: {e}"))?
        .ok_or_else(|| "opencode 暂无活跃供应商".to_string())?;
    let provider = state
        .db
        .get_provider_by_id(&provider_id, app_type.as_str())
        .map_err(|e| format!("读取 {provider_id} 失败: {e}"))?
        .ok_or_else(|| format!("供应商 {provider_id} 不存在"))?;
    Ok(provider
        .settings_config
        .get("models")
        .and_then(|models| models.as_object())
        .map(|models| models.keys().cloned().collect())
        .unwrap_or_default())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedModel {
    pub id: String,
    pub reason: String,
}

/// 保存结果：写进配置的模型，和因为目录里没有可用端点而没写的模型。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveModelsReport {
    pub saved: Vec<String>,
    pub skipped: Vec<SkippedModel>,
}

pub(crate) struct ResolvedModels {
    pub(crate) resolved: Vec<(String, Option<String>)>,
    pub(crate) skipped: Vec<SkippedModel>,
}

/// 保存时给每个模型选协议，不发探测请求：先用 7 天内的检测缓存，没有就按目录声明和
/// 默认规则选。目录里没有可用端点的模型跳过并告知，不拖累其余模型；一个都存不了才报错。
pub(crate) fn resolve_opencode_models(
    ids: &[String],
    catalog: &[crate::services::model_fetch::FetchedModel],
) -> Result<ResolvedModels, String> {
    let mut resolved = Vec::with_capacity(ids.len());
    let mut skipped = Vec::new();
    for id in ids {
        let outcome = match catalog.iter().find(|model| &model.id == id) {
            Some(model) => crate::model_compat::opencode_protocol_for_save(model),
            None => Err("模型不在当前 Ofox 目录中"),
        };
        match outcome {
            Ok(protocol) => resolved.push((id.clone(), Some(protocol.to_string()))),
            Err(reason) => skipped.push(SkippedModel {
                id: id.clone(),
                reason: reason.to_string(),
            }),
        }
    }
    if resolved.is_empty() {
        return Err("所选模型在 Ofox 目录里都没有 OpenCode 可用的协议端点".into());
    }
    Ok(ResolvedModels { resolved, skipped })
}

/// 保存的协议和后台检测结果不一致的模型，供弹窗提示「建议改协议」。
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolHint {
    pub id: String,
    pub current: String,
    pub suggested: String,
}

/// 条目里这个模型现在走的协议；没单独指定的用条目默认的 Responses。
fn configured_opencode_protocol(settings: &serde_json::Value, model_id: &str) -> &'static str {
    match settings
        .pointer(&format!(
            "/models/{}/provider/npm",
            model_id.replace('~', "~0").replace('/', "~1")
        ))
        .and_then(|npm| npm.as_str())
    {
        Some(crate::opencode_config::OFOX_CHAT_NPM) => "chatCompletions",
        _ => "responses",
    }
}

fn protocol_hints(
    settings: &serde_json::Value,
    catalog: &[crate::services::model_fetch::FetchedModel],
) -> Vec<ProtocolHint> {
    let Some(models) = settings.get("models").and_then(|m| m.as_object()) else {
        return Vec::new();
    };
    models
        .keys()
        .filter_map(|id| {
            let model = catalog.iter().find(|m| &m.id == id)?;
            let suggested = crate::model_compat::cached_opencode_protocol(model)?;
            let current = configured_opencode_protocol(settings, id);
            (suggested != current).then(|| ProtocolHint {
                id: id.clone(),
                current: current.to_string(),
                suggested: suggested.to_string(),
            })
        })
        .collect()
}

/// 后台检测发现和已保存协议不一致的模型。再保存一次就会按检测结果改过来。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_opencode_protocol_hints(
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
) -> Result<Vec<ProtocolHint>, String> {
    let provider_id = state
        .db
        .get_current_provider("opencode")
        .map_err(|e| format!("读取当前供应商失败: {e}"))?
        .ok_or_else(|| "opencode 暂无活跃供应商".to_string())?;
    let Some(provider) = state
        .db
        .get_provider_by_id(&provider_id, "opencode")
        .map_err(|e| format!("读取 {provider_id} 失败: {e}"))?
    else {
        return Ok(Vec::new());
    };
    let token = catalog_access_token(&ofox_state.0).await;
    let Ok(catalog) =
        crate::services::model_fetch::fetch_ofox_catalog("openai", token.as_deref(), false).await
    else {
        return Ok(Vec::new());
    };
    Ok(protocol_hints(&provider.settings_config, &catalog.models))
}

/// OpenCode 保存用户选的一组模型。协议见 [`resolve_opencode_models`]；真正的流式检测
/// 留给用户手动点「检测选中模型」，保存不等也不花钱。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_active_ofox_models(
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
    app: String,
    models: Vec<String>,
) -> Result<SaveModelsReport, String> {
    let app_type = AppType::from_str(&app).map_err(|e| format!("无效的应用类型: {e}"))?;
    if app_type != AppType::OpenCode {
        return Err("只有 OpenCode 支持选择多个模型".into());
    }
    let ids = clean_model_ids(&models)?;
    let app_str = app_type.as_str();
    let provider_id = state
        .db
        .get_current_provider(app_str)
        .map_err(|e| format!("读取当前供应商失败: {e}"))?
        .ok_or_else(|| format!("{app_str} 暂无活跃供应商"))?;
    if !provider_id.starts_with("ofox-") {
        return Err("当前不是 Ofox 服务商，已停止保存模型。".into());
    }
    let mut provider = state
        .db
        .get_provider_by_id(&provider_id, app_str)
        .map_err(|e| format!("读取 {provider_id} 失败: {e}"))?
        .ok_or_else(|| format!("供应商 {provider_id} 不存在"))?;

    let catalog_token = catalog_access_token(&ofox_state.0).await;
    let catalog =
        crate::services::model_fetch::fetch_ofox_catalog("openai", catalog_token.as_deref(), false)
            .await
            .map_err(|e| format!("读取模型目录失败，请稍后重试：{e}"))?;
    let ResolvedModels { resolved, skipped } = resolve_opencode_models(&ids, &catalog.models)?;

    let previous_settings = provider.settings_config.clone();
    write_opencode_models(&mut provider.settings_config, &resolved)?;
    let api_key = crate::ofox_secret::default_store()
        .load(crate::ofox_secret::Slot::ApiKey {
            tool: app_type.into(),
        })
        .map_err(|_| "读取已绑定的 API Key 失败".to_string())?
        .ok_or_else(|| "未找到已绑定的 API Key，请重新绑定".to_string())?;
    crate::services::ofox_bind::persist_bound_settings(
        &state.db,
        crate::services::ofox_bind::Tool::from_app(&app_type),
        &api_key,
        &previous_settings,
        &provider.settings_config,
    )
    .await
    .map_err(crate::redact::scrub_error)?;
    // 保存已完成；默认走 Responses 的模型在后台验证，不阻塞也不改配置。
    tauri::async_runtime::spawn(crate::model_compat::verify_in_background(
        catalog.models,
        catalog_token,
    ));
    Ok(SaveModelsReport {
        saved: resolved.into_iter().map(|(id, _)| id).collect(),
        skipped,
    })
}

/// 取 key 失败的提示。服务端的拒绝文案里可能回显请求里的 key，抹掉再给前端。
fn api_key_failure(context: &str, error: &ApiKeyError) -> String {
    crate::redact::scrub_error(format!("{context}{error}"))
}

fn require_configured(
    health: crate::services::ofox_bind::status::ToolBindingStatus,
) -> Result<(), String> {
    if health.status == crate::services::ofox_bind::status::BindingStatus::Configured {
        Ok(())
    } else {
        Err(health
            .message
            .unwrap_or_else(|| "请先确认 Ofox 接入配置。".into()))
    }
}

async fn persist_with_rollback<F, Fut>(
    db: &crate::database::Database,
    app: &str,
    provider_id: &str,
    previous: &serde_json::Value,
    updated: &serde_json::Value,
    write_live: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    db.update_provider_settings_config(app, provider_id, updated)
        .map_err(|e| format!("更新 {provider_id} settings_config 失败: {e}"))?;
    if let Err(error) = write_live().await {
        db.update_provider_settings_config(app, provider_id, previous)
            .map_err(|rollback| format!("{error}；恢复数据库配置失败: {rollback}"))?;
        return Err(error);
    }
    Ok(())
}

/// Replace the complete set of WorkBuddy models managed by Ofox. The adapter
/// performs conflict checks and writes all selected entries atomically.
#[tauri::command(rename_all = "camelCase")]
pub async fn set_workbuddy_managed_models(
    state: State<'_, AppState>,
    _ofox_state: State<'_, OfoxAuthState>,
    model_selections: Vec<crate::workbuddy_config::WorkBuddyModelSelection>,
) -> Result<(), String> {
    require_configured(crate::workbuddy_config::binding_status(&state.db).await)?;
    crate::workbuddy_config::validate_selections(&model_selections)?;
    let api_key = crate::ofox_secret::default_store()
        .load(crate::ofox_secret::Slot::ApiKey {
            tool: BindableTool::WorkBuddy,
        })
        .map_err(|_| "读取已绑定的 API Key 失败".to_string())?
        .ok_or_else(|| "未找到已绑定的 API Key，请重新绑定".to_string())?;
    crate::workbuddy_config::update_selected_models(&state.db, &api_key, &model_selections)
        .await
        .map_err(crate::redact::scrub_error)?;
    crate::ofox_api_keys::mark_key_used(BindableTool::WorkBuddy);
    Ok(())
}

/// Show the URL WorkBuddy actually has on disk alongside Ofox's selected
/// gateway. Never return the API key stored in the same JSON entries.
#[tauri::command(rename_all = "camelCase")]
pub async fn get_workbuddy_endpoint_status(
    state: State<'_, AppState>,
) -> Result<crate::workbuddy_config::WorkBuddyEndpointStatus, String> {
    crate::workbuddy_config::endpoint_status(&state.db).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn check_ofox_model_compatibility(
    app: String,
    model: String,
    force_retest: Option<bool>,
    ofox_state: State<'_, OfoxAuthState>,
) -> Result<crate::model_compat::CompatibilityResult, String> {
    let tool = BindableTool::from_str(&app)?;
    let catalog_token = catalog_access_token(&ofox_state.0).await;
    Ok(crate::model_compat::check(
        tool,
        &model,
        force_retest.unwrap_or(false),
        catalog_token.as_deref(),
    )
    .await)
}

async fn catalog_access_token(manager: &Arc<RwLock<OfoxAuthManager>>) -> Option<String> {
    manager.read().await.get_valid_access_token().await.ok()
}

/// Codex 只能走 Responses（Ofox 写死 `wire_api = "responses"`，新版 Codex 也
/// 不再支持 chat），检测既选不了协议，也没什么可拦的，所以不查。
fn needs_compatibility_check(app: &AppType, provider_id: &str, model: &str) -> bool {
    provider_id.starts_with("ofox-") && !model.trim().is_empty() && *app != AppType::Codex
}

async fn validate_compatibility(
    tool: BindableTool,
    model: &str,
    selected: Option<&str>,
    catalog_access_token: Option<&str>,
) -> Result<Option<String>, String> {
    let result = crate::model_compat::check(tool, model, false, catalog_access_token).await;
    accept_compatibility_result(tool, result, selected)
}

/// 只有确定不兼容才拦保存。余额不足、鉴权、限流、网络这类原因查不出结果时
/// 照常保存：用户没法据此做决定，拦下来只会让人困惑。OpenCode 这时按模型目录
/// 选协议，Responses 优先。
fn accept_compatibility_result(
    tool: BindableTool,
    result: crate::model_compat::CompatibilityResult,
    selected: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(selected) = selected {
        if !result.allowed_protocols.is_empty()
            && !result
                .allowed_protocols
                .iter()
                .any(|allowed| allowed == selected)
        {
            return Err("所选协议未通过模型目录或流式检查".into());
        }
    }
    match result.status.as_str() {
        "compatible" if selected.is_none() || selected == result.protocol.as_deref() => {
            Ok(result.protocol)
        }
        "incompatible" => Err(result
            .reason
            .unwrap_or_else(|| "模型与客户端协议不兼容".into())),
        "inconclusive" if tool == BindableTool::OpenCode => Ok(Some(
            selected
                .map(str::to_string)
                .or_else(|| result.allowed_protocols.first().cloned())
                .unwrap_or_else(|| "responses".into()),
        )),
        "inconclusive" => Ok(selected.map(str::to_string)),
        _ => Err("所选协议与检测结果不一致，请重新检测".into()),
    }
}

// ---------------------------------------------------------------------------
// Helpers — model field shape per app
// ---------------------------------------------------------------------------

/// Read the model from a settings_config JSON, returning `""` for any
/// "absent / wrong shape" case rather than erroring — see [`get_active_ofox_model`]
/// for the rationale.
pub(crate) fn read_model_from_settings(app: &AppType, settings: &serde_json::Value) -> String {
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
            let cfg_text = settings
                .get("config")
                .and_then(|v| v.as_str())
                .unwrap_or("");
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
/// field, which binding relies on for a seed with no model; a model choice
/// is never empty (`model_selection::validate_selection`). Mutates `settings`
/// in place.
pub(crate) fn write_model_into_settings(
    app: &AppType,
    settings: &mut serde_json::Value,
    model: &str,
) -> Result<(), String> {
    match app {
        AppType::Claude => set_env_string(settings, "ANTHROPIC_MODEL", model),
        AppType::Gemini => set_env_string(
            settings,
            "GEMINI_MODEL",
            &crate::gemini_config::normalize_gemini_env_model(model),
        ),
        AppType::Codex => set_codex_model(settings, model),
        AppType::OpenCode => {
            // Single-select: replace the entire `models` dict. Keys are model
            // ids. Keep `name` populated as required by OpenCodeModel so the
            // provider remains readable by both OpenCode and Ofox on restart.
            let obj = settings
                .as_object_mut()
                .ok_or_else(|| "settings_config 不是对象".to_string())?;
            if model.is_empty() {
                obj.insert("models".into(), serde_json::json!({}));
            } else {
                obj.insert(
                    "models".into(),
                    serde_json::json!({ model: { "name": model } }),
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
fn set_env_string(settings: &mut serde_json::Value, key: &str, value: &str) -> Result<(), String> {
    if !settings.is_object() {
        *settings = serde_json::json!({});
    }
    let root = settings.as_object_mut().expect("ensured object above");
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
        cfg_text.parse::<DocumentMut>().map_err(|e| {
            format!(
                "解析 Codex config TOML 失败: {}",
                crate::redact::describe_toml_error(&cfg_text, &e)
            )
        })?
    };

    if model.is_empty() {
        doc.as_table_mut().remove("model");
    } else {
        doc["model"] = toml_edit::value(model);
    }

    root.insert("config".into(), serde_json::Value::String(doc.to_string()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_model(id: &str, endpoints: &[&str]) -> crate::services::model_fetch::FetchedModel {
        crate::services::model_fetch::FetchedModel {
            id: id.into(),
            name: None,
            owned_by: None,
            pricing_prompt: None,
            supported_endpoints: Some(endpoints.iter().map(|e| e.to_string()).collect()),
            supported_parameters: None,
            input_modalities: None,
            output_modalities: None,
        }
    }

    #[test]
    fn a_model_without_its_own_transport_runs_on_the_entry_default_responses() {
        let settings = serde_json::json!({"models": {
            "openai/a": {"name": "openai/a"},
            "z-ai/b": {"name": "z-ai/b", "provider": {"npm": crate::opencode_config::OFOX_CHAT_NPM}},
        }});
        assert_eq!(
            configured_opencode_protocol(&settings, "openai/a"),
            "responses"
        );
        assert_eq!(
            configured_opencode_protocol(&settings, "z-ai/b"),
            "chatCompletions"
        );
    }

    #[test]
    fn opencode_save_skips_unusable_models_instead_of_failing_the_batch() {
        let both = ["/v1/chat/completions", "/v1/responses"];
        let catalog = vec![
            catalog_model("openai/gpt-5.4", &both),
            catalog_model("moonshotai/kimi-k2.6", &both),
            catalog_model("x/embed", &["/v1/embeddings"]),
        ];
        let ids: Vec<String> = [
            "openai/gpt-5.4",
            "x/embed",
            "moonshotai/kimi-k2.6",
            "gone/model",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let ResolvedModels { resolved, skipped } = resolve_opencode_models(&ids, &catalog).unwrap();
        assert_eq!(
            resolved,
            vec![
                ("openai/gpt-5.4".to_string(), Some("responses".to_string())),
                (
                    "moonshotai/kimi-k2.6".to_string(),
                    Some("chatCompletions".to_string())
                ),
            ]
        );
        assert_eq!(
            skipped.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["x/embed", "gone/model"]
        );
    }

    #[test]
    fn opencode_save_with_nothing_usable_is_an_error() {
        let ids = vec!["gone/model".to_string()];
        assert!(resolve_opencode_models(&ids, &[]).is_err());
    }

    #[test]
    fn opencode_models_keep_the_order_and_each_gets_its_own_transport() {
        let mut settings =
            serde_json::json!({ "npm": "x", "models": { "old": { "name": "old" } } });
        write_opencode_models(
            &mut settings,
            &[
                ("a/one".into(), Some("responses".into())),
                ("b/two".into(), Some("chatCompletions".into())),
                ("c/three".into(), None),
            ],
        )
        .unwrap();
        let ids: Vec<_> = settings["models"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(ids, ["a/one", "b/two", "c/three"]);
        assert_eq!(
            settings["models"]["a/one"]["provider"]["npm"],
            crate::opencode_config::OFOX_RESPONSES_NPM
        );
        assert_eq!(
            settings["models"]["b/two"]["provider"]["npm"],
            crate::opencode_config::OFOX_CHAT_NPM
        );
        assert!(settings["models"]["c/three"].get("provider").is_none());
        assert_eq!(settings["models"]["c/three"]["name"], "c/three");
    }

    #[test]
    fn an_unknown_opencode_protocol_is_rejected() {
        let mut settings = serde_json::json!({});
        assert!(
            write_opencode_models(&mut settings, &[("a/one".into(), Some("bogus".into()))])
                .is_err()
        );
    }

    #[test]
    fn selected_model_ids_are_trimmed_deduplicated_and_must_not_be_empty() {
        assert_eq!(
            clean_model_ids(&[" a/one ".into(), "a/one".into(), "".into(), "b/two".into()])
                .unwrap(),
            ["a/one", "b/two"]
        );
        assert!(clean_model_ids(&["  ".into()]).is_err());
        assert!(clean_model_ids(&[]).is_err());
    }

    #[test]
    fn ping_says_the_balance_ran_out_instead_of_dumping_the_body() {
        let body = r#"{"error":{"message":"Insufficient credits. Current balance: $0. Please top up to continue.","type":"insufficient_credits","code":402,"request_id":"6340bd36"}}"#;
        assert_eq!(
            ping_failure_message(402, body),
            "Ofox 账户余额不足，充值后即可使用。"
        );
    }

    #[test]
    fn ping_shows_only_the_gateway_message_from_a_json_error() {
        let body = r#"{"error":{"message":"model not found","type":"invalid_request_error"}}"#;
        assert_eq!(
            ping_failure_message(404, body),
            "未找到该模型，可能已下线或拼写有误。（HTTP 404）model not found"
        );
        assert_eq!(
            ping_failure_message(400, r#"{"message":"bad request"}"#),
            "请求失败（HTTP 400）bad request"
        );
    }

    #[test]
    fn a_broken_codex_config_error_never_echoes_the_key_in_it() {
        let broken = "model = \"gpt-5\"\nexperimental_bearer_token = \"sk-of-abc123456789\" oops\n";
        let mut settings = serde_json::json!({ "config": broken });

        let error = write_model_into_settings(&AppType::Codex, &mut settings, "gpt-6").unwrap_err();

        assert!(error.contains("line 2, column"), "{error}");
        assert!(!error.contains("abc123456789"), "{error}");
        assert_eq!(settings["config"], broken, "settings left untouched");
    }

    #[test]
    fn key_fetch_failures_never_repeat_a_key_the_server_echoes_back() {
        let rejected = ApiKeyError::RemoteRejected("400: bad key sk-of-abc123456789".into());
        let unauthorized = ApiKeyError::Unauthorized("Bearer abcdefgh12345678 expired".into());

        for error in [rejected, unauthorized] {
            let message = api_key_failure("获取 OfoxAI API key 失败：", &error);
            assert!(
                message.starts_with("获取 OfoxAI API key 失败："),
                "{message}"
            );
            assert!(!message.contains("abc123456789"), "{message}");
            assert!(!message.contains("abcdefgh12345678"), "{message}");
        }
    }

    #[test]
    fn ping_never_repeats_a_key_the_gateway_echoes_back() {
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-of-abc123456789xyz"}}"#;
        let message = ping_failure_message(401, body);
        assert!(!message.contains("abc123456789"), "{message}");
        assert!(message.contains("Incorrect API key provided"), "{message}");
    }

    #[test]
    fn ping_truncates_a_long_non_json_body_on_a_character_boundary() {
        let body = "网关错误".repeat(200);
        let message = ping_failure_message(502, &body);
        assert!(message.starts_with("OfoxAI 网关返回 5xx，请稍后再试。（HTTP 502）"));
        assert!(message.ends_with('…'));
        assert!(message.chars().count() < 260);
    }

    #[test]
    fn opencode_model_entry_remains_typed_config_compatible() {
        let mut settings = serde_json::json!({
            "npm": "@ai-sdk/openai",
            "name": "OfoxAI",
            "options": {},
            "models": {}
        });

        write_model_into_settings(&AppType::OpenCode, &mut settings, "openai/gpt-5.6-terra")
            .unwrap();

        assert_eq!(
            settings.pointer("/models/openai~1gpt-5.6-terra/name"),
            Some(&serde_json::json!("openai/gpt-5.6-terra"))
        );
        serde_json::from_value::<crate::provider::OpenCodeProviderConfig>(settings)
            .expect("saved provider should remain readable after restart");
    }

    #[test]
    fn opencode_glm_53_selection_receives_chat_compatibility_override() {
        let mut settings = serde_json::json!({
            "npm": "@ai-sdk/openai",
            "name": "OfoxAI",
            "options": {},
            "models": {}
        });

        write_model_into_settings(&AppType::OpenCode, &mut settings, "z-ai/glm-5.3-flash").unwrap();
        assert!(crate::opencode_config::normalize_ofox_provider_transport(
            &mut settings
        ));

        assert_eq!(
            settings.pointer("/models/z-ai~1glm-5.3-flash/provider/npm"),
            Some(&serde_json::json!("@ai-sdk/openai-compatible"))
        );
        assert_eq!(
            settings.pointer("/models/z-ai~1glm-5.3-flash/interleaved"),
            Some(&serde_json::json!("reasoning_content"))
        );
        serde_json::from_value::<crate::provider::OpenCodeProviderConfig>(settings)
            .expect("GLM compatibility fields must survive the typed config round trip");
    }

    #[test]
    fn incompatible_model_cannot_be_overridden() {
        let result = crate::model_compat::CompatibilityResult {
            app: "opencode".into(),
            model: "x".into(),
            protocol: Some("chatCompletions".into()),
            status: "incompatible".into(),
            source: "probe".into(),
            reason: Some("broken stream".into()),
            allowed_protocols: vec!["chatCompletions".into()],
        };
        assert!(accept_compatibility_result(
            BindableTool::OpenCode,
            result,
            Some("chatCompletions"),
        )
        .is_err());
    }

    fn inconclusive(app: &str, allowed: &[&str]) -> crate::model_compat::CompatibilityResult {
        crate::model_compat::CompatibilityResult {
            app: app.into(),
            model: "x".into(),
            protocol: allowed.first().map(|p| p.to_string()),
            status: "inconclusive".into(),
            source: "probe".into(),
            reason: Some("网关暂时无法完成检测，请稍后重试".into()),
            allowed_protocols: allowed.iter().map(|p| p.to_string()).collect(),
        }
    }

    #[test]
    fn inconclusive_check_does_not_block_saving() {
        assert_eq!(
            accept_compatibility_result(
                BindableTool::OpenClaw,
                inconclusive("openclaw", &["chatCompletions"]),
                None,
            ),
            Ok(None)
        );
    }

    #[test]
    fn inconclusive_opencode_defaults_to_responses_when_the_catalog_allows_it() {
        let both = inconclusive("opencode", &["responses", "chatCompletions"]);
        assert_eq!(
            accept_compatibility_result(BindableTool::OpenCode, both, None),
            Ok(Some("responses".into()))
        );
        let chat_only = inconclusive("opencode", &["chatCompletions"]);
        assert_eq!(
            accept_compatibility_result(BindableTool::OpenCode, chat_only, None),
            Ok(Some("chatCompletions".into()))
        );
        let unknown = inconclusive("opencode", &[]);
        assert_eq!(
            accept_compatibility_result(BindableTool::OpenCode, unknown, None),
            Ok(Some("responses".into()))
        );
    }

    #[test]
    fn inconclusive_opencode_still_rejects_a_protocol_the_catalog_lacks() {
        let chat_only = inconclusive("opencode", &["chatCompletions"]);
        assert!(
            accept_compatibility_result(BindableTool::OpenCode, chat_only, Some("responses"))
                .is_err()
        );
    }

    #[test]
    fn codex_models_skip_the_stream_check() {
        assert!(!needs_compatibility_check(
            &AppType::Codex,
            "ofox-codex",
            "deepseek/x"
        ));
        assert!(needs_compatibility_check(
            &AppType::OpenCode,
            "ofox-opencode",
            "deepseek/x"
        ));
        assert!(!needs_compatibility_check(
            &AppType::OpenCode,
            "ofox-opencode",
            "  "
        ));
        assert!(!needs_compatibility_check(
            &AppType::OpenCode,
            "custom",
            "deepseek/x"
        ));
    }

    #[tokio::test]
    async fn failed_live_write_restores_previous_provider_settings() {
        let db = crate::database::Database::memory().unwrap();
        let previous = serde_json::json!({ "models": {"old": {"name": "old"}} });
        let updated = serde_json::json!({ "models": {"new": {"name": "new"}} });
        let provider = crate::provider::Provider::with_id(
            "ofox-opencode".into(),
            "OfoxAI".into(),
            previous.clone(),
            None,
        );
        db.save_provider("opencode", &provider).unwrap();
        let error = persist_with_rollback(
            &db,
            "opencode",
            "ofox-opencode",
            &previous,
            &updated,
            || async { Err("disk write failed".into()) },
        )
        .await
        .unwrap_err();
        assert_eq!(error, "disk write failed");
        assert_eq!(
            db.get_provider_by_id("ofox-opencode", "opencode")
                .unwrap()
                .unwrap()
                .settings_config,
            previous
        );
    }
}

// ---------------------------------------------------------------------------
// 4) Connectivity probe
// ---------------------------------------------------------------------------

/// Result of a ping — minimal shape that the dialog can render directly.
///
/// `success` is the only field the happy path needs; `latencyMs` powers the
/// "✓ 连通成功（123 ms）" subtitle, and `error` carries the human-readable
/// reason when `success = false`. Status code is surfaced separately so the
/// front-end can special-case 402 (out of credits → top-up) without
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
    let model = model.trim().to_string();
    if model.is_empty() {
        return PingResult {
            success: false,
            latency_ms: 0,
            status_code: None,
            error: Some("请先选择一个模型再测试连通性".to_string()),
        };
    }

    if let Some(tool) = BindableTool::native(app) {
        let token = catalog_access_token(manager_arc).await;
        let result = crate::model_compat::check(tool, &model, true, token.as_deref()).await;
        return PingResult {
            success: result.status == "compatible",
            latency_ms: 0,
            status_code: None,
            error: result.reason,
        };
    }

    if app.trim().eq_ignore_ascii_case("workbuddy") {
        let api_key = match crate::ofox_api_keys::fetch_or_create_api_key(
            BindableTool::WorkBuddy,
            crate::ofox_api_keys::FetchMode::CachedOk,
            manager_arc,
        )
        .await
        {
            Ok(key) => key,
            Err(error) => {
                return PingResult {
                    success: false,
                    latency_ms: 0,
                    status_code: Some(401),
                    error: Some(api_key_failure(
                        "获取 WorkBuddy OfoxAI API key 失败：",
                        &error,
                    )),
                };
            }
        };
        let started = Instant::now();
        let url = format!("{}/v1/chat/completions", gateway_base());
        let body = serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "hi"}],
        });
        let response = crate::proxy::http_client::get()
            .post(&url)
            .bearer_auth(api_key)
            .timeout(Duration::from_secs(PING_TIMEOUT_SECS))
            .json(&body)
            .send()
            .await;
        return ping_result_from_response(started, response).await;
    }

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
                    error: Some(api_key_failure("获取 OfoxAI API key 失败：", &e)),
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

    ping_result_from_response(started, response_result).await
}

async fn ping_result_from_response(
    started: Instant,
    response_result: Result<reqwest::Response, reqwest::Error>,
) -> PingResult {
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

    let body = response.text().await.unwrap_or_default();
    PingResult {
        success: false,
        latency_ms,
        status_code: Some(status.as_u16()),
        error: Some(ping_failure_message(status.as_u16(), &body)),
    }
}

const PING_DETAIL_MAX_CHARS: usize = 200;

/// One readable sentence for a failed ping: a hint for the status plus the
/// gateway's own `message`, never the raw JSON body. A 402 means the Ofox
/// balance ran out; the dialog adds the top-up action for that status.
fn ping_failure_message(status: u16, body: &str) -> String {
    if status == 402 {
        return "Ofox 账户余额不足，充值后即可使用。".into();
    }
    let hint = match status {
        401 => "API Key 无效或已过期，请尝试解除绑定后重新绑定该工具。",
        403 => "请求被拒绝，请检查账户权限或额度。",
        404 => "未找到该模型，可能已下线或拼写有误。",
        429 => "请求过于频繁，请稍后再试。",
        500..=599 => "OfoxAI 网关返回 5xx，请稍后再试。",
        _ => "请求失败",
    };
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let detail = parsed
        .as_ref()
        .and_then(|json| {
            json.pointer("/error/message")
                .or_else(|| json.get("message"))
                .and_then(|message| message.as_str())
        })
        .unwrap_or(body)
        .trim();
    // 网关可能把请求里的 key 原样回显在报错里；先抹掉再截断，免得截断把它切成抹不掉的残段。
    let detail = crate::redact::scrub(detail);
    let detail = match detail.char_indices().nth(PING_DETAIL_MAX_CHARS) {
        Some((cut, _)) => format!("{}…", &detail[..cut]),
        None => detail.into_owned(),
    };
    format!("{hint}（HTTP {status}）{detail}")
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
