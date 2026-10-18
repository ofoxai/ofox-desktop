//! Read-only WorkBuddy 5.7.7 native JSONL facts. No route/config inference.
use crate::{
    error::AppError,
    proxy::usage::parser::TokenUsage,
    services::usage_source::{self, NormalizedUsageFact},
    session_manager::{
        title::{self, TitleCandidates},
        SessionMeta,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SOURCE_FILES: usize = 10_000;
const MAX_SCAN_BYTES: u64 = 128 * 1024 * 1024;
const MAX_NATIVE_EVENTS: usize = 100_000;

fn invalid(code: &str) -> AppError {
    AppError::InvalidInput(format!("usage.workbuddy.{code}"))
}

pub(crate) struct WorkBuddyRead {
    pub facts: Vec<NormalizedUsageFact>,
    pub line_count: i64,
    pub reasons: BTreeSet<String>,
    pub session: Option<SessionMeta>,
    pub analytics: super::session_analytics_messages::TranscriptFacts,
    pub analytics_message_hashes: BTreeMap<String, String>,
    pub native_activity: Vec<super::session_native_activity::NativeActivityEvent>,
}

pub(crate) fn native_time(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    let ms = if let Some(ms) = value.as_i64() {
        ms
    } else {
        chrono::DateTime::parse_from_rfc3339(value.as_str()?)
            .ok()?
            .timestamp_millis()
    };
    chrono::DateTime::from_timestamp_millis(ms).map(|_| ms)
}
fn identifier(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= 512)
        .map(str::to_owned)
}
fn integer(value: Option<&Value>) -> Result<u64, AppError> {
    match value {
        None | Some(Value::Null) => Ok(0),
        Some(v) => v.as_u64().ok_or_else(|| invalid("invalidTokenInteger")),
    }
}
fn detail_total(value: Option<&Value>, key: &str) -> Result<u64, AppError> {
    match value {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Object(map)) => integer(map.get(key)),
        Some(Value::Array(list)) => list.iter().try_fold(0_u64, |sum, item| {
            let object = item
                .as_object()
                .ok_or_else(|| invalid("invalidTokenDetails"))?;
            sum.checked_add(integer(object.get(key))?)
                .ok_or_else(|| invalid("tokenOverflow"))
        }),
        _ => Err(invalid("invalidTokenDetails")),
    }
}

pub(crate) fn message_text(value: &Value) -> String {
    let Some(content) = value.get("content") else {
        return String::new();
    };
    let text = crate::session_manager::providers::extract_text(content);
    title::without_context(&text)
}
pub(crate) fn visible_message(value: &Value) -> bool {
    let text = message_text(value);
    let scene_only = !text.trim().is_empty()
        && text.lines().all(|line| {
            let line = line.trim();
            (line.starts_with("@scene:") || line.starts_with("@scene#"))
                && !line.chars().any(char::is_whitespace)
        });
    let image = value
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks.iter().any(|b| {
                matches!(
                    b.get("type").and_then(Value::as_str),
                    Some("input_image" | "image" | "image_url")
                )
            })
        });
    image || (!scene_only && title::clean_prompt(&text).is_some())
}

struct Pending {
    ordinal: i64,
    native_id: Option<String>,
    model: String,
    at_ms: Option<i64>,
    usage: TokenUsage,
}

