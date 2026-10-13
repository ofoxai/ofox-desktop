//! Pi enabledModels and startup selection adapted from Magpie internal/agent/pi.go.
use super::{
    common::{self, FileRecord, Format, Record},
    model_api, model_json, url, NativeScopeModel,
};
use crate::services::model_fetch::FetchedModel;
use serde_json::{json, Value};
use std::path::Path;

const PROVIDER: &str = "ofox-pi";
const LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh", "max"];

fn strip_level(pattern: &str) -> &str {
    pattern
        .rsplit_once(':')
        .filter(|(_, level)| LEVELS.contains(level))
        .map(|(head, _)| head)
        .unwrap_or(pattern)
}
fn has_glob(value: &str) -> bool {
    value.contains(['*', '?', '['])
}

// Go path.Match semantics from Magpie: '*' never crosses a provider/model slash.
fn takes(pattern: &str, reference: &str) -> bool {
    let pattern = strip_level(pattern).to_lowercase();
    let reference = reference.to_lowercase();
    if pattern == reference {
        return true;
    }
    if !has_glob(&pattern) {
        return false;
    }
    let mut expression = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => expression.push_str("[^/]*"),
            '?' => expression.push_str("[^/]"),
            '[' => {
                expression.push('[');
                if chars.peek() == Some(&'!') {
                    chars.next();
                    expression.push('^');
                }
                let mut closed = false;
                for c in chars.by_ref() {
                    expression.push(c);
                    if c == ']' {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return false;
                }
            }
            _ => expression.push_str(&regex::escape(&ch.to_string())),
        }
    }
    expression.push('$');
    regex::Regex::new(&expression).is_ok_and(|r| {
        r.is_match(&reference)
            || reference
                .split_once('/')
                .is_some_and(|(_, id)| r.is_match(id))
    })
}
fn scope(settings: &Value) -> Option<&Vec<Value>> {
    settings
        .get("enabledModels")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty() && s.iter().all(Value::is_string))
}
fn with_scope(settings: &Value, model: &str) -> Option<Value> {
    let reference = format!("{PROVIDER}/{model}");
    let current = scope(settings)?;
    if current
        .iter()
        .any(|p| takes(p.as_str().unwrap(), &reference))
    {
        return None;
    }
    let mut next = current.clone();
    next.push(Value::String(reference));
    Some(Value::Array(next))
}

fn models(catalog: &[FetchedModel]) -> Vec<Value> {
    catalog
        .iter()
        .filter_map(|model| {
            let api = model_api(crate::app_config::BindableTool::Pi, model).ok()?;
            let mut value = model_json(model);
            value["api"] = api.into();
            value["baseUrl"] = url(api).into();
            Some(value)
        })
        .collect()
}

pub(super) fn plan(
    directory: &Path,
    key: &str,
    model: &FetchedModel,
    catalog: &[FetchedModel],
    previous: Option<&Record>,
) -> Result<Record, String> {
    let auth = common::parse(
        common::read(&directory.join("auth.json"))?.as_deref(),
        Format::Json,
    )?;
    if auth.get(PROVIDER).is_some() {
        return Err("Pi auth.json 中已有同名凭据，请先移除冲突".into());
    }
    let model_path = directory.join("models.json");
    let config = common::parse(common::read(&model_path)?.as_deref(), Format::Json)?;
    if config.get("providers").is_some_and(|v| !v.is_object()) {
        return Err("Pi providers 不是对象".into());
    }
    if previous.is_none() && config.pointer(&format!("/providers/{PROVIDER}")).is_some() {
        return Err("Pi 中已有同名 provider，未覆盖".into());
    }
    let mut record = previous.cloned().unwrap_or(Record {
        version: 1,
        directory: directory.into(),
        files: vec![],
    });
    let mut entries = models(catalog);
    if !entries.iter().any(|v| v["id"] == model.id) {
        entries.extend(models(std::slice::from_ref(model)));
    }
    let changes = vec![
        (
            common::path(&["providers", PROVIDER, "baseUrl"]),
            Some(url("openai-completions").into()),
            true,
        ),
        (
            common::path(&["providers", PROVIDER, "api"]),
            Some("openai-completions".into()),
            true,
        ),
        (
            common::path(&["providers", PROVIDER, "apiKey"]),
            Some(key.into()),
            true,
        ),
        (
            common::path(&["providers", PROVIDER, "models"]),
            Some(Value::Array(entries)),
            false,
        ),
        (
            common::path(&["providers", PROVIDER, "compat"]),
            Some(json!({"sendSessionAffinityHeaders":true})),
            true,
        ),
    ];
    update(&mut record, model_path, Format::Json, true, changes)?;
    settings_model(&mut record, &model.id)?;
    Ok(record)
}

