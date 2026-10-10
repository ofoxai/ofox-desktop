//! Read-only checks of the fields Ofox manages. Diagnostics never include values
//! or parser errors, which can contain API keys from the source configuration.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::{json, Value};

use super::json_file::{self, JsonPath};
use super::plan::read_text;
use super::{
    codex, env_file, file_locks, gemini, load_record, template, ManagedFile, Tool, BIND_LOCK,
};
use crate::database::Database;

const UNKNOWN_KEY: &str = "__ofox_saved_key_unavailable__";
/// `modified_fields` 里表示「当前服务商已不是 Ofox」，前端翻译成文案。
pub(crate) const CURRENT_PROVIDER_FIELD: &str = "current-provider";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BindingStatus {
    Configured,
    Missing,
    Modified,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolBindingStatus {
    pub status: BindingStatus,
    pub message: Option<String>,
    pub missing_files: Vec<String>,
    /// 与 Ofox 写入不一致的字段：`<文件> · <字段>`，只有名字，没有值。
    pub modified_fields: Vec<String>,
    /// 会盖过这些配置的环境变量（由命令层填，见 `services::env_override`）。
    pub env_overrides: Vec<crate::services::env_override::EnvOverride>,
}

impl ToolBindingStatus {
    pub(crate) fn configured() -> Self {
        Self {
            status: BindingStatus::Configured,
            message: None,
            missing_files: Vec::new(),
            modified_fields: Vec::new(),
            env_overrides: Vec::new(),
        }
    }

    pub(crate) fn missing(files: Vec<String>) -> Self {
        Self {
            status: BindingStatus::Missing,
            message: Some("部分 OFox 接入配置已删除，可恢复已保存的模型和绑定。".into()),
            missing_files: files,
            modified_fields: Vec::new(),
            env_overrides: Vec::new(),
        }
    }

    pub(crate) fn modified(fields: Vec<String>) -> Self {
        Self {
            status: BindingStatus::Modified,
            message: Some("OFox 接入配置已被修改，已停止自动覆盖，请检查现有配置。".into()),
            missing_files: Vec::new(),
            modified_fields: fields,
            env_overrides: Vec::new(),
        }
    }

    pub(crate) fn unknown() -> Self {
        Self {
            status: BindingStatus::Unknown,
            message: Some(
                "无法确认 OFox 接入配置，请检查文件权限、配置格式和本地绑定记录。".into(),
            ),
            missing_files: Vec::new(),
            modified_fields: Vec::new(),
            env_overrides: Vec::new(),
        }
    }
}

#[derive(Default)]
struct Comparison {
    missing: BTreeSet<String>,
    /// `<文件> · <字段>`，只记名字。
    modified: BTreeSet<String>,
    unknown: bool,
}

impl Comparison {
    /// `field` 是 `actual` 在文件里的位置（`env.ANTHROPIC_MODEL`），用来说明哪里不一致。
    fn compare(
        &mut self,
        file: &str,
        field: &str,
        actual: Option<&Value>,
        expected: Option<&Value>,
    ) {
        let mut modified = || {
            self.modified.insert(format!("{file} · {field}"));
        };
        match (actual, expected) {
            (None, None) => {}
            (None, Some(_)) => {
                self.missing.insert(file.into());
            }
            (Some(_), None) => modified(),
            (Some(actual), Some(expected)) if expected.as_str() == Some(UNKNOWN_KEY) => {
                if actual.as_str().is_some_and(|key| !key.is_empty()) {
                    self.unknown = true;
                } else {
                    modified();
                }
            }
            (Some(Value::Object(actual)), Some(Value::Object(expected))) => {
                for key in actual
                    .keys()
                    .chain(expected.keys())
                    .collect::<BTreeSet<_>>()
                {
                    self.compare(
                        file,
                        &format!("{field}.{key}"),
                        actual.get(key),
                        expected.get(key),
                    );
                }
            }
            (Some(actual), Some(expected)) if actual != expected => modified(),
            (Some(_), Some(_)) => {}
        }
    }

    fn finish(self) -> ToolBindingStatus {
        if !self.modified.is_empty() {
            ToolBindingStatus::modified(self.modified.into_iter().collect())
        } else if self.unknown {
            ToolBindingStatus::unknown()
        } else if !self.missing.is_empty() {
            ToolBindingStatus::missing(self.missing.into_iter().collect())
        } else {
            ToolBindingStatus::configured()
        }
    }
}

fn at_path(value: &Value, path: JsonPath) -> Result<Option<&Value>, String> {
    let mut node = value;
    for key in path {
        if !node.is_object() {
            return Err("受管配置字段的格式无效".into());
        }
        let Some(child) = node.get(*key) else {
            return Ok(None);
        };
        node = child;
    }
    Ok(Some(node))
}

fn compare_paths(
    comparison: &mut Comparison,
    shown: &str,
    actual: &Value,
    expected: &Value,
    paths: &[JsonPath],
) -> Result<(), String> {
    for path in paths {
        comparison.compare(
            shown,
            &path.join("."),
            at_path(actual, path)?,
            at_path(expected, path)?,
        );
    }
    Ok(())
}

fn parse_toml(text: &str) -> Result<Value, String> {
    toml::from_str::<toml::Table>(text)
        .map_err(|_| "配置不是有效 TOML".to_string())
        .and_then(|value| serde_json::to_value(value).map_err(|_| "配置无法解析".to_string()))
}

fn parse_yaml(text: &str) -> Result<Value, String> {
    let value = crate::hermes_config::parse_config_text(text)
        .map_err(|_| "配置不是有效 YAML".to_string())?;
    if value.is_null() {
        return Ok(json!({}));
    }
    serde_json::to_value(value).map_err(|_| "配置无法解析".to_string())
}

fn hermes_projection(value: &Value) -> Result<Value, String> {
    let listed = match value.get("custom_providers") {
        None => None,
        Some(Value::Array(providers)) => {
            let mut managed = providers
                .iter()
                .filter(|entry| entry.get("name").and_then(Value::as_str) == Some("ofox-hermes"));
            let first = managed.next();
            if managed.next().is_some() {
                return Err("受管配置条目重复".into());
            }
            first
        }
        _ => return Err("受管配置字段的格式无效".into()),
    };
    // Hermes v12+ 把条目迁移进 `providers:` 字典。
    let keyed = match value.get("providers") {
        None | Some(Value::Null) => None,
        Some(Value::Object(providers)) => providers.get("ofox-hermes"),
        _ => return Err("受管配置字段的格式无效".into()),
    };
    let provider = match (listed, keyed) {
        (Some(_), Some(_)) => return Err("受管配置条目重复".into()),
        (entry, None) | (None, entry) => entry,
    };
    let mut projected = json!({});
    if let Some(provider) = provider {
        projected["entry"] = provider.clone();
    }
    if let Some(model) = value.get("model") {
        if !model.is_object() {
            return Err("受管配置字段的格式无效".into());
        }
        for key in ["provider", "default"] {
            if let Some(value) = model.get(key) {
                projected[key] = value.clone();
            }
        }
    }
    Ok(projected)
}

pub(super) fn inspect(
    db: &Database,
    tool: Tool,
    api_key: Option<&str>,
) -> Result<ToolBindingStatus, String> {
    if !super::current_binding_is_ofox(db, tool)? {
        return Ok(ToolBindingStatus::modified(vec![
            CURRENT_PROVIDER_FIELD.into()
        ]));
    }
    inspect_managed_fields(db, tool, api_key)
}

/// Compatibility self-heal may still have an official active DB provider, but
/// must verify the saved OFox key/model and every managed field before writing.
pub(super) fn inspect_managed_fields(
    db: &Database,
    tool: Tool,
    api_key: Option<&str>,
) -> Result<ToolBindingStatus, String> {
    // A damaged record is not a reason to rebuild or overwrite a configuration.
    let _ = load_record(db, tool)?;
    let template = template(db, tool)?;
    let key = api_key.filter(|key| !key.is_empty()).unwrap_or(UNKNOWN_KEY);
    let mut comparison = Comparison::default();
    for file in tool.files() {
        let path = file.current_path();
        let shown = super::report::display_path(&path);
        let current = read_text(&path)?;
        if let Some(text) = current.as_deref() {
            file.validate(text)?;
        }
        match file {
            ManagedFile::CodexConfig => {
                let config = template
                    .get("config")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Ofox 模板无效".to_string())?;
                let actual = parse_toml(current.as_deref().unwrap_or_default())?;
                let expected = parse_toml(&codex::apply_patch(
                    current.as_deref().unwrap_or_default(),
                    &codex::bound_patch(config, key)?,
                )?)?;
                compare_paths(
                    &mut comparison,
                    &shown,
                    &actual,
                    &expected,
                    // 推理强度不比对：ChatGPT 和 Codex 会把界面里选的强度写回这里。
                    &[
                        &["model_provider"],
                        &["model"],
                        &["disable_response_storage"],
                        &["model_providers", "ofox"],
                    ],
                )?;
            }
            ManagedFile::ClaudeSettings => {
                let actual = json_file::parse_object(current.as_deref(), "Claude")?;
                let expected = json_file::parse_object(
                    Some(&super::claude::bound_settings(
                        current.as_deref(),
                        &template,
                        key,
                    )?),
                    "Claude",
                )?;
                compare_paths(
                    &mut comparison,
                    &shown,
                    &actual,
                    &expected,
                    &[
                        &["env", "ANTHROPIC_BASE_URL"],
                        &["env", "ANTHROPIC_AUTH_TOKEN"],
                        &["env", "ANTHROPIC_MODEL"],
                        &["env", "ANTHROPIC_API_KEY"],
                    ],
                )?;
            }
            ManagedFile::GeminiEnv => {
                let text = current.as_deref().unwrap_or_default();
                let expected = gemini::bound_env(text, &template, key)?;
                for variable in ["GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL", "GEMINI_MODEL"] {
                    comparison.compare(
                        &shown,
                        variable,
                        env_file::get_value(text, variable)
                            .map(Value::String)
                            .as_ref(),
                        env_file::get_value(&expected, variable)
                            .map(Value::String)
                            .as_ref(),
                    );
                }
            }
            ManagedFile::GeminiSettings => {
                let actual = json_file::parse_object(current.as_deref(), "Gemini")?;
                let expected = json!({"security":{"auth":{"selectedType":"gemini-api-key"}}});
                compare_paths(
                    &mut comparison,
                    &shown,
                    &actual,
                    &expected,
                    &[&["security", "auth", "selectedType"]],
                )?;
            }
            ManagedFile::OpenCodeConfig => {
                let actual = json_file::parse_object(current.as_deref(), "OpenCode")?;
                let expected = json_file::parse_object(
                    Some(&super::opencode::bound_config(
                        current.as_deref(),
                        &template,
                        key,
                    )?),
                    "OpenCode",
                )?;
                compare_paths(
                    &mut comparison,
                    &shown,
                    &actual,
                    &expected,
                    &[&["provider", "ofox-opencode"]],
                )?;
            }
            ManagedFile::OpenClawConfig => {
                let actual = json_file::parse_object(current.as_deref(), "OpenClaw")?;
                let expected = json_file::parse_object(
                    Some(&super::openclaw::bound_config(
                        current.as_deref(),
                        &template,
                        key,
                    )?),
                    "OpenClaw",
                )?;
                compare_paths(
                    &mut comparison,
                    &shown,
                    &actual,
                    &expected,
                    &[
                        &["models", "providers", "ofox-openclaw"],
                        &["agents", "defaults", "model"],
                    ],
                )?;
            }
            ManagedFile::HermesConfig => {
                let actual =
                    hermes_projection(&parse_yaml(current.as_deref().unwrap_or_default())?)?;
                let expected = hermes_projection(&parse_yaml(&super::hermes::bound_config(
                    current.as_deref(),
                    &template,
                    key,
                )?)?)?;
                for (key, field) in [
                    ("entry", "ofox-hermes"),
                    ("provider", "model.provider"),
                    ("default", "model.default"),
                ] {
                    comparison.compare(&shown, field, actual.get(key), expected.get(key));
                }
            }
        }
        if current.is_none() {
            comparison.missing.insert(shown);
        }
    }
    Ok(comparison.finish())
}

pub(crate) async fn binding_status(
    db: &Database,
    tool: Tool,
    api_key: Option<&str>,
) -> ToolBindingStatus {
    let _guard = BIND_LOCK.lock().await;
    let _file_locks = file_locks(tool);
    inspect(db, tool, api_key).unwrap_or_else(|_| ToolBindingStatus::unknown())
}
