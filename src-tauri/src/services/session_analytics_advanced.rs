//! Additional projections of the same identified cohort, and body-free frozen
//! paging. No native source discovery or parsing belongs in these operations.
use super::{AnalyticsSession, SessionAnalytics};
use crate::{
    error::AppError,
    services::{session_analytics_core::Activity, usage_query::QueryReceipt},
};
use chrono::{DateTime, Datelike, Offset, TimeZone, Utc};
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::BTreeMap,
    str::FromStr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Opt-in test profiling stays outside production and never prints data.
#[cfg(test)]
pub(crate) fn profile_stage(label: &str, start: &mut Instant) {
    if std::env::var_os("OFOX_ANALYTICS_PROFILE").is_some() {
        eprintln!("analytics profile {label}: {:?}", start.elapsed());
    }
    *start = Instant::now();
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownMetrics {
    pub messages: u64,
    pub active_ms: u64,
    pub active_days: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricCoverage {
    /// complete / partial / unavailable; the denominator is identified sessions.
    pub state: String,
    pub eligible_sessions: u64,
    pub excluded_sessions: u64,
    /// An unreadable or unidentifiable source cannot supply a missing-session count.
    pub unquantified_source_gaps: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompactTop {
    pub tokens: Vec<AnalyticsSession>,
    pub cost: Vec<AnalyticsSession>,
    pub active: Vec<AnalyticsSession>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Distribution {
    pub metric: String,
    /// Inclusive lower edges; the last bucket is open-ended. Zero is explicit.
    pub edges: Vec<u64>,
    pub counts: Vec<u64>,
    pub samples: u64,
    pub excluded_sessions: u64,
    pub unquantified_source_gaps: bool,
}

pub(super) fn distribution(
    metric: &str,
    edges: &[u64],
    values: impl Iterator<Item = Option<u64>>,
    gaps: bool,
) -> Distribution {
    let mut out = Distribution {
        metric: metric.into(),
        edges: edges.to_vec(),
        counts: vec![0; edges.len()],
        samples: 0,
        excluded_sessions: 0,
        unquantified_source_gaps: gaps,
    };
    for value in values {
        match value {
            Some(value) => {
                let index = edges
                    .partition_point(|edge| *edge <= value)
                    .saturating_sub(1);
                out.counts[index] += 1;
                out.samples += 1;
            }
            None => out.excluded_sessions += 1,
        }
    }
    out
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourCell {
    /// Monday = 0, Sunday = 6.
    pub weekday: u32,
    pub hour: u32,
    pub active_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourActivity {
    /// The host's local calendar; no IANA identifier is inferred from an offset.
    pub timezone: String,
    pub utc_offset_seconds_at_end: i32,
    /// Empty when unavailable, otherwise all 168 cells, including known zeros.
    pub cells: Vec<HourCell>,
    pub coverage: MetricCoverage,
}

fn invalid(code: &str) -> AppError {
    AppError::InvalidInput(format!("usage.analytics.{code}"))
}

fn utc(at: i64) -> Result<DateTime<Utc>, AppError> {
    DateTime::from_timestamp_millis(at).ok_or_else(|| invalid("invalidHourTime"))
}

fn offset<Tz: TimeZone>(at: i64, tz: &Tz) -> Result<i32, AppError> {
    Ok(utc(at)?.with_timezone(tz).offset().fix().local_minus_utc())
}

pub(super) fn hour_activity<Tz: TimeZone>(
    activity: &Activity,
    coverage: &MetricCoverage,
    end_ms: i64,
    tz: &Tz,
) -> Result<HourActivity, AppError> {
    let mut result = HourActivity {
        timezone: "local".into(),
        utc_offset_seconds_at_end: offset(end_ms.saturating_sub(1), tz)?,
        cells: vec![],
        coverage: coverage.clone(),
    };
    if coverage.state == "unavailable" {
        return Ok(result);
    }
    let mut amounts = [[0u64; 24]; 7];
    for &(at, duration) in &activity.native_allocations {
        let local = utc(at)?.with_timezone(tz);
        let weekday = local.weekday().num_days_from_monday() as usize;
        use chrono::Timelike;
        let hour = local.hour() as usize;
        amounts[weekday][hour] = amounts[weekday][hour]
            .checked_add(duration)
            .ok_or_else(|| invalid("counterOverflow"))?;
    }
    // Old cached/message-only activity retains interval splitting. A native
    // gap exists only in native_allocations, so the two methods cannot count
    // the same gap twice unless a caller supplies an invalid mixed projection.
    for &(from, end) in &activity.intervals {
        let mut start = from;
        while start < end {
            let local = utc(start)?.with_timezone(tz);
            let current_offset = local.offset().fix().local_minus_utc();
            let wall_ms = i128::from(start) + i128::from(current_offset) * 1_000;
            let remaining = 3_600_000 - wall_ms.rem_euclid(3_600_000);
            let mut next = i64::try_from((i128::from(start) + remaining).min(i128::from(end)))
                .map_err(|_| invalid("invalidHourTime"))?;
            // Native timezone transitions have second precision. An eligible
            // gap is under five minutes: find an offset transition before the
            // nominal next hour rather than distributing time over a missing
            // hour or losing the repeated hour during a fall-back.
            if offset(next - 1, tz)? != current_offset {
                let mut lo = start;
                let mut hi = next - 1;
                while hi - lo > 1 {
                    let mid = lo + (hi - lo) / 2;
                    if offset(mid, tz)? == current_offset {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                next = hi;
            }
            let weekday = local.weekday().num_days_from_monday() as usize;
            use chrono::Timelike;
            let hour = local.hour() as usize;
            let duration = u64::try_from(next - start).map_err(|_| invalid("invalidHourTime"))?;
            amounts[weekday][hour] = amounts[weekday][hour]
                .checked_add(duration)
                .ok_or_else(|| invalid("counterOverflow"))?;
            start = next;
        }
    }
    for (weekday, hours) in amounts.into_iter().enumerate() {
        for (hour, active_ms) in hours.into_iter().enumerate() {
            result.cells.push(HourCell {
                weekday: weekday as u32,
                hour: hour as u32,
                active_ms,
            });
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SortBy {
    Tokens,
    Cost,
    Active,
}

fn cost(row: &AnalyticsSession) -> Option<Decimal> {
    row.estimate
        .known_cost_usd
        .as_deref()
        .and_then(|v| Decimal::from_str(v).ok())
}

fn compare(a: &AnalyticsSession, b: &AnalyticsSession, sort: SortBy) -> Ordering {
    let order = match sort {
        SortBy::Tokens => b
            .tokens
            .as_ref()
            .map(|t| t.total_tokens)
            .cmp(&a.tokens.as_ref().map(|t| t.total_tokens)),
        // A partial estimate remains explicitly partial in the row. Unknown
        // prices sort after known subtotals, never as a fabricated zero.
        SortBy::Cost => cost(b).cmp(&cost(a)),
        SortBy::Active => b.active_ms.cmp(&a.active_ms),
    };
    order.then_with(|| a.session_key.cmp(&b.session_key))
}

pub(super) fn compact_top(rows: &[AnalyticsSession]) -> CompactTop {
    let top = |sort| {
        let mut indexes: Vec<_> = rows
            .iter()
            .filter(|row| match sort {
                SortBy::Tokens => row
                    .tokens
                    .as_ref()
                    .is_some_and(|tokens| tokens.total_tokens > 0),
                SortBy::Cost => cost(row).is_some_and(|amount| amount > Decimal::ZERO),
                SortBy::Active => row.active_ms.is_some_and(|amount| amount > 0),
            })
            .collect();
        indexes.sort_by(|a, b| compare(a, b, sort));
        indexes.into_iter().take(6).cloned().collect()
    };
    CompactTop {
        tokens: top(SortBy::Tokens),
        cost: top(SortBy::Cost),
        active: top(SortBy::Active),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedReceipt {
    pub query_fingerprint: String,
    pub snapshot_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedView {
    pub sort_by: SortBy,
    pub search: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manager_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPage {
    #[serde(flatten)]
    pub receipt: QueryReceipt,
    pub analytics_snapshot_id: String,
    pub data: Vec<AnalyticsSession>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
    pub applied_view: AppliedView,
    pub view_fingerprint: String,
}

const SNAPSHOT_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_SNAPSHOTS: usize = 4;
const MAX_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;
const MAX_SNAPSHOT_ROWS: usize = 100_000;

#[derive(Serialize)]
struct Snapshot {
    receipt: QueryReceipt,
    apex: String,
    rows: Vec<AnalyticsSession>,
}
struct Entry {
    created: Instant,
    bytes: usize,
    snapshot: Arc<Snapshot>,
}
#[derive(Default)]
struct SnapshotStore {
    entries: BTreeMap<String, Entry>,
}
impl SnapshotStore {
    fn retain(&mut self, snapshot: Snapshot, now: Instant) -> Result<String, AppError> {
        self.retain_bounded(
            snapshot,
            now,
            MAX_SNAPSHOT_ROWS,
            MAX_SNAPSHOT_BYTES,
            MAX_SNAPSHOTS,
        )
    }
    fn retain_bounded(
        &mut self,
        snapshot: Snapshot,
        now: Instant,
        max_rows: usize,
        max_bytes: usize,
        max_snapshots: usize,
    ) -> Result<String, AppError> {
        if snapshot.rows.len() > max_rows {
            return Err(invalid("snapshotTooLarge"));
        }
        let bytes = serde_json::to_vec(&snapshot)
            .map_err(|_| invalid("snapshotSerialization"))?
            .len();
        if bytes > max_bytes || max_snapshots == 0 {
            return Err(invalid("snapshotTooLarge"));
        }
        self.entries
            .retain(|_, e| now.duration_since(e.created) < SNAPSHOT_TTL);
        while self.entries.len() >= max_snapshots
            || self
                .entries
                .values()
                .map(|e| e.bytes)
                .sum::<usize>()
                .saturating_add(bytes)
                > max_bytes
        {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.created)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.entries.remove(&id);
            } else {
                break;
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.entries.insert(
            id.clone(),
            Entry {
                created: now,
                bytes,
                snapshot: Arc::new(snapshot),
            },
        );
        Ok(id)
    }
    fn get(&mut self, id: &str, now: Instant) -> Result<Arc<Snapshot>, AppError> {
        if self
            .entries
            .get(id)
            .is_some_and(|e| now.duration_since(e.created) >= SNAPSHOT_TTL)
        {
            self.entries.remove(id);
        }
        self.entries
            .get(id)
            .map(|e| Arc::clone(&e.snapshot))
            .ok_or_else(|| invalid("snapshotExpired"))
    }
}

static SNAPSHOTS: Lazy<Mutex<SnapshotStore>> = Lazy::new(|| Mutex::new(SnapshotStore::default()));

pub(crate) fn retain_analysis(result: &mut SessionAnalytics) {
    let rows = std::mem::take(&mut result.cohort_rows);
    if result.coverage.state == "unsupported" {
        result.paging_unavailable_reason = Some("sourceUnsupported".into());
        return;
    }
    let snapshot = Snapshot {
        receipt: result.receipt.clone(),
        apex: result.pricing_coverage.apex.clone(),
        rows,
    };
    let retained = SNAPSHOTS
        .lock()
        .map_err(|_| invalid("snapshotUnavailable"))
        .and_then(|mut store| store.retain(snapshot, Instant::now()));
    match retained {
        Ok(id) => result.analytics_snapshot_id = Some(id),
        Err(_) => result.paging_unavailable_reason = Some("snapshotUnavailable".into()),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn page(
    id: &str,
    expected: ExpectedReceipt,
    sort_by: SortBy,
    page: u32,
    page_size: u32,
    search: Option<String>,
    manager_key: Option<String>,
    apex: &str,
) -> Result<SessionPage, AppError> {
    let snapshot = SNAPSHOTS
        .lock()
        .map_err(|_| invalid("snapshotUnavailable"))?
        .get(id, Instant::now())?;
    page_on(
        &snapshot,
        id,
        expected,
        sort_by,
        page,
        page_size,
        search,
        manager_key,
        apex,
    )
}

#[allow(clippy::too_many_arguments)]
fn page_on(
    snapshot: &Snapshot,
    id: &str,
    expected: ExpectedReceipt,
    sort_by: SortBy,
    page: u32,
    page_size: u32,
    search: Option<String>,
    manager_key: Option<String>,
    apex: &str,
) -> Result<SessionPage, AppError> {
    if !(1..=100).contains(&page_size) {
        return Err(invalid("invalidPageSize"));
    }
    if snapshot.apex != apex {
        return Err(invalid("snapshotPricingApexMismatch"));
    }
    if snapshot.receipt.query_fingerprint != expected.query_fingerprint
        || snapshot.receipt.snapshot_version != expected.snapshot_version
    {
        return Err(invalid("snapshotReceiptMismatch"));
    }
    let search = search
        .map(|v| v.trim().to_lowercase())
        .filter(|v| !v.is_empty());
    if search.as_ref().is_some_and(|v| v.len() > 512) {
        return Err(invalid("invalidSearch"));
    }
    if manager_key
        .as_ref()
        .is_some_and(|v| v.is_empty() || v.len() > 512)
    {
        return Err(invalid("invalidManagerKey"));
    }
    let view = AppliedView {
        sort_by,
        search,
        manager_key,
    };
    let view_fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&view).map_err(|_| invalid("invalidView"))?)
    );
    let mut rows: Vec<_> = snapshot
        .rows
        .iter()
        .filter(|row| {
            view.manager_key
                .as_ref()
                .is_none_or(|key| row.manager_key.as_ref() == Some(key))
                && view.search.as_ref().is_none_or(|q| {
                    [&row.title, &row.native_session_id, &row.tool_id]
                        .into_iter()
                        .chain(row.project_label.iter())
                        .chain(row.project_path.iter())
                        .chain(row.models.iter().map(|m| &m.model))
                        .any(|v| v.to_lowercase().contains(q))
                })
        })
        .collect();
    rows.sort_by(|a, b| compare(a, b, sort_by));
    let total = rows.len() as u64;
    let offset = usize::try_from(u64::from(page) * u64::from(page_size))
        .map_err(|_| invalid("invalidPage"))?;
    Ok(SessionPage {
        receipt: snapshot.receipt.clone(),
        analytics_snapshot_id: id.into(),
        data: rows
            .into_iter()
            .skip(offset)
            .take(page_size as usize)
            .cloned()
            .collect(),
        total,
        page,
        page_size,
        applied_view: view,
        view_fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        ofox_pricing::CostEstimate,
        session_analytics::{AnalyticsModel, TokenTotals},
        usage_query::UsageQueryV1,
    };
    use chrono::{FixedOffset, LocalResult, NaiveDate, NaiveDateTime};

    fn coverage() -> MetricCoverage {
        MetricCoverage {
            state: "complete".into(),
            eligible_sessions: 1,
            excluded_sessions: 0,
            unquantified_source_gaps: false,
            reasons: vec![],
        }
    }
    #[derive(Clone)]
    struct Transition {
        at: i64,
        before: i32,
        after: i32,
    }
    impl TimeZone for Transition {
        type Offset = FixedOffset;
        fn from_offset(_: &Self::Offset) -> Self {
            unreachable!("UTC conversion does not reconstruct this test zone")
        }
        fn offset_from_local_date(&self, _: &NaiveDate) -> LocalResult<Self::Offset> {
            LocalResult::None
        }
        fn offset_from_local_datetime(&self, _: &NaiveDateTime) -> LocalResult<Self::Offset> {
            LocalResult::None
        }
        fn offset_from_utc_date(&self, utc: &NaiveDate) -> Self::Offset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> Self::Offset {
            FixedOffset::east_opt(if utc.and_utc().timestamp_millis() < self.at {
                self.before
            } else {
                self.after
            })
            .unwrap()
        }
    }
    #[test]
    fn analytics_hours_split_midnight_offset_transition_and_repeated_hours_without_losing_time() {
        let base = DateTime::parse_from_rfc3339("2026-10-15T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        for (transition_ms, before, after, expected) in [
            (3_600_000, 0, 3_600, vec![(0, 60_000), (2, 60_000)]),
            (3_600_000, 3_600, 0, vec![(1, 120_000)]),
            (3_570_000, 0, 1_800, vec![(0, 30_000), (1, 90_000)]),
        ] {
            let activity = Activity {
                intervals: vec![(base + 3_540_000, base + 3_660_000)],
                ..Default::default()
            };
            let tz = Transition {
                at: base + transition_ms,
                before,
                after,
            };
            let result = hour_activity(&activity, &coverage(), base + 86_400_000, &tz).unwrap();
            assert_eq!(
                result.cells.iter().map(|c| c.active_ms).sum::<u64>(),
                120_000
            );
            for (hour, value) in expected {
                assert_eq!(
                    result
                        .cells
                        .iter()
                        .find(|c| c.weekday == 3 && c.hour == hour)
                        .unwrap()
                        .active_ms,
                    value
                );
            }
        }
        let activity = Activity {
            intervals: vec![(base - 60_000, base + 60_000)],
            ..Default::default()
        };
        let result = hour_activity(&activity, &coverage(), base + 60_000, &Utc).unwrap();
        assert_eq!(
            result
                .cells
                .iter()
                .find(|c| c.weekday == 2 && c.hour == 23)
                .unwrap()
                .active_ms,
            60_000
        );
        assert_eq!(
            result
                .cells
                .iter()
                .find(|c| c.weekday == 3 && c.hour == 0)
                .unwrap()
                .active_ms,
            60_000
        );
        let mut unavailable = coverage();
        unavailable.state = "unavailable".into();
        assert!(hour_activity(&activity, &unavailable, base, &Utc)
            .unwrap()
            .cells
            .is_empty());
    }

    #[test]
    fn analytics_native_hours_assign_whole_gap_to_later_event_across_midnight_and_dst() {
        let base = DateTime::parse_from_rfc3339("2026-10-15T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        let midnight = Activity {
            active_ms: 120_000,
            native_allocations: vec![(base + 60_000, 120_000)],
            ..Default::default()
        };
        let result = hour_activity(&midnight, &coverage(), base + 86_400_000, &Utc).unwrap();
        assert_eq!(
            result.cells.iter().map(|cell| cell.active_ms).sum::<u64>(),
            120_000
        );
        assert_eq!(
            result
                .cells
                .iter()
                .find(|cell| cell.weekday == 3 && cell.hour == 0)
                .unwrap()
                .active_ms,
            120_000
        );
        assert_eq!(
            result
                .cells
                .iter()
                .find(|cell| cell.weekday == 2 && cell.hour == 23)
                .unwrap()
                .active_ms,
            0
        );

        for (before, after, hour) in [(0, 3_600, 2), (3_600, 0, 1), (0, 1_800, 1)] {
            let tz = Transition {
                at: base + 3_600_000,
                before,
                after,
            };
            let activity = Activity {
                active_ms: 120_000,
                native_allocations: vec![(base + 3_660_000, 120_000)],
                ..Default::default()
            };
            let result = hour_activity(&activity, &coverage(), base + 86_400_000, &tz).unwrap();
            assert_eq!(
                result.cells.iter().map(|cell| cell.active_ms).sum::<u64>(),
                activity.active_ms
            );
            assert_eq!(
                result
                    .cells
                    .iter()
                    .find(|cell| cell.weekday == 3 && cell.hour == hour)
                    .unwrap()
                    .active_ms,
                120_000
            );
        }
    }

    #[test]
    fn analytics_hours_can_project_native_and_legacy_allocations_without_losing_known_zero() {
        let base = DateTime::parse_from_rfc3339("2026-10-15T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        let activity = Activity {
            active_ms: 180_000,
            intervals: vec![(base - 60_000, base)],
            native_allocations: vec![(base + 60_000, 120_000)],
            ..Default::default()
        };
        let result = hour_activity(&activity, &coverage(), base + 86_400_000, &Utc).unwrap();
        assert_eq!(
            result.cells.iter().map(|cell| cell.active_ms).sum::<u64>(),
            activity.active_ms
        );
        let zero =
            hour_activity(&Activity::default(), &coverage(), base + 86_400_000, &Utc).unwrap();
        assert_eq!(zero.cells.len(), 168);
        assert!(zero.cells.iter().all(|cell| cell.active_ms == 0));
    }

    fn row(key: &str, tokens: u64, cost: Option<&str>, active: Option<u64>) -> AnalyticsSession {
        AnalyticsSession {
            session_key: key.into(),
            native_session_id: key.into(),
            tool_id: "codex".into(),
            title: format!("Title {key}"),
            project_id: Some("project".into()),
            project_label: Some("Project".into()),
            project_path: Some("/project".into()),
            manager_key: Some(format!("manager-{key}")),
            source_available: true,
            tokens: Some(TokenTotals {
                total_tokens: tokens,
                ..Default::default()
            }),
            estimate: CostEstimate {
                known_cost_usd: cost.map(str::to_owned),
                availability: if cost.is_some() { "known" } else { "unknown" }.into(),
                ..Default::default()
            },
            models: vec![AnalyticsModel {
                model: "model%_".into(),
                tokens: TokenTotals::default(),
                estimate: CostEstimate::default(),
            }],
            messages: Some(0),
            active_ms: active,
            reasons: vec![],
        }
    }
    fn snapshot() -> Snapshot {
        Snapshot {
            receipt: QueryReceipt {
                applied_query: UsageQueryV1 {
                    version: 1,
                    start_ms: Some(0),
                    end_ms: 100,
                    tool_ids: vec![],
                    model_ids: vec![],
                    project_ids: vec![],
                    source_ids: vec![],
                },
                query_fingerprint: "query".into(),
                snapshot_version: "snapshot".into(),
            },
            apex: "ofox.ai".into(),
            rows: vec![
                row("a", 100, Some("0.00001"), Some(1)),
                row("b", 1, Some("10"), Some(99)),
                row("c", 10_000, None, None),
            ],
        }
    }
    fn expected() -> ExpectedReceipt {
        ExpectedReceipt {
            query_fingerprint: "query".into(),
            snapshot_version: "snapshot".into(),
        }
    }

    #[test]
    fn analytics_snapshot_pages_sort_full_cohort_preserve_receipt_and_search_only_metadata() {
        let mut source = snapshot();
        let mut store = SnapshotStore::default();
        let now = Instant::now();
        let id = store.retain(snapshot(), now).unwrap();
        source.rows[1].estimate.known_cost_usd = Some("0".into());
        let retained = store.get(&id, now).unwrap();
        let first = page_on(
            &retained,
            &id,
            expected(),
            SortBy::Cost,
            0,
            1,
            None,
            None,
            "ofox.ai",
        )
        .unwrap();
        assert_eq!(first.data[0].session_key, "b");
        let second = page_on(
            &retained,
            &id,
            expected(),
            SortBy::Cost,
            1,
            1,
            None,
            None,
            "ofox.ai",
        )
        .unwrap();
        assert_eq!(second.data[0].session_key, "a");
        assert_eq!(first.receipt, second.receipt);
        assert_eq!(first.view_fingerprint, second.view_fingerprint);
        let unknown = page_on(
            &retained,
            &id,
            expected(),
            SortBy::Cost,
            2,
            1,
            None,
            None,
            "ofox.ai",
        )
        .unwrap();
        assert_eq!(unknown.data[0].session_key, "c");
        assert!(unknown.data[0].estimate.known_cost_usd.is_none());
        let active = page_on(
            &retained,
            &id,
            expected(),
            SortBy::Active,
            0,
            10,
            None,
            None,
            "ofox.ai",
        )
        .unwrap();
        assert_eq!(active.data[0].session_key, "b");
        assert_eq!(active.receipt, first.receipt);
        assert_ne!(active.view_fingerprint, first.view_fingerprint);
        let searched = page_on(
            &retained,
            &id,
            expected(),
            SortBy::Tokens,
            0,
            10,
            Some("  title B  ".into()),
            None,
            "ofox.ai",
        )
        .unwrap();
        assert_eq!(searched.total, 1);
        assert_eq!(searched.data[0].session_key, "b");
        assert_eq!(searched.applied_view.search.as_deref(), Some("title b"));
        assert_eq!(
            page_on(
                &retained,
                &id,
                expected(),
                SortBy::Tokens,
                0,
                10,
                Some("%_".into()),
                None,
                "ofox.ai"
            )
            .unwrap()
            .total,
            3
        );
        assert_eq!(
            page_on(
                &retained,
                &id,
                expected(),
                SortBy::Tokens,
                0,
                10,
                Some("secret body".into()),
                None,
                "ofox.ai"
            )
            .unwrap()
            .total,
            0
        );
        assert!(page_on(
            &retained,
            &id,
            expected(),
            SortBy::Tokens,
            0,
            0,
            None,
            None,
            "ofox.ai"
        )
        .is_err());
        assert!(page_on(
            &retained,
            &id,
            expected(),
            SortBy::Tokens,
            0,
            101,
            None,
            None,
            "ofox.ai"
        )
        .is_err());
        assert!(page_on(
            &retained,
            &id,
            expected(),
            SortBy::Tokens,
            0,
            10,
            None,
            None,
            "ofox.io"
        )
        .is_err());
        let mut wrong = expected();
        wrong.snapshot_version = "new".into();
        assert!(page_on(
            &retained,
            &id,
            wrong,
            SortBy::Tokens,
            0,
            10,
            None,
            None,
            "ofox.ai"
        )
        .is_err());
    }
    #[test]
    fn analytics_snapshot_expiry_eviction_and_budgets_are_explicit_without_truncation() {
        let mut store = SnapshotStore::default();
        let now = Instant::now();
        let first = store
            .retain_bounded(snapshot(), now, 3, 100_000, 2)
            .unwrap();
        let second = store
            .retain_bounded(snapshot(), now + Duration::from_secs(1), 3, 100_000, 2)
            .unwrap();
        store
            .retain_bounded(snapshot(), now + Duration::from_secs(2), 3, 100_000, 2)
            .unwrap();
        assert!(store.get(&first, now + Duration::from_secs(2)).is_err());
        assert_eq!(
            store
                .get(&second, now + Duration::from_secs(2))
                .unwrap()
                .rows
                .len(),
            3
        );
        assert!(store
            .retain_bounded(snapshot(), now + Duration::from_secs(3), 2, 100_000, 2)
            .is_err());
        assert!(store
            .retain_bounded(snapshot(), now + Duration::from_secs(3), 3, 1, 2)
            .is_err());
        assert_eq!(store.entries.len(), 2);
        assert!(store
            .get(&second, now + SNAPSHOT_TTL + Duration::from_secs(1))
            .is_err());
    }
    #[test]
    fn analytics_manager_key_is_an_exact_list_filter_in_the_frozen_cohort() {
        let data = snapshot();
        let found = page_on(
            &data,
            "id",
            expected(),
            SortBy::Tokens,
            0,
            50,
            None,
            Some("manager-b".into()),
            "ofox.ai",
        )
        .unwrap();
        assert_eq!(found.total, 1);
        assert_eq!(found.data[0].session_key, "b");
        assert_eq!(found.applied_view.manager_key.as_deref(), Some("manager-b"));
        let no_match = page_on(
            &data,
            "id",
            expected(),
            SortBy::Tokens,
            0,
            50,
            None,
            Some("manager".into()),
            "ofox.ai",
        )
        .unwrap();
        assert_eq!(no_match.total, 0);
        assert_eq!(found.receipt, no_match.receipt);
        assert_ne!(found.view_fingerprint, no_match.view_fingerprint);
        let and_search = page_on(
            &data,
            "id",
            expected(),
            SortBy::Tokens,
            0,
            50,
            Some("Title a".into()),
            Some("manager-b".into()),
            "ofox.ai",
        )
        .unwrap();
        assert_eq!(and_search.total, 0);
        assert!(page_on(
            &data,
            "id",
            expected(),
            SortBy::Tokens,
            0,
            50,
            None,
            Some(String::new()),
            "ofox.ai"
        )
        .is_err());
    }
    #[test]
    fn analytics_compact_cost_rank_keeps_decimal_precision_and_known_zero_separate_from_unknown() {
        let rows = vec![
            row("unknown", 999, None, None),
            row("tiny", 1, Some("0.00000000000000000001"), Some(0)),
            row("zero", 2, Some("0"), Some(0)),
        ];
        let top = compact_top(&rows);
        assert_eq!(
            top.cost
                .iter()
                .map(|r| r.session_key.as_str())
                .collect::<Vec<_>>(),
            ["tiny"]
        );
        assert!(top.active.is_empty());
        let distribution = distribution(
            "tokens",
            &[0, 1, 5],
            [Some(0), Some(1), Some(5), None].into_iter(),
            true,
        );
        assert_eq!(distribution.counts, [1, 1, 1]);
        assert_eq!(
            (distribution.samples, distribution.excluded_sessions),
            (3, 1)
        );
    }

    #[test]
    fn analytics_compact_rank_omits_zero_but_full_paging_preserves_zero_and_unknown() {
        let zero = row("zero", 0, Some("0"), Some(0));
        let mut unknown = row("unknown", 0, None, None);
        unknown.tokens = None;
        let mut partial = row("partial", 7, Some("0.01"), Some(1));
        partial.estimate.availability = "partial".into();
        let rows = vec![zero, unknown, partial];
        let top = compact_top(&rows);
        for ranked in [top.tokens, top.cost, top.active] {
            assert_eq!(
                ranked
                    .iter()
                    .map(|row| row.session_key.as_str())
                    .collect::<Vec<_>>(),
                ["partial"]
            );
        }
        let full = Snapshot { rows, ..snapshot() };
        for sort_by in [SortBy::Tokens, SortBy::Cost, SortBy::Active] {
            let result = page_on(
                &full,
                "id",
                expected(),
                sort_by,
                0,
                10,
                None,
                None,
                "ofox.ai",
            )
            .unwrap();
            assert_eq!(result.total, 3);
            assert_eq!(
                result
                    .data
                    .iter()
                    .map(|row| row.session_key.as_str())
                    .collect::<Vec<_>>(),
                ["partial", "zero", "unknown"]
            );
            assert_eq!(result.data[1].estimate.known_cost_usd.as_deref(), Some("0"));
            assert!(result.data[2].estimate.known_cost_usd.is_none());
        }
    }
}
