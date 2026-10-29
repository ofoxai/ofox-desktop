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
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// The run's message, session and ledger row as the run ended: the message
/// gets its end time. A run that did not complete makes its message's cost
/// and the session's unknown, unless it failed without being billed
/// (`unbilled`).
pub(super) fn settle_run(
    record: &mut SessionRecord,
    control: &RunControl,
    completed: bool,
    stopped: bool,
    unbilled: bool,
    error_code: Option<&str>,
) -> Result<(Option<Message>, Session), String> {
    let message = record
        .messages
        .iter_mut()
        .find(|m| m.id == control.reply_id())
        .map(|message| {
            message.status = if completed {
                "completed"
            } else if stopped {
                "stopped"
            } else {
                "error"
            }
            .into();
            // Only the host's stable code is saved; never provider error text
            // (a saved record refuses anything else, see `store::check_record`).
            message.error_code = error_code
                .filter(|code| !completed && !stopped && is_assistant_code(code))
                .map(str::to_owned);
            message.completed_at = Some(now());
            if !completed && !unbilled {
                mark_cost_unknown(message.usage.get_or_insert_with(|| json!({})));
            }
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
        mark_cost_unknown(&mut record.usage);
    }
    let unsent: HashSet<String> = record
        .messages
        .iter()
        .filter(|m| m.role == "user" && m.status == "queued")
        .filter_map(|m| m.run_id.clone())
        .collect();
    for op in record
        .operations
        .iter_mut()
        .filter(|op| control.owns_native_run(&op.run_id) && op.status == OperationStatus::Running)
    {
        op.status = if unsent.contains(&op.run_id) {
            OperationStatus::Cancelled
        } else if completed {
            OperationStatus::Completed
        } else if stopped {
            OperationStatus::Cancelled
        } else {
            OperationStatus::Failed
        };
    }
    for user in record.messages.iter_mut().filter(|m| {
        m.run_id
            .as_deref()
            .is_some_and(|id| control.owns_native_run(id))
            && m.role == "user"
            && m.status == "queued"
    }) {
        user.status = "error".into();
        user.error_code = Some("assistant_cancelled".into());
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
/// it interrupted. What it cost is unknown, on its reply as on the session.
pub(super) fn settle_interrupted(
    record: &mut SessionRecord,
    entries: Option<&Value>,
) -> Result<(), String> {
    mark_cost_unknown(&mut record.usage);
    let interrupted: HashSet<String> = record
        .operations
        .iter()
        .filter(|op| op.status == OperationStatus::Running)
        .map(|op| op.run_id.clone())
        .chain(record.session.active_run.iter().map(|r| r.run_id.clone()))
        .collect();
    if let Some(entries) = entries {
        merge_native(record, entries, &[])?;
    }
    let finals = entries
        .and_then(Value::as_array)
        .map(|entries| final_entries(entries))
        .unwrap_or_default();
    for run in &interrupted {
        let last = finals.get(run.as_str()).copied();
        if let Some(index) = reply_index(&record.messages, run, last) {
            mark_cost_unknown(
                record.messages[index]
                    .usage
                    .get_or_insert_with(|| json!({})),
            );
        }
    }
    for message in &mut record.messages {
        if message.role == "user"
            && message.status == "queued"
            && message
                .run_id
                .as_ref()
                .is_some_and(|id| interrupted.contains(id))
        {
            message.status = "error".into();
            message.error_code = Some("assistant_cancelled".into());
        }
        if message.status == "streaming" {
            message.status = "stopped".into();
            if let Some(media) = &mut message.media {
                if media.status == "submitting" {
                    media.status = "unknown".into();
                }
            }
        }
    }
    record.session.active_run = None;
    for op in &mut record.operations {
        if op.status == OperationStatus::Running {
            op.status = OperationStatus::Interrupted;
            if let Some(media) = &mut op.media {
                if media.status == "submitting" {
                    media.status = "unknown".into();
                }
            }
        }
    }
    Ok(())
}

pub(super) fn ensure_object(value: &mut Value) {
    if !value.is_object() {
        *value = json!({});
    }
}

/// Something may have been billed without a known cost: the session's cost,
/// and the part its searches cost, are unknown for good.
fn mark_cost_unknown(usage: &mut Value) {
    ensure_object(usage);
    usage["usageCoverage"] = json!("incomplete");
    usage["estimatedCostUsd"] = Value::Null;
    usage["searchCostUsd"] = Value::Null;
}

/// Streamed text, saved in batches: UI events go out at once, `session.json`
/// is rewritten at most every 500 ms (or each 64 KiB) and at the end. It also
/// times the reasoning: each span from the runtime's `thinking_start` (or a
/// thinking delta) to its `thinking_end` (or the next text, tool call or end
/// of message) counts, at least 1 ms.
pub(super) struct ProjectionBuffer {
    pending: Pending,
    last_flush: std::time::Instant,
    /// When the reasoning streaming now began; `None` outside reasoning.
    thinking_since: Option<std::time::Instant>,
}

#[derive(Default)]
pub(super) struct Pending {
    text: String,
    thinking: String,
    /// Reasoning spans that ended since the last batch, in milliseconds.
    reasoning_ms: u64,
}

impl Default for ProjectionBuffer {
    fn default() -> Self {
        Self {
            pending: Pending::default(),
            last_flush: std::time::Instant::now(),
            thinking_since: None,
        }
    }
}

impl ProjectionBuffer {
    pub(super) fn push(&mut self, run: &RunControl, kind: &str, data: &mut Value) {
        scrub_secrets_in(data, &run.secrets());
        data["messageId"] = json!(run.reply_id());
        if let Some(delta) = data["delta"].as_str() {
            if kind == "text_delta" {
                self.pending.text.push_str(delta);
            } else {
                self.pending.thinking.push_str(delta);
            }
        }
        if kind == "text_delta" {
            self.end_thinking();
        } else {
            self.start_thinking();
        }
    }

    /// Reasoning began streaming, unless it already had.
    pub(super) fn start_thinking(&mut self) {
        self.thinking_since
            .get_or_insert_with(std::time::Instant::now);
    }

    /// The reasoning streaming now, if any, ended: the runtime said so, or a
    /// tool call, the end of the message or of the run came. Reasoning that streamed counts at least
    /// 1 ms, so a reply with reasoning always has a time.
    pub(super) fn end_thinking(&mut self) {
        if let Some(since) = self.thinking_since.take() {
            let spent = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
            self.pending.reasoning_ms = self.pending.reasoning_ms.saturating_add(spent.max(1));
        }
    }

    fn is_empty(&self) -> bool {
        self.pending.text.is_empty()
            && self.pending.thinking.is_empty()
            && self.pending.reasoning_ms == 0
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
        if pending.reasoning_ms > 0 {
            let total = message.reasoning_ms.unwrap_or(0);
            message.reasoning_ms = Some(total.saturating_add(pending.reasoning_ms));
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
            .filter(|r| matches!(*r, "user" | "assistant" | "media"))
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
        // A step of the run that only called tools has nothing to show.
        if target.is_none() && !final_message && content.trim().is_empty() && thinking.is_none() {
            continue;
        }
        let status = if entry["stopReason"] == "error" {
            "error"
        } else if entry["stopReason"] == "aborted" {
            "stopped"
        } else {
            "completed"
        };
        let follows = if role == "user" {
            branch_from(entry)
        } else {
            None
        };
        if let Some(index) = target {
            let m = &mut record.messages[index];
            m.native_entry_id = Some(native_id.into());
            m.superseded = entry["active"] == false;
            // The host owns media results and billing. Native history only
            // determines where this already persisted bubble is visible.
            if role == "media" {
                continue;
            }
            if role == "assistant" {
                m.content = content;
            }
            if follows.is_some() {
                m.branch_from = follows;
            }
            m.reasoning = thinking;
            // A tool or transport failure can follow the last native model
            // entry. Replaying history must not erase that saved outcome.
            m.status = if m.error_code.is_some() {
                "error"
            } else {
                status
            }
            .into();
            if let Some(timestamp) = entry["timestamp"].as_i64() {
                m.created_at = timestamp;
            }
        } else {
            record.messages.push(Message {
                media: None,
                id: id(),
                role: role.into(),
                content,
                created_at: entry["timestamp"].as_i64().unwrap_or_else(now),
                reasoning: thinking,
                reasoning_ms: None,
                status: status.into(),
                error_code: None,
                attachments: vec![],
                sources: vec![],
                superseded: entry["active"] == false,
                revision_of: None,
                native_entry_id: Some(native_id.into()),
                run_id: Some(run_id.into()),
                model_id: None,
                usage: None,
                completed_at: None,
                branch_from: follows,
                anchor_entry_id: None,
            });
        }
    }
    follow_native_branch(record, entries);
    // Native history is authoritative even when the last UI event was lost.
    let active_run = record.session.active_run.as_ref().map(|run| &run.run_id);
    let live_children: HashSet<&str> = record
        .operations
        .iter()
        .filter(|op| op.status == OperationStatus::Running)
        .map(|op| op.run_id.as_str())
        .collect();
    for message in &mut record.messages {
        if message.status == "streaming"
            && !(active_run.is_some()
                && (message.run_id.as_ref() == active_run
                    || message
                        .run_id
                        .as_deref()
                        .is_some_and(|id| live_children.contains(id))))
        {
            message.status = "stopped".into();
        }
    }
    // Local replies keep their place after their question, which now carries
    // the native time. Media timestamps are also local: their native entries
    // may only be appended after generation or repaired when reopening.
    let asked: HashMap<String, i64> = record
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .filter_map(|m| Some((m.run_id.clone()?, m.created_at)))
        .collect();
    for message in &mut record.messages {
        if message.media.is_some()
            || (message.role == "assistant" && message.native_entry_id.is_none())
        {
            if let Some(asked) = message.run_id.as_ref().and_then(|run| asked.get(run)) {
                message.created_at = message.created_at.max(asked.saturating_add(1));
            }
        }
    }
    record.messages.sort_by_key(|m| m.created_at);
    recount_usage(record, entries);
    count_reply_usage(record, entries);
    Ok(())
}

/// Show the messages native history does not hold with the version they
/// belong to: one of a run whose question it holds (a reply cut before it was
/// kept) as that question; a turn it never got as its anchor, the native
/// message it was sent after, while native history still holds that one. A
/// retried turn has no anchor and stays hidden.
fn follow_native_branch(record: &mut SessionRecord, entries: &[Value]) {
    let shown: HashMap<&str, bool> = entries
        .iter()
        .filter_map(|e| Some((e["id"].as_str()?, e["active"] != false)))
        .collect();
    let questions: HashMap<String, bool> = record
        .messages
        .iter()
        .filter(|m| m.role == "user" && m.native_entry_id.is_some())
        .filter_map(|m| Some((m.run_id.clone()?, m.superseded)))
        .collect();
    for message in record
        .messages
        .iter_mut()
        .filter(|m| m.native_entry_id.is_none())
    {
        let question = message.run_id.as_ref().and_then(|run| questions.get(run));
        let anchor = message
            .anchor_entry_id
            .as_deref()
            .and_then(|anchor| shown.get(anchor));
        if let Some(superseded) = question {
            message.superseded = *superseded;
        } else if let Some(shown) = anchor {
            message.superseded = !shown;
        }
    }
}

/// The message a native user entry follows (the runtime's `branchFrom`):
/// `Some(None)` when it starts the conversation, `None` when not said.
fn branch_from(entry: &Value) -> Option<Option<String>> {
    match entry.get("branchFrom")? {
        Value::String(native) => Some(Some(native.clone())),
        Value::Null => Some(None),
        _ => None,
    }
}

/// Each run's last assistant entry (the runtime's `final`), by run id.
fn final_entries(entries: &[Value]) -> HashMap<&str, &str> {
    entries
        .iter()
        .filter(|e| e["role"] == "assistant" && e["final"] != false)
        .filter_map(|e| Some((e["runId"].as_str()?, e["id"].as_str()?)))
        .collect()
}

/// The message showing `run`'s reply: the one bound to its last native
/// entry (`last`), else its reply native history does not hold (cut, or
/// never sent).
fn reply_index(messages: &[Message], run: &str, last: Option<&str>) -> Option<usize> {
    let of_run = |m: &Message| m.role == "assistant" && m.run_id.as_deref() == Some(run);
    last.and_then(|native| {
        messages
            .iter()
            .position(|m| of_run(m) && m.native_entry_id.as_deref() == Some(native))
    })
    .or_else(|| {
        messages
            .iter()
            .position(|m| of_run(m) && m.native_entry_id.is_none())
    })
}

/// Count each run's native entries onto its reply, as the conversation's
/// are counted, but without its title: a generated title belongs to the
/// conversation, though its entry is kept under the turn it named. An
/// unknown cost stays unknown.
fn count_reply_usage(record: &mut SessionRecord, entries: &[Value]) {
    let mut runs: HashMap<&str, Vec<&Value>> = HashMap::new();
    for entry in entries {
        if entry["role"] == "usage" && entry["type"] == "title" {
            continue;
        }
        if let Some(run) = entry["runId"].as_str() {
            runs.entry(run).or_default().push(entry);
        }
    }
    let finals = final_entries(entries);
    let prices = search_prices(&record.operations);
    for (run, of_run) in runs {
        let Some(index) = reply_index(&record.messages, run, finals.get(run).copied()) else {
            continue;
        };
        let usage = record.messages[index]
            .usage
            .get_or_insert_with(|| json!({}));
        project_usage(usage, of_run.iter().copied(), &prices);
    }
}

/// Recount the conversation's usage from the whole of its native history,
/// each search at the price its run was given.
pub(super) fn recount_usage(record: &mut SessionRecord, entries: &[Value]) {
    let prices = search_prices(&record.operations);
    project_usage(&mut record.usage, entries, &prices);
    let media: Vec<_> = record
        .operations
        .iter()
        .filter_map(|op| op.media.as_ref())
        .filter(|media| media.status != "submitting")
        .collect();
    if media.is_empty() {
        return;
    }
    let sum = media.iter().try_fold(Decimal::ZERO, |sum, m| {
        sum.checked_add(m.cost.as_deref()?.parse().ok()?)
    });
    let chat_requests = record.usage["modelRequests"].as_u64().unwrap_or(0);
    let chat_cost = if chat_requests == 0 && record.usage["usageCoverage"] != "incomplete" {
        Some(Decimal::ZERO)
    } else {
        record.usage["estimatedCostUsd"]
            .as_str()
            .and_then(|s| s.parse::<Decimal>().ok())
    };
    record.usage["mediaCostUsd"] = json!(sum.map(|n| n.normalize().to_string()));
    record.usage["estimatedCostUsd"] = json!(sum
        .zip(chat_cost)
        .and_then(|(a, b)| a.checked_add(b))
        .map(|n| n.normalize().to_string()));
    record.usage["modelRequests"] = json!(chat_requests + media.len() as u64);
    if sum.is_none() {
        record.usage["usageCoverage"] = json!("incomplete");
    }
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

/// What one `web_search` tool entry of the native history cost.
#[derive(Debug, PartialEq)]
enum SearchCost {
    /// Refused, never sent, or a repeat answered from an earlier search.
    Nothing,
    /// This many paid searches, at this cost (`None`: its run had no price).
    Paid(u64, Option<Decimal>),
    /// It may have been billed at an unknown cost: an unknown outcome,
    /// unreported or malformed counts, or a model its run's price is not for.
    Unknown,
}

fn search_entry_cost(entry: &Value, prices: &HashMap<&str, &SearchPrice>) -> SearchCost {
    if entry["usageCoverage"] == "incomplete" {
        return SearchCost::Unknown;
    }
    let details = &entry["details"];
    if entry["isError"] == true || details["code"].is_string() {
        return SearchCost::Nothing;
    }
    let usage = &details["usage"];
    let count = |key: &str| usage[key].as_u64();
    let (Some(calls), Some(tokens_in), Some(tokens_out)) = (
        count("searchCalls"),
        count("inputTokens"),
        count("outputTokens"),
    ) else {
        return SearchCost::Unknown;
    };
    if calls == 0 && tokens_in == 0 && tokens_out == 0 {
        return SearchCost::Nothing;
    }
    let Some(price) = entry["runId"].as_str().and_then(|run| prices.get(run)) else {
        return SearchCost::Paid(calls, None);
    };
    if usage["model"].as_str() != Some(price.model.as_str()) {
        return SearchCost::Unknown;
    }
    SearchCost::Paid(calls, price.cost(calls, tokens_in, tokens_out))
}

/// Recount the session's usage from the native history. One unpriced or
/// incomplete request or search leaves the cost unknown for good. Searches
/// (the runtime's `web_search` tool entries) are priced with the price their
/// run was given (`search_prices`, by run id): `searchCount` paid searches
/// cost `searchCostUsd`, which `estimatedCostUsd` includes.
pub(super) fn project_usage<'a>(
    totals: &mut Value,
    entries: impl IntoIterator<Item = &'a Value> + Clone,
    search_prices: &HashMap<&str, &SearchPrice>,
) {
    ensure_object(totals);
    let (mut input, mut output, mut cache_read, mut cache_write) = (0u64, 0u64, 0u64, 0u64);
    let mut estimate = 0.0;
    let mut priced = totals["usageCoverage"] != "incomplete";
    let mut count = 0u64;
    let (mut searches, mut search_cost) = (0u64, Some(Decimal::ZERO));
    for entry in entries
        .clone()
        .into_iter()
        .filter(|e| e["role"] == "tool" && e["toolName"] == "web_search")
    {
        match search_entry_cost(entry, search_prices) {
            SearchCost::Nothing => {}
            SearchCost::Paid(calls, cost) => {
                searches = searches.saturating_add(calls);
                search_cost = search_cost
                    .zip(cost)
                    .and_then(|(sum, cost)| sum.checked_add(cost));
            }
            SearchCost::Unknown => {
                priced = false;
                totals["usageCoverage"] = json!("incomplete");
            }
        }
    }
    for entry in entries
        .into_iter()
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
    fn a_live_child_reply_survives_a_merge_under_the_parent_run() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        record.session.active_run = Some(RunRef {
            run_id: "parent".into(),
            operation_id: "parent-op".into(),
        });
        record.operations.push(
            serde_json::from_value(json!({"id":"child","runId":"child","status":"running"}))
                .unwrap(),
        );
        record
            .messages
            .push(message("reply", "assistant", "child", 1));
        merge_native(&mut record, &json!([]), &[]).unwrap();
        assert_eq!(record.messages[0].status, "streaming");
    }

    #[test]
    fn restart_marks_an_unconsumed_live_input_unsent_instead_of_leaving_it_queued() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        record.operations.push(
            serde_json::from_value(json!({"id":"child","runId":"child","status":"running"}))
                .unwrap(),
        );
        let mut input = message("child", "user", "child", 1);
        input.status = "queued".into();
        record.messages.push(input);
        settle_interrupted(&mut record, None).unwrap();
        assert_eq!(record.messages[0].status, "error");
        assert_eq!(
            record.messages[0].error_code.as_deref(),
            Some("assistant_cancelled")
        );
        assert_eq!(record.operations[0].status, OperationStatus::Interrupted);
    }

    #[test]
    fn media_results_follow_their_questions_after_late_native_history_repair() {
        let (_config, store) = store();
        let sid = store.create("image/test", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        let mut entries = vec![];
        for (run, at) in [("first", 100), ("second", 200)] {
            let mut question = message(&format!("q-{run}"), "user", run, 1);
            question.status = "completed".into();
            let mut reply = message(&format!("m-{run}"), "media", run, 2);
            reply.status = "completed".into();
            reply.media = Some(
                serde_json::from_value(json!({
                    "kind":"image","model":"image/test","params":{},
                    "status":"completed","outputs":["file"],"cost":"0.02"
                }))
                .unwrap(),
            );
            record.messages.extend([question, reply]);
            entries.extend([
                json!({"id":format!("native-q-{run}"),"role":"user","runId":run,"timestamp":at,"active":true,"content":"prompt"}),
                json!({"id":format!("native-m-{run}"),"role":"media","runId":run,"timestamp":at+1,"active":true,"content":"media"}),
            ]);
        }
        let entries = json!(entries);
        for _ in 0..2 {
            merge_native(&mut record, &entries, &[]).unwrap();
            assert_eq!(
                record
                    .messages
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<Vec<_>>(),
                ["q-first", "m-first", "q-second", "m-second"]
            );
            assert_eq!(
                record.messages[1].media.as_ref().unwrap().cost.as_deref(),
                Some("0.02")
            );
        }
    }

    #[test]
    fn merging_older_history_keeps_the_registered_reply_streaming_until_settled() {
        let (_config, store) = store();
        let sid = store.create("test", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        let live = control(&sid);
        record
            .messages
            .push(message(&live.message_id, "assistant", &live.run_id, 1));
        record.session.active_run = Some(RunRef {
            run_id: live.run_id.clone(),
            operation_id: live.operation_id.clone(),
        });
        merge_native(&mut record, &json!([]), &[]).unwrap();
        assert_eq!(record.messages[0].status, "streaming");
        assert_eq!(record.messages[0].content, "partial");
        // Recovery still settles a previous process's dangling run.
        settle_interrupted(&mut record, Some(&json!([]))).unwrap();
        assert_eq!(record.messages[0].status, "stopped");
        assert!(record.session.active_run.is_none());
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
    fn reasoning_time_sums_each_span_from_a_thinking_delta_to_what_follows_it() {
        let (_config, store) = store();
        let sid = store.create("test", false).unwrap().session.id;
        let run = control(&sid);
        store
            .mutate(&sid, |record| {
                record
                    .messages
                    .push(message(&run.message_id, "assistant", &run.run_id, 1));
                Ok(())
            })
            .unwrap();
        let ago = |ms: u64| Some(std::time::Instant::now() - Duration::from_millis(ms));
        let delta = || json!({"delta": "x"});
        let mut buffer = ProjectionBuffer::default();
        // 1.5 s of reasoning, ended by the reply's text.
        buffer.push(&run, "thinking_delta", &mut delta());
        buffer.thinking_since = ago(1500);
        buffer.push(&run, "thinking_delta", &mut delta());
        buffer.push(&run, "text_delta", &mut delta());
        buffer.push(&run, "text_delta", &mut delta());
        // Then 0.5 s more, ended by a tool call.
        buffer.push(&run, "thinking_delta", &mut delta());
        buffer.thinking_since = ago(500);
        buffer.end_thinking();
        buffer.end_thinking();
        let pending = buffer.take().unwrap();
        assert!(
            (2000..10_000).contains(&pending.reasoning_ms),
            "{}",
            pending.reasoning_ms
        );
        save_projection(&store, &sid, &run.message_id, &pending, &run.secrets()).unwrap();

        // A span still open when a batch is saved goes on into the next one.
        buffer.push(&run, "thinking_delta", &mut delta());
        buffer.thinking_since = ago(300);
        assert_eq!(buffer.take().unwrap().reasoning_ms, 0);
        buffer.end_thinking();
        let pending = buffer.take().expect("the time alone is saved");
        assert!(pending.reasoning_ms >= 300);
        save_projection(&store, &sid, &run.message_id, &pending, &run.secrets()).unwrap();
        let total = store.load_detail(&sid).unwrap().messages[0]
            .reasoning_ms
            .unwrap();
        assert!((2300..20_000).contains(&total), "{total}");

        // Native history replaces the text, never the time.
        let mut record = store.load_record(&sid).unwrap();
        let entries = json!([{
            "id": "n1", "role": "assistant", "runId": run.run_id, "content": "answer",
            "thinking": "why", "active": true, "final": true, "timestamp": 2,
        }]);
        merge_native(&mut record, &entries, &[]).unwrap();
        assert_eq!(record.messages[0].reasoning_ms, Some(total));
        assert!(record.messages[0].native_entry_id.is_some());

        // The runtime's start signal opens the span before any reasoning text
        // arrives; a later delta does not restart it.
        let mut signalled = ProjectionBuffer::default();
        signalled.start_thinking();
        signalled.thinking_since = ago(400);
        signalled.start_thinking();
        signalled.push(&run, "thinking_delta", &mut delta());
        signalled.push(&run, "text_delta", &mut delta());
        let spent = signalled.take().unwrap().reasoning_ms;
        assert!((400..10_000).contains(&spent), "{spent}");
        // Reasoning that streamed always counts at least 1 ms.
        let mut instant = ProjectionBuffer::default();
        instant.start_thinking();
        instant.end_thinking();
        assert_eq!(instant.take().unwrap().reasoning_ms, 1);

        // No reasoning, no time.
        let mut plain = ProjectionBuffer::default();
        plain.push(&run, "text_delta", &mut delta());
        plain.end_thinking();
        assert_eq!(plain.take().unwrap().reasoning_ms, 0);
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
            "model": "openai/gpt-5-nano", "perSearch": "0.014",
            "perInputToken": "0.00000005", "perOutputToken": "0.0000004",
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
    fn a_search_whose_usage_cannot_be_priced_leaves_the_cost_unknown() {
        let price = search_price();
        let prices = HashMap::from([("run1", &price)]);
        let with_usage = |usage: Value| {
            search_entry(
                "run1",
                json!({"usage": usage, "sources": []}),
                "complete",
                false,
            )
        };
        let fine = json!({"searchCalls": 1, "inputTokens": 1, "outputTokens": 1,
                          "model": "openai/gpt-5-nano"});
        let mut cases = vec![];
        // Another model than the one the run's price is for.
        let mut other = fine.clone();
        other["model"] = json!("openai/gpt-5-mini");
        cases.push(other);
        let mut unnamed = fine.clone();
        unnamed["model"] = Value::Null;
        cases.push(unnamed);
        // Calls or tokens that are not counts.
        for bad in [json!(-1), json!(1.5), json!("1"), Value::Null] {
            for key in ["searchCalls", "inputTokens", "outputTokens"] {
                let mut usage = fine.clone();
                usage[key] = bad.clone();
                cases.push(usage);
            }
        }
        cases.push(Value::Null);
        for usage in cases {
            let mut totals = json!({});
            project_usage(
                &mut totals,
                &[model_entry("run1"), with_usage(usage.clone())],
                &prices,
            );
            assert_eq!(totals["usageCoverage"], "incomplete", "{usage}");
            assert!(totals["estimatedCostUsd"].is_null(), "{usage}");
            assert!(totals["searchCostUsd"].is_null(), "{usage}");
        }

        // A repeat costs nothing whatever model it names; a search the run
        // never sent (after an unknown outcome) neither.
        let repeat = search_entry(
            "run1",
            json!({"usage": {"searchCalls": 0, "inputTokens": 0, "outputTokens": 0, "model": null},
                   "sources": [], "deduplicated": true}),
            "complete",
            false,
        );
        let unsent = search_entry(
            "run1",
            json!({"code": "unknown_outcome"}),
            "complete",
            false,
        );
        let mut totals = json!({});
        project_usage(
            &mut totals,
            &[model_entry("run1"), with_usage(fine), repeat, unsent],
            &prices,
        );
        assert!(totals.get("usageCoverage").is_none(), "{totals}");
        assert_eq!(totals["searchCount"], 1);
        assert_eq!(totals["searchCostUsd"], "0.01400045");
    }

    #[test]
    fn a_saved_failure_survives_native_replay_but_success_and_stop_clear_it() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let run = control(&sid);
        let mut record = store.load_record(&sid).unwrap();
        record.messages = vec![message(&run.message_id, "assistant", &run.run_id, 1)];
        settle_run(
            &mut record,
            &run,
            false,
            false,
            false,
            Some("assistant_search_outcome_unknown"),
        )
        .unwrap();
        let native = json!([{
            "id": "native-reply", "role": "assistant", "runId": run.run_id,
            "content": "Partial answer", "stopReason": "stop", "final": true,
        }]);
        merge_native(&mut record, &native, &[]).unwrap();
        assert_eq!(record.messages[0].status, "error");
        assert_eq!(
            record.messages[0].error_code.as_deref(),
            Some("assistant_search_outcome_unknown")
        );
        for (completed, stopped, status) in [(true, false, "completed"), (false, true, "stopped")] {
            let mut settled = record.clone();
            settle_run(
                &mut settled,
                &run,
                completed,
                stopped,
                false,
                Some("assistant_network_error"),
            )
            .unwrap();
            assert_eq!(settled.messages[0].status, status);
            assert!(settled.messages[0].error_code.is_none());
        }
        // Anything but an `assistant_*` code would make the record unsavable.
        let mut settled = record.clone();
        settle_run(&mut settled, &run, false, false, false, Some("ECONNRESET")).unwrap();
        assert_eq!(settled.messages[0].status, "error");
        assert!(settled.messages[0].error_code.is_none());
    }

    #[test]
    fn a_stopped_run_keeps_the_text_it_had_streamed() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let run = control(&sid);
        let mut record = store.load_record(&sid).unwrap();
        let mut reply = message(&run.message_id, "assistant", &run.run_id, 1);
        reply.content = "Half of the answer".into();
        record.messages = vec![reply];
        let (settled, _) = settle_run(&mut record, &run, false, true, false, None).unwrap();
        let settled = settled.unwrap();
        assert_eq!(settled.status, "stopped");
        assert_eq!(settled.content, "Half of the answer");
        assert_eq!(record.messages[0].content, "Half of the answer");
    }

    #[test]
    fn native_history_keeps_the_text_of_a_stopped_reply() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let run = control(&sid);
        let mut record = store.load_record(&sid).unwrap();
        let mut reply = message(&run.message_id, "assistant", &run.run_id, 1);
        reply.content = "Half of the answer".into();
        record.messages = vec![reply];
        settle_run(&mut record, &run, false, true, false, None).unwrap();
        // The model's own record of the stopped turn holds the same words.
        let native = json!([{
            "id": "native-reply", "role": "assistant", "runId": run.run_id,
            "content": "Half of the answer", "stopReason": "aborted", "final": true,
        }]);
        merge_native(&mut record, &native, &[]).unwrap();
        assert_eq!(record.messages[0].status, "stopped");
        assert_eq!(record.messages[0].content, "Half of the answer");
        // A history that never saw the reply (the stop beat it) changes nothing.
        merge_native(&mut record, &json!([]), &[]).unwrap();
        assert_eq!(record.messages[0].status, "stopped");
        assert_eq!(record.messages[0].content, "Half of the answer");
    }

    #[test]
    fn a_billed_run_that_did_not_complete_leaves_both_costs_unknown() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let run = control(&sid);
        let known = json!({"modelRequests": 2, "estimatedCostUsd": "0.01439",
                           "searchCount": 1, "searchCostUsd": "0.01425"});
        let mut record = store.load_record(&sid).unwrap();
        record.messages = vec![message(&run.message_id, "assistant", &run.run_id, 1)];
        record.usage = known.clone();
        // Stopped, or failed after it may have been billed.
        for (stopped, unbilled) in [(true, false), (false, false)] {
            let mut settled = record.clone();
            settle_run(&mut settled, &run, false, stopped, unbilled, None).unwrap();
            assert_eq!(settled.usage["usageCoverage"], "incomplete");
            assert!(settled.usage["estimatedCostUsd"].is_null());
            assert!(settled.usage["searchCostUsd"].is_null());
            assert_eq!(settled.usage["searchCount"], 1);
        }
        // Refused before anything was billed: both stay known.
        let mut settled = record.clone();
        settle_run(&mut settled, &run, false, false, true, None).unwrap();
        assert_eq!(settled.usage, known);
        // A run of a process that went away, without its native history.
        let mut settled = record.clone();
        settle_interrupted(&mut settled, None).unwrap();
        assert_eq!(settled.usage["usageCoverage"], "incomplete");
        assert!(settled.usage["estimatedCostUsd"].is_null());
        assert!(settled.usage["searchCostUsd"].is_null());
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
                media: None,
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
    fn a_step_that_only_called_tools_is_not_a_message() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        record.messages = vec![
            message("u1", "user", "run1", 1),
            message("a1", "assistant", "run1", 1),
        ];
        let step = |id: &str, content: &str, thinking: &str, at: i64| {
            json!({"id": id, "role": "assistant", "runId": "run1", "content": content,
                   "thinking": thinking, "active": true, "final": false, "timestamp": at,
                   "stopReason": "toolUse"})
        };
        merge_native(
            &mut record,
            &json!([
                {"id":"n-u1","role":"user","runId":"run1","content":"q","active":true,"timestamp":10},
                step("n-call", "", "", 11),
                {"id":"n-tool","role":"tool","runId":"run1","toolName":"web_search","content":"found","timestamp":12},
                step("n-said", "Let me check another page.", "", 13),
                step("n-thought", "", "Which page next?", 14),
                {"id":"n-a1","role":"assistant","runId":"run1","content":"answer","active":true,"final":true,"timestamp":15},
            ]),
            &[],
        )
        .unwrap();
        let kept: Vec<(&str, Option<&str>, &str)> = record
            .messages
            .iter()
            .map(|m| {
                (
                    m.role.as_str(),
                    m.native_entry_id.as_deref(),
                    m.content.as_str(),
                )
            })
            .collect();
        assert_eq!(
            kept,
            [
                ("user", Some("n-u1"), "partial"),
                ("assistant", Some("n-said"), "Let me check another page."),
                ("assistant", Some("n-thought"), ""),
                ("assistant", Some("n-a1"), "answer"),
            ]
        );
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

    /// A model entry of `run`; `last` as the runtime marks its run's last.
    fn native_reply(native: &str, run: &str, input: u64, cost: f64, last: bool) -> Value {
        json!({
            "id": native, "role": "assistant", "runId": run, "content": format!("{native} text"),
            "active": true, "final": last, "timestamp": 10, "pricingKnown": true,
            "usage": {"input": input, "output": input / 10, "cacheRead": 0, "cacheWrite": 0,
                      "cost": {"total": cost}},
        })
    }

    /// A request outside the conversation (a title, a compaction) of `run`.
    fn usage_entry(native: &str, run: &str, kind: &str, input: u64, cost: f64) -> Value {
        json!({
            "id": native, "role": "usage", "type": kind, "runId": run, "active": true,
            "usageCoverage": "complete", "pricingKnown": true,
            "usage": {"input": input, "output": input / 10, "cacheRead": 0, "cacheWrite": 0,
                      "cost": {"total": cost}},
        })
    }

    fn cost(usage: &Value) -> Decimal {
        usage["estimatedCostUsd"].as_str().unwrap().parse().unwrap()
    }

    #[test]
    fn each_reply_counts_its_own_run_and_with_the_title_they_add_up_to_the_conversation() {
        let (_config, store) = store();
        let sid = store.create("test", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        let (first, second) = (id(), id());
        record.messages = vec![
            message(&id(), "user", &first, 1),
            message(&id(), "assistant", &first, 1),
            message(&id(), "user", &second, 2),
            message(&id(), "assistant", &second, 2),
        ];
        // The first run could search, at the price it was given.
        record.operations.push(Operation {
            media: None,
            id: id(),
            run_id: first.clone(),
            status: OperationStatus::Completed,
            search_price: Some(search_price()),
        });
        // Costs a float sums exactly, so the totals can be compared exactly.
        let mut step = native_reply("s1", &first, 100, 0.25, false);
        step["content"] = json!("Let me look that up.");
        let title = usage_entry("t1", &first, "title", 40, 0.125);
        let entries = json!([
            {"id": "u1", "role": "user", "runId": first, "content": "q1", "active": true,
             "timestamp": 3, "branchFrom": null},
            step,
            search_entry(&first, searched(1, 1000, 100), "complete", false),
            native_reply("a1", &first, 200, 0.5, true),
            title,
            {"id": "u2", "role": "user", "runId": second, "content": "q2", "active": true,
             "timestamp": 11, "branchFrom": "a1"},
            usage_entry("c1", &second, "compaction", 30, 0.0625),
            native_reply("a2", &second, 300, 1.0, true),
        ]);
        merge_native(&mut record, &entries, &[]).unwrap();
        let usage_of = |native: &str| {
            record
                .messages
                .iter()
                .find(|m| m.native_entry_id.as_deref() == Some(native))
                .unwrap()
                .usage
                .clone()
        };
        // A run's usage is on its reply alone: not on its question, nor on
        // a step of it that showed text.
        for native in ["u1", "u2", "s1"] {
            assert!(usage_of(native).is_none(), "{native}");
        }
        let (one, two) = (usage_of("a1").unwrap(), usage_of("a2").unwrap());
        assert_eq!(one["inputTokens"], 300, "its steps, not the title");
        assert_eq!(one["modelRequests"], 2);
        assert_eq!(one["searchCount"], 1);
        assert_eq!(one["searchCostUsd"], "0.01409");
        assert_eq!(cost(&one), "0.76409".parse::<Decimal>().unwrap());
        assert_eq!(two["inputTokens"], 330, "its compaction is its own");
        assert_eq!(two["searchCount"], 0);
        let mut titled = json!({});
        project_usage(&mut titled, &[title], &HashMap::new());
        for key in [
            "inputTokens",
            "outputTokens",
            "modelRequests",
            "searchCount",
        ] {
            let sum = [&one, &two, &titled]
                .iter()
                .map(|usage| usage[key].as_u64().unwrap())
                .sum::<u64>();
            assert_eq!(json!(sum), record.usage[key], "{key}");
        }
        assert_eq!(cost(&one) + cost(&two) + cost(&titled), cost(&record.usage));
    }

    #[test]
    fn a_native_user_turn_keeps_the_message_it_follows() {
        let (_config, store) = store();
        let sid = store.create("test", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        let (first, second) = (id(), id());
        record.messages = vec![
            message(&id(), "user", &first, 1),
            message(&id(), "assistant", &first, 1),
            message(&id(), "user", &second, 2),
            message(&id(), "assistant", &second, 2),
        ];
        let entries = json!([
            {"id": "u1", "role": "user", "runId": first, "active": true, "timestamp": 3,
             "branchFrom": null},
            native_reply("a1", &first, 1, 0.0, true),
            {"id": "u2", "role": "user", "runId": second, "active": true, "timestamp": 11,
             "branchFrom": "a1"},
            native_reply("a2", &second, 1, 0.0, true),
        ]);
        merge_native(&mut record, &entries, &[]).unwrap();
        let follows = |record: &SessionRecord, native: &str| {
            record
                .messages
                .iter()
                .find(|m| m.native_entry_id.as_deref() == Some(native))
                .unwrap()
                .branch_from
                .clone()
        };
        assert_eq!(
            follows(&record, "u1"),
            Some(None),
            "it starts the conversation"
        );
        assert_eq!(follows(&record, "u2"), Some(Some("a1".into())));
        assert_eq!(follows(&record, "a1"), None, "only user turns are versions");
        // A history that does not say leaves what is known.
        let mut silent = entries.clone();
        silent[2].as_object_mut().unwrap().remove("branchFrom");
        merge_native(&mut record, &silent, &[]).unwrap();
        assert_eq!(follows(&record, "u2"), Some(Some("a1".into())));
    }

    #[test]
    fn a_settled_reply_has_its_end_time_and_a_billed_failure_leaves_its_cost_unknown() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let run = control(&sid);
        let native = json!([native_reply("n1", &run.run_id, 100, 0.5, true)]);
        // completed, stopped, unbilled -> its cost stays known
        for (completed, stopped, unbilled, known) in [
            (true, false, false, true),
            (false, true, false, false),
            (false, false, false, false),
            (false, false, true, true),
        ] {
            let mut record = store.load_record(&sid).unwrap();
            record.messages = vec![message(&run.message_id, "assistant", &run.run_id, 1)];
            merge_native(&mut record, &native, &[]).unwrap();
            let before = now();
            let (settled, _) =
                settle_run(&mut record, &run, completed, stopped, unbilled, None).unwrap();
            let settled = settled.unwrap();
            assert!(settled.completed_at.is_some_and(|at| at >= before));
            assert_eq!(record.messages[0].completed_at, settled.completed_at);
            let usage = settled.usage.unwrap();
            assert_eq!(usage["inputTokens"], 100);
            assert_eq!(usage["usageCoverage"] == "incomplete", !known, "{usage}");
            assert_eq!(usage["estimatedCostUsd"].is_string(), known, "{usage}");
            // Counted again from native history, an unknown cost stays unknown.
            merge_native(&mut record, &native, &[]).unwrap();
            let usage = record.messages[0].usage.as_ref().unwrap();
            assert_eq!(usage["estimatedCostUsd"].is_string(), known, "{usage}");
        }
    }

    #[test]
    fn an_interrupted_reply_keeps_what_it_used_at_an_unknown_cost_and_no_end_time() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let run = control(&sid);
        let mut record = store.load_record(&sid).unwrap();
        record.messages = vec![
            message(&id(), "user", &run.run_id, 1),
            message(&run.message_id, "assistant", &run.run_id, 1),
        ];
        record.operations.push(Operation {
            media: None,
            id: run.operation_id.clone(),
            run_id: run.run_id.clone(),
            status: OperationStatus::Running,
            search_price: None,
        });
        record.session.active_run = Some(RunRef {
            run_id: run.run_id.clone(),
            operation_id: run.operation_id.clone(),
        });
        let native = json!([
            {"id": "u1", "role": "user", "runId": run.run_id, "active": true, "timestamp": 3},
            native_reply("n1", &run.run_id, 100, 0.5, true),
        ]);
        settle_interrupted(&mut record, Some(&native)).unwrap();
        let reply = &record.messages[1];
        assert_eq!(reply.id, run.message_id);
        let usage = reply.usage.as_ref().unwrap();
        assert_eq!(usage["inputTokens"], 100);
        assert_eq!(usage["usageCoverage"], "incomplete");
        assert!(usage["estimatedCostUsd"].is_null());
        assert!(reply.completed_at.is_none());
        assert!(record.messages[0].usage.is_none());
    }

    /// Native user entry `native` of `run`, on the branch shown or not.
    fn native_question(native: &str, run: &str, active: bool) -> Value {
        json!({"id": native, "role": "user", "runId": run, "content": "q", "active": active,
               "timestamp": 50})
    }

    #[test]
    fn a_reply_native_history_never_kept_follows_its_question_across_versions() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        // The runtime kept the question, then died before the reply.
        record.messages = vec![message("u1", "user", "run1", 1), {
            let mut cut = message("a1", "assistant", "run1", 1);
            cut.status = "stopped".into();
            cut
        }];
        for shown in [false, true, false] {
            merge_native(
                &mut record,
                &json!([native_question("n1", "run1", shown)]),
                &[],
            )
            .unwrap();
            let flags: Vec<bool> = record.messages.iter().map(|m| m.superseded).collect();
            assert_eq!(flags, [!shown, !shown], "shown: {shown}");
            assert!(record.messages[1].native_entry_id.is_none());
        }
    }

    #[test]
    fn a_turn_native_history_never_got_shows_with_the_version_it_was_sent_on() {
        let (_config, store) = store();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut record = store.load_record(&sid).unwrap();
        record.messages = vec![
            message(&id(), "user", "run-a", 1),
            message(&id(), "assistant", "run-a", 1),
            message(&id(), "user", "run-b", 2),
            message(&id(), "assistant", "run-b", 2),
        ];
        // Sent on version A, failed before the runtime: no native entries.
        let local: Vec<Message> = ["user", "assistant"]
            .into_iter()
            .map(|role| {
                let mut turn = message(&id(), role, "run-local", 3);
                turn.status = "error".into();
                turn.anchor_entry_id = Some("a1".into());
                turn
            })
            .collect();
        record.messages.extend(local);
        // Asked again on version A: hidden on every version.
        let mut retried = message(&id(), "user", "run-retried", 3);
        retried.superseded = true;
        record.messages.push(retried);
        let versions = |a_shown: bool| {
            json!([
                native_question("ua", "run-a", a_shown),
                {"id": "a1", "role": "assistant", "runId": "run-a", "active": a_shown,
                 "final": true, "timestamp": 51},
                native_question("ub", "run-b", !a_shown),
                {"id": "b1", "role": "assistant", "runId": "run-b", "active": !a_shown,
                 "final": true, "timestamp": 52},
            ])
        };
        for a_shown in [true, false, true] {
            merge_native(&mut record, &versions(a_shown), &[]).unwrap();
            let hidden = |run: &str| {
                record
                    .messages
                    .iter()
                    .filter(|m| m.run_id.as_deref() == Some(run))
                    .map(|m| m.superseded)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                hidden("run-local"),
                [!a_shown, !a_shown],
                "A shown: {a_shown}"
            );
            assert_eq!(hidden("run-retried"), [true]);
        }
        // An anchor native history no longer holds changes nothing.
        let question = record
            .messages
            .iter_mut()
            .find(|m| m.run_id.as_deref() == Some("run-local") && m.role == "user")
            .unwrap();
        question.anchor_entry_id = Some("gone".into());
        question.superseded = false;
        merge_native(&mut record, &versions(false), &[]).unwrap();
        let unanchored = record
            .messages
            .iter()
            .find(|m| m.anchor_entry_id.as_deref() == Some("gone"))
            .unwrap();
        assert!(!unanchored.superseded);
    }
}
