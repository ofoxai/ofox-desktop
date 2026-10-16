//! Independent display evidence for one native thread. Never changes ledger IDs.
use super::super::usage_source::{NormalizedSnapshot, NormalizedUsageFact};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const VERSION: u8 = 1;
const MAX_MODELS: usize = 128;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Model {
    pub model: String,
    pub buckets: [u64; 4],
    pub records: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Projection {
    pub version: u8,
    pub native_thread_id: Option<String>,
    pub billing_session_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub state: String,
    pub reasons: Vec<String>,
    pub models: Vec<Model>,
    pub facts_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_digest: Option<String>,
}
impl Projection {
    pub(crate) fn version_supported(&self) -> bool {
        self.version == VERSION
    }
    pub(crate) fn has_current_proof(&self) -> bool {
        self.version_supported()
            && self
                .source_digest
                .as_ref()
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
    }
    pub(crate) fn missing(native: &str) -> Self {
        Self {
            version: VERSION,
            native_thread_id: Some(native.into()),
            billing_session_id: None,
            parent_session_id: None,
            state: "unavailable".into(),
            reasons: vec!["threadUsageNotIndexed".into()],
            models: vec![],
            facts_hash: None,
            source_digest: None,
        }
    }
    pub(crate) fn unavailable(mut self, reason: &str) -> Self {
        self.state = "unavailable".into();
        self.models.clear();
        self.facts_hash = None;
        self.reasons.push(reason.into());
        self.reasons.sort();
        self.reasons.dedup();
        self
    }
    pub(crate) fn valid(&self) -> bool {
        let mut total = [0u64; 4];
        let mut records = 0u64;
        let safe = self.models.iter().all(|model| {
            if model.model.is_empty() || model.model.len() > 512 {
                return false;
            }
            for (sum, value) in total.iter_mut().zip(model.buckets) {
                let Some(next) = sum.checked_add(value) else {
                    return false;
                };
                *sum = next;
            }
            let Some(next) = records.checked_add(model.records) else {
                return false;
            };
            records = next;
            true
        }) && total
            .into_iter()
            .try_fold(0u64, |sum, value| sum.checked_add(value))
            .is_some();
        self.has_current_proof()
            && matches!(self.state.as_str(), "available" | "unavailable")
            && self.reasons.len() <= 32
            && self
                .models
                .iter()
                .map(|m| &m.model)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.models.len()
            && self.models.len() <= MAX_MODELS
            && safe
            && self
                .native_thread_id
                .as_ref()
                .is_some_and(|id| !id.is_empty() && id.len() <= 512)
            && (self.state != "available"
                || (self.facts_hash.as_ref().is_some_and(|hash| {
                    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
                }) && !self.models.is_empty()))
    }
}
#[derive(Default)]
pub(crate) struct Proof {
    headers: usize,
    first: Option<Value>,
    ambiguous: bool,
    counters: bool,
    bad_counters: bool,
    zero_model: String,
    created_at: Option<i64>,
    first_counter: bool,
    source_digest: Option<String>,
}
fn id(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 512)
        .map(str::to_owned)
}
impl Proof {
    /// The refresh already read these bytes with a stable source receipt.
    /// Keep only their digest; equal token facts cannot prove equal histories.
    pub(crate) fn observe_source(&mut self, text: &str) {
        self.source_digest = Some(format!("{:x}", Sha256::digest(text.as_bytes())));
    }
    pub(crate) fn observe(&mut self, value: &Value) {
        let p = &value["payload"];
        if value["type"] == "session_meta" {
            self.headers += 1;
            if self.first.is_none() {
                // Retain only identity/provenance, never instructions or body.
                self.first = Some(
                    serde_json::json!({"id":p.get("id"),"session_id":p.get("session_id").or_else(||p.get("sessionId")),"parent":p.pointer("/source/subagent/thread_spawn/parent_thread_id")}),
                );
            }
            self.created_at = super::super::usage_source::timestamp_ms(
                value.get("timestamp").and_then(Value::as_str),
            );
            let source_ok = p.get("source").is_some_and(|source| {
                matches!(
                    source.as_str(),
                    Some("cli" | "vscode" | "exec" | "appServer")
                ) || id(source.pointer("/subagent/thread_spawn/parent_thread_id")).is_some()
            });
            self.ambiguous |= !source_ok
                || self.created_at.is_none()
                || self.headers != 1
                || p.get("forked_from_id").is_some_and(|v| !v.is_null())
                || p.get("subagent_history_start_ordinal")
                    .is_some_and(|v| !v.is_null());
        } else if value["type"] == "turn_context" {
            self.zero_model = id(p.get("model")).unwrap_or_else(|| "unknown".into());
        } else if value["type"] == "event_msg" && p["type"] == "token_count" {
            let info = &p["info"];
            if info.is_null() {
                return;
            }
            let total = info.get("total_token_usage").filter(|v| !v.is_null());
            let last = info.get("last_token_usage").filter(|v| !v.is_null());
            if total.is_none() && last.is_none() {
                return;
            }
            let header = self.first.as_ref();
            let native = id(header.and_then(|p| p.get("id")));
            let billing = id(header.and_then(|p| p.get("session_id")));
            let parent = id(header.and_then(|p| p.get("parent")));
            let child =
                parent.is_some() || billing.as_ref().is_some_and(|b| Some(b) != native.as_ref());
            // A child's first cumulative value can include the parent's seed.
            // A complete first-last counter is its only accepted local anchor.
            self.ambiguous |= self.headers != 1 || (!self.first_counter && child && last.is_none());
            self.first_counter = true;
            let at = super::super::usage_source::timestamp_ms(
                value.get("timestamp").and_then(Value::as_str),
            );
            self.ambiguous |= at
                .zip(self.created_at)
                .is_none_or(|(at, created)| at < created);
            for c in [total, last].into_iter().flatten() {
                let complete = c.is_object()
                    && c.get("input_tokens").and_then(Value::as_u64).is_some()
                    && c.get("output_tokens").and_then(Value::as_u64).is_some()
                    && c.get("cached_input_tokens")
                        .or_else(|| c.get("cache_read_input_tokens"))
                        .and_then(Value::as_u64)
                        .is_some()
                    && c.get("cache_write_input_tokens")
                        .is_none_or(|v| v.as_u64().is_some());
                self.counters |= complete;
                self.bad_counters |= !complete;
            }
        }
    }
    pub(crate) fn finish(&self, normalized: Option<&NormalizedSnapshot>) -> Projection {
        let header = self.first.as_ref();
        let mut result = Projection {
            version: VERSION,
            native_thread_id: id(header.and_then(|p| p.get("id"))),
            billing_session_id: id(header.and_then(|p| p.get("session_id"))),
            parent_session_id: id(header.and_then(|p| p.get("parent"))),
            state: "available".into(),
            reasons: vec![],
            models: vec![],
            facts_hash: None,
            source_digest: self.source_digest.clone(),
        };
        if self.ambiguous || self.headers != 1 || result.native_thread_id.is_none() {
            return result.unavailable("ambiguousThreadHistory");
        }
        let Some(snapshot) = normalized else {
            return result.unavailable("nativeUsageUnavailable");
        };
        // Native event IDs are absent for Codex. These source-local deltas do
        // not assert cross-file event ownership; replicas are compared whole.
        if snapshot
            .reasons
            .iter()
            .any(|r| r != "missingNativeEventIdentity")
        {
            return result.unavailable("threadUsageIncomplete");
        }
        if self.bad_counters || !self.counters {
            return result.unavailable("nativeUsageUnavailable");
        }
        let mut models: BTreeMap<String, Model> = BTreeMap::new();
        let mut hash = Sha256::new();
        hash.update(b"codex-native-thread-display-v1\0");
        for fact in &snapshot.facts {
            if fact.at_ms.is_none() || fact.model.is_empty() || fact.model.len() > 512 {
                return result.unavailable("threadUsageIncomplete");
            }
            let buckets = buckets(fact);
            let row = models.entry(fact.model.clone()).or_insert_with(|| Model {
                model: fact.model.clone(),
                buckets: [0; 4],
                records: 0,
            });
            for (sum, amount) in row.buckets.iter_mut().zip(buckets) {
                let Some(next) = sum.checked_add(amount) else {
                    return result.unavailable("threadUsageOverflow");
                };
                *sum = next;
            }
            let Some(records) = row.records.checked_add(1) else {
                return result.unavailable("threadUsageOverflow");
            };
            row.records = records;
            // Request IDs embed the physical filename and old billing SID; they
            // cannot prove that a different path is another paid event.
            hash.update(
                serde_json::to_vec(&(&fact.model, fact.at_ms, buckets))
                    .expect("serializable native display fact"),
            );
            if models.len() > MAX_MODELS {
                return result.unavailable("threadUsageBudgetExceeded");
            }
        }
        if models.is_empty() {
            let model = if self.zero_model.is_empty() {
                "unknown"
            } else {
                &self.zero_model
            };
            models.insert(
                model.into(),
                Model {
                    model: model.into(),
                    buckets: [0; 4],
                    records: 0,
                },
            );
        }
        result.models = models.into_values().collect();
        result.facts_hash = Some(format!("{:x}", hash.finalize()));
        if !result.valid() {
            return result.unavailable("threadUsageOverflow");
        }
        result
    }
}
fn buckets(fact: &NormalizedUsageFact) -> [u64; 4] {
    [
        fact.usage.input_tokens,
        fact.usage.output_tokens,
        fact.usage.cache_read_tokens,
        fact.usage.cache_creation_tokens,
    ]
    .map(u64::from)
}
