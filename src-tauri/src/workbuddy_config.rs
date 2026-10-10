//! Native WorkBuddy custom-model configuration.
//!
//! WorkBuddy stores models as a top-level JSON array in
//! `~/.workbuddy/models.json`. Ofox manages a selected set of entries that
//! share one API key and endpoint. Per-entry backups preserve displaced user
//! data while leaving unrelated models and unknown fields untouched.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{get_home_dir, FileTxn};
use crate::database::Database;
use crate::ofox_apex::mentions_ofox_gateway;
use crate::services::ofox_bind::report::{display_path, UnbindReport};
use crate::services::ofox_bind::status::{BindingStatus, ToolBindingStatus};

const BACKUP_VERSION: u32 = 2;
const BACKUP_KEY: &str = "workbuddy";
static CONFIG_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn endpoint_url() -> String {
    format!("{}/v1/chat/completions", crate::ofox_apex::gateway_base())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyModelSelection {
    pub id: String,
    pub name: String,
    pub supports_tool_call: bool,
    pub supports_images: bool,
    pub supports_reasoning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyManagedModelState {
    pub model_id: String,
    pub original_entry: Option<Value>,
    pub original_index: usize,
    pub last_written_entry: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyBindingState {
    pub version: u32,
    pub managed_models: Vec<WorkBuddyManagedModelState>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyEndpointStatus {
    pub expected_url: String,
    pub configured_urls: Vec<String>,
    pub externally_modified: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyWorkBuddyBindingState {
    current_model_id: String,
    original_entry: Option<Value>,
    original_index: usize,
    last_written_entry: Value,
}

pub fn models_path() -> PathBuf {
    get_home_dir().join(".workbuddy").join("models.json")
}

pub(crate) fn validate_selections(selections: &[WorkBuddyModelSelection]) -> Result<(), String> {
    if selections.is_empty() {
        return Err("WorkBuddy 至少需要选择一个模型".to_string());
    }
    let mut ids = HashSet::with_capacity(selections.len());
    for selection in selections {
        let id = selection.id.trim();
        if id.is_empty() {
            return Err("WorkBuddy 模型 ID 不能为空".to_string());
        }
        if !selection.supports_tool_call {
            return Err(format!(
                "WorkBuddy 模型 {id} 不具备工具调用能力，不能加入模型列表"
            ));
        }
        if !ids.insert(id) {
            return Err(format!("WorkBuddy 模型 {id} 被重复选择"));
        }
    }
    Ok(())
}

fn managed_entry(api_key: &str, selection: &WorkBuddyModelSelection, url: &str) -> Value {
    json!({
        "id": selection.id.trim(),
        "name": if selection.name.trim().is_empty() { selection.id.trim() } else { selection.name.trim() },
        "vendor": "OfoxAI",
        "url": url,
        "apiKey": api_key,
        "supportsToolCall": selection.supports_tool_call,
        "supportsImages": selection.supports_images,
        "supportsReasoning": selection.supports_reasoning,
        "useCustomProtocol": true,
    })
}

fn model_id(value: &Value) -> Option<&str> {
    value.get("id").and_then(Value::as_str)
}

/// 指向 Ofox 网关的条目。旧版本在绑定记录丢失后重新绑定时，会把上一次 Ofox 写的
/// 条目当成「原条目」记下来；这种原条目不能再放回去。
fn is_ofox_entry(entry: &Value) -> bool {
    entry
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(mentions_ofox_gateway)
}

/// 换模型时，任何一条 Ofox 条目被外部改过都会让整次更新停下（`Strict`）；解绑则
/// 一律还原（`Always`），Ofox 管理的条目不管被改成什么都拿掉。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreMode {
    Strict,
    Always,
}

fn read_models_text(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "读取 WorkBuddy 模型配置失败（{}）：{error}",
            path.display()
        )),
    }
}

fn parse_models(raw: &str, path: &Path) -> Result<Vec<Value>, String> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| format!("WorkBuddy 模型配置不是有效 JSON（{}）：{e}", path.display()))?;
    value
        .as_array()
        .cloned()
        .ok_or_else(|| "WorkBuddy models.json 必须是模型数组，已停止写入".to_string())
}

