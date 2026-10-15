//! OpenCode metadata/messages and usage read in one read-only native snapshot.
//! Token semantics belong to session_usage_opencode, never a second parser.
use super::{
    session_analytics::SessionDescriptor,
    session_analytics_core::{MessageEvent, Project, Role, SessionIdentity, UsageFact},
    session_analytics_index::{key, project_key, ParsedSource},
    session_usage_opencode, usage_source,
};
use crate::error::AppError;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
fn error(code: &str) -> AppError {
    AppError::Database(format!("usage.analytics.{code}"))
}
const MAX_ROWS: usize = 100_000;
const MAX_BYTES: usize = 64 * 1024 * 1024;
struct Scan {
    sessions: BTreeMap<String, SessionDescriptor>,
    facts: Vec<UsageFact>,
    messages: Vec<MessageEvent>,
    reasons: BTreeSet<String>,
    bytes: usize,
}
impl Scan {
    fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            facts: vec![],
            messages: vec![],
            reasons: BTreeSet::new(),
            bytes: 0,
        }
    }
    fn budget(&mut self, bytes: usize) -> Result<(), AppError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| error("sourceBudgetExceeded"))?;
        if self.bytes > MAX_BYTES
            || self
                .sessions
                .len()
                .max(self.facts.len())
                .max(self.messages.len())
                > MAX_ROWS
        {
            return Err(error("sourceBudgetExceeded"));
        }
        Ok(())
    }
    fn session(&mut self, value: &Value, reference: String) -> Result<(), AppError> {
        let Some(id) = value["id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 512)
        else {
            self.reasons.insert("sessionIdentityUnavailable".into());
            return Ok(());
        };
        let path = value
            .get("directory")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let project = match value.get("directory") {
            Some(Value::String(p)) if !p.trim().is_empty() => Project::Known(project_key(p)),
            Some(Value::String(_) | Value::Null) => Project::Unknown,
            _ => {
                self.reasons.insert("projectNotRecorded".into());
                Project::Unparsed
            }
        };
        let parent = value
            .get("parentID")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let title = value["title"]
            .as_str()
            .map(|s| crate::session_manager::title::one_line(&crate::redact::scrub(s)))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| id.into());
        let meta = crate::session_manager::SessionMeta {
            provider_id: "opencode".into(),
            session_id: id.into(),
            source_path: Some(reference),
            ..Default::default()
        };
        let descriptor = SessionDescriptor {
            identity: SessionIdentity {
                key: key("opencode", id),
                tool_id: "opencode".into(),
                project,
                verified_root_key: parent.map(|id| key("opencode", id)),
            },
            native_session_id: id.into(),
            title,
            project_label: path.as_ref().map(|p| {
                Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.clone())
            }),
            project_path: path,
            manager_key: Some(crate::session_manager::managed::key(&meta)),
            source_available: true,
            messages_supported: true,
            message_reasons: vec![],
        };
        if self.sessions.insert(id.into(), descriptor).is_some() {
            return Err(error("conflictingSessionMetadata"));
        }
        Ok(())
    }
    fn message(
        &mut self,
        mut value: Value,
        id: &str,
        sid: &str,
        content: Option<&Value>,
    ) -> Result<(), AppError> {
        value["id"] = json!(id);
        value["sessionID"] = json!(sid);
        if let Some(fact) =
            session_usage_opencode::normalized_message(value.clone(), !id.is_empty())?
        {
            if let Some(at_ms) = fact.at_ms {
                self.facts.push(UsageFact {
                    event_key: key(
                        "opencode",
                        fact.native_key.as_deref().unwrap_or(&fact.request_id),
                    ),
                    session_key: fact.session_id.as_deref().map(|id| key("opencode", id)),
                    tool_id: "opencode".into(),
                    source_id: "opencode_session".into(),
                    model: fact.model,
                    at_ms,
                    buckets: [
                        fact.usage.input_tokens,
                        fact.usage.output_tokens,
                        fact.usage.cache_read_tokens,
                        fact.usage.cache_creation_tokens,
                    ]
                    .map(u64::from),
                    complete: true,
                });
            } else {
                self.reasons.insert("missingTimestamp".into());
            }
        }
        let Some(role) = value["role"]
            .as_str()
            .filter(|r| matches!(*r, "user" | "assistant"))
        else {
            return Ok(());
        };
        let Some(content) = content else {
            if let Some(session) = self.sessions.get_mut(sid) {
                session.messages_supported = false;
                session
                    .message_reasons
                    .push("messagePartsUnavailable".into());
            }
            self.reasons.insert("messagePartsUnavailable".into());
            return Ok(());
        };
        if !super::session_analytics_messages::visible(Some(content), role) {
            return Ok(());
        }
        let at = value
            .pointer("/time/created")
            .and_then(Value::as_i64)
            .filter(|v| *v > 0);
        let Some(at_ms) = at else {
            if let Some(s) = self.sessions.get_mut(sid) {
                s.message_reasons.push("messageTimeUnavailable".into());
            }
            self.reasons.insert("messageTimeUnavailable".into());
            return Ok(());
        };
        self.messages.push(MessageEvent {
            session_key: key("opencode", sid),
            native_id: id.into(),
            at_ms,
            role: if role == "user" {
                Role::User
            } else {
                Role::Assistant
            },
            main: self
                .sessions
                .get(sid)
                .is_some_and(|s| s.identity.verified_root_key.is_none()),
        });
        Ok(())
    }
    fn finish(self) -> ParsedSource {
        ParsedSource {
            sessions: self.sessions.into_values().collect(),
            facts: self.facts,
            messages: self.messages,
            reasons: self.reasons.into_iter().collect(),
        }
    }
}
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id.len() <= 512
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}
fn storage_json(root: &Path) -> Result<Vec<std::path::PathBuf>, AppError> {
    fn walk(
        path: &Path,
        depth: usize,
        files: &mut Vec<std::path::PathBuf>,
    ) -> Result<(), AppError> {
        if depth > 8 || files.len() > MAX_ROWS {
            return Err(error("sourceBudgetExceeded"));
        }
        let meta = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(error("sourceReadFailed")),
        };
        if meta.file_type().is_symlink() {
            return Err(error("sourceSymlinkUnsupported"));
        }
        if meta.is_dir() {
            for e in std::fs::read_dir(path).map_err(|_| error("sourceReadFailed"))? {
                walk(
                    &e.map_err(|_| error("sourceReadFailed"))?.path(),
                    depth + 1,
                    files,
                )?;
            }
        } else if meta.is_file() && path.extension().is_some_and(|e| e == "json") {
            files.push(path.to_owned());
        }
        Ok(())
    }
    let mut files = vec![];
    walk(root, 0, &mut files)?;
    files.sort();
    Ok(files)
}
pub(crate) fn read(path: &Path) -> Result<ParsedSource, AppError> {
    let mut scan = Scan::new();
    if path.is_file() {
        let conn = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| error("sourceReadFailed"))?;
        let before: i64 = conn.query_row("PRAGMA data_version", [], |r| r.get(0))?;
        let tx = conn.unchecked_transaction()?;
        {
            let mut statement =
                tx.prepare("SELECT id,parent_id,title,directory FROM session ORDER BY id")?;
            let rows = statement.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?;
            for row in rows {
                let (id, parent, title, directory) = row?;
                scan.budget(
                    id.len()
                        + parent.as_ref().map_or(0, String::len)
                        + title.as_ref().map_or(0, String::len)
                        + directory.as_ref().map_or(0, String::len),
                )?;
                scan.session(
                    &json!({"id":id,"parentID":parent,"title":title,"directory":directory}),
                    format!("sqlite:{}:{id}", path.display()),
                )?;
            }
        }
        let has_parts: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='part')",
            [],
            |r| r.get(0),
        )?;
        let mut parts: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        if has_parts {
            let mut statement = tx.prepare("SELECT message_id,data FROM part ORDER BY id")?;
            let rows = statement
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (id, data) = row?;
                scan.budget(data.len())?;
                let value: Value =
                    serde_json::from_str(&data).map_err(|_| error("sourceReadFailed"))?;
                parts.entry(id).or_default().push(value);
            }
        }
        {
            let mut statement = tx.prepare("SELECT id,session_id,data FROM message ORDER BY id")?;
            let rows = statement.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (id, sid, data) = row?;
                scan.budget(data.len())?;
                let value: Value =
                    serde_json::from_str(&data).map_err(|_| error("sourceReadFailed"))?;
                let content = parts
                    .remove(&id)
                    .map(Value::Array)
                    .or_else(|| has_parts.then(|| json!([])));
                scan.message(value, &id, &sid, content.as_ref())?;
            }
        }
        tx.commit()?;
        let after: i64 = conn.query_row("PRAGMA data_version", [], |r| r.get(0))?;
        if before != after {
            return Err(error("sourceChangedRetry"));
        }
    } else {
        for file in storage_json(&path.join("session"))? {
            let data = usage_source::read_text(&file)?;
            scan.budget(data.len())?;
            let value: Value =
                serde_json::from_str(&data).map_err(|_| error("sourceReadFailed"))?;
            let id = value["id"]
                .as_str()
                .filter(|id| safe_id(id))
                .ok_or_else(|| error("invalidNativeIdentity"))?;
            scan.session(
                &value,
                path.join("message").join(id).to_string_lossy().into_owned(),
            )?;
        }
        for file in storage_json(&path.join("message"))? {
            let data = usage_source::read_text(&file)?;
            scan.budget(data.len())?;
            let value: Value =
                serde_json::from_str(&data).map_err(|_| error("sourceReadFailed"))?;
            let native = value["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let sid = value["sessionID"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    scan.reasons.insert("sessionIdentityUnavailable".into());
                    String::new()
                });
            let Some(id) = native else {
                // A copied file is not evidence of a second message or call.
                // Preserve the session with unknown message coverage instead.
                scan.reasons.insert("missingNativeEventIdentity".into());
                if let Some(session) = scan.sessions.get_mut(&sid) {
                    session.messages_supported = false;
                    session
                        .message_reasons
                        .push("messageNativeIdentityUnavailable".into());
                }
                continue;
            };
            let parts = if let Some(parts) = value.get("parts") {
                Some(parts.clone())
            } else {
                let mut parts = vec![];
                if !safe_id(&id) {
                    return Err(error("invalidNativeIdentity"));
                }
                let dir = path.join("part").join(&id);
                for file in storage_json(&dir)? {
                    let data = usage_source::read_text(&file)?;
                    scan.budget(data.len())?;
                    parts.push(
                        serde_json::from_str::<Value>(&data)
                            .map_err(|_| error("sourceReadFailed"))?,
                    );
                }
                if dir.exists() {
                    Some(Value::Array(parts))
                } else {
                    None
                }
            };
            scan.message(value, &id, &sid, parts.as_ref())?;
        }
    }
    Ok(scan.finish())
}
