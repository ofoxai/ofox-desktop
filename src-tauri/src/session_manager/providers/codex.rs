use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use regex::Regex;
use serde_json::Value;

use crate::codex_config::get_codex_config_dir;
use crate::session_manager::title::{clean_prompt, one_line, titled, untagged, TitleCandidates};
use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{extract_text, parse_timestamp_to_ms, read_head_tail_lines, truncate_summary};

const PROVIDER_ID: &str = "codex";

static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
        .unwrap()
});

pub fn scan_sessions() -> Vec<SessionMeta> {
    let root = get_codex_config_dir().join("sessions");
    scan_in(&root)
}

pub(crate) fn scan_in(root: &Path) -> Vec<SessionMeta> {
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files);
    // `<codex home>/sessions` sits beside the thread-name index
    let names = root
        .parent()
        .map(|home| thread_names(&home.join("session_index.jsonl")))
        .unwrap_or_default();

    let mut sessions = Vec::new();
    for path in files {
        if let Some(meta) = parse_with_names(&path, &names) {
            sessions.push(meta);
        }
    }

    sessions
}

type ThreadNames = Arc<HashMap<String, String>>;

static THREAD_NAMES: Mutex<Option<(PathBuf, u64, SystemTime, ThreadNames)>> = Mutex::new(None);

/// Codex's names for its threads (`session_index.jsonl`, append-only: the
/// last row for an id is its name). Read again only when the file changes.
/// Adapted from Magpie internal/sessions/codex_titles.go (MIT).
pub(crate) fn thread_names(index: &Path) -> ThreadNames {
    let Some(stat) = std::fs::metadata(index).ok().filter(|m| m.is_file()) else {
        return ThreadNames::default();
    };
    let modified = stat.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let mut cache = THREAD_NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((path, size, at, names)) = cache.as_ref() {
        if path == index && *size == stat.len() && *at == modified {
            return Arc::clone(names);
        }
    }
    let Ok(text) = std::fs::read_to_string(index) else {
        return ThreadNames::default();
    };
    let mut names = HashMap::new();
    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let (Some(id), Some(name)) = (
            row.get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty()),
            row.get("thread_name")
                .and_then(Value::as_str)
                .map(one_line)
                .filter(|name| !name.is_empty()),
        ) else {
            continue;
        };
        names.insert(id.to_string(), name);
    }
    let names = Arc::new(names);
    *cache = Some((
        index.to_path_buf(),
        stat.len(),
        modified,
        Arc::clone(&names),
    ));
    names
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    crate::session_manager::transcript::codex(path)
}

#[cfg(test)]
pub fn delete_session(_root: &Path, path: &Path, session_id: &str) -> Result<bool, String> {
    let meta = parse_session(path)
        .ok_or_else(|| format!("Failed to parse Codex session metadata: {}", path.display()))?;

    if meta.session_id != session_id {
        return Err(format!(
            "Codex session ID mismatch: expected {session_id}, found {}",
            meta.session_id
        ));
    }

    std::fs::remove_file(path).map_err(|e| {
        format!(
            "Failed to delete Codex session file {}: {e}",
            path.display()
        )
    })?;

    Ok(true)
}

/// What a Codex rollout's lines say about its title. Title rules adapted
/// from Magpie internal/sessions/codex.go (MIT).
#[derive(Debug, Default)]
pub(crate) struct CodexScan {
    pub candidates: TitleCandidates,
}

impl CodexScan {
    pub(crate) fn line(&mut self, value: &Value) {
        let Some(payload) = value.get("payload") else {
            return;
        };
        let payload_type = payload.get("type").and_then(Value::as_str);
        match value.get("type").and_then(Value::as_str) {
            Some("response_item")
                if payload_type == Some("message")
                    && payload.get("role").and_then(Value::as_str) == Some("user") =>
            {
                for text in user_texts(payload.get("content")) {
                    if self.candidates.prompt.is_none() {
                        self.candidates.prompt = clean_prompt(text);
                    }
                }
            }
            Some("event_msg") if payload_type == Some("user_message") => {
                let text = payload.get("message").and_then(Value::as_str).unwrap_or("");
                let candidates = &mut self.candidates;
                if candidates.prompt.is_none() {
                    candidates.prompt = clean_prompt(text);
                }
                if candidates.prompt.is_none() && candidates.message.is_none() {
                    candidates.message = Some(untagged(text)).filter(|t| !t.is_empty());
                }
            }
            _ => {}
        }
    }
}