fn read_models_from(path: &Path) -> Result<Vec<Value>, String> {
    read_models_text(path)?
        .map(|raw| parse_models(&raw, path))
        .transpose()
        .map(Option::unwrap_or_default)
}

/// 写 models.json；新建的文件只给本人读写（Windows 上权限走 ACL，不参与）。
/// 出错时由调用方回滚 `txn`。
fn write_models(txn: &mut FileTxn, path: &Path, models: &[Value]) -> Result<(), String> {
    let existed = path.exists();
    let bytes = serde_json::to_vec_pretty(models)
        .map_err(|e| format!("序列化 WorkBuddy 模型配置失败：{e}"))?;
    txn.write(path, &bytes)
        .map_err(|e| format!("写入 WorkBuddy 模型配置失败：{e}"))?;
    if !existed {
        txn.set_mode(path, 0o600)
            .map_err(|e| format!("设置 WorkBuddy 模型配置权限失败：{e}"))?;
    }
    Ok(())
}

/// 拿掉 Ofox 管理的条目，再把被它们顶替的原条目放回原位置。
///
/// `Strict`：先核对所有指纹再动手，任何一条被外部改过或删掉就整次停下，不会只改
/// 一半。`Always`：指纹对不上就按模型 ID 找，找不到说明已经被删，跳过。
fn remove_managed_entries(
    models: &mut Vec<Value>,
    state: &WorkBuddyBindingState,
    mode: RestoreMode,
) -> Result<(), String> {
    let mut indexes = Vec::with_capacity(state.managed_models.len());
    for managed in &state.managed_models {
        let by_fingerprint = models
            .iter()
            .position(|entry| entry == &managed.last_written_entry);
        let by_id = || {
            models
                .iter()
                .position(|entry| model_id(entry) == Some(managed.model_id.as_str()))
        };
        match (by_fingerprint, mode) {
            (Some(index), _) => indexes.push(index),
            (None, RestoreMode::Always) => indexes.extend(by_id()),
            (None, RestoreMode::Strict) if by_id().is_some() => {
                return Err(format!(
                    "WorkBuddy 模型 {} 已被外部修改，已停止覆盖；请先在 WorkBuddy 中确认配置",
                    managed.model_id
                ));
            }
            (None, RestoreMode::Strict) => {
                return Err(format!(
                    "WorkBuddy 中 Ofox 模型 {} 已被外部删除，已停止覆盖",
                    managed.model_id
                ));
            }
        }
    }
    indexes.sort_unstable();
    let found = indexes.len();
    indexes.dedup();
    if mode == RestoreMode::Strict && indexes.len() != found {
        return Err("WorkBuddy 绑定备份包含重复模型指纹，已停止覆盖".to_string());
    }
    for index in indexes.into_iter().rev() {
        models.remove(index);
    }

    let mut originals: Vec<_> = state
        .managed_models
        .iter()
        .filter_map(|managed| {
            managed
                .original_entry
                .clone()
                .filter(|entry| !is_ofox_entry(entry))
                .map(|entry| (managed.original_index, entry))
        })
        .collect();
    originals.sort_by_key(|(index, _)| *index);
    for (index, entry) in originals {
        models.insert(index.min(models.len()), entry);
    }
    Ok(())
}

fn sync_models(
    mut models: Vec<Value>,
    previous: Option<&WorkBuddyBindingState>,
    api_key: &str,
    selections: &[WorkBuddyModelSelection],
    url: &str,
) -> Result<(Vec<Value>, WorkBuddyBindingState), String> {
    validate_selections(selections)?;
    if let Some(state) = previous {
        remove_managed_entries(&mut models, state, RestoreMode::Strict)?;
    }

    let mut managed_models = Vec::with_capacity(selections.len());
    for selection in selections {
        let target_id = selection.id.trim();
        let original_index = models
            .iter()
            .position(|entry| model_id(entry) == Some(target_id))
            .unwrap_or(models.len());
        let original_entry = if original_index < models.len() {
            Some(models.remove(original_index)).filter(|entry| !is_ofox_entry(entry))
        } else {
            None
        };
        let entry = managed_entry(api_key, selection, url);
        models.insert(original_index.min(models.len()), entry.clone());
        managed_models.push(WorkBuddyManagedModelState {
            model_id: target_id.to_string(),
            original_entry,
            original_index,
            last_written_entry: entry,
        });
    }

    Ok((
        models,
        WorkBuddyBindingState {
            version: BACKUP_VERSION,
            managed_models,
        },
    ))
}

