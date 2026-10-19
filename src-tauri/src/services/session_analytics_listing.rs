//! Body-free native thread totals shared by all rows of one listing snapshot.
use super::{
    session_analytics::AnalyticsInput, session_analytics_index::codex_thread_usage::Model,
};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadUsageCoverage {
    #[serde(default)]
    pub reasons: BTreeSet<String>,
    #[serde(default)]
    pub incomplete_models: BTreeSet<String>,
}

type ThreadModelTotals = (Vec<Model>, BTreeSet<String>, BTreeSet<String>);
pub(crate) type CurrentThreadModels = BTreeMap<String, ThreadModelTotals>;

#[cfg(test)]
thread_local! {
    static FACT_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn observe_fact() {
    #[cfg(test)]
    FACT_VISITS.with(|count| count.set(count.get() + 1));
}

/// The input already owns event deduplication and native thread association.
/// Never infer model completeness from a source-wide warning.
pub(crate) fn current_thread_models(
    input: &AnalyticsInput,
) -> Result<CurrentThreadModels, AppError> {
    #[derive(Default)]
    struct ThreadModels<'a> {
        models: BTreeMap<&'a str, Model>,
        incomplete: BTreeSet<String>,
        coverage_reasons: BTreeSet<String>,
    }
    let mut threads = BTreeMap::<&str, ThreadModels<'_>>::new();
    for fact in &input.facts {
        observe_fact();
        if fact.tool_id != "codex" {
            continue;
        }
        let Some(session) = fact.session_key.as_deref() else {
            continue;
        };
        let thread = threads.entry(session).or_default();
        let model = thread
            .models
            .entry(fact.model.as_str())
            .or_insert_with(|| Model {
                model: fact.model.clone(),
                buckets: [0; 4],
                records: 0,
            });
        for (sum, value) in model.buckets.iter_mut().zip(fact.buckets) {
            *sum = sum.checked_add(value).ok_or_else(overflow)?;
        }
        model.records = model.records.checked_add(1).ok_or_else(overflow)?;
        if !fact.complete {
            thread.incomplete.insert(fact.model.clone());
        }
    }
    // Reconciliation can discard one conflicting response while retaining a
    // sibling response with fully known counters. Preserve that thread's gap
    // separately from the surviving facts, including a discarded model that
    // no longer has a row in the effective event set.
    for (session, coverage) in &input.thread_usage_coverage {
        let thread = threads.entry(session.as_str()).or_default();
        thread
            .incomplete
            .extend(coverage.incomplete_models.iter().cloned());
        thread
            .coverage_reasons
            .extend(coverage.reasons.iter().cloned());
    }
    Ok(threads
        .into_iter()
        .map(|(session, thread)| {
            (
                session.to_owned(),
                (
                    thread.models.into_values().collect(),
                    thread.incomplete,
                    thread.coverage_reasons,
                ),
            )
        })
        .collect())
}