/// The texts of a user message: each text item, or the message when it is
/// one string. Images are left out.
fn user_texts(content: Option<&Value>) -> Vec<&str> {
    match content {
        Some(Value::String(text)) => vec![text.as_str()],
        Some(Value::Array(items)) => items
            .iter()
            .filter(|item| {
                matches!(
                    item.get("type").and_then(Value::as_str),
                    Some("input_text" | "text")
                )
            })
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn parse_session(path: &Path) -> Option<SessionMeta> {
    parse_with_names(path, &HashMap::new())
}

fn parse_with_names(path: &Path, names: &HashMap<String, String>) -> Option<SessionMeta> {
    let (head, tail) = read_head_tail_lines(path, 10, 30).ok()?;

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut scan = CodexScan::default();

    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            if let Some(payload) = value.get("payload") {
                if session_id.is_none() {
                    session_id = payload
                        .get("id")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                }
                if project_dir.is_none() {
                    project_dir = payload
                        .get("cwd")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                }
                if let Some(ts) = payload.get("timestamp").and_then(parse_timestamp_to_ms) {
                    created_at.get_or_insert(ts);
                }
            }
        }
        scan.line(&value);
    }

    // Extract last_active_at and summary from tail lines (reverse order)
    let mut last_active_at: Option<i64> = None;
    let mut summary: Option<String> = None;

    for line in tail.iter().rev() {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if last_active_at.is_none() {
            last_active_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if summary.is_none() && value.get("type").and_then(Value::as_str) == Some("response_item") {
            if let Some(payload) = value.get("payload") {
                if payload.get("type").and_then(Value::as_str) == Some("message") {
                    let text = payload.get("content").map(extract_text).unwrap_or_default();
                    if !text.trim().is_empty() {
                        summary = Some(text);
                    }
                }
            }
        }
        if last_active_at.is_some() && summary.is_some() {
            break;
        }
    }
    if scan.candidates.prompt.is_none() {
        for line in &tail {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                scan.line(&value);
            }
        }
    }

    let session_id = session_id.or_else(|| infer_session_id_from_filename(path));
    let session_id = session_id?;

    let mut candidates = scan.candidates.with_directory(project_dir.as_deref());
    candidates.agent = names.get(&session_id).cloned();
    let (title, title_source) = titled(&candidates);

    let summary = summary.map(|text| truncate_summary(&text, 160));

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title,
        summary,
        project_dir,
        created_at,
        last_active_at,
        source_path: Some(path.to_string_lossy().to_string()),
        resume_command: Some(format!("codex resume {session_id}")),
        title_source,
    })
}

fn infer_session_id_from_filename(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_string_lossy();
    UUID_RE.find(&file_name).map(|mat| mat.as_str().to_string())
}

