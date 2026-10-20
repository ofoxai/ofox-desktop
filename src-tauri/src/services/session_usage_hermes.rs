//! Hermes SessionDB usage. Source contract: 0.21.7+18.g590af66, schema 31.
//! Hermes keeps cumulative counters per session and per session × model ×
//! task × billing route, never per request. As in magpie, each
//! `session_model_usage` row is one fact dated at its `last_seen`, and session
//! totals the rows leave unexplained become one unknown-model fact. Message
//! bodies are compared inside SQLite and never read into the process.
use super::{
    native_sqlite_snapshot::SqliteSnapshot,
    session_usage::SessionSyncResult,
    usage_source::{self, NormalizedUsageFact},
};
use crate::{database::Database, error::AppError, proxy::usage::parser::TokenUsage};
use rusqlite::{types::ValueRef, Connection, Row};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ROWS: usize = 200_000;
const MAX_LABEL_BYTES: usize = 1024;
/// Messages and their activity events share one 32 MiB analytics cache entry.
const MAX_MESSAGES: usize = 60_000;
/// Hermes counts these tasks outside the session totals.
const AUXILIARY_TASKS: [&str; 8] = [
    "vision",
    "compression",
    "title_generation",
    "background_review",
    "web_extract",
    "session_search",
    "approval",
    "mcp",
];

fn invalid(code: &str) -> AppError {
    AppError::Database(format!("usage.hermes.{code}"))
}

pub(crate) struct HermesSession {
    pub sid: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub started_ms: i64,
}

pub(crate) struct HermesMessage {
    pub sid: String,
    pub id: i64,
    pub role: String,
    pub at_ms: i64,
}

pub(crate) struct HermesRead {
    pub facts: Vec<NormalizedUsageFact>,
    /// Request ids of facts whose attribution is a lower bound.
    pub incomplete: BTreeSet<String>,
    pub sessions: Vec<HermesSession>,
    /// Deduplicated messages in Hermes id order; `None` past the budget.
    pub messages: Option<Vec<HermesMessage>>,
    pub reasons: BTreeSet<String>,
}

pub(crate) fn read_hermes(db_path: &Path) -> Result<HermesRead, AppError> {
    read_after_copy(db_path, || {})
}

pub(crate) fn read_after_copy(
    db_path: &Path,
    after_copy: impl FnOnce(),
) -> Result<HermesRead, AppError> {
    SqliteSnapshot::open(db_path, MAX_SOURCE_BYTES, invalid)?.read(after_copy, read_database)
}

fn seconds_ms(row: &Row<'_>, index: usize) -> Result<Option<i64>, AppError> {
    let seconds = match row.get_ref(index).map_err(|_| invalid("invalidDatabase"))? {
        ValueRef::Real(value) => value,
        ValueRef::Integer(value) => value as f64,
        _ => return Ok(None),
    };
    if !seconds.is_finite() || seconds <= 0.0 {
        return Ok(None);
    }
    let ms = (seconds * 1000.0).round() as i64;
    Ok(chrono::DateTime::from_timestamp_millis(ms).map(|_| ms))
}

fn label(row: &Row<'_>, index: usize) -> Result<Option<String>, AppError> {
    let value: Option<String> = row.get(index).map_err(|_| invalid("invalidDatabase"))?;
    if value.as_ref().is_some_and(|v| v.len() > MAX_LABEL_BYTES) {
        return Err(invalid("sourceTooLarge"));
    }
    Ok(value)
}

/// input, output, cache read, cache write. Reasoning is already in output.
type Counts = [u64; 4];

fn counts(row: &Row<'_>, start: usize, reasons: &mut BTreeSet<String>) -> Result<Counts, AppError> {
    let mut out = [0; 4];
    for (index, value) in out.iter_mut().enumerate() {
        *value = match row
            .get_ref(start + index)
            .map_err(|_| invalid("invalidDatabase"))?
        {
            ValueRef::Integer(n) if n >= 0 => n as u64,
            ValueRef::Null => {
                reasons.insert("missingCounters".into());
                0
            }
            _ => {
                reasons.insert("invalidCounters".into());
                0
            }
        };
    }
    Ok(out)
}

struct Session {
    meta: HermesSession,
    pool: Counts,
    /// max(started_at, ended_at, last_activity_at), magpie's session end.
    last_ms: i64,
    /// Main-loop and voice rows, which decompose the session totals.
    pooled: Counts,
}

