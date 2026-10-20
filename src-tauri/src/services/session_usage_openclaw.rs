//! OpenClaw 2026.10.3 native spend facts. No context-total or routing inference.
use crate::{
    error::AppError,
    proxy::usage::parser::TokenUsage,
    services::usage_source::{self, NormalizedUsageFact},
};
use rusqlite::{types::ValueRef, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_JSONL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SCAN_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 128 * 1024 * 1024;
const MAX_EVENTS: usize = 100_000;
const MAX_SESSIONS: usize = 20_000;
fn invalid(code: &str) -> AppError {
    AppError::InvalidInput(format!("usage.openclaw.{code}"))
}
fn identifier(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= 512)
        .map(str::to_owned)
}
fn event_identifier(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 512)
        .map(str::to_owned)
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Database,
    Legacy,
}
pub(crate) fn location(path: &Path) -> Result<(String, Kind), AppError> {
    let directory = path
        .parent()
        .ok_or_else(|| invalid("invalidSourceLayout"))?;
    let kind = match directory.file_name().and_then(|v| v.to_str()) {
        Some("agent")
            if path
                .file_name()
                .is_some_and(|f| f == "openclaw-agent.sqlite") =>
        {
            Kind::Database
        }
        Some("sessions") if path.extension().is_some_and(|f| f == "jsonl") => Kind::Legacy,
        _ => return Err(invalid("invalidSourceLayout")),
    };
    let agent = directory
        .parent()
        .ok_or_else(|| invalid("invalidSourceLayout"))?;
    if agent
        .parent()
        .and_then(Path::file_name)
        .is_none_or(|f| f != "agents")
    {
        return Err(invalid("invalidSourceLayout"));
    }
    let name = agent
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|s| !s.trim().is_empty() && s.len() <= 512)
        .ok_or_else(|| invalid("invalidAgentIdentity"))?;
    Ok((name.to_owned(), kind))
}
fn scoped_session(agent: &str, sid: &str) -> String {
    json!([agent, sid]).to_string()
}

