//! Read-only session/project analysis over one cached, body-free snapshot.
pub use super::usage_projects::ProjectOption;
use super::{
    ofox_pricing::{CostEstimate, PriceCatalog, PricingCoverage},
    session_analytics_core::{DayBoundary, MessageEvent, SessionIdentity, UsageFact},
    usage_data::FilterOptions,
    usage_query::{Coverage, QueryReceipt, SourceHealth, UsageQueryV1},
};
use crate::error::AppError;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[path = "session_analytics_advanced.rs"]
pub mod advanced;
#[path = "session_analytics_interactions.rs"]
pub mod interactions;
#[path = "session_analytics_trend.rs"]
mod trend;
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
    #[serde(default)]
    pub tool_evidence: interactions::Evidence,
    pub snapshot_version: String,
    pub sessions: Vec<SessionDescriptor>,
    pub facts: Vec<UsageFact>,
    pub messages: Vec<MessageEvent>,
    pub reasons: Vec<String>,
    pub source_reasons: Vec<SourceReason>,
    pub sources: Vec<SourceHealth>,
    #[serde(default)]
    pub native_activity: Vec<super::session_native_activity::NativeActivityEvent>,
    #[serde(default)]
    pub source_observations: Vec<SourceObservation>,
    #[serde(default)]
    pub codex_native_ready: bool,
    #[serde(default)]
    pub legacy_history_available: bool,
    #[serde(default)]
    pub thread_usage_coverage:
        std::collections::BTreeMap<String, super::session_analytics_listing::ThreadUsageCoverage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceObservation {
    pub tool_id: String,
    pub source_id: String,
    pub status: String,
    pub dated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCoverage {
    pub observed: u64,
    pub read: u64,
    pub partial: u64,
    pub failed: u64,
    pub undated: u64,
    pub legacy_history_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_availability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_usage_records: Option<u64>,
}
impl SourceCoverage {
    pub(crate) fn with_usage_samples(mut self, records: u64) -> Self {
        let gaps = self.partial > 0 || self.failed > 0 || self.undated > 0;
        self.usage_availability = Some(
            if records == 0 && gaps {
                "unavailable"
            } else if gaps {
                "partial"
            } else {
                "available"
            }
            .into(),
        );
        self.known_usage_records = Some(records);
        self
    }
}
impl AnalyticsInput {
    pub(crate) fn source_coverage(&self, query: &UsageQueryV1) -> SourceCoverage {
        let mut result = SourceCoverage {
            legacy_history_available: self.legacy_history_available
                && selected(&query.tool_ids, "codex")
                && selected(&query.source_ids, "codex_session"),
            ..Default::default()
        };
        for source in &self.source_observations {
            if (!query.tool_ids.is_empty() && !query.tool_ids.contains(&source.tool_id))
                || (!query.source_ids.is_empty() && !query.source_ids.contains(&source.source_id))
            {
                continue;
            }
            result.observed += 1;
            match source.status.as_str() {
                "ok" => result.read += 1,
                "failed" => result.failed += 1,
                _ => result.partial += 1,
            }
            result.undated += u64::from(!source.dated);
        }
        result
    }
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
    pub method_version: &'static str,
    pub input_snapshot_version: String,
    pub source_coverage: SourceCoverage,
    #[serde(flatten)]
    pub receipt: QueryReceipt,
    pub coverage: Coverage,
    pub pricing_coverage: PricingCoverage,
    pub totals: SessionKpis,
    pub series: Vec<AnalyticsDay>,
    /// Optional finer chart projection; daily series still owns natural-day KPIs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trend: Option<trend::AnalyticsTrend>,
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
    pub interactions: interactions::Interactions,
    pub compact_top: CompactTop,
    pub known_metrics: Option<KnownMetrics>,
    pub metric_coverage: MetricCoverage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_coverage: Option<MetricCoverage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_coverage: Option<MetricCoverage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_messages: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_active_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_active_days: Option<u64>,
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
fn unavailable_activity_reason(reason: &str) -> bool {
    matches!(
        reason,
        "conflictingSessionMetadata"
            | "sessionIdentityUnavailable"
            | "sessionRoleNotRecorded"
            | "sessionParentUnavailable"
            | "invalidSessionRoot"
            | "incompleteSource"
    )
}
fn usage_only_metric_reason(reason: &str) -> bool {
    matches!(
        reason,
        "nativeUsageUnavailable"
            | "missingNativeCounters"
            | "freshInputUnavailable"
            | "missingModel"
            | "incompleteUsageFact"
            | "conflictingUsageReplica"
            | "unprovenLegacyReplica"
    )
}
fn unknown_metric_range_reason(reason: &str) -> bool {
    matches!(
        reason,
        "analyticsCacheUnavailable"
            | "sourceReadFailed"
            | "sourceMissing"
            | "sourceLineBudgetExceeded"
            | "projectionBudgetExceeded"
            | "malformedNativeRecord"
            | "missingTimestamp"
            | "usageTimeUnavailable"
            | "messageTimeUnavailable"
            | "activityTimeUnavailable"
            | "missingNativeSessionIdentity"
            | "sessionIdentityUnavailable"
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
    estimate: NumericEstimate,
}

struct CostContribution<'a> {
    value: &'a CostEstimate,
    amount: Option<Decimal>,
}
impl<'a> CostContribution<'a> {
    fn new(value: &'a CostEstimate) -> Result<Self, AppError> {
        Ok(Self {
            value,
            amount: value
                .known_cost_usd
                .as_deref()
                .map(super::ofox_pricing::parse_amount)
                .transpose()?,
        })
    }
}

/// Running arithmetic stays numeric. Decimal text is emitted once per
/// completed projection rather than parsed/formatted for every fact in every
/// session/day/project/model/tool accumulator.
struct NumericEstimate {
    amount: Option<Decimal>,
    partial: bool,
    unpriced_models: BTreeSet<String>,
    official_models: BTreeSet<String>,
    unpriced_records: Option<u64>,
    additional_units: bool,
}
impl Default for NumericEstimate {
    fn default() -> Self {
        Self {
            amount: None,
            partial: false,
            unpriced_models: BTreeSet::new(),
            official_models: BTreeSet::new(),
            unpriced_records: Some(0),
            additional_units: false,
        }
    }
}
impl NumericEstimate {
    fn combine(&mut self, contribution: &CostContribution<'_>) -> Result<(), AppError> {
        let value = contribution.value;
        let overflow = || AppError::Database("usage.pricing.aggregateOverflow".into());
        let amount = match (self.amount, contribution.amount) {
            (Some(a), Some(b)) => Some(a.checked_add(b).ok_or_else(overflow)?),
            (a, b) => a.or(b),
        }
        .map(|amount| amount.normalize());
        let unpriced_records = self
            .unpriced_records
            .zip(value.unpriced_record_count)
            .map(|(a, b)| a.checked_add(b).ok_or_else(overflow))
            .transpose()?;
        let additional_units = self.additional_units || value.additional_units_unsupported;
        // Match CostEstimate::combine's sticky partial policy, including a
        // previous zero-token estimate with unsupported additional charges.
        let before_has_unknown =
            self.partial || (self.amount.is_none() && !self.unpriced_models.is_empty());
        let partial = amount.is_some()
            && (before_has_unknown
                || value.availability == "partial"
                || (value.availability == "unknown" && !value.unpriced_model_ids.is_empty())
                || (amount.is_some_and(|v| v.is_zero()) && additional_units));
        self.amount = amount;
        self.unpriced_records = unpriced_records;
        self.additional_units = additional_units;
        self.partial = partial;
        for model in &value.unpriced_model_ids {
            if !self.unpriced_models.contains(model) {
                self.unpriced_models.insert(model.clone());
            }
        }
        for model in &value.official_model_ids {
            if !self.official_models.contains(model) {
                self.official_models.insert(model.clone());
            }
        }
        Ok(())
    }
    fn value(&self) -> CostEstimate {
        CostEstimate {
            known_cost_usd: self.amount.map(super::ofox_pricing::clean_decimal),
            availability: match self.amount {
                None => "unknown",
                Some(_) if self.partial => "partial",
                Some(amount) if amount.is_zero() => "free",
                Some(_) => "known",
            }
            .into(),
            unpriced_model_ids: self.unpriced_models.iter().cloned().collect(),
            unpriced_record_count: self.unpriced_records,
            additional_units_unsupported: self.additional_units,
            official_model_ids: self.official_models.iter().cloned().collect(),
        }
    }
}
impl Sum {
    #[cfg(test)]
    fn add(&mut self, fact: &UsageFact, estimate: &CostEstimate) -> Result<(), AppError> {
        self.add_prepared(fact, &CostContribution::new(estimate)?)
    }
    fn add_prepared(
        &mut self,
        fact: &UsageFact,
        estimate: &CostContribution<'_>,
    ) -> Result<(), AppError> {
        for (total, value) in self.buckets.iter_mut().zip(fact.buckets) {
            add(total, value)?;
        }
        add(&mut self.records, 1)?;
        self.estimate.combine(estimate)
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
            self.estimate.value()
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
    use std::collections::{BTreeMap, BTreeSet, HashSet};

    #[cfg(test)]
    let mut profile = std::time::Instant::now();

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
    let mut ordered_days: Vec<_> = days.iter().enumerate().collect();
    ordered_days.sort_by_key(|(_, day)| day.start_ms);
    let identities: Vec<_> = input.sessions.iter().map(|s| s.identity.clone()).collect();
    let roots = core::resolve_roots(&identities).map_err(|code| invalid(&code))?;
    // This also validates conflicting native event replicas before filtering.
    let aggregate = core::aggregate_usage_with_roots(&input.facts, &roots, &filters)
        .map_err(|code| invalid(&code))?;
    #[cfg(test)]
    advanced::profile_stage("usage+roots", &mut profile);
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
            .collect()
    };
    let native_mode = input.codex_native_ready
        || !input.source_observations.is_empty()
        || !input.native_activity.is_empty();
    let runtime_events: Vec<_> = input
        .native_activity
        .iter()
        .filter(|event| {
            let Some(identity) = roots.get(event.session_key.as_str()) else {
                return false;
            };
            metric_reason.is_none()
                && identity.key == event.session_key
                && selected(&query.tool_ids, &identity.tool_id)
                && (query.project_ids.is_empty()
                    || project_id(&identity.project)
                        .is_some_and(|id| selected(&query.project_ids, id)))
        })
        .cloned()
        .collect();
    let runtime_supported: BTreeSet<_> = runtime_events
        .iter()
        .filter(|event| event.main)
        .map(|event| event.session_key.as_str())
        .collect();
    let message_activity =
        core::message_activity_refs(messages.iter().copied(), &filters.window, days)
            .map_err(|code| invalid(&code))?;
    let runtime_activity = if native_mode {
        super::session_native_activity::activity(&runtime_events, &filters.window, days)
            .map_err(|code| invalid(&code))?
    } else {
        message_activity.clone()
    };
    #[cfg(test)]
    advanced::profile_stage("activity", &mut profile);
    let mut cohort: BTreeSet<String> = aggregate.sessions.keys().cloned().collect();
    for message in &messages {
        if message.main
            && matches!(message.role, core::Role::User | core::Role::Assistant)
            && in_window(&filters.window, message.at_ms)
        {
            cohort.insert(message.session_key.clone());
        }
    }
    cohort.extend(
        runtime_activity
            .by_session
            .iter()
            .filter(|(_, amount)| !native_mode || **amount > 0)
            .map(|(key, _)| key.clone()),
    );
    let mut activity = if native_mode && runtime_supported.iter().any(|key| !cohort.contains(*key))
    {
        super::session_native_activity::activity(
            &runtime_events
                .iter()
                .filter(|event| cohort.contains(&event.session_key))
                .cloned()
                .collect::<Vec<_>>(),
            &filters.window,
            days,
        )
        .map_err(|code| invalid(&code))?
    } else {
        runtime_activity
    };
    // Message counts use their effective native message projection. Runtime
    // events cannot manufacture extra conversational messages.
    if native_mode {
        activity.messages = message_activity.messages;
        activity.daily_messages = message_activity.daily_messages.clone();
        activity.by_session_messages = message_activity.by_session_messages.clone();
        for (date, sessions) in &message_activity.daily_sessions {
            activity
                .daily_sessions
                .entry(date.clone())
                .or_default()
                .extend(sessions.iter().cloned());
        }
    }
    let mut evidence_dates: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    if native_mode {
        let message_dates = messages
            .iter()
            .filter(|message| {
                message.main
                    && matches!(message.role, core::Role::User | core::Role::Assistant)
                    && in_window(&filters.window, message.at_ms)
            })
            .map(|message| (message.session_key.as_str(), message.at_ms));
        let usage_dates = input
            .facts
            .iter()
            .filter(|fact| {
                in_window(&filters.window, fact.at_ms)
                    && selected(&query.tool_ids, &fact.tool_id)
                    && selected(&query.model_ids, &fact.model)
                    && selected(&query.source_ids, &fact.source_id)
            })
            .filter_map(|fact| {
                fact.session_key
                    .as_deref()
                    .and_then(|key| roots.get(key))
                    .map(|root| (root.key.as_str(), fact.at_ms))
            })
            .filter(|(key, _)| cohort.contains(*key));
        for (key, at) in message_dates.chain(usage_dates) {
            let index = ordered_days.partition_point(|(_, day)| day.end_ms <= at);
            let (_, day) = ordered_days
                .get(index)
                .filter(|(_, day)| day.start_ms <= at)
                .ok_or_else(|| invalid("missingActivityDayBoundary"))?;
            evidence_dates
                .entry(key)
                .or_default()
                .insert(day.date.as_str());
        }
    }
    let active_days_for = |projection: &core::Activity, selected: &BTreeSet<String>| {
        let mut dates: BTreeSet<&str> = projection
            .daily_active_ms
            .iter()
            .filter(|(_, amount)| **amount > 0)
            .map(|(date, _)| date.as_str())
            .collect();
        for key in selected {
            if let Some(observed) = evidence_dates.get(key.as_str()) {
                dates.extend(observed.iter().copied());
            }
        }
        dates.len() as u64
    };
    if native_mode {
        activity.active_days = active_days_for(&activity, &cohort);
    }
    for key in &cohort {
        let descriptor = descriptors[key.as_str()];
        if !descriptor.source_available {
            reasons.insert("sourceMissing".into());
        }
        reasons.extend(descriptor.message_reasons.iter().cloned());
        if project_id(&descriptor.identity.project).is_none() {
            reasons.insert("unparsedProject".into());
        }
    }
    let message_eligible: BTreeSet<_> = cohort
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
    // SourceHealth is aggregated by tool/source kind, not physical source.
    // A warning carried by an identified, out-of-window descriptor must not
    // turn another session's complete message/runtime projection into unknown.
    // Lost times and unreadable sources still have no provable query range.
    let owned_metric_reasons: BTreeSet<_> = input
        .sessions
        .iter()
        .flat_map(|session| {
            session
                .message_reasons
                .iter()
                .map(move |reason| (session.identity.tool_id.as_str(), reason.as_str()))
        })
        .collect();
    let cohort_metric_reasons: BTreeSet<_> = cohort
        .iter()
        .flat_map(|key| {
            let descriptor = descriptors[key.as_str()];
            descriptor
                .message_reasons
                .iter()
                .map(move |reason| (descriptor.identity.tool_id.as_str(), reason.as_str()))
        })
        .collect();
    let cohort_source_reason = |reason: &SourceReason| {
        let identity = (reason.tool_id.as_str(), reason.reason.as_str());
        selected(&query.tool_ids, &reason.tool_id)
            && selected(&query.source_ids, &reason.source_id)
            && (unknown_metric_range_reason(&reason.reason)
                || !owned_metric_reasons.contains(&identity)
                || cohort_metric_reasons.contains(&identity))
    };
    let partial_activity: BTreeSet<_> = cohort
        .iter()
        .filter(|key| {
            let descriptor = descriptors[key.as_str()];
            descriptor
                .message_reasons
                .iter()
                .any(|reason| reason.starts_with("activity"))
                || input
                    .reasons
                    .iter()
                    .any(|reason| reason.starts_with("activity"))
                || input.source_reasons.iter().any(|reason| {
                    reason.tool_id == descriptor.identity.tool_id
                        && cohort_source_reason(reason)
                        && reason.reason.starts_with("activity")
                        && (unknown_metric_range_reason(&reason.reason)
                            || !owned_metric_reasons
                                .contains(&(reason.tool_id.as_str(), reason.reason.as_str()))
                            || descriptor.message_reasons.contains(&reason.reason))
                })
        })
        .cloned()
        .collect();
    let activity_eligible: BTreeSet<_> = cohort
        .iter()
        .filter(|key| {
            let descriptor = descriptors[key.as_str()];
            metric_reason.is_none()
                && if native_mode {
                    runtime_supported.contains(key.as_str())
                        && !descriptor
                            .message_reasons
                            .iter()
                            .any(|reason| unavailable_activity_reason(reason))
                        && (!partial_activity.contains(*key)
                            || activity
                                .by_session
                                .get(*key)
                                .is_some_and(|amount| *amount > 0))
                } else {
                    message_eligible.contains(*key)
                }
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
    let scoped_reason = |reason: &&SourceReason| {
        selected(&query.tool_ids, &reason.tool_id) && selected(&query.source_ids, &reason.source_id)
    };
    let legacy_message_gaps = input
        .reasons
        .iter()
        .any(|reason| !reason.starts_with("activity"))
        || input
            .source_reasons
            .iter()
            .filter(scoped_reason)
            .any(|reason| !reason.reason.starts_with("activity"))
        || !input.sources.iter().any(|source| {
            selected(&query.tool_ids, &source.tool_id)
                && selected(&query.source_ids, &source.source_id)
        })
        || input.sources.iter().any(|source| {
            selected(&query.tool_ids, &source.tool_id)
                && selected(&query.source_ids, &source.source_id)
                && source.status != "ok"
        });
    let reason_affects_metric = |reason: &str, runtime: bool| {
        !usage_only_metric_reason(reason)
            && if runtime {
                !reason.starts_with("message")
            } else {
                !reason.starts_with("activity")
            }
    };
    let native_metric_gaps = |runtime: bool| {
        let selected_sources = || {
            input.sources.iter().filter(|source| {
                selected(&query.tool_ids, &source.tool_id)
                    && selected(&query.source_ids, &source.source_id)
            })
        };
        let health_gap = selected_sources().next().is_none()
            || selected_sources().any(|source| match source.status.as_str() {
                "ok" => false,
                "partial" => {
                    let source_reasons = || {
                        input.source_reasons.iter().filter(move |reason| {
                            reason.tool_id == source.tool_id && reason.source_id == source.source_id
                        })
                    };
                    // A coarse partial result without a scoped explanation is
                    // still unknown; do not silently upgrade old backends.
                    source_reasons().next().is_none()
                        || source_reasons().any(|reason| {
                            cohort_source_reason(reason)
                                && reason_affects_metric(&reason.reason, runtime)
                        })
                }
                _ => true,
            });
        health_gap
            || cohort
                .iter()
                .any(|key| !descriptors[key.as_str()].source_available)
            || input
                .reasons
                .iter()
                .any(|reason| reason_affects_metric(reason, runtime))
            || input.source_reasons.iter().any(|reason| {
                cohort_source_reason(reason) && reason_affects_metric(&reason.reason, runtime)
            })
    };
    let message_gaps = if native_mode {
        native_metric_gaps(false)
    } else {
        legacy_message_gaps
    };
    let activity_gaps = if native_mode {
        native_metric_gaps(true)
    } else {
        legacy_message_gaps
    };
    let coverage_for =
        |eligible: &BTreeSet<String>, gaps: bool, partial_reason: &str, partial_samples: bool| {
            let mut coverage_reasons: BTreeSet<String> =
                metric_reason.into_iter().map(str::to_owned).collect();
            if eligible.len() < cohort.len() || partial_samples {
                coverage_reasons.insert(partial_reason.into());
            }
            if gaps {
                coverage_reasons.insert("sourceCoveragePartial".into());
            }
            MetricCoverage {
                state: if metric_reason.is_some()
                    || (eligible.is_empty() && (native_mode || !cohort.is_empty() || gaps))
                {
                    "unavailable"
                } else if eligible.len() < cohort.len() || gaps || partial_samples {
                    "partial"
                } else {
                    "complete"
                }
                .into(),
                eligible_sessions: eligible.len() as u64,
                excluded_sessions: (cohort.len() - eligible.len()) as u64,
                unquantified_source_gaps: gaps,
                reasons: coverage_reasons.into_iter().collect(),
            }
        };
    let message_coverage = coverage_for(
        &message_eligible,
        message_gaps,
        "messageCoveragePartial",
        false,
    );
    let activity_coverage = coverage_for(
        &activity_eligible,
        activity_gaps,
        "activityCoveragePartial",
        native_mode && !partial_activity.is_empty(),
    );
    let eligible: BTreeSet<_> = message_eligible
        .intersection(&activity_eligible)
        .cloned()
        .collect();
    let metric_coverage = coverage_for(
        &eligible,
        unquantified_source_gaps,
        "messageCoveragePartial",
        native_mode && !partial_activity.is_empty(),
    );
    let mut message_reason = metric_reason.map(str::to_owned);
    if input
        .source_reasons
        .iter()
        .filter(|reason| {
            if native_mode {
                cohort_source_reason(reason)
            } else {
                scoped_reason(reason)
            }
        })
        .any(|reason| incomplete_message_reason(&reason.reason))
        || message_eligible.len() < cohort.len()
        || (native_mode && message_coverage.state != "complete")
    {
        message_reason.get_or_insert_with(|| "messageCoveragePartial".into());
    }
    let mut activity_reason = metric_reason.map(str::to_owned);
    if activity_eligible.len() < cohort.len()
        || (native_mode && activity_coverage.state != "complete")
        || (!native_mode && message_reason.is_some())
    {
        activity_reason.get_or_insert_with(|| "activityCoveragePartial".into());
    }
    if let Some(reason) = &message_reason {
        reasons.insert(reason.clone());
    }
    if let Some(reason) = &activity_reason {
        reasons.insert(reason.clone());
    }
    let messages_available = message_reason.is_none();
    let activity_available = activity_reason.is_none();
    let message_projection = |selected: &BTreeSet<String>| {
        core::message_activity_refs(
            messages
                .iter()
                .copied()
                .filter(|message| selected.contains(&message.session_key)),
            &filters.window,
            days,
        )
        .map_err(|code| invalid(&code))
    };
    let runtime_projection = |selected: &BTreeSet<String>| {
        if native_mode {
            let mut result = super::session_native_activity::activity(
                &runtime_events
                    .iter()
                    .filter(|event| selected.contains(&event.session_key))
                    .cloned()
                    .collect::<Vec<_>>(),
                &filters.window,
                days,
            )
            .map_err(|code| invalid(&code))?;
            result.active_days = active_days_for(&result, selected);
            Ok(result)
        } else {
            message_projection(selected)
        }
    };
    let known_message_activity = if message_eligible.len() == cohort.len() {
        std::borrow::Cow::Borrowed(&message_activity)
    } else {
        std::borrow::Cow::Owned(message_projection(&message_eligible)?)
    };
    let known_activity = if activity_eligible.len() == cohort.len() {
        std::borrow::Cow::Borrowed(&activity)
    } else {
        std::borrow::Cow::Owned(runtime_projection(&activity_eligible)?)
    };
    let known_messages =
        (message_coverage.state != "unavailable").then_some(known_message_activity.messages);
    let known_active_ms =
        (activity_coverage.state != "unavailable").then_some(known_activity.active_ms);
    let known_active_days =
        (activity_coverage.state != "unavailable").then_some(known_activity.active_days);
    let known_metrics = if metric_coverage.state == "unavailable" {
        None
    } else {
        let legacy_messages = if eligible == message_eligible {
            std::borrow::Cow::Borrowed(known_message_activity.as_ref())
        } else {
            std::borrow::Cow::Owned(message_projection(&eligible)?)
        };
        let legacy_activity = if eligible == activity_eligible {
            std::borrow::Cow::Borrowed(known_activity.as_ref())
        } else {
            std::borrow::Cow::Owned(runtime_projection(&eligible)?)
        };
        Some(KnownMetrics {
            messages: legacy_messages.messages,
            active_ms: legacy_activity.active_ms,
            active_days: legacy_activity.active_days,
        })
    };
    let activity_by_hour =
        advanced::hour_activity(&known_activity, &activity_coverage, query.end_ms, tz)?;
    #[cfg(test)]
    advanced::profile_stage("known+hours", &mut profile);
    let mut by_session: BTreeMap<&str, Sum> = BTreeMap::new();
    let mut by_model_session: BTreeMap<&str, BTreeMap<&str, Sum>> = BTreeMap::new();
    let mut total = Sum::default();
    let mut unassociated = Sum::default();
    let mut daily: Vec<Sum> = days.iter().map(|_| Sum::default()).collect();
    let mut daily_sessions: Vec<BTreeSet<&str>> = days.iter().map(|_| BTreeSet::new()).collect();
    let mut hourly_trend = trend::HourlyTrend::new(&query, days, tz)?;
    let mut project_sums: BTreeMap<&str, (Sum, BTreeSet<&str>)> = BTreeMap::new();
    let mut model_sums: BTreeMap<&str, (Sum, BTreeSet<&str>)> = BTreeMap::new();
    let mut tool_sums: BTreeMap<&str, (Sum, BTreeSet<&str>)> = BTreeMap::new();
    let mut seen = HashSet::new();
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
        // Every projection uses the same immutable per-record estimate from
        // the captured catalog. Resolve aliases/rates and price buckets once.
        let mut fact_estimate = catalog.estimate_with_hour_writes(
            &fact.model,
            fact.buckets,
            fact.cache_write_1h,
            Some(1),
        );
        if !fact.complete && fact_estimate.known_cost_usd.is_some() {
            fact_estimate.availability = "partial".into();
            fact_estimate.unpriced_record_count = None;
        }
        let contribution = CostContribution::new(&fact_estimate)?;
        let Some(root) = root else {
            unassociated.add_prepared(fact, &contribution)?;
            continue;
        };
        total.add_prepared(fact, &contribution)?;
        by_session
            .entry(root.key.as_str())
            .or_default()
            .add_prepared(fact, &contribution)?;
        by_model_session
            .entry(root.key.as_str())
            .or_default()
            .entry(fact.model.as_str())
            .or_default()
            .add_prepared(fact, &contribution)?;
        let day_index = ordered_days.partition_point(|(_, d)| d.end_ms <= fact.at_ms);
        let day = ordered_days
            .get(day_index)
            .filter(|(_, d)| d.start_ms <= fact.at_ms)
            .map(|(i, _)| *i)
            .ok_or_else(|| invalid("missingUsageDayBoundary"))?;
        daily[day].add_prepared(fact, &contribution)?;
        daily_sessions[day].insert(root.key.as_str());
        if let Some(trend) = &mut hourly_trend {
            trend.add_fact(fact, root.key.as_str(), &contribution)?;
        }
        if !fact.complete {
            reasons.insert("incompleteUsageFact".into());
        }
        if let Some(project) = project_id(&root.project) {
            let entry = project_sums.entry(project).or_default();
            entry.0.add_prepared(fact, &contribution)?;
            entry.1.insert(root.key.as_str());
        }
        for (map, id) in [
            (&mut model_sums, fact.model.as_str()),
            (&mut tool_sums, root.tool_id.as_str()),
        ] {
            let entry = map.entry(id).or_default();
            entry.0.add_prepared(fact, &contribution)?;
            entry.1.insert(root.key.as_str());
        }
    }
    #[cfg(test)]
    advanced::profile_stage("fact-sums", &mut profile);
    // Message-only sessions participate in cohort/project counts while their
    // missing usage remains null and stays out of token distributions.
    for key in &cohort {
        let descriptor = descriptors[key.as_str()];
        if let Some(project) = project_id(&descriptor.identity.project) {
            project_sums
                .entry(project)
                .or_default()
                .1
                .insert(key.as_str());
        }
        tool_sums
            .entry(descriptor.identity.tool_id.as_str())
            .or_default()
            .1
            .insert(key.as_str());
    }
    let mut cohort_rows = Vec::new();
    let mut cohort_tokens = cohort
        .iter()
        .map(|key| {
            Ok((
                key.clone(),
                by_session.get(key.as_str()).map(Sum::tokens).transpose()?,
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
        let sum = by_session.get(key.as_str());
        let own_messages_available = message_eligible.contains(&key);
        let own_activity_available = activity_eligible.contains(&key);
        let mut row_reasons = descriptor.message_reasons.clone();
        if native_mode && metric_reason.is_none() && !runtime_supported.contains(key.as_str()) {
            row_reasons.push("activityProjectionUnavailable".into());
        }
        if tokens.is_none() {
            row_reasons.push("sessionUsageUnavailable".into());
        } else if aggregate.sessions.get(&key).is_some_and(|s| !s.complete) {
            row_reasons.push("incompleteUsageFact".into());
        }
        let models = by_model_session
            .get(key.as_str())
            .map(|models| {
                models
                    .iter()
                    .map(|(model, sum)| {
                        Ok(AnalyticsModel {
                            model: (*model).to_owned(),
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
            messages: own_messages_available
                .then_some(activity.by_session_messages.get(&key).copied().unwrap_or(0)),
            active_ms: own_activity_available
                .then_some(activity.by_session.get(&key).copied().unwrap_or(0)),
            reasons: row_reasons,
        });
    }
    let interactions = interactions::summarize(
        input,
        &cohort_rows,
        &roots,
        &query,
        metric_reason,
        unquantified_source_gaps,
        tz,
    );
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
    #[cfg(test)]
    advanced::profile_stage("cohort+tops", &mut profile);
    let mut series = Vec::new();
    for (index, day) in days.iter().enumerate() {
        if let Some(keys) = activity.daily_sessions.get(&day.date) {
            daily_sessions[index].extend(keys.iter().map(String::as_str));
        }
        series.push(AnalyticsDay {
            date: day.date.clone(),
            start_ms: day.start_ms,
            end_ms: day.end_ms,
            session_count: daily_sessions[index].len() as u64,
            tokens: daily[index].tokens()?,
            estimate: daily[index].estimate(),
            messages: messages_available
                .then_some(activity.daily_messages.get(&day.date).copied().unwrap_or(0)),
            active_ms: activity_available.then_some(
                activity
                    .daily_active_ms
                    .get(&day.date)
                    .copied()
                    .unwrap_or(0),
            ),
            known_messages: known_messages.map(|_| {
                known_message_activity
                    .daily_messages
                    .get(&day.date)
                    .copied()
                    .unwrap_or(0)
            }),
            known_active_ms: known_active_ms.map(|_| {
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
    let mut option_tools = BTreeSet::new();
    let mut option_models = BTreeSet::new();
    let mut option_sources = BTreeSet::new();
    for descriptor in &input.sessions {
        option_tools.insert(descriptor.identity.tool_id.as_str());
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
        option_tools.insert(fact.tool_id.as_str());
        option_models.insert(fact.model.as_str());
        option_sources.insert(fact.source_id.as_str());
    }
    options.tool_ids = option_tools.into_iter().map(str::to_owned).collect();
    options.model_ids = option_models.into_iter().map(str::to_owned).collect();
    options.source_ids = option_sources.into_iter().map(str::to_owned).collect();
    let project_options = &projects;
    let rank = |map: BTreeMap<&str, (Sum, BTreeSet<&str>)>,
                projects: bool|
     -> Result<Vec<AnalyticsRank>, AppError> {
        let mut rows = Vec::new();
        for (id, (sum, sessions)) in map {
            let descriptor = projects.then(|| project_options.get(id)).flatten();
            rows.push(AnalyticsRank {
                id: id.to_owned(),
                label: descriptor
                    .map(|s| s.label.clone())
                    .unwrap_or_else(|| id.to_owned()),
                path: descriptor.and_then(|s| s.path.clone()),
                session_count: sessions.len() as u64,
                tokens: sum.tokens()?,
                estimate: {
                    let mut estimate = if sum.records == 0 && !sessions.is_empty() {
                        CostEstimate::default()
                    } else {
                        sum.estimate()
                    };
                    if sessions.iter().any(|key| !by_session.contains_key(*key))
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
    let trend = hourly_trend
        .map(|trend| {
            trend.finish(trend::Metrics {
                window: &filters.window,
                messages: &messages,
                runtime_events: &runtime_events,
                native_mode,
                cohort: &cohort,
                message_eligible: &message_eligible,
                activity: &activity,
                known_activity: &known_activity,
                messages_available,
                activity_available,
                known_messages: known_messages.is_some(),
                known_active: known_active_ms.is_some(),
            })
        })
        .transpose()?;
    let fingerprint = query.fingerprint()?;
    let mut hash = Sha256::new();
    let method_version = if input.codex_native_ready {
        "magpie-native-v1"
    } else {
        "legacy-v1"
    };
    hash.update(b"session-analytics-v3\0");
    hash.update(method_version.as_bytes());
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
    hash.update(serde_json::to_vec(&trend).map_err(|_| invalid("invalidTrendProjection"))?);
    let source_ids: BTreeSet<_> = input
        .facts
        .iter()
        .filter(|f| {
            selected(&query.tool_ids, &f.tool_id) && selected(&query.source_ids, &f.source_id)
        })
        .map(|f| f.source_id.clone())
        .collect();
    let source_coverage = input
        .source_coverage(&query)
        .with_usage_samples(total.records);
    Ok(SessionAnalytics {
        method_version,
        input_snapshot_version: input.snapshot_version.clone(),
        source_coverage,
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
            messages: messages_available.then_some(activity.messages),
            active_ms: activity_available.then_some(activity.active_ms),
            active_days: activity_available.then_some(activity.active_days),
            message_reason,
            activity_reason,
        },
        series,
        trend,
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
        interactions,
        compact_top,
        known_metrics,
        metric_coverage,
        message_coverage: Some(message_coverage),
        activity_coverage: Some(activity_coverage),
        known_messages,
        known_active_ms,
        known_active_days,
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

    #[test]
    fn analytics_sum_estimates_match_reference_decimal_and_unknown_semantics() {
        let estimate =
            |amount: Option<&str>, availability: &str, models: &[&str], count, additional| {
                CostEstimate {
                    known_cost_usd: amount.map(str::to_owned),
                    availability: availability.into(),
                    unpriced_model_ids: models.iter().map(|v| (*v).into()).collect(),
                    unpriced_record_count: count,
                    additional_units_unsupported: additional,
                    official_model_ids: vec![],
                }
            };
        let mut official = estimate(Some("4"), "known", &[], Some(0), false);
        official.official_model_ids = vec!["claude-opus-5-5".into()];
        let cases = [
            vec![official.clone(), official],
            vec![estimate(Some("0"), "free", &[], Some(0), false)],
            vec![
                estimate(None, "unknown", &["missing"], Some(1), false),
                estimate(Some("0"), "free", &[], Some(0), false),
            ],
            vec![
                estimate(Some("1e-20"), "partial", &["b", "a", "a"], Some(1), false),
                estimate(Some("0.00000000000000000002"), "known", &[], Some(0), false),
            ],
            vec![
                estimate(Some("0"), "partial", &[], None, true),
                estimate(Some("1"), "known", &[], Some(0), false),
            ],
            vec![
                estimate(Some("1"), "known", &[], Some(0), true),
                estimate(None, "unknown", &["missing"], None, false),
            ],
            vec![
                estimate(None, "partial", &[], Some(0), false),
                estimate(Some("1"), "known", &[], Some(0), false),
            ],
        ];
        for estimates in cases {
            let mut sum = Sum::default();
            let mut reference = CostEstimate::default();
            for value in estimates {
                reference.combine(&value).unwrap();
                sum.add(&fact("event", Some("s"), "m", 1, [0; 4]), &value)
                    .unwrap();
                assert_eq!(sum.estimate(), reference);
            }
        }
        let mut sum = Sum::default();
        let huge = estimate(
            Some("79228162514264337593543950335"),
            "known",
            &[],
            Some(0),
            false,
        );
        let one = estimate(Some("1"), "known", &[], Some(0), false);
        sum.add(&fact("event", Some("s"), "m", 1, [0; 4]), &huge)
            .unwrap();
        assert!(sum
            .add(&fact("event", Some("s"), "m", 1, [0; 4]), &one)
            .is_err());
        assert_eq!(sum.estimate().known_cost_usd, huge.known_cost_usd);
        assert!(sum
            .add(&fact("event", Some("s"), "m", 1, [0; 4]), &one)
            .is_err());
        let count_overflow = estimate(None, "unknown", &["missing"], Some(u64::MAX), false);
        let mut sum = Sum::default();
        sum.add(&fact("event", Some("s"), "m", 1, [0; 4]), &count_overflow)
            .unwrap();
        assert!(sum
            .add(
                &fact("event", Some("s"), "m", 1, [0; 4]),
                &estimate(None, "unknown", &["missing"], Some(1), false)
            )
            .is_err());
    }

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
            cache_write_1h: 0,
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
            native_activity: vec![],
            source_observations: vec![],
            codex_native_ready: false,
            legacy_history_available: false,
            tool_evidence: Default::default(),
            thread_usage_coverage: Default::default(),
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

    fn native_event(
        session: &str,
        id: &str,
        at_ms: i64,
    ) -> super::super::session_native_activity::NativeActivityEvent {
        super::super::session_native_activity::NativeActivityEvent {
            session_key: session.into(),
            event_key: id.into(),
            at_ms,
            main: true,
            stream_key: "stream".into(),
        }
    }

    #[test]
    fn analytics_today_trend_splits_all_metrics_by_hour_and_preserves_natural_days() {
        let hour = 3_600_000;
        let end = 2 * hour + 30_000;
        let mut query = query();
        query.end_ms = end;
        let mut data = input();
        data.codex_native_ready = true;
        data.sessions.push(descriptor("s", Project::Unknown));
        data.sources.push(health("codex", "codex_session", "ok"));
        data.facts = vec![
            fact("first", Some("s"), "m", 1_000, [10, 1, 20, 2]),
            fact("second", Some("s"), "m", hour, [30, 3, 40, 4]),
            fact("end-excluded", Some("s"), "m", end, [900, 900, 0, 0]),
        ];
        for (index, at_ms) in [hour - 10_000, hour + 10_000].into_iter().enumerate() {
            data.messages.push(MessageEvent {
                session_key: "s".into(),
                native_id: format!("message-{index}"),
                at_ms,
                role: Role::User,
                main: true,
            });
            data.native_activity
                .push(native_event("s", &format!("runtime-{index}"), at_ms));
        }
        let db = Database::memory().unwrap();
        let result = build_analysis(
            query,
            &data,
            &catalog(&db),
            &[DayBoundary {
                date: "1970-01-01".into(),
                start_ms: 0,
                end_ms: 24 * hour,
            }],
        )
        .unwrap();
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["trend"]["granularity"], "hour");
        let series = value["trend"]["series"].as_array().unwrap();
        assert_eq!(series.len(), 3);
        assert_eq!(series[0]["startMs"], 0);
        assert_eq!(series[1]["startMs"], hour);
        assert_eq!(series[2]["endMs"], end);
        assert_eq!(
            series
                .iter()
                .map(|p| p["totalTokens"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [11, 33, 0]
        );
        assert_eq!(
            series
                .iter()
                .map(|p| p["messages"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [1, 1, 0]
        );
        assert_eq!(
            series
                .iter()
                .map(|p| p["activeMs"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [0, 20_000, 0]
        );
        assert_eq!(
            series
                .iter()
                .map(|p| p["sessionCount"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [1, 1, 0]
        );
        assert_eq!(result.totals.active_days, Some(1));
        assert_eq!(result.series.len(), 1);
        assert_eq!(result.series[0].tokens.total_tokens, 44);
        assert_eq!(result.series[0].active_ms, Some(20_000));
    }

    #[test]
    fn analytics_hourly_trend_keeps_clipped_legacy_gaps_and_session_identity() {
        let hour = 3_600_000;
        let mut selected = query();
        selected.start_ms = Some(hour - 5_000);
        selected.end_ms = hour + 5_000;
        let mut data = input();
        data.sessions.push(descriptor("s", Project::Unknown));
        data.sources.push(health("codex", "codex_session", "ok"));
        for (index, at_ms) in [hour - 10_000, hour + 10_000].into_iter().enumerate() {
            data.messages.push(MessageEvent {
                session_key: "s".into(),
                native_id: format!("legacy-{index}"),
                at_ms,
                role: Role::Assistant,
                main: true,
            });
        }
        let db = Database::memory().unwrap();
        let result = build_analysis(
            selected,
            &data,
            &catalog(&db),
            &[DayBoundary {
                date: "1970-01-01".into(),
                start_ms: 0,
                end_ms: 24 * hour,
            }],
        )
        .unwrap();
        let hours = &result.trend.as_ref().unwrap().series;
        assert_eq!(
            hours.iter().map(|hour| hour.active_ms).collect::<Vec<_>>(),
            [Some(5_000), Some(5_000)]
        );
        assert_eq!(
            hours.iter().map(|hour| hour.messages).collect::<Vec<_>>(),
            [Some(0), Some(0)]
        );
        assert_eq!(
            hours
                .iter()
                .map(|hour| hour.session_count)
                .collect::<Vec<_>>(),
            [1, 1]
        );
        assert_eq!(result.totals.session_count, 1);
        assert_eq!(result.totals.active_days, Some(1));
        assert_eq!(result.totals.active_ms, Some(10_000));
    }

    #[test]
    fn analytics_hourly_trend_preserves_pricing_coverage_and_filter_unavailability() {
        let db = Database::memory().unwrap();
        let catalog = PriceCatalog::ingest_on(&db.conn.lock().unwrap(), "ofox.ai", &serde_json::json!({"data":[{"id":"m","pricing":{"prompt":"1","completion":"2","input_cache_read":"0","input_cache_write":"0"}}]}), 100_000).unwrap();
        let mut data = native_input("codex", "codex_session");
        data.facts[0].complete = false;
        data.native_activity = vec![
            native_event("s", "begin", 1_000),
            native_event("s", "end", 61_000),
        ];
        let bounds = [DayBoundary {
            date: "1970-01-01".into(),
            start_ms: 0,
            end_ms: 86_400_000,
        }];
        let result = build_analysis(query(), &data, &catalog, &bounds).unwrap();
        let hour = &result.trend.as_ref().unwrap().series[0];
        assert_eq!(hour.estimate, result.series[0].estimate);
        assert_eq!(hour.estimate.availability, "partial");
        assert_eq!(hour.estimate.unpriced_record_count, None);
        assert!(hour.estimate.known_cost_usd.is_some());
        let mut by_model = query();
        by_model.model_ids = vec!["m".into()];
        let filtered = build_analysis(by_model, &data, &catalog, &bounds).unwrap();
        let hour = &filtered.trend.as_ref().unwrap().series[0];
        assert_eq!(hour.tokens.total_tokens, result.totals.tokens.total_tokens);
        assert_eq!(
            (
                hour.messages,
                hour.active_ms,
                hour.known_messages,
                hour.known_active_ms
            ),
            (None, None, None, None)
        );
        let mut all = query();
        all.start_ms = None;
        assert!(build_analysis(all, &data, &catalog, &bounds)
            .unwrap()
            .trend
            .is_none());
        let mut multi_day = query();
        multi_day.end_ms = 2 * 86_400_000;
        assert!(build_analysis(
            multi_day,
            &data,
            &catalog,
            &[
                bounds[0].clone(),
                DayBoundary {
                    date: "1970-01-02".into(),
                    start_ms: 86_400_000,
                    end_ms: 2 * 86_400_000
                }
            ]
        )
        .unwrap()
        .trend
        .is_none());
    }

    #[test]
    fn analytics_hourly_trend_uses_local_hour_boundaries_with_quarter_hour_offsets() {
        use chrono::{FixedOffset, TimeZone};
        let tz = FixedOffset::east_opt(5 * 3_600 + 45 * 60).unwrap();
        let start = tz
            .with_ymd_and_hms(2026, 10, 23, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        let mut selected = query();
        selected.start_ms = Some(start);
        selected.end_ms = start + 3_600_000 + 15_000;
        let mut data = input();
        data.sessions.push(descriptor("s", Project::Unknown));
        data.facts = vec![fact(
            "local-hour",
            Some("s"),
            "m",
            start + 3_600_000,
            [7, 2, 0, 0],
        )];
        let db = Database::memory().unwrap();
        let result = build_analysis_in(
            selected,
            &data,
            &catalog(&db),
            &[DayBoundary {
                date: "2026-10-23".into(),
                start_ms: start,
                end_ms: start + 86_400_000,
            }],
            &tz,
        )
        .unwrap();
        let hours = &result.trend.as_ref().unwrap().series;
        assert_eq!(hours.len(), 2);
        assert_eq!(hours[0].start_ms, start);
        assert_eq!(hours[1].start_ms, start + 3_600_000);
        assert_eq!(hours[1].end_ms, start + 3_600_000 + 15_000);
        assert_eq!(
            hours
                .iter()
                .map(|hour| hour.tokens.total_tokens)
                .collect::<Vec<_>>(),
            [0, 9]
        );
        assert!(hours[0].date.ends_with("+05:45"));
    }

    fn native_input(tool: &str, source: &str) -> AnalyticsInput {
        let mut data = input();
        data.codex_native_ready = true;
        let mut session = descriptor("s", Project::Unknown);
        session.identity.tool_id = tool.into();
        data.sessions.push(session);
        let mut usage = fact("usage", Some("s"), "m", 60_000, [1, 1, 0, 0]);
        usage.tool_id = tool.into();
        usage.source_id = source.into();
        data.facts.push(usage);
        data.sources.push(health(tool, source, "ok"));
        data.source_observations.push(SourceObservation {
            tool_id: tool.into(),
            source_id: source.into(),
            status: "ok".into(),
            dated: true,
        });
        for at_ms in [1_000, 61_000] {
            data.messages.push(MessageEvent {
                session_key: "s".into(),
                native_id: format!("message-{at_ms}"),
                at_ms,
                role: Role::User,
                main: true,
            });
        }
        data
    }

    #[test]
    fn interaction_summaries_use_full_cohort_and_respect_window_and_attribution() {
        let db = Database::memory().unwrap();
        let mut data = input();
        for i in 0..125 {
            let sid = format!("session-{i}");
            data.sessions.push(descriptor(&sid, Project::Unknown));
            data.facts.push(fact(
                &format!("usage-{i}"),
                Some(&sid),
                "m",
                10_000,
                [1, 1, 0, 0],
            ));
            data.messages.push(MessageEvent {
                session_key: sid.clone(),
                native_id: format!("prompt-{i}"),
                at_ms: 1_000,
                role: Role::User,
                main: true,
            });
            let mut scan = interactions::CallScan::default();
            scan.record(Some("tool-1"), Some(20_000), "Bash", None);
            scan.record(Some("tool-2"), Some(100_000), "Read", None);
            data.tool_evidence.attach(&sid, &scan);
        }
        let mut result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.top_sessions.len(), 100);
        assert_eq!(result.interactions.tools.calls, 125);
        assert_eq!(result.interactions.tools.sessions, 125);
        assert_eq!(result.interactions.coverage.eligible_sessions, 125);
        assert_eq!(result.interactions.autonomy.samples, 125);
        assert_eq!(result.interactions.autonomy.counts[1], 125);
        // A complete repeated replica is counted once, a conflicting one is excluded.
        data.tool_evidence
            .calls
            .push(data.tool_evidence.calls[0].clone());
        result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.interactions.tools.calls, 125);
        data.tool_evidence.calls.last_mut().unwrap().call.name = "Write".into();
        result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.interactions.tools.calls, 124);
        assert_eq!(result.interactions.coverage.excluded_sessions, 1);
        data.tool_evidence.calls.last_mut().unwrap().call.name = "Bash".into();
        data.tool_evidence.calls.last_mut().unwrap().call.at_ms = -1;
        result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.interactions.tools.calls, 124);
        assert_eq!(result.interactions.coverage.excluded_sessions, 1);
        let mut q = query();
        q.model_ids = vec!["m".into()];
        result = build_analysis(q, &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.interactions.coverage.state, "unavailable");
        assert_eq!(result.interactions.tools.calls, 0);
        // A source without a tool parser is unknown, never a measured zero.
        data.tool_evidence = Default::default();
        result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.interactions.coverage.state, "unavailable");
        assert_eq!(result.interactions.coverage.excluded_sessions, 125);
    }

    #[test]
    fn interaction_weeks_follow_the_same_query_timezone_as_daily_activity() {
        use chrono::TimeZone;
        let db = Database::memory().unwrap();
        let at = chrono::Utc
            .with_ymd_and_hms(2026, 10, 18, 16, 30, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let mut data = input();
        data.sessions.push(descriptor("s", Project::Unknown));
        data.facts
            .push(fact("usage", Some("s"), "m", at, [1, 1, 0, 0]));
        data.messages.push(MessageEvent {
            session_key: "s".into(),
            native_id: "prompt".into(),
            at_ms: at,
            role: Role::User,
            main: true,
        });
        let mut scan = interactions::CallScan::default();
        scan.record(Some("call"), Some(at), "Bash", None);
        data.tool_evidence.attach("s", &scan);
        let mut q = query();
        q.start_ms = Some(at - 3_600_000);
        q.end_ms = at + 3_600_000;
        let boundaries = [DayBoundary {
            date: "2026-10-18".into(),
            start_ms: at - 3_600_000,
            end_ms: at + 3_600_000,
        }];
        let utc =
            build_analysis_in(q.clone(), &data, &catalog(&db), &boundaries, &chrono::Utc).unwrap();
        let shanghai = build_analysis_in(
            q,
            &data,
            &catalog(&db),
            &boundaries,
            &chrono::FixedOffset::east_opt(8 * 3600).unwrap(),
        )
        .unwrap();
        assert_eq!(utc.interactions.tools.weeks[0].start, "2026-10-12");
        assert_eq!(shanghai.interactions.tools.weeks[0].start, "2026-10-19");
    }

    #[test]
    fn native_analysis_runtime_beats_visible_message_gaps_and_excludes_metadata_only_cohort() {
        let db = Database::memory().unwrap();
        let mut data = native_input("codex", "codex_session");
        data.messages[1].at_ms = 601_000;
        data.sessions.push(descriptor("metadata", Project::Unknown));
        data.native_activity = vec![native_event("metadata", "header", 1_000)];
        data.native_activity.extend(
            [1_000, 201_000, 401_000, 601_000]
                .into_iter()
                .enumerate()
                .map(|(id, at)| native_event("s", &format!("runtime-{id}"), at)),
        );
        let mut q = query();
        q.end_ms = 1_000_000;
        let boundaries = [
            DayBoundary {
                date: "first".into(),
                start_ms: 0,
                end_ms: 500_000,
            },
            DayBoundary {
                date: "second".into(),
                start_ms: 500_000,
                end_ms: 1_000_000,
            },
        ];
        let result = build_analysis(q, &data, &catalog(&db), &boundaries).unwrap();
        assert_eq!(result.method_version, "magpie-native-v1");
        assert_eq!(result.totals.session_count, 1);
        assert_eq!(result.totals.messages, Some(2));
        assert_eq!(result.totals.active_ms, Some(600_000));
        assert_eq!(result.known_messages, Some(2));
        assert_eq!(result.known_active_ms, Some(600_000));
        assert_eq!(
            result
                .series
                .iter()
                .map(|day| day.active_ms.unwrap())
                .collect::<Vec<_>>(),
            [400_000, 200_000]
        );
        assert_eq!(
            result
                .activity_by_hour
                .cells
                .iter()
                .map(|cell| cell.active_ms)
                .sum::<u64>(),
            600_000
        );
    }

    #[test]
    fn native_analysis_missing_runtime_never_falls_back_to_visible_messages_or_fake_zero() {
        let db = Database::memory().unwrap();
        let data = native_input("gemini", "gemini_session");
        let result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.totals.messages, Some(2));
        assert_eq!(result.totals.active_ms, None);
        assert_eq!(result.known_messages, Some(2));
        assert_eq!(result.known_active_ms, None);
        assert_eq!(result.message_coverage.as_ref().unwrap().state, "complete");
        assert_eq!(
            result.activity_coverage.as_ref().unwrap().state,
            "unavailable"
        );
        assert!(result.activity_by_hour.cells.is_empty());
        assert_eq!(result.top_sessions[0].messages, Some(2));
        assert_eq!(result.top_sessions[0].active_ms, None);
    }

    #[test]
    fn native_analysis_metric_coverage_ignores_known_metadata_and_outside_cohort_issues() {
        let db = Database::memory().unwrap();
        let mut data = native_input("codex", "codex_session");
        data.native_activity = vec![
            native_event("s", "begin", 1_000),
            native_event("s", "end", 61_000),
            native_event("metadata", "header", 1_000),
            native_event("outside", "begin", 110_000),
            native_event("outside", "end", 120_000),
        ];
        let mut metadata = descriptor("metadata", Project::Unknown);
        metadata.messages_supported = false;
        metadata.message_reasons = vec!["messagePartsUnavailable".into()];
        let mut outside = descriptor("outside", Project::Unknown);
        outside.message_reasons = vec!["activityIdentityUnavailable".into()];
        data.sessions.extend([metadata, outside]);
        data.facts.push(fact(
            "outside-usage",
            Some("outside"),
            "m",
            120_000,
            [5, 2, 0, 0],
        ));
        data.sources[0].status = "partial".into();
        for reason in ["messagePartsUnavailable", "activityIdentityUnavailable"] {
            data.source_reasons.push(SourceReason {
                tool_id: "codex".into(),
                source_id: "codex_session".into(),
                reason: reason.into(),
            });
        }
        let result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.totals.session_count, 1);
        assert_eq!(result.totals.messages, Some(2));
        assert_eq!(result.totals.active_ms, Some(60_000));
        for coverage in [
            result.message_coverage.as_ref().unwrap(),
            result.activity_coverage.as_ref().unwrap(),
        ] {
            assert_eq!(coverage.state, "complete");
            assert_eq!(coverage.eligible_sessions, 1);
            assert_eq!(coverage.excluded_sessions, 0);
            assert!(!coverage.unquantified_source_gaps);
        }
    }

    #[test]
    fn native_analysis_usage_only_warnings_do_not_hide_verified_message_or_runtime() {
        let db = Database::memory().unwrap();
        let mut data = native_input("codex", "codex_session");
        data.native_activity = vec![
            native_event("s", "begin", 1_000),
            native_event("s", "end", 61_000),
        ];
        data.facts[0].complete = false;
        data.sources[0].status = "partial".into();
        for reason in [
            "nativeUsageUnavailable",
            "missingNativeCounters",
            "conflictingUsageReplica",
            "unprovenLegacyReplica",
        ] {
            data.source_reasons.push(SourceReason {
                tool_id: "codex".into(),
                source_id: "codex_session".into(),
                reason: reason.into(),
            });
        }
        let result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.totals.messages, Some(2));
        assert_eq!(result.totals.active_ms, Some(60_000));
        assert_eq!(result.message_coverage.as_ref().unwrap().state, "complete");
        assert_eq!(result.activity_coverage.as_ref().unwrap().state, "complete");
        assert!(result
            .coverage
            .reasons
            .contains(&"incompleteUsageFact".into()));
    }

    #[test]
    fn native_analysis_unattributed_source_failure_keeps_known_parts_and_unknown_runtime() {
        let db = Database::memory().unwrap();
        let mut data = native_input("codex", "codex_session");
        data.native_activity = vec![
            native_event("s", "begin", 1_000),
            native_event("s", "end", 61_000),
        ];
        data.sources[0].status = "failed".into();
        data.source_reasons.push(SourceReason {
            tool_id: "codex".into(),
            source_id: "codex_session".into(),
            reason: "analyticsCacheUnavailable".into(),
        });
        let partial = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(partial.totals.messages, None);
        assert_eq!(partial.totals.active_ms, None);
        assert_eq!(partial.known_messages, Some(2));
        assert_eq!(partial.known_active_ms, Some(60_000));
        assert_eq!(partial.activity_coverage.as_ref().unwrap().state, "partial");
        assert!(
            partial
                .activity_coverage
                .as_ref()
                .unwrap()
                .unquantified_source_gaps
        );
        data.native_activity.clear();
        let missing = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(missing.totals.session_count, 1);
        assert_eq!(missing.totals.active_ms, None);
        assert_eq!(missing.known_active_ms, None);
        assert_eq!(
            missing.activity_coverage.as_ref().unwrap().state,
            "unavailable"
        );
    }

    #[test]
    fn native_analysis_active_days_exclude_header_only_dates_but_keep_proven_zero_usage_day() {
        let db = Database::memory().unwrap();
        let mut data = native_input("codex", "codex_session");
        data.messages[0].at_ms = 901_000;
        data.messages[1].at_ms = 1_001_000;
        data.facts[0].at_ms = 1_001_000;
        data.native_activity = vec![
            native_event("s", "header", 1_000),
            native_event("s", "begin", 901_000),
            native_event("s", "end", 1_001_000),
        ];
        let mut q = query();
        q.end_ms = 2_000_000;
        let boundaries = [
            DayBoundary {
                date: "metadata-only".into(),
                start_ms: 0,
                end_ms: 500_000,
            },
            DayBoundary {
                date: "active".into(),
                start_ms: 500_000,
                end_ms: 2_000_000,
            },
        ];
        let result = build_analysis(q.clone(), &data, &catalog(&db), &boundaries).unwrap();
        assert_eq!(result.totals.active_days, Some(1));
        assert_eq!(result.known_active_days, Some(1));
        assert_eq!(result.series[0].session_count, 0);
        data.messages.clear();
        data.facts[0].buckets = [0; 4];
        data.native_activity = vec![native_event("s", "usage", 1_001_000)];
        let zero = build_analysis(q, &data, &catalog(&db), &boundaries).unwrap();
        assert_eq!(zero.totals.session_count, 1);
        assert_eq!(zero.totals.active_ms, Some(0));
        assert_eq!(zero.known_active_ms, Some(0));
        assert_eq!(zero.totals.active_days, Some(1));
    }

    #[test]
    fn native_analysis_message_and_runtime_coverage_are_independent() {
        let db = Database::memory().unwrap();
        let mut data = native_input("opencode", "opencode_session");
        data.native_activity = vec![
            native_event("s", "begin", 1_000),
            native_event("s", "end", 61_000),
        ];
        data.sessions[0]
            .message_reasons
            .push("activityTimeUnavailable".into());
        let result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.totals.messages, Some(2));
        assert_eq!(result.totals.active_ms, None);
        assert_eq!(result.known_active_ms, Some(60_000));
        assert_eq!(
            result.message_coverage.as_ref().unwrap().eligible_sessions,
            1
        );
        assert_eq!(
            result.activity_coverage.as_ref().unwrap().eligible_sessions,
            1
        );
        assert_eq!(result.activity_coverage.as_ref().unwrap().state, "partial");
        data.sessions[0].message_reasons = vec!["messagePartsUnavailable".into()];
        data.sessions[0].messages_supported = false;
        let result = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(result.totals.messages, None);
        assert_eq!(result.known_messages, None);
        assert_eq!(result.totals.active_ms, Some(60_000));
        assert_eq!(result.known_active_ms, Some(60_000));
        assert_eq!(result.top_sessions[0].messages, None);
        assert_eq!(result.top_sessions[0].active_ms, Some(60_000));
    }

    #[test]
    fn native_analysis_unknown_attribution_and_legacy_method_are_explicit() {
        let db = Database::memory().unwrap();
        let mut data = native_input("codex", "codex_session");
        data.native_activity = vec![
            native_event("s", "begin", 1_000),
            native_event("s", "end", 61_000),
        ];
        for (models, sources) in [
            (vec!["m".into()], vec![]),
            (vec![], vec!["codex_session".into()]),
        ] {
            let mut selected = query();
            selected.model_ids = models;
            selected.source_ids = sources;
            let result = build_analysis(selected, &data, &catalog(&db), &days()).unwrap();
            assert_eq!(result.totals.messages, None);
            assert_eq!(result.totals.active_ms, None);
            assert_eq!(result.known_messages, None);
            assert_eq!(result.known_active_ms, None);
        }
        data.codex_native_ready = false;
        data.source_observations.clear();
        data.native_activity.clear();
        let legacy = build_analysis(query(), &data, &catalog(&db), &days()).unwrap();
        assert_eq!(legacy.method_version, "legacy-v1");
        assert_eq!(legacy.totals.active_ms, Some(60_000));
    }

    #[test]
    fn native_analysis_incomplete_facts_keep_known_cost_and_mark_all_projections_partial() {
        let db = Database::memory().unwrap();
        let catalog = PriceCatalog::ingest_on(&db.conn.lock().unwrap(), "ofox.ai", &serde_json::json!({"data":[{"id":"m","pricing":{"prompt":"1","completion":"2","input_cache_read":"0","input_cache_write":"0"}}]}), 100_000).unwrap();
        for buckets in [[10, 2, 0, 0], [0; 4]] {
            let mut data = native_input("codex", "codex_session");
            data.facts[0].buckets = buckets;
            data.facts[0].complete = false;
            let expected = catalog.estimate("m", buckets, Some(1));
            let result = build_analysis(query(), &data, &catalog, &days()).unwrap();
            for estimate in [
                &result.totals.estimate,
                &result.series[1].estimate,
                &result.by_tool[0].estimate,
                &result.by_model[0].estimate,
                &result.top_sessions[0].estimate,
                &result.top_sessions[0].models[0].estimate,
            ] {
                assert_eq!(estimate.known_cost_usd, expected.known_cost_usd);
                assert_eq!(estimate.availability, "partial");
                assert_eq!(estimate.unpriced_record_count, None);
            }
            data.facts[0].model = "unpriced".into();
            data.facts[0].buckets = [10, 2, 0, 0];
            let unknown = build_analysis(query(), &data, &catalog, &days()).unwrap();
            assert_eq!(unknown.totals.estimate.availability, "unknown");
            assert!(unknown.totals.estimate.known_cost_usd.is_none());
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
        release_query_benchmark(false);
    }
    #[test]
    #[ignore = "opt-in release synthetic eight-source native-projection/API benchmark, not native history or UI"]
    fn session_analytics_release_eight_native_bridges_query_p95() {
        release_query_benchmark(true);
    }
    fn release_query_benchmark(mixed_native: bool) {
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
        let (mut data, mut query, _) = large_fixture();
        let sources = if mixed_native {
            vec![
                ("claude", "session_log"),
                ("codex", "codex_session"),
                ("gemini", "gemini_session"),
                ("opencode", "opencode_session"),
                ("workbuddy", "workbuddy_session"),
                ("pi", "pi_session"),
                ("dsh", "dsh_session"),
                ("openclaw", "openclaw_session"),
            ]
        } else {
            vec![("codex", "codex_session")]
        };
        if mixed_native {
            for (i, session) in data.sessions.iter_mut().enumerate() {
                session.identity.tool_id = sources[i / 1250].0.into();
            }
            for (i, fact) in data.facts.iter_mut().enumerate() {
                let (tool, source) = sources[i / 6250];
                fact.tool_id = tool.into();
                fact.source_id = source.into();
            }
            query.tool_ids = sources.iter().map(|(tool, _)| (*tool).into()).collect();
        }
        let envelopes = if mixed_native { 80 } else { 100 };
        let sessions_per_envelope = 10_000 / envelopes;
        let facts_per_envelope = 50_000 / envelopes;
        let runtime: Vec<_> = data
            .messages
            .iter()
            .map(
                |event| crate::services::session_native_activity::NativeActivityEvent {
                    session_key: event.session_key.clone(),
                    event_key: format!("runtime:{}", event.native_id),
                    at_ms: event.at_ms,
                    main: true,
                    stream_key: event.session_key.clone(),
                },
            )
            .collect();
        {
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction().unwrap();
            // Seed the same normalized body-free fixture used by the existing
            // aggregation regression into 100 cache envelopes. Native parsing
            // is deliberately outside the measured API's responsibility.
            for i in 0..envelopes {
                source_cache::merge_on(
                    &tx,
                    &format!("benchmark-{i:03}"),
                    sources[if mixed_native { i / 10 } else { 0 }].0,
                    None,
                    Part::Analytics,
                    &None,
                    Some(json!({
                        "version":1,"stamp":null,
                        "sessions":&data.sessions[i * sessions_per_envelope..(i + 1) * sessions_per_envelope],
                        "facts":&data.facts[i * facts_per_envelope..(i + 1) * facts_per_envelope],
                        "messages":&data.messages[i * facts_per_envelope..(i + 1) * facts_per_envelope],
                        "nativeActivity":&runtime[i * facts_per_envelope..(i + 1) * facts_per_envelope],
                        "nativeMethod":true,
                        "reasons":[],"available":true,"status":"ok","priority":0,
                        "attemptedAtMs":1,"succeededAtMs":1,
                    })),
                )
                .unwrap();
            }
            source_cache::merge_on(&tx,"analytics-codex-generation-v1","codex",None,Part::Analytics,&None,Some(json!({"version":1,"stamp":{"epoch":source_cache::ledger_epoch_on(&tx).unwrap(),"methodVersion":crate::services::codex_native::VERSION},"sessions":[],"facts":[],"messages":[],"reasons":[],"available":true,"status":"ok","priority":0,"attemptedAtMs":1,"succeededAtMs":1,"nativeMethod":true}))).unwrap();
            for (tool, source) in &sources {
                for channel in [Channel::Ledger, Channel::Analytics] {
                    let attempt = usage_health::begin_on(&tx, tool, source, channel, 1).unwrap();
                    assert!(usage_health::finish_on(&tx, &attempt, Outcome::Ok, 1).unwrap());
                }
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
        eprintln!("session analytics release warm API: os={} arch={} providers={sources:?} synthetic=true sessions=10000 facts=50000 messages=50000 cacheEnvelopes={envelopes} nativeActivity=50000 days=1460 warmups=3 rounds=30 p50={p50:?} p95={p95:?}; includes SQLite cache/catalog/health read, deduplication, aggregation, and frozen-cohort retention; excludes cold parsing, native source scan, actual historical completeness, IPC serialization/transport and UI rendering", std::env::consts::OS, std::env::consts::ARCH);
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
        assert_eq!(result.totals.median_tokens.as_deref(), Some("10"));
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
