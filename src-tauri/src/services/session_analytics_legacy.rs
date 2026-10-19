//! Read-only, explicitly legacy Codex projection used until a native generation
//! is accepted for the current ledger epoch. No native identity is inferred.
use crate::{
    error::AppError,
    services::{
        session_analytics::{AnalyticsInput, SessionDescriptor, SourceObservation, SourceReason},
        session_analytics_core::{Project, SessionIdentity, UsageFact},
        usage_query::SourceHealth,
    },
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ROWS: usize = 100_000;
const MAX_BYTES: usize = 128 * 1024 * 1024;

pub(crate) struct LegacySnapshot {
    pub digest: String,
    pub failed: bool,
    sessions: Vec<SessionDescriptor>,
    facts: Vec<UsageFact>,
    rollups: bool,
}

pub(crate) fn load_on(conn: &Connection) -> Result<LegacySnapshot, AppError> {
    load_bounded(conn, MAX_ROWS, MAX_BYTES)
}

fn load_bounded(
    conn: &Connection,
    max_rows: usize,
    max_bytes: usize,
) -> Result<LegacySnapshot, AppError> {
    // The projection reads the original counters and IDs, never saved prices,
    // HTTP measurements, request bodies or credentials. One statement supplies
    // the facts and their digest so a warm cache cannot hide changed old rows.
    let mut statement = conn.prepare(
        "SELECT request_id,session_id,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,created_at
         FROM proxy_request_logs WHERE app_type='codex' AND data_source='codex_session'
         ORDER BY request_id LIMIT ?1",
    ).map_err(sql_error)?;
    let limit = i64::try_from(max_rows)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| invalid("legacyProjectionBudgetExceeded"))?;
    let mut rows = statement.query([limit]).map_err(sql_error)?;
    let mut facts = Vec::new();
    let mut sessions = BTreeMap::new();
    let mut bytes = 0usize;
    let mut hash = Sha256::new();
    hash.update(b"ofox-codex-legacy-v1");
    while let Some(row) = rows.next().map_err(sql_error)? {
        if facts.len() >= max_rows {
            return Err(invalid("legacyProjectionBudgetExceeded"));
        }
        // Inspect borrowed SQLite values before allocating a possibly enormous
        // old ID or model. The accounting includes the projected keys/metadata.
        let mut row_bytes = 768usize;
        for index in [0, 1, 2] {
            let value = row.get_ref(index).map_err(sql_error)?;
            let len = match value {
                rusqlite::types::ValueRef::Null if index == 1 => 0,
                rusqlite::types::ValueRef::Text(value) => value.len(),
                _ => return Err(invalid("legacyHistoryInvalidValue")),
            };
            row_bytes = row_bytes
                .checked_add(len)
                .ok_or_else(|| invalid("legacyProjectionBudgetExceeded"))?;
        }
        bytes = bytes
            .checked_add(row_bytes)
            .ok_or_else(|| invalid("legacyProjectionBudgetExceeded"))?;
        if bytes > max_bytes {
            return Err(invalid("legacyProjectionBudgetExceeded"));
        }
        let request_id: String = row.get(0).map_err(sql_error)?;
        let sid: Option<String> = row.get(1).map_err(sql_error)?;
        let model: String = row.get(2).map_err(sql_error)?;
        let mut buckets = [0; 4];
        for (index, bucket) in buckets.iter_mut().enumerate() {
            let value: i64 = row.get(index + 3).map_err(sql_error)?;
            *bucket = u64::try_from(value).map_err(|_| invalid("legacyHistoryInvalidCounter"))?;
        }
        let seconds: i64 = row.get(7).map_err(sql_error)?;
        let at_ms = seconds
            .checked_mul(1000)
            .ok_or_else(|| invalid("legacyHistoryInvalidTimestamp"))?;
        hash_text(&mut hash, &request_id);
        hash.update([u8::from(sid.is_some())]);
        if let Some(sid) = &sid {
            hash_text(&mut hash, sid);
        }
        hash_text(&mut hash, &model);
        for value in buckets {
            hash.update(value.to_be_bytes());
        }
        hash.update(seconds.to_be_bytes());
        let session_key = sid.as_deref().filter(|sid| !sid.is_empty()).map(|sid| {
            let key = key("ofox-codex-legacy-session-v1", sid);
            sessions
                .entry(key.clone())
                .or_insert_with(|| SessionDescriptor {
                    identity: SessionIdentity {
                        key: key.clone(),
                        tool_id: "codex".into(),
                        project: Project::Unparsed,
                        verified_root_key: None,
                    },
                    // A stored billing SID is not proof of a native thread ID.
                    native_session_id: String::new(),
                    title: format!("Codex · {}", &key[..8]),
                    project_label: None,
                    project_path: None,
                    manager_key: None,
                    source_available: true,
                    messages_supported: false,
                    message_reasons: vec!["statisticsRebuildRequired".into()],
                });
            key
        });
        facts.push(UsageFact {
            event_key: key("ofox-codex-legacy-request-v1", &request_id),
            session_key,
            tool_id: "codex".into(),
            source_id: "codex_session".into(),
            model,
            at_ms,
            buckets,
            // Schema v11 stores all four counters as non-null integers. Zero
            // is an actual stored value; this does not certify native coverage.
            complete: true,
            cache_write_1h: 0,
        });
    }
    let rollups: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM usage_daily_rollups WHERE app_type='codex' AND provider_id='_codex_session')",[],|row|row.get(0)).map_err(sql_error)?;
    hash.update([u8::from(rollups)]);
    hash.update((facts.len() as u64).to_be_bytes());
    Ok(LegacySnapshot {
        digest: format!("{:x}", hash.finalize()),
        failed: false,
        sessions: sessions.into_values().collect(),
        facts,
        rollups,
    })
}

