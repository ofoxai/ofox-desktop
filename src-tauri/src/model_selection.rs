//! Shared model preferences and optimistic write guards for console, tray and Profiles.
//! Secrets and complete agent configurations never cross this IPC boundary.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app_config::{AppType, BindableTool};
use crate::database::Database;
use crate::services::ofox_bind::status::ToolBindingStatus;

pub(crate) static SELECTION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) const TOOLS: &[&str] = &[
    "claude",
    "codex",
    "chatgpt",
    "opencode",
    "openclaw",
    "hermes",
    "gemini",
    "workbuddy",
    "dsh",
    "pi",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ModelSelection {
    Single { model: String },
    Models { models: Vec<String> },
}

impl ModelSelection {
    pub(crate) fn ids(&self) -> Vec<&str> {
        match self {
            Self::Single { model } => {
                if model.is_empty() {
                    vec![]
                } else {
                    vec![model]
                }
            }
            Self::Models { models } => models.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileTarget {
    pub target: String,
    pub selection: ModelSelection,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedSelection {
    pub model: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeModel {
    pub scope: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolModelSnapshot {
    pub tool_id: String,
    pub write_target: String,
    pub revision: String,
    pub binding: ToolBindingStatus,
    pub managed_selection: ModelSelection,
    pub observed_selection: ObservedSelection,
    pub supports_default: bool,
    pub can_write: bool,
    pub scope: Vec<String>,
    pub scope_models: Vec<ScopeModel>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelSnapshot {
    pub tools: Vec<ToolModelSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedModel {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSelectionReport {
    pub snapshot: ToolModelSnapshot,
    pub saved: Vec<String>,
    pub skipped: Vec<SkippedModel>,
    pub write_revision: Option<String>,
}

pub(crate) fn canonical_tool(input: &str) -> Result<String, String> {
    let id = input.trim().to_ascii_lowercase();
    if !TOOLS.contains(&id.as_str()) {
        return Err("未知的工具。".into());
    }
    Ok(if id == "chatgpt" { "codex".into() } else { id })
}

pub(crate) fn supports_default(tool: &str) -> bool {
    !matches!(tool, "workbuddy" | "opencode" | "pi" | "dsh")
}

fn clean_id(id: &str, allow_empty: bool) -> Result<String, String> {
    let id = id.trim();
    if (id.is_empty() && !allow_empty)
        || id.chars().count() > 512
        || id.chars().any(char::is_control)
        || id.contains("://")
        || ["sk-", "sk_", "ghp_", "github_pat_", "eyJ"]
            .iter()
            .any(|prefix| id.starts_with(prefix))
    {
        return Err("模型 ID 无效；这里只接受模型选择。".into());
    }
    Ok(id.into())
}

pub(crate) fn validate_selection(
    tool: &str,
    selection: ModelSelection,
) -> Result<ModelSelection, String> {
    let tool = canonical_tool(tool)?;
    match selection {
        ModelSelection::Single { model } if !matches!(tool.as_str(), "opencode" | "workbuddy") => {
            let model = clean_id(&model, supports_default(&tool))?;
            let model = if tool == "gemini" {
                crate::gemini_config::normalize_gemini_env_model(&model)
            } else {
                model
            };
            Ok(ModelSelection::Single { model })
        }
        ModelSelection::Models { models } if matches!(tool.as_str(), "opencode" | "workbuddy") => {
            if models.is_empty() || models.len() > 2000 {
                return Err("请选择至少一个模型。".into());
            }
            let mut seen = HashSet::new();
            let mut clean = Vec::new();
            for model in models {
                let id = clean_id(&model, false)?;
                if seen.insert(id.clone()) {
                    clean.push(id);
                }
            }
            Ok(ModelSelection::Models { models: clean })
        }
        _ => Err("模型选择类型与工具不一致。".into()),
    }
}

pub(crate) fn validate_targets(targets: Vec<ProfileTarget>) -> Result<Vec<ProfileTarget>, String> {
    if targets.is_empty() || targets.len() > TOOLS.len() {
        return Err("Profile 至少需要一个工具目标。".into());
    }
    let mut out: Vec<ProfileTarget> = Vec::new();
    for target in targets {
        let id = canonical_tool(&target.target)?;
        let selection = validate_selection(&id, target.selection)?;
        if let Some(previous) = out.iter().find(|item| item.target == id) {
            if previous.selection != selection {
                return Err("Codex 与 ChatGPT 共用配置，不能保存不同的默认模型。".into());
            }
            continue;
        }
        out.push(ProfileTarget {
            target: id,
            selection,
        });
    }
    Ok(out)
}

pub(crate) fn digest(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "无法生成模型配置版本。".to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn file_material(path: &Path) -> Result<Value, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(path).ok().map(|m| m.permissions().mode())
            };
            #[cfg(not(unix))]
            let mode: Option<u32> = None;
            Ok(json!({
                "path": path.to_string_lossy(),
                "hash": format!("{:x}", Sha256::digest(bytes)),
                "mode": mode,
            }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(json!({ "path": path.to_string_lossy(), "missing": true }))
        }
        Err(_) => Err("无法读取模型配置，请检查权限后重试。".into()),
    }
}

/// Synchronous material is reused inside each adapter's write lock.
pub(crate) fn revision(db: &Database, input: &str) -> Result<String, String> {
    let target = canonical_tool(input)?;
    let binding = db
        .get_bind_record(&target)
        .map_err(|_| "无法读取绑定记录。".to_string())?;
    let tool = BindableTool::from_str(&target).map_err(|_| "未知的工具。".to_string())?;
    let key = crate::settings::get_api_key_meta(tool)
        .map(|meta| json!({ "id": meta.key_id, "owner": meta.owner, "revoked": meta.revoked }));
    let (provider, files, native) = if matches!(target.as_str(), "pi" | "dsh") {
        (
            Value::Null,
            vec![],
            crate::native_tools::revision_material(db, tool)?,
        )
    } else if target == "workbuddy" {
        (
            Value::Null,
            vec![crate::workbuddy_config::models_path()],
            Value::Null,
        )
    } else {
        let app = AppType::from_str(&target).map_err(|_| "未知的工具。".to_string())?;
        let current = db
            .get_current_provider(&target)
            .map_err(|_| "无法读取模型配置。".to_string())?;
        let provider = current
            .as_ref()
            .map(|id| db.get_provider_by_id(id, app.as_str()))
            .transpose()
            .map_err(|_| "无法读取模型配置。".to_string())?
            .flatten();
        let value = provider.map(|p| json!({ "id": p.id, "settings": p.settings_config }));
        let primary = crate::commands::manage_tool::get_tool_config_file_path(target.clone())?;
        let mut paths = vec![PathBuf::from(primary)];
        if let Some((legacy, _)) = crate::services::ofox_bind::tool_for(&target) {
            paths.extend(crate::services::ofox_bind::model_state::revision_paths(
                legacy,
            ));
            paths.sort();
            paths.dedup();
        }
        (value.unwrap_or(Value::Null), paths, Value::Null)
    };
    let files = files
        .iter()
        .map(|path| file_material(path))
        .collect::<Result<Vec<_>, _>>()?;
    digest(&json!({
        "tool": target, "apex": crate::ofox_apex::current_apex(), "provider": provider,
        "binding": binding.map(|row| row.record), "key": key, "files": files, "native": native,
    }))
}

#[derive(Clone)]
pub(crate) struct ExpectedWrite {
    pub target: String,
    pub revision: String,
}

tokio::task_local! {
    pub(crate) static EXPECTED_WRITE: ExpectedWrite;
    pub(crate) static WRITE_RECEIPT: Arc<Mutex<Option<String>>>;
}

/// Capture the state produced by this write before releasing the adapter lock.
/// Later diagnostics may adopt a newer CLI preference and must not become an undo receipt.
pub(crate) fn record_write_receipt(db: &Database, tool: &str) {
    let Ok(receipt) = WRITE_RECEIPT.try_with(Arc::clone) else {
        return;
    };
    let Ok(target) = canonical_tool(tool) else {
        return;
    };
    if EXPECTED_WRITE
        .try_with(|expected| expected.target != target)
        .unwrap_or(true)
    {
        return;
    }
    let value = revision(db, &target).ok();
    if let Ok(mut receipt) = receipt.lock() {
        *receipt = value;
    };
}

/// Invoked after acquiring the adapter lock, immediately before mutating files or DB.
pub(crate) fn check_write_revision(db: &Database, tool: &str) -> Result<(), String> {
    let expected = EXPECTED_WRITE.try_with(Clone::clone).ok();
    if let Some(expected) = expected {
        let target = canonical_tool(tool)?;
        if expected.target != target || revision(db, &target)? != expected.revision {
            return Err("模型配置已变化，请刷新后重试。".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(model: &str) -> ModelSelection {
        ModelSelection::Single {
            model: model.into(),
        }
    }

    #[test]
    fn shared_configuration_is_canonical_and_conflicting_targets_are_rejected() {
        let targets = vec![
            ProfileTarget {
                target: "chatgpt".into(),
                selection: single("model-a"),
            },
            ProfileTarget {
                target: "codex".into(),
                selection: single("model-a"),
            },
        ];
        let normalized = validate_targets(targets).unwrap();
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].target, "codex");
        assert!(validate_targets(vec![
            normalized[0].clone(),
            ProfileTarget {
                target: "chatgpt".into(),
                selection: single("model-b")
            },
        ])
        .is_err());
    }

    #[test]
    fn collections_preserve_order_and_missing_targets_are_not_filled() {
        let targets = validate_targets(vec![ProfileTarget {
            target: "opencode".into(),
            selection: ModelSelection::Models {
                models: vec!["b".into(), "a".into(), "b".into()],
            },
        }])
        .unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].selection,
            ModelSelection::Models {
                models: vec!["b".into(), "a".into()]
            }
        );
    }

    #[test]
    fn gemini_recipe_matches_the_id_written_and_read_by_the_agent() {
        let value = validate_selection("gemini", single("google/gemini-2.5-flash")).unwrap();
        assert_eq!(value, single("gemini-2.5-flash"));
        assert_eq!(
            validate_selection("gemini", single("bailian/qwen-max")).unwrap(),
            single("bailian/qwen-max")
        );
        let mut settings = json!({});
        crate::commands::manage_tool::write_model_into_settings(
            &AppType::Gemini,
            &mut settings,
            "google/gemini-2.5-flash",
        )
        .unwrap();
        assert_eq!(
            crate::commands::manage_tool::read_model_from_settings(&AppType::Gemini, &settings),
            "gemini-2.5-flash"
        );
    }

    #[test]
    fn default_is_explicit_and_only_available_for_supported_tools() {
        assert_eq!(validate_selection("codex", single("")).unwrap(), single(""));
        for id in ["pi", "dsh", "workbuddy", "opencode"] {
            assert!(validate_selection(id, single("")).is_err());
        }
        assert!(validate_selection("workbuddy", single("model-a")).is_err());
        assert!(validate_selection(
            "pi",
            ModelSelection::Models {
                models: vec!["a".into()]
            }
        )
        .is_err());
    }

    #[test]
    fn credentials_urls_and_extra_fields_are_not_profile_data() {
        for id in ["sk-of-secret", "https://api.example.com/v1", "model\nother"] {
            assert!(validate_selection("pi", single(id)).is_err());
        }
        assert!(serde_json::from_value::<ModelSelection>(json!({
            "kind": "single", "model": "a", "apiKey": "secret",
        }))
        .is_err());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn write_guard_rejects_external_changes_and_wrong_targets() {
        let _home = crate::settings::TempSettingsHome::new();
        let db = Database::memory().unwrap();
        let path = crate::workbuddy_config::models_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[]").unwrap();
        let before = revision(&db, "workbuddy").unwrap();
        EXPECTED_WRITE
            .scope(
                ExpectedWrite {
                    target: "workbuddy".into(),
                    revision: before.clone(),
                },
                async {
                    assert!(check_write_revision(&db, "workbuddy").is_ok());
                    assert!(check_write_revision(&db, "pi").is_err());
                    std::fs::write(&path, "[{\"modelId\":\"user-added\"}]").unwrap();
                    assert!(check_write_revision(&db, "workbuddy").is_err());
                },
            )
            .await;
        assert_ne!(before, revision(&db, "workbuddy").unwrap());
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "[{\"modelId\":\"user-added\"}]"
        );
    }

    #[test]
    fn revision_material_includes_content_and_removal_without_exposing_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "secret-value-one").unwrap();
        let first = file_material(&path).unwrap();
        assert!(!first.to_string().contains("secret-value-one"));
        std::fs::write(&path, "secret-value-two").unwrap();
        assert_ne!(first, file_material(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(file_material(&path).unwrap()["missing"], true);
    }
}
