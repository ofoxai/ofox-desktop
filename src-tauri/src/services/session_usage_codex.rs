//! Codex 会话日志使用追踪
//!
//! 从 ~/.codex/sessions/ 下的 JSONL 会话文件中提取精确 token 使用数据，
//! 替代原有的 state_5.sqlite 估算方案。
//!
//! ## 数据流
//! ```text
//! ~/.codex/sessions/YYYY/MM/DD/*.jsonl → 增量解析 → delta 计算 → 费用计算 → proxy_request_logs 表
//! ```
//!
//! ## 解析的事件类型
//! - `session_meta` → 提取 session_id
//! - `turn_context` → 提取当前 model
//! - `event_msg` (type=token_count) → 提取累计 token 用量，计算 delta

use crate::codex_config::get_codex_config_dir;
use crate::database::Database;
use crate::error::AppError;
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
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 累计 token 用量（跟踪 total_token_usage 字段）
///
/// Codex's `input_tokens` is the whole prompt: the cached part
/// (`cached_input_tokens`) and, in newer versions, what was written to the cache
/// (`cache_write_input_tokens`) are counted in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CumulativeTokens {
    input: u64,
    cached_input: u64,
    cache_write: u64,
    output: u64,
}

/// 单次 API 调用的 token 增量（input 仍是 Codex 口径：含缓存部分）
#[derive(Debug)]
struct DeltaTokens {
    input: u64,
    cached_input: u32,
    cache_write: u32,
    output: u32,
}

impl DeltaTokens {
    fn is_zero(&self) -> bool {
        self.input == 0 && self.cached_input == 0 && self.cache_write == 0 && self.output == 0
    }

    /// Fresh input as every source stores it: the prompt without what was read
    /// from or written to the cache.
    fn fresh_input(&self) -> u32 {
        self.input
            .saturating_sub(self.cached_input as u64)
            .saturating_sub(self.cache_write as u64) as u32
    }
}

/// 归一化 Codex 模型名
///
/// 处理规则（按顺序）：
/// 1. 转小写：`GLM-4.6` → `glm-4.6`
/// 2. 剥离 provider 前缀：`openai/gpt-5.4` → `gpt-5.4`
/// 3. 剥离 ISO 日期后缀：`gpt-5.4-2026-03-05` → `gpt-5.4`
/// 4. 剥离紧凑日期后缀：`gpt-5.4-20260305` → `gpt-5.4`
fn normalize_codex_model(raw: &str) -> String {
    // Step 1: 小写
    let mut name = raw.to_lowercase();

    // Step 2: 剥离 "provider/" 前缀（如 openai/, azure/）
    if let Some(pos) = name.rfind('/') {
        name = name[pos + 1..].to_string();
    }

    // Step 3: 剥离 ISO 日期后缀 -YYYY-MM-DD（正好 11 字符）
    if name.len() > 11
        && name.get(name.len() - 11..).is_some_and(|suffix| {
            let b = suffix.as_bytes();
            b[0] == b'-'
                && b[1..5].iter().all(u8::is_ascii_digit)
                && b[5] == b'-'
                && b[6..8].iter().all(u8::is_ascii_digit)
                && b[8] == b'-'
                && b[9..11].iter().all(u8::is_ascii_digit)
        })
    {
        name.truncate(name.len() - 11);
    }

    // Step 4: 剥离紧凑日期后缀 -YYYYMMDD（正好 9 字符）
    if name.len() > 9 {
        let parts: Vec<&str> = name.rsplitn(2, '-').collect();
        if parts.len() == 2 {
            if let Some(suffix) = parts.first() {
                if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
                    name = parts[1].to_string();
                }
            }
        }
    }

    name
}

/// 计算两次累计值之间的 delta
fn compute_delta(prev: &Option<CumulativeTokens>, current: &CumulativeTokens) -> DeltaTokens {
    match prev {
        None => DeltaTokens {
            input: (current.input as u32) as u64,
            cached_input: current.cached_input as u32,
            cache_write: current.cache_write as u32,
            output: current.output as u32,
        },
        Some(p) => DeltaTokens {
            input: (current.input.saturating_sub(p.input) as u32) as u64,
            cached_input: current.cached_input.saturating_sub(p.cached_input) as u32,
            cache_write: current.cache_write.saturating_sub(p.cache_write) as u32,
            output: current.output.saturating_sub(p.output) as u32,
        },
    }
}

