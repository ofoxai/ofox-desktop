use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use serde::de::IgnoredAny;
use serde::Deserialize;
use serde_json::Value;

use crate::config::get_claude_config_dir;
use crate::session_manager::managed::Subagent;
use crate::session_manager::meta_cache::SessionDetails;
use crate::session_manager::title::{clean_prompt, one_line, titled, untagged, TitleCandidates};
use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{
    extract_text, for_each_line, parse_timestamp_to_ms, read_head_tail_lines, truncate_summary,
};

const PROVIDER_ID: &str = "claude";

pub fn scan_sessions() -> Vec<SessionMeta> {
    let root = get_claude_config_dir().join("projects");
    let mut files = Vec::new();
    collect_jsonl_files(&root, &mut files);

    let mut sessions = Vec::new();
    for path in files {
        if let Some(meta) = parse_session(&path) {
            sessions.push(meta);
        }
    }

    sessions
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    crate::session_manager::transcript::claude(path)
}

#[cfg(test)]
pub fn delete_session(_root: &Path, path: &Path, session_id: &str) -> Result<bool, String> {
    let meta = parse_session(path).ok_or_else(|| {
        format!(
            "Failed to parse Claude session metadata: {}",
            path.display()
        )
    })?;

    if meta.session_id != session_id {
        return Err(format!(
            "Claude session ID mismatch: expected {session_id}, found {}",
            meta.session_id
        ));
    }

    if let Some(stem) = path.file_stem() {
        let sibling = path.parent().unwrap_or_else(|| Path::new("")).join(stem);
        remove_path_if_exists(&sibling).map_err(|e| {
            format!(
                "Failed to delete Claude session sidecar {}: {e}",
                sibling.display()
            )
        })?;
    }

    std::fs::remove_file(path).map_err(|e| {
        format!(
            "Failed to delete Claude session file {}: {e}",
            path.display()
        )
    })?;

    Ok(true)
}

/// What a Claude Code session file's lines say about its title and length.
/// Title rules adapted from Magpie internal/sessions/claude.go (MIT).
#[derive(Debug, Default)]
pub(crate) struct ClaudeScan {
    pub candidates: TitleCandidates,
    prompts: u32,
    replies: HashSet<String>,
    unnamed_replies: u32,
}

/// The parts of an assistant line a scan needs, read without the content.
#[derive(Deserialize)]
struct AssistantLine<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    message: Option<AssistantMessage<'a>>,
}

#[derive(Deserialize)]
struct AssistantMessage<'a> {
    #[serde(borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    model: Option<Cow<'a, str>>,
    usage: Option<IgnoredAny>,
}

impl ClaudeScan {
    pub(crate) fn line(&mut self, value: &Value) {
        let kind = value.get("type").and_then(Value::as_str);
        let message = value.get("message");
        let role = message.and_then(|m| m.get("role")).and_then(Value::as_str);
        match kind {
            // the agent renames a session as it goes: the last name stands
            Some("ai-title") => last_text(&mut self.candidates.agent, value.get("aiTitle")),
            Some("custom-title") => {
                last_text(&mut self.candidates.custom, value.get("customTitle"))
            }
            Some("summary") if self.candidates.summary.is_none() => {
                last_text(&mut self.candidates.summary, value.get("summary"))
            }
            Some("assistant") => {
                let field = |key| message.and_then(|m| m.get(key));
                self.reply(
                    field("id").and_then(Value::as_str),
                    field("model").and_then(Value::as_str),
                    field("usage").is_some(),
                );
            }
            Some("user") | None if role == Some("user") => {
                let flagged = |key| value.get(key).and_then(Value::as_bool) == Some(true);
                if flagged("isMeta") || flagged("isSidechain") {
                    return;
                }
                let text = message.and_then(|m| m.get("content")).and_then(user_text);
                let prompt = text.as_deref().and_then(clean_prompt);
                if prompt.is_some() {
                    self.prompts += 1;
                }
                let candidates = &mut self.candidates;
                if candidates.prompt.is_none() {
                    if prompt.is_some() {
                        candidates.prompt = prompt;
                    } else if candidates.message.is_none() {
                        candidates.message = text.map(|t| untagged(&t)).filter(|t| !t.is_empty());
                    }
                }
            }
            _ => {}
        }
    }

