//! Body-free dsh analytics over a verified complete native generation roster.
use super::{
    session_analytics_index::ParsedSource,
    session_analytics_native as native, session_usage_dsh as reader,
    usage_source::{self, Prepared, ProviderIndex, Snapshot},
    usage_sync_db::read_sync_state,
};
use crate::{
    database::Database,
    error::AppError,
    session_manager::{providers::dsh, source_cache::DshNativeGenerations},
};
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
fn files(roots: &[PathBuf]) -> Result<Vec<(PathBuf, u64)>, AppError> {
    dsh::usage_files(root(roots)?).map_err(|_| invalid())
}
pub(crate) fn stamp(roots: &[PathBuf]) -> Result<Value, AppError> {
    let files = files(roots)?;
    let paths = files
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    Ok(json!({"native":native::roster_stamp(roots, &paths)?,"generations":files}))
}
pub(crate) fn read(db: &Database, roots: &[PathBuf]) -> Result<ParsedSource, AppError> {
    let before = stamp(roots)?;
    let files = files(roots)?;
    let root = root(roots)?;
    if !root.exists() {
        let mut result = native::empty();
        result.reasons.push("sourceMissing".into());
        return Ok(result);
    }
    let index = ProviderIndex::read_roots(db, "dsh", roots)?;
    let generations = {
        let conn = db.conn.lock().map_err(|_| invalid())?;
        DshNativeGenerations::load_on(&conn, root, index.origin_empty())?
    };
    let dependency = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&files).map_err(|_| invalid())?)
    );
    let mut sources = Vec::with_capacity(files.len());
    let mut parsed = Vec::with_capacity(files.len());
    let mut latest = BTreeMap::<String, (u64, usize)>::new();
    let mut projects = BTreeMap::<String, BTreeSet<Option<String>>>::new();
    let mut winners = BTreeMap::<String, (u64, usize, usage_source::NormalizedUsageFact)>::new();
    let mut replicas = BTreeMap::<(String, u64), Value>::new();
    let mut bytes = 0_u64;
    let mut events = 0_usize;
    let mut result = native::empty();
    let mut reasons = BTreeSet::new();
    for (i, (path, generation)) in files.iter().enumerate() {
        let mut source = Prepared::with_index(
            db,
            Snapshot::file(path)?,
            read_sync_state(db, &path.to_string_lossy())?,
            super::session_usage_dsh::PARSER_REVISION,
            &index,
        )?;
        source.observe_dependency(&dependency);
        // This projection does not establish ingestion or rebuild a history
        // receipt. A changed source must first pass the native batch sync.
        if !source.unchanged {
            return Err(invalid());
        }
        let read = reader::read_dsh(path)?;
        bytes = bytes
            .checked_add(read.decoded_bytes)
            .filter(|n| *n <= 128 * 1024 * 1024)
            .ok_or_else(invalid)?;
        events = events
            .checked_add(read.facts.len())
            .filter(|n| *n <= 100_000)
            .ok_or_else(invalid)?;
        reasons.extend(read.reasons.iter().cloned());
        if read.supported_version {
            if let Some(sid) = &read.session_id {
                if let Some(project) = read
                    .metadata
                    .as_ref()
                    .and_then(|meta| meta.project.as_ref())
                {
                    projects
                        .entry(sid.clone())
                        .or_default()
                        .insert(project.clone());
                }
                match latest.get(sid) {
                    Some((prior, prior_index)) if prior == generation => {
                        let previous: &reader::DshRead = &parsed[*prior_index];
                        if previous.metadata != read.metadata
                            || previous.parent_session_id != read.parent_session_id
                        {
                            return Err(invalid());
                        }
                    }
                    Some((prior, _)) if prior > generation => {}
                    _ => {
                        latest.insert(sid.clone(), (*generation, i));
                    }
                }
            }
        }
        for fact in &read.facts {
            let Some(native) = &fact.native_key else {
                continue;
            };
            let value = serde_json::to_value(fact).map_err(|_| invalid())?;
            let replica_key = (native.clone(), *generation);
            if replicas
                .get(&replica_key)
                .is_some_and(|prior| prior != &value)
            {
                return Err(invalid());
            }
            replicas.insert(replica_key, value);
            if winners
                .get(native)
                .is_none_or(|(prior, _, _)| prior < generation)
            {
                winners.insert(native.clone(), (*generation, i, fact.clone()));
            }
        }
        sources.push(source);
        parsed.push(read);
    }
    let allowed = latest
        .iter()
        .filter_map(|(sid, (generation, _))| {
            if generations.available() && generations.session_allows(sid, *generation) {
                Some(sid.clone())
            } else {
                reasons.insert(
                    if generations.available() {
                        "nativeGenerationRollback"
                    } else {
                        "nativeGenerationLeaseUnavailable"
                    }
                    .into(),
                );
                None
            }
        })
        .collect::<BTreeSet<_>>();
    for (sid, (_, i)) in &latest {
        if !allowed.contains(sid) {
            continue;
        }
        let read = &parsed[*i];
        let Some(meta) = &read.metadata else {
            continue;
        };
        let seed_valid = !read.reasons.iter().any(|reason| {
            matches!(
                reason.as_str(),
                "seedBoundaryUnavailable" | "seedBoundaryIncomplete"
            )
        });
        if meta.delegated && seed_valid && read.parent_session_id.as_deref() == Some(sid.as_str()) {
            // A self-parent is invalid native lineage. The shared descriptor
            // intentionally omits same-ID roots, so reject it before that
            // projection could silently turn the child into a main session.
            return Err(invalid());
        }
        let parent = if meta.delegated && seed_valid {
            read.parent_session_id
                .as_deref()
                .filter(|sid| allowed.contains(*sid))
        } else {
            None
        };
        let title = meta
            .title
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(sid);
        let conflicting_project = projects.get(sid).is_some_and(|values| values.len() > 1);
        let project = if conflicting_project {
            reasons.insert("conflictingSessionProject".into());
            None
        } else {
            meta.project.as_ref().map(|project| project.as_deref())
        };
        let mut descriptor =
            native::descriptor("dsh", sid, title, project, parent, files[*i].0.to_str());
        descriptor.messages_supported = false;
        descriptor.message_reasons = vec!["messagesUnavailable".into()];
        if meta.project.is_none() {
            reasons.insert("projectNotRecorded".into());
        }
        result.sessions.push(descriptor);
    }
    for (native_id, (generation, i, fact)) in winners {
        let Some(sid) = fact.session_id.as_deref() else {
            continue;
        };
        if !allowed.contains(sid) || !generations.event_allows(sid, &native_id, generation) {
            reasons.insert("nativeGenerationRollback".into());
            continue;
        }
        if parsed[i].reasons.iter().any(|reason| {
            matches!(
                reason.as_str(),
                "seedBoundaryUnavailable" | "seedBoundaryIncomplete"
            )
        }) {
            continue;
        }
        if !sources[i].evidence.has(&native_id) || !index.has_acknowledged(&native_id) {
            reasons.insert("legacyUnknownBoundary".into());
            continue;
        }
        if let Some(fact) =
            native::normalized_fact("dsh", "dsh_session", fact, parsed[i].reasons.is_empty())
        {
            result.facts.push(fact);
        }
    }
    // The generation lease and every source receipt are checked together. No
    // ordering state, ledger row, checkpoint or acknowledgement is written.
    {
        let conn = db.conn.lock().map_err(|_| invalid())?;
        generations.check_on(&conn)?;
        index.check_on(&conn)?;
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
    let identities = result
        .sessions
        .iter()
        .map(|session| session.identity.clone())
        .collect::<Vec<_>>();
    if super::session_analytics_core::resolve_roots(&identities).is_err() {
        return Err(invalid());
    }
    if !result.sessions.is_empty() {
        reasons.insert("messagesUnavailable".into());
    }
    result.reasons = reasons.into_iter().collect();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{services::session_usage_dsh, test_sandbox::sandbox};
    use serde_json::{json, Value};
    use serial_test::serial;
    use std::path::Path;
    const AT: i64 = 1_792_053_987_262;
    fn write(root: &Path, generation: u32, input: u64, seed: bool) -> PathBuf {
        let path =
            crate::test_sandbox::native(root.join("sessions/project/s")).join(if generation == 0 {
                "session.jsonl".into()
            } else {
                format!("session.v{generation}.jsonl")
            });
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut header = json!({"type":"session","version":0,"id":"s","createdAt":AT,"delegationDepth":0,"cwd":"/fixture/project"});
        if seed {
            header["isSeeded"] = json!(true);
        }
        let rows = [
            header,
            json!({"type":"assistant/message","seq":0,"time":AT,"surfaceOp":"append","data":{"message":{"id":"a","role":"assistant","source":{"kind":"model","model":"m"},"content":[{"type":"text","text":"private body"}]},"usage":{"inputTokens":input,"outputTokens":2,"cacheReadTokens":3,"cacheWriteTokens":4}}}),
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
    #[serial]
    fn analytics_dsh_verified_latest_generation_and_project_without_invented_messages() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        write(&root, 0, 10, false);
        write(&root, 1, 20, false);
        let db = Database::memory().unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
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
        assert_eq!(parsed.sessions.len(), 1);
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(parsed.facts[0].buckets, [20, 2, 3, 4]);
        assert_eq!(
            parsed.sessions[0].project_path.as_deref(),
            Some("/fixture/project")
        );
        assert!(!parsed.sessions[0].messages_supported);
        assert!(parsed.messages.is_empty());
        assert!(!serde_json::to_string(&parsed.sessions)
            .unwrap()
            .contains("private body"));
    }
    #[test]
    #[serial]
    fn analytics_dsh_deleted_new_generation_never_revives_old_usage() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        write(&root, 0, 10, false);
        let latest = write(&root, 2, 40, false);
        let db = Database::memory().unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        std::fs::remove_file(latest).unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        let parsed = read(&db, &[root]).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed
            .reasons
            .iter()
            .any(|reason| reason == "nativeGenerationRollback"));
    }
    #[test]
    #[serial]
    fn analytics_dsh_unknown_seed_boundary_does_not_count_inherited_usage() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        write(&root, 0, 10, true);
        let db = Database::memory().unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        let parsed = read(&db, &[root]).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed
            .reasons
            .iter()
            .any(|reason| reason == "seedBoundaryUnavailable"));
    }
    #[test]
    #[serial]
    fn analytics_dsh_requires_current_ingestion_proof_and_stat_stamp_reads_no_body() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        let path = write(&root, 0, 10, false);
        let db = Database::memory().unwrap();
        assert!(read(&db, std::slice::from_ref(&root)).is_err());
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        write(&root, 0, 999, false);
        assert!(read(&db, std::slice::from_ref(&root)).is_err());
        std::fs::write(path, "malformed private body").unwrap();
        assert!(
            stamp(&[root]).is_ok(),
            "discovery must not decode transcripts"
        );
    }
    #[test]
    #[serial]
    fn analytics_dsh_compressed_twins_deduplicate_but_conflicting_project_is_unknown() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        let path = write(&root, 0, 10, false);
        let raw = std::fs::read(&path).unwrap();
        let compressed = path.with_extension("jsonl.zstd");
        std::fs::write(
            &compressed,
            zstd::stream::encode_all(raw.as_slice(), 1).unwrap(),
        )
        .unwrap();
        let db = Database::memory().unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        let parsed = read(&db, std::slice::from_ref(&root)).unwrap();
        assert_eq!(parsed.sessions.len(), 1);
        assert_eq!(parsed.facts.len(), 1);
        let changed = String::from_utf8(raw)
            .unwrap()
            .replace("/fixture/project", "/fixture/other");
        std::fs::write(
            &compressed,
            zstd::stream::encode_all(changed.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        assert!(
            read(&db, &[root]).is_err(),
            "equal generation cannot choose an arbitrary project"
        );
    }
    #[test]
    #[serial]
    fn analytics_dsh_verified_seed_excludes_inheritance_and_keeps_native_parent() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        let path = write(&root, 0, 10, false);
        let mut rows = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        rows[0]["id"] = json!("parent");
        rows[1]["seq"] = json!(0);
        let parent = root.join("sessions/project/parent/session.jsonl");
        std::fs::create_dir_all(parent.parent().unwrap()).unwrap();
        std::fs::write(
            &parent,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();
        rows[0]["id"] = json!("s");
        rows[0]["parentSession"] = json!("parent");
        rows[0]["origin"] = json!("subagent");
        rows[0]["delegationDepth"] = json!(1);
        rows[0]["seedLength"] = json!(1);
        rows.push(json!({"type":"session/end-seed","seq":1,"time":AT,"data":{}}));
        let mut own = rows[1].clone();
        own["seq"] = json!(2);
        own["data"]["message"]["id"] = json!("own");
        own["data"]["usage"]["inputTokens"] = json!(20);
        rows.push(own);
        std::fs::write(
            &path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();
        let db = Database::memory().unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        let parsed = read(&db, &[root]).unwrap();
        assert_eq!(parsed.facts.len(), 2);
        assert_eq!(
            parsed.facts.iter().map(|fact| fact.buckets[0]).sum::<u64>(),
            30
        );
        let parent = parsed
            .sessions
            .iter()
            .find(|session| session.native_session_id == "parent")
            .unwrap();
        let child = parsed
            .sessions
            .iter()
            .find(|session| session.native_session_id == "s")
            .unwrap();
        assert_eq!(
            child.identity.verified_root_key.as_deref(),
            Some(parent.identity.key.as_str())
        );
    }
    #[test]
    #[serial]
    fn analytics_dsh_failed_receipt_validation_never_repairs_database_proof() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        let path = write(&root, 0, 10, false);
        let db = Database::memory().unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
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
    fn analytics_dsh_missing_root_is_distinct_from_an_empty_present_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("missing-dsh");
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
    fn analytics_dsh_cross_generation_project_changes_are_not_guessed() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        write(&root, 0, 10, false);
        let newer = write(&root, 1, 20, false);
        let text = std::fs::read_to_string(&newer)
            .unwrap()
            .replace("/fixture/project", "/fixture/changed");
        std::fs::write(newer, text).unwrap();
        let db = Database::memory().unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        let parsed = read(&db, &[root]).unwrap();
        assert_eq!(parsed.facts[0].buckets[0], 20);
        assert_eq!(
            parsed.sessions[0].identity.project,
            super::super::session_analytics_core::Project::Unparsed
        );
        assert!(parsed.sessions[0].project_path.is_none());
        assert!(parsed
            .reasons
            .iter()
            .any(|reason| reason == "conflictingSessionProject"));
    }
    #[test]
    #[serial]
    fn analytics_dsh_cyclic_native_parent_chain_cannot_poison_the_projection() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        let template = write(&root, 0, 10, false);
        let rows = std::fs::read_to_string(&template)
            .unwrap()
            .lines()
            .map(|row| serde_json::from_str::<Value>(row).unwrap())
            .collect::<Vec<_>>();
        std::fs::remove_file(template).unwrap();
        for (sid, parent) in [("first", "second"), ("second", "first")] {
            let mut head = rows[0].clone();
            head["id"] = json!(sid);
            head["parentSession"] = json!(parent);
            head["origin"] = json!("subagent");
            head["delegationDepth"] = json!(1);
            head["seedLength"] = json!(0);
            let mut own = rows[1].clone();
            own["seq"] = json!(1);
            let content = [
                head,
                json!({"type":"session/end-seed","seq":0,"time":AT,"data":{}}),
                own,
            ];
            let path = root
                .join("sessions/project")
                .join(sid)
                .join("session.jsonl");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                path,
                content
                    .iter()
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n",
            )
            .unwrap();
        }
        let db = Database::memory().unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        assert!(
            read(&db, &[root]).is_err(),
            "a cycle must be rejected before publishing a query cache"
        );
    }
    #[test]
    #[serial]
    fn analytics_dsh_self_parent_cannot_be_silently_promoted_to_a_root() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        let path = write(&root, 0, 10, false);
        let mut rows = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|row| serde_json::from_str::<Value>(row).unwrap())
            .collect::<Vec<_>>();
        rows[0]["parentSession"] = json!("s");
        rows[0]["origin"] = json!("subagent");
        rows[0]["delegationDepth"] = json!(1);
        rows[0]["seedLength"] = json!(0);
        rows[1]["seq"] = json!(1);
        rows.insert(
            1,
            json!({"type":"session/end-seed","seq":0,"time":AT,"data":{}}),
        );
        std::fs::write(
            path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();
        let db = Database::memory().unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        assert!(
            read(&db, &[root]).is_err(),
            "a verified self-parent is a cycle, not evidence of a main session"
        );
    }
    #[test]
    #[serial]
    fn analytics_dsh_disappeared_last_source_keeps_an_explicit_gap() {
        let _sandbox = sandbox();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("dsh");
        let path = write(&root, 0, 10, false);
        let db = Database::memory().unwrap();
        session_usage_dsh::sync_dsh_usage_at(&db, &root).unwrap();
        std::fs::remove_file(path).unwrap();
        let parsed = read(&db, &[root]).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed
            .reasons
            .iter()
            .any(|reason| reason == "sourceMissing"));
    }
}
