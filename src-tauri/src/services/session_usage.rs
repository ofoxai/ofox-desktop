//! Claude Code 会话日志使用追踪
//!
//! 从 ~/.claude/projects/ 下的 JSONL 会话文件中提取 token 使用数据，
//! 实现无代理模式下的使用统计。
//!
//! ## 数据流
//! ```text
//! ~/.claude/projects/<p>/<sid>.jsonl            ┐
//! ~/.claude/projects/<p>/<sid>/subagents/*.jsonl ┴→ 增量解析 → 去重 → 费用计算 → proxy_request_logs 表
//! ```
//!
//! A subagent's transcript carries its parent's `sessionId`, so its calls count
//! toward the parent session. `input_tokens` is Claude's own: fresh input, without
//! the cache reads and writes.

#[cfg(test)]
use crate::config::get_claude_config_dir;
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::proxy::usage::calculator::{CostCalculator, ModelPricing};
use crate::proxy::usage::parser::TokenUsage;
use crate::services::usage_source::{self, Prepared, ProviderIndex, Snapshot};
use crate::services::usage_sync_db::{
    commit_sync_batch, read_sync_state, CheckpointChange, SyncState,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 同步结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSyncResult {
    pub imported: u32,
    pub skipped: u32,
    pub files_scanned: u32,
    pub errors: Vec<String>,
}

/// 数据来源分布
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceSummary {
    pub data_source: String,
    pub request_count: u32,
    pub total_cost_usd: String,
}

/// 从 JSONL 中解析出的 assistant 消息使用数据
#[derive(Debug)]
struct ParsedAssistantUsage {
    message_id: String,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
    cache_creation_1h_tokens: u32,
    stop_reason: Option<String>,
    timestamp: Option<String>,
    session_id: Option<String>,
}

/// 同步 Claude Code 会话日志到使用统计数据库
#[cfg(test)]
pub fn sync_claude_session_logs(db: &Database) -> Result<SessionSyncResult, AppError> {
    let root = super::usage_sync::resolve_root("claude")?;
    sync_claude_session_logs_at(db, &root)
}
pub(crate) fn sync_claude_session_logs_at(
    db: &Database,
    root: &Path,
) -> Result<SessionSyncResult, AppError> {
    let projects_dir = root.join("projects");
    if !projects_dir.exists() {
        return Ok(SessionSyncResult {
            imported: 0,
            skipped: 0,
            files_scanned: 0,
            errors: vec![],
        });
    }

    let mut result = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: 0,
        errors: vec![],
    };

    // 收集所有 .jsonl 文件
    let jsonl_files = collect_jsonl_files(&projects_dir);

    let mut index = ProviderIndex::load(db, "claude", &projects_dir)?;
    for file_path in &jsonl_files {
        result.files_scanned += 1;

        match sync_single_file_with_index(db, file_path, &mut index) {
            Ok((imported, skipped)) => {
                result.imported += imported;
                result.skipped += skipped;
            }
            Err(e) => {
                let msg = format!("{}: {e}", file_path.display());
                log::warn!("[SESSION-SYNC] 文件解析失败: {msg}");
                result.errors.push(msg);
            }
        }
    }

    if result.imported > 0 {
        log::debug!(
            "[SESSION-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }

    Ok(result)
}

/// Directory levels below `projects/` that are searched: `<p>` (1), `<sid>` (2)
/// and `subagents` (3), so `projects/<p>/<sid>/subagents/*.jsonl` is read.
const PROJECTS_MAX_DEPTH: u32 = 3;

/// 收集 `projects/` 下各级目录（最多 3 层）中的 .jsonl 文件
fn collect_jsonl_files(projects_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_jsonl_under(projects_dir, 0, &mut files);
    files
}

fn collect_jsonl_under(dir: &Path, depth: u32, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < PROJECTS_MAX_DEPTH {
                collect_jsonl_under(&path, depth + 1, files);
            }
        } else if depth >= 1 && path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

/// The session a transcript belongs to by its path, for lines that carry no
/// `sessionId`: `<sid>` of `<sid>/subagents/<agent>.jsonl`, else the file stem.
fn session_of_path(path: &Path) -> Option<String> {
    let dir = path.parent()?;
    if dir.file_name().and_then(|n| n.to_str()) == Some("subagents") {
        return dir
            .parent()?
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string);
    }
    path.file_stem()
        .and_then(|n| n.to_str())
        .map(str::to_string)
}

/// The replies in a transcript after its first `skip_lines` lines, one per
/// message id (the line that best stands for it: one with a stop_reason, else the
/// one with the most output), in the order they first appear; and the file's
/// line count.
fn parse_claude_file(
    file_path: &Path,
    skip_lines: i64,
) -> Result<(Vec<ParsedAssistantUsage>, i64, bool), AppError> {
    let mut messages: Vec<ParsedAssistantUsage> = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();
    let mut current_session_id: Option<String> = None;
    let mut identified = true;

    let line_offset = usage_source::json_lines(
        file_path,
        |_| true,
        |line_offset, value| {
            // 提取 session ID (从 system 或首条消息)
            if current_session_id.is_none() {
                if let Some(sid) = value.get("sessionId").and_then(|v| v.as_str()) {
                    current_session_id = Some(sid.to_string());
                }
            }
            if line_offset <= skip_lines {
                return Ok(());
            }

            // 只处理 assistant 类型的消息
            if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                return Ok(());
            }

            let message = match value.get("message") {
                Some(m) => m,
                None => return Ok(()),
            };

            let msg_id = match message.get("id").and_then(|v| v.as_str()) {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => {
                    if message.get("usage").is_some() {
                        identified = false;
                    }
                    return Ok(());
                }
            };

            let usage = match message.get("usage") {
                Some(u) => u,
                None => return Ok(()),
            };

            let parsed = ParsedAssistantUsage {
                message_id: msg_id.clone(),
                model: message
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                input_tokens: usage_source::checked_token(usage_source::number(
                    usage,
                    "input_tokens",
                )?)?,
                output_tokens: usage_source::checked_token(usage_source::number(
                    usage,
                    "output_tokens",
                )?)?,
                cache_read_tokens: usage_source::checked_token(usage_source::number(
                    usage,
                    "cache_read_input_tokens",
                )?)?,
                cache_creation_tokens: usage_source::checked_token(usage_source::number(
                    usage,
                    "cache_creation_input_tokens",
                )?)?,
                cache_creation_1h_tokens: crate::proxy::usage::parser::claude_hour_cache_writes(
                    usage,
                ),
                stop_reason: message
                    .get("stop_reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                timestamp: value
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                session_id: current_session_id
                    .clone()
                    .or_else(|| session_of_path(file_path)),
            };

            // 按 message.id 去重：优先保留有 stop_reason 的条目，否则保留最新的
            match index_of.get(&msg_id) {
                None => {
                    index_of.insert(msg_id, messages.len());
                    messages.push(parsed);
                }
                Some(&i) => {
                    let existing = &messages[i];
                    // 新条目有 stop_reason 而旧条目没有 → 替换
                    // 两个都有或都没有 stop_reason → 取 output_tokens 更大的
                    let should_replace =
                        if parsed.stop_reason.is_some() && existing.stop_reason.is_none() {
                            true
                        } else if parsed.stop_reason.is_some() == existing.stop_reason.is_some() {
                            parsed.output_tokens >= existing.output_tokens
                        } else {
                            false
                        };
                    if should_replace {
                        messages[i] = parsed;
                    }
                }
            }
            Ok(())
        },
    )?;

    Ok((
        messages,
        line_offset.offset,
        line_offset.complete && identified,
    ))
}

/// A Claude message that kept some of its cache writes for an hour.
pub(crate) struct HourWrite {
    pub request_id: String,
    pub native: String,
    pub session_id: Option<String>,
    pub model: String,
    pub created_at: i64,
    pub hour: u32,
}

/// Every Claude transcript's messages that wrote cache entries for an hour,
/// by file. The ledger keeps no such count, so pricing them again reads it here.
pub(crate) fn hour_cache_writes(projects_dir: &Path) -> Vec<(PathBuf, Vec<HourWrite>)> {
    collect_jsonl_files(projects_dir)
        .into_iter()
        .filter_map(|path| {
            let (messages, _, _) = parse_claude_file(&path, 0).ok()?;
            let writes: Vec<_> = messages
                .into_iter()
                .filter(|m| m.cache_creation_1h_tokens > 0)
                .filter_map(|m| {
                    Some(HourWrite {
                        request_id: format!(
                            "{}{}",
                            crate::proxy::usage::parser::SESSION_REQUEST_ID_PREFIX,
                            m.message_id
                        ),
                        native: usage_source::native_key(
                            "claude",
                            m.session_id.as_deref().unwrap_or(""),
                            &m.message_id,
                        ),
                        created_at: created_at_of(m.timestamp.as_deref())?,
                        hour: m.cache_creation_1h_tokens.min(m.cache_creation_tokens),
                        session_id: m.session_id,
                        model: m.model,
                    })
                })
                .collect();
            (!writes.is_empty()).then_some((path, writes))
        })
        .collect()
}

/// Preserve unknown native time rather than manufacturing a current date.
fn created_at_of(timestamp: Option<&str>) -> Option<i64> {
    usage_source::timestamp_ms(timestamp).map(|ms| ms.div_euclid(1000))
}

pub(crate) fn normalized_facts(
    path: &Path,
) -> Result<(Vec<usage_source::NormalizedUsageFact>, bool), AppError> {
    let (messages, _, complete) = parse_claude_file(path, 0)?;
    let facts = messages
        .into_iter()
        .filter(|m| {
            m.input_tokens != 0
                || m.output_tokens != 0
                || m.cache_read_tokens != 0
                || m.cache_creation_tokens != 0
        })
        .map(|m| usage_source::NormalizedUsageFact {
            request_id: format!("session:{}", m.message_id),
            native_key: (!m.message_id.is_empty()).then(|| {
                usage_source::native_key(
                    "claude",
                    m.session_id.as_deref().unwrap_or(""),
                    &m.message_id,
                )
            }),
            at_ms: usage_source::timestamp_ms(m.timestamp.as_deref()),
            session_id: m.session_id,
            usage: TokenUsage {
                input_tokens: m.input_tokens,
                output_tokens: m.output_tokens,
                cache_read_tokens: m.cache_read_tokens,
                cache_creation_tokens: m.cache_creation_tokens,
                cache_creation_1h_tokens: m.cache_creation_1h_tokens,
                model: Some(m.model.clone()),
                message_id: None,
            },
            model: m.model,
        })
        .collect();
    Ok((facts, complete))
}
#[cfg(test)]
pub(crate) type SessionUsageEntry = usage_source::NormalizedUsageFact;
#[cfg(test)]
pub(crate) fn session_entries() -> Vec<SessionUsageEntry> {
    let mut seen = std::collections::HashSet::new();
    collect_jsonl_files(&get_claude_config_dir().join("projects"))
        .into_iter()
        .flat_map(|p| usage_source::read_normalized("claude", &p).unwrap().facts)
        .filter(|f| seen.insert(f.request_id.clone()))
        .collect()
}

fn sync_single_file_with_index(
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

    // A changed file may be rewritten or replaced, not only appended. Stable
    // native IDs and transactional evidence protect already archived calls.
    let (messages, line_offset, complete) = parse_claude_file(file_path, 0)?;
    if !complete {
        source.evidence.complete_event_identity_set = false;
        source.evidence.partial("incompleteEnumeration");
    }

    let checkpoint = CheckpointChange {
        key: file_path_str,
        expected,
        next: Some(SyncState {
            modified: file_modified,
            offset: line_offset,
        }),
    };
    let next = checkpoint.next.unwrap();
    let result = commit_sync_batch(db, &[checkpoint], |conn| {
        source.check_on(conn)?;
        // 写入数据库
        let mut imported: u32 = 0;
        let mut skipped: u32 = 0;

        for msg in &messages {
            // A reply without a stop_reason is still a call: a subagent's replies
            // mostly never get one, and a reply still streaming gets its final line
            // later, which then updates the row (see insert_session_log_entry).
            let request_id = format!(
                "{}{}",
                crate::proxy::usage::parser::SESSION_REQUEST_ID_PREFIX,
                msg.message_id
            );

            // 跳过 output_tokens 为 0 的无意义条目
            if msg.input_tokens == 0
                && msg.output_tokens == 0
                && msg.cache_read_tokens == 0
                && msg.cache_creation_tokens == 0
            {
                continue;
            }
            let Some(created_at) = created_at_of(msg.timestamp.as_deref()) else {
                source.evidence.complete_event_identity_set = false;
                source.evidence.partial("missingTimestamp");
                skipped += 1;
                continue;
            };
            let native = usage_source::native_key(
                "claude",
                msg.session_id.as_deref().unwrap_or(""),
                &msg.message_id,
            );
            if !source.should_store_on(conn, &request_id, &native, "session_log", created_at)? {
                skipped += 1;
                continue;
            }
            if insert_session_log_entry(conn, &request_id, msg, created_at)? {
                imported += 1;
            } else {
                skipped += 1;
            }
            source.evidence.acknowledge(&native);
        }
        source.save_on(conn, next)?;
        Ok((imported, skipped))
    })?;
    index.advance(source);
    Ok(result)
}

/// 插入单条会话日志到 proxy_request_logs，返回是否写入 (true=新插入或更新, false=未变)
///
/// A row this parser wrote earlier from a reply's unfinished lines is updated
/// when a later line reports more output; a proxy row sharing the message id
/// is never touched.
fn insert_session_log_entry(
    conn: &rusqlite::Connection,
    request_id: &str,
    msg: &ParsedAssistantUsage,
    created_at: i64,
) -> Result<bool, AppError> {
    // 计算费用
    let usage = TokenUsage {
        input_tokens: msg.input_tokens,
        output_tokens: msg.output_tokens,
        cache_read_tokens: msg.cache_read_tokens,
        cache_creation_tokens: msg.cache_creation_tokens,
        cache_creation_1h_tokens: msg.cache_creation_1h_tokens,
        model: Some(msg.model.clone()),
        message_id: None,
    };

    let pricing = find_model_pricing_for_session_checked(conn, &msg.model)?;
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

    let changed = conn.execute(
        "INSERT INTO proxy_request_logs (
            request_id, provider_id, app_type, model, request_model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
            latency_ms, first_token_ms, status_code, error_message, session_id,
            provider_type, is_streaming, cost_multiplier, created_at, data_source
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)
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
            total_cost_usd = excluded.total_cost_usd
        WHERE data_source = 'session_log' AND (model != excluded.model OR request_model IS NOT excluded.request_model OR
            excluded.output_tokens > output_tokens OR (excluded.output_tokens = output_tokens AND (
            excluded.input_tokens != input_tokens OR excluded.cache_read_tokens != cache_read_tokens
            OR excluded.cache_creation_tokens != cache_creation_tokens)))",
        rusqlite::params![
            request_id,
            "_session",         // provider_id: 标记为会话来源
            "claude",           // app_type
            msg.model,
            msg.model,          // request_model = model
            msg.input_tokens,
            msg.output_tokens,
            msg.cache_read_tokens,
            msg.cache_creation_tokens,
            input_cost,
            output_cost,
            cache_read_cost,
            cache_creation_cost,
            total_cost,
            0i64,               // latency_ms: 会话日志无此数据
            Option::<i64>::None, // first_token_ms
            200i64,             // status_code: 会话日志只记录成功的回复
            Option::<String>::None, // error_message
            msg.session_id,
            Some("session_log"), // provider_type
            1i64,               // is_streaming: Claude Code 通常使用流式
            "1.0",              // cost_multiplier
            created_at,
            "session_log",      // data_source
        ],
    )
    .map_err(|e| AppError::Database(format!("插入会话日志失败: {e}")))?;

    Ok(changed > 0)
}

