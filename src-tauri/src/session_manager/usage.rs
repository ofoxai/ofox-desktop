//! What each session spent: its live `proxy_request_logs` rows plus the totals
//! the 30-day prune kept in `session_usage_archive`, read in one query and
//! joined to the listing in memory by (app type, session id).
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

use crate::database::Database;
use crate::error::AppError;
use crate::services::ofox_pricing::{CostEstimate, PriceCatalog, PricingCoverage};
use crate::session_manager::SessionMeta;

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

/// Provenance of Codex list values. This display projection never revises the
/// saved ledger, and its price is always the current resolved OFox catalog.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDisplay {
    pub state: String,
    pub basis: &'static str,
    pub scope: &'static str,
    pub reasons: Vec<String>,
    pub estimate: Option<CostEstimate>,
    pub pricing_coverage: Option<PricingCoverage>,
    pub model_estimates: BTreeMap<String, CostEstimate>,
    pub parent_session_id: Option<String>,
    pub billing_session_id: Option<String>,
}
impl UsageDisplay {
    pub(crate) fn unavailable(reason: &str) -> Self {
        Self {
            state: "unavailable".into(),
            basis: "currentOfoxEstimate",
            scope: "nativeThread",
            reasons: vec![reason.into()],
            estimate: None,
            pricing_coverage: None,
            model_estimates: BTreeMap::new(),
            parent_session_id: None,
            billing_session_id: None,
        }
    }
}
type DisplayBySession = HashMap<String, (Option<SessionUsage>, UsageDisplay)>;

/// One listing read transaction, one legacy usage aggregation, one bounded
/// cache batch, and one current catalog. No per-row SQL or transcript parsing.
pub(crate) fn load_for_listing(
    db: &Database,
    listing: &[SessionMeta],
) -> Result<(UsageBySession, DisplayBySession), AppError> {
    let apex = crate::ofox_apex::current_apex();
    let mut conn = db.conn.lock().unwrap_or_else(|e| e.into_inner());
    let tx = conn.transaction()?;
    let ledger = load_on(&tx)?;
    let display = display_on(&tx, listing, apex, chrono::Utc::now().timestamp_millis())
        .unwrap_or_else(|_| {
            listing
                .iter()
                .filter(|m| m.provider_id == "codex")
                .map(|m| {
                    (
                        m.session_id.clone(),
                        (
                            None,
                            UsageDisplay::unavailable("threadUsageCacheUnavailable"),
                        ),
                    )
                })
                .collect()
        });
    tx.commit()?;
    Ok((ledger, display))
}
fn display_on(
    conn: &rusqlite::Connection,
    listing: &[SessionMeta],
    apex: &str,
    now_ms: i64,
) -> Result<DisplayBySession, AppError> {
    let cached = crate::services::session_analytics_index::thread_usage_on(conn, listing)?;
    let catalog = PriceCatalog::load_on(conn, apex, now_ms)?;
    let mut result = HashMap::new();
    for meta in listing.iter().filter(|m| m.provider_id == "codex") {
        let Some(projection) = cached.get(&meta.session_id) else {
            result.insert(
                meta.session_id.clone(),
                (None, UsageDisplay::unavailable("threadUsageNotIndexed")),
            );
            continue;
        };
        let mut display = UsageDisplay {
            state: projection.state.clone(),
            basis: "currentOfoxEstimate",
            scope: "nativeThread",
            reasons: projection.reasons.clone(),
            estimate: None,
            pricing_coverage: Some(catalog.coverage.clone()),
            model_estimates: BTreeMap::new(),
            parent_session_id: projection.parent_session_id.clone(),
            billing_session_id: projection.billing_session_id.clone(),
        };
        if projection.state != "available" {
            result.insert(meta.session_id.clone(), (None, display));
            continue;
        }
        let mut estimate: Option<CostEstimate> = None;
        let mut models = vec![];
        let mut unpriced = 0;
        for row in &projection.models {
            let priced = catalog.estimate(&row.model, row.buckets, Some(row.records));
            if matches!(priced.availability.as_str(), "unknown" | "partial") {
                unpriced += 1;
            }
            let cost = priced
                .known_cost_usd
                .as_deref()
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite());
            if let Some(total) = &mut estimate {
                total.combine(&priced)?;
            } else {
                estimate = Some(priced.clone());
            }
            display.model_estimates.insert(row.model.clone(), priced);
            models.push(ModelUsage {
                model: row.model.clone(),
                requests: row.records,
                input_tokens: row.buckets[0],
                output_tokens: row.buckets[1],
                cache_read_tokens: row.buckets[2],
                cache_write_tokens: row.buckets[3],
                cost_usd: cost,
            });
        }
        let mut usage = total(models);
        usage.unpriced_models = unpriced;
        if let Some(estimate) = &estimate {
            display.pricing_coverage = Some(catalog.coverage_for(estimate));
        }
        display.estimate = estimate;
        result.insert(meta.session_id.clone(), (Some(usage), display));
    }
    Ok(result)
}

