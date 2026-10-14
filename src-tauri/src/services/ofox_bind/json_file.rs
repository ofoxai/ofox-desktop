//! JSON 系配置文件（Claude / Gemini 的 settings.json、OpenCode 的 JSONC、OpenClaw 的
//! JSON5）的字段级绑定与还原。写回时只重写有变化的顶层段落，其余段落的注释和格式
//! 原样保留（见 [`crate::json5_sections`]）。

use std::collections::BTreeSet;

use serde_json::{Map, Value};

pub(crate) use crate::json5_sections::KeyStyle;

use super::ownership::{self, Leaf, Leaves};
use super::plan::RestorePlan;

/// 从根对象出发的字段路径，例如 `["env", "ANTHROPIC_BASE_URL"]`。
pub(crate) type JsonPath = &'static [&'static str];

/// 解析成 JSON 对象；空文件视为 `{}`，带注释的（Gemini CLI 允许）也能读。不是
/// 对象就报错——不能在一份解析不了的文件上写配置，也不能把它当成原样保存。
pub(crate) fn parse_object(text: Option<&str>, label: &str) -> Result<Value, String> {
    let text = text.unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let value: Value = serde_json::from_str(text)
        .or_else(|error| json5::from_str(text).map_err(|_| error))
        .map_err(|e| {
            format!(
                "{label} 不是有效的 JSON，请先修复后再操作：{}",
                crate::redact::describe_json_error(&e)
            )
        })?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(format!(
            "{label} 的根节点必须是 JSON 对象，请先修复后再操作"
        ))
    }
}

/// 能解析成 JSON 对象，并且保留格式的写回也能处理。绑定前用它把关：解析不了的
/// 文件拒绝绑定。
pub(crate) fn validate(text: &str, label: &str) -> Result<(), String> {
    parse_object(Some(text), label)?;
    crate::json5_sections::parse(text).map(drop).map_err(|e| {
        format!(
            "{label} 解析失败，请先修复后再操作：{}",
            crate::redact::scrub(&e.to_string())
        )
    })
}

pub(crate) fn get(value: &Value, path: JsonPath) -> Option<&Value> {
    path.iter().try_fold(value, |node, key| node.get(key))
}

pub(crate) fn set(value: &mut Value, path: JsonPath, new_value: Value) {
    let (last, parents) = path.split_last().expect("non-empty path");
    let mut node = value;
    for key in parents {
        if !node.get(key).is_some_and(Value::is_object) {
            node[*key] = Value::Object(Map::new());
        }
        node = node.get_mut(key).expect("just ensured");
    }
    node[*last] = new_value;
}

/// 把模板生成的服务商条目合并进配置里已有的同名条目：接入字段取模板的值，用户
/// 在 agent 里加的其它字段、模型原样保留。`models` 按 id（对象写法按 key）合并，
/// 同一个模型以模板为准。已有条目不是对象就整条用模板的。
pub(crate) fn merge_provider_entry(existing: Option<&Value>, fresh: Value) -> Value {
    let (Some(Value::Object(existing)), Value::Object(fresh)) = (existing, &fresh) else {
        return fresh;
    };
    let mut merged = existing.clone();
    for (key, value) in fresh {
        let value = match (key.as_str(), merged.get(key), value) {
            ("models", Some(Value::Object(old)), Value::Object(new)) => {
                let mut models = old.clone();
                models.extend(new.iter().map(|(id, model)| (id.clone(), model.clone())));
                Value::Object(models)
            }
            ("models", Some(Value::Array(old)), Value::Array(new)) => {
                let id =
                    |model: &Value| model.get("id").and_then(Value::as_str).map(str::to_string);
                let mut models = old.clone();
                for model in new {
                    match models
                        .iter()
                        .position(|old| id(old).is_some() && id(old) == id(model))
                    {
                        Some(at) => models[at] = model.clone(),
                        None => models.push(model.clone()),
                    }
                }
                Value::Array(models)
            }
            _ => value.clone(),
        };
        merged.insert(key.clone(), value);
    }
    Value::Object(merged)
}

pub(crate) fn remove(value: &mut Value, path: JsonPath) -> bool {
    let (last, parents) = path.split_last().expect("non-empty path");
    let mut node = value;
    for key in parents {
        match node.get_mut(key) {
            Some(child) => node = child,
            None => return false,
        }
    }
    node.as_object_mut()
        .is_some_and(|object| object.remove(*last).is_some())
}