/// 从 model_pricing 表查找模型定价（支持模糊匹配）
fn find_model_pricing_for_session_checked(
    conn: &rusqlite::Connection,
    model_id: &str,
) -> Result<Option<ModelPricing>, AppError> {
    if let Some(price) = try_find_pricing(conn, model_id)? {
        return Ok(Some(price));
    }
    if let Some((prefix, suffix)) = model_id.rsplit_once('-') {
        if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
            if let Some(price) = try_find_pricing(conn, prefix)? {
                return Ok(Some(price));
            }
        }
    }
    crate::services::usage_sync_db::lookup_pricing(conn, &format!("{model_id}%"), true)
}

pub(crate) fn find_model_pricing_for_session(
    conn: &rusqlite::Connection,
    model_id: &str,
) -> Option<ModelPricing> {
    // 精确匹配
    if let Ok(Some(pricing)) = try_find_pricing(conn, model_id) {
        return Some(pricing);
    }

    // 模糊匹配：去掉日期后缀
    // 例如 "claude-opus-4-6-20260206" -> "claude-opus-4-6"
    let parts: Vec<&str> = model_id.rsplitn(2, '-').collect();
    if parts.len() == 2 {
        if let Some(suffix) = parts.first() {
            if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(Some(pricing)) = try_find_pricing(conn, parts[1]) {
                    return Some(pricing);
                }
            }
        }
    }

    // 尝试 LIKE 匹配
    let pattern = format!("{model_id}%");
    let result = conn.query_row(
        "SELECT input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
         FROM model_pricing WHERE model_id LIKE ?1 LIMIT 1",
        rusqlite::params![pattern],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    );

    match result {
        Ok((input, output, cache_read, cache_creation)) => {
            ModelPricing::from_strings(&input, &output, &cache_read, &cache_creation).ok()
        }
        Err(_) => None,
    }
}

