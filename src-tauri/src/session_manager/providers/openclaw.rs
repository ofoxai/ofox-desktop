use std::collections::HashMap;

use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

#[cfg(test)]
use crate::config::write_json_file;
use crate::openclaw_config::get_openclaw_dir;
use crate::session_manager::title::{titled, TitleCandidates};
use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{extract_text, parse_timestamp_to_ms, read_head_tail_lines, truncate_summary};

const PROVIDER_ID: &str = "openclaw";

/// Native usage state root, independent of the configuration-file location.
pub(crate) fn usage_root() -> Result<std::path::PathBuf, String> {
    let _io = crate::config_io::lock();
    if crate::settings::settings_health().state == crate::settings::SettingsHealthState::Unreadable
    {
        return Err("usage.openclaw.settingsUnreadable".into());
    }
    if let Some(root) = crate::settings::get_openclaw_override_dir() {
        return Ok(root);
    }
    fn env(name: &str) -> Result<Option<String>, String> {
        match std::env::var(name) {
            Ok(value) => Ok((!value.trim().is_empty()).then(|| value.trim().to_owned())),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err("usage.openclaw.invalidStateDirectory".into()),
        }
    }
    fn absolute(value: &str, home: &Path) -> Result<std::path::PathBuf, String> {
        let path = if value == "~" {
            home.to_owned()
        } else if value.starts_with("~/") || value.starts_with("~\\") {
            home.join(&value[2..])
        } else {
            std::path::PathBuf::from(value)
        };
        // The desktop cannot recover a native CLI's invocation cwd. Relative
        // overrides are unknown, rather than permission to scan a default.
        path.is_absolute()
            .then_some(path)
            .ok_or_else(|| "usage.openclaw.invalidStateDirectory".into())
    }
    let os_home = crate::config::get_home_dir();
    let home = match env("OPENCLAW_HOME")?.filter(|v| !matches!(v.as_str(), "null" | "undefined")) {
        Some(value) => absolute(&value, &os_home)?,
        None => os_home,
    };
    if let Some(value) = env("OPENCLAW_STATE_DIR")? {
        return absolute(&value, &home);
    }
    let current = home.join(".openclaw");
    match std::fs::symlink_metadata(&current) {
        Ok(_) => return Ok(current),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("usage.openclaw.sourceUnavailable".into()),
    }
    let legacy = home.join(".clawdbot");
    match std::fs::symlink_metadata(&legacy) {
        Ok(_) => Ok(legacy),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(current),
        Err(_) => Err("usage.openclaw.sourceUnavailable".into()),
    }
}

