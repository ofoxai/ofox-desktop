//! Conservative YAML block-map editing, with semantic verification after each patch.
//! Flow maps, aliases and complex target scalars require a manual edit.
use crate::error::AppError;
use serde_json::{json, Value};
fn invalid() -> AppError {
    AppError::Config("library.nativeFormatUnsupported".into())
}
fn comment_start(text: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (i, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some('"') && ch == '\\' {
            escaped = true;
            continue;
        }
        if Some(ch) == quote {
            quote = None;
        } else if quote.is_none() && (ch == '\'' || ch == '"') {
            quote = Some(ch);
        } else if quote.is_none()
            && ch == '#'
            && (i == 0 || text[..i].ends_with(char::is_whitespace))
        {
            return Some(i);
        }
    }
    None
}
fn ambiguous_plain(text: &str) -> bool {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let unsigned = text.trim_start_matches(['+', '-']);
    matches!(lower.as_str(), "y" | "n" | "yes" | "no" | "on" | "off")
        || (unsigned.len() > 1
            && unsigned.starts_with('0')
            && unsigned.bytes().all(|c| c.is_ascii_digit()))
        || unsigned.starts_with("0o")
        || unsigned.starts_with("0O")
        || (unsigned.contains(':')
            && unsigned
                .bytes()
                .all(|c| c.is_ascii_digit() || c == b':' || c == b'.'))
        || (text.len() >= 10
            && text.as_bytes()[4] == b'-'
            && text.as_bytes()[7] == b'-'
            && text.as_bytes()[..4].iter().all(u8::is_ascii_digit))
}

