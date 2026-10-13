//! OpenCode 的当前模型不在 `opencode.json` 里，而是记在 TUI 状态文件
//! `<state>/opencode/model.json` 的 `recent` 里（第一项就是下次启动用的模型；
//! 命令行 `--model` 和配置里的 `model` 优先级更高，Ofox 两者都不写）。
//!
//! 在 TUI 里切模型，OpenCode 会把它放到 `recent` 最前面。这里只读这一项来判断用户
//! 选了哪个模型，绑定 / 保存模型时把 Ofox 的模型放到最前面；其余字段（收藏、思考
//! 强度）原样保留。

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

/// `recent` 第一项：(服务商, 模型)。文件不在、没有这一项都返回 `None`。
pub(crate) fn live_selection() -> Result<Option<(String, String)>, String> {
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

/// 把 Ofox 的模型放到 `recent` 最前面，返回新的文件内容；没有变化返回 `None`。
pub(super) fn select_ofox_model(
    current: Option<&str>,
    model: &str,
) -> Result<Option<String>, String> {
    let mut state = match current {
        Some(text) => parse(text)?,
        None => json!({}),
    };
    let chosen = json!({ "providerID": PROVIDER_ID, "modelID": model });
    let mut recent = vec![chosen.clone()];
    if let Some(old) = state.get("recent").and_then(Value::as_array) {
        recent.extend(old.iter().filter(|entry| **entry != chosen).cloned());
    }
    recent.truncate(MAX_RECENT);
    if state.get("recent") == Some(&Value::Array(recent.clone())) {
        return Ok(None);
    }
    state["recent"] = Value::Array(recent);
    serde_json::to_string_pretty(&state)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// 模板里的第一个模型 id（对象写法的 key）。
pub(super) fn template_model(template: &Value) -> Option<&str> {
    template
        .get("models")?
        .as_object()?
        .keys()
        .next()
        .map(String::as_str)
        .filter(|id| !id.trim().is_empty())
}
