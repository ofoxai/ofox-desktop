//! Cached session queries; IPC never reparses native transcripts.
use super::{
    ofox_pricing::PriceCatalog,
    session_analytics::{advanced, build_analysis_in, AnalyticsInput, SessionAnalytics},
    session_analytics_core::DayBoundary,
    session_analytics_index,
    usage_query::UsageQueryV1,
    usage_time::local_day_start_ms,
};
use crate::{database::Database, error::AppError};
use chrono::{DateTime, TimeZone};

fn invalid_time() -> AppError {
    AppError::InvalidInput("usage.query.invalidTime".into())
}

fn days<Tz: TimeZone>(
    query: &UsageQueryV1,
    input: &AnalyticsInput,
    tz: &Tz,
) -> Result<Vec<DayBoundary>, AppError> {
    let Some(start) = query.start_ms.or_else(|| {
        input
            .facts
            .iter()
            .map(|f| f.at_ms)
            .chain(input.messages.iter().map(|m| m.at_ms))
            .chain(input.native_activity.iter().map(|event| event.at_ms))
            .filter(|at| *at < query.end_ms)
            .min()
    }) else {
        return Ok(vec![]);
    };
    if start == query.end_ms {
        return Ok(vec![]);
    }
    let mut date = DateTime::from_timestamp_millis(start)
        .ok_or_else(invalid_time)?
        .with_timezone(tz)
        .date_naive();
    let last =
        DateTime::from_timestamp_millis(query.end_ms.checked_sub(1).ok_or_else(invalid_time)?)
            .ok_or_else(invalid_time)?
            .with_timezone(tz)
            .date_naive();
    let mut result = vec![];
    while date <= last {
        let next = date.succ_opt().ok_or_else(invalid_time)?;
        let start_ms = local_day_start_ms(tz, date)?;
        let end_ms = local_day_start_ms(tz, next)?;
        // A date skipped by a time-zone change has no instants to chart.
        if start_ms < end_ms {
            result.push(DayBoundary {
                date: date.to_string(),
                start_ms,
                end_ms,
            });
        }
        date = next;
    }
    Ok(result)
}

pub(crate) fn query<Tz: TimeZone>(
    db: &Database,
    query: UsageQueryV1,
    apex: &str,
    now_ms: i64,
    tz: &Tz,
) -> Result<SessionAnalytics, AppError> {
    query_with_cache(
        db,
        query,
        apex,
        now_ms,
        tz,
        session_analytics_index::projection_cache(),
    )
}

