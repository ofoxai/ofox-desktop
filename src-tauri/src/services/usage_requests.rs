//! Frozen request metadata for paging, detail and complete matching CSV export.
//! Snapshots never contain request/response bodies or credentials.
use super::{
    ofox_pricing::{CostEstimate, PricingCoverage},
    usage_analytics::{self, Sum, UsageTrend},
    usage_data::{self, FilterOptions, RequestFilters},
    usage_overview::UsageTotals,
    usage_query::{Coverage, QueryReceipt, SourceHealth, UsageQueryV1},
    usage_stats::RequestLogDetail,
};
use crate::{database::Database, error::AppError};
use once_cell::sync::Lazy;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const SNAPSHOT_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_SNAPSHOTS: usize = 4;
const MAX_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestView {
    #[serde(flatten)]
    pub record: RequestLogDetail,
    pub created_at_ms: i64,
    pub recorded_cost_usd: Option<String>,
    pub observed_status_code: Option<u16>,
    pub observed_latency_ms: Option<u64>,
    pub observed_first_token_ms: Option<u64>,
    pub current_estimate: CostEstimate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<QueryReceipt>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPage {
    pub method_version: &'static str,
    pub input_snapshot_version: String,
    pub source_coverage: super::session_analytics::SourceCoverage,
    #[serde(flatten)]
    pub receipt: QueryReceipt,
    pub request_filters: RequestFilters,
    pub snapshot_id: String,
    pub data: Vec<RequestView>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
    pub coverage: Coverage,
    pub pricing_coverage: PricingCoverage,
    pub summary: UsageTotals,
    pub trend: UsageTrend,
    pub measurements: RequestMeasurements,
    pub options: FilterOptions,
    pub sources: Vec<SourceHealth>,
}
#[derive(Serialize)]
struct Snapshot {
    method_version: &'static str,
    input_snapshot_version: String,
    source_coverage: super::session_analytics::SourceCoverage,
    receipt: QueryReceipt,
    filters: RequestFilters,
    rows: Vec<RequestView>,
    coverage: Coverage,
    pricing: PricingCoverage,
    summary: UsageTotals,
    trend: UsageTrend,
    measurements: RequestMeasurements,
    options: FilterOptions,
    sources: Vec<SourceHealth>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestMeasurements {
    status_samples: u64,
    failed_samples: u64,
    failure_rate: Option<f64>,
    latency_samples: u64,
    avg_latency_ms: Option<f64>,
    first_token_samples: u64,
    avg_first_token_ms: Option<f64>,
    generation_speed_samples: u64,
    avg_generation_tokens_per_second: Option<f64>,
}

fn measure(rows: &[RequestView]) -> RequestMeasurements {
    let mut status_samples = 0;
    let mut failed_samples = 0;
    let mut latency_samples = 0;
    let mut latency_sum: u128 = 0;
    let mut first_token_samples = 0;
    let mut first_token_sum: u128 = 0;
    for row in rows {
        if let Some(status) = row.observed_status_code {
            status_samples += 1;
            failed_samples += u64::from(status >= 400);
        }
        if let Some(latency) = row.observed_latency_ms {
            latency_samples += 1;
            latency_sum += u128::from(latency);
        }
        if let Some(first) = row.observed_first_token_ms {
            first_token_samples += 1;
            first_token_sum += u128::from(first);
        }
    }
    RequestMeasurements {
        status_samples,
        failed_samples,
        failure_rate: (status_samples > 0).then(|| failed_samples as f64 / status_samples as f64),
        latency_samples,
        avg_latency_ms: (latency_samples > 0).then(|| latency_sum as f64 / latency_samples as f64),
        first_token_samples,
        avg_first_token_ms: (first_token_samples > 0)
            .then(|| first_token_sum as f64 / first_token_samples as f64),
        // Neither a generic duration column nor total latency proves the first
        // and last generated token times. Do not manufacture output speed.
        generation_speed_samples: 0,
        avg_generation_tokens_per_second: None,
    }
}
pub struct Pagination {
    pub page: u32,
    pub page_size: u32,
}
struct Entry {
    created: Instant,
    bytes: usize,
    value: Arc<Snapshot>,
}
static SNAPSHOTS: Lazy<Mutex<BTreeMap<String, Entry>>> = Lazy::new(|| Mutex::new(BTreeMap::new()));
fn invalid(message: &str) -> AppError {
    AppError::InvalidInput(format!("usage.requests.{message}"))
}

fn retain_snapshot(snapshot: Snapshot) -> Result<(String, Arc<Snapshot>), AppError> {
    let bytes = serde_json::to_vec(&snapshot)
        .map_err(|_| invalid("serialize"))?
        .len();
    if bytes > MAX_SNAPSHOT_BYTES {
        return Err(invalid("snapshotTooLarge"));
    }
    let now = Instant::now();
    let mut snapshots = SNAPSHOTS
        .lock()
        .map_err(|_| invalid("snapshotUnavailable"))?;
    snapshots.retain(|_, entry| now.duration_since(entry.created) < SNAPSHOT_TTL);
    while snapshots.len() >= MAX_SNAPSHOTS
        || snapshots
            .values()
            .map(|v| v.bytes)
            .sum::<usize>()
            .saturating_add(bytes)
            > MAX_SNAPSHOT_BYTES
    {
        let oldest = snapshots
            .iter()
            .min_by_key(|(_, v)| v.created)
            .map(|(k, _)| k.clone());
        if let Some(oldest) = oldest {
            snapshots.remove(&oldest);
        } else {
            break;
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let value = Arc::new(snapshot);
    snapshots.insert(
        id.clone(),
        Entry {
            created: now,
            bytes,
            value: Arc::clone(&value),
        },
    );
    Ok((id, value))
}
fn get_snapshot(id: &str) -> Result<Arc<Snapshot>, AppError> {
    let mut snapshots = SNAPSHOTS
        .lock()
        .map_err(|_| invalid("snapshotUnavailable"))?;
    if snapshots
        .get(id)
        .is_some_and(|entry| entry.created.elapsed() >= SNAPSHOT_TTL)
    {
        snapshots.remove(id);
    }
    snapshots
        .get(id)
        .map(|entry| Arc::clone(&entry.value))
        .ok_or_else(|| invalid("snapshotExpired"))
}
fn fingerprint(query: &UsageQueryV1, filters: &RequestFilters) -> Result<String, AppError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(query, filters)).map_err(|_| invalid("serialize"))?)
    ))
}
pub fn request_page(
    db: &Database,
    query: UsageQueryV1,
    filters: RequestFilters,
    pagination: Pagination,
    snapshot_id: Option<&str>,
    apex: &str,
    now_ms: i64,
) -> Result<RequestPage, AppError> {
    request_page_in(
        db,
        query,
        filters,
        pagination,
        snapshot_id,
        apex,
        (&chrono::Local, now_ms),
    )
}