/// 文件里各受管字段现在的值。
pub(crate) fn leaves(text: Option<&str>, label: &str, leaves: &[Leaf]) -> Result<Leaves, String> {
    Ok(ownership::leaves_at(&parse_object(text, label)?, leaves))
}

/// 按绑定前的值还原后的结果，以及还原 / 删除了哪些字段。
pub(crate) struct Restored {
    pub value: Value,
    pub restored_keys: Vec<String>,
    pub removed_keys: Vec<String>,
}

/// 受管字段还原成绑定前的值（原来没有就删掉），`keep` 里的（用户改过的）除外；
/// `containers` 里的对象只在绑定前不存在、现在又空了时删除（按给定顺序，先深后浅）。
pub(crate) fn restore_value(
    original: &Value,
    current: &Value,
    leaves: &[Leaf],
    containers: &[JsonPath],
    keep: &BTreeSet<String>,
) -> Restored {
    let mut value = current.clone();
    let mut restored_keys = Vec::new();
    let mut removed_keys = Vec::new();
    for leaf in leaves.iter().filter(|leaf| !keep.contains(&leaf.id())) {
        match get(original, leaf.path) {
            Some(item) => {
                set(&mut value, leaf.path, item.clone());
                restored_keys.push(leaf.id());
            }
            None => {
                if remove(&mut value, leaf.path) {
                    removed_keys.push(leaf.id());
                }
            }
        }
    }
    for container in containers {
        let now_empty = get(&value, container)
            .and_then(Value::as_object)
            .is_some_and(Map::is_empty);
        if get(original, container).is_none() && now_empty {
            remove(&mut value, container);
        }
    }
    Restored {
        value,
        restored_keys,
        removed_keys,
    }
}

/// 值有变化的顶层键：先按 `after` 的顺序，再是 `after` 里没有了的。
fn changed_sections<'a>(before: &'a Value, after: &'a Value) -> Vec<&'a str> {
    let keys = |value: &'a Value| {
        value
            .as_object()
            .into_iter()
            .flat_map(|object| object.keys().map(String::as_str))
    };
    let mut changed: Vec<&str> = Vec::new();
    for key in keys(after).chain(keys(before)) {
        if before.get(key) != after.get(key) && !changed.contains(&key) {
            changed.push(key);
        }
    }
    changed
}

/// 把 `after` 相对 `before` 有变化的顶层段落写回 `source`，其余段落原样保留。
pub(crate) fn render_changed(
    source: &str,
    style: KeyStyle,
    before: &Value,
    after: &Value,
) -> Result<String, String> {
    let edits: Vec<(&str, Option<&Value>)> = changed_sections(before, after)
        .into_iter()
        .map(|key| (key, after.get(key)))
        .collect();
    if edits.is_empty() {
        return Ok(source.to_string());
    }
    crate::json5_sections::edit_root_sections(source, style, &edits).map_err(|e| e.to_string())
}

