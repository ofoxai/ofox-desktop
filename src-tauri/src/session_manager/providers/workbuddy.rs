//! WorkBuddy native session metadata; transcript bodies are never cached here.
use crate::{
    error::AppError,
    session_manager::{SessionMessage, SessionMeta},
};
use std::path::{Path, PathBuf};

fn failure(code: &str) -> AppError {
    AppError::InvalidInput(format!("session.workbuddy.{code}"))
}

pub(crate) fn session_root() -> Result<PathBuf, AppError> {
    let config = match std::env::var("WORKBUDDY_CONFIG_DIR") {
        Ok(raw) if !raw.trim().is_empty() => {
            let path = PathBuf::from(raw.trim());
            if !path.is_absolute() {
                return Err(failure("invalidConfigDirectory"));
            }
            path
        }
        Ok(_) | Err(std::env::VarError::NotPresent) => {
            crate::config::get_home_dir().join(".workbuddy")
        }
        Err(_) => return Err(failure("invalidConfigDirectory")),
    };
    Ok(config.join("projects"))
}

fn sidecar_project(path: &Path) -> Result<Option<String>, AppError> {
    let sidecar = path.with_extension("meta.json");
    let meta = match std::fs::symlink_metadata(&sidecar) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(failure("metadataUnavailable")),
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
        return Err(failure("invalidMetadata"));
    }
    let snapshot = crate::services::usage_source::Snapshot::file(&sidecar)?;
    let bytes = std::fs::read(&sidecar).map_err(|_| failure("metadataUnavailable"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| failure("invalidMetadata"))?;
    snapshot.verify()?;
    match value.get("cwd") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok((!s.trim().is_empty()).then(|| s.clone())),
        _ => Err(failure("invalidMetadata")),
    }
}

pub(crate) fn scan_sessions_at(root: &Path) -> Result<Vec<SessionMeta>, AppError> {
    let mut sessions = Vec::new();
    for path in crate::services::session_usage_workbuddy::collect_files(root)? {
        let parsed = crate::services::session_usage_workbuddy::read_workbuddy(&path)?;
        if let Some(mut meta) = parsed.session {
            // A conflicting native project is not repaired with an older sidecar.
            if meta.project_dir.is_none() && !parsed.reasons.contains("conflictingSessionProject") {
                meta.project_dir = sidecar_project(&path)?;
                if meta.title.is_none() {
                    (meta.title, meta.title_source) = super::super::title::titled(
                        &super::super::title::TitleCandidates::default()
                            .with_directory(meta.project_dir.as_deref()),
                    );
                }
            }
            sessions.push(meta);
        }
    }
    Ok(sessions)
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    match session_root().and_then(|root| scan_sessions_at(&root)) {
        Ok(sessions) => sessions,
        Err(_) => {
            log::warn!("WorkBuddy session scan unavailable");
            Vec::new()
        }
    }
}

