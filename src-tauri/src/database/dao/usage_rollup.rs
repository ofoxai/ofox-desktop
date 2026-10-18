//! Usage rollup DAO
//!
//! Aggregates proxy_request_logs into daily rollups and prunes old detail rows;
//! rows of a session are also folded into `session_usage_archive` first.
//! Original Codex imports remain as an audit ledger for rebuilt native usage.

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use chrono::{Duration, Local, TimeZone};
use rusqlite::{params, OptionalExtension};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::str::FromStr;

/// How long request-log detail rows are kept before they are rolled up and
/// pruned (session rows are archived first).
pub(crate) const USAGE_DETAIL_RETAIN_DAYS: i64 = 30;

/// The cutoff the prune uses now: rows older than it are rolled up and deleted.
#[cfg(test)]
pub(crate) fn usage_prune_cutoff() -> Result<i64, AppError> {
    compute_local_midnight_cutoff(Local::now(), USAGE_DETAIL_RETAIN_DAYS)
}

const PRUNE_FRONTIER_PREFIX: &str = "usage_prune_frontier_utc_v1:";
// A rebuild replaces these imports in effective statistics, but their original
// request IDs and saved decimal strings must remain available for audit. The
// source is evidence; provider names alone do not identify native imports.
const PRUNABLE_DETAIL_SQL: &str =
    "NOT (app_type='codex' AND COALESCE(data_source,'proxy')='codex_session')";

fn frontier_error() -> AppError {
    AppError::Database("invalid usage prune frontier".into())
}

fn read_frontier_on(conn: &rusqlite::Connection, app: &str) -> Result<Option<Value>, AppError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM maintenance_flags WHERE key=?1",
            [format!("{PRUNE_FRONTIER_PREFIX}{app}")],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|raw| {
        let value: Value = serde_json::from_str(&raw).map_err(|_| frontier_error())?;
        let valid_cutoff = value["cutoff"].is_i64()
            || (value["cutoff"].is_null() && value["legacyUnknown"] == true);
        if value["version"].as_u64() != Some(1)
            || !value["legacyUnknown"].is_boolean()
            || !valid_cutoff
        {
            return Err(frontier_error());
        }
        Ok(value)
    })
    .transpose()
}

/// Bootstrap a conservative UTC upper bound for pre-frontier aggregates. The
/// archive retains absolute seconds; rollups retain a local date only. Legal
/// offsets supported by Chrono are strictly within 24 hours, so the next UTC
/// midnight plus 24 hours bounds every instant in that old local day. This is
/// a temporal boundary, never an event identity or completeness receipt.
fn legacy_frontier_on(conn: &rusqlite::Connection, app: &str) -> Result<Value, AppError> {
    let (last, mut unknown): (Option<i64>, bool) = conn.query_row(
        "SELECT MAX(CASE WHEN typeof(last_at)='integer' THEN last_at END),
            COALESCE(MAX(typeof(last_at)!='integer'),0)
         FROM session_usage_archive WHERE app_type=?1",
        [app],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut cutoff = last.and_then(|last| {
        let next = last.checked_add(1);
        unknown |= next.is_none();
        next
    });
    let mut stmt =
        conn.prepare("SELECT DISTINCT date FROM usage_daily_rollups WHERE app_type=?1")?;
    let dates = stmt.query_map([app], |r| r.get::<_, rusqlite::types::Value>(0))?;
    for date in dates {
        let date = match date? {
            rusqlite::types::Value::Text(date) => Some(date),
            _ => None,
        };
        let upper = date.and_then(|date| {
            let parsed = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()?;
            if date.len() != 10 || parsed.format("%Y-%m-%d").to_string() != date {
                return None;
            }
            parsed
                .checked_add_signed(Duration::days(2))?
                .and_hms_opt(0, 0, 0)
                .map(|dt| dt.and_utc().timestamp())
        });
        match upper {
            Some(upper) => cutoff = Some(cutoff.map_or(upper, |prior| prior.max(upper))),
            None => unknown = true,
        }
    }
    Ok(json!({"version":1,"cutoff":cutoff,"legacyUnknown":unknown}))
}

fn write_frontier_on(
    conn: &rusqlite::Connection,
    app: &str,
    value: &Value,
) -> Result<(), AppError> {
    let raw = serde_json::to_string(value).map_err(|_| frontier_error())?;
    conn.execute(
        "INSERT INTO maintenance_flags(key,value) VALUES(?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![format!("{PRUNE_FRONTIER_PREFIX}{app}"), raw],
    )?;
    Ok(())
}

/// Called only inside the same facts transaction as the unknown native event.
/// A recent timestamp does not upgrade partial history into native-ID proof.
pub(crate) fn unseen_event_is_after_history_on(
    conn: &rusqlite::Connection,
    app: &str,
    created_at: i64,
) -> Result<bool, AppError> {
    let history: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM session_usage_archive WHERE app_type=?1) OR EXISTS(SELECT 1 FROM usage_daily_rollups WHERE app_type=?1)", [app], |r| r.get(0))?;
    if !history {
        return Ok(true);
    }
    let frontier = match read_frontier_on(conn, app)? {
        Some(frontier) => frontier,
        None => {
            let frontier = legacy_frontier_on(conn, app)?;
            write_frontier_on(conn, app, &frontier)?;
            frontier
        }
    };
    Ok(frontier["legacyUnknown"] == false
        && frontier["cutoff"]
            .as_i64()
            .is_some_and(|cutoff| created_at >= cutoff))
}

