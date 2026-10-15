//! One-shot usage maintenance that the v10 -> v11 migration leaves pending in
//! `maintenance_flags`, run in the background after the pricing sync and a
//! session sync (see `usage_sync`).
//!
//! - `recompute_claude_costs`: Claude session rows were priced with a formula
//!   that subtracted cache reads from Claude's input, which already excluded
//!   them. Their costs, and those of the Claude session rollups and archive, are
//!   priced again with `CostCalculator`.
//! - `backfill_session_archive`: sessions whose old rows were pruned before v11
//!   have no provable event identities. Preserve existing aggregates and record
//!   permanent partial coverage; current parsed totals cannot reconstruct or
//!   safely subtract their contributions. Explicit future proof upgrades may
//!   requeue this maintenance step.
//!
//! A step marks its flag `done` only after it succeeded; a missing flag (a
//! database that never went through the v11 migration) means nothing to do.

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::proxy::usage::calculator::{CostCalculator, ModelPricing};
use crate::proxy::usage::parser::TokenUsage;
use crate::services::session_usage::find_model_pricing_for_session;
use rusqlite::{params, Connection, OptionalExtension};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;

pub(crate) const RECOMPUTE_CLAUDE_COSTS: &str = "recompute_claude_costs";
pub(crate) const BACKFILL_SESSION_ARCHIVE: &str = "backfill_session_archive";
const PENDING: &str = "pending";
const DONE: &str = "done";

/// Run every maintenance step whose flag is pending.
pub fn run_pending(db: &Database) -> Result<(), AppError> {
    if is_pending(db, RECOMPUTE_CLAUDE_COSTS)? {
        let updated = recompute_claude_costs(db)?;
        mark_done(db, RECOMPUTE_CLAUDE_COSTS)?;
        log::debug!("[USAGE-MAINT] recomputed Claude costs on {updated} rows");
    }
    if is_pending(db, BACKFILL_SESSION_ARCHIVE)? {
        let outcome = backfill_session_archive(db)?;
        log::debug!("[USAGE-MAINT] archive backfill: {outcome:?}");
    }
    Ok(())
}

fn is_pending(db: &Database, key: &str) -> Result<bool, AppError> {
    let conn = lock_conn!(db.conn);
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM maintenance_flags WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| AppError::Database(format!("读取 maintenance_flags 失败: {e}")))?;
    Ok(value.as_deref() == Some(PENDING))
}

fn mark_done(db: &Database, key: &str) -> Result<(), AppError> {
    let conn = lock_conn!(db.conn);
    conn.execute(
        "INSERT OR REPLACE INTO maintenance_flags (key, value) VALUES (?1, ?2)",
        params![key, DONE],
    )
    .map_err(|e| AppError::Database(format!("写入 maintenance_flags 失败: {e}")))?;
    Ok(())
}

fn usage_of(tokens: [i64; 4]) -> TokenUsage {
    let clamp = |n: i64| n.clamp(0, u32::MAX as i64) as u32;
    TokenUsage {
        input_tokens: clamp(tokens[0]),
        output_tokens: clamp(tokens[1]),
        cache_read_tokens: clamp(tokens[2]),
        cache_creation_tokens: clamp(tokens[3]),
        model: None,
        message_id: None,
    }
}

