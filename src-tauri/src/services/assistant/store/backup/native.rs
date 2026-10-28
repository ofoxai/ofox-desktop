//! Validate the pinned Pi session-3 shape before publishing imported history.
//! Keep only lineage metadata in memory; image bodies can legitimately exceed
//! the runtime protocol frame size and are bounded by the native-file limit.
use super::*;

#[derive(Clone, Default)]
pub(super) struct Entry {
    pub role: Option<String>,
    pub run: Option<(String, String)>,
}

fn text(value: &Value) -> bool {
    value.is_string()
}
fn number(value: &Value) -> bool {
    value
        .as_f64()
        .is_some_and(|value| value.is_finite() && value >= 0.0)
}

fn content(value: &Value, role: &str) -> bool {
    if matches!(role, "user" | "system" | "custom") && value.is_string() {
        return true;
    }
    value.as_array().is_some_and(|parts| {
        parts.iter().all(|part| match part["type"].as_str() {
            Some("text") => text(&part["text"]),
            Some("image") if matches!(role, "user" | "toolResult" | "custom") => {
                text(&part["data"]) && text(&part["mimeType"])
            }
            Some("thinking") if role == "assistant" => text(&part["thinking"]),
            Some("toolCall") if role == "assistant" => {
                text(&part["id"]) && text(&part["name"]) && part["arguments"].is_object()
            }
            _ => false,
        })
    })
}

fn usage(value: &Value) -> bool {
    ["input", "output", "cacheRead", "cacheWrite", "totalTokens"]
        .iter()
        .all(|name| number(&value[*name]))
        && ["input", "output", "cacheRead", "cacheWrite", "total"]
            .iter()
            .all(|name| number(&value["cost"][*name]))
}

fn message(value: &Value) -> bool {
    let Some(role) = value["role"].as_str() else {
        return false;
    };
    if !number(&value["timestamp"]) || !content(&value["content"], role) {
        return false;
    }
    match role {
        "user" => true,
        "system" => {
            (value["sections"].is_null()
                || value["sections"].as_object().is_some_and(|parts| {
                    parts
                        .values()
                        .all(|part| part.is_null() || part.is_string())
                }))
                && (value["toolsAdded"].is_null()
                    || value["toolsAdded"].as_array().is_some_and(|tools| {
                        tools.iter().all(|tool| {
                            text(&tool["name"])
                                && text(&tool["description"])
                                && tool["parameters"].is_object()
                        })
                    }))
                && (value["toolsRemoved"].is_null()
                    || value["toolsRemoved"].as_array().is_some_and(|tools| {
                        tools
                            .iter()
                            .all(|tool| tool.is_string() || text(&tool["name"]))
                    }))
        }
        "assistant" => {
            ["api", "provider", "model"]
                .iter()
                .all(|name| text(&value[*name]))
                && usage(&value["usage"])
                && matches!(
                    value["stopReason"].as_str(),
                    Some(
                        "pending"
                            | "stop"
                            | "length"
                            | "toolUse"
                            | "error"
                            | "aborted"
                            | "deferred"
                    )
                )
        }
        "toolResult" => {
            text(&value["toolCallId"])
                && text(&value["toolName"])
                && value["isError"].is_boolean()
                && (value["usage"].is_null() || usage(&value["usage"]))
        }
        _ => false,
    }
}

