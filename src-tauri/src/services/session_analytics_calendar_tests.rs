//! Calendar integration coverage through the production query entry point.
//! The bidirectional zone follows the existing usage_overview test fixture;
//! unlike a UTC-only offset stub it resolves real local gaps and folds.
use super::query;
use crate::{
    database::Database,
    services::{
        session_analytics::SessionAnalytics,
        session_analytics_index::{refresh_sources, SourceSpec},
        usage_health::{self, Channel, Outcome},
        usage_query::UsageQueryV1,
    },
};
use chrono::{
    FixedOffset, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc,
};
use serde_json::json;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 3_600_000;

#[derive(Debug, Clone, Copy)]
struct DstZone {
    standard: i32,
    daylight: i32,
    daylight_start: i64,
    daylight_end: i64,
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
    fn east_at(self, utc_ms: i64) -> i32 {
        if (self.daylight_start..self.daylight_end).contains(&utc_ms) {
            self.daylight
        } else {
            self.standard
        }
    }

    fn offset(self, east: i32) -> DstOffset {
        DstOffset { zone: self, east }
    }
}

impl TimeZone for DstZone {
    type Offset = DstOffset;

    fn from_offset(offset: &Self::Offset) -> Self {
        offset.zone
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<Self::Offset> {
        self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<Self::Offset> {
        let wall_ms = local.and_utc().timestamp_millis();
        let holds = |east: i32| self.east_at(wall_ms - i64::from(east) * 1_000) == east;
        match (holds(self.daylight), holds(self.standard)) {
            (true, true) => {
                LocalResult::Ambiguous(self.offset(self.daylight), self.offset(self.standard))
            }
            (true, false) => LocalResult::Single(self.offset(self.daylight)),
            (false, true) => LocalResult::Single(self.offset(self.standard)),
            (false, false) => LocalResult::None,
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> Self::Offset {
        self.offset_from_utc_datetime(&utc.and_time(NaiveTime::MIN))
    }

    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> Self::Offset {
        self.offset(self.east_at(utc.and_utc().timestamp_millis()))
    }
}

fn at(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .timestamp_millis()
}

fn query_range(start: i64, end: i64) -> UsageQueryV1 {
    UsageQueryV1 {
        version: 1,
        start_ms: Some(start),
        end_ms: end,
        tool_ids: vec!["claude".into()],
        model_ids: vec![],
        project_ids: vec![],
        source_ids: vec![],
    }
}

fn active_cell(result: &SessionAnalytics, weekday: u32, hour: u32) -> u64 {
    result
        .activity_by_hour
        .cells
        .iter()
        .find(|cell| cell.weekday == weekday && cell.hour == hour)
        .unwrap()
        .active_ms
}

fn assert_reconciles(result: &SessionAnalytics, active_ms: u64, messages: u64) {
    let known = result.known_metrics.as_ref().unwrap();
    assert_eq!((known.active_ms, known.messages), (active_ms, messages));
    assert_eq!(result.totals.active_ms, Some(active_ms));
    assert_eq!(
        result
            .series
            .iter()
            .map(|day| day.known_active_ms.unwrap())
            .sum::<u64>(),
        active_ms,
    );
    assert_eq!(
        result
            .series
            .iter()
            .map(|day| day.active_ms.unwrap())
            .sum::<u64>(),
        active_ms,
    );
    assert_eq!(
        result
            .activity_by_hour
            .cells
            .iter()
            .map(|cell| cell.active_ms)
            .sum::<u64>(),
        active_ms,
    );
    assert_eq!(
        result
            .series
            .iter()
            .map(|day| day.known_messages.unwrap())
            .sum::<u64>(),
        messages,
    );
    assert_eq!(result.metric_coverage.state, "complete");
}

#[test]
fn session_analytics_api_dst_natural_days_hours_and_clipped_midnight_activity_reconcile() {
    let zone = DstZone {
        standard: -5 * 3_600,
        daylight: -4 * 3_600,
        daylight_start: at("2026-03-08T07:00:00Z"),
        daylight_end: at("2026-11-01T06:00:00Z"),
    };
    // Prove that local-to-UTC conversion exercises a gap and a fold, not a
    // test stub that can only answer UTC-to-local calls.
    assert!(matches!(
        zone.with_ymd_and_hms(2026, 3, 8, 2, 30, 0),
        LocalResult::None
    ));
    assert!(matches!(
        zone.with_ymd_and_hms(2026, 11, 1, 1, 30, 0),
        LocalResult::Ambiguous(_, _)
    ));

    for (date, start, end, transition, day_hours, before_hour, after_hour) in [
        (
            "2026-03-08",
            at("2026-03-08T05:00:00Z"),
            at("2026-03-09T04:00:00Z"),
            zone.daylight_start,
            23,
            1,
            3,
        ),
        (
            "2026-11-01",
            at("2026-11-01T04:00:00Z"),
            at("2026-11-02T05:00:00Z"),
            zone.daylight_end,
            25,
            1,
            1,
        ),
    ] {
        let sandbox = tempfile::tempdir().unwrap();
        let source = sandbox.path().join("calendar.jsonl");
        let db = Database::memory().unwrap();
        let native_times = [
            start - MINUTE,
            start + MINUTE,
            transition - MINUTE,
            transition + MINUTE,
            end - MINUTE,
            end + MINUTE,
        ];
        let lines = native_times
            .into_iter()
            .enumerate()
            .map(|(i, timestamp)| {
                json!({
                    "type":"assistant","sessionId":"calendar","cwd":"/fixture",
                    "timestamp":Utc.timestamp_millis_opt(timestamp).unwrap().to_rfc3339(),
                    "message":{"id":format!("m{i}"),"role":"assistant","model":"m",
                        "content":[{"type":"text","text":"calendar fixture"}],
                        "usage":{"input_tokens":1,"output_tokens":1}}
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&source, lines).unwrap();
        refresh_sources(
            &db,
            &[SourceSpec {
                provider: "claude".into(),
                path: source,
                subagent: false,
                project_file: None,
                title_file: None,
            }],
        )
        .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            for channel in [Channel::Ledger, Channel::Analytics] {
                let attempt =
                    usage_health::begin_on(&conn, "claude", "session_log", channel, end).unwrap();
                assert!(usage_health::finish_on(&conn, &attempt, Outcome::Ok, end).unwrap());
            }
        }

        let day = query(&db, query_range(start, end), "ofox.ai", end, &zone).unwrap();
        assert_reconciles(&day, 4 * MINUTE as u64, 4);
        assert_eq!(day.series.len(), 1);
        assert_eq!(day.series[0].date, date);
        assert_eq!(
            day.series[0].end_ms - day.series[0].start_ms,
            day_hours * HOUR
        );
        assert_eq!(active_cell(&day, 6, 0), MINUTE as u64);
        assert_eq!(active_cell(&day, 6, 23), MINUTE as u64);
        if before_hour == after_hour {
            assert_eq!(active_cell(&day, 6, 1), 2 * MINUTE as u64);
        } else {
            assert_eq!(active_cell(&day, 6, before_hour), MINUTE as u64);
            assert_eq!(active_cell(&day, 6, after_hour), MINUTE as u64);
            assert_eq!(
                active_cell(&day, 6, 2),
                0,
                "the skipped local hour has no activity"
            );
        }

        // Both gaps cross a midnight and both query boundaries cut a gap.
        let expanded = query(
            &db,
            query_range(start - MINUTE / 2, end + MINUTE / 2),
            "ofox.ai",
            end,
            &zone,
        )
        .unwrap();
        assert_reconciles(&expanded, 5 * MINUTE as u64, 4);
        assert_eq!(expanded.series.len(), 3);
        assert_eq!(
            expanded
                .series
                .iter()
                .map(|day| day.known_active_ms.unwrap())
                .collect::<Vec<_>>(),
            [30_000, 240_000, 30_000],
        );
        assert_eq!(active_cell(&expanded, 5, 23), 30_000);
        assert_eq!(active_cell(&expanded, 0, 0), 30_000);

        // Neither endpoint message is in this window; the verified interval
        // still establishes a cohort and is clipped around the DST transition.
        let clipped = query(
            &db,
            query_range(transition - MINUTE / 2, transition + MINUTE / 2),
            "ofox.ai",
            end,
            &zone,
        )
        .unwrap();
        assert_reconciles(&clipped, MINUTE as u64, 0);
        assert_eq!(clipped.totals.session_count, 1);
        assert_eq!(
            clipped.series[0].end_ms - clipped.series[0].start_ms,
            day_hours * HOUR
        );
        if before_hour == after_hour {
            assert_eq!(active_cell(&clipped, 6, 1), MINUTE as u64);
        } else {
            assert_eq!(active_cell(&clipped, 6, before_hour), 30_000);
            assert_eq!(active_cell(&clipped, 6, after_hour), 30_000);
        }
    }
}