fn query_with_cache<Tz: TimeZone>(
    db: &Database,
    query: UsageQueryV1,
    apex: &str,
    now_ms: i64,
    tz: &Tz,
    cache: &session_analytics_index::ProjectionCache,
) -> Result<SessionAnalytics, AppError> {
    #[cfg(test)]
    let mut profile = std::time::Instant::now();
    let query = query.normalized()?;
    DateTime::from_timestamp_millis(query.end_ms).ok_or_else(invalid_time)?;
    if let Some(start) = query.start_ms {
        DateTime::from_timestamp_millis(start).ok_or_else(invalid_time)?;
    }
    // Cache and prices share one SQLite snapshot. No native path is reopened
    // and the connection lock is released before aggregation and serialization.
    let (mut input, catalog) = {
        let mut conn = crate::database::lock_conn!(db.conn);
        let tx = conn.transaction()?;
        let input = cache.read_on(&tx)?;
        let catalog = PriceCatalog::load_on(&tx, apex, now_ms)?;
        tx.commit()?;
        (input, catalog)
    };
    #[cfg(test)]
    advanced::profile_stage("cache+catalog", &mut profile);
    const READERS: [(&str, &str); 8] = [
        ("claude", "session_log"),
        ("codex", "codex_session"),
        ("gemini", "gemini_session"),
        ("opencode", "opencode_session"),
        ("workbuddy", "workbuddy_session"),
        ("pi", "pi_session"),
        ("dsh", "dsh_session"),
        ("openclaw", "openclaw_session"),
    ];
    for tool in &query.tool_ids {
        if !READERS.iter().any(|(id, _)| id == tool) {
            std::sync::Arc::make_mut(&mut input)
                .reasons
                .push(format!("unsupportedTool:{tool}"));
        }
    }
    for source in &query.source_ids {
        if !READERS.iter().any(|(_, id)| id == source) {
            // Unknown source IDs have no trustworthy tool mapping. This query
            // explicitly selected the ID, so the reason applies to its scope.
            std::sync::Arc::make_mut(&mut input)
                .reasons
                .push(format!("unsupportedSource:{source}"));
        }
    }
    let only_unsupported = (!query.tool_ids.is_empty()
        && query
            .tool_ids
            .iter()
            .all(|tool| !READERS.iter().any(|(id, _)| id == tool)))
        || (!query.source_ids.is_empty()
            && query
                .source_ids
                .iter()
                .all(|source| !READERS.iter().any(|(_, id)| id == source)));
    let days = days(&query, &input, tz)?;
    let mut result = build_analysis_in(query, &input, &catalog, &days, tz)?;
    #[cfg(test)]
    advanced::profile_stage("aggregate", &mut profile);
    if only_unsupported {
        result.coverage.state = "unsupported".into();
        result.totals.messages = None;
        result.totals.active_ms = None;
        result.totals.active_days = None;
        result.totals.message_reason = Some("sourceUnsupported".into());
        result.totals.activity_reason = Some("sourceUnsupported".into());
        result.known_metrics = None;
        result.known_messages = None;
        result.known_active_ms = None;
        result.known_active_days = None;
        result.metric_coverage.state = "unavailable".into();
        result.metric_coverage.reasons = vec!["sourceUnsupported".into()];
        result.message_coverage = Some(result.metric_coverage.clone());
        result.activity_coverage = Some(result.metric_coverage.clone());
        result.activity_by_hour.cells.clear();
        result.activity_by_hour.coverage = result.metric_coverage.clone();
        result.totals.estimate = super::ofox_pricing::CostEstimate {
            unpriced_record_count: None,
            ..Default::default()
        };
        result.pricing_coverage = catalog.coverage_for(&result.totals.estimate);
        for day in &mut result.series {
            day.messages = None;
            day.active_ms = None;
            day.known_messages = None;
            day.known_active_ms = None;
            day.estimate = result.totals.estimate.clone();
        }
    }
    advanced::retain_analysis(&mut result);
    #[cfg(test)]
    advanced::profile_stage("freeze", &mut profile);
    Ok(result)
}
#[cfg(test)]
#[path = "session_analytics_calendar_tests.rs"]
mod calendar_tests;