/// Change only existing Ofox-managed URLs. Deleted entries remain deleted;
/// every existing entry must still match its saved fingerprint.
fn reconcile_models_endpoint(
    mut models: Vec<Value>,
    state: &WorkBuddyBindingState,
    url: &str,
) -> Result<Option<(Vec<Value>, WorkBuddyBindingState)>, String> {
    let mut next_state = state.clone();
    let mut indexes = HashSet::with_capacity(state.managed_models.len());
    let mut changed = false;
    for managed in &mut next_state.managed_models {
        let matches: Vec<_> = models
            .iter()
            .enumerate()
            .filter(|(_, entry)| model_id(entry) == Some(managed.model_id.as_str()))
            .collect();
        let index = match matches.as_slice() {
            [] => continue,
            [(index, entry)] if *entry == &managed.last_written_entry => *index,
            _ => {
                return Err(format!(
                    "WorkBuddy 模型 {} 已被外部修改，无法自动更新地址",
                    managed.model_id
                ));
            }
        };
        if !indexes.insert(index) {
            return Err("WorkBuddy 绑定备份包含重复模型指纹，无法自动更新地址".to_string());
        }
        let entry = models[index]
            .as_object_mut()
            .ok_or_else(|| "WorkBuddy 托管模型配置不是对象".to_string())?;
        if entry.get("url").and_then(Value::as_str) != Some(url) {
            entry.insert("url".to_string(), Value::String(url.to_string()));
            managed.last_written_entry = models[index].clone();
            changed = true;
        }
    }
    Ok(changed.then_some((models, next_state)))
}

fn restore_models(
    mut models: Vec<Value>,
    state: &WorkBuddyBindingState,
) -> Result<Vec<Value>, String> {
    remove_managed_entries(&mut models, state, RestoreMode::Always)?;
    Ok(models)
}

/// 没有绑定记录时的尽力清理：删掉所有指向 Ofox 的条目，返回它们的模型 ID。
fn remove_ofox_entries(models: &mut Vec<Value>) -> Vec<String> {
    let mut removed = Vec::new();
    models.retain(|entry| {
        let ofox = is_ofox_entry(entry);
        if ofox {
            removed.push(model_id(entry).unwrap_or_default().to_string());
        }
        !ofox
    });
    removed
}

fn parse_state(raw: &str) -> Result<WorkBuddyBindingState, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|e| format!("WorkBuddy 绑定备份已损坏：{e}"))?;
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "WorkBuddy 绑定备份缺少版本号".to_string())? as u32;
    match version {
        BACKUP_VERSION => {
            serde_json::from_value(value).map_err(|e| format!("WorkBuddy 绑定备份已损坏：{e}"))
        }
        1 => {
            let legacy: LegacyWorkBuddyBindingState = serde_json::from_value(value)
                .map_err(|e| format!("WorkBuddy 绑定备份已损坏：{e}"))?;
            Ok(WorkBuddyBindingState {
                version: BACKUP_VERSION,
                managed_models: vec![WorkBuddyManagedModelState {
                    model_id: legacy.current_model_id,
                    original_entry: legacy.original_entry,
                    original_index: legacy.original_index,
                    last_written_entry: legacy.last_written_entry,
                }],
            })
        }
        unsupported => Err(format!("不支持的 WorkBuddy 绑定备份版本：{unsupported}")),
    }
}

async fn load_state(db: &Database) -> Result<Option<WorkBuddyBindingState>, String> {
    let Some(backup) = db
        .get_bind_record(BACKUP_KEY)
        .map_err(|e| format!("读取 WorkBuddy 绑定备份失败：{e}"))?
    else {
        return Ok(None);
    };
    parse_state(&backup.record).map(Some)
}

