use std::path::Path;

use serde_json::Value;

use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{extract_text, parse_timestamp_to_ms, truncate_summary};
use crate::session_manager::title::{titled, TitleCandidates};

const PROVIDER_ID: &str = "gemini";

pub(crate) fn session_identity(path: &Path) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Identity {
        #[serde(rename = "sessionId")]
        id: String,
    }
    let file = std::fs::File::open(path).ok()?;
    // Parse just the first JSON value. Unknown legacy body fields are skipped;
    // a JSONL header does not require replaying the entire conversation.
    serde_json::Deserializer::from_reader(std::io::BufReader::new(file))
        .into_iter::<Identity>()
        .next()?
        .ok()
        .map(|identity| identity.id)
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    let gemini_dir = crate::gemini_config::get_gemini_dir();
    let tmp_dir = gemini_dir.join("tmp");
    scan_in(&tmp_dir)
}

pub(crate) fn scan_in(tmp_dir: &Path) -> Vec<SessionMeta> {
    if !tmp_dir.exists() {
        return Vec::new();
    }

    let mut sessions = Vec::new();

    // Iterate over project directories: tmp/<project_name>/chats/session-*.json
    let project_dirs = match std::fs::read_dir(tmp_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    for entry in project_dirs.flatten() {
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let chats_dir = entry.path().join("chats");
        if !std::fs::symlink_metadata(&chats_dir)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            continue;
        }

        let chat_files = match std::fs::read_dir(&chats_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        let project_root_file = entry.path().join(".project_root");
        let project_dir = if std::fs::symlink_metadata(&project_root_file)
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        {
            std::fs::read_to_string(project_root_file).ok()
        } else {
            None
        };

        for file_entry in chat_files.flatten() {
            if !file_entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let path = file_entry.path();
            if !path
                .extension()
                .is_some_and(|e| e == "json" || e == "jsonl")
            {
                continue;
            }
            if let Some(mut meta) = parse_session(&path) {
                meta.project_dir = project_dir.clone();
                if meta.title.is_none() {
                    (meta.title, meta.title_source) =
                        titled(&TitleCandidates::default().with_directory(project_dir.as_deref()));
                }
                sessions.push(meta);
            }
        }
    }

    sessions
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let value = read_conversation(path)?;

    let messages = value
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| "No messages array found".to_string())?;

    let mut result = Vec::new();
    for msg in messages {
        let role = match msg.get("type").and_then(Value::as_str) {
            Some("gemini") => "assistant",
            Some("user") => "user",
            Some("info") | Some("error") => continue,
            Some(_) | None => continue,
        };

        // Gemini content may be a plain string or an array of {text: ...} objects
        let mut content = match msg.get("content") {
            Some(Value::String(s)) => s.to_string(),
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };

        // Append tool call names from the optional toolCalls array
        if let Some(Value::Array(calls)) = msg.get("toolCalls") {
            for call in calls {
                if let Some(name) = call.get("name").and_then(Value::as_str) {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&format!("[Tool: {name}]"));
                    if let Some(args) = call.get("args") {
                        content.push_str(&format!(
                            "\n{}",
                            serde_json::to_string_pretty(args).unwrap_or_default()
                        ));
                    }
                    if let Some(result) = call.get("result") {
                        content.push_str(&format!("\n{}", extract_text(result)));
                    }
                }
            }
        }

        if let Some(thoughts) = msg.get("thoughts").and_then(Value::as_array) {
            for thought in thoughts {
                content.push_str(&format!(
                    "\n[Thinking]\n{}",
                    serde_json::to_string_pretty(thought).unwrap_or_default()
                ));
            }
        }

        if content.trim().is_empty() {
            continue;
        }

        let ts = msg.get("timestamp").and_then(parse_timestamp_to_ms);

        result.push(SessionMessage {
            role: role.to_string(),
            content,
            ts,
            name: None,
        });
    }

    Ok(result)
}

#[cfg(test)]
pub fn delete_session(_root: &Path, path: &Path, session_id: &str) -> Result<bool, String> {
    let meta = parse_session(path).ok_or_else(|| {
        format!(
            "Failed to parse Gemini session metadata: {}",
            path.display()
        )
    })?;

    if meta.session_id != session_id {
        return Err(format!(
            "Gemini session ID mismatch: expected {session_id}, found {}",
            meta.session_id
        ));
    }

    std::fs::remove_file(path).map_err(|e| {
        format!(
            "Failed to delete Gemini session file {}: {e}",
            path.display()
        )
    })?;

    Ok(true)
}

