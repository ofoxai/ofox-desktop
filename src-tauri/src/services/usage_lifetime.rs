//! Cumulative source facts are independent of event-time usage and the ledger.
use super::{
    ofox_pricing::{CostEstimate, PriceCatalog, PricingCoverage},
    session_usage_hermes,
};
use crate::{
    database::Database,
    error::AppError,
    session_manager::source_cache::{self, Part},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(crate) fn resolve_hermes_root() -> Result<PathBuf, AppError> {
    // Keep the health check and override resolution in the same short barrier
    // as settings reload/update. Native reads happen after this guard drops.
    let _io = crate::config_io::lock();
    if crate::settings::settings_health().state == crate::settings::SettingsHealthState::Unreadable
    {
        return Err(AppError::Message(
            "usage.lifetime.settingsUnreadable".into(),
        ));
    }
    Ok(crate::hermes_config::get_hermes_dir())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LifetimeAmountKind {
    Known,
    LowerBound,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LifetimeCoverage {
    Complete,
    Partial,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifetimeBuckets {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// Main Tokens = fresh input + output. Cache buckets remain separate.
    pub total_tokens: Option<u64>,
    /// Native cumulative counter, not a number of attributable HTTP requests.
    pub api_call_count: Option<u64>,
    pub amount_kind: LifetimeAmountKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LifetimeCategory {
    Main,
    Voice,
    Auxiliary,
    Unattributed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifetimeModelFact {
    pub model_id: Option<String>,
    pub category: LifetimeCategory,
    pub buckets: LifetimeBuckets,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifetimeNativeSnapshot {
    pub version: u8,
    pub format: String,
    pub source_stamp: Value,
    pub session_records: u64,
    pub totals: LifetimeBuckets,
    pub models: Vec<LifetimeModelFact>,
    #[serde(default)]
    pub counter_coverage: LifetimeCoverage,
    #[serde(default)]
    pub model_coverage: LifetimeCoverage,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifetimeModelSummary {
    #[serde(flatten)]
    pub fact: LifetimeModelFact,
    pub estimate: CostEstimate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifetimeSummaryV1 {
    pub version: u8,
    pub tool_id: String,
    pub source_id: String,
    pub scope: String,
    pub snapshot_version: Option<String>,
    pub observed_at_ms: i64,
    /// Last stable native read; counter/model coverage may still be partial.
    /// Common source health separately tracks fully trustworthy synchronization.
    pub last_success_at_ms: Option<i64>,
    pub source_state: String,
    pub stale: bool,
    pub reasons: Vec<String>,
    pub session_records: Option<u64>,
    pub totals: LifetimeBuckets,
    pub by_model: Vec<LifetimeModelSummary>,
    pub counter_coverage: LifetimeCoverage,
    pub model_coverage: LifetimeCoverage,
    pub estimate: CostEstimate,
    pub pricing_coverage: PricingCoverage,
}

/// Reprices successful native facts without manufacturing event dates or IDs.
pub fn hermes_summary(
    db: &Database,
    root: &Path,
    apex: &str,
    now_ms: i64,
) -> Result<LifetimeSummaryV1, AppError> {
    let path = root.join("state.db").to_string_lossy().into_owned();
    let (expected, before) = load_cache(db, &path)?;
    let observed = match session_usage_hermes::current_stamp(root) {
        Ok(stamp)
            if before
                .last_success
                .as_ref()
                .is_some_and(|s| session_usage_hermes::same_stamp(&s.source_stamp, &stamp)) =>
        {
            before.last_success.clone().ok_or_else(invalid_cache)
        }
        Ok(_) => session_usage_hermes::read_lifetime(root),
        Err(error) => Err(error),
    };
    let mut next = before.clone();
    match observed {
        Ok(snapshot) => {
            next.source_state = "ready".into();
            next.source_error = None;
            if before.last_success.as_ref().is_none_or(|s| {
                !session_usage_hermes::same_stamp(&s.source_stamp, &snapshot.source_stamp)
            }) {
                next.last_success_at_ms = Some(now_ms);
            }
            next.last_success = Some(snapshot);
        }
        Err(error) => next.failed(&error),
    }
    // The native source may change while waiting for this machine-local DB
    // lock. Reject that snapshot as well; a later read may retry it.
    let conn = db.conn.lock().map_err(|_| invalid_cache())?;
    if next.source_state == "ready" {
        if let Some(snapshot) = &next.last_success {
            if let Err(error) = session_usage_hermes::verify_stamp(root, &snapshot.source_stamp) {
                next = before.clone();
                next.failed(&error);
            }
        }
    }
    let current = source_cache::read_on(&conn, &path, "hermes").map_err(|_| invalid_cache())?;
    source_cache::check_on(
        &conn,
        &path,
        "hermes",
        Part::Lifetime,
        &current.token(Part::Lifetime),
    )
    .map_err(|_| invalid_cache())?;
    let final_cache = if current.token(Part::Lifetime) != expected {
        // A late source read cannot replace a newer successful/failed attempt.
        decode_cache(current.value(Part::Lifetime))?
    } else {
        let changed = serde_json::to_value(&before).map_err(|_| invalid_cache())?
            != serde_json::to_value(&next).map_err(|_| invalid_cache())?;
        if changed || expected.is_none() {
            source_cache::merge_on(
                &conn,
                &path,
                "hermes",
                None,
                Part::Lifetime,
                &expected,
                Some(serde_json::to_value(&next).map_err(|_| invalid_cache())?),
            )
            .map_err(|_| invalid_cache())?;
        }
        next
    };
    let catalog = PriceCatalog::load_on(&conn, apex, now_ms)?;
    project_summary(final_cache, &catalog, now_ms)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifetimeCache {
    version: u8,
    source_state: String,
    source_error: Option<String>,
    last_success_at_ms: Option<i64>,
    last_success: Option<LifetimeNativeSnapshot>,
}
impl Default for LifetimeCache {
    fn default() -> Self {
        Self {
            version: 1,
            source_state: "missing".into(),
            source_error: None,
            last_success_at_ms: None,
            last_success: None,
        }
    }
}
impl LifetimeCache {
    fn failed(&mut self, error: &AppError) {
        let code = match error {
            AppError::Database(code) if code.starts_with("usage.hermes.") => code.as_str(),
            _ => "usage.hermes.sourceUnreadable",
        };
        self.source_state = if code == "usage.hermes.sourceMissing" {
            "missing"
        } else {
            "error"
        }
        .into();
        self.source_error = Some(code.into());
    }
}
fn invalid_cache() -> AppError {
    AppError::Database("usage.lifetime.invalidCache".into())
}

fn decode_cache(value: Option<&Value>) -> Result<LifetimeCache, AppError> {
    let Some(value) = value else {
        return Ok(LifetimeCache::default());
    };
    let cache: LifetimeCache =
        serde_json::from_value(value.clone()).map_err(|_| invalid_cache())?;
    if cache.version != 1
        || !matches!(cache.source_state.as_str(), "ready" | "missing" | "error")
        || cache
            .last_success
            .as_ref()
            .is_some_and(|s| s.version != 1 || s.format != "hermes-sqlite-schema31")
        || cache.last_success.is_some() != cache.last_success_at_ms.is_some()
    {
        return Err(invalid_cache());
    }
    Ok(cache)
}

fn load_cache(db: &Database, path: &str) -> Result<(Option<String>, LifetimeCache), AppError> {
    let conn = db.conn.lock().map_err(|_| invalid_cache())?;
    let parts = source_cache::read_on(&conn, path, "hermes").map_err(|_| invalid_cache())?;
    let token = parts.token(Part::Lifetime);
    source_cache::check_on(&conn, path, "hermes", Part::Lifetime, &token)
        .map_err(|_| invalid_cache())?;
    Ok((token, decode_cache(parts.value(Part::Lifetime))?))
}

fn estimate_model(catalog: &PriceCatalog, fact: &LifetimeModelFact) -> CostEstimate {
    let Some(model) = &fact.model_id else {
        return CostEstimate {
            unpriced_record_count: None,
            ..CostEstimate::default()
        };
    };
    let buckets = [
        fact.buckets.input_tokens,
        fact.buckets.output_tokens,
        fact.buckets.cache_read_tokens,
        fact.buckets.cache_write_tokens,
    ];
    let mut estimate = catalog.estimate(model, buckets.map(|v| v.unwrap_or(0)), None);
    if buckets.iter().any(Option::is_none) {
        if buckets.iter().all(|v| v.is_none_or(|n| n == 0)) {
            estimate.known_cost_usd = None;
        }
        estimate.availability = if estimate.known_cost_usd.is_some() {
            "partial"
        } else {
            "unknown"
        }
        .into();
    }
    estimate.unpriced_record_count = None;
    estimate
}

fn project_summary(
    cache: LifetimeCache,
    catalog: &PriceCatalog,
    now_ms: i64,
) -> Result<LifetimeSummaryV1, AppError> {
    let mut estimate = CostEstimate {
        unpriced_record_count: None,
        ..CostEstimate::default()
    };
    let mut unpriced = false;
    let mut by_model = Vec::new();
    let mut reasons = cache
        .last_success
        .as_ref()
        .map_or_else(Vec::new, |s| s.reasons.clone());
    if let Some(code) = &cache.source_error {
        reasons.push(code.clone());
    }
    if let Some(snapshot) = &cache.last_success {
        for fact in &snapshot.models {
            let model_estimate = estimate_model(catalog, fact);
            unpriced |= matches!(model_estimate.availability.as_str(), "unknown" | "partial");
            estimate.combine(&model_estimate)?;
            by_model.push(LifetimeModelSummary {
                fact: fact.clone(),
                estimate: model_estimate,
            });
        }
        unpriced |= snapshot.totals.amount_kind != LifetimeAmountKind::Known;
    }
    // Unknown-model residuals have no fake model identifier to put into the
    // shared estimator's unpriced-ID set, but still make the aggregate partial.
    if unpriced {
        estimate.availability = if estimate.known_cost_usd.is_some() {
            "partial"
        } else {
            "unknown"
        }
        .into();
    }
    estimate.unpriced_record_count = None;
    let snapshot_version = cache
        .last_success
        .as_ref()
        .map(|s| {
            serde_json::to_vec(s)
                .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
                .map_err(|_| invalid_cache())
        })
        .transpose()?;
    let snapshot = cache.last_success;
    let pricing_coverage = catalog.coverage_for(&estimate);
    Ok(LifetimeSummaryV1 {
        version: 1,
        tool_id: "hermes".into(),
        source_id: "hermes-lifetime".into(),
        scope: "lifetime".into(),
        snapshot_version,
        observed_at_ms: now_ms,
        last_success_at_ms: cache.last_success_at_ms,
        source_state: cache.source_state.clone(),
        stale: cache.source_state != "ready" && snapshot.is_some(),
        reasons,
        session_records: snapshot.as_ref().map(|s| s.session_records),
        totals: snapshot
            .as_ref()
            .map(|s| s.totals.clone())
            .unwrap_or_default(),
        by_model,
        counter_coverage: snapshot
            .as_ref()
            .map_or(LifetimeCoverage::Unknown, |s| s.counter_coverage),
        model_coverage: snapshot
            .as_ref()
            .map_or(LifetimeCoverage::Unknown, |s| s.model_coverage),
        estimate,
        pricing_coverage,
    })
}

#[cfg(test)]
mod tests;
