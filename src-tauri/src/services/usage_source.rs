//! Source snapshots and exact ingestion evidence, committed beside usage facts.
use super::usage_sync_db::SyncState;
use crate::{
    database::{lock_conn, Database},
    error::AppError,
    session_manager::source_cache::{
        self, Part, SourceStamp, MAX_ACKNOWLEDGED_EVENTS, MAX_PROVIDER_ACKNOWLEDGED_EVENTS,
        MAX_USAGE_BYTES,
    },
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

fn error(reason: &str) -> AppError {
    AppError::Database(format!("usage source: {reason}"))
}
#[derive(Clone, Copy)]
enum Kind {
    File,
    Directory,
    Sqlite,
    Tree,
    Storage,
}
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub path: PathBuf,
    pub stamp: Value,
    kind: Kind,
}
impl Snapshot {
    pub(crate) fn file(path: &Path) -> Result<Self, AppError> {
        Self::capture(path, Kind::File)
    }
    pub(crate) fn directory(path: &Path) -> Result<Self, AppError> {
        Self::capture(path, Kind::Directory)
    }
    pub(crate) fn sqlite(path: &Path) -> Result<Self, AppError> {
        Self::capture(path, Kind::Sqlite)
    }
    pub(crate) fn tree(path: &Path) -> Result<Self, AppError> {
        Self::capture(path, Kind::Tree)
    }
    fn storage(path: &Path) -> Result<Self, AppError> {
        Self::capture(path, Kind::Storage)
    }
    fn capture(path: &Path, kind: Kind) -> Result<Self, AppError> {
        if matches!(kind, Kind::Sqlite) {
            super::native_sqlite_snapshot::validate_paths(path, error)?;
        }
        if matches!(kind, Kind::Tree) {
            return Ok(Self {
                path: path.to_owned(),
                stamp: tree_stamp(path)?,
                kind,
            });
        }
        let own = SourceStamp::at(path)?;
        let stamp = match kind {
            Kind::File => json!(own),
            Kind::Tree => unreachable!(),
            Kind::Storage => {
                json!({"directory":own,"messages":tree_stamp(&path.join("message"))?,"sessions":tree_stamp(&path.join("session"))?})
            }
            Kind::Sqlite => {
                let wal = PathBuf::from(format!("{}-wal", path.to_string_lossy()));
                let wal = match fs::metadata(&wal) {
                    Ok(_) => Some(SourceStamp::at(&wal)?),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(_) => return Err(error("WAL metadata unavailable")),
                };
                json!({"main":own,"wal":wal})
            }
            Kind::Directory => {
                let mut files = Vec::new();
                for entry in fs::read_dir(path).map_err(|_| error("directory unavailable"))? {
                    let entry = entry.map_err(|_| error("directory enumeration failed"))?;
                    let file = entry.path();
                    if file.extension().is_some_and(|e| e == "json") {
                        files.push((
                            entry.file_name().to_string_lossy().into_owned(),
                            SourceStamp::at(&file)?,
                        ));
                    }
                }
                files.sort_by(|a, b| a.0.cmp(&b.0));
                let raw = serde_json::to_vec(&files).map_err(|_| error("directory stamp"))?;
                json!({"directory":own,"files":files.len(),"manifest":format!("{:x}",Sha256::digest(raw))})
            }
        };
        Ok(Self {
            path: path.to_owned(),
            stamp,
            kind,
        })
    }
    pub(crate) fn verify(&self) -> Result<(), AppError> {
        if Self::capture(&self.path, self.kind)?.stamp != self.stamp {
            return Err(error("source changed; retry"));
        }
        Ok(())
    }
}

fn tree_stamp(path: &Path) -> Result<Value, AppError> {
    fn collect(
        path: &Path,
        root: &Path,
        depth: usize,
        files: &mut Vec<(String, SourceStamp)>,
    ) -> Result<(), AppError> {
        if depth > 8 || files.len() >= MAX_ACKNOWLEDGED_EVENTS {
            return Err(error("source manifest exceeds budget"));
        }
        let meta = fs::symlink_metadata(path).map_err(|_| error("source manifest unavailable"))?;
        if meta.file_type().is_symlink() {
            return Err(error("source manifest contains a symlink"));
        }
        if meta.is_dir() {
            files.push((
                path.strip_prefix(root)
                    .map_err(|_| error("source manifest path"))?
                    .to_string_lossy()
                    .into_owned(),
                SourceStamp::at(path)?,
            ));
            for entry in fs::read_dir(path).map_err(|_| error("source directory unavailable"))? {
                let entry = entry.map_err(|_| error("source enumeration failed"))?;
                let file = entry.path();
                if entry
                    .file_type()
                    .map_err(|_| error("source metadata unavailable"))?
                    .is_dir()
                    || file.extension().is_some_and(|e| e == "json")
                {
                    collect(&file, root, depth + 1, files)?;
                }
            }
        } else if meta.is_file() {
            files.push((
                path.strip_prefix(root)
                    .map_err(|_| error("source manifest path"))?
                    .to_string_lossy()
                    .into_owned(),
                SourceStamp::at(path)?,
            ));
        }
        Ok(())
    }
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Null),
        Err(_) => return Err(error("source manifest unavailable")),
        Ok(_) => {}
    }
    let mut files = Vec::new();
    collect(path, path, 0, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(
        json!({"entries":files.len(),"digest":format!("{:x}",Sha256::digest(serde_json::to_vec(&files).map_err(|_|error("source manifest"))?))}),
    )
}