#[cfg(test)]
#[path = "session_analytics_projection_tests.rs"]
mod projection_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::session_analytics_index::{refresh_sources, SourceSpec};
    use serde_json::json;
    fn q() -> UsageQueryV1 {
        UsageQueryV1 {
            version: 1,
            start_ms: Some(1792022400000),
            end_ms: 1792108800000,
            tool_ids: vec![],
            model_ids: vec![],
            project_ids: vec![],
            source_ids: vec![],
        }
    }
    #[test]
    fn native_analysis_days_include_cached_runtime_when_other_projections_have_no_timestamp() {
        let mut input = AnalyticsInput {
            snapshot_version: "native-only".into(),
            sessions: vec![],
            facts: vec![],
            messages: vec![],
            reasons: vec![],
            source_reasons: vec![],
            sources: vec![],
            native_activity: vec![],
            source_observations: vec![],
            codex_native_ready: true,
            legacy_history_available: false,
            thread_usage_coverage: Default::default(),
        };
        input.native_activity.push(
            crate::services::session_native_activity::NativeActivityEvent {
                session_key: "s".into(),
                event_key: "runtime".into(),
                at_ms: 1792022400000,
                main: true,
                stream_key: "stream".into(),
            },
        );
        let mut query = q();
        query.start_ms = None;
        let days = days(&query, &input, &chrono::Utc).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].date, "2026-10-15");
    }
    #[test]
    fn session_analytics_api_reads_cache_without_reopening_corrupt_native_source_and_echoes_receipt(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        let db = Database::memory().unwrap();
        std::fs::write(&path,json!({"type":"assistant","sessionId":"s","cwd":"/project","timestamp":"2026-10-15T00:00:00.123Z","message":{"id":"m","role":"assistant","model":"m","content":[{"type":"text","text":"answer"}],"usage":{"input_tokens":3,"output_tokens":2}}}).to_string()).unwrap();
        refresh_sources(
            &db,
            &[SourceSpec {
                provider: "claude".into(),
                path: path.clone(),
                subagent: false,
                project_file: None,
                title_file: None,
            }],
        )
        .unwrap();
        std::fs::write(&path, "invalid private payload must not be parsed by query").unwrap();
        let result = query(&db, q(), "ofox.ai", 1792108800000, &chrono::Utc).unwrap();
        assert_eq!(result.receipt.applied_query, q());
        assert_eq!(result.totals.tokens.total_tokens, 5);
        assert_eq!(result.totals.session_count, 1);
        assert_eq!(result.series.len(), 1);
        assert_eq!(result.pricing_coverage.apex, "ofox.ai");
        assert!(!format!("{result:?}").contains("private payload"));
        assert!(result.analytics_snapshot_id.is_some());
        assert!(result.cohort_rows.is_empty());
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("cohortRows").is_none());
        assert_eq!(json["compactTop"]["tokens"][0]["tokens"]["totalTokens"], 5);
    }
    #[test]
    fn session_analytics_api_handles_empty_window_and_separate_apex_without_creating_days() {
        let db = Database::memory().unwrap();
        let mut query_value = q();
        query_value.start_ms = Some(query_value.end_ms);
        let result = query(&db, query_value, "ofox.io", 1792108800000, &chrono::Utc).unwrap();
        assert!(result.series.is_empty());
        assert_eq!(result.totals.session_count, 0);
        assert_eq!(result.pricing_coverage.apex, "ofox.io");
        assert_eq!(result.coverage.state, "partial");
    }
    #[test]
    fn session_analytics_api_all_range_uses_actual_cached_event_start_and_natural_days() {
        let db = Database::memory().unwrap();
        let mut query_value = q();
        query_value.start_ms = None;
        let result = query(
            &db,
            query_value,
            "ofox.ai",
            1792108800000,
            &chrono::FixedOffset::east_opt(8 * 3600).unwrap(),
        )
        .unwrap();
        assert!(result.series.is_empty());
        assert_eq!(result.totals.tokens.total_tokens, 0);
    }
    #[test]
    fn session_analytics_api_unsupported_selection_is_explicit_instead_of_a_successful_empty_chart()
    {
        let db = Database::memory().unwrap();
        for (tools, sources, reason) in [
            (vec!["hermes".to_owned()], vec![], "unsupportedTool:hermes"),
            (vec![], vec!["proxy".to_owned()], "unsupportedSource:proxy"),
        ] {
            let mut selected = q();
            selected.tool_ids = tools;
            selected.source_ids = sources;
            let result = query(&db, selected, "ofox.ai", 1792108800000, &chrono::Utc).unwrap();
            assert_eq!(result.coverage.state, "unsupported");
            assert!(result.coverage.reasons.iter().any(|r| r == reason));
            assert_eq!(result.totals.session_count, 0);
            assert!(result.known_metrics.is_none());
            assert!(result.activity_by_hour.cells.is_empty());
            assert!(result.analytics_snapshot_id.is_none());
        }
    }
}
