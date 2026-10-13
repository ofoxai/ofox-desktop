//! OpenCode 自己的优先级是：命令行 `--model`、配置里的顶层 `model`、TUI 状态文件
//! `<state>/opencode/model.json` 的 `recent` 第一项。用户在 TUI 里选的模型只进
//! `recent`，所以用户配置里写了 `model` 时，直接启动 opencode 会被它盖住；桌面端打开
//! 终端时用 `--model` 带上 `recent` 第一项，让用户最近的选择生效。Ofox 不写配置里的
//! `model`，只读它（没有 `recent` 时才看）；绑定 / 保存模型时把 Ofox 的模型放到
//! `recent` 最前面，其余字段（收藏、思考强度）原样保留。
//!
//! 全局配置有 `config.json`、`opencode.json`、`opencode.jsonc` 三种文件，OpenCode
//! 按这个顺序合并，后面的覆盖前面的。用户自己配的服务商条目（比如指向 Ofox 网关的
//! `openai`）也可能在 jsonc 里，所以查条目也要看这几个文件。

use std::path::PathBuf;

use serde_json::{json, Value};

use super::read_text;

pub(crate) const PROVIDER_ID: &str = "ofox-opencode";
/// OpenCode 自己只留最近的 10 个。
const MAX_RECENT: usize = 10;

/// OpenCode 用 xdg-basedir：`$XDG_STATE_HOME`，没设就是 `~/.local/state`（各平台一致）。
pub(super) fn state_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| crate::config::get_home_dir().join(".local").join("state"));
    base.join("opencode").join("model.json")
}

fn parse(text: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(text)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| "OpenCode 的 model.json 格式无效".to_string())
}

/// OpenCode 实际会选的模型。
pub(crate) struct Selection {
    pub provider: String,
    pub model: String,
    /// 来自配置里的 `model`（不是 TUI 记的）。
    pub from_config: bool,
}

/// 全局配置文件，按 OpenCode 的合并顺序（后面的覆盖前面的）。
fn config_values() -> Vec<Value> {
    let dir = crate::opencode_config::get_opencode_dir();
    ["config.json", "opencode.json", "opencode.jsonc"]
        .iter()
        .filter_map(|name| read_text(&dir.join(name)).ok().flatten())
        .filter_map(|text| super::json_file::parse_object(Some(&text), "OpenCode").ok())
        .collect()
}

fn split_model(model: &str) -> Option<(String, String)> {
    let (provider, model) = model.split_once('/')?;
    (!provider.is_empty() && !model.is_empty()).then(|| (provider.into(), model.into()))
}

/// 用户最近一次选的模型：TUI 记的第一项优先（那是用户最近的明确选择，桌面端启动时
/// 会用 `--model` 带上它），没有才看配置里的 `model`。
pub(crate) fn live_selection() -> Result<Option<Selection>, String> {
    if let Some((provider, model)) = recent_selection()? {
        return Ok(Some(Selection {
            provider,
            model,
            from_config: false,
        }));
    }
    let configured = config_values()
        .iter()
        .rev()
        .find_map(|config| config.get("model")?.as_str().and_then(split_model));
    Ok(configured.map(|(provider, model)| Selection {
        provider,
        model,
        from_config: true,
    }))
}

/// 这个服务商条目（任一全局配置文件里）是不是指向 Ofox 网关。
pub(crate) fn is_gateway(provider: &str) -> bool {
    config_values()
        .iter()
        .rev()
        .any(|config| super::route::opencode_is_gateway(config, provider))
}

/// `recent` 第一项：(服务商, 模型)。文件不在、没有这一项都返回 `None`。
fn recent_selection() -> Result<Option<(String, String)>, String> {
    let Some(text) = read_text(&state_path())? else {
        return Ok(None);
    };
    let state = parse(&text)?;
    let first = state
        .get("recent")
        .and_then(Value::as_array)
        .and_then(|recent| recent.first());
    let field = |name: &str| {
        first
            .and_then(|entry| entry.get(name))
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty())
    };
    Ok(field("providerID").zip(field("modelID")))
}

/// 保存模型后让 Ofox 的模型成为 OpenCode 下次启动用的：用户在 TUI 里当前选的如果还在
/// 这一组里就不动，否则把这一组的第一个放到 `recent` 最前面。没有变化返回 `None`。
pub(super) fn select_ofox_model(
    current: Option<&str>,
    models: &[&str],
) -> Result<Option<String>, String> {
    let Some(&first) = models.first() else {
        return Ok(None);
    };
    let mut state = match current {
        Some(text) => parse(text)?,
        None => json!({}),
    };
    let current_is_kept = state
        .get("recent")
        .and_then(Value::as_array)
        .and_then(|recent| recent.first())
        .is_some_and(|entry| {
            entry.get("providerID").and_then(Value::as_str) == Some(PROVIDER_ID)
                && entry
                    .get("modelID")
                    .and_then(Value::as_str)
                    .is_some_and(|id| models.contains(&id))
        });
    if current_is_kept {
        return Ok(None);
    }
    let chosen = json!({ "providerID": PROVIDER_ID, "modelID": first });
    let mut recent = vec![chosen.clone()];
    if let Some(old) = state.get("recent").and_then(Value::as_array) {
        recent.extend(old.iter().filter(|entry| **entry != chosen).cloned());
    }
    recent.truncate(MAX_RECENT);
    state["recent"] = Value::Array(recent);
    serde_json::to_string_pretty(&state)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// 模板里的模型 id（对象写法的 key），按保存的顺序。
pub(super) fn template_models(template: &Value) -> Vec<&str> {
    template
        .get("models")
        .and_then(Value::as_object)
        .map(|models| {
            models
                .keys()
                .map(String::as_str)
                .filter(|id| !id.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}