fn advance_frontiers_on(conn: &rusqlite::Connection, cutoff: i64) -> Result<(), AppError> {
    let apps = conn
        .prepare(&format!("SELECT DISTINCT app_type FROM proxy_request_logs WHERE created_at<?1 AND {PRUNABLE_DETAIL_SQL}"))?
        .query_map([cutoff], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for app in apps {
        let mut frontier = match read_frontier_on(conn, &app)? {
            Some(frontier) => frontier,
            None => legacy_frontier_on(conn, &app)?,
        };
        frontier["cutoff"] = json!(frontier["cutoff"]
            .as_i64()
            .map_or(cutoff, |prior| prior.max(cutoff)));
        write_frontier_on(conn, &app, &frontier)?;
    }
    Ok(())
}

/// Compute the rollup/prune cutoff aligned to a local-day boundary.
///
/// Anything strictly older than the returned timestamp will be aggregated into
/// `usage_daily_rollups` and deleted from `proxy_request_logs`. Aligning to the
/// next local midnight after `(now - retain_days)` guarantees that the youngest
/// rollup row always represents a *complete* local day. Without this alignment
/// the cutoff falls mid-day, leaving the day half-rolled-up and half-pruned —
/// which would silently under-count any range query that touches that day
/// after `compute_rollup_date_bounds` trims partial-coverage rollup days.
fn compute_local_midnight_cutoff(
    now: chrono::DateTime<Local>,
    retain_days: i64,
) -> Result<i64, AppError> {
    let target_day = now
        .checked_sub_signed(Duration::days(retain_days))
        .ok_or_else(|| AppError::Database("rollup cutoff overflow".to_string()))?
        .date_naive();

    // Use the *next* day's midnight so anything before it has fully been bucketed.
    let next_day = target_day
        .succ_opt()
        .ok_or_else(|| AppError::Database("rollup cutoff next-day overflow".to_string()))?;
    let naive_midnight = next_day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::Database("rollup cutoff midnight overflow".to_string()))?;

    let local_dt = match Local.from_local_datetime(&naive_midnight) {
        chrono::LocalResult::Single(dt) => dt,
        chrono::LocalResult::Ambiguous(earliest, _) => earliest,
        chrono::LocalResult::None => {
            // DST gap: fall back to one hour later, which always exists.
            let bumped = naive_midnight + Duration::hours(1);
            match Local.from_local_datetime(&bumped) {
                chrono::LocalResult::Single(dt) => dt,
                chrono::LocalResult::Ambiguous(earliest, _) => earliest,
                chrono::LocalResult::None => {
                    return Err(AppError::Database(
                        "rollup cutoff fell into DST gap".to_string(),
                    ))
                }
            }
        }
    };

    Ok(local_dt.timestamp())
}

/// A cost stored as text, read as a decimal: plain (`0.000009`), or the
/// exponent form SQLite gives a REAL cast to text (`9.0e-06`). Unreadable text
/// counts as 0.
pub(crate) fn parse_cost_usd(text: &str) -> Decimal {
    let text = text.trim();
    Decimal::from_str(text)
        .or_else(|_| Decimal::from_scientific(text))
        .ok()
        .or_else(|| {
            text.parse::<f64>()
                .ok()
                .and_then(|f| Decimal::try_from(f).ok())
        })
        .unwrap_or(Decimal::ZERO)
}

/// One session/model's archived usage while it is being summed.
struct ArchiveTotals {
    tokens: [i64; 4],
    count: i64,
    cost: Decimal,
    first_at: i64,
    last_at: i64,
}

impl ArchiveTotals {
    fn add(&mut self, other: &ArchiveTotals) {
        for (sum, n) in self.tokens.iter_mut().zip(other.tokens) {
            *sum += n;
        }
        self.count += other.count;
        self.cost += other.cost;
        self.first_at = self.first_at.min(other.first_at);
        self.last_at = self.last_at.max(other.last_at);
    }
}

impl Database {
    /// Aggregate eligible proxy_request_logs older than `retain_days` into
    /// usage_daily_rollups, then delete the aggregated detail rows. Original
    /// Codex imports are retained for audit without reaggregating them.
    /// Returns the number of deleted detail rows.
    pub fn rollup_and_prune(&self, retain_days: i64) -> Result<u64, AppError> {
        let cutoff = compute_local_midnight_cutoff(Local::now(), retain_days)?;
        self.rollup_and_prune_before(cutoff)
    }

