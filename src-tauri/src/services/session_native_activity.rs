//! Body-free native-runtime activity. Callers supply one verified source set
//! in each source's original event order, rather than sorting wall-clock time.
//!
//! The high-water-mark and later-event allocation follow Magpie's `state.saw`
//! at bfaedf18301d02a423bd5422d22e41432d231ab8 (MIT, Copyright 2026 yetone).
//! The permission notice is retained in `../session_manager/MAGPIE_LICENSE`.
use super::session_analytics_core::{Activity, DayBoundary, Window};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeActivityEvent {
    pub session_key: String,
    pub event_key: String,
    pub at_ms: i64,
    pub main: bool,
    /// A distinct native source stream. Empty preserves older cache readers.
    #[serde(default)]
    pub stream_key: String,
}

pub fn activity(
    events: &[NativeActivityEvent],
    window: &Window,
    days: &[DayBoundary],
) -> Result<Activity, String> {
    if window.start_ms.is_some_and(|start| start > window.end_ms) {
        return Err("invalidAnalyticsWindow".into());
    }
    let mut ordered_days: Vec<_> = days.iter().collect();
    ordered_days.sort_unstable_by_key(|day| day.start_ms);
    let mut dates = BTreeSet::new();
    let mut previous_end = None;
    for day in &ordered_days {
        if day.date.is_empty()
            || day.start_ms >= day.end_ms
            || !dates.insert(day.date.as_str())
            || previous_end.is_some_and(|end| end > day.start_ms)
        {
            return Err("invalidActivityDayBoundaries".into());
        }
        previous_end = Some(day.end_ms);
    }
    let mut seen = HashMap::new();
    let mut highmarks: HashMap<(&str, &str), i64> = HashMap::new();
    let mut active_dates = BTreeSet::new();
    let mut result = Activity::default();
    for event in events {
        if event.event_key.is_empty() || event.session_key.is_empty() {
            return Err("missingNativeActivityIdentity".into());
        }
        let duplicate = match seen.insert(
            (event.session_key.as_str(), event.event_key.as_str()),
            (event.at_ms, event.main),
        ) {
            Some(prior) if prior != (event.at_ms, event.main) => {
                return Err("conflictingNativeActivityEvent".into());
            }
            Some(_) => true,
            None => false,
        };
        if !event.main {
            continue;
        }
        // A copied prefix can establish a continuation stream's baseline, but
        // cannot charge an already-seen event again. Clock rollback never
        // lowers this source stream's high-water mark.
        let prior = match highmarks.entry((event.session_key.as_str(), event.stream_key.as_str())) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let prior = *entry.get();
                if event.at_ms > prior {
                    entry.insert(event.at_ms);
                    Some(prior)
                } else {
                    None
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(event.at_ms);
                None
            }
        };
        if duplicate
            || event.at_ms >= window.end_ms
            || window.start_ms.is_some_and(|start| event.at_ms < start)
        {
            continue;
        }
        let day_index = ordered_days.partition_point(|day| day.end_ms <= event.at_ms);
        let day = ordered_days
            .get(day_index)
            .filter(|day| day.start_ms <= event.at_ms)
            .ok_or("missingActivityDayBoundary")?;
        let session_amount = result
            .by_session
            .entry(event.session_key.clone())
            .or_default();
        let Some(prior) = prior else {
            continue;
        };
        let gap = i128::from(event.at_ms) - i128::from(prior);
        if !(1..300_000).contains(&gap) {
            continue;
        }
        result
            .daily_sessions
            .entry(day.date.clone())
            .or_default()
            .insert(event.session_key.clone());
        active_dates.insert(day.date.as_str());
        let amount = gap as u64;
        checked_add(&mut result.active_ms, amount)?;
        checked_add(session_amount, amount)?;
        checked_add(
            result.daily_active_ms.entry(day.date.clone()).or_default(),
            amount,
        )?;
        result.native_allocations.push((event.at_ms, amount));
    }
    result.active_days = active_dates.len() as u64;
    Ok(result)
}

