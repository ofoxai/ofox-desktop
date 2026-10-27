//! How a run shows up in `session.json`: streamed text in batches, usage
//! totals, the runtime's native history bound to the session's messages, and
//! the state a run leaves when it ends or its process died.
use super::{
    catalog::SearchPrice,
    host::{scrub_secrets, scrub_secrets_in, RunControl, PROTOCOL},
    store::{AssistantStore, SessionRecord},
    types::*,
};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

/// The run's message, session and ledger row as the run ended. A run that
/// did not complete makes the session's cost unknown, unless it failed
/// without being billed (`unbilled`).
pub(super) fn settle_run(
    record: &mut SessionRecord,
    control: &RunControl,
    completed: bool,
    stopped: bool,
    unbilled: bool,
) -> Result<(Option<Message>, Session), String> {
    let message = record
        .messages
        .iter_mut()
        .find(|m| m.id == control.message_id)
        .map(|message| {
            message.status = if completed {
                "completed"
            } else if stopped {
                "stopped"
            } else {
                "error"
            }
            .into();
            message.clone()
        });
    if record
        .session
        .active_run
        .as_ref()
        .is_some_and(|r| r.run_id == control.run_id)
    {
        record.session.active_run = None;
    }
    if !completed && !unbilled {
        ensure_object(&mut record.usage);
        record.usage["usageCoverage"] = json!("incomplete");
        record.usage["estimatedCostUsd"] = Value::Null;
    }
    for op in record
        .operations
        .iter_mut()
        .filter(|op| op.id == control.operation_id)
    {
        op.status = if completed {
            OperationStatus::Completed
        } else if stopped {
            OperationStatus::Cancelled
        } else {
            OperationStatus::Failed
        };
    }
    Ok((message, record.session.clone()))
}

pub(super) fn is_dangling(record: &SessionRecord) -> bool {
    record.session.active_run.is_some()
        || record
            .operations
            .iter()
            .any(|op| op.status == OperationStatus::Running)
        || record.messages.iter().any(|m| m.status == "streaming")
}

/// A run of a previous process: take what its native history kept, then mark
/// it interrupted.
pub(super) fn settle_interrupted(
    record: &mut SessionRecord,
    entries: Option<&Value>,
) -> Result<(), String> {
    ensure_object(&mut record.usage);
    record.usage["usageCoverage"] = json!("incomplete");
    if let Some(entries) = entries {
        merge_native(record, entries, &[])?;
    }
    for message in &mut record.messages {
        if message.status == "streaming" {
            message.status = "stopped".into();
        }
    }
    record.session.active_run = None;
    for op in &mut record.operations {
        if op.status == OperationStatus::Running {
            op.status = OperationStatus::Interrupted;
        }
    }
    Ok(())
}

pub(super) fn ensure_object(value: &mut Value) {
    if !value.is_object() {
        *value = json!({});
    }
}

/// Streamed text, saved in batches: UI events go out at once, `session.json`
/// is rewritten at most every 500 ms (or each 64 KiB) and at the end.
pub(super) struct ProjectionBuffer {
    pending: Pending,
    last_flush: std::time::Instant,
}

#[derive(Default)]
pub(super) struct Pending {
    text: String,
    thinking: String,
}

impl Default for ProjectionBuffer {
    fn default() -> Self {
        Self {
            pending: Pending::default(),
            last_flush: std::time::Instant::now(),
        }
    }
}

