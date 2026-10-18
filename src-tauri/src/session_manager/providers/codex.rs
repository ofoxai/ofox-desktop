use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codex_config::get_codex_config_dir;
use crate::session_manager::meta_cache::SessionDetails;
use crate::session_manager::title::{clean_prompt, one_line, titled, untagged, TitleCandidates};
use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{
    extract_text, for_each_line, parse_timestamp_to_ms, read_head_tail_lines, truncate_summary,
};

const PROVIDER_ID: &str = "codex";

static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
        .unwrap()
});

pub fn scan_sessions() -> Vec<SessionMeta> {
    let home = get_codex_config_dir();
    let mut sessions = scan_in(&home.join("sessions"));
    sessions.extend(scan_in(&home.join("archived_sessions")));
    sessions
}

pub(crate) fn is_archived(meta: &SessionMeta) -> bool {
    meta.provider_id == PROVIDER_ID
        && meta.source_path.as_deref().is_some_and(|path| {
            Path::new(path).starts_with(get_codex_config_dir().join("archived_sessions"))
        })
}

/// Conversation sources include native archives but do not grant any mutation
/// capability. A compressed mirror is represented by its plain rollout.
pub(crate) fn conversation_paths(meta: &SessionMeta) -> Result<Vec<PathBuf>, String> {
    let mut paths: Vec<_> = crate::session_manager::managed::cached_listing()?
        .iter()
        .filter(|record| record.provider_id == PROVIDER_ID && record.session_id == meta.session_id)
        .filter_map(|record| record.source_path.as_deref().map(PathBuf::from))
        .collect();
    if paths.is_empty() {
        return Err("session.search.changed".into());
    }
    paths.sort_by(|left, right| {
        left.file_name()
            .cmp(&right.file_name())
            .then_with(|| left.cmp(right))
    });
    paths.dedup();
    let plain: std::collections::HashSet<_> = paths
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .cloned()
        .collect();
    paths.retain(|path| {
        !path
            .extension()
            .is_some_and(|ext| ext == "zst" || ext == "zstd")
            || !plain.contains(&path.with_extension(""))
    });
    for path in &paths {
        crate::session_manager::managed::validate_path(
            path,
            &crate::session_manager::managed::roots(PROVIDER_ID)?,
        )?;
    }
    Ok(paths)
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

/// The thread names of the Codex home in use.
pub(crate) fn current_thread_names() -> ThreadNames {
    thread_names(&get_codex_config_dir().join("session_index.jsonl"))
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

/// A thread's parent, adapted from Magpie internal/sessions/codex.go
/// (`codexIdentityFromMeta`, `codexSource`; MIT). The spawn in `source` names
/// it, else the top-level `parent_thread_id`. When the two disagree, or a main
/// thread's source still names a parent, the thread's place is unknown and it
/// has none. `forked_from_id` never makes a parent.
pub(crate) fn codex_parent(payload: &Value) -> Option<String> {
    let top = payload
        .get("parent_thread_id")
        .and_then(Value::as_str)
        .filter(|parent| !parent.is_empty());
    let (main, spawn) = codex_source(payload.get("source"));
    if spawn.is_some() && top.is_some() && spawn != top || main && top.is_some() {
        return None;
    }
    spawn.or(top).map(str::to_owned)
}

/// Whether a session_meta `source` is a main thread's, and the parent its
/// `subagent.thread_spawn` names.
fn codex_source(source: Option<&Value>) -> (bool, Option<&str>) {
    match source {
        Some(Value::String(name)) => (
            matches!(name.as_str(), "cli" | "vscode" | "exec" | "mcp"),
            None,
        ),
        Some(Value::Object(variants)) if variants.len() == 1 => {
            if let Some(custom) = variants.get("custom") {
                return (custom.as_str().is_some_and(|c| !c.is_empty()), None);
            }
            let parent = variants
                .get("subagent")
                .and_then(Value::as_object)
                .filter(|kind| kind.len() == 1)
                .and_then(|kind| kind.get("thread_spawn"))
                .and_then(|spawn| spawn.get("parent_thread_id"))
                .and_then(Value::as_str)
                .filter(|parent| !parent.is_empty());
            (false, parent)
        }
        _ => (false, None),
    }
}

/// The threads a rollout's own `spawn_agent` calls started and the task name
/// each call gave, by call id (Magpie `codexSubagentTaskTitles`, MIT). A call
/// id claimed twice with different values names nothing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CodexSpawns {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub children: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub conflicts: BTreeSet<String>,
}

impl CodexSpawns {
    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.children.is_empty() && self.conflicts.is_empty()
    }

    fn note(
        map: &mut BTreeMap<String, String>,
        conflicts: &mut BTreeSet<String>,
        call: &str,
        value: &str,
    ) {
        if call.is_empty() || value.is_empty() {
            return;
        }
        match map.get(call) {
            Some(old) if old != value => {
                conflicts.insert(call.to_owned());
            }
            _ => {
                map.insert(call.to_owned(), value.to_owned());
            }
        }
    }

    fn named(&mut self, call: &str, task: &str) {
        Self::note(&mut self.names, &mut self.conflicts, call, task);
    }

    fn started(&mut self, call: &str, child: &str) {
        Self::note(&mut self.children, &mut self.conflicts, call, child);
    }

    /// Fold in another rollout of the same thread.
    pub fn merge(&mut self, other: &CodexSpawns) {
        self.conflicts.extend(other.conflicts.iter().cloned());
        for (call, task) in &other.names {
            self.named(call, task);
        }
        for (call, child) in &other.children {
            self.started(call, child);
        }
    }

    /// Each started child thread and the task name its call gave.
    pub fn titles(&self) -> impl Iterator<Item = (&str, &str)> {
        self.names
            .iter()
            .filter(|(call, _)| !self.conflicts.contains(*call))
            .filter_map(|(call, task)| Some((self.children.get(call)?.as_str(), task.as_str())))
    }
}

