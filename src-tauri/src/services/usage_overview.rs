//! Everything the usage page shows for one period, in one value.
//!
//! [`build_overview`] answers `get_usage_overview`: the totals, a time series
//! (hourly for today, daily, or weekly for a long "all"), and the per-tool and
//! per-model breakdowns. They all come from the same rows (live calls plus the
//! daily rollups of old ones), with the same range and the same rollup-day rule,
//! so the series, the tools and the models each add up to the totals.
//!
//! Times are epoch milliseconds in the shape sent to the page; the database keeps
//! seconds. A period starts at local midnight (today, 7d = today and the 6 days
//! before it, 30d = today and the 29 before) and ends now; `all` has no start.
//! Buckets are cut at local midnights, so a day with a DST change has 23 or 25
//! hours and its hourly buckets are still one hour of real time each.
//!
//! Costs are estimates (tokens priced at OFox's prices); the page says so.

use crate::database::Database;
use crate::error::AppError;
use crate::services::usage_stats::{AppTypeStats, ModelStats, SeriesBucket, UsageSums};
use chrono::{Datelike, Duration, LocalResult, NaiveDate, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};

/// An "all" period longer than this many days is drawn in weeks.
const WEEKLY_AFTER_DAYS: i64 = 180;

/// Tools OFox can bind but whose sessions have no usage reader yet.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsagePeriod {
    #[serde(rename = "today")]
    Today,
    #[serde(rename = "7d")]
    SevenDays,
    #[serde(rename = "30d")]
    ThirtyDays,
    #[serde(rename = "all")]
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    Hour,
    Day,
    Week,
}

/// A sum of calls. `total_tokens` is input + output (cache traffic is counted
/// apart); `cache_hit_rate` is cache reads over all the prompt tokens
/// (input + cache reads + cache writes), 0 when there were none.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cache_hit_rate: f64,
    pub cost_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<super::ofox_pricing::CostEstimate>,
}

impl UsageTotals {
    fn new(
        requests: u64,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        cost_usd: f64,
    ) -> Self {
        let prompt_tokens = input_tokens + cache_read_tokens + cache_write_tokens;
        let cache_hit_rate = if prompt_tokens == 0 {
            0.0
        } else {
            cache_read_tokens as f64 / prompt_tokens as f64
        };
        Self {
            requests,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            total_tokens: input_tokens + output_tokens,
            cache_hit_rate,
            cost_usd: round6(cost_usd),
            estimate: None,
        }
    }
}

/// One bucket of the series; `start` is when it begins.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSeriesPoint {
    pub start: i64,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<super::ofox_pricing::CostEstimate>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUsage {
    pub app_type: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

/// `priced` is false when no call of the model got a price: its cost is then
/// unknown, not zero. `app_types` are the tools that used it, heaviest first.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub app_types: Vec<String>,
    pub priced: bool,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

/// `start` is `None` for `all`; `end` is now. `by_tool` and `by_model` come
/// largest first (by input + output tokens).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageOverview {
    pub period: UsagePeriod,
    pub start: Option<i64>,
    pub end: i64,
    pub granularity: Granularity,
    pub totals: UsageTotals,
    pub series: Vec<UsageSeriesPoint>,
    pub by_tool: Vec<ToolUsage>,
    pub by_model: Vec<ModelUsage>,
    /// When the last usage sync finished; `None` before the first of this run.
    pub last_synced_at: Option<i64>,
    pub unsupported_tools: Vec<String>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub analytics: Option<super::usage_analytics::OverviewAnalytics>,
}

fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

fn cost_of(text: &str) -> f64 {
    text.parse().unwrap_or(0.0)
}

/// The range of a period and the buckets its series is cut into (seconds).
#[derive(Debug, Clone, PartialEq)]
struct Plan {
    start: Option<i64>,
    /// The last second of the range: now.
    end: i64,
    granularity: Granularity,
    buckets: Vec<SeriesBucket>,
}

/// The first instant of `date` in `tz`. A midnight that does not exist (a clock
/// that skips 00:00 to 01:00) is the first instant after the gap; one that
/// happens twice is the first of them.
fn local_midnight<Tz: TimeZone>(tz: &Tz, date: NaiveDate) -> i64 {
    let midnight = date.and_time(NaiveTime::MIN);
    let mut local = midnight;
    // a gap is at most a few hours; step through it half an hour at a time
    for _ in 0..48 {
        match tz.from_local_datetime(&local) {
            LocalResult::Single(t) => return t.timestamp(),
            LocalResult::Ambiguous(first, _) => return first.timestamp(),
            LocalResult::None => local += Duration::minutes(30),
        }
    }
    midnight.and_utc().timestamp()
}

fn day_buckets<Tz: TimeZone>(tz: &Tz, first: NaiveDate, last: NaiveDate) -> Vec<SeriesBucket> {
    first
        .iter_days()
        .take_while(|day| *day <= last)
        .map(|day| SeriesBucket {
            start: local_midnight(tz, day),
            first_date: day,
        })
        .collect()
}

fn week_buckets<Tz: TimeZone>(tz: &Tz, first: NaiveDate, last: NaiveDate) -> Vec<SeriesBucket> {
    let monday = first - Duration::days(first.weekday().num_days_from_monday() as i64);
    monday
        .iter_days()
        .step_by(7)
        .take_while(|day| *day <= last)
        .map(|day| SeriesBucket {
            start: local_midnight(tz, day),
            first_date: day,
        })
        .collect()
}

