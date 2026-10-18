//! Machine-local, body-free analytics cache. Native files are read on refresh;
//! changing a query or opening details reads the cache in one batch only.
#[cfg(test)]
pub(crate) use super::session_analytics_discovery::discover_sources;
pub(crate) use super::session_analytics_discovery::{BatchSource, Discovery, DiscoveryFailure};
use super::{
    session_analytics::{AnalyticsInput, SessionDescriptor, SourceReason},
    session_analytics_core::{MessageEvent, Project, Role, SessionIdentity, UsageFact},
    session_analytics_messages::{TranscriptFacts, TranscriptScan},
    usage_query::SourceHealth,
    usage_source,
};
use crate::{
    database::Database,
    error::AppError,
    session_manager::source_cache::{self, Part, SourceStamp},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
#[path = "session_analytics_codex.rs"]
mod codex_index;
#[path = "codex_thread_usage.rs"]
pub(crate) mod codex_thread_usage;
#[path = "session_analytics_rebuild.rs"]
mod rebuild;

const VERSION: u8 = 1;
const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_SOURCES: usize = 20_000;
const PROJECTION_CACHE_BYTES: usize = 128 * 1024 * 1024;
const PROJECTION_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

#[cfg(test)]
#[path = "session_analytics_codex_rebuild_tests.rs"]
mod codex_rebuild_tests;
#[cfg(test)]
#[path = "session_analytics_parity_tests.rs"]
mod parity_tests;

#[cfg(test)]
thread_local! {
    static QUERY_DECODE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_query_decode_count() {
    QUERY_DECODE_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn query_decode_count() -> usize {
    QUERY_DECODE_COUNT.get()
}

struct CachedProjection {
    key: [u8; 32],
    input: Arc<AnalyticsInput>,
    thread_sources: Arc<Vec<ThreadSource>>,
    native_sources: Arc<Vec<NativeBatchSource>>,
    created_at: Instant,
}

/// One verified, body-free input projection. Keys describe the full SQLite
/// observation, never a connection address, query filter, price or native path
/// reopened outside the transaction. An in-flight Arc may outlive eviction;
/// the budget limits this slot's retention, not concurrent request memory.
pub(crate) struct ProjectionCache {
    slot: Mutex<Option<CachedProjection>>,
    max_bytes: usize,
    ttl: Duration,
}
type ProjectionParts = (
    Arc<AnalyticsInput>,
    Arc<Vec<ThreadSource>>,
    Arc<Vec<NativeBatchSource>>,
);
impl Default for ProjectionCache {
    fn default() -> Self {
        Self {
            slot: Mutex::new(None),
            max_bytes: PROJECTION_CACHE_BYTES,
            ttl: PROJECTION_CACHE_TTL,
        }
    }
}
impl ProjectionCache {
    pub(crate) fn read_on(
        &self,
        conn: &rusqlite::Connection,
    ) -> Result<Arc<AnalyticsInput>, AppError> {
        self.read_at(conn, Instant::now())
    }

    fn read_at(
        &self,
        conn: &rusqlite::Connection,
        now: Instant,
    ) -> Result<Arc<AnalyticsInput>, AppError> {
        Ok(self.parts_at(conn, now)?.0)
    }
    fn parts_at(
        &self,
        conn: &rusqlite::Connection,
        now: Instant,
    ) -> Result<ProjectionParts, AppError> {
        // Read and validate SQL/budgets before any lookup. Even a matching
        // warm slot must not conceal a read failure or an oversized table.
        let raw_rows = raw_rows_on(conn)?;
        let common = super::usage_health::source_health_on(conn)?;
        let epoch = source_cache::ledger_epoch_on(conn)?;
        let ready = raw_rows
            .iter()
            .find(|row| row.path == rebuild::GENERATION_KEY)
            .is_some_and(|row| {
                // The generation marker is a tiny receipt, not a source envelope.
                // Do not charge it as re-decoding all native projections on a warm query.
                let Some(raw) = row
                    .raw
                    .as_deref()
                    .filter(|raw| raw.len() <= source_cache::MAX_ENVELOPE_BYTES)
                else {
                    return false;
                };
                if serde_json::from_str::<ReadableJson>(raw).is_err() {
                    return false;
                }
                serde_json::from_str::<MapOnly<QueryEnvelope>>(raw)
                    .ok()
                    .filter(|MapOnly(envelope)| {
                        envelope.format_version == 1 && envelope.parts.valid_wrappers()
                    })
                    .and_then(|MapOnly(envelope)| envelope.parts.analytics)
                    .is_some_and(|part| {
                        part.value.version == VERSION && generation_is_current(&part.value, &epoch)
                    })
            });
        let legacy = if ready {
            None
        } else {
            Some(
                super::session_analytics_legacy::load_on(conn).unwrap_or_else(|_| {
                    super::session_analytics_legacy::LegacySnapshot::unavailable()
                }),
            )
        };
        let legacy_failed = legacy.as_ref().is_some_and(|snapshot| snapshot.failed);
        let legacy_history_available = legacy_history_on(conn)?;
        let mut key_hash = Sha256::new();
        key_hash.update(projection_key(&raw_rows, &common)?);
        key_hash.update([u8::from(legacy_history_available)]);
        if let Some(legacy) = &legacy {
            key_hash.update(legacy.digest.as_bytes());
        }
        key_hash.update(
            serde_json::to_vec(&source_cache::ledger_epoch_on(conn)?)
                .map_err(|_| invalid("invalidEpoch"))?,
        );
        let key: [u8; 32] = key_hash.finalize().into();
        {
            let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
            if slot
                .as_ref()
                .is_some_and(|entry| now.saturating_duration_since(entry.created_at) >= self.ttl)
            {
                *slot = None;
            }
            if let Some(entry) = slot.as_ref().filter(|entry| entry.key == key) {
                return Ok((
                    Arc::clone(&entry.input),
                    Arc::clone(&entry.thread_sources),
                    Arc::clone(&entry.native_sources),
                ));
            }
        }
        // Decode and deduplicate the already-read rows without another SELECT.
        // No raw envelope or native transcript is retained by the slot.
        let rows = decode_rows(raw_rows);
        let thread_sources = Arc::new(thread_sources_from_rows(&rows));
        let native_sources = Arc::new(native_sources_from_rows(&rows));
        let mut input = input_from_rows(rows, common, source_cache::ledger_epoch_on(conn)?);
        input.legacy_history_available = legacy_history_available;
        if let Some(legacy) = legacy {
            legacy.apply(&mut input)?;
        }
        let input = Arc::new(input);
        let bytes = projection_bytes(&input)
            .and_then(|bytes| bytes.checked_add(thread_source_bytes(&thread_sources)?))
            .and_then(|bytes| bytes.checked_add(native_source_bytes(&native_sources)?));
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        *slot = bytes
            .filter(|_| !legacy_failed)
            .filter(|bytes| *bytes <= self.max_bytes)
            .map(|_| CachedProjection {
                key,
                input: Arc::clone(&input),
                thread_sources: Arc::clone(&thread_sources),
                native_sources: Arc::clone(&native_sources),
                created_at: now,
            });
        Ok((input, thread_sources, native_sources))
    }
}

pub(crate) fn projection_cache() -> &'static ProjectionCache {
    static CACHE: OnceLock<ProjectionCache> = OnceLock::new();
    CACHE.get_or_init(ProjectionCache::default)
}

struct RawRow {
    path: String,
    provider: String,
    stamp: Option<String>,
    parser_version: Option<i64>,
    raw: Option<String>,
}

fn projection_key(rows: &[RawRow], common: &[SourceHealth]) -> Result<[u8; 32], AppError> {
    fn string(hash: &mut Sha256, value: &str) {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    fn optional_string(hash: &mut Sha256, value: Option<&str>) {
        hash.update([u8::from(value.is_some())]);
        if let Some(value) = value {
            string(hash, value);
        }
    }
    let mut hash = Sha256::new();
    hash.update(b"analytics-input-projection-v1\0");
    hash.update((rows.len() as u64).to_be_bytes());
    for row in rows {
        string(&mut hash, &row.path);
        string(&mut hash, &row.provider);
        optional_string(&mut hash, row.stamp.as_deref());
        hash.update([u8::from(row.parser_version.is_some())]);
        if let Some(version) = row.parser_version {
            hash.update(version.to_be_bytes());
        }
        optional_string(&mut hash, row.raw.as_deref());
    }
    // All effective health fields matter, including epochs resolved to unknown,
    // scan counters and retained-detail ranges. The raw health setting itself
    // is not consumed by input construction and need not be read a second time.
    let common = serde_json::to_vec(common).map_err(|_| invalid("invalidSourceHealth"))?;
    hash.update((common.len() as u64).to_be_bytes());
    hash.update(common);
    Ok(hash.finalize().into())
}

/// Count the owned allocation capacities, rather than serialized length. This
/// covers nested strings and Vec storage, plus conservative Arc/slot overhead;
/// checked arithmetic refuses retention if a size cannot be represented.
fn projection_bytes(input: &AnalyticsInput) -> Option<usize> {
    struct Bytes(usize);
    impl Bytes {
        fn add(&mut self, bytes: usize) -> Option<()> {
            self.0 = self.0.checked_add(bytes)?;
            Some(())
        }
        fn string(&mut self, value: &String) -> Option<()> {
            self.add(value.capacity())
        }
        fn optional(&mut self, value: &Option<String>) -> Option<()> {
            if let Some(value) = value {
                self.string(value)?;
            }
            Some(())
        }
        fn vector<T>(&mut self, value: &Vec<T>) -> Option<()> {
            self.add(value.capacity().checked_mul(std::mem::size_of::<T>())?)
        }
        fn strings(&mut self, values: &Vec<String>) -> Option<()> {
            self.vector(values)?;
            for value in values {
                self.string(value)?;
            }
            Some(())
        }
    }
    let mut bytes = Bytes(std::mem::size_of::<AnalyticsInput>());
    bytes.add(std::mem::size_of::<CachedProjection>())?;
    bytes.add(4 * std::mem::size_of::<usize>())?;
    bytes.add(std::mem::size_of::<Vec<ThreadSource>>())?;
    bytes.string(&input.snapshot_version)?;
    bytes.vector(&input.sessions)?;
    for session in &input.sessions {
        bytes.string(&session.identity.key)?;
        bytes.string(&session.identity.tool_id)?;
        if let Project::Known(project) = &session.identity.project {
            bytes.string(project)?;
        }
        bytes.optional(&session.identity.verified_root_key)?;
        bytes.string(&session.native_session_id)?;
        bytes.string(&session.title)?;
        bytes.optional(&session.project_label)?;
        bytes.optional(&session.project_path)?;
        bytes.optional(&session.manager_key)?;
        bytes.strings(&session.message_reasons)?;
    }
    bytes.vector(&input.facts)?;
    for fact in &input.facts {
        bytes.string(&fact.event_key)?;
        bytes.optional(&fact.session_key)?;
        bytes.string(&fact.tool_id)?;
        bytes.string(&fact.source_id)?;
        bytes.string(&fact.model)?;
    }
    bytes.vector(&input.messages)?;
    for message in &input.messages {
        bytes.string(&message.session_key)?;
        bytes.string(&message.native_id)?;
    }
    bytes.vector(&input.native_activity)?;
    for event in &input.native_activity {
        bytes.string(&event.session_key)?;
        bytes.string(&event.event_key)?;
        bytes.string(&event.stream_key)?;
    }
    bytes.vector(&input.source_observations)?;
    for source in &input.source_observations {
        bytes.string(&source.tool_id)?;
        bytes.string(&source.source_id)?;
        bytes.string(&source.status)?;
    }
    for (thread, coverage) in &input.thread_usage_coverage {
        // Conservative tree-node allowance plus the actual owned strings.
        bytes.add(
            std::mem::size_of::<(
                String,
                super::session_analytics_listing::ThreadUsageCoverage,
            )>() + 4 * std::mem::size_of::<usize>(),
        )?;
        bytes.string(thread)?;
        for value in coverage.reasons.iter().chain(&coverage.incomplete_models) {
            bytes.add(std::mem::size_of::<String>() + 4 * std::mem::size_of::<usize>())?;
            bytes.string(value)?;
        }
    }
    bytes.strings(&input.reasons)?;
    bytes.vector(&input.source_reasons)?;
    for reason in &input.source_reasons {
        bytes.string(&reason.tool_id)?;
        bytes.string(&reason.source_id)?;
        bytes.string(&reason.reason)?;
    }
    bytes.vector(&input.sources)?;
    for source in &input.sources {
        bytes.string(&source.source_id)?;
        bytes.string(&source.tool_id)?;
        bytes.string(&source.status)?;
        bytes.optional(&source.error_code)?;
    }
    Some(bytes.0)
}

#[derive(Clone)]
pub(crate) struct SourceSpec {
    pub provider: String,
    pub path: PathBuf,
    pub subagent: bool,
    pub project_file: Option<PathBuf>,
    pub title_file: Option<PathBuf>,
}
#[derive(Default, Debug)]
pub(crate) struct RefreshReport {
    pub parsed: usize,
    pub unchanged: usize,
    pub failed: usize,
    pub provider_states: BTreeMap<String, RefreshState>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum RefreshState {
    Ok,
    Partial(&'static str),
    Failed(&'static str),
}
impl RefreshState {
    fn severity(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Partial(_) => 1,
            Self::Failed(_) => 2,
        }
    }
}
impl RefreshReport {
    fn mark(&mut self, provider: &str, state: RefreshState) {
        let slot = self
            .provider_states
            .entry(provider.to_owned())
            .or_insert(RefreshState::Ok);
        if state.severity() >= slot.severity() {
            *slot = state;
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    version: u8,
    stamp: Option<Value>,
    sessions: Vec<SessionDescriptor>,
    facts: Vec<UsageFact>,
    messages: Vec<MessageEvent>,
    reasons: Vec<String>,
    available: bool,
    status: String,
    priority: u8,
    attempted_at_ms: i64,
    succeeded_at_ms: Option<i64>,
    // Append optional fields: legacy positional Entry arrays remain readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thread_usage: Option<codex_thread_usage::Projection>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        with = "super::analytics_compact::activity"
    )]
    native_activity: Vec<super::session_native_activity::NativeActivityEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex_native: Option<codex_index::NativeReceipt>,
    #[serde(default)]
    native_method: bool,
    #[serde(default)]
    physical_source_count: Option<usize>,
}
struct Row {
    provider: String,
    token: Option<String>,
    entry: Result<Option<Entry>, ()>,
    raw: String,
}

/// A query only needs Analytics' typed value. Other known parts still have
/// their wrapper validated, but their potentially large opaque values can be
/// skipped without materializing another JSON tree. Noncanonical inputs fall
/// back to the shared decoder below, preserving its legacy/duplicate-key rules.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryEnvelope {
    format_version: u64,
    #[serde(deserialize_with = "map_only")]
    parts: QueryParts,
}

struct MapOnly<T>(T);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for MapOnly<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        map_only(deserializer).map(Self)
    }
}

fn map_only<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct ObjectVisitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for ObjectVisitor<T> {
        type Value = T;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a JSON object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    // Only wrappers that the shared decoder checks with Value::as_object are
    // constrained here. Entry and its contents retain their original serde
    // behavior, including compatible positional representations.
    deserializer.deserialize_map(ObjectVisitor(std::marker::PhantomData))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryParts {
    #[serde(default, deserialize_with = "present_part")]
    metadata: Option<OpaqueQueryPart>,
    #[serde(default, deserialize_with = "present_part")]
    usage_sync: Option<OpaqueQueryPart>,
    #[serde(default, deserialize_with = "present_part")]
    lifetime_usage: Option<OpaqueQueryPart>,
    #[serde(default, deserialize_with = "present_part")]
    analytics: Option<AnalyticsQueryPart>,
}

fn present_part<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    // A present null wrapper is invalid in the shared contract. Ordinary
    // Option<T> deserialization would confuse it with an absent part.
    map_only(deserializer).map(Some)
}

#[derive(Deserialize)]
struct OpaqueQueryPart {
    revision: String,
    #[serde(rename = "value")]
    _value: serde::de::IgnoredAny,
}

#[derive(Deserialize)]
struct AnalyticsQueryPart {
    revision: String,
    // A cleared/null value uses the existing shared decoder; the hot path is
    // an actual typed Entry, whose required fields remain required.
    value: Entry,
}

/// Validate opaque extensions with the same numeric and recursion checks as
/// serde_json::Value, without allocating maps, vectors or owned JSON strings.
/// IgnoredAny/derived unknown-field skipping alone can accept out-of-range
/// numbers or skip the normal depth budget, so it cannot establish readability.
struct ReadableJson;
impl<'de> Deserialize<'de> for ReadableJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct JsonVisitor;
        impl<'de> serde::de::Visitor<'de> for JsonVisitor {
            type Value = ReadableJson;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("readable bounded JSON")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<ReadableJson, E> {
                Ok(ReadableJson)
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<ReadableJson, E> {
                Ok(ReadableJson)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<ReadableJson, E> {
                Ok(ReadableJson)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<ReadableJson, E> {
                Ok(ReadableJson)
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<ReadableJson, E> {
                if value.is_finite() {
                    Ok(ReadableJson)
                } else {
                    Err(E::custom("number out of range"))
                }
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<ReadableJson, E> {
                Ok(ReadableJson)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<ReadableJson, A::Error> {
                while sequence.next_element::<ReadableJson>()?.is_some() {}
                Ok(ReadableJson)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<ReadableJson, A::Error> {
                while map.next_key::<ReadableJson>()?.is_some() {
                    map.next_value::<ReadableJson>()?;
                }
                Ok(ReadableJson)
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}

impl QueryParts {
    fn valid_wrappers(&self) -> bool {
        [&self.metadata, &self.usage_sync, &self.lifetime_usage]
            .into_iter()
            .flatten()
            .all(|part| !part.revision.is_empty())
            && self
                .analytics
                .as_ref()
                .is_none_or(|part| !part.revision.is_empty())
    }
}
fn invalid(code: &str) -> AppError {
    AppError::Database(format!("usage.analytics.{code}"))
}
pub(crate) fn key(provider: &str, id: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(provider, id)).expect("string tuple"))
    )
}
/// Project filters must remain bounded even for a long native path. The exact
/// path is retained separately for display; no basename or whitespace folding
/// can make two distinct projects share an identity.
pub(crate) fn project_key(path: &str) -> String {
    key("project-v1", path)
}
/// Analytics observations tolerate opaque stamp extensions. This comparison
/// must never be used for ledger acknowledgements or negative identity proof.
fn same_known_revision(saved: &Value, current: &Value) -> bool {
    match (saved, current) {
        (Value::Object(saved), Value::Object(current)) => current.iter().all(|(key, value)| {
            saved
                .get(key)
                .is_some_and(|old| same_known_revision(old, value))
        }),
        (Value::Array(saved), Value::Array(current)) => {
            saved.len() == current.len()
                && saved
                    .iter()
                    .zip(current)
                    .all(|(a, b)| same_known_revision(a, b))
        }
        _ => saved == current,
    }
}
fn source_id(provider: &str) -> &str {
    match provider {
        "claude" => "session_log",
        "codex" => "codex_session",
        "gemini" => "gemini_session",
        "opencode" => "opencode_session",
        "workbuddy" => "workbuddy_session",
        "pi" => "pi_session",
        "dsh" => "dsh_session",
        "openclaw" => "openclaw_session",
        _ => "unsupported",
    }
}
/// Exactly one cache-table SELECT for a query/refresh. Changed sources alone
/// perform CAS writes; warm aggregation never point-reads each session.
fn rows_on(conn: &rusqlite::Connection) -> Result<BTreeMap<String, Row>, AppError> {
    Ok(decode_rows(raw_rows_on(conn)?))
}

fn raw_rows_on(conn: &rusqlite::Connection) -> Result<Vec<RawRow>, AppError> {
    let mut statement=conn.prepare("SELECT source_path,provider_id,stamp,parser_version,meta_json FROM session_meta_cache WHERE provider_id IN ('claude','codex','gemini','opencode','workbuddy','pi','dsh','openclaw') ORDER BY source_path")?;
    let mut result = Vec::new();
    let mut bytes = 0usize;
    let rows = statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    for row in rows {
        let (path, provider, stamp, version, raw) = row?;
        bytes = bytes
            .checked_add(raw.as_ref().map_or(0, String::len))
            .ok_or_else(|| invalid("cacheBudgetExceeded"))?;
        if bytes > MAX_CACHE_BYTES || result.len() >= MAX_SOURCES {
            return Err(invalid("cacheBudgetExceeded"));
        }
        result.push(RawRow {
            path,
            provider,
            stamp,
            parser_version: version,
            raw,
        });
    }
    Ok(result)
}

fn decode_rows(raw_rows: Vec<RawRow>) -> BTreeMap<String, Row> {
    raw_rows
        .into_iter()
        .map(|row| {
            let raw = row.raw.unwrap_or_default();
            let (token, entry) =
                decode_analytics_row(&row.provider, row.stamp, row.parser_version, &raw);
            (
                row.path,
                Row {
                    provider: row.provider,
                    token,
                    entry,
                    raw,
                },
            )
        })
        .collect()
}

fn decode_analytics_row(
    provider: &str,
    stamp: Option<String>,
    version: Option<i64>,
    raw: &str,
) -> (Option<String>, Result<Option<Entry>, ()>) {
    #[cfg(test)]
    QUERY_DECODE_COUNT.set(QUERY_DECODE_COUNT.get().saturating_add(1));
    if raw.len() <= source_cache::MAX_ENVELOPE_BYTES
        && serde_json::from_str::<ReadableJson>(raw).is_ok()
    {
        if let Ok(MapOnly(envelope)) = serde_json::from_str::<MapOnly<QueryEnvelope>>(raw) {
            if envelope.format_version == 1 && envelope.parts.valid_wrappers() {
                return match envelope.parts.analytics {
                    Some(part) => (
                        Some(part.revision),
                        if part.value.version == VERSION {
                            Ok(Some(part.value))
                        } else {
                            Err(())
                        },
                    ),
                    None => (None, Ok(None)),
                };
            }
        }
    }
    let decoded = source_cache::decode_row(
        (Some(provider.into()), stamp, version, Some(raw.into())),
        provider,
    );
    match decoded {
        Ok(parts) => {
            let token = parts.token(Part::Analytics);
            let entry = parts
                .value(Part::Analytics)
                .map(|value| {
                    let entry = Entry::deserialize(value).map_err(|_| ())?;
                    if entry.version != VERSION {
                        return Err(());
                    }
                    Ok(entry)
                })
                .transpose();
            (token, entry)
        }
        Err(_) => (None, Err(())),
    }
}
pub(crate) fn source_revision(spec: &SourceSpec) -> Result<Value, AppError> {
    fn dependency(path: Option<&Path>) -> Result<Option<Value>, AppError> {
        match path {
            Some(path) => match std::fs::symlink_metadata(path) {
                Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                    Ok(Some(json!(SourceStamp::at(path)?)))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                _ => Err(invalid("metadataDependencyUnavailable")),
            },
            None => Ok(None),
        }
    }
    let project = dependency(spec.project_file.as_deref())?;
    let title = dependency(spec.title_file.as_deref())?;
    let parts = if spec.provider == "opencode" && spec.path.is_dir() {
        Some(usage_source::Snapshot::tree(&spec.path.join("part"))?.stamp)
    } else {
        None
    };
    Ok(
        json!({"native":usage_source::normalized_stamp(&spec.provider,&spec.path)?,"project":project,"title":title,"parts":parts,"subagent":spec.subagent,"activityParserVersion":1,"codexParserVersion":if spec.provider=="codex" {Some(super::codex_native::VERSION)} else {None}}),
    )
}
fn empty_entry(now: i64) -> Entry {
    Entry {
        version: VERSION,
        thread_usage: None,
        native_activity: vec![],
        codex_native: None,
        native_method: false,
        physical_source_count: None,
        stamp: None,
        sessions: vec![],
        facts: vec![],
        messages: vec![],
        reasons: vec![],
        available: true,
        status: "failed".into(),
        priority: 0,
        attempted_at_ms: now,
        succeeded_at_ms: None,
    }
}
fn save(
    db: &Database,
    path: &str,
    provider: &str,
    token: &Option<String>,
    entry: &Entry,
) -> Result<(), AppError> {
    let mut conn = crate::database::lock_conn!(db.conn);
    let tx = conn.transaction()?;
    source_cache::merge_on(
        &tx,
        path,
        provider,
        None,
        Part::Analytics,
        token,
        Some(serde_json::to_value(entry).map_err(|_| invalid("invalidCacheValue"))?),
    )?;
    tx.commit()?;
    Ok(())
}
pub(crate) fn refresh_discovered(
    db: &Database,
    discovery: &Discovery,
) -> Result<RefreshReport, AppError> {
    let has_codex = discovery.specs.iter().any(|s| s.provider == "codex")
        || discovery.roots.iter().any(|(p, _)| p == "codex")
        || discovery.failures.iter().any(|f| f.provider == "codex");
    refresh_sources_inner(
        db,
        &discovery.specs,
        &discovery.failures,
        &discovery.batches,
        &discovery.roots,
        discovery.require_route_match,
        has_codex.then(|| {
            super::usage_rebuild::begin(
                discovery
                    .specs
                    .iter()
                    .filter(|s| s.provider == "codex")
                    .count(),
            )
        }),
    )
}

#[cfg(test)]
pub(crate) fn refresh_sources(
    db: &Database,
    specs: &[SourceSpec],
) -> Result<RefreshReport, AppError> {
    refresh_sources_inner(db, specs, &[], &[], &[], false, None)
}
fn refresh_sources_inner(
    db: &Database,
    specs: &[SourceSpec],
    failures: &[DiscoveryFailure],
    batches: &[BatchSource],
    roots: &[(String, PathBuf)],
    require_route_match: bool,
    control: Option<Arc<super::usage_rebuild::Control>>,
) -> Result<RefreshReport, AppError> {
    if specs.len() + batches.len() > MAX_SOURCES {
        return Err(invalid("sourceBudgetExceeded"));
    }
    let rows = {
        let conn = crate::database::lock_conn!(db.conn);
        // rows an older version wrote in the plain form can outgrow the byte
        // budget of the read below, which would then refuse every refresh
        if let Err(e) = super::analytics_compact::compact_legacy_on(&conn) {
            log::warn!("[USAGE-ANALYTICS] cache compaction failed: {e}");
        }
        rows_on(&conn)?
    };
    let mut generation =
        rebuild::Generation::new(db, &rows, specs, roots, require_route_match, control)?;
    let mut report = RefreshReport::default();
    let failed_providers: BTreeSet<_> = failures.iter().map(|f| f.provider.as_str()).collect();
    for failure in failures {
        report.failed += 1;
        report.mark(&failure.provider, RefreshState::Failed(failure.code));
    }
    let mut title_maps = BTreeMap::new();
    for path in specs
        .iter()
        .filter(|s| s.provider == "codex")
        .filter_map(|s| s.title_file.as_ref())
    {
        title_maps
            .entry(path.clone())
            .or_insert_with(|| crate::session_manager::providers::codex::thread_names(path));
    }
    let mut found = BTreeSet::new();
    for spec in specs {
        if spec.provider == "codex" && generation.check().is_err() {
            report.mark("codex", RefreshState::Partial("usage.rebuild.cancelled"));
            continue;
        }
        if failed_providers.contains(spec.provider.as_str()) {
            continue;
        }
        let Some(path) = spec.path.to_str() else {
            report.failed += 1;
            report.mark(
                &spec.provider,
                RefreshState::Failed("sourcePathUnavailable"),
            );
            continue;
        };
        if !found.insert(path.to_owned()) {
            return Err(invalid("duplicatePhysicalSource"));
        }
        let old = rows.get(path);
        if old.is_some_and(|r| r.provider != spec.provider || r.entry.is_err()) {
            report.failed += 1;
            report.mark(
                &spec.provider,
                RefreshState::Failed("analyticsCacheUnavailable"),
            );
            continue;
        }
        let previous = old
            .and_then(|r| r.entry.as_ref().ok())
            .and_then(Option::as_ref);
        if previous
            .and_then(|e| e.thread_usage.as_ref())
            .is_some_and(|p| !p.version_supported())
        {
            report.failed += 1;
            report.mark(
                &spec.provider,
                RefreshState::Failed("analyticsCacheUnavailable"),
            );
            continue;
        }
        let token = old.and_then(|r| r.token.clone());
        let now = chrono::Utc::now().timestamp_millis();
        let next_stamp = source_revision(spec);
        if let (Ok(next), Some(old)) = (&next_stamp, previous) {
            if old
                .stamp
                .as_ref()
                .is_some_and(|saved| same_known_revision(saved, next))
                && old.available
                && old.status != "failed"
                && (spec.provider != "codex"
                    || (old.native_method
                        && old
                            .thread_usage
                            .as_ref()
                            .is_some_and(|p| p.has_current_proof())))
            {
                if spec.provider == "codex" {
                    generation.observed(spec, Some(next.clone()))?;
                    // An epoch restore invalidates the generation marker, not
                    // the verified bytes of every native source. Republish the
                    // warm receipts under the same source/roster/epoch CAS.
                    if generation.needs_publication() {
                        generation
                            .pending
                            .push((path.to_owned(), token.clone(), old.clone()));
                    }
                }
                report.unchanged += 1;
                report.mark(
                    &spec.provider,
                    if old.status == "ok" {
                        RefreshState::Ok
                    } else {
                        RefreshState::Partial("partialSource")
                    },
                );
                continue;
            }
        }
        let parsed = next_stamp.and_then(|before| {
            let title_only = spec.provider == "codex"
                && previous.is_some_and(|old| {
                    old.available
                        && old.native_method
                        && old.status != "failed"
                        && old
                            .thread_usage
                            .as_ref()
                            .is_some_and(|projection| projection.has_current_proof())
                        && old.stamp.as_ref().is_some_and(|stamp| {
                            [
                                "native",
                                "project",
                                "parts",
                                "subagent",
                                "activityParserVersion",
                                "codexParserVersion",
                            ]
                            .iter()
                            .all(|field| stamp[*field] == before[*field])
                        })
                });
            let mut entry = if title_only {
                previous.unwrap().clone()
            } else {
                parse(spec, now)?
            };
            if spec.provider == "codex" {
                for session in &mut entry.sessions {
                    if let Some(title) = spec
                        .title_file
                        .as_ref()
                        .and_then(|p| title_maps.get(p))
                        .and_then(|m| m.get(&session.native_session_id))
                    {
                        session.title = crate::redact::scrub(title).into_owned();
                    }
                }
            }
            if source_revision(spec)? != before {
                return Err(invalid("sourceChangedRetry"));
            }
            entry.stamp = Some(before);
            Ok(entry)
        });
        let entry = match parsed {
            Ok(entry) => {
                report.parsed += 1;
                entry
            }
            Err(_) => {
                report.failed += 1;
                let mut entry = previous.cloned().unwrap_or_else(|| empty_entry(now));
                entry.status = "failed".into();
                entry.attempted_at_ms = now;
                entry.reasons.retain(|r| r != "sourceMissing");
                if !entry.reasons.iter().any(|r| r == "sourceReadFailed") {
                    entry.reasons.push("sourceReadFailed".into());
                }
                entry
            }
        };
        report.mark(
            &spec.provider,
            match entry.status.as_str() {
                "ok" => RefreshState::Ok,
                "failed" => RefreshState::Failed("sourceReadFailed"),
                _ => RefreshState::Partial("partialSource"),
            },
        );
        if spec.provider == "codex" {
            if generation
                .observed(
                    spec,
                    entry.stamp.clone().filter(|_| entry.status != "failed"),
                )
                .is_err()
            {
                report.mark("codex", RefreshState::Partial("usage.rebuild.cancelled"));
                continue;
            }
            generation.pending.push((path.to_owned(), token, entry));
        } else if save(db, path, &spec.provider, &token, &entry).is_err() {
            report.failed += 1;
            report.mark(
                &spec.provider,
                RefreshState::Failed("analyticsCacheWriteFailed"),
            );
        }
    }
    for batch in batches {
        let path = batch_key(batch)?;
        found.insert(path.clone());
        if failed_providers.contains(batch.provider.as_str()) {
            continue;
        }
        refresh_batch(db, batch, &path, rows.get(&path), &mut report)?;
    }
    for (path, row) in rows {
        if path == rebuild::GENERATION_KEY {
            continue;
        }
        if found.contains(&path) {
            continue;
        }
        let Ok(Some(mut entry)) = row.entry else {
            if row.entry.is_err() {
                report.mark(
                    &row.provider,
                    RefreshState::Failed("analyticsCacheUnavailable"),
                );
            }
            continue;
        };
        if failed_providers.contains(row.provider.as_str()) {
            // Failed enumeration says nothing about deletion. Keep the last
            // successful facts and the manager reference, but expose staleness.
            entry.status = "failed".into();
            entry.attempted_at_ms = chrono::Utc::now().timestamp_millis();
            entry.reasons.retain(|r| r != "sourceDiscoveryFailed");
            entry.reasons.push("sourceDiscoveryFailed".into());
            if row.provider == "codex" {
                generation.pending.push((path, row.token, entry));
            } else if save(db, &path, &row.provider, &row.token, &entry).is_err() {
                report.failed += 1;
                report.mark(
                    &row.provider,
                    RefreshState::Failed("analyticsCacheWriteFailed"),
                );
            }
            continue;
        }
        if !entry.available {
            report.mark(&row.provider, RefreshState::Partial("sourceMissing"));
            continue;
        }
        entry.available = false;
        entry.status = "partial".into();
        report.mark(&row.provider, RefreshState::Partial("sourceMissing"));
        entry.reasons.push("sourceMissing".into());
        for session in &mut entry.sessions {
            session.source_available = false;
            session.manager_key = None;
        }
        if row.provider == "codex" {
            generation.pending.push((path, row.token, entry));
        } else if save(db, &path, &row.provider, &row.token, &entry).is_err() {
            report.failed += 1;
            report.mark(
                &row.provider,
                RefreshState::Failed("analyticsCacheWriteFailed"),
            );
        }
    }
    if let Err(error) = generation.commit(db) {
        if error.to_string().contains("usage.rebuild.cancelled") {
            report.mark("codex", RefreshState::Partial("usage.rebuild.cancelled"));
        } else {
            report.failed += 1;
            report.mark(
                "codex",
                RefreshState::Failed("analyticsGenerationUnavailable"),
            );
        }
    }
    Ok(report)
}
pub(crate) struct ParsedSource {
    pub sessions: Vec<SessionDescriptor>,
    pub facts: Vec<UsageFact>,
    pub messages: Vec<MessageEvent>,
    pub native_activity: Vec<super::session_native_activity::NativeActivityEvent>,
    pub reasons: Vec<String>,
}
pub(crate) fn batch_key(batch: &BatchSource) -> Result<String, AppError> {
    let roots = serde_json::to_vec(&batch.roots).map_err(|_| invalid("invalidSourceRoots"))?;
    Ok(format!(
        "analytics-roster-v1:{}:{:x}",
        batch.provider,
        Sha256::digest(roots)
    ))
}
/// Observation only: source-specific readers retain their roster and lineage
/// rules. The shared wrapper never turns a physical file into an ownership proof.
pub(crate) fn batch_stamp(batch: &BatchSource) -> Result<Value, AppError> {
    match batch.provider.as_str() {
        "workbuddy" => super::session_analytics_workbuddy::stamp(&batch.roots),
        "pi" => super::session_analytics_pi::stamp(&batch.roots),
        "dsh" => super::session_analytics_dsh::stamp(&batch.roots),
        "openclaw" => super::session_analytics_openclaw::stamp(&batch.roots),
        _ => Err(invalid("unsupportedTranscriptReader")),
    }
}
fn batch_read(db: &Database, batch: &BatchSource) -> Result<ParsedSource, AppError> {
    match batch.provider.as_str() {
        "workbuddy" => super::session_analytics_workbuddy::read(db, &batch.roots),
        "pi" => super::session_analytics_pi::read(db, &batch.roots),
        "dsh" => super::session_analytics_dsh::read(db, &batch.roots),
        "openclaw" => super::session_analytics_openclaw::read(db, &batch.roots),
        _ => Err(invalid("unsupportedTranscriptReader")),
    }
}
fn batch_db_revision_on(conn: &rusqlite::Connection, provider: &str) -> Result<Value, AppError> {
    let manifest = source_cache::provider_on(conn, provider)?;
    let manifest =
        serde_json::to_vec(&manifest).map_err(|_| invalid("analyticsCacheUnavailable"))?;
    Ok(
        json!({"epoch":source_cache::ledger_epoch_on(conn)?,"manifest":format!("{:x}",Sha256::digest(manifest))}),
    )
}
fn native_revision(stamp: &Value) -> Result<String, AppError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(stamp).map_err(|_| invalid("invalidSourceStamp"))?)
    ))
}
fn preserve_absent_batch_sessions(entry: &mut Entry, previous: Option<&Entry>) {
    let Some(previous) = previous else {
        return;
    };
    let current = entry
        .sessions
        .iter()
        .map(|session| session.identity.key.as_str())
        .collect::<BTreeSet<_>>();
    let absent = previous
        .sessions
        .iter()
        .filter(|session| !current.contains(session.identity.key.as_str()))
        .map(|session| session.identity.key.clone())
        .collect::<BTreeSet<_>>();
    if absent.is_empty() {
        return;
    }
    for session in previous
        .sessions
        .iter()
        .filter(|session| absent.contains(&session.identity.key))
    {
        let mut session = session.clone();
        session.source_available = false;
        session.manager_key = None;
        if !session
            .message_reasons
            .iter()
            .any(|reason| reason == "sourceMissing")
        {
            session.message_reasons.push("sourceMissing".into());
        }
        entry.sessions.push(session);
    }
    entry.facts.extend(
        previous
            .facts
            .iter()
            .filter(|fact| {
                fact.session_key
                    .as_ref()
                    .is_some_and(|key| absent.contains(key))
            })
            .cloned(),
    );
    entry.messages.extend(
        previous
            .messages
            .iter()
            .filter(|message| absent.contains(&message.session_key))
            .cloned(),
    );
    entry.reasons.push("sourceMissing".into());
}
fn save_batch(
    db: &Database,
    batch: &BatchSource,
    path: &str,
    token: &Option<String>,
    entry: &Entry,
    db_revision: &Value,
    native: Option<&Value>,
) -> Result<(), AppError> {
    let mut conn = crate::database::lock_conn!(db.conn);
    let tx = conn.transaction()?;
    if batch_db_revision_on(&tx, &batch.provider)? != *db_revision {
        return Err(invalid("sourceChangedRetry"));
    }
    if let Some(native) = native {
        if batch_stamp(batch)? != *native {
            return Err(invalid("sourceChangedRetry"));
        }
    }
    source_cache::merge_on(
        &tx,
        path,
        &batch.provider,
        None,
        Part::Analytics,
        token,
        Some(serde_json::to_value(entry).map_err(|_| invalid("invalidCacheValue"))?),
    )?;
    tx.commit()?;
    Ok(())
}
fn refresh_batch(
    db: &Database,
    batch: &BatchSource,
    path: &str,
    old: Option<&Row>,
    report: &mut RefreshReport,
) -> Result<(), AppError> {
    if old.is_some_and(|row| row.provider != batch.provider || row.entry.is_err()) {
        report.failed += 1;
        report.mark(
            &batch.provider,
            RefreshState::Failed("analyticsCacheUnavailable"),
        );
        return Ok(());
    }
    let previous = old
        .and_then(|row| row.entry.as_ref().ok())
        .and_then(Option::as_ref);
    let token = old.and_then(|row| row.token.clone());
    let now = chrono::Utc::now().timestamp_millis();
    let db_revision = {
        let conn = crate::database::lock_conn!(db.conn);
        batch_db_revision_on(&conn, &batch.provider)?
    };
    let before = batch_stamp(batch);
    if let (Ok(native), Some(previous)) = (&before, previous) {
        let revision =
            json!({"native":native_revision(native)?,"db":db_revision,"roots":batch.roots});
        if previous.available
            && previous.status != "failed"
            && previous.stamp.as_ref() == Some(&revision)
        {
            report.unchanged += 1;
            report.mark(
                &batch.provider,
                if previous.status == "ok" {
                    RefreshState::Ok
                } else {
                    RefreshState::Partial("partialSource")
                },
            );
            return Ok(());
        }
    }
    let parsed = before
        .as_ref()
        .map_err(|_| invalid("sourceReadFailed"))
        .and_then(|native| {
            let parsed = batch_read(db, batch)?;
            if batch_stamp(batch)? != *native {
                return Err(invalid("sourceChangedRetry"));
            }
            let mut entry = empty_entry(now);
            entry.sessions = parsed.sessions;
            entry.facts = parsed.facts;
            entry.messages = parsed.messages;
            entry.native_activity = parsed.native_activity;
            entry.reasons = parsed.reasons;
            entry.physical_source_count = native
                .get("files")
                .or_else(|| native.pointer("/native/files"))
                .and_then(Value::as_array)
                .map(Vec::len);
            preserve_absent_batch_sessions(&mut entry, previous);
            entry.reasons.sort();
            entry.reasons.dedup();
            entry.priority = 3;
            entry.status = if entry.reasons.is_empty() {
                "ok"
            } else {
                "partial"
            }
            .into();
            entry.stamp = Some(
                json!({"native":native_revision(native)?,"db":db_revision,"roots":batch.roots}),
            );
            entry.succeeded_at_ms = Some(now);
            Ok(entry)
        });
    let (entry, verified) = match parsed {
        Ok(entry) => {
            report.parsed += 1;
            (entry, true)
        }
        Err(_) => {
            report.failed += 1;
            let mut entry = previous.cloned().unwrap_or_else(|| empty_entry(now));
            entry.status = "failed".into();
            entry.attempted_at_ms = now;
            entry.reasons.retain(|reason| reason != "sourceReadFailed");
            entry.reasons.push("sourceReadFailed".into());
            (entry, false)
        }
    };
    report.mark(
        &batch.provider,
        match entry.status.as_str() {
            "ok" => RefreshState::Ok,
            "failed" => RefreshState::Failed("sourceReadFailed"),
            _ => RefreshState::Partial("partialSource"),
        },
    );
    if save_batch(
        db,
        batch,
        path,
        &token,
        &entry,
        &db_revision,
        if verified { before.as_ref().ok() } else { None },
    )
    .is_err()
    {
        report.failed += 1;
        report.mark(
            &batch.provider,
            RefreshState::Failed("analyticsCacheWriteFailed"),
        );
    }
    Ok(())
}
fn parse(spec: &SourceSpec, now: i64) -> Result<Entry, AppError> {
    if spec.provider == "codex" {
        return codex_index::parse(spec, now);
    }
    let mut entry = empty_entry(now);
    if spec.provider == "opencode" {
        let parsed = super::session_analytics_opencode::read(&spec.path)?;
        entry.sessions = parsed.sessions;
        entry.facts = parsed.facts;
        entry.messages = parsed.messages;
        entry.native_activity = parsed.native_activity;
        entry.reasons = parsed.reasons;
        entry.status = if entry.reasons.is_empty() {
            "ok"
        } else {
            "partial"
        }
        .into();
        entry.priority = if spec.path.is_file() { 3 } else { 2 };
        entry.succeeded_at_ms = Some(now);
        return Ok(entry);
    }
    entry.priority = if spec.subagent { 1 } else { 2 };
    let mut candidates = crate::session_manager::title::TitleCandidates::default();
    let mut native_activity_candidates = Vec::new();
    let mut native_stream_segment = 0u32;
    let transcript = match spec.provider.as_str() {
        "claude" => {
            let mut claude = crate::session_manager::providers::claude::ClaudeScan::default();
            let mut scan = TranscriptScan::new(
                &spec.provider,
                &key(&spec.provider, &spec.path.to_string_lossy()),
                spec.subagent,
            )?;
            let text = usage_source::read_text(&spec.path)?;
            for (i, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let value: Value =
                    serde_json::from_str(line).map_err(|_| invalid("sourceReadFailed"))?;
                if let Some(at_ms) = value
                    .get("timestamp")
                    .and_then(crate::session_manager::providers::parse_timestamp_to_ms)
                {
                    let digest = format!(
                        "{:x}",
                        Sha256::digest(
                            serde_json::to_vec(&value).map_err(|_| invalid("sourceReadFailed"))?
                        )
                    );
                    native_activity_candidates.push((digest, at_ms, native_stream_segment));
                } else {
                    native_stream_segment = native_stream_segment.saturating_add(1);
                    if !entry.reasons.iter().any(|r| r == "activityTimeUnavailable") {
                        entry.reasons.push("activityTimeUnavailable".into());
                    }
                }
                scan.line(i as i64 + 1, &value)?;
                claude.line(&value);
            }
            candidates = claude.candidates;
            scan.finish()
        }
        "gemini" => {
            let conversation =
                crate::session_manager::providers::gemini::read_conversation(&spec.path)
                    .map_err(|_| invalid("sourceReadFailed"))?;
            let mut transcript = TranscriptFacts {
                is_main: !spec.subagent,
                ..Default::default()
            };
            match conversation.get("kind").and_then(Value::as_str) {
                Some("main") => {}
                Some("subagent") => {
                    // Native 0.64 JSONL records the role, but not a verifiable
                    // parent session ID. Keep spending without main activity.
                    transcript.is_main = false;
                    transcript.reasons.insert("sessionParentUnavailable".into());
                }
                None if spec.path.extension().is_some_and(|ext| ext == "json") => {}
                _ => {
                    transcript.is_main = false;
                    transcript.reasons.insert("sessionRoleNotRecorded".into());
                }
            }
            transcript.session_id = conversation["sessionId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            if let Some(project) = &spec.project_file {
                match std::fs::read_to_string(project) {
                    Ok(path) => {
                        transcript.project_parsed = true;
                        transcript.project_dir = (!path.trim().is_empty()).then_some(path);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        transcript.reasons.insert("projectNotRecorded".into());
                    }
                    Err(_) => return Err(invalid("projectDependencyUnavailable")),
                }
            } else {
                transcript.reasons.insert("projectNotRecorded".into());
            }
            for (i, message) in conversation["messages"]
                .as_array()
                .ok_or_else(|| invalid("messagesUnavailable"))?
                .iter()
                .enumerate()
            {
                let role = match message["type"].as_str() {
                    Some("user") => "user",
                    Some("gemini") => "assistant",
                    _ => continue,
                };
                let content = crate::session_manager::providers::extract_text(&message["content"]);
                if (role == "user"
                    && crate::session_manager::title::clean_prompt(&content).is_none())
                    || content.trim().is_empty()
                {
                    continue;
                }
                if role == "user" && candidates.prompt.is_none() {
                    candidates = candidates.with_first_user_text(Some(&content));
                }
                let id = message["id"].as_str().filter(|s| !s.is_empty());
                if id.is_none() {
                    transcript
                        .reasons
                        .insert("messageNativeIdentityUnavailable".into());
                }
                let at_ms = message
                    .get("timestamp")
                    .and_then(crate::session_manager::providers::parse_timestamp_to_ms);
                if at_ms.is_none() {
                    transcript.reasons.insert("messageTimeUnavailable".into());
                }
                let identity = id
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("local:{i}"));
                transcript.messages.insert(
                    identity.clone(),
                    super::session_analytics_messages::MessageFact {
                        identity,
                        at_ms,
                        role: role.into(),
                        native_identity: id.is_some(),
                    },
                );
            }
            transcript
        }
        _ => return Err(invalid("unsupportedTranscriptReader")),
    };
    let normalized = usage_source::read_normalized(&spec.provider, &spec.path)?;
    entry.reasons.extend(normalized.reasons);
    let native_id = transcript.session_id.clone();
    if let Some(native) = &native_id {
        let session_key = key(&spec.provider, native);
        let project = if !transcript.project_parsed {
            Project::Unparsed
        } else {
            transcript
                .project_dir
                .as_deref()
                .map(|path| Project::Known(project_key(path)))
                .unwrap_or(Project::Unknown)
        };
        let source_path = spec.path.to_string_lossy().into_owned();
        let meta = crate::session_manager::SessionMeta {
            provider_id: spec.provider.clone(),
            session_id: native.clone(),
            source_path: Some(source_path),
            ..Default::default()
        };
        let project_label = transcript.project_dir.as_ref().map(|p| {
            Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone())
        });
        if spec.provider == "claude" {
            let stream = key("claude-stream", &spec.path.to_string_lossy());
            entry.native_activity = native_activity_candidates
                .into_iter()
                .map(|(digest, at_ms, segment)| {
                    super::session_native_activity::NativeActivityEvent {
                        session_key: session_key.clone(),
                        event_key: key("claude-runtime", &format!("{native}:{digest}")),
                        at_ms,
                        main: transcript.is_main && !spec.subagent,
                        stream_key: format!("{stream}:{segment}"),
                    }
                })
                .collect();
        }
        entry.sessions.push(SessionDescriptor {
            identity: SessionIdentity {
                key: session_key.clone(),
                tool_id: spec.provider.clone(),
                project,
                verified_root_key: transcript
                    .parent_session_id
                    .as_deref()
                    .filter(|p| *p != native)
                    .map(|id| key(&spec.provider, id)),
            },
            native_session_id: native.clone(),
            title: crate::session_manager::title::pick_title(
                &candidates.with_directory(transcript.project_dir.as_deref()),
            )
            .map(|(title, _)| crate::redact::scrub(&title).into_owned())
            .unwrap_or_else(|| native.clone()),
            project_label,
            project_path: transcript.project_dir,
            manager_key: Some(crate::session_manager::managed::key(&meta)),
            source_available: true,
            messages_supported: true,
            message_reasons: transcript
                .reasons
                .iter()
                .chain(entry.reasons.iter().filter(|r| r.starts_with("activity")))
                .cloned()
                .collect(),
        });
        for message in transcript.messages.into_values() {
            if let Some(at_ms) = message.at_ms {
                entry.messages.push(MessageEvent {
                    session_key: session_key.clone(),
                    native_id: message.identity,
                    at_ms,
                    role: if message.role == "user" {
                        Role::User
                    } else {
                        Role::Assistant
                    },
                    main: transcript.is_main,
                });
            }
        }
    } else {
        entry.reasons.push("sessionIdentityUnavailable".into());
    }
    for fact in normalized.facts {
        let Some(at_ms) = fact.at_ms else {
            continue;
        };
        let identity = fact.native_key.as_deref().unwrap_or(&fact.request_id);
        entry.facts.push(UsageFact {
            event_key: key(&spec.provider, identity),
            session_key: fact.session_id.as_deref().map(|id| key(&spec.provider, id)),
            tool_id: spec.provider.clone(),
            source_id: source_id(&spec.provider).into(),
            model: fact.model,
            at_ms,
            buckets: [
                fact.usage.input_tokens,
                fact.usage.output_tokens,
                fact.usage.cache_read_tokens,
                fact.usage.cache_creation_tokens,
            ]
            .map(u64::from),
            complete: true,
        });
    }
    entry.reasons.extend(transcript.reasons);
    entry.reasons.sort();
    entry.reasons.dedup();
    entry.status = if entry.reasons.is_empty() {
        "ok"
    } else {
        "partial"
    }
    .into();
    entry.succeeded_at_ms = Some(now);
    Ok(entry)
}

fn health_severity(status: &str) -> u8 {
    match status {
        "unsupported" => 4,
        "failed" => 3,
        "partial" => 2,
        "unknown" => 1,
        _ => 0,
    }
}

/// A corrupt native parent component must not make an unrelated source query
/// fail. Clear unverifiable links with a scoped reason rather than guessing a
/// root. Memoizing each functional-graph chain keeps this linear in its edges.
fn isolate_invalid_roots(
    sessions: &mut BTreeMap<String, SessionDescriptor>,
    reasons: &mut Vec<SourceReason>,
) {
    let parents: BTreeMap<_, _> = sessions
        .iter()
        .map(|(key, s)| {
            (
                key.clone(),
                (
                    s.identity.tool_id.clone(),
                    s.identity.verified_root_key.clone(),
                ),
            )
        })
        .collect();
    let mut resolved: BTreeMap<String, bool> = BTreeMap::new();
    for start in parents.keys() {
        if resolved.contains_key(start) {
            continue;
        }
        let mut current = start.clone();
        let mut path = vec![];
        let mut visiting = BTreeSet::new();
        let valid = loop {
            if let Some(valid) = resolved.get(&current) {
                break *valid;
            }
            if !visiting.insert(current.clone()) {
                break false;
            }
            path.push(current.clone());
            let Some((tool, parent)) = parents.get(&current) else {
                break false;
            };
            let Some(parent) = parent else {
                break true;
            };
            if parents.get(parent).is_none_or(|(other, _)| other != tool) {
                break false;
            }
            current = parent.clone();
        };
        for key in path {
            resolved.insert(key, valid);
        }
    }
    for (key, valid) in resolved {
        if valid {
            continue;
        }
        let session = sessions.get_mut(&key).expect("known native graph key");
        session.identity.verified_root_key = None;
        session.messages_supported = false;
        session.message_reasons.push("invalidSessionRoot".into());
        reasons.push(SourceReason {
            tool_id: session.identity.tool_id.clone(),
            source_id: source_id(&session.identity.tool_id).into(),
            reason: "invalidSessionRoot".into(),
        });
    }
}

#[cfg(test)]
pub(crate) fn read_input(db: &Database) -> Result<AnalyticsInput, AppError> {
    let conn = crate::database::lock_conn!(db.conn);
    read_input_on(&conn)
}

fn legacy_history_on(conn: &rusqlite::Connection) -> Result<bool, AppError> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM proxy_request_logs WHERE app_type='codex' AND data_source='codex_session') OR EXISTS(SELECT 1 FROM usage_daily_rollups WHERE app_type='codex' AND provider_id='_codex_session')",[],|r|r.get(0))?)
}

