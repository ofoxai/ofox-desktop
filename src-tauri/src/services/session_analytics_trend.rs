//! A single natural day's chart, without changing daily cohort/KPI semantics.
use super::{add, invalid, AnalyticsDay, CostContribution, Sum};
use crate::{
    error::AppError,
    services::{
        session_analytics_core::{
            self as core, Activity, DayBoundary, MessageEvent, Role, UsageFact, Window,
        },
        session_native_activity::{self, NativeActivityEvent},
        usage_overview::Granularity,
        usage_query::UsageQueryV1,
        usage_time::local_day_start_ms,
    },
};
use chrono::{DateTime, Offset, TimeZone, Utc};
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsTrend {
    pub granularity: Granularity,
    pub series: Vec<AnalyticsDay>,
}

pub(super) struct HourlyTrend {
    bounds: Vec<DayBoundary>,
    sums: Vec<Sum>,
    sessions: Vec<BTreeSet<String>>,
}

pub(super) struct Metrics<'a> {
    pub window: &'a Window,
    pub messages: &'a [&'a MessageEvent],
    pub runtime_events: &'a [NativeActivityEvent],
    pub native_mode: bool,
    pub cohort: &'a BTreeSet<String>,
    pub message_eligible: &'a BTreeSet<String>,
    pub activity: &'a Activity,
    pub known_activity: &'a Activity,
    pub messages_available: bool,
    pub activity_available: bool,
    pub known_messages: bool,
    pub known_active: bool,
}

fn utc(at: i64) -> Result<DateTime<Utc>, AppError> {
    DateTime::from_timestamp_millis(at).ok_or_else(|| invalid("invalidHourTime"))
}

fn offset<Tz: TimeZone>(at: i64, tz: &Tz) -> Result<i32, AppError> {
    Ok(utc(at)?.with_timezone(tz).offset().fix().local_minus_utc())
}

