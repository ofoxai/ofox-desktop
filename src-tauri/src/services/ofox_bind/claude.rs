//! Claude Code 的绑定配置：只改 `settings.json` 里 `env` 下的接入字段，
//! `mcpServers`、hooks、permissions、其它 env 一律不动。

use std::path::PathBuf;

use serde_json::Value;

use crate::config::{get_claude_settings_path, FileTxn};
use crate::ofox_apex::mentions_ofox_gateway;

use super::json_file::{self, JsonPath, KeyStyle};
use super::plan::{read_text, FileEdit, RestorePlan};

const LABEL: &str = "Claude 的 settings.json";
const BASE_URL: JsonPath = &["env", "ANTHROPIC_BASE_URL"];
const AUTH_TOKEN: JsonPath = &["env", "ANTHROPIC_AUTH_TOKEN"];
const MODEL: JsonPath = &["env", "ANTHROPIC_MODEL"];
/// 中转站常用的 x-api-key。绑定期间拿掉，免得和 Ofox 的 key 一起发给 Ofox。
const API_KEY: JsonPath = &["env", "ANTHROPIC_API_KEY"];
/// 解绑时一律还原成绑定前的值（原来没有就删掉）。
const LEAVES: &[JsonPath] = &[BASE_URL, AUTH_TOKEN, MODEL, API_KEY];
const CONTAINERS: &[JsonPath] = &[&["env"]];

pub(crate) fn settings_path() -> PathBuf {
    get_claude_settings_path()
}

pub(crate) fn validate(text: &str) -> Result<(), String> {
    json_file::validate(text, LABEL)
}

/// `env.ANTHROPIC_BASE_URL` 指向 Ofox 网关——当前（或旧版本）绑定留下的配置。
pub(crate) fn is_ofox_bound(text: &str) -> bool {
    json_file::parse_object(Some(text), LABEL)
        .ok()
        .and_then(|value| {
            json_file::get(&value, BASE_URL)?
                .as_str()
                .map(mentions_ofox_gateway)
        })
        .unwrap_or(false)
}

