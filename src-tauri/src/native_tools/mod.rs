//! Pi and DeepSeek Harness native configuration adapters.
//! Patch/scope algorithms adapted from Magpie (MIT; see MAGPIE_LICENSE).
mod common;
mod dsh;
mod pi;
#[cfg(test)]
mod tests;

use crate::app_config::BindableTool;
use crate::database::Database;
use crate::services::model_fetch::FetchedModel;
use crate::services::ofox_bind::{
    report::UnbindReport,
    status::{BindingStatus, ToolBindingStatus},
    BIND_LOCK,
};
use common::{Format, Record};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeScopeModel {
    pub scope: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeSnapshot {
    pub model_ids: Vec<String>,
    pub scope_models: Vec<NativeScopeModel>,
    pub observed_model: Option<String>,
    pub revision_material: Value,
}

/// Share this resolver with session discovery; relative overrides are ambiguous.
pub(crate) fn config_dir(tool: BindableTool) -> Result<PathBuf, String> {
    let (variable, default) = match tool {
        BindableTool::Pi => ("PI_CODING_AGENT_DIR", ".pi/agent"),
        BindableTool::Dsh => ("DSH_HOME", ".dsh"),
        _ => return Err("不是原生工具".into()),
    };
    let home = crate::config::get_home_dir();
    let Some(value) = std::env::var_os(variable).filter(|value| !value.is_empty()) else {
        return Ok(home.join(default));
    };
    let value = value.to_string_lossy();
    let path = if value == "~" {
        home
    } else if let Some(relative) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        home.join(relative)
    } else {
        PathBuf::from(value.as_ref())
    };
    if !path.is_absolute() {
        return Err(format!("{variable} 必须是绝对路径或以 ~/ 开头"));
    }
    Ok(path)
}

pub(crate) fn primary_path(tool: BindableTool) -> Result<PathBuf, String> {
    Ok(config_dir(tool)?.join(if tool == BindableTool::Pi {
        "settings.json"
    } else {
        "cordis.patch.yml"
    }))
}

/// Protocol selection is based on advertised endpoints, never a vendor-name guess.
pub(crate) fn model_api(tool: BindableTool, model: &FetchedModel) -> Result<&'static str, String> {
    match crate::model_compat::native_protocol_for_save(tool, model)? {
        "anthropic" => Ok("anthropic-messages"),
        "responses" => Ok("openai-responses"),
        "chatCompletions" => Ok("openai-completions"),
        _ => Err("模型没有 Pi/dsh 支持的协议".into()),
    }
}

fn url(api: &str) -> String {
    if api == "anthropic-messages" {
        crate::ofox_endpoints::anthropic_base_url()
    } else {
        crate::ofox_endpoints::openai_v1_base_url()
    }
}

fn model_json(model: &FetchedModel) -> Value {
    let mut value = json!({"id":model.id,"name":model.name.as_deref().unwrap_or(&model.id)});
    if let Some(input) = &model.input_modalities {
        value["input"] = input
            .iter()
            .filter(|v| matches!(v.as_str(), "text" | "image"))
            .cloned()
            .collect::<Vec<_>>()
            .into();
    }
    if let Some(parameters) = &model.supported_parameters {
        value["reasoning"] = parameters
            .iter()
            .any(|p| matches!(p.as_str(), "reasoning" | "reasoning_effort" | "thinking"))
            .into();
    }
    value
}

fn require_record(db: &Database, tool: BindableTool) -> Result<Record, String> {
    common::load(db, tool.as_str())?.ok_or_else(|| "工具尚未绑定 Ofox".into())
}

fn require_current_key(record: &Record, key: &str) -> Result<(), String> {
    if record
        .files
        .iter()
        .flat_map(|f| &f.owned)
        .filter(|o| {
            o.path
                .last()
                .is_some_and(|p| matches!(p.as_str(), "apiKey" | "OFOX_DSH_API_KEY"))
        })
        .any(|o| o.written.as_ref().and_then(Value::as_str) != Some(key))
    {
        return Err("绑定密钥已改变，请刷新后重试".into());
    }
    Ok(())
}

fn health(record: &Record, tool: BindableTool) -> Result<ToolBindingStatus, String> {
    let mut status = common::status(record, &config_dir(tool)?)?;
    if status.status == BindingStatus::Configured {
        let selected = match tool {
            BindableTool::Pi => pi::scopes(record),
            BindableTool::Dsh => dsh::scopes(record),
            _ => Err("不是原生工具".into()),
        };
        if let Err(message) = selected {
            status = ToolBindingStatus::modified(vec!["current-provider".into()]);
            status.message = Some(message);
        }
    }
    Ok(status)
}

