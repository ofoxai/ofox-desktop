//! dsh native spend facts; independent of the active conversation surface.
//! dsh 0.1 writes session format 0, dsh 0.2 writes format 4 (`session.v4.jsonl.zstd`)
//! with the same row grammar (checked against dsh 0.2.1-rc.2 sessions), so both
//! are read by the same rules. Other versions stay unsupported.
use crate::{
    error::AppError,
    proxy::usage::parser::TokenUsage,
    services::usage_source::{self, NormalizedUsageFact},
    session_manager::providers::dsh,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Read},
    path::Path,
};

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
/// 2: session format 4 (dsh 0.2) is read. Files a revision-1 parser skipped as
/// unsupported are read again.
pub(crate) const PARSER_REVISION: u32 = 2;
const MAX_SCAN_BYTES: u64 = 128 * 1024 * 1024;
const MAX_NATIVE_EVENTS: usize = 100_000;
fn invalid(code: &str) -> AppError {
    AppError::InvalidInput(format!("usage.dsh.{code}"))
}
fn identifier(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= 512)
        .map(str::to_owned)
}
fn integer(usage: &Value, field: &str) -> Result<u32, AppError> {
    usage_source::checked_token(usage_source::number(usage, field)?)
}
fn native_time(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_i64)
        .filter(|at| *at >= 0 && chrono::DateTime::from_timestamp_millis(*at).is_some())
}
fn event_identity(id: &str) -> String {
    json!(["assistant", id]).to_string()
}

const JS_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
fn safe_integer(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .filter(|n| n.checked_abs().is_some_and(|n| n <= JS_SAFE_INTEGER))
}
fn nonnegative_safe(value: &Value) -> bool {
    safe_integer(value).is_some_and(|n| n >= 0)
}
fn supported_header(value: &Value) -> bool {
    nonnegative_safe(&value["createdAt"])
        && nonnegative_safe(&value["delegationDepth"])
        && value.get("seedLength").is_none_or(nonnegative_safe)
        && value
            .get("origin")
            .is_none_or(|origin| origin == "subagent")
        && value.get("agentPreset").is_none_or(Value::is_string)
        && value.get("sandboxMode").is_none()
        && value.get("approvalPolicy").is_none()
}
fn exact_keys(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|m| m.len() == keys.len() && keys.iter().all(|k| m.contains_key(*k)))
}
fn packed_length(value: &Value, kind: &str) -> Option<usize> {
    if !exact_keys(value, &["type", "seq0", "time0", "data"]) || !nonnegative_safe(&value["seq0"]) {
        return None;
    }
    let mut time = safe_integer(&value["time0"])?;
    let data = &value["data"];
    let field = if kind == "tool-call-chunks" {
        let named = data.get("name").is_some();
        let keys = if named {
            &["turn", "step", "index", "id", "name", "dt", "args"][..]
        } else {
            &["turn", "step", "index", "id", "dt", "args"][..]
        };
        if !exact_keys(data, keys)
            || !data["id"].is_string()
            || (named && !data["name"].is_string())
        {
            return None;
        }
        "args"
    } else {
        if !exact_keys(data, &["turn", "step", "index", "dt", "texts"]) {
            return None;
        }
        "texts"
    };
    if ["turn", "step", "index"]
        .iter()
        .any(|key| !data[*key].is_number())
    {
        return None;
    }
    let members = data[field].as_array()?;
    let dt = data["dt"].as_array()?;
    if members.is_empty() || members.iter().any(|v| !v.is_string()) || dt.len() != members.len() - 1
    {
        return None;
    }
    for gap in dt {
        time = time.checked_add(safe_integer(gap)?)?;
        if time.checked_abs()? > JS_SAFE_INTEGER {
            return None;
        }
    }
    let last = value["seq0"]
        .as_u64()?
        .checked_add(members.len() as u64 - 1)?;
    (last <= JS_SAFE_INTEGER as u64).then_some(members.len())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DshMetadata {
    /// Outer None means the header did not establish project metadata.
    pub project: Option<Option<String>>,
    pub title: Option<String>,
    pub delegated: bool,
}
pub(crate) struct DshRead {
    pub facts: Vec<NormalizedUsageFact>,
    pub reasons: BTreeSet<String>,
    pub line_count: i64,
    pub session_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub metadata: Option<DshMetadata>,
    pub decoded_bytes: u64,
    pub supported_version: bool,
}

/// Native v0 commits newline-terminated rows only. Keep the same open handle
/// through zstd decoding and both physical identity checks.
pub(crate) fn read_dsh(path: &Path) -> Result<DshRead, AppError> {
    if std::fs::metadata(path)
        .map_err(|_| invalid("sourceUnavailable"))?
        .len()
        > MAX_SOURCE_BYTES
    {
        return Err(invalid("sourceTooLarge"));
    }
    usage_source::with_stable_file(path, |file| {
        let decoder = dsh::open_handle(file, path).map_err(|_| invalid("sourceReadFailed"))?;
        let mut reader = BufReader::new(decoder.take(MAX_SOURCE_BYTES + 1));
        let mut bytes = Vec::new();
        let mut read = DshRead {
            facts: vec![],
            reasons: BTreeSet::new(),
            line_count: 0,
            session_id: None,
            parent_session_id: None,
            metadata: None,
            decoded_bytes: 0,
            supported_version: false,
        };
        let mut facts = BTreeMap::new();
        let mut seq = 0_u64;
        let mut seed_cut = 0_u64;
        let mut seed_known = true;
        let mut seeded = false;
        let mut seed_marker = false;
        loop {
            bytes.clear();
            let count = reader
                .read_until(b'\n', &mut bytes)
                .map_err(|_| invalid("sourceReadFailed"))?;
            if count == 0 {
                break;
            }
            read.decoded_bytes = read
                .decoded_bytes
                .checked_add(count as u64)
                .ok_or_else(|| invalid("sourceTooLarge"))?;
            if read.decoded_bytes > MAX_SOURCE_BYTES {
                return Err(invalid("sourceTooLarge"));
            }
            if bytes.last() != Some(&b'\n') {
                read.reasons.insert("incompleteEnumeration".into());
                break;
            }
            let value: Value = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(_) if read.line_count > 0 => {
                    read.reasons.insert("incompleteEnumeration".into());
                    break;
                }
                Err(_) => return Err(invalid("invalidSessionHeader")),
            };
            if read.line_count == 0 {
                if value["type"] != "session" {
                    return Err(invalid("invalidSessionHeader"));
                }
                read.session_id = identifier(value.get("id"));
                read.parent_session_id = identifier(value.get("parentSession"));
                if read.session_id.is_none() {
                    read.reasons.insert("missingNativeSessionIdentity".into());
                }
                read.line_count = 1;
                if !matches!(value["version"].as_u64(), Some(0 | 4)) {
                    read.reasons.insert("unsupportedSourceVersion".into());
                    break;
                }
                if !supported_header(&value) {
                    read.reasons.insert("unsupportedSourceHeader".into());
                    break;
                }
                read.supported_version = true;
                read.metadata = Some(DshMetadata {
                    project: match value.get("cwd") {
                        Some(Value::String(path)) if !path.trim().is_empty() => {
                            Some(Some(path.clone()))
                        }
                        Some(Value::String(_) | Value::Null) => Some(None),
                        _ => None,
                    },
                    title: None,
                    delegated: value["delegationDepth"].as_u64().is_some_and(|n| n > 0)
                        && value["origin"] == "subagent",
                });
                if let Some(cut) = value.get("seedLength") {
                    seeded = true;
                    if let Some(cut) = cut.as_u64() {
                        seed_cut = cut;
                    } else {
                        seed_known = false;
                    }
                } else if value["isSeeded"] == true {
                    seeded = true;
                    seed_known = false;
                }
                if !seed_known {
                    read.reasons.insert("seedBoundaryUnavailable".into());
                }
                continue;
            }
            let kind = value["type"].as_str().unwrap_or("");
            let packed = matches!(
                kind,
                "text-chunks" | "reasoning-chunks" | "tool-call-chunks"
            );
            let start = value[if packed { "seq0" } else { "seq" }].as_u64();
            let expanded = if packed {
                packed_length(&value, kind)
            } else {
                Some(1)
            };
            let Some(end) = expanded.and_then(|n| seq.checked_add(n as u64)) else {
                read.reasons.insert("incompleteEnumeration".into());
                break;
            };
            if start != Some(seq) || seq > 1_000_000 {
                read.reasons.insert("incompleteEnumeration".into());
                break;
            }
            read.line_count += 1;
            let own = seed_known && seq >= seed_cut;
            if seeded
                && kind == "session/end-seed"
                && (seq == seed_cut || seq.checked_add(1) == Some(seed_cut))
            {
                seed_marker = true;
            }
            seq = end;
            if packed {
                continue;
            }
            if kind == "session/title" {
                if let Some(title) = value.pointer("/data/title").and_then(Value::as_str) {
                    if title.len() <= 8192 {
                        if let Some(meta) = &mut read.metadata {
                            meta.title = Some(crate::session_manager::title::one_line(
                                &crate::redact::scrub(title),
                            ));
                        }
                    }
                }
            }
            let Some(usage) = value.pointer("/data/usage").filter(|v| !v.is_null()) else {
                continue;
            };
            if !own {
                continue;
            }
            if kind != "assistant/message" {
                read.reasons.insert("unsupportedUsageEvent".into());
                continue;
            }
            if value
                .pointer("/data/message/source/kind")
                .and_then(Value::as_str)
                != Some("model")
                || value.pointer("/data/message/role").and_then(Value::as_str) != Some("assistant")
            {
                read.reasons.insert("unsupportedUsageSource".into());
                continue;
            }
            if !usage.is_object()
                || !usage["inputTokens"].is_number()
                || !usage["outputTokens"].is_number()
            {
                read.reasons.insert("usageShapeUnsupported".into());
                continue;
            }
            let model = identifier(
                value.pointer("/data/message/source/replayState/response/responseModel"),
            )
            .or_else(|| identifier(value.pointer("/data/message/source/model")))
            .unwrap_or_else(|| "unknown".into());
            let usage = TokenUsage {
                input_tokens: integer(usage, "inputTokens")?,
                output_tokens: integer(usage, "outputTokens")?,
                cache_read_tokens: integer(usage, "cacheReadTokens")?,
                cache_creation_tokens: integer(usage, "cacheWriteTokens")?,
                cache_creation_1h_tokens: 0,
                model: Some(model.clone()),
                message_id: identifier(value.pointer("/data/message/id")),
            };
            let id = identifier(value.pointer("/data/message/id"));
            let native_key = read
                .session_id
                .as_ref()
                .zip(id.as_ref())
                .map(|(sid, id)| usage_source::native_key("dsh", sid, &event_identity(id)));
            let request_id = read
                .session_id
                .as_ref()
                .zip(id.as_ref())
                .map(|(sid, id)| format!("dsh_session:{}", json!([sid, "assistant", id])))
                .unwrap_or_else(|| {
                    format!(
                        "dsh_recovered:{:x}",
                        Sha256::digest(
                            json!([path.to_string_lossy(), read.line_count])
                                .to_string()
                                .as_bytes()
                        )
                    )
                });
            let fact = NormalizedUsageFact {
                request_id: request_id.clone(),
                native_key: native_key.clone(),
                session_id: read.session_id.clone(),
                model,
                usage,
                at_ms: native_time(value.get("time")),
            };
            if facts.len() >= MAX_NATIVE_EVENTS {
                return Err(invalid("eventBudgetExceeded"));
            }
            facts.insert(native_key.unwrap_or(request_id), fact);
        }
        if seeded && seed_known && (!seed_marker || seq < seed_cut) {
            read.reasons.insert("seedBoundaryIncomplete".into());
        }
        read.facts = facts.into_values().collect();
        if read.facts.iter().any(|f| f.at_ms.is_none()) {
            read.reasons.insert("missingTimestamp".into());
        }
        if read.facts.iter().any(|f| f.native_key.is_none()) {
            read.reasons.insert("missingNativeEventIdentity".into());
        }
        if read.facts.iter().any(|f| f.model == "unknown") {
            read.reasons.insert("missingModel".into());
        }
        Ok(read)
    })
}