pub(crate) fn parse_session(path: &Path) -> Option<SessionMeta> {
    let value = read_conversation(path).ok()?;

    let session_id = value.get("sessionId").and_then(Value::as_str)?.to_string();

    let created_at = value.get("startTime").and_then(parse_timestamp_to_ms);
    let last_active_at = value.get("lastUpdated").and_then(parse_timestamp_to_ms);

    // Derive title from first user message
    let first_user = value
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|msgs| {
            msgs.iter()
                .find(|m| m.get("type").and_then(Value::as_str) == Some("user"))
                .map(|m| extract_text(&m["content"]))
                .filter(|s| !s.trim().is_empty())
        });
    let (title, title_source) =
        titled(&TitleCandidates::default().with_first_user_text(first_user.as_deref()));

    let source_path = path.to_string_lossy().to_string();

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title,
        summary: first_user.map(|s| truncate_summary(&s, 160)),
        project_dir: None, // (optionally) populated later
        created_at,
        last_active_at: last_active_at.or(created_at),
        source_path: Some(source_path),
        resume_command: Some(format!("gemini --resume {session_id}")),
        title_source,
    })
}

// The JSONL schema is documented by Gemini CLI chatRecordingService.ts:
// https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/services/chatRecordingService.ts
// This independent reader preserves current message order after rewind/patch/set.
pub(crate) fn read_conversation(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if value.get("messages").is_some() || path.extension().is_some_and(|e| e == "json") {
            return Ok(value);
        }
    }
    let mut metadata = serde_json::Map::new();
    let mut messages: Vec<Value> = Vec::new();
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(id) = record["$rewindTo"].as_str() {
            let index = messages
                .iter()
                .position(|m| m["id"].as_str() == Some(id))
                .unwrap_or(0);
            messages.truncate(index);
        } else if let Some(patch) = record.get("$patch") {
            apply_patch(&mut messages, patch);
        } else if let Some(set) = record["$set"].as_object() {
            if let Some(replacement) = set.get("messages").and_then(Value::as_array) {
                messages = replacement.clone();
            }
            metadata.extend(set.clone());
        } else if record["id"].is_string() && record["type"].is_string() {
            if let Some(existing) = messages.iter_mut().find(|m| m["id"] == record["id"]) {
                *existing = record;
            } else {
                messages.push(record);
            }
        } else if let Some(object) = record
            .as_object()
            .filter(|object| object.get("sessionId").is_some())
        {
            if let Some(initial) = object.get("messages").and_then(Value::as_array) {
                messages.extend(initial.clone());
            }
            metadata.extend(object.clone());
        }
    }
    if !metadata.get("sessionId").is_some_and(Value::is_string) {
        return Err("Gemini session metadata is missing".into());
    }
    metadata.insert("messages".into(), Value::Array(messages));
    Ok(Value::Object(metadata))
}