/// Price Claude session usage again: live `session_log` rows, the Claude session
/// rollups (`provider_id = '_session'`) and the Claude archive rows. A model with
/// no price is left as it is. Returns the number of rows updated.
pub(crate) fn recompute_claude_costs(db: &Database) -> Result<usize, AppError> {
    let mut conn = lock_conn!(db.conn);
    let tx = conn
        .transaction()
        .map_err(|e| AppError::Database(format!("recompute begin tx: {e}")))?;
    let mut prices: HashMap<String, Option<ModelPricing>> = HashMap::new();
    let mut price = |conn: &Connection, model: &str| -> Option<ModelPricing> {
        prices
            .entry(model.to_string())
            .or_insert_with(|| find_model_pricing_for_session(conn, model))
            .clone()
    };
    let db_err = |e: rusqlite::Error| AppError::Database(format!("recompute Claude costs: {e}"));
    let mut updated = 0;

    let logs: Vec<(String, String, [i64; 4], String)> = {
        let mut stmt = tx
            .prepare(
                "SELECT request_id, model, input_tokens, output_tokens, cache_read_tokens,
                        cache_creation_tokens, cost_multiplier
                 FROM proxy_request_logs WHERE data_source = 'session_log'",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    [r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?],
                    r.get(6)?,
                ))
            })
            .map_err(db_err)?;
        rows.collect::<Result<_, _>>().map_err(db_err)?
    };
    for (request_id, model, tokens, multiplier) in logs {
        let Some(pricing) = price(&tx, &model) else {
            continue;
        };
        let multiplier = Decimal::from_str(multiplier.trim()).unwrap_or(Decimal::ONE);
        let cost = CostCalculator::calculate(&usage_of(tokens), &pricing, multiplier);
        updated += tx
            .execute(
                "UPDATE proxy_request_logs
                 SET input_cost_usd = ?2, output_cost_usd = ?3, cache_read_cost_usd = ?4,
                     cache_creation_cost_usd = ?5, total_cost_usd = ?6
                 WHERE request_id = ?1",
                params![
                    request_id,
                    cost.input_cost.normalize().to_string(),
                    cost.output_cost.normalize().to_string(),
                    cost.cache_read_cost.normalize().to_string(),
                    cost.cache_creation_cost.normalize().to_string(),
                    cost.total_cost.normalize().to_string(),
                ],
            )
            .map_err(db_err)?;
    }

    let rollups: Vec<(String, String, [i64; 4])> = {
        let mut stmt = tx
            .prepare(
                "SELECT date, model, input_tokens, output_tokens, cache_read_tokens,
                        cache_creation_tokens
                 FROM usage_daily_rollups WHERE app_type = 'claude' AND provider_id = '_session'",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    [r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?],
                ))
            })
            .map_err(db_err)?;
        rows.collect::<Result<_, _>>().map_err(db_err)?
    };
    for (date, model, tokens) in rollups {
        let Some(pricing) = price(&tx, &model) else {
            continue;
        };
        let cost = CostCalculator::calculate(&usage_of(tokens), &pricing, Decimal::ONE);
        updated += tx
            .execute(
                "UPDATE usage_daily_rollups SET total_cost_usd = ?3
                 WHERE date = ?1 AND app_type = 'claude' AND provider_id = '_session'
                   AND model = ?2",
                params![date, model, cost.total_cost.normalize().to_string()],
            )
            .map_err(db_err)?;
    }

    let archived: Vec<(String, String, [i64; 4])> = {
        let mut stmt = tx
            .prepare(
                "SELECT session_id, model, COALESCE(input_tokens, 0), COALESCE(output_tokens, 0),
                        COALESCE(cache_read_tokens, 0), COALESCE(cache_creation_tokens, 0)
                 FROM session_usage_archive WHERE app_type = 'claude'",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    [r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?],
                ))
            })
            .map_err(db_err)?;
        rows.collect::<Result<_, _>>().map_err(db_err)?
    };
    for (session_id, model, tokens) in archived {
        let Some(pricing) = price(&tx, &model) else {
            continue;
        };
        let cost = CostCalculator::calculate(&usage_of(tokens), &pricing, Decimal::ONE);
        updated += tx
            .execute(
                "UPDATE session_usage_archive SET cost_usd = ?3
                 WHERE app_type = 'claude' AND session_id = ?1 AND model = ?2",
                params![session_id, model, cost.total_cost.normalize().to_string()],
            )
            .map_err(db_err)?;
    }

    tx.commit()
        .map_err(|e| AppError::Database(format!("recompute commit: {e}")))?;
    Ok(updated)
}

/// Legacy archive aggregates have no event IDs and can include proxy usage.
/// No parser revision can prove which individual missing rows they represent.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackfillOutcome {
    pub written: usize,
    pub complete: bool,
    pub unproven: Vec<&'static str>,
}