pub(crate) fn read_workbuddy(path: &Path) -> Result<WorkBuddyRead, AppError> {
    let size = std::fs::metadata(path)
        .map_err(|_| invalid("sourceUnavailable"))?
        .len();
    if size > MAX_SOURCE_BYTES {
        return Err(invalid("sourceTooLarge"));
    }
    let mut session_id: Option<String> = None;
    let mut project: Option<String> = None;
    let mut project_seen = false;
    let mut project_conflict = false;
    let mut created: Option<i64> = None;
    let mut last: Option<i64> = None;
    let mut candidates = TitleCandidates::default();
    let mut messages = BTreeSet::new();
    let mut message_count_known = true;
    let mut analytics = super::session_analytics_messages::TranscriptFacts {
        is_main: true,
        ..Default::default()
    };
    let mut analytics_message_hashes = BTreeMap::new();
    let mut runtime_events = Vec::new();
    let mut runtime_generation = 0u64;
    let mut pending = Vec::new();
    let mut reasons = BTreeSet::new();
    let progress = usage_source::json_lines(
        path,
        |_| true,
        |ordinal, value| {
            if let Some(id) = identifier(value.get("sessionId")) {
                if session_id.as_ref().is_some_and(|prior| prior != &id) {
                    return Err(invalid("conflictingSessionIdentity"));
                }
                session_id = Some(id);
            }
            if let Some(cwd) = value.get("cwd") {
                let current = match cwd {
                    Value::Null => Some(None),
                    Value::String(s) => Some((!s.trim().is_empty()).then(|| s.clone())),
                    _ => None,
                };
                if let Some(current) = current {
                    if project_seen && project != current {
                        project_conflict = true;
                    }
                    project_seen = true;
                    project = current;
                }
            }
            let at_ms = native_time(value.get("timestamp"));
            if let Some(at) = at_ms {
                created = Some(created.map_or(at, |prior| prior.min(at)));
                last = Some(last.map_or(at, |prior| prior.max(at)));
                if runtime_events.len() >= MAX_NATIVE_EVENTS {
                    return Err(invalid("eventBudgetExceeded"));
                }
                let digest = format!(
                    "{:x}",
                    Sha256::digest(
                        serde_json::to_vec(&value).map_err(|_| invalid("invalidNativeEntry"))?
                    )
                );
                runtime_events.push((
                    json!(["record", digest]).to_string(),
                    at,
                    runtime_generation,
                ));
            } else if matches!(
                value["type"].as_str(),
                Some("message" | "reasoning" | "function_call" | "function_call_result")
            ) {
                analytics.reasons.insert("activityTimeUnavailable".into());
                runtime_generation = runtime_generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("eventBudgetExceeded"))?;
            }
            match value.get("type").and_then(Value::as_str) {
                Some("custom-title") => {
                    if let Some(t) = value
                        .get("customTitle")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                    {
                        candidates.custom = Some(title::one_line(&crate::redact::scrub(t)));
                    }
                }
                Some("ai-title") => {
                    if let Some(t) = value
                        .get("aiTitle")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                    {
                        candidates.agent = Some(title::one_line(&crate::redact::scrub(t)));
                    }
                }
                Some("message")
                    if matches!(
                        value.get("role").and_then(Value::as_str),
                        Some("user" | "assistant")
                    ) && visible_message(&value) =>
                {
                    let role = value["role"].as_str().unwrap_or("");
                    let id = if role == "assistant" {
                        identifier(value.pointer("/providerData/messageId"))
                            .or_else(|| identifier(value.get("id")))
                    } else {
                        identifier(value.get("id"))
                    };
                    let native = id.is_some();
                    let identity = id
                        .as_ref()
                        .map(|id| format!("{role}:{id}"))
                        .unwrap_or_else(|| format!("local:{ordinal}"));
                    if analytics.messages.len() >= MAX_NATIVE_EVENTS
                        && !analytics.messages.contains_key(&identity)
                    {
                        return Err(invalid("eventBudgetExceeded"));
                    }
                    if let Some(id) = id {
                        messages.insert((role.to_owned(), id));
                    } else {
                        message_count_known = false;
                        analytics
                            .reasons
                            .insert("messageNativeIdentityUnavailable".into());
                    }
                    if at_ms.is_none() {
                        analytics.reasons.insert("messageTimeUnavailable".into());
                    }
                    analytics.messages.insert(
                        identity.clone(),
                        super::session_analytics_messages::MessageFact {
                            identity: identity.clone(),
                            at_ms,
                            role: role.into(),
                            native_identity: native,
                        },
                    );
                    analytics_message_hashes.insert(
                        identity,
                        format!(
                            "{:x}",
                            Sha256::digest(
                                serde_json::to_vec(&value)
                                    .map_err(|_| invalid("invalidNativeEntry"))?
                            )
                        ),
                    );
                    if role == "user" && candidates.prompt.is_none() {
                        candidates.prompt =
                            title::clean_prompt(&crate::redact::scrub(&message_text(&value)));
                    }
                }
                _ => {}
            }
            let Some(provider) = value.get("providerData") else {
                return Ok(());
            };
            let Some(usage) = provider.get("usage").filter(|v| !v.is_null()) else {
                return Ok(());
            };
            if !usage.is_object() {
                return Err(invalid("invalidUsageShape"));
            }
            match usage.get("requests").and_then(Value::as_u64) {
                Some(1) => {}
                Some(n) if n > 1 => {
                    reasons.insert("aggregatedUsageUnsupported".into());
                    return Ok(());
                }
                _ => {
                    reasons.insert("requestCountUnsupported".into());
                    return Ok(());
                }
            }
            if !usage.get("inputTokens").is_some_and(Value::is_number)
                || !usage.get("outputTokens").is_some_and(Value::is_number)
            {
                reasons.insert("usageShapeUnsupported".into());
                return Ok(());
            }
            let input = integer(usage.get("inputTokens"))?;
            let output = integer(usage.get("outputTokens"))?;
            let read = detail_total(
                usage
                    .get("inputTokensDetails")
                    .or_else(|| usage.get("input_tokens_details")),
                "cached_tokens",
            )?;
            let write = integer(
                usage
                    .get("cacheCreationTokens")
                    .or_else(|| usage.get("cache_creation_input_tokens")),
            )?;
            let new_input = input
                .checked_sub(
                    read.checked_add(write)
                        .ok_or_else(|| invalid("tokenOverflow"))?,
                )
                .ok_or_else(|| invalid("inconsistentCacheBuckets"))?;
            let model = identifier(provider.get("model")).unwrap_or_else(|| "unknown".into());
            let id = identifier(provider.get("messageId"));
            if pending.len() >= MAX_NATIVE_EVENTS {
                return Err(invalid("eventBudgetExceeded"));
            }
            pending.push(Pending {
                ordinal,
                native_id: id.clone(),
                model: model.clone(),
                at_ms,
                usage: TokenUsage {
                    input_tokens: usage_source::checked_token(new_input)?,
                    output_tokens: usage_source::checked_token(output)?,
                    cache_read_tokens: usage_source::checked_token(read)?,
                    cache_creation_tokens: usage_source::checked_token(write)?,
                    model: Some(model),
                    message_id: id,
                },
            });
            Ok(())
        },
    )?;
    if !progress.complete {
        reasons.insert("incompleteEnumeration".into());
    }
    let mut facts = BTreeMap::new();
    for call in pending {
        let native_key = session_id
            .as_deref()
            .zip(call.native_id.as_deref())
            .map(|(sid, id)| usage_source::native_key("workbuddy", sid, id));
        let request_id = match session_id.as_deref().zip(call.native_id.as_deref()) {
            Some((sid, id)) => format!("workbuddy_session:{}", json!([sid, id])),
            None => format!(
                "workbuddy_recovered:{:x}",
                Sha256::digest(
                    json!([path.to_string_lossy(), call.ordinal])
                        .to_string()
                        .as_bytes()
                )
            ),
        };
        facts.insert(
            native_key.clone().unwrap_or_else(|| request_id.clone()),
            NormalizedUsageFact {
                request_id,
                native_key,
                session_id: session_id.clone(),
                model: call.model,
                usage: call.usage,
                at_ms: call.at_ms,
            },
        );
    }
    let facts = facts.into_values().collect::<Vec<_>>();
    if facts.iter().any(|f| f.at_ms.is_none()) {
        reasons.insert("missingTimestamp".into());
    }
    if facts.iter().any(|f| f.native_key.is_none()) {
        reasons.insert("missingNativeEventIdentity".into());
    }
    if session_id.is_none() {
        reasons.insert("missingNativeSessionIdentity".into());
    }
    if facts.iter().any(|f| f.model == "unknown") {
        reasons.insert("missingModel".into());
    }
    if project_conflict {
        project = None;
        reasons.insert("conflictingSessionProject".into());
    }
    analytics.session_id = session_id.clone();
    analytics.project_parsed = project_seen && !project_conflict;
    analytics.project_dir = project.clone();
    if !analytics.project_parsed {
        analytics.reasons.insert("projectNotRecorded".into());
    }
    if !progress.complete {
        analytics.reasons.insert("incompleteSource".into());
    }
    let native_activity = session_id
        .as_deref()
        .map(|sid| {
            runtime_events
                .into_iter()
                .map(|(id, at, generation)| {
                    let mut event =
                        super::session_analytics_native::runtime_event("workbuddy", sid, &id, at);
                    if generation > 0 {
                        event.stream_key = super::session_analytics_index::key(
                            "workbuddy-activity-stream",
                            &json!([sid, generation]).to_string(),
                        );
                    }
                    event
                })
                .collect()
        })
        .unwrap_or_default();
    let session = session_id.map(|session_id| {
        let (title, title_source) = title::titled(&candidates.with_directory(project.as_deref()));
        SessionMeta {
            provider_id: "workbuddy".into(),
            session_id,
            title,
            summary: None,
            project_dir: project,
            created_at: created,
            last_active_at: last,
            source_path: Some(path.to_string_lossy().into_owned()),
            resume_command: None,
            title_source,
            message_count: message_count_known
                .then(|| u32::try_from(messages.len()).ok())
                .flatten(),
            parent_session_id: None,
        }
    });
    Ok(WorkBuddyRead {
        facts,
        line_count: progress.offset,
        reasons,
        session,
        analytics,
        analytics_message_hashes,
        native_activity,
    })
}

