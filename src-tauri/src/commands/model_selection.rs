use crate::app_config::BindableTool;
use crate::commands::{manage_tool, ofox_auth::OfoxAuthState};
use crate::model_selection::{
    canonical_tool, validate_selection, ExpectedWrite, ModelSelection, ModelSnapshot,
    ObservedSelection, SaveSelectionReport, ScopeModel, SkippedModel, ToolModelSnapshot,
    EXPECTED_WRITE, SELECTION_LOCK, TOOLS, WRITE_RECEIPT,
};
use crate::services::model_fetch::FetchedModel;
use crate::services::ofox_bind::status::{BindingStatus, ToolBindingStatus};
use crate::store::AppState;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, State};

static CODEX_WRITE_CAPABILITY: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

async fn codex_can_write() -> bool {
    if let Some((at, writable)) = CODEX_WRITE_CAPABILITY.lock().ok().and_then(|guard| *guard) {
        if at.elapsed() < Duration::from_secs(120) {
            return writable;
        }
    }
    let writable =
        match crate::commands::get_tool_versions(Some(vec!["codex".into()]), None, Some(false))
            .await
        {
            Ok(versions) => versions
                .first()
                .and_then(|version| serde_json::to_value(version).ok())
                .is_some_and(|version| {
                    version["installationStatus"] == "installed"
                        && version["installationKind"] == "cli"
                }),
            Err(_) => false,
        };
    if let Ok(mut guard) = CODEX_WRITE_CAPABILITY.lock() {
        *guard = Some((Instant::now(), writable));
    }
    writable
}

pub(crate) async fn read_one(state: &AppState, id: &str) -> Result<ToolModelSnapshot, String> {
    let target = canonical_tool(id)?;
    let tool = BindableTool::from_str(&target).map_err(|_| "未知的工具。".to_string())?;
    let mut snapshot = if target == "workbuddy" {
        crate::workbuddy_config::model_snapshot(&state.db).await?
    } else if matches!(target.as_str(), "pi" | "dsh") {
        let (binding, native, revision) =
            crate::native_tools::model_snapshot(&state.db, tool).await?;
        let model = if native.model_ids.len() == 1 {
            native.model_ids[0].clone()
        } else {
            String::new()
        };
        let scope_models: Vec<ScopeModel> = native
            .scope_models
            .iter()
            .map(|entry| ScopeModel {
                scope: entry.scope.clone(),
                model_id: entry.model_id.clone(),
            })
            .collect();
        ToolModelSnapshot {
            tool_id: id.into(),
            write_target: target.clone(),
            revision,
            binding,
            managed_selection: ModelSelection::Single { model },
            observed_selection: ObservedSelection {
                model: native.observed_model,
                source: "config".into(),
            },
            supports_default: false,
            can_write: true,
            scope: scope_models
                .iter()
                .map(|entry| entry.scope.clone())
                .collect(),
            scope_models,
        }
    } else {
        let (legacy, _) = crate::services::ofox_bind::tool_for(&target)
            .ok_or_else(|| "未知的工具。".to_string())?;
        let key = crate::ofox_secret::default_store()
            .load(crate::ofox_secret::Slot::ApiKey { tool })
            .map_err(|_| "无法读取工具 Key。".to_string())?;
        crate::services::ofox_bind::model_state::snapshot(&state.db, legacy, id, key.as_deref())
            .await?
    };
    snapshot.tool_id = id.into();
    if id == "codex" {
        snapshot.can_write = codex_can_write().await;
    }
    if crate::settings::get_api_key_meta(tool).is_some_and(|meta| meta.revoked) {
        snapshot.binding = ToolBindingStatus::unknown();
    }
    Ok(snapshot)
}