type Changes = Vec<(Vec<String>, Option<Value>, bool)>;
pub(super) fn update(
    record: &mut Record,
    path: std::path::PathBuf,
    format: Format,
    secret: bool,
    changes: Changes,
) -> Result<(), String> {
    let at = record.files.iter().position(|f| f.path == path);
    let file = common::plan(path, format, secret, changes, at.map(|i| &record.files[i]))?;
    if let Some(at) = at {
        record.files[at] = file;
    } else {
        record.files.push(file);
    }
    Ok(())
}
fn settings_model(record: &mut Record, model: &str) -> Result<(), String> {
    let path = record.directory.join("settings.json");
    let current = common::parse(common::read(&path)?.as_deref(), Format::Json)?;
    let mut changes = vec![
        (
            common::path(&["defaultProvider"]),
            Some(PROVIDER.into()),
            false,
        ),
        (common::path(&["defaultModel"]), Some(model.into()), false),
    ];
    if let Some(scope) = with_scope(&current, model) {
        changes.push((common::path(&["enabledModels"]), Some(scope), false));
    }
    update(record, path, Format::Json, false, changes)
}
pub(super) fn set_model(
    record: &Record,
    key: &str,
    model: &FetchedModel,
) -> Result<Record, String> {
    let api = model_api(crate::app_config::BindableTool::Pi, model)?;
    let mut next = record.clone();
    let path = record.directory.join("models.json");
    let value = common::parse(common::read(&path)?.as_deref(), Format::Json)?;
    let mut entries = value
        .pointer(&format!("/providers/{PROVIDER}/models"))
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "Pi 模型目录已被修改".to_string())?;
    let mut entry = model_json(model);
    entry["api"] = api.into();
    entry["baseUrl"] = url(api).into();
    if let Some(at) = entries.iter().position(|v| v["id"] == model.id) {
        let map = entries[at]
            .as_object_mut()
            .ok_or_else(|| "Pi 模型配置已修改".to_string())?;
        for (key, value) in entry.as_object().unwrap() {
            map.insert(key.clone(), value.clone());
        }
    } else {
        entries.push(entry);
    }
    update(
        &mut next,
        path,
        Format::Json,
        true,
        vec![
            (
                common::path(&["providers", PROVIDER, "models"]),
                Some(entries.into()),
                false,
            ),
            (
                common::path(&["providers", PROVIDER, "apiKey"]),
                Some(key.into()),
                true,
            ),
        ],
    )?;
    settings_model(&mut next, &model.id)?;
    Ok(next)
}

pub(super) fn scopes(record: &Record) -> Result<Vec<NativeScopeModel>, String> {
    let auth = common::parse(
        common::read(&record.directory.join("auth.json"))?.as_deref(),
        Format::Json,
    )?;
    if auth.get(PROVIDER).is_some() {
        return Err("Pi 同名认证覆盖了受管凭据".into());
    }
    let settings = common::parse(
        common::read(&record.directory.join("settings.json"))?.as_deref(),
        Format::Json,
    )?;
    let provider = settings
        .get("defaultProvider")
        .and_then(Value::as_str)
        .unwrap_or("");
    let model = settings
        .get("defaultModel")
        .and_then(Value::as_str)
        .unwrap_or("");
    if provider != PROVIDER || model.is_empty() {
        return Err("Pi 当前 provider 已改变".into());
    }
    let mut reference = format!("{provider}/{model}");
    if let Some(scope) = scope(&settings) {
        if !scope.iter().any(|p| takes(p.as_str().unwrap(), &reference)) {
            let first = strip_level(scope[0].as_str().unwrap());
            if !has_glob(first) && first.contains('/') {
                reference = first.into();
            }
        }
    }
    let model = reference
        .strip_prefix(&format!("{PROVIDER}/"))
        .filter(|m| !m.is_empty())
        .ok_or_else(|| "Pi enabledModels 当前启动到其他 provider".to_string())?;
    Ok(vec![NativeScopeModel {
        scope: "settings".into(),
        model_id: model.into(),
    }])
}

