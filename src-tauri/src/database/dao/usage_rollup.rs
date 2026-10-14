//! Usage rollup DAO
//!
//! Aggregates proxy_request_logs into daily rollups and prunes old detail rows;
//! rows of a session are also folded into `session_usage_archive` first.

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use chrono::{Duration, Local, TimeZone};
use rusqlite::{params, OptionalExtension};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::str::FromStr;

/// How long request-log detail rows are kept before they are rolled up and
/// pruned (session rows are archived first).
pub(crate) const USAGE_DETAIL_RETAIN_DAYS: i64 = 30;

/// The cutoff the prune uses now: rows older than it are rolled up and deleted.
pub(crate) fn usage_prune_cutoff() -> Result<i64, AppError> {
    compute_local_midnight_cutoff(Local::now(), USAGE_DETAIL_RETAIN_DAYS)
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
    /// Aggregate proxy_request_logs older than `retain_days` into usage_daily_rollups,
    /// then delete the aggregated detail rows.
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
                "SELECT COUNT(*) FROM proxy_request_logs WHERE created_at < ?1",
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
                    "SELECT app_type, session_id, model, input_tokens, output_tokens,
                            cache_read_tokens, cache_creation_tokens, total_cost_usd, created_at
                     FROM proxy_request_logs
                     WHERE created_at < ?1 AND session_id IS NOT NULL AND session_id != ''",
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
        // Aggregate old logs, merging with any pre-existing rollup rows via LEFT JOIN.
        conn.execute(
            "INSERT OR REPLACE INTO usage_daily_rollups
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
                FROM proxy_request_logs WHERE created_at < ?1
                GROUP BY d, a, p, m
            ) agg
            LEFT JOIN usage_daily_rollups old
                ON old.date = agg.d AND old.app_type = agg.a
                AND old.provider_id = agg.p AND old.model = agg.m",
            [cutoff],
        )
        .map_err(|e| AppError::Database(format!("Rollup aggregation failed: {e}")))?;

        Self::archive_session_rows(conn, cutoff)?;

        // Delete the aggregated detail rows
        let deleted = conn
            .execute(
                "DELETE FROM proxy_request_logs WHERE created_at < ?1",
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
}