/// 从 JSON Value 中提取累计 token 用量
fn parse_cumulative_tokens(total_usage: &serde_json::Value) -> Option<CumulativeTokens> {
    if total_usage.is_null() || !total_usage.is_object() {
        return None;
    }
    Some(CumulativeTokens {
        input: total_usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cached_input: total_usage
            .get("cached_input_tokens")
            .or_else(|| total_usage.get("cache_read_input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_write: total_usage
            .get("cache_write_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output: total_usage
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    })
}

/// 同步 Codex 使用数据（从 JSONL 会话日志）
pub fn sync_codex_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    let codex_dir = get_codex_config_dir();

    let files = collect_codex_session_files(&codex_dir);

    let mut result = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: files.len() as u32,
        errors: vec![],
    };

    if files.is_empty() {
        return Ok(result);
    }

    let mut index = ProviderIndex::load(db, "codex", &codex_dir)?;
    for file_path in &files {
        match sync_single_codex_file_with_index(db, file_path, &mut index) {
            Ok((imported, skipped)) => {
                result.imported += imported;
                result.skipped += skipped;
            }
            Err(e) => {
                let msg = format!("Codex 会话文件解析失败 {}: {e}", file_path.display());
                log::warn!("[CODEX-SYNC] {msg}");
                result.errors.push(msg);
            }
        }
    }

    if result.imported > 0 {
        log::debug!(
            "[CODEX-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }

    Ok(result)
}

/// 收集所有 Codex 会话 JSONL 文件
fn collect_codex_session_files(codex_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    // 1. 扫描 sessions/YYYY/MM/DD/*.jsonl（日期分区目录）
    let sessions_dir = codex_dir.join("sessions");
    if sessions_dir.is_dir() {
        collect_jsonl_recursive(&sessions_dir, &mut files, 0, 3);
    }

    // 2. 扫描 archived_sessions/*.jsonl（扁平归档目录）
    let archived_dir = codex_dir.join("archived_sessions");
    if archived_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&archived_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    files.push(path);
                }
            }
        }
    }

    files
}

/// 递归扫描目录下的 .jsonl 文件（限制最大深度）
fn collect_jsonl_recursive(dir: &Path, files: &mut Vec<PathBuf>, depth: u32, max_depth: u32) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && depth < max_depth {
            collect_jsonl_recursive(&path, files, depth + 1, max_depth);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

/// 同步单个 Codex JSONL 文件，返回 (imported, skipped)
#[cfg(test)]
fn sync_single_codex_file(db: &Database, file_path: &Path) -> Result<(u32, u32), AppError> {
    let mut index = ProviderIndex::load(db, "codex", file_path.parent().unwrap())?;
    sync_single_codex_file_with_index(db, file_path, &mut index)
}
fn sync_single_codex_file_with_index(
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
    let previous = expected.unwrap_or_default();

    let mut source = Prepared::with_index(db, Snapshot::file(file_path)?, expected, 2, index)?;
    if source.unchanged {
        return Ok((0, 0));
    }

    // 整个文件都要解析以恢复累计值状态，只写入上次偏移之后的调用
    let (calls, line_offset, complete) = parse_codex_file(file_path)?;
    source.evidence.complete_event_identity_set = false;
    source.evidence.partial("missingNativeEventIdentity");
    if !complete {
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
        // Read the latest alias under the commit lock: a new moved path cannot
        // appear between this dedup decision and publishing the destination state.
        let last_offset = if expected.is_none() {
            offset_at_another_path(conn, file_path)?.unwrap_or(0)
        } else {
            previous.offset
        };
        // A source stamp proves that bytes changed, not that the change was
        // append-only. Compare the consumed normalized prefix before reusing
        // its ordinal boundary; it is never a native event acknowledgement.
        let same_prefix = last_offset == 0
            || source.evidence.codex_prefix()
                == Some(calls_fingerprint(calls.iter().filter(|c| c.line <= last_offset)).as_str());
        let reconcile =
            source.evidence.codex_requires_reconciliation() || (last_offset > 0 && !same_prefix);
        source
            .evidence
            .note_codex_prefix(calls_fingerprint(calls.iter()), reconcile);
        let cutoff = crate::database::dao::usage_rollup::usage_prune_cutoff()?;
        let history = usage_source::has_history_on(conn, "codex")?;
        let mut imported: u32 = 0;
        let mut skipped: u32 = 0;
        for call in calls.iter().filter(|c| !c.delta.is_zero()) {
            if same_prefix && !reconcile && call.line <= last_offset {
                continue;
            }
            let Some(created_at) = created_at_of(call.timestamp.as_deref()) else {
                source.evidence.partial("missingTimestamp");
                skipped += 1;
                continue;
            };
            let old_or_unknown = created_at < cutoff;
            if old_or_unknown && (reconcile || (expected.is_none() && last_offset == 0 && history))
            {
                source.evidence.partial("unverifiedHistoricalBoundary");
                skipped += 1;
                continue;
            }
            if insert_codex_session_entry(
                conn,
                &call.ids,
                &call.delta,
                &call.model,
                call.session_id.as_deref(),
                created_at,
            )? {
                imported += 1;
            } else {
                skipped += 1;
            }
        }

        source.save_on(conn, next)?;
        Ok((imported, skipped))
    })?;
    index.advance(source);
    Ok(result)
}

