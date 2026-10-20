//! OpenCode 会话使用追踪
//!
//! Ported from Magpie `internal/sessions/calls_opencode.go` and `opencode.go`
//! (Copyright (c) 2026 yetone, MIT; see `../session_manager/MAGPIE_LICENSE`).
//!
//! OpenCode keeps a session as rows, not a file of lines: since 1.2 in its SQLite
//! database (`opencode.db`: `session` and `message` tables, each message a JSON
//! document in `data`, its id in the `id` column), before that as JSON files under
//! `storage/` (`session/<project>/<id>.json`, `message/<session>/<id>.json`). The
//! database is made from those files once and they stay behind, so where there is
//! a database only it is read.
//!
//! Each assistant message with tokens is one call: `input` already excludes the
//! cache (stored as is), the output counts the reasoning. A subagent's work is a
//! session of its own whose parent is the session it ran in, and counts there.
//! A message's tokens are written in place while its reply streams, so rows are
//! upserted. Exact physical source stamps avoid reading unchanged sources;
//! changed sources are fully enumerated because native update timestamps need
//! not advance when a message or its session ancestry changes.
//!
//! ## 数据流
//! ```text
//! opencode.db / storage/message/<sid>/*.json ─→ 原生身份去重 → proxy_request_logs (opencode_session)
//! ```

use crate::database::Database;
use crate::error::AppError;
use crate::proxy::usage::calculator::CostCalculator;
use crate::proxy::usage::parser::TokenUsage;
use crate::services::session_usage::SessionSyncResult;
use crate::services::usage_source::{self, Prepared, ProviderIndex, Snapshot};
use crate::services::usage_sync_db::find_session_pricing_checked;
use crate::services::usage_sync_db::{
    commit_sync_batch, read_sync_state, CheckpointChange, SyncState,
};
#[cfg(test)]
use crate::session_manager::providers::opencode::get_opencode_base_dir;
use rusqlite::{Connection, OpenFlags};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const DATA_SOURCE: &str = "opencode_session";

/// A message as OpenCode keeps it, in either store.
#[derive(Debug, Default, Deserialize)]
struct OcMessage {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "sessionID")]
    session_id: String,
    #[serde(default)]
    role: String,
    #[serde(default, rename = "modelID")]
    model_id: String,
    #[serde(default)]
    time: OcTime,
    tokens: Option<OcTokens>,
}

#[derive(Debug, Default, Deserialize)]
struct OcTime {
    #[serde(default)]
    created: i64,
    #[serde(default)]
    completed: i64,
}

#[derive(Debug, Default, Deserialize)]
struct OcTokens {
    #[serde(default)]
    input: u64,
    #[serde(default)]
    output: u64,
    #[serde(default)]
    reasoning: u64,
    #[serde(default)]
    cache: OcCache,
}

#[derive(Debug, Default, Deserialize)]
struct OcCache {
    #[serde(default)]
    read: u64,
    #[serde(default)]
    write: u64,
}

/// A session's id and its parent's, from a row or a `storage/session` file.
#[derive(Debug, Default, Deserialize)]
struct OcSessionInfo {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "parentID")]
    parent_id: Option<String>,
}

/// One assistant message as a call.
#[derive(Debug, Clone)]
struct OcCall {
    msg_id: String,
    /// The session at the top: a subagent's (and its subagent's) calls count there.
    session_id: String,
    model: String,
    usage: TokenUsage,
    created_at: i64,
    latency_ms: i64,
    source: usize,
    native_id: bool,
    native_session_id: String,
    at_ms: Option<i64>,
}

#[derive(Default)]
struct ParsedBatch {
    calls: Vec<OcCall>,
    checkpoints: Vec<CheckpointChange>,
    sources: Vec<Prepared>,
    dependencies: Vec<Snapshot>,
}