/// Walk chronological instants, retaining both occurrences of a folded hour.
/// Offset transitions can occur between wall-clock hours (e.g. Lord Howe).
fn next_hour<Tz: TimeZone>(at: i64, end: i64, tz: &Tz) -> Result<i64, AppError> {
    let current_offset = offset(at, tz)?;
    let wall_ms = i128::from(at) + i128::from(current_offset) * 1_000;
    let remaining = 3_600_000 - wall_ms.rem_euclid(3_600_000);
    let mut next = i64::try_from((i128::from(at) + remaining).min(i128::from(end)))
        .map_err(|_| invalid("invalidHourTime"))?;
    if offset(next - 1, tz)? != current_offset {
        let mut lo = at;
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
    Ok(next)
}

impl HourlyTrend {
    pub(super) fn new<Tz: TimeZone>(
        query: &UsageQueryV1,
        days: &[DayBoundary],
        tz: &Tz,
    ) -> Result<Option<Self>, AppError> {
        let Some(start) = query.start_ms else {
            return Ok(None);
        };
        if days.len() > 1 {
            return Ok(None);
        }
        let first_date = utc(start)?.with_timezone(tz).date_naive();
        if start < query.end_ms
            && utc(query.end_ms - 1)?.with_timezone(tz).date_naive() != first_date
        {
            return Ok(None);
        }
        let mut at = local_day_start_ms(tz, first_date)?;
        let mut bounds = Vec::new();
        while at < query.end_ms {
            let end_ms = next_hour(at, query.end_ms, tz)?;
            if end_ms <= at || bounds.len() >= 128 {
                return Err(invalid("invalidHourBoundaries"));
            }
            if end_ms > start {
                bounds.push(DayBoundary {
                    // A unique key is necessary for repeated local hours and
                    // the existing body-free message/runtime projections.
                    date: utc(at)?.with_timezone(tz).to_rfc3339(),
                    start_ms: at,
                    end_ms,
                });
            }
            at = end_ms;
        }
        Ok(Some(Self {
            sums: bounds.iter().map(|_| Sum::default()).collect(),
            sessions: bounds.iter().map(|_| BTreeSet::new()).collect(),
            bounds,
        }))
    }

    fn index(&self, at: i64) -> Result<usize, AppError> {
        let index = self.bounds.partition_point(|bound| bound.end_ms <= at);
        self.bounds
            .get(index)
            .filter(|bound| bound.start_ms <= at)
            .map(|_| index)
            .ok_or_else(|| invalid("missingUsageHourBoundary"))
    }

    pub(super) fn add_fact(
        &mut self,
        fact: &UsageFact,
        root: &str,
        estimate: &CostContribution<'_>,
    ) -> Result<(), AppError> {
        let index = self.index(fact.at_ms)?;
        self.sums[index].add_prepared(fact, estimate)?;
        if !self.sessions[index].contains(root) {
            self.sessions[index].insert(root.to_owned());
        }
        Ok(())
    }

    fn activity_amounts(&self, activity: &Activity) -> Result<Vec<u64>, AppError> {
        let mut values = vec![0; self.bounds.len()];
        // Native runtime belongs wholly to the later event, including a gap
        // crossing the query start, midnight or an offset transition.
        for &(at, amount) in &activity.native_allocations {
            add(&mut values[self.index(at)?], amount)?;
        }
        // Older cached message activity preserves its verified clipped gaps.
        for &(start, end) in &activity.intervals {
            for (index, bound) in self.bounds.iter().enumerate() {
                let from = start.max(bound.start_ms);
                let to = end.min(bound.end_ms);
                if from < to {
                    add(&mut values[index], (to - from) as u64)?;
                }
            }
        }
        Ok(values)
    }

    pub(super) fn finish(mut self, metrics: Metrics<'_>) -> Result<AnalyticsTrend, AppError> {
        let mut messages = vec![0; self.bounds.len()];
        let mut known_messages = vec![0; self.bounds.len()];
        let mut seen = HashSet::new();
        for message in metrics.messages {
            if !message.main
                || !matches!(message.role, Role::User | Role::Assistant)
                || !super::in_window(metrics.window, message.at_ms)
                || !metrics.cohort.contains(&message.session_key)
                || !seen.insert((message.session_key.as_str(), message.native_id.as_str()))
            {
                continue;
            }
            // The original core projection already validated conflicting
            // identities before this chart is built.
            let index = self.index(message.at_ms)?;
            add(&mut messages[index], 1)?;
            if !self.sessions[index].contains(&message.session_key) {
                self.sessions[index].insert(message.session_key.clone());
            }
            if metrics.message_eligible.contains(&message.session_key) {
                add(&mut known_messages[index], 1)?;
            }
        }
        let session_activity = if metrics.native_mode {
            session_native_activity::activity(
                &metrics
                    .runtime_events
                    .iter()
                    .filter(|event| metrics.cohort.contains(&event.session_key))
                    .cloned()
                    .collect::<Vec<_>>(),
                metrics.window,
                &self.bounds,
            )
        } else {
            core::message_activity_refs(
                metrics.messages.iter().copied(),
                metrics.window,
                &self.bounds,
            )
        }
        .map_err(|code| invalid(&code))?;
        // Runtime/message-only sessions contribute to the same hourly cohort
        // as facts, while one session spanning hours is counted once per hour.
        let active = self.activity_amounts(metrics.activity)?;
        let known_active = self.activity_amounts(metrics.known_activity)?;
        let mut series = Vec::with_capacity(self.bounds.len());
        for (index, bound) in self.bounds.iter().enumerate() {
            let mut sessions = self.sessions[index].clone();
            if let Some(keys) = session_activity.daily_sessions.get(&bound.date) {
                sessions.extend(keys.iter().cloned());
            }
            series.push(AnalyticsDay {
                date: bound.date.clone(),
                start_ms: bound.start_ms,
                end_ms: bound.end_ms,
                session_count: sessions.len() as u64,
                tokens: self.sums[index].tokens()?,
                estimate: self.sums[index].estimate(),
                messages: metrics.messages_available.then_some(messages[index]),
                active_ms: metrics.activity_available.then_some(active[index]),
                known_messages: metrics.known_messages.then_some(known_messages[index]),
                known_active_ms: metrics.known_active.then_some(known_active[index]),
            });
        }
        Ok(AnalyticsTrend {
            granularity: Granularity::Hour,
            series,
        })
    }
}