fn checked_add(value: &mut u64, amount: u64) -> Result<(), String> {
    *value = value
        .checked_add(amount)
        .ok_or("analyticsCounterOverflow")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn event(session: &str, id: &str, at: i64) -> NativeActivityEvent {
        NativeActivityEvent {
            session_key: session.into(),
            event_key: id.into(),
            at_ms: at,
            main: true,
            stream_key: "native-file".into(),
        }
    }

    fn day(date: &str, start_ms: i64, end_ms: i64) -> DayBoundary {
        DayBoundary {
            date: date.into(),
            start_ms,
            end_ms,
        }
    }

    fn window(end_ms: i64) -> Window {
        Window {
            start_ms: Some(0),
            end_ms,
        }
    }

    #[test]
    fn native_activity_counts_runtime_events_once_without_manufacturing_messages() {
        let result = activity(
            &[
                event("thread", "dialogue", 0),
                event("thread", "runtime", 40_000),
                event("thread", "usage", 70_000),
                event("thread", "usage", 70_000),
            ],
            &window(100_000),
            &[day("date", 0, 100_000)],
        )
        .unwrap();
        assert_eq!(result.active_ms, 70_000);
        assert_eq!(result.by_session["thread"], 70_000);
        assert_eq!(result.messages, 0);
        assert!(result.daily_messages.is_empty());
        assert!(result.by_session_messages.is_empty());
        assert_eq!(
            result.native_allocations,
            [(40_000, 40_000), (70_000, 30_000)]
        );
        assert!(result.intervals.is_empty());
    }

    #[test]
    fn native_activity_has_strict_idle_limit_and_adds_only_main_concurrent_streams() {
        let mut child_first = event("a", "child-first", 150_000);
        child_first.main = false;
        let mut child_last = event("a", "child-last", 250_000);
        child_last.main = false;
        let result = activity(
            &[
                event("a", "a-first", 0),
                child_first,
                child_last,
                event("a", "a-last", 299_999),
                event("a", "a-idle", 599_999),
                event("b", "b-first", 0),
                event("b", "b-last", 299_999),
            ],
            &window(700_000),
            &[day("date", 0, 700_000)],
        )
        .unwrap();
        assert_eq!(result.active_ms, 599_998);
        assert_eq!(result.by_session["a"], 299_999);
        assert_eq!(result.by_session["b"], 299_999);
    }

    #[test]
    fn native_activity_keeps_source_order_highmark_across_equal_or_reversed_clocks() {
        let result = activity(
            &[
                event("thread", "first", 0),
                event("thread", "after-idle", 400_000),
                event("thread", "clock-reversed", 100_000),
                event("thread", "same-time", 400_000),
                event("thread", "last", 450_000),
            ],
            &window(500_000),
            &[day("date", 0, 500_000)],
        )
        .unwrap();
        assert_eq!(result.active_ms, 50_000);
        assert_eq!(result.native_allocations, [(450_000, 50_000)]);
    }

    #[test]
    fn native_activity_never_invents_a_gap_between_independent_same_thread_segments() {
        let mut next_stream = event("thread", "continuation", 90_000);
        next_stream.stream_key = "second-file".into();
        let mut last_stream = event("thread", "continuation-last", 110_000);
        last_stream.stream_key = "second-file".into();
        let result = activity(
            &[
                event("thread", "first", 0),
                event("thread", "last", 30_000),
                next_stream,
                last_stream,
            ],
            &window(120_000),
            &[day("date", 0, 120_000)],
        )
        .unwrap();
        assert_eq!(result.active_ms, 50_000);
        assert_eq!(result.by_session["thread"], 50_000);
    }

    #[test]
    fn native_activity_repeated_prefix_keeps_continuation_baseline_without_recounting() {
        let first = event("thread", "first", 0);
        let second = event("thread", "second", 50_000);
        let mut copied_first = first.clone();
        copied_first.stream_key = "continued-file".into();
        let mut copied_second = second.clone();
        copied_second.stream_key = "continued-file".into();
        let mut next = event("thread", "next", 110_000);
        next.stream_key = "continued-file".into();
        let result = activity(
            &[first, second, copied_first, copied_second, next],
            &window(120_000),
            &[day("date", 0, 120_000)],
        )
        .unwrap();
        assert_eq!(result.active_ms, 110_000);
        assert_eq!(
            result.native_allocations,
            [(50_000, 50_000), (110_000, 60_000)]
        );
    }

    #[test]
    fn native_activity_assigns_whole_cross_midnight_gap_to_later_event_and_window() {
        let events = [
            event("thread", "before", 80_000),
            event("thread", "after", 140_000),
            event("thread", "end-excluded", 200_000),
        ];
        let days = [day("first", 0, 100_000), day("second", 100_000, 200_000)];
        let result = activity(
            &events,
            &Window {
                start_ms: Some(140_000),
                end_ms: 200_000,
            },
            &days,
        )
        .unwrap();
        assert_eq!(result.active_ms, 60_000);
        assert_eq!(result.active_days, 1);
        assert_eq!(result.daily_active_ms["second"], 60_000);
        assert!(!result.daily_active_ms.contains_key("first"));
        assert_eq!(result.native_allocations, [(140_000, 60_000)]);
        assert_eq!(
            result.daily_sessions["second"],
            BTreeSet::from(["thread".into()])
        );

        let before_endpoint = activity(
            &events,
            &Window {
                start_ms: Some(90_000),
                end_ms: 130_000,
            },
            &days,
        )
        .unwrap();
        assert_eq!(before_endpoint.active_ms, 0);
        assert!(before_endpoint.native_allocations.is_empty());
    }

    #[test]
    fn native_activity_accepts_short_and_long_dst_days_without_splitting_gaps() {
        for day_length in [23 * 3_600_000, 25 * 3_600_000] {
            let result = activity(
                &[
                    event("thread", "before", day_length - 60_000),
                    event("thread", "after", day_length + 60_000),
                ],
                &window(day_length + 3_600_000),
                &[
                    day("transition", 0, day_length),
                    day("following", day_length, day_length + 3_600_000),
                ],
            )
            .unwrap();
            assert_eq!(result.active_ms, 120_000);
            assert_eq!(result.daily_active_ms["following"], 120_000);
            assert!(!result.daily_active_ms.contains_key("transition"));
        }
    }

    #[test]
    fn native_activity_distinguishes_observed_zero_from_no_source_evidence() {
        let days = [day("date", 0, 100)];
        assert_eq!(
            activity(&[], &window(100), &days).unwrap(),
            Activity::default()
        );
        let mut child = event("child", "only", 50);
        child.main = false;
        assert_eq!(
            activity(&[child], &window(100), &days).unwrap(),
            Activity::default()
        );
        let one = activity(&[event("thread", "only", 50)], &window(100), &days).unwrap();
        assert_eq!(one.by_session["thread"], 0);
        assert_eq!(one.active_ms, 0);
        assert_eq!(one.active_days, 0);
        assert!(one.daily_sessions.is_empty());
    }

    #[test]
    fn native_activity_metadata_only_events_do_not_create_active_days() {
        let result = activity(
            &[
                event("s", "header", 1_000),
                event("s", "first-runtime", 901_000),
                event("s", "second-runtime", 1_001_000),
            ],
            &window(2_000_000),
            &[
                day("metadata-only", 0, 500_000),
                day("active", 500_000, 2_000_000),
            ],
        )
        .unwrap();
        assert_eq!(result.active_days, 1);
        assert!(!result.daily_sessions.contains_key("metadata-only"));
        assert_eq!(
            result.daily_sessions["active"],
            BTreeSet::from(["s".into()])
        );
    }

    #[test]
    fn native_activity_rejects_conflicting_identity_and_invalid_calendar_evidence() {
        let original = event("thread", "same", 10);
        let changed = event("thread", "same", 20);
        assert!(activity(&[original, changed], &window(100), &[day("date", 0, 100)]).is_err());
        assert!(activity(&[event("", "id", 10)], &window(100), &[day("date", 0, 100)]).is_err());
        assert!(activity(
            &[event("thread", "", 10)],
            &window(100),
            &[day("date", 0, 100)]
        )
        .is_err());
        assert!(activity(&[event("thread", "id", 10)], &window(100), &[]).is_err());
        assert!(activity(
            &[],
            &window(100),
            &[day("date", 0, 50), day("date", 50, 100)]
        )
        .is_err());
        assert!(activity(
            &[],
            &Window {
                start_ms: Some(100),
                end_ms: 0
            },
            &[]
        )
        .is_err());
    }
}