    /// `rollup_and_prune` for the rows older than `cutoff`.
    pub(crate) fn rollup_and_prune_before(&self, cutoff: i64) -> Result<u64, AppError> {
        let conn = lock_conn!(self.conn);

        // Check if there are any rows to process
        let count: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM proxy_request_logs WHERE created_at < ?1 AND {PRUNABLE_DETAIL_SQL}"),
                [cutoff],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if count == 0 {
            return Ok(0);
        }

        // Use a savepoint for atomicity
        conn.execute("SAVEPOINT rollup_prune;", [])
            .map_err(|e| AppError::Database(e.to_string()))?;

        let result = Self::do_rollup_and_prune(&conn, cutoff);

        match result {
            Ok(deleted) => {
                conn.execute("RELEASE rollup_prune;", [])
                    .map_err(|e| AppError::Database(e.to_string()))?;
                if deleted > 0 {
                    log::info!(
                        "Rolled up and pruned {deleted} proxy_request_logs (before {cutoff})"
                    );
                }
                Ok(deleted)
            }
            Err(e) => {
                conn.execute("ROLLBACK TO rollup_prune;", []).ok();
                conn.execute("RELEASE rollup_prune;", []).ok();
                Err(e)
            }
        }
    }

    /// Fold the rows about to be pruned that belong to a session into
    /// `session_usage_archive`, so a session's totals (live rows + archive) stay
    /// whole after the detail rows go. Costs are summed as decimals and stored as
    /// plain decimal text (never SQLite's `9.0e-06` form of a REAL).
    fn archive_session_rows(conn: &rusqlite::Connection, cutoff: i64) -> Result<(), AppError> {
        let db_err =
            |e: rusqlite::Error| AppError::Database(format!("Archiving session usage failed: {e}"));
        let mut totals: BTreeMap<(String, String, String), ArchiveTotals> = BTreeMap::new();
        {
            let mut stmt = conn
                .prepare(
                    &format!("SELECT app_type, session_id, model, input_tokens, output_tokens,
                            cache_read_tokens, cache_creation_tokens, total_cost_usd, created_at
                     FROM proxy_request_logs
                     WHERE created_at < ?1 AND session_id IS NOT NULL AND session_id != '' AND {PRUNABLE_DETAIL_SQL}"),
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([cutoff], |r| {
                    Ok((
                        (r.get(0)?, r.get(1)?, r.get(2)?),
                        ArchiveTotals {
                            tokens: [r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?],
                            count: 1,
                            cost: parse_cost_usd(&r.get::<_, String>(7)?),
                            first_at: r.get(8)?,
                            last_at: r.get(8)?,
                        },
                    ))
                })
                .map_err(db_err)?;
            for row in rows {
                let (key, row) = row.map_err(db_err)?;
                match totals.get_mut(&key) {
                    Some(sum) => sum.add(&row),
                    None => {
                        totals.insert(key, row);
                    }
                }
            }
        }

        for ((app_type, session_id, model), mut sum) in totals {
            let archived = conn
                .query_row(
                    "SELECT COALESCE(input_tokens, 0), COALESCE(output_tokens, 0),
                            COALESCE(cache_read_tokens, 0), COALESCE(cache_creation_tokens, 0),
                            COALESCE(request_count, 0), COALESCE(cost_usd, '0'),
                            first_at, last_at
                     FROM session_usage_archive
                     WHERE app_type = ?1 AND session_id = ?2 AND model = ?3",
                    params![app_type, session_id, model],
                    |r| {
                        Ok(ArchiveTotals {
                            tokens: [r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?],
                            count: r.get(4)?,
                            cost: parse_cost_usd(&r.get::<_, String>(5)?),
                            first_at: r.get::<_, Option<i64>>(6)?.unwrap_or(i64::MAX),
                            last_at: r.get::<_, Option<i64>>(7)?.unwrap_or(i64::MIN),
                        })
                    },
                )
                .optional()
                .map_err(db_err)?;
            if let Some(archived) = archived {
                sum.add(&archived);
            }
            conn.execute(
                "INSERT OR REPLACE INTO session_usage_archive
                    (app_type, session_id, model, input_tokens, output_tokens,
                     cache_read_tokens, cache_creation_tokens, request_count, cost_usd,
                     first_at, last_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    app_type,
                    session_id,
                    model,
                    sum.tokens[0],
                    sum.tokens[1],
                    sum.tokens[2],
                    sum.tokens[3],
                    sum.count,
                    sum.cost.normalize().to_string(),
                    sum.first_at,
                    sum.last_at,
                ],
            )
            .map_err(db_err)?;
        }
        Ok(())
    }

    fn do_rollup_and_prune(conn: &rusqlite::Connection, cutoff: i64) -> Result<u64, AppError> {
        // Publish the UTC boundary in the same savepoint as the aggregates and
        // deletion. Bootstrap before adding new aggregates to distinguish
        // pre-frontier legacy history from this transaction's known cutoff.
        advance_frontiers_on(conn, cutoff)?;
        // Aggregate old logs, merging with any pre-existing rollup rows via LEFT JOIN.
        conn.execute(
            &format!("INSERT OR REPLACE INTO usage_daily_rollups
                (date, app_type, provider_id, model,
                 request_count, success_count,
                 input_tokens, output_tokens,
                 cache_read_tokens, cache_creation_tokens,
                 total_cost_usd, avg_latency_ms)
            SELECT
                d, a, p, m,
                COALESCE(old.request_count, 0) + new_req,
                COALESCE(old.success_count, 0) + new_succ,
                COALESCE(old.input_tokens, 0) + new_in,
                COALESCE(old.output_tokens, 0) + new_out,
                COALESCE(old.cache_read_tokens, 0) + new_cr,
                COALESCE(old.cache_creation_tokens, 0) + new_cc,
                CAST(COALESCE(CAST(old.total_cost_usd AS REAL), 0) + new_cost AS TEXT),
                CASE WHEN COALESCE(old.request_count, 0) + new_req > 0
                    THEN (COALESCE(old.avg_latency_ms, 0) * COALESCE(old.request_count, 0)
                          + new_lat * new_req)
                         / (COALESCE(old.request_count, 0) + new_req)
                    ELSE 0 END
            FROM (
                SELECT
                    date(created_at, 'unixepoch', 'localtime') as d,
                    app_type as a, provider_id as p, model as m,
                    COUNT(*) as new_req,
                    SUM(CASE WHEN status_code >= 200 AND status_code < 300 THEN 1 ELSE 0 END) as new_succ,
                    COALESCE(SUM(input_tokens), 0) as new_in,
                    COALESCE(SUM(output_tokens), 0) as new_out,
                    COALESCE(SUM(cache_read_tokens), 0) as new_cr,
                    COALESCE(SUM(cache_creation_tokens), 0) as new_cc,
                    COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0) as new_cost,
                    COALESCE(AVG(latency_ms), 0) as new_lat
                FROM proxy_request_logs WHERE created_at < ?1 AND {PRUNABLE_DETAIL_SQL}
                GROUP BY d, a, p, m
            ) agg
            LEFT JOIN usage_daily_rollups old
                ON old.date = agg.d AND old.app_type = agg.a
                AND old.provider_id = agg.p AND old.model = agg.m"),
            [cutoff],
        )
        .map_err(|e| AppError::Database(format!("Rollup aggregation failed: {e}")))?;

        Self::archive_session_rows(conn, cutoff)?;

        // Delete the aggregated detail rows
        let deleted = conn
            .execute(
                &format!("DELETE FROM proxy_request_logs WHERE created_at < ?1 AND {PRUNABLE_DETAIL_SQL}"),
                [cutoff],
            )
            .map_err(|e| AppError::Database(format!("Pruning old logs failed: {e}")))?;

        Ok(deleted as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::compute_local_midnight_cutoff;
    use crate::database::Database;
    use crate::error::AppError;
    use chrono::{Local, TimeZone};

    type SavedCodexAudit = (String, String, [i64; 4], [String; 5]);

    fn saved_codex_audit(db: &Database) -> Vec<SavedCodexAudit> {
        let conn = db.conn.lock().unwrap();
        let mut statement = conn.prepare("SELECT request_id,provider_id,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,input_cost_usd,output_cost_usd,cache_read_cost_usd,cache_creation_cost_usd,total_cost_usd FROM proxy_request_logs WHERE app_type='codex' AND data_source='codex_session' ORDER BY request_id").unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?],
                    [
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                    ],
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn insert_codex_audit_history(db: &Database) -> Vec<SavedCodexAudit> {
        let conn = db.conn.lock().unwrap();
        for (index, provider) in ["_codex_session", "_session", "retired-provider"]
            .into_iter()
            .enumerate()
        {
            conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,input_cost_usd,output_cost_usd,cache_read_cost_usd,cache_creation_cost_usd,total_cost_usd,latency_ms,status_code,session_id,created_at,data_source) VALUES(?1,?2,'codex','native-saved',11,22,33,44,'0.000000100000000000','1.2300e-7','0.00000000000','0','0.000000223000000000',0,200,'thread-old',1,'codex_session')", rusqlite::params![format!("old-codex-id-{index}"),provider]).unwrap();
        }
        conn.execute("INSERT INTO usage_daily_rollups(date,app_type,provider_id,model,request_count,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,total_cost_usd) VALUES('1970-01-01','codex','_codex_session','native-saved',7,1,2,3,4,'9.870000e-7')", []).unwrap();
        drop(conn);
        saved_codex_audit(db)
    }

    fn saved_codex_rollup(db: &Database) -> (i64, [i64; 4], String) {
        db.conn.lock().unwrap().query_row("SELECT request_count,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,total_cost_usd FROM usage_daily_rollups WHERE app_type='codex' AND provider_id='_codex_session' AND model='native-saved'", [], |row| Ok((row.get(0)?,[row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?],row.get(5)?))).unwrap()
    }

    #[test]
    #[serial_test::serial]
    fn native_codex_audit_survives_first_and_repeated_pruning_without_reaggregating() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let expected = insert_codex_audit_history(&db);
        let rollup = saved_codex_rollup(&db);
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at,data_source) VALUES('codex-real-http','_codex_session','codex','http-measured',5,40,429,1,'proxy'),('claude-import','_session','claude','claude-native',6,0,0,1,'session_log')",[]).unwrap();
        for (pass, cutoff) in [86_400_i64, 86_400, 172_800].into_iter().enumerate() {
            let deleted = db.rollup_and_prune_before(cutoff).unwrap();
            assert_eq!(saved_codex_audit(&db), expected, "maintenance pass {pass} must retain original Codex IDs, four Token buckets, and exact saved decimal strings");
            assert_eq!(
                saved_codex_rollup(&db),
                rollup,
                "retained audit rows must not be folded into an existing legacy rollup again"
            );
            assert_eq!(deleted, if pass == 0 { 2 } else { 0 });
            let conn = db.conn.lock().unwrap();
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM proxy_request_logs WHERE request_id IN ('codex-real-http','claude-import')", [], |row| row.get::<_, i64>(0)).unwrap(), 0, "unrelated proxy and native sources retain normal pruning");
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM session_usage_archive WHERE app_type='codex' AND session_id='thread-old'", [], |row| row.get::<_, i64>(0)).unwrap(), 0, "preserved audit rows are not duplicated in the session archive");
        }
    }

    #[test]
    #[serial_test::serial]
    fn native_codex_audit_survives_database_startup_maintenance_and_restart() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::init().unwrap();
        let expected = insert_codex_audit_history(&db);
        let rollup = saved_codex_rollup(&db);
        drop(db);
        for restart in 0..2 {
            let db = Database::init().unwrap();
            assert_eq!(
                saved_codex_audit(&db),
                expected,
                "startup {restart} must preserve historical Codex details"
            );
            assert_eq!(saved_codex_rollup(&db), rollup);
        }
    }