fn try_find_pricing(
    conn: &rusqlite::Connection,
    model_id: &str,
) -> Result<Option<ModelPricing>, AppError> {
    let result = conn.query_row(
        "SELECT input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
         FROM model_pricing WHERE model_id = ?1",
        rusqlite::params![model_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    );

    match result {
        Ok((input, output, cache_read, cache_creation)) => {
            ModelPricing::from_strings(&input, &output, &cache_read, &cache_creation)
                .map(Some)
                .map_err(|e| AppError::Database(format!("解析定价失败: {e}")))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(AppError::Database(format!("查询定价失败: {e}"))),
    }
}

/// 查询数据来源分布统计
pub fn get_data_source_breakdown(db: &Database) -> Result<Vec<DataSourceSummary>, AppError> {
    let conn = lock_conn!(db.conn);

    let mut stmt = conn.prepare(
        "SELECT COALESCE(data_source, 'proxy') as ds, COUNT(*) as cnt,
                COALESCE(SUM(CAST(total_cost_usd AS REAL)), 0) as cost
         FROM proxy_request_logs
         GROUP BY ds
         ORDER BY cnt DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(DataSourceSummary {
            data_source: row.get(0)?,
            request_count: row.get::<_, i64>(1)? as u32,
            total_cost_usd: format!("{:.6}", row.get::<_, f64>(2)?),
        })
    })?;

    let mut summaries = Vec::new();
    for row in rows {
        summaries.push(row.map_err(|e| AppError::Database(e.to_string()))?);
    }

    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_usage_from_jsonl_line() {
        let line = r#"{"type":"assistant","message":{"id":"msg_test123","model":"claude-opus-4-6","usage":{"input_tokens":3,"output_tokens":150,"cache_read_input_tokens":5000,"cache_creation_input_tokens":10000},"stop_reason":"end_turn"},"timestamp":"2026-04-05T12:00:00Z","sessionId":"session-abc"}"#;

        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(
            value.get("type").and_then(|t| t.as_str()),
            Some("assistant")
        );

        let message = value.get("message").unwrap();
        let usage = message.get("usage").unwrap();

        assert_eq!(usage.get("input_tokens").unwrap().as_u64().unwrap(), 3);
        assert_eq!(usage.get("output_tokens").unwrap().as_u64().unwrap(), 150);
        assert_eq!(
            usage
                .get("cache_read_input_tokens")
                .unwrap()
                .as_u64()
                .unwrap(),
            5000
        );
        assert_eq!(
            usage
                .get("cache_creation_input_tokens")
                .unwrap()
                .as_u64()
                .unwrap(),
            10000
        );
        assert_eq!(
            message.get("stop_reason").unwrap().as_str().unwrap(),
            "end_turn"
        );
    }

    #[test]
    fn test_dedup_by_message_id() {
        // 同一个 message.id 有多条，应该取 stop_reason 有值的那条
        let mut messages: HashMap<String, ParsedAssistantUsage> = HashMap::new();

        // 中间条目（无 stop_reason）
        let intermediate = ParsedAssistantUsage {
            message_id: "msg_1".to_string(),
            model: "claude-opus-4-6".to_string(),
            input_tokens: 3,
            output_tokens: 26,
            cache_read_tokens: 5000,
            cache_creation_tokens: 10000,
            cache_creation_1h_tokens: 0,
            stop_reason: None,
            timestamp: Some("2026-04-05T12:00:00Z".to_string()),
            session_id: None,
        };
        messages.insert("msg_1".to_string(), intermediate);

        // 最终条目（有 stop_reason）
        let final_entry = ParsedAssistantUsage {
            message_id: "msg_1".to_string(),
            model: "claude-opus-4-6".to_string(),
            input_tokens: 3,
            output_tokens: 1349,
            cache_read_tokens: 5000,
            cache_creation_tokens: 10000,
            cache_creation_1h_tokens: 0,
            stop_reason: Some("end_turn".to_string()),
            timestamp: Some("2026-04-05T12:00:00Z".to_string()),
            session_id: None,
        };

        // 应该替换
        let should_replace = final_entry.stop_reason.is_some()
            && messages.get("msg_1").unwrap().stop_reason.is_none();
        assert!(should_replace);

        messages.insert("msg_1".to_string(), final_entry);
        assert_eq!(messages.get("msg_1").unwrap().output_tokens, 1349);
    }

    use crate::test_sandbox::sandbox;
    use serial_test::serial;
    use std::time::{Duration, SystemTime};

    const PARENT: &str = "0f3a1c2d-1111-4222-8333-444455556666";

    /// An assistant JSONL line as Claude Code writes it.
    fn assistant_line(msg_id: &str, output: u32, stop: Option<&str>, ts: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "sessionId": PARENT,
            "timestamp": ts,
            "message": {
                "id": msg_id,
                "model": "claude-opus-4-6",
                "stop_reason": stop,
                "usage": {
                    "input_tokens": 2,
                    "output_tokens": output,
                    "cache_read_input_tokens": 2894,
                    "cache_creation_input_tokens": 64985
                }
            }
        })
        .to_string()
    }

    fn write_lines(path: &Path, lines: &[String]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    /// Push a file's mtime forward so the next sync sees it as changed even
    /// within the same second.
    fn touch_later(path: &Path, secs: u64) {
        let file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(secs))
            .unwrap();
    }

    fn logged(db: &Database) -> Vec<(String, Option<String>, i64)> {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT request_id, session_id, output_tokens FROM proxy_request_logs
                 WHERE data_source = 'session_log' ORDER BY request_id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    #[test]
    #[serial]
    fn subagent_transcripts_count_toward_the_parent_session() {
        let sandbox = sandbox();
        let project = sandbox.home().join(".claude/projects/-work-app");
        write_lines(
            &project.join(format!("{PARENT}.jsonl")),
            &[assistant_line(
                "msg_main",
                300,
                Some("end_turn"),
                "2026-10-10T18:07:30Z",
            )],
        );
        // a subagent's own transcript: its lines carry the parent's sessionId, and
        // most of its replies never get a stop_reason (only their first usage)
        write_lines(
            &project.join(format!("{PARENT}/subagents/agent-a2c17cdb.jsonl")),
            &[
                assistant_line("msg_sub_1", 7, None, "2026-10-10T18:07:36Z"),
                assistant_line("msg_sub_1", 7, None, "2026-10-10T18:07:37Z"),
                assistant_line("msg_sub_2", 124, Some("tool_use"), "2026-10-10T18:07:39Z"),
            ],
        );
        let db = Database::memory().unwrap();

        let result = sync_claude_session_logs(&db).unwrap();

        assert_eq!(result.files_scanned, 2);
        let parent = Some(PARENT.to_string());
        assert_eq!(
            logged(&db),
            vec![
                ("session:msg_main".to_string(), parent.clone(), 300),
                ("session:msg_sub_1".to_string(), parent.clone(), 7),
                ("session:msg_sub_2".to_string(), parent, 124),
            ]
        );
    }

    #[test]
    #[serial]
    fn a_reply_read_before_it_finished_is_updated_when_its_final_line_arrives() {
        let sandbox = sandbox();
        let file = sandbox
            .home()
            .join(format!(".claude/projects/-work-app/{PARENT}.jsonl"));
        let first = assistant_line("msg_live", 5, None, "2026-10-10T18:07:30Z");
        write_lines(&file, std::slice::from_ref(&first));
        let db = Database::memory().unwrap();
        sync_claude_session_logs(&db).unwrap();

        write_lines(
            &file,
            &[
                first,
                assistant_line("msg_live", 480, Some("end_turn"), "2026-10-10T18:07:42Z"),
            ],
        );
        touch_later(&file, 5);
        sync_claude_session_logs(&db).unwrap();

        assert_eq!(
            logged(&db),
            vec![(
                "session:msg_live".to_string(),
                Some(PARENT.to_string()),
                480
            )]
        );
    }

    #[test]
    #[serial]
    fn a_proxy_row_with_the_same_message_id_is_left_alone() {
        let sandbox = sandbox();
        write_lines(
            &sandbox
                .home()
                .join(format!(".claude/projects/-work-app/{PARENT}.jsonl")),
            &[assistant_line(
                "msg_proxied",
                900,
                Some("end_turn"),
                "2026-10-10T18:07:30Z",
            )],
        );
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                    output_tokens, latency_ms, status_code, created_at, data_source)
                 VALUES ('session:msg_proxied', 'ofox', 'claude', 'claude-opus-4-6', 10, 5, 200,
                    1, 'proxy')",
                [],
            )
            .unwrap();
        }

        sync_claude_session_logs(&db).unwrap();

        let conn = db.conn.lock().unwrap();
        let (source, output): (String, i64) = conn
            .query_row(
                "SELECT data_source, output_tokens FROM proxy_request_logs
                 WHERE request_id = 'session:msg_proxied'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((source.as_str(), output), ("proxy", 10));
    }
}