fn apply_patch(messages: &mut Vec<Value>, patch: &Value) {
    if let Some(id) = patch["id"].as_str() {
        if let Some(message) = messages.iter_mut().find(|m| m["id"].as_str() == Some(id)) {
            if let Some(content) = patch.get("content") {
                message["content"] = content.clone();
            }
            if let (Some(updates), Some(calls)) = (
                patch["toolCalls"].as_array(),
                message["toolCalls"].as_array_mut(),
            ) {
                for update in updates {
                    if let Some(call) = calls.iter_mut().find(|call| call["id"] == update["id"]) {
                        if let Some(result) = update.get("result") {
                            call["result"] = result.clone();
                        }
                    }
                }
            }
        }
    }
    if let Some(updates) = patch["updates"].as_array() {
        for update in updates {
            apply_patch(messages, update);
        }
    }
    if let Some(ids) = patch["removeIds"].as_array() {
        messages.retain(|m| !ids.contains(&m["id"]));
    }
    if let Some(ids) = patch["orderIds"].as_array() {
        let mut ordered = Vec::new();
        for id in ids {
            if let Some(index) = messages.iter().position(|m| m["id"] == *id) {
                ordered.push(messages.remove(index));
            }
        }
        messages.extend(ordered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn jsonl_rewind_patch_order_and_set_keep_only_current_conversation() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("session-s.jsonl");
        let lines = [
            r#"{"sessionId":"s","projectHash":"p","startTime":"2026-01-01T00:00:00Z"}"#,
            r#"{"id":"a","type":"user","content":[{"text":"first prompt"}]}"#,
            r#"{"id":"discard","type":"gemini","content":"abandoned"}"#,
            r#"{"$rewindTo":"discard"}"#,
            r#"{"id":"b","type":"gemini","content":"initial","toolCalls":[{"id":"tool","name":"read_file","args":{"path":"a.rs"},"result":[{"text":"old result"}]}]}"#,
            r#"{"$patch":{"id":"b","content":"updated","toolCalls":[{"id":"tool","result":[{"text":"full result"}]}]}}"#,
            r#"{"id":"c","type":"user","content":"to remove"}"#,
            r#"{"$patch":{"removeIds":["c"],"orderIds":["a"]}}"#,
            r#"{"$set":{"lastUpdated":"2026-01-02T00:00:00Z"}}"#,
            "{incomplete",
        ];
        let before = lines.join("\n");
        std::fs::write(&path, &before).unwrap();
        let record = read_conversation(&path).unwrap();
        assert_eq!(record["messages"][0]["id"], "b");
        assert_eq!(record["messages"][1]["id"], "a");
        let messages = load_messages(&path).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages[0].content.contains("updated"));
        assert!(messages[0].content.contains("full result"));
        assert!(messages[0].content.contains("a.rs"));
        assert!(!messages
            .iter()
            .any(|m| m.content.contains("abandoned") || m.content.contains("to remove")));
        assert_eq!(
            parse_session(&path).unwrap().last_active_at,
            Some(1_767_312_000_000)
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        std::fs::write(&path, "{\"sessionId\":\"s\",\"projectHash\":\"p\"}\n{\"$set\":{\"messages\":[{\"id\":\"x\",\"type\":\"user\",\"content\":\"reset\"}]}}\n").unwrap();
        assert_eq!(load_messages(&path).unwrap()[0].content, "reset");
    }

    #[test]
    fn delete_session_removes_json_file() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-2026-03-06T10-17-test.json");
        std::fs::write(
            &path,
            r#"{
              "sessionId": "gemini-session-123",
              "startTime": "2026-03-06T10:17:58.000Z",
              "lastUpdated": "2026-03-06T10:20:00.000Z",
              "messages": [
                {
                  "id": "msg-1",
                  "timestamp": "2026-03-06T10:17:58.000Z",
                  "type": "user",
                  "content": "hello"
                }
              ]
            }"#,
        )
        .expect("write session");

        delete_session(temp.path(), &path, "gemini-session-123").expect("delete session");

        assert!(!path.exists());
    }

    #[test]
    fn load_messages_handles_array_content() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.json");
        std::fs::write(
            &path,
            r#"{
              "sessionId": "test",
              "messages": [
                {"id":"1","timestamp":"2026-03-06T10:00:00Z","type":"user","content":[{"text":"hello"}]},
                {"id":"2","timestamp":"2026-03-06T10:00:01Z","type":"gemini","content":"world"},
                {"id":"3","timestamp":"2026-03-06T10:00:02Z","type":"info","content":"system info"},
                {"id":"4","timestamp":"2026-03-06T10:00:03Z","type":"error","content":"MCP ERROR"}
              ]
            }"#,
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "world");
    }

    #[test]
    fn load_messages_includes_tool_calls() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.json");
        std::fs::write(
            &path,
            r#"{
              "sessionId": "test",
              "messages": [
                {"id":"1","timestamp":"2026-03-10T08:24:50Z","type":"gemini","content":"","toolCalls":[{"id":"call_1","name":"web_search","args":{"query":"test"}}]},
                {"id":"2","timestamp":"2026-03-10T08:25:00Z","type":"gemini","content":"Here are the results.","toolCalls":[{"id":"call_2","name":"web_fetch","args":{"url":"http://example.com"}}]}
              ]
            }"#,
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "assistant");
        assert!(msgs[0].content.contains("[Tool: web_search]"));
        assert_eq!(msgs[1].role, "assistant");
        assert!(msgs[1].content.contains("Here are the results."));
        assert!(msgs[1].content.contains("[Tool: web_fetch]"));
    }
}
