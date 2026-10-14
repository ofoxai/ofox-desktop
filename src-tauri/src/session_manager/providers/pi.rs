//! Pi session discovery and active-branch traversal, adapted from Magpie's
//! internal/sessions/{pi,transcript}.go. Copyright (c) 2026 yetone, MIT;
//! the complete license is in ../MAGPIE_LICENSE.
use super::utils::{extract_text, parse_timestamp_to_ms, truncate_summary};
use crate::session_manager::title::{titled, TitleCandidates};
use crate::session_manager::{SessionMessage, SessionMeta};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn agent_dir() -> Result<PathBuf, String> {
    crate::native_tools::config_dir(crate::app_config::BindableTool::Pi)
}

pub fn session_roots() -> Result<Vec<PathBuf>, String> {
    roots_in(
        &agent_dir()?,
        std::env::var("PI_CODING_AGENT_SESSION_DIR").ok().as_deref(),
    )
}

fn roots_in(agent: &Path, override_dir: Option<&str>) -> Result<Vec<PathBuf>, String> {
    let mut roots = vec![agent.join("sessions")];
    let settings = std::fs::read_to_string(agent.join("settings.json"))
        .ok()
        .and_then(|s| json5::from_str::<Value>(&s).ok());
    if let Some(raw) = override_dir
        .filter(|s| !s.trim().is_empty())
        .or_else(|| settings.as_ref()?.get("sessionDir")?.as_str())
    {
        let path = if raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\") {
            crate::config::get_home_dir()
                .join(raw.trim_start_matches('~').trim_start_matches(['/', '\\']))
        } else {
            PathBuf::from(raw)
        };
        if !path.is_absolute() {
            return Err("Pi session directory must be absolute".into());
        }
        if !roots.contains(&path) {
            roots.push(path);
        }
    }
    Ok(roots)
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    session_roots()
        .map(|roots| scan_roots(&roots))
        .unwrap_or_default()
}

pub(crate) fn scan_roots(roots: &[PathBuf]) -> Vec<SessionMeta> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for (index, root) in roots.iter().enumerate() {
        // Default storage is sessions/<project>/*.jsonl; custom storage is flat.
        collect_files(root, usize::from(index == 0), &mut paths);
    }
    paths
        .into_iter()
        .filter_map(|path| {
            let canonical = path.canonicalize().ok()?;
            if !seen.insert(canonical) {
                return None;
            }
            parse_session(&path)
        })
        .collect()
}

fn collect_files(root: &Path, depth: usize, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && depth > 0 {
            collect_files(&entry.path(), depth - 1, paths);
        } else if kind.is_file() && entry.path().extension().is_some_and(|e| e == "jsonl") {
            paths.push(entry.path());
        }
    }
}

fn entries(path: &Path) -> Result<Vec<Value>, String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        // A CLI may leave its final line incomplete; completed entries remain readable.
        if let Ok(value) = serde_json::from_str(&line) {
            entries.push(value);
        }
    }
    Ok(entries)
}

pub(crate) fn active_branch(all: &[Value]) -> Vec<&Value> {
    let mut by_id = HashMap::new();
    let mut leaf = None;
    for entry in all {
        if entry.get("type").and_then(Value::as_str) == Some("session") {
            continue;
        }
        if let Some(id) = entry.get("id").and_then(Value::as_str) {
            by_id.insert(id, entry);
            leaf = Some(id);
        }
    }
    let mut branch = Vec::new();
    let mut seen = HashSet::new();
    while let Some(id) = leaf {
        if !seen.insert(id) {
            break;
        }
        let Some(entry) = by_id.get(id) else {
            break;
        };
        branch.push(*entry);
        leaf = entry.get("parentId").and_then(Value::as_str);
    }
    branch.reverse();
    branch
}

pub(crate) fn parse_session(path: &Path) -> Option<SessionMeta> {
    let all = entries(path).ok()?;
    let head = all
        .iter()
        .find(|v| v.get("type").and_then(Value::as_str) == Some("session"))?;
    let id = head.get("id")?.as_str()?.to_string();
    let branch = active_branch(&all);
    let messages = messages_from_branch(&branch);
    let project_dir = head.get("cwd").and_then(Value::as_str);
    let mut candidates = TitleCandidates::default()
        .with_first_user_text(
            messages
                .iter()
                .find(|m| m.role == "user")
                .map(|m| m.content.as_str()),
        )
        .with_directory(project_dir);
    // the name given with /name
    candidates.custom = branch.iter().rev().find_map(|v| {
        (v.get("type").and_then(Value::as_str) == Some("session_info"))
            .then(|| v.get("name").and_then(Value::as_str))
            .flatten()
            .map(str::to_string)
    });
    let (title, title_source) = titled(&candidates);
    Some(SessionMeta {
        provider_id: "pi".into(),
        session_id: id,
        title,
        summary: messages.last().map(|m| truncate_summary(&m.content, 160)),
        project_dir: project_dir.map(str::to_string),
        created_at: head.get("timestamp").and_then(parse_timestamp_to_ms),
        last_active_at: all
            .iter()
            .rev()
            .find_map(|v| v.get("timestamp").and_then(parse_timestamp_to_ms)),
        source_path: Some(path.to_string_lossy().into()),
        resume_command: None,
        title_source,
    })
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let all = entries(path)?;
    Ok(messages_from_branch(&active_branch(&all)))
}

