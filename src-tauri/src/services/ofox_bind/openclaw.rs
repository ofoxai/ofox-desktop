//! OpenClaw 的绑定配置：`openclaw.json`（JSON5）里的 `models.providers."ofox-openclaw"`
//! 和默认模型 `agents.defaults.model`。其它 provider、skills、auth-profile 一律不动。
//! 写回时只重写有变化的顶层段落，其余段落的注释和格式原样保留。

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::config::FileTxn;
use crate::ofox_apex::mentions_ofox_gateway;
use crate::openclaw_config::{self, get_openclaw_config_path, OpenClawDefaultModel};

use super::json_file::{self, JsonPath, KeyStyle};
use super::ownership::{self, Leaf, Leaves};
use super::plan::{read_text, FileEdit, RestorePlan};

const LABEL: &str = "OpenClaw 的 openclaw.json";
const PROVIDER_ID: &str = "ofox-openclaw";
const ENTRY: JsonPath = &["models", "providers", "ofox-openclaw"];
/// 原来没有 `models` 段落时，绑定会按 OpenClaw 的默认值建出 `mode: "merge"`。
const MODE: JsonPath = &["models", "mode"];
const DEFAULT_MODEL: JsonPath = &["agents", "defaults", "model"];
const TOKEN: JsonPath = &["apiKey"];
const BASE_URL: JsonPath = &["baseUrl"];
/// 解绑时还原成绑定前的值（原来没有就删掉）；用户之后改过的按规则留着。默认模型
/// （`ofox-openclaw/<模型>`）自己就说明了走哪个服务商：还指向 Ofox 条目就还原（在
/// agent 里换成别的 Ofox 模型也一样），指向用户自己的服务商就留着。
pub(crate) const LEAVES: &[Leaf] = &[
    Leaf::route(ENTRY, ownership::ofox_entry),
    Leaf::preference(MODE),
    Leaf::route(DEFAULT_MODEL, routes_to_ofox),
];
const CONTAINERS: &[JsonPath] = &[
    &["models", "providers"],
    &["models"],
    &["agents", "defaults"],
    &["agents"],
];

pub(crate) fn config_path() -> PathBuf {
    get_openclaw_config_path()
}

fn parse(text: &str) -> Result<Value, String> {
    json_file::parse_object(Some(text), LABEL)
}

pub(crate) fn validate(text: &str) -> Result<(), String> {
    json_file::validate(text, LABEL)
}

/// `models.providers."ofox-openclaw"` 指向 Ofox 网关——当前（或旧版本）绑定留下的配置。
pub(crate) fn is_ofox_bound(text: &str) -> bool {
    parse(text)
        .ok()
        .and_then(|value| {
            let entry = json_file::get(&value, ENTRY)?;
            json_file::get(entry, BASE_URL)?
                .as_str()
                .map(mentions_ofox_gateway)
        })
        .unwrap_or(false)
}

fn render_changed(source: &str, before: &Value, after: &Value) -> Result<String, String> {
    json_file::render_changed(source, KeyStyle::Json5, before, after)
}