pub(crate) fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    use crate::services::session_usage_workbuddy as wb;
    use serde_json::Value;
    let mut messages: Vec<SessionMessage> = Vec::new();
    let mut identities = std::collections::BTreeMap::new();
    let progress = crate::session_manager::read_context::json_lines(
        path,
        |_| true,
        |_, value| {
            let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
            let (role, content, name) = match kind {
                "message" => {
                    let role = value.get("role").and_then(Value::as_str).unwrap_or("");
                    if !matches!(role, "user" | "assistant") || !wb::visible_message(&value) {
                        return Ok(());
                    }
                    (role, wb::message_text(&value), None)
                }
                "function_call" => (
                    "tool_call",
                    value
                        .get("arguments")
                        .map(super::extract_text)
                        .unwrap_or_default(),
                    value.get("name").and_then(Value::as_str).map(str::to_owned),
                ),
                "function_call_output" => (
                    "tool",
                    value
                        .get("output")
                        .map(super::extract_text)
                        .unwrap_or_default(),
                    value.get("name").and_then(Value::as_str).map(str::to_owned),
                ),
                "reasoning" => (
                    "thinking",
                    value
                        .get("content")
                        .map(super::extract_text)
                        .unwrap_or_default(),
                    None,
                ),
                _ => return Ok(()),
            };
            if content.trim().is_empty() && name.is_none() {
                return Ok(());
            }
            let message = SessionMessage {
                role: role.into(),
                content,
                ts: wb::native_time(value.get("timestamp")),
                name,
            };
            let id = value
                .pointer("/providerData/messageId")
                .or_else(|| value.get("id"))
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty());
            if let Some(id) = id {
                let key = (role.to_owned(), id.to_owned());
                if let Some(&index) = identities.get(&key) {
                    messages[index] = message;
                } else {
                    identities.insert(key, messages.len());
                    messages.push(message);
                }
            } else {
                messages.push(message);
            }
            Ok(())
        },
    )
    .map_err(|_| "session.workbuddy.sourceUnavailable".to_owned())?;
    if !progress.complete {
        return Err("session.workbuddy.incompleteEnumeration".into());
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workbuddy_metadata_root_file_is_unknown_instead_of_an_empty_listing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        std::fs::write(&root, "invalid native directory").unwrap();
        assert!(
            scan_sessions_at(&root).is_err(),
            "an existing file must not report a successful empty session listing"
        );
    }

    #[test]
    #[serial_test::serial]
    fn workbuddy_resolver_honors_native_config_dir_in_a_sandbox() {
        let sandbox = crate::test_sandbox::sandbox();
        let chosen = sandbox.home().join("native-wb");
        std::env::set_var("WORKBUDDY_CONFIG_DIR", &chosen);
        assert_eq!(session_root().unwrap(), chosen.join("projects"));
    }

    #[test]
    #[serial_test::serial]
    fn workbuddy_resolver_matches_the_installed_vendor_trim_contract() {
        let sandbox = crate::test_sandbox::sandbox();
        let chosen = sandbox.home().join("native-wb");
        std::env::set_var("WORKBUDDY_CONFIG_DIR", format!("  {}  ", chosen.display()));
        assert_eq!(session_root().unwrap(), chosen.join("projects"));
        std::env::set_var("WORKBUDDY_CONFIG_DIR", "  ");
        assert_eq!(
            session_root().unwrap(),
            sandbox.home().join(".workbuddy/projects")
        );
    }

    #[test]
    #[serial_test::serial]
    fn workbuddy_resolver_does_not_treat_an_unresolvable_override_as_default() {
        let _sandbox = crate::test_sandbox::sandbox();
        std::env::set_var("WORKBUDDY_CONFIG_DIR", "relative-directory");
        assert!(session_root().is_err());
    }

    #[test]
    #[serial_test::serial]
    fn workbuddy_resolver_default_never_leaves_the_sandbox() {
        let sandbox = crate::test_sandbox::sandbox();
        assert_eq!(
            session_root().unwrap(),
            sandbox.home().join(".workbuddy").join("projects")
        );
    }

    #[test]
    fn workbuddy_metadata_keeps_html_prompts_custom_titles_and_exact_project_paths() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("s.jsonl");
        let rows = [
            json!({"type":"message","role":"user","id":"system","sessionId":"s","cwd":"/one/project ","timestamp":1_792_022_400_000_i64,"content":"<system-reminder>private context</system-reminder>"}),
            json!({"type":"message","role":"user","id":"question","sessionId":"s","timestamp":1_792_022_401_000_i64,"content":[{"type":"input_text","text":"<html>为什么空白？</html>"}]}),
            json!({"type":"ai-title","sessionId":"s","aiTitle":"native title","timestamp":1_792_022_402_000_i64}),
            json!({"type":"custom-title","sessionId":"s","customTitle":"chosen title","timestamp":1_792_022_403_000_i64}),
        ];
        std::fs::write(
            &path,
            rows.iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let found = scan_sessions_at(temp.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session_id, "s");
        assert_eq!(found[0].title.as_deref(), Some("chosen title"));
        assert_eq!(found[0].project_dir.as_deref(), Some("/one/project "));
        assert_eq!(found[0].created_at, Some(1_792_022_400_000));
        assert_eq!(found[0].message_count, Some(1));
        assert_eq!(found[0].resume_command, None);
        assert_eq!(found[0].summary, None);
        let transcript = load_messages(&path).unwrap();
        assert!(transcript.iter().any(|m| m.content.contains("为什么空白")));
        assert!(!transcript
            .iter()
            .any(|m| m.content.contains("private context")));
    }

    #[test]
    fn workbuddy_html_prompt_alone_names_the_session() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("s.jsonl"),json!({"type":"message","role":"user","id":"q","sessionId":"s","timestamp":1000,"content":"<html>页面为什么空白？</html>"}).to_string()).unwrap();
        let found = scan_sessions_at(temp.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title.as_deref(), Some("页面为什么空白？"));
        assert_eq!(found[0].created_at, Some(1000));
    }

    #[test]
    fn workbuddy_historical_native_meta_sidecar_supplies_cwd_without_inventing_a_session_id() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("s.jsonl"),json!({"type":"message","role":"user","id":"q","sessionId":"s","timestamp":1000,"content":"task"}).to_string()).unwrap();
        std::fs::write(
            temp.path().join("s.meta.json"),
            json!({"cwd":"/real/project "}).to_string(),
        )
        .unwrap();
        let found = scan_sessions_at(temp.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].project_dir.as_deref(), Some("/real/project "));
        assert_eq!(found[0].session_id, "s");
    }

    #[test]
    fn workbuddy_subagent_without_native_parent_is_not_folded_into_a_guessed_root() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("root").join("subagents");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("child.jsonl"),json!({"type":"message","role":"user","id":"q","sessionId":"child-native","timestamp":1_792_022_400_000_i64,"content":"task"}).to_string()).unwrap();
        let found = scan_sessions_at(temp.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session_id, "child-native");
        assert_ne!(found[0].session_id, "root");
    }
}
