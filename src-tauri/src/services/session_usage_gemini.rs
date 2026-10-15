//! Gemini CLI 会话日志使用追踪
//!
//! 从 ~/.gemini/tmp/<project_hash>/chats/session-*.json 中提取精确 token 使用数据。
//!
//! ## 数据流
//! ```text
//! ~/.gemini/tmp/*/chats/session-*.json → 全量解析 → 费用计算 → proxy_request_logs 表
//! ```
//!
//! ## 与 Claude/Codex 解析器的差异
//! - JSON 格式（非 JSONL）：每个文件是单个 JSON 对象，包含 messages 数组
//! - 无需 delta 计算：tokens 字段是 per-message 独立值
//! - 无需状态恢复：不依赖前一条消息的累计值
//! - 天然去重：每条消息有唯一 id 字段

use crate::database::Database;
use crate::error::AppError;
use crate::gemini_config::get_gemini_dir;
use crate::proxy::usage::calculator::{CostCalculator, ModelPricing};
use crate::proxy::usage::parser::TokenUsage;
use crate::services::session_usage::SessionSyncResult;
#[cfg(test)]
use crate::services::session_usage::SessionUsageEntry;
use crate::services::usage_source::{self, Prepared, ProviderIndex, Snapshot};
use crate::services::usage_sync_db::{
    commit_sync_batch, read_sync_state, CheckpointChange, SyncState,
};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 从 Gemini message 中提取的 token 数据
#[derive(Debug)]
struct GeminiTokens {
    input: u32,
    output: u32,
    cached: u32,
    thoughts: u32,
    combined_output: u32,
}

