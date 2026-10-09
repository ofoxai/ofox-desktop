//! 保留格式的 JSON / JSONC / JSON5 顶层段落编辑：只重写指定的顶层段落，其余段落
//! （含注释、空白、键的写法）原样保留。改工具配置文件时用它，尽量不动用户的格式。

use json_five::rt::parser::{
    from_str as rt_from_str, JSONKeyValuePair as RtJSONKeyValuePair,
    JSONObjectContext as RtJSONObjectContext, JSONText as RtJSONText, JSONValue as RtJSONValue,
    KeyValuePairContext as RtKeyValuePairContext,
};
use serde_json::Value;

use crate::error::AppError;

/// 新键的写法：JSON5 能用裸标识符；JSON / JSONC 必须加双引号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyStyle {
    Json5,
    Json,
}

/// 解析成保留格式的文档。空文本当成 `{}`。
pub(crate) fn parse(source: &str) -> Result<RtJSONText, AppError> {
    let source = if source.trim().is_empty() {
        "{}"
    } else {
        source
    };
    rt_from_str(source).map_err(|e| {
        AppError::Config(format!(
            "Failed to parse config as round-trip JSON5 document: {}",
            e.message
        ))
    })
}

fn root_object(
    text: &mut RtJSONText,
) -> Result<
    (
        &mut Vec<RtJSONKeyValuePair>,
        &mut Option<RtJSONObjectContext>,
    ),
    AppError,
> {
    match &mut text.value {
        RtJSONValue::JSONObject {
            key_value_pairs,
            context,
        } => Ok((key_value_pairs, context)),
        _ => Err(AppError::Config(
            "Config root must be a JSON object".to_string(),
        )),
    }
}

/// 设置顶层段落 `key`，其余段落（含注释和格式）原样保留。`make_value` 拿到子级缩进。
fn put_root_section(
    text: &mut RtJSONText,
    key: &str,
    style: KeyStyle,
    make_value: impl FnOnce(&str) -> Result<RtJSONValue, AppError>,
) -> Result<(), AppError> {
    let (key_value_pairs, context) = root_object(text)?;

    if key_value_pairs.is_empty()
        && context
            .as_ref()
            .map(|ctx| ctx.wsc.0.is_empty())
            .unwrap_or(true)
    {
        *context = Some(RtJSONObjectContext {
            wsc: ("\n  ".to_string(),),
        });
    }

    let leading_ws = context
        .as_ref()
        .map(|ctx| ctx.wsc.0.clone())
        .unwrap_or_default();
    let entry_separator_ws = derive_entry_separator(&leading_ws);
    let child_indent = extract_trailing_indent(&leading_ws);
    let new_value = make_value(&child_indent)?;

    if let Some(existing) = key_value_pairs
        .iter_mut()
        .find(|pair| json5_key_name(&pair.key) == Some(key))
    {
        existing.value = new_value;
        return Ok(());
    }

    let new_pair = if let Some(last_pair) = key_value_pairs.last_mut() {
        let last_ctx = ensure_kvp_context(last_pair);
        let closing_ws = if let Some(after_comma) = last_ctx.wsc.3.clone() {
            last_ctx.wsc.3 = Some(entry_separator_ws.clone());
            after_comma
        } else {
            let closing_ws = std::mem::take(&mut last_ctx.wsc.2);
            last_ctx.wsc.3 = Some(entry_separator_ws.clone());
            closing_ws
        };

        make_root_pair(key, new_value, closing_ws, style)
    } else {
        make_root_pair(
            key,
            new_value,
            derive_closing_ws_from_separator(&leading_ws),
            style,
        )
    };

    key_value_pairs.push(new_pair);
    Ok(())
}

/// 删除顶层段落 `key`：它前面的注释随它一起删掉，后面的注释和格式保持不变。
fn remove_root_section(text: &mut RtJSONText, key: &str) -> Result<bool, AppError> {
    let (key_value_pairs, context) = root_object(text)?;
    let Some(index) = key_value_pairs
        .iter()
        .position(|pair| json5_key_name(&pair.key) == Some(key))
    else {
        return Ok(false);
    };
    let removed = key_value_pairs.remove(index);
    let (after_value, after_comma) = removed
        .context
        .map(|ctx| (ctx.wsc.2, ctx.wsc.3))
        .unwrap_or_default();
    let is_last = index == key_value_pairs.len();

    if !is_last {
        // 原来引出被删段落的空白（含它的注释），换成引出下一段的空白。
        let lead_in = after_comma.unwrap_or_default();
        match index.checked_sub(1) {
            Some(previous) => {
                ensure_kvp_context(&mut key_value_pairs[previous]).wsc.3 = Some(lead_in)
            }
            None => *context = Some(RtJSONObjectContext { wsc: (lead_in,) }),
        }
        return Ok(true);
    }

    match index.checked_sub(1) {
        Some(previous) => {
            let ctx = ensure_kvp_context(&mut key_value_pairs[previous]);
            match after_comma {
                // 原来最后一段带逗号：前一段沿用逗号和收尾空白。
                Some(closing) => ctx.wsc.3 = Some(closing),
                None => {
                    ctx.wsc.3 = None;
                    ctx.wsc.2.push_str(&after_value);
                }
            }
        }
        None => {
            *context = Some(RtJSONObjectContext {
                wsc: (after_comma.unwrap_or(after_value),),
            })
        }
    }
    Ok(true)
}

