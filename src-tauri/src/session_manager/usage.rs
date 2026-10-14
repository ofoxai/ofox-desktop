//! What each session spent: its live `proxy_request_logs` rows plus the totals
//! the 30-day prune kept in `session_usage_archive`, read in one query and
//! joined to the listing in memory by (app type, session id).
use serde::Serialize;
use std::collections::HashMap;

use crate::database::Database;
use crate::error::AppError;

/// One model's share of a session.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// `None` when the model spent tokens at no known price.
    pub cost_usd: Option<f64>,
}

/// A session's tokens and cost, by model and in total.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsage {
    pub models: Vec<ModelUsage>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    /// The priced models' cost.
    pub cost_usd: f64,
    /// Models that spent tokens but have no known price.
    pub unpriced_models: u32,
}

/// The usage `app_type` of a session provider, for those whose usage is read.
pub fn app_type(provider: &str) -> Option<&'static str> {
    Some(match provider {
        "claude" => "claude",
        "codex" => "codex",
        "gemini" => "gemini",
        "opencode" => "opencode",
        _ => return None,
    })
}

pub type UsageBySession = HashMap<(String, String), SessionUsage>;

/// Live rows by (app, session, model) on `idx_request_logs_app_session`, plus
/// the archive, summed.
const SESSION_USAGE_SQL: &str = "
    SELECT app_type, session_id, model,
           SUM(requests), SUM(input), SUM(output), SUM(cache_read), SUM(cache_write), SUM(cost)
    FROM (
        SELECT app_type, session_id, model, COUNT(*) AS requests,
               SUM(input_tokens) AS input, SUM(output_tokens) AS output,
               SUM(cache_read_tokens) AS cache_read, SUM(cache_creation_tokens) AS cache_write,
               SUM(CAST(total_cost_usd AS REAL)) AS cost
        FROM proxy_request_logs
        WHERE app_type IN ('claude', 'codex', 'gemini', 'opencode')
          AND session_id IS NOT NULL AND session_id != ''
        GROUP BY app_type, session_id, model
        UNION ALL
        SELECT app_type, session_id, model, COALESCE(request_count, 0),
               COALESCE(input_tokens, 0), COALESCE(output_tokens, 0),
               COALESCE(cache_read_tokens, 0), COALESCE(cache_creation_tokens, 0),
               COALESCE(CAST(cost_usd AS REAL), 0)
        FROM session_usage_archive
        WHERE session_id IS NOT NULL AND session_id != ''
    )
    GROUP BY app_type, session_id, model";

/// Every session's usage, by (app type, session id).
pub fn load(db: &Database) -> Result<UsageBySession, AppError> {
    let conn = db.conn.lock().unwrap_or_else(|e| e.into_inner());
    let query = || -> rusqlite::Result<Vec<(String, String, ModelUsage)>> {
        let mut stmt = conn.prepare(SESSION_USAGE_SQL)?;
        let count = |row: &rusqlite::Row, i| -> rusqlite::Result<u64> {
            Ok(row.get::<_, Option<i64>>(i)?.unwrap_or(0).max(0) as u64)
        };
        let rows = stmt.query_map([], |row| {
            let model = ModelUsage {
                model: row.get(2)?,
                requests: count(row, 3)?,
                input_tokens: count(row, 4)?,
                output_tokens: count(row, 5)?,
                cache_read_tokens: count(row, 6)?,
                cache_write_tokens: count(row, 7)?,
                cost_usd: None,
            };
            let cost = row.get::<_, Option<f64>>(8)?.unwrap_or(0.0);
            Ok((row.get(0)?, row.get(1)?, priced(model, cost)))
        })?;
        rows.collect()
    };
    let rows =
        query().map_err(|e| AppError::Database(format!("Reading session usage failed: {e}")))?;
    drop(conn);

    let mut by_session: HashMap<(String, String), Vec<ModelUsage>> = HashMap::new();
    for (app, session, model) in rows {
        by_session.entry((app, session)).or_default().push(model);
    }
    Ok(by_session
        .into_iter()
        .map(|(key, models)| (key, total(models)))
        .collect())
}

/// A model's cost, or none when it spent tokens and nothing was charged:
/// rows are priced as they are written, so no price was known.
fn priced(mut model: ModelUsage, cost: f64) -> ModelUsage {
    let tokens = model.input_tokens
        + model.output_tokens
        + model.cache_read_tokens
        + model.cache_write_tokens;
    model.cost_usd = (cost > 0.0 || tokens == 0).then_some(cost.max(0.0));
    model
}