/// 同步 Gemini 使用数据（从 JSON 会话日志）
pub fn sync_gemini_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    let gemini_dir = get_gemini_dir();

    let files = collect_gemini_session_files(&gemini_dir);

    let mut result = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: files.len() as u32,
        errors: vec![],
    };

    if files.is_empty() {
        return Ok(result);
    }

    let mut index = ProviderIndex::load(db, "gemini", &gemini_dir)?;
    for file_path in &files {
        match sync_single_gemini_file_with_index(db, file_path, &mut index) {
            Ok((imported, skipped)) => {
                result.imported += imported;
                result.skipped += skipped;
            }
            Err(e) => {
                let msg = format!("Gemini 会话文件解析失败 {}: {e}", file_path.display());
                log::warn!("[GEMINI-SYNC] {msg}");
                result.errors.push(msg);
            }
        }
    }

    if result.imported > 0 {
        log::debug!(
            "[GEMINI-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }

    Ok(result)
}

/// 收集所有 Gemini 会话 JSON 文件
fn collect_gemini_session_files(gemini_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    let tmp_dir = gemini_dir.join("tmp");
    if !tmp_dir.is_dir() {
        return files;
    }

    // 遍历 tmp/<project_hash>/chats/session-*.json
    let project_dirs = match fs::read_dir(&tmp_dir) {
        Ok(entries) => entries,
        Err(_) => return files,
    };

    for entry in project_dirs.flatten() {
        let chats_dir = entry.path().join("chats");
        if !chats_dir.is_dir() {
            continue;
        }

        let chat_files = match fs::read_dir(&chats_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for file_entry in chat_files.flatten() {
            let path = file_entry.path();
            let is_session = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("session-") && n.ends_with(".json"))
                .unwrap_or(false);
            if is_session {
                files.push(path);
            }
        }
    }

    files
}

/// 同步单个 Gemini 会话 JSON 文件，返回 (imported, skipped)
#[cfg(test)]
fn sync_single_gemini_file(db: &Database, file_path: &Path) -> Result<(u32, u32), AppError> {
    let mut index = ProviderIndex::load(db, "gemini", file_path.parent().unwrap())?;
    sync_single_gemini_file_with_index(db, file_path, &mut index)
}
fn sync_single_gemini_file_with_index(
    db: &Database,
    file_path: &Path,
    index: &mut ProviderIndex,
) -> Result<(u32, u32), AppError> {
    let file_path_str = file_path.to_string_lossy().to_string();

    // 获取文件元数据
    let metadata = fs::metadata(file_path)
        .map_err(|e| AppError::Config(format!("无法读取文件元数据: {e}")))?;
    let file_modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // 检查同步状态
    let expected = read_sync_state(db, &file_path_str)?;

    let mut source = Prepared::with_index(db, Snapshot::file(file_path)?, expected, 2, index)?;
    if source.unchanged {
        return Ok((0, 0));
    }

    // 读取并解析整个 JSON 文件
    let (session_id, calls, raw_count) = parse_gemini_file(file_path)?;
    let checkpoint = CheckpointChange {
        key: file_path_str,
        expected,
        next: Some(SyncState {
            modified: file_modified,
            offset: raw_count,
        }),
    };
    let next = checkpoint.next.unwrap();
    let result = commit_sync_batch(db, &[checkpoint], |conn| {
        source.check_on(conn)?;
        let mut imported: u32 = 0;
        let mut skipped: u32 = 0;

        for call in &calls {
            // 生成唯一 request_id
            let session_id_str = session_id.as_deref().unwrap_or("unknown");
            let request_id = format!("gemini_session:{session_id_str}:{}", call.message_id);

            let Some(created_at) = created_at_of(call.timestamp.as_deref()) else {
                source.evidence.complete_event_identity_set = false;
                source.evidence.partial("missingTimestamp");
                skipped += 1;
                continue;
            };
            let native = usage_source::native_key("gemini", session_id_str, &call.message_id);
            if !source.should_store_on(conn, &request_id, &native, "gemini_session", created_at)? {
                skipped += 1;
                continue;
            }
            if insert_gemini_session_entry(
                conn,
                &request_id,
                &call.tokens,
                &call.model,
                session_id.as_deref(),
                created_at,
            )? {
                imported += 1;
            } else {
                skipped += 1;
            }
            if call.native_id && session_id.as_deref().is_some_and(|s| !s.is_empty()) {
                source.evidence.acknowledge(&native);
            } else {
                source.evidence.complete_event_identity_set = false;
                source.evidence.partial("missingNativeEventIdentity");
            }
        }
        source.save_on(conn, next)?;
        Ok((imported, skipped))
    })?;
    index.advance(source);
    Ok(result)
}

/// One Gemini reply with tokens.
struct GeminiCall {
    message_id: String,
    native_id: bool,
    model: String,
    tokens: GeminiTokens,
    timestamp: Option<String>,
}

/// A chat file's session id and its replies with tokens, in order.
fn parse_gemini_file(file_path: &Path) -> Result<(Option<String>, Vec<GeminiCall>, i64), AppError> {
    let content = usage_source::read_text(file_path)?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| AppError::Config(format!("JSON 解析失败: {e}")))?;

    // 提取顶层 sessionId
    let session_id = value
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // 遍历 messages 数组
    let Some(messages) = value.get("messages").and_then(|v| v.as_array()) else {
        return Ok((session_id, Vec::new(), 0));
    };

    let mut calls = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();
    let mut raw_count = 0;
    for msg in messages {
        // 只处理 type == "gemini" 的消息
        if msg.get("type").and_then(|t| t.as_str()) != Some("gemini") {
            continue;
        }

        // 提取 tokens 对象
        let tokens_obj = match msg.get("tokens") {
            Some(t) if t.is_object() => t,
            _ => continue,
        };

        let tokens = parse_gemini_tokens(tokens_obj)?;
        if tokens.input == 0 && tokens.output == 0 && tokens.thoughts == 0 && tokens.cached == 0 {
            continue; // 跳过全零的空 token 消息
        }

        // 提取消息 ID 和模型
        raw_count += 1;
        let call = GeminiCall {
            native_id: msg
                .get("id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| !id.is_empty()),
            message_id: msg
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            model: msg
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            tokens,
            timestamp: msg
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        };
        if let Some(&index) = index_of.get(&call.message_id) {
            calls[index] = call;
        } else {
            index_of.insert(call.message_id.clone(), calls.len());
            calls.push(call);
        }
    }
    Ok((session_id, calls, raw_count))
}

/// Preserve unknown native time rather than manufacturing a current date.
fn created_at_of(timestamp: Option<&str>) -> Option<i64> {
    usage_source::timestamp_ms(timestamp).map(|ms| ms.div_euclid(1000))
}

/// Fresh input as every source stores it: Gemini's input (promptTokenCount)
/// counts the cached tokens.
fn usage_of(tokens: &GeminiTokens, model: &str) -> TokenUsage {
    TokenUsage {
        input_tokens: tokens.input.saturating_sub(tokens.cached),
        // 合并 thoughts 到 output（思考 token 按输出计费）
        output_tokens: tokens.combined_output,
        cache_read_tokens: tokens.cached,
        cache_creation_tokens: 0,
        model: Some(model.to_string()),
        message_id: None,
    }
}

/// Every Gemini reply the sync would store, read from every chat file in full
/// (a request id counts once). Files without a sessionId are left out.
pub(crate) fn normalized_facts(
    path: &Path,
) -> Result<(Vec<usage_source::NormalizedUsageFact>, bool), AppError> {
    let (session, calls, _) = parse_gemini_file(path)?;
    Ok((
        calls
            .into_iter()
            .map(|c| usage_source::NormalizedUsageFact {
                request_id: format!(
                    "gemini_session:{}:{}",
                    session.as_deref().unwrap_or("unknown"),
                    c.message_id
                ),
                native_key: (c.native_id && session.as_deref().is_some_and(|s| !s.is_empty()))
                    .then(|| {
                        usage_source::native_key(
                            "gemini",
                            session.as_deref().unwrap(),
                            &c.message_id,
                        )
                    }),
                at_ms: usage_source::timestamp_ms(c.timestamp.as_deref()),
                session_id: session.clone(),
                usage: usage_of(&c.tokens, &c.model),
                model: c.model,
            })
            .collect(),
        true,
    ))
}
#[cfg(test)]
pub(crate) fn session_entries() -> Vec<SessionUsageEntry> {
    let mut seen = std::collections::HashSet::new();
    collect_gemini_session_files(&get_gemini_dir())
        .into_iter()
        .flat_map(|p| usage_source::read_normalized("gemini", &p).unwrap().facts)
        .filter(|f| seen.insert(f.request_id.clone()))
        .collect()
}

fn parse_gemini_tokens(tokens: &serde_json::Value) -> Result<GeminiTokens, AppError> {
    let input = usage_source::number(tokens, "input")?;
    let output = usage_source::number(tokens, "output")?;
    let cached = usage_source::number(tokens, "cached")?;
    let thoughts = usage_source::number(tokens, "thoughts")?;
    Ok(GeminiTokens {
        input: usage_source::checked_token(input)?,
        output: usage_source::checked_token(output)?,
        cached: usage_source::checked_token(cached)?,
        thoughts: usage_source::checked_token(thoughts)?,
        combined_output: usage_source::checked_token(
            output
                .checked_add(thoughts)
                .ok_or_else(|| AppError::Config("Gemini output integer overflow".into()))?,
        )?,
    })
}

/// 插入单条 Gemini 会话记录到 proxy_request_logs
fn insert_gemini_session_entry(
    conn: &rusqlite::Connection,
    request_id: &str,
    tokens: &GeminiTokens,
    model: &str,
    session_id: Option<&str>,
    created_at: i64,
) -> Result<bool, AppError> {
    // 计算费用
    let usage = usage_of(tokens, model);

    let pricing = find_gemini_pricing_checked(conn, model)?;
    let multiplier = Decimal::from(1);
    let (input_cost, output_cost, cache_read_cost, cache_creation_cost, total_cost) = match pricing
    {
        Some(p) => {
            let cost = CostCalculator::calculate(&usage, &p, multiplier);
            (
                cost.input_cost.to_string(),
                cost.output_cost.to_string(),
                cost.cache_read_cost.to_string(),
                cost.cache_creation_cost.to_string(),
                cost.total_cost.to_string(),
            )
        }
        None => (
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
        ),
    };

    // 使用 UPSERT：新记录插入，已存在记录更新 token 和费用（Gemini 全量重读可能携带更新值）
    conn.execute(
        "INSERT INTO proxy_request_logs (
            request_id, provider_id, app_type, model, request_model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
            latency_ms, first_token_ms, status_code, error_message, session_id,
            provider_type, is_streaming, cost_multiplier, created_at, data_source
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)
        ON CONFLICT(request_id) DO UPDATE SET
            model = excluded.model,
            input_tokens = excluded.input_tokens,
            output_tokens = excluded.output_tokens,
            cache_read_tokens = excluded.cache_read_tokens,
            input_cost_usd = excluded.input_cost_usd,
            output_cost_usd = excluded.output_cost_usd,
            cache_read_cost_usd = excluded.cache_read_cost_usd,
            cache_creation_cost_usd = excluded.cache_creation_cost_usd,
            total_cost_usd = excluded.total_cost_usd
        WHERE input_tokens != excluded.input_tokens
           OR output_tokens != excluded.output_tokens
           OR cache_read_tokens != excluded.cache_read_tokens
           OR model != excluded.model",
        rusqlite::params![
            request_id,
            "_gemini_session",   // provider_id
            "gemini",            // app_type
            model,
            model,               // request_model = model
            usage.input_tokens,  // fresh input
            usage.output_tokens,
            tokens.cached,
            0i64,                // cache_creation_tokens
            input_cost,
            output_cost,
            cache_read_cost,
            cache_creation_cost,
            total_cost,
            0i64,                // latency_ms
            Option::<i64>::None, // first_token_ms
            200i64,              // status_code
            Option::<String>::None, // error_message
            session_id.map(|s| s.to_string()),
            Some("gemini_session"), // provider_type
            1i64,                // is_streaming
            "1.0",               // cost_multiplier
            created_at,
            "gemini_session",    // data_source
        ],
    )
    .map_err(|e| AppError::Database(format!("插入 Gemini 会话日志失败: {e}")))?;

    // changes() > 0 表示新插入或已更新，== 0 表示值完全相同（无实际变更）
    Ok(conn.changes() > 0)
}

