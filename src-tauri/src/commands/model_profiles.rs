use crate::app_config::BindableTool;
use crate::commands::{model_selection as selections, ofox_auth::OfoxAuthState};
use crate::model_profiles::*;
use crate::model_selection::{
    canonical_tool, ExpectedWrite, ProfileTarget, EXPECTED_WRITE, SELECTION_LOCK, TOOLS,
};
use crate::services::ofox_bind::status::BindingStatus;
use crate::store::AppState;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Instant;
use tauri::State;

async fn context(auth: &OfoxAuthState) -> Context {
    let manager = auth.0.read().await;
    Context {
        apex: crate::ofox_apex::current_apex().into(),
        owner: manager.current_key_owner().await,
        authenticated: manager.is_authenticated(),
    }
}

fn all_tools() -> Vec<String> {
    TOOLS
        .iter()
        .filter(|id| **id != "chatgpt")
        .map(|id| (*id).into())
        .collect()
}

#[tauri::command]
pub async fn model_profiles_list(state: State<'_, AppState>) -> Result<Vec<ModelProfile>, String> {
    let _guard = SELECTION_LOCK.lock().await;
    let snapshots = selections::read_many(&state, all_tools()).await?.tools;
    state
        .db
        .list_model_profiles()
        .map_err(|_| "无法读取模型 Profiles。".to_string())
        .map(|rows| {
            rows.into_iter()
                .map(|row| with_match(row, &snapshots))
                .collect()
        })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn model_profiles_save(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: Option<String>,
    name: String,
    targets: Vec<ProfileTarget>,
    expected_revision: Option<String>,
) -> Result<ModelProfile, String> {
    let _guard = SELECTION_LOCK.lock().await;
    let row = state
        .db
        .save_model_profile(id.as_deref(), &name, targets, expected_revision.as_deref())
        .map_err(|error| error.to_string())?;
    let snapshots = selections::read_many(
        &state,
        row.targets
            .iter()
            .map(|target| target.target.clone())
            .collect(),
    )
    .await?
    .tools;
    selections::notify(&app, vec![]);
    Ok(with_match(row, &snapshots))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn model_profiles_delete(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    expected_revision: String,
) -> Result<(), String> {
    let _guard = SELECTION_LOCK.lock().await;
    state
        .db
        .delete_model_profile(&id, &expected_revision)
        .map_err(|error| error.to_string())?;
    selections::notify(&app, vec![]);
    Ok(())
}

async fn installations(targets: &[ProfileTarget]) -> Result<HashMap<String, String>, String> {
    let mut requested: Vec<String> = targets.iter().map(|target| target.target.clone()).collect();
    if requested.iter().any(|id| id == "codex") {
        requested.push("chatgpt".into());
    }
    let versions = crate::commands::get_tool_versions(Some(requested), None, Some(false)).await?;
    let mut out = HashMap::new();
    for version in versions {
        let version =
            serde_json::to_value(version).map_err(|_| "无法确认工具安装状态。".to_string())?;
        let Some(id) = version["name"].as_str() else {
            continue;
        };
        out.insert(
            id.into(),
            version["installationStatus"]
                .as_str()
                .unwrap_or("unknown")
                .into(),
        );
    }
    Ok(out)
}

async fn preflight(auth: &OfoxAuthState, target: &ProfileTarget) -> Result<Option<String>, String> {
    let ids = target.selection.ids();
    if ids.is_empty() {
        return Ok(None);
    }
    let tool = BindableTool::from_str(&target.target).map_err(|_| "未知的工具。".to_string())?;
    match target.target.as_str() {
        "workbuddy" => {
            let models = selections::catalog(auth, "openai").await?;
            let selections = ids
                .iter()
                .map(|id| {
                    models
                        .iter()
                        .find(|model| model.id == **id)
                        .map(selections::workbuddy_selection)
                        .ok_or_else(|| format!("模型 {id} 已不在当前目录中。"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            crate::workbuddy_config::validate_selections(&selections)?;
            Ok(None)
        }
        "opencode" => {
            let models = selections::catalog(auth, "openai").await?;
            let ids: Vec<_> = ids.into_iter().map(str::to_string).collect();
            let result = crate::commands::manage_tool::resolve_opencode_models(&ids, &models)?;
            Ok((!result.skipped.is_empty())
                .then(|| format!("{} 个模型将按当前目录规则跳过。", result.skipped.len())))
        }
        "pi" | "dsh" => {
            let models = selections::catalog(auth, "openai").await?;
            for id in ids {
                let model = models
                    .iter()
                    .find(|model| model.id == id)
                    .ok_or_else(|| format!("模型 {id} 已不在当前目录中。"))?;
                crate::native_tools::model_api(tool, model)?;
            }
            Ok(None)
        }
        "codex" => Ok(None), // Codex only supports Responses; preserve its existing save rule.
        _ => {
            let token = auth.0.read().await.get_valid_access_token().await.ok();
            for id in ids {
                let result = crate::model_compat::check(tool, id, false, token.as_deref()).await;
                if result.status == "incompatible" {
                    return Err(result
                        .reason
                        .unwrap_or_else(|| "模型与客户端协议不兼容。".into()));
                }
            }
            Ok(None)
        }
    }
}

#[tauri::command]
pub async fn model_profiles_preview(
    state: State<'_, AppState>,
    auth: State<'_, OfoxAuthState>,
    id: String,
) -> Result<ProfilePreview, String> {
    let _guard = SELECTION_LOCK.lock().await;
    let profile = state
        .db
        .get_model_profile(&id)
        .map_err(|_| "无法读取 Profile。".to_string())?
        .ok_or_else(|| "Profile 已删除，请刷新。".to_string())?;
    let targets = crate::model_selection::validate_targets(profile.targets.clone())?;
    let mut snapshots = selections::read_many(
        &state,
        targets.iter().map(|target| target.target.clone()).collect(),
    )
    .await?
    .tools;
    let installed = installations(&targets).await?;
    if installed
        .get("chatgpt")
        .is_some_and(|status| status == "installed")
    {
        if let Some(snapshot) = snapshots
            .iter_mut()
            .find(|snapshot| snapshot.write_target == "codex" && !snapshot.can_write)
        {
            *snapshot = selections::read_one(&state, "chatgpt").await?;
        }
    }
    let ctx = context(&auth).await;
    let mut items = Vec::new();
    for target in targets {
        let snapshot = snapshots
            .iter()
            .find(|snapshot| snapshot.write_target == target.target);
        let before = snapshot.map(|snapshot| snapshot.managed_selection.clone());
        let scope = snapshot
            .map(|snapshot| snapshot.scope.clone())
            .unwrap_or_default();
        let mut item = PreviewItem {
            target: target.target.clone(),
            before,
            after: Some(target.selection.clone()),
            status: PreviewStatus::Ready,
            reason: None,
            scope,
        };
        let has_binding = state
            .db
            .get_bind_record(&target.target)
            .map_err(|_| "无法读取绑定记录。".to_string())?
            .is_some();
        let installation = if target.target == "codex"
            && snapshot.is_some_and(|snapshot| snapshot.can_write)
            && installed
                .get("chatgpt")
                .is_some_and(|status| status == "installed")
        {
            Some("installed")
        } else {
            installed.get(&target.target).map(String::as_str)
        };
        match installation {
            Some("notInstalled") => {
                item.status = PreviewStatus::Skipped;
                item.reason = Some("modelProfiles.reasonNotInstalled".into());
            }
            Some("installed")
                if !has_binding
                    && snapshot.is_none_or(|snapshot| {
                        snapshot.binding.status != BindingStatus::Configured
                    }) =>
            {
                item.status = PreviewStatus::Skipped;
                item.reason = Some("modelProfiles.reasonNotBound".into());
            }
            Some("installed") => match snapshot {
                Some(snapshot) if !snapshot.can_write => {
                    item.status = PreviewStatus::Skipped;
                    item.reason = Some("modelProfiles.reasonDesktopManaged".into());
                }
                Some(snapshot) if snapshot.binding.status != BindingStatus::Configured => {
                    item.status = PreviewStatus::Blocked;
                    item.reason = Some("modelProfiles.reasonBinding".into());
                }
                Some(snapshot) if snapshot.managed_selection == target.selection => {
                    item.status = PreviewStatus::Unchanged;
                }
                Some(_) if !ctx.authenticated => {
                    item.status = PreviewStatus::Blocked;
                    item.reason = Some("modelProfiles.reasonSignIn".into());
                }
                Some(_) => match preflight(&auth, &target).await {
                    Ok(reason) => item.reason = reason,
                    Err(reason) => {
                        item.status = PreviewStatus::Blocked;
                        item.reason = Some(reason);
                    }
                },
                None => {
                    item.status = PreviewStatus::Blocked;
                    item.reason = Some("modelProfiles.reasonBinding".into());
                }
            },
            _ => {
                item.status = PreviewStatus::Blocked;
                item.reason = Some("modelProfiles.reasonInstallationUnknown".into());
            }
        }
        items.push(item);
    }
    let can_apply = !items
        .iter()
        .any(|item| item.status == PreviewStatus::Blocked)
        && items.iter().any(|item| item.status == PreviewStatus::Ready);
    let token = remember_preview(PreparedPlan {
        created_at: Instant::now(),
        profile_id: profile.id,
        profile_revision: profile.revision,
        context: ctx,
        items: items.clone(),
        snapshots,
    })?;
    Ok(ProfilePreview {
        plan_token: token,
        can_apply,
        items,
    })
}

/// A stale preview fails before the first write; later external changes stop at that target.
#[tauri::command(rename_all = "camelCase")]
pub async fn model_profiles_apply(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    auth: State<'_, OfoxAuthState>,
    plan_token: String,
) -> Result<ApplyReport, String> {
    let _guard = SELECTION_LOCK.lock().await;
    let plan = take_preview(&plan_token)?;
    if context(&auth).await != plan.context {
        return Err("账号或区域已变化，请重新预览。".into());
    }
    let row = state
        .db
        .get_model_profile(&plan.profile_id)
        .map_err(|_| "无法读取 Profile。".to_string())?
        .ok_or_else(|| "Profile 已删除，请重新预览。".to_string())?;
    if row.revision != plan.profile_revision {
        return Err("Profile 已修改，请重新预览。".into());
    }
    if plan
        .items
        .iter()
        .any(|item| item.status == PreviewStatus::Blocked)
    {
        return Err("请先解决预览中阻止应用的问题。".into());
    }
    for item in &plan.items {
        if matches!(item.status, PreviewStatus::Ready | PreviewStatus::Unchanged) {
            let old = plan
                .snapshots
                .iter()
                .find(|snapshot| snapshot.write_target == item.target)
                .ok_or_else(|| "预览目标不可用。".to_string())?;
            if selections::read_one(&state, &old.tool_id).await?.revision != old.revision {
                return Err("工具配置已变化，请重新预览。".into());
            }
        }
    }
    let execution = execute_items(&plan, |item, before| {
        let state = state.clone();
        let auth = auth.clone();
        let expected_context = plan.context.clone();
        async move {
            if context(&auth).await != expected_context {
                return Err("账号或区域已变化，已停止应用。".into());
            }
            let selection = item.after.ok_or_else(|| "预览缺少模型选择。".to_string())?;
            let report =
                selections::save_one(state, auth, &before.tool_id, selection, &before.revision)
                    .await?;
            Ok(ExecutionSuccess {
                after_revision: report.write_revision,
                skipped: report
                    .skipped
                    .into_iter()
                    .map(|item| format!("{}: {}", item.id, item.reason))
                    .collect(),
            })
        }
    })
    .await?;
    let outcomes = execution.items;
    let undo = execution.undo;
    let mut status = report_status(&outcomes);
    if execution.incomplete && status == "applied" {
        status = "partial".into();
    }
    let undo_token = if undo.is_empty() {
        None
    } else {
        Some(remember_undo(UndoPlan {
            created_at: Instant::now(),
            context: plan.context,
            entries: undo,
        })?)
    };
    selections::notify(
        &app,
        outcomes
            .iter()
            .filter(|item| item.outcome == ApplyOutcome::Applied)
            .map(|item| item.target.clone())
            .collect(),
    );
    Ok(ApplyReport {
        status,
        items: outcomes,
        undo_token,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn model_profiles_undo(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    auth: State<'_, OfoxAuthState>,
    undo_token: String,
) -> Result<ApplyReport, String> {
    let _guard = SELECTION_LOCK.lock().await;
    let plan = take_undo(&undo_token)?;
    if context(&auth).await != plan.context {
        return Err("账号或区域已变化，已停止撤销。".into());
    }
    for entry in &plan.entries {
        if selections::read_one(&state, &entry.before.tool_id)
            .await?
            .revision
            != entry.after_revision
        {
            return Err("配置在应用后已被修改，已停止撤销。".into());
        }
    }
    let mut outcomes = Vec::new();
    let mut failed = false;
    let mut incomplete = false;
    for entry in plan.entries.into_iter().rev() {
        let target = canonical_tool(&entry.before.write_target)?;
        let mut outcome = ApplyItem {
            target: target.clone(),
            outcome: ApplyOutcome::Skipped,
            reason: None,
        };
        if failed {
            outcome.reason = Some("modelProfiles.reasonStopped".into());
        } else {
            let result = if context(&auth).await != plan.context {
                Err("账号或区域已变化，已停止撤销。".into())
            } else if target == "dsh" && !entry.before.scope_models.is_empty() {
                async {
                    let catalog = selections::catalog(&auth, "openai").await?;
                    let tool =
                        BindableTool::from_str(&target).map_err(|_| "未知的工具。".to_string())?;
                    let key = crate::ofox_secret::default_store()
                        .load(crate::ofox_secret::Slot::ApiKey { tool })
                        .map_err(|_| "无法读取工具 Key。".to_string())?
                        .ok_or_else(|| "工具 Key 已不可用。".to_string())?;
                    let scopes = entry
                        .before
                        .scope_models
                        .iter()
                        .map(|scope| crate::native_tools::NativeScopeModel {
                            scope: scope.scope.clone(),
                            model_id: scope.model_id.clone(),
                        })
                        .collect::<Vec<_>>();
                    EXPECTED_WRITE
                        .scope(
                            ExpectedWrite {
                                target: target.clone(),
                                revision: entry.after_revision.clone(),
                            },
                            crate::native_tools::restore_scope_models(
                                &state.db, tool, &key, &scopes, &catalog,
                            ),
                        )
                        .await
                        .map(|_| ())
                }
                .await
                .map(|()| Vec::new())
            } else {
                selections::save_one(
                    state.clone(),
                    auth.clone(),
                    &entry.before.tool_id,
                    entry.before.managed_selection,
                    &entry.after_revision,
                )
                .await
                .map(|report| {
                    report
                        .skipped
                        .into_iter()
                        .map(|item| format!("{}: {}", item.id, item.reason))
                        .collect::<Vec<_>>()
                })
            };
            match result {
                Ok(skipped) => {
                    outcome.outcome = ApplyOutcome::Applied;
                    if !skipped.is_empty() {
                        outcome.reason = Some(skipped.join("; "));
                        incomplete = true;
                        failed = true;
                    }
                }
                Err(error) => {
                    outcome.outcome = ApplyOutcome::Failed;
                    outcome.reason = Some(error);
                    failed = true;
                }
            }
        }
        outcomes.push(outcome);
    }
    selections::notify(
        &app,
        outcomes
            .iter()
            .filter(|item| item.outcome == ApplyOutcome::Applied)
            .map(|item| item.target.clone())
            .collect(),
    );
    let status = if incomplete {
        "partial".into()
    } else {
        report_status(&outcomes)
    };
    Ok(ApplyReport {
        status,
        items: outcomes,
        undo_token: None,
    })
}