#[cfg(test)]
pub fn sync_dsh_usage(
    db: &crate::database::Database,
) -> Result<super::session_usage::SessionSyncResult, AppError> {
    let home = super::usage_sync::resolve_root("dsh")?;
    sync_dsh_usage_at(db, &home)
}
pub(crate) fn sync_dsh_usage_at(
    db: &crate::database::Database,
    home: &Path,
) -> Result<super::session_usage::SessionSyncResult, AppError> {
    use super::usage_source::{Prepared, ProviderIndex, Snapshot};
    use super::usage_sync_db::{commit_sync_batch, read_sync_state, CheckpointChange, SyncState};
    use crate::session_manager::source_cache::DshNativeGenerations;
    use rusqlite::{params, OptionalExtension};
    let files = dsh::usage_files(home).map_err(|_| invalid("sourceEnumerationFailed"))?;
    let mut result = super::session_usage::SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: files.len() as u32,
        errors: vec![],
    };
    if files.is_empty() {
        return Ok(result);
    }
    let mut index = ProviderIndex::load(db, "dsh", home)?;
    let mut generations = {
        let conn = crate::database::lock_conn!(db.conn);
        DshNativeGenerations::load_on(&conn, home, index.origin_empty())?
    };
    let roster = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&files).map_err(|_| invalid("invalidRoster"))?)
    );
    let mut sources = Vec::with_capacity(files.len());
    let mut expected = Vec::with_capacity(files.len());
    let mut total_bytes = 0_u64;
    for (path, _) in &files {
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
        let checkpoint = read_sync_state(db, &path.to_string_lossy())?;
        let mut source = Prepared::with_index(db, snapshot, checkpoint, PARSER_REVISION, &index)?;
        source.observe_dependency(&roster);
        sources.push(source);
        expected.push(checkpoint);
    }
    if generations.available() && sources.iter().all(|s| s.unchanged) {
        return Ok(result);
    }
    let mut parsed = Vec::with_capacity(files.len());
    let mut checkpoints = Vec::with_capacity(files.len());
    let mut winners = BTreeMap::<String, (usize, u64, Value)>::new();
    let mut session_generations = BTreeMap::<String, u64>::new();
    let mut replicas = BTreeMap::<(String, u64), Value>::new();
    let mut events = 0usize;
    total_bytes = 0;
    for (i, (path, generation)) in files.iter().enumerate() {
        let read = read_dsh(path)?;
        total_bytes = total_bytes
            .checked_add(read.decoded_bytes)
            .ok_or_else(|| invalid("sourceBudgetExceeded"))?;
        events = events
            .checked_add(read.facts.len())
            .ok_or_else(|| invalid("eventBudgetExceeded"))?;
        if total_bytes > MAX_SCAN_BYTES || events > MAX_NATIVE_EVENTS {
            return Err(invalid("sourceBudgetExceeded"));
        }
        if read.supported_version {
            if let Some(sid) = &read.session_id {
                session_generations
                    .entry(sid.clone())
                    .and_modify(|g| *g = (*g).max(*generation))
                    .or_insert(*generation);
            }
        }
        for fact in &read.facts {
            let Some(native) = &fact.native_key else {
                continue;
            };
            let value = serde_json::to_value(fact).map_err(|_| invalid("invalidFact"))?;
            let key = (native.clone(), *generation);
            if replicas.get(&key).is_some_and(|prior| prior != &value) {
                return Err(invalid("conflictingNativeReplicas"));
            }
            replicas.insert(key, value.clone());
            if winners
                .get(native)
                .is_none_or(|(_, prior, _)| generation > prior)
            {
                winners.insert(native.clone(), (i, *generation, value));
            }
        }
        let modified = sources[i].snapshot.stamp["mtimeNs"]
            .as_str()
            .and_then(|s| s.parse::<u128>().ok())
            .and_then(|ns| i64::try_from(ns / 1_000_000_000).ok())
            .ok_or_else(|| invalid("invalidSourceStamp"))?;
        checkpoints.push(CheckpointChange {
            key: path.to_string_lossy().into_owned(),
            expected: expected[i],
            next: Some(SyncState {
                modified,
                offset: read.line_count,
            }),
        });
        parsed.push(read);
    }
    for read in &mut parsed {
        if read
            .parent_session_id
            .as_ref()
            .is_some_and(|p| !session_generations.contains_key(p))
        {
            read.reasons.insert("parentSourceUnavailable".into());
        }
    }
    commit_sync_batch(db, &checkpoints, |conn| {
        if dsh::usage_files(home).map_err(|_| invalid("sourceEnumerationFailed"))? != files {
            return Err(invalid("sourceRosterChanged"));
        }
        for source in &sources {
            source.check_on(conn)?;
        }
        generations.check_on(conn)?;
        let mut accepted = BTreeSet::new();
        let mut blocked = BTreeSet::new();
        let mut generation_reasons = BTreeSet::new();
        for (sid, generation) in &session_generations {
            if generations.available() && generations.session_allows(sid, *generation) {
                generations.observe_session(sid, *generation)?;
            } else {
                blocked.insert(sid.clone());
                generation_reasons.insert(sid.clone());
            }
        }
        for (native, (i, generation, value)) in &winners {
            let fact: NormalizedUsageFact =
                serde_json::from_value(value.clone()).map_err(|_| invalid("invalidFact"))?;
            let Some(sid) = fact.session_id.as_deref() else {
                result.skipped += 1;
                continue;
            };
            if blocked.contains(sid) || !generations.event_allows(sid, native, *generation) {
                generation_reasons.insert(sid.to_owned());
                result.skipped += 1;
                continue;
            }
            generations.observe_event(sid, native, *generation)?;
            let Some(at) = fact.at_ms else {
                result.skipped += 1;
                continue;
            };
            let row:Option<(String,String,Option<String>,Option<String>)>=conn.query_row("SELECT provider_id,app_type,data_source,session_id FROM proxy_request_logs WHERE request_id=?1",[&fact.request_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
            if row.as_ref().is_some_and(|(owner, app, origin, session)| {
                owner != "_dsh_session"
                    || app != "dsh"
                    || origin.as_deref() != Some("dsh_session")
                    || session.as_deref() != Some(sid)
            }) {
                return Err(invalid("foreignRecordIdentity"));
            }
            if !sources[*i].should_store_on(
                conn,
                &fact.request_id,
                native,
                "dsh_session",
                at.div_euclid(1000),
            )? {
                result.skipped += 1;
                if sources[*i].evidence.has(native) {
                    accepted.insert(native.clone());
                }
                continue;
            }
            let u = &fact.usage;
            let changed=conn.execute("INSERT INTO proxy_request_logs (request_id,provider_id,app_type,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,latency_ms,status_code,session_id,created_at,data_source)
                VALUES (?1,'_dsh_session','dsh',?2,?3,?4,?5,?6,0,0,?7,?8,'dsh_session')
                ON CONFLICT(request_id) DO UPDATE SET model=excluded.model,input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cache_read_tokens=excluded.cache_read_tokens,cache_creation_tokens=excluded.cache_creation_tokens,created_at=excluded.created_at
                WHERE model!=excluded.model OR input_tokens!=excluded.input_tokens OR output_tokens!=excluded.output_tokens OR cache_read_tokens!=excluded.cache_read_tokens OR cache_creation_tokens!=excluded.cache_creation_tokens OR created_at!=excluded.created_at",
                params![fact.request_id,fact.model,u.input_tokens,u.output_tokens,u.cache_read_tokens,u.cache_creation_tokens,sid,at.div_euclid(1000)])?;
            if changed > 0 {
                result.imported += 1;
            } else {
                result.skipped += 1;
            }
            accepted.insert(native.clone());
        }
        // Persist ordering before receipts capture the provider's final CAS
        // generation. A late checkpoint failure rolls back all four together.
        generations.save_on(conn)?;
        for (i, read) in parsed.iter().enumerate() {
            let source = &mut sources[i];
            if !generations.available()
                || read
                    .session_id
                    .as_ref()
                    .is_some_and(|sid| generation_reasons.contains(sid))
            {
                source.evidence.complete_event_identity_set = false;
                source.evidence.partial(if generations.available() {
                    "nativeGenerationRollback"
                } else {
                    "nativeGenerationLeaseUnavailable"
                });
            }
            for reason in &read.reasons {
                source.evidence.complete_event_identity_set = false;
                source.evidence.partial(reason);
            }
            for fact in &read.facts {
                if let Some(native) = &fact.native_key {
                    if accepted.contains(native) {
                        source.evidence.acknowledge(native);
                    }
                }
            }
            source.save_on(conn, checkpoints[i].next.unwrap())?;
        }
        Ok(())
    })?;
    index.advance_many(sources);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};

    const AT: i64 = 1_792_053_987_262;
    fn header(sid: &str) -> Value {
        json!({"type":"session","version":0,"id":sid,"createdAt":AT-1,"delegationDepth":0})
    }
    fn call(id: &str, seq: u64, at: i64, buckets: [u64; 4]) -> Value {
        json!({"type":"assistant/message","seq":seq,"time":at,"surfaceOp":"append",
            "data":{"message":{"id":id,"role":"assistant","source":{"kind":"model","provider":"fixture","model":"native-model"},"content":[{"type":"text","text":"private-dsh-body-not-indexed"}]},
                "usage":{"inputTokens":buckets[0],"outputTokens":buckets[1],"cacheReadTokens":buckets[2],"cacheWriteTokens":buckets[3]}}})
    }
    fn user(seq: u64) -> Value {
        json!({"type":"user/message","seq":seq,"time":AT,"surfaceOp":"append","data":{"id":"u1","role":"user","source":{"kind":"user"},"content":[{"type":"text","text":"synthetic prompt"}]}})
    }
    fn marker(seq: u64) -> Value {
        json!({"type":"session/end-seed","seq":seq,"time":AT+1,"data":{}})
    }
    fn parent() -> Vec<Value> {
        vec![
            header("base"),
            user(0),
            call("a1", 1, AT, [40, 10, 100, 20]),
        ]
    }
    fn child() -> Vec<Value> {
        let mut head = header("child");
        head["parentSession"] = json!("base");
        head["seedLength"] = json!(2);
        head["origin"] = json!("subagent");
        head["delegationDepth"] = json!(1);
        vec![
            head,
            user(0),
            call("a1", 1, AT, [40, 10, 100, 20]),
            marker(2),
            call("a2", 3, AT + 1, [20, 2, 10, 0]),
        ]
    }
    #[test]
    fn dsh_02_format_4_session_is_read_by_the_same_rules() {
        let home = tempfile::tempdir().unwrap();
        let mut head = header("v4");
        head["version"] = json!(4);
        head["cwd"] = json!("/fixture/project");
        head["isSeeded"] = json!(false);
        let rows = [
            head,
            json!({"type":"permission/preset","seq":0,"time":AT,"data":{}}),
            user(1),
            call("a1", 2, AT + 1, [7907, 3, 4096, 0]),
            json!({"type":"turn/end","seq":3,"time":AT+2,"data":{"turn":1}}),
        ];
        let read = read_dsh(&write(home.path(), "v4", "session.v4.jsonl.zstd", &rows)).unwrap();
        assert!(read.supported_version);
        assert!(read.reasons.is_empty(), "{:?}", read.reasons);
        assert_eq!(read.facts.len(), 1);
        assert_eq!(buckets(&read.facts[0]), [7907, 3, 4096, 0]);
        assert_eq!(
            read.metadata.unwrap().project,
            Some(Some("/fixture/project".into()))
        );
    }

    #[test]
    fn dsh_02_session_skipped_by_a_revision_1_parser_is_read_again() {
        let home = tempfile::tempdir().unwrap();
        let mut head = header("v4");
        head["version"] = json!(4);
        let rows = [head, user(0), call("a1", 1, AT, [7907, 3, 4096, 0])];
        write(home.path(), "v4", "session.v4.jsonl.zstd", &rows);
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        {
            // What a build without format 4 stored: no rows, an unchanged stamp.
            let conn = db.conn.lock().unwrap();
            conn.execute("DELETE FROM proxy_request_logs WHERE app_type='dsh'", [])
                .unwrap();
            let raw: String = conn
                .query_row(
                    "SELECT meta_json FROM session_meta_cache WHERE provider_id='dsh'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let mut meta: Value = serde_json::from_str(&raw).unwrap();
            let evidence = meta.pointer_mut("/parts/usageSync/value").unwrap();
            evidence["parserRevision"] = json!(1);
            evidence["coverage"] = json!("partial");
            evidence["completeEventIdentitySet"] = json!(false);
            evidence["reasons"] = json!(["legacyUnknownBoundary", "unsupportedSourceVersion"]);
            evidence["acknowledgedEventIds"] = json!([]);
            conn.execute(
                "UPDATE session_meta_cache SET meta_json=?1 WHERE provider_id='dsh'",
                [meta.to_string()],
            )
            .unwrap();
        }
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db), (1, 7907, 3, 4096, 0));
    }

    fn write(home: &Path, sid: &str, name: &str, rows: &[Value]) -> PathBuf {
        let path = home.join("sessions/_no-cwd").join(sid).join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = rows
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        if name.ends_with(".zstd") {
            let mut bytes = Vec::new();
            // Native persistence appends independent complete zstd frames.
            for line in text.split_inclusive('\n') {
                bytes.extend(zstd::stream::encode_all(line.as_bytes(), 1).unwrap());
            }
            std::fs::write(&path, bytes).unwrap();
        } else {
            std::fs::write(&path, text).unwrap();
        }
        path
    }
    fn buckets(fact: &NormalizedUsageFact) -> [u32; 4] {
        [
            fact.usage.input_tokens,
            fact.usage.output_tokens,
            fact.usage.cache_read_tokens,
            fact.usage.cache_creation_tokens,
        ]
    }
    fn totals(db: &Database) -> (i64, i64, i64, i64, i64) {
        db.conn.lock().unwrap().query_row("SELECT COUNT(*),COALESCE(SUM(input_tokens),0),COALESCE(SUM(output_tokens),0),COALESCE(SUM(cache_read_tokens),0),COALESCE(SUM(cache_creation_tokens),0) FROM proxy_request_logs WHERE app_type='dsh'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap()
    }
    fn dump(db: &Database) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
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
                .query_map([], |r| {
                    (0..columns)
                        .map(|i| r.get(i))
                        .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
                })
                .unwrap();
            rows.map(Result::unwrap).collect()
        })
        .collect()
    }

    #[test]
    fn dsh_native_v0_cache_buckets_are_disjoint_and_reasoning_is_not_added_again() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2]["data"]["usage"]["reasoningTokens"] = json!(7);
        let parsed = read_dsh(&write(home.path(), "base", "session.jsonl", &rows)).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(buckets(&parsed.facts[0]), [40, 10, 100, 20]);
        assert_eq!(parsed.facts[0].at_ms, Some(AT));
        assert_eq!(parsed.session_id.as_deref(), Some("base"));
        assert_eq!(parsed.facts[0].model, "native-model");
        assert!(parsed.reasons.is_empty());
        assert!(!serde_json::to_string(&parsed.facts)
            .unwrap()
            .contains("private-dsh-body"));
    }
    #[test]
    fn dsh_seeded_child_has_only_its_own_native_event_and_keeps_native_sid() {
        let home = tempfile::tempdir().unwrap();
        let parsed = read_dsh(&write(home.path(), "child", "session.jsonl", &child())).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(buckets(&parsed.facts[0]), [20, 2, 10, 0]);
        assert_eq!(parsed.facts[0].session_id.as_deref(), Some("child"));
        assert_eq!(parsed.parent_session_id.as_deref(), Some("base"));
    }
    #[test]
    fn dsh_later_resume_marker_does_not_erase_earlier_own_spend() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = child();
        rows.push(marker(4));
        rows.push(call("a3", 5, AT + 2, [30, 3, 0, 0]));
        let parsed = read_dsh(&write(home.path(), "child", "session.jsonl", &rows)).unwrap();
        assert_eq!(parsed.facts.len(), 2);
        assert_eq!(
            parsed
                .facts
                .iter()
                .map(|f| f.usage.input_tokens)
                .sum::<u32>(),
            50
        );
    }
    #[test]
    fn dsh_native_unseeded_child_is_not_declared_copied_only_because_it_has_a_parent() {
        let home = tempfile::tempdir().unwrap();
        let mut head = header("fresh-child");
        head["parentSession"] = json!("base");
        head["delegationDepth"] = json!(1);
        let parsed = read_dsh(&write(
            home.path(),
            "fresh-child",
            "session.jsonl",
            &[head, call("own", 0, AT, [20, 2, 10, 0])],
        ))
        .unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert!(!parsed.reasons.contains("seedBoundaryUnavailable"));
    }
    #[test]
    fn dsh_declared_seed_with_no_durable_cut_is_partial_instead_of_counting_copies() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = child();
        rows[0].as_object_mut().unwrap().remove("seedLength");
        rows[0]["isSeeded"] = json!(true);
        let parsed = read_dsh(&write(home.path(), "child", "session.jsonl", &rows)).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed.reasons.contains("seedBoundaryUnavailable"));
    }
    #[test]
    fn dsh_seed_cut_with_no_end_marker_is_explicitly_partial_and_never_imports_inherited_events() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = child();
        rows.remove(3);
        rows[3]["seq"] = json!(2);
        let parsed = read_dsh(&write(home.path(), "child", "session.jsonl", &rows)).unwrap();
        assert!(parsed.reasons.contains("seedBoundaryIncomplete"));
        assert!(!parsed
            .facts
            .iter()
            .any(|f| f.usage.message_id.as_deref() == Some("a1")));
    }
    #[test]
    fn dsh_unknown_format_version_is_partial_without_guessing_v2_marker_rules() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[0]["version"] = json!(2);
        let parsed = read_dsh(&write(home.path(), "base", "session.v2.jsonl", &rows)).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed.reasons.contains("unsupportedSourceVersion"));
    }
    #[test]
    fn dsh_model_source_is_required_instead_of_counting_plugin_projection_usage() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2]["data"]["message"]["source"]["kind"] = json!("plugin");
        let parsed = read_dsh(&write(home.path(), "base", "session.jsonl", &rows)).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed.reasons.contains("unsupportedUsageSource"));
    }
    #[test]
    fn dsh_compaction_usage_is_partial_and_a_replacement_summary_does_not_duplicate_a_call() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.push(json!({"type":"compaction/summary","seq":2,"time":AT+1,"data":{"compactionId":"c1","model":"summary-model","llmStreamCall":true,"usage":{"inputTokens":9,"outputTokens":3}}}));
        rows.push(json!({"type":"user/message","seq":3,"time":AT+1,"surfaceOp":{"op":"replace","start":0,"end":1},"sourceEventSeqs":[0,1,2],"data":{"id":"summary","role":"user","source":{"kind":"plugin"},"content":[{"type":"text","text":"synthetic summary"}]}}));
        let parsed = read_dsh(&write(home.path(), "base", "session.jsonl", &rows)).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(buckets(&parsed.facts[0]), [40, 10, 100, 20]);
        assert!(parsed.reasons.contains("unsupportedUsageEvent"));
    }
    #[test]
    fn dsh_native_commit_rule_ignores_even_valid_json_at_eof_without_newline() {
        let home = tempfile::tempdir().unwrap();
        let rows = parent();
        let path = write(home.path(), "base", "session.jsonl", &rows);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.pop();
        std::fs::write(&path, bytes).unwrap();
        let parsed = read_dsh(&path).unwrap();
        assert!(parsed.facts.is_empty());
        assert_eq!(parsed.line_count, 2);
        assert!(parsed.reasons.contains("incompleteEnumeration"));
    }
    #[test]
    fn dsh_unfinished_tail_preserves_only_complete_native_events() {
        let home = tempfile::tempdir().unwrap();
        let path = write(home.path(), "base", "session.jsonl", &parent());
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"type\":\"assistant/message\"")
            .unwrap();
        let parsed = read_dsh(&path).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(parsed.line_count, 3);
        assert!(parsed.reasons.contains("incompleteEnumeration"));
    }
    #[test]
    fn dsh_native_time_is_ms_without_threshold_or_created_at_fallback() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2]["time"] = json!(1000);
        let path = write(home.path(), "base", "session.jsonl", &rows);
        assert_eq!(read_dsh(&path).unwrap().facts[0].at_ms, Some(1000));
        rows[2].as_object_mut().unwrap().remove("time");
        let parsed = read_dsh(&write(home.path(), "base", "session.jsonl", &rows)).unwrap();
        assert_eq!(parsed.facts[0].at_ms, None);
        assert!(parsed.reasons.contains("missingTimestamp"));
    }
    #[test]
    fn dsh_missing_native_message_id_remains_partial_not_an_ordinal_ack() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2]["data"]["message"]
            .as_object_mut()
            .unwrap()
            .remove("id");
        let parsed = read_dsh(&write(home.path(), "base", "session.jsonl", &rows)).unwrap();
        assert!(parsed.facts[0].native_key.is_none());
        assert!(parsed.reasons.contains("missingNativeEventIdentity"));
    }
    #[test]
    fn dsh_zero_usage_is_a_native_record_and_explicit_zero_does_not_fall_back() {
        let home = tempfile::tempdir().unwrap();
        let rows = vec![header("base"), call("zero", 0, AT, [0, 0, 0, 0])];
        let parsed = read_dsh(&write(home.path(), "base", "session.jsonl", &rows)).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(buckets(&parsed.facts[0]), [0, 0, 0, 0]);
    }
    #[test]
    fn dsh_invalid_token_integers_are_never_silently_zeroed() {
        for value in [json!(-1), json!(1.5), json!(u64::MAX)] {
            let home = tempfile::tempdir().unwrap();
            let mut rows = parent();
            rows[2]["data"]["usage"]["inputTokens"] = value;
            assert!(read_dsh(&write(home.path(), "base", "session.jsonl", &rows)).is_err());
        }
    }
    #[test]
    fn dsh_nonconsecutive_same_native_id_revision_replaces_the_live_fact() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows.push(call("other", 2, AT + 1, [2, 1, 0, 0]));
        let mut revised = call("a1", 3, AT + 86_400_000, [80, 20, 200, 40]);
        revised["data"]["message"]["source"]["model"] = json!("revised-model");
        rows.push(revised);
        let parsed = read_dsh(&write(home.path(), "base", "session.jsonl", &rows)).unwrap();
        assert_eq!(parsed.facts.len(), 2);
        let fact = parsed
            .facts
            .iter()
            .find(|f| f.model == "revised-model")
            .unwrap();
        assert_eq!(buckets(fact), [80, 20, 200, 40]);
        assert_eq!(fact.at_ms, Some(AT + 86_400_000));
    }
    #[test]
    fn dsh_recorded_ofox_provider_name_is_not_used_to_discard_historical_usage() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2]["data"]["message"]["source"]["provider"] = json!("ofox-dsh-responses");
        assert_eq!(
            read_dsh(&write(home.path(), "base", "session.jsonl", &rows))
                .unwrap()
                .facts
                .len(),
            1
        );
    }
    #[test]
    fn dsh_native_event_identity_is_scoped_to_native_session_and_survives_a_move() {
        let home = tempfile::tempdir().unwrap();
        let path = write(home.path(), "base", "session.jsonl", &parent());
        let before = read_dsh(&path).unwrap();
        let moved = path.with_file_name("session.v1.jsonl");
        std::fs::rename(&path, &moved).unwrap();
        let after = read_dsh(&moved).unwrap();
        assert_eq!(before.facts[0].native_key, after.facts[0].native_key);
        let mut other = parent();
        other[0]["id"] = json!("other");
        let separate = read_dsh(&write(home.path(), "other", "session.jsonl", &other)).unwrap();
        assert_ne!(before.facts[0].native_key, separate.facts[0].native_key);
    }
    #[test]
    fn dsh_plaintext_and_concatenated_zstd_frames_have_identical_native_facts() {
        let home = tempfile::tempdir().unwrap();
        let plain = write(home.path(), "base", "session.jsonl", &parent());
        let packed = write(home.path(), "base", "session.jsonl.zstd", &parent());
        assert_eq!(
            serde_json::to_value(read_dsh(&plain).unwrap().facts).unwrap(),
            serde_json::to_value(read_dsh(&packed).unwrap().facts).unwrap()
        );
    }
    #[test]
    fn dsh_packed_delta_rows_do_not_add_spend_or_hide_the_final_assistant() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2]["seq"] = json!(5);
        rows.insert(2,json!({"type":"text-chunks","seq0":1,"time0":AT,"data":{"turn":0,"step":0,"index":0,"dt":[1,1,1],"texts":["synthetic-a","synthetic-b","synthetic-c","synthetic-d"]}}));
        let parsed = read_dsh(&write(home.path(), "base", "session.jsonl", &rows)).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(buckets(&parsed.facts[0]), [40, 10, 100, 20]);
        assert!(parsed.reasons.is_empty());
    }

    #[test]
    fn dsh_parent_and_child_spend_are_counted_once_under_their_native_sessions() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        write(home.path(), "child", "session.jsonl.zstd", &child());
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db), (2, 60, 12, 110, 20));
        let conn = db.conn.lock().unwrap();
        let mut statement = conn
            .prepare("SELECT session_id FROM proxy_request_logs ORDER BY session_id")
            .unwrap();
        assert_eq!(
            statement
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>(),
            ["base", "child"]
        );
    }
    #[test]
    fn dsh_child_without_its_parent_never_recovers_the_inherited_copy_as_new_spend() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "child", "session.jsonl", &child());
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db), (1, 20, 2, 10, 0));
        let conn = db.conn.lock().unwrap();
        let parts = crate::session_manager::source_cache::read_on(
            &conn,
            home.path()
                .join("sessions/_no-cwd/child/session.jsonl")
                .to_str()
                .unwrap(),
            "dsh",
        )
        .unwrap();
        assert!(parts
            .value(crate::session_manager::source_cache::Part::UsageSync)
            .unwrap()["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "parentSourceUnavailable"));
    }
    #[test]
    fn dsh_generations_keep_all_spend_and_higher_generation_replaces_same_native_event() {
        let home = tempfile::tempdir().unwrap();
        let mut older = parent();
        older.push(call("old-only", 2, AT + 1, [3, 1, 0, 0]));
        write(home.path(), "base", "session.jsonl", &older);
        let mut newer = parent();
        newer[2]["data"]["usage"]["inputTokens"] = json!(80);
        newer[2]["time"] = json!(AT + 86_400_000);
        newer[2]["data"]["message"]["source"]["model"] = json!("revised-model");
        write(home.path(), "base", "session.v1.jsonl.zstd", &newer);
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db), (2, 83, 11, 100, 20));
        let conn = db.conn.lock().unwrap();
        assert_eq!(
            conn.query_row::<String, _, _>(
                "SELECT model FROM proxy_request_logs WHERE input_tokens=80",
                [],
                |r| r.get(0)
            )
            .unwrap(),
            "revised-model"
        );
    }
    #[test]
    fn dsh_same_generation_compressed_twins_are_not_counted_twice() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        write(home.path(), "base", "session.jsonl.zstd", &parent());
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db), (1, 40, 10, 100, 20));
    }
    #[test]
    fn dsh_conflicting_same_generation_twins_fail_without_selecting_an_mtime_winner() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        let mut conflict = parent();
        conflict[2]["data"]["usage"]["inputTokens"] = json!(999);
        write(home.path(), "base", "session.jsonl.zstd", &conflict);
        let db = Database::memory().unwrap();
        assert!(sync_dsh_usage_at(&db, home.path()).is_err());
        assert_eq!(totals(&db), (0, 0, 0, 0, 0));
    }
    #[test]
    fn dsh_compressed_read_failure_keeps_last_successful_ledger_checkpoint_and_proof() {
        let home = tempfile::tempdir().unwrap();
        let path = write(home.path(), "base", "session.jsonl.zstd", &parent());
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        let before = dump(&db);
        std::fs::write(&path, b"invalid-zstd-frame").unwrap();
        assert!(sync_dsh_usage_at(&db, home.path()).is_err());
        assert_eq!(dump(&db), before);
    }
    #[test]
    fn dsh_missing_time_or_native_identity_is_not_inserted_or_acknowledged() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        let mut untimed = call("untimed", 2, AT, [2, 1, 0, 0]);
        untimed.as_object_mut().unwrap().remove("time");
        rows.push(untimed);
        let mut unidentified = call("", 3, AT, [3, 1, 0, 0]);
        unidentified["data"]["message"]
            .as_object_mut()
            .unwrap()
            .remove("id");
        rows.push(unidentified);
        write(home.path(), "base", "session.jsonl", &rows);
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db), (1, 40, 10, 100, 20));
        let conn = db.conn.lock().unwrap();
        let parts = crate::session_manager::source_cache::read_on(
            &conn,
            home.path()
                .join("sessions/_no-cwd/base/session.jsonl")
                .to_str()
                .unwrap(),
            "dsh",
        )
        .unwrap();
        let evidence = parts
            .value(crate::session_manager::source_cache::Part::UsageSync)
            .unwrap();
        assert_eq!(
            evidence["acknowledgedEventIds"].as_array().unwrap().len(),
            1,
            "unknown time/identity cannot acquire a native ack"
        );
        assert!(!format!("{evidence:?}").contains("private-dsh-body"));
    }
    #[test]
    fn dsh_late_checkpoint_failure_rolls_back_the_entire_source_roster() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        write(home.path(), "child", "session.jsonl", &child());
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        let before = dump(&db);
        let mut rows = parent();
        rows.push(call("new", 2, AT + 1, [1, 1, 0, 0]));
        write(home.path(), "base", "session.jsonl", &rows);
        {
            let conn = db.conn.lock().unwrap();
            let child = home.path().join("sessions/_no-cwd/child/session.jsonl");
            let literal: String = conn
                .query_row("SELECT quote(?1)", [child.to_str().unwrap()], |r| r.get(0))
                .unwrap();
            conn.execute_batch(&format!("CREATE TEMP TRIGGER fail_dsh_child_checkpoint BEFORE INSERT ON session_log_sync WHEN NEW.file_path={literal} BEGIN SELECT RAISE(ABORT,'fixture checkpoint failure'); END;")).unwrap();
        }
        assert!(sync_dsh_usage_at(&db, home.path()).is_err());
        assert_eq!(dump(&db), before);
    }
    #[test]
    fn dsh_unchanged_roster_does_not_read_source_bodies_again() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        let before = super::super::usage_source::body_reads_under(home.path());
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(
            super::super::usage_source::body_reads_under(home.path()),
            before
        );
    }
    #[test]
    fn dsh_pruned_native_zero_gap_fill_and_file_move_keep_exact_positive_identity() {
        let home = tempfile::tempdir().unwrap();
        let old = 1_577_836_800_123;
        let mut rows = vec![
            header("base"),
            call("zero", 0, old, [0, 0, 0, 0]),
            call("a", 1, old, [40, 10, 100, 20]),
        ];
        rows[0]["createdAt"] = json!(old - 1);
        let path = write(home.path(), "base", "session.jsonl", &rows);
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(db.rollup_and_prune(30).unwrap(), 2);
        rows.push(call("old-gap", 2, old + 1, [20, 2, 10, 0]));
        write(home.path(), "base", "session.jsonl", &rows);
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(db.rollup_and_prune(30).unwrap(), 1);
        let moved = home.path().join("sessions/moved/base/session.v1.jsonl");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::rename(path, moved).unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db), (0, 0, 0, 0, 0));
        let conn = db.conn.lock().unwrap();
        let totals:(i64,i64,i64)=conn.query_row("SELECT SUM(request_count),SUM(input_tokens),SUM(output_tokens) FROM usage_daily_rollups WHERE app_type='dsh'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(totals, (3, 60, 12));
        let manifest = crate::session_manager::source_cache::provider_on(&conn, "dsh")
            .unwrap()
            .unwrap();
        let native = &manifest["nativeGenerations"]["sessions"];
        assert_eq!(native.as_object().unwrap().len(), 1);
        let ordered = native.as_object().unwrap().values().next().unwrap();
        assert_eq!(ordered["generation"], 1);
        assert_eq!(ordered["events"].as_object().unwrap().len(), 3);
        assert!(
            ordered["events"]
                .as_object()
                .unwrap()
                .values()
                .all(|g| *g == json!(1)),
            "observed higher ordering is positive evidence even when every fact was already pruned"
        );
    }
    #[test]
    fn dsh_normalized_api_preserves_compressed_native_ms_and_partial_reasons() {
        let home = tempfile::tempdir().unwrap();
        let path = write(home.path(), "base", "session.jsonl.zstd", &parent());
        let normalized = super::super::usage_source::read_normalized("dsh", &path).unwrap();
        assert!(normalized.complete);
        assert_eq!(normalized.facts[0].at_ms, Some(AT));
        assert_eq!(buckets(&normalized.facts[0]), [40, 10, 100, 20]);
    }
    #[test]
    fn dsh_current_price_estimate_uses_the_shared_scientific_four_bucket_catalog() {
        let home = tempfile::tempdir().unwrap();
        let parsed = read_dsh(&write(home.path(), "base", "session.jsonl", &parent())).unwrap();
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let catalog=super::super::ofox_pricing::PriceCatalog::ingest_on(&conn,"ofox.ai",&json!({"data":[{"id":"native-model","pricing":{"prompt":"1e-9","completion":"2e-9","input_cache_read":"3e-9","input_cache_write":"4e-9"}}]}),1).unwrap();
        assert_eq!(
            catalog
                .estimate(
                    "native-model",
                    buckets(&parsed.facts[0]).map(u64::from),
                    Some(1)
                )
                .known_cost_usd
                .as_deref(),
            Some("0.00000044")
        );
    }
    #[test]
    #[serial_test::serial]
    fn dsh_runtime_sync_respects_the_existing_native_dsh_home_resolver() {
        let sandbox = crate::test_sandbox::sandbox();
        let home = sandbox.home().join("native-dsh");
        std::env::set_var("DSH_HOME", &home);
        write(&home, "base", "session.jsonl", &parent());
        let db = Database::memory().unwrap();
        sync_dsh_usage(&db).unwrap();
        assert_eq!(totals(&db), (1, 40, 10, 100, 20));
    }
    #[test]
    fn dsh_generation_lease_prevents_downgrade_after_the_highest_file_disappears() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        let mut newer = parent();
        newer[2]["data"]["usage"]["inputTokens"] = json!(80);
        let high = write(home.path(), "base", "session.v1.jsonl", &newer);
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db).1, 80);
        std::fs::remove_file(high).unwrap();
        let result = sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(result.imported, 0);
        assert_eq!(
            totals(&db).1,
            80,
            "a remaining old generation must not downgrade counted spend"
        );
        assert!(dsh_proof_reasons(&db).contains("nativeGenerationRollback"));
    }
    #[test]
    fn dsh_event_generation_lease_survives_a_higher_projection_omitting_the_event() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        let mut newer = parent();
        newer[2]["data"]["usage"]["inputTokens"] = json!(80);
        write(home.path(), "base", "session.v1.jsonl", &newer);
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        write(
            home.path(),
            "base",
            "session.v1.jsonl",
            &[header("base"), call("other", 0, AT + 1, [3, 1, 0, 0])],
        );
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db), (2, 83, 11, 100, 20));
        assert!(dsh_proof_reasons(&db).contains("nativeGenerationRollback"));
    }
    #[test]
    fn dsh_missing_generation_lease_preserves_prior_owned_facts_as_partial() {
        check_unavailable_generation_lease(false);
    }
    #[test]
    fn dsh_corrupt_generation_lease_preserves_prior_owned_facts_as_partial() {
        check_unavailable_generation_lease(true);
    }
    fn check_unavailable_generation_lease(corrupt: bool) {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let raw:String=conn.query_row("SELECT value FROM maintenance_flags WHERE key='usage_provider_evidence_v1:dsh'",[],|r|r.get(0)).unwrap();
            let mut manifest: Value = serde_json::from_str(&raw).unwrap();
            if corrupt {
                manifest["nativeGenerations"] = json!({"version":77,"sessions":{}});
            } else {
                manifest
                    .as_object_mut()
                    .unwrap()
                    .remove("nativeGenerations");
            }
            conn.execute(
                "UPDATE maintenance_flags SET value=?1 WHERE key='usage_provider_evidence_v1:dsh'",
                [manifest.to_string()],
            )
            .unwrap();
        }
        let mut revision = parent();
        revision[2]["data"]["usage"]["inputTokens"] = json!(999);
        write(home.path(), "base", "session.jsonl", &revision);
        let result = sync_dsh_usage_at(&db, home.path()).unwrap();
        assert!(
            result.errors.is_empty(),
            "typed partial coverage is not transport failure"
        );
        assert_eq!(totals(&db).1, 40);
        assert!(dsh_proof_reasons(&db).contains("nativeGenerationLeaseUnavailable"));
    }
    #[test]
    fn dsh_higher_unsupported_header_version_cannot_authorize_a_revision() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        let mut unknown = parent();
        unknown[0]["version"] = json!(2);
        unknown[2]["data"]["usage"]["inputTokens"] = json!(999);
        write(home.path(), "base", "session.v9.jsonl", &unknown);
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db).1, 40);
        assert!(dsh_proof_reasons(&db).contains("unsupportedSourceVersion"));
    }
    fn dsh_proof_reasons(db: &Database) -> BTreeSet<String> {
        let conn = db.conn.lock().unwrap();
        let mut statement = conn
            .prepare("SELECT meta_json FROM session_meta_cache WHERE provider_id='dsh'")
            .unwrap();
        let rows = statement.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(Result::unwrap)
            .flat_map(|raw| {
                let value: Value = serde_json::from_str(&raw).unwrap();
                value
                    .pointer("/parts/usageSync/value/reasons")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn dsh_capabilities_expose_verified_usage_and_partial_session_project_analytics() {
        let cap = super::super::usage_query::source_capabilities()
            .into_iter()
            .find(|c| c.tool_id == "dsh")
            .unwrap();
        assert!(cap.usage_supported);
        assert_eq!(cap.source_ids, ["dsh_session"]);
        assert!(cap.session_analytics_supported);
        assert!(cap.project_filter_supported);
    }
    #[test]
    fn dsh_new_provider_can_record_recent_native_facts_after_an_unrelated_global_epoch() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO maintenance_flags(key,value) VALUES('usage_ledger_epoch_v1','unrelated-prior-cache-reset')",[]).unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db), (1, 40, 10, 100, 20));
        let conn = db.conn.lock().unwrap();
        let manifest = crate::session_manager::source_cache::provider_on(&conn, "dsh")
            .unwrap()
            .unwrap();
        assert_eq!(
            manifest["complete"], false,
            "new ordering evidence must not upgrade historical negative proof"
        );
        assert!(manifest["nativeGenerations"]["sessions"].is_object());
    }
    #[test]
    fn dsh_invalid_v0_header_cannot_establish_a_trusted_generation() {
        for (field, bad) in [
            ("createdAt", json!("yesterday")),
            ("delegationDepth", json!(-1)),
            ("approvalPolicy", json!("retired")),
        ] {
            let home = tempfile::tempdir().unwrap();
            let mut rows = parent();
            rows[0][field] = bad;
            let read = read_dsh(&write(home.path(), "base", "session.v9.jsonl", &rows)).unwrap();
            assert!(
                !read.supported_version,
                "native v0 validation must reject {field}"
            );
            assert!(read.facts.is_empty());
            assert!(read.reasons.contains("unsupportedSourceHeader"));
        }
    }
    #[test]
    fn dsh_malformed_packed_row_keeps_only_the_native_contiguous_prefix() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = parent();
        rows[2]["seq"] = json!(5);
        rows.insert(2,json!({"type":"text-chunks","seq0":1,"time0":AT,"data":{"turn":0,"step":0,"index":0,"dt":[],"texts":["a","b","c","d"]}}));
        let read = read_dsh(&write(home.path(), "base", "session.jsonl", &rows)).unwrap();
        assert!(
            read.facts.is_empty(),
            "native decoder rejects arity mismatch before the assistant call"
        );
        assert!(read.reasons.contains("incompleteEnumeration"));
        assert_eq!(read.line_count, 2);
    }
    #[test]
    fn dsh_valid_json_lease_missing_sessions_is_unknown_instead_of_generation_zero() {
        check_shape_valid_ordering_loss(false);
    }
    #[test]
    fn dsh_valid_json_lease_lowered_generations_cannot_downgrade_counted_spend() {
        check_shape_valid_ordering_loss(true);
    }
    fn check_shape_valid_ordering_loss(lower: bool) {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "base", "session.jsonl", &parent());
        let mut newer = parent();
        newer[2]["data"]["usage"]["inputTokens"] = json!(80);
        let high = write(home.path(), "base", "session.v1.jsonl", &newer);
        let db = Database::memory().unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(totals(&db).1, 80);
        {
            let conn = db.conn.lock().unwrap();
            let mut manifest = crate::session_manager::source_cache::provider_on(&conn, "dsh")
                .unwrap()
                .unwrap();
            if lower {
                for session in manifest["nativeGenerations"]["sessions"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                {
                    session["generation"] = json!(0);
                    for event in session["events"].as_object_mut().unwrap().values_mut() {
                        *event = json!(0);
                    }
                }
            } else {
                manifest["nativeGenerations"]["sessions"] = json!({});
            }
            conn.execute(
                "UPDATE maintenance_flags SET value=?1 WHERE key='usage_provider_evidence_v1:dsh'",
                [manifest.to_string()],
            )
            .unwrap();
        }
        std::fs::remove_file(high).unwrap();
        sync_dsh_usage_at(&db, home.path()).unwrap();
        assert_eq!(
            totals(&db).1,
            80,
            "structurally valid cache loss must not reset native ordering to zero"
        );
        assert!(dsh_proof_reasons(&db).contains("nativeGenerationLeaseUnavailable"));
    }
    #[test]
    fn dsh_seeded_prefix_already_ending_in_a_native_marker_has_a_complete_cut() {
        let home = tempfile::tempdir().unwrap();
        let mut rows = child();
        rows[0]["seedLength"] = json!(3);
        // Native Session construction reuses this final inherited marker and
        // does not append another marker at the new child's cut.
        let read = read_dsh(&write(home.path(), "child", "session.jsonl", &rows)).unwrap();
        assert_eq!(read.facts.len(), 1);
        assert_eq!(buckets(&read.facts[0]), [20, 2, 10, 0]);
        assert!(
            read.reasons.is_empty(),
            "a terminal inherited native marker establishes the same header cut"
        );
        rows.push(marker(4));
        rows.push(call("later", 5, AT + 2, [3, 1, 0, 0]));
        let resumed = read_dsh(&write(home.path(), "child", "session.jsonl", &rows)).unwrap();
        assert_eq!(resumed.facts.len(), 2);
        assert!(resumed.reasons.is_empty());
    }
}