#[cfg(test)]
pub(crate) fn read_input_on(conn: &rusqlite::Connection) -> Result<AnalyticsInput, AppError> {
    let rows = rows_on(conn)?;
    let common = super::usage_health::source_health_on(conn)?;
    let mut input = input_from_rows(rows, common, source_cache::ledger_epoch_on(conn)?);
    input.legacy_history_available = legacy_history_on(conn)?;
    super::session_analytics_legacy::apply_on(conn, &mut input)?;
    Ok(input)
}

/// Listing-only thread display evidence. This never rewrites ledger ancestry or
/// opens transcript bodies, and deliberately does not reuse root-normalized facts.
#[derive(Clone)]
struct ThreadSource {
    path: String,
    native_stamp: Option<String>,
    healthy: bool,
    projection: codex_thread_usage::Projection,
}
struct NativeBatchSource {
    provider: String,
    roots: Vec<PathBuf>,
    native_hash: String,
    db_hash: String,
    managers: BTreeSet<String>,
    healthy: bool,
}
fn native_sources_from_rows(rows: &BTreeMap<String, Row>) -> Vec<NativeBatchSource> {
    rows.iter()
        .filter_map(|(path, row)| {
            if !path.starts_with("analytics-roster-v1:") {
                return None;
            }
            let entry = row.entry.as_ref().ok()?.as_ref()?;
            let stamp = entry.stamp.as_ref()?;
            Some(NativeBatchSource {
                provider: row.provider.clone(),
                roots: serde_json::from_value(stamp.get("roots")?.clone()).ok()?,
                native_hash: stamp.get("native")?.as_str()?.into(),
                db_hash: native_revision(stamp.get("db")?).ok()?,
                managers: entry
                    .sessions
                    .iter()
                    .filter(|session| session.source_available)
                    .filter_map(|session| session.manager_key.clone())
                    .collect(),
                healthy: entry.available && entry.status != "failed",
            })
        })
        .collect()
}
fn native_source_bytes(sources: &Vec<NativeBatchSource>) -> Option<usize> {
    let mut bytes = sources
        .capacity()
        .checked_mul(std::mem::size_of::<NativeBatchSource>())?;
    for source in sources {
        bytes = bytes
            .checked_add(source.provider.capacity())?
            .checked_add(source.native_hash.capacity())?
            .checked_add(source.db_hash.capacity())?;
        bytes = bytes.checked_add(
            source
                .roots
                .capacity()
                .checked_mul(std::mem::size_of::<PathBuf>())?,
        )?;
        for root in &source.roots {
            bytes = bytes.checked_add(root.capacity())?;
        }
        for key in &source.managers {
            bytes = bytes.checked_add(key.capacity())?.checked_add(128)?;
        }
    }
    Some(bytes)
}
pub(crate) struct NativeListingProjection {
    pub available: bool,
    pub complete: bool,
    pub reasons: Vec<String>,
    pub models: Vec<codex_thread_usage::Model>,
}
pub(crate) type ListingProjections = (
    BTreeMap<String, codex_thread_usage::Projection>,
    BTreeMap<String, NativeListingProjection>,
);
/// Both display consumers share the same one-query, decoded snapshot.
pub(crate) fn listing_projections_on(
    conn: &rusqlite::Connection,
    listing: &[crate::session_manager::SessionMeta],
) -> Result<ListingProjections, AppError> {
    let (input, threads, native) = projection_cache().parts_at(conn, Instant::now())?;
    let codex = thread_usage_from_sources(&threads, listing, &input)?;
    let mut verified = BTreeSet::new();
    let mut changed = BTreeSet::new();
    for source in native.iter() {
        let batch = BatchSource {
            provider: source.provider.clone(),
            roots: source.roots.clone(),
        };
        let valid = source.healthy
            && batch_stamp(&batch)
                .and_then(|stamp| native_revision(&stamp))
                .as_ref()
                .ok()
                == Some(&source.native_hash)
            && native_revision(&batch_db_revision_on(conn, &source.provider)?)
                .as_ref()
                .ok()
                == Some(&source.db_hash);
        for key in &source.managers {
            if valid {
                verified.insert(key.clone());
            } else {
                changed.insert(key.clone());
            }
        }
    }
    let descriptors = input
        .sessions
        .iter()
        .filter_map(|session| session.manager_key.as_ref().map(|key| (key, session)))
        .collect::<BTreeMap<_, _>>();
    let mut models = BTreeMap::<String, BTreeMap<String, codex_thread_usage::Model>>::new();
    let mut incomplete = BTreeSet::new();
    for fact in &input.facts {
        let Some(session) = &fact.session_key else {
            continue;
        };
        let model = models
            .entry(session.clone())
            .or_default()
            .entry(fact.model.clone())
            .or_insert_with(|| codex_thread_usage::Model {
                model: fact.model.clone(),
                buckets: [0; 4],
                records: 0,
            });
        for (sum, value) in model.buckets.iter_mut().zip(fact.buckets) {
            *sum = sum
                .checked_add(value)
                .ok_or_else(|| invalid("aggregateOverflow"))?;
        }
        model.records = model
            .records
            .checked_add(1)
            .ok_or_else(|| invalid("aggregateOverflow"))?;
        if !fact.complete {
            incomplete.insert(session.clone());
        }
    }
    let mut result = BTreeMap::new();
    for meta in listing.iter().filter(|meta| {
        matches!(
            meta.provider_id.as_str(),
            "workbuddy" | "pi" | "dsh" | "openclaw"
        )
    }) {
        let manager = crate::session_manager::managed::key(meta);
        let mut projection = NativeListingProjection {
            available: false,
            complete: false,
            reasons: vec!["nativeSessionNotAssociated".into()],
            models: vec![],
        };
        if let Some(descriptor) = descriptors.get(&manager) {
            if descriptor.identity.tool_id == meta.provider_id
                && descriptor.source_available
                && verified.contains(&manager)
            {
                if let Some(rows) = models.get(&descriptor.identity.key) {
                    projection.available = true;
                    projection.complete = !incomplete.contains(&descriptor.identity.key);
                    projection.reasons = if projection.complete {
                        vec![]
                    } else {
                        vec!["incompleteSource".into()]
                    };
                    projection.models = rows.values().cloned().collect();
                }
            } else if changed.contains(&manager) {
                projection.reasons = vec!["threadUsageSourceChanged".into()];
            }
        }
        result.insert(manager, projection);
    }
    Ok((codex, result))
}
fn thread_sources_from_rows(rows: &BTreeMap<String, Row>) -> Vec<ThreadSource> {
    rows.iter()
        .filter_map(|(path, row)| {
            if row.provider != "codex" {
                return None;
            }
            let entry = row.entry.as_ref().ok()?.as_ref()?;
            Some(ThreadSource {
                path: path.clone(),
                native_stamp: entry.stamp.as_ref()?.get("native").map(Value::to_string),
                healthy: entry.available && entry.status != "failed",
                projection: entry.thread_usage.clone()?,
            })
        })
        .collect()
}
fn thread_source_bytes(sources: &Vec<ThreadSource>) -> Option<usize> {
    let mut bytes = sources
        .capacity()
        .checked_mul(std::mem::size_of::<ThreadSource>())?;
    for source in sources {
        let p = &source.projection;
        let strings = std::iter::once(&source.path)
            .chain(source.native_stamp.iter())
            .chain(p.native_thread_id.iter())
            .chain(p.billing_session_id.iter())
            .chain(p.parent_session_id.iter())
            .chain(std::iter::once(&p.state))
            .chain(p.reasons.iter())
            .chain(p.incomplete_models.iter())
            .chain(p.facts_hash.iter())
            .chain(p.source_digest.iter())
            .chain(p.models.iter().map(|m| &m.model));
        for value in strings {
            bytes = bytes.checked_add(value.capacity())?;
        }
        bytes = bytes.checked_add(
            p.reasons
                .capacity()
                .checked_mul(std::mem::size_of::<String>())?,
        )?;
        bytes = bytes.checked_add(
            p.incomplete_models
                .capacity()
                .checked_mul(std::mem::size_of::<String>())?,
        )?;
        bytes = bytes.checked_add(
            p.models
                .capacity()
                .checked_mul(std::mem::size_of::<codex_thread_usage::Model>())?,
        )?;
    }
    Some(bytes)
}
#[cfg(test)]
pub(crate) fn thread_usage_on(
    conn: &rusqlite::Connection,
    listing: &[crate::session_manager::SessionMeta],
) -> Result<BTreeMap<String, codex_thread_usage::Projection>, AppError> {
    thread_usage_cached_on(conn, listing, projection_cache())
}
#[cfg(test)]
fn thread_usage_cached_on(
    conn: &rusqlite::Connection,
    listing: &[crate::session_manager::SessionMeta],
    cache: &ProjectionCache,
) -> Result<BTreeMap<String, codex_thread_usage::Projection>, AppError> {
    let (input, sources, _) = cache.parts_at(conn, Instant::now())?;
    thread_usage_from_sources(&sources, listing, &input)
}
fn thread_usage_from_sources(
    sources: &[ThreadSource],
    listing: &[crate::session_manager::SessionMeta],
    input: &AnalyticsInput,
) -> Result<BTreeMap<String, codex_thread_usage::Projection>, AppError> {
    let by_path: BTreeMap<_, _> = sources.iter().map(|s| (s.path.as_str(), s)).collect();
    let current_models = if input.codex_native_ready {
        super::session_analytics_listing::current_thread_models(input)?
    } else {
        BTreeMap::new()
    };
    let mut projections: BTreeMap<String, codex_thread_usage::Projection> = BTreeMap::new();
    for meta in listing.iter().filter(|m| m.provider_id == "codex") {
        if !input.codex_native_ready {
            projections.insert(
                meta.session_id.clone(),
                codex_thread_usage::Projection::missing(&meta.session_id)
                    .unavailable("statisticsRebuildRequired"),
            );
            continue;
        }
        let source = meta
            .source_path
            .as_deref()
            .and_then(|path| by_path.get(path));
        let mut projection = source.map_or_else(
            || codex_thread_usage::Projection::missing(&meta.session_id),
            |s| s.projection.clone(),
        );
        let native = meta.session_id.clone();
        if input.codex_native_ready && projection.has_current_proof() {
            // Same deduplicated events as overview and analysis, never a second
            // token_count decoder nor per-physical-file sums for this thread.
            let session_key = key("codex", &native);
            let (models, incomplete, coverage_reasons) = current_models
                .get(&session_key)
                .cloned()
                .unwrap_or_default();
            projection.reasons.extend(coverage_reasons);
            projection.models = models;
            projection.incomplete_models = incomplete.into_iter().collect();
            projection.state = if projection.models.is_empty() {
                "unavailable"
            } else {
                "available"
            }
            .into();
            projection.facts_hash = Some(input.snapshot_version.clone());
            if !projection.incomplete_models.is_empty() {
                projection.reasons.push("incompleteUsageFact".into());
            }
        }
        if !projection.valid() || projection.native_thread_id.as_deref() != Some(&meta.session_id) {
            projection = projection.unavailable("threadUsageNotIndexed");
        } else if source.is_some_and(|s| !s.healthy) {
            projection = projection.unavailable("threadUsageSourceUnavailable");
        } else if let Some(source) = source {
            let current = usage_source::normalized_stamp("codex", Path::new(&source.path));
            if current.as_ref().ok().map(Value::to_string) != source.native_stamp {
                projection = projection.unavailable("threadUsageSourceChanged");
            }
        }
        match projections.entry(native) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(projection);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let consistent = input.codex_native_ready
                    && slot.get().state == projection.state
                    && slot.get().models == projection.models;
                if !consistent && slot.get() != &projection {
                    let rejected = slot.get().clone().unavailable("conflictingThreadSources");
                    slot.insert(rejected);
                }
            }
        }
    }
    Ok(projections)
}

