//! One Hermes SessionDB roster, read by the same reader as the ledger.
//! Activity follows magpie: each session starts at `started_at`, then every
//! displayed message in Hermes id order. No route or billing inference.
use super::{
    session_analytics_core::{MessageEvent, Role},
    session_analytics_index::{key, ParsedSource},
    session_analytics_native as native, session_usage_hermes as reader, usage_source,
};
use crate::{database::Database, error::AppError};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn invalid() -> AppError {
    AppError::InvalidInput("usage.analytics.sourceReadFailed".into())
}
fn files(roots: &[PathBuf]) -> Result<Vec<PathBuf>, AppError> {
    let [root] = roots else {
        return Err(invalid());
    };
    let path = root.join("state.db");
    match std::fs::symlink_metadata(&path) {
        Ok(_) => Ok(vec![path]),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(_) => Err(invalid()),
    }
}
pub(crate) fn stamp(roots: &[PathBuf]) -> Result<Value, AppError> {
    let files = files(roots)?;
    let source = files
        .iter()
        .map(|path| usage_source::normalized_stamp("hermes", path))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"native":native::roster_stamp(roots,&files)?,"source":source}))
}
pub(crate) fn read(_db: &Database, roots: &[PathBuf]) -> Result<ParsedSource, AppError> {
    let before = stamp(roots)?;
    let mut result = native::empty();
    let Some(path) = files(roots)?.pop() else {
        result.reasons.push("sourceMissing".into());
        return Ok(result);
    };
    let read = reader::read_hermes(&path)?;
    result.sessions = read
        .sessions
        .iter()
        .map(|session| {
            let mut descriptor = native::descriptor(
                "hermes",
                &session.sid,
                session.title.as_deref().unwrap_or(&session.sid),
                Some(session.cwd.as_deref()),
                None,
                Some(&source_path(&path, &session.sid)),
            );
            if read.messages.is_none() {
                descriptor.messages_supported = false;
                descriptor.message_reasons = vec!["activityBudgetExceeded".into()];
            }
            descriptor
        })
        .collect();
    result.facts = read
        .facts
        .into_iter()
        .filter_map(|fact| {
            let complete = !read.incomplete.contains(&fact.request_id);
            native::normalized_fact("hermes", "hermes_session", fact, complete)
        })
        .collect();
    if let Some(messages) = &read.messages {
        let mut scans = std::collections::BTreeMap::<
            String,
            super::session_analytics::interactions::CallScan,
        >::new();
        for message in messages {
            let scan = scans.entry(message.sid.clone()).or_default();
            scan.incomplete |= message.tools.incomplete;
            scan.conflicts
                .extend(message.tools.conflicts.iter().cloned());
            scan.records.retain(|id, _| !scan.conflicts.contains(id));
            for (id, call) in &message.tools.records {
                if scan.conflicts.contains(id) {
                    continue;
                }
                if scan.records.get(id).is_some_and(|prior| prior != call) {
                    scan.incomplete = true;
                    scan.records.remove(id);
                    scan.conflicts.insert(id.clone());
                } else {
                    scan.records.insert(id.clone(), call.clone());
                }
            }
        }
        for session in &read.sessions {
            let mut scan = scans.remove(&session.sid).unwrap_or_default();
            scan.incomplete |= read.reasons.contains("invalidMessageTime");
            result
                .tool_evidence
                .attach(&key("hermes", &session.sid), &scan);
        }
        for session in read.sessions.iter().filter(|s| s.started_ms > 0) {
            let start = json!([session.sid, "start"]).to_string();
            result.native_activity.push(native::runtime_event(
                "hermes",
                &session.sid,
                &start,
                session.started_ms,
            ));
            for message in messages.iter().filter(|m| m.sid == session.sid) {
                let id = json!([session.sid, message.id]).to_string();
                result.native_activity.push(native::runtime_event(
                    "hermes",
                    &session.sid,
                    &id,
                    message.at_ms,
                ));
            }
        }
        result.messages = messages
            .iter()
            .filter_map(|message| {
                let role = match message.role.as_str() {
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    _ => return None,
                };
                Some(MessageEvent {
                    session_key: key("hermes", &message.sid),
                    native_id: message.id.to_string(),
                    at_ms: message.at_ms,
                    role,
                    main: true,
                })
            })
            .collect();
    }
    result.reasons = read.reasons.into_iter().collect();
    if stamp(roots)? != before {
        return Err(invalid());
    }
    Ok(result)
}
fn source_path(db_path: &Path, sid: &str) -> String {
    crate::session_manager::providers::hermes::sqlite_source(db_path, sid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_sandbox::sandbox;
    use rusqlite::Connection;
    use serial_test::serial;
    use std::fs;

    const NATIVE_SQL: &str = include_str!("fixtures/hermes-native-0.21.7-sessions.sql");
    const T0_MS: i64 = 1_790_820_000_000;

    fn root(home: &Path, keep: &[&str]) -> PathBuf {
        let root = home.join("hermes");
        fs::create_dir_all(&root).unwrap();
        let conn = Connection::open(root.join("state.db")).unwrap();
        conn.execute_batch(NATIVE_SQL).unwrap();
        let keep = serde_json::to_string(keep).unwrap();
        for (table, column) in [
            ("sessions", "id"),
            ("session_model_usage", "session_id"),
            ("messages", "session_id"),
        ] {
            conn.execute(
                &format!(
                    "DELETE FROM {table} WHERE {column} NOT IN (SELECT value FROM json_each(?1))"
                ),
                [&keep],
            )
            .unwrap();
        }
        root
    }

    #[test]
    #[serial]
    fn analytics_hermes_conflicting_call_times_have_no_winner() {
        let sandbox = sandbox();
        let root = root(sandbox.home(), &["main-aux"]);
        let conn = Connection::open(root.join("state.db")).unwrap();
        for timestamp in [1790820012.0, 1790820013.0, 1790820014.0] {
            conn.execute("INSERT INTO messages(session_id,role,content,tool_calls,timestamp) VALUES('main-aux','assistant','synthetic',?1,?2)",rusqlite::params![r#"[{"id":"reused","function":{"name":"Bash"}}]"#,timestamp]).unwrap();
        }
        drop(conn);
        let parsed = read(&Database::memory().unwrap(), std::slice::from_ref(&root)).unwrap();
        assert!(parsed
            .tool_evidence
            .calls
            .iter()
            .all(|event| event.call.name != "Bash"));
        assert!(parsed
            .tool_evidence
            .incomplete
            .contains(&key("hermes", "main-aux")));
    }

    #[test]
    #[serial]
    fn analytics_hermes_descriptor_project_and_manager_key_match_listing() {
        let sandbox = sandbox();
        let root = root(sandbox.home(), &["main-aux", "legacy"]);
        let parsed = read(&Database::memory().unwrap(), std::slice::from_ref(&root)).unwrap();
        let alpha = parsed
            .sessions
            .iter()
            .find(|s| s.native_session_id == "main-aux")
            .unwrap();
        assert_eq!(alpha.project_path.as_deref(), Some("/work/alpha"));
        assert_eq!(alpha.title, "Alpha");
        let calls: Vec<_> = parsed
            .tool_evidence
            .calls
            .iter()
            .filter(|event| event.session_key == alpha.identity.key)
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call.name, "terminal");
        assert!(!parsed
            .tool_evidence
            .incomplete
            .contains(&alpha.identity.key));
        let listing = crate::session_manager::SessionMeta {
            provider_id: "hermes".into(),
            session_id: "main-aux".into(),
            source_path: Some(format!(
                "sqlite:{}#main-aux",
                root.join("state.db").display()
            )),
            ..Default::default()
        };
        assert_eq!(
            alpha.manager_key.as_deref(),
            Some(crate::session_manager::managed::key(&listing).as_str())
        );
        let legacy = parsed
            .sessions
            .iter()
            .find(|s| s.native_session_id == "legacy")
            .unwrap();
        assert_eq!(
            legacy.identity.project,
            super::super::session_analytics_core::Project::Unknown
        );
    }

    #[test]
    #[serial]
    fn analytics_hermes_facts_equal_ledger_rows() {
        let sandbox = sandbox();
        let root = root(sandbox.home(), &["main-aux", "partial"]);
        let db = Database::memory().unwrap();
        reader::sync_hermes_usage_at(&db, &root).unwrap();
        let parsed = read(&db, &[root]).unwrap();
        let mut facts: Vec<_> = parsed
            .facts
            .iter()
            .map(|f| (f.model.clone(), f.buckets, f.at_ms / 1000, f.complete))
            .collect();
        facts.sort();
        let conn = db.conn.lock().unwrap();
        let mut rows: Vec<_> = conn
            .prepare("SELECT model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,created_at FROM proxy_request_logs WHERE app_type='hermes'")
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    [r.get::<_, u64>(1)?, r.get(2)?, r.get(3)?, r.get(4)?],
                    r.get::<_, i64>(5)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows.sort();
        assert_eq!(
            facts
                .iter()
                .map(|(m, b, at, _)| (m.clone(), *b, *at))
                .collect::<Vec<_>>(),
            rows
        );
        assert_eq!(
            facts
                .iter()
                .filter(|f| !f.3)
                .map(|f| f.0.as_str())
                .collect::<Vec<_>>(),
            ["unknown"],
            "only the residual is incomplete"
        );
    }

    #[test]
    #[serial]
    fn analytics_hermes_messages_body_free_deduped_filtered() {
        let sandbox = sandbox();
        let root = root(sandbox.home(), &["main-aux"]);
        let parsed = read(&Database::memory().unwrap(), &[root]).unwrap();
        let roles: Vec<_> = parsed
            .messages
            .iter()
            .map(|m| (m.native_id.as_str(), m.role.clone()))
            .collect();
        assert_eq!(
            roles,
            [
                ("1", Role::User),
                ("2", Role::Assistant),
                ("4", Role::Assistant)
            ]
        );
        let serialized = serde_json::to_string(&(
            &parsed.sessions,
            &parsed.facts,
            &parsed.messages,
            &parsed.native_activity,
        ))
        .unwrap();
        assert!(!serialized.contains("private"));
    }

    #[test]
    #[serial]
    fn analytics_hermes_activity_matches_magpie_rule() {
        let sandbox = sandbox();
        let root = root(sandbox.home(), &["main-aux"]);
        let parsed = read(&Database::memory().unwrap(), &[root]).unwrap();
        let times: Vec<_> = parsed
            .native_activity
            .iter()
            .map(|e| (e.at_ms - T0_MS) / 1000)
            .collect();
        assert_eq!(times, [0, 1, 4, 6, 9], "start, then messages in id order");
        let window = super::super::session_analytics_core::Window {
            start_ms: Some(T0_MS - 3_600_000),
            end_ms: T0_MS + 3_600_000,
        };
        let day = super::super::session_analytics_core::DayBoundary {
            date: "2026-10-01".into(),
            start_ms: T0_MS - 3_600_000,
            end_ms: T0_MS + 3_600_000,
        };
        let activity = super::super::session_native_activity::activity(
            &parsed.native_activity,
            &window,
            &[day],
        )
        .unwrap();
        assert_eq!(activity.active_ms, 9_000);
    }

    #[test]
    #[serial]
    fn analytics_hermes_cross_day_row_lands_on_last_seen_day() {
        let sandbox = sandbox();
        let root = root(sandbox.home(), &["cross-day"]);
        let parsed = read(&Database::memory().unwrap(), &[root]).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(parsed.facts[0].at_ms, 1_790_871_000_000);
    }

    #[test]
    #[serial]
    fn analytics_hermes_missing_database_is_an_empty_roster() {
        let sandbox = sandbox();
        let root = sandbox.home().join("hermes");
        let db = Database::memory().unwrap();
        let missing_root = read(&db, std::slice::from_ref(&root)).unwrap();
        assert!(missing_root.sessions.is_empty());
        assert_eq!(missing_root.reasons, ["sourceMissing"]);
        fs::create_dir_all(&root).unwrap();
        let missing_db = read(&db, std::slice::from_ref(&root)).unwrap();
        assert_eq!(missing_db.reasons, ["sourceMissing"]);
        fs::write(root.join("state.db"), b"not sqlite").unwrap();
        assert!(read(&db, &[root]).is_err());
    }

    #[test]
    #[serial]
    fn analytics_hermes_wal_change_changes_stamp() {
        let sandbox = sandbox();
        let root = root(sandbox.home(), &["main-aux"]);
        let before = stamp(std::slice::from_ref(&root)).unwrap();
        fs::write(root.join("state.db-wal"), b"").unwrap();
        assert_ne!(stamp(&[root]).unwrap(), before);
    }
}