/// 回滚文件，把回滚结果附在 `error` 后面。
fn rollback_with(txn: FileTxn, error: String) -> String {
    match txn.rollback() {
        Ok(()) => format!("{error}（已回滚配置）"),
        Err(rollback_error) => format!("{error}；同时回滚 WorkBuddy 配置失败：{rollback_error}"),
    }
}

async fn persist_binding_state(
    db: &Database,
    path: &Path,
    models: &[Value],
    state: &WorkBuddyBindingState,
) -> Result<(), String> {
    let state_json =
        serde_json::to_string(state).map_err(|e| format!("序列化 WorkBuddy 绑定备份失败：{e}"))?;
    let mut txn = FileTxn::new();
    if let Err(error) = write_models(&mut txn, path, models) {
        return Err(rollback_with(txn, error));
    }
    if let Err(error) = db.upsert_bind_record(BACKUP_KEY, &state_json) {
        return Err(rollback_with(
            txn,
            format!("保存 WorkBuddy 绑定备份失败：{error}"),
        ));
    }
    Ok(())
}

pub async fn sync_selected_models(
    db: &Database,
    api_key: &str,
    selections: &[WorkBuddyModelSelection],
) -> Result<(), String> {
    let _guard = CONFIG_LOCK.lock().await;
    let path = models_path();
    let previous = load_state(db).await?;
    let models = read_models_from(&path)?;
    let url = endpoint_url();
    let (next_models, next_state) =
        sync_models(models, previous.as_ref(), api_key, selections, &url)?;
    persist_binding_state(db, &path, &next_models, &next_state).await
}

/// Ordinary model changes require the saved connection to still exist at the
/// final write lock, including after an asynchronous key/catalog refresh.
pub(crate) async fn update_selected_models(
    db: &Database,
    api_key: &str,
    selections: &[WorkBuddyModelSelection],
) -> Result<(), String> {
    let _guard = CONFIG_LOCK.lock().await;
    let health = inspect_binding(db)
        .await
        .map_err(|_| "无法确认现有 WorkBuddy 配置，已停止保存模型。".to_string())?;
    if health.status != BindingStatus::Configured {
        return Err(health
            .message
            .unwrap_or_else(|| "请先恢复 WorkBuddy 接入配置。".into()));
    }
    let path = models_path();
    let previous = load_state(db).await?;
    let models = read_models_from(&path)?;
    let (next_models, next_state) = sync_models(
        models,
        previous.as_ref(),
        api_key,
        selections,
        &endpoint_url(),
    )?;
    persist_binding_state(db, &path, &next_models, &next_state).await
}

/// Repair bindings created by older builds or left behind by an apex switch.
/// No key refresh is needed: the existing key and every other model field stay
/// exactly as they were. Startup and region switching both call this method.
pub async fn reconcile_managed_endpoint(db: &Database) -> Result<bool, String> {
    let _guard = CONFIG_LOCK.lock().await;
    let Some(state) = load_state(db).await? else {
        return Ok(false);
    };
    let path = models_path();
    let models = read_models_from(&path)?;
    let url = endpoint_url();
    let Some((next_models, next_state)) = reconcile_models_endpoint(models, &state, &url)? else {
        return Ok(false);
    };
    persist_binding_state(db, &path, &next_models, &next_state).await?;
    Ok(true)
}

/// Return only URLs for display; API keys never cross the IPC boundary.
pub async fn endpoint_status(db: &Database) -> Result<WorkBuddyEndpointStatus, String> {
    let _guard = CONFIG_LOCK.lock().await;
    let expected_url = endpoint_url();
    let mut status = WorkBuddyEndpointStatus {
        expected_url,
        configured_urls: Vec::new(),
        externally_modified: false,
    };
    let Some(state) = load_state(db).await? else {
        return Ok(status);
    };
    let models = read_models_from(&models_path())?;
    for managed in &state.managed_models {
        let Some(entry) = models
            .iter()
            .find(|entry| model_id(entry) == Some(managed.model_id.as_str()))
        else {
            status.externally_modified = true;
            continue;
        };
        if entry != &managed.last_written_entry {
            status.externally_modified = true;
        }
        if let Some(url) = entry.get("url").and_then(Value::as_str) {
            if !status.configured_urls.iter().any(|item| item == url) {
                status.configured_urls.push(url.to_string());
            }
        }
    }
    Ok(status)
}