pub(crate) async fn binding_status(db: &Database, tool: BindableTool) -> ToolBindingStatus {
    let _guard = BIND_LOCK.lock().await;
    common::load(db, tool.as_str())
        .and_then(|r| r.ok_or_else(|| "尚未绑定".to_string()))
        .and_then(|r| health(&r, tool))
        .unwrap_or_else(|_| ToolBindingStatus::unknown())
}
pub(crate) async fn is_bound(db: &Database, tool: BindableTool) -> bool {
    let _guard = BIND_LOCK.lock().await;
    common::load(db, tool.as_str()).ok().flatten().is_some()
}

/// Safe, synchronous material for the common revision/CAS guard. Never returns config values.
pub(crate) fn revision_material(db: &Database, tool: BindableTool) -> Result<Value, String> {
    let Some(record) = common::load(db, tool.as_str())? else {
        return Ok(json!({"directory":config_dir(tool)?,"absent":true}));
    };
    let serialized = serde_json::to_vec(&record).map_err(|_| "绑定记录损坏".to_string())?;
    use sha2::{Digest, Sha256};
    let extra = if tool == BindableTool::Pi {
        vec![record.directory.join("auth.json")]
    } else {
        vec![record.directory.join("cordis.patch.yml")]
    };
    let overlays=extra.into_iter().map(|path|Ok(json!({"path":path,"digest":format!("{:x}",Sha256::digest(common::read(&path)?.unwrap_or_default().as_bytes()))}))).collect::<Result<Vec<_>,String>>()?;
    Ok(
        json!({"files":common::revision(&record)?,"overlays":overlays,"recordDigest":format!("{:x}",Sha256::digest(serialized)),"resolvedDirectory":config_dir(tool)?}),
    )
}

fn snapshot_unlocked(db: &Database, tool: BindableTool) -> Result<NativeSnapshot, String> {
    let record = require_record(db, tool)?;
    let scope_models = match tool {
        BindableTool::Pi => pi::scopes(&record)?,
        BindableTool::Dsh => dsh::scopes(&record)?,
        _ => return Err("不是原生工具".into()),
    };
    let mut model_ids = Vec::new();
    for scope in &scope_models {
        if !model_ids.contains(&scope.model_id) {
            model_ids.push(scope.model_id.clone());
        }
    }
    let observed_model = (model_ids.len() == 1).then(|| model_ids[0].clone());
    Ok(NativeSnapshot {
        model_ids,
        scope_models,
        observed_model,
        revision_material: revision_material(db, tool)?,
    })
}
pub(crate) async fn snapshot(db: &Database, tool: BindableTool) -> Result<NativeSnapshot, String> {
    let _guard = BIND_LOCK.lock().await;
    snapshot_unlocked(db, tool)
}

/// dsh 只把在 dsh 里切换的模型存进当前 profile；读取前先采纳它并同步到其它 profile。
/// Pi 只有一处默认，读回的就是它，不需要这一步。
fn adopt_agent_model(db: &Database, tool: BindableTool) -> Result<(), String> {
    if tool != BindableTool::Dsh {
        return Ok(());
    }
    let Some(record) = common::load(db, tool.as_str())? else {
        return Ok(());
    };
    if health(&record, tool)?.status != BindingStatus::Configured {
        return Ok(());
    }
    match dsh::adopt_changed(&record)? {
        Some(next) => common::commit(db, tool.as_str(), &next),
        None => Ok(()),
    }
}

/// A coherent read for both webviews: binding, current models, and revision
/// share the same lock as rekey, region migration, and model writes.
pub(crate) async fn model_snapshot(
    db: &Database,
    tool: BindableTool,
) -> Result<(ToolBindingStatus, NativeSnapshot, String), String> {
    let _guard = BIND_LOCK.lock().await;
    if let Err(error) = adopt_agent_model(db, tool) {
        log::warn!(
            "[native_tools] 采纳 {} 里切换的模型失败：{error}",
            tool.as_str()
        );
    }
    let binding = common::load(db, tool.as_str())
        .and_then(|r| r.ok_or_else(|| "尚未绑定".to_string()))
        .and_then(|r| health(&r, tool))
        .unwrap_or_else(|_| ToolBindingStatus::unknown());
    let native = snapshot_unlocked(db, tool).unwrap_or(NativeSnapshot {
        model_ids: vec![],
        scope_models: vec![],
        observed_model: None,
        revision_material: revision_material(db, tool)?,
    });
    let revision = crate::model_selection::revision(db, tool.as_str())?;
    Ok((binding, native, revision))
}