/// Read-only normalized facts shared by the ledger and session analytics.
/// `native_key` is absent when a provider has no stable event identity. It is
/// never synthesized from the accounting ordinal. Time remains absent rather
/// than assigning a precise day to an undated native event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedUsageFact {
    pub request_id: String,
    pub native_key: Option<String>,
    pub session_id: Option<String>,
    pub model: String,
    pub usage: crate::proxy::usage::parser::TokenUsage,
    pub at_ms: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedSnapshot {
    pub source_path: PathBuf,
    pub source_stamp: Value,
    pub facts: Vec<NormalizedUsageFact>,
    pub complete: bool,
    pub reasons: BTreeSet<String>,
}
pub(crate) fn timestamp_ms(timestamp: Option<&str>) -> Option<i64> {
    timestamp
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis())
}
fn normalized_source(provider: &str, path: &Path) -> Result<Snapshot, AppError> {
    match provider {
        "claude" | "codex" | "gemini" | "workbuddy" | "dsh" | "pi" => Snapshot::file(path),
        "openclaw"
            if path
                .file_name()
                .is_some_and(|s| s == "openclaw-agent.sqlite") =>
        {
            Snapshot::sqlite(path)
        }
        "openclaw" => Snapshot::file(path),
        "opencode" if path.is_file() => Snapshot::sqlite(path),
        "opencode" => Snapshot::storage(path),
        _ => Err(error("unsupported normalized provider")),
    }
}
/// Canonical scope: one transcript for Claude/Codex/Gemini; the OpenCode
/// database or its storage root (message + session dependencies) for OpenCode.
/// Only stats files; it never parses source bodies or mutates the cache.
pub fn normalized_stamp(provider: &str, path: &Path) -> Result<Value, AppError> {
    Ok(normalized_source(provider, path)?.stamp)
}
pub fn read_normalized(provider: &str, path: &Path) -> Result<NormalizedSnapshot, AppError> {
    let snapshot = normalized_source(provider, path)?;
    let mut reasons = BTreeSet::new();
    let (mut facts, enumerated) = match provider {
        "claude" => super::session_usage::normalized_facts(path)?,
        "codex" => super::session_usage_codex::normalized_facts(path)?,
        "gemini" => super::session_usage_gemini::normalized_facts(path)?,
        "opencode" => super::session_usage_opencode::normalized_facts(path)?,
        "workbuddy" => {
            let read = super::session_usage_workbuddy::read_workbuddy(path)?;
            let complete = !read.reasons.contains("incompleteEnumeration");
            reasons.extend(read.reasons);
            (read.facts, complete)
        }
        "dsh" => {
            let read = super::session_usage_dsh::read_dsh(path)?;
            let complete = !read.reasons.contains("incompleteEnumeration");
            reasons.extend(read.reasons);
            (read.facts, complete)
        }
        "pi" => {
            let read = super::session_usage_pi::read_pi(path)?;
            let complete = !read.reasons.contains("incompleteEnumeration");
            reasons.extend(read.reasons);
            (read.facts, complete)
        }
        "openclaw" => {
            let read = super::session_usage_openclaw::read_openclaw(path)?;
            let complete = !read.reasons.contains("incompleteEnumeration");
            reasons.extend(read.reasons);
            (read.facts, complete)
        }
        _ => return Err(error("unsupported normalized provider")),
    };
    snapshot.verify()?;
    facts.sort_by(|a, b| {
        a.at_ms
            .cmp(&b.at_ms)
            .then_with(|| a.request_id.cmp(&b.request_id))
    });
    if !enumerated {
        reasons.insert("incompleteEnumeration".into());
    }
    if facts.iter().any(|f| f.at_ms.is_none()) {
        reasons.insert("missingTimestamp".into());
    }
    if facts.iter().any(|f| f.native_key.is_none()) {
        reasons.insert("missingNativeEventIdentity".into());
    }
    if facts
        .iter()
        .any(|f| f.session_id.as_deref().is_none_or(str::is_empty))
    {
        reasons.insert("missingNativeSessionIdentity".into());
    }
    if facts.iter().any(|f| f.model == "unknown") {
        reasons.insert("missingModel".into());
    }
    Ok(NormalizedSnapshot {
        source_path: path.to_owned(),
        source_stamp: snapshot.stamp,
        facts,
        complete: reasons.is_empty(),
        reasons,
    })
}

/// Keep an open handle across parsing, checking both that handle and the path.
pub(crate) fn with_stable_file<T>(
    path: &Path,
    parse: impl FnOnce(fs::File) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let file = fs::File::open(path).map_err(|_| error("file unavailable"))?;
    with_stable_open_file(path, file, parse)
}
/// Pi source/config reads must never wait on a FIFO swapped in after stat.
/// Verify the opened descriptor; a pathname-only check cannot close that race.
pub(crate) fn with_stable_regular_file<T>(
    path: &Path,
    parse: impl FnOnce(fs::File) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options.open(path).map_err(|_| error("file unavailable"))?;
    if !file
        .metadata()
        .map_err(|_| error("file metadata unavailable"))?
        .is_file()
    {
        return Err(error("non-regular file"));
    }
    with_stable_open_file(path, file, parse)
}
fn with_stable_open_file<T>(
    path: &Path,
    file: fs::File,
    parse: impl FnOnce(fs::File) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let before = SourceStamp::of(&file)?;
    record_body_read(path);
    let result = parse(
        file.try_clone()
            .map_err(|_| error("file handle unavailable"))?,
    )?;
    if SourceStamp::of(&file)? != before {
        return Err(error("file changed during parse; retry"));
    }
    before.verify(path)?;
    Ok(result)
}
pub(crate) fn read_text(path: &Path) -> Result<String, AppError> {
    with_stable_file(path, |mut file| {
        let mut text = String::new();
        file.read_to_string(&mut text)
            .map_err(|_| error("file read failed"))?;
        Ok(text)
    })
}
/// Newline-delimited malformed records keep the old tolerant behavior. An
/// incomplete EOF record is not acknowledged; valid JSON at EOF needs no LF.
pub(crate) struct JsonlProgress {
    pub offset: i64,
    pub complete: bool,
}
pub(crate) fn json_lines(
    path: &Path,
    wanted: impl Fn(&str) -> bool,
    mut accept: impl FnMut(i64, Value) -> Result<(), AppError>,
) -> Result<JsonlProgress, AppError> {
    with_stable_file(path, |file| {
        let mut reader = BufReader::new(file);
        let mut offset = 0;
        let mut enumeration_complete = true;
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            let n = reader
                .read_until(b'\n', &mut bytes)
                .map_err(|_| error("JSONL read failed"))?;
            if n == 0 {
                break;
            }
            let complete = bytes.last() == Some(&b'\n');
            let line = match std::str::from_utf8(&bytes) {
                Ok(line) => line,
                Err(_) if complete => {
                    enumeration_complete = false;
                    offset += 1;
                    continue;
                }
                Err(_) => {
                    enumeration_complete = false;
                    break;
                }
            };
            if line.trim().is_empty() {
                if !complete {
                    enumeration_complete = false;
                    break;
                }
                offset += 1;
                continue;
            }
            if complete && !wanted(line) {
                offset += 1;
                continue;
            }
            let value = match serde_json::from_str(line) {
                Ok(value) => value,
                Err(_) if complete => {
                    enumeration_complete = false;
                    offset += 1;
                    continue;
                }
                Err(_) => {
                    enumeration_complete = false;
                    break;
                }
            };
            offset += 1;
            if wanted(line) {
                accept(offset, value)?;
            }
        }
        Ok(JsonlProgress {
            offset,
            complete: enumeration_complete,
        })
    })
}

pub(crate) fn checked_token(value: u64) -> Result<u32, AppError> {
    u32::try_from(value).map_err(|_| error("token bucket exceeds supported integer range"))
}
pub(crate) fn number(object: &Value, key: &str) -> Result<u64, AppError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value.as_u64().ok_or_else(|| error("invalid token integer")),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Evidence {
    version: u32,
    parser_revision: u32,
    pub trusted: bool,
    pub coverage: String,
    pub complete_event_identity_set: bool,
    pub reasons: BTreeSet<String>,
    source_stamp: Value,
    checkpoint: SyncState,
    pub acknowledged_event_ids: BTreeSet<String>,
    #[serde(default)]
    codex_prefix_hash: Option<String>,
    #[serde(default)]
    codex_reconciliation_required: bool,
    #[serde(default)]
    dependency_hash: Option<String>,
}
impl Evidence {
    pub(crate) fn codex_prefix(&self) -> Option<&str> {
        self.codex_prefix_hash.as_deref()
    }
    pub(crate) fn codex_requires_reconciliation(&self) -> bool {
        self.codex_reconciliation_required
    }
    pub(crate) fn note_codex_prefix(&mut self, hash: String, reconcile: bool) {
        self.codex_prefix_hash = Some(hash);
        self.codex_reconciliation_required |= reconcile;
        if self.codex_reconciliation_required {
            self.partial("legacyLiveReconciliationRequired");
        }
    }
    pub(crate) fn partial(&mut self, reason: &str) {
        self.coverage = "partial".into();
        self.reasons.insert(reason.into());
    }
    pub(crate) fn has(&self, native: &str) -> bool {
        self.acknowledged_event_ids.contains(&native_digest(native))
    }
    pub(crate) fn acknowledge(&mut self, native: &str) {
        let digest = native_digest(native);
        if self.acknowledged_event_ids.contains(&digest) {
            return;
        }
        // Exact digests are bounded by both count and their serialized bytes;
        // never evict positive proof then infer absence from the smaller set.
        if self.acknowledged_event_ids.len() >= MAX_ACKNOWLEDGED_EVENTS
            || (self.acknowledged_event_ids.len() + 1) * 67 > MAX_USAGE_BYTES / 2
        {
            self.complete_event_identity_set = false;
            self.partial("acknowledgementBudget");
            return;
        }
        self.acknowledged_event_ids.insert(digest);
    }
}
pub(crate) fn native_digest(native: &str) -> String {
    format!("{:x}", Sha256::digest(native.as_bytes()))
}
pub(crate) fn native_key(provider: &str, session: &str, id: &str) -> String {
    // Claude and OpenCode ledger identities are provider-global; a session
    // header rename must not turn an archived message into an unseen event.
    let scope = if matches!(provider, "claude" | "opencode") {
        ""
    } else {
        session
    };
    json!([provider, scope, id]).to_string()
}

