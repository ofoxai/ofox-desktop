//! One coherent WorkBuddy roster; no route or billing inference.
use super::{
    session_analytics_core::{MessageEvent, Role},
    session_analytics_index::{key, ParsedSource},
    session_analytics_native as native, session_usage_workbuddy as reader,
};
use crate::{database::Database, error::AppError};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn root(roots: &[PathBuf]) -> Result<&Path, AppError> {
    match roots {
        [root] => Ok(root),
        _ => Err(AppError::InvalidInput(
            "usage.analytics.sourceReadFailed".into(),
        )),
    }
}
fn files(roots: &[PathBuf]) -> Result<Vec<PathBuf>, AppError> {
    reader::collect_files(root(roots)?)
}
pub(crate) fn stamp(roots: &[PathBuf]) -> Result<Value, AppError> {
    let files = files(roots)?;
    let mut dependencies = Vec::new();
    for path in &files {
        let sidecar = path.with_extension("meta.json");
        let proof = match std::fs::symlink_metadata(&sidecar) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Some(
                crate::session_manager::source_cache::SourceStamp::at(&sidecar)?,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            _ => {
                return Err(AppError::InvalidInput(
                    "usage.analytics.projectDependencyUnavailable".into(),
                ))
            }
        };
        dependencies.push((sidecar, proof));
    }
    Ok(json!({"native":native::roster_stamp(roots,&files)?,"dependencies":dependencies}))
}
pub(crate) fn read(_db: &Database, roots: &[PathBuf]) -> Result<ParsedSource, AppError> {
    let before = stamp(roots)?;
    let files = files(roots)?;
    let mut result = native::empty();
    if roots.iter().any(|root| {
        std::fs::symlink_metadata(root)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    }) {
        result.reasons.push("sourceMissing".into());
    }
    let mut sessions = BTreeMap::new();
    let mut facts = BTreeMap::new();
    let mut messages = BTreeMap::new();
    let mut message_hashes = BTreeMap::new();
    let mut native_streams = BTreeMap::new();
    let mut activity_entries = 0usize;
    let mut bytes = 0u64;
    for path in files {
        bytes = bytes
            .checked_add(std::fs::metadata(&path).map_err(|_| invalid())?.len())
            .ok_or_else(invalid)?;
        if bytes > 128 * 1024 * 1024 {
            return Err(invalid());
        }
        let parsed = reader::read_workbuddy(&path)?;
        let transcript = parsed.analytics;
        let complete = parsed.reasons.is_empty();
        for fact in parsed.facts {
            let Some(fact) =
                native::normalized_fact("workbuddy", "workbuddy_session", fact, complete)
            else {
                continue;
            };
            if facts
                .get(&fact.event_key)
                .is_some_and(|prior| prior != &fact)
            {
                return Err(invalid());
            }
            facts.insert(fact.event_key.clone(), fact);
        }
        result.reasons.extend(parsed.reasons.iter().cloned());
        let Some(meta) = parsed.session else {
            result.reasons.push("sessionIdentityUnavailable".into());
            continue;
        };
        for (id, hash) in parsed.analytics_message_hashes {
            let id = (meta.session_id.clone(), id);
            if message_hashes.get(&id).is_some_and(|prior| prior != &hash) {
                return Err(invalid());
            }
            message_hashes.insert(id, hash);
        }
        let sidecar = if !transcript.project_parsed
            && !parsed.reasons.contains("conflictingSessionProject")
        {
            crate::session_manager::providers::workbuddy::sidecar_project(&path)?
        } else {
            None
        };
        let project = if transcript.project_parsed {
            Some(transcript.project_dir.as_deref())
        } else {
            sidecar.as_deref().map(Some)
        };
        let mut descriptor = native::descriptor(
            "workbuddy",
            &meta.session_id,
            meta.title.as_deref().unwrap_or(&meta.session_id),
            project,
            None,
            path.to_str(),
        );
        descriptor.message_reasons = transcript
            .reasons
            .iter()
            .filter(|reason| sidecar.is_none() || reason.as_str() != "projectNotRecorded")
            .cloned()
            .collect();
        activity_entries = activity_entries
            .checked_add(native::merge_activity_stream(
                &mut native_streams,
                key("workbuddy", &meta.session_id),
                parsed.native_activity,
            )?)
            .ok_or_else(invalid)?;
        if activity_entries > 100_000 {
            return Err(invalid());
        }
        if let Some(prior) = sessions.get(&meta.session_id) {
            let prior: &super::session_analytics::SessionDescriptor = prior;
            if prior.identity.project != descriptor.identity.project
                || prior.title != descriptor.title
                || prior.message_reasons != descriptor.message_reasons
            {
                return Err(invalid());
            }
        } else {
            sessions.insert(meta.session_id.clone(), descriptor);
        }
        result.reasons.extend(
            transcript
                .reasons
                .iter()
                .filter(|reason| sidecar.is_none() || reason.as_str() != "projectNotRecorded")
                .cloned(),
        );
        for message in transcript.messages.into_values() {
            let Some(at_ms) = message.at_ms else {
                continue;
            };
            let event = MessageEvent {
                session_key: key("workbuddy", &meta.session_id),
                native_id: message.identity,
                at_ms,
                role: if message.role == "user" {
                    Role::User
                } else {
                    Role::Assistant
                },
                main: true,
            };
            let id = (event.session_key.clone(), event.native_id.clone());
            if messages.get(&id).is_some_and(|prior| prior != &event) {
                return Err(invalid());
            }
            messages.insert(id, event);
        }
        if facts.len().max(messages.len()) > 100_000 {
            return Err(invalid());
        }
    }
    if stamp(roots)? != before {
        return Err(invalid());
    }
    result.sessions = sessions.into_values().collect();
    result.facts = facts.into_values().collect();
    result.messages = messages.into_values().collect();
    result.native_activity = native_streams.into_values().flatten().collect();
    result.reasons.sort();
    result.reasons.dedup();
    Ok(result)
}
fn invalid() -> AppError {
    AppError::InvalidInput("usage.analytics.sourceReadFailed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn source(root: &Path, name: &str, sid: &str, input: u64) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let rows = [
            json!({"type":"message","role":"user","id":"u","sessionId":sid,"cwd":"/fixture/project","timestamp":1000,"content":"synthetic question"}),
            json!({"type":"message","role":"assistant","id":"a","sessionId":sid,"cwd":"/fixture/project","timestamp":2000,"content":"private answer","providerData":{"messageId":"a","model":"fixture","usage":{"requests":1,"inputTokens":input,"outputTokens":2}}}),
        ];
        std::fs::write(
            &path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();
        path
    }
    #[test]
    fn analytics_workbuddy_replicas_deduplicate_but_conflicts_do_not_choose_a_winner() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        source(&root, "one.jsonl", "s", 10);
        source(&root, "copy.jsonl", "s", 10);
        let db = Database::memory().unwrap();
        let parsed = read(&db, std::slice::from_ref(&root)).unwrap();
        assert_eq!(parsed.sessions.len(), 1);
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(parsed.messages.len(), 2);
        assert!(!serde_json::to_string(&parsed.sessions)
            .unwrap()
            .contains("private answer"));
        source(&root, "copy.jsonl", "s", 11);
        assert!(read(&db, &[root]).is_err());
    }
    #[test]
    fn analytics_workbuddy_missing_time_is_unknown_and_stat_scan_does_not_parse() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        let path = source(&root, "one.jsonl", "s", 0);
        let mut rows = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|row| serde_json::from_str::<Value>(row).unwrap())
            .collect::<Vec<_>>();
        rows[1].as_object_mut().unwrap().remove("timestamp");
        std::fs::write(
            &path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();
        let parsed = read(&Database::memory().unwrap(), std::slice::from_ref(&root)).unwrap();
        assert!(parsed.facts.is_empty());
        assert_eq!(parsed.messages.len(), 1);
        assert!(parsed.sessions[0]
            .message_reasons
            .contains(&"messageTimeUnavailable".into()));
        std::fs::write(path, "invalid private body").unwrap();
        assert!(stamp(&[root]).is_ok());
    }

    #[test]
    fn native_runtime_workbuddy_projects_the_validated_roster_once_and_rejects_runtime_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        let path = source(&root, "one.jsonl", "s", 10);
        let runtime = json!({"type":"function_call_result","id":"tool","sessionId":"s","timestamp":3000,"output":"private runtime"});
        let text = std::fs::read_to_string(&path).unwrap() + &runtime.to_string() + "\n";
        std::fs::write(&path, &text).unwrap();
        std::fs::write(root.join("copy.jsonl"), &text).unwrap();
        let parsed = read(&Database::memory().unwrap(), std::slice::from_ref(&root)).unwrap();
        assert_eq!(parsed.native_activity.len(), 3);
        assert_eq!(
            parsed
                .native_activity
                .iter()
                .map(|event| event.at_ms)
                .collect::<Vec<_>>(),
            [1000, 2000, 3000]
        );
        let encoded = serde_json::to_string(&parsed.native_activity).unwrap();
        assert!(!encoded.contains("private runtime"));
        assert!(!encoded.contains("private answer"));

        let mut changed = runtime;
        changed["output"] = json!("conflicting runtime");
        let changed = text.lines().take(2).collect::<Vec<_>>().join("\n")
            + "\n"
            + &changed.to_string()
            + "\n";
        std::fs::write(root.join("copy.jsonl"), changed).unwrap();
        assert!(read(&Database::memory().unwrap(), &[root]).is_err());
    }
}