/// The usage `app_type` of a session provider, for those whose usage is read.
pub fn app_type(provider: &str) -> Option<&'static str> {
    Some(match provider {
        "claude" => "claude",
        "codex" => "codex",
        "gemini" => "gemini",
        "opencode" => "opencode",
        "workbuddy" => "workbuddy",
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
        WHERE app_type IN ('claude', 'codex', 'gemini', 'opencode', 'workbuddy')
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

#[cfg(test)]
thread_local! {
    pub(crate) static LOAD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Every session's usage, by (app type, session id).
#[cfg(test)]
pub fn load(db: &Database) -> Result<UsageBySession, AppError> {
    let conn = db.conn.lock().unwrap_or_else(|e| e.into_inner());
    load_on(&conn)
}
fn load_on(conn: &rusqlite::Connection) -> Result<UsageBySession, AppError> {
    #[cfg(test)]
    LOAD_COUNT.with(|count| count.set(count.get() + 1));
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
        total_tokens: input + output,
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
    fn workbuddy_session_usage_provider_maps_to_its_own_native_app() {
        assert_eq!(app_type("workbuddy"), Some("workbuddy"));
    }

    #[test]
    fn workbuddy_session_usage_keeps_saved_zero_unknown_and_unions_live_with_archive() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES('workbuddy_session:[\"s\",\"m\"]','_workbuddy_session','workbuddy','native',100,20,30,40,0,0,'s',1,'workbuddy_session')",[]).unwrap();
        archived(&db, "workbuddy", "s", "native", [1, 2, 3, 4], 1, "0");
        let all = load(&db).unwrap();
        let session = all
            .get(&("workbuddy".into(), "s".into()))
            .expect("native WorkBuddy session usage must be visible");
        assert_eq!(
            (
                session.input_tokens,
                session.output_tokens,
                session.cache_read_tokens,
                session.cache_write_tokens
            ),
            (101, 22, 33, 44)
        );
        assert_eq!(session.total_tokens, 123);
        assert_eq!(session.models[0].requests, 2);
        assert_eq!(
            session.models[0].cost_usd, None,
            "saved 0 with tokens is not a verified native bill or known-free current estimate"
        );
        assert_eq!(session.unpriced_models, 1);
    }

    #[test]
    fn exponent_costs_and_cache_buckets_remain_separate() {
        let db = Database::memory().unwrap();
        live(&db, "exp", "codex", "s", "m", [3, 4, 100, 200], "1e-7");
        archived(&db, "codex", "s", "m", [5, 6, 300, 400], 1, "2e-7");
        let all = load(&db).unwrap();
        let s = &all[&("codex".into(), "s".into())];
        assert_eq!(s.total_tokens, 18);
        assert_eq!((s.cache_read_tokens, s.cache_write_tokens), (400, 600));
        assert!((s.cost_usd - 3e-7).abs() < 1e-15);
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
                total_tokens: 118 + 230,
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

#[cfg(test)]
#[path = "codex_display_tests.rs"]
mod codex_display_tests;