    fn local_dt(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> chrono::DateTime<Local> {
        match Local.with_ymd_and_hms(year, month, day, hour, minute, second) {
            chrono::LocalResult::Single(dt) => dt,
            chrono::LocalResult::Ambiguous(earliest, _) => earliest,
            chrono::LocalResult::None => panic!("invalid local datetime in test fixture"),
        }
    }

    #[test]
    fn cutoff_is_aligned_to_local_midnight_after_target_day() -> Result<(), AppError> {
        // now = 2026-04-16 14:32:17 local; retain_days = 30
        // target day = 2026-03-17; cutoff should be 2026-03-18 00:00 local.
        let now = local_dt(2026, 4, 16, 14, 32, 17);
        let cutoff_ts = compute_local_midnight_cutoff(now, 30)?;
        let cutoff_dt = Local.timestamp_opt(cutoff_ts, 0).single().unwrap();
        let expected = local_dt(2026, 3, 18, 0, 0, 0);
        assert_eq!(cutoff_dt, expected);
        Ok(())
    }

    #[test]
    fn cutoff_at_local_midnight_now_still_lands_on_midnight() -> Result<(), AppError> {
        // If `now` is itself local midnight, the math should not introduce drift.
        let now = local_dt(2026, 4, 16, 0, 0, 0);
        let cutoff_ts = compute_local_midnight_cutoff(now, 7)?;
        let cutoff_dt = Local.timestamp_opt(cutoff_ts, 0).single().unwrap();
        // (2026-04-16 - 7d) = 2026-04-09; cutoff = 2026-04-10 00:00 local.
        let expected = local_dt(2026, 4, 10, 0, 0, 0);
        assert_eq!(cutoff_dt, expected);
        Ok(())
    }

    #[test]
    fn test_rollup_and_prune() -> Result<(), AppError> {
        let db = Database::memory()?;
        let now = chrono::Utc::now().timestamp();
        let old_ts = now - 40 * 86400; // 40 days ago
        let recent_ts = now - 5 * 86400; // 5 days ago

        {
            let conn = crate::database::lock_conn!(db.conn);
            for i in 0..5 {
                conn.execute(
                    "INSERT INTO proxy_request_logs (
                        request_id, provider_id, app_type, model,
                        input_tokens, output_tokens, total_cost_usd,
                        latency_ms, status_code, created_at
                    ) VALUES (?1, 'p1', 'claude', 'claude-3', 100, 50, '0.01', 100, 200, ?2)",
                    rusqlite::params![format!("old-{i}"), old_ts + i as i64],
                )?;
            }
            for i in 0..3 {
                conn.execute(
                    "INSERT INTO proxy_request_logs (
                        request_id, provider_id, app_type, model,
                        input_tokens, output_tokens, total_cost_usd,
                        latency_ms, status_code, created_at
                    ) VALUES (?1, 'p1', 'claude', 'claude-3', 200, 100, '0.02', 150, 200, ?2)",
                    rusqlite::params![format!("recent-{i}"), recent_ts + i as i64],
                )?;
            }
        }