/// What a Codex rollout's lines say about its title, length and the
/// subagents it spawned. Title rules adapted from Magpie
/// internal/sessions/codex.go (MIT).
#[derive(Debug, Default)]
pub(crate) struct CodexScan {
    pub candidates: TitleCandidates,
    prompts: u32,
    replies: u32,
    pub spawns: CodexSpawns,
}

impl CodexScan {
    pub(crate) fn line(&mut self, value: &Value) {
        let Some(payload) = value.get("payload") else {
            return;
        };
        let payload_type = payload.get("type").and_then(Value::as_str);
        match value.get("type").and_then(Value::as_str) {
            Some("response_item") if payload_type == Some("message") => {
                match payload.get("role").and_then(Value::as_str) {
                    Some("user") => {
                        let mut typed = false;
                        for text in user_texts(payload.get("content")) {
                            let by_hand = typed_by_hand(text);
                            typed |= by_hand;
                            if by_hand && self.candidates.prompt.is_none() {
                                self.candidates.prompt = clean_prompt(text);
                            }
                        }
                        if typed {
                            self.prompts += 1;
                        }
                    }
                    Some("assistant") => self.replies += 1,
                    _ => {}
                }
            }
            Some("event_msg") if payload_type == Some("user_message") => {
                let text = payload.get("message").and_then(Value::as_str).unwrap_or("");
                let candidates = &mut self.candidates;
                if candidates.prompt.is_none() && typed_by_hand(text) {
                    candidates.prompt = clean_prompt(text);
                }
                if candidates.prompt.is_none() && candidates.message.is_none() {
                    candidates.message = Some(untagged(text)).filter(|t| !t.is_empty());
                }
            }
            Some("response_item")
                if payload_type == Some("function_call")
                    && payload.get("name").and_then(Value::as_str) == Some("spawn_agent") =>
            {
                let arguments = match payload.get("arguments") {
                    Some(Value::String(text)) => serde_json::from_str(text).ok(),
                    other => other.cloned(),
                };
                let task = arguments
                    .as_ref()
                    .and_then(|a| a.get("task_name"))
                    .and_then(Value::as_str)
                    .map(one_line)
                    .unwrap_or_default();
                self.spawns.named(str_at(payload, "call_id"), &task);
            }
            Some("event_msg") if payload_type == Some("item_completed") => {
                let Some(item) = payload.get("item") else {
                    return;
                };
                if str_at(item, "type") == "SubAgentActivity" && str_at(item, "kind") == "started" {
                    self.spawns
                        .started(str_at(item, "id"), str_at(item, "agent_thread_id"));
                }
            }
            // a legacy rollout's started activity, its id the spawn call's
            Some("event_msg")
                if payload_type == Some("sub_agent_activity")
                    && str_at(payload, "kind") == "started" =>
            {
                self.spawns.started(
                    str_at(payload, "event_id"),
                    str_at(payload, "agent_thread_id"),
                );
            }
            _ => {}
        }
    }

    /// Typed prompts plus the agent's messages back.
    pub(crate) fn message_count(&self) -> u32 {
        self.prompts.saturating_add(self.replies)
    }
}