pub(crate) fn collect_files(root: &Path) -> Result<Vec<PathBuf>, AppError> {
    fn walk(path: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), AppError> {
        if depth > 16 || out.len() > MAX_SOURCE_FILES {
            return Err(invalid("sourceBudgetExceeded"));
        }
        let meta = std::fs::symlink_metadata(path).map_err(|_| invalid("sourceUnavailable"))?;
        if meta.file_type().is_symlink() {
            return Err(invalid("sourceSymlinkUnsupported"));
        }
        if meta.is_dir() {
            for entry in std::fs::read_dir(path).map_err(|_| invalid("sourceUnavailable"))? {
                walk(
                    &entry.map_err(|_| invalid("sourceUnavailable"))?.path(),
                    depth + 1,
                    out,
                )?;
            }
        } else if meta.is_file() && path.extension().is_some_and(|s| s == "jsonl") {
            out.push(path.to_owned());
        }
        Ok(())
    }
    match std::fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => return Err(invalid("sourceUnavailable")),
        Ok(meta) if !meta.is_dir() => return Err(invalid("sourceRootNotDirectory")),
        Ok(_) => {}
    }
    let mut files = Vec::new();
    walk(root, 0, &mut files)?;
    if files.len() > MAX_SOURCE_FILES {
        return Err(invalid("sourceBudgetExceeded"));
    }
    files.sort();
    Ok(files)
}