pub(crate) fn validate_bind(
    tool: BindableTool,
    model: &FetchedModel,
    catalog: &[FetchedModel],
) -> Result<(), String> {
    let directory = config_dir(tool)?;
    match tool {
        BindableTool::Pi => pi::plan(&directory, "__ofox_validation_only__", model, catalog, None),
        BindableTool::Dsh => {
            dsh::plan(&directory, "__ofox_validation_only__", model, catalog, None)
        }
        _ => return Err("不是原生工具".into()),
    }
    .map(|_| ())
}
pub(crate) async fn bind_with_catalog(
    db: &Database,
    tool: BindableTool,
    key: &str,
    model: &FetchedModel,
    catalog: &[FetchedModel],
) -> Result<(), String> {
    let _guard = BIND_LOCK.lock().await;
    model_api(tool, model)?;
    let previous = common::load(db, tool.as_str())?;
    if let Some(record) = &previous {
        if health(record, tool)?.status != BindingStatus::Configured {
            return Err("接入配置已改变，请先检查或恢复绑定".into());
        }
    }
    let directory = config_dir(tool)?;
    let record = match tool {
        BindableTool::Pi => pi::plan(&directory, key, model, catalog, previous.as_ref())?,
        BindableTool::Dsh => dsh::plan(&directory, key, model, catalog, previous.as_ref())?,
        _ => return Err("不是原生工具".into()),
    };
    common::commit(db, tool.as_str(), &record)
}

pub(crate) async fn set_model(
    db: &Database,
    tool: BindableTool,
    key: &str,
    model: &FetchedModel,
) -> Result<NativeSnapshot, String> {
    let _guard = BIND_LOCK.lock().await;
    crate::model_selection::check_write_revision(db, tool.as_str())?;
    let record = require_record(db, tool)?;
    if health(&record, tool)?.status != BindingStatus::Configured {
        return Err("接入配置已改变，请先检查或恢复绑定".into());
    }
    require_current_key(&record, key)?;
    let next = match tool {
        BindableTool::Pi => pi::set_model(&record, key, model)?,
        BindableTool::Dsh => {
            dsh::set_models(&record, key, &[], std::slice::from_ref(model), Some(model))?
        }
        _ => return Err("不是原生工具".into()),
    };
    common::commit(db, tool.as_str(), &next)?;
    snapshot_unlocked(db, tool)
}

pub(crate) async fn restore_scope_models(
    db: &Database,
    tool: BindableTool,
    key: &str,
    scope_models: &[NativeScopeModel],
    catalog: &[FetchedModel],
) -> Result<NativeSnapshot, String> {
    let _guard = BIND_LOCK.lock().await;
    crate::model_selection::check_write_revision(db, tool.as_str())?;
    let record = require_record(db, tool)?;
    if health(&record, tool)?.status != BindingStatus::Configured {
        return Err("接入配置已改变，无法还原模型".into());
    }
    require_current_key(&record, key)?;
    let current = match tool {
        BindableTool::Pi => pi::scopes(&record)?,
        BindableTool::Dsh => dsh::scopes(&record)?,
        _ => return Err("不是原生工具".into()),
    };
    if current.len() != scope_models.len()
        || current
            .iter()
            .any(|c| scope_models.iter().filter(|s| s.scope == c.scope).count() != 1)
    {
        return Err("受管配置范围已改变，无法还原模型".into());
    }
    let next = match tool {
        BindableTool::Pi => {
            let id = &scope_models[0].model_id;
            let model = catalog
                .iter()
                .find(|m| &m.id == id)
                .ok_or_else(|| "原模型已不在目录中".to_string())?;
            pi::set_model(&record, key, model)?
        }
        BindableTool::Dsh => dsh::set_models(&record, key, scope_models, catalog, None)?,
        _ => return Err("不是原生工具".into()),
    };
    common::commit(db, tool.as_str(), &next)?;
    snapshot_unlocked(db, tool)
}