/// The whole rollout's title candidates and message count (the thread name
/// is not in the rollout: see `thread_names`).
pub(crate) fn details(path: &Path) -> Option<SessionDetails> {
    let mut scan = CodexScan::default();
    for_each_line(path, |line| {
        // Read discriminators without allocating potentially large content blocks.
        #[derive(serde::Deserialize)]
        struct Header<'a> {
            #[serde(rename = "type", borrow)]
            kind: Option<std::borrow::Cow<'a, str>>,
            #[serde(borrow)]
            payload: Option<Payload<'a>>,
        }
        #[derive(serde::Deserialize)]
        struct Payload<'a> {
            #[serde(rename = "type", borrow)]
            kind: Option<std::borrow::Cow<'a, str>>,
        }
        let Ok(header) = serde_json::from_str::<Header<'_>>(line) else {
            return;
        };
        let payload = header.payload.as_ref().and_then(|p| p.kind.as_deref());
        let wanted = match (header.kind.as_deref(), payload) {
            (Some("response_item"), Some("message"))
            | (Some("event_msg"), Some("user_message" | "sub_agent_activity")) => true,
            (Some("response_item"), Some("function_call")) => line.contains("\"spawn_agent\""),
            (Some("event_msg"), Some("item_completed")) => line.contains("SubAgentActivity"),
            _ => false,
        };
        if wanted {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                scan.line(&value);
            }
        }
    })
    .ok()?;
    Some(SessionDetails {
        message_count: scan.message_count(),
        candidates: scan.candidates,
        spawns: scan.spawns,
    })
}

/// Whether a user message's text is a prompt someone typed, for its title and
/// message count: one Codex put in itself starts with a tag or AGENTS.md
/// (Magpie `cxPrompt`, MIT).
fn typed_by_hand(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty() && !text.starts_with('<') && !text.starts_with("# AGENTS.md")
}

fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
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
    let mut parent_session_id: Option<String> = None;
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
                // a subagent's rollout repeats its parent's meta after its own
                if session_id.is_none() {
                    session_id = payload
                        .get("id")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                    parent_session_id = codex_parent(payload);
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
        message_count: None,
        parent_session_id,
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
    fn metadata_accepts_json_whitespace() {
        let temp = tempdir().unwrap();
        let path = rollout(temp.path(), &[user_items(&[input_text("A real question")])]);
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

    /// Magpie titles a thread by the first text that does not start with `<`
    /// (`cxPrompt`, MIT): a wrapped question reply is no prompt.
    #[test]
    fn a_wrapped_reply_does_not_title_a_thread() {
        let temp = tempdir().expect("tempdir");
        let path = rollout(
            temp.path(),
            &[
                serde_json::json!({"timestamp": "2026-10-14T10:00:01Z", "type": "event_msg",
                    "payload": {"type": "user_message", "message":
                        "<send_user_message_question_reply>[{\"questionItemId\":\"q1\"}]</send_user_message_question_reply>"}}),
                user_items(&[input_text(
                    "<send_user_message_question_reply>[{\"questionItemId\":\"q1\"}]</send_user_message_question_reply>",
                )]),
                user_items(&[input_text("# Files mentioned by the user: notes.md")]),
            ],
        );
        let details = details(&path).expect("details");
        assert_eq!(
            details.candidates.prompt.as_deref(),
            Some("# Files mentioned by the user: notes.md")
        );
    }

    /// Magpie counts a user message as typed only when one of its texts does
    /// not start with `<` or `# AGENTS.md` (`cxPrompt`, MIT).
    #[test]
    fn only_a_typed_prompt_counts_as_a_message() {
        let temp = tempdir().expect("tempdir");
        let path = rollout(
            temp.path(),
            &[
                user_items(&[input_text(
                    "<send_user_message_question_reply>[{\"questionItemId\":\"q1\"}]</send_user_message_question_reply>",
                )]),
                user_items(&[input_text(
                    "<codex_internal_context source=\"goal\">keep going</codex_internal_context>",
                )]),
                user_items(&[input_text("# AGENTS.md instructions for /work/app")]),
                user_items(&[
                    input_text("<image name=[Image #1] path=\"/tmp/a.png\">"),
                    input_text("</image>"),
                    input_text("make it green"),
                ]),
                item(serde_json::json!({"type": "message", "role": "assistant",
                    "content": [{"type": "output_text", "text": "Done."}]})),
            ],
        );
        assert_eq!(details(&path).expect("details").message_count, 2);
    }

    /// Cases from Magpie `TestCodexIdentityParentChainsAndSummaryCache` (MIT).
    #[test]
    fn a_thread_parent_comes_from_its_spawn_else_the_top_level_field() {
        let spawn = |parent: &str| serde_json::json!({"subagent": {"thread_spawn": {"parent_thread_id": parent, "depth": 1}}});
        let cases = [
            (
                "main",
                serde_json::json!({"id": "main-a", "source": "vscode"}),
                None,
            ),
            (
                "child",
                serde_json::json!({"id": "child-b", "source": spawn("main-a")}),
                Some("main-a"),
            ),
            (
                "nested",
                serde_json::json!({"id": "nested-c", "source": spawn("child-b"),
                "parent_thread_id": "child-b"}),
                Some("child-b"),
            ),
            (
                "fork",
                serde_json::json!({"id": "fork-f", "source": "vscode",
                "forked_from_id": "main-a"}),
                None,
            ),
            (
                "missing parent",
                serde_json::json!({"id": "orphan", "source": spawn("absent-parent")}),
                Some("absent-parent"),
            ),
            (
                "empty spawn parent",
                serde_json::json!({"id": "e", "source": spawn("")}),
                None,
            ),
            (
                "review subagent",
                serde_json::json!({"id": "r", "source": {"subagent": "review"},
                "parent_thread_id": "main-a"}),
                Some("main-a"),
            ),
            (
                "conflicting parents",
                serde_json::json!({"id": "x", "source": spawn("main-a"),
                "parent_thread_id": "other"}),
                None,
            ),
            (
                "main naming a parent",
                serde_json::json!({"id": "y", "source": "cli",
                "parent_thread_id": "main-a"}),
                None,
            ),
            (
                "custom naming a parent",
                serde_json::json!({"id": "z", "source": {"custom": "ide"},
                "parent_thread_id": "main-a"}),
                None,
            ),
            (
                "unknown source",
                serde_json::json!({"id": "u", "source": {"weird": {}},
                "parent_thread_id": "main-a"}),
                Some("main-a"),
            ),
            ("no source", serde_json::json!({"id": "n"}), None),
        ];
        for (name, payload, parent) in cases {
            assert_eq!(codex_parent(&payload).as_deref(), parent, "{name}");
        }
    }

    #[test]
    fn a_subagent_rollout_is_its_first_meta_not_the_parent_copy_after_it() {
        let temp = tempdir().expect("tempdir");
        let dir = temp.path().join("sessions/2026/10/14");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-2026-10-14T10-00-00-{THREAD}.jsonl"));
        let lines = [
            serde_json::json!({"timestamp": "2026-10-14T10:00:00Z", "type": "session_meta",
                "payload": {"id": THREAD, "session_id": "parent-p", "parent_thread_id": "parent-p",
                    "cwd": "/work/app",
                    "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent-p"}}}}}),
            serde_json::json!({"timestamp": "2026-10-13T09:00:00Z", "type": "session_meta",
                "payload": {"id": "parent-p", "session_id": "parent-p", "cwd": "/work/app",
                    "source": "vscode"}}),
        ];
        std::fs::write(
            &path,
            lines.iter().map(|l| format!("{l}\n")).collect::<String>(),
        )
        .unwrap();

        let meta = parse_session(&path).expect("parse");
        assert_eq!(meta.session_id, THREAD);
        assert_eq!(meta.parent_session_id.as_deref(), Some("parent-p"));
    }

    #[test]
    fn a_rollout_notes_the_threads_its_spawn_calls_started() {
        let temp = tempdir().expect("tempdir");
        let spawn = |call: &str, task: &str| {
            item(
                serde_json::json!({"type": "function_call", "name": "spawn_agent", "call_id": call,
                "arguments": serde_json::json!({"task_name": task, "message": "go"}).to_string()}),
            )
        };
        let started = |call: &str, child: &str| {
            serde_json::json!({"timestamp": "2026-10-14T10:00:04Z", "type": "event_msg",
                "payload": {"type": "item_completed", "item": {"type": "SubAgentActivity",
                    "id": call, "kind": "started", "agent_thread_id": child}}})
        };
        let path = rollout(
            temp.path(),
            &[
                spawn("c1", "  usage\n audit "),
                started("c1", "child-1"),
                serde_json::json!({"timestamp": "2026-10-14T10:00:05Z", "type": "event_msg",
                    "payload": {"type": "item_completed", "item": {"type": "SubAgentActivity",
                        "id": "c1", "kind": "completed", "agent_thread_id": "child-1"}}}),
                // a legacy rollout's started event
                spawn("c2", "bugbot"),
                serde_json::json!({"timestamp": "2026-10-14T10:00:06Z", "type": "event_msg",
                    "payload": {"type": "sub_agent_activity", "event_id": "c2", "kind": "started",
                        "agent_thread_id": "child-2"}}),
                // one call id named twice names nothing
                spawn("c3", "first"),
                spawn("c3", "second"),
                started("c3", "child-3"),
            ],
        );

        let spawns = details(&path).expect("details").spawns;
        let titles = spawns.titles().collect::<Vec<_>>();
        assert_eq!(
            titles,
            vec![("child-1", "usage audit"), ("child-2", "bugbot")]
        );
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