pub(crate) fn request_page_in<Tz: chrono::TimeZone>(
    db: &Database,
    query: UsageQueryV1,
    filters: RequestFilters,
    pagination: Pagination,
    snapshot_id: Option<&str>,
    apex: &str,
    context: (&Tz, i64),
) -> Result<RequestPage, AppError> {
    let (tz, now_ms) = context;
    let Pagination { page, page_size } = pagination;
    if !(1..=100).contains(&page_size) {
        return Err(invalid("invalidPageSize"));
    }
    let offset = u64::from(page)
        .checked_mul(u64::from(page_size))
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| invalid("invalidPage"))?;
    let query = query.normalized()?;
    let filters = filters.normalized()?;
    let query_fingerprint = fingerprint(&query, &filters)?;
    let (id, snapshot) = if let Some(id) = snapshot_id {
        let snapshot = get_snapshot(id)?;
        if snapshot.pricing.apex != apex {
            return Err(invalid("snapshotPricingApexMismatch"));
        }
        if snapshot.receipt.query_fingerprint != query_fingerprint {
            return Err(invalid("snapshotQueryMismatch"));
        }
        (id.to_owned(), snapshot)
    } else {
        if page != 0 {
            return Err(invalid("snapshotRequired"));
        }
        let data = usage_data::read_dataset(db, query.clone(), Some(&filters), apex, now_ms, tz)?;
        let mut total = Sum::default();
        for atom in &data.atoms {
            total.add(atom, &data.catalog)?;
        }
        let summary = total.totals()?;
        let trend = usage_analytics::build_trend(&data.query, &data.atoms, &data.catalog, tz)?;
        let projection_version =
            usage_analytics::projection_version(&data.snapshot_version, &trend)?;
        let rows: Vec<_> = data
            .records
            .into_iter()
            .map(|record| {
                let evidence = data
                    .record_evidence
                    .get(&record.request_id)
                    .ok_or_else(|| invalid("recordEvidenceUnavailable"))?;
                let mut current_estimate = data.catalog.estimate(
                    &record.model,
                    [
                        record.input_tokens,
                        record.output_tokens,
                        record.cache_read_tokens,
                        record.cache_creation_tokens,
                    ],
                    Some(1),
                );
                if !evidence.complete && current_estimate.known_cost_usd.is_some() {
                    current_estimate.availability = "partial".into();
                    current_estimate.unpriced_record_count = None;
                }
                Ok(RequestView {
                    created_at_ms: evidence.at_ms,
                    recorded_cost_usd: evidence.recorded_cost_usd.clone(),
                    observed_status_code: usage_data::observed_status(&record),
                    observed_latency_ms: usage_data::observed_latency(&record),
                    // The legacy writer times the first SSE event, including
                    // lifecycle/role-only events. It does not prove a first
                    // generated token, even when the raw column is zero.
                    observed_first_token_ms: None,
                    record,
                    current_estimate,
                    receipt: None,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let measurements = measure(&rows);
        retain_snapshot(Snapshot {
            method_version: data.method_version,
            input_snapshot_version: data.input_snapshot_version,
            source_coverage: data.source_coverage,
            receipt: QueryReceipt {
                applied_query: query,
                query_fingerprint,
                snapshot_version: format!(
                    "{:x}",
                    Sha256::digest(format!(
                        "{}:{}",
                        projection_version,
                        fingerprint(&data.query, &filters)?
                    ))
                ),
            },
            filters,
            rows,
            coverage: data.coverage,
            pricing: data.catalog.coverage_for(
                summary
                    .estimate
                    .as_ref()
                    .ok_or_else(|| invalid("estimateUnavailable"))?,
            ),
            summary,
            trend,
            measurements,
            options: data.options,
            sources: data.sources,
        })?
    };
    let data = snapshot
        .rows
        .iter()
        .skip(offset)
        .take(page_size as usize)
        .cloned()
        .collect();
    Ok(RequestPage {
        method_version: snapshot.method_version,
        input_snapshot_version: snapshot.input_snapshot_version.clone(),
        source_coverage: snapshot.source_coverage.clone(),
        receipt: snapshot.receipt.clone(),
        request_filters: snapshot.filters.clone(),
        snapshot_id: id,
        data,
        total: snapshot.rows.len() as u64,
        page,
        page_size,
        coverage: snapshot.coverage.clone(),
        pricing_coverage: snapshot.pricing.clone(),
        summary: snapshot.summary.clone(),
        trend: snapshot.trend.clone(),
        measurements: snapshot.measurements.clone(),
        options: snapshot.options.clone(),
        sources: snapshot.sources.clone(),
    })
}
pub fn request_detail(
    request_id: &str,
    snapshot_id: &str,
) -> Result<Option<RequestView>, AppError> {
    let snapshot = get_snapshot(snapshot_id)?;
    Ok(snapshot
        .rows
        .iter()
        .find(|row| row.record.request_id == request_id)
        .cloned()
        .map(|mut row| {
            row.receipt = Some(snapshot.receipt.clone());
            row
        }))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvExport {
    pub file_name: String,
    pub csv: String,
    pub record_count: u64,
    pub coverage: Coverage,
    pub query_fingerprint: String,
    pub snapshot_version: String,
    pub saved_path: Option<String>,
}
fn csv_cell(value: &str) -> String {
    let clean = crate::redact::scrub(value);
    let trimmed = clean.trim_start_matches(char::is_whitespace);
    let formula =
        trimmed.starts_with(['=', '+', '-', '@']) || clean.starts_with(['\t', '\r', '\n']);
    format!(
        "\"{}{}\"",
        if formula { "'" } else { "" },
        clean.replace('"', "\"\"")
    )
}
pub fn export_csv(snapshot_id: &str, destination: Option<&str>) -> Result<CsvExport, AppError> {
    let snapshot = get_snapshot(snapshot_id)?;
    let mut csv="\u{feff}requestId,createdAtMs,tool,model,requestModel,newInputTokens,outputTokens,cacheReadTokens,cacheWriteTokens,savedCostUsd,currentKnownEstimateUsd,estimateAvailability,source,recordedStatusCode,recordedLatencyMs,estimatePriceSource\r\n".to_owned();
    // Iterate in bounded pages; the immutable rows and price context are shared
    // with paging/detail, and the full matching set is exported, never one UI page.
    for page in snapshot.rows.chunks(1000) {
        for view in page {
            let row = &view.record;
            let values = [
                row.request_id.clone(),
                view.created_at_ms.to_string(),
                row.app_type.clone(),
                row.model.clone(),
                row.request_model.clone().unwrap_or_default(),
                row.input_tokens.to_string(),
                row.output_tokens.to_string(),
                row.cache_read_tokens.to_string(),
                row.cache_creation_tokens.to_string(),
                view.recorded_cost_usd.clone().unwrap_or_default(),
                view.current_estimate
                    .known_cost_usd
                    .clone()
                    .unwrap_or_default(),
                view.current_estimate.availability.clone(),
                row.data_source.clone().unwrap_or_else(|| "proxy".into()),
                view.observed_status_code
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                view.observed_latency_ms
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                if view.current_estimate.known_cost_usd.is_none() {
                    String::new()
                } else if view.current_estimate.official_model_ids.is_empty() {
                    "ofox".into()
                } else {
                    "manufacturer".into()
                },
            ];
            csv.push_str(
                &values
                    .iter()
                    .map(|v| csv_cell(v))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            csv.push_str("\r\n");
        }
    }
    let saved_path = if let Some(destination) = destination {
        let path = Path::new(destination);
        if !path.is_absolute() {
            return Err(invalid("absoluteExportPathRequired"));
        }
        crate::config::atomic_write(path, csv.as_bytes())?;
        Some(destination.to_owned())
    } else {
        None
    };
    Ok(CsvExport {
        file_name: "ofox-request-records.csv".into(),
        csv: if saved_path.is_some() {
            String::new()
        } else {
            csv
        },
        record_count: snapshot.rows.len() as u64,
        coverage: snapshot.coverage.clone(),
        query_fingerprint: snapshot.receipt.query_fingerprint.clone(),
        snapshot_version: snapshot.receipt.snapshot_version.clone(),
        saved_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    fn query() -> UsageQueryV1 {
        UsageQueryV1 {
            version: 1,
            start_ms: Some(1000),
            end_ms: 3000,
            tool_ids: vec![],
            model_ids: vec![],
            project_ids: vec![],
            source_ids: vec![],
        }
    }

    #[test]
    #[serial_test::serial]
    fn manufacturer_fallback_prices_remain_frozen_in_pages_detail_and_csv() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,created_at,data_source) VALUES('official-opus','p','claude','claude-opus-5-5',1000000,0,0,200,1,'proxy'),('official-sonnet','p','claude','claude-sonnet-5-5',1000000,0,0,200,2,'proxy')",[]).unwrap();
        let first = request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 1,
            },
            None,
            "ofox.ai",
            3000,
        )
        .unwrap();
        assert_eq!(
            first
                .summary
                .estimate
                .as_ref()
                .unwrap()
                .known_cost_usd
                .as_deref(),
            Some("6")
        );
        assert_eq!(first.pricing_coverage.official_model_ids.len(), 2);
        super::super::ofox_pricing::PriceCatalog::ingest_on(
            &db.conn.lock().unwrap(),
            "ofox.ai",
            &serde_json::json!({"data":[
                {"id":"claude-opus-5-5","pricing":{"prompt":"0.000009"}},
                {"id":"claude-sonnet-5-5","pricing":{"prompt":"0.000009"}}
            ]}),
            4000,
        )
        .unwrap();
        let second = request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 1,
                page_size: 1,
            },
            Some(&first.snapshot_id),
            "ofox.ai",
            4000,
        )
        .unwrap();
        assert_eq!(
            second.data[0].current_estimate.known_cost_usd.as_deref(),
            Some("4")
        );
        let detail = request_detail("official-opus", &first.snapshot_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            detail.current_estimate.official_model_ids,
            vec!["claude-opus-5-5"]
        );
        let csv = export_csv(&first.snapshot_id, None).unwrap();
        assert!(csv.csv.contains("estimatePriceSource"));
        assert_eq!(csv.csv.matches("\"manufacturer\"").count(), 2);
        let latest = request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 1,
            },
            None,
            "ofox.ai",
            4000,
        )
        .unwrap();
        assert_eq!(
            latest
                .summary
                .estimate
                .as_ref()
                .unwrap()
                .known_cost_usd
                .as_deref(),
            Some("18")
        );
        assert!(latest.pricing_coverage.official_model_ids.is_empty());
        assert_ne!(
            first.receipt.snapshot_version,
            latest.receipt.snapshot_version
        );
    }
    #[test]
    #[serial_test::serial]
    fn analytics_request_snapshot_rejects_a_different_pricing_apex() {
        let db = Database::memory().unwrap();
        let first = request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 20,
            },
            None,
            "ofox.ai",
            3000,
        )
        .unwrap();
        assert!(request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 20
            },
            Some(&first.snapshot_id),
            "ofox.io",
            3000
        )
        .is_err());
    }

    #[test]
    #[serial_test::serial]
    fn analytics_request_snapshot_freezes_pages_details_and_all_matching_csv() {
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            for id in ["a", "b", "c"] {
                conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,created_at,data_source) VALUES(?1,'p','claude',?2,10,1,0,200,1,'session_log')",params![id,if id=="a" {" =SUM(1,2)\n\"x\""}else{"m"}]).unwrap();
            }
        }
        let first = request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 2,
            },
            None,
            "ofox.ai",
            3000,
        )
        .unwrap();
        assert_eq!(
            first
                .data
                .iter()
                .map(|r| r.record.request_id.as_str())
                .collect::<Vec<_>>(),
            ["c", "b"]
        );
        assert_eq!(first.summary.requests, 3);
        assert_eq!(first.data[0].observed_status_code, None);
        assert_eq!(first.data[0].observed_latency_ms, None);
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,latency_ms,status_code,created_at) VALUES('d','p','codex','new',0,200,2)",[]).unwrap();
        let second = request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 1,
                page_size: 2,
            },
            Some(&first.snapshot_id),
            "ofox.ai",
            3000,
        )
        .unwrap();
        assert_eq!(second.total, 3);
        assert_eq!(second.data[0].record.request_id, "a");
        assert_eq!(
            second.receipt.snapshot_version,
            first.receipt.snapshot_version
        );
        let detail = request_detail("a", &first.snapshot_id).unwrap().unwrap();
        assert_eq!(
            detail.receipt.unwrap().snapshot_version,
            first.receipt.snapshot_version
        );
        let export = export_csv(&first.snapshot_id, None).unwrap();
        assert_eq!(export.record_count, 3);
        assert!(export.csv.contains("\"' =SUM(1,2)\n\"\"x\"\"\""));
        assert!(!export.csv.contains("codex"));
        let mut changed = query();
        changed.tool_ids.push("codex".into());
        assert!(request_page(
            &db,
            changed,
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 2
            },
            Some(&first.snapshot_id),
            "ofox.ai",
            3000
        )
        .is_err());
        assert!(request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 0
            },
            None,
            "ofox.ai",
            3000
        )
        .is_err());
    }
    #[test]
    #[serial_test::serial]
    fn analytics_csv_native_save_uses_the_same_frozen_complete_snapshot() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at) VALUES('a','p','claude','m',1,0,200,1)",[]).unwrap();
        let page = request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 1,
            },
            None,
            "ofox.ai",
            3000,
        )
        .unwrap();
        let preview = export_csv(&page.snapshot_id, None).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("records.csv");
        let saved = export_csv(&page.snapshot_id, Some(destination.to_str().unwrap())).unwrap();
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), preview.csv);
        assert_eq!(saved.query_fingerprint, preview.query_fingerprint);
        assert_eq!(saved.snapshot_version, preview.snapshot_version);
        assert_eq!(saved.saved_path.as_deref(), destination.to_str());
        assert!(saved.csv.is_empty());
        assert!(export_csv(&page.snapshot_id, Some("relative.csv")).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn analytics_request_filters_match_summary_count_and_literal_search() {
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            for (id, model, source, status) in [
                ("a", "m%_", "proxy", 429),
                ("b", "m%_", "session_log", 200),
                ("c", "other", "proxy", 200),
            ] {
                conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at,data_source) VALUES(?1,'p','claude',?2,10,0,?4,1,?3)",params![id,model,source,status]).unwrap();
            }
        }
        let filters = RequestFilters {
            status_code: Some(429),
            search: Some("%_".into()),
            ..Default::default()
        };
        let result = request_page(
            &db,
            query(),
            filters,
            Pagination {
                page: 0,
                page_size: 20,
            },
            None,
            "ofox.ai",
            3000,
        )
        .unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.summary.requests, 1);
        assert_eq!(result.data[0].record.request_id, "a");
        assert_eq!(result.data[0].observed_status_code, Some(429));
    }

    #[test]
    #[serial_test::serial]
    fn request_trend_and_measurements_use_complete_filters_and_remain_frozen() {
        let db = Database::memory().unwrap();
        for (id, model, source, status, at) in [
            ("a", "match", "proxy", 200, 1),
            ("b", "match", "proxy", 503, 86_401),
            ("other-model", "excluded", "proxy", 500, 1),
            ("other-source", "match", "session_log", 200, 1),
        ] {
            db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,first_token_ms,is_streaming,status_code,created_at,data_source) VALUES(?1,'p','claude',?2,10,20,1000,100,1,?4,?5,?3)",params![id,model,source,status,at]).unwrap();
        }
        let q = UsageQueryV1 {
            start_ms: Some(0),
            end_ms: 3 * 86_400_000,
            ..query()
        };
        let filters = RequestFilters {
            source_ids: vec!["proxy".into()],
            status_kind: Some("recorded".into()),
            search: Some("match".into()),
            ..Default::default()
        };
        let page = request_page(
            &db,
            q.clone(),
            filters.clone(),
            Pagination {
                page: 0,
                page_size: 1,
            },
            None,
            "ofox.ai",
            q.end_ms,
        )
        .unwrap();
        let before = serde_json::to_value(&page).unwrap();
        assert_eq!(before["trend"]["granularity"], "day");
        assert_eq!(
            before["trend"]["series"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["requests"].as_u64().unwrap())
                .sum::<u64>(),
            2
        );
        assert_eq!(before["measurements"]["statusSamples"], 2);
        assert_eq!(before["measurements"]["failedSamples"], 1);
        assert_eq!(before["measurements"]["failureRate"], 0.5);
        assert_eq!(before["measurements"]["firstTokenSamples"], 0);
        assert!(before["measurements"]["avgFirstTokenMs"].is_null());
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at,data_source) VALUES('late','p','claude','match',900,400,500,2,'proxy')",[]).unwrap();
        let second = request_page(
            &db,
            q.clone(),
            filters.clone(),
            Pagination {
                page: 1,
                page_size: 1,
            },
            Some(&page.snapshot_id),
            "ofox.ai",
            q.end_ms,
        )
        .unwrap();
        let frozen = serde_json::to_value(&second).unwrap();
        assert_eq!(frozen["trend"], before["trend"]);
        assert_eq!(frozen["measurements"], before["measurements"]);
        assert_eq!(export_csv(&page.snapshot_id, None).unwrap().record_count, 2);
        let latest = request_page(
            &db,
            q.clone(),
            filters,
            Pagination {
                page: 0,
                page_size: 1,
            },
            None,
            "ofox.ai",
            q.end_ms,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(latest).unwrap()["measurements"]["statusSamples"],
            3
        );
    }

    #[test]
    #[serial_test::serial]
    fn request_measurements_distinguish_zero_from_imported_placeholders_and_unknown_speed() {
        let db = Database::memory().unwrap();
        for (id, source, latency, first, streaming) in [
            ("zero", "proxy", 0, Some(0), true),
            ("native", "opencode_session", 200, Some(50), true),
            ("placeholder", "session_log", 0, Some(0), true),
            ("invalid-first", "proxy", 100, Some(200), true),
            ("nonstream", "proxy", 100, Some(20), false),
        ] {
            db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,first_token_ms,duration_ms,is_streaming,status_code,created_at,data_source) VALUES(?1,'p','claude','m',1,20,?3,?4,1000,?5,200,1,?2)",params![id,source,latency,first,streaming]).unwrap();
        }
        let page = request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 20,
            },
            None,
            "ofox.ai",
            3000,
        )
        .unwrap();
        let value = serde_json::to_value(page).unwrap();
        assert_eq!(value["measurements"]["statusSamples"], 3);
        assert_eq!(value["measurements"]["failureRate"], 0.0);
        assert_eq!(value["measurements"]["latencySamples"], 4);
        assert_eq!(value["measurements"]["avgLatencyMs"], 100.0);
        assert_eq!(value["measurements"]["firstTokenSamples"], 0);
        assert!(value["measurements"]["avgFirstTokenMs"].is_null());
        assert_eq!(value["measurements"]["generationSpeedSamples"], 0);
        assert!(value["measurements"]["avgGenerationTokensPerSecond"].is_null());
    }

    #[test]
    #[serial_test::serial]
    fn request_measurements_without_observations_never_claim_a_zero_failure_rate() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,latency_ms,status_code,created_at,data_source) VALUES('native','p','claude','m',0,200,1,'session_log')",[]).unwrap();
        let page = request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 20,
            },
            None,
            "ofox.ai",
            3000,
        )
        .unwrap();
        let value = serde_json::to_value(page).unwrap();
        assert_eq!(value["measurements"]["statusSamples"], 0);
        assert!(value["measurements"]["failureRate"].is_null());
        assert_eq!(value["measurements"]["latencySamples"], 0);
        assert!(value["measurements"]["avgLatencyMs"].is_null());
        assert_eq!(value["measurements"]["firstTokenSamples"], 0);
        assert!(value["measurements"]["avgFirstTokenMs"].is_null());
    }

    #[test]
    #[serial_test::serial]
    fn local_source_placeholders_keep_tokens_without_claiming_http_observations() {
        let db = Database::memory().unwrap();
        // Earlier importers used 200 and later importers use 0 as compatible
        // ledger placeholders. Even a non-success raw value cannot prove HTTP
        // evidence for a source whose supported projection does not supply it.
        for (source, tool, status) in [
            ("session_log", "claude", 200),
            ("codex_session", "codex", 200),
            ("gemini_session", "gemini", 200),
            ("opencode_session", "opencode", 200),
            ("workbuddy_session", "workbuddy", 0),
            ("pi_session", "pi", 0),
            ("dsh_session", "dsh", 0),
            ("openclaw_session", "openclaw", 429),
        ] {
            db.conn.lock().unwrap().execute(
                "INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,latency_ms,first_token_ms,duration_ms,is_streaming,status_code,created_at,data_source) VALUES(?1,'local',?2,'unpriced',10,20,30,40,500,10,600,1,?3,1,?1)",
                params![source, tool, status],
            ).unwrap();
        }
        let page = request_page(
            &db,
            query(),
            RequestFilters {
                status_kind: Some("unrecorded".into()),
                ..Default::default()
            },
            Pagination {
                page: 0,
                page_size: 20,
            },
            None,
            "ofox.ai",
            3000,
        )
        .unwrap();
        assert_eq!(page.total, 8);
        assert_eq!(page.summary.requests, 8);
        assert_eq!(page.summary.total_tokens, 240);
        assert_eq!(page.summary.cache_read_tokens, 240);
        assert_eq!(page.summary.cache_write_tokens, 320);
        assert_eq!(page.measurements.status_samples, 0);
        assert_eq!(page.measurements.failed_samples, 0);
        assert_eq!(page.measurements.failure_rate, None);
        assert_eq!(page.measurements.latency_samples, 1);
        assert_eq!(page.measurements.avg_latency_ms, Some(500.0));
        assert_eq!(page.measurements.first_token_samples, 0);
        for row in &page.data {
            assert_eq!(row.observed_status_code, None);
            assert_eq!(row.observed_first_token_ms, None);
            assert_eq!(
                row.observed_latency_ms,
                (row.record.data_source.as_deref() == Some("opencode_session")).then_some(500)
            );
            let detail = request_detail(&row.record.request_id, &page.snapshot_id)
                .unwrap()
                .unwrap();
            assert_eq!(detail.observed_status_code, None);
            assert_eq!(detail.current_estimate.availability, "unknown");
            assert_eq!(detail.current_estimate.known_cost_usd, None);
        }
        assert_eq!(page.trend.series.iter().map(|p| p.requests).sum::<u64>(), 8);
        assert_eq!(export_csv(&page.snapshot_id, None).unwrap().record_count, 8);
    }

    #[test]
    #[serial_test::serial]
    fn http_status_filters_and_failure_rate_only_use_measured_samples() {
        let db = Database::memory().unwrap();
        for (id, source, status) in [
            ("measured-success", "proxy", 200),
            ("measured-rate-limit", "proxy", 429),
            ("unmeasured-proxy", "proxy", 0),
            ("imported-output", "workbuddy_session", 200),
        ] {
            db.conn.lock().unwrap().execute(
                "INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,created_at,data_source) VALUES(?1,'p','workbuddy','unpriced',10,20,0,?3,1,?2)",
                params![id, source, status],
            ).unwrap();
        }
        let read = |status_kind: Option<&str>, status_code| {
            request_page(
                &db,
                query(),
                RequestFilters {
                    status_kind: status_kind.map(str::to_owned),
                    status_code,
                    ..Default::default()
                },
                Pagination {
                    page: 0,
                    page_size: 20,
                },
                None,
                "ofox.ai",
                3000,
            )
            .unwrap()
        };
        let all = read(None, None);
        assert_eq!(all.summary.total_tokens, 120);
        assert_eq!(all.measurements.status_samples, 2);
        assert_eq!(all.measurements.failed_samples, 1);
        assert_eq!(all.measurements.failure_rate, Some(0.5));
        let no_status = read(Some("unrecorded"), None);
        assert_eq!(no_status.total, 2);
        assert_eq!(no_status.summary.total_tokens, 60);
        assert_eq!(no_status.measurements.failure_rate, None);
        let success = read(None, Some(200));
        assert_eq!(success.total, 1);
        assert_eq!(success.data[0].record.request_id, "measured-success");
        assert_eq!(success.measurements.failure_rate, Some(0.0));
        let rate_limit = read(None, Some(429));
        assert_eq!(rate_limit.total, 1);
        assert_eq!(rate_limit.data[0].observed_status_code, Some(429));
        assert_eq!(rate_limit.measurements.failure_rate, Some(1.0));
    }

    async fn assert_lifecycle_is_not_a_token(event: serde_json::Value) {
        let timing = Arc::new(Mutex::new(None));
        let captured = timing.clone();
        let collector = crate::proxy::response_processor::SseUsageCollector::new(
            Instant::now(),
            move |_, first| *captured.lock().unwrap() = first,
        );
        collector.push(event).await;
        collector.finish().await;
        let first = timing
            .lock()
            .unwrap()
            .expect("the existing writer records a first event even without generated content");
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,latency_ms,first_token_ms,is_streaming,status_code,created_at,data_source) VALUES('lifecycle','p','claude','m',?1,?1,1,200,1,'proxy')",[first]).unwrap();
        let page = request_page(
            &db,
            query(),
            RequestFilters::default(),
            Pagination {
                page: 0,
                page_size: 20,
            },
            None,
            "ofox.ai",
            3000,
        )
        .unwrap();
        let value = serde_json::to_value(page).unwrap();
        assert_eq!(
            value["measurements"]["firstTokenSamples"], 0,
            "first SSE lifecycle event is not a generated token"
        );
        assert!(value["measurements"]["avgFirstTokenMs"].is_null());
        assert!(value["data"][0]["observedFirstTokenMs"].is_null());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn request_anthropic_message_start_is_not_a_first_token_measurement() {
        assert_lifecycle_is_not_a_token(
            serde_json::json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
        )
        .await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn request_openai_role_delta_is_not_a_first_token_measurement() {
        assert_lifecycle_is_not_a_token(
            serde_json::json!({"choices":[{"delta":{"role":"assistant"}}]}),
        )
        .await;
    }

    #[test]
    #[serial_test::serial]
    fn request_timezone_changes_new_projection_version_but_keeps_open_snapshot_frozen() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at) VALUES('tz','p','claude','m',10,0,200,1)",[]).unwrap();
        let q = UsageQueryV1 {
            start_ms: Some(0),
            end_ms: 3 * 86_400_000,
            ..query()
        };
        let page = |tz: &chrono::FixedOffset, id: Option<&str>| {
            request_page_in(
                &db,
                q.clone(),
                RequestFilters::default(),
                Pagination {
                    page: 0,
                    page_size: 1,
                },
                id,
                "ofox.ai",
                (tz, q.end_ms),
            )
            .unwrap()
        };
        let utc = chrono::FixedOffset::east_opt(0).unwrap();
        let other = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        let initial = page(&utc, None);
        let same = page(&utc, None);
        assert_eq!(
            initial.receipt.snapshot_version,
            same.receipt.snapshot_version
        );
        let frozen = page(&other, Some(&initial.snapshot_id));
        assert_eq!(initial.trend, frozen.trend);
        assert_eq!(
            initial.receipt.snapshot_version,
            frozen.receipt.snapshot_version
        );
        let changed = page(&other, None);
        assert_ne!(initial.trend, changed.trend);
        assert_eq!(
            initial.receipt.query_fingerprint,
            changed.receipt.query_fingerprint
        );
        assert_ne!(
            initial.receipt.snapshot_version, changed.receipt.snapshot_version,
            "different local calendar projections need distinct snapshot versions"
        );
    }
}