pub(crate) async fn restore_missing(
    db: &Database,
    tool: BindableTool,
    key: &str,
) -> Result<(), String> {
    let _guard = BIND_LOCK.lock().await;
    let mut record = require_record(db, tool)?;
    if health(&record, tool)?.status != BindingStatus::Missing {
        return Err("配置未缺失或已被用户修改".into());
    }
    for file in &mut record.files {
        let current = common::parse(common::read(&file.path)?.as_deref(), file.format)?;
        let changes = file
            .owned
            .iter()
            .filter(|owned| common::get(&current, &owned.path).is_none())
            .map(|owned| (owned.path.clone(), owned.written.clone(), owned.connection))
            .collect();
        *file = common::plan(
            file.path.clone(),
            file.format,
            file.secret,
            changes,
            Some(file),
        )?;
    }
    replace_record_key(&mut record, key, true)?;
    // A missing file prevented the region reconciler from migrating this
    // record. Restore its managed endpoints from the current region decision.
    for file in &mut record.files {
        let mut changes = Vec::new();
        for owned in &file.owned {
            if !owned
                .path
                .last()
                .is_some_and(|p| matches!(p.as_str(), "baseUrl" | "baseURL"))
            {
                continue;
            }
            let Some(old) = owned.written.as_ref().and_then(Value::as_str) else {
                continue;
            };
            let endpoint = url(if old.ends_with("/anthropic") {
                "anthropic-messages"
            } else {
                "openai-completions"
            });
            if old != endpoint {
                changes.push((owned.path.clone(), Some(Value::String(endpoint)), true));
            }
        }
        if tool == BindableTool::Pi && file.format == Format::Json {
            pi::pending_endpoint_changes(file, &mut changes)?;
        }
        if !changes.is_empty() {
            *file = common::replan_pending(file, changes)?;
        }
    }
    common::commit(db, tool.as_str(), &record)
}

fn replace_record_key(record: &mut Record, key: &str, pending: bool) -> Result<(), String> {
    for file in &mut record.files {
        let changes = file
            .owned
            .iter()
            .filter(|owned| {
                owned
                    .path
                    .last()
                    .is_some_and(|p| matches!(p.as_str(), "apiKey" | "OFOX_DSH_API_KEY"))
            })
            .map(|owned| (owned.path.clone(), Some(Value::String(key.into())), true))
            .collect::<Vec<_>>();
        if !changes.is_empty() {
            *file = if pending {
                common::replan_pending(file, changes)?
            } else {
                common::plan(
                    file.path.clone(),
                    file.format,
                    file.secret,
                    changes,
                    Some(file),
                )?
            };
        }
    }
    Ok(())
}
#[cfg(test)]
pub(crate) async fn replace_key(
    db: &Database,
    tool: BindableTool,
    old: &str,
    key: &str,
) -> Result<(), String> {
    let _guard = BIND_LOCK.lock().await;
    let mut record = require_record(db, tool)?;
    if health(&record, tool)?.status != BindingStatus::Configured {
        return Err("接入配置已改变，未更换 key".into());
    }
    if record
        .files
        .iter()
        .flat_map(|f| &f.owned)
        .filter(|o| {
            o.path
                .last()
                .is_some_and(|p| matches!(p.as_str(), "apiKey" | "OFOX_DSH_API_KEY"))
        })
        .any(|o| o.written.as_ref().and_then(Value::as_str) != Some(old))
    {
        return Err("接入密钥与已保存的密钥不一致".into());
    }
    replace_record_key(&mut record, key, false)?;
    common::commit(db, tool.as_str(), &record)
}