    /// An assistant line: a reply when it spent tokens (a message streamed
    /// over several lines repeats its id and counts once).
    fn reply(&mut self, id: Option<&str>, model: Option<&str>, usage: bool) {
        if !usage || model == Some("<synthetic>") {
            return;
        }
        match id {
            Some(id) => {
                self.replies.insert(id.to_string());
            }
            None => self.unnamed_replies += 1,
        }
    }

    /// Prompts typed plus replies made.
    pub(crate) fn message_count(&self) -> u32 {
        let replies = u32::try_from(self.replies.len()).unwrap_or(u32::MAX);
        self.prompts
            .saturating_add(replies)
            .saturating_add(self.unnamed_replies)
    }
}

/// The whole file's title candidates and message count. Assistant lines, the
/// bulk of a file, are read without their content.
pub(crate) fn details(path: &Path) -> Option<SessionDetails> {
    let mut scan = ClaudeScan::default();
    for_each_line(path, |line| {
        let Ok(header) = serde_json::from_str::<AssistantLine<'_>>(line) else {
            return;
        };
        if header.kind.as_deref() == Some("assistant") {
            if let Some(message) = header.message {
                scan.reply(
                    message.id.as_deref(),
                    message.model.as_deref(),
                    message.usage.is_some(),
                );
            }
        } else if matches!(
            header.kind.as_deref(),
            None | Some("user" | "ai-title" | "custom-title" | "summary")
        ) {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                scan.line(&value);
            }
        }
    })
    .ok()?;
    Some(SessionDetails {
        message_count: scan.message_count(),
        candidates: scan.candidates,
        ..SessionDetails::default()
    })
}

/// The subagents a Claude Code session keeps beside its file, under
/// `<session>/subagents/`, oldest first (Magpie internal/sessions/subagents.go
/// `subagentsOf`, MIT).
pub(crate) fn subagents(source: &Path) -> Vec<Subagent> {
    let (Some(dir), Some(stem)) = (source.parent(), source.file_stem()) else {
        return Vec::new();
    };
    let filed = dir.join(stem).join("subagents");
    let entries = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    };
    let mut files = entries(&filed);
    for run in entries(&filed.join("workflows")) {
        files.extend(entries(&run));
    }
    let mut found: Vec<_> = files.iter().filter_map(|path| subagent(path)).collect();
    found.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    found
}

/// The id of the subagent whose transcript `path` is:
/// `<session>/subagents/agent-<id>.jsonl`, or a workflow run's
/// `<session>/subagents/workflows/<run>/agent-<id>.jsonl` (Magpie `SubagentOf`).
fn subagent_id(path: &Path) -> Option<&str> {
    let dir = path.parent()?;
    let named = |p: Option<&Path>, name: &str| p.and_then(Path::file_name) == Some(name.as_ref());
    let workflow = dir.parent();
    let filed = named(Some(dir), "subagents")
        || named(workflow, "workflows") && named(workflow.and_then(Path::parent), "subagents");
    if !filed {
        return None;
    }
    path.file_name()?
        .to_str()?
        .strip_suffix(".jsonl")?
        .strip_prefix("agent-")
        .filter(|id| !id.is_empty())
}

/// A file's length and modification time, or none when it is not there.
type Stamp = Option<(u64, Option<SystemTime>)>;

fn stamp_of(path: &Path) -> Stamp {
    let file = std::fs::metadata(path).ok()?;
    Some((file.len(), file.modified().ok()))
}

/// By transcript: its stamp and its meta file's, and what they said.
type SubagentCache = HashMap<PathBuf, (Stamp, Stamp, Subagent)>;

/// Subagents read before, by file: read again only once the file changes.
static SUBAGENTS: LazyLock<Mutex<SubagentCache>> = LazyLock::new(Default::default);

