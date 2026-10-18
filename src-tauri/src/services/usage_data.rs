//! One read transaction supplies every block of a versioned analytics response.
use super::{
    ofox_pricing::{parse_amount, PriceCatalog},
    session_analytics::{AnalyticsInput, SourceCoverage},
    session_analytics_core::{Project, UsageFact},
    session_analytics_index,
    usage_projects::{ProjectIndex, ProjectOption},
    usage_query::{Coverage, SourceHealth, UsageQueryV1},
    usage_stats::RequestLogDetail,
};
use crate::{database::Database, error::AppError};
use chrono::{NaiveDate, TimeZone};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ROWS: usize = 100_000;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestFilters {
    pub record_basis: Option<String>,
    #[serde(default)]
    pub source_ids: Vec<String>,
    pub status_code: Option<u16>,
    pub status_kind: Option<String>,
    pub search: Option<String>,
}
impl RequestFilters {
    pub fn normalized(mut self) -> Result<Self, AppError> {
        if self.source_ids.len() > 256
            || self
                .record_basis
                .as_deref()
                .is_some_and(|basis| !["effective", "savedHistory"].contains(&basis))
            || self
                .source_ids
                .iter()
                .any(|s| s.is_empty() || s.len() > 512)
            || self.search.as_ref().is_some_and(|s| s.len() > 512)
            || self
                .status_kind
                .as_deref()
                .is_some_and(|s| !["recorded", "unrecorded"].contains(&s))
            || (self.status_kind.as_deref() == Some("unrecorded") && self.status_code.is_some())
        {
            return Err(AppError::InvalidInput(
                "usage.requests.invalidFilters".into(),
            ));
        }
        self.source_ids.sort();
        self.source_ids.dedup();
        self.search = self
            .search
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        self.record_basis = Some(self.record_basis.unwrap_or_else(|| "effective".into()));
        Ok(self)
    }
    pub(crate) fn matches(&self, row: &RequestLogDetail) -> bool {
        let source = row.data_source.as_deref().unwrap_or("proxy");
        let status = observed_status(row);
        (self.source_ids.is_empty() || self.source_ids.iter().any(|s| s == source))
            && self.status_code.is_none_or(|s| status == Some(s))
            && self
                .status_kind
                .as_deref()
                .is_none_or(|kind| (kind == "recorded") == status.is_some())
            && self.search.as_ref().is_none_or(|needle| {
                let needle = needle.to_lowercase();
                [&row.request_id, &row.provider_id, &row.app_type, &row.model]
                    .into_iter()
                    .map(String::as_str)
                    .chain(row.provider_name.as_deref())
                    .chain(row.request_model.as_deref())
                    .any(|value| value.to_lowercase().contains(&needle))
            })
    }
}
pub fn observed_status(row: &RequestLogDetail) -> Option<u16> {
    (row.data_source.as_deref().unwrap_or("proxy") == "proxy"
        && (100..600).contains(&row.status_code))
    .then_some(row.status_code)
}
pub fn observed_latency(row: &RequestLogDetail) -> Option<u64> {
    match row.data_source.as_deref().unwrap_or("proxy") {
        "proxy" => Some(row.latency_ms),
        "opencode_session" if row.latency_ms > 0 => Some(row.latency_ms),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterOptions {
    pub tool_ids: Vec<String>,
    pub model_ids: Vec<String>,
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub projects: Vec<ProjectOption>,
}

#[derive(Debug, Clone)]
pub struct Atom {
    pub tool: String,
    pub model: String,
    pub source: Option<String>,
    pub at_ms: i64,
    pub tokens: [u64; 4],
    pub tokens_complete: bool,
    pub requests: u64,
    pub saved_cost: Decimal,
    pub archived: bool,
    pub session_key: Option<String>,
    pub project_id: Option<String>,
}

pub struct Dataset {
    pub method_version: &'static str,
    pub input_snapshot_version: String,
    pub source_coverage: SourceCoverage,
    pub query: UsageQueryV1,
    pub records: Vec<RequestLogDetail>,
    pub record_evidence: BTreeMap<String, RecordEvidence>,
    pub atoms: Vec<Atom>,
    pub options: FilterOptions,
    pub coverage: Coverage,
    pub catalog: PriceCatalog,
    pub snapshot_version: String,
    pub sources: Vec<SourceHealth>,
}
#[derive(Serialize)]
pub struct RecordEvidence {
    pub at_ms: i64,
    pub recorded_cost_usd: Option<String>,
    pub complete: bool,
}
fn ceil_seconds(ms: i64) -> Result<i64, AppError> {
    ms.div_euclid(1000)
        .checked_add(i64::from(ms.rem_euclid(1000) != 0))
        .ok_or_else(|| AppError::InvalidInput("usage.query.invalidTime".into()))
}
fn checked_u64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, value))
}
struct Record {
    row: RequestLogDetail,
    native_sid: Option<String>,
    at_ms: i64,
    canonical: bool,
    session_key: Option<String>,
    project_id: Option<String>,
    usage_complete: bool,
}
fn record_predicate(native_ready: bool, saved_history: bool) -> Option<&'static str> {
    if saved_history {
        Some("l.app_type='codex' AND l.data_source='codex_session'")
    } else if native_ready {
        Some("NOT (l.app_type='codex' AND COALESCE(l.data_source,'proxy')='codex_session')")
    } else {
        None
    }
}
fn read_records(
    conn: &Connection,
    query: &UsageQueryV1,
    native_ready: bool,
    saved_history: bool,
) -> Result<Vec<Record>, AppError> {
    let mut predicates = vec!["l.created_at < ?".to_owned()];
    let mut values = vec![SqlValue::Integer(ceil_seconds(query.end_ms)?)];
    if let Some(predicate) = record_predicate(native_ready, saved_history) {
        predicates.push(predicate.into());
    }
    if let Some(start) = query.start_ms {
        predicates.push("l.created_at >= ?".into());
        values.push(SqlValue::Integer(ceil_seconds(start)?));
    }
    for (field, ids) in [
        ("l.app_type", &query.tool_ids),
        ("l.model", &query.model_ids),
        ("l.data_source", &query.source_ids),
    ] {
        if !ids.is_empty() {
            predicates.push(format!("{field} IN ({})", vec!["?"; ids.len()].join(",")));
            values.extend(ids.iter().cloned().map(SqlValue::Text));
        }
    }
    let sql=format!("SELECT l.request_id,l.provider_id,p.name,l.app_type,l.model,l.request_model,l.cost_multiplier,
        l.input_tokens,l.output_tokens,l.cache_read_tokens,l.cache_creation_tokens,
        l.input_cost_usd,l.output_cost_usd,l.cache_read_cost_usd,l.cache_creation_cost_usd,l.total_cost_usd,
        l.is_streaming,l.latency_ms,l.first_token_ms,l.duration_ms,l.status_code,l.error_message,l.created_at,l.data_source,l.session_id
        FROM proxy_request_logs l LEFT JOIN providers p ON p.id=l.provider_id AND p.app_type=l.app_type
        WHERE {} ORDER BY l.created_at DESC,l.request_id DESC LIMIT {}",predicates.join(" AND "),MAX_ROWS+1);
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), |row| {
        let status = row.get::<_, i64>(20)?;
        let created_at = row.get::<_, i64>(22)?;
        Ok(Record {
            row: RequestLogDetail {
                request_id: row.get(0)?,
                provider_id: row.get(1)?,
                provider_name: row.get(2)?,
                app_type: row.get(3)?,
                model: row.get(4)?,
                request_model: row.get(5)?,
                cost_multiplier: row
                    .get::<_, Option<String>>(6)?
                    .unwrap_or_else(|| "1".into()),
                input_tokens: checked_u64(row.get(7)?)?,
                output_tokens: checked_u64(row.get(8)?)?,
                cache_read_tokens: checked_u64(row.get(9)?)?,
                cache_creation_tokens: checked_u64(row.get(10)?)?,
                input_cost_usd: row.get(11)?,
                output_cost_usd: row.get(12)?,
                cache_read_cost_usd: row.get(13)?,
                cache_creation_cost_usd: row.get(14)?,
                total_cost_usd: row.get(15)?,
                is_streaming: row.get::<_, i64>(16)? != 0,
                latency_ms: checked_u64(row.get(17)?)?,
                first_token_ms: row
                    .get::<_, Option<i64>>(18)?
                    .map(checked_u64)
                    .transpose()?,
                duration_ms: row
                    .get::<_, Option<i64>>(19)?
                    .map(checked_u64)
                    .transpose()?,
                status_code: u16::try_from(status)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(20, status))?,
                error_message: row
                    .get::<_, Option<String>>(21)?
                    .map(crate::redact::scrub_error),
                created_at,
                data_source: row.get(23)?,
            },
            native_sid: row.get(24)?,
            at_ms: created_at
                .checked_mul(1000)
                .ok_or(rusqlite::Error::IntegralValueOutOfRange(22, created_at))?,
            canonical: false,
            session_key: None,
            project_id: None,
            usage_complete: true,
        })
    })?;
    let mut result = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        let row = row?;
        bytes = bytes.saturating_add(
            serde_json::to_vec(&row.row)
                .map_err(|_| AppError::Database("usage.requests.serialize".into()))?
                .len(),
        );
        if result.len() >= MAX_ROWS || bytes > MAX_BYTES {
            return Err(AppError::InvalidInput(
                "usage.requests.snapshotTooLarge".into(),
            ));
        }
        result.push(row);
    }
    Ok(result)
}