/// Keep the external credential and local key metadata coherent under BIND_LOCK.
pub(crate) async fn replace_key_with_metadata(
    db: &Database,
    tool: BindableTool,
    old: &str,
    key: &str,
    meta: crate::settings::ApiKeyMeta,
    store: &std::sync::Arc<dyn crate::ofox_secret::SecretStore>,
) -> Result<(), String> {
    let _guard = BIND_LOCK.lock().await;
    let mut record = require_record(db, tool)?;
    if health(&record, tool)?.status != BindingStatus::Configured {
        return Err("接入配置已改变，未更换 key".into());
    }
    if record
        .files
        .iter()
        .flat_map(|f| &f.owned)
        .filter(|o| {
            o.path
                .last()
                .is_some_and(|p| matches!(p.as_str(), "apiKey" | "OFOX_DSH_API_KEY"))
        })
        .any(|o| o.written.as_ref().and_then(Value::as_str) != Some(old))
    {
        return Err("接入密钥与已保存的密钥不一致".into());
    }
    let old_meta = crate::settings::get_api_key_meta(tool);
    replace_record_key(&mut record, key, false)?;
    common::commit(db, tool.as_str(), &record)?;
    if crate::ofox_api_keys::save_key(store, key, meta).is_err() {
        let files = (|| {
            // The keychain can block while a CLI edits other fields. Replan
            // only the key against the committed new binding, never an old file.
            let mut rollback = require_record(db, tool)?;
            if health(&rollback, tool)?.status != BindingStatus::Configured {
                return Err("接入配置在保存 key 期间已改变".to_string());
            }
            if rollback
                .files
                .iter()
                .flat_map(|f| &f.owned)
                .filter(|o| {
                    o.path
                        .last()
                        .is_some_and(|p| matches!(p.as_str(), "apiKey" | "OFOX_DSH_API_KEY"))
                })
                .any(|o| o.written.as_ref().and_then(Value::as_str) != Some(key))
            {
                return Err("接入密钥已被其他操作更换".to_string());
            }
            replace_record_key(&mut rollback, old, false)?;
            common::commit(db, tool.as_str(), &rollback)
        })();
        let secret = store.save(crate::ofox_secret::Slot::ApiKey { tool }, old);
        let metadata = if let Some(meta) = old_meta {
            crate::settings::upsert_api_key_meta(meta)
        } else {
            crate::settings::remove_api_key_meta(tool)
        };
        return Err(if files.is_err() || secret.is_err() || metadata.is_err() {
            "保存新 key 失败；回滚失败，请检查工具绑定".into()
        } else {
            "保存新 key 失败，已还原原接入配置".into()
        });
    }
    Ok(())
}
pub(crate) async fn unbind(
    db: &Database,
    tool: BindableTool,
    dry_run: bool,
) -> Result<UnbindReport, String> {
    let _guard = BIND_LOCK.lock().await;
    match common::load(db, tool.as_str())? {
        Some(record) => {
            let record = if tool == BindableTool::Pi {
                pi::unbind_record(&record)?
            } else {
                record
            };
            common::unbind(db, tool.as_str(), &record, dry_run)
        }
        None => Ok(UnbindReport::new(tool.as_str(), dry_run)),
    }
}

pub(crate) async fn reconcile_endpoints(db: &Database) -> Result<bool, String> {
    let _guard = BIND_LOCK.lock().await;
    let mut changed = false;
    let mut failed = Vec::new();
    for tool in [BindableTool::Pi, BindableTool::Dsh] {
        match reconcile_one(db, tool) {
            Ok(tool_changed) => changed |= tool_changed,
            Err(_) => failed.push(tool.as_str()),
        }
    }
    if !failed.is_empty() {
        return Err(format!(
            "{} 接入配置存在冲突，未切换该工具地区",
            failed.join(", ")
        ));
    }
    Ok(changed)
}

fn reconcile_one(db: &Database, tool: BindableTool) -> Result<bool, String> {
    let Some(mut record) = common::load(db, tool.as_str())? else {
        return Ok(false);
    };
    if health(&record, tool)?.status != BindingStatus::Configured {
        return Err(format!("{} 接入配置已修改，未切换地区", tool.as_str()));
    }
    let mut tool_changed = false;
    for file in &mut record.files {
        let mut changes = Vec::new();
        for owned in &file.owned {
            if !owned
                .path
                .last()
                .is_some_and(|p| matches!(p.as_str(), "baseUrl" | "baseURL"))
            {
                continue;
            }
            let Some(old) = owned.written.as_ref().and_then(Value::as_str) else {
                continue;
            };
            let next = if old.ends_with("/anthropic") {
                url("anthropic-messages")
            } else {
                url("openai-completions")
            };
            if old != next {
                changes.push((owned.path.clone(), Some(Value::String(next)), true));
            }
        }
        // Pi also stores native protocol URLs in each registered model.
        if tool == BindableTool::Pi && file.format == Format::Json {
            pi::endpoint_changes(file, &mut changes)?;
        }
        if !changes.is_empty() {
            tool_changed = true;
            *file = common::plan(
                file.path.clone(),
                file.format,
                file.secret,
                changes,
                Some(file),
            )?;
        }
    }
    if tool_changed {
        common::commit(db, tool.as_str(), &record)?;
    }
    Ok(tool_changed)
}