struct ModelRow {
    sid: String,
    model: String,
    route: String,
    pooled: bool,
    known_task: bool,
    counts: Counts,
    last_seen_ms: Option<i64>,
}

fn read_database(conn: &Connection) -> Result<HermesRead, AppError> {
    let mut reasons = BTreeSet::new();
    let version: i64 = conn
        .query_row(
            "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| invalid("unsupportedSchema"))?;
    if version < 31 {
        return Err(invalid("unsupportedSchema"));
    }
    if version > 31 {
        reasons.insert("unverifiedSchemaVersion".into());
    }
    let mut sessions = read_sessions(conn, &mut reasons)?;
    let rows = read_model_rows(conn, &mut reasons)?;
    let message_last = read_message_last(conn)?;
    let messages = read_messages(conn, &mut reasons)?;

    let mut facts = Vec::new();
    let mut incomplete = BTreeSet::new();
    for row in rows {
        let Some(session) = sessions.get_mut(&row.sid) else {
            reasons.insert("orphanModelRows".into());
            continue;
        };
        if row.pooled {
            for (sum, n) in session.pooled.iter_mut().zip(row.counts) {
                *sum = sum.saturating_add(n);
            }
        }
        if !row.known_task {
            reasons.insert("unknownTask".into());
        }
        let session_last = session
            .last_ms
            .max(message_last.get(&row.sid).copied().unwrap_or(0));
        let digest = format!(
            "{:x}",
            Sha256::digest(json!([row.model, row.route]).to_string().as_bytes())
        );
        let Some(fact) = fact(
            &row.sid,
            &digest,
            row.model,
            row.counts,
            row.last_seen_ms.unwrap_or(session_last),
        ) else {
            reasons.insert("counterOverflow".into());
            continue;
        };
        if !row.known_task {
            incomplete.insert(fact.request_id.clone());
        }
        facts.push(fact);
    }
    for session in sessions.values() {
        let mut residual = [0; 4];
        for (index, value) in residual.iter_mut().enumerate() {
            match session.pool[index].checked_sub(session.pooled[index]) {
                Some(n) => *value = n,
                None => {
                    reasons.insert("inconsistentModelCounters".into());
                }
            }
        }
        if residual.iter().all(|n| *n == 0) {
            continue;
        }
        reasons.insert("modelAttributionUnavailable".into());
        let Some(fact) = fact(
            &session.meta.sid,
            "residual",
            "unknown".into(),
            residual,
            session.last_ms,
        ) else {
            reasons.insert("counterOverflow".into());
            continue;
        };
        incomplete.insert(fact.request_id.clone());
        facts.push(fact);
    }
    Ok(HermesRead {
        facts,
        incomplete,
        sessions: sessions.into_values().map(|s| s.meta).collect(),
        messages,
        reasons,
    })
}

fn fact(
    sid: &str,
    id: &str,
    model: String,
    counts: Counts,
    at_ms: i64,
) -> Option<NormalizedUsageFact> {
    let [input, output, read, write] = counts.map(|n| usage_source::checked_token(n).ok());
    Some(NormalizedUsageFact {
        request_id: format!("hermes_session:{}", json!([sid, id])),
        native_key: Some(usage_source::native_key("hermes", sid, id)),
        session_id: Some(sid.to_owned()),
        usage: TokenUsage {
            input_tokens: input?,
            output_tokens: output?,
            cache_read_tokens: read?,
            cache_creation_tokens: write?,
            cache_creation_1h_tokens: 0,
            model: Some(model.clone()),
            message_id: None,
        },
        model,
        at_ms: (at_ms > 0).then_some(at_ms),
    })
}

fn read_sessions(
    conn: &Connection,
    reasons: &mut BTreeSet<String>,
) -> Result<BTreeMap<String, Session>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT id,title,cwd,started_at,ended_at,last_activity_at,\
             input_tokens,output_tokens,cache_read_tokens,cache_write_tokens \
             FROM sessions ORDER BY id LIMIT ?1",
        )
        .map_err(|_| invalid("unsupportedSchema"))?;
    let mut rows = stmt
        .query([(MAX_ROWS + 1) as i64])
        .map_err(|_| invalid("invalidDatabase"))?;
    let mut out = BTreeMap::new();
    while let Some(row) = rows.next().map_err(|_| invalid("invalidDatabase"))? {
        if out.len() >= MAX_ROWS {
            return Err(invalid("sourceTooLarge"));
        }
        let sid = label(row, 0)?
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| invalid("invalidDatabase"))?;
        let started_ms = seconds_ms(row, 3)?;
        if started_ms.is_none() {
            reasons.insert("missingSessionTime".into());
        }
        let last_ms = [started_ms, seconds_ms(row, 4)?, seconds_ms(row, 5)?]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(0);
        out.insert(
            sid.clone(),
            Session {
                meta: HermesSession {
                    sid,
                    title: label(row, 1)?.filter(|t| !t.trim().is_empty()),
                    cwd: label(row, 2)?.filter(|c| !c.trim().is_empty()),
                    started_ms: started_ms.unwrap_or(0),
                },
                pool: counts(row, 6, reasons)?,
                last_ms,
                pooled: [0; 4],
            },
        );
    }
    Ok(out)
}

