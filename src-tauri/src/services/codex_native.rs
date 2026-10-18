//! Body-free Codex native usage and thread projection.
//!
//! The counter-domain reconciler is adapted from Magpie bfaedf18
//! `internal/sessions/codex_usage.go` and `codex.go` (MIT, copyright 2026 yetone).
//! See adjacent `MAGPIE_LICENSE` and the fixture README for attribution.

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

pub(crate) const VERSION: u8 = 2;
const MAX_SOURCE_BYTES: usize = 128 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;
const MAX_STREAM_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ITEMS: usize = 100_000;
const MAX_RECONCILIATION_WORK: usize = 4_000_000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeSnapshot {
    pub version: u8,
    pub native_thread_id: Option<String>,
    pub billing_session_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub forked_from_id: Option<String>,
    pub project_dir: Option<String>,
    /// Existing short title metadata only; never part of the native usage proof.
    #[serde(skip)]
    pub title_hint: Option<String>,
    pub main: bool,
    pub events: Vec<NativeUsageEvent>,
    pub messages: Vec<NativeMessage>,
    pub activity: Vec<NativeActivity>,
    pub reasons: BTreeSet<String>,
    pub source_digest: String,
    /// Same-pass canonical row hashes, including body bytes only through SHA-256.
    /// A partial source cannot establish a complete cross-file prefix proof.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub record_hashes: Vec<String>,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeUsageEvent {
    pub key: String,
    /// Legacy-only conflict hint for the observed native interval. It is not
    /// sufficient proof to deduplicate independent sources or copied history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_identity: Option<String>,
    pub response_id: Option<String>,
    pub native_thread_id: String,
    pub billing_session_id: Option<String>,
    pub at_ms: Option<i64>,
    pub model: String,
    /// New input, output, cache read, cache write; never inclusive input.
    pub buckets: [u64; 4],
    pub complete: bool,
    pub canonical_ordinal: u64,
    pub counter_epoch: u32,
    pub provenance: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeMessage {
    pub key: String,
    pub role: String,
    pub at_ms: Option<i64>,
    pub canonical_ordinal: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeActivity {
    pub key: String,
    pub at_ms: i64,
    pub canonical_ordinal: u64,
    #[serde(default)]
    pub stream_segment: u32,
}

pub(crate) fn read(path: &Path) -> Result<NativeSnapshot, AppError> {
    read_with_buffer_budget(path, MAX_SOURCE_BYTES)
}

fn read_with_buffer_budget(path: &Path, buffer_budget: usize) -> Result<NativeSnapshot, AppError> {
    super::usage_source::with_stable_regular_file(path, |file| {
        let source_bytes = file
            .metadata()
            .map_err(|_| invalid("sourceUnavailable"))?
            .len();
        if source_bytes > MAX_STREAM_BYTES {
            return Err(invalid("sourceBudgetExceeded"));
        }
        let compressed = path.extension().is_some_and(|e| e == "zst" || e == "zstd");
        let reader: Box<dyn Read> = if compressed {
            let mut decoder =
                zstd::stream::read::Decoder::new(file).map_err(|_| invalid("sourceReadFailed"))?;
            decoder
                .window_log_max(23)
                .map_err(|_| invalid("sourceReadFailed"))?;
            Box::new(decoder)
        } else {
            Box::new(file)
        };
        // A tiny uncompressed file can use the deterministic in-memory parser;
        // large histories and decompression never allocate a whole-source Vec.
        let small_limit = buffer_budget.clamp(1, 64 * 1024);
        if !compressed && source_bytes <= small_limit as u64 {
            let mut bytes = Vec::new();
            reader
                .take(small_limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| invalid("sourceReadFailed"))?;
            if bytes.len() > small_limit {
                return Err(invalid("sourceChangedRetry"));
            }
            let text = std::str::from_utf8(&bytes).map_err(|_| invalid("invalidSourceEncoding"))?;
            return parse(text, &path.to_string_lossy());
        }
        parse_stream(reader, buffer_budget)
    })
}

fn parse_stream(reader: impl Read, buffer_budget: usize) -> Result<NativeSnapshot, AppError> {
    let capacity = buffer_budget.clamp(1, 64 * 1024);
    let line_limit = MAX_LINE_BYTES.min((buffer_budget / 2).max(1));
    let mut reader = BufReader::with_capacity(capacity, reader.take(MAX_STREAM_BYTES + 1));
    let mut line = Vec::with_capacity(capacity.min(line_limit));
    let mut oversized = false;
    let mut parse_enabled = true;
    let mut total = 0u64;
    let mut hash = Sha256::new();
    let mut snapshot = NativeSnapshot {
        version: VERSION,
        main: true,
        ..Default::default()
    };
    let mut parser = Parser::default();
    let mut record_index = 0;
    loop {
        let chunk = reader.fill_buf().map_err(|_| invalid("sourceReadFailed"))?;
        if chunk.is_empty() {
            break;
        }
        let count = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |index| index + 1);
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| invalid("sourceBudgetExceeded"))?;
        if total > MAX_STREAM_BYTES {
            return Err(invalid("sourceBudgetExceeded"));
        }
        hash.update(&chunk[..count]);
        if !oversized {
            if line
                .len()
                .checked_add(count)
                .is_none_or(|len| len > line_limit.saturating_add(2))
            {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(&chunk[..count]);
            }
        }
        let end = chunk[count - 1] == b'\n';
        reader.consume(count);
        if end {
            if oversized {
                snapshot.reasons.insert("sourceLineBudgetExceeded".into());
                parser.cut_activity_stream();
            } else if parse_enabled {
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                let text =
                    std::str::from_utf8(&line).map_err(|_| invalid("invalidSourceEncoding"))?;
                parse_enabled = observe_line(
                    &mut parser,
                    &mut snapshot,
                    &mut record_index,
                    text,
                    line_limit,
                );
            }
            line.clear();
            oversized = false;
        }
    }
    if oversized {
        snapshot.reasons.insert("sourceLineBudgetExceeded".into());
        parser.cut_activity_stream();
    } else if parse_enabled && !line.is_empty() {
        let text = std::str::from_utf8(&line).map_err(|_| invalid("invalidSourceEncoding"))?;
        observe_line(
            &mut parser,
            &mut snapshot,
            &mut record_index,
            text,
            line_limit,
        );
    }
    // Even after a projection budget stops decoding, consume/hash to EOF. A
    // truncated digest must never masquerade as proof of byte-identical files.
    snapshot.source_digest = format!("{:x}", hash.finalize());
    Ok(finish_projection(snapshot, parser))
}

pub(crate) fn parse(text: &str, _source_key: &str) -> Result<NativeSnapshot, AppError> {
    if text.len() > MAX_SOURCE_BYTES {
        return Err(invalid("sourceBudgetExceeded"));
    }
    let mut snapshot = NativeSnapshot {
        version: VERSION,
        main: true,
        source_digest: digest(text.as_bytes()),
        ..Default::default()
    };
    let mut parser = Parser::default();
    let mut record_index = 0u64;
    for line in text.lines() {
        if !observe_line(
            &mut parser,
            &mut snapshot,
            &mut record_index,
            line,
            MAX_LINE_BYTES,
        ) {
            break;
        }
    }
    Ok(finish_projection(snapshot, parser))
}

fn observe_line(
    parser: &mut Parser,
    snapshot: &mut NativeSnapshot,
    record_index: &mut u64,
    line: &str,
    line_limit: usize,
) -> bool {
    if line.trim().is_empty() {
        return true;
    }
    if line.len() > line_limit {
        snapshot.reasons.insert("sourceLineBudgetExceeded".into());
        parser.cut_activity_stream();
        return true;
    }
    if snapshot.record_hashes.len() >= MAX_ITEMS {
        snapshot.reasons.insert("projectionBudgetExceeded".into());
        parser.cut_activity_stream();
        return false;
    }
    let value: Value = match serde_json::from_str(line) {
        Ok(Value::Object(value)) => Value::Object(value),
        _ => {
            snapshot.reasons.insert("malformedNativeRecord".into());
            parser.cut_activity_stream();
            return true;
        }
    };
    snapshot
        .record_hashes
        .push(digest_json(&json!(["codex-native-row-v1", value])));
    // The canonical metadata is the first parsed header, even if its ordering
    // metadata is invalid. A later copied header can never replace its identity.
    parser.capture_header(&value, snapshot);
    let ordinal = parser.ordinal(&value, *record_index, snapshot);
    *record_index = record_index.saturating_add(1);
    if let Some(ordinal) = ordinal {
        parser.observe(&value, ordinal, snapshot);
    } else {
        parser.cut_activity_stream();
    }
    if parser.state.entries.len() > MAX_ITEMS
        || snapshot.activity.len() > MAX_ITEMS
        || snapshot.messages.len() > MAX_ITEMS
        || parser.state.work_exceeded.get()
    {
        snapshot.reasons.insert("projectionBudgetExceeded".into());
        parser.cut_activity_stream();
        return false;
    }
    true
}

fn finish_projection(mut snapshot: NativeSnapshot, parser: Parser) -> NativeSnapshot {
    if snapshot.native_thread_id.is_none() {
        snapshot
            .reasons
            .insert("missingNativeSessionIdentity".into());
    }
    if parser.state.conflicts > 0 {
        snapshot.reasons.insert("conflictingNativeUsage".into());
    }
    if !parser.state.counter_gaps.is_empty() || !parser.state.deferred_counts.is_empty() {
        snapshot.reasons.insert("unprovenCounterRevision".into());
    }
    let native = snapshot.native_thread_id.as_deref().unwrap_or("");
    for entry in parser.state.entries {
        if entry.excluded {
            continue;
        }
        let at_ms = (entry.at != i64::MIN).then_some(entry.at);
        let model = if entry.model.is_empty() {
            "unknown".into()
        } else {
            entry.model
        };
        if at_ms.is_none() {
            snapshot.reasons.insert("missingTimestamp".into());
        }
        if model == "unknown" {
            snapshot.reasons.insert("missingModel".into());
        }
        // The native inclusive input cannot prove fresh input when cache reads
        // are absent. Keep the coverage gap rather than turning unknown into zero.
        if entry.usage.known & 7 != 7 {
            snapshot.reasons.insert("missingNativeCounters".into());
            snapshot.reasons.insert("freshInputUnavailable".into());
            continue;
        }
        let complete = entry.usage.known & 15 == 15;
        if !complete {
            snapshot.reasons.insert("missingNativeCounters".into());
        }
        let provenance = if entry.response.is_empty() {
            "legacyInterval"
        } else {
            "nativeResponse"
        };
        let key = if entry.response.is_empty() {
            digest_json(&json!([
                "codex-native-legacy-v1",
                native,
                entry.session,
                entry.epoch,
                at_ms,
                entry.turn,
                entry.usage,
                entry.count_total.or(entry.expected_count).or(entry.total)
            ]))
        } else {
            digest_json(&json!(["codex-native-response-v1", native, entry.response]))
        };
        let interval_identity = entry.response.is_empty().then(|| {
            digest_json(&json!([
                "codex-native-legacy-interval-v1",
                native,
                entry.session,
                entry.epoch,
                at_ms,
                entry.turn,
                entry.ordinal
            ]))
        });
        snapshot.events.push(NativeUsageEvent {
            key,
            interval_identity,
            response_id: (!entry.response.is_empty()).then_some(entry.response),
            native_thread_id: native.to_owned(),
            billing_session_id: (!entry.session.is_empty()).then_some(entry.session),
            at_ms,
            model,
            buckets: entry.usage.buckets(),
            complete,
            canonical_ordinal: entry.ordinal,
            counter_epoch: entry.epoch,
            provenance: provenance.into(),
        });
    }
    snapshot.events.sort_by(|a, b| {
        a.canonical_ordinal
            .cmp(&b.canonical_ordinal)
            .then_with(|| a.key.cmp(&b.key))
    });
    snapshot.complete = snapshot.reasons.is_empty();
    snapshot
}

fn invalid(reason: &str) -> AppError {
    AppError::Message(format!("Codex native usage: {reason}"))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest_json(value: &Value) -> String {
    digest(value.to_string().as_bytes())
}
fn identity(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 512 && !v.chars().any(char::is_control))
        .map(str::to_owned)
}
fn at(value: &Value) -> Option<i64> {
    super::usage_source::timestamp_ms(value.get("timestamp").and_then(Value::as_str))
}