pub(crate) fn backfill_session_archive(db: &Database) -> Result<BackfillOutcome, AppError> {
    let outcome = BackfillOutcome {
        written: 0,
        complete: false,
        unproven: vec!["legacyArchiveHasNoEventIdentity"],
    };
    let mut conn = lock_conn!(db.conn);
    let tx = conn.transaction()?;
    // State and any future proven writes belong to this same transaction.
    // A permanent proof gap is not pending work: retrying it every refresh
    // would repeatedly read hundreds of files without establishing identity.
    tx.execute(
        "INSERT OR REPLACE INTO maintenance_flags(key,value) VALUES(?1,?2)",
        params![
            BACKFILL_SESSION_ARCHIVE,
            if outcome.complete { DONE } else { "partial" }
        ],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO maintenance_flags(key,value) VALUES(?1,?2)",
        params![
            "backfill_session_archive_coverage",
            serde_json::to_string(&outcome)
                .map_err(|e| AppError::Database(format!("serialize backfill coverage: {e}")))?
        ],
    )?;
    tx.commit()?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::dao::usage_rollup::usage_prune_cutoff;
    use crate::services::{session_usage, session_usage_codex, session_usage_gemini};
    use crate::test_sandbox::sandbox;
    use chrono::{Duration, Utc};
    use serial_test::serial;
    use std::collections::BTreeMap;
    use std::path::Path;

    const SESSION: &str = "0f3a1c2d-1111-4222-8333-444455556666";
    const THREAD: &str = "019e4d66-eb35-7cd2-a5fb-11011b0aebfb";
    const CHAT: &str = "gemini-chat-1";

    fn ago(days: i64) -> String {
        (Utc::now() - Duration::days(days)).to_rfc3339()
    }

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    /// Transcripts of each source with calls before and after the prune cutoff.
    fn write_transcripts(home: &Path) {
        let claude = |id: &str, out: u32, days: i64| {
            serde_json::json!({
                "type": "assistant", "sessionId": SESSION, "timestamp": ago(days),
                "message": {"id": id, "model": "test-opus", "stop_reason": "end_turn",
                    "usage": {"input_tokens": 3, "output_tokens": out,
                        "cache_read_input_tokens": 5000, "cache_creation_input_tokens": 1000}}
            })
            .to_string()
        };
        write(
            &home.join(format!(".claude/projects/-work/{SESSION}.jsonl")),
            &[
                claude("msg_a", 100, 60),
                claude("msg_b", 200, 45),
                claude("msg_c", 300, 1),
            ]
            .join("\n"),
        );

        let count = |days: i64, input: u64, cached: u64, out: u64| {
            serde_json::json!({"timestamp": ago(days), "type": "event_msg", "payload": {
                "type": "token_count", "info": {"total_token_usage": {"input_tokens": input,
                    "cached_input_tokens": cached, "output_tokens": out}}}})
            .to_string()
        };
        write(
            &home.join(format!(
                ".codex/sessions/2026/08/01/rollout-2026-08-01T10-00-00-{THREAD}.jsonl"
            )),
            &[
                serde_json::json!({"timestamp": ago(50), "type": "session_meta",
                    "payload": {"id": THREAD}})
                .to_string(),
                serde_json::json!({"timestamp": ago(50), "type": "turn_context",
                    "payload": {"model": "test-codex"}})
                .to_string(),
                count(50, 1000, 200, 50),
                count(2, 3000, 1200, 150),
            ]
            .join("\n"),
        );

        let gemini = |id: &str, input: u32, days: i64| {
            serde_json::json!({"id": id, "type": "gemini", "model": "test-gemini",
                "timestamp": ago(days),
                "tokens": {"input": input, "output": 20, "cached": 100, "thoughts": 5}})
        };
        write(
            &home.join(".gemini/tmp/project-hash/chats/session-1.json"),
            &serde_json::json!({"sessionId": CHAT, "messages": [
                gemini("g1", 900, 40), gemini("g2", 700, 3)
            ]})
            .to_string(),
        );
    }

    fn database_with_prices() -> Database {
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            for model in ["test-opus", "test-codex", "test-gemini"] {
                conn.execute(
                    "INSERT INTO model_pricing (model_id, display_name, input_cost_per_million,
                        output_cost_per_million, cache_read_cost_per_million,
                        cache_creation_cost_per_million)
                     VALUES (?1, ?1, '3', '15', '0.3', '3.75')",
                    [model],
                )
                .unwrap();
            }
        }
        db
    }

    fn sync_all_sources(db: &Database) {
        session_usage::sync_claude_session_logs(db).unwrap();
        session_usage_codex::sync_codex_usage(db).unwrap();
        session_usage_gemini::sync_gemini_usage(db).unwrap();
    }

    fn set_pending(db: &Database) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO maintenance_flags (key, value) VALUES
                ('recompute_claude_costs', 'pending'), ('backfill_session_archive', 'pending')",
            [],
        )
        .unwrap();
    }

    /// (app, session, model) -> (tokens, calls, cost in micro-dollars).
    type SessionTotals = BTreeMap<(String, String, String), ([i64; 4], i64, i64)>;

    /// Every session's usage: live rows plus the archive.
    fn session_totals(db: &Database) -> SessionTotals {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT app_type, session_id, model, SUM(i), SUM(o), SUM(cr), SUM(cc), SUM(n),
                        CAST(ROUND(SUM(cost) * 1000000) AS INTEGER)
                 FROM (
                    SELECT app_type, session_id, model, input_tokens i, output_tokens o,
                           cache_read_tokens cr, cache_creation_tokens cc, 1 n,
                           CAST(total_cost_usd AS REAL) cost
                    FROM proxy_request_logs WHERE session_id IS NOT NULL
                    UNION ALL
                    SELECT app_type, session_id, model, input_tokens, output_tokens,
                           cache_read_tokens, cache_creation_tokens, request_count,
                           CAST(cost_usd AS REAL)
                    FROM session_usage_archive
                 ) GROUP BY app_type, session_id, model",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    (r.get(0)?, r.get(1)?, r.get(2)?),
                    (
                        [r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?],
                        r.get(7)?,
                        r.get(8)?,
                    ),
                ))
            })
            .unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    /// What every session holds when nothing was ever pruned.
    fn reference_totals() -> SessionTotals {
        let reference = database_with_prices();
        sync_all_sources(&reference);
        session_totals(&reference)
    }

    fn archive_rows(db: &Database) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM session_usage_archive", [], |r| {
            r.get(0)
        })
        .unwrap()
    }

    /// The v10 prune: old rows deleted, nothing archived.
    fn prune_without_archive(db: &Database) {
        let cutoff = usage_prune_cutoff().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM proxy_request_logs WHERE created_at < ?1",
            [cutoff],
        )
        .unwrap();
    }

    #[test]
    #[serial]
    fn legacy_pruned_sessions_remain_partial_without_event_proof() {
        let sandbox = sandbox();
        write_transcripts(sandbox.home());
        let expected = reference_totals();
        assert_eq!(expected.len(), 3, "{expected:?}");
        let db = database_with_prices();
        sync_all_sources(&db);
        prune_without_archive(&db);
        let surviving = session_totals(&db);
        assert_ne!(surviving, expected);
        set_pending(&db);

        run_pending(&db).unwrap();

        assert_eq!(session_totals(&db), surviving);
        assert_eq!(archive_rows(&db), 0);
        assert_eq!(
            backfill_session_archive(&db).unwrap().unproven,
            ["legacyArchiveHasNoEventIdentity"]
        );
    }

    #[test]
    #[serial]
    fn the_backfill_never_counts_a_call_twice() {
        let sandbox = sandbox();
        write_transcripts(sandbox.home());
        let expected = reference_totals();
        let db = database_with_prices();
        sync_all_sources(&db);

        // old rows still live: the backfill leaves them to the prune
        set_pending(&db);
        run_pending(&db).unwrap();
        assert_eq!(session_totals(&db), expected);
        assert_eq!(archive_rows(&db), 0);

        db.rollup_and_prune(30).unwrap();
        assert_eq!(session_totals(&db), expected);

        // run again: the archive is set, not added to
        set_pending(&db);
        run_pending(&db).unwrap();
        assert_eq!(session_totals(&db), expected);
        set_pending(&db);
        run_pending(&db).unwrap();
        assert_eq!(session_totals(&db), expected);
    }

    #[test]
    #[serial]
    fn steps_run_once_and_only_when_pending() {
        let sandbox = sandbox();
        write_transcripts(sandbox.home());
        let db = database_with_prices();
        sync_all_sources(&db);
        prune_without_archive(&db);

        // no flags (never migrated): nothing runs
        run_pending(&db).unwrap();
        assert_eq!(archive_rows(&db), 0);

        set_pending(&db);
        run_pending(&db).unwrap();
        let conn = db.conn.lock().unwrap();
        let flags: Vec<(String, String)> = conn
            .prepare("SELECT key, value FROM maintenance_flags WHERE key IN ('backfill_session_archive','recompute_claude_costs') ORDER BY key")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            flags,
            [
                (
                    "backfill_session_archive".to_string(),
                    "partial".to_string()
                ),
                ("recompute_claude_costs".to_string(), "done".to_string())
            ]
        );
    }

    #[test]
    fn claude_session_costs_are_priced_again_with_fresh_input() {
        let db = database_with_prices();
        {
            let conn = db.conn.lock().unwrap();
            // priced by v10: (3 - 5000 -> 0) input, so no input cost
            conn.execute(
                "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_cost_usd, total_cost_usd, latency_ms, status_code, session_id,
                    created_at, data_source)
                 VALUES ('session:m1', '_session', 'claude', 'test-opus', 3, 150, 5000, 10000,
                    '0', '0.04125', 0, 200, 's1', 1, 'session_log'),
                        ('proxy-1', 'ofox', 'claude', 'test-opus', 3, 150, 5000, 10000,
                    '0', '0.04125', 0, 200, 's1', 1, 'proxy')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO usage_daily_rollups (date, app_type, provider_id, model,
                    request_count, input_tokens, output_tokens, cache_read_tokens,
                    cache_creation_tokens, total_cost_usd)
                 VALUES ('2026-01-01', 'claude', '_session', 'test-opus', 1, 3, 150, 5000,
                    10000, '0.04125')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO session_usage_archive VALUES
                    ('claude', 's0', 'test-opus', 3, 150, 5000, 10000, 1, '0.04125', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO maintenance_flags VALUES ('recompute_claude_costs', 'pending')",
                [],
            )
            .unwrap();
        }

        run_pending(&db).unwrap();

        // 3*3 + 150*15 + 5000*0.3 + 10000*3.75 per million
        let conn = db.conn.lock().unwrap();
        let (input_cost, total): (String, String) = conn
            .query_row(
                "SELECT input_cost_usd, total_cost_usd FROM proxy_request_logs
                 WHERE request_id = 'session:m1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (input_cost.as_str(), total.as_str()),
            ("0.000009", "0.041259")
        );
        let proxy_total: String = conn
            .query_row(
                "SELECT total_cost_usd FROM proxy_request_logs WHERE request_id = 'proxy-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(proxy_total, "0.04125", "proxy rows are not touched");
        let rollup: String = conn
            .query_row(
                "SELECT total_cost_usd FROM usage_daily_rollups WHERE app_type = 'claude'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rollup, "0.041259");
        let archived: String = conn
            .query_row(
                "SELECT cost_usd FROM session_usage_archive WHERE session_id = 's0'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(archived, "0.041259");
    }

    #[test]
    fn backfill_does_not_read_sources_or_overwrite_a_prune() {
        let db = database_with_prices();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO session_usage_archive VALUES
            ('claude','fixture','test-opus',100,10,0,0,1,'0.1234',1,2)",
                [],
            )
            .unwrap();
        let outcome = backfill_session_archive(&db).unwrap();
        assert!(!outcome.complete);
        assert_eq!(outcome.written, 0);
        let cost: String = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT cost_usd FROM session_usage_archive", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(cost, "0.1234");
    }
}
