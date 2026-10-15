//! Machine-local, epoch-fenced source health shared by ingestion and analytics.
use super::usage_query::SourceHealth;
use crate::{error::AppError, session_manager::source_cache};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const SETTING_KEY: &str = "usage_source_health_v1";
const MAX_BYTES: usize = 256 * 1024;
const MAX_SOURCES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Channel {
    Ledger,
    Analytics,
}
impl Channel {
    fn key(self) -> &'static str {
        match self {
            Self::Ledger => "ledger",
            Self::Analytics => "analytics",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Outcome {
    Ok,
    Partial(&'static str),
    Failed(&'static str),
}

#[derive(Clone, Debug)]
pub(crate) struct Attempt {
    tool_id: String,
    source_id: String,
    channel: Channel,
    revision: String,
    epoch: Option<String>,
}

fn invalid() -> AppError {
    AppError::Database("usage.sources.invalidHealth".into())
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn read_raw_on(conn: &Connection) -> Result<Option<String>, AppError> {
    Ok(conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            [SETTING_KEY],
            |r| r.get(0),
        )
        .optional()?)
}
fn decode(raw: &str) -> Result<Value, AppError> {
    if raw.len() > MAX_BYTES {
        return Err(invalid());
    }
    let value: Value = serde_json::from_str(raw).map_err(|_| invalid())?;
    if value["version"].as_u64() != Some(1) {
        return Err(invalid());
    }
    let sources = value["sources"].as_array().ok_or_else(invalid)?;
    if sources.len() > MAX_SOURCES {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    for row in sources {
        let tool = row["toolId"]
            .as_str()
            .filter(|v| identifier(v))
            .ok_or_else(invalid)?;
        let source = row["sourceId"]
            .as_str()
            .filter(|v| identifier(v))
            .ok_or_else(invalid)?;
        if !seen.insert((tool, source)) {
            return Err(invalid());
        }
    }
    Ok(value)
}
fn load_for_write_on(conn: &Connection) -> Result<Value, AppError> {
    match read_raw_on(conn)? {
        None => Ok(json!({"version":1,"sources":[]})),
        Some(raw) => {
            // The legacy Vec is recognized but carries no local epoch proof.
            // Retain it opaquely while the first real attempt builds new health.
            if raw.len() <= MAX_BYTES {
                if let Ok(legacy) = serde_json::from_str::<Value>(&raw) {
                    if legacy.is_array()
                        && serde_json::from_value::<Vec<SourceHealth>>(legacy.clone()).is_ok()
                    {
                        return Ok(json!({"version":1,"sources":[],"legacy":legacy}));
                    }
                }
            }
            decode(&raw)
        }
    }
}
fn write_on(conn: &Connection, value: &Value) -> Result<(), AppError> {
    let raw = serde_json::to_string(value).map_err(|_| invalid())?;
    if raw.len() > MAX_BYTES {
        return Err(invalid());
    }
    conn.execute(
        "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![SETTING_KEY, raw],
    )?;
    Ok(())
}
fn source_mut<'a>(
    value: &'a mut Value,
    tool: &str,
    source: &str,
) -> Result<&'a mut Value, AppError> {
    let sources = value["sources"].as_array_mut().ok_or_else(invalid)?;
    let index = match sources
        .iter()
        .position(|r| r["toolId"] == tool && r["sourceId"] == source)
    {
        Some(index) => index,
        None => {
            if sources.len() >= MAX_SOURCES {
                return Err(invalid());
            }
            sources.push(json!({"toolId":tool,"sourceId":source}));
            sources.len() - 1
        }
    };
    Ok(&mut sources[index])
}
struct ChannelState {
    status: String,
    epoch: Option<String>,
    attempted: i64,
    succeeded: Option<i64>,
    code: Option<String>,
}
fn channel_state(value: &Value) -> Result<ChannelState, AppError> {
    if value["version"].as_u64() != Some(1)
        || value["revision"].as_str().is_none_or(|v| !identifier(v))
        || !value["pending"].is_boolean()
    {
        return Err(invalid());
    }
    let status = value["status"]
        .as_str()
        .filter(|s| ["unknown", "ok", "partial", "failed"].contains(s))
        .ok_or_else(invalid)?;
    let epoch = match value.get("ledgerEpoch") {
        Some(Value::Null) => None,
        Some(Value::String(s)) if identifier(s) => Some(s.clone()),
        _ => return Err(invalid()),
    };
    let attempted = value["lastAttemptAtMs"]
        .as_i64()
        .filter(|v| *v >= 0)
        .ok_or_else(invalid)?;
    let succeeded = match value.get("lastSuccessAtMs") {
        Some(Value::Null) => None,
        Some(n) => Some(n.as_i64().filter(|v| *v >= 0).ok_or_else(invalid)?),
        None => return Err(invalid()),
    };
    let code = match value.get("errorCode") {
        Some(Value::Null) => None,
        Some(Value::String(s)) if identifier(s) => Some(s.clone()),
        _ => return Err(invalid()),
    };
    if (status == "ok" && succeeded.is_none()) || (value["pending"] == true && status != "unknown")
    {
        return Err(invalid());
    }
    Ok(ChannelState {
        status: status.into(),
        epoch,
        attempted,
        succeeded,
        code,
    })
}

/// Conn-level API: callers own the surrounding transaction and lock. The
/// returned token is specific to this source, channel, and ledger incarnation.
pub(crate) fn begin_on(
    conn: &Connection,
    tool_id: &str,
    source_id: &str,
    channel: Channel,
    at_ms: i64,
) -> Result<Attempt, AppError> {
    if !identifier(tool_id) || !identifier(source_id) || at_ms < 0 {
        return Err(invalid());
    }
    let mut value = load_for_write_on(conn)?;
    let row = source_mut(&mut value, tool_id, source_id)?;
    let epoch = source_cache::ledger_epoch_on(conn)?;
    let prior = row.get(channel.key()).map(channel_state).transpose()?;
    let succeeded = prior
        .filter(|state| state.epoch == epoch)
        .and_then(|s| s.succeeded);
    let revision = uuid::Uuid::new_v4().to_string();
    let mut state = row.get(channel.key()).cloned().unwrap_or_else(|| json!({}));
    let object = state.as_object_mut().ok_or_else(invalid)?;
    for (key, entry) in json!({"version":1,"revision":revision,"ledgerEpoch":epoch,
        "status":"unknown","lastAttemptAtMs":at_ms,"lastSuccessAtMs":succeeded,
        "errorCode":null,"pending":true})
    .as_object()
    .expect("object")
    {
        object.insert(key.clone(), entry.clone());
    }
    row[channel.key()] = state;
    write_on(conn, &value)?;
    Ok(Attempt {
        tool_id: tool_id.into(),
        source_id: source_id.into(),
        channel,
        revision,
        epoch,
    })
}

/// Only a full trustworthy success advances lastSuccess. Partial and failed
/// attempts preserve that older success; native read times remain in the cache.
/// Returns false when consumed, superseded, or invalidated by ledger restore.
pub(crate) fn finish_on(
    conn: &Connection,
    attempt: &Attempt,
    outcome: Outcome,
    at_ms: i64,
) -> Result<bool, AppError> {
    if source_cache::ledger_epoch_on(conn)? != attempt.epoch {
        return Ok(false);
    }
    if at_ms < 0 {
        return Err(invalid());
    }
    let mut value = load_for_write_on(conn)?;
    let Some(row) = value["sources"]
        .as_array_mut()
        .ok_or_else(invalid)?
        .iter_mut()
        .find(|r| r["toolId"] == attempt.tool_id && r["sourceId"] == attempt.source_id)
    else {
        return Ok(false);
    };
    let Some(state) = row.get_mut(attempt.channel.key()) else {
        return Ok(false);
    };
    let old = channel_state(state)?;
    if old.epoch != attempt.epoch
        || state["revision"] != attempt.revision
        || state["pending"] != true
    {
        return Ok(false);
    }
    let (status, code) = match outcome {
        Outcome::Ok => ("ok", None),
        Outcome::Partial(code) => ("partial", Some(code)),
        Outcome::Failed(code) => ("failed", Some(code)),
    };
    if code.is_some_and(|c| !identifier(c)) {
        return Err(invalid());
    }
    state["status"] = json!(status);
    state["pending"] = json!(false);
    state["errorCode"] = json!(code);
    if status == "ok" {
        state["lastSuccessAtMs"] = json!(at_ms);
    }
    write_on(conn, &value)?;
    Ok(true)
}

fn skeleton() -> BTreeMap<(String, String), (SourceHealth, bool)> {
    super::usage_query::source_capabilities()
        .into_iter()
        .flat_map(|c| {
            let ids = if c.source_ids.is_empty() {
                vec![format!("{}:unsupported", c.tool_id)]
            } else {
                c.source_ids
            };
            ids.into_iter().map(move |source_id| {
                let row = SourceHealth {
                    source_id,
                    tool_id: c.tool_id.clone(),
                    status: if c.usage_supported {
                        "unknown"
                    } else {
                        "unsupported"
                    }
                    .into(),
                    last_attempt_at_ms: None,
                    last_success_at_ms: None,
                    error_code: c.reason.clone(),
                };
                (
                    (row.tool_id.clone(), row.source_id.clone()),
                    (row, c.session_analytics_supported),
                )
            })
        })
        .collect()
}
fn priority(status: &str) -> u8 {
    match status {
        "failed" => 3,
        "partial" => 2,
        "unknown" => 1,
        _ => 0,
    }
}

/// A read-only query never turns legacy, corrupt, or foreign-epoch health into
/// a local success. Real SQLite read errors remain visible to the caller.
pub(crate) fn source_health_on(conn: &Connection) -> Result<Vec<SourceHealth>, AppError> {
    let mut health = skeleton();
    let Some(raw) = read_raw_on(conn)? else {
        return Ok(health.into_values().map(|v| v.0).collect());
    };
    let value = match decode(&raw) {
        Ok(value) => value,
        Err(_) => {
            for (row, _) in health
                .values_mut()
                .filter(|(r, _)| r.status != "unsupported")
            {
                row.error_code = Some("usage.sources.invalidHealth".into());
            }
            return Ok(health.into_values().map(|v| v.0).collect());
        }
    };
    let epoch = source_cache::ledger_epoch_on(conn)?;
    for source in value["sources"].as_array().expect("decoded") {
        let key = (
            source["toolId"].as_str().expect("decoded").to_owned(),
            source["sourceId"].as_str().expect("decoded").to_owned(),
        );
        let Some((row, analytics_required)) = health
            .get_mut(&key)
            .filter(|(r, _)| r.status != "unsupported")
        else {
            continue;
        };
        let channels = [Channel::Ledger].into_iter().chain(
            (*analytics_required || source.get("analytics").is_some())
                .then_some(Channel::Analytics),
        );
        let mut states = Vec::new();
        for channel in channels {
            let state = source.get(channel.key()).map(channel_state).transpose();
            let state = match state {
                Ok(Some(state)) if state.epoch == epoch => state,
                _ => ChannelState {
                    status: "unknown".into(),
                    epoch: epoch.clone(),
                    attempted: 0,
                    succeeded: None,
                    code: Some("usage.sources.incompleteHealth".into()),
                },
            };
            states.push(state);
        }
        let worst = states
            .iter()
            .max_by_key(|s| priority(&s.status))
            .expect("ledger required");
        row.status = worst.status.clone();
        row.error_code = worst.code.clone();
        row.last_attempt_at_ms = states.iter().map(|s| s.attempted).filter(|v| *v > 0).max();
        row.last_success_at_ms = states
            .iter()
            .map(|s| s.succeeded)
            .collect::<Option<Vec<_>>>()
            .and_then(|s| s.into_iter().min());
    }
    Ok(health.into_values().map(|v| v.0).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use serde_json::{json, Value};

    fn health(conn: &Connection, tool: &str) -> SourceHealth {
        source_health_on(conn)
            .unwrap()
            .into_iter()
            .find(|row| row.tool_id == tool && !row.source_id.ends_with(":proxy"))
            .unwrap()
    }

    fn begin(conn: &Connection, channel: Channel, at: i64) -> Attempt {
        begin_on(conn, "gemini", "gemini_session", channel, at).unwrap()
    }

    #[test]
    fn health_success_and_failure_keep_distinct_attempt_and_success_times() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let first = begin(&conn, Channel::Ledger, 1000);
        assert_eq!(health(&conn, "gemini").last_attempt_at_ms, Some(1000));
        assert!(finish_on(&conn, &first, Outcome::Ok, 1100).unwrap());
        let failed = begin(&conn, Channel::Ledger, 2000);
        assert!(finish_on(
            &conn,
            &failed,
            Outcome::Failed("usage.sources.readFailed"),
            2100
        )
        .unwrap());
        let row = health(&conn, "gemini");
        assert_eq!(row.status, "failed");
        assert_eq!(row.last_attempt_at_ms, Some(2000));
        assert_eq!(row.last_success_at_ms, Some(1100));
        assert_eq!(row.error_code.as_deref(), Some("usage.sources.readFailed"));
        assert_eq!(health(&conn, "claude").status, "unknown");
    }

    #[test]
    fn health_partial_does_not_become_failed_or_a_fabricated_success() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let attempt = begin(&conn, Channel::Ledger, 1000);
        assert!(finish_on(
            &conn,
            &attempt,
            Outcome::Partial("usage.sources.incompleteCoverage"),
            1100
        )
        .unwrap());
        let row = health(&conn, "gemini");
        assert_eq!(row.status, "partial");
        assert_eq!(row.last_success_at_ms, None);
    }

    #[test]
    fn health_analytics_failure_is_not_overwritten_by_ledger_success() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let analytics = begin(&conn, Channel::Analytics, 1000);
        assert!(finish_on(
            &conn,
            &analytics,
            Outcome::Failed("usage.sources.analyticsReadFailed"),
            1100
        )
        .unwrap());
        let ledger = begin(&conn, Channel::Ledger, 1200);
        assert!(finish_on(&conn, &ledger, Outcome::Ok, 1300).unwrap());
        let row = health(&conn, "gemini");
        assert_eq!(row.status, "failed");
        assert_eq!(
            row.error_code.as_deref(),
            Some("usage.sources.analyticsReadFailed")
        );
    }

    #[test]
    fn health_late_attempt_cannot_override_newer_result() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let old = begin(&conn, Channel::Ledger, 1000);
        let newer = begin(&conn, Channel::Ledger, 2000);
        assert!(finish_on(&conn, &newer, Outcome::Ok, 2100).unwrap());
        assert!(!finish_on(
            &conn,
            &old,
            Outcome::Failed("usage.sources.readFailed"),
            2200
        )
        .unwrap());
        let row = health(&conn, "gemini");
        assert_eq!(
            (row.status.as_str(), row.last_success_at_ms),
            ("ok", Some(2100))
        );
    }

