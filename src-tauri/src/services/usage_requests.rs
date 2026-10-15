//! Frozen request metadata for paging, detail and complete matching CSV export.
//! Snapshots never contain request/response bodies or credentials.
use super::{
    ofox_pricing::{CostEstimate, PricingCoverage},
    usage_analytics::Sum,
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
    pub observed_status_code: Option<u16>,
    pub observed_latency_ms: Option<u64>,
    pub current_estimate: CostEstimate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<QueryReceipt>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPage {
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
    pub options: FilterOptions,
    pub sources: Vec<SourceHealth>,
}
struct Snapshot {
    receipt: QueryReceipt,
    filters: RequestFilters,
    rows: Vec<RequestView>,
    coverage: Coverage,
    pricing: PricingCoverage,
    summary: UsageTotals,
    options: FilterOptions,
    sources: Vec<SourceHealth>,
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
    let bytes = serde_json::to_vec(&snapshot.rows)
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
    page: u32,
    page_size: u32,
    snapshot_id: Option<&str>,
    apex: &str,
    now_ms: i64,
) -> Result<RequestPage, AppError> {
    if !(1..=100).contains(&page_size) {
        return Err(invalid("invalidPageSize"));
    }
    let offset = u64::from(page)
        .checked_mul(u64::from(page_size))
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| invalid("invalidPage"))?;
    let query = query.normalized()?;
    if !query.project_ids.is_empty() {
        return Err(AppError::InvalidInput(
            "usage.query.projectsUnsupported".into(),
        ));
    }
    let filters = filters.normalized()?;
    let query_fingerprint = fingerprint(&query, &filters)?;
    let (id, snapshot) = if let Some(id) = snapshot_id {
        let snapshot = get_snapshot(id)?;
        if snapshot.receipt.query_fingerprint != query_fingerprint {
            return Err(invalid("snapshotQueryMismatch"));
        }
        (id.to_owned(), snapshot)
    } else {
        if page != 0 {
            return Err(invalid("snapshotRequired"));
        }
        let data = usage_data::read_dataset(
            db,
            query.clone(),
            Some(&filters),
            apex,
            now_ms,
            &chrono::Local,
        )?;
        let mut total = Sum::default();
        for atom in &data.atoms {
            total.add(atom, &data.catalog)?;
        }
        let summary = total.totals()?;
        let rows = data
            .records
            .into_iter()
            .map(|record| {
                let current_estimate = data.catalog.estimate(
                    &record.model,
                    [
                        record.input_tokens as u64,
                        record.output_tokens as u64,
                        record.cache_read_tokens as u64,
                        record.cache_creation_tokens as u64,
                    ],
                    Some(1),
                );
                RequestView {
                    observed_status_code: usage_data::observed_status(&record),
                    observed_latency_ms: usage_data::observed_latency(&record),
                    record,
                    current_estimate,
                    receipt: None,
                }
            })
            .collect();
        retain_snapshot(Snapshot {
            receipt: QueryReceipt {
                applied_query: query,
                query_fingerprint,
                snapshot_version: format!(
                    "{:x}",
                    Sha256::digest(format!(
                        "{}:{}",
                        data.snapshot_version,
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
    let mut csv="\u{feff}requestId,createdAtMs,tool,model,requestModel,newInputTokens,outputTokens,cacheReadTokens,cacheWriteTokens,savedCostUsd,currentKnownEstimateUsd,estimateAvailability,source,recordedStatusCode,recordedLatencyMs\r\n".to_owned();
    // Iterate in bounded pages; the immutable rows and price context are shared
    // with paging/detail, and the full matching set is exported, never one UI page.
    for page in snapshot.rows.chunks(1000) {
        for view in page {
            let row = &view.record;
            let values = [
                row.request_id.clone(),
                row.created_at
                    .checked_mul(1000)
                    .ok_or_else(|| invalid("invalidTime"))?
                    .to_string(),
                row.app_type.clone(),
                row.model.clone(),
                row.request_model.clone().unwrap_or_default(),
                row.input_tokens.to_string(),
                row.output_tokens.to_string(),
                row.cache_read_tokens.to_string(),
                row.cache_creation_tokens.to_string(),
                row.total_cost_usd.clone(),
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
            0,
            2,
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
            1,
            2,
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
            0,
            2,
            Some(&first.snapshot_id),
            "ofox.ai",
            3000
        )
        .is_err());
        assert!(request_page(
            &db,
            query(),
            RequestFilters::default(),
            0,
            0,
            None,
            "ofox.ai",
            3000
        )
        .is_err());
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
        let result = request_page(&db, query(), filters, 0, 20, None, "ofox.ai", 3000).unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.summary.requests, 1);
        assert_eq!(result.data[0].record.request_id, "a");
        assert_eq!(result.data[0].observed_status_code, Some(429));
    }
}