/// The Rust YAML parser uses 1.2 scalar resolution; Hermes uses 1.1. Never
/// infer ownership from an ambiguous unquoted value in the MCP subtree.
pub fn validate_native_mcp(source: &str) -> Result<(), AppError> {
    let mut inside = false;
    for line in source.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let leading = line.len() - line.trim_start_matches(' ').len();
        if leading == 0 {
            inside = header(line)?.is_some_and(|h| h.key == "mcp_servers");
            if !inside {
                continue;
            }
        } else if !inside {
            continue;
        }
        let plain = line.trim();
        if let Some(value) = plain.strip_prefix("- ") {
            let value = value.trim();
            let value = value[..comment_start(value).unwrap_or(value.len())].trim();
            if ambiguous_plain(value) {
                return Err(invalid());
            }
            continue;
        }
        if let Some(h) = header(line)? {
            let key = line[h.indent..h.colon].trim();
            let value = h.tail[..comment_start(&h.tail).unwrap_or(h.tail.len())].trim();
            if ambiguous_plain(key) || ambiguous_plain(value) {
                return Err(invalid());
            }
            // Complex affected values have scalar-resolution and alias rules
            // outside this conservative writer's contract.
            if value.starts_with(['[', '{', '&', '*', '!', '|', '>']) && value != "{}" {
                let parsed: Value = serde_json::from_str(value).map_err(|_| invalid())?;
                if !parsed.is_array() && !parsed.is_object() {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}

struct Header {
    indent: usize,
    key: String,
    colon: usize,
    tail: String,
}
fn header(line: &str) -> Result<Option<Header>, AppError> {
    let text = line.trim_end_matches(['\r', '\n']);
    if text.trim().is_empty() || text.trim_start().starts_with('#') {
        return Ok(None);
    }
    let indent = text.len() - text.trim_start_matches(' ').len();
    if text[indent..].starts_with('\t') {
        return Err(invalid());
    }
    let mut quote = None;
    let mut escaped = false;
    let mut colon = None;
    for (i, ch) in text[indent..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some('"') && ch == '\\' {
            escaped = true;
            continue;
        }
        if Some(ch) == quote {
            quote = None;
        } else if quote.is_none() && (ch == '\'' || ch == '"') {
            quote = Some(ch);
        } else if quote.is_none() && ch == ':' {
            let after = &text[indent + i + 1..];
            if after.is_empty() || after.starts_with(char::is_whitespace) {
                colon = Some(indent + i);
                break;
            }
        }
    }
    let Some(colon) = colon else {
        return Ok(None);
    };
    let key_root: Value = serde_yaml::from_str(&format!("{}: null", text[indent..colon].trim()))
        .map_err(|_| invalid())?;
    let key = key_root
        .as_object()
        .and_then(|map| map.keys().next())
        .cloned()
        .ok_or_else(invalid)?;
    Ok(Some(Header {
        indent,
        key,
        colon,
        tail: text[colon + 1..].trim().to_string(),
    }))
}
fn edit_block(
    source: &str,
    path: &[String],
    value: Option<&Value>,
    indent: usize,
    eol: &str,
) -> Result<String, AppError> {
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let mut offset = 0;
    let mut found = None;
    for line in &lines {
        let leading = line.len() - line.trim_start_matches(' ').len();
        if leading != indent || line.trim_start().starts_with("- ") || line.trim() == "-" {
            offset += line.len();
            continue;
        }
        if let Some(h) = header(line)? {
            if h.indent == indent
                && h.key == path[0]
                && found.replace((offset, line.len(), h)).is_some()
            {
                return Err(invalid());
            }
        }
        offset += line.len();
    }
    if let Some((start, line_len, h)) = found {
        let end = {
            let mut end = source.len();
            let mut offset = start + line_len;
            for line in source[start + line_len..].split_inclusive('\n') {
                if !line.trim().is_empty() && !line.trim_start().starts_with('#') {
                    let leading = line.len() - line.trim_start_matches(' ').len();
                    let indentless_item = leading == indent
                        && (line.trim_start().starts_with("- ") || line.trim() == "-");
                    if leading <= indent && !indentless_item {
                        end = offset;
                        break;
                    }
                }
                offset += line.len();
            }
            end
        };
        let first = &source[start..start + line_len];
        let tail = h.tail[..comment_start(&h.tail).unwrap_or(h.tail.len())].trim();
        if path.len() > 1 {
            if !tail.is_empty() && tail != "{}" {
                return Err(invalid());
            }
            let child = &source[start + line_len..end];
            let child_indent = child
                .split_inclusive('\n')
                .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
                .map(|line| line.len() - line.trim_start_matches(' ').len())
                .min()
                .unwrap_or(indent + 2);
            if child_indent <= indent {
                return Err(invalid());
            }
            let edited = edit_block(child, &path[1..], value, child_indent, eol)?;
            let empty_child = edited
                .lines()
                .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'));
            let header_text = if empty_child {
                let comment = comment_start(first)
                    .map(|i| first[i..].trim_end_matches(['\r', '\n']))
                    .unwrap_or("");
                format!("{}: {{}} {}{eol}", &first[..h.colon], comment)
            } else if tail == "{}" {
                let comment = comment_start(first).map(|i| &first[i..]).unwrap_or(eol);
                format!("{}: {comment}", &first[..h.colon])
            } else {
                first.to_string()
            };
            return Ok(format!(
                "{}{header_text}{edited}{}",
                &source[..start],
                &source[end..]
            ));
        }
        if tail.starts_with(['|', '>', '&', '*', '!']) {
            return Err(invalid());
        }
        let mut comments = String::new();
        for line in source[start..end].split_inclusive('\n') {
            if line.trim_start().starts_with('#') {
                comments.push_str(line);
            } else if let Some(index) = comment_start(line) {
                comments.push_str(&" ".repeat(indent));
                comments.push_str(&line[index..]);
                if !comments.ends_with('\n') {
                    comments.push_str(eol);
                }
            }
        }
        let replacement = if let Some(value) = value {
            render(&path[0], value, indent, eol)?
        } else {
            String::new()
        };
        return Ok(format!(
            "{}{comments}{replacement}{}",
            &source[..start],
            &source[end..]
        ));
    }
    if value.is_none() {
        return Ok(source.to_string());
    }
    let mut leaf = value.expect("value was checked").clone();
    for key in path[1..].iter().rev() {
        leaf = json!({key:leaf});
    }
    let addition = render(&path[0], &leaf, indent, eol)?;
    Ok(format!(
        "{}{}{addition}",
        source,
        if source.is_empty() || source.ends_with('\n') {
            ""
        } else {
            eol
        }
    ))
}
fn render(key: &str, value: &Value, indent: usize, eol: &str) -> Result<String, AppError> {
    let key = serde_json::to_string(key).map_err(|_| invalid())?;
    let padding = " ".repeat(indent);
    if let Some(map) = value.as_object().filter(|map| !map.is_empty()) {
        let mut result = format!("{padding}{key}:{eol}");
        for (key, value) in map {
            result.push_str(&render(key, value, indent + 2, eol)?);
        }
        Ok(result)
    } else {
        // JSON flow values are valid YAML 1.1 and quote every string, including
        // on/yes/n and mapping keys which the native Hermes loader treats as booleans.
        let value = serde_json::to_string(value).map_err(|_| invalid())?;
        Ok(format!("{padding}{key}: {value}{eol}"))
    }
}
pub fn edit(source: &str, edits: &[(Vec<String>, Option<Value>)]) -> Result<String, AppError> {
    let (bom, body) = source
        .strip_prefix('\u{feff}')
        .map_or(("", source), |body| ("\u{feff}", body));
    let mut result = if body.trim() == "{}" {
        String::new()
    } else {
        body.to_string()
    };
    let eol = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let mut expected: Value = if result.trim().is_empty() {
        json!({})
    } else {
        serde_yaml::from_str(&result).map_err(|_| invalid())?
    };
    if !expected.is_object() {
        return Err(invalid());
    }
    for (path, value) in edits {
        if path.is_empty() {
            return Err(invalid());
        }
        let mut target = &mut expected;
        for key in &path[..path.len() - 1] {
            let map = target.as_object_mut().ok_or_else(invalid)?;
            if !map.contains_key(key) {
                map.insert(key.clone(), json!({}));
            }
            target = map.get_mut(key).expect("inserted key");
        }
        let map = target.as_object_mut().ok_or_else(invalid)?;
        if let Some(value) = value {
            map.insert(path.last().unwrap().clone(), value.clone());
        } else {
            map.remove(path.last().unwrap());
        }
        result = edit_block(&result, path, value.as_ref(), 0, eol)?;
        let actual: Value = if result.trim().is_empty() {
            json!({})
        } else {
            serde_yaml::from_str(&result).map_err(|_| invalid())?
        };
        if actual != expected {
            return Err(invalid());
        }
    }
    Ok(format!("{bom}{result}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn actual_hermes_provider_serializer_is_a_preserved_neighbor() {
        let source = serde_yaml::to_string(
            &json!({"custom_providers":[{"name":"acme","base_url":"https://example.org"}]}),
        )
        .unwrap();
        let result = edit(
            &source,
            &[(
                vec!["mcp_servers".into(), "demo".into(), "command".into()],
                Some(json!("server")),
            )],
        )
        .unwrap();
        assert!(result.starts_with(&source));
    }

    #[test]
    fn unrelated_provider_sequences_are_preserved() {
        let source="custom_providers:\n  - name: acme\n    base_url: https://example.org\nmodel: personal\n";
        let result = edit(
            source,
            &[(
                vec!["mcp_servers".into(), "demo".into(), "command".into()],
                Some(json!("server")),
            )],
        )
        .unwrap();
        assert!(result.starts_with(source));
    }

    #[test]
    fn edits_preserve_nested_comments_and_unknown_fields() {
        let original="\u{feff}model: personal\r\nmcp_servers:\r\n  demo:\r\n    # command note\r\n    command: old # inline\r\n    timeout: 12\r\n    env:\r\n      KEEP: mine\r\n      TOKEN: '${TOKEN}'\r\n  other:\r\n    command: user\r\n";
        let result = edit(
            original,
            &[(
                vec!["mcp_servers".into(), "demo".into(), "command".into()],
                Some(json!("new")),
            )],
        )
        .unwrap();
        assert!(result.contains("# command note"));
        assert!(result.contains("# inline"));
        assert!(result.starts_with("\u{feff}model: personal\r\n"));
        assert!(!result.replace("\r\n", "").contains('\n'));
        let value: Value = serde_yaml::from_str(result.trim_start_matches('\u{feff}')).unwrap();
        assert_eq!(value["mcp_servers"]["demo"]["timeout"], 12);
        assert_eq!(value["mcp_servers"]["demo"]["command"], "new");
        let removed = edit(
            &result,
            &[(
                vec!["mcp_servers".into(), "demo".into(), "command".into()],
                None,
            )],
        )
        .unwrap();
        assert!(removed.contains("# command note"));
        assert!(removed.contains("# inline"));
    }

    #[test]
    fn complex_or_duplicate_mapping_targets_refuse_lossy_rewrite() {
        for source in [
            "mcp_servers: {demo: {command: old}}\n",
            "mcp_servers:\n  demo:\n    command: old\n    command: other\n",
            "mcp_servers:\n  demo: &d\n    command: old\n",
        ] {
            assert!(edit(
                source,
                &[(
                    vec!["mcp_servers".into(), "demo".into(), "command".into()],
                    Some(json!("new"))
                )]
            )
            .is_err());
        }
    }
}