pub(super) fn validate(
    bytes: &[u8],
    record: &SessionRecord,
) -> Result<HashMap<String, Entry>, String> {
    let mut operations = HashMap::new();
    let mut operation_ids = HashSet::new();
    for operation in &record.operations {
        validate_id(&operation.run_id).map_err(|_| INVALID)?;
        validate_id(&operation.id).map_err(|_| INVALID)?;
        if operations
            .insert(operation.run_id.as_str(), operation.id.as_str())
            .is_some()
            || !operation_ids.insert(operation.id.as_str())
        {
            return Err(INVALID.into());
        }
    }
    let run_link = |value: &Value| -> Result<(String, String), String> {
        let run = value["runId"].as_str().ok_or(INVALID)?;
        let operation = value["operationId"].as_str().ok_or(INVALID)?;
        if operations.get(run).copied() != Some(operation) {
            return Err(INVALID.into());
        }
        Ok((run.into(), operation.into()))
    };
    let raw = std::str::from_utf8(bytes).map_err(|_| INVALID)?;
    let mut lines = raw.lines().filter(|line| !line.trim().is_empty());
    let header: Value = serde_json::from_str(lines.next().ok_or(INVALID)?).map_err(|_| INVALID)?;
    if header["type"] != "session"
        || header["version"] != 3
        || !text(&header["cwd"])
        || !text(&header["timestamp"])
        || validate_id(header["id"].as_str().ok_or(INVALID)?).is_err()
    {
        return Err(INVALID.into());
    }
    let mut entries: HashMap<String, Entry> = HashMap::new();
    for line in lines {
        let item: Value = serde_json::from_str(line).map_err(|_| INVALID)?;
        let id = item["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or(INVALID)?;
        if entries.contains_key(id) || !text(&item["timestamp"]) {
            return Err(INVALID.into());
        }
        let mut entry = match item.get("parentId") {
            Some(Value::Null) => Entry::default(),
            Some(Value::String(parent)) => entries.get(parent).cloned().ok_or(INVALID)?,
            _ => return Err(INVALID.into()),
        };
        // Only run lineage is inherited; a non-message cannot satisfy a saved
        // nativeEntryId merely because an ancestor was a message.
        entry.role = None;
        let valid = match item["type"].as_str() {
            Some("message") => {
                if !message(&item["message"]) {
                    return Err(INVALID.into());
                }
                let role = item["message"]["role"].as_str().ok_or(INVALID)?;
                entry.role = Some(role.into());
                if matches!(role, "user" | "assistant" | "toolResult") {
                    let (run, _) = entry.run.as_ref().ok_or(INVALID)?;
                    if !record
                        .messages
                        .iter()
                        .any(|message| message.run_id.as_ref() == Some(run))
                    {
                        return Err(INVALID.into());
                    }
                }
                true
            }
            Some("custom") => match item["customType"].as_str() {
                Some("ofox-desktop-run") => {
                    entry.run = Some(run_link(&item["data"])?);
                    true
                }
                Some("ofox-desktop-usage") => {
                    run_link(&item["data"])?;
                    item["data"].is_object()
                }
                Some(_) => true,
                None => false,
            },
            Some("model_change") => text(&item["provider"]) && text(&item["modelId"]),
            Some("thinking_level_change") => text(&item["thinkingLevel"]),
            Some("usage") => {
                text(&item["kind"])
                    && text(&item["provider"])
                    && text(&item["model"])
                    && usage(&item["usage"])
            }
            Some("compaction") => {
                text(&item["summary"])
                    && number(&item["tokensBefore"])
                    && item["firstKeptEntryId"]
                        .as_str()
                        .is_some_and(|id| entries.contains_key(id))
                    && (item["systemMessage"].is_null() || message(&item["systemMessage"]))
            }
            Some("branch_summary") => {
                text(&item["summary"])
                    && item["fromId"]
                        .as_str()
                        .is_some_and(|id| entries.contains_key(id))
            }
            Some("custom_message") => {
                text(&item["customType"])
                    && content(&item["content"], "custom")
                    && item["display"].is_boolean()
            }
            Some("context_edit") => item["targetId"]
                .as_str()
                .and_then(|id| entries.get(id))
                .is_some_and(|target| {
                    item["replacement"].is_null()
                        || content(
                            &item["replacement"]["content"],
                            target.role.as_deref().unwrap_or("custom"),
                        )
                }),
            Some("label") => {
                item["targetId"]
                    .as_str()
                    .is_some_and(|id| entries.contains_key(id))
                    && (item["label"].is_null() || text(&item["label"]))
            }
            Some("session_info") => item["name"].is_null() || text(&item["name"]),
            _ => false,
        };
        if !valid {
            return Err(INVALID.into());
        }
        entries.insert(id.into(), entry);
    }
    Ok(entries)
}