fn read_model_rows(
    conn: &Connection,
    reasons: &mut BTreeSet<String>,
) -> Result<Vec<ModelRow>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id,model,billing_provider,billing_base_url,billing_mode,task,\
             input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,last_seen \
             FROM session_model_usage \
             ORDER BY session_id,model,billing_provider,billing_base_url,billing_mode,task \
             LIMIT ?1",
        )
        .map_err(|_| invalid("unsupportedSchema"))?;
    let mut rows = stmt
        .query([(MAX_ROWS + 1) as i64])
        .map_err(|_| invalid("invalidDatabase"))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|_| invalid("invalidDatabase"))? {
        if out.len() >= MAX_ROWS {
            return Err(invalid("sourceTooLarge"));
        }
        let text = |index| label(row, index).map(Option::unwrap_or_default);
        let sid = text(0)?;
        let model = Some(text(1)?)
            .filter(|m| !m.trim().is_empty() && m != "unknown")
            .unwrap_or_else(|| "unknown".into());
        let task = text(5)?;
        out.push(ModelRow {
            route: json!([text(2)?, text(3)?, text(4)?, task]).to_string(),
            pooled: matches!(task.as_str(), "" | "voice_chat"),
            known_task: task.is_empty()
                || task == "voice_chat"
                || AUXILIARY_TASKS.contains(&task.as_str()),
            sid,
            model,
            counts: counts(row, 6, reasons)?,
            last_seen_ms: seconds_ms(row, 10)?,
        });
    }
    Ok(out)
}

fn read_message_last(conn: &Connection) -> Result<BTreeMap<String, i64>, AppError> {
    let mut stmt = conn
        .prepare("SELECT session_id,MAX(timestamp) FROM messages GROUP BY session_id")
        .map_err(|_| invalid("unsupportedSchema"))?;
    let mut rows = stmt.query([]).map_err(|_| invalid("invalidDatabase"))?;
    let mut out = BTreeMap::new();
    while let Some(row) = rows.next().map_err(|_| invalid("invalidDatabase"))? {
        if let (Some(sid), Some(at)) = (label(row, 0)?, seconds_ms(row, 1)?) {
            out.insert(sid, at);
        }
    }
    Ok(out)
}

/// Hermes's display identity, as magpie matches it: duplicates of one turn
/// across compaction generations collapse to their first id. Model-only
/// scaffolding and inactive rows (unless compacted) are not conversation.
fn read_messages(
    conn: &Connection,
    reasons: &mut BTreeSet<String>,
) -> Result<Option<Vec<HermesMessage>>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id,role,timestamp,MIN(id) FROM messages \
             WHERE (COALESCE(active,1)!=0 OR COALESCE(compacted,0)!=0) \
             AND COALESCE(json_extract(CASE WHEN json_valid(display_metadata) \
             THEN display_metadata ELSE '{}' END,'$.model_only'),0)=0 \
             GROUP BY session_id,role,content,tool_calls,tool_call_id,tool_name,timestamp \
             ORDER BY session_id,4 LIMIT ?1",
        )
        .map_err(|_| invalid("unsupportedSchema"))?;
    let mut rows = stmt
        .query([(MAX_MESSAGES + 1) as i64])
        .map_err(|_| invalid("invalidDatabase"))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|_| invalid("invalidDatabase"))? {
        if out.len() >= MAX_MESSAGES {
            reasons.insert("activityBudgetExceeded".into());
            return Ok(None);
        }
        let (Some(sid), Some(role), Some(at_ms)) =
            (label(row, 0)?, label(row, 1)?, seconds_ms(row, 2)?)
        else {
            reasons.insert("invalidMessageTime".into());
            continue;
        };
        out.push(HermesMessage {
            sid,
            role,
            at_ms,
            id: row.get(3).map_err(|_| invalid("invalidDatabase"))?,
        });
    }
    Ok(Some(out))
}