fn calls_fingerprint<'a>(calls: impl Iterator<Item = &'a CodexCall>) -> String {
    let mut hash = Sha256::new();
    for c in calls {
        hash.update(
            serde_json::json!([
                c.line,
                c.ids.request_id,
                c.delta.input,
                c.delta.cached_input,
                c.delta.cache_write,
                c.delta.output,
                c.model,
                c.timestamp
            ])
            .to_string()
            .as_bytes(),
        );
        hash.update(b"\n");
    }
    format!("{:x}", hash.finalize())
}

/// How far the same rollout file (same file name) was read at another path.
fn offset_at_another_path(
    conn: &rusqlite::Connection,
    file_path: &Path,
) -> Result<Option<i64>, AppError> {
    let Some(name) = file_path.file_name() else {
        return Ok(None);
    };
    let name_str = name.to_string_lossy().to_string();
    let mut stmt = conn
        .prepare(
            "SELECT file_path, last_line_offset FROM session_log_sync
             WHERE file_path != ?1 AND substr(file_path, -length(?2)) = ?2",
        )
        .map_err(|e| AppError::Database(format!("查询同步状态失败: {e}")))?;
    let rows = stmt
        .query_map(
            rusqlite::params![file_path.to_string_lossy(), name_str],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|e| AppError::Database(format!("查询同步状态失败: {e}")))?;
    let mut offset = None;
    for row in rows {
        let (other, other_offset) =
            row.map_err(|e| AppError::Database(format!("读取同步状态失败: {e}")))?;
        if Path::new(&other).file_name() == Some(name) {
            offset = Some(offset.unwrap_or(0).max(other_offset));
        }
    }
    Ok(offset)
}

/// One call in a rollout file: the line it was reported on and its token delta.
struct CodexCall {
    line: i64,
    ids: CodexRequestIds,
    delta: DeltaTokens,
    model: String,
    session_id: Option<String>,
    timestamp: Option<String>,
}