/// Describe the *accepted* semantic records, not merely a readable file. This
/// preflight shares the later conflict rules and marks only affected origins.
fn note_thread_usage_gap(
    coverage: &mut BTreeMap<String, super::session_analytics_listing::ThreadUsageCoverage>,
    fact: &UsageFact,
    reason: &str,
) {
    if fact.tool_id != "codex" {
        return;
    }
    if let Some(thread) = &fact.session_key {
        let value = coverage.entry(thread.clone()).or_default();
        value.reasons.insert(reason.to_owned());
        value.incomplete_models.insert(fact.model.clone());
    }
}
fn reconcile_coverage(
    entries: &mut [(String, Entry)],
    coverage: &mut BTreeMap<String, super::session_analytics_listing::ThreadUsageCoverage>,
) {
    let mut facts = BTreeMap::new();
    let mut messages = BTreeMap::new();
    let mut activity = BTreeMap::new();
    let mut metadata = BTreeMap::new();
    let mut bad_facts = BTreeSet::new();
    let mut bad_messages = BTreeSet::new();
    let mut bad_activity = BTreeSet::new();
    let mut bad_metadata = BTreeSet::new();
    for (_, entry) in entries
        .iter()
        .filter(|(_, entry)| entry.available && entry.status != "failed")
    {
        for fact in &entry.facts {
            if facts
                .insert(fact.event_key.as_str(), fact)
                .is_some_and(|prior| prior != fact)
            {
                bad_facts.insert(fact.event_key.clone());
            }
        }
        for message in &entry.messages {
            let key = (message.session_key.as_str(), message.native_id.as_str());
            if messages
                .insert(key, message)
                .is_some_and(|prior| prior != message)
            {
                bad_messages.insert((message.session_key.clone(), message.native_id.clone()));
            }
        }
        for event in &entry.native_activity {
            let key = (event.session_key.as_str(), event.event_key.as_str());
            if activity
                .insert(key, (event.at_ms, event.main))
                .is_some_and(|prior| prior != (event.at_ms, event.main))
            {
                bad_activity.insert((event.session_key.clone(), event.event_key.clone()));
            }
        }
        for session in &entry.sessions {
            if metadata
                .insert(
                    session.identity.key.as_str(),
                    (entry.priority, &session.identity),
                )
                .is_some_and(|(priority, prior)| {
                    priority == entry.priority && prior != &session.identity
                })
            {
                bad_metadata.insert(session.identity.key.clone());
            }
        }
    }
    // Drop borrowed maps before updating their owners.
    drop((facts, messages, activity, metadata));
    for (_, entry) in entries
        .iter_mut()
        .filter(|(_, entry)| entry.available && entry.status != "failed")
    {
        for fact in entry
            .facts
            .iter()
            .filter(|fact| bad_facts.contains(&fact.event_key))
        {
            note_thread_usage_gap(coverage, fact, "conflictingUsageReplica");
        }
        let usage = entry.facts.iter().any(|f| bad_facts.contains(&f.event_key));
        let message = entry
            .messages
            .iter()
            .any(|m| bad_messages.contains(&(m.session_key.clone(), m.native_id.clone())));
        let runtime = entry
            .native_activity
            .iter()
            .any(|e| bad_activity.contains(&(e.session_key.clone(), e.event_key.clone())));
        let meta = entry
            .sessions
            .iter()
            .any(|s| bad_metadata.contains(&s.identity.key));
        for (affected, reason) in [
            (usage, "conflictingUsageReplica"),
            (message, "conflictingMessageReplica"),
            (runtime, "activityIdentityUnavailable"),
            (meta, "conflictingSessionMetadata"),
        ] {
            if affected {
                entry.status = "partial".into();
                if !entry.reasons.iter().any(|r| r == reason) {
                    entry.reasons.push(reason.into());
                }
            }
        }
    }
}