impl ProjectionBuffer {
    pub(super) fn push(&mut self, run: &RunControl, kind: &str, data: &mut Value) {
        scrub_secrets_in(data, &run.secrets());
        data["messageId"] = json!(run.message_id);
        if let Some(delta) = data["delta"].as_str() {
            if kind == "text_delta" {
                self.pending.text.push_str(delta);
            } else {
                self.pending.thinking.push_str(delta);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.pending.text.is_empty() && self.pending.thinking.is_empty()
    }

    pub(super) fn due(&self) -> bool {
        !self.is_empty()
            && (self.last_flush.elapsed() >= Duration::from_millis(500)
                || self.pending.text.len() + self.pending.thinking.len() >= 64 * 1024)
    }

    pub(super) fn take(&mut self) -> Option<Pending> {
        if self.is_empty() {
            return None;
        }
        self.last_flush = std::time::Instant::now();
        Some(std::mem::take(&mut self.pending))
    }
}

/// Append streamed text to the run's message. Blocking file IO.
pub(super) fn save_projection(
    store: &AssistantStore,
    sid: &str,
    mid: &str,
    pending: &Pending,
    secrets: &[String],
) -> Result<(), String> {
    store.mutate(sid, |record| {
        let message = record
            .messages
            .iter_mut()
            .find(|m| m.id == mid)
            .ok_or("assistant_message_not_found")?;
        // The whole text again: a key split across two batches is whole now.
        message.content.push_str(&pending.text);
        message.content = scrub_secrets(&message.content, secrets);
        if !pending.thinking.is_empty() {
            let reasoning = message.reasoning.get_or_insert_default();
            reasoning.push_str(&pending.thinking);
            *reasoning = scrub_secrets(reasoning, secrets);
        }
        record.session.updated_at = now();
        Ok(())
    })
}

/// Add one runtime usage report to the session totals until the native
/// history recounts them at the end of the run.
pub(super) fn add_usage(totals: &mut Value, usage: &Value) {
    ensure_object(totals);
    for (target, source) in [
        ("inputTokens", "input"),
        ("outputTokens", "output"),
        ("cacheReadTokens", "cacheRead"),
        ("cacheWriteTokens", "cacheWrite"),
    ] {
        totals[target] = json!(totals[target]
            .as_u64()
            .unwrap_or(0)
            .saturating_add(usage[source].as_u64().unwrap_or(0)));
    }
    totals["estimatedCostUsd"] = Value::Null;
    totals["billedCostUsd"] = Value::Null;
    totals["searchCostUsd"] = Value::Null;
}

/// Bind the runtime's native entries to the session's messages. Native
/// history is the authority for content, branch (superseded) and status.
pub(super) fn merge_native(
    record: &mut SessionRecord,
    entries: &Value,
    secrets: &[String],
) -> Result<(), String> {
    let entries = entries.as_array().ok_or(PROTOCOL)?;
    for entry in entries {
        let Some(native_id) = entry["id"].as_str() else {
            continue;
        };
        let Some(role) = entry["role"]
            .as_str()
            .filter(|r| matches!(*r, "user" | "assistant"))
        else {
            continue;
        };
        let run_id = entry["runId"]
            .as_str()
            .ok_or("assistant_native_run_missing")?;
        if !record
            .messages
            .iter()
            .any(|m| m.run_id.as_deref() == Some(run_id))
        {
            return Err("assistant_native_unknown_run".into());
        }
        let existing = record
            .messages
            .iter()
            .position(|m| m.native_entry_id.as_deref() == Some(native_id));
        let final_message = role == "user" || entry["final"] != false;
        let target = existing.or_else(|| {
            final_message
                .then(|| {
                    record.messages.iter().position(|m| {
                        m.run_id.as_deref() == Some(run_id)
                            && m.role == role
                            && m.native_entry_id.is_none()
                    })
                })
                .flatten()
        });
        let content = scrub_secrets(entry["content"].as_str().unwrap_or(""), secrets);
        let thinking = entry["thinking"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(|t| scrub_secrets(t, secrets));
        let status = if entry["stopReason"] == "error" {
            "error"
        } else if entry["stopReason"] == "aborted" {
            "stopped"
        } else {
            "completed"
        };
        if let Some(index) = target {
            let m = &mut record.messages[index];
            m.native_entry_id = Some(native_id.into());
            m.superseded = entry["active"] == false;
            if role == "assistant" {
                m.content = content;
            }
            m.reasoning = thinking;
            m.status = status.into();
            if let Some(timestamp) = entry["timestamp"].as_i64() {
                m.created_at = timestamp;
            }
        } else {
            record.messages.push(Message {
                id: id(),
                role: role.into(),
                content,
                created_at: entry["timestamp"].as_i64().unwrap_or_else(now),
                reasoning: thinking,
                status: status.into(),
                attachments: vec![],
                sources: vec![],
                superseded: entry["active"] == false,
                revision_of: None,
                native_entry_id: Some(native_id.into()),
                run_id: Some(run_id.into()),
            });
        }
    }
    // Native history is authoritative even when the last UI event was lost.
    for message in &mut record.messages {
        if message.status == "streaming" {
            message.status = "stopped".into();
        }
    }
    // A reply the native history does not hold (its stream was cut) keeps
    // its place after its question, which now carries the native time.
    let asked: HashMap<String, i64> = record
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .filter_map(|m| Some((m.run_id.clone()?, m.created_at)))
        .collect();
    for message in &mut record.messages {
        if message.role == "assistant" && message.native_entry_id.is_none() {
            if let Some(asked) = message.run_id.as_ref().and_then(|run| asked.get(run)) {
                message.created_at = message.created_at.max(asked + 1);
            }
        }
    }
    record.messages.sort_by_key(|m| m.created_at);
    let prices = search_prices(&record.operations);
    project_usage(&mut record.usage, entries, &prices);
    Ok(())
}

/// The search price each run was given, by run id.
fn search_prices(operations: &[Operation]) -> HashMap<&str, &SearchPrice> {
    operations
        .iter()
        .filter_map(|op| Some((op.run_id.as_str(), op.search_price.as_ref()?)))
        .collect()
}

/// One attachment's outcome as a run read it (assistant-runtime README,
/// "Outcome events").
#[derive(Clone, Debug, PartialEq)]
pub(super) struct AttachmentOutcome {
    pub id: String,
    /// One of [`ATTACHMENT_STATUSES`].
    pub status: String,
    /// `assistant_attachment_partial` or the blocking `assistant_*` code.
    pub code: Option<String>,
    pub details: Option<Value>,
}

impl AttachmentOutcome {
    /// The outcome an `attachment` event carries; anything else is a
    /// protocol error.
    pub(super) fn parse(data: &Value) -> Result<Self, String> {
        let id = data["id"].as_str().filter(|id| !id.is_empty());
        let status = data["status"]
            .as_str()
            .filter(|status| ATTACHMENT_STATUSES.contains(status));
        let code = match &data["code"] {
            Value::Null => None,
            Value::String(code) if is_assistant_code(code) => Some(code.clone()),
            _ => return Err(PROTOCOL.into()),
        };
        let details = match &data["details"] {
            Value::Null => None,
            details @ Value::Object(_) => Some(details.clone()),
            _ => return Err(PROTOCOL.into()),
        };
        match (id, status) {
            (Some(id), Some(status)) => Ok(Self {
                id: id.into(),
                status: status.into(),
                code,
                details,
            }),
            _ => Err(PROTOCOL.into()),
        }
    }

    /// The `attachment` event the renderer gets.
    pub(super) fn to_event(&self) -> Value {
        json!({
            "id": self.id, "status": self.status, "code": self.code, "details": self.details,
        })
    }
}

/// An `assistant_*` code: lowercase letters, digits and underscores.
fn is_assistant_code(code: &str) -> bool {
    code.len() <= 100
        && code.strip_prefix("assistant_").is_some_and(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

/// Keep a run's outcome for one file: on the conversation's file (its last
/// outcome) and on that file of the run's user message (this turn's). Only a
/// file the run's user message sent takes it; for any other nothing changes
/// and `false` is returned.
pub(super) fn record_attachment_outcome(
    record: &mut SessionRecord,
    run_id: &str,
    outcome: &AttachmentOutcome,
) -> bool {
    let sent = record
        .messages
        .iter()
        .filter(|m| m.role == "user" && m.run_id.as_deref() == Some(run_id))
        .flat_map(|m| m.attachments.iter())
        .any(|file| file.id == outcome.id);
    if !sent {
        return false;
    }
    let turn = record
        .messages
        .iter_mut()
        .filter(|m| m.role == "user" && m.run_id.as_deref() == Some(run_id))
        .flat_map(|m| m.attachments.iter_mut());
    for file in record
        .attachments
        .iter_mut()
        .chain(turn)
        .filter(|file| file.id == outcome.id)
    {
        file.status = outcome.status.clone();
        file.reason = outcome.code.clone();
        file.details = outcome.details.clone();
    }
    true
}

/// Recount the session's usage from the native history. One unpriced or
/// incomplete request or search leaves the cost unknown for good. Searches
/// (the runtime's `web_search` tool entries) are priced with the price their
/// run was given (`search_prices`, by run id): `searchCount` paid searches
/// cost `searchCostUsd`, which `estimatedCostUsd` includes.
pub(super) fn project_usage(
    totals: &mut Value,
    entries: &[Value],
    search_prices: &HashMap<&str, &SearchPrice>,
) {
    ensure_object(totals);
    let (mut input, mut output, mut cache_read, mut cache_write) = (0u64, 0u64, 0u64, 0u64);
    let mut estimate = 0.0;
    let mut priced = totals["usageCoverage"] != "incomplete";
    let mut count = 0u64;
    let (mut searches, mut search_cost) = (0u64, Some(Decimal::ZERO));
    for entry in entries
        .iter()
        .filter(|e| e["role"] == "tool" && e["toolName"] == "web_search")
    {
        // An unknown outcome, or a search whose tokens went unreported.
        if entry["usageCoverage"] == "incomplete" {
            priced = false;
            totals["usageCoverage"] = json!("incomplete");
        }
        let usage = &entry["details"]["usage"];
        let count_of = |key: &str| usage[key].as_u64().unwrap_or(0);
        let (calls, tokens_in, tokens_out) = (
            count_of("searchCalls"),
            count_of("inputTokens"),
            count_of("outputTokens"),
        );
        searches = searches.saturating_add(calls);
        // A failed or repeated search cost nothing.
        if calls == 0 && tokens_in == 0 && tokens_out == 0 {
            continue;
        }
        let cost = entry["runId"]
            .as_str()
            .and_then(|run| search_prices.get(run))
            .and_then(|price| price.cost(calls, tokens_in, tokens_out));
        search_cost = search_cost
            .zip(cost)
            .and_then(|(sum, cost)| sum.checked_add(cost));
    }
    for entry in entries
        .iter()
        .filter(|e| e["role"] == "assistant" || e["role"] == "usage")
    {
        let usage = &entry["usage"];
        if entry["usageCoverage"] == "incomplete" || !usage.is_object() {
            priced = false;
            totals["usageCoverage"] = json!("incomplete");
        }
        if !usage.is_object() {
            continue;
        }
        count += 1;
        input += usage["input"].as_u64().unwrap_or(0);
        output += usage["output"].as_u64().unwrap_or(0);
        cache_read += usage["cacheRead"].as_u64().unwrap_or(0);
        cache_write += usage["cacheWrite"].as_u64().unwrap_or(0);
        if entry["pricingKnown"] == true && entry["usageCoverage"] != "incomplete" {
            estimate += usage["cost"]["total"].as_f64().unwrap_or(0.0);
        } else {
            priced = false;
        }
    }
    totals["inputTokens"] = json!(input);
    totals["outputTokens"] = json!(output);
    totals["cacheReadTokens"] = json!(cache_read);
    totals["cacheWriteTokens"] = json!(cache_write);
    let search_cost = search_cost.filter(|_| priced && count > 0);
    let total = search_cost.and_then(|search| {
        if search.is_zero() {
            return Some(estimate.to_string());
        }
        // Rust prints an f64 in plain decimal notation, never as 1e-7.
        let tokens = estimate.to_string().parse::<Decimal>().ok()?;
        Some(tokens.checked_add(search)?.normalize().to_string())
    });
    totals["estimatedCostUsd"] = json!(total);
    totals["billedCostUsd"] = Value::Null;
    totals["modelRequests"] = json!(count);
    totals["searchCount"] = json!(searches);
    totals["searchCostUsd"] = json!(search_cost.map(|cost| cost.normalize().to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::assistant::host::Identity;

    fn store() -> (tempfile::TempDir, AssistantStore) {
        let config = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(config.path(), &"ef".repeat(32)).unwrap();
        (config, store)
    }

    fn message(id: &str, role: &str, run: &str, created_at: i64) -> Message {
        serde_json::from_value(json!({
            "id": id, "role": role, "content": "partial", "createdAt": created_at,
            "status": "streaming", "runId": run,
        }))
        .unwrap()
    }

    fn control(sid: &str) -> RunControl {
        let request: SendRequest = serde_json::from_value(json!({
            "sessionId": sid, "runId": id(), "operationId": id(), "text": "hi",
        }))
        .unwrap();
        RunControl::new(
            Identity {
                owner: "ef".repeat(32),
                apex: "ofox.ai".into(),
            },
            &request,
            "test-opaque-credential".into(),
        )
    }

    #[test]
    fn streaming_projection_batches_writes_and_flushes_the_last_text_and_reasoning() {
        let (_config, store) = store();
        let sid = store.create("test", false).unwrap().session.id;
        let run = control(&sid);
        store
            .mutate(&sid, |record| {
                let mut reply = message(&run.message_id, "assistant", &run.run_id, 1);
                reply.content.clear();
                record.messages.push(reply);
                Ok(())
            })
            .unwrap();
        let file = store
            .native_history_path(&sid)
            .unwrap()
            .with_file_name("session.json");
        let before = std::fs::read(&file).unwrap();

        let mut buffer = ProjectionBuffer::default();
        for _ in 0..10_000 {
            let mut data = json!({"delta": "x"});
            buffer.push(&run, "text_delta", &mut data);
            assert_eq!(data["messageId"], run.message_id);
        }
        assert!(
            !buffer.due(),
            "10 000 small deltas inside 500 ms stay in memory"
        );
        assert_eq!(std::fs::read(&file).unwrap(), before);
        buffer.last_flush -= Duration::from_millis(500);
        assert!(buffer.due());
        let pending = buffer.take().unwrap();
        save_projection(&store, &sid, &run.message_id, &pending, &run.secrets()).unwrap();
        let mut data = json!({"delta": "last test-opaque-credential"});
        buffer.push(&run, "text_delta", &mut data);
        assert!(!data.to_string().contains("test-opaque-credential"));
        buffer.push(&run, "thinking_delta", &mut json!({"delta": "reasoning"}));
        let pending = buffer.take().unwrap();
        assert!(buffer.take().is_none(), "nothing left to save twice");
        save_projection(&store, &sid, &run.message_id, &pending, &run.secrets()).unwrap();

        let saved = store.load_detail(&sid).unwrap();
        let content = &saved.messages[0].content;
        assert!(content.starts_with(&"x".repeat(10_000)));
        assert!(content.ends_with(crate::redact::REDACTED), "{content}");
        assert_eq!(saved.messages[0].reasoning.as_deref(), Some("reasoning"));
    }

    #[test]
    fn a_large_burst_is_saved_before_500_ms() {
        let (_config, store) = store();
        let run = control(&store.create("test", false).unwrap().session.id);
        let mut buffer = ProjectionBuffer::default();
        buffer.push(
            &run,
            "text_delta",
            &mut json!({"delta": "y".repeat(64 * 1024)}),
        );
        assert!(buffer.due());
    }

    #[test]
    fn compaction_usage_is_counted_and_unknown_coverage_stays_unknown() {
        let mut usage = json!({});
        let entries = vec![
            json!({"role":"assistant","pricingKnown":true,"usage":{"input":10,"output":5,"cost":{"total":0}}}),
            json!({"role":"usage","pricingKnown":true,"usageCoverage":"complete","usage":{"input":20,"output":3,"cost":{"total":0.1}}}),
        ];
        let prices = HashMap::new();
        project_usage(&mut usage, &entries, &prices);
        assert_eq!(usage["inputTokens"], 30);
        assert_eq!(usage["outputTokens"], 8);
        assert_eq!(usage["estimatedCostUsd"], "0.1");
        assert_eq!(usage["modelRequests"], 2);
        project_usage(
            &mut usage,
            &[json!({"role":"usage","usage":null,"usageCoverage":"incomplete"})],
            &prices,
        );
        assert!(usage["estimatedCostUsd"].is_null());
        project_usage(&mut usage, &entries, &prices);
        assert!(
            usage["estimatedCostUsd"].is_null(),
            "a later complete response cannot erase an unaccounted interrupted request"
        );
    }

    fn search_price() -> SearchPrice {
        serde_json::from_value(json!({
            "perSearch": "0.014", "perInputToken": "0.00000005", "perOutputToken": "0.0000004",
        }))
        .unwrap()
    }

    /// A `web_search` tool entry of `run` as the runtime projects it.
    fn search_entry(run: &str, details: Value, coverage: &str, is_error: bool) -> Value {
        json!({
            "role": "tool", "toolName": "web_search", "toolCallId": id(), "runId": run,
            "isError": is_error, "details": details, "usageCoverage": coverage,
        })
    }

    fn searched(calls: u64, input: u64, output: u64) -> Value {
        json!({
            "usage": {"searchCalls": calls, "inputTokens": input, "outputTokens": output,
                      "model": "openai/gpt-5-nano"},
            "sources": [{"title": "Pi", "url": "https://pi.dev/"}],
        })
    }

    fn model_entry(run: &str) -> Value {
        json!({"role": "assistant", "runId": run, "pricingKnown": true,
               "usage": {"input": 10, "output": 5, "cost": {"total": 0.1}}})
    }

    #[test]
    fn searches_are_counted_and_priced_at_their_runs_price_on_top_of_the_model_cost() {
        let price = search_price();
        let prices = HashMap::from([("run1", &price)]);
        let mut repeated = searched(0, 0, 0);
        repeated["deduplicated"] = json!(true);
        let entries = vec![
            model_entry("run1"),
            search_entry("run1", searched(1, 1000, 500), "complete", false),
            // The same query again: answered from the first, at no cost.
            search_entry("run1", repeated, "complete", false),
            // A refused search cost nothing.
            search_entry("run1", json!({}), "complete", true),
            search_entry("run1", searched(1, 2000, 100), "complete", false),
        ];
        let mut usage = json!({});
        project_usage(&mut usage, &entries, &prices);
        assert_eq!(usage["searchCount"], 2);
        // 2 × 0.014 + 3000 × 0.00000005 + 600 × 0.0000004
        assert_eq!(usage["searchCostUsd"], "0.02839");
        assert_eq!(usage["estimatedCostUsd"], "0.12839");
        assert_eq!(usage["modelRequests"], 1);
        assert_eq!(
            usage["inputTokens"], 10,
            "search tokens are not model tokens"
        );
        assert!(usage.get("usageCoverage").is_none());

        // No search: the model cost as before, and a known zero for searches.
        let mut plain = json!({});
        project_usage(&mut plain, &[model_entry("run1")], &prices);
        assert_eq!(plain["estimatedCostUsd"], "0.1");
        assert_eq!(plain["searchCount"], 0);
        assert_eq!(plain["searchCostUsd"], "0");
    }

    #[test]
    fn a_search_with_an_unknown_outcome_leaves_the_cost_unknown_for_good() {
        let price = search_price();
        let prices = HashMap::from([("run1", &price)]);
        let unknown = search_entry(
            "run1",
            json!({"code": "unknown_outcome", "error": {"code": "search_outcome_unknown"}}),
            "incomplete",
            false,
        );
        let mut usage = json!({});
        project_usage(&mut usage, &[model_entry("run1"), unknown], &prices);
        assert_eq!(usage["usageCoverage"], "incomplete");
        assert!(usage["estimatedCostUsd"].is_null());
        assert!(usage["searchCostUsd"].is_null());
        assert_eq!(usage["searchCount"], 0);
        project_usage(&mut usage, &[model_entry("run1")], &prices);
        assert!(usage["estimatedCostUsd"].is_null(), "still unknown");
        assert!(usage["searchCostUsd"].is_null());
    }

    #[test]
    fn a_search_without_a_price_for_its_run_has_no_known_cost() {
        let entries = [
            model_entry("run1"),
            search_entry("run1", searched(1, 10, 10), "complete", false),
        ];
        let mut usage = json!({});
        project_usage(&mut usage, &entries, &HashMap::new());
        assert_eq!(usage["searchCount"], 1);
        assert!(usage["estimatedCostUsd"].is_null());
        assert!(usage["searchCostUsd"].is_null());
        // Its tokens and calls are known; only the price is not.
        assert!(usage.get("usageCoverage").is_none());
    }

    #[test]
    fn each_run_prices_its_searches_with_the_price_it_was_given() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        let mut dearer = search_price();
        dearer.per_search = "0.02".parse().unwrap();
        record.messages = vec![
            message("u1", "user", "run1", 1),
            message("u2", "user", "run2", 2),
        ];
        for (run, price) in [("run1", Some(search_price())), ("run2", Some(dearer))] {
            record.operations.push(Operation {
                id: id(),
                run_id: run.into(),
                status: OperationStatus::Completed,
                search_price: price,
            });
        }
        let entries = json!([
            search_entry("run1", searched(1, 0, 0), "complete", false),
            search_entry("run2", searched(1, 0, 0), "complete", false),
            model_entry("run2"),
        ]);
        merge_native(&mut record, &entries, &[]).unwrap();
        assert_eq!(record.usage["searchCount"], 2);
        assert_eq!(record.usage["searchCostUsd"], "0.034");
        // Tool entries are not messages.
        assert_eq!(record.messages.len(), 2);
    }

    #[test]
    fn native_history_binds_each_run_instead_of_claiming_previous_turns() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&sid, |record| {
                for (mid, role, run) in [
                    ("u1", "user", "run1"),
                    ("a1", "assistant", "run1"),
                    ("u2", "user", "run2"),
                    ("a2", "assistant", "run2"),
                ] {
                    record.messages.push(message(mid, role, run, 1));
                }
                merge_native(
                    record,
                    &json!([
                        {"id":"native-u1","role":"user","runId":"run1","content":"old question","active":true,"timestamp":10},
                        {"id":"native-a1","role":"assistant","runId":"run1","content":"old answer","active":true,"timestamp":11},
                        {"id":"native-u2","role":"user","runId":"run2","content":"new question","active":true,"timestamp":12},
                        {"id":"native-a2","role":"assistant","runId":"run2","content":"new answer","active":true,"timestamp":13}
                    ]),
                    &[],
                )
            })
            .unwrap();
        let saved = store.load_detail(&sid).unwrap();
        let bound: Vec<_> = saved
            .messages
            .iter()
            .map(|m| (m.id.as_str(), m.native_entry_id.as_deref().unwrap()))
            .collect();
        assert_eq!(
            bound,
            [
                ("u1", "native-u1"),
                ("a1", "native-a1"),
                ("u2", "native-u2"),
                ("a2", "native-a2")
            ]
        );
        assert_eq!(saved.messages[3].content, "new answer");
        assert!(saved.messages.iter().all(|m| m.status == "completed"));
    }

    #[test]
    fn a_cut_reply_stays_after_its_question() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&sid, |record| {
                record.messages.push(message("u1", "user", "run1", 100));
                record.messages.push(message("a1", "assistant", "run1", 100));
                // Only the question reached the native history before the crash.
                settle_interrupted(
                    record,
                    Some(&json!([{"id":"native-u1","role":"user","runId":"run1","content":"q","active":true,"timestamp":500}])),
                )
            })
            .unwrap();
        let saved = store.load_record(&sid).unwrap();
        assert_eq!(
            saved
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["u1", "a1"]
        );
        assert_eq!(saved.messages[1].status, "stopped");
        assert_eq!(saved.usage["usageCoverage"], "incomplete");
    }

    #[test]
    fn a_native_entry_of_a_run_the_session_never_had_is_refused() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let result = store.mutate(&sid, |record| {
            merge_native(
                record,
                &json!([{"id":"n","role":"user","runId":"foreign","content":"x"}]),
                &[],
            )
        });
        assert_eq!(result.unwrap_err(), "assistant_native_unknown_run");
        assert!(store.load_detail(&sid).unwrap().messages.is_empty());
    }

    fn file(id: &str) -> Attachment {
        serde_json::from_value(json!({
            "id": id, "name": "scan.pdf", "mimeType": "application/pdf", "sizeBytes": 9,
            "status": "ready",
        }))
        .unwrap()
    }

    #[test]
    fn an_outcome_is_kept_on_the_file_and_on_the_turn_that_read_it() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let (earlier, now_run) = (id(), id());
        let (pdf, other) = (id(), id());
        let mut asked_before = message(&id(), "user", &earlier, 1);
        asked_before.attachments = vec![file(&pdf)];
        let mut asked = message(&id(), "user", &now_run, 2);
        asked.attachments = vec![file(&other), file(&pdf)];
        let mut record = store.load_record(&sid).unwrap();
        record.messages = vec![asked_before, asked];
        record.attachments = vec![file(&pdf), file(&other)];

        let outcome = AttachmentOutcome::parse(&json!({
            "id": pdf, "status": "partial", "code": "assistant_attachment_partial",
            "details": {"format": "pdf", "pageCount": 3, "pagesWithoutText": [2]},
        }))
        .unwrap();
        record_attachment_outcome(&mut record, &now_run, &outcome);

        let read = |a: &Attachment| (a.status.clone(), a.reason.clone(), a.details.clone());
        let expected = (
            "partial".to_string(),
            Some("assistant_attachment_partial".to_string()),
            Some(json!({"format": "pdf", "pageCount": 3, "pagesWithoutText": [2]})),
        );
        assert_eq!(read(&record.attachments[0]), expected);
        assert_eq!(read(&record.messages[1].attachments[1]), expected);
        // Another file, and the turn that read this one before, keep theirs.
        assert_eq!(record.attachments[1].status, "ready");
        assert_eq!(record.messages[1].attachments[0].status, "ready");
        assert_eq!(record.messages[0].attachments[0].status, "ready");
        assert_eq!(
            outcome.to_event(),
            json!({
                "id": pdf, "status": "partial", "code": "assistant_attachment_partial",
                "details": {"format": "pdf", "pageCount": 3, "pagesWithoutText": [2]},
            })
        );

        // A later run that reads it fully clears the code and details it had.
        let ready = AttachmentOutcome::parse(&json!({
            "id": pdf, "status": "ready", "code": null, "details": null,
        }))
        .unwrap();
        assert!(record_attachment_outcome(&mut record, &now_run, &ready));
        assert_eq!(read(&record.attachments[0]), ("ready".into(), None, None));
    }