fn total(mut models: Vec<ModelUsage>) -> SessionUsage {
    models.sort_by(|a, b| {
        (b.input_tokens + b.output_tokens)
            .cmp(&(a.input_tokens + a.output_tokens))
            .then_with(|| a.model.cmp(&b.model))
    });
    let sum = |field: fn(&ModelUsage) -> u64| models.iter().map(field).sum::<u64>();
    let (input, output) = (sum(|m| m.input_tokens), sum(|m| m.output_tokens));
    let (cache_read, cache_write) = (sum(|m| m.cache_read_tokens), sum(|m| m.cache_write_tokens));
    SessionUsage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        total_tokens: input + output + cache_read + cache_write,
        cost_usd: models.iter().filter_map(|m| m.cost_usd).sum(),
        unpriced_models: models.iter().filter(|m| m.cost_usd.is_none()).count() as u32,
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(
        db: &Database,
        id: &str,
        app: &str,
        session: &str,
        model: &str,
        tokens: [i64; 4],
        cost: &str,
    ) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, latency_ms, status_code, session_id, created_at, data_source)
                 VALUES (?1, '_session', ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 200, ?9, 1, 'session_log')",
                rusqlite::params![
                    id, app, model, tokens[0], tokens[1], tokens[2], tokens[3], cost, session
                ],
            )
            .unwrap();
    }

    fn archived(
        db: &Database,
        app: &str,
        session: &str,
        model: &str,
        tokens: [i64; 4],
        requests: i64,
        cost: &str,
    ) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO session_usage_archive VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0)",
                rusqlite::params![app, session, model, tokens[0], tokens[1], tokens[2], tokens[3], requests, cost],
            )
            .unwrap();
    }

    #[test]
    fn a_sessions_usage_is_its_live_rows_plus_its_archive_by_model() {
        let db = Database::memory().unwrap();
        live(
            &db,
            "a",
            "claude",
            "s1",
            "claude-opus-5-5",
            [10, 20, 300, 40],
            "0.5",
        );
        live(
            &db,
            "b",
            "claude",
            "s1",
            "claude-opus-5-5",
            [1, 2, 3, 4],
            "0.25",
        );
        archived(
            &db,
            "claude",
            "s1",
            "claude-opus-5-5",
            [100, 200, 3000, 400],
            5,
            "1.25",
        );
        // tokens at no known price
        live(&db, "c", "claude", "s1", "mystery-model", [7, 8, 0, 0], "0");
        // the same id in another agent, and rows of no session
        live(&db, "d", "codex", "s1", "gpt-5.5", [1, 1, 0, 0], "0.01");
        live(
            &db,
            "e",
            "claude",
            "",
            "claude-opus-5-5",
            [1, 1, 0, 0],
            "0.01",
        );

        let usage = load(&db).unwrap();
        let s1 = &usage[&("claude".to_string(), "s1".to_string())];
        assert_eq!(
            s1,
            &SessionUsage {
                models: vec![
                    ModelUsage {
                        model: "claude-opus-5-5".into(),
                        requests: 7,
                        input_tokens: 111,
                        output_tokens: 222,
                        cache_read_tokens: 3303,
                        cache_write_tokens: 444,
                        cost_usd: Some(2.0),
                    },
                    ModelUsage {
                        model: "mystery-model".into(),
                        requests: 1,
                        input_tokens: 7,
                        output_tokens: 8,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                        cost_usd: None,
                    },
                ],
                input_tokens: 118,
                output_tokens: 230,
                cache_read_tokens: 3303,
                cache_write_tokens: 444,
                total_tokens: 118 + 230 + 3303 + 444,
                cost_usd: 2.0,
                unpriced_models: 1,
            }
        );
        assert_eq!(
            usage[&("codex".to_string(), "s1".to_string())].models.len(),
            1
        );
        assert!(!usage.contains_key(&("claude".to_string(), String::new())));
    }

    #[test]
    fn a_session_known_only_from_the_archive_still_has_its_usage() {
        let db = Database::memory().unwrap();
        archived(&db, "opencode", "ses_1", "kimi-k3", [5, 6, 0, 0], 2, "0.1");
        let usage = load(&db).unwrap();
        let session = &usage[&("opencode".to_string(), "ses_1".to_string())];
        assert_eq!(session.models[0].requests, 2);
        assert_eq!(session.total_tokens, 11);
        assert!((session.cost_usd - 0.1).abs() < 1e-9);
    }
}
