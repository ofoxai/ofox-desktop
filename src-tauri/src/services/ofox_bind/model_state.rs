//! One lock covers live-model adoption, binding health and preference reads.

use super::{file_locks, live_model, opencode_state, status, Tool, BIND_LOCK};
use crate::commands::manage_tool::read_model_from_settings;
use crate::database::Database;
use crate::model_selection::{ModelSelection, ObservedSelection, ToolModelSnapshot};
use serde_json::{json, Value};
use std::path::PathBuf;

pub(crate) fn revision_paths(tool: Tool) -> Vec<PathBuf> {
    let mut paths: Vec<_> = tool
        .files()
        .iter()
        .map(|file| file.current_path())
        .collect();
    if tool == Tool::OpenCode {
        paths.push(opencode_state::state_path());
        let directory = crate::opencode_config::get_opencode_dir();
        paths.extend(
            ["config.json", "opencode.json", "opencode.jsonc"].map(|name| directory.join(name)),
        );
    }
    paths
}

pub(crate) async fn snapshot(
    db: &Database,
    tool: Tool,
    display_id: &str,
    api_key: Option<&str>,
) -> Result<ToolModelSnapshot, String> {
    crate::agent_env::ready().await;
    let _guard = BIND_LOCK.lock().await;
    let _file_locks = file_locks(tool);
    let mut health =
        status::inspect(db, tool, api_key).unwrap_or_else(|_| status::ToolBindingStatus::unknown());
    if health.status == status::BindingStatus::Configured {
        live_model::adopt_live_model(db, tool)?;
    }
    health.env_overrides = crate::services::env_override::find(tool.app().as_str());
    let provider = db
        .get_provider_by_id(tool.provider_id(), tool.app().as_str())
        .map_err(|_| "无法读取受管模型。".to_string())?;
    let settings = provider
        .map(|provider| provider.settings_config)
        .unwrap_or(Value::Null);
    let managed = if tool == Tool::OpenCode {
        ModelSelection::Models {
            models: settings
                .get("models")
                .and_then(Value::as_object)
                .map(|models| models.keys().cloned().collect())
                .unwrap_or_default(),
        }
    } else {
        ModelSelection::Single {
            model: read_model_from_settings(&tool.app(), &settings),
        }
    };
    let fallback = match &managed {
        ModelSelection::Single { model } => model.clone(),
        ModelSelection::Models { models } => models.first().cloned().unwrap_or_default(),
    };
    let (observed, source) = if display_id == "chatgpt" {
        match super::chatgpt_state::recent_model() {
            Some(model) => (model, "appRecent"),
            None => (String::new(), "unknown"),
        }
    } else if tool == Tool::OpenCode {
        match opencode_state::live_selection().ok().flatten() {
            Some(selection) => (
                selection.model,
                if selection.from_config {
                    "config"
                } else {
                    "tui"
                },
            ),
            None => (String::new(), "unknown"),
        }
    } else {
        (live_model::display_model_or(tool, fallback), "config")
    };
    Ok(ToolModelSnapshot {
        tool_id: display_id.into(),
        write_target: tool.app().as_str().into(),
        revision: crate::model_selection::revision(db, tool.app().as_str())?,
        binding: health,
        managed_selection: managed,
        observed_selection: ObservedSelection {
            model: (!observed.is_empty()).then_some(observed),
            source: source.into(),
        },
        can_write: true,
        scope: revision_paths(tool)
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        scope_models: Vec::new(),
    })
}

pub(crate) fn event_payload(tools: &[String]) -> Value {
    json!({ "tools": tools })
}