/// 同步 OpenCode 使用数据（数据库优先，否则读 storage/ 下的 JSON 文件）
#[cfg(test)]
pub fn sync_opencode_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    let root = super::usage_sync::resolve_root("opencode")?;
    sync_opencode_usage_at(db, &root)
}
pub(crate) fn sync_opencode_usage_at(
    db: &Database,
    base: &Path,
) -> Result<SessionSyncResult, AppError> {
    let db_path = base.join("opencode.db");
    let mut result = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: 0,
        errors: vec![],
    };
    let mut index = ProviderIndex::load(db, "opencode", base)?;
    let mut batch = if db_path.is_file() {
        result.files_scanned = 1;
        read_db_calls(db, &db_path, &index)?
    } else {
        read_storage_calls(db, &base.join("storage"), &mut result, &index)?
    };

    let (imported, skipped) = commit_sync_batch(db, &batch.checkpoints, |conn| {
        // Validate every physical source and the shared provider lease before
        // any fact; all directories are one batch, including late checkpoints.
        for dependency in &batch.dependencies {
            dependency.verify()?;
        }
        for source in &batch.sources {
            source.check_on(conn)?;
        }
        let mut imported = 0;
        let mut skipped = 0;
        for call in &batch.calls {
            let source = &mut batch.sources[call.source];
            if call.at_ms.is_none() {
                source.evidence.complete_event_identity_set = false;
                source.evidence.partial("missingTimestamp");
                skipped += 1;
                continue;
            }
            let native = usage_source::native_key("opencode", "", &call.msg_id);
            if !source.should_store_on(
                conn,
                &format!("opencode:{}", call.msg_id),
                &native,
                DATA_SOURCE,
                call.created_at,
            )? {
                skipped += 1;
                continue;
            }
            if upsert_call(conn, call)? {
                imported += 1;
            } else {
                skipped += 1;
            }
            if call.native_id {
                source.evidence.acknowledge(&native);
            } else {
                source.evidence.complete_event_identity_set = false;
                source.evidence.partial("missingNativeEventIdentity");
            }
        }
        for source in &mut batch.sources {
            let next = batch
                .checkpoints
                .iter()
                .find(|c| c.key == source.snapshot.path.to_string_lossy())
                .and_then(|c| c.next)
                .ok_or_else(|| AppError::Database("missing OpenCode source checkpoint".into()))?;
            source.save_on(conn, next)?;
        }
        Ok((imported, skipped))
    })?;
    index.advance_many(batch.sources);
    result.imported = imported;
    result.skipped = skipped;

    if result.imported > 0 {
        log::debug!(
            "[OPENCODE-SYNC] 同步完成: 写入 {} 条, 跳过 {} 条",
            result.imported,
            result.skipped
        );
    }
    Ok(result)
}

// ---- the database ------------------------------------------------------------

/// Parse outside the OFox DB lock, returning the watermark to commit with facts.
fn read_db_calls(
    db: &Database,
    db_path: &Path,
    index: &ProviderIndex,
) -> Result<ParsedBatch, AppError> {
    let key = db_path.to_string_lossy().to_string();
    let expected = read_sync_state(db, &key)?;
    let mut source = Prepared::with_index(db, Snapshot::sqlite(db_path)?, expected, 2, index)?;
    if source.unchanged && source.has_dependency() {
        return Ok(ParsedBatch::default());
    }
    let (calls, newest, dependency) = read_database(db_path)?;
    source.observe_dependency(&dependency);
    source.snapshot.verify()?;
    // JSON -> SQLite migration is deduplicated by provider native identities,
    // never by a timestamp that merely says a directory was scanned.
    Ok(ParsedBatch {
        calls,
        checkpoints: vec![CheckpointChange {
            key,
            expected,
            next: Some(SyncState {
                modified: newest,
                offset: 0,
            }),
        }],
        sources: vec![source],
        dependencies: Vec::new(),
    })
}

fn read_database(db_path: &Path) -> Result<(Vec<OcCall>, i64, String), AppError> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| AppError::Database("OpenCode database unavailable".into()))?;
    let before: i64 = conn.query_row("PRAGMA data_version", [], |r| r.get(0))?;
    let tx = conn.unchecked_transaction()?;
    let parents = db_session_parents(&tx)?;
    let dependency = parent_dependency(&parents)?;
    let mut newest = 0;
    let mut calls = Vec::new();
    {
        let mut stmt=tx.prepare("SELECT id,session_id,time_created,time_updated,data FROM message ORDER BY time_updated,id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (id, sid, _created, updated, data) = row?;
            newest = newest.max(updated);
            let mut message: OcMessage = serde_json::from_str(&data)
                .map_err(|_| AppError::Config("invalid OpenCode message".into()))?;
            message.id = id;
            message.session_id = sid;
            if let Some(call) = to_call(&message, &parents, 0, true)? {
                calls.push(call);
            }
        }
    }
    tx.commit()?;
    let after: i64 = conn.query_row("PRAGMA data_version", [], |r| r.get(0))?;
    if before != after {
        return Err(AppError::Database(
            "OpenCode database changed during read; retry".into(),
        ));
    }
    Ok((calls, newest, dependency))
}