pub(crate) fn sync_workbuddy_usage_at(
    db: &crate::database::Database,
    root: &Path,
) -> Result<super::session_usage::SessionSyncResult, AppError> {
    use super::usage_source::{Prepared, ProviderIndex, Snapshot};
    use super::usage_sync_db::{commit_sync_batch, read_sync_state, CheckpointChange, SyncState};
    use rusqlite::{params, OptionalExtension};
    let files = collect_files(root)?;
    let mut result = super::session_usage::SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: files.len() as u32,
        errors: Vec::new(),
    };
    if files.is_empty() {
        return Ok(result);
    }
    let mut index = ProviderIndex::load(db, "workbuddy", root)?;
    let mut sources = Vec::with_capacity(files.len());
    let mut expected_states = Vec::with_capacity(files.len());
    let mut total_bytes = 0_u64;
    for path in &files {
        let snapshot = Snapshot::file(path)?;
        total_bytes = total_bytes
            .checked_add(
                snapshot.stamp["size"]
                    .as_u64()
                    .ok_or_else(|| invalid("invalidSourceStamp"))?,
            )
            .ok_or_else(|| invalid("sourceBudgetExceeded"))?;
        if total_bytes > MAX_SCAN_BYTES {
            return Err(invalid("sourceBudgetExceeded"));
        }
        let expected = read_sync_state(db, &path.to_string_lossy())?;
        sources.push(Prepared::with_index(db, snapshot, expected, 1, &index)?);
        expected_states.push(expected);
    }
    if sources.iter().all(|source| source.unchanged) {
        return Ok(result);
    }
    // A changed scan rereads the bounded roster, including unchanged replicas.
    // Otherwise a changed duplicate could overwrite an unchanged conflicting
    // physical copy solely because that copy's body was skipped.
    let mut parsed = Vec::with_capacity(files.len());
    let mut identities = BTreeMap::<String, Value>::new();
    let mut event_count = 0_usize;
    let mut checkpoints = Vec::with_capacity(files.len());
    for (i, path) in files.iter().enumerate() {
        let read = read_workbuddy(path)?;
        event_count = event_count
            .checked_add(read.facts.len())
            .ok_or_else(|| invalid("eventBudgetExceeded"))?;
        if event_count > MAX_NATIVE_EVENTS {
            return Err(invalid("eventBudgetExceeded"));
        }
        for fact in &read.facts {
            if let Some(native) = &fact.native_key {
                let value = serde_json::to_value(fact).map_err(|_| invalid("invalidFact"))?;
                if identities.get(native).is_some_and(|prior| prior != &value) {
                    return Err(invalid("conflictingNativeReplicas"));
                }
                identities.insert(native.clone(), value);
            }
        }
        let modified = sources[i].snapshot.stamp["mtimeNs"]
            .as_str()
            .and_then(|v| v.parse::<u128>().ok())
            .and_then(|v| i64::try_from(v / 1_000_000_000).ok())
            .ok_or_else(|| invalid("invalidSourceStamp"))?;
        checkpoints.push(CheckpointChange {
            key: path.to_string_lossy().into_owned(),
            expected: expected_states[i],
            next: Some(SyncState {
                modified,
                offset: read.line_count,
            }),
        });
        parsed.push(read);
    }
    commit_sync_batch(db, &checkpoints, |conn| {
        // All leases must be checked before the first facts/proof write changes
        // this scan's shared provider manifest or ledger cardinality.
        if collect_files(root)? != files {
            return Err(invalid("sourceRosterChanged"));
        }
        for source in &sources {
            source.check_on(conn)?;
        }
        for (i, read) in parsed.iter().enumerate() {
            let source = &mut sources[i];
            for reason in &read.reasons {
                source.evidence.complete_event_identity_set = false;
                source.evidence.partial(reason);
            }
            for fact in &read.facts {
                let (Some(native), Some(sid), Some(at_ms)) = (
                    fact.native_key.as_deref(),
                    fact.session_id.as_deref(),
                    fact.at_ms,
                ) else {
                    result.skipped += 1;
                    continue;
                };
                let row: Option<(String,String,Option<String>,Option<String>)> = conn.query_row("SELECT provider_id,app_type,data_source,session_id FROM proxy_request_logs WHERE request_id=?1", [&fact.request_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
                if row.as_ref().is_some_and(|(owner, app, origin, session)| {
                    owner != "_workbuddy_session"
                        || app != "workbuddy"
                        || origin.as_deref() != Some("workbuddy_session")
                        || session.as_deref() != Some(sid)
                }) {
                    return Err(invalid("foreignRecordIdentity"));
                }
                if !source.should_store_on(
                    conn,
                    &fact.request_id,
                    native,
                    "workbuddy_session",
                    at_ms.div_euclid(1000),
                )? {
                    result.skipped += 1;
                    continue;
                }
                let u = &fact.usage;
                let changed = conn.execute("INSERT INTO proxy_request_logs
                    (request_id,provider_id,app_type,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,latency_ms,status_code,session_id,created_at,data_source)
                    VALUES (?1,'_workbuddy_session','workbuddy',?2,?3,?4,?5,?6,0,0,?7,?8,'workbuddy_session')
                    ON CONFLICT(request_id) DO UPDATE SET model=excluded.model,input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cache_read_tokens=excluded.cache_read_tokens,cache_creation_tokens=excluded.cache_creation_tokens,created_at=excluded.created_at
                    WHERE model!=excluded.model OR input_tokens!=excluded.input_tokens OR output_tokens!=excluded.output_tokens OR cache_read_tokens!=excluded.cache_read_tokens OR cache_creation_tokens!=excluded.cache_creation_tokens OR created_at!=excluded.created_at",
                    params![fact.request_id,fact.model,u.input_tokens,u.output_tokens,u.cache_read_tokens,u.cache_creation_tokens,sid,at_ms.div_euclid(1000)])?;
                if changed > 0 {
                    result.imported += 1;
                } else {
                    result.skipped += 1;
                }
                source.evidence.acknowledge(native);
            }
            source.save_on(conn, checkpoints[i].next.unwrap())?;
            // Partial native coverage stays in typed evidence. It is not a
            // failed transaction and must not block unrelated maintenance.
        }
        Ok(())
    })?;
    index.advance_many(sources);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn workbuddy_analytics_messages_without_usage_are_bounded_before_publication() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("many.jsonl");
        use std::io::Write;
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        for i in 0..=MAX_NATIVE_EVENTS {
            writeln!(file,"{}",json!({"type":"message","role":"user","id":format!("u{i}"),"sessionId":"s","timestamp":1000,"content":"q"})).unwrap();
        }
        file.flush().unwrap();
        assert!(
            matches!(read_workbuddy(&path),Err(AppError::InvalidInput(code)) if code=="usage.workbuddy.eventBudgetExceeded")
        );
    }

    fn line(sid: &str, id: &str, at: i64, input: u64, output: u64) -> Value {
        json!({"type":"message","role":"assistant","id":format!("item-{id}"),"sessionId":sid,"timestamp":at,
            "content":[{"type":"output_text","text":"private-body-not-indexed"}],
            "providerData":{"messageId":id,"model":"billed-native","requestModelId":"custom-local:historical-route",
                "usage":{"requests":1,"inputTokens":input,"outputTokens":output,"inputTokensDetails":[{"cached_tokens":20}],"cacheCreationTokens":10,"outputTokensDetails":[{"reasoning_tokens":30_u64.min(output)}]}}})
    }
    fn source(rows: &[Value]) -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("s.jsonl");
        let text = rows
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&path, text).unwrap();
        (temp, path)
    }
    fn buckets(fact: &NormalizedUsageFact) -> [u32; 4] {
        [
            fact.usage.input_tokens,
            fact.usage.output_tokens,
            fact.usage.cache_read_tokens,
            fact.usage.cache_creation_tokens,
        ]
    }

    #[test]
    fn native_runtime_workbuddy_reader_retains_all_timed_records_without_body_or_guessing_sid() {
        let (_temp, path) = source(&[
            json!({"type":"function_call","id":"call","timestamp":1000,"arguments":"private runtime not indexed"}),
            json!({"type":"function_call_result","id":"result","sessionId":"native-s","timestamp":1500,"output":"private runtime not indexed"}),
            line("native-s", "assistant", 2000, 130, 50),
            json!({"type":"reasoning","id":"invalid-clock","sessionId":"native-s","timestamp":"bad time","content":"private runtime not indexed"}),
        ]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(
            parsed
                .native_activity
                .iter()
                .map(|event| event.at_ms)
                .collect::<Vec<_>>(),
            [1000, 1500, 2000]
        );
        assert!(parsed.native_activity.iter().all(|event| event.session_key
            == super::super::session_analytics_index::key("workbuddy", "native-s")
            && event.main));
        let encoded = serde_json::to_string(&parsed.native_activity).unwrap();
        assert!(!encoded.contains("private runtime"));
        assert!(!encoded.contains("private-body-not-indexed"));
    }

    #[test]
    fn native_runtime_workbuddy_missing_time_cuts_activity_stream() {
        let (_temp, path) = source(&[
            json!({"type":"message","role":"user","id":"u","sessionId":"s","timestamp":1000,"content":"question"}),
            json!({"type":"reasoning","id":"unknown-clock","sessionId":"s","content":"private reasoning"}),
            line("s", "assistant", 61_000, 130, 50),
        ]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        let result = super::super::session_native_activity::activity(
            &parsed.native_activity,
            &super::super::session_analytics_core::Window {
                start_ms: Some(0),
                end_ms: 100_000,
            },
            &[super::super::session_analytics_core::DayBoundary {
                date: "date".into(),
                start_ms: 0,
                end_ms: 100_000,
            }],
        )
        .unwrap();
        assert_eq!(
            result.active_ms, 0,
            "an unknown intervening timestamp cannot establish one verified interval"
        );
        assert!(parsed.analytics.reasons.contains("activityTimeUnavailable"));
    }

    #[test]
    fn workbuddy_usage_root_file_is_unknown_instead_of_an_empty_installation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        std::fs::write(&root, "invalid native directory").unwrap();
        let db = crate::database::Database::memory().unwrap();
        assert!(
            sync_workbuddy_usage_at(&db, &root).is_err(),
            "an existing file must not report successful empty usage"
        );
    }

    #[test]
    fn workbuddy_modern_input_includes_cache_and_output_already_includes_reasoning() {
        let (_temp, path) = source(&[line("s", "m", 1_792_022_400_123, 130, 50)]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(buckets(&parsed.facts[0]), [100, 50, 20, 10]);
        assert_eq!(parsed.facts[0].at_ms, Some(1_792_022_400_123));
        assert_eq!(parsed.facts[0].model, "billed-native");
        assert!(parsed.reasons.is_empty());
        let encoded = serde_json::to_string(&parsed.facts).unwrap();
        assert!(!encoded.contains("private-body-not-indexed"));
        assert!(!encoded.contains("content"));
    }

    #[test]
    fn workbuddy_native_timestamp_is_milliseconds_without_a_magnitude_heuristic() {
        let (_temp, path) = source(&[line("s", "m", 1000, 130, 50)]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts[0].at_ms, Some(1000));
    }

    #[test]
    fn workbuddy_function_call_usage_is_a_native_record_without_an_assistant_message() {
        let mut row = line("s", "m", 1_792_022_400_000, 130, 50);
        row["type"] = json!("function_call");
        row["name"] = json!("tool");
        row["arguments"] = json!("private-tool-input-not-indexed");
        row.as_object_mut().unwrap().remove("content");
        let (_temp, path) = source(&[row]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(buckets(&parsed.facts[0]), [100, 50, 20, 10]);
        assert!(!serde_json::to_string(&parsed.facts)
            .unwrap()
            .contains("private-tool-input-not-indexed"));
    }

    #[test]
    fn workbuddy_object_cache_details_and_explicit_zero_do_not_fall_back_to_legacy_counts() {
        let mut row = line("s", "m", 1_792_022_400_000, 0, 0);
        row["providerData"]["usage"] = json!({"requests":1,"inputTokens":0,"outputTokens":0,"input_tokens":999,"output_tokens":888,"inputTokensDetails":{"cached_tokens":0},"cacheCreationTokens":0,"input_tokens_details":{"cached_tokens":777}});
        let (_temp, path) = source(&[row]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(buckets(&parsed.facts[0]), [0, 0, 0, 0]);
    }

    #[test]
    fn workbuddy_nonconsecutive_native_revision_replaces_tokens_model_and_day() {
        let a = line("s", "a", 1_792_022_400_000, 130, 50);
        let b = line("s", "b", 1_792_022_401_000, 230, 60);
        let mut revised = line("s", "a", 1_792_108_800_000, 330, 70);
        revised["providerData"]["model"] = json!("changed-native");
        let (_temp, path) = source(&[a, b, revised]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts.len(), 2);
        let a = parsed
            .facts
            .iter()
            .find(|f| f.model == "changed-native")
            .unwrap();
        assert_eq!(buckets(a), [300, 70, 20, 10]);
        assert_eq!(a.at_ms, Some(1_792_108_800_000));
    }

    #[test]
    fn workbuddy_native_identity_survives_file_move_and_is_scoped_to_native_session() {
        let (temp, path) = source(&[line("s", "same", 1_792_022_400_000, 130, 50)]);
        let before = read_workbuddy(&path).unwrap();
        let moved = temp.path().join("renamed.jsonl");
        std::fs::rename(&path, &moved).unwrap();
        let after = read_workbuddy(&moved).unwrap();
        assert_eq!(before.facts[0].native_key, after.facts[0].native_key);
        assert_eq!(before.facts[0].request_id, after.facts[0].request_id);
        let (_other, other) = source(&[line(
            "different-session",
            "same",
            1_792_022_400_000,
            130,
            50,
        )]);
        let separate = read_workbuddy(&other).unwrap();
        assert_ne!(before.facts[0].native_key, separate.facts[0].native_key);
        assert_ne!(before.facts[0].request_id, separate.facts[0].request_id);
    }

    #[test]
    fn workbuddy_native_identity_cannot_collide_at_a_string_delimiter() {
        let (_a, a) = source(&[line("a:b", "c", 1_792_022_400_000, 130, 50)]);
        let (_b, b) = source(&[line("a", "b:c", 1_792_022_400_000, 130, 50)]);
        let a = read_workbuddy(&a).unwrap();
        let b = read_workbuddy(&b).unwrap();
        assert_ne!(a.facts[0].native_key, b.facts[0].native_key);
        assert_ne!(a.facts[0].request_id, b.facts[0].request_id);
    }

    #[test]
    fn workbuddy_missing_time_and_native_identity_stay_partial_without_now_or_ordinal_ack() {
        let mut row = line("s", "m", 1_792_022_400_000, 130, 50);
        row.as_object_mut().unwrap().remove("timestamp");
        row["providerData"]
            .as_object_mut()
            .unwrap()
            .remove("messageId");
        let (_temp, path) = source(&[row]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(parsed.facts[0].at_ms, None);
        assert_eq!(parsed.facts[0].native_key, None);
        assert!(parsed.reasons.contains("missingTimestamp"));
        assert!(parsed.reasons.contains("missingNativeEventIdentity"));
    }

    #[test]
    fn workbuddy_later_native_header_supplies_the_session_scope_without_a_filename_guess() {
        let mut usage = line("s", "m", 1_792_022_400_000, 130, 50);
        usage.as_object_mut().unwrap().remove("sessionId");
        let (_temp, path) = source(&[
            usage,
            json!({"type":"ai-title","sessionId":"native-s","aiTitle":"title"}),
        ]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts[0].session_id.as_deref(), Some("native-s"));
        assert!(parsed.facts[0].native_key.is_some());
    }

    #[test]
    fn workbuddy_no_native_session_id_is_partial_even_when_the_filename_looks_like_one() {
        let mut usage = line("s", "m", 1_792_022_400_000, 130, 50);
        usage.as_object_mut().unwrap().remove("sessionId");
        let (_temp, path) = source(&[usage]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts[0].session_id, None);
        assert_eq!(parsed.facts[0].native_key, None);
        assert!(parsed.reasons.contains("missingNativeSessionIdentity"));
    }

    #[test]
    fn workbuddy_valid_json_without_newline_is_complete_but_incomplete_tail_is_not() {
        let (temp, path) = source(&[line("s", "a", 1_792_022_400_000, 130, 50)]);
        let complete = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, complete.trim_end_matches('\n')).unwrap();
        let valid = read_workbuddy(&path).unwrap();
        assert_eq!(valid.facts.len(), 1);
        assert!(valid.reasons.is_empty());
        let partial = temp.path().join("partial.jsonl");
        std::fs::write(
            &partial,
            format!("{complete}{{\"providerData\":{{\"usage\":"),
        )
        .unwrap();
        let parsed = read_workbuddy(&partial).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert!(parsed.reasons.contains("incompleteEnumeration"));
        assert_eq!(parsed.line_count, 1);
    }

    #[test]
    fn workbuddy_aggregate_usage_is_explicitly_unsupported_instead_of_one_native_call() {
        let mut row = line("s", "m", 1_792_022_400_000, 130, 50);
        row["providerData"]["usage"]["requests"] = json!(2);
        let (_temp, path) = source(&[row]);
        let parsed = read_workbuddy(&path).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed.reasons.contains("aggregatedUsageUnsupported"));
    }

    #[test]
    fn workbuddy_missing_model_is_not_reconstructed_from_requested_route() {
        let mut row = line("s", "m", 1_792_022_400_000, 130, 50);
        row["providerData"].as_object_mut().unwrap().remove("model");
        let (_temp, path) = source(&[row]);
        let parsed = read_workbuddy(&path).unwrap();
        assert_eq!(parsed.facts[0].model, "unknown");
        assert!(parsed.reasons.contains("missingModel"));
    }

    #[test]
    fn workbuddy_invalid_token_integers_and_cache_underflow_fail_closed() {
        for value in [json!(-1), json!(1.5), json!(u64::MAX)] {
            let mut row = line("s", "m", 1_792_022_400_000, 130, 50);
            row["providerData"]["usage"]["inputTokens"] = value;
            let (_temp, path) = source(&[row]);
            assert!(read_workbuddy(&path).is_err());
        }
        let (_temp, path) = source(&[line("s", "m", 1_792_022_400_000, 29, 50)]);
        assert!(read_workbuddy(&path).is_err());
    }

    #[test]
    fn workbuddy_current_price_scientific_estimate_uses_normalized_four_buckets() {
        let (_temp, path) = source(&[line("s", "m", 1_792_022_400_000, 130, 50)]);
        let parsed = read_workbuddy(&path).unwrap();
        let db = crate::database::Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let catalog=crate::services::ofox_pricing::PriceCatalog::ingest_on(&conn,"ofox.ai",&json!({"data":[{"id":"billed-native","pricing":{"prompt":"1e-9","completion":"2e-9","input_cache_read":"3e-9","input_cache_write":"4e-9"}}]}),1).unwrap();
        let estimated = catalog.estimate(
            &parsed.facts[0].model,
            buckets(&parsed.facts[0]).map(u64::from),
            Some(1),
        );
        assert_eq!(estimated.known_cost_usd.as_deref(), Some("0.0000003"));
        assert_eq!(estimated.availability, "known");
    }

    #[test]
    fn workbuddy_sync_native_revision_moves_model_and_day_without_adding_a_second_record() {
        let (_temp, path) = source(&[line("s", "m", 1_792_022_400_000, 130, 50)]);
        let db = crate::database::Database::memory().unwrap();
        sync_workbuddy_usage_at(&db, path.parent().unwrap()).unwrap();
        let before: String = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT request_id FROM proxy_request_logs", [], |r| {
                r.get(0)
            })
            .unwrap();
        let mut revised = line("s", "m", 1_792_108_800_000, 230, 60);
        revised["providerData"]["model"] = json!("revised-native");
        std::fs::write(&path, revised.to_string()).unwrap();
        sync_workbuddy_usage_at(&db, path.parent().unwrap()).unwrap();
        let conn = db.conn.lock().unwrap();
        let actual:(String,String,i64,i64,i64,String,String,String)=conn.query_row("SELECT request_id,model,input_tokens,output_tokens,created_at,provider_id,data_source,total_cost_usd FROM proxy_request_logs",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).unwrap();
        assert_eq!(
            actual,
            (
                before,
                "revised-native".into(),
                200,
                60,
                1_792_108_800,
                "_workbuddy_session".into(),
                "workbuddy_session".into(),
                "0".into()
            )
        );
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r
                .get(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn workbuddy_sync_same_session_replicas_are_global_across_files_but_distinct_sessions_are_not()
    {
        let (temp, path) = source(&[line("s", "m", 1_792_022_400_000, 130, 50)]);
        std::fs::copy(&path, temp.path().join("mirror.jsonl")).unwrap();
        std::fs::write(
            temp.path().join("other.jsonl"),
            line("other", "m", 1_792_022_400_000, 230, 60).to_string(),
        )
        .unwrap();
        let db = crate::database::Database::memory().unwrap();
        sync_workbuddy_usage_at(&db, temp.path()).unwrap();
        let conn = db.conn.lock().unwrap();
        let actual: (i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*),SUM(input_tokens),SUM(output_tokens) FROM proxy_request_logs",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(actual, (2, 300, 110));
    }

    #[test]
    fn workbuddy_sync_conflicting_physical_replicas_do_not_publish_an_arbitrary_winner() {
        let (temp, _path) = source(&[line("s", "m", 1_792_022_400_000, 130, 50)]);
        std::fs::write(
            temp.path().join("conflict.jsonl"),
            line("s", "m", 1_792_022_400_000, 230, 60).to_string(),
        )
        .unwrap();
        let db = crate::database::Database::memory().unwrap();
        assert!(sync_workbuddy_usage_at(&db, temp.path()).is_err());
        let conn = db.conn.lock().unwrap();
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r
                .get(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM session_log_sync", [], |r| r.get(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn workbuddy_late_checkpoint_failure_rolls_back_every_roster_fact_and_receipt() {
        fn dump(db: &crate::database::Database) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
            let conn = db.conn.lock().unwrap();
            [
                "SELECT * FROM proxy_request_logs ORDER BY request_id",
                "SELECT * FROM session_log_sync ORDER BY file_path",
                "SELECT * FROM session_meta_cache ORDER BY source_path",
                "SELECT * FROM maintenance_flags ORDER BY key",
            ]
            .iter()
            .map(|sql| {
                let mut statement = conn.prepare(sql).unwrap();
                let columns = statement.column_count();
                let rows = statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|i| row.get::<_, rusqlite::types::Value>(i))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap();
                rows.map(Result::unwrap).collect()
            })
            .collect()
        }
        let (temp, path) = source(&[line("s", "a", 1_792_022_400_000, 130, 50)]);
        let last = temp.path().join("z.jsonl");
        let first_z = line("z", "z", 1_792_022_400_000, 230, 60);
        std::fs::write(&last, first_z.to_string()).unwrap();
        let db = crate::database::Database::memory().unwrap();
        sync_workbuddy_usage_at(&db, temp.path()).unwrap();
        let before = dump(&db);
        std::fs::write(
            &path,
            [
                line("s", "a", 1_792_022_400_000, 130, 50),
                line("s", "a2", 1_792_022_401_000, 330, 70),
            ]
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
        )
        .unwrap();
        std::fs::write(
            &last,
            [first_z, line("z", "z2", 1_792_022_401_000, 430, 80)]
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        db.conn.lock().unwrap().execute_batch("CREATE TEMP TRIGGER fail_wb_last_checkpoint BEFORE INSERT ON session_log_sync WHEN NEW.file_path LIKE '%z.jsonl' BEGIN SELECT RAISE(ABORT,'fixture late checkpoint failure'); END;").unwrap();
        assert!(sync_workbuddy_usage_at(&db, temp.path()).is_err());
        assert_eq!(dump(&db),before,"a failed final checkpoint must preserve every fact, checkpoint, source part and provider manifest");
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_wb_last_checkpoint;")
            .unwrap();
        assert_eq!(
            sync_workbuddy_usage_at(&db, temp.path()).unwrap().imported,
            2
        );
    }

    #[test]
    fn workbuddy_sync_parse_failure_preserves_the_last_facts_checkpoint_and_proof() {
        let (_temp, path) = source(&[line("s", "m", 1_792_022_400_000, 130, 50)]);
        let db = crate::database::Database::memory().unwrap();
        sync_workbuddy_usage_at(&db, path.parent().unwrap()).unwrap();
        let before:(String,i64)=db.conn.lock().unwrap().query_row("SELECT meta_json,(SELECT last_line_offset FROM session_log_sync LIMIT 1) FROM session_meta_cache WHERE provider_id='workbuddy'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let mut corrupt = line("s", "m", 1_792_022_400_000, 230, 60);
        corrupt["providerData"]["usage"]["inputTokens"] = json!(-1);
        std::fs::write(&path, corrupt.to_string()).unwrap();
        assert!(sync_workbuddy_usage_at(&db, path.parent().unwrap()).is_err());
        let conn = db.conn.lock().unwrap();
        let after:(String,i64)=conn.query_row("SELECT meta_json,(SELECT last_line_offset FROM session_log_sync LIMIT 1) FROM session_meta_cache WHERE provider_id='workbuddy'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(before, after);
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT input_tokens FROM proxy_request_logs", [], |r| r
                .get(0))
                .unwrap(),
            100
        );
    }

    #[test]
    fn workbuddy_sync_never_overwrites_or_acknowledges_a_foreign_session_row() {
        let (_temp, path) = source(&[line("s", "m", 1_792_022_400_000, 130, 50)]);
        let parsed = read_workbuddy(&path).unwrap();
        let db = crate::database::Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES(?1,'_workbuddy_session','workbuddy','foreign',999,0,0,'other-session',1792022400,'workbuddy_session')",[&parsed.facts[0].request_id]).unwrap();
        assert!(sync_workbuddy_usage_at(&db, path.parent().unwrap()).is_err());
        let conn = db.conn.lock().unwrap();
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT input_tokens FROM proxy_request_logs", [], |r| r
                .get(0))
                .unwrap(),
            999
        );
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM session_log_sync", [], |r| r.get(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM session_meta_cache WHERE provider_id='workbuddy'",
                [],
                |r| r.get(0)
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn workbuddy_pruned_native_zero_and_old_gap_fill_are_not_reimported_after_a_move() {
        let old = 1_577_836_800_123;
        let mut zero = line("s", "zero", old, 0, 0);
        zero["providerData"]["usage"]["inputTokensDetails"] = json!({"cached_tokens":0});
        zero["providerData"]["usage"]["cacheCreationTokens"] = json!(0);
        let a = line("s", "a", old, 130, 13);
        let (temp, path) = source(&[zero.clone(), a.clone()]);
        let db = crate::database::Database::memory().unwrap();
        sync_workbuddy_usage_at(&db, temp.path()).unwrap();
        assert_eq!(db.rollup_and_prune(30).unwrap(), 2);
        let c = line("s", "old-missing", old + 1, 230, 23);
        std::fs::write(
            &path,
            [zero, a, c]
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        sync_workbuddy_usage_at(&db, temp.path()).unwrap();
        assert_eq!(db.rollup_and_prune(30).unwrap(), 1);
        std::fs::rename(&path, temp.path().join("moved.jsonl")).unwrap();
        sync_workbuddy_usage_at(&db, temp.path()).unwrap();
        let conn = db.conn.lock().unwrap();
        assert_eq!(
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r
                .get(0))
                .unwrap(),
            0
        );
        let totals:(i64,i64,i64)=conn.query_row("SELECT SUM(request_count),SUM(input_tokens),SUM(output_tokens) FROM usage_daily_rollups WHERE app_type='workbuddy'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(totals, (3, 300, 36));
        let raw:String=conn.query_row("SELECT meta_json FROM session_meta_cache WHERE provider_id='workbuddy' ORDER BY source_path DESC LIMIT 1",[],|r|r.get(0)).unwrap();
        assert!(!raw.contains("private-body-not-indexed"));
    }

    #[test]
    fn workbuddy_normalized_api_reports_native_time_and_four_buckets() {
        let (_temp, path) = source(&[line("s", "m", 1_792_022_400_123, 130, 50)]);
        let normalized = super::super::usage_source::read_normalized("workbuddy", &path).unwrap();
        assert!(normalized.complete);
        assert_eq!(normalized.facts[0].at_ms, Some(1_792_022_400_123));
        assert_eq!(buckets(&normalized.facts[0]), [100, 50, 20, 10]);
    }

    #[test]
    fn workbuddy_normalized_api_keeps_aggregate_unsupported_reason() {
        let mut row = line("s", "m", 1_792_022_400_000, 130, 50);
        row["providerData"]["usage"]["requests"] = json!(2);
        let (_temp, path) = source(&[row]);
        let normalized = super::super::usage_source::read_normalized("workbuddy", &path).unwrap();
        assert!(!normalized.complete);
        assert!(normalized.facts.is_empty());
        assert!(normalized.reasons.contains("aggregatedUsageUnsupported"));
    }
}
