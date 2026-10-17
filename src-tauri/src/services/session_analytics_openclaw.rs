//! Body-free OpenClaw analytics retains native agent and primary authority.
use super::{
    session_analytics_index::ParsedSource,
    session_analytics_native as native, session_usage_openclaw as reader,
    usage_source::{Prepared, ProviderIndex, Snapshot},
    usage_sync_db::read_sync_state,
};
use crate::{database::Database, error::AppError, session_manager::providers::openclaw};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

fn invalid() -> AppError {
    AppError::InvalidInput("usage.analytics.sourceReadFailed".into())
}
fn root(roots: &[PathBuf]) -> Result<&Path, AppError> {
    match roots {
        [root] => Ok(root),
        _ => Err(invalid()),
    }
}
fn files(roots: &[PathBuf]) -> Result<Vec<PathBuf>, AppError> {
    openclaw::usage_files(root(roots)?).map_err(|_| invalid())
}
pub(crate) fn stamp(roots: &[PathBuf]) -> Result<Value, AppError> {
    let files = files(roots)?;
    // The physical main file alone is insufficient for a live SQLite source.
    // Reuse the normalized main/WAL proof and its symlink restrictions.
    let sources = files
        .iter()
        .map(|path| {
            Ok((
                path,
                super::usage_source::normalized_stamp("openclaw", path)?,
            ))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(json!({"native":native::roster_stamp(roots, &files)?,"sources":sources}))
}
pub(crate) fn read(db: &Database, roots: &[PathBuf]) -> Result<ParsedSource, AppError> {
    use reader::{Kind, OpenClawRead};
    let before = stamp(roots)?;
    let root = root(roots)?;
    let files = files(roots)?;
    if !root.exists() {
        let mut result = native::empty();
        result.reasons.push("sourceMissing".into());
        return Ok(result);
    }
    let index = ProviderIndex::read_roots(db, "openclaw", roots)?;
    let remembered = {
        let conn = db.conn.lock().map_err(|_| invalid())?;
        reader::remembered_primaries_on(&conn, root)?
    };
    let mut missing_agents = BTreeSet::new();
    for path in remembered.keys() {
        if files.binary_search(path).is_err() {
            missing_agents.insert(reader::location(path)?.0);
        }
    }
    let mut sources = Vec::with_capacity(files.len());
    let mut scan_bytes = 0_u64;
    for path in &files {
        let snapshot = if reader::location(path)?.1 == Kind::Database {
            Snapshot::sqlite(path)?
        } else {
            Snapshot::file(path)?
        };
        let size = if reader::location(path)?.1 == Kind::Database {
            snapshot.stamp["main"]["size"].as_u64().and_then(|main| {
                main.checked_add(snapshot.stamp["wal"]["size"].as_u64().unwrap_or(0))
            })
        } else {
            snapshot.stamp["size"].as_u64()
        }
        .ok_or_else(invalid)?;
        scan_bytes = scan_bytes
            .checked_add(size)
            .filter(|n| *n <= 512 * 1024 * 1024)
            .ok_or_else(invalid)?;
        sources.push(Prepared::with_index(
            db,
            snapshot,
            read_sync_state(db, &path.to_string_lossy())?,
            1,
            &index,
        )?);
    }
    let dependency = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&json!([
                sources
                    .iter()
                    .map(|source| (&source.snapshot.path, &source.snapshot.stamp))
                    .collect::<Vec<_>>(),
                missing_agents
            ]))
            .map_err(|_| invalid())?
        )
    );
    for source in &mut sources {
        source.observe_dependency(&dependency);
    }
    let mut primary_sessions = BTreeMap::<String, BTreeSet<String>>::new();
    let mut parsed = Vec::with_capacity(files.len());
    let mut decoded = 0_usize;
    let mut event_count = 0_usize;
    for path in &files {
        let (agent, kind) = reader::location(path)?;
        let read = if kind == Kind::Legacy && missing_agents.contains(&agent) {
            OpenClawRead::suppressed(None, 0, "nativePrimaryMissing")
        } else if kind == Kind::Legacy {
            match primary_sessions.get(&agent) {
                Some(known) => reader::shadowed_legacy(path, known)?
                    .map(Ok)
                    .unwrap_or_else(|| reader::read_openclaw(path))?,
                None => reader::read_openclaw(path)?,
            }
        } else {
            reader::read_openclaw(path)?
        };
        decoded = decoded
            .checked_add(read.decoded_bytes)
            .filter(|n| *n <= 128 * 1024 * 1024)
            .ok_or_else(invalid)?;
        event_count = event_count
            .checked_add(read.event_count as usize)
            .filter(|n| *n <= 100_000)
            .ok_or_else(invalid)?;
        if kind == Kind::Database {
            primary_sessions.insert(agent, read.sessions.clone());
        }
        parsed.push(read);
    }
    let mut result = native::empty();
    let mut reasons = BTreeSet::new();
    let mut identities = BTreeMap::new();
    let mut facts = BTreeMap::new();
    let mut sessions = BTreeMap::new();
    for (i, read) in parsed.into_iter().enumerate() {
        let (agent, kind) = reader::location(&files[i])?;
        let suppressed = kind == Kind::Legacy
            && (missing_agents.contains(&agent)
                || primary_sessions
                    .get(&agent)
                    .is_some_and(|known| read.sessions.iter().any(|sid| known.contains(sid))));
        reasons.extend(read.reasons.iter().cloned());
        if suppressed {
            reasons.insert(
                if missing_agents.contains(&agent) {
                    "nativePrimaryMissing"
                } else {
                    "primaryDatabaseSessionPreferred"
                }
                .into(),
            );
            continue;
        }
        // An unchanged sync receipt is an additional prerequisite, not a new
        // acknowledgement. Never replay a legacy source from a missing primary.
        if !sources[i].unchanged {
            return Err(invalid());
        }
        for (native, digest) in &read.native_events {
            if identities.get(native).is_some_and(|prior| prior != digest) {
                return Err(invalid());
            }
            identities.insert(native.clone(), digest.clone());
        }
        for sid in &read.sessions {
            let scope = json!([agent, sid]).to_string();
            let project = read
                .projects
                .get(sid)
                .and_then(Option::as_ref)
                .map(|project| project.as_deref());
            let mut descriptor = native::descriptor("openclaw", &scope, sid, project, None, None);
            // The manager's current OpenClaw surface is JSONL metadata only.
            // Do not route a primary database SID through a JSONL reader.
            if kind == Kind::Legacy {
                let meta = crate::session_manager::SessionMeta {
                    provider_id: "openclaw".into(),
                    session_id: sid.clone(),
                    source_path: files[i].to_str().map(str::to_owned),
                    ..Default::default()
                };
                descriptor.manager_key = Some(crate::session_manager::managed::key(&meta));
            }
            descriptor.messages_supported = false;
            descriptor.message_reasons = vec!["messagesUnavailable".into()];
            if project.is_none() {
                reasons.insert("projectNotRecorded".into());
            }
            if let Some(prior) = sessions.get(&scope) {
                let prior: &super::session_analytics::SessionDescriptor = prior;
                if prior.identity.project != descriptor.identity.project {
                    return Err(invalid());
                }
            } else {
                sessions.insert(scope, descriptor);
            }
        }
        for fact in read.facts {
            let (Some(native), Some(scope)) =
                (fact.native_key.as_deref(), fact.session_id.as_deref())
            else {
                continue;
            };
            let raw_scope: [String; 2] = serde_json::from_str(scope).map_err(|_| invalid())?;
            if raw_scope[0] != agent || read.blocked_sessions.contains(&raw_scope[1]) {
                continue;
            }
            if !sources[i].evidence.has(native) || !index.has_acknowledged(native) {
                reasons.insert("legacyUnknownBoundary".into());
                continue;
            }
            if let Some(fact) = native::normalized_fact(
                "openclaw",
                "openclaw_session",
                fact,
                read.reasons.is_empty(),
            ) {
                if facts
                    .get(&fact.event_key)
                    .is_some_and(|prior| prior != &fact)
                {
                    return Err(invalid());
                }
                facts.insert(fact.event_key.clone(), fact);
            }
        }
    }
    {
        let conn = db.conn.lock().map_err(|_| invalid())?;
        index.check_on(&conn)?;
        if reader::remembered_primaries_on(&conn, root)? != remembered {
            return Err(invalid());
        }
        for source in &sources {
            source.check_on(&conn)?;
        }
    }
    if stamp(roots)? != before {
        return Err(invalid());
    }
    if files.is_empty() && !index.origin_empty() {
        reasons.insert("sourceMissing".into());
    }
    if !missing_agents.is_empty() {
        reasons.insert("nativePrimaryMissing".into());
    }
    result.sessions = sessions.into_values().collect();
    result.facts = facts.into_values().collect();
    if !result.sessions.is_empty() {
        reasons.insert("messagesUnavailable".into());
    }
    result.reasons = reasons.into_iter().collect();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{services::session_usage_openclaw, test_sandbox::sandbox};
    use rusqlite::{params, Connection};
    use serde_json::{json, Value};
    use serial_test::serial;
    use std::path::Path;
    fn header() -> Value {
        json!({"type":"session","version":4,"id":"same","cwd":"/fixture/project","timestamp":"2026-10-15T00:00:00Z"})
    }
    fn message(input: u64) -> Value {
        json!({"type":"message","id":"a","timestamp":"2026-10-15T00:00:01Z","message":{"role":"assistant","model":"m","content":[{"type":"text","text":"private body"}],"usage":{"input":input,"output":2,"cacheRead":3,"cacheWrite":4}}})
    }
    fn legacy(root: &Path, agent: &str, input: u64) -> PathBuf {
        let path = root.join("agents").join(agent).join("sessions/same.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{}\n{}\n", header(), message(input))).unwrap();
        path
    }
    fn primary(root: &Path, agent: &str, input: u64) -> PathBuf {
        let path = root
            .join("agents")
            .join(agent)
            .join("agent/openclaw-agent.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version=24; CREATE TABLE schema_meta(meta_key TEXT PRIMARY KEY,role TEXT,schema_version INTEGER,agent_id TEXT); CREATE TABLE session_windows(session_id TEXT PRIMARY KEY,reason TEXT); CREATE TABLE transcript_events(session_id TEXT,seq INTEGER,event_json TEXT,event_zstd BLOB,event_utf8_bytes INTEGER); CREATE TABLE session_transcript_archives(session_id TEXT); CREATE TABLE session_transcript_cold_archives(session_id TEXT);").unwrap();
        conn.execute(
            "INSERT INTO schema_meta VALUES('primary','agent',24,?1)",
            [agent],
        )
        .unwrap();
        conn.execute("INSERT INTO session_windows VALUES('same','initial')", [])
            .unwrap();
        for (seq, value) in [header(), message(input)].into_iter().enumerate() {
            let raw = value.to_string();
            conn.execute(
                "INSERT INTO transcript_events VALUES('same',?1,?2,NULL,?3)",
                params![seq as i64, raw, raw.len() as i64],
            )
            .unwrap();
        }
        path
    }
    #[test]
    #[serial]
    fn analytics_openclaw_primary_wins_and_agent_scoped_sessions_stay_distinct() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("openclaw");
        primary(&root, "main", 20);
        legacy(&root, "main", 999);
        legacy(&root, "other", 30);
        let db = Database::memory().unwrap();
        session_usage_openclaw::sync_openclaw_usage_at(&db, &root).unwrap();
        let writes: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get(0))
            .unwrap();
        let parsed = read(&db, &[root]).unwrap();
        let after: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get(0))
            .unwrap();
        assert_eq!(after, writes);
        assert_eq!(parsed.sessions.len(), 2);
        assert_ne!(
            parsed.sessions[0].identity.key,
            parsed.sessions[1].identity.key
        );
        assert_eq!(
            parsed.facts.iter().map(|fact| fact.buckets[0]).sum::<u64>(),
            50
        );
        assert!(parsed
            .sessions
            .iter()
            .all(|session| !session.messages_supported));
        assert!(parsed.messages.is_empty());
        assert!(!serde_json::to_string(&parsed.sessions)
            .unwrap()
            .contains("private body"));
    }
    #[test]
    #[serial]
    fn analytics_openclaw_missing_remembered_primary_never_resurrects_legacy() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("openclaw");
        let path = primary(&root, "main", 20);
        legacy(&root, "main", 999);
        let db = Database::memory().unwrap();
        session_usage_openclaw::sync_openclaw_usage_at(&db, &root).unwrap();
        std::fs::remove_file(path).unwrap();
        session_usage_openclaw::sync_openclaw_usage_at(&db, &root).unwrap();
        let parsed = read(&db, &[root]).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed
            .reasons
            .iter()
            .any(|reason| reason == "nativePrimaryMissing"));
    }
    #[test]
    #[serial]
    fn analytics_openclaw_unsynchronized_source_change_does_not_claim_verified_usage() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("openclaw");
        legacy(&root, "main", 20);
        let db = Database::memory().unwrap();
        session_usage_openclaw::sync_openclaw_usage_at(&db, &root).unwrap();
        legacy(&root, "main", 999);
        assert!(read(&db, &[root]).is_err());
    }
    #[test]
    fn analytics_openclaw_stamp_observes_roster_without_decoding_transcript_bodies() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("openclaw");
        let path = legacy(&root, "main", 20);
        let before = stamp(std::slice::from_ref(&root)).unwrap();
        std::fs::write(path, "malformed private body").unwrap();
        assert_ne!(before, stamp(&[root]).unwrap());
    }
    #[test]
    #[serial]
    fn analytics_openclaw_compressed_primary_retains_project_and_archive_gap() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("openclaw");
        let path = primary(&root, "main", 20);
        let conn = Connection::open(path).unwrap();
        let raw = message(20).to_string();
        let blob = zstd::stream::encode_all(raw.as_bytes(), 1).unwrap();
        conn.execute("UPDATE transcript_events SET event_json=NULL,event_zstd=?1,event_utf8_bytes=?2 WHERE seq=1", params![blob,raw.len() as i64]).unwrap();
        conn.execute(
            "INSERT INTO session_transcript_cold_archives VALUES('cold')",
            [],
        )
        .unwrap();
        drop(conn);
        let db = Database::memory().unwrap();
        session_usage_openclaw::sync_openclaw_usage_at(&db, &root).unwrap();
        let parsed = read(&db, &[root]).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        let current = parsed
            .sessions
            .iter()
            .find(|session| session.title == "same")
            .unwrap();
        assert_eq!(current.project_path.as_deref(), Some("/fixture/project"));
        let archived = parsed
            .sessions
            .iter()
            .find(|session| session.title == "cold")
            .unwrap();
        assert_eq!(
            archived.identity.project,
            super::super::session_analytics_core::Project::Unparsed
        );
        assert!(parsed
            .reasons
            .iter()
            .any(|reason| reason == "nativeArchiveNotEnumerated"));
    }
    #[test]
    #[serial]
    fn analytics_openclaw_unknown_fork_seed_never_receives_parent_usage() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("openclaw");
        let path = primary(&root, "main", 20);
        let mut head = header();
        head["parentSession"] = json!("parent");
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "UPDATE transcript_events SET event_json=?1,event_utf8_bytes=?2 WHERE seq=0",
            params![head.to_string(), head.to_string().len() as i64],
        )
        .unwrap();
        drop(conn);
        let db = Database::memory().unwrap();
        session_usage_openclaw::sync_openclaw_usage_at(&db, &root).unwrap();
        let parsed = read(&db, &[root]).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed
            .sessions
            .iter()
            .all(|session| session.identity.verified_root_key.is_none()));
        assert!(parsed
            .reasons
            .iter()
            .any(|reason| reason == "forkProvenanceUnverified"));
    }
    #[test]
    #[serial]
    fn analytics_openclaw_failed_receipt_validation_never_repairs_database_proof() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("openclaw");
        let path = legacy(&root, "main", 20);
        let db = Database::memory().unwrap();
        session_usage_openclaw::sync_openclaw_usage_at(&db, &root).unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE session_log_sync SET last_line_offset=last_line_offset+1 WHERE file_path=?1",
            [path.to_string_lossy().into_owned()],
        )
        .unwrap();
        let before: i64 = conn
            .query_row("SELECT total_changes()", [], |row| row.get(0))
            .unwrap();
        drop(conn);
        assert!(read(&db, &[root]).is_err());
        let after: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, after);
    }
    #[test]
    fn analytics_openclaw_missing_root_is_distinct_from_an_empty_present_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("missing-openclaw");
        let db = Database::memory().unwrap();
        assert_eq!(
            read(&db, std::slice::from_ref(&root)).unwrap().reasons,
            vec!["sourceMissing"]
        );
        std::fs::create_dir_all(&root).unwrap();
        assert!(read(&db, &[root]).unwrap().reasons.is_empty());
    }
    #[test]
    #[serial]
    fn analytics_openclaw_disappeared_last_source_keeps_an_explicit_gap() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("openclaw");
        let path = legacy(&root, "main", 20);
        let db = Database::memory().unwrap();
        session_usage_openclaw::sync_openclaw_usage_at(&db, &root).unwrap();
        std::fs::remove_file(path).unwrap();
        let parsed = read(&db, &[root]).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed
            .reasons
            .iter()
            .any(|reason| reason == "sourceMissing"));
    }
}