pub(crate) struct OpenClawRead {
    pub facts: Vec<NormalizedUsageFact>,
    pub event_count: i64,
    pub reasons: BTreeSet<String>,
    pub sessions: BTreeSet<String>,
    pub projects: BTreeMap<String, Option<Option<String>>>,
    pub blocked_sessions: BTreeSet<String>,
    pub decoded_bytes: usize,
    pub native_events: BTreeMap<String, String>,
}
impl OpenClawRead {
    pub(crate) fn suppressed(sid: Option<String>, decoded_bytes: usize, reason: &str) -> Self {
        Self {
            facts: Vec::new(),
            event_count: 0,
            reasons: BTreeSet::from([reason.to_owned()]),
            sessions: sid.into_iter().collect(),
            projects: BTreeMap::new(),
            blocked_sessions: BTreeSet::new(),
            decoded_bytes,
            native_events: BTreeMap::new(),
        }
    }
}
struct Call {
    sid: Option<String>,
    id: Option<String>,
    ordinal: i64,
    model: String,
    at_ms: Option<i64>,
    usage: TokenUsage,
}
struct Reader {
    path: PathBuf,
    agent: String,
    jsonl_session: Option<String>,
    calls: Vec<Call>,
    native_events: Vec<(Option<String>, String, String)>,
    sessions: BTreeSet<String>,
    projects: BTreeMap<String, Option<Option<String>>>,
    conflicting_projects: BTreeSet<String>,
    blocked: BTreeSet<String>,
    reasons: BTreeSet<String>,
    count: i64,
}
fn native_time(value: Option<&Value>) -> Option<i64> {
    let v = value?;
    let ms = if let Some(n) = v.as_i64() {
        n
    } else {
        chrono::DateTime::parse_from_rfc3339(v.as_str()?)
            .ok()?
            .timestamp_millis()
    };
    chrono::DateTime::from_timestamp_millis(ms).map(|_| ms)
}
fn bucket(value: Option<&Value>) -> Result<Option<u64>, AppError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| invalid("invalidTokenInteger")),
    }
}
fn usage(value: &Value, reasons: &mut BTreeSet<String>) -> Result<Option<TokenUsage>, AppError> {
    if !value.is_object() {
        return Err(invalid("invalidUsageShape"));
    }
    // Only the verified normalized and Responses shapes; never guess aliases.
    let canonical = value.get("input").is_some() || value.get("output").is_some();
    let counts = if canonical {
        [
            bucket(value.get("input"))?,
            bucket(value.get("output"))?,
            bucket(value.get("cacheRead"))?,
            bucket(value.get("cacheWrite"))?,
        ]
    } else {
        let input = bucket(value.get("input_tokens"))?;
        let output = bucket(value.get("output_tokens"))?;
        let read = bucket(value.pointer("/input_tokens_details/cached_tokens"))?;
        let write = bucket(value.pointer("/input_tokens_details/cache_write_tokens"))?;
        let new = match (input, read, write) {
            (Some(i), Some(r), Some(w)) => Some(
                i.checked_sub(r.checked_add(w).ok_or_else(|| invalid("tokenOverflow"))?)
                    .ok_or_else(|| invalid("inconsistentCacheBuckets"))?,
            ),
            _ => None,
        };
        [new, output, read, write]
    };
    let [Some(input), Some(output), Some(read), Some(write)] = counts else {
        reasons.insert("missingUsageBuckets".into());
        return Ok(None);
    };
    Ok(Some(TokenUsage {
        input_tokens: usage_source::checked_token(input)?,
        output_tokens: usage_source::checked_token(output)?,
        cache_read_tokens: usage_source::checked_token(read)?,
        cache_creation_tokens: usage_source::checked_token(write)?,
        cache_creation_1h_tokens: 0,
        model: None,
        message_id: None,
    }))
}
impl Reader {
    fn new(path: &Path, agent: String) -> Self {
        Self {
            path: path.to_owned(),
            agent,
            jsonl_session: None,
            calls: Vec::new(),
            native_events: Vec::new(),
            sessions: BTreeSet::new(),
            projects: BTreeMap::new(),
            conflicting_projects: BTreeSet::new(),
            blocked: BTreeSet::new(),
            reasons: BTreeSet::new(),
            count: 0,
        }
    }
    fn accept(&mut self, value: Value, sid: Option<&str>, ordinal: i64) -> Result<(), AppError> {
        if let Some(id) = event_identifier(value.get("id")) {
            let digest = format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&value).map_err(|_| invalid("invalidNativeEvent"))?
                )
            );
            self.native_events
                .push((sid.map(str::to_owned), id, digest));
        }
        if value.get("type").and_then(Value::as_str) == Some("session") {
            if !matches!(value.get("version").and_then(Value::as_u64), Some(3 | 4)) {
                return Err(invalid("unsupportedTranscriptVersion"));
            }
            if let Some(id) = identifier(value.get("id")) {
                if sid.is_some_and(|s| s != id)
                    || (sid.is_none() && self.jsonl_session.as_ref().is_some_and(|s| s != &id))
                {
                    return Err(invalid("conflictingSessionIdentity"));
                }
                self.sessions.insert(id.clone());
                let project = match value.get("cwd") {
                    Some(Value::String(path)) if !path.trim().is_empty() => {
                        Some(Some(path.clone()))
                    }
                    Some(Value::String(_) | Value::Null) => Some(None),
                    _ => None,
                };
                if self
                    .projects
                    .get(&id)
                    .is_some_and(|prior| prior != &project)
                {
                    self.conflicting_projects.insert(id.clone());
                }
                self.projects.insert(
                    id.clone(),
                    if self.conflicting_projects.contains(&id) {
                        None
                    } else {
                        project
                    },
                );
                if value.get("parentSession").is_some_and(|v| !v.is_null()) {
                    self.blocked.insert(id.clone());
                    self.reasons.insert("forkProvenanceUnverified".into());
                }
                if sid.is_none() {
                    self.jsonl_session = Some(id);
                }
            }
            return Ok(());
        }
        let Some(message) = value.get("message") else {
            return Ok(());
        };
        let Some(raw) = message.get("usage").filter(|v| !v.is_null()) else {
            return Ok(());
        };
        if value.get("type").and_then(Value::as_str) != Some("message")
            || message.get("role").and_then(Value::as_str) != Some("assistant")
        {
            self.reasons.insert("nonAssistantUsageUnsupported".into());
            return Ok(());
        }
        let Some(mut usage) = usage(raw, &mut self.reasons)? else {
            return Ok(());
        };
        if self.calls.len() >= MAX_EVENTS {
            return Err(invalid("eventBudgetExceeded"));
        }
        let id = event_identifier(value.get("id"));
        let model = identifier(message.get("model")).unwrap_or_else(|| "unknown".into());
        usage.model = Some(model.clone());
        usage.message_id = id.clone();
        self.calls.push(Call {
            sid: sid.map(str::to_owned),
            id,
            ordinal,
            model,
            usage,
            at_ms: native_time(value.get("timestamp"))
                .or_else(|| native_time(message.get("timestamp"))),
        });
        Ok(())
    }
    fn finish(mut self) -> Result<OpenClawRead, AppError> {
        let mut facts = BTreeMap::new();
        for call in self.calls {
            let sid = call.sid.or_else(|| self.jsonl_session.clone());
            let scope = sid.as_deref().map(|s| scoped_session(&self.agent, s));
            let native = scope
                .as_deref()
                .zip(call.id.as_deref())
                .map(|(s, id)| usage_source::native_key("openclaw", s, id));
            let request_id = match sid.as_deref().zip(call.id.as_deref()) {
                Some((sid, id)) => format!("openclaw_session:{}", json!([self.agent, sid, id])),
                None => format!(
                    "openclaw_recovered:{:x}",
                    Sha256::digest(
                        json!([self.path.to_string_lossy(), call.ordinal])
                            .to_string()
                            .as_bytes()
                    )
                ),
            };
            let fact = NormalizedUsageFact {
                request_id: request_id.clone(),
                native_key: native,
                session_id: scope,
                model: call.model,
                usage: call.usage,
                at_ms: call.at_ms,
            };
            // Identical duplicate events represent one native call. Full native
            // payload conflicts are checked independently below, for every ID.
            facts.entry(request_id).or_insert(fact);
        }
        let mut native_events = BTreeMap::new();
        for (sid, id, digest) in self.native_events {
            if let Some(sid) = sid.or_else(|| self.jsonl_session.clone()) {
                let native =
                    usage_source::native_key("openclaw", &scoped_session(&self.agent, &sid), &id);
                if native_events
                    .get(&native)
                    .is_some_and(|prior| prior != &digest)
                {
                    return Err(invalid("conflictingNativeEvents"));
                }
                native_events.insert(native, digest);
            }
        }
        let facts = facts.into_values().collect::<Vec<_>>();
        if facts.iter().any(|f| f.native_key.is_none()) {
            self.reasons.insert("missingNativeEventIdentity".into());
        }
        if self.sessions.is_empty() {
            self.reasons.insert("missingNativeSessionIdentity".into());
        }
        if facts.iter().any(|f| f.at_ms.is_none()) {
            self.reasons.insert("missingTimestamp".into());
        }
        if facts.iter().any(|f| f.model == "unknown") {
            self.reasons.insert("missingModel".into());
        }
        Ok(OpenClawRead {
            facts,
            event_count: self.count,
            reasons: self.reasons,
            sessions: self.sessions,
            projects: self.projects,
            blocked_sessions: self.blocked,
            decoded_bytes: 0,
            native_events,
        })
    }
}
fn read_jsonl(path: &Path, agent: String) -> Result<OpenClawRead, AppError> {
    usage_source::with_stable_regular_file(path, |file| {
        let mut bytes = Vec::new();
        file.take(MAX_JSONL_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid("sourceUnreadable"))?;
        if bytes.len() as u64 > MAX_JSONL_BYTES {
            return Err(invalid("sourceTooLarge"));
        }
        let mut reader = Reader::new(path, agent);
        for line in bytes.split_inclusive(|b| *b == b'\n') {
            if reader.count as usize >= MAX_EVENTS {
                return Err(invalid("eventBudgetExceeded"));
            }
            if line.len() > MAX_EVENT_BYTES {
                return Err(invalid("eventTooLarge"));
            }
            let value = match serde_json::from_slice(line) {
                Ok(v) => v,
                Err(_) if line.iter().all(u8::is_ascii_whitespace) => {
                    reader.count += 1;
                    continue;
                }
                Err(_) => {
                    reader.reasons.insert("incompleteEnumeration".into());
                    if line.last() != Some(&b'\n') {
                        break;
                    }
                    reader.count += 1;
                    continue;
                }
            };
            reader.count += 1;
            reader.accept(value, None, reader.count)?;
        }
        let mut read = reader.finish()?;
        read.decoded_bytes = bytes.len();
        Ok(read)
    })
}
// Only a native first-line header can identify an obsolete JSONL projection.
// Its body/version is irrelevant once the primary database owns that SID.
pub(crate) fn shadowed_legacy(
    path: &Path,
    primary_sessions: &BTreeSet<String>,
) -> Result<Option<OpenClawRead>, AppError> {
    usage_source::with_stable_regular_file(path, |file| {
        let mut header = Vec::new();
        BufReader::new(file.take(MAX_EVENT_BYTES as u64 + 1))
            .read_until(b'\n', &mut header)
            .map_err(|_| invalid("sourceUnreadable"))?;
        if header.len() > MAX_EVENT_BYTES {
            return Err(invalid("eventTooLarge"));
        }
        let value: Value = match serde_json::from_slice(&header) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        let sid = (value.get("type").and_then(Value::as_str) == Some("session"))
            .then(|| identifier(value.get("id")))
            .flatten()
            .filter(|sid| primary_sessions.contains(sid));
        Ok(sid.map(|sid| {
            OpenClawRead::suppressed(Some(sid), header.len(), "primaryDatabaseSessionPreferred")
        }))
    })
}
fn sql<T>(result: rusqlite::Result<T>) -> Result<T, AppError> {
    result.map_err(|_| invalid("invalidDatabase"))
}
fn read_database(conn: &Connection, path: &Path, agent: String) -> Result<OpenClawRead, AppError> {
    let version: i64 = sql(conn.pragma_query_value(None, "user_version", |r| r.get(0)))?;
    let owner: (String, i64, String) = sql(conn.query_row(
        "SELECT role,schema_version,agent_id FROM schema_meta WHERE meta_key='primary'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ))?;
    if version != 24 || owner.0 != "agent" || owner.1 != 24 || owner.2 != agent {
        return Err(invalid("unsupportedSchemaOrAgentIdentity"));
    }
    let mut reader = Reader::new(path, agent);
    {
        let mut stmt =
            sql(conn.prepare("SELECT session_id,reason FROM session_windows LIMIT 20001"))?;
        let mut rows = sql(stmt.query([]))?;
        while let Some(row) = sql(rows.next())? {
            let sid: String = sql(row.get(0))?;
            if sid.trim().is_empty() || sid.len() > 512 {
                return Err(invalid("invalidSessionIdentity"));
            }
            if reader.sessions.len() >= MAX_SESSIONS {
                return Err(invalid("sessionBudgetExceeded"));
            }
            reader.sessions.insert(sid.clone());
            if sql(row.get::<_, Option<String>>(1))?.as_deref() == Some("fork") {
                reader.blocked.insert(sid);
                reader.reasons.insert("forkProvenanceUnverified".into());
            }
        }
    }
    for table in [
        "session_transcript_archives",
        "session_transcript_cold_archives",
    ] {
        let mut stmt = sql(conn.prepare(&format!(
            "SELECT DISTINCT session_id FROM {table} LIMIT 20001"
        )))?;
        let mut rows = sql(stmt.query([]))?;
        while let Some(row) = sql(rows.next())? {
            let sid: String = sql(row.get(0))?;
            if sid.trim().is_empty() || sid.len() > 512 {
                return Err(invalid("invalidSessionIdentity"));
            }
            if reader.sessions.len() >= MAX_SESSIONS && !reader.sessions.contains(&sid) {
                return Err(invalid("sessionBudgetExceeded"));
            }
            reader.sessions.insert(sid);
            reader.reasons.insert("nativeArchiveNotEnumerated".into());
        }
    }
    let mut stmt=sql(conn.prepare("SELECT session_id,seq,typeof(event_json),length(CAST(event_json AS BLOB)),CASE WHEN length(CAST(event_json AS BLOB))<=4194304 THEN event_json END,typeof(event_zstd),length(event_zstd),CASE WHEN length(event_zstd)<=4194304 THEN event_zstd END,event_utf8_bytes FROM transcript_events ORDER BY session_id,seq LIMIT 100001"))?;
    let mut rows = sql(stmt.query([]))?;
    let mut decoded = 0_usize;
    while let Some(row) = sql(rows.next())? {
        reader.count += 1;
        if reader.count as usize > MAX_EVENTS {
            return Err(invalid("eventBudgetExceeded"));
        }
        let sid: String = sql(row.get(0))?;
        let seq: i64 = sql(row.get(1))?;
        if sid.trim().is_empty() || sid.len() > 512 || seq < 0 {
            return Err(invalid("invalidSessionIdentity"));
        }
        reader.sessions.insert(sid.clone());
        if reader.sessions.len() > MAX_SESSIONS {
            return Err(invalid("sessionBudgetExceeded"));
        }
        let text_type: String = sql(row.get(2))?;
        let blob_type: String = sql(row.get(5))?;
        let raw = if text_type == "text" && blob_type == "null" {
            let len: i64 = sql(row.get(3))?;
            if len <= 0 || len as u64 > MAX_EVENT_BYTES as u64 {
                return Err(invalid("eventTooLarge"));
            }
            sql(row.get::<_, String>(4))?.into_bytes()
        } else if text_type == "null" && blob_type == "blob" {
            let len: i64 = sql(row.get(6))?;
            let raw_len = match sql(row.get_ref(8))? {
                ValueRef::Integer(n) if n > 0 && n <= MAX_EVENT_BYTES as i64 => n as usize,
                _ => return Err(invalid("invalidCompressedPayload")),
            };
            if len <= 0 || len as u64 > MAX_EVENT_BYTES as u64 {
                return Err(invalid("eventTooLarge"));
            }
            let blob: Vec<u8> = sql(row.get(7))?;
            let mut decoder = zstd::stream::read::Decoder::new(blob.as_slice())
                .map_err(|_| invalid("invalidCompressedPayload"))?;
            decoder
                .window_log_max(23)
                .map_err(|_| invalid("invalidCompressedPayload"))?;
            let mut raw = Vec::new();
            decoder
                .take(raw_len as u64 + 1)
                .read_to_end(&mut raw)
                .map_err(|_| invalid("invalidCompressedPayload"))?;
            if raw.len() != raw_len {
                return Err(invalid("invalidCompressedPayload"));
            }
            raw
        } else {
            return Err(invalid("invalidEventStorage"));
        };
        decoded = decoded
            .checked_add(raw.len())
            .filter(|n| *n <= MAX_DECODED_BYTES)
            .ok_or_else(|| invalid("decodedBudgetExceeded"))?;
        let value: Value =
            serde_json::from_slice(&raw).map_err(|_| invalid("invalidNativeEvent"))?;
        reader.accept(value, Some(&sid), seq)?;
    }
    let mut read = reader.finish()?;
    read.decoded_bytes = decoded;
    Ok(read)
}
pub(crate) fn read_openclaw(path: &Path) -> Result<OpenClawRead, AppError> {
    let (agent, kind) = location(path)?;
    match kind {
        Kind::Legacy => read_jsonl(path, agent),
        Kind::Database => {
            let mut snapshot = super::native_sqlite_snapshot::SqliteSnapshot::open(
                path,
                MAX_SOURCE_BYTES,
                invalid,
            )?;
            snapshot.read(|| {}, |conn| read_database(conn, path, agent))
        }
    }
}

// A strictly matched positive source receipt remembers primary authority. It
// does not acknowledge any event or make absence into ingestion evidence.
pub(crate) fn remembered_primaries_on(
    conn: &Connection,
    root: &Path,
) -> Result<BTreeMap<PathBuf, Value>, AppError> {
    use crate::session_manager::source_cache::{self, Part};
    let mut out = BTreeMap::new();
    let mut stmt=sql(conn.prepare("SELECT source_path,provider_id,stamp,parser_version,meta_json FROM session_meta_cache WHERE provider_id='openclaw' ORDER BY source_path"))?;
    let mut rows = sql(stmt.query([]))?;
    let mut count = 0;
    while let Some(row) = sql(rows.next())? {
        count += 1;
        if count > 20_000 {
            return Err(invalid("sourceBudgetExceeded"));
        }
        let path = PathBuf::from(sql(row.get::<_, String>(0))?);
        let Ok((agent, Kind::Database)) = location(&path) else {
            continue;
        };
        if path
            != root
                .join("agents")
                .join(agent)
                .join("agent/openclaw-agent.sqlite")
        {
            continue;
        }
        let parts = source_cache::decode_row(
            (
                sql(row.get(1))?,
                sql(row.get(2))?,
                sql(row.get(3))?,
                sql(row.get(4))?,
            ),
            "openclaw",
        )?;
        if let Some(evidence) = parts.value(Part::UsageSync) {
            usage_source::decode_evidence(evidence)?;
            if let Some(receipt) = parts.receipt() {
                out.insert(path, receipt);
            }
        }
    }
    Ok(out)
}
pub(crate) fn sync_openclaw_usage_at(
    db: &crate::database::Database,
    root: &Path,
) -> Result<super::session_usage::SessionSyncResult, AppError> {
    use super::usage_source::{Prepared, ProviderIndex, Snapshot};
    use super::usage_sync_db::{commit_sync_batch, read_sync_state, CheckpointChange, SyncState};
    use crate::session_manager::{providers::openclaw, source_cache};
    use rusqlite::{params, OptionalExtension};
    let files = openclaw::usage_files(root).map_err(|_| invalid("sourceUnavailable"))?;
    let mut result = super::session_usage::SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: files.len() as u32,
        errors: Vec::new(),
    };
    if files.is_empty() {
        return Ok(result);
    }
    let index = ProviderIndex::load(db, "openclaw", root)?;
    let remembered = {
        let conn = db.conn.lock().map_err(|_| invalid("databaseUnavailable"))?;
        remembered_primaries_on(&conn, root)?
    };
    let mut missing_agents = BTreeSet::new();
    for path in remembered.keys() {
        if files.binary_search(path).is_err() {
            missing_agents.insert(location(path)?.0);
        }
    }
    let mut sources = Vec::new();
    let mut states = Vec::new();
    let mut scan_bytes = 0_u64;
    for path in &files {
        let snapshot = if location(path)?.1 == Kind::Database {
            Snapshot::sqlite(path)?
        } else {
            Snapshot::file(path)?
        };
        let size = if location(path)?.1 == Kind::Database {
            snapshot.stamp["main"]["size"]
                .as_u64()
                .and_then(|n| n.checked_add(snapshot.stamp["wal"]["size"].as_u64().unwrap_or(0)))
        } else {
            snapshot.stamp["size"].as_u64()
        }
        .ok_or_else(|| invalid("invalidSourceStamp"))?;
        scan_bytes = scan_bytes
            .checked_add(size)
            .filter(|n| *n <= MAX_SCAN_BYTES)
            .ok_or_else(|| invalid("sourceBudgetExceeded"))?;
        let state = read_sync_state(db, &path.to_string_lossy())?;
        sources.push(Prepared::with_index(db, snapshot, state, 1, &index)?);
        states.push(state);
    }
    let dependency = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&json!([
                sources
                    .iter()
                    .map(|s| (&s.snapshot.path, &s.snapshot.stamp))
                    .collect::<Vec<_>>(),
                missing_agents
            ]))
            .map_err(|_| invalid("invalidSourceStamp"))?
        )
    );
    for source in &mut sources {
        source.observe_dependency(&dependency);
    }
    if sources.iter().all(|s| s.unchanged) {
        return Ok(result);
    }
    let mut parsed = Vec::new();
    let mut checkpoints = Vec::new();
    let mut primary_sessions = BTreeMap::<String, BTreeSet<String>>::new();
    let mut decoded = 0_usize;
    let mut event_count = 0_usize;
    for (i, path) in files.iter().enumerate() {
        let (agent, kind) = location(path)?;
        let read = if kind == Kind::Legacy && missing_agents.contains(&agent) {
            OpenClawRead::suppressed(None, 0, "nativePrimaryMissing")
        } else if kind == Kind::Legacy {
            match primary_sessions.get(&agent) {
                Some(known) => match shadowed_legacy(path, known)? {
                    Some(read) => read,
                    None => read_openclaw(path)?,
                },
                None => read_openclaw(path)?,
            }
        } else {
            read_openclaw(path)?
        };
        decoded = decoded
            .checked_add(read.decoded_bytes)
            .filter(|n| *n <= MAX_DECODED_BYTES)
            .ok_or_else(|| invalid("decodedBudgetExceeded"))?;
        event_count = event_count
            .checked_add(read.event_count as usize)
            .filter(|n| *n <= MAX_EVENTS)
            .ok_or_else(|| invalid("eventBudgetExceeded"))?;
        if kind == Kind::Database {
            primary_sessions.insert(agent, read.sessions.clone());
        }
        let stamp = &sources[i].snapshot.stamp;
        let own = if kind == Kind::Database {
            &stamp["main"]
        } else {
            stamp
        };
        let modified = own["mtimeNs"]
            .as_str()
            .and_then(|v| v.parse::<u128>().ok())
            .and_then(|v| i64::try_from(v / 1_000_000_000).ok())
            .ok_or_else(|| invalid("invalidSourceStamp"))?;
        checkpoints.push(CheckpointChange {
            key: path.to_string_lossy().into_owned(),
            expected: states[i],
            next: Some(SyncState {
                modified,
                offset: read.event_count,
            }),
        });
        parsed.push(read);
    }
    let mut suppressed = Vec::new();
    let mut identities = BTreeMap::<String, String>::new();
    for (i, read) in parsed.iter().enumerate() {
        let (agent, kind) = location(&files[i])?;
        let ignore = kind == Kind::Legacy
            && (missing_agents.contains(&agent)
                || primary_sessions
                    .get(&agent)
                    .is_some_and(|known| read.sessions.iter().any(|sid| known.contains(sid))));
        suppressed.push(ignore);
        if ignore {
            continue;
        }
        for (native, digest) in &read.native_events {
            if identities.get(native).is_some_and(|old| old != digest) {
                return Err(invalid("conflictingNativeReplicas"));
            }
            identities.insert(native.clone(), digest.clone());
        }
    }
    commit_sync_batch(db, &checkpoints, |conn| {
        if openclaw::usage_files(root).map_err(|_| invalid("sourceUnavailable"))? != files
            || remembered_primaries_on(conn, root)? != remembered
        {
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
            if suppressed[i] {
                source.evidence.complete_event_identity_set = false;
                source
                    .evidence
                    .partial(if missing_agents.contains(&location(&files[i])?.0) {
                        "nativePrimaryMissing"
                    } else {
                        "primaryDatabaseSessionPreferred"
                    });
            }
            for fact in &read.facts {
                let (Some(native), Some(scope), Some(at_ms)) = (
                    fact.native_key.as_deref(),
                    fact.session_id.as_deref(),
                    fact.at_ms,
                ) else {
                    result.skipped += 1;
                    continue;
                };
                let raw_scope: [String; 2] =
                    serde_json::from_str(scope).map_err(|_| invalid("invalidSessionIdentity"))?;
                if suppressed[i] || read.blocked_sessions.contains(&raw_scope[1]) {
                    result.skipped += 1;
                    continue;
                }
                let row:Option<(String,String,Option<String>,Option<String>)>=conn.query_row("SELECT provider_id,app_type,data_source,session_id FROM proxy_request_logs WHERE request_id=?1",[&fact.request_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
                if row.as_ref().is_some_and(|(owner, app, origin, sid)| {
                    owner != "_openclaw_session"
                        || app != "openclaw"
                        || origin.as_deref() != Some("openclaw_session")
                        || sid.as_deref() != Some(scope)
                }) {
                    return Err(invalid("foreignRecordIdentity"));
                }
                if !source.should_store_on(
                    conn,
                    &fact.request_id,
                    native,
                    "openclaw_session",
                    at_ms.div_euclid(1000),
                )? {
                    result.skipped += 1;
                    continue;
                }
                let u = &fact.usage;
                let changed=conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES(?1,'_openclaw_session','openclaw',?2,?3,?4,?5,?6,0,0,?7,?8,'openclaw_session') ON CONFLICT(request_id) DO UPDATE SET model=excluded.model,input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cache_read_tokens=excluded.cache_read_tokens,cache_creation_tokens=excluded.cache_creation_tokens,created_at=excluded.created_at WHERE model!=excluded.model OR input_tokens!=excluded.input_tokens OR output_tokens!=excluded.output_tokens OR cache_read_tokens!=excluded.cache_read_tokens OR cache_creation_tokens!=excluded.cache_creation_tokens OR created_at!=excluded.created_at",params![fact.request_id,fact.model,u.input_tokens,u.output_tokens,u.cache_read_tokens,u.cache_creation_tokens,scope,at_ms.div_euclid(1000)])?;
                if changed > 0 {
                    result.imported += 1;
                } else {
                    result.skipped += 1;
                }
                source.evidence.acknowledge(native);
            }
            source.save_on(conn, checkpoints[i].next.unwrap())?;
        }
        if !missing_agents.is_empty() {
            source_cache::invalidate_provider_on(conn, "openclaw", "nativePrimaryMissing")?;
        }
        Ok(())
    })?;
    Ok(result)
}

#[cfg(test)]
#[path = "session_usage_openclaw_tests.rs"]
mod tests;
