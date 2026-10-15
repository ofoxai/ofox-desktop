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
//! upserted; progress is a `time_updated` watermark (database) or a file mtime
//! watermark (per session directory) in `session_log_sync`, so a message is read
//! again only when it changed.
//!
//! ## 数据流
//! ```text
//! opencode.db (message.time_updated ≥ watermark) ┐
//! storage/message/<sid>/*.json (mtime ≥ watermark) ┴→ 费用计算 → proxy_request_logs (opencode_session)
//! ```

use crate::database::dao::usage_rollup::usage_prune_cutoff;
use crate::database::Database;
use crate::error::AppError;
use crate::proxy::usage::calculator::CostCalculator;
use crate::proxy::usage::parser::TokenUsage;
use crate::services::session_usage::SessionSyncResult;
use crate::services::session_usage_codex::find_codex_pricing_checked;
use crate::services::usage_sync_db::{
    commit_sync_batch, read_sync_state, CheckpointChange, SyncState,
};
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

impl OcMessage {
    /// When the message was created (else completed), in milliseconds.
    fn created_ms(&self) -> i64 {
        if self.time.created > 0 {
            self.time.created
        } else {
            self.time.completed
        }
    }
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
    /// The message was created before what an earlier read already covered (the
    /// previous watermark of its source), so that read stored it: if its row is
    /// gone now and it is older than the prune cutoff, the prune archived it.
    covered_before: bool,
}

#[derive(Default)]
struct ParsedBatch {
    calls: Vec<OcCall>,
    checkpoints: Vec<CheckpointChange>,
}

/// 同步 OpenCode 使用数据（数据库优先，否则读 storage/ 下的 JSON 文件）
pub fn sync_opencode_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    let base = get_opencode_base_dir();
    let db_path = base.join("opencode.db");
    let mut result = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: 0,
        errors: vec![],
    };
    let batch = if db_path.is_file() {
        result.files_scanned = 1;
        read_db_calls(db, &db_path)?
    } else {
        read_storage_calls(db, &base.join("storage"), &mut result)?
    };

    let (imported, skipped) = commit_sync_batch(db, &batch.checkpoints, |conn| {
        // Prune cannot interleave its archive write and this live-row decision.
        let cutoff = usage_prune_cutoff()?;
        let mut imported = 0;
        let mut skipped = 0;
        for call in &batch.calls {
            // A message rewritten after the prune folded it into the rollups and the
            // archive moves past the watermark again; it is not stored a second time.
            // One read for the first time is stored however old it is.
            if call.covered_before && call.created_at < cutoff && !row_exists(conn, &call.msg_id)? {
                skipped += 1;
                continue;
            }
            if upsert_call(conn, call)? {
                imported += 1;
            } else {
                skipped += 1;
            }
        }
        Ok((imported, skipped))
    })?;
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
fn read_db_calls(db: &Database, db_path: &Path) -> Result<ParsedBatch, AppError> {
    let key = db_path.to_string_lossy().to_string();
    let expected = read_sync_state(db, &key)?;
    let watermark = expected.unwrap_or_default().modified;
    // OpenCode 1.2 builds its database from the JSON files: a message the JSON
    // read covered (created before its session directory's watermark) was read.
    let json_dir = db_path.with_file_name("storage").join("message");
    let mut json_watermarks: HashMap<String, i64> = HashMap::new();

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| AppError::Database(format!("无法打开 OpenCode 数据库: {e}")))?;

    let parents = db_session_parents(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, time_created, time_updated, data FROM message
             WHERE time_updated >= ?1 ORDER BY time_updated",
        )
        .map_err(|e| AppError::Database(format!("查询 OpenCode 消息失败: {e}")))?;
    let rows = stmt
        .query_map([watermark], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| AppError::Database(format!("查询 OpenCode 消息失败: {e}")))?;

    let mut batch = ParsedBatch::default();
    let mut newest = watermark;
    for row in rows {
        let (id, session_id, created, updated, data) =
            row.map_err(|e| AppError::Database(format!("读取 OpenCode 消息失败: {e}")))?;
        newest = newest.max(updated);
        let json_watermark = match json_watermarks.get(&session_id) {
            Some(w) => *w,
            None => {
                let dir_key = json_dir.join(&session_id).to_string_lossy().to_string();
                let expected = read_sync_state(db, &dir_key)?;
                let w = expected.unwrap_or_default().modified;
                batch.checkpoints.push(CheckpointChange {
                    key: dir_key,
                    expected,
                    next: None,
                });
                json_watermarks.insert(session_id.clone(), w);
                w
            }
        };
        let covered_before = covered_by(created, watermark.max(json_watermark));
        let Ok(mut message) = serde_json::from_str::<OcMessage>(&data) else {
            continue;
        };
        // the database keeps the ids in their columns, not in the document
        message.id = id;
        message.session_id = session_id;
        if let Some(call) = to_call(&message, &parents, covered_before) {
            batch.calls.push(call);
        }
    }
    batch.checkpoints.push(CheckpointChange {
        key,
        expected,
        next: Some(SyncState {
            modified: newest,
            offset: 0,
        }),
    });
    Ok(batch)
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