    #[test]
    fn health_restore_epoch_rejects_pending_result_and_invalidates_previous_success() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let first = begin(&conn, Channel::Ledger, 1000);
        assert!(finish_on(&conn, &first, Outcome::Ok, 1100).unwrap());
        let pending = begin(&conn, Channel::Ledger, 1200);
        conn.execute("INSERT INTO maintenance_flags(key,value) VALUES('usage_ledger_epoch_v1','restored-epoch')", []).unwrap();
        assert!(!finish_on(&conn, &pending, Outcome::Ok, 1300).unwrap());
        let row = health(&conn, "gemini");
        assert_eq!(row.status, "unknown");
        assert_eq!(row.last_success_at_ms, None);
    }

    #[test]
    fn health_legacy_vec_and_future_format_are_never_trusted() {
        for raw in [
            r#"[{"sourceId":"gemini_session","toolId":"gemini","status":"ok","lastAttemptAtMs":1000,"lastSuccessAtMs":1100,"errorCode":null}]"#,
            r#"{"version":999,"sources":[]}"#,
            "{broken",
        ] {
            let db = Database::memory().unwrap();
            db.set_setting(SETTING_KEY, raw).unwrap();
            let conn = db.conn.lock().unwrap();
            let row = health(&conn, "gemini");
            assert_eq!(row.status, "unknown");
            assert_eq!(row.last_success_at_ms, None);
        }
    }

    #[test]
    fn health_unreadable_or_future_health_is_not_overwritten() {
        for raw in ["{broken", r#"{"version":999,"future":"keep"}"#] {
            let db = Database::memory().unwrap();
            db.set_setting(SETTING_KEY, raw).unwrap();
            let conn = db.conn.lock().unwrap();
            assert!(begin_on(&conn, "gemini", "gemini_session", Channel::Ledger, 1000).is_err());
            drop(conn);
            assert_eq!(db.get_setting(SETTING_KEY).unwrap().as_deref(), Some(raw));
        }
    }

    #[test]
    fn health_peer_and_future_fields_survive_channel_update() {
        let db = Database::memory().unwrap();
        db.set_setting(SETTING_KEY, &json!({"version":1,"future":{"keep":true},"sources":[{"toolId":"gemini","sourceId":"gemini_session","futureSource":"keep","futureChannel":{"opaque":true}}]}).to_string()).unwrap();
        let conn = db.conn.lock().unwrap();
        let ledger = begin(&conn, Channel::Ledger, 1000);
        assert!(finish_on(&conn, &ledger, Outcome::Ok, 1100).unwrap());
        drop(conn);
        let value: Value =
            serde_json::from_str(&db.get_setting(SETTING_KEY).unwrap().unwrap()).unwrap();
        assert_eq!(value["future"]["keep"], true);
        assert_eq!(value["sources"][0]["futureSource"], "keep");
        assert_eq!(value["sources"][0]["futureChannel"]["opaque"], true);
    }

    #[test]
    fn health_analytics_can_commit_atomically_with_its_cache_transaction() {
        let db = Database::memory().unwrap();
        let mut conn = db.conn.lock().unwrap();
        {
            let tx = conn.transaction().unwrap();
            let attempt = begin(&tx, Channel::Analytics, 1000);
            assert!(finish_on(&tx, &attempt, Outcome::Ok, 1100).unwrap());
            let raw: String = tx
                .query_row(
                    "SELECT value FROM settings WHERE key=?1",
                    [SETTING_KEY],
                    |row| row.get(0),
                )
                .unwrap();
            let stored: Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(stored["sources"][0]["analytics"]["status"], "ok");
        }
        assert_eq!(health(&conn, "gemini").status, "unknown");
    }
}