/// 设置顶层段落 `key`（按 `style` 写新键），其余段落原样保留。
pub(crate) fn set_root_section(
    text: &mut RtJSONText,
    key: &str,
    value: &Value,
    style: KeyStyle,
) -> Result<(), AppError> {
    put_root_section(text, key, style, |indent| value_to_rt_value(value, indent))
}

/// 在配置文本上设置（`Some`）或删除（`None`）几个顶层段落，其余段落的注释和格式
/// 原样保留。不读写磁盘。
pub(crate) fn edit_root_sections(
    source: &str,
    style: KeyStyle,
    edits: &[(&str, Option<&Value>)],
) -> Result<String, AppError> {
    let mut text = parse(source)?;
    for (key, value) in edits {
        match value {
            Some(value) => set_root_section(&mut text, key, value, style)?,
            None => {
                remove_root_section(&mut text, key)?;
            }
        }
    }
    Ok(text.to_string())
}

/// 把 `from` 里的顶层段落 `key` 原样（含注释和格式）放进 `source`。
pub(crate) fn copy_root_section(
    source: &str,
    from: &str,
    key: &str,
    style: KeyStyle,
) -> Result<String, AppError> {
    let mut from_text = parse(from)?;
    let (pairs, _) = root_object(&mut from_text)?;
    // 重复的键以最后一个为准（和 JSON 解析器一致）。
    let value = pairs
        .iter()
        .rev()
        .find(|pair| json5_key_name(&pair.key) == Some(key))
        .map(|pair| pair.value.clone())
        .ok_or_else(|| AppError::Config(format!("Config has no '{key}' section")))?;
    let mut text = parse(source)?;
    put_root_section(&mut text, key, style, |_| Ok(value))?;
    Ok(text.to_string())
}

fn ensure_kvp_context(pair: &mut RtJSONKeyValuePair) -> &mut RtKeyValuePairContext {
    pair.context.get_or_insert_with(|| RtKeyValuePairContext {
        wsc: (String::new(), " ".to_string(), String::new(), None),
    })
}

fn extract_trailing_indent(separator_ws: &str) -> String {
    separator_ws
        .rsplit_once('\n')
        .map(|(_, tail)| tail.to_string())
        .unwrap_or_default()
}

fn derive_closing_ws_from_separator(separator_ws: &str) -> String {
    let Some((prefix, indent)) = separator_ws.rsplit_once('\n') else {
        return String::new();
    };

    let reduced_indent = if indent.ends_with('\t') {
        &indent[..indent.len().saturating_sub(1)]
    } else if indent.ends_with("  ") {
        &indent[..indent.len().saturating_sub(2)]
    } else if indent.ends_with(' ') {
        &indent[..indent.len().saturating_sub(1)]
    } else {
        indent
    };

    format!("{prefix}\n{reduced_indent}")
}

fn derive_entry_separator(leading_ws: &str) -> String {
    if leading_ws.is_empty() {
        return String::new();
    }

    if leading_ws.contains('\n') {
        return format!("\n{}", extract_trailing_indent(leading_ws));
    }

    String::new()
}

fn value_to_rt_value(value: &Value, parent_indent: &str) -> Result<RtJSONValue, AppError> {
    // `json-five` 0.3.1 can panic when pretty-printing nested empty maps/arrays.
    // Serialize with `serde_json` instead; the resulting JSON is valid JSON5 and
    // can still be parsed back into the round-trip AST we use for insertion.
    let source = serde_json::to_string_pretty(value)
        .map_err(|e| AppError::Config(format!("Failed to serialize JSON section: {e}")))?;

    let adjusted = reindent_json5_block(&source, parent_indent);
    let text = rt_from_str(&adjusted).map_err(|e| {
        AppError::Config(format!(
            "Failed to parse generated JSON5 section: {}",
            e.message
        ))
    })?;
    Ok(text.value)
}