pub(crate) fn sync_hermes_usage_at(
    db: &Database,
    root: &Path,
) -> Result<SessionSyncResult, AppError> {
    use super::usage_source::{Prepared, ProviderIndex, Snapshot};
    use super::usage_sync_db::{commit_sync_batch, read_sync_state, CheckpointChange, SyncState};
    use rusqlite::{params, OptionalExtension};
    let path = root.join("state.db");
    let mut result = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: 0,
        errors: Vec::new(),
    };
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(result),
        _ => result.files_scanned = 1,
    }
    let index = ProviderIndex::load(db, "hermes", root)?;
    let key = path.to_string_lossy().into_owned();
    let state = read_sync_state(db, &key)?;
    let mut source = Prepared::with_index(db, Snapshot::sqlite(&path)?, state, 1, &index)?;
    if source.unchanged {
        return Ok(result);
    }
    let read = read_hermes(&path)?;
    let modified = source.snapshot.stamp["main"]["mtimeNs"]
        .as_str()
        .and_then(|v| v.parse::<u128>().ok())
        .and_then(|v| i64::try_from(v / 1_000_000_000).ok())
        .ok_or_else(|| invalid("invalidSourceStamp"))?;
    let next = SyncState {
        modified,
        offset: read.facts.len() as i64,
    };
    let checkpoints = [CheckpointChange {
        key,
        expected: state,
        next: Some(next),
    }];
    commit_sync_batch(db, &checkpoints, |conn| {
        source.check_on(conn)?;
        for reason in &read.reasons {
            source.evidence.complete_event_identity_set = false;
            source.evidence.partial(reason);
        }
        for fact in &read.facts {
            let (Some(native), Some(sid), Some(at_ms)) = (
                fact.native_key.as_deref(),
                fact.session_id.as_deref(),
                fact.at_ms,
            ) else {
                result.skipped += 1;
                continue;
            };
            let row: Option<(String, String, Option<String>, Option<String>)> = conn
                .query_row(
                    "SELECT provider_id,app_type,data_source,session_id FROM proxy_request_logs WHERE request_id=?1",
                    [&fact.request_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            if row.as_ref().is_some_and(|(owner, app, origin, row_sid)| {
                owner != "_hermes_session"
                    || app != "hermes"
                    || origin.as_deref() != Some("hermes_session")
                    || row_sid.as_deref() != Some(sid)
            }) {
                return Err(invalid("foreignRecordIdentity"));
            }
            let created_at = at_ms.div_euclid(1000);
            if !source.should_store_on(
                conn,
                &fact.request_id,
                native,
                "hermes_session",
                created_at,
            )? {
                result.skipped += 1;
                continue;
            }
            let u = &fact.usage;
            // A growing session updates its row in place and moves it to the
            // row's latest activity, the way magpie dates Hermes usage.
            let changed = conn.execute(
                "INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES(?1,'_hermes_session','hermes',?2,?3,?4,?5,?6,0,0,?7,?8,'hermes_session') ON CONFLICT(request_id) DO UPDATE SET model=excluded.model,input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cache_read_tokens=excluded.cache_read_tokens,cache_creation_tokens=excluded.cache_creation_tokens,created_at=excluded.created_at WHERE model!=excluded.model OR input_tokens!=excluded.input_tokens OR output_tokens!=excluded.output_tokens OR cache_read_tokens!=excluded.cache_read_tokens OR cache_creation_tokens!=excluded.cache_creation_tokens OR created_at!=excluded.created_at",
                params![fact.request_id, fact.model, u.input_tokens, u.output_tokens, u.cache_read_tokens, u.cache_creation_tokens, sid, created_at],
            )?;
            if changed > 0 {
                result.imported += 1;
            } else {
                result.skipped += 1;
            }
            source.evidence.acknowledge(native);
        }
        source.save_on(conn, next)
    })?;
    Ok(result)
}

#[cfg(test)]
#[path = "session_usage_hermes_tests.rs"]
mod tests;
