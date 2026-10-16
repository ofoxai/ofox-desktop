//! Read-only session/project analysis over one cached, body-free snapshot.
pub use super::usage_projects::ProjectOption;
use super::{
    ofox_pricing::{CostEstimate, PriceCatalog, PricingCoverage},
    session_analytics_core::{DayBoundary, MessageEvent, SessionIdentity, UsageFact},
    usage_data::FilterOptions,
    usage_query::{Coverage, QueryReceipt, SourceHealth, UsageQueryV1},
};
use crate::error::AppError;
use serde::{Deserialize, Serialize};

#[path = "session_analytics_advanced.rs"]
pub mod advanced;
use advanced::{CompactTop, Distribution, HourActivity, KnownMetrics, MetricCoverage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDescriptor {
    pub identity: SessionIdentity,
    pub native_session_id: String,
    pub title: String,
    pub project_label: Option<String>,
    pub project_path: Option<String>,
    pub manager_key: Option<String>,
    pub source_available: bool,
    pub messages_supported: bool,
    pub message_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceReason {
    pub tool_id: String,
    pub source_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsInput {
    pub snapshot_version: String,
    pub sessions: Vec<SessionDescriptor>,
    pub facts: Vec<UsageFact>,
    pub messages: Vec<MessageEvent>,
    pub reasons: Vec<String>,
    pub source_reasons: Vec<SourceReason>,
    pub sources: Vec<SourceHealth>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cache_hit_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionKpis {
    #[serde(flatten)]
    pub tokens: TokenTotals,
    pub session_count: u64,
    pub project_count: u64,
    pub median_tokens: Option<String>,
    pub p90_tokens: Option<u64>,
    pub quantile_samples: u64,
    pub quantile_excluded: u64,
    pub estimate: CostEstimate,
    pub messages: Option<u64>,
    pub active_ms: Option<u64>,
    pub active_days: Option<u64>,
    pub message_reason: Option<String>,
    pub activity_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsDay {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub session_count: u64,
    #[serde(flatten)]
    pub tokens: TokenTotals,
    pub estimate: CostEstimate,
    pub messages: Option<u64>,
    pub active_ms: Option<u64>,
    pub known_messages: Option<u64>,
    pub known_active_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsRank {
    pub id: String,
    pub label: String,
    pub path: Option<String>,
    pub session_count: u64,
    #[serde(flatten)]
    pub tokens: TokenTotals,
    pub estimate: CostEstimate,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsModel {
    pub model: String,
    #[serde(flatten)]
    pub tokens: TokenTotals,
    pub estimate: CostEstimate,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsSession {
    pub session_key: String,
    pub native_session_id: String,
    pub tool_id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub project_label: Option<String>,
    pub project_path: Option<String>,
    pub manager_key: Option<String>,
    pub source_available: bool,
    pub tokens: Option<TokenTotals>,
    pub estimate: CostEstimate,
    pub models: Vec<AnalyticsModel>,
    pub messages: Option<u64>,
    pub active_ms: Option<u64>,
    pub reasons: Vec<String>,
}

pub type SessionOptions = FilterOptions;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnassociatedUsage {
    pub records: u64,
    #[serde(flatten)]
    pub tokens: TokenTotals,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAnalytics {
    #[serde(flatten)]
    pub receipt: QueryReceipt,
    pub coverage: Coverage,
    pub pricing_coverage: PricingCoverage,
    pub totals: SessionKpis,
    pub series: Vec<AnalyticsDay>,
    pub by_project: Vec<AnalyticsRank>,
    pub by_model: Vec<AnalyticsRank>,
    pub by_tool: Vec<AnalyticsRank>,
    /// Full detail is embedded in each of the bounded Top rows, so opening a
    /// dialog cannot accidentally combine a fresh query with an older total.
    pub top_sessions: Vec<AnalyticsSession>,
    pub top_limit: u32,
    pub options: SessionOptions,
    pub unassociated: UnassociatedUsage,
    pub sources: Vec<SourceHealth>,
    pub compact_top: CompactTop,
    pub known_metrics: Option<KnownMetrics>,
    pub metric_coverage: MetricCoverage,
    pub distributions: Vec<Distribution>,
    pub activity_by_hour: HourActivity,
    pub analytics_snapshot_id: Option<String>,
    pub paging_unavailable_reason: Option<String>,
    /// Owned by the IPC snapshot, excluded from the renderer response.
    #[serde(skip)]
    pub(crate) cohort_rows: Vec<AnalyticsSession>,
}

const TOP_LIMIT: usize = 100;

fn invalid(code: &str) -> AppError {
    AppError::InvalidInput(format!("usage.analytics.{code}"))
}
fn add(a: &mut u64, b: u64) -> Result<(), AppError> {
    *a = a.checked_add(b).ok_or_else(|| invalid("counterOverflow"))?;
    Ok(())
}
fn selected(ids: &[String], value: &str) -> bool {
    ids.is_empty() || ids.iter().any(|id| id == value)
}
fn incomplete_message_reason(reason: &str) -> bool {
    matches!(
        reason,
        "messageTimeUnavailable"
            | "messageNativeIdentityUnavailable"
            | "missingNativeEventIdentity"
            | "messagePartsUnavailable"
            | "conflictingMessageReplica"
            | "conflictingSessionMetadata"
            | "sessionIdentityUnavailable"
            | "sessionRoleNotRecorded"
            | "sessionParentUnavailable"
            | "invalidSessionRoot"
            | "incompleteSource"
    )
}
fn in_window(window: &super::session_analytics_core::Window, time: i64) -> bool {
    time < window.end_ms && window.start_ms.is_none_or(|start| time >= start)
}
fn project_id(project: &super::session_analytics_core::Project) -> Option<&str> {
    use super::session_analytics_core::{Project, UNKNOWN_PROJECT_ID};
    match project {
        Project::Known(id) => Some(id),
        Project::Unknown => Some(UNKNOWN_PROJECT_ID),
        Project::Unparsed => None,
    }
}

#[derive(Default)]
struct Sum {
    buckets: [u64; 4],
    records: u64,
    estimate: CostEstimate,
}
impl Sum {
    fn add(&mut self, fact: &UsageFact, catalog: &PriceCatalog) -> Result<(), AppError> {
        for (total, value) in self.buckets.iter_mut().zip(fact.buckets) {
            add(total, value)?;
        }
        add(&mut self.records, 1)?;
        self.estimate
            .combine(&catalog.estimate(&fact.model, fact.buckets, Some(1)))
    }
    fn tokens(&self) -> Result<TokenTotals, AppError> {
        let mut total_tokens = self.buckets[0];
        add(&mut total_tokens, self.buckets[1])?;
        let mut prompt = self.buckets[0];
        add(&mut prompt, self.buckets[2])?;
        add(&mut prompt, self.buckets[3])?;
        Ok(TokenTotals {
            input_tokens: self.buckets[0],
            output_tokens: self.buckets[1],
            cache_read_tokens: self.buckets[2],
            cache_write_tokens: self.buckets[3],
            total_tokens,
            cache_hit_rate: (prompt > 0).then(|| self.buckets[2] as f64 / prompt as f64),
        })
    }
    fn estimate(&self) -> CostEstimate {
        if self.records == 0 {
            CostEstimate::empty()
        } else {
            self.estimate.clone()
        }
    }
}

/// The caller supplies a single cache revision, never transcript bodies. Usage
/// determines the token cohort after all filters; eligible main-thread messages
/// can additionally establish a session whose token amount is unknown.
#[cfg(test)]
pub fn build_analysis(
    query: UsageQueryV1,
    input: &AnalyticsInput,
    catalog: &PriceCatalog,
    days: &[DayBoundary],
) -> Result<SessionAnalytics, AppError> {
    build_analysis_in(query, input, catalog, days, &chrono::Utc)
}

pub(crate) fn build_analysis_in<Tz: chrono::TimeZone>(
    query: UsageQueryV1,
    input: &AnalyticsInput,
    catalog: &PriceCatalog,
    days: &[DayBoundary],
    tz: &Tz,
) -> Result<SessionAnalytics, AppError> {
    use super::session_analytics_core::{self as core, Filters, Window};
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, BTreeSet};

    let query = query.normalized()?;
    let filters = Filters {
        window: Window {
            start_ms: query.start_ms,
            end_ms: query.end_ms,
        },
        tool_ids: query.tool_ids.clone(),
        model_ids: query.model_ids.clone(),
        project_ids: query.project_ids.clone(),
        source_ids: query.source_ids.clone(),
    };
    let identities: Vec<_> = input.sessions.iter().map(|s| s.identity.clone()).collect();
    let roots = core::resolve_roots(&identities).map_err(|code| invalid(&code))?;
    // This also validates conflicting native event replicas before filtering.
    let aggregate = core::aggregate_usage(&input.facts, &identities, &filters)
        .map_err(|code| invalid(&code))?;
    let descriptors: BTreeMap<_, _> = input
        .sessions
        .iter()
        .map(|s| (s.identity.key.as_str(), s))
        .collect();
    if descriptors.len() != input.sessions.len() {
        return Err(invalid("duplicateSessionDescriptor"));
    }
    let mut reasons: BTreeSet<String> = input.reasons.iter().cloned().collect();
    reasons.extend(
        input
            .source_reasons
            .iter()
            .filter(|r| {
                selected(&query.tool_ids, &r.tool_id) && selected(&query.source_ids, &r.source_id)
            })
            .map(|r| r.reason.clone()),
    );
    if aggregate.excluded_unparsed_project_records > 0 {
        reasons.insert("unparsedProjectExcluded".into());
    }
    if aggregate.unassociated_records > 0 {
        reasons.insert("unassociatedUsage".into());
    }
    let metric_reason = if !query.model_ids.is_empty() {
        Some("modelAttributionUnavailable")
    } else if !query.source_ids.is_empty() {
        Some("sourceAttributionUnavailable")
    } else {
        None
    };
    let messages: Vec<_> = if metric_reason.is_some() {
        vec![]
    } else {
        input
            .messages
            .iter()
            .filter(|message| {
                let Some(identity) = roots.get(message.session_key.as_str()) else {
                    return false;
                };
                // Child messages cannot become main-thread activity by a caller flag.
                identity.key == message.session_key
                    && selected(&query.tool_ids, &identity.tool_id)
                    && (query.project_ids.is_empty()
                        || project_id(&identity.project)
                            .is_some_and(|id| selected(&query.project_ids, id)))
            })
            .cloned()
            .collect()
    };
    let activity =
        core::message_activity(&messages, &filters.window, days).map_err(|code| invalid(&code))?;
    let mut cohort: BTreeSet<String> = aggregate.sessions.keys().cloned().collect();
    for message in &messages {
        if message.main
            && matches!(message.role, core::Role::User | core::Role::Assistant)
            && in_window(&filters.window, message.at_ms)
        {
            cohort.insert(message.session_key.clone());
        }
    }
    cohort.extend(activity.by_session.keys().cloned());
    let mut message_reason = metric_reason.map(str::to_owned);
    // An undated/missing-identity message cannot enter the timed cohort. Its
    // absence from that cohort must not certify a zero message/activity count.
    if input.source_reasons.iter().any(|r| {
        selected(&query.tool_ids, &r.tool_id)
            && selected(&query.source_ids, &r.source_id)
            && incomplete_message_reason(&r.reason)
    }) {
        message_reason.get_or_insert_with(|| "messageCoveragePartial".into());
    }
    for key in &cohort {
        let descriptor = descriptors[key.as_str()];
        if !descriptor.messages_supported
            || descriptor
                .message_reasons
                .iter()
                .any(|reason| incomplete_message_reason(reason))
        {
            message_reason.get_or_insert_with(|| "messageCoveragePartial".into());
        }
        if !descriptor.source_available {
            reasons.insert("sourceMissing".into());
        }
        reasons.extend(descriptor.message_reasons.iter().cloned());
        if project_id(&descriptor.identity.project).is_none() {
            reasons.insert("unparsedProject".into());
        }
    }
    if message_reason.as_deref() == Some("messageCoveragePartial") {
        reasons.insert("messageCoveragePartial".into());
    }
    let metrics_available = message_reason.is_none();
    let eligible: BTreeSet<_> = cohort
        .iter()
        .filter(|key| {
            let descriptor = descriptors[key.as_str()];
            metric_reason.is_none()
                && descriptor.messages_supported
                && !descriptor
                    .message_reasons
                    .iter()
                    .any(|r| incomplete_message_reason(r))
        })
        .cloned()
        .collect();
    let unquantified_source_gaps = !input.reasons.is_empty()
        || input.source_reasons.iter().any(|r| {
            selected(&query.tool_ids, &r.tool_id) && selected(&query.source_ids, &r.source_id)
        })
        || !input.sources.iter().any(|s| {
            selected(&query.tool_ids, &s.tool_id) && selected(&query.source_ids, &s.source_id)
        })
        || input.sources.iter().any(|s| {
            selected(&query.tool_ids, &s.tool_id)
                && selected(&query.source_ids, &s.source_id)
                && s.status != "ok"
        });
    let mut metric_reasons: BTreeSet<String> = message_reason.iter().cloned().collect();
    if unquantified_source_gaps {
        metric_reasons.insert("sourceCoveragePartial".into());
    }
    let metric_coverage = MetricCoverage {
        state: if metric_reason.is_some()
            || (eligible.is_empty() && (!cohort.is_empty() || unquantified_source_gaps))
        {
            "unavailable"
        } else if eligible.len() < cohort.len() || unquantified_source_gaps {
            "partial"
        } else {
            "complete"
        }
        .into(),
        eligible_sessions: eligible.len() as u64,
        excluded_sessions: (cohort.len() - eligible.len()) as u64,
        unquantified_source_gaps,
        reasons: metric_reasons.into_iter().collect(),
    };
    let known_activity = if eligible.len() == cohort.len() {
        activity.clone()
    } else {
        let known_messages: Vec<_> = messages
            .iter()
            .filter(|m| eligible.contains(&m.session_key))
            .cloned()
            .collect();
        core::message_activity(&known_messages, &filters.window, days)
            .map_err(|code| invalid(&code))?
    };
    let known_metrics = (metric_coverage.state != "unavailable").then_some(KnownMetrics {
        messages: known_activity.messages,
        active_ms: known_activity.active_ms,
        active_days: known_activity.active_days,
    });
    let activity_by_hour =
        advanced::hour_activity(&known_activity, &metric_coverage, query.end_ms, tz)?;
    let mut by_session: BTreeMap<String, Sum> = BTreeMap::new();
    let mut by_model_session: BTreeMap<String, BTreeMap<String, Sum>> = BTreeMap::new();
    let mut total = Sum::default();
    let mut unassociated = Sum::default();
    let mut daily: Vec<Sum> = days.iter().map(|_| Sum::default()).collect();
    let mut ordered_days: Vec<_> = days.iter().enumerate().collect();
    ordered_days.sort_by_key(|(_, d)| d.start_ms);
    let mut daily_sessions: Vec<BTreeSet<String>> = days.iter().map(|_| BTreeSet::new()).collect();
    let mut project_sums: BTreeMap<String, (Sum, BTreeSet<String>)> = BTreeMap::new();
    let mut model_sums: BTreeMap<String, (Sum, BTreeSet<String>)> = BTreeMap::new();
    let mut tool_sums: BTreeMap<String, (Sum, BTreeSet<String>)> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for fact in &input.facts {
        if !seen.insert(fact.event_key.as_str())
            || !in_window(&filters.window, fact.at_ms)
            || !selected(&query.tool_ids, &fact.tool_id)
            || !selected(&query.model_ids, &fact.model)
            || !selected(&query.source_ids, &fact.source_id)
        {
            continue;
        }
        let root = fact
            .session_key
            .as_deref()
            .and_then(|key| roots.get(key))
            .copied();
        if !query.project_ids.is_empty()
            && !root
                .and_then(|s| project_id(&s.project))
                .is_some_and(|id| selected(&query.project_ids, id))
        {
            continue;
        }
        let Some(root) = root else {
            unassociated.add(fact, catalog)?;
            continue;
        };
        total.add(fact, catalog)?;
        by_session
            .entry(root.key.clone())
            .or_default()
            .add(fact, catalog)?;
        by_model_session
            .entry(root.key.clone())
            .or_default()
            .entry(fact.model.clone())
            .or_default()
            .add(fact, catalog)?;
        let day_index = ordered_days.partition_point(|(_, d)| d.end_ms <= fact.at_ms);
        let day = ordered_days
            .get(day_index)
            .filter(|(_, d)| d.start_ms <= fact.at_ms)
            .map(|(i, _)| *i)
            .ok_or_else(|| invalid("missingUsageDayBoundary"))?;
        daily[day].add(fact, catalog)?;
        daily_sessions[day].insert(root.key.clone());
        if !fact.complete {
            reasons.insert("incompleteUsageFact".into());
        }
        if let Some(project) = project_id(&root.project) {
            let entry = project_sums.entry(project.to_owned()).or_default();
            entry.0.add(fact, catalog)?;
            entry.1.insert(root.key.clone());
        }
        for (map, id) in [
            (&mut model_sums, fact.model.as_str()),
            (&mut tool_sums, root.tool_id.as_str()),
        ] {
            let entry = map.entry(id.to_owned()).or_default();
            entry.0.add(fact, catalog)?;
            entry.1.insert(root.key.clone());
        }
    }
    // Message-only sessions participate in cohort/project counts while their
    // missing usage remains null and stays out of token distributions.
    for key in &cohort {
        let descriptor = descriptors[key.as_str()];
        if let Some(project) = project_id(&descriptor.identity.project) {
            project_sums
                .entry(project.to_owned())
                .or_default()
                .1
                .insert(key.clone());
        }
        tool_sums
            .entry(descriptor.identity.tool_id.clone())
            .or_default()
            .1
            .insert(key.clone());
    }
    let mut cohort_rows = Vec::new();
    let mut cohort_tokens = cohort
        .iter()
        .map(|key| {
            Ok((
                key.clone(),
                by_session.get(key).map(Sum::tokens).transpose()?,
            ))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let quantiles = core::quantiles(cohort_tokens.iter().map(|(key, tokens)| {
        tokens
            .as_ref()
            .filter(|_| aggregate.sessions.get(key).is_some_and(|s| s.complete))
            .map(|t| t.total_tokens)
    }));
    let usage_unknown = cohort_tokens.iter().any(|(_, tokens)| tokens.is_none());
    if usage_unknown {
        reasons.insert("sessionUsageUnavailable".into());
    }
    cohort_tokens.sort_by(|(ka, a), (kb, b)| {
        b.as_ref()
            .map(|t| t.total_tokens)
            .cmp(&a.as_ref().map(|t| t.total_tokens))
            .then_with(|| ka.cmp(kb))
    });
    let token_distribution = advanced::distribution(
        "tokens",
        &[0, 1_000, 10_000, 100_000, 1_000_000, 10_000_000],
        cohort_tokens.iter().map(|(key, tokens)| {
            tokens
                .as_ref()
                .filter(|_| aggregate.sessions.get(key).is_some_and(|s| s.complete))
                .map(|t| t.total_tokens)
        }),
        unquantified_source_gaps,
    );
    for (key, tokens) in cohort_tokens {
        let descriptor = descriptors[key.as_str()];
        let sum = by_session.get(&key);
        let own_available = eligible.contains(&key);
        let mut row_reasons = descriptor.message_reasons.clone();
        if tokens.is_none() {
            row_reasons.push("sessionUsageUnavailable".into());
        } else if aggregate.sessions.get(&key).is_some_and(|s| !s.complete) {
            row_reasons.push("incompleteUsageFact".into());
        }
        let models = by_model_session
            .get(&key)
            .map(|models| {
                models
                    .iter()
                    .map(|(model, sum)| {
                        Ok(AnalyticsModel {
                            model: model.clone(),
                            tokens: sum.tokens()?,
                            estimate: sum.estimate(),
                        })
                    })
                    .collect::<Result<Vec<_>, AppError>>()
            })
            .transpose()?
            .unwrap_or_default();
        cohort_rows.push(AnalyticsSession {
            session_key: key.clone(),
            native_session_id: descriptor.native_session_id.clone(),
            tool_id: descriptor.identity.tool_id.clone(),
            title: descriptor.title.clone(),
            project_id: project_id(&descriptor.identity.project).map(str::to_owned),
            project_label: descriptor.project_label.clone(),
            project_path: descriptor.project_path.clone(),
            manager_key: descriptor.manager_key.clone(),
            source_available: descriptor.source_available,
            tokens,
            estimate: sum.map(Sum::estimate).unwrap_or_default(),
            models,
            messages: own_available
                .then_some(activity.by_session_messages.get(&key).copied().unwrap_or(0)),
            active_ms: own_available.then_some(activity.by_session.get(&key).copied().unwrap_or(0)),
            reasons: row_reasons,
        });
    }
    let top_sessions = cohort_rows.iter().take(TOP_LIMIT).cloned().collect();
    let compact_top = advanced::compact_top(&cohort_rows);
    let distributions = vec![
        token_distribution,
        advanced::distribution(
            "messages",
            &[0, 1, 6, 16, 31, 61, 121],
            cohort_rows.iter().map(|r| r.messages),
            unquantified_source_gaps,
        ),
        advanced::distribution(
            "activeMs",
            &[0, 1, 60_000, 360_000, 960_000, 1_860_000, 3_660_000],
            cohort_rows.iter().map(|r| r.active_ms),
            unquantified_source_gaps,
        ),
    ];
    let mut series = Vec::new();
    for (index, day) in days.iter().enumerate() {
        if let Some(keys) = activity.daily_sessions.get(&day.date) {
            daily_sessions[index].extend(keys.iter().cloned());
        }
        series.push(AnalyticsDay {
            date: day.date.clone(),
            start_ms: day.start_ms,
            end_ms: day.end_ms,
            session_count: daily_sessions[index].len() as u64,
            tokens: daily[index].tokens()?,
            estimate: daily[index].estimate(),
            messages: metrics_available
                .then_some(activity.daily_messages.get(&day.date).copied().unwrap_or(0)),
            active_ms: metrics_available.then_some(
                activity
                    .daily_active_ms
                    .get(&day.date)
                    .copied()
                    .unwrap_or(0),
            ),
            known_messages: known_metrics.as_ref().map(|_| {
                known_activity
                    .daily_messages
                    .get(&day.date)
                    .copied()
                    .unwrap_or(0)
            }),
            known_active_ms: known_metrics.as_ref().map(|_| {
                known_activity
                    .daily_active_ms
                    .get(&day.date)
                    .copied()
                    .unwrap_or(0)
            }),
        });
    }
    let project_count = project_sums.len() as u64;
    let mut projects: BTreeMap<String, ProjectOption> = BTreeMap::new();
    let mut options = FilterOptions::default();
    for descriptor in &input.sessions {
        options.tool_ids.push(descriptor.identity.tool_id.clone());
        if let Some(id) = project_id(&descriptor.identity.project) {
            projects
                .entry(id.to_owned())
                .or_insert_with(|| ProjectOption {
                    id: id.to_owned(),
                    label: descriptor
                        .project_label
                        .clone()
                        .unwrap_or_else(|| id.into()),
                    path: descriptor.project_path.clone(),
                });
        }
    }
    for fact in &input.facts {
        options.tool_ids.push(fact.tool_id.clone());
        options.model_ids.push(fact.model.clone());
        options.source_ids.push(fact.source_id.clone());
    }
    for ids in [
        &mut options.tool_ids,
        &mut options.model_ids,
        &mut options.source_ids,
    ] {
        ids.sort();
        ids.dedup();
    }
    let project_options = &projects;
    let rank = |map: BTreeMap<String, (Sum, BTreeSet<String>)>,
                projects: bool|
     -> Result<Vec<AnalyticsRank>, AppError> {
        let mut rows = Vec::new();
        for (id, (sum, sessions)) in map {
            let descriptor = projects.then(|| project_options.get(&id)).flatten();
            rows.push(AnalyticsRank {
                id: id.clone(),
                label: descriptor
                    .map(|s| s.label.clone())
                    .unwrap_or_else(|| id.clone()),
                path: descriptor.and_then(|s| s.path.clone()),
                session_count: sessions.len() as u64,
                tokens: sum.tokens()?,
                estimate: {
                    let mut estimate = if sum.records == 0 && !sessions.is_empty() {
                        CostEstimate::default()
                    } else {
                        sum.estimate()
                    };
                    if sessions.iter().any(|key| !by_session.contains_key(key))
                        && estimate.known_cost_usd.is_some()
                    {
                        estimate.availability = "partial".into();
                        estimate.unpriced_record_count = None;
                    }
                    estimate
                },
            });
        }
        rows.sort_by(|a, b| {
            b.tokens
                .total_tokens
                .cmp(&a.tokens.total_tokens)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(rows)
    };
    let mut estimate = if total.records == 0 && !cohort.is_empty() {
        reasons.insert("sessionUsageUnavailable".into());
        CostEstimate::default()
    } else {
        total.estimate()
    };
    if usage_unknown && estimate.known_cost_usd.is_some() {
        estimate.availability = "partial".into();
        estimate.unpriced_record_count = None;
    }
    let pricing_coverage = catalog.coverage_for(&estimate);
    let fingerprint = query.fingerprint()?;
    let mut hash = Sha256::new();
    hash.update(b"session-analytics-v2\0");
    hash.update(input.snapshot_version.as_bytes());
    hash.update(fingerprint.as_bytes());
    hash.update(
        serde_json::to_vec(&pricing_coverage).map_err(|_| invalid("invalidPricingSnapshot"))?,
    );
    for day in days {
        hash.update(day.date.as_bytes());
        hash.update(day.start_ms.to_be_bytes());
        hash.update(day.end_ms.to_be_bytes());
    }
    let sources: Vec<_> = input
        .sources
        .iter()
        .filter(|source| {
            selected(&filters.tool_ids, &source.tool_id)
                && selected(&filters.source_ids, &source.source_id)
        })
        .cloned()
        .collect();
    if sources.iter().any(|s| s.status == "unsupported") {
        reasons.insert("sourceUnsupported".into());
    }
    if sources.iter().any(|s| s.status != "ok") || sources.is_empty() {
        reasons.insert("sourceCoveragePartial".into());
    }
    hash.update(serde_json::to_vec(&sources).map_err(|_| invalid("invalidSourceHealth"))?);
    hash.update(
        serde_json::to_vec(&activity_by_hour).map_err(|_| invalid("invalidHourProjection"))?,
    );
    let source_ids: BTreeSet<_> = input
        .facts
        .iter()
        .filter(|f| {
            selected(&query.tool_ids, &f.tool_id) && selected(&query.source_ids, &f.source_id)
        })
        .map(|f| f.source_id.clone())
        .collect();
    Ok(SessionAnalytics {
        receipt: QueryReceipt {
            applied_query: query,
            query_fingerprint: fingerprint,
            snapshot_version: format!("{:x}", hash.finalize()),
        },
        coverage: Coverage {
            state: if reasons.is_empty() {
                "complete"
            } else {
                "partial"
            }
            .into(),
            reasons: reasons.into_iter().collect(),
            time_basis: "eventTime".into(),
            detail_start_ms: input.facts.iter().map(|f| f.at_ms).min(),
            detail_end_ms: input.facts.iter().map(|f| f.at_ms).max(),
            aggregate_only: false,
            excluded_rollup_days: 0,
            source_ids: source_ids.into_iter().collect(),
        },
        pricing_coverage,
        totals: SessionKpis {
            tokens: total.tokens()?,
            session_count: cohort.len() as u64,
            project_count,
            median_tokens: quantiles.median_tokens,
            p90_tokens: quantiles.p90_tokens,
            quantile_samples: quantiles.samples,
            quantile_excluded: quantiles.excluded,
            estimate,
            messages: metrics_available.then_some(activity.messages),
            active_ms: metrics_available.then_some(activity.active_ms),
            active_days: metrics_available.then_some(activity.active_days),
            message_reason: message_reason.clone(),
            activity_reason: message_reason,
        },
        series,
        by_project: rank(project_sums, true)?,
        by_model: rank(model_sums, false)?,
        by_tool: rank(tool_sums, false)?,
        top_sessions,
        top_limit: TOP_LIMIT as u32,
        options: SessionOptions {
            projects: projects.into_values().collect(),
            ..options
        },
        unassociated: UnassociatedUsage {
            records: unassociated.records,
            tokens: unassociated.tokens()?,
        },
        sources,
        compact_top,
        known_metrics,
        metric_coverage,
        distributions,
        activity_by_hour,
        analytics_snapshot_id: None,
        paging_unavailable_reason: None,
        cohort_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        database::Database,
        services::session_analytics_core::{Project, Role, UNKNOWN_PROJECT_ID},
    };

    fn query() -> UsageQueryV1 {
        UsageQueryV1 {
            version: 1,
            start_ms: Some(0),
            end_ms: 100_000,
            tool_ids: vec![],
            model_ids: vec![],
            project_ids: vec![],
            source_ids: vec![],
        }
    }
    fn descriptor(key: &str, project: Project) -> SessionDescriptor {
        SessionDescriptor {
            identity: SessionIdentity {
                key: key.into(),
                tool_id: "codex".into(),
                project,
                verified_root_key: None,
            },
            native_session_id: key.into(),
            title: key.into(),
            project_label: Some("project".into()),
            project_path: None,
            manager_key: Some(format!("manager:{key}")),
            source_available: true,
            messages_supported: true,
            message_reasons: vec![],
        }
    }
    fn fact(
        id: &str,
        session: Option<&str>,
        model: &str,
        at_ms: i64,
        tokens: [u64; 4],
    ) -> UsageFact {
        UsageFact {
            event_key: id.into(),
            session_key: session.map(str::to_owned),
            tool_id: "codex".into(),
            source_id: "codex_session".into(),
            model: model.into(),
            at_ms,
            buckets: tokens,
            complete: true,
        }
    }
    fn days() -> Vec<DayBoundary> {
        vec![
            DayBoundary {
                date: "first".into(),
                start_ms: 0,
                end_ms: 50_000,
            },
            DayBoundary {
                date: "second".into(),
                start_ms: 50_000,
                end_ms: 100_000,
            },
        ]
    }
    fn input() -> AnalyticsInput {
        AnalyticsInput {
            snapshot_version: "source-snapshot".into(),
            sessions: vec![],
            facts: vec![],
            messages: vec![],
            reasons: vec![],
            source_reasons: vec![],
            sources: vec![],
        }
    }
    fn catalog(db: &Database) -> PriceCatalog {
        PriceCatalog::load_on(&db.conn.lock().unwrap(), "ofox.ai", 100_000).unwrap()
    }

    fn health(tool: &str, source: &str, status: &str) -> SourceHealth {
        SourceHealth {
            tool_id: tool.into(),
            source_id: source.into(),
            status: status.into(),
            last_attempt_at_ms: Some(1),
            last_success_at_ms: Some(1),
            error_code: None,
            last_scan: None,
            retained_detail_range: None,
        }
    }
    fn large_fixture() -> (AnalyticsInput, UsageQueryV1, Vec<DayBoundary>) {
        let mut input = input();
        let day_ms = 86_400_000;
        let end = 1460 * day_ms;
        let base = end - day_ms;
        for i in 0..10_000 {
            let sid = format!("s{i:05}");
            input
                .sessions
                .push(descriptor(&sid, Project::Known("/project".into())));
            for j in 0..5 {
                let id = format!("m{i}-{j}");
                input
                    .facts
                    .push(fact(&id, Some(&sid), "m", base + j * 1000, [1, 1, 0, 0]));
                input.messages.push(MessageEvent {
                    session_key: sid.clone(),
                    native_id: id,
                    at_ms: base + j * 1000,
                    role: Role::User,
                    main: true,
                });
            }
        }
        input.sources = vec![health("codex", "codex_session", "ok")];
        let days: Vec<_> = (0..1460)
            .map(|i| DayBoundary {
                date: i.to_string(),
                start_ms: i * day_ms,
                end_ms: (i + 1) * day_ms,
            })
            .collect();
        let mut q = query();
        q.end_ms = end;
        (input, q, days)
    }

    #[test]
    fn session_analysis_ten_thousand_sessions_fifty_thousand_events_keep_bounded_details() {
        let db = Database::memory().unwrap();
        let (input, q, days) = large_fixture();
        let started = std::time::Instant::now();
        let result = build_analysis(q, &input, &catalog(&db), &days).unwrap();
        eprintln!("session analytics 10k/50k/1460days {:?}", started.elapsed());
        assert_eq!(result.totals.session_count, 10_000);
        assert_eq!(result.totals.tokens.total_tokens, 100_000);
        assert_eq!(result.totals.messages, Some(50_000));
        assert_eq!(result.totals.active_ms, Some(40_000_000));
        assert_eq!(result.totals.median_tokens.as_deref(), Some("10"));
        assert_eq!(result.top_sessions.len(), 100);
        assert_eq!(result.series.last().unwrap().session_count, 10_000);
        assert!(serde_json::to_vec(&result).unwrap().len() < 1024 * 1024);
    }

    #[test]
    #[ignore = "opt-in release API benchmark; no IPC, rendering, or native-source scan"]
    fn session_analytics_release_warm_query_p95() {
        use crate::{
            services::usage_health::{self, Channel, Outcome},
            session_manager::source_cache::{self, Part},
        };
        use serde_json::json;
        use std::time::{Duration, Instant};

        assert!(
            !std::hint::black_box(cfg!(debug_assertions)),
            "run this benchmark with cargo test --release"
        );
        let db = Database::memory().unwrap();
        let (data, query, _) = large_fixture();
        {
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction().unwrap();
            // Seed the same normalized body-free fixture used by the existing
            // aggregation regression into 100 cache envelopes. Native parsing
            // is deliberately outside the measured API's responsibility.
            for i in 0..100 {
                source_cache::merge_on(
                    &tx,
                    &format!("benchmark-{i:03}"),
                    "codex",
                    None,
                    Part::Analytics,
                    &None,
                    Some(json!({
                        "version":1,"stamp":null,
                        "sessions":&data.sessions[i * 100..(i + 1) * 100],
                        "facts":&data.facts[i * 500..(i + 1) * 500],
                        "messages":&data.messages[i * 500..(i + 1) * 500],
                        "reasons":[],"available":true,"status":"ok","priority":0,
                        "attemptedAtMs":1,"succeededAtMs":1,
                    })),
                )
                .unwrap();
            }
            for channel in [Channel::Ledger, Channel::Analytics] {
                let attempt =
                    usage_health::begin_on(&tx, "codex", "codex_session", channel, 1).unwrap();
                assert!(usage_health::finish_on(&tx, &attempt, Outcome::Ok, 1).unwrap());
            }
            PriceCatalog::ingest_on(&tx, "ofox.ai", &json!({"data":[{"id":"m","pricing":{"prompt":"0.000001","completion":"0.000002"}}]}), query.end_ms).unwrap();
            tx.commit().unwrap();
        }
        let run = || {
            let result = super::super::session_analytics_api::query(
                &db,
                query.clone(),
                "ofox.ai",
                query.end_ms,
                &chrono::Local,
            )
            .unwrap();
            assert_eq!(result.totals.session_count, 10_000);
            assert_eq!(result.totals.tokens.total_tokens, 100_000);
            assert_eq!(result.known_metrics.as_ref().unwrap().messages, 50_000);
            assert!(result.analytics_snapshot_id.is_some());
            result
        };
        for _ in 0..3 {
            drop(run());
        }
        let mut samples = Vec::with_capacity(30);
        for _ in 0..30 {
            let started = Instant::now();
            let result = run();
            samples.push(started.elapsed());
            drop(result);
        }
        samples.sort_unstable();
        let p50 = samples[14];
        let p95 = samples[28];
        eprintln!("session analytics release warm API: os={} arch={} sessions=10000 facts=50000 messages=50000 cacheEnvelopes=100 days=1460 warmups=3 rounds=30 p50={p50:?} p95={p95:?}; includes SQLite cache/catalog/health read, deduplication, aggregation, and frozen-cohort retention; excludes cold parsing, native source scan, IPC serialization/transport and UI rendering", std::env::consts::OS, std::env::consts::ARCH);
        assert!(
            p95 < Duration::from_millis(300),
            "warm API p95 exceeds 300ms: {p95:?}"
        );
    }
    #[test]
    fn session_analysis_mixed_known_usage_and_message_only_keeps_unknown_coverage() {
        let db = Database::memory().unwrap();
        let mut input = input();
        input.sessions = vec![
            descriptor("known", Project::Unknown),
            descriptor("unknown", Project::Unknown),
        ];
        input.facts = vec![fact("a", Some("known"), "m", 1000, [1, 2, 0, 0])];
        input.messages = vec![MessageEvent {
            session_key: "unknown".into(),
            native_id: "u".into(),
            at_ms: 2000,
            role: Role::User,
            main: true,
        }];
        input.sources = vec![health("codex", "codex_session", "ok")];
        let catalog=PriceCatalog::ingest_on(&db.conn.lock().unwrap(),"ofox.ai",&serde_json::json!({"data":[{"id":"m","pricing":{"prompt":"1","completion":"2","input_cache_read":"0","input_cache_write":"0"}}]}),100_000).unwrap();
        let result = build_analysis(query(), &input, &catalog, &days()).unwrap();
        assert_eq!(result.totals.session_count, 2);
        assert_eq!(result.totals.quantile_excluded, 1);
        assert_eq!(result.coverage.state, "partial");
        assert!(result
            .coverage
            .reasons
            .contains(&"sessionUsageUnavailable".into()));
        assert_eq!(result.totals.estimate.availability, "partial");
    }
    #[test]
    fn session_analysis_selected_healthy_source_is_not_polluted_by_other_failed_or_unsupported_sources(
    ) {
        let db = Database::memory().unwrap();
        let mut input = input();
        input.sessions = vec![descriptor("known", Project::Unknown)];
        input.facts = vec![fact("a", Some("known"), "m", 1000, [1, 2, 0, 0])];
        input.sources = vec![
            health("codex", "codex_session", "ok"),
            health("claude", "session_log", "failed"),
            health("workbuddy", "workbuddy_session", "unsupported"),
        ];
        let mut q = query();
        q.tool_ids = vec!["codex".into()];
        let result = build_analysis(q, &input, &catalog(&db), &days()).unwrap();
        assert_eq!(result.coverage.state, "complete");
        assert_eq!(result.sources.len(), 1);
        let mut unsupported = query();
        unsupported.tool_ids = vec!["workbuddy".into()];
        let result = build_analysis(unsupported, &input, &catalog(&db), &days()).unwrap();
        assert_eq!(result.coverage.state, "partial");
        assert!(result
            .coverage
            .reasons
            .contains(&"sourceUnsupported".into()));
    }
    #[test]
    fn session_analysis_filters_model_before_distribution_and_disables_unattributed_activity() {
        let db = Database::memory().unwrap();
        let mut input = input();
        input.sessions = vec![
            descriptor("one", Project::Known("/one/project".into())),
            descriptor("two", Project::Known("/two/project".into())),
        ];
        input.facts = vec![
            fact("a", Some("one"), "model-a", 1000, [4, 6, 100, 20]),
            fact("b", Some("one"), "model-b", 2000, [900, 100, 0, 0]),
            fact("c", Some("two"), "model-a", 60_000, [10, 10, 0, 0]),
        ];
        input.messages = vec![MessageEvent {
            session_key: "one".into(),
            native_id: "m".into(),
            at_ms: 1000,
            role: Role::User,
            main: true,
        }];
        let mut q = query();
        q.model_ids.push("model-a".into());
        let result = build_analysis(q.clone(), &input, &catalog(&db), &days()).unwrap();
        assert_eq!(result.totals.session_count, 2);
        assert_eq!(result.totals.tokens.total_tokens, 30);
        assert_eq!(result.totals.median_tokens.as_deref(), Some("15"));
        assert_eq!(result.totals.p90_tokens, Some(20));
        assert_eq!(result.totals.messages, None);
        assert_eq!(result.totals.active_ms, None);
        assert_eq!(
            result.totals.message_reason.as_deref(),
            Some("modelAttributionUnavailable")
        );
        assert!(result
            .series
            .iter()
            .all(|d| d.messages.is_none() && d.active_ms.is_none()));
        assert_eq!(result.receipt.applied_query, q);
        assert_eq!(result.totals.estimate.availability, "unknown");
    }

    #[test]
    fn session_analysis_distinguishes_unknown_unparsed_and_unassociated_and_retains_metadata_options(
    ) {
        let db = Database::memory().unwrap();
        let mut input = input();
        input.sessions = vec![
            descriptor("unknown", Project::Unknown),
            descriptor("unparsed", Project::Unparsed),
            descriptor("elsewhere", Project::Known("/elsewhere".into())),
        ];
        input.facts = vec![
            fact("a", Some("unknown"), "m", 1000, [1, 2, 0, 0]),
            fact("b", Some("unparsed"), "m", 1000, [3, 4, 0, 0]),
            fact("c", None, "m", 1000, [5, 6, 0, 0]),
        ];
        let result = build_analysis(query(), &input, &catalog(&db), &days()).unwrap();
        assert_eq!(result.totals.tokens.total_tokens, 10);
        assert_eq!(result.unassociated.tokens.total_tokens, 11);
        assert_eq!(result.unassociated.records, 1);
        assert_eq!(result.totals.project_count, 1);
        assert_eq!(result.coverage.state, "partial");
        assert!(result.coverage.reasons.contains(&"unparsedProject".into()));
        assert!(result.options.projects.iter().any(|p| p.id == "/elsewhere"));
        let mut q = query();
        q.project_ids.push(UNKNOWN_PROJECT_ID.into());
        let filtered = build_analysis(q, &input, &catalog(&db), &days()).unwrap();
        assert_eq!(filtered.totals.session_count, 1);
        assert_eq!(filtered.totals.tokens.total_tokens, 3);
        assert!(filtered
            .coverage
            .reasons
            .contains(&"unparsedProjectExcluded".into()));
    }

    #[test]
    fn session_analysis_message_only_unknown_tokens_are_excluded_from_quantiles_and_child_activity()
    {
        let db = Database::memory().unwrap();
        let mut input = input();
        let root = descriptor("root", Project::Known("/project".into()));
        let mut child = descriptor("child", Project::Known("/child".into()));
        child.identity.verified_root_key = Some("root".into());
        let mut missing = descriptor("message-only", Project::Known("/other".into()));
        missing.source_available = false;
        missing.manager_key = None;
        input.sessions = vec![root, child, missing];
        input.facts = vec![
            fact("a", Some("root"), "m", 10_000, [1, 2, 0, 0]),
            fact("b", Some("child"), "m", 20_000, [3, 4, 0, 0]),
        ];
        input.messages = vec![
            MessageEvent {
                session_key: "root".into(),
                native_id: "r0".into(),
                at_ms: 10_000,
                role: Role::User,
                main: true,
            },
            MessageEvent {
                session_key: "root".into(),
                native_id: "r1".into(),
                at_ms: 70_000,
                role: Role::Assistant,
                main: true,
            },
            MessageEvent {
                session_key: "child".into(),
                native_id: "c0".into(),
                at_ms: 15_000,
                role: Role::User,
                main: false,
            },
            MessageEvent {
                session_key: "message-only".into(),
                native_id: "m0".into(),
                at_ms: 30_000,
                role: Role::User,
                main: true,
            },
        ];
        let result = build_analysis(query(), &input, &catalog(&db), &days()).unwrap();
        assert_eq!(result.totals.session_count, 2);
        assert_eq!(result.totals.quantile_samples, 1);
        assert_eq!(result.totals.quantile_excluded, 1);
        assert_eq!(result.totals.median_tokens.as_deref(), Some("10"));
        assert_eq!(result.totals.messages, Some(3));
        assert_eq!(result.totals.active_ms, Some(60_000));
        assert_eq!(result.totals.active_days, Some(2));
        assert_eq!(result.series[0].active_ms, Some(40_000));
        assert_eq!(result.series[1].active_ms, Some(20_000));
        let missing = result
            .top_sessions
            .iter()
            .find(|s| s.session_key == "message-only")
            .unwrap();
        assert!(missing.tokens.is_none());
        assert!(missing.manager_key.is_none());
        assert!(!missing.source_available);
        assert!(result.top_sessions.iter().all(|s| s.session_key != "child"));
    }
    #[test]
    fn session_analysis_undated_messages_cannot_become_zero_when_the_session_has_no_timed_cohort() {
        let db = Database::memory().unwrap();
        let mut data = input();
        data.sessions = vec![descriptor("undated", Project::Unknown)];
        data.sessions[0]
            .message_reasons
            .push("messageTimeUnavailable".into());
        data.source_reasons.push(SourceReason {
            tool_id: "codex".into(),
            source_id: "codex_session".into(),
            reason: "messageTimeUnavailable".into(),
        });
        data.sources = vec![health("codex", "codex_session", "partial")];
        let result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.totals.messages, None);
        assert_eq!(result.totals.active_ms, None);
        assert!(result
            .series
            .iter()
            .all(|d| d.messages.is_none() && d.active_ms.is_none()));
        let mut selected = query();
        selected.tool_ids = vec!["claude".into()];
        let other = build_analysis(selected, &data, &catalog(&db), &days()).unwrap();
        assert_eq!(
            other.totals.messages,
            Some(0),
            "an unselected source cannot taint this query"
        );
    }

    #[test]
    fn session_analysis_known_metrics_preserve_complete_subset_without_changing_legacy_nulls() {
        let db = Database::memory().unwrap();
        let mut data = input();
        data.sessions = vec![
            descriptor("good", Project::Unknown),
            descriptor("partial", Project::Unknown),
        ];
        data.sessions[1]
            .message_reasons
            .push("messageTimeUnavailable".into());
        data.facts = vec![
            fact("g", Some("good"), "m", 1_000, [1, 1, 0, 0]),
            fact("p", Some("partial"), "m", 1_000, [1, 1, 0, 0]),
        ];
        for (session, at) in [("good", 1_000), ("good", 61_000), ("partial", 2_000)] {
            data.messages.push(MessageEvent {
                session_key: session.into(),
                native_id: format!("{session}-{at}"),
                at_ms: at,
                role: Role::User,
                main: true,
            });
        }
        let result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert!(result.totals.messages.is_none());
        assert!(result.totals.active_ms.is_none());
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["knownMetrics"]["messages"], 2);
        assert_eq!(json["knownMetrics"]["activeMs"], 60_000);
        assert_eq!(json["metricCoverage"]["eligibleSessions"], 1);
        assert_eq!(json["metricCoverage"]["excludedSessions"], 1);
        assert_eq!(
            result
                .series
                .iter()
                .map(|day| day.known_active_ms.unwrap())
                .sum::<u64>(),
            60_000
        );
        assert_eq!(
            result
                .series
                .iter()
                .map(|day| day.known_messages.unwrap())
                .sum::<u64>(),
            2
        );
        assert_eq!(
            result
                .activity_by_hour
                .cells
                .iter()
                .map(|cell| cell.active_ms)
                .sum::<u64>(),
            60_000
        );
        let mut filtered = query();
        filtered.model_ids = vec!["m".into()];
        let filtered =
            serde_json::to_value(build_analysis(filtered, &data, &catalog(&db), &days()).unwrap())
                .unwrap();
        assert!(filtered["knownMetrics"].is_null());
        assert_eq!(filtered["metricCoverage"]["state"], "unavailable");
        let mut source_filter = query();
        source_filter.source_ids = vec!["codex_session".into()];
        let filtered = build_analysis(source_filter, &data, &catalog(&db), &days()).unwrap();
        assert!(filtered.known_metrics.is_none());
        assert!(filtered.activity_by_hour.cells.is_empty());
        assert!(filtered
            .series
            .iter()
            .all(|day| day.known_messages.is_none() && day.known_active_ms.is_none()));
    }

    #[test]
    fn session_analysis_compact_active_top_uses_entire_cohort_and_distributions_count_zero() {
        let db = Database::memory().unwrap();
        let mut data = input();
        for i in 0..110 {
            let key = format!("s{i:03}");
            data.sessions.push(descriptor(&key, Project::Unknown));
            data.facts
                .push(fact(&key, Some(&key), "m", 1_000, [i, 0, 0, 0]));
        }
        for at in [1_000, 61_000] {
            data.messages.push(MessageEvent {
                session_key: "s000".into(),
                native_id: format!("m-{at}"),
                at_ms: at,
                role: Role::User,
                main: true,
            });
        }
        let result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.top_sessions.len(), 100);
        assert!(!result
            .top_sessions
            .iter()
            .any(|row| row.session_key == "s000"));
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["compactTop"]["active"][0]["sessionKey"], "s000");
        assert_eq!(json["compactTop"]["tokens"].as_array().unwrap().len(), 6);
        let token_distribution = json["distributions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["metric"] == "tokens")
            .unwrap();
        assert_eq!(token_distribution["samples"], 110);
        assert_eq!(
            token_distribution["counts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap())
                .sum::<u64>(),
            110
        );
    }

    #[test]
    fn session_analysis_unidentifiable_source_does_not_supply_a_fake_missing_session_count() {
        let db = Database::memory().unwrap();
        let mut data = input();
        data.sources = vec![health("codex", "codex_session", "partial")];
        data.source_reasons.push(SourceReason {
            tool_id: "codex".into(),
            source_id: "codex_session".into(),
            reason: "sessionIdentityUnavailable".into(),
        });
        let result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.metric_coverage.state, "unavailable");
        assert!(result.known_metrics.is_none());
        assert!(result.metric_coverage.unquantified_source_gaps);
        assert_eq!(result.metric_coverage.eligible_sessions, 0);
        assert_eq!(result.metric_coverage.excluded_sessions, 0);
        assert!(result.activity_by_hour.cells.is_empty());
        assert!(result
            .distributions
            .iter()
            .all(|d| d.samples == 0 && d.unquantified_source_gaps));
    }
}