fn first_model_id(entry: &Value) -> Option<&str> {
    entry
        .get("models")?
        .as_array()?
        .first()?
        .get("id")?
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// 由 DB 里 `ofox-openclaw` 模板生成绑定后的配置：写入 Ofox 条目，默认模型指向它。
/// `select_route`：绑定、保存模型时要让默认模型指向 Ofox；其余（换 key、修复）
/// 默认模型已经指向 Ofox 就不动，用户在 agent 里选的模型要留着。
pub(crate) fn bound_config(
    current: Option<&str>,
    template: &Value,
    api_key: &str,
    select_route: bool,
) -> Result<String, String> {
    // 文件不存在时从 OpenClaw 自己的默认配置开始。
    let source = current.unwrap_or(openclaw_config::OPENCLAW_DEFAULT_SOURCE);
    let before = parse(source)?;
    let mut after = before.clone();
    if json_file::get(&after, &["models"]).is_none() {
        json_file::set(
            &mut after,
            &["models"],
            json!({ "mode": "merge", "providers": {} }),
        );
    }
    let mut entry = template.clone();
    json_file::set(&mut entry, TOKEN, api_key.into());
    let routed_to_ofox = json_file::get(&after, DEFAULT_MODEL)
        .and_then(|model| model.as_str().or_else(|| model.get("primary")?.as_str()))
        .is_some_and(|primary| primary.starts_with(&format!("{PROVIDER_ID}/")));
    if let Some(model_id) = first_model_id(&entry).filter(|_| select_route || !routed_to_ofox) {
        let mut default = match json_file::get(&after, DEFAULT_MODEL) {
            Some(value) => serde_json::from_value::<OpenClawDefaultModel>(value.clone())
                .map_err(|e| format!("解析 OpenClaw 默认模型失败：{e}"))?,
            None => OpenClawDefaultModel {
                primary: String::new(),
                fallbacks: Vec::new(),
                extra: HashMap::new(),
            },
        };
        default.primary = format!("{PROVIDER_ID}/{model_id}");
        json_file::set(
            &mut after,
            DEFAULT_MODEL,
            serde_json::to_value(default).map_err(|e| e.to_string())?,
        );
    }
    let entry = json_file::merge_provider_entry(json_file::get(&before, ENTRY), entry);
    json_file::set(&mut after, ENTRY, entry);
    render_changed(source, &before, &after)
}

/// 绑定和切换模型共用。
pub(crate) fn write_bound(
    template: &Value,
    api_key: &str,
    select_route: bool,
    txn: &mut FileTxn,
) -> Result<(), String> {
    let path = config_path();
    let bound = bound_config(
        read_text(&path)?.as_deref(),
        template,
        api_key,
        select_route,
    )?;
    txn.write(&path, bound.as_bytes())
        .map_err(|e| format!("写入 OpenClaw 配置失败：{e}"))
}

/// 默认模型（字符串或 `{primary, fallbacks}`）指向 Ofox 条目。
fn routes_to_ofox(model: &Value) -> bool {
    model
        .as_str()
        .or_else(|| model.get("primary")?.as_str())
        .is_some_and(|primary| primary.starts_with(&format!("{PROVIDER_ID}/")))
}

pub(crate) fn leaves(text: Option<&str>) -> Result<Leaves, String> {
    json_file::leaves(text, LABEL, LEAVES)
}

pub(crate) fn plan_restore(
    original: Option<&str>,
    current: Option<&str>,
    keep: &BTreeSet<String>,
) -> Result<RestorePlan, String> {
    json_file::plan_restore(
        original,
        current,
        LABEL,
        KeyStyle::Json5,
        LEAVES,
        CONTAINERS,
        keep,
    )
}

/// 旧版本（`__ofoxDirectBackupVersion` 外层）记下的绑定前默认模型。
fn legacy_runtime_default(legacy: Option<&Value>) -> Option<&Value> {
    legacy
        .filter(|record| record.get("__ofoxDirectBackupVersion").is_some())
        .and_then(|record| record.get("runtimeDefault"))
}

/// 没有绑定前快照时的尽力清理：删掉 Ofox 条目；旧版本记过绑定前的默认模型就
/// 还原它，否则只去掉指向 Ofox 的默认模型。
pub(crate) fn legacy_edits(legacy: Option<&Value>) -> Result<Vec<FileEdit>, String> {
    let path = config_path();
    let Some(text) = read_text(&path)? else {
        return Ok(Vec::new());
    };
    let before = parse(&text)?;
    let mut after = before.clone();
    let mut restored_keys = Vec::new();
    let mut removed_keys = Vec::new();
    if json_file::remove(&mut after, ENTRY) {
        removed_keys.push(ENTRY.join("."));
    }
    let points_to_ofox = json_file::get(&after, DEFAULT_MODEL)
        .and_then(|model| model.get("primary"))
        .and_then(Value::as_str)
        .is_some_and(|primary| primary.starts_with(&format!("{PROVIDER_ID}/")));
    match legacy_runtime_default(legacy) {
        Some(Value::Null) | None if points_to_ofox => {
            json_file::remove(&mut after, DEFAULT_MODEL);
            removed_keys.push(DEFAULT_MODEL.join("."));
        }
        Some(model) if !model.is_null() && json_file::get(&after, DEFAULT_MODEL) != Some(model) => {
            json_file::set(&mut after, DEFAULT_MODEL, model.clone());
            restored_keys.push(DEFAULT_MODEL.join("."));
        }
        _ => {}
    }
    if after == before {
        return Ok(Vec::new());
    }
    Ok(vec![FileEdit {
        content: Some(render_changed(&text, &before, &after)?),
        path,
        restored_keys,
        removed_keys,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_CONFIG: &str = "{\n  // routing\n  models: {\n    mode: 'merge',\n    providers: {\n      deepseek: { baseUrl: 'https://api.deepseek.com', apiKey: 'sk-ds' },\n    },\n  },\n  agents: {\n    defaults: {\n      // my default\n      model: { primary: 'deepseek/chat', fallbacks: ['deepseek/reasoner'] },\n      timeoutSeconds: 120,\n    },\n  },\n}\n";

    fn template(model: Option<&str>) -> Value {
        let models = model.map_or(json!([]), |id| json!([{ "id": id, "name": id }]));
        json!({
            "baseUrl": "https://api.ofox.ai/v1",
            "apiKey": "",
            "api": "openai-completions",
            "models": models,
        })
    }

    #[test]
    fn bind_adds_the_entry_and_points_the_default_model_at_it() {
        let bound = bound_config(
            Some(USER_CONFIG),
            &template(Some("openai/gpt-x")),
            "sk-of-K",
            true,
        )
        .unwrap();
        let value = parse(&bound).unwrap();
        assert_eq!(
            value["models"]["providers"]["ofox-openclaw"]["apiKey"],
            "sk-of-K"
        );
        assert_eq!(value["models"]["providers"]["deepseek"]["apiKey"], "sk-ds");
        assert_eq!(
            value["agents"]["defaults"]["model"],
            json!({ "primary": "ofox-openclaw/openai/gpt-x", "fallbacks": ["deepseek/reasoner"] })
        );
        assert_eq!(value["agents"]["defaults"]["timeoutSeconds"], 120);
        assert!(is_ofox_bound(&bound));
        assert!(!is_ofox_bound(USER_CONFIG));
    }

    #[test]
    fn restore_brings_back_the_commented_file() {
        let bound = bound_config(
            Some(USER_CONFIG),
            &template(Some("openai/gpt-x")),
            "sk-of-K",
            true,
        )
        .unwrap();
        let plan = plan_restore(Some(USER_CONFIG), Some(&bound), &BTreeSet::new()).unwrap();
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(USER_CONFIG));
    }

    #[test]
    fn restore_copies_untouched_sections_back_with_their_comments() {
        let bound = bound_config(
            Some(USER_CONFIG),
            &template(Some("openai/gpt-x")),
            "sk-of-K",
            true,
        )
        .unwrap();
        // OpenClaw 自己在绑定期间写了别的段落。
        let touched = crate::json5_sections::edit_root_sections(
            &bound,
            KeyStyle::Json5,
            &[("meta", Some(&json!({ "lastTouchedAt": "2026-10-01" })))],
        )
        .unwrap();
        let plan = plan_restore(Some(USER_CONFIG), Some(&touched), &BTreeSet::new()).unwrap();
        assert!(!plan.exact);
        let text = plan.content.unwrap();
        assert!(
            text.contains("// my default"),
            "agents section comes back verbatim: {text}"
        );
        assert!(text.contains("// routing"));
        let value = parse(&text).unwrap();
        assert_eq!(value["meta"]["lastTouchedAt"], "2026-10-01");
        assert!(value["models"]["providers"].get("ofox-openclaw").is_none());
    }

    #[test]
    fn restore_deletes_a_file_bind_created() {
        let bound = bound_config(None, &template(Some("openai/gpt-x")), "sk-of-K", true).unwrap();
        assert_eq!(
            plan_restore(None, Some(&bound), &BTreeSet::new())
                .unwrap()
                .content,
            None
        );
    }
}