fn find_gemini_pricing_checked(
    conn: &rusqlite::Connection,
    model_id: &str,
) -> Result<Option<ModelPricing>, AppError> {
    if let Some(price) = crate::services::usage_sync_db::lookup_pricing(conn, model_id, false)? {
        return Ok(Some(price));
    }
    crate::services::usage_sync_db::lookup_pricing(conn, &format!("{model_id}%"), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_gemini_session_files_nonexistent() {
        let files = collect_gemini_session_files(Path::new("/nonexistent/path"));
        assert!(files.is_empty());
    }

    #[test]
    fn test_parse_gemini_tokens() {
        let json: serde_json::Value = serde_json::json!({
            "input": 8522,
            "output": 29,
            "cached": 3138,
            "thoughts": 405,
            "tool": 0,
            "total": 8956
        });
        let tokens = parse_gemini_tokens(&json).unwrap();
        assert_eq!(tokens.input, 8522);
        assert_eq!(tokens.output, 29);
        assert_eq!(tokens.cached, 3138);
        assert_eq!(tokens.thoughts, 405);
        // output + thoughts = 29 + 405 = 434（用于计费）
        assert_eq!(tokens.output + tokens.thoughts, 434);
    }

    #[test]
    fn test_parse_gemini_tokens_missing_fields() {
        // 缺少某些字段时应返回 0
        let json: serde_json::Value = serde_json::json!({
            "input": 100,
            "output": 50
        });
        let tokens = parse_gemini_tokens(&json).unwrap();
        assert_eq!(tokens.input, 100);
        assert_eq!(tokens.output, 50);
        assert_eq!(tokens.cached, 0);
        assert_eq!(tokens.thoughts, 0);
    }

    #[test]
    fn test_parse_gemini_tokens_all_zero() {
        let json: serde_json::Value = serde_json::json!({
            "input": 0,
            "output": 0,
            "cached": 0,
            "thoughts": 0,
            "tool": 0,
            "total": 0
        });
        let tokens = parse_gemini_tokens(&json).unwrap();
        assert_eq!(tokens.input, 0);
        assert_eq!(tokens.output, 0);
        // 全零（包括 cached=0）会被 sync 逻辑跳过
        assert!(
            tokens.input == 0 && tokens.output == 0 && tokens.thoughts == 0 && tokens.cached == 0
        );
    }

    #[test]
    fn test_parse_gemini_tokens_cache_only_not_skipped() {
        // 纯缓存命中消息（input/output/thoughts=0 但 cached>0）不应被跳过
        let json: serde_json::Value = serde_json::json!({
            "input": 0,
            "output": 0,
            "cached": 5000,
            "thoughts": 0
        });
        let tokens = parse_gemini_tokens(&json).unwrap();
        assert_eq!(tokens.cached, 5000);
        // 跳过条件：所有四个字段都为 0 才跳过
        let should_skip =
            tokens.input == 0 && tokens.output == 0 && tokens.thoughts == 0 && tokens.cached == 0;
        assert!(!should_skip, "纯缓存命中记录不应被跳过");
    }

    #[test]
    fn gemini_rows_store_fresh_input_without_cached_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-fresh.json");
        std::fs::write(
            &path,
            r#"{"sessionId":"g-fresh","messages":[{"id":"m1","type":"gemini","model":"gemini-2.5-pro","timestamp":"2026-05-22T02:00:00Z","tokens":{"input":900,"output":30,"cached":600,"thoughts":10}}]}"#,
        )
        .unwrap();
        let db = Database::memory().unwrap();

        sync_single_gemini_file(&db, &path).unwrap();

        let conn = db.conn.lock().unwrap();
        let row: (i64, i64, i64) = conn
            .query_row(
                "SELECT input_tokens, output_tokens, cache_read_tokens FROM proxy_request_logs
                 WHERE request_id = 'gemini_session:g-fresh:m1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, (300, 40, 600));
    }

    fn chat_file(messages: &[(&str, i64)]) -> String {
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|(id, days)| {
                serde_json::json!({"id": id, "type": "gemini", "model": "gemini-2.5-pro",
                    "timestamp": (chrono::Utc::now() - chrono::Duration::days(*days)).to_rfc3339(),
                    "tokens": {"input": 100, "output": 10}})
            })
            .collect();
        serde_json::json!({"sessionId": "g-long", "messages": msgs}).to_string()
    }

    fn session_calls(db: &Database) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT (SELECT COUNT(*) FROM proxy_request_logs WHERE session_id = 'g-long')
                  + (SELECT COALESCE(SUM(request_count), 0) FROM session_usage_archive
                     WHERE session_id = 'g-long')",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn a_pruned_reply_is_not_logged_again_when_its_chat_file_grows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-long.json");
        std::fs::write(&path, chat_file(&[("old", 40), ("new", 1)])).unwrap();
        let db = Database::memory().unwrap();
        sync_single_gemini_file(&db, &path).unwrap();
        db.rollup_and_prune(30).unwrap();
        assert_eq!(session_calls(&db), 2);

        std::fs::write(&path, chat_file(&[("old", 40), ("new", 1), ("newer", 0)])).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        sync_single_gemini_file(&db, &path).unwrap();
        db.rollup_and_prune(30).unwrap();

        assert_eq!(session_calls(&db), 3);
    }
}