/// Strict, stat-only roster used by local refresh and the native reader.
pub(crate) fn usage_files(root: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    fn kind(path: &Path) -> Result<Option<std::fs::Metadata>, String> {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                Err("usage.openclaw.sourceSymlinkUnsupported".into())
            }
            Ok(meta) => Ok(Some(meta)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("usage.openclaw.sourceUnavailable".into()),
        }
    }
    fn directory(path: &Path) -> Result<bool, String> {
        match kind(path)? {
            None => Ok(false),
            Some(m) if m.is_dir() => Ok(true),
            Some(_) => Err("usage.openclaw.sourceRootNotDirectory".into()),
        }
    }
    if !directory(root)? || !directory(&root.join("agents"))? {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut visited = 0;
    for agent in
        std::fs::read_dir(root.join("agents")).map_err(|_| "usage.openclaw.sourceUnavailable")?
    {
        visited += 1;
        if visited > 20_000 {
            return Err("usage.openclaw.sourceBudgetExceeded".into());
        }
        let agent = agent
            .map_err(|_| "usage.openclaw.sourceUnavailable")?
            .path();
        if !kind(&agent)?.is_some_and(|m| m.is_dir()) {
            continue;
        }
        let primary_dir = agent.join("agent");
        if directory(&primary_dir)? {
            let primary = primary_dir.join("openclaw-agent.sqlite");
            if let Some(meta) = kind(&primary)? {
                if !meta.is_file() {
                    return Err("usage.openclaw.sourceUnavailable".into());
                }
                // WAL is observed by normalized_stamp; reject redirected sidecars.
                for suffix in ["-wal", "-journal"] {
                    let _ = kind(&std::path::PathBuf::from(format!(
                        "{}{suffix}",
                        primary.to_string_lossy()
                    )))?;
                }
                out.push(primary);
            }
        }
        let sessions = agent.join("sessions");
        if !directory(&sessions)? {
            continue;
        }
        for entry in std::fs::read_dir(sessions).map_err(|_| "usage.openclaw.sourceUnavailable")? {
            visited += 1;
            if visited > 20_000 || out.len() >= 10_000 {
                return Err("usage.openclaw.sourceBudgetExceeded".into());
            }
            let path = entry
                .map_err(|_| "usage.openclaw.sourceUnavailable")?
                .path();
            if path.extension().is_some_and(|e| e == "jsonl") {
                if !kind(&path)?.is_some_and(|m| m.is_file()) {
                    return Err("usage.openclaw.sourceUnavailable".into());
                }
                out.push(path);
            }
        }
    }
    if out.len() > 10_000 {
        return Err("usage.openclaw.sourceBudgetExceeded".into());
    }
    out.sort();
    Ok(out)
}

/// Strip trailing `\n[message_id: ...]` metadata injected by OpenClaw gateway.
fn strip_message_id_suffix(text: &str) -> &str {
    if let Some(pos) = text.rfind("\n[message_id:") {
        text[..pos].trim_end()
    } else {
        text
    }
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    let agents_dir = get_openclaw_dir().join("agents");
    if !agents_dir.exists() {
        return Vec::new();
    }

    let mut sessions = Vec::new();

    // Traverse each agent directory
    let agent_entries = match std::fs::read_dir(&agents_dir) {
        Ok(entries) => entries,
        Err(_) => return sessions,
    };

    for agent_entry in agent_entries.flatten() {
        let agent_path = agent_entry.path();
        if !agent_entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }

        let sessions_dir = agent_path.join("sessions");
        if !std::fs::symlink_metadata(&sessions_dir)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            continue;
        }

        let session_entries = match std::fs::read_dir(&sessions_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        let display_names = load_display_names(&sessions_dir);

        for entry in session_entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }

            if let Some(meta) = parse_session(&path, Some(&display_names)) {
                sessions.push(meta);
            }
        }
    }

    sessions
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let file = crate::session_manager::read_context::open(path)?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        crate::session_manager::read_context::check()?;
        let line = line.map_err(|error| error.to_string())?;
        let value: Value = match serde_json::from_str(&line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if value.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }

        let message = match value.get("message") {
            Some(msg) => msg,
            None => continue,
        };

        let raw_role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        // Map OpenClaw roles to our standard roles
        let role = match raw_role {
            "toolResult" => "tool".to_string(),
            other => other.to_string(),
        };

        let content = message.get("content").map(extract_text).unwrap_or_default();
        if content.trim().is_empty() {
            continue;
        }

        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);

        messages.push(SessionMessage {
            role,
            content,
            ts,
            name: None,
        });
    }

    Ok(messages)
}

#[cfg(test)]
pub fn delete_session(_root: &Path, path: &Path, session_id: &str) -> Result<bool, String> {
    let meta = parse_session(path, None).ok_or_else(|| {
        format!(
            "Failed to parse OpenClaw session metadata: {}",
            path.display()
        )
    })?;

    if meta.session_id != session_id {
        return Err(format!(
            "OpenClaw session ID mismatch: expected {session_id}, found {}",
            meta.session_id
        ));
    }

    let index_path = path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("sessions.json");
    prune_sessions_index(&index_path, session_id, path)?;

    std::fs::remove_file(path).map_err(|e| {
        format!(
            "Failed to delete OpenClaw session file {}: {e}",
            path.display()
        )
    })?;

    Ok(true)
}

