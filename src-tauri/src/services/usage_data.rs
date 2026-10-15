//! One read transaction supplies every block of a versioned analytics response.
use super::{
    ofox_pricing::PriceCatalog,
    usage_query::{Coverage, SourceHealth, UsageQueryV1},
    usage_stats::RequestLogDetail,
};
use crate::{database::Database, error::AppError};
use chrono::{NaiveDate, TimeZone};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, str::FromStr};

const MAX_ROWS: usize = 100_000;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestFilters {
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
}

#[derive(Debug, Clone)]
pub struct Atom {
    pub tool: String,
    pub model: String,
    pub source: Option<String>,
    pub at_ms: i64,
    pub tokens: [u64; 4],
    pub requests: u64,
    pub saved_cost: Decimal,
    pub archived: bool,
}

pub struct Dataset {
    pub query: UsageQueryV1,
    pub records: Vec<RequestLogDetail>,
    pub atoms: Vec<Atom>,
    pub options: FilterOptions,
    pub coverage: Coverage,
    pub catalog: PriceCatalog,
    pub snapshot_version: String,
    pub sources: Vec<SourceHealth>,
}
fn ceil_seconds(ms: i64) -> Result<i64, AppError> {
    ms.div_euclid(1000)
        .checked_add(i64::from(ms.rem_euclid(1000) != 0))
        .ok_or_else(|| AppError::InvalidInput("usage.query.invalidTime".into()))
}
fn checked_u64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, value))
}
fn checked_u32(value: i64) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, value))
}
pub(crate) fn read_records(
    conn: &Connection,
    query: &UsageQueryV1,
) -> Result<Vec<RequestLogDetail>, AppError> {
    let mut predicates = vec!["l.created_at < ?".to_owned()];
    let mut values = vec![SqlValue::Integer(ceil_seconds(query.end_ms)?)];
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
        l.is_streaming,l.latency_ms,l.first_token_ms,l.duration_ms,l.status_code,l.error_message,l.created_at,l.data_source
        FROM proxy_request_logs l LEFT JOIN providers p ON p.id=l.provider_id AND p.app_type=l.app_type
        WHERE {} ORDER BY l.created_at DESC,l.request_id DESC LIMIT {}",predicates.join(" AND "),MAX_ROWS+1);
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), |row| {
        let status = row.get::<_, i64>(20)?;
        Ok(RequestLogDetail {
            request_id: row.get(0)?,
            provider_id: row.get(1)?,
            provider_name: row.get(2)?,
            app_type: row.get(3)?,
            model: row.get(4)?,
            request_model: row.get(5)?,
            cost_multiplier: row
                .get::<_, Option<String>>(6)?
                .unwrap_or_else(|| "1".into()),
            input_tokens: checked_u32(row.get(7)?)?,
            output_tokens: checked_u32(row.get(8)?)?,
            cache_read_tokens: checked_u32(row.get(9)?)?,
            cache_creation_tokens: checked_u32(row.get(10)?)?,
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
            created_at: row.get(22)?,
            data_source: row.get(23)?,
        })
    })?;
    let mut result = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        let row = row?;
        bytes = bytes.saturating_add(
            serde_json::to_vec(&row)
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

fn options_on(conn: &Connection) -> Result<FilterOptions, AppError> {
    let mut options = FilterOptions::default();
    for (sql,dest) in [
        ("SELECT app_type FROM proxy_request_logs UNION SELECT app_type FROM usage_daily_rollups ORDER BY 1",&mut options.tool_ids),
        ("SELECT model FROM proxy_request_logs UNION SELECT model FROM usage_daily_rollups ORDER BY 1",&mut options.model_ids),
        ("SELECT DISTINCT data_source FROM proxy_request_logs ORDER BY 1",&mut options.source_ids),
    ] {
        *dest=conn.prepare(sql)?.query_map([],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    }
    Ok(options)
}
fn midnight<Tz: TimeZone>(tz: &Tz, date: NaiveDate) -> Result<i64, AppError> {
    let mut value = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::InvalidInput("usage.query.invalidTime".into()))?;
    for _ in 0..48 {
        match tz.from_local_datetime(&value) {
            chrono::LocalResult::Single(v) => return Ok(v.timestamp_millis()),
            chrono::LocalResult::Ambiguous(a, b) => {
                return Ok(a.timestamp_millis().min(b.timestamp_millis()))
            }
            chrono::LocalResult::None => value += chrono::Duration::minutes(30),
        }
    }
    Err(AppError::InvalidInput("usage.query.invalidTime".into()))
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
    if !query.project_ids.is_empty() {
        return Err(AppError::InvalidInput(
            "usage.query.projectsUnsupported".into(),
        ));
    }
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
    let options = options_on(&tx)?;
    let mut records = read_records(&tx, &query)?;
    if let Some(filters) = request_filters {
        records.retain(|row| filters.matches(row));
    }
    let mut atoms = records
        .iter()
        .map(|row| {
            Ok(Atom {
                tool: row.app_type.clone(),
                model: row.model.clone(),
                source: row.data_source.clone(),
                at_ms: row
                    .created_at
                    .checked_mul(1000)
                    .ok_or_else(|| AppError::Database("usage.query.invalidTime".into()))?,
                tokens: [
                    row.input_tokens as u64,
                    row.output_tokens as u64,
                    row.cache_read_tokens as u64,
                    row.cache_creation_tokens as u64,
                ],
                requests: 1,
                saved_cost: Decimal::from_str(&row.total_cost_usd).unwrap_or(Decimal::ZERO),
                archived: false,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let mut coverage = Coverage {
        state: "complete".into(),
        reasons: vec![],
        time_basis: "eventTime".into(),
        detail_start_ms: atoms.iter().map(|a| a.at_ms).min(),
        detail_end_ms: atoms.iter().map(|a| a.at_ms).max(),
        aggregate_only: false,
        excluded_rollup_days: 0,
        source_ids: records
            .iter()
            .filter_map(|r| r.data_source.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    };
    let mut statement=tx.prepare("SELECT date,app_type,model,request_count,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,total_cost_usd FROM usage_daily_rollups ORDER BY date,app_type,model,provider_id")?;
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
        ))
    })?;
    for row in rows {
        let (date, tool, model, requests, tokens, cost) = row?;
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
            saved_cost: Decimal::from_str(&cost).unwrap_or(Decimal::ZERO),
            archived: true,
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
    let sources = source_health_on(&tx)?;
    if sources.iter().any(|s| {
        ["failed", "partial"].contains(&s.status.as_str())
            && (query.tool_ids.is_empty() || query.tool_ids.contains(&s.tool_id))
    }) {
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
    coverage.reasons.sort();
    coverage.reasons.dedup();
    if !coverage.reasons.is_empty() {
        coverage.state = if atoms.is_empty()
            && coverage
                .reasons
                .iter()
                .all(|r| r.starts_with("unsupportedTool:"))
        {
            "unsupported"
        } else {
            "partial"
        }
        .into();
    }
    let mut hash = Sha256::new();
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
            "{}\0{}\0{}\0{:?}\0{}\0{}\0{}",
            atom.tool,
            atom.model,
            atom.at_ms,
            atom.tokens,
            atom.requests,
            atom.saved_cost,
            atom.archived
        ));
    }
    hash.update(
        serde_json::to_vec(&catalog.coverage)
            .map_err(|_| AppError::Database("usage.query.serialize".into()))?,
    );
    tx.commit()?;
    Ok(Dataset {
        query,
        records,
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
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key='usage_source_health_v1'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(raw) = raw {
        return serde_json::from_str(&raw)
            .map_err(|_| AppError::Database("usage.sources.invalidHealth".into()));
    }
    Ok(super::usage_query::source_capabilities()
        .into_iter()
        .flat_map(|capability| {
            let ids = if capability.source_ids.is_empty() {
                vec![format!("{}:unsupported", capability.tool_id)]
            } else {
                capability.source_ids
            };
            ids.into_iter().map(move |source_id| SourceHealth {
                source_id,
                tool_id: capability.tool_id.clone(),
                status: if capability.usage_supported {
                    "unknown"
                } else {
                    "unsupported"
                }
                .into(),
                last_attempt_at_ms: None,
                last_success_at_ms: None,
                error_code: capability.reason.clone(),
            })
        })
        .collect())
}