/// `earliest` is the local date of the oldest usage, only read for `All`.
fn plan<Tz: TimeZone>(
    period: UsagePeriod,
    now_ms: i64,
    tz: &Tz,
    earliest: Option<NaiveDate>,
) -> Result<Plan, AppError> {
    let end = now_ms.div_euclid(1000);
    let today = tz
        .timestamp_opt(end, 0)
        .single()
        .ok_or_else(|| AppError::Message(format!("cannot read {end} as a local time")))?
        .date_naive();

    let days_ending_today = |count: i64| Plan {
        start: Some(local_midnight(tz, today - Duration::days(count - 1))),
        end,
        granularity: Granularity::Day,
        buckets: day_buckets(tz, today - Duration::days(count - 1), today),
    };

    Ok(match period {
        UsagePeriod::Today => {
            let start = local_midnight(tz, today);
            Plan {
                start: Some(start),
                end,
                granularity: Granularity::Hour,
                buckets: (0..)
                    .map(|hour| start + hour * 3600)
                    .take_while(|bucket_start| *bucket_start <= end)
                    .map(|bucket_start| SeriesBucket {
                        start: bucket_start,
                        first_date: today,
                    })
                    .collect(),
            }
        }
        UsagePeriod::SevenDays => days_ending_today(7),
        UsagePeriod::ThirtyDays => days_ending_today(30),
        UsagePeriod::All => {
            let first = earliest.map(|date| date.min(today));
            match first {
                None => Plan {
                    start: None,
                    end,
                    granularity: Granularity::Day,
                    buckets: Vec::new(),
                },
                Some(first) if (today - first).num_days() > WEEKLY_AFTER_DAYS => Plan {
                    start: None,
                    end,
                    granularity: Granularity::Week,
                    buckets: week_buckets(tz, first, today),
                },
                Some(first) => Plan {
                    start: None,
                    end,
                    granularity: Granularity::Day,
                    buckets: day_buckets(tz, first, today),
                },
            }
        }
    })
}

/// The local date of the oldest usage, calls or rollups.
fn earliest_local_date<Tz: TimeZone>(
    db: &Database,
    tz: &Tz,
) -> Result<Option<NaiveDate>, AppError> {
    let (created_at, rollup_date) = db.get_earliest_usage()?;
    let call_date = created_at
        .and_then(|secs| tz.timestamp_opt(secs, 0).single())
        .map(|time| time.date_naive());
    Ok([call_date, rollup_date].into_iter().flatten().min())
}

fn series_point(start: i64, sums: &UsageSums) -> UsageSeriesPoint {
    UsageSeriesPoint {
        start: start * 1000,
        requests: sums.requests,
        input_tokens: sums.input_tokens,
        output_tokens: sums.output_tokens,
        cache_read_tokens: sums.cache_read_tokens,
        cache_write_tokens: sums.cache_creation_tokens,
        cost_usd: round6(sums.cost),
        estimate: None,
    }
}

fn tool_usage(stats: AppTypeStats) -> ToolUsage {
    ToolUsage {
        totals: UsageTotals::new(
            stats.request_count,
            stats.input_tokens,
            stats.output_tokens,
            stats.cache_read_tokens,
            stats.cache_creation_tokens,
            cost_of(&stats.total_cost),
        ),
        app_type: stats.app_type,
    }
}

fn model_usage(stats: ModelStats) -> ModelUsage {
    ModelUsage {
        totals: UsageTotals::new(
            stats.request_count,
            stats.input_tokens,
            stats.output_tokens,
            stats.cache_read_tokens,
            stats.cache_creation_tokens,
            cost_of(&stats.total_cost),
        ),
        model: stats.model,
        app_types: stats.app_types,
        priced: stats.priced,
    }
}