#[derive(Default)]
struct Parser {
    state: UsageState,
    header_seen: bool,
    canonical_at: Option<i64>,
    inherited: bool,
    history_start: Option<u64>,
    paginated: bool,
    explicit_own: bool,
    foreign_future: bool,
    current_model: String,
    seen_messages: BTreeSet<String>,
    ordinal_next: Option<u64>,
    ordinal_native_seen: bool,
    ordinal_has_base: bool,
    ordinal_current_trusted: bool,
    stream_segment: u32,
}
impl Parser {
    fn cut_activity_stream(&mut self) {
        self.stream_segment = self.stream_segment.saturating_add(1);
        self.state.pending = None;
    }
    fn capture_header(&mut self, value: &Value, snapshot: &mut NativeSnapshot) {
        if self.header_seen || value["type"] != "session_meta" {
            return;
        }
        self.header_seen = true;
        let p = &value["payload"];
        snapshot.native_thread_id = identity(p.get("id"));
        snapshot.billing_session_id = identity(p.get("session_id").or_else(|| p.get("sessionId")))
            .or_else(|| snapshot.native_thread_id.clone());
        snapshot.parent_thread_id = identity(p.get("parent_thread_id"))
            .or_else(|| identity(p.pointer("/source/subagent/thread_spawn/parent_thread_id")));
        snapshot.forked_from_id = identity(p.get("forked_from_id"));
        snapshot.project_dir = identity(p.get("cwd"));
        self.canonical_at = at(value);
        self.history_start = p
            .get("subagent_history_start_ordinal")
            .and_then(Value::as_u64);
        self.paginated = p["history_mode"] == "paginated";
        self.inherited = snapshot.forked_from_id.is_some() || self.history_start.is_some();
        self.state
            .context(snapshot.billing_session_id.as_deref().unwrap_or(""), "", "");
        if self.paginated {
            self.ordinal_has_base = p.get("history_base").is_some_and(|base| !base.is_null());
            self.ordinal_next = if self.ordinal_has_base {
                p.pointer("/history_base/end_ordinal_exclusive")
                    .and_then(Value::as_u64)
            } else {
                Some(0)
            };
            if self.ordinal_next.is_none() {
                snapshot.reasons.insert("invalidNativeOrdinal".into());
            }
        }
        if self.paginated
            && p.get("history_base").is_none_or(Value::is_null)
            && self.history_start.is_some_and(|v| v > 0)
        {
            self.state.snapshot_at = self.canonical_at;
            self.state.snapshot_thread = snapshot.native_thread_id.clone();
        }
    }
    fn ordinal(
        &mut self,
        value: &Value,
        record_index: u64,
        snapshot: &mut NativeSnapshot,
    ) -> Option<u64> {
        let expected = self.ordinal_next.unwrap_or(record_index);
        self.ordinal_current_trusted = true;
        let ordinal = match value.get("ordinal").filter(|value| !value.is_null()) {
            Some(raw) => {
                self.ordinal_native_seen = true;
                let Some(ordinal) = raw.as_u64() else {
                    self.ordinal_current_trusted = false;
                    snapshot.reasons.insert("invalidNativeOrdinal".into());
                    return None;
                };
                if ordinal < expected {
                    self.ordinal_current_trusted = false;
                    snapshot.reasons.insert("nativeOrdinalDiscontinuity".into());
                    return None;
                }
                if ordinal > expected {
                    snapshot.reasons.insert("nativeOrdinalDiscontinuity".into());
                    self.cut_activity_stream();
                }
                ordinal
            }
            None => {
                if self.ordinal_native_seen {
                    self.ordinal_current_trusted = false;
                    snapshot.reasons.insert("missingNativeOrdinal".into());
                    return None;
                }
                expected
            }
        };
        self.ordinal_next = ordinal.checked_add(1);
        if self.ordinal_next.is_none() {
            snapshot.reasons.insert("invalidNativeOrdinal".into());
        }
        Some(ordinal)
    }
    fn observe(&mut self, value: &Value, ordinal: u64, snapshot: &mut NativeSnapshot) {
        let p = &value["payload"];
        let top = value["type"].as_str().unwrap_or("");
        let time = at(value);
        if top == "session_meta" {
            // Codex 0.162 keeps copied source headers after the new thread's
            // canonical first header. They are history, never a new identity.
            if identity(p.get("id")) != snapshot.native_thread_id {
                self.inherited = true;
                self.explicit_own = false;
                self.state.pending = None;
                if self
                    .canonical_at
                    .zip(time)
                    .is_none_or(|(created, time)| time >= created)
                {
                    self.cut_activity_stream();
                }
                return;
            }
        }
        let native = snapshot.native_thread_id.as_deref().unwrap_or("");
        if native.is_empty() {
            self.cut_activity_stream();
            if matches!(top, "token_usage_record" | "compacted") || p["type"] == "token_count" {
                snapshot
                    .reasons
                    .insert("missingNativeSessionIdentity".into());
            }
            return;
        }
        // A same-timestamp turn can precede the first explicit own response.
        // Retain only its model keyed by native turn identity; do not classify
        // its messages, activity, or usage as child-owned by doing so.
        if top == "turn_context" {
            if let (Some(turn), Some(model)) =
                (identity(p.get("turn_id")), identity(p.get("model")))
            {
                self.state.models.insert(turn, model);
            }
        }
        let record = if top == "compacted" {
            &p["latest_token_usage_record"]
        } else {
            p
        };
        let explicit_thread = if matches!(top, "token_usage_record" | "compacted") {
            identity(record.get("thread_id"))
        } else {
            None
        };
        if explicit_thread.as_deref().is_some_and(|id| id != native) {
            self.state.pending = None;
            self.inherited = true;
            self.explicit_own = false;
            self.foreign_future |= self
                .canonical_at
                .zip(time)
                .is_none_or(|(created, time)| time >= created);
            if self.foreign_future {
                self.cut_activity_stream();
                snapshot.reasons.insert("unprovenInheritedBoundary".into());
            }
            return;
        }
        if explicit_thread.as_deref() == Some(native) {
            self.explicit_own = true;
            self.foreign_future = false;
        }
        // Native paginated ordinals start at history_base's exclusive end.
        // Old sanitized snapshots without ordinals/base retain the previous
        // explicit-thread/turn fallback; they never assert ordinal ownership.
        let ordinal_boundary = self.history_start.is_some()
            && (!self.paginated || self.ordinal_native_seen || self.ordinal_has_base);
        let ordinal_owned = ordinal_boundary
            && self.ordinal_current_trusted
            && self.history_start.is_some_and(|start| ordinal >= start);
        let before_creation = self
            .canonical_at
            .zip(time)
            .is_some_and(|(created, time)| time < created);
        let new_turn = top == "turn_context" || top == "event_msg" && p["type"] == "task_started";
        let new_turn_after_creation = !ordinal_boundary
            && !self.foreign_future
            && new_turn
            && self
                .canonical_at
                .zip(time)
                .is_some_and(|(created, time)| time > created);
        if ordinal_owned || new_turn_after_creation {
            self.explicit_own = true;
        }
        let own = top == "session_meta"
            || if ordinal_boundary {
                ordinal_owned
            } else {
                explicit_thread.as_deref() == Some(native)
                    || !(self.inherited || snapshot.parent_thread_id.is_some() && before_creation)
                    || !before_creation && (self.explicit_own || ordinal_owned)
            };
        if !own {
            self.state.pending = None;
            let proven_inherited = before_creation
                || ordinal_boundary && self.history_start.is_some_and(|start| ordinal < start);
            if !proven_inherited {
                self.cut_activity_stream();
                snapshot.reasons.insert("unprovenInheritedBoundary".into());
            }
            return;
        }
        if snapshot.title_hint.is_none() {
            let title = |text: &str| {
                use crate::session_manager::title::{self, TitleCandidates};
                if title::is_context(text) || title::clean_prompt(text).is_none() {
                    return None;
                }
                title::pick_title(&TitleCandidates::default().with_first_user_text(Some(text)))
                    .map(|(value, _)| title::one_line(&crate::redact::scrub(&value)))
            };
            snapshot.title_hint = if top == "event_msg" && p["type"] == "user_message" {
                p["message"].as_str().and_then(title)
            } else if top == "response_item" && p["type"] == "message" && p["role"] == "user" {
                p["content"].as_str().and_then(title).or_else(|| {
                    p["content"].as_array().and_then(|parts| {
                        parts
                            .iter()
                            .filter(|part| {
                                matches!(part["type"].as_str(), Some("input_text" | "text"))
                            })
                            .filter_map(|part| part["text"].as_str())
                            .find_map(title)
                    })
                })
            } else {
                None
            };
        }
        if let Some(at_ms) = time {
            snapshot.activity.push(NativeActivity {
                key: digest_json(&json!(["codex-activity-v1", native, ordinal, at_ms])),
                at_ms,
                canonical_ordinal: ordinal,
                stream_segment: self.stream_segment,
            });
        } else {
            snapshot.reasons.insert("activityTimeUnavailable".into());
            self.cut_activity_stream();
        }
        match top {
            "turn_context" => {
                let turn = identity(p.get("turn_id")).unwrap_or_default();
                let model = identity(p.get("model")).unwrap_or_default();
                self.state.context("", &turn, &model);
                if !model.is_empty() {
                    self.current_model = model;
                }
                if snapshot.project_dir.is_none() {
                    snapshot.project_dir = identity(p.get("cwd"));
                }
            }
            "token_usage_record" => {
                self.observe_record(p, time, ordinal, false, snapshot);
            }
            "compacted" => {
                let response = identity(p.get("compaction_response_id"));
                if response.is_some() && response == identity(record.get("response_id")) {
                    self.observe_record(record, time, ordinal, true, snapshot);
                } else if !record.is_null() {
                    snapshot.reasons.insert("missingCompactionIdentity".into());
                }
            }
            "event_msg" if p["type"] == "task_started" => {
                self.state
                    .context("", &identity(p.get("turn_id")).unwrap_or_default(), "");
                self.state.pending = None;
            }
            "event_msg" if p["type"] == "thread_settings_applied" => {
                if let Some(model) = identity(p.pointer("/thread_settings/model")) {
                    self.state.context("", "", &model);
                    self.current_model = model;
                }
            }
            "event_msg" if p["type"] == "user_message" => self.state.pending = None,
            "event_msg" if p["type"] == "token_count" => {
                if p["info"].is_null() {
                    return;
                }
                let total = Usage::from_value(&p["info"]["total_token_usage"]);
                let last = Usage::from_value(&p["info"]["last_token_usage"]);
                if let Some(total) = total {
                    if (self.inherited || snapshot.parent_thread_id.is_some())
                        && self.state.high.is_none()
                        && last.is_none()
                    {
                        self.state.advance(time.unwrap_or(i64::MIN), total);
                        snapshot.reasons.insert("unprovenInheritedBaseline".into());
                        return;
                    }
                    self.state.count(
                        time.unwrap_or(i64::MIN),
                        &self.current_model,
                        total,
                        last,
                        ordinal,
                    );
                } else {
                    snapshot.reasons.insert("invalidNativeUsage".into());
                }
            }
            "response_item" if p["type"] == "message" => {
                let role = p["role"].as_str().unwrap_or("");
                if role == "user" {
                    self.state.pending = None;
                }
                let real_user = role == "user"
                    && p["content"].as_array().is_some_and(|parts| {
                        parts.iter().any(|part| {
                            matches!(part["type"].as_str(), Some("input_text" | "text"))
                                && part["text"].as_str().is_some_and(|text| {
                                    !crate::session_manager::title::is_context(text)
                                        && crate::session_manager::title::clean_prompt(text)
                                            .is_some()
                                })
                        })
                    });
                if role == "assistant" || real_user {
                    // Only the digest survives; message text and tool output are
                    // intentionally excluded from this cache projection.
                    let key = digest_json(&json!([
                        "codex-message-v1",
                        native,
                        time,
                        role,
                        identity(p.get("id")),
                        digest_json(p)
                    ]));
                    if self.seen_messages.insert(key.clone()) {
                        snapshot.messages.push(NativeMessage {
                            key,
                            role: role.into(),
                            at_ms: time,
                            canonical_ordinal: ordinal,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    fn observe_record(
        &mut self,
        record: &Value,
        time: Option<i64>,
        ordinal: u64,
        compaction: bool,
        snapshot: &mut NativeSnapshot,
    ) {
        let Some(usage) = Usage::from_value(&record["usage"]) else {
            snapshot.reasons.insert("invalidNativeUsage".into());
            return;
        };
        let Some(response) = identity(record.get("response_id")) else {
            snapshot.reasons.insert("missingNativeEventIdentity".into());
            return;
        };
        let contribution = Contribution {
            response,
            session: identity(record.get("session_id")).unwrap_or_default(),
            turn: identity(record.get("turn_id")).unwrap_or_default(),
            model: self.current_model.clone(),
            at: time.unwrap_or(i64::MIN),
            updated: time.unwrap_or(i64::MIN),
            usage,
            total: Usage::from_value(&record["thread_token_usage"]),
            ordinal,
            compaction,
            ..Default::default()
        };
        if compaction {
            self.state
                .compacted(contribution, identity(record.get("thread_id")).as_deref());
        } else {
            self.state.record(contribution);
        }
    }
}

// Native order: inclusive input, output, cache read, cache write, reasoning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
struct Usage {
    values: [u64; 5],
    known: u8,
}
impl Usage {
    fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let mut usage = Self::default();
        for (index, name) in [
            "input_tokens",
            "output_tokens",
            "cached_input_tokens",
            "cache_write_input_tokens",
            "reasoning_output_tokens",
        ]
        .into_iter()
        .enumerate()
        {
            let field = object.get(name).or_else(|| {
                (index == 2)
                    .then(|| object.get("cache_read_input_tokens"))
                    .flatten()
            });
            if let Some(field) = field.filter(|v| !v.is_null()) {
                usage.values[index] = field.as_u64()?;
                usage.known |= 1 << index;
            }
        }
        (usage.known & 3 == 3 && usage.valid()).then_some(usage)
    }
    fn valid(self) -> bool {
        self.values[2]
            .checked_add(self.values[3])
            .is_some_and(|cache| cache <= self.values[0])
    }
    fn covers(self, other: Self) -> bool {
        self.values
            .into_iter()
            .zip(other.values)
            .all(|(a, b)| a >= b)
    }
    fn less(self, other: Self) -> Self {
        let mut values = [0; 5];
        for ((out, a), b) in values.iter_mut().zip(self.values).zip(other.values) {
            *out = a.saturating_sub(b);
        }
        Self {
            values,
            known: self.known & other.known,
        }
    }
    fn plus(self, other: Self) -> Option<Self> {
        let mut values = [0; 5];
        for ((out, a), b) in values.iter_mut().zip(self.values).zip(other.values) {
            *out = a.checked_add(b)?;
        }
        Some(Self {
            values,
            known: self.known & other.known,
        })
    }
    fn rebase(self, previous: Self, revised: Self) -> Option<Self> {
        let mut values = [0; 5];
        for ((value, current), (old, new)) in values
            .iter_mut()
            .zip(self.values)
            .zip(previous.values.into_iter().zip(revised.values))
        {
            *value = if new >= old {
                current.checked_add(new - old)?
            } else {
                current.checked_sub(old - new)?
            };
        }
        let result = Self {
            values,
            known: self.known & previous.known & revised.known,
        };
        result.valid().then_some(result)
    }
    fn zero(self) -> bool {
        self.values == [0; 5]
    }
    fn matches(self, other: Self) -> bool {
        self.known & 3 == 3
            && other.known & 3 == 3
            && self
                .values
                .into_iter()
                .zip(other.values)
                .enumerate()
                .all(|(i, (a, b))| self.known & other.known & (1 << i) == 0 || a == b)
    }
    fn buckets(self) -> [u64; 4] {
        [
            self.values[0]
                .saturating_sub(self.values[2])
                .saturating_sub(self.values[3]),
            self.values[1],
            self.values[2],
            self.values[3],
        ]
    }
}

#[derive(Clone, Debug, Default)]
struct Contribution {
    response: String,
    session: String,
    turn: String,
    model: String,
    epoch: u32,
    at: i64,
    updated: i64,
    usage: Usage,
    total: Option<Usage>,
    count_total: Option<Usage>,
    count_epoch: u32,
    expected_count: Option<Usage>,
    compaction: bool,
    excluded: bool,
    ordinal: u64,
    revised: bool,
    replaced_count: Option<Usage>,
}
impl Contribution {
    fn checkpoint(&self) -> (Option<Usage>, u32) {
        if self.count_total.is_some() {
            (self.count_total, self.count_epoch)
        } else if self.expected_count.is_some() {
            (self.expected_count, self.epoch)
        } else {
            (self.total, self.epoch)
        }
    }
    fn interval_key(&self) -> Option<IntervalKey> {
        let (total, epoch) = self.checkpoint();
        let total = total?;
        Some((
            self.session.clone(),
            epoch,
            total.values[0],
            total.values[1],
            self.usage.values[0],
            self.usage.values[1],
        ))
    }
    fn same_interval(&self, other: &Self) -> bool {
        let (a, ae) = self.checkpoint();
        let (b, be) = other.checkpoint();
        !self.compaction
            && !other.compaction
            && !self.excluded
            && !other.excluded
            && ae == be
            && self.session == other.session
            && (self.turn.is_empty() || other.turn.is_empty() || self.turn == other.turn)
            && a.zip(b).is_some_and(|(a, b)| a.matches(b))
            && self.usage.matches(other.usage)
    }
}
#[derive(Clone, Debug)]
struct CounterBase {
    session: String,
    epoch: u32,
    at: i64,
    previous_at: i64,
    base: Usage,
    start: Usage,
    confirmed: bool,
}
type IntervalKey = (String, u32, u64, u64, u64, u64);
type CountKey = [u8; 32];
#[derive(Clone)]
struct DeferredCount {
    session: String,
    turn: String,
    epoch: u32,
    at: i64,
    total: Usage,
    last: Usage,
    ordinal: u64,
}
#[derive(Default)]
struct UsageState {
    pending: Option<usize>,
    entries: Vec<Contribution>,
    intervals: BTreeMap<IntervalKey, Vec<usize>>,
    responses: BTreeMap<(String, String), usize>,
    high: Option<Usage>,
    high_at: i64,
    epoch: u32,
    session: String,
    turn: String,
    models: BTreeMap<String, String>,
    conflicts: usize,
    compactions: BTreeMap<(String, String), Contribution>,
    counter_bases: Vec<CounterBase>,
    snapshot_at: Option<i64>,
    snapshot_thread: Option<String>,
    snapshot_total: Option<Usage>,
    snapshot_offset: Option<Usage>,
    thread_domain_epoch: u32,
    observed_counts: BTreeMap<CountKey, BTreeSet<u32>>,
    deferred_counts: BTreeMap<(u32, CountKey), DeferredCount>,
    unpaired_revisions: BTreeSet<usize>,
    counter_gaps: BTreeSet<u32>,
    work: Cell<usize>,
    work_exceeded: Cell<bool>,
}

impl UsageState {
    fn charge_work(&self, amount: usize) -> bool {
        let work = self.work.get().saturating_add(amount);
        self.work.set(work);
        if work > MAX_RECONCILIATION_WORK {
            self.work_exceeded.set(true);
            return false;
        }
        true
    }
    fn context(&mut self, session: &str, turn: &str, model: &str) {
        if !turn.is_empty() && turn != self.turn {
            self.pending = None;
        }
        if !session.is_empty() {
            self.session = session.into();
        }
        if !turn.is_empty() {
            self.turn = turn.into();
        }
        if !model.is_empty() && !self.turn.is_empty() {
            self.models.insert(self.turn.clone(), model.into());
        }
    }
    fn model(&self, turn: &str, fallback: &str) -> String {
        self.models
            .get(turn)
            .filter(|_| !turn.is_empty())
            .cloned()
            .unwrap_or_else(|| {
                if turn.is_empty() {
                    fallback.into()
                } else {
                    String::new()
                }
            })
    }
    fn advance(&mut self, at: i64, total: Usage) {
        if self.high.is_none_or(|high| total.covers(high)) {
            if self.high.is_none() || at > self.high_at {
                self.high_at = at;
            }
            self.high = Some(total);
        }
    }
    fn count_key(&self, at: i64, total: Usage, last: Option<Usage>) -> CountKey {
        let mut hash = Sha256::new();
        hash.update(b"codex-observed-counter-v2");
        hash.update((self.session.len() as u64).to_be_bytes());
        hash.update(self.session.as_bytes());
        // Legacy COUNT has no turn or runtime identity. Those are evidence
        // attached when it is paired, never identity borrowed from its replay.
        hash.update(at.to_be_bytes());
        for value in total.values {
            hash.update(value.to_be_bytes());
        }
        hash.update([total.known, u8::from(last.is_some())]);
        if let Some(last) = last {
            for value in last.values {
                hash.update(value.to_be_bytes());
            }
            hash.update([last.known]);
        }
        hash.finalize().into()
    }
    fn remember_count(&mut self, at: i64, total: Usage, last: Option<Usage>, epoch: u32) {
        if self.observed_counts.len() >= MAX_ITEMS {
            self.work_exceeded.set(true);
            self.counter_gaps.insert(self.epoch);
            return;
        }
        let key = self.count_key(at, total, last);
        self.observed_counts.entry(key).or_default().insert(epoch);
    }
    fn pair_count(
        &mut self,
        index: usize,
        mut value: Contribution,
        at: i64,
        total: Usage,
        last: Option<Usage>,
        replace_frontier: bool,
    ) {
        let correction = value.revised;
        let previous = value.replaced_count;
        let epoch = if correction {
            value.count_epoch
        } else {
            self.epoch
        };
        value.count_total = Some(total);
        value.count_epoch = epoch;
        value.revised = false;
        value.replaced_count = None;
        self.confirm_base(&value);
        self.entries[index] = value.clone();
        self.index(index);
        self.unpaired_revisions.remove(&index);
        self.pending = None;
        self.remember_count(at, total, last, epoch);
        if correction {
            // A revision of an older runtime cannot rewrite the current
            // runtime's frontier. The response itself still keeps its update.
            if epoch != self.epoch {
                return;
            }
            let Some(previous) = previous else {
                self.counter_gaps.insert(epoch);
                return;
            };
            let Some(high) = self.high else {
                self.high = Some(total);
                self.high_at = at;
                return;
            };
            let Some(next) = high
                .rebase(previous, total)
                .filter(|_| high.covers(previous))
            else {
                self.counter_gaps.insert(epoch);
                return;
            };
            if !self.charge_work(self.entries.len()) {
                self.counter_gaps.insert(epoch);
                return;
            }
            let mut shifted = Vec::new();
            for (other_index, other) in self.entries.iter_mut().enumerate() {
                let (checkpoint, other_epoch) = other.checkpoint();
                if other_index == index
                    || other.excluded
                    || other.compaction
                    || other.session != value.session
                    || other_epoch != epoch
                    || (other.at, other.ordinal) <= (value.at, value.ordinal)
                {
                    continue;
                }
                let Some(checkpoint) = checkpoint.filter(|checkpoint| checkpoint.covers(previous))
                else {
                    continue;
                };
                let Some(rebased) = checkpoint.rebase(previous, total) else {
                    self.counter_gaps.insert(epoch);
                    continue;
                };
                if other.count_total.is_some() {
                    other.count_total = Some(rebased);
                } else if other.expected_count.is_some() {
                    other.expected_count = Some(rebased);
                } else {
                    other.total = Some(rebased);
                }
                shifted.push(other_index);
            }
            for index in shifted {
                self.index(index);
            }
            self.high = Some(next);
            self.high_at = self.high_at.max(at);
        } else if replace_frontier {
            self.high = Some(total);
            self.high_at = at;
        } else {
            self.advance(at, total);
        }
    }
    fn resolve_deferred(&mut self, index: usize) {
        let value = &self.entries[index];
        if !value.revised || value.compaction || !self.charge_work(self.deferred_counts.len()) {
            return;
        }
        let keys: Vec<_> = self
            .deferred_counts
            .iter()
            .filter(|(_, count)| {
                count.session == value.session
                    && (count.turn.is_empty()
                        || value.turn.is_empty()
                        || count.turn == value.turn
                        || count.at == value.updated)
                    && count.epoch == value.count_epoch
                    && count.at >= value.at
                    && value.usage.matches(count.last)
                    && value
                        .expected_count
                        .or(value.total)
                        .is_some_and(|total| total.matches(count.total))
            })
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            if let Some(count) = self.deferred_counts.remove(&key) {
                self.count(count.at, "", count.total, Some(count.last), count.ordinal);
            }
        }
    }
    fn index(&mut self, index: usize) {
        if let Some(key) = self.entries[index].interval_key() {
            let indices = self.intervals.entry(key).or_default();
            if indices.last() != Some(&index) {
                indices.push(index);
            }
        }
    }
    fn candidates(&self, value: &Contribution) -> Vec<usize> {
        let candidates = value
            .interval_key()
            .and_then(|key| self.intervals.get(&key));
        candidates
            .filter(|candidates| self.charge_work(candidates.len()))
            .cloned()
            .unwrap_or_default()
    }
    fn align_snapshot(&mut self, at: i64, total: Usage, last: Option<Usage>) {
        let snapshot_thread = self.snapshot_thread.take();
        let Some(snapshot_total) = self.snapshot_total else {
            return;
        };
        if snapshot_thread.is_none()
            || self.high.is_some()
            || self.snapshot_at != Some(at)
            || last.is_none_or(|last| last.known != 31 || !last.zero())
            || total.known != 31
            || snapshot_total.known != 31
            || !snapshot_total.covers(total)
            || self
                .entries
                .iter()
                .any(|entry| !entry.compaction || entry.at != at)
        {
            return;
        }
        let mut offset = snapshot_total.less(total);
        for compaction in self.compactions.values() {
            if compaction.session != self.session
                || compaction.usage.known != 31
                || compaction.at != at
                || !offset.covers(compaction.usage)
            {
                return;
            }
            offset = offset.less(compaction.usage);
        }
        self.snapshot_offset = Some(offset);
    }
    fn project_count(&self, value: &Contribution) -> (Option<Usage>, u32) {
        if !self.charge_work(
            self.compactions
                .len()
                .saturating_add(self.counter_bases.len())
                .saturating_add(1),
        ) {
            return (None, value.epoch);
        }
        let Some(total) = value.total.filter(|_| !value.compaction) else {
            return (None, value.epoch);
        };
        let mut offset = Usage {
            known: 31,
            ..Default::default()
        };
        if let Some(saved_offset) = self.snapshot_offset.filter(|_| {
            self.snapshot_total.is_some_and(|base| total.covers(base))
                && self.snapshot_at.is_some_and(|at| value.at >= at)
        }) {
            offset = saved_offset;
        }
        for compaction in self.compactions.values() {
            if compaction.session != value.session
                || value.at < compaction.at
                || compaction.total.is_some_and(|c| !total.covers(c))
            {
                continue;
            }
            let Some(next) = offset.plus(compaction.usage) else {
                return (None, value.epoch);
            };
            offset = next;
        }
        if !total.covers(offset) {
            return (None, value.epoch);
        }
        let mut projected = total.less(offset);
        let mut epoch = value.epoch;
        let mut base = None;
        for counter in &self.counter_bases {
            if counter.session != value.session {
                continue;
            }
            let first = counter.start.less(counter.base);
            let proven_first = projected.matches(counter.start)
                && value.usage.matches(first)
                && value.at <= counter.at;
            if projected.covers(counter.start) && (counter.confirmed || proven_first) {
                base = Some(counter.base);
                epoch = counter.epoch;
            } else if value.at <= counter.previous_at {
                epoch = epoch.min(counter.epoch.saturating_sub(1));
            }
        }
        if let Some(base) = base {
            projected = projected.less(base);
        }
        if offset.zero() && base.is_none() {
            (None, epoch)
        } else {
            (Some(projected), epoch)
        }
    }
    fn confirm_base(&mut self, value: &Contribution) {
        if value
            .count_total
            .zip(value.expected_count)
            .is_none_or(|(a, b)| !a.matches(b))
        {
            return;
        }
        for base in &mut self.counter_bases {
            if base.session == value.session && base.epoch == value.count_epoch {
                base.confirmed = true;
            }
        }
    }
    fn reset_thread_domain(&mut self) {
        self.thread_domain_epoch = self.epoch;
        self.counter_bases.clear();
        self.compactions.clear();
        self.snapshot_offset = None;
        self.snapshot_total = None;
    }
    fn compacted(&mut self, mut value: Contribution, thread: Option<&str>) {
        if value.session.is_empty() {
            value.session = self.session.clone();
        }
        let key = (value.session.clone(), value.response.clone());
        let Some(index) = self.record(value) else {
            if self
                .responses
                .get(&key)
                .is_some_and(|&index| self.entries[index].excluded)
            {
                self.compactions.remove(&key);
            }
            return;
        };
        self.entries[index].compaction = true;
        let value = self.entries[index].clone();
        if value.epoch < self.thread_domain_epoch {
            return;
        }
        self.compactions.insert(key, value.clone());
        if self.snapshot_thread.as_deref() == thread
            && self.snapshot_at == Some(value.at)
            && self.compactions.len() == 1
            && value.total.is_some()
        {
            self.snapshot_total = value.total;
        }
    }
    fn record(&mut self, mut value: Contribution) -> Option<usize> {
        let pending = self.pending.take();
        if value.response.is_empty() || !value.usage.valid() || value.usage.known & 3 != 3 {
            return None;
        }
        if value.session.is_empty() {
            value.session = self.session.clone();
        } else if self.session.is_empty() {
            self.session = value.session.clone();
        }
        if value.turn.is_empty() {
            value.turn = self.turn.clone();
        }
        value.epoch = self.epoch;
        value.model = self.model(&value.turn, &value.model);
        value.updated = value.at;
        (value.expected_count, value.epoch) = self.project_count(&value);
        let key = (value.session.clone(), value.response.clone());
        if let Some(&index) = self.responses.get(&key) {
            let old = self.entries[index].clone();
            if old.excluded {
                return None;
            }
            value.epoch = old.epoch;
            value.count_total = old.count_total;
            value.count_epoch = old.count_epoch;
            value.compaction = old.compaction;
            value.total = value.total.or(old.total);
            if !old.turn.is_empty() && !value.turn.is_empty() && old.turn != value.turn {
                self.conflicts += 1;
                self.entries[index].excluded = true;
                self.compactions.remove(&key);
                return None;
            }
            if old.usage == value.usage
                && old.model == value.model
                && (old.total.is_some() || value.total.is_none())
                && old
                    .total
                    .zip(value.total)
                    .is_none_or(|(old, new)| old.matches(new))
            {
                if value.updated > old.updated {
                    self.entries[index].updated = value.updated;
                }
                if !old.compaction {
                    self.pending = Some(index);
                }
                return Some(index);
            }
            if value.updated <= old.updated {
                if value.updated == old.updated {
                    self.conflicts += 1;
                    self.entries[index].excluded = true;
                    self.compactions.remove(&key);
                }
                return None;
            }
            value.at = old.at;
            value.ordinal = old.ordinal;
            let counter_changed = old.usage != value.usage
                || old
                    .total
                    .zip(value.total)
                    .is_some_and(|(old, new)| !old.matches(new));
            value.revised = old.revised || counter_changed;
            value.replaced_count = old.replaced_count.or(old.count_total);
            if counter_changed {
                value.count_total = None;
            }
            if value.compaction && self.compactions.contains_key(&key) {
                self.compactions.insert(key.clone(), value.clone());
            }
            self.entries[index] = value;
            self.index(index);
            if counter_changed && !self.entries[index].compaction {
                self.unpaired_revisions.insert(index);
                self.pending = Some(index);
                self.resolve_deferred(index);
            }
            return Some(index);
        }
        let mut matched = None;
        let mut ambiguous = false;
        for index in self.candidates(&value) {
            let old = &self.entries[index];
            if old.response.is_empty() && old.same_interval(&value) {
                if matched.is_some_and(|matched| matched != index) {
                    ambiguous = true;
                    break;
                }
                matched = Some(index);
            }
        }
        if matched.is_none() && !ambiguous && !value.compaction {
            if let Some(index) = pending {
                let old = &self.entries[index];
                if !old.excluded
                    && old.response.is_empty()
                    && old.turn == value.turn
                    && old.session == value.session
                    && old.usage.matches(value.usage)
                    && value.at <= old.at
                    && old
                        .total
                        .zip(value.total)
                        .is_none_or(|(count, thread)| thread.covers(count))
                {
                    matched = Some(index);
                }
            }
        }
        if !ambiguous {
            if let Some(index) = matched {
                let old = &self.entries[index];
                value.count_total = old.total;
                value.count_epoch = old.epoch;
                self.confirm_base(&value);
                self.entries[index] = value;
                self.responses.insert(key, index);
                self.index(index);
                return Some(index);
            }
        } else {
            self.conflicts += 1;
        }
        let index = self.entries.len();
        let compaction = value.compaction;
        self.entries.push(value);
        self.index(index);
        self.responses.insert(key, index);
        if !compaction {
            self.pending = Some(index);
        }
        Some(index)
    }
    fn count(&mut self, at: i64, model: &str, total: Usage, mut last: Option<Usage>, ordinal: u64) {
        self.align_snapshot(at, total, last);
        if last.is_some_and(|last| last.known & 3 != 3) {
            self.conflicts += 1;
            last = None;
        }
        if let Some((index, last)) = self.pending.zip(last) {
            let old = &self.entries[index];
            if !old.excluded
                && !old.response.is_empty()
                && !old.compaction
                && old.session == self.session
                && old.usage.matches(last)
                && old.count_total.is_some_and(|count| count.matches(total))
                && at >= old.at
            {
                // The immediately preceding native response identifies this
                // already paired checkpoint even after the context moved to a
                // different turn. Its later refresh is not a new runtime.
                let epoch = old.count_epoch;
                self.pending = None;
                self.remember_count(at, total, Some(last), epoch);
                return;
            }
        }
        if let Some(last) = last.filter(|_| !self.responses.is_empty()) {
            let candidate = Contribution {
                epoch: self.epoch,
                session: self.session.clone(),
                turn: self.turn.clone(),
                usage: last,
                total: Some(total),
                ..Default::default()
            };
            for index in self.candidates(&candidate) {
                let old = self.entries[index].clone();
                if !old.same_interval(&candidate) {
                    continue;
                }
                if old.response.is_empty() || old.count_total.is_some() {
                    if self.pending != Some(index)
                        && self.high.is_some_and(|high| !total.covers(high))
                        && at > self.high_at
                        && total.matches(last)
                    {
                        continue;
                    }
                    self.remember_count(at, total, Some(last), self.epoch);
                    return;
                }
                let mut value = old.clone();
                if self
                    .high
                    .is_some_and(|high| !high.zero() && total.matches(last) && !total.matches(high))
                    && at > self.high_at
                    && old.at > self.high_at
                    && old.expected_count.is_none()
                    && old.total.is_some_and(|old_total| old_total.matches(total))
                    && !old.revised
                {
                    self.epoch = self.epoch.saturating_add(1);
                    self.high = None;
                    value.epoch = self.epoch;
                    self.reset_thread_domain();
                }
                self.pair_count(index, value, at, total, Some(last), false);
                return;
            }
        }
        if let Some((index, last)) = self.pending.zip(last) {
            let old = &self.entries[index];
            if !old.excluded
                && !old.response.is_empty()
                && old.count_total.is_none()
                && !old.compaction
                && (old.turn == self.turn || old.revised)
                && old.session == self.session
                && old.usage.matches(last)
                && at >= old.at
            {
                let mut value = old.clone();
                // A paired response proves the fresh legacy runtime boundary.
                // Moving High alone would make an equal later checkpoint look
                // like an old replay and inflate the next legacy-only delta.
                let reset = !value.revised
                    && self.high.is_some_and(|high| {
                        !high.zero()
                            && (!total.covers(high) || total.matches(last) && !total.matches(high))
                    })
                    && at > self.high_at;
                if reset {
                    self.epoch = self.epoch.saturating_add(1);
                    if value.total.is_some_and(|thread| thread.matches(total))
                        && value.expected_count.is_none()
                    {
                        self.reset_thread_domain();
                    } else if let Some(projected) = value
                        .expected_count
                        .or(value.total)
                        .filter(|thread| thread.covers(total))
                    {
                        let carry = self
                            .counter_bases
                            .iter()
                            .rev()
                            .find(|base| base.session == value.session && base.confirmed)
                            .map(|base| base.base)
                            .unwrap_or(Usage {
                                known: 31,
                                ..Default::default()
                            });
                        if let Some(base) = carry.plus(projected.less(total)) {
                            if let Some(start) = base.plus(total) {
                                self.counter_bases.push(CounterBase {
                                    session: value.session.clone(),
                                    epoch: self.epoch,
                                    at,
                                    previous_at: self.high_at,
                                    base,
                                    start,
                                    confirmed: true,
                                });
                                value.expected_count = Some(total);
                            }
                        }
                    }
                    value.epoch = self.epoch;
                }
                self.pair_count(index, value, at, total, Some(last), true);
                return;
            }
        }
        if self
            .observed_counts
            .get(&self.count_key(at, total, last))
            .is_some_and(|epochs| epochs.contains(&self.epoch) || at != i64::MIN)
        {
            return;
        }
        // A counter that completes before an already observed frontier cannot
        // establish a fresh runtime reset. Wait for response identity to prove
        // a revision; otherwise only the unproved legacy tail is unavailable.
        if at != i64::MIN && at < self.high_at && self.high.is_some() && !self.responses.is_empty()
        {
            if let Some(last) = last {
                if self.deferred_counts.len() >= MAX_ITEMS {
                    self.work_exceeded.set(true);
                    self.counter_gaps.insert(self.epoch);
                    return;
                }
                self.deferred_counts.insert(
                    (self.epoch, self.count_key(at, total, Some(last))),
                    DeferredCount {
                        session: self.session.clone(),
                        turn: self.turn.clone(),
                        epoch: self.epoch,
                        at,
                        total,
                        last,
                        ordinal,
                    },
                );
            } else {
                self.counter_gaps.insert(self.epoch);
            }
            self.pending = None;
            return;
        }
        if !self.charge_work(self.unpaired_revisions.len()) {
            self.counter_gaps.insert(self.epoch);
        }
        if self.unpaired_revisions.iter().any(|&index| {
            !self.entries[index].excluded && self.entries[index].count_epoch == self.epoch
        }) {
            self.counter_gaps.insert(self.epoch);
        }
        // A proved runtime reset supplies a new baseline. Old missing
        // revisions stay partial in the result but cannot block its tail.
        if self.counter_gaps.contains(&self.epoch)
            || self
                .deferred_counts
                .range((self.epoch, [0; 32])..=(self.epoch, [u8::MAX; 32]))
                .next()
                .is_some()
        {
            self.pending = None;
            return;
        }
        self.pending = None;
        let mut previous = self.high;
        let mut reset = false;
        let first_after_reset = previous.is_some_and(|previous| {
            !previous.zero()
                && last.is_some_and(|last| !last.zero() && total.matches(last))
                && !total.matches(previous)
        });
        if previous.is_some_and(|previous| !total.covers(previous)) || first_after_reset {
            self.epoch = self.epoch.saturating_add(1);
            if last.is_some_and(|last| {
                !self.responses.is_empty() && !last.zero() && total.matches(last)
            }) {
                let mut base = previous.unwrap();
                if let Some(counter) = self
                    .counter_bases
                    .iter()
                    .rev()
                    .find(|counter| counter.session == self.session && counter.confirmed)
                {
                    let Some(next) = base.plus(counter.base) else {
                        self.conflicts += 1;
                        return;
                    };
                    base = next;
                }
                let Some(start) = base.plus(total) else {
                    self.conflicts += 1;
                    return;
                };
                self.counter_bases.push(CounterBase {
                    session: self.session.clone(),
                    epoch: self.epoch,
                    at,
                    previous_at: self.high_at,
                    base,
                    start,
                    confirmed: false,
                });
                reset = true;
            }
            self.high = None;
            previous = None;
        }
        let mut value = Contribution {
            epoch: self.epoch,
            session: self.session.clone(),
            turn: self.turn.clone(),
            model: self.model(&self.turn, model),
            at,
            updated: at,
            total: Some(total),
            ordinal,
            ..Default::default()
        };
        if let Some(last) = last {
            value.usage = last;
            if reset {
                let mut matched = None;
                let mut ambiguous = false;
                for (index, old) in self.entries.iter().enumerate() {
                    if !self.charge_work(1) {
                        break;
                    }
                    if old.response.is_empty()
                        || old.excluded
                        || old.count_total.is_some()
                        || old.at > at
                    {
                        continue;
                    }
                    let mut candidate = old.clone();
                    (candidate.expected_count, candidate.epoch) = self.project_count(&candidate);
                    if candidate.same_interval(&value) {
                        if matched.is_some() {
                            ambiguous = true;
                            break;
                        }
                        matched = Some((index, candidate));
                    }
                }
                if !ambiguous {
                    if let Some((index, mut projected)) = matched {
                        projected.count_total = Some(total);
                        projected.count_epoch = self.epoch;
                        self.confirm_base(&projected);
                        self.entries[index] = projected;
                        self.index(index);
                        self.advance(at, total);
                        self.remember_count(at, total, Some(last), self.epoch);
                        return;
                    }
                }
            }
            for index in self.candidates(&value) {
                if self.entries[index].same_interval(&value) {
                    self.advance(at, total);
                    self.remember_count(at, total, Some(last), self.epoch);
                    return;
                }
            }
        }
        if previous == Some(total) {
            return;
        }
        self.advance(at, total);
        if let Some(previous) = previous {
            value.usage = total.less(previous);
        } else if last.is_none() {
            value.usage = total;
        }
        if value.usage.zero() || !value.usage.valid() {
            return;
        }
        let index = self.entries.len();
        self.entries.push(value);
        self.index(index);
        self.pending = Some(index);
        self.remember_count(at, total, last, self.epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const PAGINATED: &str = include_str!("../../tests/fixtures/codex-native/paginated.jsonl");
    const COMPACTION: &str = include_str!("../../tests/fixtures/codex-native/compaction.jsonl");
    const DOMAINS: &str = include_str!("../../tests/fixtures/codex-native/counter-domains.jsonl");

    fn parsed(lines: Vec<Value>) -> NativeSnapshot {
        parse(
            &lines
                .into_iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
            "fixture",
        )
        .unwrap()
    }
    fn line(second: u32, kind: &str, payload: Value) -> Value {
        json!({"timestamp":format!("2026-01-01T00:00:{second:02}.000Z"),"type":kind,"payload":payload})
    }
    fn usage(n: u64) -> Value {
        json!({"input_tokens":100*n,"output_tokens":10*n,"cached_input_tokens":20*n,"cache_write_input_tokens":0,"reasoning_output_tokens":2*n})
    }
    fn meta(thread: &str, billing: &str) -> Value {
        line(
            0,
            "session_meta",
            json!({"id":thread,"session_id":billing,"source":"cli","cwd":"/fixture"}),
        )
    }
    fn turn(second: u32, turn: &str) -> Value {
        line(
            second,
            "turn_context",
            json!({"turn_id":turn,"model":"gpt-6-astra"}),
        )
    }
    fn count(second: u32, total: u64, last: u64) -> Value {
        line(
            second,
            "event_msg",
            json!({"type":"token_count","info":{"total_token_usage":usage(total),"last_token_usage":usage(last)}}),
        )
    }
    fn record(second: u32, response: &str, total: u64, last: u64) -> Value {
        line(
            second,
            "token_usage_record",
            json!({"thread_id":"child","session_id":"billing","turn_id":"t","response_id":response,"usage":usage(last),"thread_token_usage":usage(total)}),
        )
    }
    fn sum(snapshot: &NativeSnapshot) -> [u64; 4] {
        snapshot.events.iter().fold([0; 4], |mut a, e| {
            for (x, y) in a.iter_mut().zip(e.buckets) {
                *x += y;
            }
            a
        })
    }

    #[test]
    fn codex_native_paginated_record_and_legacy_are_one_call() {
        let s = parse(PAGINATED, "page").unwrap();
        assert_eq!(s.events.len(), 3);
        assert_eq!(sum(&s), [128290, 293, 254720, 0]);
    }

    #[test]
    fn codex_native_compaction_embedded_and_record_only_are_preserved_once() {
        let s = parse(COMPACTION, "page").unwrap();
        assert_eq!(s.events.len(), 4);
        assert_eq!(sum(&s), [22857, 8001, 520320, 0]);
    }

    #[test]
    fn codex_native_counter_domains_do_not_cross_subtract_compaction() {
        let s = parse(DOMAINS, "page").unwrap();
        assert_eq!(s.events.len(), 4);
        assert_eq!(sum(&s), [11157, 6856, 496640, 0]);
    }

    #[test]
    fn codex_native_canonical_head_survives_copied_parent_header() {
        let mut head = meta("child", "billing");
        head["payload"]["forked_from_id"] = json!("parent");
        let s = parsed(vec![
            head,
            meta("parent", "billing"),
            turn(1, "t"),
            record(2, "own", 100, 1),
            count(3, 100, 1),
        ]);
        assert_eq!(s.native_thread_id.as_deref(), Some("child"));
        assert_eq!(s.billing_session_id.as_deref(), Some("billing"));
        assert_eq!(s.events.len(), 1);
        assert_eq!(sum(&s), [80, 10, 20, 0]);
    }

    #[test]
    fn codex_native_shared_billing_never_replaces_native_thread() {
        let s = parsed(vec![
            meta("child", "billing"),
            turn(1, "t"),
            record(2, "own", 1, 1),
        ]);
        assert_eq!(s.events[0].native_thread_id, "child");
        assert_eq!(s.events[0].billing_session_id.as_deref(), Some("billing"));
        assert!(s.main);
    }

    #[test]
    fn codex_native_response_conflict_excludes_only_conflicting_response() {
        let s = parsed(vec![
            meta("child", "billing"),
            turn(1, "t"),
            record(2, "conflict", 1, 1),
            record(2, "conflict", 2, 2),
            record(3, "valid", 3, 1),
        ]);
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].response_id.as_deref(), Some("valid"));
        assert!(s.reasons.contains("conflictingNativeUsage"));
    }

    #[test]
    fn codex_native_runtime_reset_does_not_replay_previous_high_water_mark() {
        let s = parsed(vec![
            meta("child", "billing"),
            turn(1, "t"),
            count(2, 100, 1),
            record(3, "first-runtime", 1, 1),
            count(4, 1, 1),
            record(5, "continues", 2, 1),
            count(6, 2, 1),
            record(7, "second-runtime", 3, 1),
            count(8, 1, 1),
            record(9, "after-resume", 4, 1),
            count(10, 2, 1),
        ]);
        assert_eq!(s.events.len(), 5);
        assert_eq!(sum(&s), [400, 50, 100, 0]);
        assert_eq!(
            s.events
                .iter()
                .filter(|event| event.response_id.is_some())
                .count(),
            4
        );
        assert_eq!(
            s.events
                .iter()
                .find(|event| event.response_id.as_deref() == Some("first-runtime"))
                .unwrap()
                .at_ms,
            Some(1767225603000)
        );
    }

    #[test]
    fn codex_native_all_event_timestamps_but_no_body_are_projected() {
        let secret = "fixture-body-never-retained";
        let s = parsed(vec![
            meta("child", "billing"),
            turn(1, "t"),
            line(
                2,
                "response_item",
                json!({"type":"message","role":"user","content":[{"type":"input_text","text":secret}]}),
            ),
            line(
                3,
                "response_item",
                json!({"type":"custom_tool_call_output","output":secret}),
            ),
            record(4, "own", 1, 1),
        ]);
        assert_eq!(s.activity.len(), 5);
        assert_eq!(s.messages.len(), 1);
        assert!(!serde_json::to_string(&s).unwrap().contains(secret));
    }

    #[test]
    fn codex_native_pairs_both_record_count_orders_and_replays() {
        for fixture in [PAGINATED, DOMAINS] {
            let original = parse(fixture, "original").unwrap();
            let mut lines: Vec<Value> = fixture
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let mut index = 0;
            while index + 1 < lines.len() {
                if lines[index]["type"] == "token_usage_record"
                    && lines[index + 1]["payload"]["type"] == "token_count"
                {
                    lines.swap(index, index + 1);
                    index += 1;
                }
                index += 1;
            }
            let replay: Vec<_> = lines
                .iter()
                .filter(|line| line["type"] == "token_usage_record")
                .cloned()
                .collect();
            lines.extend(replay);
            let reversed = parsed(lines);
            assert_eq!(reversed.events.len(), original.events.len());
            assert_eq!(sum(&reversed), sum(&original));
        }
    }

    #[test]
    fn codex_native_equal_usage_after_input_is_another_call() {
        for reverse in [false, true] {
            let mut pair = [record(2, "first", 1, 1), count(4, 2, 1)];
            if reverse {
                pair.reverse();
            }
            let s = parsed(vec![
                meta("child", "billing"),
                turn(1, "t"),
                pair[0].clone(),
                line(
                    3,
                    "event_msg",
                    json!({"type":"user_message","message":"new input"}),
                ),
                pair[1].clone(),
            ]);
            assert_eq!(s.events.len(), 2);
            assert_eq!(sum(&s), [160, 20, 40, 0]);
        }
    }

    #[test]
    fn codex_native_full_history_ordinal_excludes_parent_tokens_messages_and_activity() {
        let mut header = meta("child", "billing");
        header["payload"]["subagent_history_start_ordinal"] = json!(4);
        let user = |n| {
            line(
                n,
                "response_item",
                json!({"type":"message","role":"user","content":[{"type":"input_text","text":"fixture"}]}),
            )
        };
        let s = parsed(vec![
            header,
            user(1),
            turn(2, "old"),
            count(3, 90, 90),
            turn(4, "t"),
            user(5),
            record(6, "own", 91, 1),
            count(7, 91, 1),
        ]);
        assert_eq!(s.events.len(), 1);
        assert_eq!(sum(&s), [80, 10, 20, 0]);
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.messages[0].canonical_ordinal, 5);
        assert!(s
            .activity
            .iter()
            .all(|a| a.canonical_ordinal == 0 || a.canonical_ordinal >= 4));
    }

    #[test]
    fn codex_native_foreign_record_does_not_make_following_legacy_count_own() {
        let mut foreign = record(2, "foreign", 100, 100);
        foreign["payload"]["thread_id"] = json!("parent");
        let s = parsed(vec![
            meta("child", "billing"),
            turn(1, "t"),
            foreign,
            count(3, 100, 100),
            record(4, "own", 101, 1),
            count(5, 101, 1),
        ]);
        assert_eq!(s.events.len(), 1);
        assert_eq!(sum(&s), [80, 10, 20, 0]);
        assert!(s.reasons.contains("unprovenInheritedBoundary"));
    }

    #[test]
    fn codex_native_inherited_unknown_baseline_preserves_only_later_verified_delta() {
        let mut header = meta("child", "billing");
        header["payload"]["forked_from_id"] = json!("parent");
        let mut baseline = count(2, 100, 0);
        baseline["payload"]["info"]["last_token_usage"] = Value::Null;
        let s = parsed(vec![header, turn(1, "t"), baseline, count(3, 101, 1)]);
        assert_eq!(s.events.len(), 1);
        assert_eq!(sum(&s), [80, 10, 20, 0]);
        assert!(s.reasons.contains("unprovenInheritedBaseline"));
    }

    #[test]
    fn codex_native_late_response_uses_its_turn_model_without_changing_completion() {
        let mut other = turn(3, "other");
        other["payload"]["model"] = json!("gpt-6-luna");
        let s = parsed(vec![
            meta("child", "billing"),
            turn(1, "t"),
            record(2, "r", 1, 1),
            other,
            record(2, "r", 1, 1),
            record(5, "r", 1, 1),
        ]);
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].model, "gpt-6-astra");
        assert_eq!(s.events[0].at_ms, Some(1767225602000));
    }

    #[test]
    fn codex_native_zero_named_response_is_known_zero_and_missing_model_stays_unknown() {
        let s = parsed(vec![meta("child", "billing"), record(2, "zero", 0, 0)]);
        assert_eq!(s.events.len(), 1);
        assert_eq!(sum(&s), [0; 4]);
        assert_eq!(s.events[0].model, "unknown");
        assert!(s.reasons.contains("missingModel"));
    }

    #[test]
    fn codex_native_missing_cache_or_invalid_values_are_not_replaced_with_zero() {
        let mut missing = record(2, "missing", 1, 1);
        missing["payload"]["usage"]
            .as_object_mut()
            .unwrap()
            .remove("cached_input_tokens");
        let mut invalid = record(3, "invalid", 2, 1);
        invalid["payload"]["usage"]["input_tokens"] = json!(-1);
        let s = parsed(vec![
            meta("child", "billing"),
            turn(1, "t"),
            missing,
            invalid,
            record(4, "valid", 3, 1),
        ]);
        assert_eq!(s.events.len(), 1);
        assert!(s.reasons.contains("missingNativeCounters"));
        assert!(s.reasons.contains("invalidNativeUsage"));
    }

    #[test]
    fn codex_native_truncated_tail_and_bad_record_do_not_remove_valid_facts() {
        let source = format!(
            "{}\n{}\n{}\n{{invalid\n{}\n{{\"timestamp\":",
            meta("child", "billing"),
            turn(1, "t"),
            record(2, "a", 1, 1),
            record(3, "b", 2, 1)
        );
        let s = parse(&source, "fixture").unwrap();
        assert_eq!(s.events.len(), 2);
        assert_eq!(sum(&s), [160, 20, 40, 0]);
        assert!(!s.complete);
        assert!(s.reasons.contains("malformedNativeRecord"));
        assert_eq!(runtime_activity_ms(&s), 2000);
    }

    #[test]
    fn codex_native_zstd_read_is_identical_and_does_not_modify_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl.zst");
        let bytes = zstd::stream::encode_all(PAGINATED.as_bytes(), 1).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let s = read(&path).unwrap();
        assert_eq!(s, parse(PAGINATED, "uncompressed").unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn codex_native_paginated_restored_compaction_aligns_same_boundary_only() {
        for record_before in [false, true] {
            let mut header = meta("child", "billing");
            header["payload"]["history_mode"] = json!("paginated");
            header["payload"]["subagent_history_start_ordinal"] = json!(219);
            let custom_usage = |input, cache, output| json!({"input_tokens":input,"output_tokens":output,"cached_input_tokens":cache,"cache_write_input_tokens":0,"reasoning_output_tokens":0});
            let make_record = |second, response: &str, total: Value, usage: Value| {
                line(
                    second,
                    "token_usage_record",
                    json!({"thread_id":"child","session_id":"billing","turn_id":"t","response_id":response,"usage":usage,"thread_token_usage":total}),
                )
            };
            let compact = |second, response: &str, total: Value, usage: Value| {
                let record = make_record(second, response, total, usage);
                line(
                    second,
                    "compacted",
                    json!({"compaction_response_id":response,"latest_token_usage_record":record["payload"]}),
                )
            };
            let checkpoint = |second, total: Value, last: Value| {
                line(
                    second,
                    "event_msg",
                    json!({"type":"token_count","info":{"total_token_usage":total,"last_token_usage":last}}),
                )
            };
            let restored = make_record(
                0,
                "snapshot",
                custom_usage(1000, 500, 100),
                custom_usage(30, 10, 2),
            );
            let mut lines = vec![header];
            if record_before {
                lines.push(restored);
            }
            lines.extend([
                compact(
                    0,
                    "snapshot",
                    custom_usage(1000, 500, 100),
                    custom_usage(30, 10, 2),
                ),
                checkpoint(0, custom_usage(900, 450, 90), custom_usage(0, 0, 0)),
                turn(1, "t"),
                make_record(
                    2,
                    "a",
                    custom_usage(1100, 550, 110),
                    custom_usage(100, 50, 10),
                ),
                checkpoint(2, custom_usage(1000, 500, 100), custom_usage(100, 50, 10)),
                compact(
                    3,
                    "new-compaction",
                    custom_usage(1300, 580, 130),
                    custom_usage(200, 30, 20),
                ),
                make_record(
                    4,
                    "b",
                    custom_usage(1400, 630, 140),
                    custom_usage(100, 50, 10),
                ),
                checkpoint(4, custom_usage(1100, 550, 110), custom_usage(100, 50, 10)),
                checkpoint(5, custom_usage(1200, 600, 120), custom_usage(100, 50, 10)),
            ]);
            let s = parsed(lines);
            assert_eq!(s.events.len(), 5);
            assert_eq!(sum(&s), [340, 52, 190, 0]);
        }
    }

    #[test]
    fn codex_native_late_checkpoint_and_both_counter_reset_do_not_invent_call() {
        let s = parsed(vec![
            meta("child", "billing"),
            turn(0, "t"),
            record(1, "a", 1, 1),
            record(3, "b", 2, 1),
            count(2, 1, 1),
            record(5, "c", 1, 1),
            count(6, 1, 1),
        ]);
        assert_eq!(s.events.len(), 3);
        assert_eq!(sum(&s), [240, 30, 60, 0]);
    }

    #[test]
    fn codex_native_legacy_larger_first_request_after_reset_remains_distinct() {
        let mut first = count(1, 0, 0);
        let before = json!({"input_tokens":91814,"cached_input_tokens":46976,"cache_write_input_tokens":0,"output_tokens":396,"reasoning_output_tokens":123});
        let after = json!({"input_tokens":92225,"cached_input_tokens":91008,"cache_write_input_tokens":0,"output_tokens":885,"reasoning_output_tokens":516});
        first["payload"]["info"] = json!({"total_token_usage":before,"last_token_usage":before});
        let mut second = count(3, 0, 0);
        second["payload"]["info"] = json!({"total_token_usage":after,"last_token_usage":after});
        let mut repeated = second.clone();
        repeated["timestamp"] = json!("2026-01-01T00:00:04Z");
        let s = parsed(vec![
            meta("child", "billing"),
            turn(0, "t"),
            first,
            turn(2, "next"),
            second,
            repeated,
        ]);
        assert_eq!(s.events.len(), 2);
        assert_eq!(sum(&s), [46055, 1281, 137984, 0]);
    }

    #[test]
    fn codex_native_title_hint_uses_only_owned_user_text_and_stays_out_of_proof() {
        let mut header = meta("child", "billing");
        header["payload"]["subagent_history_start_ordinal"] = json!(3);
        let s = parsed(vec![
            header,
            line(
                0,
                "response_item",
                json!({"type":"message","role":"user","content":[{"type":"input_text","text":"inherited parent prompt"}]}),
            ),
            line(
                0,
                "response_item",
                json!({"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions: private harness"}]}),
            ),
            line(
                1,
                "response_item",
                json!({"type":"message","role":"user","content":[{"type":"input_text","text":"<user_query>Repair the owned feature</user_query>"}]}),
            ),
            line(
                2,
                "response_item",
                json!({"type":"message","role":"user","content":[{"type":"input_text","text":"later own prompt"}]}),
            ),
        ]);
        assert_eq!(s.title_hint.as_deref(), Some("Repair the owned feature"));
        let proof = serde_json::to_string(&s).unwrap();
        assert!(!proof.contains("Repair the owned feature"));
        assert!(!proof.contains("titleHint"));
        assert!(!proof.contains("inherited parent prompt"));
    }

    #[test]
    fn codex_native_title_hint_is_bounded_scrubbed_and_handles_runtime_user_event() {
        let text = format!("Repair {}", "long user content ".repeat(100));
        let s = parsed(vec![
            meta("child", "billing"),
            line(
                1,
                "event_msg",
                json!({"type":"user_message","message":"<environment_context>private harness</environment_context>"}),
            ),
            line(
                2,
                "event_msg",
                json!({"type":"user_message","message":text}),
            ),
        ]);
        let title = s
            .title_hint
            .as_deref()
            .expect("owned runtime user messages also supply title metadata");
        assert!(title.starts_with("Repair long user content"));
        assert!(title.chars().count() <= crate::session_manager::title::TITLE_MAX_CHARS + 1);
        assert!(!serde_json::to_string(&s)
            .unwrap()
            .contains("long user content"));
        let secret = format!("Inspect token {}", ["sk-", &"A".repeat(48)].concat());
        let s = parsed(vec![
            meta("child", "billing"),
            line(
                1,
                "event_msg",
                json!({"type":"user_message","message":secret}),
            ),
        ]);
        assert_eq!(
            s.title_hint.as_deref(),
            Some(crate::redact::scrub(&secret).as_ref())
        );
        assert!(!s.title_hint.unwrap().contains(&"A".repeat(48)));
    }

    #[test]
    fn codex_native_real_xml_user_prompts_count_but_known_harness_context_does_not() {
        let prompts = [
            "# AGENTS.md instructions for a synthetic fixture",
            "<environment_context>synthetic harness only</environment_context>",
            "<user_query>Repair the real owned feature</user_query>",
            "<article>Review this real XML document</article>",
            "A plain user question",
        ];
        let mut rows = vec![meta("child", "billing")];
        rows.extend(prompts.into_iter().enumerate().map(|(index,text)|line((index+1) as u32,"response_item",json!({"type":"message","role":"user","content":[{"type":"input_text","text":text}]}))));
        let snapshot = parsed(rows);
        assert_eq!(
            snapshot
                .messages
                .iter()
                .filter(|message| message.role == "user")
                .count(),
            3,
            "XML is legitimate user text; only recognized harness context is excluded"
        );
        assert_eq!(
            snapshot.title_hint.as_deref(),
            Some("Repair the real owned feature")
        );
    }

    #[test]
    fn codex_native_reader_streams_sources_larger_than_its_read_buffer_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large-by-injected-budget.jsonl");
        let mut rows = vec![meta("child", "billing"), turn(0, "t")];
        rows.extend((0..128).map(|_| {
            line(
                1,
                "response_item",
                json!({"type":"function_call_output","output":"x".repeat(1024)}),
            )
        }));
        rows.push(record(2, "own-response", 1, 1));
        let body = rows
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, &body).unwrap();
        assert!(body.len() > 4096 * 16);
        let streamed = read_with_buffer_budget(&path, 4096)
            .expect("whole source size is not a read-buffer memory limit");
        assert_eq!(streamed.events.len(), 1);
        assert_eq!(sum(&streamed), [80, 10, 20, 0]);
        assert_eq!(streamed.source_digest, digest(body.as_bytes()));
        assert_eq!(streamed.record_hashes.len(), rows.len());
    }

    #[test]
    fn codex_native_legacy_only_tail_after_paired_runtime_reset_is_not_inflated() {
        let s = parsed(vec![
            meta("child", "billing"),
            turn(0, "t"),
            record(1, "a", 1, 1),
            count(2, 1, 1),
            record(3, "b", 2, 1),
            count(4, 2, 1),
            record(5, "c", 3, 1),
            count(6, 1, 1),
            record(7, "d", 4, 1),
            count(8, 2, 1),
            count(9, 3, 1),
        ]);
        assert_eq!(s.events.len(), 5);
        assert_eq!(sum(&s), [400, 50, 100, 0]);
        assert_eq!(
            s.events.iter().filter(|e| e.response_id.is_some()).count(),
            4
        );
    }

    #[test]
    fn codex_native_both_counter_reset_and_old_compaction_replay_keep_new_domain() {
        let compacted = line(
            3,
            "compacted",
            json!({"compaction_response_id":"old-compaction","latest_token_usage_record":record(3,"old-compaction",3,1)["payload"]}),
        );
        let s = parsed(vec![
            meta("child", "billing"),
            turn(0, "t"),
            record(1, "a", 1, 1),
            count(2, 1, 1),
            compacted.clone(),
            record(4, "b", 4, 1),
            count(5, 2, 1),
            record(6, "new-runtime", 1, 1),
            count(7, 1, 1),
            compacted,
            record(8, "d", 2, 1),
            count(9, 2, 1),
            count(10, 3, 1),
        ]);
        assert_eq!(s.events.len(), 6);
        assert_eq!(sum(&s), [480, 60, 120, 0]);
    }

    #[test]
    fn codex_native_unknown_old_fork_boundary_is_not_guessed_from_tokens() {
        let mut header = meta("child", "billing");
        header["payload"]["forked_from_id"] = json!("parent");
        let s = parsed(vec![
            header,
            meta("parent", "billing"),
            count(1, 1000, 1000),
        ]);
        assert!(s.events.is_empty());
        assert!(s.reasons.contains("unprovenInheritedBoundary"));
    }

    #[test]
    fn codex_native_same_timestamp_owned_turn_model_survives_inherited_filter() {
        let mut header = meta("child", "billing");
        header["payload"]["parent_thread_id"] = json!("parent");
        let s = parsed(vec![header, turn(0, "t"), record(1, "own", 10, 1)]);
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].model, "gpt-6-astra");
    }

    #[test]
    fn codex_native_first_metadata_stays_canonical_even_when_invalid() {
        let s = parsed(vec![
            line(0, "session_meta", json!({"session_id":"billing"})),
            meta("copied", "billing"),
            turn(1, "t"),
            count(2, 1, 1),
        ]);
        assert!(s.native_thread_id.is_none());
        assert!(s.events.is_empty());
        assert!(s.reasons.contains("missingNativeSessionIdentity"));
    }

    #[test]
    fn codex_native_review_duplicate_compaction_revalidates_usage_turn_and_total() {
        for changed in ["usage", "turn", "total"] {
            let compacted = |payload: Value| {
                line(
                    2,
                    "compacted",
                    json!({"compaction_response_id":"compaction","latest_token_usage_record":payload}),
                )
            };
            let first = record(2, "compaction", 1, 1)["payload"].clone();
            let mut conflicting = first.clone();
            match changed {
                "usage" => conflicting["usage"] = usage(2),
                "turn" => conflicting["turn_id"] = json!("different-turn"),
                "total" => conflicting["thread_token_usage"] = usage(2),
                _ => unreachable!(),
            }
            let s = parsed(vec![
                meta("child", "billing"),
                turn(0, "t"),
                compacted(first),
                compacted(conflicting),
                record(3, "sibling", 3, 1),
            ]);
            assert_eq!(s.events.len(), 1, "{changed}");
            assert_eq!(
                s.events[0].response_id.as_deref(),
                Some("sibling"),
                "{changed}"
            );
            assert!(s.reasons.contains("conflictingNativeUsage"), "{changed}");
        }
    }

    #[test]
    fn codex_native_review_explicit_unknown_old_turn_never_uses_current_model() {
        let mut current = turn(1, "current");
        current["payload"]["model"] = json!("gpt-6-luna");
        let mut delayed = record(2, "old-response", 1, 1);
        delayed["payload"]["turn_id"] = json!("unknown-old-turn");
        let s = parsed(vec![meta("child", "billing"), current, delayed]);
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].model, "unknown");
        assert!(s.reasons.contains("missingModel"));
    }

    #[test]
    fn codex_native_review_paginated_base_assigns_logical_ordinals_for_ownership() {
        let mut header = meta("child", "billing");
        header["payload"]["history_mode"] = json!("paginated");
        header["payload"]["history_base"] =
            json!({"thread_id":"parent","end_ordinal_exclusive":1000,"end_byte_offset":100});
        header["payload"]["subagent_history_start_ordinal"] = json!(1002);
        let message = |id| {
            line(
                0,
                "response_item",
                json!({"type":"message","id":id,"role":"user","content":[{"type":"input_text","text":"fixture"}]}),
            )
        };
        let s = parsed(vec![header, message("inherited"), message("own")]);
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.messages[0].canonical_ordinal, 1002);
        assert!(s
            .activity
            .iter()
            .all(|event| event.canonical_ordinal == 1000 || event.canonical_ordinal >= 1002));
    }

    #[test]
    fn codex_native_review_missing_runtime_or_message_time_is_partial_not_zero_activity() {
        for kind in ["tool", "message"] {
            let mut unknown = if kind == "tool" {
                line(
                    2,
                    "response_item",
                    json!({"type":"function_call","name":"fixture"}),
                )
            } else {
                line(
                    2,
                    "response_item",
                    json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"fixture"}]}),
                )
            };
            unknown.as_object_mut().unwrap().remove("timestamp");
            let s = parsed(vec![
                meta("child", "billing"),
                turn(1, "t"),
                unknown,
                record(3, "valid", 1, 1),
            ]);
            assert_eq!(sum(&s), [80, 10, 20, 0]);
            assert!(s.reasons.contains("activityTimeUnavailable"), "{kind}");
            assert!(!s.complete, "{kind}");
        }
    }

    #[test]
    fn codex_native_review_corrected_response_checkpoint_does_not_become_second_call() {
        let s = parsed(vec![
            meta("child", "billing"),
            turn(0, "t"),
            record(1, "corrected", 1, 1),
            count(2, 1, 1),
            record(3, "corrected", 2, 2),
            count(4, 2, 2),
            record(5, "sibling", 3, 1),
            count(6, 3, 1),
        ]);
        assert_eq!(s.events.len(), 2);
        assert_eq!(sum(&s), [240, 30, 60, 0]);
        let corrected = s
            .events
            .iter()
            .find(|event| event.response_id.as_deref() == Some("corrected"))
            .unwrap();
        assert_eq!(corrected.at_ms, Some(1767225601000));
        assert_eq!(corrected.buckets, [160, 20, 40, 0]);
    }

    #[test]
    fn codex_native_same_pass_row_hashes_prove_real_prefix_not_equal_token_totals() {
        let base = vec![
            meta("child", "billing"),
            turn(0, "t"),
            line(
                1,
                "response_item",
                json!({"type":"function_call_output","output":"private body A"}),
            ),
            count(2, 1, 1),
        ];
        let a = parsed(base.clone());
        let mut extended = base.clone();
        extended.push(count(3, 2, 1));
        let b = parsed(extended);
        let mut different = base;
        different[2]["payload"]["output"] = json!("private body B");
        let c = parsed(different);
        assert_eq!(a.record_hashes.len(), 4);
        assert!(b.record_hashes.starts_with(&a.record_hashes));
        assert_eq!(sum(&a), sum(&c));
        assert_ne!(a.record_hashes, c.record_hashes);
        assert!(a
            .record_hashes
            .iter()
            .all(|hash| hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())));
        assert!(!serde_json::to_string(&a).unwrap().contains("private body"));
    }

    #[test]
    fn codex_native_paginated_explicit_ordinal_owns_same_timestamp_messages() {
        let mut header = meta("child", "billing");
        header["payload"]["history_mode"] = json!("paginated");
        header["payload"]["history_base"] =
            json!({"thread_id":"parent","end_ordinal_exclusive":1000,"end_byte_offset":100});
        header["payload"]["subagent_history_start_ordinal"] = json!(1002);
        let mut rows = vec![
            header,
            line(
                0,
                "response_item",
                json!({"type":"message","id":"parent-message","role":"assistant"}),
            ),
            line(
                0,
                "response_item",
                json!({"type":"message","id":"own-message","role":"assistant"}),
            ),
        ];
        for (row, ordinal) in rows.iter_mut().zip([1000, 1001, 1002]) {
            row["ordinal"] = json!(ordinal);
        }
        let s = parsed(rows);
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.messages[0].canonical_ordinal, 1002);
        assert!(s
            .activity
            .iter()
            .all(|event| event.canonical_ordinal == 1000 || event.canonical_ordinal >= 1002));
    }

    #[test]
    fn codex_native_paginated_ordinals_skip_duplicates_and_report_missing_records() {
        let mut header = meta("child", "billing");
        header["payload"]["history_mode"] = json!("paginated");
        header["payload"]["history_base"] =
            json!({"thread_id":"child","end_ordinal_exclusive":10,"end_byte_offset":100});
        let mut rows = vec![
            header,
            turn(0, "t"),
            record(1, "a", 1, 1),
            record(2, "duplicate-ordinal", 2, 1),
            record(3, "valid-after-gap", 3, 1),
        ];
        for (row, ordinal) in rows.iter_mut().zip([10, 11, 12, 12, 14]) {
            row["ordinal"] = json!(ordinal);
        }
        let s = parsed(rows);
        assert_eq!(s.events.len(), 2);
        assert_eq!(sum(&s), [160, 20, 40, 0]);
        assert!(s.reasons.contains("nativeOrdinalDiscontinuity"));
        assert_eq!(s.record_hashes.len(), 5);
    }

    fn runtime_activity_ms(snapshot: &NativeSnapshot) -> u64 {
        use super::super::session_analytics_core::{DayBoundary, Window};
        use super::super::session_native_activity::{activity, NativeActivityEvent};
        let events = snapshot
            .activity
            .iter()
            .map(|event| NativeActivityEvent {
                session_key: "fixture-session".into(),
                event_key: event.key.clone(),
                at_ms: event.at_ms,
                main: true,
                stream_key: format!("fixture:{}", event.stream_segment),
            })
            .collect::<Vec<_>>();
        activity(
            &events,
            &Window {
                start_ms: Some(1767225600000),
                end_ms: 1767312000000,
            },
            &[DayBoundary {
                date: "2026-01-01".into(),
                start_ms: 1767225600000,
                end_ms: 1767312000000,
            }],
        )
        .unwrap()
        .active_ms
    }

    #[test]
    fn codex_native_review_activity_never_bridges_unknown_time_ordinal_or_ownership() {
        for unknown_kind in ["missingTime", "badOrdinal", "unknownBoundary"] {
            let mut unknown = line(
                2,
                "response_item",
                json!({"type":"function_call","name":"fixture"}),
            );
            if unknown_kind == "missingTime" {
                unknown.as_object_mut().unwrap().remove("timestamp");
            }
            if unknown_kind == "unknownBoundary" {
                unknown = record(2, "foreign", 100, 100);
                unknown["payload"]["thread_id"] = json!("foreign-thread");
            }
            let mut rows = vec![
                meta("child", "billing"),
                turn(0, "t"),
                record(1, "before", 1, 1),
                unknown,
                record(4, "after", 2, 1),
            ];
            if unknown_kind == "badOrdinal" {
                for (index, row) in rows.iter_mut().enumerate() {
                    row["ordinal"] = json!(index);
                }
                rows[3]["ordinal"] = json!("invalid");
            }
            let s = parsed(rows);
            assert_eq!(sum(&s), [160, 20, 40, 0], "{unknown_kind}");
            assert_eq!(runtime_activity_ms(&s), 1000, "{unknown_kind}");
        }
    }

    #[test]
    fn codex_native_activity_clock_rollback_keeps_same_stream_high_water_mark() {
        for before_creation in [false, true] {
            let mut rollback = line(
                1,
                "response_item",
                json!({"type":"function_call","name":"fixture"}),
            );
            if before_creation {
                rollback["timestamp"] = json!("2025-12-31T23:59:59Z");
            }
            let s = parsed(vec![
                meta("child", "billing"),
                turn(0, "t"),
                record(3, "before", 1, 1),
                rollback,
                record(4, "after", 2, 1),
            ]);
            assert_eq!(s.activity.len(), 5);
            assert_eq!(runtime_activity_ms(&s), 4000);
            assert!(s.activity.iter().all(|event| event.stream_segment == 0));
        }
    }

    #[test]
    fn codex_native_missing_cache_write_is_partial_while_explicit_zero_is_known() {
        for missing_write in [true, false] {
            let mut event = record(2, "usage", 1, 1);
            if missing_write {
                event["payload"]["usage"]
                    .as_object_mut()
                    .unwrap()
                    .remove("cache_write_input_tokens");
            }
            let s = parsed(vec![meta("child", "billing"), turn(1, "t"), event]);
            assert_eq!(s.events.len(), 1);
            assert_eq!(s.events[0].buckets, [80, 10, 20, 0]);
            assert_eq!(s.events[0].complete, !missing_write);
            assert_eq!(s.reasons.contains("missingNativeCounters"), missing_write);
            assert_eq!(s.complete, !missing_write);
        }
        let zero = parsed(vec![
            meta("child", "billing"),
            turn(1, "t"),
            record(2, "zero", 0, 0),
        ]);
        assert_eq!(zero.events[0].buckets, [0; 4]);
        assert!(zero.events[0].complete);
        assert!(zero.complete);
    }

    #[test]
    fn codex_native_missing_cache_read_cannot_prove_fresh_input_or_become_zero_cache() {
        for modern in [false, true] {
            let partial = json!({"input_tokens":5,"output_tokens":2});
            let row = if modern {
                line(
                    2,
                    "token_usage_record",
                    json!({"thread_id":"child","session_id":"billing","turn_id":"t","response_id":"partial-response","usage":partial,"thread_token_usage":partial}),
                )
            } else {
                line(
                    2,
                    "event_msg",
                    json!({"type":"token_count","info":{"total_token_usage":partial,"last_token_usage":partial}}),
                )
            };
            let snapshot = parsed(vec![meta("child", "billing"), turn(1, "t"), row]);
            assert!(
                snapshot.events.is_empty(),
                "known output does not make an unknown fresh-input bucket a real zero"
            );
            assert!(snapshot.reasons.contains("missingNativeCounters"));
            assert!(
                snapshot.reasons.contains("freshInputUnavailable"),
                "coverage must explain why these primary Tokens cannot be established"
            );
            let zero = json!({"input_tokens":5,"output_tokens":2,"cached_input_tokens":0,"cache_write_input_tokens":0});
            let known = parsed(vec![
                meta("child", "billing"),
                turn(1, "t"),
                line(
                    2,
                    "token_usage_record",
                    json!({"thread_id":"child","session_id":"billing","turn_id":"t","response_id":"known-response","usage":zero,"thread_token_usage":zero}),
                ),
            ]);
            assert_eq!(sum(&known), [5, 2, 0, 0]);
            assert!(known.events[0].complete);
            assert!(!known.reasons.contains("freshInputUnavailable"));
        }
    }

    #[test]
    fn codex_native_legacy_interval_hint_is_stable_across_conflicting_values_only() {
        let one = parsed(vec![meta("child", "billing"), turn(1, "t"), count(2, 1, 1)]);
        let two = parsed(vec![meta("child", "billing"), turn(1, "t"), count(2, 2, 2)]);
        assert_eq!(one.events.len(), 1);
        assert_eq!(two.events.len(), 1);
        assert_ne!(one.events[0].buckets, two.events[0].buckets);
        assert_ne!(
            one.events[0].key, two.events[0].key,
            "the existing event key remains value-sensitive"
        );
        let hint = one.events[0]
            .interval_identity
            .as_deref()
            .expect("a legacy interval has an independent conflict hint");
        assert_eq!(
            Some(hint),
            two.events[0].interval_identity.as_deref(),
            "same native interval must reveal a contradiction rather than two calls"
        );
        let separate = parsed(vec![
            meta("child", "billing"),
            turn(1, "t"),
            count(2, 1, 1),
            turn(3, "next"),
            count(4, 2, 1),
        ]);
        assert_eq!(separate.events.len(), 2);
        assert_eq!(separate.events[0].buckets, separate.events[1].buckets);
        assert_ne!(
            separate.events[0].interval_identity, separate.events[1].interval_identity,
            "equal usage in independently observed intervals is not identity"
        );
        let strong = parsed(vec![
            meta("child", "billing"),
            turn(1, "t"),
            record(2, "native-response", 1, 1),
        ]);
        assert!(
            strong.events[0].interval_identity.is_none(),
            "native response identity needs no legacy fallback hint"
        );
    }

    fn late_revision_carry(count_first: bool) -> NativeSnapshot {
        let mut rows = vec![
            meta("child", "billing"),
            turn(0, "t"),
            record(1, "r", 1, 1),
            count(1, 1, 1),
            record(3, "s", 2, 1),
            count(3, 2, 1),
        ];
        let revised_record = record(2, "r", 3, 3);
        let revised_count = count(2, 3, 3);
        if count_first {
            rows.extend([revised_count, revised_record]);
        } else {
            rows.extend([revised_record, revised_count]);
        }
        rows.push(count(5, 5, 1));
        parsed(rows)
    }

    fn assert_late_revision_carry(snapshot: NativeSnapshot) {
        assert_eq!(
            sum(&snapshot),
            [400, 50, 100, 0],
            "r=3u, s=1u, and the legacy-only tail=1u must sum to 5u"
        );
        assert_eq!(
            snapshot.events.len(),
            3,
            "the correction checkpoint is not another call"
        );
        let revised = snapshot
            .events
            .iter()
            .find(|event| event.response_id.as_deref() == Some("r"))
            .unwrap();
        assert_eq!(revised.buckets, [240, 30, 60, 0]);
        assert_eq!(
            revised.at_ms,
            Some(1767225601000),
            "late revisions preserve the original response completion time"
        );
        let sibling = snapshot
            .events
            .iter()
            .find(|event| event.response_id.as_deref() == Some("s"))
            .unwrap();
        assert_eq!(sibling.buckets, [80, 10, 20, 0]);
        assert_eq!(sibling.at_ms, Some(1767225603000));
        let tail = snapshot
            .events
            .iter()
            .find(|event| event.response_id.is_none())
            .unwrap();
        assert_eq!(tail.buckets, [80, 10, 20, 0]);
        assert_eq!(tail.at_ms, Some(1767225605000));
    }

    #[test]
    fn codex_native_late_revision_carry_record_before_count_keeps_observed_sibling() {
        assert_late_revision_carry(late_revision_carry(false));
    }

    #[test]
    fn codex_native_late_revision_carry_count_before_record_keeps_observed_sibling() {
        assert_late_revision_carry(late_revision_carry(true));
    }

    #[test]
    fn codex_native_late_revision_carry_is_not_reapplied_by_later_sibling_refresh_or_raw_replay() {
        let snapshot = parsed(vec![
            meta("child", "billing"),
            turn(0, "t"),
            record(1, "r", 1, 1),
            count(1, 1, 1),
            record(3, "s", 2, 1),
            count(3, 2, 1),
            record(2, "r", 3, 3),
            count(2, 3, 3),
            record(4, "r", 3, 3),
            count(4, 3, 3),
            record(5, "s", 4, 1),
            count(5, 4, 1),
            count(1, 1, 1),
            count(3, 2, 1),
            count(6, 5, 1),
        ]);
        assert_eq!(sum(&snapshot), [400, 50, 100, 0]);
        assert_eq!(snapshot.events.len(), 3);
        assert_eq!(
            snapshot
                .events
                .iter()
                .find(|event| event.response_id.as_deref() == Some("r"))
                .unwrap()
                .at_ms,
            Some(1767225601000)
        );
        assert_eq!(
            snapshot
                .events
                .iter()
                .find(|event| event.response_id.as_deref() == Some("s"))
                .unwrap()
                .at_ms,
            Some(1767225603000)
        );
    }

    #[test]
    fn codex_native_late_revision_carry_does_not_borrow_the_current_turn() {
        for count_first in [false, true] {
            let own_record = |second, response, total, last, turn_id| {
                let mut value = record(second, response, total, last);
                value["payload"]["turn_id"] = json!(turn_id);
                value
            };
            let mut second_turn = turn(2, "second");
            second_turn["payload"]["model"] = json!("gpt-6-luna");
            let mut rows = vec![
                meta("child", "billing"),
                turn(0, "first"),
                own_record(1, "r", 1, 1, "first"),
                count(1, 1, 1),
                second_turn,
                own_record(3, "s", 2, 1, "second"),
                count(3, 2, 1),
            ];
            let revised = own_record(2, "r", 3, 3, "first");
            if count_first {
                rows.extend([count(2, 3, 3), revised]);
            } else {
                rows.extend([revised, count(2, 3, 3)]);
            }
            rows.push(count(5, 5, 1));
            let snapshot = parsed(rows);
            assert_eq!(
                sum(&snapshot),
                [400, 50, 100, 0],
                "count_first={count_first}"
            );
            assert_eq!(
                snapshot
                    .events
                    .iter()
                    .find(|event| event.response_id.as_deref() == Some("r"))
                    .unwrap()
                    .model,
                "gpt-6-astra"
            );
            assert_eq!(
                snapshot
                    .events
                    .iter()
                    .find(|event| event.response_id.as_deref() == Some("s"))
                    .unwrap()
                    .model,
                "gpt-6-luna"
            );
        }
    }

    #[test]
    fn codex_native_unpaired_revision_keeps_proven_responses_and_excludes_uncertain_legacy_tail() {
        let snapshot = parsed(vec![
            meta("child", "billing"),
            turn(0, "t"),
            record(1, "r", 1, 1),
            count(1, 1, 1),
            record(3, "s", 2, 1),
            count(3, 2, 1),
            record(2, "r", 3, 3),
            count(5, 5, 1),
        ]);
        assert_eq!(
            sum(&snapshot),
            [320, 40, 80, 0],
            "only the unproved tail is omitted; strong r=3u and s=1u remain"
        );
        assert_eq!(snapshot.events.len(), 2);
        assert!(snapshot.reasons.contains("unprovenCounterRevision"));
        assert!(!snapshot.complete);
    }

    #[test]
    fn codex_native_proven_runtime_reset_scopes_uncertain_counters_to_the_old_epoch() {
        for deferred_only in [false, true] {
            let mut rows = vec![
                meta("child", "billing"),
                turn(0, "t"),
                record(1, "r", 1, 1),
                count(1, 1, 1),
                record(3, "s", 2, 1),
                count(3, 2, 1),
            ];
            rows.push(if deferred_only {
                count(2, 3, 3)
            } else {
                record(2, "r", 3, 3)
            });
            rows.extend([
                count(5, 5, 1),
                record(6, "new-runtime", 1, 1),
                count(6, 1, 1),
                count(7, 2, 1),
            ]);
            let snapshot = parsed(rows);
            let expected_units = if deferred_only { 4 } else { 6 };
            assert_eq!(
                sum(&snapshot),
                [80 * expected_units, 10 * expected_units, 20 * expected_units, 0],
                "deferred_only={deferred_only}: a proven fresh counter domain must retain its own legacy tail"
            );
            let tail = snapshot
                .events
                .iter()
                .find(|event| event.at_ms == Some(1767225607000))
                .expect("the new epoch's 1u tail has a verified baseline");
            assert_eq!(tail.buckets, [80, 10, 20, 0]);
            assert_eq!(tail.counter_epoch, 1);
            assert!(snapshot
                .events
                .iter()
                .all(|event| event.at_ms != Some(1767225605000)));
            assert!(snapshot.reasons.contains("unprovenCounterRevision"));
            assert!(!snapshot.complete);
        }
    }

    fn assert_raw_counter_replay_across_context(turn_changed: bool, epoch_changed: bool) {
        let own_record = |second, response, total, last, turn_id| {
            let mut value = record(second, response, total, last);
            value["payload"]["turn_id"] = json!(turn_id);
            value
        };
        let current_turn = if turn_changed { "second" } else { "first" };
        let mut rows = vec![
            meta("child", "billing"),
            turn(0, "first"),
            own_record(1, "r", 1, 1, "first"),
            count(1, 1, 1),
        ];
        if turn_changed {
            rows.push(turn(2, current_turn));
        }
        rows.extend([own_record(3, "s", 2, 1, current_turn), count(3, 2, 1)]);
        if epoch_changed {
            rows.extend([
                own_record(6, "new-runtime", 1, 1, current_turn),
                count(6, 1, 1),
            ]);
        }
        rows.extend([
            count(1, 1, 1),
            count(8, if epoch_changed { 2 } else { 3 }, 1),
        ]);
        let snapshot = parsed(rows);
        let expected_units = if epoch_changed { 4 } else { 3 };
        assert_eq!(
            sum(&snapshot),
            [80 * expected_units, 10 * expected_units, 20 * expected_units, 0],
            "turn_changed={turn_changed}, epoch_changed={epoch_changed}: replay identity cannot borrow its new surrounding context"
        );
        assert!(!snapshot.reasons.contains("unprovenCounterRevision"));
        let tail = snapshot
            .events
            .iter()
            .find(|event| event.at_ms == Some(1767225608000))
            .expect("the replay must not block the current epoch's tail");
        assert_eq!(tail.buckets, [80, 10, 20, 0]);
        assert_eq!(tail.counter_epoch, u32::from(epoch_changed));
    }

    #[test]
    fn codex_native_raw_counter_replay_keeps_its_original_turn_context() {
        assert_raw_counter_replay_across_context(true, false);
    }

    #[test]
    fn codex_native_raw_counter_replay_keeps_its_original_runtime_epoch() {
        assert_raw_counter_replay_across_context(false, true);
    }

    #[test]
    fn codex_native_raw_counter_replay_keeps_its_original_turn_and_epoch() {
        assert_raw_counter_replay_across_context(true, true);
    }

    #[test]
    fn codex_native_cross_turn_corrected_response_refresh_does_not_start_another_runtime() {
        let own_record = |second, response, total, last, turn_id| {
            let mut value = record(second, response, total, last);
            value["payload"]["turn_id"] = json!(turn_id);
            value
        };
        let snapshot = parsed(vec![
            meta("child", "billing"),
            turn(0, "first"),
            own_record(1, "r", 1, 1, "first"),
            count(1, 1, 1),
            turn(2, "second"),
            own_record(3, "s", 2, 1, "second"),
            count(3, 2, 1),
            own_record(2, "r", 3, 3, "first"),
            count(2, 3, 3),
            own_record(4, "r", 3, 3, "first"),
            count(4, 3, 3),
            count(5, 5, 1),
        ]);
        assert_eq!(sum(&snapshot), [400, 50, 100, 0]);
        assert_eq!(snapshot.events.len(), 3);
        assert!(snapshot.events.iter().all(|event| event.counter_epoch == 0));
        assert_eq!(
            snapshot
                .events
                .iter()
                .find(|event| event.response_id.as_deref() == Some("r"))
                .unwrap()
                .at_ms,
            Some(1767225601000)
        );
        assert!(!snapshot.reasons.contains("unprovenCounterRevision"));
    }

    #[test]
    fn codex_native_invalid_first_header_ordinal_never_promotes_a_copied_parent() {
        let mut canonical = meta("child", "billing");
        canonical["ordinal"] = json!("bad");
        let mut copied = meta("parent", "billing");
        copied["ordinal"] = json!(1);
        copied["payload"]["cwd"] = json!("/parent-project");
        let mut parent_record = record(2, "parent-response", 1, 1);
        parent_record["ordinal"] = json!(2);
        parent_record["payload"]["thread_id"] = json!("parent");
        let mut own = record(3, "child-response", 1, 1);
        own["ordinal"] = json!(3);
        let snapshot = parsed(vec![canonical, copied, parent_record, own]);
        assert_eq!(snapshot.native_thread_id.as_deref(),Some("child"),"ordinal validation cannot replace the canonical parsed identity with inherited metadata");
        assert_eq!(snapshot.project_dir.as_deref(), Some("/fixture"));
        assert!(snapshot.reasons.contains("invalidNativeOrdinal"));
        assert!(!snapshot.complete);
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].native_thread_id, "child");
        assert_eq!(
            snapshot.events[0].response_id.as_deref(),
            Some("child-response")
        );
    }

    #[test]
    fn codex_native_legacy_subagent_without_fork_or_history_boundary_keeps_own_usage() {
        for prebirth_copy in [false, true] {
            let mut header = meta("child", "root");
            header["payload"]["source"] =
                json!({"subagent":{"thread_spawn":{"parent_thread_id":"root"}}});
            let count = line(
                1,
                "event_msg",
                json!({"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":20},"last_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":20}}}),
            );
            let mut rows = vec![header];
            if prebirth_copy {
                let mut inherited = count.clone();
                inherited["timestamp"] = json!("2025-12-31T23:59:59Z");
                rows.push(inherited);
            }
            rows.push(json!({"type":"turn_context","payload":{"model":"gpt-fixture"}}));
            rows.push(count);
            let s = parsed(rows);
            assert_eq!(s.native_thread_id.as_deref(), Some("child"));
            assert_eq!(s.billing_session_id.as_deref(), Some("root"));
            assert_eq!(s.parent_thread_id.as_deref(), Some("root"));
            assert_eq!(s.events.len(), 1, "prebirth={prebirth_copy}");
            assert_eq!(s.events[0].buckets, [40, 20, 60, 0]);
            assert_eq!(s.events[0].at_ms, Some(1767225601000));
            assert_eq!(s.events[0].model, "gpt-fixture");
            assert!(s.reasons.contains("activityTimeUnavailable"));
            assert!(!s.reasons.contains("unprovenInheritedBoundary"));
        }
    }

    #[test]
    fn codex_native_legacy_subagent_without_last_keeps_unknown_initial_baseline() {
        let mut header = meta("child", "root");
        header["payload"]["source"] =
            json!({"subagent":{"thread_spawn":{"parent_thread_id":"root"}}});
        let mut baseline = count(2, 100, 1);
        baseline["payload"]["info"]["last_token_usage"] = Value::Null;
        let s = parsed(vec![header, turn(1, "t"), baseline, count(3, 101, 1)]);
        assert_eq!(s.events.len(), 1);
        assert_eq!(sum(&s), [80, 10, 20, 0]);
        assert!(s.reasons.contains("unprovenInheritedBaseline"));
    }
}