fn reindent_json5_block(source: &str, parent_indent: &str) -> String {
    let normalized = normalize_json_five_output(source);
    if parent_indent.is_empty() || !normalized.contains('\n') {
        return normalized;
    }

    let mut lines = normalized.lines();
    let Some(first_line) = lines.next() else {
        return String::new();
    };

    let mut result = String::from(first_line);
    for line in lines {
        result.push('\n');
        result.push_str(parent_indent);
        result.push_str(line);
    }
    result
}

fn normalize_json_five_output(source: &str) -> String {
    source.replace("\\/", "/")
}

fn make_root_pair(
    key: &str,
    value: RtJSONValue,
    closing_ws: String,
    style: KeyStyle,
) -> RtJSONKeyValuePair {
    RtJSONKeyValuePair {
        key: make_key(key, style),
        value,
        context: Some(RtKeyValuePairContext {
            wsc: (String::new(), " ".to_string(), closing_ws, None),
        }),
    }
}

fn make_key(key: &str, style: KeyStyle) -> RtJSONValue {
    if style == KeyStyle::Json5 && is_identifier_key(key) {
        RtJSONValue::Identifier(key.to_string())
    } else {
        RtJSONValue::DoubleQuotedString(key.to_string())
    }
}

fn is_identifier_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    matches!(first, 'a'..='z' | 'A'..='Z' | '_' | '$')
        && chars.all(|ch| matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '$'))
}

fn json5_key_name(key: &RtJSONValue) -> Option<&str> {
    match key {
        RtJSONValue::Identifier(name)
        | RtJSONValue::DoubleQuotedString(name)
        | RtJSONValue::SingleQuotedString(name) => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn without(source: &str, key: &str) -> String {
        edit_root_sections(source, KeyStyle::Json5, &[(key, None)]).unwrap()
    }

    #[test]
    fn removing_a_section_drops_its_comment_and_keeps_the_next_one() {
        let source = "{\n  // models config\n  models: { mode: 'merge' },\n  // agents config\n  agents: { defaults: {} },\n  // tools config\n  tools: {},\n}\n";
        assert_eq!(
            without(source, "agents"),
            "{\n  // models config\n  models: { mode: 'merge' },\n  // tools config\n  tools: {},\n}\n"
        );
        assert_eq!(
            without(source, "models"),
            "{\n  // agents config\n  agents: { defaults: {} },\n  // tools config\n  tools: {},\n}\n"
        );
    }

    #[test]
    fn removing_the_last_section_keeps_the_closing_layout() {
        assert_eq!(
            without("{\n  models: {},\n  agents: {}\n}", "agents"),
            "{\n  models: {}\n}"
        );
        assert_eq!(
            without("{\n  models: {},\n  agents: {},\n}", "agents"),
            "{\n  models: {},\n}"
        );
        assert_eq!(without("{\n  agents: {},\n}", "agents"), "{\n}");
        assert_eq!(
            without("{\n  models: {},\n}", "agents"),
            "{\n  models: {},\n}"
        );
    }

    #[test]
    fn copying_a_section_keeps_its_original_comments() {
        let from = "{\n  agents: {\n    // keep me\n    defaults: { model: 'a/b' },\n  },\n}\n";
        let source =
            "{\n  meta: { touched: 1 },\n  agents: { defaults: { model: 'ofox/x' } },\n}\n";
        assert_eq!(
            copy_root_section(source, from, "agents", KeyStyle::Json5).unwrap(),
            "{\n  meta: { touched: 1 },\n  agents: {\n    // keep me\n    defaults: { model: 'a/b' },\n  },\n}\n"
        );
    }

    #[test]
    fn json_style_quotes_new_keys_and_keeps_other_sections_verbatim() {
        let source = "{\n  // keep\n  \"hooks\": {   \"a\": 1 }\n}\n";
        let edited = edit_root_sections(
            source,
            KeyStyle::Json,
            &[("env", Some(&json!({ "X": "1" })))],
        )
        .unwrap();
        assert_eq!(
            edited,
            "{\n  // keep\n  \"hooks\": {   \"a\": 1 },\n  \"env\": {\n    \"X\": \"1\"\n  }\n}\n"
        );
        assert_eq!(
            edit_root_sections(&edited, KeyStyle::Json, &[("env", None)]).unwrap(),
            source
        );
    }

    #[test]
    fn empty_source_is_an_empty_object() {
        let edited = edit_root_sections("", KeyStyle::Json, &[("a", Some(&json!(1)))]).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&edited).unwrap(),
            json!({ "a": 1 })
        );
    }

    #[test]
    fn copy_takes_the_last_duplicate_key() {
        let from = "{\n  \"a\": 1,\n  \"a\": 2\n}";
        let copied = copy_root_section("{\n  \"a\": 0\n}", from, "a", KeyStyle::Json).unwrap();
        assert_eq!(copied, "{\n  \"a\": 2\n}");
    }
}