pub async fn active_models(db: &Database) -> Result<Vec<String>, String> {
    Ok(load_state(db)
        .await?
        .map(|state| {
            state
                .managed_models
                .into_iter()
                .map(|managed| managed.model_id)
                .collect()
        })
        .unwrap_or_default())
}

/// Compatibility helper for the existing connectivity probe.
pub async fn active_model(db: &Database) -> Result<String, String> {
    Ok(active_models(db)
        .await?
        .into_iter()
        .next()
        .unwrap_or_default())
}

fn check_saved_entries(models: &[Value], state: &WorkBuddyBindingState) -> ToolBindingStatus {
    if state.managed_models.is_empty() {
        return ToolBindingStatus::unknown();
    }
    let mut missing = false;
    for managed in &state.managed_models {
        let matching: Vec<_> = models
            .iter()
            .filter(|entry| model_id(entry) == Some(managed.model_id.as_str()))
            .collect();
        let mut expected = managed.last_written_entry.clone();
        if !expected.is_object() {
            return ToolBindingStatus::unknown();
        }
        expected["url"] = endpoint_url().into();
        match matching.as_slice() {
            [] => missing = true,
            [entry] if *entry == &expected => {}
            _ => {
                return ToolBindingStatus::modified(vec![format!(
                    "{} · {}",
                    display_path(&models_path()),
                    managed.model_id
                )])
            }
        }
    }
    if missing {
        ToolBindingStatus::missing(vec![display_path(&models_path())])
    } else {
        ToolBindingStatus::configured()
    }
}

async fn inspect_binding(db: &Database) -> Result<ToolBindingStatus, String> {
    let Some(state) = load_state(db).await? else {
        return Ok(ToolBindingStatus::unknown());
    };
    let path = models_path();
    let models = read_models_from(&path)?;
    Ok(check_saved_entries(&models, &state))
}

/// Read-only diagnostics never serialize model entries, saved keys, or parse errors.
pub(crate) async fn binding_status(db: &Database) -> ToolBindingStatus {
    let _guard = CONFIG_LOCK.lock().await;
    inspect_binding(db)
        .await
        .unwrap_or_else(|_| ToolBindingStatus::unknown())
}

/// Re-create only deleted entries from the saved selection, preserving displaced
/// user entries in the original backup. Existing entries are never overwritten.
pub(crate) async fn restore_missing_binding(db: &Database, api_key: &str) -> Result<(), String> {
    let _guard = CONFIG_LOCK.lock().await;
    let health = inspect_binding(db)
        .await
        .map_err(|_| "无法确认 WorkBuddy 配置缺失，已停止恢复。".to_string())?;
    if health.status != BindingStatus::Missing {
        return Err(health
            .message
            .unwrap_or_else(|| "配置未缺失，无需恢复。".into()));
    }
    let mut state = load_state(db)
        .await?
        .ok_or_else(|| "未找到保存的 WorkBuddy 模型，已停止恢复。".to_string())?;
    let path = models_path();
    let mut models = read_models_from(&path)?;
    for managed in &mut state.managed_models {
        let mut expected = managed.last_written_entry.clone();
        if !expected.is_object() {
            return Err("保存的 WorkBuddy 模型格式无效。".into());
        }
        expected["url"] = endpoint_url().into();
        expected["apiKey"] = api_key.into();
        if let Some(existing) = models
            .iter()
            .find(|entry| model_id(entry) == Some(&managed.model_id))
        {
            if existing != &expected {
                return Err("WorkBuddy 接入配置已被修改，已停止恢复。".into());
            }
        } else {
            models.push(expected.clone());
        }
        managed.last_written_entry = expected;
    }
    persist_binding_state(db, &path, &models, &state)
        .await
        .map_err(|_| "恢复 WorkBuddy 配置失败，请检查文件权限和本地绑定记录。".to_string())
}