fn options_on(
    conn: &Connection,
    native_ready: bool,
    saved_history: bool,
) -> Result<FilterOptions, AppError> {
    let mut options = FilterOptions::default();
    let where_rows = record_predicate(native_ready, saved_history)
        .map(|p| format!(" WHERE {p}"))
        .unwrap_or_default();
    for (column, dest) in [
        ("app_type", &mut options.tool_ids),
        ("model", &mut options.model_ids),
        ("data_source", &mut options.source_ids),
    ] {
        let rollup = if saved_history || column == "data_source" {
            String::new()
        } else {
            format!(
                " UNION SELECT {column} FROM usage_daily_rollups{}",
                if native_ready {
                    " WHERE NOT (app_type='codex' AND provider_id='_codex_session')"
                } else {
                    ""
                }
            )
        };
        let sql = format!(
            "SELECT DISTINCT l.{column} FROM proxy_request_logs l{where_rows}{rollup} ORDER BY 1"
        );
        *dest = conn
            .prepare(&sql)?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(options)
}
pub(crate) fn midnight<Tz: TimeZone>(tz: &Tz, date: NaiveDate) -> Result<i64, AppError> {
    super::usage_time::local_day_start_ms(tz, date)
}

fn selected_fact(fact: &UsageFact, query: &UsageQueryV1) -> bool {
    fact.tool_id == "codex"
        && fact.source_id == "codex_session"
        && query.start_ms.is_none_or(|start| fact.at_ms >= start)
        && fact.at_ms < query.end_ms
        && (query.tool_ids.is_empty() || query.tool_ids.contains(&fact.tool_id))
        && (query.model_ids.is_empty() || query.model_ids.contains(&fact.model))
        && (query.source_ids.is_empty() || query.source_ids.contains(&fact.source_id))
}

fn native_records(input: &AnalyticsInput, query: &UsageQueryV1) -> Result<Vec<Record>, AppError> {
    let descriptors: BTreeMap<_, _> = input
        .sessions
        .iter()
        .filter(|s| s.identity.tool_id == "codex")
        .map(|s| (s.identity.key.as_str(), s))
        .collect();
    input
        .facts
        .iter()
        .filter(|fact| selected_fact(fact, query))
        .take(MAX_ROWS + 1)
        .map(|fact| {
            if chrono::DateTime::from_timestamp_millis(fact.at_ms).is_none() {
                return Err(AppError::InvalidInput("usage.query.invalidTime".into()));
            }
            let [input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens] =
                fact.buckets;
            let descriptor = fact
                .session_key
                .as_deref()
                .and_then(|key| descriptors.get(key));
            let project_id = descriptor.and_then(|descriptor| match &descriptor.identity.project {
                Project::Known(id) => Some(id.clone()),
                Project::Unknown => Some(super::session_analytics_core::UNKNOWN_PROJECT_ID.into()),
                Project::Unparsed => None,
            });
            Ok(Record {
                row: RequestLogDetail {
                    request_id: format!("codex_native_v1:{}", fact.event_key),
                    provider_id: "_codex_session".into(),
                    provider_name: None,
                    app_type: fact.tool_id.clone(),
                    model: fact.model.clone(),
                    request_model: None,
                    cost_multiplier: "1".into(),
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    input_cost_usd: "0".into(),
                    output_cost_usd: "0".into(),
                    cache_read_cost_usd: "0".into(),
                    cache_creation_cost_usd: "0".into(),
                    total_cost_usd: "0".into(),
                    is_streaming: false,
                    latency_ms: 0,
                    first_token_ms: None,
                    duration_ms: None,
                    status_code: 0,
                    error_message: None,
                    created_at: fact.at_ms.div_euclid(1000),
                    data_source: Some(fact.source_id.clone()),
                },
                native_sid: descriptor.map(|d| d.native_session_id.clone()),
                at_ms: fact.at_ms,
                canonical: true,
                session_key: descriptor.map(|d| d.identity.key.clone()),
                project_id,
                usage_complete: fact.complete,
            })
        })
        .collect()
}

fn record_budget(records: &[Record]) -> Result<(), AppError> {
    let mut bytes = 0usize;
    let mut identities = BTreeSet::new();
    for record in records {
        if !identities.insert(record.row.request_id.as_str()) {
            return Err(AppError::InvalidInput(
                "usage.requests.duplicateRecordIdentity".into(),
            ));
        }
        bytes = bytes
            .saturating_add(
                serde_json::to_vec(&record.row)
                    .map_err(|_| AppError::Database("usage.requests.serialize".into()))?
                    .len(),
            )
            .saturating_add(record.row.request_id.len())
            .saturating_add(64);
        if records.len() > MAX_ROWS || bytes > MAX_BYTES {
            return Err(AppError::InvalidInput(
                "usage.requests.snapshotTooLarge".into(),
            ));
        }
    }
    Ok(())
}

fn legacy_codex_history_on(conn: &Connection) -> Result<bool, AppError> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM proxy_request_logs WHERE app_type='codex' AND data_source='codex_session') OR EXISTS(SELECT 1 FROM usage_daily_rollups WHERE app_type='codex' AND provider_id='_codex_session')", [], |r| r.get(0))?)
}