        let deleted = db.rollup_and_prune(30)?;
        assert_eq!(deleted, 5);

        // Verify rollup data
        let conn = crate::database::lock_conn!(db.conn);
        let count: i64 = conn.query_row(
            "SELECT request_count FROM usage_daily_rollups WHERE app_type = 'claude'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 5);

        // Verify recent logs untouched
        let remaining: i64 =
            conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                row.get(0)
            })?;
        assert_eq!(remaining, 3);
        Ok(())
    }

    #[test]
    fn test_rollup_noop_when_no_old_data() -> Result<(), AppError> {
        let db = Database::memory()?;
        assert_eq!(db.rollup_and_prune(30)?, 0);
        Ok(())
    }

    #[test]
    fn test_rollup_merges_with_existing() -> Result<(), AppError> {
        let db = Database::memory()?;
        let now = chrono::Utc::now().timestamp();
        let old_ts = now - 40 * 86400;

        {
            let conn = crate::database::lock_conn!(db.conn);
            let date_str = Local
                .timestamp_opt(old_ts, 0)
                .single()
                .expect("test timestamp is unambiguous")
                .format("%Y-%m-%d")
                .to_string();
            conn.execute(
                "INSERT INTO usage_daily_rollups
                    (date, app_type, provider_id, model, request_count, success_count,
                     input_tokens, output_tokens, total_cost_usd, avg_latency_ms)
                 VALUES (?1, 'claude', 'p1', 'claude-3', 10, 10, 1000, 500, '0.10', 100)",
                [&date_str],
            )?;
            for i in 0..3 {
                conn.execute(
                    "INSERT INTO proxy_request_logs (
                        request_id, provider_id, app_type, model,
                        input_tokens, output_tokens, total_cost_usd,
                        latency_ms, status_code, created_at
                    ) VALUES (?1, 'p1', 'claude', 'claude-3', 100, 50, '0.01', 200, 200, ?2)",
                    rusqlite::params![format!("merge-{i}"), old_ts + i as i64],
                )?;
            }
        }

        let deleted = db.rollup_and_prune(30)?;
        assert_eq!(deleted, 3);

        let conn = crate::database::lock_conn!(db.conn);
        let (count, input): (i64, i64) = conn.query_row(
            "SELECT request_count, input_tokens FROM usage_daily_rollups
             WHERE app_type = 'claude' AND provider_id = 'p1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(count, 13, "10 existing + 3 new");
        assert_eq!(input, 1300, "1000 existing + 300 new");
        Ok(())
    }

    /// (app_type, session_id, model) -> (input, output, cache read, cache write,
    /// requests, cost, first_at, last_at): live rows plus the archive.
    type SessionTotals = std::collections::BTreeMap<
        (String, String, String),
        (i64, i64, i64, i64, i64, f64, i64, i64),
    >;

    fn session_totals(db: &Database) -> Result<SessionTotals, AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        let mut stmt = conn.prepare(
            "SELECT app_type, session_id, model,
                    SUM(i), SUM(o), SUM(cr), SUM(cc), SUM(n), SUM(cost), MIN(f), MAX(l)
             FROM (
                SELECT app_type, session_id, model, input_tokens i, output_tokens o,
                       cache_read_tokens cr, cache_creation_tokens cc, 1 n,
                       CAST(total_cost_usd AS REAL) cost, created_at f, created_at l
                FROM proxy_request_logs WHERE session_id IS NOT NULL AND session_id != ''
                UNION ALL
                SELECT app_type, session_id, model, input_tokens, output_tokens,
                       cache_read_tokens, cache_creation_tokens, request_count,
                       CAST(cost_usd AS REAL), first_at, last_at
                FROM session_usage_archive
             ) GROUP BY app_type, session_id, model",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                (r.get(0)?, r.get(1)?, r.get(2)?),
                (
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ),
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn insert_log(
        db: &Database,
        id: &str,
        session: Option<&str>,
        model: &str,
        tokens: (i64, i64, i64, i64),
        cost: &str,
        created_at: i64,
    ) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, provider_id, app_type, model, input_tokens, output_tokens,
                cache_read_tokens, cache_creation_tokens, total_cost_usd, latency_ms,
                status_code, session_id, created_at, data_source
            ) VALUES (?1, '_session', 'claude', ?2, ?3, ?4, ?5, ?6, ?7, 0, 200, ?8, ?9, 'session_log')",
            rusqlite::params![
                id, model, tokens.0, tokens.1, tokens.2, tokens.3, cost, session, created_at
            ],
        )?;
        Ok(())
    }

    #[test]
    fn prune_moves_session_rows_into_the_archive_and_keeps_session_totals() -> Result<(), AppError>
    {
        let db = Database::memory()?;
        let now = chrono::Utc::now().timestamp();
        let old = now - 40 * 86400;
        let older = now - 50 * 86400;
        let recent = now - 2 * 86400;
        insert_log(
            &db,
            "a1",
            Some("sess-a"),
            "opus",
            (3, 100, 5000, 200),
            "0.25",
            older,
        )?;
        insert_log(
            &db,
            "a2",
            Some("sess-a"),
            "opus",
            (5, 50, 6000, 0),
            "0.5",
            old,
        )?;
        insert_log(
            &db,
            "a3",
            Some("sess-a"),
            "haiku",
            (1, 10, 0, 0),
            "0.125",
            old,
        )?;
        insert_log(
            &db,
            "a4",
            Some("sess-a"),
            "opus",
            (2, 20, 100, 0),
            "0.0625",
            recent,
        )?;
        insert_log(
            &db,
            "b1",
            Some("sess-b"),
            "opus",
            (7, 70, 700, 70),
            "1.5",
            old,
        )?;
        insert_log(&db, "no-session", None, "opus", (9, 9, 9, 9), "2", old)?;
        insert_log(
            &db,
            "empty-session",
            Some(""),
            "opus",
            (9, 9, 9, 9),
            "2",
            old,
        )?;
        let before = session_totals(&db)?;

        let deleted = db.rollup_and_prune(30)?;

        assert_eq!(deleted, 6);
        assert_eq!(
            session_totals(&db)?,
            before,
            "per-session totals survive the prune"
        );
        let conn = crate::database::lock_conn!(db.conn);
        let archived: (i64, i64, i64, i64, i64, String, i64, i64) = conn.query_row(
            "SELECT input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    request_count, cost_usd, first_at, last_at
             FROM session_usage_archive
             WHERE app_type = 'claude' AND session_id = 'sess-a' AND model = 'opus'",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )?;
        assert_eq!(
            archived,
            (8, 150, 11000, 200, 2, "0.75".to_string(), older, old),
            "only the pruned rows are archived"
        );
        let archive_rows: i64 =
            conn.query_row("SELECT COUNT(*) FROM session_usage_archive", [], |r| {
                r.get(0)
            })?;
        assert_eq!(archive_rows, 3, "rows without a session are not archived");
        Ok(())
    }

    #[test]
    fn a_later_prune_adds_to_the_archived_session_totals() -> Result<(), AppError> {
        let db = Database::memory()?;
        let now = chrono::Utc::now().timestamp();
        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO session_usage_archive VALUES
                    ('claude', 'sess-a', 'opus', 10, 20, 30, 40, 3, '1.5', ?1, ?2)",
                [now - 90 * 86400, now - 80 * 86400],
            )?;
        }
        insert_log(
            &db,
            "a5",
            Some("sess-a"),
            "opus",
            (1, 2, 3, 4),
            "0.25",
            now - 40 * 86400,
        )?;
        let before = session_totals(&db)?;

        db.rollup_and_prune(30)?;

        assert_eq!(session_totals(&db)?, before);
        Ok(())
    }

    fn archived_cost(db: &Database, session: &str) -> Result<String, AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        Ok(conn.query_row(
            "SELECT cost_usd FROM session_usage_archive WHERE session_id = ?1",
            [session],
            |r| r.get(0),
        )?)
    }

    #[test]
    fn a_tiny_cost_is_archived_as_a_plain_decimal() -> Result<(), AppError> {
        let db = Database::memory()?;
        let old = chrono::Utc::now().timestamp() - 40 * 86400;
        insert_log(
            &db,
            "tiny",
            Some("sess-tiny"),
            "opus",
            (3, 0, 0, 0),
            "0.000009",
            old,
        )?;

        db.rollup_and_prune(30)?;

        let cost = archived_cost(&db, "sess-tiny")?;
        assert_eq!(cost, "0.000009");
        assert_eq!(
            rust_decimal::Decimal::from_str_exact(&cost).ok(),
            rust_decimal::Decimal::from_str_exact("0.000009").ok()
        );
        Ok(())
    }

    #[test]
    fn a_prune_adds_to_an_archived_cost_written_in_exponent_form() -> Result<(), AppError> {
        let db = Database::memory()?;
        let now = chrono::Utc::now().timestamp();
        {
            // what SQLite's CAST(REAL AS TEXT) wrote before
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO session_usage_archive VALUES
                    ('claude', 'sess-exp', 'opus', 1, 0, 0, 0, 1, '1.0e-06', ?1, ?1)",
                [now - 60 * 86400],
            )?;
        }
        insert_log(
            &db,
            "exp",
            Some("sess-exp"),
            "opus",
            (3, 0, 0, 0),
            "0.000009",
            now - 40 * 86400,
        )?;

        db.rollup_and_prune(30)?;

        assert_eq!(archived_cost(&db, "sess-exp")?, "0.00001");
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn prune_frontier_is_monotonic_when_a_later_cutoff_moves_backwards() {
        let _home = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let high = 1_800_000_000;
        insert_log(&db, "a", Some("s"), "fixture", (1, 1, 0, 0), "0", high - 1).unwrap();
        db.rollup_and_prune_before(high).unwrap();
        let low = high - 16 * 3600;
        insert_log(&db, "b", Some("s"), "fixture", (1, 1, 0, 0), "0", low - 1).unwrap();
        db.rollup_and_prune_before(low).unwrap();
        let conn = db.conn.lock().unwrap();
        let value = conn.query_row("SELECT value FROM maintenance_flags WHERE key='usage_prune_frontier_utc_v1:claude'", [], |r| r.get::<_, String>(0)).ok();
        let value: Option<serde_json::Value> = value.map(|s| serde_json::from_str(&s).unwrap());
        assert_eq!(
            value.as_ref().and_then(|v| v["cutoff"].as_i64()),
            Some(high)
        );
        assert_eq!(
            value.as_ref().map(|v| &v["legacyUnknown"]),
            Some(&serde_json::Value::Bool(false))
        );
    }

    #[test]
    #[serial_test::serial]
    fn prune_frontier_write_failure_rolls_back_facts_rollup_and_archive() {
        let _home = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let cutoff = 1_800_000_000;
        insert_log(
            &db,
            "a",
            Some("s"),
            "fixture",
            (1, 1, 0, 0),
            "0",
            cutoff - 1,
        )
        .unwrap();
        db.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_prune_frontier BEFORE INSERT ON maintenance_flags WHEN substr(NEW.key,1,28)='usage_prune_frontier_utc_v1:' BEGIN SELECT RAISE(FAIL,'fixture frontier failure'); END;").unwrap();
        assert!(
            db.rollup_and_prune_before(cutoff).is_err(),
            "prune did not atomically publish its historical boundary"
        );
        let conn = db.conn.lock().unwrap();
        for (table, count) in [
            ("proxy_request_logs", 1),
            ("usage_daily_rollups", 0),
            ("session_usage_archive", 0),
            ("maintenance_flags", 0),
        ] {
            let actual: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                actual, count,
                "{table} changed despite the failed frontier write"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn prune_frontier_keeps_unknown_fields_and_a_higher_previous_boundary() {
        let _home = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let cutoff = 1_800_000_000;
        let before = serde_json::json!({"version":1,"cutoff":cutoff+16*3600,"legacyUnknown":false,"future":{"opaque":7}});
        db.conn.lock().unwrap().execute("INSERT INTO maintenance_flags(key,value) VALUES('usage_prune_frontier_utc_v1:claude',?1)", [before.to_string()]).unwrap();
        insert_log(
            &db,
            "a",
            Some("s"),
            "fixture",
            (1, 1, 0, 0),
            "0",
            cutoff - 1,
        )
        .unwrap();
        db.rollup_and_prune_before(cutoff).unwrap();
        let raw: String = db.conn.lock().unwrap().query_row("SELECT value FROM maintenance_flags WHERE key='usage_prune_frontier_utc_v1:claude'", [], |r| r.get(0)).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&raw).unwrap(),
            before
        );
    }

    #[test]
    #[serial_test::serial]
    fn prune_frontier_refuses_unknown_or_corrupt_formats_without_deleting_facts() {
        let _home = crate::test_sandbox::sandbox();
        for raw in [
            "{broken",
            r#"{"version":999,"cutoff":1,"legacyUnknown":false,"future":7}"#,
        ] {
            let db = Database::memory().unwrap();
            let cutoff = 1_800_000_000;
            db.conn.lock().unwrap().execute("INSERT INTO maintenance_flags(key,value) VALUES('usage_prune_frontier_utc_v1:claude',?1)", [raw]).unwrap();
            insert_log(
                &db,
                "a",
                Some("s"),
                "fixture",
                (1, 1, 0, 0),
                "0",
                cutoff - 1,
            )
            .unwrap();
            assert!(db.rollup_and_prune_before(cutoff).is_err());
            let conn = db.conn.lock().unwrap();
            let after: String = conn.query_row("SELECT value FROM maintenance_flags WHERE key='usage_prune_frontier_utc_v1:claude'", [], |r| r.get(0)).unwrap();
            assert_eq!(after, raw);
            let live: i64 = conn
                .query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r.get(0))
                .unwrap();
            assert_eq!(live, 1);
        }
    }
}