/// 解除绑定：Ofox 管理的条目一律拿掉（被改过也一样），被顶替的原条目放回原位置；
/// 其它模型不动。没有绑定记录（旧版本退出时清掉了）就尽力删掉指向 Ofox 的条目。
/// `dry_run` 只算不写（预览）。
pub async fn unbind(db: &Database, dry_run: bool) -> Result<UnbindReport, String> {
    let _guard = CONFIG_LOCK.lock().await;
    let path = models_path();
    let shown = display_path(&path);
    let mut report = UnbindReport::new(BACKUP_KEY, dry_run);
    if read_models_text(&path)?.is_none() {
        report.warn("configAlreadyMissing", Some(shown));
        if !dry_run {
            db.delete_bind_record(BACKUP_KEY)
                .map_err(|e| format!("删除 WorkBuddy 绑定备份失败：{e}"))?;
        }
        return Ok(report);
    }
    let models = read_models_from(&path)?;
    let restored = match load_state(db).await? {
        Some(state) => {
            for managed in &state.managed_models {
                let key = format!("{shown}: {}", managed.model_id);
                let has_original = managed
                    .original_entry
                    .as_ref()
                    .is_some_and(|entry| !is_ofox_entry(entry));
                if has_original {
                    report.restored_keys.push(key);
                } else {
                    report.removed_keys.push(key);
                }
            }
            let mut restored = restore_models(models.clone(), &state)?;
            // 记录之外还指向 Ofox 的条目（更早的绑定留下的）一并去掉。
            let leftover = remove_ofox_entries(&mut restored);
            report
                .removed_keys
                .extend(leftover.iter().map(|id| format!("{shown}: {id}")));
            restored
        }
        None => {
            let mut cleaned = models.clone();
            let removed = remove_ofox_entries(&mut cleaned);
            report.legacy = !removed.is_empty();
            report.already_unbound = removed.is_empty();
            report
                .removed_keys
                .extend(removed.iter().map(|id| format!("{shown}: {id}")));
            cleaned
        }
    };
    if dry_run {
        return Ok(report);
    }

    let mut txn = FileTxn::new();
    if restored != models {
        if let Err(error) = write_models(&mut txn, &path, &restored) {
            return Err(rollback_with(txn, error));
        }
    }
    if let Err(error) = db.delete_bind_record(BACKUP_KEY) {
        return Err(rollback_with(
            txn,
            format!("删除 WorkBuddy 绑定备份失败：{error}"),
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://api.ofox.ai/v1/chat/completions";

    fn write_models_to(path: &Path, models: &[Value]) -> Result<(), String> {
        write_models(&mut FileTxn::new(), path, models)
    }

    fn selection(id: &str) -> WorkBuddyModelSelection {
        WorkBuddyModelSelection {
            id: id.to_string(),
            name: format!("Display {id}"),
            supports_tool_call: true,
            supports_images: true,
            supports_reasoning: false,
        }
    }

    #[test]
    fn bind_many_preserves_unrelated_models_and_replaces_collisions() {
        let a = json!({"id":"model-a", "vendor":"User A", "extra": 1});
        let unrelated = json!({"id":"local/model", "vendor":"Local"});
        let b = json!({"id":"model-b", "vendor":"User B"});
        let (models, state) = sync_models(
            vec![a.clone(), unrelated.clone(), b.clone()],
            None,
            "sk-of-test",
            &[selection("model-a"), selection("model-b")],
            "https://api.ofox.io/v1/chat/completions",
        )
        .unwrap();

        assert_eq!(models[0]["vendor"], "OfoxAI");
        assert_eq!(models[1], unrelated);
        assert_eq!(models[2]["vendor"], "OfoxAI");
        assert_eq!(models[0]["apiKey"], "sk-of-test");
        assert_eq!(models[0]["url"], "https://api.ofox.io/v1/chat/completions");
        assert_eq!(state.managed_models[0].original_entry, Some(a));
        assert_eq!(state.managed_models[1].original_entry, Some(b));
    }

    #[test]
    fn changing_selection_restores_dropped_model_and_manages_new_one() {
        let a = json!({"id":"model-a", "vendor":"User A"});
        let b = json!({"id":"model-b", "vendor":"User B"});
        let (models, first_state) = sync_models(
            vec![a.clone(), b.clone()],
            None,
            "key",
            &[selection("model-a"), selection("model-b")],
            "https://api.ofox.ai/v1/chat/completions",
        )
        .unwrap();
        let (models, state) = sync_models(
            models,
            Some(&first_state),
            "key",
            &[selection("model-b"), selection("model-c")],
            "https://api.ofox.io/v1/chat/completions",
        )
        .unwrap();

        assert_eq!(models[0], a);
        assert_eq!(models[1]["id"], "model-b");
        assert_eq!(models[2]["id"], "model-c");
        assert_eq!(models[1]["url"], "https://api.ofox.io/v1/chat/completions");
        assert_eq!(state.managed_models[0].original_entry, Some(b));
    }

    #[test]
    fn unbind_restores_all_exact_entries_and_order() {
        let original = vec![
            json!({"id":"model-a", "unknown": {"x": 1}}),
            json!({"id":"local", "vendor":"Local"}),
            json!({"id":"model-b", "unknown": [1, 2]}),
        ];
        let (models, state) = sync_models(
            original.clone(),
            None,
            "key",
            &[selection("model-a"), selection("model-b")],
            "https://api.ofox.ai/v1/chat/completions",
        )
        .unwrap();
        assert_eq!(restore_models(models, &state).unwrap(), original);
    }

    #[test]
    fn unbind_restores_even_after_external_edits() {
        let original = vec![
            json!({"id":"model-a", "vendor":"User A"}),
            json!({"id":"local", "vendor":"Local"}),
        ];
        let (mut models, state) = sync_models(
            original.clone(),
            None,
            "key",
            &[selection("model-a"), selection("model-b")],
            URL,
        )
        .unwrap();
        models[0]["name"] = json!("Edited in WorkBuddy");
        models.retain(|entry| model_id(entry) != Some("model-b"));
        assert_eq!(restore_models(models, &state).unwrap(), original);
    }

    #[test]
    fn ofox_entries_are_never_kept_as_originals() {
        let leftover = managed_entry("sk-of-OLD", &selection("model-a"), URL);
        let local = json!({"id":"local"});
        let (models, mut state) = sync_models(
            vec![leftover.clone(), local.clone()],
            None,
            "key",
            &[selection("model-a")],
            URL,
        )
        .unwrap();
        assert_eq!(state.managed_models[0].original_entry, None);
        // 旧版本记下的「原条目」本身就是 Ofox 的：解绑时也不放回去。
        state.managed_models[0].original_entry = Some(leftover);
        assert_eq!(restore_models(models, &state).unwrap(), vec![local]);
    }

    #[test]
    fn leftover_ofox_entries_are_found_without_a_binding_state() {
        let mut models = vec![
            json!({"id":"local", "url":"http://localhost:11434/v1"}),
            managed_entry("sk-of-OLD", &selection("model-a"), URL),
        ];
        assert_eq!(remove_ofox_entries(&mut models), ["model-a"]);
        assert_eq!(
            models,
            [json!({"id":"local", "url":"http://localhost:11434/v1"})]
        );
    }

    #[test]
    fn one_external_edit_blocks_the_whole_update() {
        let (mut models, state) = sync_models(
            Vec::new(),
            None,
            "key",
            &[selection("model-a"), selection("model-b")],
            "https://api.ofox.ai/v1/chat/completions",
        )
        .unwrap();
        models[1]["name"] = json!("Changed outside Ofox");
        let error = sync_models(
            models,
            Some(&state),
            "key",
            &[selection("model-c")],
            "https://api.ofox.io/v1/chat/completions",
        )
        .unwrap_err();
        assert!(error.contains("已被外部修改"));
    }

    #[test]
    fn invalid_selection_sets_are_rejected() {
        assert!(sync_models(
            Vec::new(),
            None,
            "key",
            &[],
            "https://api.ofox.ai/v1/chat/completions"
        )
        .unwrap_err()
        .contains("至少"));
        assert!(sync_models(
            Vec::new(),
            None,
            "key",
            &[selection("same"), selection("same")],
            "https://api.ofox.ai/v1/chat/completions",
        )
        .unwrap_err()
        .contains("重复"));
        let mut invalid = selection("model-a");
        invalid.supports_tool_call = false;
        assert!(sync_models(
            Vec::new(),
            None,
            "key",
            &[invalid],
            "https://api.ofox.ai/v1/chat/completions"
        )
        .unwrap_err()
        .contains("工具调用"));
    }

    #[test]
    fn endpoint_reconciliation_updates_only_managed_urls_and_fingerprints() {
        let unrelated = json!({"id":"local", "url":"http://localhost:11434/v1"});
        let (models, state) = sync_models(
            vec![unrelated.clone()],
            None,
            "secret-key",
            &[selection("model-a"), selection("model-b")],
            "https://api.ofox.ai/v1/chat/completions",
        )
        .unwrap();
        let (updated, next_state) =
            reconcile_models_endpoint(models, &state, "https://api.ofox.io/v1/chat/completions")
                .unwrap()
                .unwrap();
        assert_eq!(updated[0], unrelated);
        for managed in &next_state.managed_models {
            let entry = updated
                .iter()
                .find(|entry| model_id(entry) == Some(managed.model_id.as_str()))
                .unwrap();
            assert_eq!(entry["url"], "https://api.ofox.io/v1/chat/completions");
            assert_eq!(entry["apiKey"], "secret-key");
            assert_eq!(entry, &managed.last_written_entry);
        }
        assert!(reconcile_models_endpoint(
            updated.clone(),
            &next_state,
            "https://api.ofox.io/v1/chat/completions"
        )
        .unwrap()
        .is_none());
        assert_eq!(
            restore_models(updated, &next_state).unwrap(),
            vec![unrelated]
        );
    }

    #[test]
    fn endpoint_reconciliation_rejects_external_edit_without_partial_change() {
        let (mut models, state) = sync_models(
            Vec::new(),
            None,
            "key",
            &[selection("model-a"), selection("model-b")],
            "https://api.ofox.ai/v1/chat/completions",
        )
        .unwrap();
        models[1]["url"] = json!("https://user.example/v1/chat/completions");
        let original = models.clone();
        assert!(reconcile_models_endpoint(
            models,
            &state,
            "https://api.ofox.io/v1/chat/completions"
        )
        .unwrap_err()
        .contains("外部修改"));
        assert_eq!(
            original[0]["url"],
            "https://api.ofox.ai/v1/chat/completions"
        );
    }

    #[test]
    fn legacy_single_model_backup_migrates_to_v2() {
        let raw = json!({
            "version": 1,
            "currentModelId": "model-a",
            "originalEntry": {"id":"model-a", "vendor":"User"},
            "originalIndex": 3,
            "lastWrittenEntry": {"id":"model-a", "vendor":"OfoxAI"}
        })
        .to_string();
        let state = parse_state(&raw).unwrap();
        assert_eq!(state.version, 2);
        assert_eq!(state.managed_models[0].model_id, "model-a");
        assert_eq!(state.managed_models[0].original_index, 3);
    }

    #[test]
    fn malformed_or_object_config_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("models.json");
        fs::write(&path, "{not json").unwrap();
        assert!(read_models_from(&path).unwrap_err().contains("有效 JSON"));
        fs::write(&path, "{\"models\":[]}").unwrap();
        assert!(read_models_from(&path)
            .unwrap_err()
            .contains("必须是模型数组"));
    }

    #[test]
    fn new_config_is_private_on_unix() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested").join("models.json");
        write_models_to(&path, &[json!({"id":"model-a"})]).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn existing_config_permissions_are_preserved() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("models.json");
        fs::write(&path, "[]").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        write_models_to(&path, &[json!({"id":"model-a"})]).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_failure_leaves_existing_config_untouched() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("readonly");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("models.json");
        fs::write(&path, "[\n  {\"id\":\"original\"}\n]").unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();
        let result = write_models_to(&path, &[json!({"id":"replacement"})]);
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "[\n  {\"id\":\"original\"}\n]"
        );
    }
}