pub fn read_dataset<Tz: TimeZone>(
    db: &Database,
    query: UsageQueryV1,
    request_filters: Option<&RequestFilters>,
    apex: &str,
    now_ms: i64,
    tz: &Tz,
) -> Result<Dataset, AppError> {
    let query = query.normalized()?;
    if chrono::DateTime::from_timestamp_millis(query.end_ms).is_none()
        || query
            .start_ms
            .is_some_and(|v| chrono::DateTime::from_timestamp_millis(v).is_none())
    {
        return Err(AppError::InvalidInput("usage.query.invalidTime".into()));
    }
    let mut conn = crate::database::lock_conn!(db.conn);
    let tx = conn.transaction()?;
    let catalog = PriceCatalog::load_on(&tx, apex, now_ms)?;
    let input = session_analytics_index::projection_cache().read_on(&tx)?;
    let request_filters = request_filters
        .cloned()
        .map(RequestFilters::normalized)
        .transpose()?;
    let request_filters = request_filters.as_ref();
    let saved_history =
        request_filters.is_some_and(|f| f.record_basis.as_deref() == Some("savedHistory"));
    let native_ready = input.codex_native_ready;
    let mut source_coverage = input.source_coverage(&query);
    source_coverage.legacy_history_available = legacy_codex_history_on(&tx)?;
    let mut options = options_on(&tx, native_ready, saved_history)?;
    let projects = ProjectIndex::from_input(&input)?;
    options.projects = projects.options.clone();
    let mut records = read_records(&tx, &query, native_ready, saved_history)?;
    for record in &mut records {
        let association = projects.associate(
            &record.row.app_type,
            &record.row.provider_id,
            record.row.data_source.as_deref(),
            record.native_sid.as_deref(),
        );
        record.session_key = association.map(|a| a.session_key.clone());
        record.project_id = association.and_then(|a| a.project_id.clone());
    }
    if native_ready && !saved_history {
        records.extend(native_records(&input, &query)?);
        let mut tools: BTreeSet<_> = options.tool_ids.into_iter().collect();
        let mut models: BTreeSet<_> = options.model_ids.into_iter().collect();
        let mut source_ids: BTreeSet<_> = options.source_ids.into_iter().collect();
        for fact in input
            .facts
            .iter()
            .filter(|fact| fact.tool_id == "codex" && fact.source_id == "codex_session")
        {
            if !tools.contains(&fact.tool_id) {
                tools.insert(fact.tool_id.clone());
            }
            if !models.contains(&fact.model) {
                models.insert(fact.model.clone());
            }
            if !source_ids.contains(&fact.source_id) {
                source_ids.insert(fact.source_id.clone());
            }
        }
        options.tool_ids = tools.into_iter().collect();
        options.model_ids = models.into_iter().collect();
        options.source_ids = source_ids.into_iter().collect();
        record_budget(&records)?;
        records.sort_by(|a, b| {
            b.at_ms
                .cmp(&a.at_ms)
                .then_with(|| b.row.request_id.cmp(&a.row.request_id))
        });
    }
    if let Some(filters) = request_filters {
        records.retain(|row| filters.matches(&row.row));
    }
    let project_incomplete = records.iter().any(|record| record.project_id.is_none());
    if !query.project_ids.is_empty() {
        records.retain(|record| {
            record
                .project_id
                .as_ref()
                .is_some_and(|id| query.project_ids.contains(id))
        });
    }
    let mut atoms = records
        .iter()
        .map(|record| {
            let row = &record.row;
            Ok(Atom {
                tool: row.app_type.clone(),
                model: row.model.clone(),
                source: row.data_source.clone(),
                at_ms: record.at_ms,
                tokens: [
                    row.input_tokens,
                    row.output_tokens,
                    row.cache_read_tokens,
                    row.cache_creation_tokens,
                ],
                requests: 1,
                tokens_complete: record.usage_complete,
                saved_cost: parse_amount(&row.total_cost_usd)?,
                archived: false,
                session_key: record.session_key.clone(),
                project_id: record.project_id.clone(),
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let mut coverage = Coverage {
        state: "complete".into(),
        reasons: if project_incomplete {
            vec!["projectAttributionIncomplete".into()]
        } else {
            vec![]
        },
        time_basis: "eventTime".into(),
        detail_start_ms: atoms.iter().map(|a| a.at_ms).min(),
        detail_end_ms: atoms.iter().map(|a| a.at_ms).max(),
        aggregate_only: false,
        excluded_rollup_days: 0,
        source_ids: records
            .iter()
            .filter_map(|r| r.row.data_source.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    };
    if records.iter().any(|record| !record.usage_complete) {
        coverage.reasons.push("incompleteUsageFact".into());
    }
    if native_ready
        && (source_coverage.partial > 0
            || source_coverage.failed > 0
            || source_coverage.undated > 0)
    {
        coverage.reasons.push("sourceReadIncomplete".into());
    }
    let mut statement=tx.prepare("SELECT date,app_type,model,request_count,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,total_cost_usd,provider_id FROM usage_daily_rollups ORDER BY date,app_type,model,provider_id")?;
    let rows = statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            checked_u64(r.get(3)?)?,
            [
                checked_u64(r.get(4)?)?,
                checked_u64(r.get(5)?)?,
                checked_u64(r.get(6)?)?,
                checked_u64(r.get(7)?)?,
            ],
            r.get::<_, String>(8)?,
            r.get::<_, String>(9)?,
        ))
    })?;
    for row in rows {
        let (date, tool, model, requests, tokens, cost, provider) = row?;
        let old_codex = tool == "codex" && provider == "_codex_session";
        if (native_ready && !saved_history && old_codex) || (saved_history && !old_codex) {
            continue;
        }
        if (!query.tool_ids.is_empty() && !query.tool_ids.contains(&tool))
            || (!query.model_ids.is_empty() && !query.model_ids.contains(&model))
        {
            continue;
        }
        let date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map_err(|_| AppError::Database("usage.query.invalidRollupDate".into()))?;
        let start = midnight(tz, date)?;
        let end = midnight(
            tz,
            date.succ_opt()
                .ok_or_else(|| AppError::Database("usage.query.invalidRollupDate".into()))?,
        )?;
        if start >= query.end_ms || query.start_ms.is_some_and(|q| end <= q) {
            continue;
        }
        coverage.aggregate_only = true;
        coverage.time_basis = if records.is_empty() {
            "legacyLocalDate"
        } else {
            "mixed"
        }
        .into();
        if !query.project_ids.is_empty() {
            coverage.excluded_rollup_days += 1;
            coverage.reasons.push("legacyProjectUnavailable".into());
            continue;
        }
        if request_filters.is_some() {
            coverage.reasons.push("historicalDetailUnavailable".into());
            continue;
        }
        if !query.source_ids.is_empty() {
            coverage.excluded_rollup_days += 1;
            coverage.reasons.push("legacySourceUnavailable".into());
            continue;
        }
        if start < query.start_ms.unwrap_or(i64::MIN) || end > query.end_ms {
            coverage.excluded_rollup_days += 1;
            coverage.reasons.push("partialLegacyDayExcluded".into());
            continue;
        }
        coverage.reasons.push("legacyLocalDate".into());
        atoms.push(Atom {
            tool,
            model,
            source: None,
            at_ms: start,
            tokens,
            requests,
            tokens_complete: true,
            saved_cost: parse_amount(&cost)?,
            archived: true,
            session_key: None,
            project_id: None,
        });
    }
    drop(statement);
    let partial: Option<String> = tx
        .query_row(
            "SELECT value FROM maintenance_flags WHERE key='backfill_session_archive'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if partial.as_deref() == Some("partial") || partial.as_deref() == Some("pending") {
        coverage
            .reasons
            .push("legacyArchiveCoverageUnproven".into());
    }
    let mut sources = source_health_on(&tx)?;
    if native_ready && !saved_history {
        let times: Vec<_> = input
            .facts
            .iter()
            .filter(|fact| fact.tool_id == "codex" && fact.source_id == "codex_session")
            .map(|fact| fact.at_ms)
            .collect();
        let range = times
            .iter()
            .min()
            .zip(times.iter().max())
            .and_then(|(start, end)| {
                Some(super::usage_query::RetainedDetailRange {
                    start_ms: *start,
                    end_ms: end.checked_add(1)?,
                })
            });
        for source in &mut sources {
            if source.tool_id == "codex" && source.source_id == "codex_session" {
                source.retained_detail_range = range;
            }
        }
    }
    let source_selected = |s: &SourceHealth| {
        (query.tool_ids.is_empty() || query.tool_ids.contains(&s.tool_id))
            && (query.source_ids.is_empty() || query.source_ids.contains(&s.source_id))
            && request_filters
                .is_none_or(|f| f.source_ids.is_empty() || f.source_ids.contains(&s.source_id))
    };
    if sources
        .iter()
        .any(|s| s.status == "unknown" && source_selected(s))
    {
        coverage.reasons.push("sourceHealthUnknown".into());
    }
    if sources
        .iter()
        .any(|s| ["failed", "partial"].contains(&s.status.as_str()) && source_selected(s))
    {
        coverage.reasons.push("sourceReadIncomplete".into());
    }
    for tool in &query.tool_ids {
        if super::usage_query::source_capabilities()
            .iter()
            .find(|c| &c.tool_id == tool)
            .is_none_or(|c| !c.usage_supported)
        {
            coverage.reasons.push(format!("unsupportedTool:{tool}"));
        }
    }
    let capabilities = super::usage_query::source_capabilities();
    for source in query
        .source_ids
        .iter()
        .chain(request_filters.into_iter().flat_map(|f| &f.source_ids))
    {
        if source != "proxy"
            && !capabilities
                .iter()
                .any(|c| c.usage_supported && c.source_ids.contains(source))
        {
            coverage.reasons.push(format!("unsupportedSource:{source}"));
        }
    }
    coverage.reasons.sort();
    coverage.reasons.dedup();
    if !coverage.reasons.is_empty() {
        coverage.state = if atoms.is_empty()
            && coverage
                .reasons
                .iter()
                .all(|r| r.starts_with("unsupportedTool:") || r.starts_with("unsupportedSource:"))
        {
            "unsupported"
        } else {
            "partial"
        }
        .into();
    }
    let mut hash = Sha256::new();
    hash.update(projects.snapshot_version);
    hash.update(&input.snapshot_version);
    hash.update([u8::from(native_ready)]);
    let record_evidence: BTreeMap<_, _> = records
        .iter()
        .map(|record| {
            (
                record.row.request_id.clone(),
                RecordEvidence {
                    at_ms: record.at_ms,
                    recorded_cost_usd: (!record.canonical)
                        .then(|| record.row.total_cost_usd.clone()),
                    complete: record.usage_complete,
                },
            )
        })
        .collect();
    let records: Vec<_> = records.into_iter().map(|r| r.row).collect();
    let known_records = atoms.iter().try_fold(0_u64, |sum, atom| {
        sum.checked_add(atom.requests)
            .ok_or_else(|| AppError::Database("usage.query.aggregateOverflow".into()))
    })?;
    source_coverage = source_coverage.with_usage_samples(known_records);
    hash.update(
        serde_json::to_vec(&query)
            .map_err(|_| AppError::Database("usage.query.serialize".into()))?,
    );
    hash.update(
        serde_json::to_vec(&records)
            .map_err(|_| AppError::Database("usage.query.serialize".into()))?,
    );
    for atom in &atoms {
        hash.update(atom.source.as_deref().unwrap_or("legacy-rollup").as_bytes());
        hash.update(format!(
            "{}\0{}\0{}\0{:?}\0{}\0{}\0{}\0{:?}\0{:?}\0{}",
            atom.tool,
            atom.model,
            atom.at_ms,
            atom.tokens,
            atom.requests,
            atom.saved_cost,
            atom.archived,
            atom.session_key,
            atom.project_id,
            atom.tokens_complete
        ));
    }
    hash.update(
        serde_json::to_vec(&catalog.coverage)
            .map_err(|_| AppError::Database("usage.query.serialize".into()))?,
    );
    hash.update(
        serde_json::to_vec(&(
            &coverage,
            &options,
            &sources,
            &source_coverage,
            &record_evidence,
        ))
        .map_err(|_| AppError::Database("usage.query.serialize".into()))?,
    );
    tx.commit()?;
    Ok(Dataset {
        method_version: if native_ready {
            "magpie-native-v1"
        } else {
            "legacy-v1"
        },
        input_snapshot_version: input.snapshot_version.clone(),
        source_coverage,
        query,
        records,
        record_evidence,
        atoms,
        options,
        coverage,
        catalog,
        snapshot_version: format!("{:x}", hash.finalize()),
        sources,
    })
}

/// Explicit unknown until a source-specific successful run records health. Never
/// substitute a combined run's completion time for per-source success.
pub fn source_health_on(conn: &Connection) -> Result<Vec<SourceHealth>, AppError> {
    super::usage_health::source_health_on(conn)
}

#[cfg(test)]
#[path = "usage_data_native_tests.rs"]
mod native_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn query() -> UsageQueryV1 {
        UsageQueryV1 {
            version: 1,
            start_ms: Some(1000),
            end_ms: 3000,
            tool_ids: vec!["claude".into()],
            model_ids: vec![],
            project_ids: vec![],
            source_ids: vec![],
        }
    }
    #[test]
    fn analytics_saved_scientific_amount_is_not_replaced_by_zero() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at,total_cost_usd) VALUES('a','p','claude','m',1,0,200,1,'1e-7')",[]).unwrap();
        let data = read_dataset(&db, query(), None, "ofox.ai", 3000, &chrono::Utc).unwrap();
        assert_eq!(
            data.atoms[0].saved_cost,
            Decimal::from_scientific("1e-7").unwrap()
        );
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE proxy_request_logs SET total_cost_usd='invalid'", [])
            .unwrap();
        assert!(read_dataset(&db, query(), None, "ofox.ai", 3000, &chrono::Utc).is_err());
    }

    #[test]
    fn analytics_rollup_scientific_amount_is_not_replaced_by_zero() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO usage_daily_rollups(date,app_type,provider_id,model,request_count,input_tokens,total_cost_usd) VALUES('2026-10-01','claude','p','m',1,1,'1e-7')",[]).unwrap();
        let mut q = query();
        q.start_ms = Some(
            chrono::Utc
                .with_ymd_and_hms(2026, 10, 1, 0, 0, 0)
                .unwrap()
                .timestamp_millis(),
        );
        q.end_ms = q.start_ms.unwrap() + 86_400_000;
        let data = read_dataset(&db, q.clone(), None, "ofox.ai", q.end_ms, &chrono::Utc).unwrap();
        assert_eq!(data.atoms.len(), 1);
        assert_eq!(
            data.atoms[0].saved_cost,
            Decimal::from_scientific("1e-7").unwrap()
        );
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE usage_daily_rollups SET total_cost_usd='invalid'",
                [],
            )
            .unwrap();
        assert!(read_dataset(&db, q.clone(), None, "ofox.ai", q.end_ms, &chrono::Utc).is_err());
    }
    #[test]
    fn analytics_millisecond_boundaries_are_half_open_and_do_not_round_down() {
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            for (id, at) in [("a", 1), ("b", 2), ("c", 3)] {
                conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at) VALUES(?1,'p','claude','m',1,0,200,?2)",rusqlite::params![id,at]).unwrap();
            }
        }
        let mut q = query();
        q.start_ms = Some(1001);
        q.end_ms = 2001;
        let data = read_dataset(&db, q, None, "ofox.ai", 3000, &chrono::Utc).unwrap();
        assert_eq!(
            data.records
                .iter()
                .map(|r| r.request_id.as_str())
                .collect::<Vec<_>>(),
            ["b"]
        );
    }

    #[test]
    fn analytics_missing_source_health_cannot_certify_complete_coverage() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at) VALUES('a','p','claude','m',1,0,200,1)",[]).unwrap();
        let data = read_dataset(&db, query(), None, "ofox.ai", 3000, &chrono::Utc).unwrap();
        assert_eq!(data.coverage.state, "partial");
        assert!(data
            .coverage
            .reasons
            .iter()
            .any(|r| r == "sourceHealthUnknown"));
    }

    #[test]
    fn analytics_one_source_health_does_not_hide_other_unattempted_sources() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO settings(key,value) VALUES('usage_source_health_v1',?1)", [r#"[{"sourceId":"gemini_session","toolId":"gemini","status":"ok","lastAttemptAtMs":2000,"lastSuccessAtMs":2000,"errorCode":null}]"#]).unwrap();
        let data = read_dataset(&db, query(), None, "ofox.ai", 3000, &chrono::Utc).unwrap();
        assert_eq!(data.coverage.state, "partial");
        assert!(data
            .sources
            .iter()
            .any(|s| s.tool_id == "claude" && s.status == "unknown"));
    }

    #[test]
    fn analytics_unknown_source_filter_is_not_certified_as_complete_empty_usage() {
        let db = Database::memory().unwrap();
        let mut q = query();
        q.source_ids.push("not-a-reader".into());
        let data = read_dataset(&db, q, None, "ofox.ai", 3000, &chrono::Utc).unwrap();
        assert_eq!(data.coverage.state, "unsupported");
        assert!(data
            .coverage
            .reasons
            .iter()
            .any(|r| r == "unsupportedSource:not-a-reader"));
    }
}