/// [`restore_value`] 之后写回：语义上等于绑定前时写回原文本；否则只重写有变化的
/// 顶层段落，和绑定前一样的段落从原文本整段搬回（含注释和键的顺序）。
pub(crate) fn plan_restore(
    original: Option<&str>,
    current: Option<&str>,
    label: &str,
    style: KeyStyle,
    leaves: &[Leaf],
    containers: &[JsonPath],
    keep: &BTreeSet<String>,
) -> Result<RestorePlan, String> {
    let original_value = parse_object(original, label)?;
    let current_value = parse_object(current, label)?;
    let restored = restore_value(&original_value, &current_value, leaves, containers, keep);
    if restored.value == original_value {
        return Ok(RestorePlan {
            content: original.map(str::to_string),
            exact: true,
            restored_keys: restored.restored_keys,
            removed_keys: restored.removed_keys,
            kept_keys: Vec::new(),
        });
    }
    let mut text = current.unwrap_or_default().to_string();
    for key in changed_sections(&current_value, &restored.value) {
        let target = restored.value.get(key);
        text = match (target, original) {
            (Some(value), Some(original_text)) if original_value.get(key) == Some(value) => {
                crate::json5_sections::copy_root_section(&text, original_text, key, style)
            }
            _ => crate::json5_sections::edit_root_sections(&text, style, &[(key, target)]),
        }
        .map_err(|e| e.to_string())?;
    }
    Ok(RestorePlan {
        content: Some(text),
        exact: false,
        restored_keys: restored.restored_keys,
        removed_keys: restored.removed_keys,
        kept_keys: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEAVES: &[Leaf] = &[
        Leaf::route(&["env", "BASE"], ownership::ofox_url),
        Leaf::route(&["env", "TOKEN"], ownership::ofox_key),
    ];
    const CONTAINERS: &[JsonPath] = &[&["env"]];

    #[test]
    fn unparseable_json_error_keeps_the_position_and_never_echoes_the_key_in_it() {
        let broken = r#"{"env": {"TOKEN": "sk-of-abc123456789" oops}}"#;

        let error = parse_object(Some(broken), "settings.json").unwrap_err();

        assert!(error.contains("settings.json"), "{error}");
        assert!(error.contains("line 1, column"), "{error}");
        assert!(!error.contains("abc123456789"), "{error}");
    }

    #[test]
    fn restores_original_bytes_when_only_managed_fields_changed() {
        let original = "{\n    \"env\": { \"BASE\": \"https://relay\" },\n    \"statusLine\": 1\n}";
        let current = r#"{"env":{"BASE":"https://api.ofox.ai","TOKEN":"sk-of-x"},"statusLine":1}"#;
        let plan = plan_restore(
            Some(original),
            Some(current),
            "f",
            KeyStyle::Json,
            LEAVES,
            CONTAINERS,
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(original));
    }

    #[test]
    fn keeps_unmanaged_changes_and_prunes_created_containers() {
        let current =
            r#"{"env":{"BASE":"https://api.ofox.ai","TOKEN":"sk-of-x"},"mcpServers":{"a":{}}}"#;
        let plan = plan_restore(
            Some("{}"),
            Some(current),
            "f",
            KeyStyle::Json,
            LEAVES,
            CONTAINERS,
            &BTreeSet::new(),
        )
        .unwrap();
        let value: Value = serde_json::from_str(plan.content.as_deref().unwrap()).unwrap();
        assert!(value.get("env").is_none());
        assert!(value["mcpServers"]["a"].is_object());
        assert!(!plan.exact);
    }

    #[test]
    fn deletes_a_file_that_did_not_exist_before() {
        let plan = plan_restore(
            None,
            Some(r#"{"env":{"TOKEN":"sk-of-x"}}"#),
            "f",
            KeyStyle::Json,
            LEAVES,
            CONTAINERS,
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(plan.content, None);
    }

    #[test]
    fn non_exact_restore_keeps_comments_and_copies_untouched_sections_back() {
        let original = "{\n  // relay\n  \"env\": { \"BASE\": \"https://relay\", \"X\": 1 },\n  \"hooks\": {}\n}\n";
        let current = "{\n  // relay\n  \"env\": {\"X\": 1, \"BASE\": \"https://api.ofox.ai\", \"TOKEN\": \"sk-of-x\"},\n  \"hooks\": {},\n  \"mcp\": { \"a\": 1 }\n}\n";
        let plan = plan_restore(
            Some(original),
            Some(current),
            "f",
            KeyStyle::Json,
            LEAVES,
            CONTAINERS,
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(!plan.exact);
        assert_eq!(
            plan.content.as_deref(),
            Some("{\n  // relay\n  \"env\": { \"BASE\": \"https://relay\", \"X\": 1 },\n  \"hooks\": {},\n  \"mcp\": { \"a\": 1 }\n}\n")
        );
    }

    #[test]
    fn render_changed_rewrites_only_changed_sections() {
        let source = "{\n  // keep\n  \"a\":   1,\n  \"b\": 2\n}";
        let before: Value = serde_json::from_str("{\"a\":1,\"b\":2}").unwrap();
        let mut after = before.clone();
        after["b"] = Value::from(3);
        assert_eq!(
            render_changed(source, KeyStyle::Json, &before, &after).unwrap(),
            "{\n  // keep\n  \"a\":   1,\n  \"b\": 3\n}"
        );
    }

    #[test]
    fn invalid_or_non_object_json_is_rejected() {
        assert!(parse_object(Some("{oops"), "f").is_err());
        assert!(parse_object(Some("[1]"), "f").is_err());
        assert!(parse_object(Some("  "), "f").unwrap().is_object());
        let commented = parse_object(Some("{\n  // pick auth\n  \"a\": 1,\n}"), "f").unwrap();
        assert_eq!(commented["a"], 1);
    }
}
