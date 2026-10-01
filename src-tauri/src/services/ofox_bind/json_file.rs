//! JSON 配置文件（Claude 的 settings.json、Gemini 的 settings.json）的字段级还原。

use serde_json::{Map, Value};

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
        .map_err(|e| format!("{label} 不是有效的 JSON，请先修复后再操作：{e}"))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(format!(
            "{label} 的根节点必须是 JSON 对象，请先修复后再操作"
        ))
    }
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

/// 按绑定前的值还原后的结果，以及还原 / 删除了哪些字段。
pub(crate) struct Restored {
    pub value: Value,
    pub restored_keys: Vec<String>,
    pub removed_keys: Vec<String>,
}

/// 受管字段一律还原成绑定前的值（原来没有就删掉）；`containers` 里的对象只在
/// 绑定前不存在、现在又空了时删除（按给定顺序，先深后浅）。
pub(crate) fn restore_value(
    original: &Value,
    current: &Value,
    leaves: &[JsonPath],
    containers: &[JsonPath],
) -> Restored {
    let mut value = current.clone();
    let mut restored_keys = Vec::new();
    let mut removed_keys = Vec::new();
    for leaf in leaves {
        match get(original, leaf) {
            Some(item) => {
                set(&mut value, leaf, item.clone());
                restored_keys.push(leaf.join("."));
            }
            None => {
                if remove(&mut value, leaf) {
                    removed_keys.push(leaf.join("."));
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

/// [`restore_value`] 之后按 JSON 写回；语义上等于绑定前时写回原文本。
pub(crate) fn plan_restore(
    original: Option<&str>,
    current: Option<&str>,
    label: &str,
    leaves: &[JsonPath],
    containers: &[JsonPath],
) -> Result<RestorePlan, String> {
    let original_value = parse_object(original, label)?;
    let restored = restore_value(
        &original_value,
        &parse_object(current, label)?,
        leaves,
        containers,
    );
    let same = restored.value == original_value;
    let text = serde_json::to_string_pretty(&restored.value).map_err(|e| e.to_string())?;
    Ok(RestorePlan::finish(
        text,
        original,
        same,
        restored.restored_keys,
        restored.removed_keys,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEAVES: &[JsonPath] = &[&["env", "BASE"], &["env", "TOKEN"]];
    const CONTAINERS: &[JsonPath] = &[&["env"]];

    #[test]
    fn restores_original_bytes_when_only_managed_fields_changed() {
        let original = "{\n    \"env\": { \"BASE\": \"https://relay\" },\n    \"statusLine\": 1\n}";
        let current = r#"{"env":{"BASE":"https://api.ofox.ai","TOKEN":"sk-of-x"},"statusLine":1}"#;
        let plan = plan_restore(Some(original), Some(current), "f", LEAVES, CONTAINERS).unwrap();
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(original));
    }

    #[test]
    fn keeps_unmanaged_changes_and_prunes_created_containers() {
        let current =
            r#"{"env":{"BASE":"https://api.ofox.ai","TOKEN":"sk-of-x"},"mcpServers":{"a":{}}}"#;
        let plan = plan_restore(Some("{}"), Some(current), "f", LEAVES, CONTAINERS).unwrap();
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
            LEAVES,
            CONTAINERS,
        )
        .unwrap();
        assert_eq!(plan.content, None);
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