impl LegacySnapshot {
    pub(crate) fn unavailable() -> Self {
        Self {
            digest: key("ofox-codex-legacy-unavailable-v1", "unavailable"),
            failed: true,
            sessions: vec![],
            facts: vec![],
            rollups: false,
        }
    }
    pub(crate) fn apply(&self, input: &mut AnalyticsInput) -> Result<(), AppError> {
        if input.codex_native_ready {
            return Ok(());
        }
        let prior_facts = input
            .facts
            .iter()
            .filter(|fact| fact.tool_id == "codex")
            .collect::<Vec<_>>();
        let prior_sessions = input
            .sessions
            .iter()
            .filter(|session| session.identity.tool_id == "codex")
            .collect::<Vec<_>>();
        let already_applied = prior_facts.len() == self.facts.len()
            && prior_facts.into_iter().eq(self.facts.iter())
            && prior_sessions.len() == self.sessions.len()
            && prior_sessions
                .into_iter()
                .zip(&self.sessions)
                .all(|(prior, next)| {
                    prior.identity == next.identity
                        && prior.native_session_id.is_empty()
                        && prior.manager_key.is_none()
                        && !prior.messages_supported
                })
            && input.source_reasons.iter().any(|reason| {
                reason.tool_id == "codex" && reason.reason == "statisticsRebuildRequired"
            })
            && self.rollups
                == input.source_reasons.iter().any(|reason| {
                    reason.tool_id == "codex" && reason.reason == "legacyCodexHistoryUnverified"
                })
            && self.failed
                == input
                    .source_observations
                    .iter()
                    .any(|source| source.tool_id == "codex" && source.status == "failed");
        if already_applied {
            return Ok(());
        }
        let removed: BTreeSet<_> = input
            .sessions
            .iter()
            .filter(|session| session.identity.tool_id == "codex")
            .map(|session| session.identity.key.clone())
            .chain(
                input
                    .facts
                    .iter()
                    .filter(|fact| fact.tool_id == "codex")
                    .filter_map(|fact| fact.session_key.clone()),
            )
            .collect();
        let has_codex = !removed.is_empty()
            || input.sources.iter().any(|source| source.tool_id == "codex")
            || input
                .source_reasons
                .iter()
                .any(|reason| reason.tool_id == "codex")
            || input
                .source_observations
                .iter()
                .any(|source| source.tool_id == "codex")
            || !self.facts.is_empty()
            || self.rollups
            || self.failed;
        input
            .sessions
            .retain(|session| session.identity.tool_id != "codex");
        input.facts.retain(|fact| fact.tool_id != "codex");
        input
            .messages
            .retain(|message| !removed.contains(&message.session_key));
        input
            .native_activity
            .retain(|event| !removed.contains(&event.session_key));
        input
            .source_reasons
            .retain(|reason| reason.tool_id != "codex");
        input
            .source_observations
            .retain(|source| source.tool_id != "codex");
        input.sources.retain(|source| source.tool_id != "codex");
        input.sessions.extend(self.sessions.iter().cloned());
        input.facts.extend(self.facts.iter().cloned());
        input.legacy_history_available |= !self.facts.is_empty() || self.rollups;
        if !self.facts.is_empty() {
            input
                .reasons
                .retain(|reason| reason != "analyticsIndexEmpty");
        }
        if has_codex {
            input.source_reasons.push(SourceReason {
                tool_id: "codex".into(),
                source_id: "codex_session".into(),
                reason: "statisticsRebuildRequired".into(),
            });
            if self.rollups {
                input.source_reasons.push(SourceReason {
                    tool_id: "codex".into(),
                    source_id: "codex_session".into(),
                    reason: "legacyCodexHistoryUnverified".into(),
                });
            }
            if self.failed {
                input.source_reasons.push(SourceReason {
                    tool_id: "codex".into(),
                    source_id: "codex_session".into(),
                    reason: "legacyCodexHistoryUnavailable".into(),
                });
            }
            input.source_observations.push(SourceObservation {
                tool_id: "codex".into(),
                source_id: "codex_session".into(),
                status: if self.failed { "failed" } else { "partial" }.into(),
                dated: !self.failed && !self.facts.is_empty() && !self.rollups,
            });
            input.sources.push(SourceHealth {
                tool_id: "codex".into(),
                source_id: "codex_session".into(),
                status: if self.failed { "failed" } else { "partial" }.into(),
                last_attempt_at_ms: None,
                last_success_at_ms: None,
                error_code: Some(
                    if self.failed {
                        "legacyCodexHistoryUnavailable"
                    } else {
                        "statisticsRebuildRequired"
                    }
                    .into(),
                ),
                last_scan: None,
                retained_detail_range: None,
            });
        }
        let mut hash = Sha256::new();
        hash.update(b"ofox-session-analytics-legacy-view-v1");
        hash_text(&mut hash, &input.snapshot_version);
        hash_text(&mut hash, &self.digest);
        input.snapshot_version = format!("{:x}", hash.finalize());
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn apply_on(conn: &Connection, input: &mut AnalyticsInput) -> Result<String, AppError> {
    if input.codex_native_ready {
        return Ok(String::new());
    }
    let snapshot = load_on(conn).unwrap_or_else(|_| LegacySnapshot::unavailable());
    snapshot.apply(input)?;
    Ok(snapshot.digest)
}

fn hash_text(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value.as_bytes());
}
fn key(domain: &str, value: &str) -> String {
    let mut hash = Sha256::new();
    hash_text(&mut hash, domain);
    hash_text(&mut hash, value);
    format!("{:x}", hash.finalize())
}
fn invalid(reason: &str) -> AppError {
    AppError::Message(format!("Session analytics legacy: {reason}"))
}
fn sql_error(_error: rusqlite::Error) -> AppError {
    invalid("legacyHistoryReadFailed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        database::Database,
        services::{
            session_analytics::{SessionDescriptor, SourceObservation, SourceReason},
            session_analytics_core::{MessageEvent, Project, Role, SessionIdentity, UsageFact},
            session_analytics_index::{self, ProjectionCache, SourceSpec},
            session_native_activity::NativeActivityEvent,
            usage_data,
            usage_query::{SourceHealth, UsageQueryV1},
        },
    };
    use serde_json::json;

    fn input() -> AnalyticsInput {
        let sessions = ["codex", "claude"].map(|tool| SessionDescriptor {
            identity: SessionIdentity {
                key: format!("{tool}-native"),
                tool_id: tool.into(),
                project: Project::Known(format!("/{tool}/project")),
                verified_root_key: None,
            },
            native_session_id: format!("{tool}-native-id"),
            title: format!("{tool} native title"),
            project_label: Some(tool.into()),
            project_path: Some(format!("/{tool}/project")),
            manager_key: Some(format!("{tool}-manager")),
            source_available: true,
            messages_supported: true,
            message_reasons: vec![],
        });
        AnalyticsInput {
            snapshot_version: "unaccepted-native".into(),
            sessions: sessions.into(),
            facts: ["codex", "claude"]
                .map(|tool| UsageFact {
                    event_key: format!("{tool}-native-event"),
                    session_key: Some(format!("{tool}-native")),
                    tool_id: tool.into(),
                    source_id: format!("{tool}_session"),
                    model: "native-model".into(),
                    at_ms: 2000,
                    buckets: [80, 10, 20, 0],
                    complete: true,
                    cache_write_1h: 0,
                })
                .into(),
            messages: ["codex", "claude"]
                .map(|tool| MessageEvent {
                    session_key: format!("{tool}-native"),
                    native_id: format!("{tool}-message"),
                    at_ms: 2000,
                    role: Role::User,
                    main: true,
                })
                .into(),
            reasons: vec![],
            source_reasons: ["codex", "claude"]
                .map(|tool| SourceReason {
                    tool_id: tool.into(),
                    source_id: format!("{tool}_session"),
                    reason: "native-reason".into(),
                })
                .into(),
            sources: ["codex", "claude"]
                .map(|tool| SourceHealth {
                    tool_id: tool.into(),
                    source_id: format!("{tool}_session"),
                    status: "ok".into(),
                    last_attempt_at_ms: Some(2000),
                    last_success_at_ms: Some(2000),
                    error_code: None,
                    last_scan: None,
                    retained_detail_range: None,
                })
                .into(),
            native_activity: ["codex", "claude"]
                .map(|tool| NativeActivityEvent {
                    session_key: format!("{tool}-native"),
                    event_key: format!("{tool}-runtime"),
                    at_ms: 2000,
                    main: true,
                    stream_key: tool.into(),
                })
                .into(),
            source_observations: ["codex", "claude"]
                .map(|tool| SourceObservation {
                    tool_id: tool.into(),
                    source_id: format!("{tool}_session"),
                    status: "ok".into(),
                    dated: true,
                })
                .into(),
            codex_native_ready: false,
            legacy_history_available: true,
            thread_usage_coverage: Default::default(),
        }
    }

    fn row(conn: &Connection, id: &str, sid: Option<&str>, tokens: i64, time: i64) {
        conn.execute(
            "INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,latency_ms,status_code,created_at,data_source,session_id,total_cost_usd) VALUES(?1,'_codex_session','codex','legacy-model',?2,0,3,0,0,200,?3,'codex_session',?4,'7.77')",
            rusqlite::params![id,tokens,time,sid],
        ).unwrap();
    }

    #[test]
    fn legacy_fallback_replaces_only_codex_with_stored_sid_and_preserves_ledger() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        row(&conn, "legacy-request", Some("stored-billing"), 1000, 2);
        let mut input = input();
        let claude_before = input.facts[1].clone();
        apply_on(&conn, &mut input).unwrap();
        let codex = input
            .facts
            .iter()
            .filter(|f| f.tool_id == "codex")
            .collect::<Vec<_>>();
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].buckets, [1000, 0, 3, 0]);
        assert_eq!(codex[0].at_ms, 2000);
        assert!(
            codex[0].complete,
            "stored zero is a known zero, not missing"
        );
        assert!(input.facts.contains(&claude_before));
        let session = input
            .sessions
            .iter()
            .find(|s| s.identity.tool_id == "codex")
            .unwrap();
        assert_ne!(session.identity.key, "codex-native");
        assert_eq!(session.native_session_id, "");
        assert_eq!(session.identity.project, Project::Unparsed);
        assert!(session.manager_key.is_none());
        assert!(!session.messages_supported);
        assert!(session
            .message_reasons
            .iter()
            .any(|r| r == "statisticsRebuildRequired"));
        assert_eq!(input.messages.len(), 1);
        assert_eq!(input.messages[0].session_key, "claude-native");
        assert_eq!(input.native_activity.len(), 1);
        assert_eq!(input.native_activity[0].session_key, "claude-native");
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.tool_id == "codex" && r.reason == "statisticsRebuildRequired"));
        assert!(input
            .source_observations
            .iter()
            .any(|r| r.tool_id == "codex" && r.status == "partial"));
        let saved: (String, i64, String) = conn
            .query_row(
                "SELECT request_id,input_tokens,total_cost_usd FROM proxy_request_logs",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(saved, ("legacy-request".into(), 1000, "7.77".into()));
    }

    #[test]
    fn legacy_fallback_keeps_no_sid_usage_unassociated_and_groups_stored_sid_only() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        row(&conn, "a", Some("stored"), 100, 1);
        row(&conn, "b", Some("stored"), 200, 2);
        row(&conn, "c", None, 300, 3);
        row(&conn, "d", Some(""), 400, 4);
        let mut input = input();
        apply_on(&conn, &mut input).unwrap();
        let codex = input
            .facts
            .iter()
            .filter(|f| f.tool_id == "codex")
            .collect::<Vec<_>>();
        assert_eq!(
            codex
                .iter()
                .map(|f| f.buckets[0] + f.buckets[1])
                .sum::<u64>(),
            1000
        );
        assert_eq!(codex.iter().filter(|f| f.session_key.is_none()).count(), 2);
        assert_eq!(
            input
                .sessions
                .iter()
                .filter(|s| s.identity.tool_id == "codex")
                .count(),
            1
        );
        assert_eq!(
            codex
                .iter()
                .filter_map(|f| f.session_key.as_ref())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            1
        );
    }

    #[test]
    fn legacy_fallback_digest_tracks_values_and_identity_but_not_saved_amount() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        row(&conn, "a", Some("stored"), 100, 2);
        let before = load_on(&conn).unwrap().digest;
        conn.execute("UPDATE proxy_request_logs SET input_tokens=200,session_id='different',model='other',created_at=3", []).unwrap();
        let after = load_on(&conn).unwrap().digest;
        assert_ne!(before, after);
        conn.execute("UPDATE proxy_request_logs SET total_cost_usd='9.99'", [])
            .unwrap();
        assert_eq!(after, load_on(&conn).unwrap().digest);
    }

    #[test]
    fn legacy_fallback_snapshot_is_64_hex_stable_and_repeated_apply_is_idempotent() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        row(&conn, "a", Some("stored"), 100, 2);
        let snapshot = load_on(&conn).unwrap();
        let mut first = input();
        let mut second = input();
        snapshot.apply(&mut first).unwrap();
        snapshot.apply(&mut second).unwrap();
        assert_eq!(first.snapshot_version, second.snapshot_version);
        assert_eq!(first.snapshot_version.len(), 64);
        assert!(first
            .snapshot_version
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
        let before = serde_json::to_value(&first).unwrap();
        snapshot.apply(&mut first).unwrap();
        assert_eq!(before, serde_json::to_value(&first).unwrap());
        let mut other_base = input();
        other_base.snapshot_version = "other-source-generation".into();
        snapshot.apply(&mut other_base).unwrap();
        assert_ne!(other_base.snapshot_version, first.snapshot_version);
    }

    #[test]
    fn legacy_fallback_does_not_change_accepted_native_input() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        row(&conn, "a", Some("stored"), 1000, 2);
        let mut input = input();
        input.codex_native_ready = true;
        let before = serde_json::to_value(&input).unwrap();
        apply_on(&conn, &mut input).unwrap();
        assert_eq!(serde_json::to_value(&input).unwrap(), before);
    }

    #[test]
    fn legacy_fallback_unavailable_is_failed_without_blocking_sibling_tool() {
        let snapshot = LegacySnapshot::unavailable();
        assert!(snapshot.failed);
        let mut input = input();
        let claude_before = input.facts[1].clone();
        snapshot.apply(&mut input).unwrap();
        assert!(input.facts.iter().all(|fact| fact.tool_id != "codex"));
        assert!(input.facts.contains(&claude_before));
        assert_eq!(input.messages[0].session_key, "claude-native");
        assert!(input
            .source_observations
            .iter()
            .any(|source| source.tool_id == "codex" && source.status == "failed" && !source.dated));
        assert!(input
            .source_reasons
            .iter()
            .any(|reason| reason.tool_id == "codex"
                && reason.reason == "legacyCodexHistoryUnavailable"));
    }

    #[test]
    fn legacy_fallback_apply_on_contains_bad_rows_and_preserves_original_invalid_values() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        row(&conn, "invalid", Some("stored"), -1, 2);
        assert!(load_on(&conn).is_err());
        let mut input = input();
        let claude_before = input.facts[1].clone();
        let digest = apply_on(&conn, &mut input)
            .expect("one failed local source cannot block unrelated tools");
        assert_eq!(digest.len(), 64);
        assert!(input.facts.contains(&claude_before));
        assert!(input.facts.iter().all(|fact| fact.tool_id != "codex"));
        assert!(input
            .source_observations
            .iter()
            .any(|source| source.tool_id == "codex" && source.status == "failed"));
        let saved:(i64,String) = conn.query_row("SELECT input_tokens,total_cost_usd FROM proxy_request_logs WHERE request_id='invalid'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(saved, (-1, "7.77".into()));
    }

    #[test]
    fn legacy_fallback_rejects_invalid_counters_and_time_without_zero_substitution() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        row(&conn, "invalid", Some("stored"), -1, 2);
        assert!(load_on(&conn).is_err());
        conn.execute(
            "UPDATE proxy_request_logs SET input_tokens=0,created_at=?1",
            [i64::MAX],
        )
        .unwrap();
        assert!(load_on(&conn).is_err());
    }

    #[test]
    fn legacy_fallback_honors_row_and_byte_budgets_before_publishing_partial_rows() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        row(&conn, "first", Some("stored"), 100, 1);
        row(&conn, "second", Some("stored"), 200, 2);
        assert!(load_bounded(&conn, 1, MAX_BYTES).is_err());
        assert!(load_bounded(&conn, MAX_ROWS, 1).is_err());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn legacy_fallback_rollups_cannot_invent_session_events_or_event_time() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute("INSERT INTO usage_daily_rollups(date,app_type,provider_id,model,request_count,input_tokens,total_cost_usd) VALUES('1970-01-01','codex','_codex_session','legacy-model',1,100,'8.88')", []).unwrap();
        let mut input = input();
        apply_on(&conn, &mut input).unwrap();
        assert!(input.facts.iter().all(|f| f.tool_id != "codex"));
        assert!(input.sessions.iter().all(|s| s.identity.tool_id != "codex"));
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.tool_id == "codex" && r.reason == "legacyCodexHistoryUnverified"));
        assert!(input
            .source_observations
            .iter()
            .any(|r| r.tool_id == "codex" && !r.dated));
    }

    fn native_fixture() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-thread.jsonl");
        let rows = [
            json!({"type":"session_meta","timestamp":"1970-01-01T00:00:00Z","payload":{"id":"thread","session_id":"billing","source":"cli","cwd":"/fixture/project"}}),
            json!({"type":"turn_context","timestamp":"1970-01-01T00:00:01Z","payload":{"turn_id":"t","model":"native-model"}}),
            json!({"type":"token_usage_record","timestamp":"1970-01-01T00:00:02Z","payload":{"thread_id":"thread","session_id":"billing","turn_id":"t","response_id":"r","usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":20,"cache_write_input_tokens":0},"thread_token_usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":20,"cache_write_input_tokens":0}}}),
        ];
        std::fs::write(
            &path,
            rows.iter()
                .map(serde_json::Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let db = Database::memory().unwrap();
        session_analytics_index::refresh_sources(
            &db,
            &[SourceSpec {
                provider: "codex".into(),
                path,
                subagent: false,
                project_file: None,
                title_file: None,
            }],
        )
        .unwrap();
        row(
            &db.conn.lock().unwrap(),
            "legacy-original",
            Some("billing"),
            1000,
            2,
        );
        (dir, db)
    }

    fn query() -> UsageQueryV1 {
        UsageQueryV1 {
            version: 1,
            start_ms: Some(0),
            end_ms: 86_400_000,
            tool_ids: vec!["codex".into()],
            model_ids: vec![],
            project_ids: vec![],
            source_ids: vec![],
        }
    }

    #[test]
    fn legacy_fallback_missing_generation_aligns_public_analysis_with_dataset() {
        let (_dir, db) = native_fixture();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM session_meta_cache WHERE source_path='analytics-codex-generation-v1'",
                [],
            )
            .unwrap();
        let input = session_analytics_index::read_input(&db).unwrap();
        assert!(!input.codex_native_ready);
        let data =
            usage_data::read_dataset(&db, query(), None, "ofox.ai", 86_400_000, &chrono::Utc)
                .unwrap();
        let analysis_tokens = input
            .facts
            .iter()
            .filter(|f| f.tool_id == "codex")
            .map(|f| f.buckets[0] + f.buckets[1])
            .sum::<u64>();
        let dataset_tokens = data
            .atoms
            .iter()
            .map(|a| a.tokens[0] + a.tokens[1])
            .sum::<u64>();
        assert_eq!(analysis_tokens,dataset_tokens,"an unaccepted native generation must not leak into analysis while overview uses legacy");
        assert_eq!(analysis_tokens, 1000);
    }

    #[test]
    fn legacy_fallback_changed_epoch_aligns_public_analysis_with_dataset() {
        let (_dir, db) = native_fixture();
        db.conn.lock().unwrap().execute("INSERT OR REPLACE INTO maintenance_flags(key,value) VALUES('usage_ledger_epoch_v1','changed-test-epoch')", []).unwrap();
        let input = session_analytics_index::read_input(&db).unwrap();
        assert!(!input.codex_native_ready);
        let data =
            usage_data::read_dataset(&db, query(), None, "ofox.ai", 86_400_000, &chrono::Utc)
                .unwrap();
        let analysis_tokens = input
            .facts
            .iter()
            .filter(|f| f.tool_id == "codex")
            .map(|f| f.buckets[0] + f.buckets[1])
            .sum::<u64>();
        let dataset_tokens = data
            .atoms
            .iter()
            .map(|a| a.tokens[0] + a.tokens[1])
            .sum::<u64>();
        assert_eq!(
            analysis_tokens, dataset_tokens,
            "a native generation from another epoch cannot feed only one consumer"
        );
        assert_eq!(analysis_tokens, 1000);
    }

    #[test]
    fn legacy_fallback_projection_cache_tracks_changed_legacy_rows() {
        let (_dir, db) = native_fixture();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM session_meta_cache WHERE source_path='analytics-codex-generation-v1'",
            [],
        )
        .unwrap();
        let cache = ProjectionCache::default();
        let first = cache.read_on(&conn).unwrap();
        conn.execute("UPDATE proxy_request_logs SET input_tokens=2000,model='changed-model',session_id='changed-sid' WHERE request_id='legacy-original'", []).unwrap();
        let second = cache.read_on(&conn).unwrap();
        assert!(
            !std::sync::Arc::ptr_eq(&first, &second),
            "legacy values, not only existence, must participate in the warm projection key"
        );
        let facts = second
            .facts
            .iter()
            .filter(|f| f.tool_id == "codex")
            .collect::<Vec<_>>();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].buckets[0], 2000);
        assert_eq!(facts[0].model, "changed-model");
    }
}