fn overflow() -> AppError {
    AppError::Database("usage.analytics.aggregateOverflow".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{session_analytics::SourceReason, session_analytics_core::UsageFact};

    fn input() -> AnalyticsInput {
        AnalyticsInput {
            snapshot_version: "frozen-listing".into(),
            sessions: vec![],
            facts: vec![],
            messages: vec![],
            reasons: vec![],
            source_reasons: vec![],
            sources: vec![],
            native_activity: vec![],
            source_observations: vec![],
            codex_native_ready: true,
            legacy_history_available: false,
            thread_usage_coverage: BTreeMap::new(),
        }
    }
    fn fact(session: Option<&str>, model: &str, n: u64, complete: bool) -> UsageFact {
        UsageFact {
            event_key: format!("{session:?}/{model}/{n}"),
            session_key: session.map(str::to_owned),
            tool_id: "codex".into(),
            source_id: "codex_session".into(),
            model: model.into(),
            at_ms: 1,
            buckets: [n, 2, 3, 4],
            complete,
            cache_write_1h: 0,
        }
    }

    #[test]
    fn native_listing_models_one_fact_pass_for_640_threads() {
        let mut input = input();
        for n in 0..640 {
            input.facts.push(fact(Some(&format!("s{n}")), "m", 1, true));
            input.facts.push(fact(Some(&format!("s{n}")), "m", 2, true));
        }
        FACT_VISITS.with(|count| count.set(0));
        let rows = current_thread_models(&input).unwrap();
        let visits = FACT_VISITS.with(std::cell::Cell::get);
        assert_eq!(rows.len(), 640);
        assert_eq!(rows["s639"].0[0].buckets, [3, 4, 6, 8]);
        assert_eq!(rows["s639"].0[0].records, 2);
        assert_eq!(
            visits,
            input.facts.len(),
            "listing size must not multiply the work needed to read one frozen fact set"
        );
    }

    #[test]
    fn native_listing_models_keep_thread_model_completeness_and_real_zero() {
        let mut input = input();
        let mut zero = fact(Some("child"), "free", 0, true);
        zero.buckets = [0; 4];
        input.facts = vec![
            fact(Some("root"), "same", 10, true),
            fact(Some("root"), "same", 5, false),
            fact(Some("root"), "complete", 7, true),
            fact(Some("child"), "same", 3, true),
            zero,
            fact(None, "unassociated", 100, false),
        ];
        let mut foreign = fact(Some("root"), "same", 999, false);
        foreign.tool_id = "workbuddy".into();
        input.facts.push(foreign);
        input.source_reasons.push(SourceReason {
            tool_id: "codex".into(),
            source_id: "codex_session".into(),
            reason: "missingNativeCounters".into(),
        });
        let rows = current_thread_models(&input).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows["root"].0[1].model, "same");
        assert_eq!(rows["root"].0[1].buckets, [15, 4, 6, 8]);
        assert_eq!(rows["root"].1, BTreeSet::from(["same".into()]));
        assert!(rows["child"].1.is_empty());
        assert_eq!(rows["child"].0[0].model, "free");
        assert_eq!(rows["child"].0[0].buckets, [0; 4]);
        assert_eq!(rows["child"].0[0].records, 1);
    }

    #[test]
    fn native_listing_models_overflow_is_not_saturated_or_silently_dropped() {
        let mut input = input();
        input.facts = vec![
            fact(Some("thread"), "m", u64::MAX, true),
            fact(Some("thread"), "m", 1, true),
        ];
        assert!(current_thread_models(&input).is_err());
    }

    #[test]
    fn native_listing_reconciled_thread_coverage_keeps_surviving_complete_fact() {
        let mut input = input();
        input.facts = vec![
            fact(Some("partial"), "m", 5, true),
            fact(Some("clean"), "m", 7, true),
        ];
        input.thread_usage_coverage.insert(
            "partial".into(),
            ThreadUsageCoverage {
                reasons: BTreeSet::from(["conflictingUsageReplica".into()]),
                incomplete_models: BTreeSet::from(["m".into()]),
            },
        );
        input.thread_usage_coverage.insert(
            "no-survivor".into(),
            ThreadUsageCoverage {
                reasons: BTreeSet::from(["unprovenLegacyReplica".into()]),
                incomplete_models: BTreeSet::from(["missing-model".into()]),
            },
        );
        input.source_reasons.push(SourceReason {
            tool_id: "codex".into(),
            source_id: "codex_session".into(),
            reason: "conflictingUsageReplica".into(),
        });
        let rows = current_thread_models(&input).unwrap();
        assert_eq!(rows["partial"].0[0].buckets, [5, 2, 3, 4]);
        assert_eq!(rows["partial"].0[0].records, 1);
        assert!(input.facts.iter().all(|fact| fact.complete));
        assert_eq!(rows["partial"].1, BTreeSet::from(["m".into()]));
        assert_eq!(
            rows["partial"].2,
            BTreeSet::from(["conflictingUsageReplica".into()])
        );
        assert!(rows["clean"].1.is_empty());
        assert!(rows["clean"].2.is_empty());
        assert!(rows["no-survivor"].0.is_empty());
        assert_eq!(
            rows["no-survivor"].1,
            BTreeSet::from(["missing-model".into()])
        );
        assert_eq!(
            rows["no-survivor"].2,
            BTreeSet::from(["unprovenLegacyReplica".into()])
        );
    }
}