// The old parser is an identity algorithm as well as a usage algorithm. Keep
// its lossy arithmetic isolated here: changing it would renumber existing N
// slots. Corrected accounting below never uses this state as its baseline.
#[derive(Default)]
struct LegacySlots {
    previous: Option<CumulativeTokens>,
    next: u32,
}
impl LegacySlots {
    fn step(&mut self, info: &Value) -> Result<Option<u32>, AppError> {
        let (value, is_total) = if let Some(v) = info.get("total_token_usage") {
            (v, true)
        } else if let Some(v) = info.get("last_token_usage") {
            (v, false)
        } else {
            return Ok(None);
        };
        let Some(current) = parse_cumulative_tokens(value) else {
            return Ok(None);
        };
        let delta = if is_total {
            let first = if self.previous.is_none() {
                info.get("last_token_usage")
                    .and_then(parse_cumulative_tokens)
            } else {
                None
            };
            let delta = compute_delta(&self.previous, &first.unwrap_or_else(|| current.clone()));
            self.previous = Some(current);
            delta
        } else {
            compute_delta(&None, &current)
        };
        let delta = DeltaTokens {
            cached_input: delta.cached_input.min(delta.input as u32),
            ..delta
        };
        if delta.is_zero() {
            return Ok(None);
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| codex_error("event identity limit"))?;
        Ok(Some(self.next))
    }
}
fn codex_error(reason: &str) -> AppError {
    AppError::Config(format!("Codex usage: {reason}"))
}
fn checked_cumulative(value: Option<&Value>) -> Result<Option<CumulativeTokens>, AppError> {
    let Some(value) = value.filter(|v| v.is_object()) else {
        return Ok(None);
    };
    let keys = [
        "input_tokens",
        "output_tokens",
        "cached_input_tokens",
        "cache_read_input_tokens",
        "cache_write_input_tokens",
    ];
    if !keys
        .iter()
        .any(|k| value.get(k).is_some_and(|v| !v.is_null()))
    {
        return Ok(None);
    }
    Ok(Some(CumulativeTokens {
        input: usage_source::number(value, "input_tokens")?,
        output: usage_source::number(value, "output_tokens")?,
        cached_input: usage_source::number(
            value,
            if value.get("cached_input_tokens").is_some() {
                "cached_input_tokens"
            } else {
                "cache_read_input_tokens"
            },
        )?,
        cache_write: usage_source::number(value, "cache_write_input_tokens")?,
    }))
}
impl CumulativeTokens {
    fn add(&self, rhs: &Self) -> Result<Self, AppError> {
        let add = |a: u64, b: u64| {
            a.checked_add(b)
                .ok_or_else(|| codex_error("cumulative token overflow"))
        };
        Ok(Self {
            input: add(self.input, rhs.input)?,
            cached_input: add(self.cached_input, rhs.cached_input)?,
            cache_write: add(self.cache_write, rhs.cache_write)?,
            output: add(self.output, rhs.output)?,
        })
    }
    fn subtract(&self, rhs: &Self) -> Result<Self, AppError> {
        let sub = |a: u64, b: u64| {
            a.checked_sub(b)
                .ok_or_else(|| codex_error("unanchored cumulative reset"))
        };
        Ok(Self {
            input: sub(self.input, rhs.input)?,
            cached_input: sub(self.cached_input, rhs.cached_input)?,
            cache_write: sub(self.cache_write, rhs.cache_write)?,
            output: sub(self.output, rhs.output)?,
        })
    }
    fn checked_delta(self) -> Result<DeltaTokens, AppError> {
        let cached_input = self.cached_input.min(self.input);
        usage_source::checked_token(
            self.input
                .saturating_sub(cached_input)
                .saturating_sub(self.cache_write),
        )?;
        Ok(DeltaTokens {
            input: self.input,
            cached_input: usage_source::checked_token(cached_input)?,
            cache_write: usage_source::checked_token(self.cache_write)?,
            output: usage_source::checked_token(self.output)?,
        })
    }
}
#[derive(Default)]
struct Accounting {
    effective: Option<CumulativeTokens>,
    raw_total: Option<CumulativeTokens>,
    unanchored_last: bool,
}
impl Accounting {
    fn step(&mut self, info: &Value) -> Result<Option<DeltaTokens>, AppError> {
        let total = checked_cumulative(info.get("total_token_usage"))?;
        let last = checked_cumulative(info.get("last_token_usage"))?;
        let delta = if let Some(total) = total {
            let delta = match &self.effective {
                None => {
                    if let Some(last) = last {
                        last
                    } else if self.unanchored_last {
                        return Err(codex_error(
                            "first total has no anchor after last-only events",
                        ));
                    } else {
                        total.clone()
                    }
                }
                Some(previous) => {
                    // A repeated older total must not rewind the last-only
                    // contributions already counted after that notification.
                    if self.raw_total.as_ref() == Some(&total) && &total != previous {
                        return Ok(Some(CumulativeTokens::default().checked_delta()?));
                    }
                    total.subtract(previous)?
                }
            };
            self.effective = Some(total.clone());
            self.raw_total = Some(total);
            delta
        } else if let Some(last) = last {
            if let Some(previous) = &self.effective {
                self.effective = Some(previous.add(&last)?);
            } else {
                self.unanchored_last = true;
            }
            last
        } else {
            return Ok(None);
        };
        Ok(Some(delta.checked_delta()?))
    }
}
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        other => other.clone(),
    }
}
/// Parse accounting and legacy identity in independent state machines.
fn parse_codex_file(file_path: &Path) -> Result<(Vec<CodexCall>, i64, bool), AppError> {
    let stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("rollout");
    let mut session: Option<String> = None;
    let mut model = "unknown".to_owned();
    let mut legacy = LegacySlots::default();
    let mut accounting = Accounting::default();
    let mut recovered: BTreeMap<String, u32> = BTreeMap::new();
    let mut calls = Vec::new();
    let progress = usage_source::json_lines(
        file_path,
        |line| {
            line.contains("\"session_meta\"")
                || line.contains("\"turn_context\"")
                || (line.contains("\"event_msg\"") && line.contains("\"token_count\""))
        },
        |line, value| {
            let payload = &value["payload"];
            match value["type"].as_str() {
                Some("session_meta") if session.is_none() => {
                    session = payload
                        .get("session_id")
                        .or_else(|| payload.get("sessionId"))
                        .or_else(|| payload.get("id"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                Some("turn_context") => {
                    if let Some(raw) = payload
                        .get("model")
                        .or_else(|| payload.get("info").and_then(|i| i.get("model")))
                        .and_then(Value::as_str)
                    {
                        model = normalize_codex_model(raw);
                    }
                }
                Some("event_msg") if payload["type"] == "token_count" => {
                    let Some(info) = payload.get("info").filter(|v| !v.is_null()) else {
                        return Ok(());
                    };
                    if let Some(raw) = info
                        .get("model")
                        .or_else(|| info.get("model_name"))
                        .or_else(|| payload.get("model"))
                        .and_then(Value::as_str)
                    {
                        model = normalize_codex_model(raw);
                    }
                    let old_slot = legacy.step(info)?;
                    let Some(delta) = accounting.step(info)? else {
                        return Ok(());
                    };
                    if old_slot.is_none() && delta.is_zero() {
                        return Ok(());
                    }
                    let sid = session.as_deref().unwrap_or("unknown");
                    let ids = if let Some(n) = old_slot {
                        CodexRequestIds::new(sid, stem, n)
                    } else {
                        let digest = format!(
                            "{:x}",
                            Sha256::digest(canonical(&value).to_string().as_bytes())
                        );
                        let occurrence = recovered.entry(digest.clone()).or_default();
                        *occurrence = occurrence
                            .checked_add(1)
                            .ok_or_else(|| codex_error("recovered identity limit"))?;
                        CodexRequestIds {
                            request_id: format!(
                                "codex_session:{sid}:{stem}:recovered-v2:{digest}:{occurrence}"
                            ),
                            legacy_id: None,
                        }
                    };
                    calls.push(CodexCall {
                        line,
                        ids,
                        delta,
                        model: model.clone(),
                        session_id: session.clone(),
                        timestamp: value["timestamp"].as_str().map(str::to_owned),
                    });
                }
                _ => {}
            }
            Ok(())
        },
    )?;
    Ok((calls, progress.offset, progress.complete))
}

/// Preserve unknown native time rather than manufacturing a current date.
fn created_at_of(timestamp: Option<&str>) -> Option<i64> {
    usage_source::timestamp_ms(timestamp).map(|ms| ms.div_euclid(1000))
}

pub(crate) fn normalized_facts(
    path: &Path,
) -> Result<(Vec<usage_source::NormalizedUsageFact>, bool), AppError> {
    let (calls, _, complete) = parse_codex_file(path)?;
    Ok((
        calls
            .into_iter()
            .filter(|c| !c.delta.is_zero())
            .map(|c| usage_source::NormalizedUsageFact {
                request_id: c.ids.request_id,
                native_key: None,
                session_id: c.session_id,
                at_ms: usage_source::timestamp_ms(c.timestamp.as_deref()),
                usage: TokenUsage {
                    input_tokens: c.delta.fresh_input(),
                    output_tokens: c.delta.output,
                    cache_read_tokens: c.delta.cached_input,
                    cache_creation_tokens: c.delta.cache_write,
                    model: Some(c.model.clone()),
                    message_id: None,
                },
                model: c.model,
            })
            .collect(),
        complete,
    ))
}
#[cfg(test)]
pub(crate) fn session_entries() -> Vec<SessionUsageEntry> {
    let mut seen = std::collections::HashSet::new();
    collect_codex_session_files(&get_codex_config_dir())
        .into_iter()
        .flat_map(|p| usage_source::read_normalized("codex", &p).unwrap().facts)
        .filter(|f| seen.insert(f.request_id.clone()))
        .collect()
}

/// The request id of a Codex call, and the one versions before v11 gave it.
///
/// A thread picked up again later may go on in another rollout file under the
/// same thread id, which numbers its calls from 1 again, so the file's stem is
/// part of the id. The old id (`codex_session:{thread}:{n}`) did not have it.
struct CodexRequestIds {
    request_id: String,
    legacy_id: Option<String>,
}

impl CodexRequestIds {
    fn new(session_id: &str, file_stem: &str, event_index: u32) -> Self {
        Self {
            request_id: format!("codex_session:{session_id}:{file_stem}:{event_index}"),
            legacy_id: Some(format!("codex_session:{session_id}:{event_index}")),
        }
    }
}

/// 插入单条 Codex 会话记录到 proxy_request_logs
fn insert_codex_session_entry(
    conn: &rusqlite::Connection,
    ids: &CodexRequestIds,
    delta: &DeltaTokens,
    model: &str,
    session_id: Option<&str>,
    created_at: i64,
) -> Result<bool, AppError> {
    let request_id = ids.request_id.as_str();

    // 检查是否已存在
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM proxy_request_logs WHERE request_id = ?1",
            rusqlite::params![request_id],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )
        .map_err(|e| AppError::Database(format!("查询 Codex 调用失败: {e}")))?;

    if exists {
        return Ok(false);
    }

    // The same call stored by an earlier version under its old id (a file read
    // again from its new path in archived_sessions/): same number, same time.
    let logged_under_legacy_id: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM proxy_request_logs WHERE request_id = ?1 AND created_at = ?2",
            rusqlite::params![ids.legacy_id, created_at],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )
        .map_err(|e| AppError::Database(format!("查询旧 Codex 调用失败: {e}")))?;
    if logged_under_legacy_id {
        return Ok(false);
    }

    // 计算费用
    let usage = TokenUsage {
        input_tokens: delta.fresh_input(),
        output_tokens: delta.output,
        cache_read_tokens: delta.cached_input,
        cache_creation_tokens: delta.cache_write,
        model: Some(model.to_string()),
        message_id: None,
    };

    let pricing = find_codex_pricing_checked(conn, model)?;
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
        ON CONFLICT(request_id) DO NOTHING",
        rusqlite::params![
            request_id,
            "_codex_session",    // provider_id
            "codex",             // app_type
            model,
            model,               // request_model = model
            usage.input_tokens,  // fresh input
            delta.output,
            delta.cached_input,
            delta.cache_write,
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
            Some("codex_session"), // provider_type
            1i64,                // is_streaming
            "1.0",               // cost_multiplier
            created_at,
            "codex_session",     // data_source
        ],
    )
    .map_err(|e| AppError::Database(format!("插入 Codex 会话日志失败: {e}")))?;

    Ok(changed > 0)
}