/// One subagent: kind, parent and title from the `agent-<id>.meta.json`
/// Claude Code writes beside it, else the title from the first thing it was
/// asked. A compaction's own run (`agent-acompact-<id>`) is no subagent.
fn subagent(path: &Path) -> Option<Subagent> {
    let id = subagent_id(path)?;
    if id.starts_with("acompact-") {
        return None;
    }
    let meta_path = path.with_extension("meta.json");
    let stamps = (stamp_of(path)?, stamp_of(&meta_path));
    if let Some((transcript, meta, found)) = SUBAGENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
    {
        if (*transcript, *meta) == (Some(stamps.0), stamps.1) {
            return Some(found.clone());
        }
    }
    let meta = std::fs::read(&meta_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or_default();
    let field = |key: &str, max: usize| {
        meta.get(key)
            .and_then(Value::as_str)
            .map(|text| text.chars().take(max).collect::<String>())
            .filter(|text| !text.is_empty())
    };
    let (head, tail) = read_head_tail_lines(path, 16, 16).unwrap_or_default();
    let lines = |lines: &[String]| -> Vec<Value> {
        lines
            .iter()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    };
    let (head, tail) = (lines(&head), lines(&tail));
    let at = |value: &Value| value.get("timestamp").and_then(parse_timestamp_to_ms);
    let task = head.iter().find_map(|value| {
        let user = value.get("type").and_then(Value::as_str) == Some("user")
            && value.get("isMeta").and_then(Value::as_bool) != Some(true);
        let text = value.get("message")?.get("content").and_then(user_text);
        Some(untagged(&text.filter(|_| user)?)).filter(|t| !t.is_empty())
    });
    let found = Subagent {
        id: id.to_owned(),
        title: field("description", usize::MAX)
            .map(|d| one_line(&d))
            .filter(|d| !d.is_empty())
            .or(task),
        kind: field("agentType", 128),
        parent: field("parentAgentId", 128),
        started_at: head.iter().find_map(at),
        last_active_at: tail.iter().rev().find_map(at),
        source: path.to_string_lossy().into_owned(),
    };
    SUBAGENTS.lock().unwrap_or_else(|e| e.into_inner()).insert(
        path.to_path_buf(),
        (Some(stamps.0), stamps.1, found.clone()),
    );
    Some(found)
}

fn last_text(slot: &mut Option<String>, value: Option<&Value>) {
    if let Some(text) = value
        .and_then(Value::as_str)
        .map(one_line)
        .filter(|t| !t.is_empty())
    {
        *slot = Some(text);
    }
}

/// The text of a user message, or `None` for a tool's result. Image blocks
/// are left out.
fn user_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn parse_session(path: &Path) -> Option<SessionMeta> {
    if is_agent_session(path) {
        return None;
    }

    let (head, tail) = read_head_tail_lines(path, 10, 30).ok()?;

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;

    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if session_id.is_none() {
            session_id = value
                .get("sessionId")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
        }
        if project_dir.is_none() {
            project_dir = value
                .get("cwd")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
        }
        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if session_id.is_some() && project_dir.is_some() && created_at.is_some() {
            break;
        }
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
        if summary.is_none() {
            if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if let Some(message) = value.get("message") {
                let text = message.get("content").map(extract_text).unwrap_or_default();
                if !text.trim().is_empty() {
                    summary = Some(text);
                }
            }
        }
        if last_active_at.is_some() && summary.is_some() {
            break;
        }
    }

    // The head and tail are all that is read here; the session listing reads
    // the whole file once per change (meta cache) for the full candidates.
    let mut scan = ClaudeScan::default();
    for line in head.iter().chain(&tail) {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            scan.line(&value);
        }
    }

    let session_id = session_id.or_else(|| infer_session_id_from_filename(path));
    let session_id = session_id?;

    let (title, title_source) = titled(&scan.candidates.with_directory(project_dir.as_deref()));

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
        resume_command: Some(format!("claude --resume {session_id}")),
        title_source,
        message_count: None,
        parent_session_id: None,
    })
}

fn is_agent_session(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with("agent-"))
        .unwrap_or(false)
}

fn infer_session_id_from_filename(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_string())
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
            && path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
        {
            files.push(path);
        }
    }
}