    #[test]
    fn an_outcome_for_a_file_this_turn_did_not_send_changes_nothing() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let (earlier, now_run) = (id(), id());
        let (sent_before, sent_now) = (id(), id());
        let mut asked_before = message(&id(), "user", &earlier, 1);
        asked_before.attachments = vec![file(&sent_before)];
        let mut asked = message(&id(), "user", &now_run, 2);
        asked.attachments = vec![file(&sent_now)];
        let mut record = store.load_record(&sid).unwrap();
        record.messages = vec![asked_before, asked];
        record.attachments = vec![file(&sent_before), file(&sent_now)];

        // The file is the conversation's, but another turn sent it.
        let foreign = AttachmentOutcome::parse(&json!({
            "id": sent_before, "status": "unreadable",
            "code": "assistant_attachment_unreadable", "details": {"reason": "missing"},
        }))
        .unwrap();
        assert!(!record_attachment_outcome(&mut record, &now_run, &foreign));
        assert_eq!(record.attachments[0].status, "ready");
        assert!(record.attachments[0].reason.is_none());
        assert_eq!(record.messages[0].attachments[0].status, "ready");
    }

    #[test]
    fn a_malformed_outcome_is_a_protocol_error() {
        let id = id();
        for data in [
            json!({"status": "ready", "code": null, "details": {}}),
            json!({"id": "", "status": "ready"}),
            json!({"id": id, "status": "processing"}),
            json!({"id": id, "status": "ready", "code": 1}),
            json!({"id": id, "status": "ready", "details": "text"}),
            // Codes are `assistant_*` identifiers, never free text.
            json!({"id": id, "status": "partial", "code": "attachment_partial"}),
            json!({"id": id, "status": "partial", "code": "assistant_"}),
            json!({"id": id, "status": "partial", "code": "assistant_Partial"}),
            json!({"id": id, "status": "partial", "code": "assistant_partial: cut"}),
            json!({"id": id, "status": "partial", "code": crate::redact::REDACTED}),
        ] {
            assert_eq!(
                AttachmentOutcome::parse(&data).unwrap_err(),
                PROTOCOL,
                "{data}"
            );
        }
        let bare = AttachmentOutcome::parse(&json!({"id": id, "status": "unsupported"})).unwrap();
        assert_eq!((bare.code, bare.details), (None, None));
    }
}