fn parent_dependency(parents: &HashMap<String, String>) -> Result<String, AppError> {
    let mut edges: Vec<_> = parents.iter().collect();
    edges.sort();
    let canonical = serde_json::to_string(&edges)
        .map_err(|_| AppError::Config("OpenCode ancestry unavailable".into()))?;
    Ok(usage_source::native_digest(&canonical))
}

fn db_session_parents(conn: &Connection) -> Result<HashMap<String, String>, AppError> {
    let mut stmt = conn
        .prepare("SELECT id, COALESCE(parent_id, '') FROM session")
        .map_err(|e| AppError::Database(format!("查询 OpenCode 会话失败: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| AppError::Database(format!("查询 OpenCode 会话失败: {e}")))?;
    let mut parents = HashMap::new();
    for row in rows {
        let row = row.map_err(|e| AppError::Database(format!("读取 OpenCode 会话失败: {e}")))?;
        if !row.1.is_empty() {
            parents.insert(row.0, row.1);
        }
    }
    Ok(parents)
}

// ---- the files ---------------------------------------------------------------

/// Enumerate every message of a changed directory or changed ancestry graph,
/// together with each directory's checkpoint. Reading never publishes it.
fn read_storage_calls(
    db: &Database,
    storage: &Path,
    result: &mut SessionSyncResult,
    index: &ProviderIndex,
) -> Result<ParsedBatch, AppError> {
    let dependency = Snapshot::tree(&storage.join("session"))?;
    let parents = storage_session_parents(storage)?;
    let dependency_hash = parent_dependency(&parents)?;
    dependency.verify()?;
    let session_dirs = match fs::read_dir(storage.join("message")) {
        Ok(dirs) => dirs,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ParsedBatch::default()),
        Err(_) => {
            return Err(AppError::Config(
                "OpenCode message directories unavailable".into(),
            ))
        }
    };
    let mut dirs = Vec::new();
    for entry in session_dirs {
        let path = entry
            .map_err(|_| AppError::Config("OpenCode directory enumeration failed".into()))?
            .path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    let mut batch = ParsedBatch {
        dependencies: vec![dependency],
        ..ParsedBatch::default()
    };
    for dir in dirs {
        let key = dir.to_string_lossy().to_string();
        let expected = read_sync_state(db, &key)?;
        let mut source = Prepared::with_index(db, Snapshot::directory(&dir)?, expected, 2, index)?;
        source.observe_dependency(&dependency_hash);
        if source.unchanged {
            continue;
        }
        let mut newest = expected.unwrap_or_default().modified;
        let source_index = batch.sources.len();
        for (path, modified) in checked_json_files(&dir)? {
            newest = newest.max(modified);
            result.files_scanned += 1;
            let text = usage_source::read_text(&path)?;
            let mut message: OcMessage = serde_json::from_str(&text)
                .map_err(|_| AppError::Config("invalid OpenCode message".into()))?;
            let native_id = !message.id.is_empty();
            if !native_id {
                message.id = file_stem(&path);
                source.evidence.complete_event_identity_set = false;
                source.evidence.partial("missingNativeEventIdentity");
            }
            if message.session_id.is_empty() {
                message.session_id = file_name(&dir);
            }
            if let Some(call) = to_call(&message, &parents, source_index, native_id)? {
                batch.calls.push(call);
            }
        }
        source.snapshot.verify()?;
        batch.checkpoints.push(CheckpointChange {
            key,
            expected,
            next: Some(SyncState {
                modified: newest,
                offset: 0,
            }),
        });
        batch.sources.push(source);
    }
    Ok(batch)
}
fn checked_json_files(dir: &Path) -> Result<Vec<(PathBuf, i64)>, AppError> {
    let entries =
        fs::read_dir(dir).map_err(|_| AppError::Config("OpenCode directory unavailable".into()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| AppError::Config("OpenCode enumeration failed".into()))?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .map_err(|_| AppError::Config("OpenCode file metadata unavailable".into()))?
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| AppError::Config("invalid OpenCode file time".into()))?
            .as_millis();
        files.push((
            path,
            i64::try_from(modified)
                .map_err(|_| AppError::Config("OpenCode file time exceeds range".into()))?,
        ));
    }
    files.sort();
    Ok(files)
}

fn storage_session_parents(storage: &Path) -> Result<HashMap<String, String>, AppError> {
    let mut parents = HashMap::new();
    let projects = match fs::read_dir(storage.join("session")) {
        Ok(dirs) => dirs,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(parents),
        Err(_) => return Err(AppError::Config("OpenCode sessions unavailable".into())),
    };
    for project in projects {
        let project = project
            .map_err(|_| AppError::Config("OpenCode session enumeration failed".into()))?
            .path();
        if !project.is_dir() {
            continue;
        }
        for (path, _) in checked_json_files(&project)? {
            let text = usage_source::read_text(&path)?;
            let info: OcSessionInfo = serde_json::from_str(&text)
                .map_err(|_| AppError::Config("invalid OpenCode session".into()))?;
            if let Some(parent) = info.parent_id {
                if !info.id.is_empty() && !parent.is_empty() {
                    parents.insert(info.id, parent);
                }
            }
        }
    }
    Ok(parents)
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

// ---- a message as a call -----------------------------------------------------

/// The session at the top of `session`'s parent chain.
fn root_session(session: &str, parents: &HashMap<String, String>) -> String {
    let mut root = session;
    for _ in 0..64 {
        match parents.get(root) {
            Some(parent) if !parent.is_empty() => root = parent,
            _ => break,
        }
    }
    root.to_string()
}

/// An assistant message as a call: its tokens and model, its time the reply's end
/// (else its start). None for any other message, or one with no tokens or model.
fn to_call(
    message: &OcMessage,
    parents: &HashMap<String, String>,
    source: usize,
    native_id: bool,
) -> Result<Option<OcCall>, AppError> {
    if message.role != "assistant" || message.model_id.is_empty() || message.id.is_empty() {
        return Ok(None);
    }
    let Some(tokens) = message.tokens.as_ref() else {
        return Ok(None);
    };
    let usage = TokenUsage {
        input_tokens: usage_source::checked_token(tokens.input)?,
        output_tokens: usage_source::checked_token(
            tokens
                .output
                .checked_add(tokens.reasoning)
                .ok_or_else(|| AppError::Config("OpenCode output overflow".into()))?,
        )?,
        cache_read_tokens: usage_source::checked_token(tokens.cache.read)?,
        cache_creation_tokens: usage_source::checked_token(tokens.cache.write)?,
        cache_creation_1h_tokens: 0,
        model: Some(message.model_id.clone()),
        message_id: None,
    };
    if usage.input_tokens == 0
        && usage.output_tokens == 0
        && usage.cache_read_tokens == 0
        && usage.cache_creation_tokens == 0
    {
        return Ok(None);
    }
    let (created, completed) = (message.time.created, message.time.completed);
    let at_ms = if completed > 0 { completed } else { created };
    let latency_ms = if created > 0 && completed > created {
        completed - created
    } else {
        0
    };
    Ok(Some(OcCall {
        msg_id: message.id.clone(),
        session_id: root_session(&message.session_id, parents),
        model: message.model_id.clone(),
        usage,
        created_at: at_ms / 1000,
        at_ms: (at_ms > 0).then_some(at_ms),
        latency_ms,
        source,
        native_id,
        native_session_id: message.session_id.clone(),
    }))
}

fn call_to_fact(c: OcCall) -> usage_source::NormalizedUsageFact {
    usage_source::NormalizedUsageFact {
        request_id: format!("opencode:{}", c.msg_id),
        native_key: c
            .native_id
            .then(|| usage_source::native_key("opencode", "", &c.msg_id)),
        session_id: Some(c.native_session_id),
        model: c.model,
        usage: c.usage,
        at_ms: c.at_ms,
    }
}
/// Reuses the ledger's token semantics for a message already read in a native
/// transaction. The caller supplies authoritative native row identifiers.
pub(crate) fn normalized_message(
    value: serde_json::Value,
    native_id: bool,
) -> Result<Option<usage_source::NormalizedUsageFact>, AppError> {
    let message: OcMessage = serde_json::from_value(value)
        .map_err(|_| AppError::Config("invalid OpenCode message".into()))?;
    Ok(to_call(&message, &HashMap::new(), 0, native_id)?.map(call_to_fact))
}

pub(crate) fn normalized_facts(
    path: &Path,
) -> Result<(Vec<usage_source::NormalizedUsageFact>, bool), AppError> {
    let calls = if path.is_file() {
        read_database(path)?.0
    } else {
        let parents = storage_session_parents(path)?;
        let root = path.join("message");
        let mut calls = Vec::new();
        let dirs = match fs::read_dir(&root) {
            Ok(dirs) => Some(dirs),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                return Err(AppError::Config(
                    "OpenCode message directories unavailable".into(),
                ))
            }
        };
        if let Some(dirs) = dirs {
            for dir in dirs {
                let dir = dir
                    .map_err(|_| AppError::Config("OpenCode enumeration failed".into()))?
                    .path();
                if !dir.is_dir() {
                    continue;
                }
                for (file, _) in checked_json_files(&dir)? {
                    let text = usage_source::read_text(&file)?;
                    let mut message: OcMessage = serde_json::from_str(&text)
                        .map_err(|_| AppError::Config("invalid OpenCode message".into()))?;
                    let native_id = !message.id.is_empty();
                    if !native_id {
                        message.id = file_stem(&file);
                    }
                    if message.session_id.is_empty() {
                        message.session_id = file_name(&dir);
                    }
                    if let Some(call) = to_call(&message, &parents, 0, native_id)? {
                        calls.push(call);
                    }
                }
            }
        }
        calls
    };
    Ok((calls.into_iter().map(call_to_fact).collect(), true))
}

/// Insert a call, or update its row when the reply has gone on since; returns
/// whether the row changed.
fn upsert_call(conn: &Connection, call: &OcCall) -> Result<bool, AppError> {
    let request_id = format!("opencode:{}", call.msg_id);
    let cost = find_session_pricing_checked(conn, &call.model)?
        .map(|p| CostCalculator::calculate(&call.usage, &p, Decimal::ONE));
    let costs = |pick: fn(&crate::proxy::usage::CostBreakdown) -> Decimal| {
        cost.as_ref()
            .map(|c| pick(c).to_string())
            .unwrap_or_else(|| "0".to_string())
    };

    let changed = conn
        .execute(
            "INSERT INTO proxy_request_logs (
                request_id, provider_id, app_type, model, request_model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                total_cost_usd, latency_ms, first_token_ms, status_code, error_message, session_id,
                provider_type, is_streaming, cost_multiplier, created_at, data_source
            ) VALUES (?1, '_opencode_session', 'opencode', ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, NULL, 200, NULL, ?13, ?14, 1, '1.0', ?15, ?14)
            ON CONFLICT(request_id) DO UPDATE SET
                model = excluded.model,
                request_model = excluded.request_model,
                input_tokens = excluded.input_tokens,
                output_tokens = excluded.output_tokens,
                cache_read_tokens = excluded.cache_read_tokens,
                cache_creation_tokens = excluded.cache_creation_tokens,
                input_cost_usd = excluded.input_cost_usd,
                output_cost_usd = excluded.output_cost_usd,
                cache_read_cost_usd = excluded.cache_read_cost_usd,
                cache_creation_cost_usd = excluded.cache_creation_cost_usd,
                total_cost_usd = excluded.total_cost_usd,
                latency_ms = excluded.latency_ms,
                session_id = excluded.session_id,
                created_at = excluded.created_at
            WHERE data_source = excluded.data_source AND (
                input_tokens != excluded.input_tokens
                OR output_tokens != excluded.output_tokens
                OR cache_read_tokens != excluded.cache_read_tokens
                OR cache_creation_tokens != excluded.cache_creation_tokens
                OR model != excluded.model
                OR session_id IS NOT excluded.session_id
                OR created_at != excluded.created_at)",
            rusqlite::params![
                request_id,
                call.model,
                call.usage.input_tokens,
                call.usage.output_tokens,
                call.usage.cache_read_tokens,
                call.usage.cache_creation_tokens,
                costs(|c| c.input_cost),
                costs(|c| c.output_cost),
                costs(|c| c.cache_read_cost),
                costs(|c| c.cache_creation_cost),
                costs(|c| c.total_cost),
                call.latency_ms,
                call.session_id,
                DATA_SOURCE,
                call.created_at,
            ],
        )
        .map_err(|e| AppError::Database(format!("写入 OpenCode 会话用量失败: {e}")))?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_sandbox::sandbox;
    use serial_test::serial;

    const MAIN: &str = "ses_1a2b3c4d5e6fAAAAAAAAAAAAAA";

    /// Magpie's OpenCode fixtures (MIT, see tests/fixtures/opencode/).
    fn fixture_storage() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/opencode/storage")
    }

    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap().flatten() {
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// (request_id, session, model, input, output, cache read, cache write, created_at)
    type Row = (String, String, String, i64, i64, i64, i64, i64);

    fn rows(db: &Database) -> Vec<Row> {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT request_id, session_id, model, input_tokens, output_tokens,
                        cache_read_tokens, cache_creation_tokens, created_at
                 FROM proxy_request_logs
                 WHERE data_source = 'opencode_session' AND app_type = 'opencode'
                 ORDER BY request_id",
            )
            .unwrap();
        let mapped = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })
            .unwrap();
        mapped.map(|r| r.unwrap()).collect()
    }

    /// The fixtures' replies as rows: the reasoning in the output, the subagent's
    /// (msg_0004, in the child session) counted in the session it ran under.
    fn expected_rows() -> Vec<Row> {
        let row = |id: &str, model: &str, t: (i64, i64, i64, i64), at: i64| {
            (
                format!("opencode:{id}"),
                MAIN.to_string(),
                model.to_string(),
                t.0,
                t.1,
                t.2,
                t.3,
                at,
            )
        };
        vec![
            row("msg_0002", "gpt-6-astra", (1000, 120, 5000, 0), 1790416830),
            row(
                "msg_0003",
                "codex/gpt-6-luna",
                (300, 60, 2000, 0),
                1790416980,
            ),
            row(
                "msg_0004",
                "claude-opus-5-5",
                (40, 60, 700, 300),
                1790416890,
            ),
        ]
    }

    fn opencode_dir() -> PathBuf {
        get_opencode_base_dir()
    }

    /// OpenCode's database built from the fixtures as OpenCode 1.2+ keeps it: the
    /// message ids in their column, not in the document.
    fn make_db(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id text PRIMARY KEY, project_id text NOT NULL,
                parent_id text, slug text NOT NULL, directory text NOT NULL,
                title text NOT NULL, version text NOT NULL, time_created integer NOT NULL,
                time_updated integer NOT NULL);
             CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL,
                time_created integer NOT NULL, time_updated integer NOT NULL,
                data text NOT NULL);",
        )
        .unwrap();
        let storage = fixture_storage();
        for (path, _) in checked_json_files(&storage.join("session/0f3a")).unwrap() {
            let info: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            conn.execute(
                "INSERT INTO session VALUES (?1, '0f3a', ?2, 's', '/work/oc', 't', '1.1.53', 0, 0)",
                rusqlite::params![info["id"].as_str(), info["parentID"].as_str()],
            )
            .unwrap();
        }
        for dir in fs::read_dir(storage.join("message")).unwrap().flatten() {
            for (path, _) in checked_json_files(&dir.path()).unwrap() {
                let mut doc: serde_json::Value =
                    serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
                let id = doc["id"].as_str().unwrap().to_string();
                let sid = doc["sessionID"].as_str().unwrap().to_string();
                let created = doc["time"]["created"].as_i64().unwrap();
                let updated = doc["time"]["completed"].as_i64().unwrap_or(created);
                let obj = doc.as_object_mut().unwrap();
                obj.remove("id");
                obj.remove("sessionID");
                conn.execute(
                    "INSERT INTO message VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![id, sid, created, updated, doc.to_string()],
                )
                .unwrap();
            }
        }
        conn
    }

    #[test]
    #[serial]
    fn json_storage_replies_are_imported_with_subagents_under_their_root() {
        let _sandbox = sandbox();
        copy_tree(&fixture_storage(), &opencode_dir().join("storage"));
        let db = Database::memory().unwrap();

        sync_opencode_usage(&db).unwrap();

        assert_eq!(rows(&db), expected_rows());
    }

    #[test]
    #[serial]
    fn json_storage_is_not_counted_twice_on_the_next_sync() {
        let _sandbox = sandbox();
        copy_tree(&fixture_storage(), &opencode_dir().join("storage"));
        let db = Database::memory().unwrap();
        sync_opencode_usage(&db).unwrap();

        let again = sync_opencode_usage(&db).unwrap();

        assert_eq!(again.imported, 0);
        assert_eq!(rows(&db), expected_rows());
    }

    #[test]
    #[serial]
    fn database_replies_are_imported_and_a_streaming_reply_is_updated() {
        let _sandbox = sandbox();
        fs::create_dir_all(opencode_dir()).unwrap();
        // the JSON files stay behind after the database is made; only it is read
        copy_tree(&fixture_storage(), &opencode_dir().join("storage"));
        let oc = make_db(&opencode_dir().join("opencode.db"));
        let db = Database::memory().unwrap();

        sync_opencode_usage(&db).unwrap();
        assert_eq!(rows(&db), expected_rows());

        // Native timestamps are not append proof: changed bytes must refresh
        // an owned live fact even when time_updated did not advance.
        oc.execute(
            "UPDATE message SET data = json_set(data, '$.tokens.input', 9999)
             WHERE id = 'msg_0002'",
            [],
        )
        .unwrap();
        sync_opencode_usage(&db).unwrap();
        assert_eq!(rows(&db)[0].3, 9999);

        // the reply goes on: OpenCode writes its tokens and stamps it now
        oc.execute(
            "UPDATE message SET data = json_set(data, '$.tokens.output', 400),
                time_updated = 1790417100000
             WHERE id = 'msg_0002'",
            [],
        )
        .unwrap();
        let result = sync_opencode_usage(&db).unwrap();
        assert_eq!(result.imported, 1);
        let rows = rows(&db);
        assert_eq!(rows.len(), 3);
        assert_eq!((rows[0].3, rows[0].4), (9999, 420));
    }

    const LONG: &str = "ses_long";

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    fn days_ago_ms(days: i64) -> i64 {
        now_ms() - days * 86_400_000
    }

    /// An assistant message document as the JSON storage keeps it.
    fn message_doc(id: &str, at_ms: i64) -> serde_json::Value {
        serde_json::json!({"id": id, "sessionID": LONG, "role": "assistant",
            "modelID": "gpt-5.4", "tokens": {"input": 10, "output": 5, "reasoning": 0,
                "cache": {"read": 0, "write": 0}},
            "time": {"created": at_ms - 1000, "completed": at_ms}})
    }

    /// OpenCode's database holding `LONG` with a reply 40 days old and one from
    /// yesterday, each stamped when it was written.
    fn make_long_db(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id text PRIMARY KEY, parent_id text);
             CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL,
                time_created integer NOT NULL, time_updated integer NOT NULL,
                data text NOT NULL);",
        )
        .unwrap();
        conn.execute("INSERT INTO session VALUES (?1, NULL)", [LONG])
            .unwrap();
        for (id, at) in [("msg_old", days_ago_ms(40)), ("msg_new", days_ago_ms(1))] {
            let mut doc = message_doc(id, at);
            doc.as_object_mut().unwrap().remove("id");
            conn.execute(
                "INSERT INTO message VALUES (?1, ?2, ?3, ?3, ?4)",
                rusqlite::params![id, LONG, at, doc.to_string()],
            )
            .unwrap();
        }
        conn
    }

    /// Calls of `LONG`: live rows plus the archive.
    fn long_session_calls(db: &Database) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT (SELECT COUNT(*) FROM proxy_request_logs WHERE session_id = ?1)
                  + (SELECT COALESCE(SUM(request_count), 0) FROM session_usage_archive
                     WHERE session_id = ?1)",
            [LONG],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    #[serial]
    fn a_first_read_imports_old_replies_so_the_prune_can_archive_them() {
        let _sandbox = sandbox();
        fs::create_dir_all(opencode_dir()).unwrap();
        make_long_db(&opencode_dir().join("opencode.db"));
        let db = Database::memory().unwrap();

        sync_opencode_usage(&db).unwrap();
        db.rollup_and_prune(30).unwrap();

        assert_eq!(long_session_calls(&db), 2);
    }

    #[test]
    #[serial]
    fn an_old_reply_written_again_after_the_prune_is_not_stored_again() {
        let _sandbox = sandbox();
        fs::create_dir_all(opencode_dir()).unwrap();
        let oc = make_long_db(&opencode_dir().join("opencode.db"));
        let db = Database::memory().unwrap();
        sync_opencode_usage(&db).unwrap();
        db.rollup_and_prune(30).unwrap();

        // OpenCode writes the old message again: it moves past the watermark
        oc.execute(
            "UPDATE message SET time_updated = ?1 WHERE id = 'msg_old'",
            [now_ms()],
        )
        .unwrap();
        sync_opencode_usage(&db).unwrap();
        db.rollup_and_prune(30).unwrap();

        assert_eq!(long_session_calls(&db), 2);
    }

    #[test]
    #[serial]
    fn replies_read_from_json_are_not_stored_again_when_opencode_moves_to_its_database() {
        let _sandbox = sandbox();
        let storage = opencode_dir().join("storage");
        for (id, at) in [("msg_old", days_ago_ms(40)), ("msg_new", days_ago_ms(1))] {
            let path = storage.join(format!("message/{LONG}/{id}.json"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, message_doc(id, at).to_string()).unwrap();
        }
        let db = Database::memory().unwrap();
        sync_opencode_usage(&db).unwrap();
        db.rollup_and_prune(30).unwrap();

        // OpenCode 1.2 copies the JSON files into its new database
        make_long_db(&opencode_dir().join("opencode.db"));
        sync_opencode_usage(&db).unwrap();
        db.rollup_and_prune(30).unwrap();

        assert_eq!(long_session_calls(&db), 2);
    }

    /// Write `doc` as a JSON storage message of `LONG`, its file last written at `at_ms`.
    fn write_json_message(storage: &Path, doc: &serde_json::Value, at_ms: i64) {
        let path = storage.join(format!(
            "message/{LONG}/{}.json",
            doc["id"].as_str().unwrap()
        ));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, doc.to_string()).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(at_ms as u64))
            .unwrap();
    }

    #[test]
    #[serial]
    fn a_reply_first_seen_after_a_long_gap_is_counted_once_in_the_database() {
        let _sandbox = sandbox();
        fs::create_dir_all(opencode_dir()).unwrap();
        let oc = make_long_db(&opencode_dir().join("opencode.db"));
        // the last read was 45 days ago: only a reply from then existed
        oc.execute("DELETE FROM message WHERE id = 'msg_new'", [])
            .unwrap();
        oc.execute(
            "UPDATE message SET time_created = ?1, time_updated = ?1 WHERE id = 'msg_old'",
            [days_ago_ms(45)],
        )
        .unwrap();
        let db = Database::memory().unwrap();
        sync_opencode_usage(&db).unwrap();
        db.rollup_and_prune(30).unwrap();

        // Ofox did not run since; a reply completed 40 days ago appears
        let mut doc = message_doc("msg_gap", days_ago_ms(40));
        doc.as_object_mut().unwrap().remove("id");
        oc.execute(
            "INSERT INTO message VALUES ('msg_gap', ?1, ?2, ?2, ?3)",
            rusqlite::params![LONG, days_ago_ms(40), doc.to_string()],
        )
        .unwrap();
        sync_opencode_usage(&db).unwrap();
        db.rollup_and_prune(30).unwrap();

        assert_eq!(long_session_calls(&db), 2);
    }

    #[test]
    #[serial]
    fn a_reply_first_seen_after_a_long_gap_is_counted_once_in_json_storage() {
        let _sandbox = sandbox();
        let storage = opencode_dir().join("storage");
        write_json_message(
            &storage,
            &message_doc("msg_old", days_ago_ms(45)),
            days_ago_ms(45),
        );
        let db = Database::memory().unwrap();
        sync_opencode_usage(&db).unwrap();
        db.rollup_and_prune(30).unwrap();

        write_json_message(
            &storage,
            &message_doc("msg_gap", days_ago_ms(40)),
            days_ago_ms(40),
        );
        sync_opencode_usage(&db).unwrap();
        db.rollup_and_prune(30).unwrap();

        assert_eq!(long_session_calls(&db), 2);
    }
}