/// The calls among message files changed since each session directory's last
/// read, together with every directory's checkpoint. Reading never publishes it.
fn read_storage_calls(
    db: &Database,
    storage: &Path,
    result: &mut SessionSyncResult,
) -> Result<ParsedBatch, AppError> {
    let parents = storage_session_parents(storage);
    let Ok(session_dirs) = fs::read_dir(storage.join("message")) else {
        return Ok(ParsedBatch::default());
    };

    let mut batch = ParsedBatch::default();
    for dir in session_dirs
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
    {
        let key = dir.to_string_lossy().to_string();
        let expected = read_sync_state(db, &key)?;
        let watermark = expected.unwrap_or_default().modified;
        let mut newest = watermark;
        for (path, modified) in json_files_with_mtime(&dir) {
            if modified < watermark {
                continue;
            }
            newest = newest.max(modified);
            result.files_scanned += 1;
            let message = fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<OcMessage>(&text).ok());
            let Some(mut message) = message else {
                continue;
            };
            if message.id.is_empty() {
                message.id = file_stem(&path);
            }
            if message.session_id.is_empty() {
                message.session_id = file_name(&dir);
            }
            // a file read again (written since) whose message was created before
            // the directory's watermark was read before
            let covered_before = covered_by(message.created_ms(), watermark);
            if let Some(call) = to_call(&message, &parents, covered_before) {
                batch.calls.push(call);
            }
        }
        batch.checkpoints.push(CheckpointChange {
            key,
            expected,
            next: (newest > watermark).then_some(SyncState {
                modified: newest,
                offset: 0,
            }),
        });
    }
    Ok(batch)
}

fn storage_session_parents(storage: &Path) -> HashMap<String, String> {
    let mut parents = HashMap::new();
    let Ok(projects) = fs::read_dir(storage.join("session")) else {
        return parents;
    };
    for project in projects.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        for (path, _) in json_files_with_mtime(&project) {
            let info = fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<OcSessionInfo>(&text).ok());
            if let Some(OcSessionInfo {
                id,
                parent_id: Some(parent),
            }) = info
            {
                if !id.is_empty() && !parent.is_empty() {
                    parents.insert(id, parent);
                }
            }
        }
    }
    parents
}

/// The `.json` files directly in `dir`, each with its mtime in milliseconds.
fn json_files_with_mtime(dir: &Path) -> Vec<(PathBuf, i64)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(PathBuf, i64)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                return None;
            }
            let modified = entry
                .metadata()
                .ok()?
                .modified()
                .ok()?
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()?
                .as_millis() as i64;
            Some((path, modified))
        })
        .collect();
    files.sort();
    files
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
    covered_before: bool,
) -> Option<OcCall> {
    if message.role != "assistant" || message.model_id.is_empty() || message.id.is_empty() {
        return None;
    }
    let tokens = message.tokens.as_ref()?;
    let usage = TokenUsage {
        input_tokens: tokens.input as u32,
        output_tokens: (tokens.output + tokens.reasoning) as u32,
        cache_read_tokens: tokens.cache.read as u32,
        cache_creation_tokens: tokens.cache.write as u32,
        model: Some(message.model_id.clone()),
        message_id: None,
    };
    if usage.input_tokens == 0
        && usage.output_tokens == 0
        && usage.cache_read_tokens == 0
        && usage.cache_creation_tokens == 0
    {
        return None;
    }
    let (created, completed) = (message.time.created, message.time.completed);
    let at_ms = if completed > 0 { completed } else { created };
    if at_ms <= 0 {
        return None;
    }
    let latency_ms = if created > 0 && completed > created {
        completed - created
    } else {
        0
    };
    Some(OcCall {
        msg_id: message.id.clone(),
        session_id: root_session(&message.session_id, parents),
        model: message.model_id.clone(),
        usage,
        created_at: at_ms / 1000,
        latency_ms,
        covered_before,
    })
}

/// Whether a message created at `created_ms` was covered by the read that left
/// `watermark` (the newest time it read; messages at it are read again, so they
/// were read then too). No watermark: nothing was read.
fn covered_by(created_ms: i64, watermark: i64) -> bool {
    watermark > 0 && created_ms <= watermark
}

/// Whether the call of OpenCode message `msg_id` has a row.
fn row_exists(conn: &Connection, msg_id: &str) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT COUNT(*) FROM proxy_request_logs WHERE request_id = ?1",
        [format!("opencode:{msg_id}")],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(|e| AppError::Database(format!("查询 OpenCode 会话用量失败: {e}")))
}

/// Insert a call, or update its row when the reply has gone on since; returns
/// whether the row changed.
fn upsert_call(conn: &Connection, call: &OcCall) -> Result<bool, AppError> {
    let request_id = format!("opencode:{}", call.msg_id);
    let cost = find_codex_pricing_checked(conn, &call.model)?
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
        for (path, _) in json_files_with_mtime(&storage.join("session/0f3a")) {
            let info: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            conn.execute(
                "INSERT INTO session VALUES (?1, '0f3a', ?2, 's', '/work/oc', 't', '1.1.53', 0, 0)",
                rusqlite::params![info["id"].as_str(), info["parentID"].as_str()],
            )
            .unwrap();
        }
        for dir in fs::read_dir(storage.join("message")).unwrap().flatten() {
            for (path, _) in json_files_with_mtime(&dir.path()) {
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

        // tokens written in place without a newer time_updated: not read again
        oc.execute(
            "UPDATE message SET data = json_set(data, '$.tokens.input', 9999)
             WHERE id = 'msg_0002'",
            [],
        )
        .unwrap();
        sync_opencode_usage(&db).unwrap();
        assert_eq!(rows(&db)[0].3, 1000);

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