/// Fallible lookup for a sync transaction; the legacy Option wrapper below
/// stays available to maintenance callers.
pub(crate) fn find_codex_pricing_checked(
    conn: &rusqlite::Connection,
    model_id: &str,
) -> Result<Option<ModelPricing>, AppError> {
    let normalized = normalize_codex_model(model_id);
    if let Some(price) = crate::services::usage_sync_db::lookup_pricing(conn, &normalized, false)? {
        return Ok(Some(price));
    }
    crate::services::usage_sync_db::lookup_pricing(conn, &format!("{normalized}%"), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_first_event() {
        let prev = None;
        let current = CumulativeTokens {
            input: 17934,
            cached_input: 9600,
            cache_write: 0,
            output: 454,
        };
        let delta = compute_delta(&prev, &current);
        assert_eq!(delta.input, 17934);
        assert_eq!(delta.cached_input, 9600);
        assert_eq!(delta.output, 454);
        assert!(!delta.is_zero());
    }

    #[test]
    fn test_delta_subsequent_event() {
        let prev = Some(CumulativeTokens {
            input: 17934,
            cached_input: 9600,
            cache_write: 0,
            output: 454,
        });
        let current = CumulativeTokens {
            input: 36722,
            cached_input: 27904,
            cache_write: 0,
            output: 804,
        };
        let delta = compute_delta(&prev, &current);
        assert_eq!(delta.input, 36722 - 17934);
        assert_eq!(delta.cached_input, 27904 - 9600);
        assert_eq!(delta.output, 804 - 454);
    }

    #[test]
    fn test_delta_zero_at_task_boundary() {
        let prev = Some(CumulativeTokens {
            input: 58346,
            cached_input: 46976,
            cache_write: 0,
            output: 1045,
        });
        // task 边界：相同的累计值
        let current = CumulativeTokens {
            input: 58346,
            cached_input: 46976,
            cache_write: 0,
            output: 1045,
        };
        let delta = compute_delta(&prev, &current);
        assert!(delta.is_zero());
    }

    #[test]
    fn test_delta_saturating_sub() {
        // 异常情况：当前值小于前值（不应发生，但需防护）
        let prev = Some(CumulativeTokens {
            input: 100,
            cached_input: 50,
            cache_write: 0,
            output: 30,
        });
        let current = CumulativeTokens {
            input: 80,
            cached_input: 40,
            cache_write: 0,
            output: 20,
        };
        let delta = compute_delta(&prev, &current);
        assert_eq!(delta.input, 0);
        assert_eq!(delta.cached_input, 0);
        assert_eq!(delta.output, 0);
        assert!(delta.is_zero());
    }

    #[test]
    fn test_parse_cumulative_tokens_valid() {
        let json: serde_json::Value = serde_json::json!({
            "input_tokens": 17934,
            "cached_input_tokens": 9600,
            "output_tokens": 454,
            "reasoning_output_tokens": 233,
            "total_tokens": 18388
        });
        let tokens = parse_cumulative_tokens(&json).unwrap();
        assert_eq!(tokens.input, 17934);
        assert_eq!(tokens.cached_input, 9600);
        assert_eq!(tokens.output, 454);
    }

    #[test]
    fn test_parse_cumulative_tokens_null() {
        let json = serde_json::Value::Null;
        assert!(parse_cumulative_tokens(&json).is_none());
    }

    #[test]
    fn test_parse_cumulative_tokens_alt_field_names() {
        // 某些版本可能使用 cache_read_input_tokens 而非 cached_input_tokens
        let json: serde_json::Value = serde_json::json!({
            "input_tokens": 1000,
            "cache_read_input_tokens": 500,
            "output_tokens": 200
        });
        let tokens = parse_cumulative_tokens(&json).unwrap();
        assert_eq!(tokens.cached_input, 500);
    }

    #[test]
    fn test_collect_codex_session_files_nonexistent() {
        let files = collect_codex_session_files(Path::new("/nonexistent/path"));
        assert!(files.is_empty());
    }

    // ── 模型名归一化测试 ──

    #[test]
    fn test_normalize_codex_model_lowercase() {
        assert_eq!(normalize_codex_model("GLM-4.6"), "glm-4.6");
        assert_eq!(normalize_codex_model("DeepSeek-Chat"), "deepseek-chat");
        assert_eq!(normalize_codex_model("GPT-5.4"), "gpt-5.4");
    }

    #[test]
    fn test_normalize_codex_model_strip_prefix() {
        assert_eq!(normalize_codex_model("openai/gpt-5.4"), "gpt-5.4");
        assert_eq!(
            normalize_codex_model("azure/gpt-5.2-codex"),
            "gpt-5.2-codex"
        );
        assert_eq!(normalize_codex_model("OPENAI/GPT-5.4"), "gpt-5.4");
    }

    #[test]
    fn test_normalize_codex_model_strip_iso_date() {
        assert_eq!(normalize_codex_model("gpt-5.4-2026-03-05"), "gpt-5.4");
        assert_eq!(
            normalize_codex_model("gpt-5.4-pro-2026-03-05"),
            "gpt-5.4-pro"
        );
    }

    #[test]
    fn test_normalize_codex_model_strip_compact_date() {
        assert_eq!(normalize_codex_model("gpt-5.4-20260305"), "gpt-5.4");
        assert_eq!(
            normalize_codex_model("claude-opus-4-6-20260206"),
            "claude-opus-4-6"
        );
    }

    #[test]
    fn test_normalize_codex_model_no_change() {
        assert_eq!(normalize_codex_model("gpt-5.4"), "gpt-5.4");
        assert_eq!(normalize_codex_model("gpt-5.2-codex"), "gpt-5.2-codex");
        assert_eq!(normalize_codex_model("o3"), "o3");
        assert_eq!(normalize_codex_model("deepseek-chat"), "deepseek-chat");
    }

    #[test]
    fn test_normalize_codex_model_combined() {
        // prefix + uppercase + ISO date
        assert_eq!(
            normalize_codex_model("openai/GPT-5.4-2026-03-05"),
            "gpt-5.4"
        );
        // prefix + compact date
        assert_eq!(normalize_codex_model("openai/gpt-5.4-20260305"), "gpt-5.4");
    }

    #[test]
    fn test_cached_clamped_to_input() {
        // cached > input 的异常场景应被 min() 钳制
        let prev = Some(CumulativeTokens {
            input: 100,
            cached_input: 0,
            cache_write: 0,
            output: 50,
        });
        let current = CumulativeTokens {
            input: 110,       // delta = 10
            cached_input: 80, // delta = 80（异常：大于 input delta）
            cache_write: 0,
            output: 60,
        };
        let delta = compute_delta(&prev, &current);
        // 钳制前：cached_input = 80, input = 10
        assert_eq!(delta.cached_input, 80);
        assert_eq!(delta.input, 10);
        // 实际钳制在调用侧：delta.cached_input.min(delta.input)
        let clamped = delta.cached_input.min(delta.input as u32);
        assert_eq!(clamped, 10);
    }

    fn stored_tokens(db: &Database, request_id_like: &str) -> Vec<(i64, i64, i64, i64)> {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens
                 FROM proxy_request_logs WHERE request_id LIKE ?1 ORDER BY created_at",
            )
            .unwrap();
        let rows = stmt
            .query_map([request_id_like], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    #[test]
    fn codex_rows_store_fresh_input_without_cache_reads_or_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("rollout-2026-05-22T09-57-31-sess-fresh.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"timestamp":"2026-05-22T02:00:30Z","type":"session_meta","payload":{"id":"sess-fresh"}}"#,
                "\n",
                r#"{"timestamp":"2026-05-22T02:00:31Z","type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                "\n",
                r#"{"timestamp":"2026-05-22T02:00:34Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"cache_write_input_tokens":50,"output_tokens":120}}}}"#,
                "\n",
            ),
        )
        .unwrap();
        let db = Database::memory().unwrap();

        sync_single_codex_file(&db, &path).unwrap();

        assert_eq!(
            stored_tokens(&db, "codex_session:sess-fresh:%"),
            vec![(150, 120, 800, 50)]
        );
    }

    /// A rollout file of thread `sid` with one call at `ts`.
    fn write_rollout(dir: &Path, name: &str, sid: &str, ts: &str, input: u64) -> PathBuf {
        let path = dir.join(name);
        let lines = [
            serde_json::json!({"timestamp": ts, "type": "session_meta", "payload": {"id": sid}}),
            serde_json::json!({"timestamp": ts, "type": "turn_context", "payload": {"model": "gpt-5.4"}}),
            serde_json::json!({"timestamp": ts, "type": "event_msg", "payload": {"type": "token_count",
                "info": {"total_token_usage": {"input_tokens": input, "cached_input_tokens": 0,
                    "output_tokens": 10}}}}),
        ];
        let body: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        std::fs::write(&path, body.join("\n") + "\n").unwrap();
        path
    }

    #[test]
    fn a_thread_continued_in_a_second_rollout_file_keeps_both_files_calls() {
        let dir = tempfile::tempdir().unwrap();
        let first = write_rollout(
            dir.path(),
            "rollout-2026-05-22T09-57-31-thread-1.jsonl",
            "thread-1",
            "2026-05-22T02:00:34Z",
            100,
        );
        let resumed = write_rollout(
            dir.path(),
            "rollout-2026-05-23T08-00-00-thread-1.jsonl",
            "thread-1",
            "2026-05-23T01:00:00Z",
            300,
        );
        let db = Database::memory().unwrap();

        sync_single_codex_file(&db, &first).unwrap();
        sync_single_codex_file(&db, &resumed).unwrap();

        let inputs: Vec<i64> = stored_tokens(&db, "codex_session:thread-1:%")
            .into_iter()
            .map(|t| t.0)
            .collect();
        assert_eq!(inputs, vec![100, 300]);
    }

    #[test]
    fn a_call_logged_under_the_old_request_id_is_not_logged_again() {
        let dir = tempfile::tempdir().unwrap();
        let file = write_rollout(
            dir.path(),
            "rollout-2026-05-22T09-57-31-thread-2.jsonl",
            "thread-2",
            "2026-05-22T02:00:34Z",
            100,
        );
        let db = Database::memory().unwrap();
        {
            // what an earlier version stored for this call, before the file was
            // moved to archived_sessions/ (a new path, so it is read again)
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, latency_ms, status_code, session_id,
                    created_at, data_source)
                 VALUES ('codex_session:thread-2:1', '_codex_session', 'codex', 'gpt-5.4',
                    100, 10, 0, 200, 'thread-2', ?1, 'codex_session')",
                [chrono::DateTime::parse_from_rfc3339("2026-05-22T02:00:34Z")
                    .unwrap()
                    .timestamp()],
            )
            .unwrap();
        }

        sync_single_codex_file(&db, &file).unwrap();

        assert_eq!(stored_tokens(&db, "codex_session:thread-2:%").len(), 1);
    }

    #[test]
    fn a_rollout_moved_to_archived_sessions_is_not_counted_again() {
        let home = tempfile::tempdir().unwrap();
        let ago = |days: i64| (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        let count = |days: i64, input: u64| {
            serde_json::json!({"timestamp": ago(days), "type": "event_msg", "payload": {
                "type": "token_count", "info": {"total_token_usage": {"input_tokens": input,
                    "cached_input_tokens": 0, "output_tokens": 10}}}})
            .to_string()
        };
        let name = "rollout-2026-08-01T10-00-00-thread-3.jsonl";
        let live = home.path().join("sessions/2026/08/01").join(name);
        std::fs::create_dir_all(live.parent().unwrap()).unwrap();
        std::fs::write(
            &live,
            [
                serde_json::json!({"timestamp": ago(40), "type": "session_meta",
                    "payload": {"id": "thread-3"}})
                .to_string(),
                count(40, 100),
                count(1, 300),
            ]
            .join("\n"),
        )
        .unwrap();
        let db = Database::memory().unwrap();
        sync_single_codex_file(&db, &live).unwrap();
        db.rollup_and_prune(30).unwrap();

        // Codex archives a thread by moving its file
        let archived = home.path().join("archived_sessions").join(name);
        std::fs::create_dir_all(archived.parent().unwrap()).unwrap();
        std::fs::rename(&live, &archived).unwrap();
        sync_single_codex_file(&db, &archived).unwrap();
        db.rollup_and_prune(30).unwrap();

        let conn = db.conn.lock().unwrap();
        let calls: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM proxy_request_logs WHERE session_id = 'thread-3')
                      + (SELECT COALESCE(SUM(request_count), 0) FROM session_usage_archive
                         WHERE session_id = 'thread-3')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(calls, 2);
    }

    #[test]
    fn a_continued_rollout_counts_its_first_call_not_the_threads_running_total() {
        let dir = tempfile::tempdir().unwrap();
        let first = write_rollout(
            dir.path(),
            "rollout-2026-05-22T09-57-31-thread-4.jsonl",
            "thread-4",
            "2026-05-22T02:00:34Z",
            100,
        );
        // the thread goes on in a second file whose running total carries the
        // first file's 100 input tokens; its own call is the last_token_usage
        let resumed = dir
            .path()
            .join("rollout-2026-05-23T08-00-00-thread-4.jsonl");
        std::fs::write(
            &resumed,
            [
                serde_json::json!({"timestamp": "2026-05-23T01:00:00Z", "type": "session_meta",
                    "payload": {"id": "thread-4"}}),
                serde_json::json!({"timestamp": "2026-05-23T01:00:01Z", "type": "event_msg",
                    "payload": {"type": "token_count", "info": {
                        "total_token_usage": {"input_tokens": 400, "cached_input_tokens": 0,
                            "output_tokens": 30},
                        "last_token_usage": {"input_tokens": 300, "cached_input_tokens": 0,
                            "output_tokens": 20}}}}),
                serde_json::json!({"timestamp": "2026-05-23T01:00:09Z", "type": "event_msg",
                    "payload": {"type": "token_count", "info": {
                        "total_token_usage": {"input_tokens": 900, "cached_input_tokens": 0,
                            "output_tokens": 70},
                        "last_token_usage": {"input_tokens": 500, "cached_input_tokens": 0,
                            "output_tokens": 40}}}}),
            ]
            .map(|l| l.to_string())
            .join("\n"),
        )
        .unwrap();
        let db = Database::memory().unwrap();

        sync_single_codex_file(&db, &first).unwrap();
        sync_single_codex_file(&db, &resumed).unwrap();

        let calls: Vec<(i64, i64)> = stored_tokens(&db, "codex_session:thread-4:%")
            .into_iter()
            .map(|t| (t.0, t.1))
            .collect();
        assert_eq!(calls, vec![(100, 10), (300, 20), (500, 40)]);
    }
}
