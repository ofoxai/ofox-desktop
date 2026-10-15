//! Pure aggregation of normalized facts. File parsing and database writes do
//! not belong here; a caller must supply one coherent, deduplicated snapshot.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const UNKNOWN_PROJECT_ID: &str = "__unknown_project__";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub start_ms: Option<i64>,
    pub end_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    pub window: Window,
    pub tool_ids: Vec<String>,
    pub model_ids: Vec<String>,
    pub project_ids: Vec<String>,
    pub source_ids: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Project {
    Known(String),
    Unknown,
    /// This reader has not established project identity. It is not evidence
    /// that the original session had no working directory.
    Unparsed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionIdentity {
    pub key: String,
    pub tool_id: String,
    pub project: Project,
    /// Set only by a reader with a verified native parent relation.
    pub verified_root_key: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UsageFact {
    pub event_key: String,
    pub session_key: Option<String>,
    pub tool_id: String,
    pub source_id: String,
    pub model: String,
    pub at_ms: i64,
    /// New input, output, cache read, cache write.
    pub buckets: [u64; 4],
    pub complete: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionTotal {
    pub key: String,
    pub tool_id: String,
    pub project_id: Option<String>,
    pub buckets: [u64; 4],
    pub models: BTreeMap<String, [u64; 4]>,
    pub records: u64,
    pub complete: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageAggregate {
    pub sessions: BTreeMap<String, SessionTotal>,
    pub unassociated_buckets: [u64; 4],
    pub unassociated_records: u64,
    pub excluded_unparsed_project_records: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Quantiles {
    /// Exact base-10 text also preserves .5 and values beyond JS safe integers.
    pub median_tokens: Option<String>,
    pub p90_tokens: Option<u64>,
    pub samples: u64,
    pub excluded: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
    Thinking,
    System,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MessageEvent {
    pub session_key: String,
    pub native_id: String,
    pub at_ms: i64,
    pub role: Role,
    pub main: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DayBoundary {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    pub messages: u64,
    /// Sum of eligible gaps for each session; concurrent sessions add.
    pub active_ms: u64,
    pub active_days: u64,
    pub daily_active_ms: BTreeMap<String, u64>,
    pub daily_messages: BTreeMap<String, u64>,
    pub by_session: BTreeMap<String, u64>,
    pub by_session_messages: BTreeMap<String, u64>,
    pub daily_sessions: BTreeMap<String, BTreeSet<String>>,
}

pub fn quantiles(samples: impl IntoIterator<Item = Option<u64>>) -> Quantiles {
    let mut known = Vec::new();
    let mut excluded = 0;
    for value in samples {
        match value {
            Some(value) => known.push(value),
            None => excluded += 1,
        }
    }
    known.sort_unstable();
    let count = known.len();
    let median_tokens = match count {
        0 => None,
        n if n % 2 == 1 => Some(known[n / 2].to_string()),
        n => {
            let sum = u128::from(known[n / 2 - 1]) + u128::from(known[n / 2]);
            Some(if sum % 2 == 0 {
                (sum / 2).to_string()
            } else {
                format!("{}.5", sum / 2)
            })
        }
    };
    let p90_tokens = (count > 0).then(|| {
        let rank = (9 * count as u128).div_ceil(10) as usize;
        known[rank - 1]
    });
    Quantiles {
        median_tokens,
        p90_tokens,
        samples: count as u64,
        excluded,
    }
}

pub fn resolve_roots(
    sessions: &[SessionIdentity],
) -> Result<BTreeMap<&str, &SessionIdentity>, String> {
    let mut identities = BTreeMap::new();
    for session in sessions {
        if session.key.is_empty() || session.tool_id.is_empty() {
            return Err("invalidSessionIdentity".into());
        }
        if let Some(prior) = identities.insert(session.key.as_str(), session) {
            if prior != session {
                return Err("conflictingSessionIdentity".into());
            }
        }
    }
    let mut roots: BTreeMap<&str, &SessionIdentity> = BTreeMap::new();
    for session in identities.values() {
        let mut current = *session;
        let mut visited = std::collections::BTreeSet::new();
        let mut chain = Vec::new();
        while let Some(parent_key) = &current.verified_root_key {
            if let Some(root) = roots.get(current.key.as_str()) {
                current = root;
                break;
            }
            chain.push(current.key.as_str());
            if !visited.insert(current.key.as_str()) {
                return Err("cyclicSessionParent".into());
            }
            current = identities
                .get(parent_key.as_str())
                .copied()
                .ok_or("missingVerifiedSessionParent")?;
            if current.tool_id != session.tool_id {
                return Err("crossToolSessionParent".into());
            }
        }
        if current.tool_id != session.tool_id {
            return Err("crossToolSessionParent".into());
        }
        roots.insert(session.key.as_str(), current);
        for key in chain {
            roots.insert(key, current);
        }
    }
    Ok(roots)
}

pub fn aggregate_usage(
    facts: &[UsageFact],
    sessions: &[SessionIdentity],
    filters: &Filters,
) -> Result<UsageAggregate, String> {
    validate_window(&filters.window)?;
    let roots = resolve_roots(sessions)?;
    let mut seen = BTreeMap::new();
    let mut result = UsageAggregate::default();
    for fact in facts {
        if fact.event_key.is_empty() {
            return Err("missingUsageEventIdentity".into());
        }
        if let Some(prior) = seen.insert(fact.event_key.as_str(), fact) {
            if prior != fact {
                return Err("conflictingUsageEvent".into());
            }
            continue;
        }
        if !contains_time(&filters.window, fact.at_ms)
            || !selected(&filters.tool_ids, &fact.tool_id)
            || !selected(&filters.model_ids, &fact.model)
            || !selected(&filters.source_ids, &fact.source_id)
        {
            continue;
        }
        let root = fact
            .session_key
            .as_deref()
            .and_then(|key| roots.get(key))
            .copied();
        if root.is_some_and(|root| root.tool_id != fact.tool_id) {
            return Err("usageSessionToolMismatch".into());
        }
        let project_id = root.and_then(|root| match &root.project {
            Project::Known(id) => Some(id.as_str()),
            Project::Unknown => Some(UNKNOWN_PROJECT_ID),
            Project::Unparsed => None,
        });
        if !filters.project_ids.is_empty() {
            let Some(project_id) = project_id else {
                checked_add(&mut result.excluded_unparsed_project_records, 1)?;
                continue;
            };
            if !selected(&filters.project_ids, project_id) {
                continue;
            }
        }
        if let Some(root) = root {
            let total = result
                .sessions
                .entry(root.key.clone())
                .or_insert_with(|| SessionTotal {
                    key: root.key.clone(),
                    tool_id: root.tool_id.clone(),
                    project_id: project_id.map(str::to_owned),
                    complete: true,
                    ..Default::default()
                });
            add_buckets(&mut total.buckets, &fact.buckets)?;
            add_buckets(
                total.models.entry(fact.model.clone()).or_default(),
                &fact.buckets,
            )?;
            checked_add(&mut total.records, 1)?;
            total.complete &= fact.complete;
        } else {
            add_buckets(&mut result.unassociated_buckets, &fact.buckets)?;
            checked_add(&mut result.unassociated_records, 1)?;
        }
    }
    Ok(result)
}

pub fn message_activity(
    messages: &[MessageEvent],
    window: &Window,
    days: &[DayBoundary],
) -> Result<Activity, String> {
    validate_window(window)?;
    let mut dates = std::collections::BTreeSet::new();
    let mut ordered_days: Vec<_> = days.iter().collect();
    ordered_days.sort_unstable_by_key(|day| day.start_ms);
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
    let mut seen = BTreeMap::new();
    let mut timelines: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    let mut active_dates = std::collections::BTreeSet::new();
    let mut result = Activity::default();
    for message in messages {
        if message.native_id.is_empty() || message.session_key.is_empty() {
            return Err("missingMessageIdentity".into());
        }
        let key = (message.session_key.as_str(), message.native_id.as_str());
        if let Some(prior) = seen.insert(key, message) {
            if prior != message {
                return Err("conflictingMessageEvent".into());
            }
            continue;
        }
        if !message.main || !matches!(message.role, Role::User | Role::Assistant) {
            continue;
        }
        // Keep neighbours outside the window. A valid gap may cross either
        // query boundary even when neither endpoint is inside it.
        timelines
            .entry(&message.session_key)
            .or_default()
            .push(message.at_ms);
        if contains_time(window, message.at_ms) {
            let index = ordered_days.partition_point(|day| day.end_ms <= message.at_ms);
            let day = ordered_days
                .get(index)
                .filter(|day| day.start_ms <= message.at_ms)
                .ok_or("missingActivityDayBoundary")?;
            checked_add(&mut result.messages, 1)?;
            checked_add(
                result.daily_messages.entry(day.date.clone()).or_default(),
                1,
            )?;
            checked_add(
                result
                    .by_session_messages
                    .entry(message.session_key.clone())
                    .or_default(),
                1,
            )?;
            result
                .daily_sessions
                .entry(day.date.clone())
                .or_default()
                .insert(message.session_key.clone());
            active_dates.insert(day.date.as_str());
        }
    }
    for (session, mut times) in timelines {
        times.sort_unstable();
        for pair in times.windows(2) {
            let gap = i128::from(pair[1]) - i128::from(pair[0]);
            if !(1..300_000).contains(&gap) {
                continue;
            }
            let start = pair[0].max(window.start_ms.unwrap_or(i64::MIN));
            let end = pair[1].min(window.end_ms);
            if start >= end {
                continue;
            }
            let duration = (i128::from(end) - i128::from(start)) as u64;
            let mut covered = 0;
            let first_day = ordered_days.partition_point(|day| day.end_ms <= start);
            for day in ordered_days[first_day..]
                .iter()
                .take_while(|day| day.start_ms < end)
            {
                let segment_start = start.max(day.start_ms);
                let segment_end = end.min(day.end_ms);
                if segment_start < segment_end {
                    let segment = (i128::from(segment_end) - i128::from(segment_start)) as u64;
                    checked_add(&mut covered, segment)?;
                    checked_add(
                        result.daily_active_ms.entry(day.date.clone()).or_default(),
                        segment,
                    )?;
                    result
                        .daily_sessions
                        .entry(day.date.clone())
                        .or_default()
                        .insert(session.to_owned());
                    active_dates.insert(day.date.as_str());
                }
            }
            if covered != duration {
                return Err("incompleteActivityDayBoundaries".into());
            }
            checked_add(&mut result.active_ms, duration)?;
            checked_add(
                result.by_session.entry(session.to_owned()).or_default(),
                duration,
            )?;
        }
    }
    result.active_days = active_dates.len() as u64;
    Ok(result)
}

fn validate_window(window: &Window) -> Result<(), String> {
    if window.start_ms.is_some_and(|start| start > window.end_ms) {
        return Err("invalidAnalyticsWindow".into());
    }
    Ok(())
}

fn contains_time(window: &Window, time: i64) -> bool {
    time < window.end_ms && window.start_ms.is_none_or(|start| time >= start)
}

fn selected(ids: &[String], value: &str) -> bool {
    ids.is_empty() || ids.iter().any(|id| id == value)
}

fn checked_add(value: &mut u64, increment: u64) -> Result<(), String> {
    *value = value
        .checked_add(increment)
        .ok_or("analyticsCounterOverflow")?;
    Ok(())
}

fn add_buckets(total: &mut [u64; 4], increment: &[u64; 4]) -> Result<(), String> {
    for (total, increment) in total.iter_mut().zip(increment) {
        checked_add(total, *increment)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(key: &str, tool: &str, project: Project) -> SessionIdentity {
        SessionIdentity {
            key: key.into(),
            tool_id: tool.into(),
            project,
            verified_root_key: None,
        }
    }
    fn fact(id: &str, key: Option<&str>, buckets: [u64; 4]) -> UsageFact {
        UsageFact {
            event_key: id.into(),
            session_key: key.map(str::to_owned),
            tool_id: "codex".into(),
            source_id: "codex_session".into(),
            model: "model-a".into(),
            at_ms: 1000,
            buckets,
            complete: true,
        }
    }
    fn filters() -> Filters {
        Filters {
            window: Window {
                start_ms: Some(1000),
                end_ms: 2000,
            },
            ..Default::default()
        }
    }
    fn message(key: &str, id: &str, time: i64) -> MessageEvent {
        MessageEvent {
            session_key: key.into(),
            native_id: id.into(),
            at_ms: time,
            role: Role::User,
            main: true,
        }
    }
    fn day(date: &str, start: i64, end: i64) -> DayBoundary {
        DayBoundary {
            date: date.into(),
            start_ms: start,
            end_ms: end,
        }
    }

    #[test]
    fn analytics_activity_emits_daily_unique_sessions_and_per_session_message_counts_once() {
        let result = message_activity(
            &[
                message("one", "a", 10),
                message("one", "b", 70),
                message("two", "c", 30),
            ],
            &Window {
                start_ms: Some(0),
                end_ms: 100,
            },
            &[day("a", 0, 50), day("b", 50, 100)],
        )
        .unwrap();
        assert_eq!(result.by_session_messages.get("one"), Some(&2));
        assert_eq!(result.by_session_messages.get("two"), Some(&1));
        assert_eq!(result.daily_sessions.get("a").unwrap().len(), 2);
        assert_eq!(result.daily_sessions.get("b").unwrap().len(), 1);
        assert_eq!(
            result.daily_sessions["b"]
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            ["one"]
        );
    }
    #[test]
    fn analytics_quantiles_exclude_unknown_but_keep_known_zero_and_nearest_rank() {
        assert_eq!(
            quantiles([Some(0), Some(10), None, Some(20), Some(31)]),
            Quantiles {
                median_tokens: Some("15".into()),
                p90_tokens: Some(31),
                samples: 4,
                excluded: 1,
            }
        );
        assert_eq!(
            quantiles([Some(10), Some(11)]).median_tokens.as_deref(),
            Some("10.5")
        );
        assert_eq!(quantiles((1..=10).map(Some)).p90_tokens, Some(9));
    }
    #[test]
    fn analytics_quantile_median_does_not_overflow_or_round_large_values() {
        assert_eq!(
            quantiles([Some(u64::MAX - 1), Some(u64::MAX)])
                .median_tokens
                .as_deref(),
            Some("18446744073709551614.5")
        );
        assert_eq!(
            quantiles([None, None]),
            Quantiles {
                excluded: 2,
                ..Default::default()
            }
        );
    }
    #[test]
    fn analytics_window_is_half_open_and_filters_apply_before_session_distribution() {
        let sessions = [identity("s", "codex", Project::Known("p".into()))];
        let mut before = fact("before", Some("s"), [100, 10, 0, 0]);
        before.at_ms = 999;
        let mut edge = fact("edge", Some("s"), [100, 10, 0, 0]);
        edge.at_ms = 2000;
        let mut other = fact("other-model", Some("s"), [200, 20, 0, 0]);
        other.model = "model-b".into();
        let mut query = filters();
        query.model_ids.push("model-a".into());
        let result = aggregate_usage(
            &[
                before,
                edge,
                other,
                fact("selected", Some("s"), [4, 6, 100, 20]),
            ],
            &sessions,
            &query,
        )
        .unwrap();
        assert_eq!(result.sessions["s"].buckets, [4, 6, 100, 20]);
        assert_eq!(result.sessions["s"].records, 1);
        assert_eq!(result.sessions["s"].models.len(), 1);
    }
    #[test]
    fn analytics_verified_children_roll_into_root_project_but_unverified_sessions_stay_separate() {
        let root = identity("root", "codex", Project::Known("root-project".into()));
        let mut child = identity("child", "codex", Project::Known("child-project".into()));
        child.verified_root_key = Some("root".into());
        let independent = identity("unverified", "codex", Project::Known("other".into()));
        let result = aggregate_usage(
            &[
                fact("a", Some("root"), [1, 2, 0, 0]),
                fact("b", Some("child"), [3, 4, 0, 0]),
                fact("c", Some("unverified"), [5, 6, 0, 0]),
            ],
            &[root, child, independent],
            &filters(),
        )
        .unwrap();
        assert_eq!(result.sessions.len(), 2);
        assert_eq!(result.sessions["root"].buckets, [4, 6, 0, 0]);
        assert_eq!(
            result.sessions["root"].project_id.as_deref(),
            Some("root-project")
        );
        assert!(result.sessions.contains_key("unverified"));
    }
    #[test]
    fn analytics_unknown_project_does_not_absorb_unparsed_or_unassociated_records() {
        let sessions = [
            identity("unknown", "codex", Project::Unknown),
            identity("unparsed", "codex", Project::Unparsed),
        ];
        let facts = [
            fact("a", Some("unknown"), [1, 2, 0, 0]),
            fact("b", Some("unparsed"), [3, 4, 0, 0]),
            fact("c", None, [5, 6, 0, 0]),
        ];
        let all = aggregate_usage(&facts, &sessions, &filters()).unwrap();
        assert_eq!(all.unassociated_buckets, [5, 6, 0, 0]);
        assert_eq!(all.unassociated_records, 1);
        let mut query = filters();
        query.project_ids.push(UNKNOWN_PROJECT_ID.into());
        let filtered = aggregate_usage(&facts, &sessions, &query).unwrap();
        assert_eq!(filtered.sessions.len(), 1);
        assert_eq!(filtered.sessions["unknown"].buckets, [1, 2, 0, 0]);
        assert_eq!(filtered.excluded_unparsed_project_records, 2);
    }
    #[test]
    fn analytics_exact_event_replicas_are_counted_once_and_conflicts_are_explicit() {
        let sessions = [identity("s", "codex", Project::Unknown)];
        let a = fact("a", Some("s"), [1, 2, 0, 0]);
        let result = aggregate_usage(&[a.clone(), a.clone()], &sessions, &filters()).unwrap();
        assert_eq!(result.sessions["s"].records, 1);
        let mut conflict = a.clone();
        conflict.buckets[0] = 99;
        assert!(aggregate_usage(&[a, conflict], &sessions, &filters()).is_err());
    }
    #[test]
    fn analytics_tool_source_and_project_filters_do_not_merge_lookalike_sessions() {
        let sessions = [
            identity("codex:s", "codex", Project::Known("/one/project".into())),
            identity("claude:s", "claude", Project::Known("/two/project".into())),
        ];
        let a = fact("a", Some("codex:s"), [1, 2, 0, 0]);
        let mut b = fact("b", Some("claude:s"), [9, 10, 0, 0]);
        b.tool_id = "claude".into();
        b.source_id = "session_log".into();
        let mut query = filters();
        query.tool_ids.push("claude".into());
        query.source_ids.push("session_log".into());
        query.project_ids.push("/two/project".into());
        let result = aggregate_usage(&[a, b], &sessions, &query).unwrap();
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions["claude:s"].buckets, [9, 10, 0, 0]);
    }
    #[test]
    fn analytics_bucket_overflow_and_invalid_parent_relations_do_not_silently_wrap() {
        let mut s = identity("s", "codex", Project::Unknown);
        assert!(aggregate_usage(
            &[
                fact("a", Some("s"), [u64::MAX, 0, 0, 0]),
                fact("b", Some("s"), [1, 0, 0, 0])
            ],
            &[s.clone()],
            &filters()
        )
        .is_err());
        s.verified_root_key = Some("missing".into());
        assert!(aggregate_usage(&[fact("a", Some("s"), [1, 0, 0, 0])], &[s], &filters()).is_err());
    }
    #[test]
    fn analytics_main_message_activity_excludes_system_tools_thinking_and_children() {
        let mut messages = vec![
            message("s", "a", 0),
            message("s", "b", 60_000),
            message("s", "b", 60_000),
        ];
        for (index, role) in [Role::System, Role::Tool, Role::Thinking]
            .into_iter()
            .enumerate()
        {
            let mut m = message("s", &format!("extra{index}"), 30_000);
            m.role = role;
            messages.push(m);
        }
        let mut child = message("child", "child", 45_000);
        child.main = false;
        messages.push(child);
        let result = message_activity(
            &messages,
            &Window {
                start_ms: Some(0),
                end_ms: 120_000,
            },
            &[day("d", 0, 120_000)],
        )
        .unwrap();
        assert_eq!(
            (result.messages, result.active_ms, result.active_days),
            (2, 60_000, 1)
        );
    }
    #[test]
    fn analytics_activity_uses_strict_five_minute_gaps_and_adds_overlapping_sessions() {
        let messages = [
            message("a", "a0", 0),
            message("a", "a1", 299_999),
            message("a", "a2", 599_999),
            message("b", "b0", 0),
            message("b", "b1", 299_999),
        ];
        let result = message_activity(
            &messages,
            &Window {
                start_ms: Some(0),
                end_ms: 700_000,
            },
            &[day("d", 0, 700_000)],
        )
        .unwrap();
        assert_eq!(result.active_ms, 599_998);
        assert_eq!(result.messages, 5);
        assert_eq!(result.by_session["a"], 299_999);
    }
    #[test]
    fn analytics_activity_clips_cross_midnight_and_preserves_boundary_neighbours() {
        let result = message_activity(
            &[
                message("s", "before", 80_000),
                message("s", "after", 140_000),
            ],
            &Window {
                start_ms: Some(90_000),
                end_ms: 130_000,
            },
            &[day("first", 0, 100_000), day("second", 100_000, 200_000)],
        )
        .unwrap();
        assert_eq!(result.messages, 0);
        assert_eq!(result.active_ms, 40_000);
        assert_eq!(result.active_days, 2);
        assert_eq!(result.daily_active_ms["first"], 10_000);
        assert_eq!(result.daily_active_ms["second"], 30_000);
    }
    #[test]
    fn analytics_activity_respects_supplied_23_and_25_hour_natural_days() {
        let short = 23 * 3_600_000;
        let long = 25 * 3_600_000;
        let messages = [
            message("s", "a", short - 30_000),
            message("s", "b", short + 30_000),
            message("s", "c", short + long - 30_000),
            message("s", "d", short + long + 30_000),
        ];
        let result = message_activity(
            &messages,
            &Window {
                start_ms: Some(0),
                end_ms: short + long + 60_000,
            },
            &[
                day("23h", 0, short),
                day("25h", short, short + long),
                day("next", short + long, short + long + 60_000),
            ],
        )
        .unwrap();
        assert_eq!(result.active_ms, 120_000);
        assert_eq!(result.daily_active_ms["23h"], 30_000);
        assert_eq!(result.daily_active_ms["25h"], 60_000);
        assert_eq!(result.daily_active_ms["next"], 30_000);
    }
    #[test]
    fn analytics_activity_rejects_conflicting_identity_and_overlapping_day_buckets() {
        let window = Window {
            start_ms: Some(0),
            end_ms: 100_000,
        };
        assert!(message_activity(
            &[message("s", "same", 1), message("s", "same", 2)],
            &window,
            &[day("d", 0, 100_000)]
        )
        .is_err());
        assert!(message_activity(
            &[message("s", "a", 0), message("s", "b", 60_000)],
            &window,
            &[day("a", 0, 60_000), day("b", 30_000, 100_000)]
        )
        .is_err());
    }

    #[test]
    fn analytics_empty_half_open_window_is_valid_and_has_no_events_or_activity() {
        let window = Window {
            start_ms: Some(1000),
            end_ms: 1000,
        };
        let result = aggregate_usage(
            &[fact("a", Some("s"), [1, 2, 0, 0])],
            &[identity("s", "codex", Project::Unknown)],
            &Filters {
                window: window.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result, UsageAggregate::default());
        let result = message_activity(
            &[message("s", "a", 999), message("s", "b", 1001)],
            &window,
            &[],
        )
        .unwrap();
        assert_eq!(result, Activity::default());
    }

    #[test]
    fn analytics_parent_cycles_cross_tool_and_fact_tool_mismatch_are_rejected() {
        let mut a = identity("a", "codex", Project::Unknown);
        let mut b = identity("b", "codex", Project::Unknown);
        a.verified_root_key = Some("b".into());
        b.verified_root_key = Some("a".into());
        assert!(aggregate_usage(&[], &[a.clone(), b.clone()], &filters()).is_err());
        b.verified_root_key = None;
        b.tool_id = "claude".into();
        assert!(aggregate_usage(&[], &[a, b], &filters()).is_err());
        assert!(aggregate_usage(
            &[fact("wrong", Some("a"), [1, 0, 0, 0])],
            &[identity("a", "claude", Project::Unknown)],
            &filters(),
        )
        .is_err());
    }

    #[test]
    fn analytics_activity_missing_day_coverage_does_not_lose_time_silently() {
        let window = Window {
            start_ms: Some(0),
            end_ms: 100_000,
        };
        assert!(message_activity(
            &[message("s", "a", 0), message("s", "b", 60_000)],
            &window,
            &[day("a", 0, 10_000), day("b", 50_000, 100_000)],
        )
        .is_err());
        assert!(message_activity(&[message("s", "a", 0)], &window, &[]).is_err());
    }
}