fn generation_is_current(entry: &Entry, epoch: &Option<String>) -> bool {
    entry.native_method
        && entry.status == "ok"
        && entry.stamp.as_ref().is_some_and(|stamp| {
            stamp["epoch"] == json!(epoch)
                && stamp["methodVersion"] == json!(super::codex_native::VERSION)
        })
}

fn input_from_rows(
    rows: BTreeMap<String, Row>,
    common: Vec<SourceHealth>,
    epoch: Option<String>,
) -> AnalyticsInput {
    let mut input = AnalyticsInput {
        snapshot_version: String::new(),
        sessions: vec![],
        facts: vec![],
        messages: vec![],
        reasons: vec![],
        source_reasons: vec![],
        sources: vec![],
        native_activity: vec![],
        source_observations: vec![],
        codex_native_ready: false,
        legacy_history_available: false,
        thread_usage_coverage: Default::default(),
    };
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(&epoch).unwrap_or_default());
    let mut entries = Vec::new();
    input.codex_native_ready = rows
        .get(rebuild::GENERATION_KEY)
        .and_then(|r| r.entry.as_ref().ok())
        .and_then(Option::as_ref)
        .is_some_and(|entry| generation_is_current(entry, &epoch));
    for (path, row) in rows {
        hash.update(path.as_bytes());
        hash.update(row.raw.as_bytes());
        if path == rebuild::GENERATION_KEY {
            continue;
        }
        match row.entry {
            Ok(Some(entry)) => {
                if row.provider == "codex" && input.codex_native_ready && !entry.native_method {
                    input
                        .source_observations
                        .push(super::session_analytics::SourceObservation {
                            tool_id: row.provider.clone(),
                            source_id: source_id(&row.provider).into(),
                            status: "partial".into(),
                            dated: false,
                        });
                    input.source_reasons.push(SourceReason {
                        tool_id: "codex".into(),
                        source_id: "codex_session".into(),
                        reason: "legacyCodexHistoryUnverified".into(),
                    });
                    continue;
                }
                entries.push((row.provider, entry));
            }
            Ok(None) => {}
            Err(()) => {
                input
                    .source_observations
                    .push(super::session_analytics::SourceObservation {
                        tool_id: row.provider.clone(),
                        source_id: source_id(&row.provider).into(),
                        status: "failed".into(),
                        dated: false,
                    });
                input.source_reasons.push(SourceReason {
                    tool_id: row.provider.clone(),
                    source_id: source_id(&row.provider).into(),
                    reason: "analyticsCacheUnavailable".into(),
                });
            }
        }
    }
    codex_index::reconcile(&mut entries, &mut input.thread_usage_coverage);
    reconcile_coverage(&mut entries, &mut input.thread_usage_coverage);
    for (provider, entry) in &entries {
        let count = entry
            .physical_source_count
            .unwrap_or(1)
            .max(usize::from(entry.status == "failed"));
        let dated = (!entry.facts.is_empty()
            || !entry.messages.is_empty()
            || !entry.native_activity.is_empty())
            && !entry.reasons.iter().any(|r| {
                matches!(
                    r.as_str(),
                    "missingTimestamp" | "usageTimeUnavailable" | "messageTimeUnavailable"
                )
            });
        input.source_observations.extend((0..count).map(|_| {
            super::session_analytics::SourceObservation {
                tool_id: provider.clone(),
                source_id: source_id(provider).into(),
                status: entry.status.clone(),
                dated,
            }
        }));
    }
    let has_obsolete_entries = entries
        .iter()
        .any(|(_, entry)| !entry.available || entry.status == "failed");
    let current_sessions: BTreeSet<_> = entries
        .iter()
        .filter(|(_, e)| has_obsolete_entries && e.available && e.status != "failed")
        .flat_map(|(_, e)| e.sessions.iter().map(|s| s.identity.key.clone()))
        .collect();
    let current_facts: BTreeSet<_> = entries
        .iter()
        .filter(|(_, e)| has_obsolete_entries && e.available && e.status != "failed")
        .flat_map(|(_, e)| e.facts.iter().map(|f| f.event_key.clone()))
        .collect();
    let current_messages: BTreeSet<_> = entries
        .iter()
        .filter(|(_, e)| has_obsolete_entries && e.available && e.status != "failed")
        .flat_map(|(_, e)| {
            e.messages
                .iter()
                .map(|m| (m.session_key.clone(), m.native_id.clone()))
        })
        .collect();
    entries.sort_by_key(|(_, entry)| (entry.available && entry.status != "failed", entry.priority));
    let mut sessions: BTreeMap<String, SessionDescriptor> = BTreeMap::new();
    let mut priorities: BTreeMap<String, u8> = BTreeMap::new();
    let mut metadata_conflicts = BTreeSet::new();
    let mut message_conflicts = BTreeSet::new();
    let mut message_conflict_sessions = BTreeSet::new();
    let mut facts: BTreeMap<String, UsageFact> = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    let mut messages: BTreeMap<(String, String), MessageEvent> = BTreeMap::new();
    let mut health: BTreeMap<String, SourceHealth> = BTreeMap::new();
    for (provider, mut entry) in entries {
        if !entry.available || entry.status == "failed" {
            // Preserve absent history, but a currently observed native identity
            // supersedes an obsolete cached copy of that same identity. Only
            // simultaneous current copies can disagree as native replicas.
            entry
                .sessions
                .retain(|s| !current_sessions.contains(&s.identity.key));
            entry
                .facts
                .retain(|f| !current_facts.contains(&f.event_key));
            entry.messages.retain(|m| {
                !current_messages.contains(&(m.session_key.clone(), m.native_id.clone()))
            });
        }
        let source = source_id(&provider).to_owned();
        for reason in &entry.reasons {
            input.source_reasons.push(SourceReason {
                tool_id: provider.clone(),
                source_id: source.clone(),
                reason: reason.clone(),
            });
        }
        let health = health
            .entry(source.clone())
            .or_insert_with(|| SourceHealth {
                source_id: source,
                tool_id: provider.clone(),
                status: "ok".into(),
                last_attempt_at_ms: None,
                last_success_at_ms: entry.succeeded_at_ms,
                error_code: None,
                last_scan: None,
                retained_detail_range: None,
            });
        if entry.status != "ok" {
            if entry.status == "failed" || health.status != "failed" {
                health.status = if entry.status == "failed" {
                    "failed"
                } else {
                    "partial"
                }
                .into();
            }
            health.error_code = Some(
                if entry.status == "failed" {
                    "sourceReadFailed"
                } else {
                    "partialSource"
                }
                .into(),
            );
        }
        health.last_attempt_at_ms = Some(
            health
                .last_attempt_at_ms
                .unwrap_or(i64::MIN)
                .max(entry.attempted_at_ms),
        );
        health.last_success_at_ms = health
            .last_success_at_ms
            .zip(entry.succeeded_at_ms)
            .map(|(a, b)| a.min(b));
        for mut session in entry.sessions {
            let id = session.identity.key.clone();
            if let Some(prior) = sessions.get(&id) {
                if priorities.get(&id) == Some(&entry.priority)
                    && prior.identity != session.identity
                {
                    metadata_conflicts.insert(id.clone());
                    input.source_reasons.push(SourceReason {
                        tool_id: provider.clone(),
                        source_id: source_id(&provider).into(),
                        reason: "conflictingSessionMetadata".into(),
                    });
                }
            }
            if metadata_conflicts.contains(&id) {
                session.identity.project = Project::Unparsed;
                session.identity.verified_root_key = None;
                session.project_path = None;
                session.project_label = None;
                session.messages_supported = false;
                session
                    .message_reasons
                    .push("conflictingSessionMetadata".into());
            }
            priorities.insert(id.clone(), entry.priority);
            sessions.insert(id, session);
        }
        for fact in entry.facts {
            if let Some(prior) = facts.get(&fact.event_key) {
                if prior != &fact {
                    conflicts.insert(fact.event_key.clone());
                }
            }
            facts.insert(fact.event_key.clone(), fact);
        }
        input.native_activity.extend(entry.native_activity);
        for message in entry.messages {
            let k = (message.session_key.clone(), message.native_id.clone());
            if message_conflicts.contains(&k) {
                continue;
            }
            if let Some(prior) = messages.get(&k) {
                if prior != &message {
                    message_conflicts.insert(k.clone());
                    message_conflict_sessions.insert(message.session_key.clone());
                    messages.remove(&k);
                    input.source_reasons.push(SourceReason {
                        tool_id: provider.clone(),
                        source_id: source_id(&provider).into(),
                        reason: "conflictingMessageReplica".into(),
                    });
                    continue;
                }
            }
            messages.insert(k, message);
        }
    }
    for id in conflicts {
        if let Some(fact) = facts.remove(&id) {
            input.source_reasons.push(SourceReason {
                tool_id: fact.tool_id,
                source_id: fact.source_id,
                reason: "conflictingUsageReplica".into(),
            });
        }
    }
    let mut seen_activity = BTreeMap::new();
    let mut bad_activity = BTreeSet::new();
    for e in &input.native_activity {
        let id = (e.session_key.clone(), e.event_key.clone());
        if seen_activity
            .insert(id.clone(), (e.at_ms, e.main))
            .is_some_and(|prior| prior != (e.at_ms, e.main))
        {
            bad_activity.insert(id);
        }
    }
    if !bad_activity.is_empty() {
        input
            .native_activity
            .retain(|e| !bad_activity.contains(&(e.session_key.clone(), e.event_key.clone())));
        for (key, _) in bad_activity {
            if let Some(s) = sessions.get_mut(&key) {
                s.message_reasons.push("activityIdentityUnavailable".into());
                input.source_reasons.push(SourceReason {
                    tool_id: s.identity.tool_id.clone(),
                    source_id: source_id(&s.identity.tool_id).into(),
                    reason: "activityIdentityUnavailable".into(),
                });
            }
        }
    }
    let keys: BTreeSet<_> = sessions.keys().cloned().collect();
    for session in sessions.values_mut() {
        if message_conflict_sessions.contains(&session.identity.key) {
            session.messages_supported = false;
            session
                .message_reasons
                .push("conflictingMessageReplica".into());
        }
        if session
            .identity
            .verified_root_key
            .as_ref()
            .is_some_and(|key| !keys.contains(key))
        {
            session.identity.verified_root_key = None;
            session.messages_supported = false;
            session
                .message_reasons
                .push("sessionParentUnavailable".into());
            input.source_reasons.push(SourceReason {
                tool_id: session.identity.tool_id.clone(),
                source_id: source_id(&session.identity.tool_id).into(),
                reason: "sessionParentUnavailable".into(),
            });
        }
    }
    isolate_invalid_roots(&mut sessions, &mut input.source_reasons);
    input.sessions = sessions.into_values().collect();
    input.facts = facts.into_values().collect();
    input.messages = messages.into_values().collect();
    // Read the common health under this query's existing SQLite snapshot.
    // A successful old cache never upgrades an unattempted/failed channel.
    for mut source in common
        .into_iter()
        .filter(|s| source_id(&s.tool_id) != "unsupported")
    {
        if let Some(cached) = health.remove(&source.source_id) {
            if health_severity(&cached.status) > health_severity(&source.status) {
                source.status = cached.status;
                source.error_code = cached.error_code;
            }
            source.last_attempt_at_ms = source
                .last_attempt_at_ms
                .into_iter()
                .chain(cached.last_attempt_at_ms)
                .max();
            source.last_success_at_ms = source
                .last_success_at_ms
                .zip(cached.last_success_at_ms)
                .map(|(a, b)| a.min(b));
        }
        input.sources.push(source);
    }
    input.sources.extend(health.into_values());
    if input.sessions.is_empty() && input.source_reasons.is_empty() {
        input.reasons.push("analyticsIndexEmpty".into());
    }
    input.source_reasons.sort_by(|a, b| {
        (&a.tool_id, &a.source_id, &a.reason).cmp(&(&b.tool_id, &b.source_id, &b.reason))
    });
    input.source_reasons.dedup_by(|a, b| {
        a.tool_id == b.tool_id && a.source_id == b.source_id && a.reason == b.reason
    });
    input.snapshot_version = format!("{:x}", hash.finalize());
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::source_cache::{self, Part};
    use serde_json::json;

    #[test]
    fn analytics_envelope_projection_matches_shared_decoder_compatibility_and_refusals() {
        fn shared(raw: &str) -> (Option<String>, Result<Option<Entry>, ()>) {
            match source_cache::decode_row(
                (Some("codex".into()), None, None, Some(raw.into())),
                "codex",
            ) {
                Ok(parts) => {
                    let entry = parts
                        .value(Part::Analytics)
                        .map(|value| {
                            let entry = Entry::deserialize(value).map_err(|_| ())?;
                            if entry.version != VERSION {
                                return Err(());
                            }
                            Ok(entry)
                        })
                        .transpose();
                    (parts.token(Part::Analytics), entry)
                }
                Err(_) => (None, Err(())),
            }
        }
        let entry = json!({"version":1,"stamp":{"futureStamp":{"keep":true}},"sessions":[],"facts":[],"messages":[],"reasons":[],"available":true,"status":"ok","priority":2,"attemptedAtMs":1,"succeededAtMs":1,"futureEntry":"opaque"});
        let canonical = json!({"formatVersion":1,"parts":{
            "analytics":{"revision":"analytics-r","value":entry},
            "metadata":{"revision":"meta-r","value":{"futureVersion":999}},
            "usageSync":{"revision":"usage-r","value":{"version":999,"opaque":[1,2]}},
            "lifetimeUsage":{"revision":"life-r","value":null},
            "futurePart":{"unrecognized":"opaque"}
        },"futureRoot":true});
        let mut cases = vec![
            canonical.to_string(),
            "invalid JSON".into(),
            json!({"candidates":{},"messageCount":3}).to_string(),
            "{}".into(),
            json!([1, {}]).to_string(),
        ];
        let mut sequence_parts = canonical.clone();
        sequence_parts["parts"] = json!([]);
        cases.push(sequence_parts.to_string());
        for part in ["metadata", "usageSync", "analytics", "lifetimeUsage"] {
            let mut sequence_wrapper = canonical.clone();
            let value = sequence_wrapper["parts"][part]["value"].clone();
            sequence_wrapper["parts"][part] = json!(["sequence-r", value]);
            cases.push(sequence_wrapper.to_string());
        }
        for field in ["formatVersion", "parts"] {
            let mut value = canonical.clone();
            value.as_object_mut().unwrap().remove(field);
            cases.push(value.to_string());
        }
        for format in [json!(2), Value::Null, json!("1")] {
            let mut value = canonical.clone();
            value["formatVersion"] = format;
            cases.push(value.to_string());
        }
        for part in ["metadata", "usageSync", "analytics", "lifetimeUsage"] {
            for field in ["revision", "value"] {
                let mut value = canonical.clone();
                value["parts"][part].as_object_mut().unwrap().remove(field);
                cases.push(value.to_string());
            }
            for wrapper in [Value::Null, json!(7), json!({"revision":"","value":null})] {
                let mut value = canonical.clone();
                value["parts"][part] = wrapper;
                cases.push(value.to_string());
            }
        }
        let mut cleared = canonical.clone();
        cleared["parts"]["analytics"]["value"] = Value::Null;
        cases.push(cleared.to_string());
        let mut future = canonical.clone();
        future["parts"]["analytics"]["value"]["version"] = json!(2);
        cases.push(future.to_string());
        let mut absent = canonical.clone();
        absent["parts"].as_object_mut().unwrap().remove("analytics");
        cases.push(absent.to_string());
        // Shared Value decoding accepts duplicate object keys with the final
        // value winning; a typed parser must preserve that fallback behavior.
        cases.push(canonical.to_string().replace(
            "\"formatVersion\":1",
            "\"formatVersion\":0,\"formatVersion\":1",
        ));
        for replacement in [
            "1e400".to_owned(),
            format!("{}0{}", "[".repeat(140), "]".repeat(140)),
        ] {
            for needle in [
                "\"futureRoot\":true",
                "\"futureVersion\":999",
                "\"futureEntry\":\"opaque\"",
            ] {
                let raw = canonical.to_string();
                assert!(raw.contains(needle));
                let key = needle.split_once(':').unwrap().0;
                let raw = raw.replace(needle, &format!("{key}:{replacement}"));
                assert!(
                    shared(&raw).1.is_err(),
                    "shared decoder must reject malformed opaque values"
                );
                cases.push(raw);
            }
        }
        let mut large = canonical.clone();
        large["padding"] = json!("x".repeat(source_cache::MAX_ENVELOPE_BYTES));
        cases.push(large.to_string());
        for (index, raw) in cases.iter().enumerate() {
            let expected = serde_json::to_value(shared(raw)).unwrap();
            let actual =
                serde_json::to_value(decode_analytics_row("codex", None, None, raw)).unwrap();
            assert_eq!(actual, expected, "envelope case {index}");
        }
    }

    fn write(path: &std::path::Path, value: &serde_json::Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }
    fn row(sid: &str, id: &str, tokens: u64) -> serde_json::Value {
        json!({"type":"assistant","sessionId":sid,"cwd":"/project","timestamp":"2026-10-15T00:00:00.123Z",
            "message":{"id":id,"role":"assistant","model":"m","content":[{"type":"text","text":"not retained in cache"}],
                "usage":{"input_tokens":tokens,"output_tokens":2}}})
    }
    fn spec(path: &std::path::Path) -> SourceSpec {
        SourceSpec {
            provider: "claude".into(),
            path: path.to_owned(),
            subagent: false,
            project_file: None,
            title_file: None,
        }
    }

    #[test]
    fn analytics_projection_ttl_is_absolute_and_reuses_arc_until_exact_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("session.jsonl");
        write(&path, &row("ttl", "event", 3));
        refresh_sources(&db, &[spec(&path)]).unwrap();
        let cache = ProjectionCache::default();
        let start = Instant::now();
        let conn = db.conn.lock().unwrap();
        let first = cache.read_at(&conn, start).unwrap();
        reset_query_decode_count();
        let warm = cache
            .read_at(
                &conn,
                start + PROJECTION_CACHE_TTL - Duration::from_nanos(1),
            )
            .unwrap();
        assert!(Arc::ptr_eq(&first, &warm));
        assert_eq!(query_decode_count(), 0);
        let expired = cache.read_at(&conn, start + PROJECTION_CACHE_TTL).unwrap();
        assert!(!Arc::ptr_eq(&first, &expired));
        assert_eq!(query_decode_count(), 1);
        assert_eq!(
            serde_json::to_value(&*first).unwrap(),
            serde_json::to_value(&*expired).unwrap()
        );
    }

    #[test]
    fn analytics_projection_retention_budget_counts_owned_capacities_and_refuses_oversized_input() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("session.jsonl");
        write(&path, &row("budget", "event", 3));
        refresh_sources(&db, &[spec(&path)]).unwrap();
        let conn = db.conn.lock().unwrap();
        let baseline = read_input_on(&conn).unwrap();
        let bytes = projection_bytes(&baseline).unwrap();
        assert!(bytes > std::mem::size_of::<AnalyticsInput>());
        let cache = ProjectionCache {
            max_bytes: bytes - 1,
            ..ProjectionCache::default()
        };
        reset_query_decode_count();
        let first = cache.read_on(&conn).unwrap();
        let second = cache.read_on(&conn).unwrap();
        assert_eq!(query_decode_count(), 2);
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(cache.slot.lock().unwrap().is_none());
        assert_eq!(
            first.facts[0].buckets[0], 3,
            "budget refusal must not drop the query's input"
        );

        let exact = ProjectionCache {
            max_bytes: bytes,
            ..ProjectionCache::default()
        };
        let retained = exact.read_on(&conn).unwrap();
        assert!(Arc::ptr_eq(&retained, &exact.read_on(&conn).unwrap()));

        let mut spare = baseline.clone();
        let before = projection_bytes(&spare).unwrap();
        let old_capacity = spare.sessions[0].title.capacity();
        spare.sessions[0].title.reserve(4096);
        assert_eq!(
            projection_bytes(&spare).unwrap() - before,
            spare.sessions[0].title.capacity() - old_capacity
        );
        spare.facts.reserve(500);
        assert!(
            projection_bytes(&spare).unwrap() >= before + 500 * std::mem::size_of::<UsageFact>()
        );
        assert!(!serde_json::to_string(&*retained)
            .unwrap()
            .contains("not retained in cache"));
    }

    #[test]
    fn analytics_projection_warm_slot_cannot_hide_raw_source_count_budget_failure() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        let cache = ProjectionCache::default();
        let empty = cache.read_on(&conn).unwrap();
        assert!(empty.facts.is_empty());
        conn.execute("WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i+1 FROM n WHERE i<20000) INSERT INTO session_meta_cache(source_path,provider_id,meta_json,updated_at) SELECT 'source-'||i,'codex','',0 FROM n", []).unwrap();
        assert!(cache.read_on(&conn).is_err());
    }

    fn oc_db(path: &Path) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE session(id TEXT PRIMARY KEY,parent_id TEXT,title TEXT,directory TEXT,time_created INTEGER,time_updated INTEGER);CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT); INSERT INTO session VALUES('ses_a',NULL,'A','/one/project',1,1),('ses_b',NULL,'B','/two/project',1,1);").unwrap();
        for (id, sid, input) in [("ma", "ses_a", 3), ("mb", "ses_b", 4)] {
            let data = json!({"id":id,"sessionID":sid,"role":"assistant","modelID":"m","tokens":{"input":input,"output":2},"time":{"created":1792022400123_i64}});
            conn.execute(
                "INSERT INTO message VALUES(?1,?2,1792022400123,1792022400123,?3)",
                rusqlite::params![id, sid, data.to_string()],
            )
            .unwrap();
        }
        conn
    }
    #[test]
    fn session_analytics_index_conflicting_project_replicas_do_not_choose_by_path_order() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        let mut one = row("s", "m", 3);
        one["cwd"] = json!("/a");
        write(&a, &one);
        one["cwd"] = json!("/b");
        write(&b, &one);
        refresh_sources(&db, &[spec(&a), spec(&b)]).unwrap();
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        assert_eq!(input.sessions.len(), 1);
        assert_eq!(input.sessions[0].identity.project, Project::Unparsed);
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "conflictingSessionMetadata"));
    }
    #[test]
    fn session_analytics_index_conflicting_message_times_are_unknown_not_first_replica_wins() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        for (path, time) in [(&a, "2026-10-14T00:00:00Z"), (&b, "2026-10-15T00:00:00Z")] {
            let user = json!({"type":"user","sessionId":"s","cwd":"/project","uuid":"same-user","timestamp":time,"message":{"role":"user","content":"question"}});
            std::fs::write(path, format!("{}\n{}", user, row("s", "m", 3))).unwrap();
        }
        refresh_sources(&db, &[spec(&a), spec(&b)]).unwrap();
        let input = read_input(&db).unwrap();
        assert!(!input
            .messages
            .iter()
            .any(|m| m.native_id == "user:same-user"));
        assert!(!input.sessions[0].messages_supported);
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "conflictingMessageReplica"));
    }
    #[test]
    fn session_analytics_index_opencode_shared_database_keeps_all_sessions_and_wal_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let native = oc_db(&path);
        let db = Database::memory().unwrap();
        let specs = [SourceSpec {
            provider: "opencode".into(),
            path,
            subagent: false,
            project_file: None,
            title_file: None,
        }];
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        let input = read_input(&db).unwrap();
        assert_eq!(input.sessions.len(), 2);
        assert_eq!(input.facts.len(), 2);
        assert!(input
            .sessions
            .iter()
            .any(|s| s.project_path.as_deref() == Some("/two/project")));
        let mut data: Value = serde_json::from_str(
            &native
                .query_row::<String, _, _>("SELECT data FROM message WHERE id='ma'", [], |r| {
                    r.get(0)
                })
                .unwrap(),
        )
        .unwrap();
        data["tokens"]["input"] = json!(20);
        native
            .execute(
                "UPDATE message SET data=?1 WHERE id='ma'",
                [data.to_string()],
            )
            .unwrap();
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        assert_eq!(
            read_input(&db)
                .unwrap()
                .facts
                .iter()
                .map(|f| f.buckets[0])
                .sum::<u64>(),
            24
        );
    }
    #[test]
    fn session_analytics_index_opencode_parent_table_only_change_refreshes_root_relation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let native = oc_db(&path);
        let db = Database::memory().unwrap();
        let specs = [SourceSpec {
            provider: "opencode".into(),
            path,
            subagent: false,
            project_file: None,
            title_file: None,
        }];
        refresh_sources(&db, &specs).unwrap();
        native
            .execute("UPDATE session SET parent_id='ses_a' WHERE id='ses_b'", [])
            .unwrap();
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        let input = read_input(&db).unwrap();
        let parent = input
            .sessions
            .iter()
            .find(|s| s.native_session_id == "ses_a")
            .unwrap();
        let child = input
            .sessions
            .iter()
            .find(|s| s.native_session_id == "ses_b")
            .unwrap();
        assert_eq!(
            child.identity.verified_root_key.as_deref(),
            Some(parent.identity.key.as_str())
        );
    }
    #[test]
    fn session_analytics_index_opencode_storage_metadata_is_a_dependency() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage");
        let db = Database::memory().unwrap();
        let session = path.join("session/project/ses_a.json");
        write(
            &session,
            &json!({"id":"ses_a","directory":"/one/project ","title":"A"}),
        );
        write(
            &path.join("message/ses_a/ma.json"),
            &json!({"id":"ma","sessionID":"ses_a","role":"assistant","modelID":"m","tokens":{"input":3,"output":2},"time":{"created":1792022400123_i64}}),
        );
        let specs = [SourceSpec {
            provider: "opencode".into(),
            path,
            subagent: false,
            project_file: None,
            title_file: None,
        }];
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        assert_eq!(
            read_input(&db).unwrap().sessions[0].project_path.as_deref(),
            Some("/one/project ")
        );
        write(
            &session,
            &json!({"id":"ses_a","directory":"/two/project","title":"B"}),
        );
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        assert_eq!(
            read_input(&db).unwrap().sessions[0].project_path.as_deref(),
            Some("/two/project")
        );
    }
    #[test]
    fn session_analytics_index_six_hundred_sources_reuse_cache_without_parsing_and_query_is_read_only(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let specs: Vec<_> = (0..601)
            .map(|i| {
                let path = dir.path().join(format!("{i}.jsonl"));
                write(&path, &row(&format!("s{i}"), &format!("m{i}"), 3));
                spec(&path)
            })
            .collect();
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 601);
        let second = refresh_sources(&db, &specs).unwrap();
        assert_eq!(second.parsed, 0);
        assert_eq!(second.unchanged, 601);
        let before: Vec<(String, String)> = db
            .conn
            .lock()
            .unwrap()
            .prepare("SELECT source_path,meta_json FROM session_meta_cache ORDER BY source_path")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let input = read_input(&db).unwrap();
        assert_eq!(input.sessions.len(), 601);
        assert_eq!(input.facts.len(), 601);
        assert_eq!(input.facts[0].at_ms, 1_792_022_400_123);
        let after: Vec<(String, String)> = db
            .conn
            .lock()
            .unwrap()
            .prepare("SELECT source_path,meta_json FROM session_meta_cache ORDER BY source_path")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(before, after);
        assert!(before
            .iter()
            .all(|(_, v)| !v.contains("not retained in cache")));
    }
    #[test]
    fn session_analytics_index_changed_source_replaces_contribution_and_metadata_survives() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("a.jsonl");
        write(&path, &row("s", "m", 3));
        let specs = [spec(&path)];
        refresh_sources(&db, &specs).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            source_cache::merge_on(&conn,path.to_str().unwrap(),"claude",Some("s"),Part::Metadata,&None,Some(json!({"parserVersion":crate::session_manager::meta_cache::PARSER_VERSION,"candidates":{"custom":"user title"}}))).unwrap();
        }
        write(&path, &row("s", "m", 999));
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        assert_eq!(input.facts[0].buckets[0], 999);
        let conn = db.conn.lock().unwrap();
        let parts = source_cache::read_on(&conn, path.to_str().unwrap(), "claude").unwrap();
        assert_eq!(
            parts.value(Part::Metadata).unwrap()["candidates"]["custom"],
            "user title"
        );
    }
    #[test]
    fn session_analytics_index_missing_source_retains_known_facts_and_marks_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("a.jsonl");
        write(&path, &row("s", "m", 3));
        refresh_sources(&db, &[spec(&path)]).unwrap();
        std::fs::remove_file(&path).unwrap();
        refresh_sources(&db, &[]).unwrap();
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        assert_eq!(input.sessions.len(), 1);
        assert!(!input.sessions[0].source_available);
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "sourceMissing"));
    }
    #[test]
    fn session_analytics_index_one_corrupt_source_preserves_last_success_and_other_source_updates()
    {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        write(&a, &row("a", "ma", 3));
        write(&b, &row("b", "mb", 4));
        let specs = [spec(&a), spec(&b)];
        refresh_sources(&db, &specs).unwrap();
        std::fs::write(&a, "{broken private payload").unwrap();
        write(&b, &row("b", "mb", 9));
        let result = refresh_sources(&db, &specs).unwrap();
        assert_eq!(result.failed, 1);
        assert_eq!(result.parsed, 1);
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 2);
        assert_eq!(input.facts.iter().map(|f| f.buckets[0]).sum::<u64>(), 12);
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "sourceReadFailed"));
        assert!(!format!("{input:?}").contains("private payload"));
    }
    #[test]
    fn session_analytics_index_future_owned_version_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("a.jsonl");
        write(&path, &row("s", "m", 3));
        {
            let conn = db.conn.lock().unwrap();
            source_cache::merge_on(
                &conn,
                path.to_str().unwrap(),
                "claude",
                Some("s"),
                Part::Analytics,
                &None,
                Some(json!({"version":99,"future":"preserve"})),
            )
            .unwrap();
        }
        let report = refresh_sources(&db, &[spec(&path)]).unwrap();
        assert_eq!(report.failed, 1);
        let conn = db.conn.lock().unwrap();
        let parts = source_cache::read_on(&conn, path.to_str().unwrap(), "claude").unwrap();
        assert_eq!(parts.value(Part::Analytics).unwrap()["version"], 99);
        assert_eq!(parts.value(Part::Analytics).unwrap()["future"], "preserve");
    }
    #[test]
    fn session_analytics_index_gemini_project_dependency_change_preserves_path_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("chats/session-a.json");
        let project = dir.path().join(".project_root");
        write(
            &path,
            &json!({"sessionId":"s","messages":[{"id":"m","type":"gemini","model":"m","timestamp":"2026-10-15T00:00:00Z","content":"reply","tokens":{"input":3,"output":2}}]}),
        );
        std::fs::write(&project, "/one/project").unwrap();
        let specs = [SourceSpec {
            provider: "gemini".into(),
            path,
            subagent: false,
            project_file: Some(project.clone()),
            title_file: None,
        }];
        refresh_sources(&db, &specs).unwrap();
        std::fs::write(&project, "/one/project ").unwrap();
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        assert_eq!(
            read_input(&db).unwrap().sessions[0].project_path.as_deref(),
            Some("/one/project ")
        );
    }
    #[test]
    fn session_analytics_index_project_ids_are_bounded_exact_paths_not_basenames() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let paths = [
            format!("/one/{}/project ", "long/".repeat(150)),
            "/two/project ".into(),
            "/two/project".into(),
        ];
        let specs: Vec<_> = paths
            .iter()
            .enumerate()
            .map(|(i, project)| {
                let file = dir.path().join(format!("{i}.jsonl"));
                let mut value = row(&format!("s{i}"), &format!("m{i}"), 3);
                value["cwd"] = json!(project);
                write(&file, &value);
                spec(&file)
            })
            .collect();
        refresh_sources(&db, &specs).unwrap();
        let input = read_input(&db).unwrap();
        let mut ids = BTreeSet::new();
        for session in &input.sessions {
            let Project::Known(id) = &session.identity.project else {
                panic!("known native path");
            };
            assert!(
                id.len() <= 512,
                "project filter must satisfy query ID limit"
            );
            assert_ne!(Some(id.as_str()), session.project_path.as_deref());
            assert!(paths
                .iter()
                .any(|p| Some(p.as_str()) == session.project_path.as_deref()));
            ids.insert(id.clone());
        }
        assert_eq!(
            ids.len(),
            3,
            "same basename and significant whitespace remain distinct"
        );
    }
    #[test]
    fn session_analytics_index_titles_follow_native_candidates_for_each_text_provider() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let claude = dir.path().join("claude.jsonl");
        std::fs::write(&claude, [
            json!({"type":"user","sessionId":"c","cwd":"/project","uuid":"u","timestamp":"2026-10-15T00:00:00Z","message":{"role":"user","content":"<context>metadata</context>\n[Image #1]\nActual request"}}),
            json!({"type":"custom-title","customTitle":"Native renamed title"}), row("c","m",3)
        ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let codex = dir.path().join("codex.jsonl");
        std::fs::write(&codex,[
            json!({"type":"session_meta","payload":{"id":"x","cwd":"/project","source":"cli"}}),
            json!({"type":"response_item","timestamp":"2026-10-15T00:00:00Z","payload":{"type":"message","role":"user","id":"u","content":[{"type":"input_text","text":"<div>How should this render?</div>"}]}})
        ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let gemini = dir.path().join("gemini.json");
        write(
            &gemini,
            &json!({"sessionId":"g","messages":[{"id":"u","type":"user","timestamp":"2026-10-15T00:00:00Z","content":"Explain the Gemini renderer"}]}),
        );
        let mut specs = vec![spec(&claude), spec(&codex), spec(&gemini)];
        specs[1].provider = "codex".into();
        specs[2].provider = "gemini".into();
        refresh_sources(&db, &specs).unwrap();
        let input = read_input(&db).unwrap();
        let titles: BTreeMap<_, _> = input
            .sessions
            .iter()
            .map(|s| (s.native_session_id.as_str(), s.title.as_str()))
            .collect();
        assert_eq!(titles.get("c"), Some(&"Native renamed title"));
        assert!(titles.get("x").unwrap().contains("How should this render?"));
        assert_eq!(titles.get("g"), Some(&"Explain the Gemini renderer"));
    }
    #[test]
    fn session_analytics_index_opencode_filename_is_not_native_message_identity() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let storage = dir.path().join("storage");
        write(
            &storage.join("session/p/s.json"),
            &json!({"id":"s","directory":"/p","title":"S"}),
        );
        for name in ["copy-a", "copy-b"] {
            write(
                &storage.join(format!("message/s/{name}.json")),
                &json!({"sessionID":"s","role":"assistant","modelID":"m","tokens":{"input":3,"output":2},"time":{"created":1792022400123_i64},"parts":[{"type":"text","text":"answer"}]}),
            );
        }
        let mut source = spec(&storage);
        source.provider = "opencode".into();
        refresh_sources(&db, &[source]).unwrap();
        let input = read_input(&db).unwrap();
        assert!(
            input.facts.is_empty(),
            "copies without native IDs cannot become independent usage calls"
        );
        assert!(
            input.messages.is_empty(),
            "filenames cannot certify message identity"
        );
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "missingNativeEventIdentity"));
        assert!(!input.sessions[0].messages_supported);
    }

    #[test]
    fn session_analytics_index_codex_name_index_is_a_revision_dependency() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("s.jsonl");
        let names = dir.path().join("session_index.jsonl");
        std::fs::write(&path,[json!({"type":"session_meta","payload":{"id":"x","cwd":"/p","source":"cli"}}),json!({"type":"event_msg","payload":{"type":"user_message","message":"First question"}})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let mut source = spec(&path);
        source.provider = "codex".into();
        source.title_file = Some(names.clone());
        refresh_sources(&db, &[source.clone()]).unwrap();
        let before = source_revision(&source).unwrap();
        std::fs::write(
            &names,
            json!({"id":"x","thread_name":"Native thread name"}).to_string(),
        )
        .unwrap();
        assert_ne!(
            before,
            source_revision(&source).unwrap(),
            "metadata-only update must invalidate analytics cache"
        );
        assert_eq!(refresh_sources(&db, &[source]).unwrap().parsed, 1);
        assert_eq!(
            read_input(&db).unwrap().sessions[0].title,
            "Native thread name"
        );
    }
    #[test]
    fn session_analytics_index_failed_discovery_preserves_last_success_without_claiming_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let claude = dir.path().join("c.jsonl");
        let codex = dir.path().join("x.jsonl");
        write(&claude, &row("c", "m", 3));
        std::fs::write(&codex,[json!({"type":"session_meta","payload":{"id":"x","cwd":"/p","source":"cli"}}),json!({"type":"turn_context","payload":{"model":"fixture"}}),json!({"type":"event_msg","timestamp":"2026-10-15T00:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":2}}}})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let mut x = spec(&codex);
        x.provider = "codex".into();
        refresh_sources(&db, &[spec(&claude), x.clone()]).unwrap();
        let discovery = Discovery {
            roots: vec![],
            require_route_match: false,
            specs: vec![x],
            batches: vec![],
            failures: vec![DiscoveryFailure {
                provider: "claude".into(),
                code: "sourceDiscoveryFailed",
            }],
        };
        let report = refresh_discovered(&db, &discovery).unwrap();
        assert!(report.failed > 0);
        let input = read_input(&db).unwrap();
        let retained = input
            .sessions
            .iter()
            .find(|s| s.native_session_id == "c")
            .unwrap();
        assert!(
            retained.source_available,
            "an unreadable roster is not evidence of deletion"
        );
        assert!(retained.manager_key.is_some());
        assert!(input
            .facts
            .iter()
            .any(|f| f.tool_id == "claude" && f.buckets[0] == 3));
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.tool_id == "claude" && r.reason == "sourceDiscoveryFailed"));
        assert!(!input
            .source_reasons
            .iter()
            .any(|r| r.tool_id == "claude" && r.reason == "sourceMissing"));
    }
    #[test]
    fn session_analytics_index_cache_success_cannot_hide_common_failed_or_pending_health() {
        use crate::services::usage_health::{self, Channel, Outcome};
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("c.jsonl");
        write(&path, &row("c", "m", 3));
        refresh_sources(&db, &[spec(&path)]).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let ledger =
                usage_health::begin_on(&conn, "claude", "session_log", Channel::Ledger, 1000)
                    .unwrap();
            usage_health::finish_on(&conn, &ledger, Outcome::Ok, 1100).unwrap();
            let analytics =
                usage_health::begin_on(&conn, "claude", "session_log", Channel::Analytics, 1200)
                    .unwrap();
            usage_health::finish_on(
                &conn,
                &analytics,
                Outcome::Failed("usage.sources.readFailed"),
                1300,
            )
            .unwrap();
        }
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        let source = input
            .sources
            .iter()
            .find(|s| s.tool_id == "claude")
            .unwrap();
        assert_eq!(source.status, "failed");
        assert_eq!(source.last_success_at_ms, None);
        {
            let conn = db.conn.lock().unwrap();
            usage_health::begin_on(&conn, "claude", "session_log", Channel::Analytics, 1400)
                .unwrap();
        }
        let pending = read_input(&db).unwrap();
        assert_eq!(
            pending
                .sources
                .iter()
                .find(|s| s.tool_id == "claude")
                .unwrap()
                .status,
            "unknown"
        );
    }
    #[test]
    fn session_analytics_index_corrupt_common_health_remains_unknown_and_keeps_native_cache() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("c.jsonl");
        write(&path, &row("c", "m", 3));
        refresh_sources(&db, &[spec(&path)]).unwrap();
        db.set_setting(
            crate::services::usage_health::SETTING_KEY,
            "not valid health",
        )
        .unwrap();
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        let source = input
            .sources
            .iter()
            .find(|s| s.tool_id == "claude")
            .unwrap();
        assert_eq!(source.status, "unknown");
        assert_eq!(source.last_success_at_ms, None);
        assert_eq!(
            source.error_code.as_deref(),
            Some("usage.sources.invalidHealth")
        );
    }
    #[test]
    fn session_analytics_index_invalid_native_parent_cycle_is_scoped_and_does_not_block_a_healthy_tool(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let native = dir.path().join("opencode.db");
        let oc = oc_db(&native);
        oc.execute_batch(
            "UPDATE session SET parent_id=CASE id WHEN 'ses_a' THEN 'ses_b' ELSE 'ses_a' END",
        )
        .unwrap();
        let codex = dir.path().join("x.jsonl");
        std::fs::write(&codex,[json!({"type":"session_meta","payload":{"id":"x","cwd":"/p","source":"cli"}}),json!({"type":"turn_context","payload":{"model":"fixture"}}),json!({"type":"event_msg","timestamp":"2026-10-15T00:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":2}}}})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let mut oc_spec = spec(&native);
        oc_spec.provider = "opencode".into();
        let mut cx_spec = spec(&codex);
        cx_spec.provider = "codex".into();
        refresh_sources(&db, &[oc_spec, cx_spec]).unwrap();
        let query = crate::services::usage_query::UsageQueryV1 {
            version: 1,
            start_ms: Some(1792022400000),
            end_ms: 1792108800000,
            tool_ids: vec!["codex".into()],
            model_ids: vec![],
            project_ids: vec![],
            source_ids: vec![],
        };
        let result = crate::services::session_analytics_api::query(
            &db,
            query.clone(),
            "ofox.ai",
            1792108800000,
            &chrono::Utc,
        )
        .unwrap();
        assert_eq!(result.totals.tokens.total_tokens, 7);
        assert!(!result
            .coverage
            .reasons
            .iter()
            .any(|r| r == "invalidSessionRoot"));
        let mut all = query;
        all.tool_ids.clear();
        let result = crate::services::session_analytics_api::query(
            &db,
            all,
            "ofox.ai",
            1792108800000,
            &chrono::Utc,
        )
        .unwrap();
        assert_eq!(result.coverage.state, "partial");
        assert!(result
            .coverage
            .reasons
            .iter()
            .any(|r| r == "invalidSessionRoot"));
    }
    #[test]
    fn session_analytics_index_unknown_stamp_peer_does_not_reparse_an_unchanged_source() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("c.jsonl");
        write(&path, &row("c", "m", 3));
        refresh_sources(&db, &[spec(&path)]).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let parts = source_cache::read_on(&conn, path.to_str().unwrap(), "claude").unwrap();
            let mut value = parts.value(Part::Analytics).unwrap().clone();
            value["stamp"]["futurePeer"] = json!({"preserve":"opaque"});
            value["stamp"]["native"]["futurePhysicalPeer"] = json!("opaque");
            source_cache::merge_on(
                &conn,
                path.to_str().unwrap(),
                "claude",
                None,
                Part::Analytics,
                &parts.token(Part::Analytics),
                Some(value),
            )
            .unwrap();
        }
        let warm = refresh_sources(&db, &[spec(&path)]).unwrap();
        assert_eq!(warm.parsed, 0);
        assert_eq!(warm.unchanged, 1);
        let conn = db.conn.lock().unwrap();
        let parts = source_cache::read_on(&conn, path.to_str().unwrap(), "claude").unwrap();
        assert_eq!(
            parts.value(Part::Analytics).unwrap()["stamp"]["futurePeer"]["preserve"],
            "opaque"
        );
        assert_eq!(
            parts.value(Part::Analytics).unwrap()["stamp"]["native"]["futurePhysicalPeer"],
            "opaque"
        );
    }
    #[test]
    fn session_analytics_index_moved_revised_source_supersedes_its_missing_cached_replica() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let old = dir.path().join("old.jsonl");
        let new = dir.path().join("new.jsonl");
        let mut before = row("s", "m", 3);
        before["cwd"] = json!("/before");
        write(&old, &before);
        refresh_sources(&db, &[spec(&old)]).unwrap();
        std::fs::rename(&old, &new).unwrap();
        let mut revised = row("s", "m", 9);
        revised["cwd"] = json!("/after");
        revised["timestamp"] = json!("2026-10-15T00:01:00Z");
        write(&new, &revised);
        refresh_sources(&db, &[spec(&new)]).unwrap();
        let input = read_input(&db).unwrap();
        assert_eq!(
            input.facts.len(),
            1,
            "missing old replicas are not current conflicting calls"
        );
        assert_eq!(input.facts[0].buckets[0], 9);
        assert_eq!(input.messages.len(), 1);
        assert_eq!(input.messages[0].at_ms, 1792022460000);
        assert_eq!(input.sessions.len(), 1);
        assert_eq!(input.sessions[0].project_path.as_deref(), Some("/after"));
        assert!(input.sessions[0].source_available);
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "sourceMissing"));
        assert!(!input.source_reasons.iter().any(|r| matches!(
            r.reason.as_str(),
            "conflictingUsageReplica" | "conflictingMessageReplica" | "conflictingSessionMetadata"
        )));
    }
}

#[cfg(test)]
#[path = "codex_thread_usage_tests.rs"]
mod codex_thread_usage_tests;