#[cfg(test)]
fn remove_path_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::metadata(path) {
        Ok(meta) => {
            if meta.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::title::TitleSource;
    use tempfile::tempdir;

    fn write_session(dir: &Path, lines: &[serde_json::Value]) -> PathBuf {
        let path = dir.join("11111111-2222-3333-4444-555555555555.jsonl");
        let body = lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>();
        std::fs::write(&path, body).expect("write session");
        path
    }

    fn user(content: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": content},
            "sessionId": "11111111-2222-3333-4444-555555555555",
            "cwd": "/work/app",
            "timestamp": "2026-10-14T10:00:00Z",
        })
    }

    fn title_of(lines: &[serde_json::Value]) -> Option<String> {
        let temp = tempdir().expect("tempdir");
        parse_session(&write_session(temp.path(), lines)).and_then(|meta| meta.title)
    }

    #[test]
    fn mixed_tool_results_keep_the_users_words() {
        let temp = tempdir().unwrap();
        let path = write_session(
            temp.path(),
            &[user(serde_json::json!([
                {"type": "tool_result", "content": "done"},
                {"type": "text", "text": "Please continue"}
            ]))],
        );
        let details = details(&path).unwrap();
        assert_eq!(details.message_count, 1);
        assert_eq!(
            details.candidates.prompt.as_deref(),
            Some("Please continue")
        );
    }

    #[test]
    fn metadata_accepts_json_whitespace() {
        let temp = tempdir().unwrap();
        let path = write_session(temp.path(), &[user(serde_json::json!("A real question"))]);
        let body = std::fs::read_to_string(&path).unwrap().replace(":", ": ");
        std::fs::write(&path, body).unwrap();
        let details = details(&path).unwrap();
        assert_eq!(details.message_count, 1);
        assert_eq!(
            details.candidates.prompt.as_deref(),
            Some("A real question")
        );
    }

    #[test]
    fn the_title_claude_code_made_beats_the_first_prompt() {
        let title = title_of(&[
            user(serde_json::json!("fix the login")),
            serde_json::json!({"type": "ai-title", "aiTitle": "Old name", "sessionId": "s"}),
            serde_json::json!({"type": "ai-title", "aiTitle": "Fix login bug", "sessionId": "s"}),
        ]);
        assert_eq!(title.as_deref(), Some("Fix login bug"));
    }

    #[test]
    fn a_summary_names_the_session_only_when_claude_made_no_title() {
        let summary =
            serde_json::json!({"type": "summary", "summary": "Login summary", "leafUuid": "u"});
        let ai =
            serde_json::json!({"type": "ai-title", "aiTitle": "Fix login bug", "sessionId": "s"});
        let prompt = user(serde_json::json!("fix the login"));
        assert_eq!(
            title_of(&[summary.clone(), prompt.clone()]).as_deref(),
            Some("Login summary")
        );
        assert_eq!(
            title_of(&[summary, prompt, ai]).as_deref(),
            Some("Fix login bug")
        );
    }

    #[test]
    fn a_slash_command_is_titled_by_its_name_and_args() {
        let title = title_of(&[
            user(serde_json::json!("<command-message>review</command-message>\n<command-name>/review</command-name>\n<command-args>42</command-args>")),
            user(serde_json::json!("later prompt")),
        ]);
        assert_eq!(title.as_deref(), Some("/review 42"));
    }

    #[test]
    fn images_meta_lines_and_interruptions_stay_out_of_the_title() {
        let mut meta =
            user(serde_json::json!([{"type": "text", "text": "[Image: source: /tmp/1.png]"}]));
        meta["isMeta"] = serde_json::json!(true);
        let title = title_of(&[
            meta,
            user(serde_json::json!("[Request interrupted by user]")),
            user(serde_json::json!([
                {"type": "text", "text": "look at this [Image #24]"},
                {"type": "image", "source": {"type": "base64", "data": "AAAA"}}
            ])),
        ]);
        assert_eq!(title.as_deref(), Some("look at this"));
    }

    #[test]
    fn a_session_with_only_tagged_messages_is_titled_by_their_words() {
        let title = title_of(&[user(serde_json::json!(
            "<task-notification>\n<summary>Agent finished</summary>\n</task-notification>"
        ))]);
        assert_eq!(title.as_deref(), Some("app"));
    }

    fn assistant(content: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": content},
            "timestamp": "2026-10-14T10:00:01Z",
        })
    }

    fn roles_and_texts(lines: &[serde_json::Value]) -> Vec<(String, String)> {
        let temp = tempdir().expect("tempdir");
        load_messages(&write_session(temp.path(), lines))
            .expect("load")
            .into_iter()
            .map(|m| (m.role, m.content))
            .collect()
    }

    fn pair(role: &str, text: &str) -> (String, String) {
        (role.to_string(), text.to_string())
    }

    #[test]
    fn transcript_shows_slash_commands_and_their_output_without_tags() {
        let mut meta_caveat = user(serde_json::json!(
            "<local-command-caveat>Caveat: The messages below were generated by the user while running local commands.</local-command-caveat>"
        ));
        meta_caveat["isMeta"] = serde_json::json!(true);
        let messages = roles_and_texts(&[
            meta_caveat,
            user(serde_json::json!(
                "<local-command-caveat>Caveat: The messages below were generated by the user while running local commands.</local-command-caveat>"
            )),
            user(serde_json::json!(
                "<command-name>/clear</command-name>\n            <command-message>clear</command-message>\n            <command-args></command-args>"
            )),
            user(serde_json::json!("<local-command-stdout></local-command-stdout>")),
            user(serde_json::json!(
                "<command-message>model</command-message>\n<command-name>/model</command-name>\n<command-args>opus</command-args>"
            )),
            user(serde_json::json!(
                "<local-command-stdout>Set model to Opus</local-command-stdout>"
            )),
        ]);
        assert_eq!(
            messages,
            vec![
                pair("command", "/clear"),
                pair("command", "/model opus"),
                pair("command_output", "Set model to Opus"),
            ]
        );
    }

    #[test]
    fn transcript_strips_reminders_and_drops_empty_thinking() {
        let messages = roles_and_texts(&[
            user(serde_json::json!([
                {"type": "text", "text": "<system-reminder>\nThe date is 2026-10-14.\n</system-reminder>"},
                {"type": "text", "text": "fix the bug"}
            ])),
            user(serde_json::json!(
                "<system-reminder>\nBackground review found 2 issues\n</system-reminder>"
            )),
            assistant(serde_json::json!([{"type": "thinking", "thinking": "", "signature": "s"}])),
            assistant(serde_json::json!([{"type": "redacted_thinking", "data": "opaque"}])),
            assistant(
                serde_json::json!([{"type": "thinking", "thinking": "look at main.rs first"}]),
            ),
            assistant(serde_json::json!([
                {"type": "text", "text": "Reading it."},
                {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"file_path": "src/main.rs"}}
            ])),
            user(serde_json::json!([
                {"type": "tool_result", "tool_use_id": "toolu_1", "content": "fn main() {}"}
            ])),
        ]);
        assert_eq!(
            messages,
            vec![
                pair("user", "fix the bug"),
                pair("system", "Background review found 2 issues"),
                pair("thinking", "look at main.rs first"),
                pair("assistant", "Reading it."),
                pair("tool_call", "{\n  \"file_path\": \"src/main.rs\"\n}"),
                pair("tool", "fn main() {}"),
            ]
        );
    }

    #[test]
    fn delete_session_removes_main_file_and_sidecar_directory() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("abc123-session.jsonl");
        let sidecar = temp.path().join("abc123-session");
        let subagents = sidecar.join("subagents");
        let tool_results = sidecar.join("tool-results");

        std::fs::create_dir_all(&subagents).expect("create subagents");
        std::fs::create_dir_all(&tool_results).expect("create tool-results");
        std::fs::write(subagents.join("agent-1.jsonl"), "{}").expect("write subagent");
        std::fs::write(tool_results.join("tool-1.txt"), "result").expect("write tool result");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"session-123\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n"
            ),
        )
        .expect("write session");

        delete_session(temp.path(), &path, "session-123").expect("delete session");

        assert!(!path.exists());
        assert!(!sidecar.exists());
    }

    #[test]
    fn load_messages_tool_use_is_a_named_tool_call_and_its_result_is_named_too() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Write\",\"input\":{\"file_path\":\"a.txt\"}}]},\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_1\",\"content\":\"File written\"}]},\"timestamp\":\"2026-03-06T10:00:01Z\"}\n",
            ),
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "tool_call");
        assert_eq!(msgs[0].name.as_deref(), Some("Write"));
        assert!(msgs[0].content.contains("\"file_path\": \"a.txt\""));
        assert_eq!(msgs[1].role, "tool");
        assert_eq!(msgs[1].name.as_deref(), Some("Write"));
        assert_eq!(msgs[1].content, "File written");
    }

    #[test]
    fn load_messages_mixed_text_and_tool_use() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            "{\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Let me help.\"},{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Read\",\"input\":{}}]},\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "assistant");
        assert_eq!(msgs[0].content, "Let me help.");
        assert_eq!(msgs[1].role, "tool_call");
        assert_eq!(msgs[1].name.as_deref(), Some("Read"));
    }

    #[test]
    fn load_messages_mixed_user_tool_result_and_text_splits_into_tool_and_user() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            "{\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_1\",\"content\":\"result\"},{\"type\":\"text\",\"text\":\"Please continue\"}]},\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "tool");
        assert_eq!(msgs[0].content, "result");
        assert_eq!(msgs[1].role, "user");
        assert_eq!(msgs[1].content, "Please continue");
    }

    #[test]
    fn parse_session_uses_first_user_message_as_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"session-abc\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"How do I deploy?\"},\"sessionId\":\"session-abc\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"Here is how...\"},\"timestamp\":\"2026-03-06T10:02:00Z\"}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("How do I deploy?"));
    }

    #[test]
    fn parse_session_custom_title_overrides_first_message() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-def.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"session-def\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"fix something\"},\"sessionId\":\"session-def\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"Done.\"},\"timestamp\":\"2026-03-06T10:02:00Z\"}\n",
                "{\"type\":\"custom-title\",\"customTitle\":\"fix-login-bug\",\"sessionId\":\"session-def\"}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("fix-login-bug"));
        assert_eq!(meta.title_source, Some(TitleSource::Custom));
    }

    #[test]
    fn parse_session_falls_back_to_dir_basename() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-ghi.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"session-ghi\",\"cwd\":\"/tmp/my-project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        // No user message and no custom-title → falls back to dir basename
        assert_eq!(meta.title.as_deref(), Some("my-project"));
        assert_eq!(meta.title_source, Some(TitleSource::Directory));
    }

    #[test]
    fn parse_session_truncates_long_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-trunc.jsonl");
        let long_msg = "a".repeat(200);
        std::fs::write(
            &path,
            format!(
                "{{\"sessionId\":\"session-trunc\",\"cwd\":\"/tmp/p\",\"timestamp\":\"2026-03-06T10:00:00Z\"}}\n\
                 {{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{long_msg}\"}},\"sessionId\":\"session-trunc\",\"timestamp\":\"2026-03-06T10:01:00Z\"}}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        let title = meta.title.unwrap();
        assert_eq!(title.chars().count(), 161);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn parse_session_new_format_with_snapshot() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-new.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"file-history-snapshot\",\"messageId\":\"msg-1\",\"snapshot\":{},\"isSnapshotUpdate\":false}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"请帮我重构这个函数\"},\"sessionId\":\"session-new\",\"timestamp\":\"2026-03-06T10:00:00Z\",\"cwd\":\"/tmp/project\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"OK\"},\"timestamp\":\"2026-03-06T10:01:00Z\",\"cwd\":\"/tmp/project\"}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("请帮我重构这个函数"));
    }

    #[test]
    fn parse_session_skips_command_caveat_and_titles_by_the_slash_command() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-clear.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"file-history-snapshot\",\"messageId\":\"msg-1\",\"snapshot\":{},\"isSnapshotUpdate\":false}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<local-command-caveat>Caveat: The messages below were generated by the user while running local commands.</local-command-caveat>\"},\"sessionId\":\"session-clear\",\"timestamp\":\"2026-03-06T10:00:00Z\",\"cwd\":\"/tmp/project\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/clear</command-name>\\n<command-message>clear</command-message>\"},\"sessionId\":\"session-clear\",\"timestamp\":\"2026-03-06T10:00:01Z\",\"cwd\":\"/tmp/project\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"Done.\"},\"timestamp\":\"2026-03-06T10:00:02Z\",\"cwd\":\"/tmp/project\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"帮我看看工作区的改动\"},\"sessionId\":\"session-clear\",\"timestamp\":\"2026-03-06T10:01:00Z\",\"cwd\":\"/tmp/project\"}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        // Magpie's ccPrompt: a slash command someone typed is the first prompt
        assert_eq!(meta.title.as_deref(), Some("/clear"));
    }
}
