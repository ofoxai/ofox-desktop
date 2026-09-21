//! Native WorkBuddy custom-model configuration.
//!
//! WorkBuddy stores its current desktop model list as a top-level JSON array
//! in `~/.workbuddy/models.json`. Ofox owns exactly one entry at a time. A
//! versioned backup kept in `proxy_live_backup` remembers any same-id entry
//! that was displaced so model changes and unbinds can restore user data
//! without replacing the rest of the file.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{atomic_write, get_home_dir};
use crate::database::Database;

const BACKUP_VERSION: u32 = 1;
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
pub struct WorkBuddyBindingState {
    pub version: u32,
    pub current_model_id: String,
    pub original_entry: Option<Value>,
    pub original_index: usize,
    pub last_written_entry: Value,
}

pub fn models_path() -> PathBuf {
    get_home_dir().join(".workbuddy").join("models.json")
}

fn validate_selection(selection: &WorkBuddyModelSelection) -> Result<(), String> {
    if selection.id.trim().is_empty() {
        return Err("WorkBuddy 模型 ID 不能为空".to_string());
    }
    if !selection.supports_tool_call {
        return Err("WorkBuddy 仅支持接入具备工具调用能力的模型".to_string());
    }
    Ok(())
}

fn managed_entry(api_key: &str, selection: &WorkBuddyModelSelection) -> Value {
    json!({
        "id": selection.id.trim(),
        "name": if selection.name.trim().is_empty() {
            selection.id.trim()
        } else {
            selection.name.trim()
        },
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

fn remove_managed_entry(
    models: &mut Vec<Value>,
    state: &WorkBuddyBindingState,
) -> Result<(), String> {
    let Some(index) = models
        .iter()
        .position(|entry| model_id(entry) == Some(state.current_model_id.as_str()))
    else {
        return Err(format!(
            "WorkBuddy 中 Ofox 模型 {} 已被外部删除，已停止覆盖",
            state.current_model_id
        ));
    };
    if models[index] != state.last_written_entry {
        return Err(format!(
            "WorkBuddy 模型 {} 已被外部修改，已停止覆盖；请先在 WorkBuddy 中确认配置",
            state.current_model_id
        ));
    }
    models.remove(index);
    if let Some(original) = state.original_entry.clone() {
        let restore_index = state.original_index.min(models.len());
        models.insert(restore_index, original);
    }
    Ok(())
}

fn switch_models(
    mut models: Vec<Value>,
    previous: Option<&WorkBuddyBindingState>,
    api_key: &str,
    selection: &WorkBuddyModelSelection,
) -> Result<(Vec<Value>, WorkBuddyBindingState), String> {
    validate_selection(selection)?;
    if let Some(state) = previous {
        remove_managed_entry(&mut models, state)?;
    }

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
    let insert_index = original_index.min(models.len());
    models.insert(insert_index, entry.clone());

    Ok((
        models,
        WorkBuddyBindingState {
            version: BACKUP_VERSION,
            current_model_id: target_id.to_string(),
            original_entry,
            original_index,
            last_written_entry: entry,
        },
    ))
}

fn restore_models(
    mut models: Vec<Value>,
    state: &WorkBuddyBindingState,
) -> Result<Vec<Value>, String> {
    remove_managed_entry(&mut models, state)?;
    Ok(models)
}

async fn load_state(db: &Database) -> Result<Option<WorkBuddyBindingState>, String> {
    let Some(backup) = db
        .get_live_backup(BACKUP_KEY)
        .await
        .map_err(|e| format!("读取 WorkBuddy 绑定备份失败：{e}"))?
    else {
        return Ok(None);
    };
    let state: WorkBuddyBindingState = serde_json::from_str(&backup.original_config)
        .map_err(|e| format!("WorkBuddy 绑定备份已损坏：{e}"))?;
    if state.version != BACKUP_VERSION {
        return Err(format!(
            "不支持的 WorkBuddy 绑定备份版本：{}",
            state.version
        ));
    }
    Ok(Some(state))
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

pub async fn bind_or_switch(
    db: &Database,
    api_key: &str,
    selection: &WorkBuddyModelSelection,
) -> Result<(), String> {
    let path = models_path();
    let previous = load_state(db).await?;
    let models = read_models_from(&path)?;
    let (next_models, next_state) = switch_models(models, previous.as_ref(), api_key, selection)?;
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

pub async fn active_model(db: &Database) -> Result<String, String> {
    Ok(load_state(db)
        .await?
        .map(|state| state.current_model_id)
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
    fn bind_preserves_unrelated_models_and_replaces_same_id() {
        let original = json!({"id":"openai/gpt-5", "vendor":"User", "extra": 1});
        let unrelated = json!({"id":"local/model", "vendor":"Local"});
        let (models, state) = switch_models(
            vec![unrelated.clone(), original.clone()],
            None,
            "sk-of-test",
            &selection("openai/gpt-5"),
        )
        .unwrap();

        assert_eq!(models[0], unrelated);
        assert_eq!(models[1]["vendor"], "OfoxAI");
        assert_eq!(models[1]["url"], OFOX_CHAT_COMPLETIONS_URL);
        assert_eq!(state.original_entry, Some(original));
    }

    #[test]
    fn switch_restores_previous_collision_before_managing_next_model() {
        let first_original = json!({"id":"model-a", "vendor":"User A"});
        let second_original = json!({"id":"model-b", "vendor":"User B"});
        let (models, first_state) = switch_models(
            vec![first_original.clone(), second_original.clone()],
            None,
            "sk-of-test",
            &selection("model-a"),
        )
        .unwrap();
        let (models, second_state) = switch_models(
            models,
            Some(&first_state),
            "sk-of-test",
            &selection("model-b"),
        )
        .unwrap();

        assert_eq!(models[0], first_original);
        assert_eq!(models[1]["vendor"], "OfoxAI");
        assert_eq!(second_state.original_entry, Some(second_original));
    }

    #[test]
    fn unbind_restores_exact_original_entry() {
        let original = json!({"id":"model-a", "vendor":"User", "unknown": {"x": 1}});
        let (models, state) = switch_models(
            vec![original.clone()],
            None,
            "sk-of-test",
            &selection("model-a"),
        )
        .unwrap();
        assert_eq!(restore_models(models, &state).unwrap(), vec![original]);
    }

    #[test]
    fn external_edit_blocks_switch_and_unbind() {
        let (mut models, state) =
            switch_models(Vec::new(), None, "sk-of-test", &selection("model-a")).unwrap();
        models[0]["name"] = json!("Changed outside Ofox");

        let error = restore_models(models, &state).unwrap_err();
        assert!(error.contains("已被外部修改"));
    }

    #[test]
    fn tool_call_support_is_required() {
        let mut invalid = selection("model-a");
        invalid.supports_tool_call = false;
        let error = switch_models(Vec::new(), None, "sk-of-test", &invalid).unwrap_err();
        assert!(error.contains("工具调用"));
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