fn template_str(template: &Value, path: JsonPath) -> Option<&str> {
    json_file::get(template, path)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

/// 按 DB 里 `ofox-claude` 模板的 settings_config 生成绑定后的 settings.json。
pub(crate) fn bound_settings(
    current: Option<&str>,
    template: &Value,
    api_key: &str,
) -> Result<String, String> {
    let base_url = template_str(template, BASE_URL)
        .ok_or_else(|| "Ofox Claude 模板缺少 ANTHROPIC_BASE_URL".to_string())?;
    let before = json_file::parse_object(current, LABEL)?;
    let mut value = before.clone();
    json_file::set(&mut value, BASE_URL, base_url.into());
    json_file::set(&mut value, AUTH_TOKEN, api_key.into());
    match template_str(template, MODEL) {
        Some(model) => json_file::set(&mut value, MODEL, model.into()),
        None => {
            json_file::remove(&mut value, MODEL);
        }
    }
    json_file::remove(&mut value, API_KEY);
    json_file::render_changed(current.unwrap_or_default(), KeyStyle::Json, &before, &value)
}

/// 绑定和切换模型共用。
pub(crate) fn write_bound(
    template: &Value,
    api_key: &str,
    txn: &mut FileTxn,
) -> Result<(), String> {
    let path = settings_path();
    let current = read_text(&path)?;
    let bound = bound_settings(current.as_deref(), template, api_key)?;
    txn.write(&path, bound.as_bytes())
        .map_err(|e| format!("写入 Claude 配置失败：{e}"))
}

pub(crate) fn plan_restore(
    original: Option<&str>,
    current: Option<&str>,
) -> Result<RestorePlan, String> {
    json_file::plan_restore(original, current, LABEL, KeyStyle::Json, LEAVES, CONTAINERS)
}

/// 没有绑定前快照时的尽力清理：去掉 Ofox 的地址、key 和 Ofox 的模型名，
/// 回到 Claude Code 自己的登录方式。
pub(crate) fn legacy_edits() -> Result<Vec<FileEdit>, String> {
    let path = settings_path();
    let Some(text) = read_text(&path)? else {
        return Ok(Vec::new());
    };
    let before = json_file::parse_object(Some(&text), LABEL)?;
    let mut value = before.clone();
    let points_to_ofox = is_ofox_bound(&text);
    let ofox_token = json_file::get(&value, AUTH_TOKEN)
        .and_then(Value::as_str)
        .is_some_and(|token| token.starts_with("sk-of-"));
    let mut doomed = Vec::new();
    if points_to_ofox {
        doomed.extend([BASE_URL, MODEL]);
    }
    if points_to_ofox || ofox_token {
        doomed.push(AUTH_TOKEN);
    }
    let mut removed_keys = Vec::new();
    for path in doomed {
        if json_file::remove(&mut value, path) {
            removed_keys.push(path.join("."));
        }
    }
    if removed_keys.is_empty() {
        return Ok(Vec::new());
    }
    if json_file::get(&value, &["env"])
        .and_then(Value::as_object)
        .is_some_and(serde_json::Map::is_empty)
    {
        json_file::remove(&mut value, &["env"]);
    }
    Ok(vec![FileEdit {
        path,
        content: Some(json_file::render_changed(
            &text,
            KeyStyle::Json,
            &before,
            &value,
        )?),
        restored_keys: Vec::new(),
        removed_keys,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const RELAY: &str = "{\n  \"env\": {\n    \"ANTHROPIC_BASE_URL\": \"https://relay.example\",\n    \"ANTHROPIC_API_KEY\": \"sk-relay\",\n    \"ANTHROPIC_MODEL\": \"claude-opus-x\",\n    \"DISABLE_TELEMETRY\": \"1\"\n  },\n  \"statusLine\": {\n    \"type\": \"command\"\n  }\n}\n";

    fn template(model: Option<&str>) -> Value {
        let mut env = json!({
            "ANTHROPIC_BASE_URL": "https://api.ofox.ai/anthropic",
            "ANTHROPIC_AUTH_TOKEN": "",
        });
        if let Some(model) = model {
            env["ANTHROPIC_MODEL"] = json!(model);
        }
        json!({ "env": env })
    }

    fn parsed(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn bind_points_to_ofox_and_drops_the_relay_key() {
        let bound = bound_settings(Some(RELAY), &template(Some("anthropic/x")), "sk-of-K").unwrap();
        let value = parsed(&bound);
        assert_eq!(
            value["env"]["ANTHROPIC_BASE_URL"],
            "https://api.ofox.ai/anthropic"
        );
        assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-of-K");
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "anthropic/x");
        assert!(value["env"].get("ANTHROPIC_API_KEY").is_none());
        assert_eq!(value["env"]["DISABLE_TELEMETRY"], "1");
        assert_eq!(value["statusLine"]["type"], "command");
        assert!(is_ofox_bound(&bound));
        assert!(!is_ofox_bound(RELAY));
    }

    #[test]
    fn bind_without_a_model_removes_the_old_model() {
        let bound = bound_settings(Some(RELAY), &template(None), "sk-of-K").unwrap();
        assert!(parsed(&bound)["env"].get("ANTHROPIC_MODEL").is_none());
    }

    #[test]
    fn restore_puts_the_relay_setup_back_byte_for_byte() {
        let bound = bound_settings(Some(RELAY), &template(Some("anthropic/x")), "sk-of-K").unwrap();
        let plan = plan_restore(Some(RELAY), Some(&bound)).unwrap();
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(RELAY));
    }

    #[test]
    fn bind_refuses_settings_it_cannot_parse() {
        assert!(bound_settings(Some("{ oops"), &template(None), "sk-of-K").is_err());
        assert!(validate("{ oops").is_err());
    }
}