/// Read `sessions.json` index and build a sessionId → displayName lookup map.
/// Returns an empty map if the file does not exist or cannot be parsed.
fn load_display_names(sessions_dir: &Path) -> HashMap<String, String> {
    let index_path = sessions_dir.join("sessions.json");
    if !std::fs::symlink_metadata(&index_path)
        .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
    {
        return HashMap::new();
    }
    let content = match std::fs::read_to_string(&index_path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    let index: serde_json::Map<String, Value> = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(_) => return HashMap::new(),
    };

    let mut map = HashMap::new();
    for (_key, entry) in &index {
        if let (Some(id), Some(name)) = (
            entry.get("sessionId").and_then(Value::as_str),
            entry.get("displayName").and_then(Value::as_str),
        ) {
            if !name.is_empty() {
                map.insert(id.to_string(), name.to_string());
            }
        }
    }
    map
}

fn parse_session(
    path: &Path,
    display_names: Option<&HashMap<String, String>>,
) -> Option<SessionMeta> {
    let (head, tail) = read_head_tail_lines(path, 10, 30).ok()?;

    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut summary: Option<String> = None;
    let mut first_user_message: Option<String> = None;

    // Extract metadata, summary, and first user message from head lines
    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }

        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        if event_type == "session" {
            if session_id.is_none() {
                session_id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
            }
            if cwd.is_none() {
                cwd = value
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
            }
            if let Some(ts) = value.get("timestamp").and_then(parse_timestamp_to_ms) {
                created_at.get_or_insert(ts);
            }
            continue;
        }

        if event_type == "message" {
            if let Some(message) = value.get("message") {
                let text = message.get("content").map(extract_text).unwrap_or_default();
                let cleaned = strip_message_id_suffix(&text);
                if !cleaned.trim().is_empty() {
                    if first_user_message.is_none()
                        && message.get("role").and_then(Value::as_str) == Some("user")
                    {
                        first_user_message = Some(cleaned.trim().to_string());
                    }
                    if summary.is_none() {
                        summary = Some(cleaned.trim().to_string());
                    }
                }
            }
        }

        if session_id.is_some()
            && cwd.is_some()
            && created_at.is_some()
            && summary.is_some()
            && first_user_message.is_some()
        {
            break;
        }
    }

    // Extract last_active_at from tail lines (reverse order)
    let mut last_active_at: Option<i64> = None;
    for line in tail.iter().rev() {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if let Some(ts) = value.get("timestamp").and_then(parse_timestamp_to_ms) {
            last_active_at = Some(ts);
            break;
        }
    }

    // Fall back to filename as session ID
    let session_id = session_id.or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    });
    let session_id = session_id?;

    // Title priority: displayName (from sessions.json) > first user message > dir basename
    let mut candidates = TitleCandidates::default()
        .with_first_user_text(first_user_message.as_deref())
        .with_directory(cwd.as_deref());
    candidates.agent = display_names.and_then(|m| m.get(&session_id)).cloned();
    let (title, title_source) = titled(&candidates);

    let summary = summary.map(|text| truncate_summary(&text, 160));

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title,
        summary,
        project_dir: cwd,
        created_at,
        last_active_at,
        source_path: Some(path.to_string_lossy().to_string()),
        resume_command: None, // OpenClaw sessions are gateway-managed, no CLI resume
        title_source,
        message_count: None,
    })
}

