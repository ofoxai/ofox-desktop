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
    /// Its Ofox key was stopped at logout; signing in issues a new one.
    Paused,
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
            message: Some("部分 Ofox 接入配置已删除，可恢复已保存的模型和绑定。".into()),
            missing_files: files,
            modified_fields: Vec::new(),
            env_overrides: Vec::new(),
        }
    }

    pub(crate) fn modified(fields: Vec<String>) -> Self {
        Self {
            status: BindingStatus::Modified,
            message: Some("Ofox 接入配置已被修改，已停止自动覆盖，请检查现有配置。".into()),
            missing_files: Vec::new(),
            modified_fields: fields,
            env_overrides: Vec::new(),
        }
    }

    pub(crate) fn paused() -> Self {
        Self {
            status: BindingStatus::Paused,
            message: Some("这个工具的 Ofox Key 已暂停，登录后打开 Ofox 会自动恢复。".into()),
            missing_files: Vec::new(),
            modified_fields: Vec::new(),
            env_overrides: Vec::new(),
        }
    }

    pub(crate) fn unknown() -> Self {
        Self {
            status: BindingStatus::Unknown,
            message: Some(
                "无法确认 Ofox 接入配置，请检查文件权限、配置格式和本地绑定记录。".into(),
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
        // 用户自己配的、同样指向 Ofox 网关的条目：用的还是 Ofox。
        let own_gateway = model
            .get("provider")
            .and_then(Value::as_str)
            .is_some_and(|name| {
                name != "ofox-hermes" && super::route::hermes_is_gateway(value, name)
            });
        if own_gateway {
            projected["provider"] = Value::String("ofox-hermes".into());
        }
    }
    Ok(projected)
}

/// Hermes 条目里决定「连到哪、怎么连」的字段：列表写法和 `providers:` 字典写法
/// 的名字不同，两种都列上，没有的字段两边都是空。
const HERMES_ACCESS_FIELDS: [&str; 5] = ["base_url", "api", "api_key", "api_mode", "transport"];

/// OpenClaw 默认模型指向哪个服务商（`agents.defaults.model` 可以是字符串，也可以
/// 是 `{primary, fallbacks}`）。
const OPENCLAW_PROVIDER: &str = "ofox-openclaw";

fn openclaw_route(config: &Value) -> Result<Option<Value>, String> {
    let Some(model) = at_path(config, &["agents", "defaults", "model"])? else {
        return Ok(None);
    };
    let primary = match model {
        Value::String(primary) => primary.as_str(),
        Value::Object(_) => model.get("primary").and_then(Value::as_str).unwrap_or(""),
        _ => return Err("受管配置字段的格式无效".into()),
    };
    let provider = primary.split('/').next().unwrap_or_default();
    // 用户自己配的、同样指向 Ofox 网关的条目：用的还是 Ofox。
    let provider =
        if provider != OPENCLAW_PROVIDER && super::route::openclaw_is_gateway(config, provider) {
            OPENCLAW_PROVIDER
        } else {
            provider
        };
    Ok(Some(Value::String(provider.to_string())))
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
                let mut actual = parse_toml(current.as_deref().unwrap_or_default())?;
                // 默认服务商是用户自己配的、同样指向 Ofox 网关的条目：用的还是 Ofox。
                let own_gateway = actual
                    .get("model_provider")
                    .and_then(Value::as_str)
                    .is_some_and(|name| {
                        name != "ofox" && super::route::codex_is_gateway(&actual, name)
                    });
                if own_gateway {
                    actual["model_provider"] = Value::String("ofox".into());
                }
                let expected = parse_toml(&codex::apply_patch(
                    current.as_deref().unwrap_or_default(),
                    &codex::bound_patch(config, key)?,
                )?)?;
                compare_paths(
                    &mut comparison,
                    &shown,
                    &actual,
                    &expected,
                    // 推理强度、模型都不比对：它们是用户的选择，ChatGPT 和 Codex 会把
                    // 界面或 `/model` 里选的值写回这里。
                    &compared_codex_paths(),
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
                    // 模型是用户的选择，不比对。
                    &[
                        &["env", "ANTHROPIC_BASE_URL"],
                        &["env", "ANTHROPIC_AUTH_TOKEN"],
                        &["env", "ANTHROPIC_API_KEY"],
                    ],
                )?;
            }
            ManagedFile::GeminiEnv => {
                let text = current.as_deref().unwrap_or_default();
                let expected = gemini::bound_env(text, &template, key)?;
                // 模型是用户的选择，不比对。
                for variable in ["GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL"] {
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
                        false,
                    )?),
                    "OpenCode",
                )?;
                compare_paths(
                    &mut comparison,
                    &shown,
                    &actual,
                    &expected,
                    // 只比接入字段。条目里的模型列表是用户的选择，agent 里加模型、
                    // 切模型都不算冲突。
                    &[
                        &["provider", "ofox-opencode", "npm"],
                        &["provider", "ofox-opencode", "options", "baseURL"],
                        &["provider", "ofox-opencode", "options", "apiKey"],
                    ],
                )?;
                // 用户在 TUI 里选了别家的模型：下次启动就不走 Ofox 了。用户自己配的
                // 条目只要也指向 Ofox 网关（比如自带 key 的 `ofox`），仍然是在用 Ofox。
                if let Some(selection) = super::opencode_state::live_selection()? {
                    let provider = selection.provider;
                    let still_ofox = provider == super::opencode_state::PROVIDER_ID
                        || super::opencode_state::is_gateway(&provider);
                    let shown_provider = if still_ofox {
                        super::opencode_state::PROVIDER_ID.to_string()
                    } else {
                        provider
                    };
                    comparison.compare(
                        &shown,
                        if selection.from_config {
                            "model"
                        } else {
                            "model.json · recent[0]"
                        },
                        Some(&Value::String(shown_provider)),
                        Some(&Value::String(super::opencode_state::PROVIDER_ID.into())),
                    );
                }
            }
            ManagedFile::OpenClawConfig => {
                let actual = json_file::parse_object(current.as_deref(), "OpenClaw")?;
                let expected = json_file::parse_object(
                    Some(&super::openclaw::bound_config(
                        current.as_deref(),
                        &template,
                        key,
                        false,
                    )?),
                    "OpenClaw",
                )?;
                compare_paths(
                    &mut comparison,
                    &shown,
                    &actual,
                    &expected,
                    // 条目只比接入字段；默认模型只要还指向 Ofox 条目就行，具体选哪个
                    // 模型是用户的选择。
                    &[
                        &["models", "providers", "ofox-openclaw", "baseUrl"],
                        &["models", "providers", "ofox-openclaw", "apiKey"],
                        &["models", "providers", "ofox-openclaw", "api"],
                    ],
                )?;
                comparison.compare(
                    &shown,
                    "agents.defaults.model",
                    openclaw_route(&actual)?.as_ref(),
                    openclaw_route(&expected)?.as_ref(),
                );
            }
            ManagedFile::HermesConfig => {
                let actual =
                    hermes_projection(&parse_yaml(current.as_deref().unwrap_or_default())?)?;
                let expected = hermes_projection(&parse_yaml(&super::hermes::bound_config(
                    current.as_deref(),
                    &template,
                    key,
                    false,
                )?)?)?;
                // 条目只比接入字段；`model.default`、条目里的模型列表是用户的选择。
                for field in HERMES_ACCESS_FIELDS {
                    comparison.compare(
                        &shown,
                        &format!("ofox-hermes.{field}"),
                        actual.get("entry").and_then(|entry| entry.get(field)),
                        expected.get("entry").and_then(|entry| entry.get(field)),
                    );
                }
                comparison.compare(
                    &shown,
                    "model.provider",
                    actual.get("provider"),
                    expected.get("provider"),
                );
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
    crate::agent_env::ready().await;
    let _guard = BIND_LOCK.lock().await;
    let _file_locks = file_locks(tool);
    inspect(db, tool, api_key).unwrap_or_else(|_| ToolBindingStatus::unknown())
}

/// Codex 配置里要和 Ofox 模板比对的字段。Codex 明确不认的 `disable_response_storage`
/// 我们不再写，老版本留下的也不算被改动，所以只有 Codex 认它时才比。
fn compared_codex_paths() -> Vec<&'static [&'static str]> {
    let mut paths: Vec<&'static [&'static str]> =
        vec![&["model_provider"], &["model_providers", "ofox"]];
    if super::codex_probe::should_write_flag() {
        paths.push(&["disable_response_storage"]);
    }
    paths
}