pub(crate) async fn read_many(
    state: &AppState,
    tools: Vec<String>,
) -> Result<ModelSnapshot, String> {
    for id in &tools {
        canonical_tool(id)?;
    }
    if tools
        .iter()
        .any(|id| matches!(id.as_str(), "codex" | "chatgpt"))
    {
        // Run CLI schema detection away from the async runtime before any synchronous inspector.
        tauri::async_runtime::spawn_blocking(crate::services::ofox_bind::codex_probe::flag_support)
            .await
            .map_err(|_| "无法准备 Codex 配置检查。".to_string())?;
    }
    let mut snapshots = Vec::new();
    for id in tools {
        let snapshot = match read_one(state, &id).await {
            Ok(snapshot) => snapshot,
            Err(_) => {
                let target = canonical_tool(&id)?;
                ToolModelSnapshot {
                    tool_id: id,
                    write_target: target.clone(),
                    revision: crate::model_selection::digest(&serde_json::json!({
                        "target":target, "unreadable":true, "apex":crate::ofox_apex::current_apex(),
                    }))?,
                    binding: ToolBindingStatus::unknown(),
                    managed_selection: if matches!(target.as_str(), "opencode" | "workbuddy") {
                        ModelSelection::Models { models: vec![] }
                    } else {
                        ModelSelection::Single {
                            model: String::new(),
                        }
                    },
                    observed_selection: ObservedSelection {
                        model: None,
                        source: "unknown".into(),
                    },
                    supports_default: crate::model_selection::supports_default(&target),
                    can_write: false,
                    scope: vec![],
                    scope_models: vec![],
                }
            }
        };
        snapshots.push(snapshot);
    }
    Ok(ModelSnapshot { tools: snapshots })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn model_selection_snapshot(
    state: State<'_, AppState>,
    tools: Option<Vec<String>>,
) -> Result<ModelSnapshot, String> {
    let _guard = SELECTION_LOCK.lock().await;
    read_many(
        &state,
        tools.unwrap_or_else(|| TOOLS.iter().map(|id| (*id).into()).collect()),
    )
    .await
}

pub(crate) async fn catalog(
    auth: &OfoxAuthState,
    protocol: &str,
) -> Result<Vec<FetchedModel>, String> {
    let token = auth.0.read().await.get_valid_access_token().await.ok();
    crate::services::model_fetch::fetch_ofox_catalog(protocol, token.as_deref(), false)
        .await
        .map(|catalog| catalog.models)
        .map_err(|_| "模型目录暂时不可用，请刷新后重试。".into())
}

pub(crate) fn workbuddy_selection(
    model: &FetchedModel,
) -> crate::workbuddy_config::WorkBuddyModelSelection {
    let parameters = model.supported_parameters.as_deref().unwrap_or_default();
    crate::workbuddy_config::WorkBuddyModelSelection {
        id: model.id.clone(),
        name: model.name.clone().unwrap_or_else(|| model.id.clone()),
        supports_tool_call: parameters.iter().any(|parameter| parameter == "tools"),
        supports_images: model
            .input_modalities
            .as_ref()
            .is_some_and(|modalities| modalities.iter().any(|item| item == "image")),
        supports_reasoning: parameters.iter().any(|parameter| parameter == "reasoning"),
        supported_endpoints: model.supported_endpoints.clone(),
    }
}

/// Called with SELECTION_LOCK held; each final writer additionally checks the task-local revision.
pub(crate) async fn save_one(
    state: State<'_, AppState>,
    auth: State<'_, OfoxAuthState>,
    tool: &str,
    selection: ModelSelection,
    expected_revision: &str,
) -> Result<SaveSelectionReport, String> {
    let target = canonical_tool(tool)?;
    let selection = validate_selection(&target, selection)?;
    let before = read_one(&state, tool).await?;
    if before.revision != expected_revision {
        return Err("模型配置已变化，请刷新后重试。".into());
    }
    if before.binding.status != BindingStatus::Configured {
        return Err(before
            .binding
            .message
            .unwrap_or_else(|| "请先确认 OFox 接入配置。".into()));
    }
    if !before.can_write {
        return Err("当前工具通过桌面 App 管理模型，请在 App 内选择。".into());
    }
    let receipt = Arc::new(Mutex::new(None));
    let result = WRITE_RECEIPT
        .scope(
            receipt.clone(),
            EXPECTED_WRITE.scope(
                ExpectedWrite {
                    target: target.clone(),
                    revision: expected_revision.into(),
                },
                async {
                    match &selection {
                        ModelSelection::Models { models } if target == "opencode" => {
                            let report = manage_tool::set_active_ofox_models(
                                state.clone(),
                                auth.clone(),
                                target.clone(),
                                models.clone(),
                            )
                            .await?;
                            Ok((
                                report.saved,
                                report
                                    .skipped
                                    .into_iter()
                                    .map(|item| SkippedModel {
                                        id: item.id,
                                        reason: item.reason,
                                    })
                                    .collect(),
                            ))
                        }
                        ModelSelection::Models { models } if target == "workbuddy" => {
                            let catalog = catalog(&auth, "openai").await?;
                            let selections = models
                                .iter()
                                .map(|id| {
                                    catalog
                                        .iter()
                                        .find(|model| model.id == *id)
                                        .map(workbuddy_selection)
                                        .ok_or_else(|| format!("模型 {id} 已不在当前目录中。"))
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            crate::workbuddy_config::validate_selections(&selections)?;
                            manage_tool::set_workbuddy_managed_models(
                                state.clone(),
                                auth.clone(),
                                selections,
                            )
                            .await?;
                            Ok((models.clone(), vec![]))
                        }
                        ModelSelection::Single { model } => {
                            manage_tool::set_active_ofox_model(
                                state.clone(),
                                auth.clone(),
                                target.clone(),
                                model.clone(),
                                None,
                                None,
                            )
                            .await?;
                            Ok((
                                if model.is_empty() {
                                    vec![]
                                } else {
                                    vec![model.clone()]
                                },
                                vec![],
                            ))
                        }
                        _ => Err::<(Vec<String>, Vec<SkippedModel>), String>(
                            "模型选择类型无效。".into(),
                        ),
                    }
                },
            ),
        )
        .await?;
    let write_revision = receipt.lock().ok().and_then(|value| value.clone());
    // A completed write remains a completed write even if a later diagnostic read fails.
    let snapshot = match read_one(&state, tool).await {
        Ok(snapshot) => snapshot,
        Err(_) => {
            let revision = crate::model_selection::revision(&state.db, &target)
                .unwrap_or_else(|_| format!("unreadable-{}", uuid::Uuid::new_v4()));
            let managed_selection = match selection {
                ModelSelection::Single { model } => ModelSelection::Single { model },
                ModelSelection::Models { .. } => ModelSelection::Models {
                    models: result.0.clone(),
                },
            };
            ToolModelSnapshot {
                tool_id: tool.into(),
                write_target: target.clone(),
                revision,
                binding: ToolBindingStatus::unknown(),
                managed_selection,
                observed_selection: ObservedSelection {
                    model: None,
                    source: "unknown".into(),
                },
                supports_default: before.supports_default,
                can_write: false,
                scope: before.scope,
                scope_models: Vec::new(),
            }
        }
    };
    Ok(SaveSelectionReport {
        snapshot,
        saved: result.0,
        skipped: result.1,
        write_revision,
    })
}

pub(crate) fn notify(app: &tauri::AppHandle, targets: Vec<String>) {
    let mut tools = targets.clone();
    if targets.iter().any(|id| id == "codex") && !tools.iter().any(|id| id == "chatgpt") {
        tools.push("chatgpt".into());
    }
    let _ = app.emit(
        "model-selection-changed",
        crate::services::ofox_bind::model_state::event_payload(&tools),
    );
}

#[tauri::command(rename_all = "camelCase")]
pub async fn model_selection_save(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    auth: State<'_, OfoxAuthState>,
    tool: String,
    selection: ModelSelection,
    expected_revision: String,
) -> Result<SaveSelectionReport, String> {
    let _guard = SELECTION_LOCK.lock().await;
    let result = save_one(state, auth, &tool, selection, &expected_revision).await?;
    notify(&app, vec![result.snapshot.write_target.clone()]);
    Ok(result)
}