#[cfg(test)]
fn prune_sessions_index(
    index_path: &Path,
    session_id: &str,
    source_path: &Path,
) -> Result<(), String> {
    if !index_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(index_path).map_err(|e| {
        format!(
            "Failed to read OpenClaw sessions index {}: {e}",
            index_path.display()
        )
    })?;
    let mut index: serde_json::Map<String, Value> =
        serde_json::from_str(&content).map_err(|e| {
            format!(
                "Failed to parse OpenClaw sessions index {}: {e}",
                index_path.display()
            )
        })?;

    let source = source_path.to_string_lossy();
    index.retain(|_, entry| {
        let same_id = entry.get("sessionId").and_then(Value::as_str) == Some(session_id);
        let same_file = entry.get("sessionFile").and_then(Value::as_str) == Some(source.as_ref());
        !(same_id || same_file)
    });

    write_json_file(index_path, &index).map_err(|e| {
        format!(
            "Failed to update OpenClaw sessions index {}: {e}",
            index_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_session_uses_first_user_message_as_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"session\",\"id\":\"session-abc\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"How do I deploy?\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":\"Here is how...\"},\"timestamp\":\"2026-03-06T10:02:00Z\"}\n"
            ),
        )
        .expect("write");

        let meta = parse_session(&path, None).unwrap();
        assert_eq!(meta.title.as_deref(), Some("How do I deploy?"));
    }

    #[test]
    fn parse_session_display_name_overrides_user_message() {
        let temp = tempdir().expect("tempdir");
        let sessions_dir = temp.path();

        let path = sessions_dir.join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"session\",\"id\":\"session-abc\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"fix something\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n"
            ),
        )
        .expect("write session");

        std::fs::write(
            sessions_dir.join("sessions.json"),
            r#"{
                "agent:main:main": {
                    "sessionId": "session-abc",
                    "displayName": "重构登录模块"
                }
            }"#,
        )
        .expect("write index");

        let display_names = load_display_names(sessions_dir);
        let meta = parse_session(&path, Some(&display_names)).unwrap();
        assert_eq!(meta.title.as_deref(), Some("重构登录模块"));
    }

    #[test]
    fn parse_session_falls_back_to_dir_basename() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-def.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"session\",\"id\":\"session-def\",\"cwd\":\"/tmp/my-project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n"
            ),
        )
        .expect("write");

        let meta = parse_session(&path, None).unwrap();
        // No user message and no displayName → falls back to dir basename
        assert_eq!(meta.title.as_deref(), Some("my-project"));
    }

    #[test]
    fn parse_session_truncates_long_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-trunc.jsonl");
        let long_msg = "a".repeat(200);
        std::fs::write(
            &path,
            format!(
                "{{\"type\":\"session\",\"id\":\"session-trunc\",\"cwd\":\"/tmp/p\",\"timestamp\":\"2026-03-06T10:00:00Z\"}}\n\
                 {{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"{long_msg}\"}},\"timestamp\":\"2026-03-06T10:01:00Z\"}}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path, None).unwrap();
        let title = meta.title.unwrap();
        assert_eq!(title.chars().count(), 161);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn delete_session_updates_index_and_removes_jsonl() {
        let temp = tempdir().expect("tempdir");
        let sessions_dir = temp.path().join("main").join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");

        let session_path = sessions_dir.join("session-123.jsonl");
        std::fs::write(
            &session_path,
            concat!(
                "{\"type\":\"session\",\"id\":\"session-123\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n"
            ),
        )
        .expect("write session");
        std::fs::write(
            sessions_dir.join("sessions.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "agent:main:main": {
                    "sessionId": "session-123",
                    "sessionFile": session_path,
                },
                "agent:main:other": {
                    "sessionId": "session-456",
                    "sessionFile": sessions_dir.join("session-456.jsonl"),
                },
            }))
            .expect("serialize index"),
        )
        .expect("write index");

        delete_session(temp.path(), &session_path, "session-123").expect("delete session");

        assert!(!session_path.exists());
        let updated: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(sessions_dir.join("sessions.json")).expect("read index"),
        )
        .expect("parse index");
        assert!(updated.get("agent:main:main").is_none());
        assert!(updated.get("agent:main:other").is_some());
    }
}