pub(super) fn endpoint_changes(file: &FileRecord, changes: &mut Changes) -> Result<(), String> {
    let text = common::read(&file.path)?;
    endpoint_changes_from_text(file, text.as_deref(), changes)
}

pub(super) fn pending_endpoint_changes(
    file: &FileRecord,
    changes: &mut Changes,
) -> Result<(), String> {
    endpoint_changes_from_text(file, Some(&file.last_text), changes)
}

fn endpoint_changes_from_text(
    file: &FileRecord,
    text: Option<&str>,
    changes: &mut Changes,
) -> Result<(), String> {
    let path = common::path(&["providers", PROVIDER, "models"]);
    let Some(owned) = file.owned.iter().find(|o| o.path == path) else {
        return Ok(());
    };
    let current = common::parse(text, file.format)?;
    let Some(entries) = common::get(&current, &path).and_then(Value::as_array) else {
        return Err("Pi 模型目录缺失".into());
    };
    let mut next = entries.clone();
    let mut changed = false;
    for entry in &mut next {
        let id = entry["id"].as_str().unwrap_or("");
        let expected = owned
            .written
            .as_ref()
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find(|m| m["id"] == id));
        let Some(expected) = expected else {
            continue;
        };
        if entry.get("baseUrl") != expected.get("baseUrl")
            || entry.get("api") != expected.get("api")
        {
            return Err("Pi 的受管模型地址已修改".into());
        }
        let api = entry["api"].as_str().unwrap_or("openai-completions");
        let endpoint = url(api);
        if entry["baseUrl"] != endpoint {
            entry["baseUrl"] = endpoint.into();
            changed = true;
        }
    }
    if changed {
        changes.push((path, Some(next.into()), false));
    }
    Ok(())
}

/// Removing our exact additions must retain scopes added by the user after binding.
pub(super) fn unbind_record(record: &Record) -> Result<Record, String> {
    let mut next = record.clone();
    for file in &mut next.files {
        for owned in &mut file.owned {
            if owned.path != common::path(&["enabledModels"]) {
                continue;
            }
            let current = common::parse(common::read(&file.path)?.as_deref(), file.format)?;
            if common::get(&current, &owned.path) == owned.written.as_ref() {
                continue;
            }
            let Some(current) = current.get("enabledModels").and_then(Value::as_array) else {
                owned.original = current.get("enabledModels").cloned();
                continue;
            };
            let original = owned.original.as_ref().and_then(Value::as_array);
            let added = owned
                .written
                .as_ref()
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|p| !original.is_some_and(|a| a.contains(p)))
                .collect::<Vec<_>>();
            owned.original = Some(Value::Array(
                current
                    .iter()
                    .filter(|p| !added.contains(p))
                    .cloned()
                    .collect(),
            ));
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_globs_thinking_and_startup_match_magpie() {
        assert!(takes("OFOX-PI/*/*:high", "ofox-pi/openai/gpt"));
        assert!(!takes("ofox-pi/*", "ofox-pi/openai/gpt"));
        assert!(takes("openai/*:xhigh", "ofox-pi/openai/gpt"));
        assert!(!takes("openai/gpt", "ofox-pi/openai/gpt"));
        assert!(with_scope(&json!({"enabledModels":[]}), "x").is_none());
        assert_eq!(
            with_scope(&json!({"enabledModels":["anthropic/*:high"]}), "openai/gpt"),
            Some(json!(["anthropic/*:high", "ofox-pi/openai/gpt"]))
        );
    }
}