const MAX_PROVIDER_ACKNOWLEDGED_BYTES: usize = 80 * 1024 * 1024;
fn extend_acknowledged(
    set: &mut BTreeSet<String>,
    ids: impl IntoIterator<Item = String>,
    limit: usize,
) -> bool {
    let mut complete = true;
    for id in ids {
        if set.contains(&id) {
            continue;
        }
        if set.len() >= limit || (set.len() + 1) * 67 > MAX_PROVIDER_ACKNOWLEDGED_BYTES {
            complete = false;
            continue;
        }
        set.insert(id);
    }
    complete
}
/// One batch read per provider scan. Positive identities remain per physical
/// source; this in-memory union is shared by all files in that scan. The small
/// persistent roster detects missing receipts rather than treating the
/// remaining files as complete again.
pub(crate) struct ProviderIndex {
    provider: &'static str,
    roots: Vec<PathBuf>,
    generation: Option<String>,
    epoch: Option<String>,
    acknowledged: Arc<BTreeSet<String>>,
    negative_complete: bool,
    origin_empty: bool,
    ledger_count: i64,
}

/// Health-only coverage summary; never a replacement for Prepared's identity
/// proofs, generation lease, or transaction-time checks.
pub(crate) fn has_complete_coverage_on(
    conn: &rusqlite::Connection,
    provider: &str,
) -> Result<bool, AppError> {
    let Some(manifest) = source_cache::provider_on(conn, provider)? else {
        return Ok(false);
    };
    if manifest["originEmpty"] != true
        || manifest["complete"] != true
        || manifest["ledgerEpoch"] != json!(source_cache::ledger_epoch_on(conn)?)
    {
        return Ok(false);
    }
    let checkpoints = checkpoints_on(conn)?;
    let mut acknowledged = BTreeSet::new();
    let mut observed = BTreeMap::new();
    let mut statement = conn.prepare(
        "SELECT source_path,provider_id,stamp,parser_version,meta_json FROM session_meta_cache WHERE provider_id=?1",
    )?;
    let rows = statement.query_map([provider], |r| {
        Ok((
            r.get::<_, String>(0)?,
            (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?),
        ))
    })?;
    for row in rows {
        let (path, row) = row?;
        let Ok(parts) = source_cache::decode_row(row, provider) else {
            return Ok(false);
        };
        let Some(receipt) = parts.receipt() else {
            continue;
        };
        let Some(value) = parts.value(Part::UsageSync) else {
            return Ok(false);
        };
        let Ok(evidence) = decode_evidence(value) else {
            return Ok(false);
        };
        let revision = if matches!(provider, "claude" | "codex" | "gemini" | "opencode") {
            2
        } else {
            1
        };
        if !evidence.trusted
            || !evidence.complete_event_identity_set
            || evidence.parser_revision != revision
            || evidence.coverage != "completeFromStart"
            || !evidence.reasons.is_empty()
            || checkpoints.get(&path) != Some(&evidence.checkpoint)
        {
            return Ok(false);
        }
        observed.insert(source_cache::receipt_key(&path), receipt);
        if !extend_acknowledged(
            &mut acknowledged,
            evidence.acknowledged_event_ids,
            MAX_PROVIDER_ACKNOWLEDGED_EVENTS,
        ) {
            return Ok(false);
        }
    }
    let expected = manifest["receipts"]
        .as_object()
        .ok_or_else(|| error("invalid provider roster"))?;
    if expected.len() != observed.len() || expected.iter().any(|(k, v)| observed.get(k) != Some(v))
    {
        return Ok(false);
    }
    let mut statement = conn.prepare("SELECT request_id,session_id FROM proxy_request_logs WHERE app_type=?1 AND provider_id=?2 AND data_source=?3")?;
    let rows = statement.query_map(
        rusqlite::params![provider, owner(provider)?, origin(provider)?],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
    )?;
    for row in rows {
        let (id, session) = row?;
        if owned_identity(provider, &id, session.as_deref())
            .is_none_or(|n| !acknowledged.contains(&native_digest(&n)))
        {
            return Ok(false);
        }
    }
    Ok(usize::try_from(ledger_count_on(conn, provider)?).ok() == Some(acknowledged.len()))
}