fn collect_jsonl_files(root: &Path, files: &mut Vec<PathBuf>) {
    if !root.exists() {
        return;
    }

    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            collect_jsonl_files(&path, files);
        } else if entry.file_type().is_ok_and(|t| t.is_file())
            && (path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
                || path.file_name().is_some_and(|n| {
                    n.to_string_lossy().ends_with(".jsonl.zst")
                        || n.to_string_lossy().ends_with(".jsonl.zstd")
                }))
        {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::title::TitleSource;
    use tempfile::tempdir;

    const THREAD: &str = "01a0bdc9-b5fd-7e63-9658-68bc8dce5ecd";

    fn rollout(root: &Path, lines: &[serde_json::Value]) -> PathBuf {
        let dir = root.join("sessions/2026/10/14");
        std::fs::create_dir_all(&dir).expect("create sessions");
        let path = dir.join(format!("rollout-2026-10-14T10-00-00-{THREAD}.jsonl"));
        let mut body = format!(
            "{}\n",
            serde_json::json!({"timestamp": "2026-10-14T10:00:00Z", "type": "session_meta",
                "payload": {"id": THREAD, "cwd": "/work/app"}})
        );
        for line in lines {
            body.push_str(&format!("{line}\n"));
        }
        std::fs::write(&path, body).expect("write rollout");
        path
    }

    fn user_items(texts: &[serde_json::Value]) -> serde_json::Value {
        serde_json::json!({"timestamp": "2026-10-14T10:00:01Z", "type": "response_item",
            "payload": {"type": "message", "role": "user", "content": texts}})
    }

    fn input_text(text: &str) -> serde_json::Value {
        serde_json::json!({"type": "input_text", "text": text})
    }

    #[test]
    fn the_codex_thread_name_beats_the_first_prompt() {
        let temp = tempdir().expect("tempdir");
        rollout(
            temp.path(),
            &[user_items(&[input_text("how do I deploy?")])],
        );
        std::fs::write(
            temp.path().join("session_index.jsonl"),
            format!(
                "{}\n{}\n{}\n",
                serde_json::json!({"id": THREAD, "thread_name": "查询长沙天气", "updated_at": "2026-10-14T10:00:30Z"}),
                serde_json::json!({"id": "other", "thread_name": "另一个聊天"}),
                serde_json::json!({"id": THREAD, "thread_name": "  长沙天气更新  \n ", "updated_at": "2026-10-14T10:01:00Z"}),
            ),
        )
        .expect("write index");

        let sessions = scan_in(&temp.path().join("sessions"));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title.as_deref(), Some("长沙天气更新"));
        assert_eq!(sessions[0].title_source, Some(TitleSource::Agent));
    }

    #[test]
    fn harness_items_are_not_prompts() {
        let temp = tempdir().expect("tempdir");
        let path = rollout(
            temp.path(),
            &[
                user_items(&[input_text("<environment_context>\n  <cwd>/work/app</cwd>\n</environment_context>")]),
                serde_json::json!({"timestamp": "2026-10-14T10:00:02Z", "type": "event_msg",
                    "payload": {"type": "user_message", "message": "<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>"}}),
                user_items(&[input_text("<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>")]),
                user_items(&[
                    input_text("<image name=[Image #1] path=\"/tmp/a.png\">"),
                    serde_json::json!({"type": "input_image", "image_url": "data:image/png;base64,AAAA"}),
                    input_text("</image>"),
                    input_text("make the button green"),
                ]),
            ],
        );

        let meta = parse_session(&path).expect("parse");
        assert_eq!(meta.title.as_deref(), Some("make the button green"));
        assert_eq!(meta.title_source, Some(TitleSource::Prompt));
    }

    fn item(payload: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"timestamp": "2026-10-14T10:00:03Z", "type": "response_item", "payload": payload})
    }

    #[test]
    fn transcript_marks_harness_context_as_system_and_reasoning_as_thinking() {
        let temp = tempdir().expect("tempdir");
        let path = rollout(
            temp.path(),
            &[
                item(serde_json::json!({"type": "message", "role": "developer",
                    "content": [{"type": "input_text", "text": "<permissions instructions>sandboxed</permissions instructions>"}]})),
                user_items(&[input_text(
                    "<environment_context>\n  <cwd>/work/app</cwd>\n</environment_context>",
                )]),
                user_items(&[input_text(
                    "<user_instructions>be brief</user_instructions>",
                )]),
                user_items(&[
                    input_text("<image name=[Image #1] path=\"/tmp/a.png\">"),
                    serde_json::json!({"type": "input_image", "image_url": "data:image/png;base64,AAAA"}),
                    input_text("</image>"),
                    input_text("make it green"),
                ]),
                item(
                    serde_json::json!({"type": "reasoning", "summary": [], "encrypted_content": "opaque"}),
                ),
                item(serde_json::json!({"type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "Checking the CSS"}], "encrypted_content": "opaque"})),
                item(
                    serde_json::json!({"type": "custom_tool_call", "call_id": "c1", "name": "apply_patch",
                    "input": "*** Begin Patch"}),
                ),
                item(
                    serde_json::json!({"type": "custom_tool_call_output", "call_id": "c1",
                    "output": [{"type": "input_text", "text": "Done"}]}),
                ),
                item(serde_json::json!({"type": "message", "role": "assistant",
                    "content": [{"type": "output_text", "text": "It is green."}]})),
            ],
        );

        let messages = load_messages(&path)
            .expect("load")
            .into_iter()
            .map(|m| (m.role, m.content))
            .collect::<Vec<_>>();
        let pair = |role: &str, text: &str| (role.to_string(), text.to_string());
        assert_eq!(
            messages,
            vec![
                pair(
                    "system",
                    "<permissions instructions>sandboxed</permissions instructions>"
                ),
                pair(
                    "system",
                    "<environment_context>\n  <cwd>/work/app</cwd>\n</environment_context>"
                ),
                pair("system", "<user_instructions>be brief</user_instructions>"),
                pair("user", "[Image]\nmake it green"),
                pair("thinking", "Checking the CSS"),
                pair("tool_call", "*** Begin Patch"),
                pair("tool", "Done"),
                pair("assistant", "It is green."),
            ]
        );
    }

    #[test]
    fn delete_session_removes_jsonl_file() {
        let temp = tempdir().expect("tempdir");
        let path = temp
            .path()
            .join("rollout-2026-03-06T21-50-12-019cc369-bd7c-7891-b371-7b20b4fe0b18.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"019cc369-bd7c-7891-b371-7b20b4fe0b18\",\"cwd\":\"/tmp/project\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"hello\"}}\n"
            ),
        )
        .expect("write session");

        delete_session(temp.path(), &path, "019cc369-bd7c-7891-b371-7b20b4fe0b18")
            .expect("delete session");

        assert!(!path.exists());
    }

    #[test]
    fn parse_session_uses_first_user_message_as_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp/project\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"How do I deploy?\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:14Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"Here is how...\"}}\n"
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("How do I deploy?"));
    }

    #[test]
    fn parse_session_skips_agents_md_injection() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp/project\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",\"content\":\"<permissions>\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"# AGENTS.md instructions for /tmp/project\\n<INSTRUCTIONS>Do stuff</INSTRUCTIONS>\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:14Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"Fix the login bug\"}}\n"
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        // Should skip AGENTS.md injection and use the real user message
        assert_eq!(meta.title.as_deref(), Some("Fix the login bug"));
    }

    #[test]
    fn parse_session_falls_back_to_dir_basename() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp/my-project\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"Hello\"}}\n"
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        // No user message → falls back to dir basename
        assert_eq!(meta.title.as_deref(), Some("my-project"));
    }

    #[test]
    fn parse_session_truncates_long_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        let long_msg = "a".repeat(200);
        std::fs::write(
            &path,
            format!(
                "{{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"test-id\",\"cwd\":\"/tmp/p\"}}}}\n\
                 {{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":\"{long_msg}\"}}}}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        let title = meta.title.unwrap();
        assert_eq!(title.chars().count(), 161);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn load_messages_includes_function_call_and_output() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"list files\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:14Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"shell\",\"arguments\":\"{\\\"cmd\\\":[\\\"ls\\\"]}\",\"call_id\":\"call_1\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:15Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_1\",\"output\":\"file1.txt\\nfile2.txt\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:16Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done.\"}]}}\n",
            ),
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 4);

        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "list files");

        assert_eq!(msgs[1].role, "tool_call");
        assert_eq!(msgs[1].name.as_deref(), Some("shell"));
        assert_eq!(msgs[1].content, "{\n  \"cmd\": [\n    \"ls\"\n  ]\n}");

        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].name.as_deref(), Some("shell"));
        assert!(msgs[2].content.contains("file1.txt"));

        assert_eq!(msgs[3].role, "assistant");
        assert_eq!(msgs[3].content, "Done.");
    }
}
