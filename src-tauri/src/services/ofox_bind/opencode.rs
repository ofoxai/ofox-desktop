//! OpenCode 的绑定配置：只动 `opencode.json` 里的 `provider."ofox-opencode"`，
//! 其它 provider、mcp、agents、plugin 一律不动。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::config::FileTxn;
use crate::ofox_apex::mentions_ofox_gateway;
use crate::opencode_config::{get_opencode_config_path, normalize_ofox_provider_transport};

use super::json_file::{self, JsonPath};
use super::plan::{read_text, FileEdit, RestorePlan};

const LABEL: &str = "OpenCode 的 opencode.json";
const ENTRY: JsonPath = &["provider", "ofox-opencode"];
const TOKEN: JsonPath = &["options", "apiKey"];
const BASE_URL: JsonPath = &["options", "baseURL"];
/// 文件原本不存在时绑定会顺带写上 `$schema`（与 OpenCode 自己的默认配置一致）。
const SCHEMA: JsonPath = &["$schema"];
const SCHEMA_URL: &str = "https://opencode.ai/config.json";
/// 解绑时一律还原成绑定前的值（原来没有就删掉）：Ofox 的条目整条还原。
const LEAVES: &[JsonPath] = &[ENTRY, SCHEMA];
const CONTAINERS: &[JsonPath] = &[&["provider"]];

pub(crate) fn config_path() -> PathBuf {
    get_opencode_config_path()
}

pub(crate) fn validate(text: &str) -> Result<(), String> {
    json_file::parse_object(Some(text), LABEL).map(drop)
}

/// `provider."ofox-opencode"` 指向 Ofox 网关——当前（或旧版本）绑定留下的配置。
pub(crate) fn is_ofox_bound(text: &str) -> bool {
    json_file::parse_object(Some(text), LABEL)
        .ok()
        .and_then(|value| {
            let entry = json_file::get(&value, ENTRY)?;
            json_file::get(entry, BASE_URL)?
                .as_str()
                .map(mentions_ofox_gateway)
        })
        .unwrap_or(false)
}

/// 由 DB 里 `ofox-opencode` 模板生成 Ofox 条目：注入 key，按模型修正传输方式。
fn bound_entry(template: &Value, api_key: &str) -> Value {
    let mut entry = template.clone();
    json_file::set(&mut entry, TOKEN, api_key.into());
    normalize_ofox_provider_transport(&mut entry);
    entry
}

pub(crate) fn bound_config(
    current: Option<&str>,
    template: &Value,
    api_key: &str,
) -> Result<String, String> {
    let mut value = match current {
        Some(text) => json_file::parse_object(Some(text), LABEL)?,
        None => json!({ "$schema": SCHEMA_URL }),
    };
    json_file::set(&mut value, ENTRY, bound_entry(template, api_key));
    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}

/// 绑定和切换模型共用。
pub(crate) fn write_bound(
    template: &Value,
    api_key: &str,
    txn: &mut FileTxn,
) -> Result<(), String> {
    let path = config_path();
    let bound = bound_config(read_text(&path)?.as_deref(), template, api_key)?;
    txn.write(&path, bound.as_bytes())
        .map_err(|e| format!("写入 OpenCode 配置失败：{e}"))
}

pub(crate) fn plan_restore(
    original: Option<&str>,
    current: Option<&str>,
) -> Result<RestorePlan, String> {
    json_file::plan_restore(original, current, LABEL, LEAVES, CONTAINERS)
}

/// 没有绑定前快照时的尽力清理：`ofox-opencode` 是 Ofox 专用的条目，整条删掉。
pub(crate) fn legacy_edits() -> Result<Vec<FileEdit>, String> {
    let path = config_path();
    let Some(text) = read_text(&path)? else {
        return Ok(Vec::new());
    };
    let mut value = json_file::parse_object(Some(&text), LABEL)?;
    if !json_file::remove(&mut value, ENTRY) {
        return Ok(Vec::new());
    }
    if json_file::get(&value, &["provider"])
        .and_then(Value::as_object)
        .is_some_and(serde_json::Map::is_empty)
    {
        json_file::remove(&mut value, &["provider"]);
    }
    Ok(vec![FileEdit {
        path,
        content: Some(serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?),
        restored_keys: Vec::new(),
        removed_keys: vec![ENTRY.join(".")],
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_CONFIG: &str = "{\n  // my providers\n  \"provider\": {\n    \"deepseek\": { \"options\": { \"apiKey\": \"sk-ds\" } }\n  },\n  \"mcp\": {}\n}\n";

    fn template() -> Value {
        json!({
            "npm": "@ai-sdk/openai",
            "name": "OfoxAI",
            "options": { "baseURL": "https://api.ofox.ai/v1", "apiKey": "" },
            "models": { "openai/gpt-x": { "name": "openai/gpt-x" } }
        })
    }

    #[test]
    fn bind_adds_only_the_ofox_entry() {
        let bound = bound_config(Some(USER_CONFIG), &template(), "sk-of-K").unwrap();
        let value: Value = serde_json::from_str(&bound).unwrap();
        assert_eq!(
            value["provider"]["ofox-opencode"]["options"]["apiKey"],
            "sk-of-K"
        );
        assert_eq!(value["provider"]["deepseek"]["options"]["apiKey"], "sk-ds");
        assert!(value.get("$schema").is_none());
        assert!(is_ofox_bound(&bound));
        assert!(!is_ofox_bound(USER_CONFIG));
    }

    #[test]
    fn restore_brings_back_the_commented_file() {
        let bound = bound_config(Some(USER_CONFIG), &template(), "sk-of-K").unwrap();
        let plan = plan_restore(Some(USER_CONFIG), Some(&bound)).unwrap();
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(USER_CONFIG));
    }

    #[test]
    fn restore_deletes_a_file_bind_created() {
        let bound = bound_config(None, &template(), "sk-of-K").unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&bound).unwrap()["$schema"],
            SCHEMA_URL
        );
        assert_eq!(plan_restore(None, Some(&bound)).unwrap().content, None);
    }
}