/// The overview of `period` as of `now_ms`, with days cut at midnight in `tz`.
/// Does not sync: call [`crate::services::usage_sync::sync_all`] first.
pub fn build_overview<Tz: TimeZone>(
    db: &Database,
    period: UsagePeriod,
    now_ms: i64,
    tz: &Tz,
    last_synced_at: Option<i64>,
) -> Result<UsageOverview, AppError> {
    let earliest = if period == UsagePeriod::All {
        earliest_local_date(db, tz)?
    } else {
        None
    };
    let plan = plan(period, now_ms, tz, earliest)?;

    let summary = db.get_usage_summary(plan.start, Some(plan.end), None)?;
    let totals = UsageTotals::new(
        summary.total_requests,
        summary.total_input_tokens,
        summary.total_output_tokens,
        summary.total_cache_read_tokens,
        summary.total_cache_creation_tokens,
        cost_of(&summary.total_cost),
    );
    let series = db
        .get_usage_series(&plan.buckets, plan.start, plan.end)?
        .iter()
        .zip(&plan.buckets)
        .map(|(sums, bucket)| series_point(bucket.start, sums))
        .collect();

    let by_tokens = |a: &UsageTotals, b: &UsageTotals| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| b.cost_usd.total_cmp(&a.cost_usd))
    };
    let mut by_tool: Vec<ToolUsage> = db
        .get_app_type_stats(plan.start, Some(plan.end))?
        .into_iter()
        .map(tool_usage)
        .collect();
    by_tool
        .sort_by(|a, b| by_tokens(&a.totals, &b.totals).then_with(|| a.app_type.cmp(&b.app_type)));
    let mut by_model: Vec<ModelUsage> = db
        .get_model_stats(plan.start, Some(plan.end), None)?
        .into_iter()
        .map(model_usage)
        .collect();
    by_model.sort_by(|a, b| by_tokens(&a.totals, &b.totals).then_with(|| a.model.cmp(&b.model)));

    Ok(UsageOverview {
        period,
        start: plan.start.map(|start| start * 1000),
        end: now_ms,
        granularity: plan.granularity,
        totals,
        series,
        by_tool,
        by_model,
        last_synced_at,
        unsupported_tools: super::usage_query::source_capabilities()
            .into_iter()
            .filter(|c| !c.usage_supported)
            .map(|c| c.tool_id)
            .collect(),
        analytics: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytics_legacy_and_query_overviews_share_one_capability_registry() {
        let db = Database::memory().unwrap();
        let legacy =
            build_overview(&db, UsagePeriod::All, 1_760_000_000_000, &chrono::Utc, None).unwrap();
        let expected = crate::services::usage_query::source_capabilities()
            .into_iter()
            .filter(|c| !c.usage_supported)
            .map(|c| c.tool_id)
            .collect::<Vec<_>>();
        assert_eq!(legacy.unsupported_tools, expected);
    }

    #[test]
    fn analytics_capabilities_register_workbuddy_usage_without_claiming_session_analytics() {
        let db = Database::memory().unwrap();
        let result =
            build_overview(&db, UsagePeriod::All, 1_760_000_000_000, &chrono::Utc, None).unwrap();
        assert!(!result.unsupported_tools.iter().any(|id| id == "workbuddy"));
        let capability = crate::services::usage_query::source_capabilities()
            .into_iter()
            .find(|c| c.tool_id == "workbuddy")
            .unwrap();
        assert!(capability.usage_supported);
        assert_eq!(capability.source_ids, ["workbuddy_session"]);
        assert!(!capability.session_analytics_supported);
    }
    use chrono::{FixedOffset, Local, NaiveDateTime, Offset, Timelike, Weekday};
    use rusqlite::params;

    // ── time zones with a DST change, built by hand: the tests must not depend
    //    on the zone of the machine that runs them ───────────────────────────

    /// `std` seconds east of UTC, `dst` between the UTC instants `dst_start` and
    /// `dst_end` (epoch seconds).
    #[derive(Debug, Clone, Copy)]
    struct DstZone {
        std: i32,
        dst: i32,
        dst_start: i64,
        dst_end: i64,
    }

    #[derive(Debug, Clone, Copy)]
    struct DstOffset {
        zone: DstZone,
        east: i32,
    }

    impl Offset for DstOffset {
        fn fix(&self) -> FixedOffset {
            FixedOffset::east_opt(self.east).unwrap()
        }
    }

    impl DstZone {
        fn offset_at(&self, utc: i64) -> i32 {
            if utc >= self.dst_start && utc < self.dst_end {
                self.dst
            } else {
                self.std
            }
        }

        fn offset(&self, east: i32) -> DstOffset {
            DstOffset { zone: *self, east }
        }
    }

    impl TimeZone for DstZone {
        type Offset = DstOffset;

        fn from_offset(offset: &DstOffset) -> Self {
            offset.zone
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<DstOffset> {
            self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
        }

        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<DstOffset> {
            let as_utc = local.and_utc().timestamp();
            let holds = |east: i32| self.offset_at(as_utc - east as i64) == east;
            match (holds(self.dst), holds(self.std)) {
                (true, true) => {
                    LocalResult::Ambiguous(self.offset(self.dst), self.offset(self.std))
                }
                (true, false) => LocalResult::Single(self.offset(self.dst)),
                (false, true) => LocalResult::Single(self.offset(self.std)),
                (false, false) => LocalResult::None,
            }
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> DstOffset {
            self.offset_from_utc_datetime(&utc.and_time(NaiveTime::MIN))
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> DstOffset {
            self.offset(self.offset_at(utc.and_utc().timestamp()))
        }
    }

    const HOUR: i64 = 3600;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        chrono::Utc
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp()
    }

    fn quarter_hour_midnight_gap() -> DstZone {
        DstZone {
            std: 0,
            dst: 15 * 60,
            dst_start: utc(2026, 10, 4, 0, 0),
            dst_end: utc(2026, 10, 6, 0, 0),
        }
    }

    fn skipped_natural_day() -> DstZone {
        DstZone {
            std: -12 * 3600,
            dst: 12 * 3600,
            dst_start: utc(2026, 10, 4, 12, 0),
            dst_end: utc(2026, 10, 6, 12, 0),
        }
    }

    #[test]
    fn analytics_midnight_data_finds_the_first_valid_second_of_a_quarter_hour_gap() {
        let at = super::super::usage_data::midnight(
            &quarter_hour_midnight_gap(),
            NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(),
        )
        .unwrap();
        assert_eq!(at, utc(2026, 10, 4, 0, 0) * 1000);
    }

    #[test]
    fn analytics_midnight_series_finds_the_first_valid_second_of_a_quarter_hour_gap() {
        let at = super::super::usage_analytics::boundary(
            &quarter_hour_midnight_gap(),
            NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(),
        )
        .unwrap();
        assert_eq!(at, utc(2026, 10, 4, 0, 0) * 1000);
    }

    #[test]
    fn analytics_midnight_data_accepts_an_entire_skipped_day_at_the_next_boundary() {
        let at = super::super::usage_data::midnight(
            &skipped_natural_day(),
            NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(),
        )
        .unwrap();
        assert_eq!(at, utc(2026, 10, 4, 12, 0) * 1000);
    }

    #[test]
    fn analytics_midnight_series_accepts_an_entire_skipped_day_at_the_next_boundary() {
        let at = super::super::usage_analytics::boundary(
            &skipped_natural_day(),
            NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(),
        )
        .unwrap();
        assert_eq!(at, utc(2026, 10, 4, 12, 0) * 1000);
    }

    /// New York in 2026: UTC-5, UTC-4 from 2026-03-08 02:00 to 2026-11-01 02:00 local.
    fn new_york() -> DstZone {
        DstZone {
            std: -5 * 3600,
            dst: -4 * 3600,
            dst_start: utc(2026, 3, 8, 7, 0),
            dst_end: utc(2026, 11, 1, 6, 0),
        }
    }

    /// A zone whose clocks jump from 00:00 to 01:00 on 2026-10-04, so that day
    /// has no midnight.
    fn midnight_gap() -> DstZone {
        DstZone {
            std: -3 * 3600,
            dst: -2 * 3600,
            dst_start: utc(2026, 10, 4, 3, 0),
            dst_end: utc(2027, 2, 21, 2, 0),
        }
    }

    /// A zone whose clocks go back from 01:00 to 00:00 on 2026-11-01, so that
    /// day's midnight happens twice.
    fn midnight_twice() -> DstZone {
        DstZone {
            std: -5 * 3600,
            dst: -4 * 3600,
            dst_start: utc(2026, 3, 8, 6, 0),
            dst_end: utc(2026, 11, 1, 5, 0),
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn starts(plan: &Plan) -> Vec<i64> {
        plan.buckets.iter().map(|b| b.start).collect()
    }

    fn gaps(starts: &[i64]) -> Vec<i64> {
        starts.windows(2).map(|w| (w[1] - w[0]) / HOUR).collect()
    }

    // ── the period builder ───────────────────────────────────────────────────

    #[test]
    fn today_starts_at_local_midnight_and_has_an_hourly_bucket_up_to_now() {
        let tz = FixedOffset::east_opt(8 * 3600).unwrap();
        let now = utc(2026, 10, 14, 5, 30); // 13:30 in +08:00
        let plan = plan(UsagePeriod::Today, now * 1000, &tz, None).unwrap();

        let midnight = utc(2026, 10, 13, 16, 0); // 00:00 on the 14th, +08:00
        assert_eq!(plan.start, Some(midnight));
        assert_eq!(plan.end, now);
        assert_eq!(plan.granularity, Granularity::Hour);
        assert_eq!(plan.buckets.len(), 14); // 00:00 .. 13:00
        assert_eq!(plan.buckets[0].start, midnight);
        assert_eq!(plan.buckets[13].start, midnight + 13 * HOUR);
    }

    #[test]
    fn today_at_exactly_midnight_has_one_bucket() {
        let tz = FixedOffset::west_opt(7 * 3600).unwrap();
        let midnight = utc(2026, 10, 14, 7, 0); // 00:00 on the 14th, -07:00
        let plan = plan(UsagePeriod::Today, midnight * 1000, &tz, None).unwrap();

        assert_eq!(plan.start, Some(midnight));
        assert_eq!(starts(&plan), [midnight]);
    }

    #[test]
    fn one_second_before_midnight_is_still_the_previous_day() {
        let tz = FixedOffset::east_opt(2 * 3600).unwrap();
        let before = utc(2026, 10, 13, 22, 0) - 1; // 23:59:59 on the 13th, +02:00
        let plan = plan(UsagePeriod::Today, before * 1000 + 999, &tz, None).unwrap();

        assert_eq!(plan.start, Some(utc(2026, 10, 12, 22, 0)));
        assert_eq!(plan.buckets.len(), 24);
    }

    #[test]
    fn seven_and_thirty_days_are_daily_buckets_ending_with_today() {
        let tz = FixedOffset::east_opt(9 * 3600).unwrap();
        let now = utc(2026, 10, 14, 3, 0) * 1000; // 12:00 on the 14th, +09:00

        let week = plan(UsagePeriod::SevenDays, now, &tz, None).unwrap();
        let month = plan(UsagePeriod::ThirtyDays, now, &tz, None).unwrap();

        assert_eq!(week.granularity, Granularity::Day);
        assert_eq!(week.buckets.len(), 7);
        assert_eq!(week.start, Some(utc(2026, 10, 7, 15, 0))); // 00:00 on the 8th
        assert_eq!(week.buckets[0].first_date, date(2026, 10, 8));
        assert_eq!(week.buckets[6].first_date, date(2026, 10, 14));
        assert_eq!(gaps(&starts(&week)), [24; 6]);
        assert_eq!(month.buckets.len(), 30);
        assert_eq!(month.buckets[0].first_date, date(2026, 9, 15));
    }

    #[test]
    fn a_day_with_a_dst_change_is_23_or_25_hours_long() {
        let tz = new_york();
        // 2026-03-08 is 23 hours long, 2026-11-01 is 25
        let spring = plan(
            UsagePeriod::SevenDays,
            utc(2026, 3, 10, 12, 0) * 1000,
            &tz,
            None,
        )
        .unwrap();
        let fall = plan(
            UsagePeriod::SevenDays,
            utc(2026, 11, 3, 12, 0) * 1000,
            &tz,
            None,
        )
        .unwrap();

        // buckets 2026-03-04 .. 03-10: 03-08 is the 5th
        assert_eq!(gaps(&starts(&spring)), [24, 24, 24, 24, 23, 24]);
        // buckets 2026-10-28 .. 11-03: 11-01 is the 5th
        assert_eq!(gaps(&starts(&fall)), [24, 24, 24, 24, 25, 24]);
        assert_eq!(spring.buckets[4].first_date, date(2026, 3, 8));
    }

    #[test]
    fn today_on_a_dst_day_is_counted_in_real_hours() {
        let tz = new_york();
        // 23:30 local on the 23-hour day: 02:00 does not exist, so 23 buckets
        let spring_now = utc(2026, 3, 9, 3, 30); // 23:30 EDT on 2026-03-08
        let spring = plan(UsagePeriod::Today, spring_now * 1000, &tz, None).unwrap();
        // 23:30 local on the 25-hour day: 25 buckets
        let fall_now = utc(2026, 11, 2, 4, 30); // 23:30 EST on 2026-11-01
        let fall = plan(UsagePeriod::Today, fall_now * 1000, &tz, None).unwrap();

        assert_eq!(spring.start, Some(utc(2026, 3, 8, 5, 0)));
        assert_eq!(spring.buckets.len(), 23);
        assert_eq!(fall.start, Some(utc(2026, 11, 1, 4, 0)));
        assert_eq!(fall.buckets.len(), 25);
        assert_eq!(gaps(&starts(&fall)), [1; 24]);
    }

    #[test]
    fn a_midnight_that_does_not_exist_starts_the_day_when_the_clocks_jump() {
        let tz = midnight_gap();
        let now = utc(2026, 10, 4, 15, 0); // 13:00 local on the 4th
        let plan = plan(UsagePeriod::Today, now * 1000, &tz, None).unwrap();

        // 00:00 never happened; the day began at 01:00 local = 03:00 UTC
        assert_eq!(plan.start, Some(utc(2026, 10, 4, 3, 0)));
        assert_eq!(plan.buckets.len(), 13);
    }

    #[test]
    fn a_midnight_that_happens_twice_starts_the_day_at_the_first() {
        let tz = midnight_twice();
        let now = utc(2026, 11, 1, 17, 0); // 12:00 local on the 1st
        let plan = plan(UsagePeriod::Today, now * 1000, &tz, None).unwrap();

        // 00:00 EDT = 04:00 UTC came before 00:00 EST = 05:00 UTC
        assert_eq!(plan.start, Some(utc(2026, 11, 1, 4, 0)));
        assert_eq!(plan.buckets.len(), 14);
    }

    #[test]
    fn all_without_usage_has_no_buckets() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let plan = plan(UsagePeriod::All, utc(2026, 10, 14, 3, 0) * 1000, &tz, None).unwrap();

        assert_eq!(plan.start, None);
        assert_eq!(plan.granularity, Granularity::Day);
        assert!(plan.buckets.is_empty());
    }

    #[test]
    fn all_is_daily_up_to_180_days_and_weekly_beyond() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let now = utc(2026, 10, 14, 3, 0) * 1000; // Wednesday
        let today = date(2026, 10, 14);

        let daily = plan(
            UsagePeriod::All,
            now,
            &tz,
            Some(today - Duration::days(180)),
        )
        .unwrap();
        let weekly = plan(
            UsagePeriod::All,
            now,
            &tz,
            Some(today - Duration::days(181)),
        )
        .unwrap();

        assert_eq!(daily.granularity, Granularity::Day);
        assert_eq!(daily.buckets.len(), 181);
        assert_eq!(weekly.granularity, Granularity::Week);
        assert_eq!(weekly.start, None);
        // weeks start on Monday; the first holds the earliest day, the last holds today
        let first = weekly.buckets[0].first_date;
        let last = weekly.buckets.last().unwrap().first_date;
        assert_eq!(first.weekday(), Weekday::Mon);
        assert!(first <= today - Duration::days(181));
        assert_eq!(last, date(2026, 10, 12));
        assert_eq!(gaps(&starts(&weekly)).iter().max(), Some(&168));
    }

    #[test]
    fn usage_dated_in_the_future_is_clamped_to_today() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let now = utc(2026, 10, 14, 3, 0) * 1000;

        let plan = plan(UsagePeriod::All, now, &tz, Some(date(2030, 1, 1))).unwrap();

        assert_eq!(starts(&plan), [utc(2026, 10, 14, 0, 0)]);
    }

    #[test]
    fn periods_are_named_as_the_page_names_them() {
        for (text, period) in [
            ("today", UsagePeriod::Today),
            ("7d", UsagePeriod::SevenDays),
            ("30d", UsagePeriod::ThirtyDays),
            ("all", UsagePeriod::All),
        ] {
            let quoted = format!("\"{text}\"");
            assert_eq!(
                serde_json::from_str::<UsagePeriod>(&quoted).unwrap(),
                period
            );
            assert_eq!(serde_json::to_string(&period).unwrap(), quoted);
        }
        assert!(serde_json::from_str::<UsagePeriod>("\"week\"").is_err());
    }

    // ── the overview over seeded rows ───────────────────────────────────────

    fn local_ts(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .earliest()
            .unwrap()
            .timestamp()
    }

    fn insert_log(
        db: &Database,
        id: &str,
        app_type: &str,
        model: &str,
        created_at: i64,
        tokens: [i64; 4],
        cost: &str,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, provider_id, app_type, model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                total_cost_usd, latency_ms, status_code, created_at
            ) VALUES (?, 'p', ?, ?, ?, ?, ?, ?, ?, 0, 200, ?)",
            params![
                id, app_type, model, tokens[0], tokens[1], tokens[2], tokens[3], cost, created_at
            ],
        )
        .unwrap();
    }

    fn insert_rollup(
        db: &Database,
        day: &str,
        app_type: &str,
        model: &str,
        requests: i64,
        tokens: [i64; 4],
        cost: &str,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage_daily_rollups (
                date, app_type, provider_id, model, request_count, success_count,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                total_cost_usd, avg_latency_ms
            ) VALUES (?, ?, 'p', ?, ?, ?, ?, ?, ?, ?, ?, 0)",
            params![
                day, app_type, model, requests, requests, tokens[0], tokens[1], tokens[2],
                tokens[3], cost
            ],
        )
        .unwrap();
    }

    /// 7d at 2024-01-08 12:00 covers 01-02 00:00 .. now. What is outside it is
    /// seeded too: a call after now and before the start, the rollup of the
    /// partial end day and the rollup before the start.
    fn seed_week(db: &Database) {
        let call = |id, app, model, (d, h), tokens, cost| {
            insert_log(db, id, app, model, local_ts(2024, 1, d, h, 0), tokens, cost);
        };
        call(
            "r1",
            "claude",
            "claude-sonnet",
            (2, 10),
            [100, 50, 1000, 200],
            "0.10",
        );
        call(
            "r2",
            "claude",
            "claude-sonnet",
            (3, 9),
            [10, 5, 0, 0],
            "0.01",
        );
        call("r3", "codex", "gpt-5", (3, 10), [300, 100, 2000, 0], "0.20");
        call("r4", "opencode", "free-model", (8, 11), [1, 2, 3, 4], "0");
        call(
            "after",
            "claude",
            "claude-sonnet",
            (8, 13),
            [9, 9, 9, 9],
            "9",
        );
        call(
            "before",
            "claude",
            "claude-sonnet",
            (1, 23),
            [9, 9, 9, 9],
            "9",
        );

        insert_rollup(
            db,
            "2024-01-04",
            "claude",
            "claude-sonnet",
            4,
            [1000, 500, 5000, 100],
            "1.00",
        );
        insert_rollup(
            db,
            "2024-01-07",
            "opencode",
            "gpt-5",
            2,
            [20, 10, 0, 0],
            "0.50",
        );
        insert_rollup(
            db,
            "2024-01-08",
            "claude",
            "claude-sonnet",
            9,
            [9, 9, 9, 9],
            "9",
        );
        insert_rollup(
            db,
            "2024-01-01",
            "claude",
            "claude-sonnet",
            9,
            [9, 9, 9, 9],
            "9",
        );
    }

    fn week_overview(db: &Database) -> UsageOverview {
        let now_ms = local_ts(2024, 1, 8, 12, 0) * 1000;
        build_overview(db, UsagePeriod::SevenDays, now_ms, &Local, Some(42)).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_totals_are_the_hand_computed_sums_of_calls_and_rollups() {
        let db = Database::memory().unwrap();
        seed_week(&db);

        let o = week_overview(&db);

        assert_eq!(o.period, UsagePeriod::SevenDays);
        assert_eq!(o.start, Some(local_ts(2024, 1, 2, 0, 0) * 1000));
        assert_eq!(o.end, local_ts(2024, 1, 8, 12, 0) * 1000);
        assert_eq!(o.granularity, Granularity::Day);
        let t = &o.totals;
        assert_eq!(
            (t.requests, t.input_tokens, t.output_tokens),
            (10, 1431, 667)
        );
        assert_eq!((t.cache_read_tokens, t.cache_write_tokens), (8003, 304));
        assert_eq!(t.total_tokens, 2098);
        assert!(close(t.cost_usd, 1.81), "{}", t.cost_usd);
        assert!(close(t.cache_hit_rate, 8003.0 / (1431.0 + 8003.0 + 304.0)));
        assert_eq!(o.last_synced_at, Some(42));
        assert_eq!(o.unsupported_tools, ["chatgpt", "hermes"]);
    }

    #[test]
    fn the_series_has_a_bucket_per_day_and_adds_up_to_the_totals() {
        let db = Database::memory().unwrap();
        seed_week(&db);

        let o = week_overview(&db);

        let day = |d| local_ts(2024, 1, d, 0, 0) * 1000;
        let got: Vec<(i64, u64)> = o.series.iter().map(|p| (p.start, p.requests)).collect();
        assert_eq!(
            got,
            [
                (day(2), 1),
                (day(3), 2),
                (day(4), 4), // the rollup
                (day(5), 0),
                (day(6), 0),
                (day(7), 2), // the rollup
                (day(8), 1),
            ]
        );
        let sum = |f: fn(&UsageSeriesPoint) -> u64| o.series.iter().map(f).sum::<u64>();
        assert_eq!(sum(|p| p.requests), o.totals.requests);
        assert_eq!(sum(|p| p.input_tokens), o.totals.input_tokens);
        assert_eq!(sum(|p| p.output_tokens), o.totals.output_tokens);
        assert_eq!(sum(|p| p.cache_read_tokens), o.totals.cache_read_tokens);
        assert_eq!(sum(|p| p.cache_write_tokens), o.totals.cache_write_tokens);
        assert!(close(
            o.series.iter().map(|p| p.cost_usd).sum::<f64>(),
            o.totals.cost_usd
        ));
    }

    #[test]
    fn the_tools_are_largest_first_and_add_up_to_the_totals() {
        let db = Database::memory().unwrap();
        seed_week(&db);

        let o = week_overview(&db);

        let got: Vec<(&str, u64, u64)> = o
            .by_tool
            .iter()
            .map(|t| {
                (
                    t.app_type.as_str(),
                    t.totals.requests,
                    t.totals.total_tokens,
                )
            })
            .collect();
        assert_eq!(
            got,
            [("claude", 6, 1665), ("codex", 1, 400), ("opencode", 3, 33)]
        );
        assert_eq!(o.by_tool[0].totals.cache_read_tokens, 6000);
        assert_eq!(o.by_tool.iter().map(|t| t.totals.requests).sum::<u64>(), 10);
    }

    #[test]
    fn the_models_say_which_tools_used_them_and_whether_they_are_priced() {
        let db = Database::memory().unwrap();
        seed_week(&db);

        let o = week_overview(&db);

        let got: Vec<(&str, Vec<&str>, bool, u64)> = o
            .by_model
            .iter()
            .map(|m| {
                (
                    m.model.as_str(),
                    m.app_types.iter().map(String::as_str).collect(),
                    m.priced,
                    m.totals.requests,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("claude-sonnet", vec!["claude"], true, 6),
                ("gpt-5", vec!["codex", "opencode"], true, 3),
                ("free-model", vec!["opencode"], false, 1),
            ]
        );
        assert!(close(o.by_model[1].totals.cost_usd, 0.70));
        assert_eq!(
            o.by_model.iter().map(|m| m.totals.requests).sum::<u64>(),
            10
        );
    }

    #[test]
    fn today_is_hourly_and_ignores_what_came_before_midnight() {
        let db = Database::memory().unwrap();
        let at = |d, h, mi| local_ts(2024, 1, d, h, mi);
        insert_log(&db, "a", "claude", "m", at(8, 0, 30), [10, 5, 0, 0], "0.01");
        insert_log(&db, "b", "claude", "m", at(8, 2, 0), [20, 5, 0, 0], "0.02");
        insert_log(&db, "c", "claude", "m", at(8, 2, 59), [30, 5, 0, 0], "0.03");
        insert_log(
            &db,
            "yesterday",
            "claude",
            "m",
            at(7, 23, 59),
            [99, 99, 0, 0],
            "9",
        );

        let now_ms = at(8, 3, 30) * 1000;
        let o = build_overview(&db, UsagePeriod::Today, now_ms, &Local, None).unwrap();

        assert_eq!(o.granularity, Granularity::Hour);
        assert_eq!(o.series.len(), 4); // 00:00 .. 03:00
        let got: Vec<u64> = o.series.iter().map(|p| p.requests).collect();
        assert_eq!(got, [1, 0, 2, 0]);
        assert_eq!(o.totals.requests, 3);
        assert_eq!(o.totals.input_tokens, 60);
        assert_eq!(o.last_synced_at, None);
    }

    #[test]
    fn all_is_weekly_beyond_180_days_and_keeps_the_rollups_of_old_weeks() {
        let db = Database::memory().unwrap();
        // 2024-01-03 is a Wednesday; the rollups are the oldest usage, a call is today
        insert_rollup(&db, "2023-06-07", "claude", "m", 3, [30, 3, 0, 0], "0.30");
        insert_rollup(&db, "2023-06-08", "claude", "m", 2, [20, 2, 0, 0], "0.20");
        insert_log(
            &db,
            "now",
            "claude",
            "m",
            local_ts(2024, 1, 3, 9, 0),
            [5, 1, 0, 0],
            "0.05",
        );

        let now_ms = local_ts(2024, 1, 3, 12, 0) * 1000;
        let o = build_overview(&db, UsagePeriod::All, now_ms, &Local, None).unwrap();

        assert_eq!(o.start, None);
        assert_eq!(o.granularity, Granularity::Week);
        // the weeks of 2023-06-05 and 2024-01-01 are the first and last
        assert_eq!(o.series[0].start, local_ts(2023, 6, 5, 0, 0) * 1000);
        assert_eq!(o.series[0].requests, 5);
        assert_eq!(
            o.series.last().unwrap().start,
            local_ts(2024, 1, 1, 0, 0) * 1000
        );
        assert_eq!(o.series.last().unwrap().requests, 1);
        assert_eq!(o.totals.requests, 6);
        assert_eq!(o.series.iter().map(|p| p.requests).sum::<u64>(), 6);
    }

    #[test]
    fn all_is_daily_for_a_short_history() {
        let db = Database::memory().unwrap();
        insert_log(
            &db,
            "a",
            "claude",
            "m",
            local_ts(2024, 1, 1, 9, 0),
            [5, 1, 0, 0],
            "0.05",
        );
        insert_log(
            &db,
            "b",
            "claude",
            "m",
            local_ts(2024, 1, 3, 9, 0),
            [5, 1, 0, 0],
            "0.05",
        );

        let now_ms = local_ts(2024, 1, 3, 12, 0) * 1000;
        let o = build_overview(&db, UsagePeriod::All, now_ms, &Local, None).unwrap();

        assert_eq!(o.granularity, Granularity::Day);
        let got: Vec<u64> = o.series.iter().map(|p| p.requests).collect();
        assert_eq!(got, [1, 0, 1]);
    }

    #[test]
    fn a_very_old_call_makes_a_long_weekly_series_not_an_error() {
        let db = Database::memory().unwrap();
        insert_log(&db, "old", "claude", "m", 1000, [5, 1, 0, 0], "0.05");

        let now_ms = local_ts(2024, 1, 3, 12, 0) * 1000;
        let o = build_overview(&db, UsagePeriod::All, now_ms, &Local, None).unwrap();

        assert_eq!(o.granularity, Granularity::Week);
        assert!(o.series.len() > 2500);
        assert_eq!(o.series.iter().map(|p| p.requests).sum::<u64>(), 1);
    }

    #[test]
    fn an_empty_database_gives_zeros_not_errors() {
        let db = Database::memory().unwrap();
        let now_ms = local_ts(2024, 1, 3, 12, 0) * 1000;

        let week = build_overview(&db, UsagePeriod::SevenDays, now_ms, &Local, None).unwrap();
        let all = build_overview(&db, UsagePeriod::All, now_ms, &Local, None).unwrap();

        assert_eq!(week.totals.requests, 0);
        assert_eq!(week.totals.cache_hit_rate, 0.0);
        assert_eq!(week.series.len(), 7);
        assert!(week.series.iter().all(|p| p.requests == 0));
        assert!(week.by_tool.is_empty() && week.by_model.is_empty());
        assert!(all.series.is_empty());
        assert_eq!(all.totals.requests, 0);
    }

    #[test]
    fn the_cache_hit_rate_is_reads_over_all_prompt_tokens() {
        let none = UsageTotals::new(0, 0, 0, 0, 0, 0.0);
        let some = UsageTotals::new(1, 100, 7, 300, 100, 0.0);

        assert_eq!(none.cache_hit_rate, 0.0);
        assert_eq!(some.cache_hit_rate, 300.0 / 500.0);
        assert_eq!(some.total_tokens, 107);
    }

    #[test]
    fn the_overview_serializes_in_the_shape_the_page_reads() {
        let db = Database::memory().unwrap();
        seed_week(&db);

        let json = serde_json::to_value(week_overview(&db)).unwrap();

        let object = json.as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "byModel",
                "byTool",
                "end",
                "granularity",
                "lastSyncedAt",
                "period",
                "series",
                "start",
                "totals",
                "unsupportedTools"
            ]
        );
        assert_eq!(json["period"], "7d");
        assert_eq!(json["granularity"], "day");
        let mut totals: Vec<&str> = json["totals"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        totals.sort_unstable();
        assert_eq!(
            totals,
            [
                "cacheHitRate",
                "cacheReadTokens",
                "cacheWriteTokens",
                "costUsd",
                "inputTokens",
                "outputTokens",
                "requests",
                "totalTokens"
            ]
        );
        // a tool and a model carry their totals alongside their own fields
        assert_eq!(json["byTool"][0]["appType"], "claude");
        assert_eq!(json["byTool"][0]["requests"], 6);
        assert_eq!(json["byModel"][0]["model"], "claude-sonnet");
        assert_eq!(json["byModel"][0]["appTypes"][0], "claude");
        assert_eq!(json["byModel"][0]["priced"], true);
        assert_eq!(json["byModel"][0]["totalTokens"], 1665);
        let point = json["series"][0].as_object().unwrap();
        assert!(point.contains_key("start") && point.contains_key("cacheWriteTokens"));
    }

    #[test]
    fn a_period_start_is_local_midnight_in_the_machines_zone() {
        // the same arithmetic as the tests above, but with the zone of production
        let db = Database::memory().unwrap();
        let now_ms = local_ts(2024, 1, 3, 12, 0) * 1000;

        let o = build_overview(&db, UsagePeriod::Today, now_ms, &Local, None).unwrap();

        let start = Local.timestamp_millis_opt(o.start.unwrap()).unwrap();
        assert_eq!((start.hour(), start.minute(), start.day()), (0, 0, 3));
    }
}