fn owned_identity(provider: &str, request: &str, session: Option<&str>) -> Option<String> {
    let session = session.unwrap_or("");
    if provider == "openclaw" {
        let [agent, sid, id]: [String; 3] =
            serde_json::from_str(request.strip_prefix("openclaw_session:")?).ok()?;
        let canonical_session = json!([agent, sid]).to_string();
        if agent.trim().is_empty()
            || sid.trim().is_empty()
            || id.trim().is_empty()
            || canonical_session != session
        {
            return None;
        }
        return Some(native_key(provider, &canonical_session, &id));
    }
    if matches!(provider, "dsh" | "pi") {
        let [sid, kind, id]: [String; 3] =
            serde_json::from_str(request.strip_prefix(&format!("{provider}_session:"))?).ok()?;
        if sid.trim().is_empty() || id.trim().is_empty() || sid != session || kind != "assistant" {
            return None;
        }
        return Some(native_key(provider, &sid, &json!([kind, id]).to_string()));
    }
    if provider == "workbuddy" {
        let [sid, id]: [String; 2] =
            serde_json::from_str(request.strip_prefix("workbuddy_session:")?).ok()?;
        if sid.trim().is_empty() || id.trim().is_empty() || sid != session {
            return None;
        }
        return Some(native_key(provider, &sid, &id));
    }
    let id = match provider {
        "claude" => request.strip_prefix("session:")?,
        "gemini" => request.strip_prefix(&format!("gemini_session:{session}:"))?,
        "opencode" => request.strip_prefix("opencode:")?,
        _ => return None,
    };
    Some(native_key(
        provider,
        if provider == "opencode" { "" } else { session },
        id,
    ))
}
// A cardinality fence detects unreceipted writers crossing prepare/prune.
// It is usable only with the intact, from-zero native-identity roster. The
// prune transaction conserves live + daily-rollup request_count, so pruning a
// new proxy contribution cannot hide that insert from this lease.
fn ledger_count_on(conn: &rusqlite::Connection, provider: &str) -> Result<i64, AppError> {
    let count:i64=conn.query_row("SELECT (SELECT COUNT(*) FROM proxy_request_logs WHERE app_type=?1)+(SELECT COALESCE(SUM(request_count),0) FROM usage_daily_rollups WHERE app_type=?1)",[provider],|r|r.get(0))?;
    if count < 0 {
        return Err(error("invalid ledger cardinality"));
    }
    Ok(count)
}
fn has_owned_on(conn: &rusqlite::Connection, provider: &str) -> Result<bool, AppError> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM proxy_request_logs WHERE app_type=?1 AND provider_id=?2 AND data_source=?3)",
        rusqlite::params![provider,owner(provider)?,origin(provider)?],|r|r.get(0))?)
}
fn owner(provider: &str) -> Result<&'static str, AppError> {
    match provider {
        "claude" => Ok("_session"),
        "gemini" => Ok("_gemini_session"),
        "opencode" => Ok("_opencode_session"),
        "codex" => Ok("_codex_session"),
        "workbuddy" => Ok("_workbuddy_session"),
        "dsh" => Ok("_dsh_session"),
        "pi" => Ok("_pi_session"),
        "openclaw" => Ok("_openclaw_session"),
        _ => Err(error("unknown provider")),
    }
}
fn origin(provider: &str) -> Result<&'static str, AppError> {
    match provider {
        "claude" => Ok("session_log"),
        "gemini" => Ok("gemini_session"),
        "opencode" => Ok("opencode_session"),
        "codex" => Ok("codex_session"),
        "workbuddy" => Ok("workbuddy_session"),
        "dsh" => Ok("dsh_session"),
        "pi" => Ok("pi_session"),
        "openclaw" => Ok("openclaw_session"),
        _ => Err(error("unknown provider")),
    }
}
fn checkpoints_on(conn: &rusqlite::Connection) -> Result<BTreeMap<String, SyncState>, AppError> {
    Ok(conn
        .prepare("SELECT file_path,last_modified,last_line_offset FROM session_log_sync")?
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                SyncState {
                    modified: r.get(1)?,
                    offset: r.get(2)?,
                },
            ))
        })?
        .collect::<Result<_, _>>()?)
}
fn no_prior_ledger_on(
    conn: &rusqlite::Connection,
    provider: &str,
    roots: &[PathBuf],
) -> Result<bool, AppError> {
    Ok(ledger_count_on(conn, provider)? == 0
        && !has_history_on(conn, provider)?
        && !has_owned_on(conn, provider)?
        && !checkpoints_on(conn)?
            .keys()
            .any(|p| roots.iter().any(|root| Path::new(p).starts_with(root))))
}
pub(crate) fn decode_evidence(value: &Value) -> Result<Evidence, AppError> {
    if value["version"].as_u64() != Some(1) {
        return Err(error("unsupported evidence version"));
    }
    if serde_json::to_vec(value)
        .map_err(|_| error("invalid evidence"))?
        .len()
        > MAX_USAGE_BYTES
    {
        return Err(error("evidence exceeds byte budget"));
    }
    let evidence: Evidence =
        serde_json::from_value(value.clone()).map_err(|_| error("invalid evidence"))?;
    if evidence.acknowledged_event_ids.len() > MAX_ACKNOWLEDGED_EVENTS {
        return Err(error("evidence exceeds event budget"));
    }
    if evidence.acknowledged_event_ids.iter().any(|id| {
        id.len() != 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err(error("invalid canonical event digest"));
    }
    Ok(evidence)
}
impl ProviderIndex {
    /// Analytics may inspect existing ownership evidence but must not repair or
    /// invalidate an ingestion manifest merely by reading a projection.
    pub(crate) fn read_roots(
        db: &Database,
        provider: &'static str,
        roots: &[PathBuf],
    ) -> Result<Self, AppError> {
        let mut seen = BTreeSet::new();
        if roots.is_empty()
            || roots.iter().any(|root| {
                !root.is_absolute()
                    || root
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir))
                    || !seen.insert(root.clone())
            })
        {
            return Err(error("invalid source roots"));
        }
        Self::load_scoped_mode(db, provider, roots, false)
    }
    pub(crate) fn load_roots(
        db: &Database,
        provider: &'static str,
        roots: &[PathBuf],
    ) -> Result<Self, AppError> {
        let mut seen = BTreeSet::new();
        if roots.is_empty()
            || roots.iter().any(|root| {
                !root.is_absolute()
                    || root
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                    || !seen.insert(root.clone())
            })
        {
            return Err(error("invalid source roots"));
        }
        Self::load_scoped(db, provider, roots)
    }
    pub(crate) fn origin_empty(&self) -> bool {
        self.origin_empty
    }
    /// Trusted positive receipts are independent of complete negative evidence.
    pub(crate) fn has_acknowledged(&self, native: &str) -> bool {
        self.acknowledged.contains(&native_digest(native))
    }
    pub(crate) fn load(
        db: &Database,
        provider: &'static str,
        root: &Path,
    ) -> Result<Self, AppError> {
        Self::load_scoped(db, provider, &[root.to_owned()])
    }
    fn load_scoped(
        db: &Database,
        provider: &'static str,
        roots: &[PathBuf],
    ) -> Result<Self, AppError> {
        Self::load_scoped_mode(db, provider, roots, true)
    }
    fn load_scoped_mode(
        db: &Database,
        provider: &'static str,
        roots: &[PathBuf],
        invalidate: bool,
    ) -> Result<Self, AppError> {
        let mut conn = lock_conn!(db.conn);
        let tx = conn.transaction()?;
        let mut manifest = source_cache::provider_on(&tx, provider)?;
        let epoch = source_cache::ledger_epoch_on(&tx)?;
        let checkpoints = checkpoints_on(&tx)?;
        let mut acknowledged = BTreeSet::new();
        let mut observed = BTreeMap::new();
        let mut complete = manifest.as_ref().is_some_and(|m| {
            m["originEmpty"] == true && m["complete"] == true && m["ledgerEpoch"] == json!(epoch)
        });
        let mut any_receipt = false;
        let mut ambiguous_cache = false;
        let mut valid_paths = BTreeSet::new();
        {
            let mut statement=tx.prepare("SELECT source_path,provider_id,stamp,parser_version,meta_json FROM session_meta_cache WHERE provider_id=?1 ORDER BY source_path")?;
            let rows = statement.query_map([provider], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?),
                ))
            })?;
            for row in rows {
                let (path, row) = row?;
                let parts = match source_cache::decode_row(row, provider) {
                    Ok(p) => p,
                    Err(_) => {
                        complete = false;
                        ambiguous_cache = true;
                        continue;
                    }
                };
                let Some(receipt) = parts.receipt() else {
                    continue;
                };
                any_receipt = true;
                let key = source_cache::receipt_key(&path);
                observed.insert(key.clone(), receipt.clone());
                let Some(value) = parts.value(Part::UsageSync) else {
                    complete = false;
                    continue;
                };
                let evidence = match decode_evidence(value) {
                    Ok(e) => e,
                    Err(_) => {
                        complete = false;
                        continue;
                    }
                };
                let registered = manifest.as_ref().is_some_and(|m| {
                    m["ledgerEpoch"] == json!(epoch) && m["receipts"][&key] == receipt
                });
                if !registered
                    || !evidence.trusted
                    || checkpoints.get(&path) != Some(&evidence.checkpoint)
                {
                    complete = false;
                    continue;
                }
                valid_paths.insert(path);
                complete &= evidence.complete_event_identity_set;
                complete &= extend_acknowledged(
                    &mut acknowledged,
                    evidence.acknowledged_event_ids,
                    MAX_PROVIDER_ACKNOWLEDGED_EVENTS,
                );
            }
        }
        if let Some(m) = &manifest {
            let expected = m["receipts"]
                .as_object()
                .ok_or_else(|| error("invalid provider roster"))?;
            if expected.len() != observed.len()
                || expected.iter().any(|(k, v)| observed.get(k) != Some(v))
            {
                complete = false;
            }
        }
        if checkpoints.keys().any(|p| {
            roots.iter().any(|root| Path::new(p).starts_with(root)) && !valid_paths.contains(p)
        }) {
            complete = false;
        }
        let mut any_live = false;
        {
            let mut statement=tx.prepare("SELECT request_id,session_id FROM proxy_request_logs WHERE app_type=?1 AND provider_id=?2 AND data_source=?3")?;
            let rows = statement.query_map(
                rusqlite::params![provider, owner(provider)?, origin(provider)?],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )?;
            for row in rows {
                any_live = true;
                let (id, session) = row?;
                if owned_identity(provider, &id, session.as_deref())
                    .is_none_or(|n| !acknowledged.contains(&native_digest(&n)))
                {
                    complete = false;
                }
            }
        }
        let ledger_count = ledger_count_on(&tx, provider)?;
        complete &= usize::try_from(ledger_count).ok() == Some(acknowledged.len());
        let origin_empty = manifest.is_none()
            && !any_receipt
            && !ambiguous_cache
            && epoch.is_none()
            && !any_live
            && ledger_count == 0
            && !has_history_on(&tx, provider)?
            && !checkpoints
                .keys()
                .any(|p| roots.iter().any(|root| Path::new(p).starts_with(root)));
        if origin_empty {
            complete = true;
        }
        // Persist detected proof loss once. Recreating only some cache rows can
        // never turn a former complete ledger into a new empty origin.
        if invalidate && manifest.as_ref().is_some_and(|m| m["complete"] == true) && !complete {
            source_cache::invalidate_provider_on(&tx, provider, "incompleteReceiptCoverage")?;
            manifest = source_cache::provider_on(&tx, provider)?;
        }
        let generation = source_cache::provider_generation(&manifest);
        tx.commit()?;
        Ok(Self {
            provider,
            roots: roots.to_owned(),
            generation,
            epoch,
            acknowledged: Arc::new(acknowledged),
            negative_complete: complete,
            origin_empty,
            ledger_count,
        })
    }
    pub(crate) fn check_on(&self, conn: &rusqlite::Connection) -> Result<(), AppError> {
        if source_cache::ledger_epoch_on(conn)? != self.epoch
            || source_cache::provider_generation(&source_cache::provider_on(conn, self.provider)?)
                != self.generation
        {
            return Err(error("provider evidence changed; retry"));
        }
        if self.negative_complete && ledger_count_on(conn, self.provider)? != self.ledger_count {
            return Err(error("ledger contribution boundary changed; retry"));
        }
        if self.origin_empty && !no_prior_ledger_on(conn, self.provider, &self.roots)? {
            return Err(error("empty ledger boundary changed; retry"));
        }
        Ok(())
    }
    pub(crate) fn advance(&mut self, prepared: Prepared) {
        self.advance_many(vec![prepared]);
    }
    pub(crate) fn advance_many(&mut self, prepared: Vec<Prepared>) {
        let mut next = BTreeSet::new();
        for prepared in prepared {
            drop(prepared.index);
            self.generation = prepared.saved_generation;
            self.ledger_count = prepared.saved_ledger_count;
            self.origin_empty = false;
            self.negative_complete &=
                prepared.evidence.complete_event_identity_set && prepared.saved_negative_complete;
            self.negative_complete &= extend_acknowledged(
                &mut next,
                prepared.evidence.acknowledged_event_ids,
                MAX_PROVIDER_ACKNOWLEDGED_EVENTS,
            );
        }
        self.negative_complete &= extend_acknowledged(
            Arc::make_mut(&mut self.acknowledged),
            next,
            MAX_PROVIDER_ACKNOWLEDGED_EVENTS,
        );
    }
}
pub(crate) struct Prepared {
    pub snapshot: Snapshot,
    pub provider: &'static str,
    pub expected: Option<String>,
    pub evidence: Evidence,
    pub unchanged: bool,
    pub may_import_unseen_history: bool,
    index: ProviderIndex,
    saved_generation: Option<String>,
    saved_negative_complete: bool,
    saved_ledger_count: i64,
    dependency_changed: bool,
}
pub(crate) fn has_history_on(
    conn: &rusqlite::Connection,
    provider: &str,
) -> Result<bool, AppError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_usage_archive WHERE app_type=?1)
        OR EXISTS(SELECT 1 FROM usage_daily_rollups WHERE app_type=?1)
        OR (?1 IN ('claude','codex','gemini','opencode')
            AND EXISTS(SELECT 1 FROM maintenance_flags WHERE key='backfill_session_archive'))",
        [provider],
        |r| r.get(0),
    )?)
}
impl Prepared {
    #[cfg(test)]
    pub(crate) fn load(
        db: &Database,
        snapshot: Snapshot,
        provider: &'static str,
        checkpoint: Option<SyncState>,
        parser_revision: u32,
    ) -> Result<Self, AppError> {
        let index = ProviderIndex::load(
            db,
            provider,
            snapshot.path.parent().unwrap_or(&snapshot.path),
        )?;
        Self::with_index(db, snapshot, checkpoint, parser_revision, &index)
    }
    pub(crate) fn with_index(
        db: &Database,
        snapshot: Snapshot,
        checkpoint: Option<SyncState>,
        parser_revision: u32,
        index: &ProviderIndex,
    ) -> Result<Self, AppError> {
        let provider = index.provider;
        let conn = lock_conn!(db.conn);
        let parts = source_cache::read_on(&conn, &snapshot.path.to_string_lossy(), provider)?;
        let expected = parts.token(Part::UsageSync);
        let previous = parts
            .value(Part::UsageSync)
            .map(decode_evidence)
            .transpose()?;
        let companion_matches = previous
            .as_ref()
            .is_some_and(|v| v.trusted && Some(v.checkpoint) == checkpoint);
        let fresh = checkpoint.is_none() && previous.is_none() && index.negative_complete;
        let unchanged = previous.as_ref().is_some_and(|v| {
            companion_matches
                && v.parser_revision == parser_revision
                && v.source_stamp == snapshot.stamp
        });
        let mut evidence = previous
            .filter(|_| companion_matches)
            .unwrap_or_else(|| Evidence {
                version: 1,
                parser_revision,
                trusted: true,
                coverage: if fresh {
                    "completeFromStart"
                } else {
                    "legacyUnknownBoundary"
                }
                .into(),
                complete_event_identity_set: fresh,
                reasons: if fresh {
                    BTreeSet::new()
                } else {
                    BTreeSet::from(["legacyUnknownBoundary".into()])
                },
                source_stamp: snapshot.stamp.clone(),
                checkpoint: checkpoint.unwrap_or_default(),
                acknowledged_event_ids: BTreeSet::new(),
                codex_prefix_hash: None,
                codex_reconciliation_required: false,
                dependency_hash: None,
            });
        if evidence.parser_revision != parser_revision {
            evidence.complete_event_identity_set = false;
            evidence.partial("parserRevisionChanged");
        }
        if physical_identity(&evidence.source_stamp) != physical_identity(&snapshot.stamp) {
            evidence.complete_event_identity_set = false;
            evidence.partial("sourceIdentityChanged");
        }
        evidence.parser_revision = parser_revision;
        let may_import_unseen_history = index.negative_complete;
        Ok(Self {
            snapshot,
            provider,
            expected,
            evidence,
            unchanged,
            may_import_unseen_history,
            index: ProviderIndex {
                provider,
                roots: index.roots.clone(),
                generation: index.generation.clone(),
                epoch: index.epoch.clone(),
                acknowledged: Arc::clone(&index.acknowledged),
                negative_complete: index.negative_complete,
                origin_empty: index.origin_empty,
                ledger_count: index.ledger_count,
            },
            saved_generation: index.generation.clone(),
            saved_negative_complete: index.negative_complete,
            saved_ledger_count: index.ledger_count,
            dependency_changed: false,
        })
    }
    /// A physical message source can depend on another native structure (for
    /// example OpenCode's session ancestry). Missing legacy proof also requires
    /// a read; recording a new dependency never upgrades historical coverage.
    pub(crate) fn observe_dependency(&mut self, hash: &str) {
        if self.evidence.dependency_hash.as_deref() != Some(hash) {
            self.unchanged = false;
            self.dependency_changed = true;
            self.evidence.dependency_hash = Some(hash.to_owned());
        }
    }
    pub(crate) fn has_dependency(&self) -> bool {
        self.evidence.dependency_hash.is_some()
    }
    pub(crate) fn check_on(&self, conn: &rusqlite::Connection) -> Result<(), AppError> {
        source_cache::check_on(
            conn,
            &self.snapshot.path.to_string_lossy(),
            self.provider,
            Part::UsageSync,
            &self.expected,
        )?;
        self.index.check_on(conn)?;
        self.snapshot.verify()
    }
    pub(crate) fn save_on(
        &mut self,
        conn: &rusqlite::Connection,
        checkpoint: SyncState,
    ) -> Result<(), AppError> {
        self.evidence.source_stamp = self.snapshot.stamp.clone();
        self.evidence.checkpoint = checkpoint;
        let new_events = self
            .evidence
            .acknowledged_event_ids
            .iter()
            .filter(|id| !self.index.acknowledged.contains(*id))
            .count();
        if self.index.acknowledged.len().saturating_add(new_events)
            > MAX_PROVIDER_ACKNOWLEDGED_EVENTS
        {
            self.evidence.complete_event_identity_set = false;
            self.evidence.partial("providerAcknowledgementBudget");
        }
        source_cache::bootstrap_provider_on(conn, self.provider, self.index.origin_empty)?;
        source_cache::merge_on(
            conn,
            &self.snapshot.path.to_string_lossy(),
            self.provider,
            None,
            Part::UsageSync,
            &self.expected,
            Some(serde_json::to_value(&self.evidence).map_err(|_| error("serialize evidence"))?),
        )?;
        let manifest = source_cache::provider_on(conn, self.provider)?;
        self.saved_generation = source_cache::provider_generation(&manifest);
        self.saved_negative_complete = manifest.as_ref().is_some_and(|m| m["complete"] == true);
        self.saved_ledger_count = ledger_count_on(conn, self.provider)?;
        Ok(())
    }
    pub(crate) fn should_store_on(
        &mut self,
        conn: &rusqlite::Connection,
        request_id: &str,
        native: &str,
        data_source: &str,
        created_at: i64,
    ) -> Result<bool, AppError> {
        let row:Option<(Option<String>,String,String)>=conn.query_row("SELECT data_source,app_type,provider_id FROM proxy_request_logs WHERE request_id=?1",
            [request_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let expected_provider = match self.provider {
            "claude" => "_session",
            "gemini" => "_gemini_session",
            "opencode" => "_opencode_session",
            "workbuddy" => "_workbuddy_session",
            "dsh" => "_dsh_session",
            "pi" => "_pi_session",
            "openclaw" => "_openclaw_session",
            _ => return Err(error("unsupported native evidence source")),
        };
        if let Some((source, app, provider)) = row {
            if source.as_deref() == Some(data_source)
                && app == self.provider
                && provider == expected_provider
            {
                return Ok(true);
            }
            self.evidence.complete_event_identity_set = false;
            self.evidence.partial("foreignRecordIdentity");
            return Ok(false);
        }
        if self.evidence.has(native) || self.index.acknowledged.contains(&native_digest(native)) {
            if self.dependency_changed {
                self.evidence.partial("historicalSessionAncestryUnverified");
            }
            return Ok(false);
        }
        if !self.may_import_unseen_history {
            self.evidence.partial("unverifiedHistoricalBoundary");
            if !crate::database::dao::usage_rollup::unseen_event_is_after_history_on(
                conn,
                self.provider,
                created_at,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn physical_identity(stamp: &Value) -> Option<&Value> {
    stamp.get("physicalIdentity").or_else(|| {
        stamp
            .get("directory")
            .and_then(|s| s.get("physicalIdentity"))
    })
}

#[cfg(test)]
static BODY_READS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, usize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
fn record_body_read(path: &Path) {
    #[cfg(test)]
    {
        *BODY_READS
            .lock()
            .unwrap()
            .entry(path.to_owned())
            .or_default() += 1;
    }
    #[cfg(not(test))]
    let _ = path;
}
#[cfg(test)]
pub(crate) fn body_reads_under(path: &Path) -> usize {
    BODY_READS
        .lock()
        .unwrap()
        .iter()
        .filter(|(p, _)| p.starts_with(path))
        .map(|(_, n)| n)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        services::usage_sync_db::{commit_sync_batch, CheckpointChange},
        test_sandbox::sandbox,
    };
    use serial_test::serial;
    #[test]
    #[serial]
    fn evidence_legacy_backfill_flag_is_scoped_to_historical_readers() {
        let _home = sandbox();
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO maintenance_flags(key,value) VALUES('backfill_session_archive','partial')",
            [],
        )
        .unwrap();
        for provider in ["claude", "codex", "gemini", "opencode"] {
            assert!(has_history_on(&conn, provider).unwrap());
        }
        assert!(
            !has_history_on(&conn, "workbuddy").unwrap(),
            "an unrelated legacy backfill flag is not evidence of new-provider history"
        );
        conn.execute(
            "INSERT INTO session_usage_archive VALUES('workbuddy','s','fixture',1,1,0,0,1,'0',1,1)",
            [],
        )
        .unwrap();
        assert!(has_history_on(&conn, "workbuddy").unwrap());
    }

    #[test]
    #[serial]
    fn evidence_invalid_digest_cannot_bypass_the_provider_byte_budget() {
        let home = sandbox();
        let db = Database::memory().unwrap();
        let path = home.home().join("receipt.jsonl");
        native_receipt(&db, &path, "native");
        let key = path.to_string_lossy();
        let original = {
            let conn = db.conn.lock().unwrap();
            let parts = source_cache::read_on(&conn, &key, "claude").unwrap();
            let mut value = parts.value(Part::UsageSync).unwrap().clone();
            value["acknowledgedEventIds"] = json!(["a".repeat(4096)]);
            source_cache::merge_on(
                &conn,
                &key,
                "claude",
                None,
                Part::UsageSync,
                &parts.token(Part::UsageSync),
                Some(value),
            )
            .unwrap();
            conn.query_row(
                "SELECT meta_json FROM session_meta_cache WHERE source_path=?1",
                [&*key],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
        };
        let cp = read_sync_state_for_test(&db, &key);
        assert!(
            Prepared::load(&db, Snapshot::file(&path).unwrap(), "claude", cp, 2).is_err(),
            "a variable-sized digest defeats the fixed 67-byte index budget"
        );
        let envelope: Value = serde_json::from_str(&original).unwrap();
        for invalid in ["g".repeat(64), "é".repeat(32), "A".repeat(64)] {
            let mut value = envelope["parts"]["usageSync"]["value"].clone();
            value["acknowledgedEventIds"] = json!([invalid]);
            assert!(decode_evidence(&value).is_err());
        }
        let conn = db.conn.lock().unwrap();
        let after: String = conn
            .query_row(
                "SELECT meta_json FROM session_meta_cache WHERE source_path=?1",
                [&*key],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, original);
    }

    fn read_sync_state_for_test(db: &Database, path: &str) -> Option<SyncState> {
        crate::services::usage_sync_db::read_sync_state(db, path).unwrap()
    }

    fn unknown_pruned_boundary(legacy_without_frontier: bool) {
        let home = sandbox();
        let db = Database::memory().unwrap();
        let original = home.home().join("original.jsonl");
        native_receipt(&db, &original, "archived");
        // Model two timezone day boundaries at one fixed instant. The old
        // boundary is sixteen UTC hours later than the current local cutoff.
        let current_cutoff = crate::database::dao::usage_rollup::usage_prune_cutoff().unwrap();
        let old_cutoff = current_cutoff + 16 * 3600;
        let at = current_cutoff + 4 * 3600;
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE proxy_request_logs SET created_at=?1", [at])
            .unwrap();
        db.rollup_and_prune_before(old_cutoff).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            if legacy_without_frontier {
                conn.execute(
                    "DELETE FROM maintenance_flags WHERE key='usage_prune_frontier_utc_v1:claude'",
                    [],
                )
                .unwrap();
            }
            let key = original.to_string_lossy();
            let parts = source_cache::read_on(&conn, &key, "claude").unwrap();
            source_cache::merge_on(
                &conn,
                &key,
                "claude",
                None,
                Part::UsageSync,
                &parts.token(Part::UsageSync),
                None,
            )
            .unwrap();
        }
        let alias = home.home().join("alias.jsonl");
        fs::write(&alias, "{}").unwrap();
        let mut source =
            Prepared::load(&db, Snapshot::file(&alias).unwrap(), "claude", None, 2).unwrap();
        assert!(!source.may_import_unseen_history);
        let next = SyncState {
            modified: 1,
            offset: 1,
        };
        commit_sync_batch(&db, &[CheckpointChange { key: alias.to_string_lossy().into(), expected: None, next: Some(next) }], |conn| {
            source.check_on(conn)?;
            let native = native_key("claude", "s", "archived");
            if source.should_store_on(conn, "session:archived", &native, "session_log", at)? {
                conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES('session:archived','_session','claude','fixture',1,1,0,200,'s',?1,'session_log')", [at])?;
                source.evidence.acknowledge(&native);
            }
            source.save_on(conn, next)
        }).unwrap();
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT (SELECT COUNT(*) FROM proxy_request_logs)+(SELECT COALESCE(SUM(request_count),0) FROM session_usage_archive)", [], |r| r.get(0)).unwrap();
        assert_eq!(
            count, 1,
            "a receding local cutoff reimported an archived unknown identity"
        );
        assert_eq!(source.evidence.coverage, "partial");
        let recent = native_key("claude", "s", "after-frontier");
        assert_eq!(
            source
                .should_store_on(
                    &conn,
                    "session:after-frontier",
                    &recent,
                    "session_log",
                    old_cutoff + 1
                )
                .unwrap(),
            !legacy_without_frontier
        );
    }

    #[test]
    #[serial]
    fn evidence_pruned_unknown_identity_cannot_become_recent_after_timezone_change() {
        unknown_pruned_boundary(false);
    }

    #[test]
    #[serial]
    fn evidence_legacy_archive_without_frontier_cannot_prove_a_recent_identity_is_new() {
        unknown_pruned_boundary(true);
    }

    fn legacy_calendar_boundary(date: &str, last_at: i64, allowed: bool) {
        let home = sandbox();
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO session_usage_archive VALUES('claude','s','fixture',1,1,0,0,1,'0',?1,?1)", [last_at]).unwrap();
            conn.execute("INSERT INTO usage_daily_rollups(date,app_type,provider_id,model,request_count,success_count,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,total_cost_usd,avg_latency_ms) VALUES(?1,'claude','_session','fixture',1,1,1,1,0,0,'0',0)", [date]).unwrap();
        }
        let path = home.home().join("new.jsonl");
        fs::write(&path, "{}").unwrap();
        let mut source =
            Prepared::load(&db, Snapshot::file(&path).unwrap(), "claude", None, 2).unwrap();
        let next = SyncState {
            modified: 1,
            offset: 1,
        };
        commit_sync_batch(
            &db,
            &[CheckpointChange {
                key: path.to_string_lossy().into(),
                expected: None,
                next: Some(next),
            }],
            |conn| {
                source.check_on(conn)?;
                let native = native_key("claude", "s", "new");
                assert_eq!(
                    source.should_store_on(
                        conn,
                        "session:new",
                        &native,
                        "session_log",
                        chrono::Utc::now().timestamp()
                    )?,
                    allowed,
                    "legacy calendar history must bound, rather than guess, a new event's date"
                );
                source.save_on(conn, next)
            },
        )
        .unwrap();
        if !allowed {
            let conn = db.conn.lock().unwrap();
            let raw: String = conn.query_row("SELECT value FROM maintenance_flags WHERE key='usage_prune_frontier_utc_v1:claude'", [], |r| r.get(0)).unwrap();
            let flag: Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(flag["legacyUnknown"], true);
            assert_eq!(source.evidence.coverage, "partial");
        }
    }

    #[test]
    #[serial]
    fn evidence_legacy_valid_archive_and_rollup_still_allow_current_native_events() {
        let old = chrono::Utc::now() - chrono::Duration::days(40);
        legacy_calendar_boundary(&old.format("%Y-%m-%d").to_string(), old.timestamp(), true);
    }

    #[test]
    #[serial]
    fn evidence_legacy_invalid_rollup_date_cannot_prove_a_new_identity() {
        let old = (chrono::Utc::now() - chrono::Duration::days(40)).timestamp();
        legacy_calendar_boundary("2026-02-30", old, false);
    }

    #[test]
    #[serial]
    fn evidence_legacy_overflowing_archive_boundary_cannot_prove_a_new_identity() {
        legacy_calendar_boundary("2026-01-01", i64::MAX, false);
    }

    #[test]
    #[serial]
    fn evidence_prune_after_fresh_prepare_cannot_reinsert_an_archived_fact() {
        let home = sandbox();
        let db = Database::memory().unwrap();
        let path = home.home().join("source.jsonl");
        fs::write(&path, "{}").unwrap();
        let old = (chrono::Utc::now() - chrono::Duration::days(40)).timestamp();
        let mut prepared =
            Prepared::load(&db, Snapshot::file(&path).unwrap(), "claude", None, 2).unwrap();
        assert!(prepared.may_import_unseen_history);
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source)
            VALUES('session:old','_session','claude','fixture',100,10,0,200,'s',?1,'session_log')",[old]).unwrap();
        db.rollup_and_prune(30).unwrap();
        let next = SyncState {
            modified: 7,
            offset: 1,
        };
        let result = commit_sync_batch(
            &db,
            &[CheckpointChange {
                key: path.to_string_lossy().into(),
                expected: None,
                next: Some(next),
            }],
            |conn| {
                prepared.check_on(conn)?;
                let native = native_key("claude", "s", "old");
                if prepared.should_store_on(conn, "session:old", &native, "session_log", old)? {
                    conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source)
                    VALUES('session:old','_session','claude','fixture',100,10,0,200,'s',?1,'session_log')",[old])?;
                    prepared.evidence.acknowledge(&native);
                }
                prepared.save_on(conn, next)?;
                Ok(())
            },
        );
        assert!(
            result.is_err(),
            "fresh negative evidence became stale during parsing"
        );
        let conn = db.conn.lock().unwrap();
        for table in [
            "proxy_request_logs",
            "session_log_sync",
            "session_meta_cache",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
        let count: i64 = conn
            .query_row(
                "SELECT SUM(request_count) FROM session_usage_archive",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
    fn native_receipt(db: &Database, path: &Path, id: &str) {
        fs::write(path, "{}").unwrap();
        let mut prepared =
            Prepared::load(db, Snapshot::file(path).unwrap(), "claude", None, 2).unwrap();
        let next = SyncState {
            modified: 1,
            offset: 1,
        };
        commit_sync_batch(db,&[CheckpointChange{key:path.to_string_lossy().into(),expected:None,next:Some(next)}],|conn|{
            prepared.check_on(conn)?;
            conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES(?1,'_session','claude','fixture',1,1,0,200,'s',?2,'session_log')",
                rusqlite::params![format!("session:{id}"),chrono::Utc::now().timestamp()])?;
            prepared.evidence.acknowledge(&native_key("claude","s",id));prepared.save_on(conn,next)
        }).unwrap();
    }
    #[test]
    #[serial]
    fn evidence_missing_receipt_is_sticky_partial_even_after_the_row_is_restored() {
        let home = sandbox();
        let db = Database::memory().unwrap();
        let a = home.home().join("a.jsonl");
        let b = home.home().join("b.jsonl");
        native_receipt(&db, &a, "a");
        native_receipt(&db, &b, "b");
        assert!(
            ProviderIndex::load(&db, "claude", home.home())
                .unwrap()
                .negative_complete
        );
        let raw: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT meta_json FROM session_meta_cache WHERE source_path=?1",
                [b.to_string_lossy()],
                |r| r.get(0),
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM session_meta_cache WHERE source_path=?1",
                [b.to_string_lossy()],
            )
            .unwrap();
        let lost = ProviderIndex::load(&db, "claude", home.home()).unwrap();
        assert!(!lost.negative_complete);
        assert!(lost
            .acknowledged
            .contains(&native_digest(&native_key("claude", "s", "a"))));
        db.conn.lock().unwrap().execute("INSERT INTO session_meta_cache(source_path,provider_id,meta_json)VALUES(?1,'claude',?2)",rusqlite::params![b.to_string_lossy(),raw]).unwrap();
        let restored = ProviderIndex::load(&db, "claude", home.home()).unwrap();
        assert!(!restored.negative_complete);
        assert!(restored
            .acknowledged
            .contains(&native_digest(&native_key("claude", "s", "b"))));
    }
    #[test]
    #[serial]
    fn evidence_provider_generation_rejects_stale_work_but_metadata_does_not_invalidate_it() {
        let home = sandbox();
        let db = Database::memory().unwrap();
        let a = home.home().join("a.jsonl");
        let b = home.home().join("b.jsonl");
        native_receipt(&db, &a, "a");
        fs::write(&b, "{}").unwrap();
        let prepared = Prepared::load(&db, Snapshot::file(&b).unwrap(), "claude", None, 2).unwrap();
        let conn = db.conn.lock().unwrap();
        source_cache::merge_on(&conn,&a.to_string_lossy(),"claude",None,Part::Metadata,&None,
            Some(json!({"parserVersion":2,"stamp":"fixture","details":{"candidates":{},"messageCount":1}}))).unwrap();
        prepared.check_on(&conn).unwrap();
        let current = source_cache::read_on(&conn, &a.to_string_lossy(), "claude").unwrap();
        source_cache::merge_on(
            &conn,
            &a.to_string_lossy(),
            "claude",
            None,
            Part::UsageSync,
            &current.token(Part::UsageSync),
            Some(json!({"version":1,"future":true})),
        )
        .unwrap();
        assert!(prepared.check_on(&conn).is_err());
    }
    #[test]
    #[serial]
    fn evidence_unknown_peer_and_untracked_live_cannot_create_an_empty_origin() {
        let home = sandbox();
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO session_meta_cache(source_path,provider_id,meta_json)VALUES('future','claude',?1)",[json!({"formatVersion":999,"parts":{}}).to_string()]).unwrap();
        assert!(
            !ProviderIndex::load(&db, "claude", home.home())
                .unwrap()
                .negative_complete
        );
        db.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM session_meta_cache", [])
            .unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES('session:untracked','_session','claude','fixture',1,1,0,200,'s',1,'session_log')",[]).unwrap();
        assert!(
            !ProviderIndex::load(&db, "claude", home.home())
                .unwrap()
                .negative_complete
        );
    }
    #[test]
    #[serial]
    fn evidence_acknowledgement_budget_keeps_existing_positive_proof() {
        let home = sandbox();
        let db = Database::memory().unwrap();
        let path = home.home().join("a.jsonl");
        fs::write(&path, "{}").unwrap();
        let mut p = Prepared::load(&db, Snapshot::file(&path).unwrap(), "claude", None, 2).unwrap();
        p.evidence.acknowledged_event_ids = (0..MAX_ACKNOWLEDGED_EVENTS)
            .map(|n| format!("{n:064x}"))
            .collect();
        let before = p.evidence.acknowledged_event_ids.clone();
        p.evidence.acknowledge("new");
        assert_eq!(p.evidence.acknowledged_event_ids, before);
        assert!(!p.evidence.complete_event_identity_set);
        assert!(p.evidence.reasons.contains("acknowledgementBudget"));
    }
    #[test]
    fn evidence_provider_index_budget_never_evicts_positive_entries() {
        let mut values = BTreeSet::from(["first".into()]);
        assert!(!extend_acknowledged(
            &mut values,
            ["second".into(), "third".into()],
            2
        ));
        assert_eq!(values, BTreeSet::from(["first".into(), "second".into()]));
    }
    #[test]
    #[serial]
    fn evidence_proxy_insert_then_prune_invalidates_an_existing_complete_provider_prepare() {
        let home = sandbox();
        let db = Database::memory().unwrap();
        let a = home.home().join("a.jsonl");
        let b = home.home().join("b.jsonl");
        native_receipt(&db, &a, "owned");
        fs::write(&b, "{}").unwrap();
        let mut prepared =
            Prepared::load(&db, Snapshot::file(&b).unwrap(), "claude", None, 2).unwrap();
        assert!(prepared.may_import_unseen_history);
        let old = (chrono::Utc::now() - chrono::Duration::days(40)).timestamp();
        db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES('session:from-proxy','fixture-proxy','claude','fixture',100,10,0,200,'s',?1,'proxy')",[old]).unwrap();
        db.rollup_and_prune(30).unwrap();
        let next = SyncState {
            modified: 1,
            offset: 1,
        };
        let result = commit_sync_batch(
            &db,
            &[CheckpointChange {
                key: b.to_string_lossy().into(),
                expected: None,
                next: Some(next),
            }],
            |conn| {
                prepared.check_on(conn)?;
                let native = native_key("claude", "s", "from-proxy");
                if prepared.should_store_on(
                    conn,
                    "session:from-proxy",
                    &native,
                    "session_log",
                    old,
                )? {
                    conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES('session:from-proxy','_session','claude','fixture',100,10,0,200,'s',?1,'session_log')",[old])?;
                    prepared.evidence.acknowledge(&native);
                }
                prepared.save_on(conn, next)
            },
        );
        assert!(
            result.is_err(),
            "proxy contribution crossed prepare/prune without invalidating negative evidence"
        );
        let conn = db.conn.lock().unwrap();
        let count:i64=conn.query_row("SELECT (SELECT COUNT(*) FROM proxy_request_logs)+(SELECT COALESCE(SUM(request_count),0) FROM session_usage_archive)",[],|r|r.get(0)).unwrap();
        assert_eq!(count, 2);
        assert!(source_cache::read_on(&conn, &b.to_string_lossy(), "claude")
            .unwrap()
            .value(Part::UsageSync)
            .is_none());
    }
}