pub(crate) fn models(path: &Path) -> Vec<String> {
    let Ok(all) = entries(path) else {
        return Vec::new();
    };
    let mut models = Vec::new();
    for entry in active_branch(&all) {
        let model = if entry["type"] == "model_change" {
            entry["modelId"].as_str()
        } else {
            entry["message"]["model"].as_str()
        };
        if let Some(model) = model.filter(|model| !model.is_empty()) {
            if !models.iter().any(|m| m == model) {
                models.push(model.to_owned());
            }
        }
    }
    models
}

fn messages_from_branch(branch: &[&Value]) -> Vec<SessionMessage> {
    let mut out = Vec::new();
    for entry in branch {
        let ts = entry.get("timestamp").and_then(parse_timestamp_to_ms);
        let kind = entry.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(kind, "compaction" | "branch_summary") {
            if let Some(summary) = entry.get("summary").and_then(Value::as_str) {
                out.push(SessionMessage {
                    role: "context".into(),
                    content: summary.into(),
                    ts,
                    name: None,
                });
            }
            continue;
        }
        if kind != "message" {
            continue;
        }
        let Some(message) = entry.get("message") else {
            continue;
        };
        let raw_role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let role = if raw_role == "toolResult" {
            "tool"
        } else {
            raw_role
        };
        let Some(content) = message.get("content") else {
            continue;
        };
        if let Some(parts) = content.as_array() {
            for part in parts {
                let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                let mut name = None;
                let (role, text) = match kind {
                    "thinking" => (
                        "thinking",
                        part.get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .into(),
                    ),
                    "toolCall" => {
                        name = Some(
                            part.get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .to_string(),
                        );
                        (
                            "tool_call",
                            serde_json::to_string_pretty(&part["arguments"]).unwrap_or_default(),
                        )
                    }
                    "image" => (role, "[Image]".into()),
                    _ => (role, extract_text(part)),
                };
                if !text.trim().is_empty() {
                    out.push(SessionMessage {
                        role: role.into(),
                        content: text,
                        ts,
                        name,
                    });
                }
            }
        } else {
            let text = extract_text(content);
            if !text.trim().is_empty() {
                out.push(SessionMessage {
                    role: role.into(),
                    content: text,
                    ts,
                    name: None,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn current_branch_excludes_abandoned_answer_and_handles_cycle() {
        let all = vec![
            json!({"id":"e1","parentId":null}),
            json!({"id":"old","parentId":"e1"}),
            json!({"id":"e2","parentId":"e1"}),
            json!({"id":"e3","parentId":"e2"}),
        ];
        let ids: Vec<_> = active_branch(&all)
            .iter()
            .map(|v| v["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["e1", "e2", "e3"]);
        assert_eq!(active_branch(&[json!({"id":"x","parentId":"x"})]).len(), 1);
    }
    #[test]
    fn settings_custom_directory_and_environment_precedence() {
        let temp = tempfile::tempdir().unwrap();
        let custom = temp.path().join("custom");
        std::fs::write(
            temp.path().join("settings.json"),
            format!("{{sessionDir: {:?}}}", custom.to_string_lossy()),
        )
        .unwrap();
        assert_eq!(roots_in(temp.path(), None).unwrap()[1], custom);
        let env = temp.path().join("env");
        assert_eq!(
            roots_in(temp.path(), Some(env.to_str().unwrap())).unwrap()[1],
            env
        );
        assert!(roots_in(temp.path(), Some("relative")).is_err());
    }

    // Adapted from Magpie TestTranscriptPiBranch: tool/thinking parts and a
    // discarded sibling remain distinct, with complete body text for exports.
    #[test]
    fn branch_transcript_keeps_full_text_tools_context_and_model_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("date_s.jsonl");
        let long = "body ``` nested ".repeat(2000);
        let records = [
            serde_json::json!({"type":"session","id":"s","cwd":"/w","timestamp":"2026-01-01T00:00:00Z"}),
            serde_json::json!({"type":"model_change","id":"e1","parentId":null,"modelId":"new-model"}),
            serde_json::json!({"type":"message","id":"e2","parentId":"e1","message":{"role":"user","content":"Rename foo"}}),
            serde_json::json!({"type":"message","id":"old","parentId":"e2","message":{"role":"assistant","content":"abandoned"}}),
            serde_json::json!({"type":"message","id":"e3","parentId":"e2","message":{"role":"assistant","content":[{"type":"thinking","thinking":"grep first"},{"type":"toolCall","name":"bash","arguments":{"command":"grep -r foo"}}]}}),
            serde_json::json!({"type":"message","id":"e4","parentId":"e3","message":{"role":"toolResult","content":[{"type":"text","text":"a.go: foo()"}]}}),
            serde_json::json!({"type":"compaction","id":"e5","parentId":"e4","summary":"context summary"}),
            serde_json::json!({"type":"message","id":"e6","parentId":"e5","message":{"role":"assistant","content":long}}),
        ];
        let content = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, &content).unwrap();
        let messages = load_messages(&path).unwrap();
        assert!(!messages.iter().any(|m| m.content.contains("abandoned")));
        assert!(messages
            .iter()
            .any(|m| m.role == "thinking" && m.content == "grep first"));
        assert!(messages.iter().any(|m| m.content.contains("grep -r foo")));
        assert!(messages.iter().any(|m| m.role == "context"));
        assert_eq!(messages.last().unwrap().content, long);
        assert_eq!(models(&path), ["new-model"]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    }
}
