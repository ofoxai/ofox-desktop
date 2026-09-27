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

use crate::config::{atomic_write, get_home_dir};
use crate::database::Database;

const BACKUP_VERSION: u32 = 2;
const BACKUP_KEY: &str = "workbuddy";
const OFOX_CHAT_COMPLETIONS_URL: &str = "https://api.ofox.ai/v1/chat/completions";

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

fn validate_selections(selections: &[WorkBuddyModelSelection]) -> Result<(), String> {
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

fn managed_entry(api_key: &str, selection: &WorkBuddyModelSelection) -> Value {
    json!({
        "id": selection.id.trim(),
        "name": if selection.name.trim().is_empty() { selection.id.trim() } else { selection.name.trim() },
        "vendor": "OfoxAI",
        "url": OFOX_CHAT_COMPLETIONS_URL,
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

fn read_models_from(path: &Path) -> Result<Vec<Value>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("读取 WorkBuddy 模型配置失败（{}）：{e}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("WorkBuddy 模型配置不是有效 JSON（{}）：{e}", path.display()))?;
    value
        .as_array()
        .cloned()
        .ok_or_else(|| "WorkBuddy models.json 必须是模型数组，已停止写入".to_string())
}

fn write_models_to(path: &Path, models: &[Value]) -> Result<(), String> {
    // 只在 unix 才用 existed 做"新建文件才 chmod 600"的判断；Windows 上
    // 文件权限走 ACL，我们不参与，变量本身也没意义。
    #[cfg(unix)]
    let existed = path.exists();
    let bytes = serde_json::to_vec_pretty(models)
        .map_err(|e| format!("序列化 WorkBuddy 模型配置失败：{e}"))?;
    atomic_write(path, &bytes).map_err(|e| format!("写入 WorkBuddy 模型配置失败：{e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !existed {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("设置 WorkBuddy 模型配置权限失败：{e}"))?;
        }
    }
    Ok(())
}

/// Validate every fingerprint before mutating the list. One external edit
/// therefore blocks the entire selection update rather than partially
/// restoring or overwriting user configuration.
fn remove_managed_entries(
    models: &mut Vec<Value>,
    state: &WorkBuddyBindingState,
) -> Result<(), String> {
    let mut indexes = Vec::with_capacity(state.managed_models.len());
    for managed in &state.managed_models {
        let Some(index) = models
            .iter()
            .position(|entry| entry == &managed.last_written_entry)
        else {
            if models
                .iter()
                .any(|entry| model_id(entry) == Some(managed.model_id.as_str()))
            {
                return Err(format!(
                    "WorkBuddy 模型 {} 已被外部修改，已停止覆盖；请先在 WorkBuddy 中确认配置",
                    managed.model_id
                ));
            }
            return Err(format!(
                "WorkBuddy 中 Ofox 模型 {} 已被外部删除，已停止覆盖",
                managed.model_id
            ));
        };
        indexes.push(index);
    }
    indexes.sort_unstable();
    indexes.dedup();
    if indexes.len() != state.managed_models.len() {
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
) -> Result<(Vec<Value>, WorkBuddyBindingState), String> {
    validate_selections(selections)?;
    if let Some(state) = previous {
        remove_managed_entries(&mut models, state)?;
    }

    let mut managed_models = Vec::with_capacity(selections.len());
    for selection in selections {
        let target_id = selection.id.trim();
        let original_index = models
            .iter()
            .position(|entry| model_id(entry) == Some(target_id))
            .unwrap_or(models.len());
        let original_entry = if original_index < models.len() {
            Some(models.remove(original_index))
        } else {
            None
        };
        let entry = managed_entry(api_key, selection);
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

fn restore_models(
    mut models: Vec<Value>,
    state: &WorkBuddyBindingState,
) -> Result<Vec<Value>, String> {
    remove_managed_entries(&mut models, state)?;
    Ok(models)
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
        .get_live_backup(BACKUP_KEY)
        .await
        .map_err(|e| format!("读取 WorkBuddy 绑定备份失败：{e}"))?
    else {
        return Ok(None);
    };
    parse_state(&backup.original_config).map(Some)
}

fn file_snapshot(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if !path.exists() {
        return Ok(None);
    }
    fs::read(path)
        .map(Some)
        .map_err(|e| format!("读取 WorkBuddy 配置快照失败（{}）：{e}", path.display()))
}

fn restore_file_snapshot(path: &Path, snapshot: Option<&[u8]>) -> Result<(), String> {
    match snapshot {
        Some(bytes) => atomic_write(path, bytes)
            .map_err(|e| format!("回滚 WorkBuddy 配置失败（{}）：{e}", path.display())),
        None if path.exists() => fs::remove_file(path)
            .map_err(|e| format!("回滚 WorkBuddy 新配置失败（{}）：{e}", path.display())),
        None => Ok(()),
    }
}

pub async fn sync_selected_models(
    db: &Database,
    api_key: &str,
    selections: &[WorkBuddyModelSelection],
) -> Result<(), String> {
    let path = models_path();
    let previous = load_state(db).await?;
    let models = read_models_from(&path)?;
    let (next_models, next_state) = sync_models(models, previous.as_ref(), api_key, selections)?;
    let snapshot = file_snapshot(&path)?;
    write_models_to(&path, &next_models)?;

    let state_json = serde_json::to_string(&next_state)
        .map_err(|e| format!("序列化 WorkBuddy 绑定备份失败：{e}"))?;
    if let Err(error) = db.save_live_backup(BACKUP_KEY, &state_json).await {
        let rollback = restore_file_snapshot(&path, snapshot.as_deref());
        return Err(match rollback {
            Ok(()) => format!("保存 WorkBuddy 绑定备份失败：{error}（已回滚配置）"),
            Err(rollback_error) => {
                format!("保存 WorkBuddy 绑定备份失败：{error}；同时{rollback_error}")
            }
        });
    }
    Ok(())
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

pub async fn unbind(db: &Database) -> Result<(), String> {
    let Some(state) = load_state(db).await? else {
        return Ok(());
    };
    let path = models_path();
    let models = read_models_from(&path)?;
    let restored = restore_models(models, &state)?;
    let snapshot = file_snapshot(&path)?;
    write_models_to(&path, &restored)?;
    if let Err(error) = db.delete_live_backup(BACKUP_KEY).await {
        let rollback = restore_file_snapshot(&path, snapshot.as_deref());
        return Err(match rollback {
            Ok(()) => format!("删除 WorkBuddy 绑定备份失败：{error}（已回滚配置）"),
            Err(rollback_error) => {
                format!("删除 WorkBuddy 绑定备份失败：{error}；同时{rollback_error}")
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        )
        .unwrap();

        assert_eq!(models[0]["vendor"], "OfoxAI");
        assert_eq!(models[1], unrelated);
        assert_eq!(models[2]["vendor"], "OfoxAI");
        assert_eq!(models[0]["apiKey"], "sk-of-test");
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
        )
        .unwrap();
        let (models, state) = sync_models(
            models,
            Some(&first_state),
            "key",
            &[selection("model-b"), selection("model-c")],
        )
        .unwrap();

        assert_eq!(models[0], a);
        assert_eq!(models[1]["id"], "model-b");
        assert_eq!(models[2]["id"], "model-c");
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
        )
        .unwrap();
        assert_eq!(restore_models(models, &state).unwrap(), original);
    }

    #[test]
    fn one_external_edit_blocks_the_whole_update() {
        let (mut models, state) = sync_models(
            Vec::new(),
            None,
            "key",
            &[selection("model-a"), selection("model-b")],
        )
        .unwrap();
        models[1]["name"] = json!("Changed outside Ofox");
        let error = sync_models(models, Some(&state), "key", &[selection("model-c")]).unwrap_err();
        assert!(error.contains("已被外部修改"));
    }

    #[test]
    fn invalid_selection_sets_are_rejected() {
        assert!(sync_models(Vec::new(), None, "key", &[])
            .unwrap_err()
            .contains("至少"));
        assert!(sync_models(
            Vec::new(),
            None,
            "key",
            &[selection("same"), selection("same")]
        )
        .unwrap_err()
        .contains("重复"));
        let mut invalid = selection("model-a");
        invalid.supports_tool_call = false;
        assert!(sync_models(Vec::new(), None, "key", &[invalid])
            .unwrap_err()
            .contains("工具调用"));
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
