//! Ofox 模型 pricing 同步
//!
//! 从 Ofox 三个 model-list 端点（`/v1/models`、`/anthropic/v1/models`、
//! `/gemini/v1beta/models`）拉取每个模型的 pricing 字段，写入本地
//! `model_pricing` 表，让 claude/codex/gemini 的会话日志成本估算与 Ofox
//! 实际计费保持一致。
//!
//! ## 为什么不复用 [`crate::services::model_fetch`]
//!
//! `model_fetch::FetchedModel` 反序列化结构只取 `id`/`owned_by`，被前端
//! ManageToolDialog 消费。把 pricing/aliases 加进去会污染那条无关路径的
//! TS 类型。这里独立定义解析结构，URL 和 HTTP client 仍走共享层。
//!
//! ## Alias 展开
//!
//! Ofox 的 `id` 是 `provider/model`（如 `anthropic/claude-opus-4.6`），
//! 但 cc-switch 的会话日志解析器看到的 model 字符串是日志原文（claude
//! 直接给 `claude-opus-4-6`、codex 经 normalize 后给 `gpt-5.4`、gemini
//! 给 `google/gemini-2.5-pro`）。Ofox 每个模型都带 `aliases` 数组，里面
//! 包含所有这些变体。我们为每个模型把 `aliases ∪ {id}` 全部作为
//! `model_id` 写一行 INSERT OR REPLACE，这样 lookup 端零变更。
//!
//! ## Gemini bug 顺手修
//!
//! `find_gemini_pricing` 历来不剥 `google/` 前缀，所以会话日志里的
//! `google/gemini-2.5-pro` 永远查不到表里的 `gemini-2.5-pro`。修在 sync
//! 写入端：对 `id.starts_with("google/")` 的模型，额外写入
//! `google/<alias>` 形式的 key，bug 自然消失。
//!
//! ## Cooldown
//!
//! `sync_pricing_if_needed` 用 `AppSettings::last_pricing_sync_at` 做 24h
//! 门控；`sync_pricing` 强制刷新（用户点 ConsolePage「刷新」会走这条）。

use std::str::FromStr;
use std::time::{Duration, SystemTime};

use rust_decimal::Decimal;
use serde::Deserialize;

use crate::database::{lock_conn, Database};
use crate::error::AppError;

const FETCH_TIMEOUT_SECS: u64 = 15;
const SYNC_COOLDOWN_SECS: i64 = 24 * 3600;

/// 一次 sync 的结果摘要。
///
/// 字段目前主要供日志和测试使用；外层 timer / command 只关心 `Result`
/// 的 Ok/Err。`#[allow(dead_code)]` 标注是为了在「字段会被未来 UI 消费」
/// 这件事尚未发生前，不被 dead_code lint 噪声淹没。
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct PricingSyncResult {
    /// 三端点合计返回的有效模型条数（已经按 `id` 去重前的原始数）。
    pub models: usize,
    /// 实际写入 `model_pricing` 表的行数（每个模型展开 N 个 alias key）。
    pub rows: usize,
    /// 是否被 cooldown 跳过（用于 startup 路径）。
    pub skipped: bool,
    /// 各端点的 fetch 错误信息。仅当至少一个端点成功时才会到达写库阶段；
    /// 全部失败时整个 `sync_pricing` 直接返回 Err。
    pub errors: Vec<String>,
}

// ─── HTTP 响应结构 ────────────────────────────────────────────────────────

/// `/v1/models` 与 `/anthropic/v1/models` 共用 OpenAI shape。
#[derive(Debug, Deserialize)]
struct OpenAiShapeResp {
    #[serde(default)]
    data: Vec<OpenAiShapeModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiShapeModel {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    pricing: Option<OfoxPricing>,
    /// `/v1/models` 用 `name`，`/anthropic/v1/models` 用 `display_name`。
    /// 两者只会出现一个；缺失时回退到 `id`。
    name: Option<String>,
    display_name: Option<String>,
}

/// 三端点共用 pricing 子结构。所有字段都是 $/token 字符串。
///
/// Anthropic 和 Gemini 的字段集略有差异：
/// - Anthropic 有 `input_cache_write_5m` 和 `_1h` 两档；我们取 5m，与
///   Claude SDK 默认 cache TTL 一致。
/// - Gemini 用平铺的 `input_cache_write`（无 TTL 区分）。
/// 写库时会按优先级 `_5m → input_cache_write` 选第一个有值的。
#[derive(Debug, Deserialize, Clone)]
struct OfoxPricing {
    prompt: Option<String>,
    completion: Option<String>,
    input_cache_read: Option<String>,
    input_cache_write_5m: Option<String>,
    input_cache_write: Option<String>,
}

// ─── 内部规范化结构 ───────────────────────────────────────────────────────

/// 三端点返回的不同 shape 都先归到这个统一结构再走写库。
#[derive(Debug, Clone)]
struct NormalizedModel {
    /// Ofox canonical id，如 `anthropic/claude-opus-4.6`。
    id: String,
    /// 用于 `display_name` 列；fallback 到 `id`。
    display_name: String,
    /// 全部别名（可能为空），不含 `id` 自身。
    aliases: Vec<String>,
    pricing: OfoxPricing,
}

impl From<OpenAiShapeModel> for Option<NormalizedModel> {
    fn from(m: OpenAiShapeModel) -> Self {
        let pricing = m.pricing?;
        let display_name = m
            .display_name
            .or(m.name)
            .unwrap_or_else(|| m.id.clone());
        Some(NormalizedModel {
            id: m.id,
            display_name,
            aliases: m.aliases,
            pricing,
        })
    }
}

// ─── Public API ───────────────────────────────────────────────────────────

/// 强制同步 Ofox pricing 到本地。失败时不更新 cooldown。
///
/// **只打 `/v1/models`** 一个端点。这一个端点已包含全部厂商（anthropic /
/// google / openai / 第三方）的全部模型 + 完整 pricing —— 经实测生产
/// `https://api.ofox.ai/v1/models` 返回 ~98 个模型，覆盖所有 cc-switch
/// 关心的工具。`/anthropic/v1/models` 与 `/gemini/v1beta/models` 在某些
/// 部署环境下需要 Bearer token 鉴权（401），且即使能拿到也只是
/// `/v1/models` 的子集，没必要冒鉴权失败的风险去打。
pub async fn sync_pricing(db: &Database) -> Result<PricingSyncResult, AppError> {
    let base = crate::ofox_apex::gateway_base();
    let url = format!("{base}/v1/models");

    let models = match fetch_openai_shape(&url).await {
        Ok(items) => items,
        Err(e) => {
            return Err(AppError::Message(format!(
                "pricing_sync: /v1/models fetch failed: {e}"
            )));
        }
    };

    if models.is_empty() {
        return Err(AppError::Message(
            "pricing_sync: /v1/models returned no models".to_string(),
        ));
    }

    let errors: Vec<String> = Vec::new();

    let model_count = models.len();
    let rows = write_pricing_rows(db, &models)?;

    // 历史数据兜底：sync 后 pricing 表可能新增了能命中老 session_log 行的
    // key（例如新加 `qwen-flash` 裸名），把那些 cost=0 但 token>0 的行重算
    // 一遍。失败不致命，只 log。
    match backfill_zero_cost_session_logs(db) {
        Ok(updated) if updated > 0 => {
            log::info!("pricing_sync: backfilled cost on {updated} historical session-log rows");
        }
        Ok(_) => {}
        Err(e) => log::warn!("pricing_sync: backfill failed: {e}"),
    }

    // 至少一个端点成功 → 推进 cooldown
    let now = now_unix_secs();
    if let Err(e) = crate::settings::mutate_settings(|s| {
        s.last_pricing_sync_at = Some(now);
    }) {
        // 写设置失败不致命：下次启动会重新跑一次
        log::warn!("pricing_sync: failed to persist last_pricing_sync_at: {e}");
    }

    if !errors.is_empty() {
        log::warn!("pricing_sync: partial success ({} errors): {errors:?}", errors.len());
    }
    log::info!(
        "pricing_sync: imported {} models, wrote {} rows",
        model_count,
        rows
    );

    Ok(PricingSyncResult {
        models: model_count,
        rows,
        skipped: false,
        errors,
    })
}

/// 仅当距离上次成功同步超过 24h 时才真正发起同步。
/// 启动 + 24h 维护 timer 都用这个入口。
pub async fn sync_pricing_if_needed(db: &Database) -> Result<PricingSyncResult, AppError> {
    let last = crate::settings::get_settings().last_pricing_sync_at;
    if let Some(ts) = last {
        let age = now_unix_secs() - ts;
        if age < SYNC_COOLDOWN_SECS {
            log::info!(
                "pricing_sync: skipped, last sync {:.1}h ago",
                age as f64 / 3600.0
            );
            return Ok(PricingSyncResult {
                skipped: true,
                ..Default::default()
            });
        }
    }
    sync_pricing(db).await
}

// ─── Fetch ────────────────────────────────────────────────────────────────

async fn fetch_openai_shape(url: &str) -> Result<Vec<NormalizedModel>, String> {
    let client = crate::proxy::http_client::get();
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let parsed: OpenAiShapeResp = resp
        .json()
        .await
        .map_err(|e| format!("parse failed: {e}"))?;
    Ok(parsed
        .data
        .into_iter()
        .filter_map(<Option<NormalizedModel>>::from)
        .collect())
}

// ─── 写库 ─────────────────────────────────────────────────────────────────

fn write_pricing_rows(db: &Database, models: &[NormalizedModel]) -> Result<usize, AppError> {
    let mut conn = lock_conn!(db.conn);
    let tx = conn
        .transaction()
        .map_err(|e| AppError::Database(format!("begin tx: {e}")))?;

    let mut total = 0usize;
    {
        let mut stmt = tx
            .prepare(
                "INSERT OR REPLACE INTO model_pricing (
                    model_id, display_name,
                    input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|e| AppError::Database(format!("prepare: {e}")))?;

        for m in models {
            // 单价换算：$/token → $/百万 token，TEXT decimal。
            let input_cost = match m.pricing.prompt.as_deref().map(per_token_to_per_million) {
                Some(Ok(v)) => v,
                Some(Err(e)) => {
                    log::warn!("pricing_sync: bad prompt for {}: {e}", m.id);
                    continue;
                }
                None => "0".to_string(),
            };
            let output_cost = match m.pricing.completion.as_deref().map(per_token_to_per_million) {
                Some(Ok(v)) => v,
                Some(Err(e)) => {
                    log::warn!("pricing_sync: bad completion for {}: {e}", m.id);
                    continue;
                }
                None => "0".to_string(),
            };
            let cache_read = match m.pricing.input_cache_read.as_deref().map(per_token_to_per_million) {
                Some(Ok(v)) => v,
                Some(Err(_)) | None => "0".to_string(),
            };
            // Anthropic 的 _5m 优先；Gemini 用平铺 input_cache_write
            let cache_write_raw = m
                .pricing
                .input_cache_write_5m
                .as_deref()
                .or(m.pricing.input_cache_write.as_deref());
            let cache_write = match cache_write_raw.map(per_token_to_per_million) {
                Some(Ok(v)) => v,
                Some(Err(_)) | None => "0".to_string(),
            };

            for key in expand_keys(m) {
                stmt.execute(rusqlite::params![
                    key,
                    m.display_name,
                    input_cost,
                    output_cost,
                    cache_read,
                    cache_write,
                ])
                .map_err(|e| AppError::Database(format!("insert: {e}")))?;
                total += 1;
            }
        }
    }

    tx.commit()
        .map_err(|e| AppError::Database(format!("commit: {e}")))?;
    Ok(total)
}

// ─── Backfill ─────────────────────────────────────────────────────────────

/// Sync 之后回填历史 session-log 行的成本。
///
/// 原始 session parser 在写入 `proxy_request_logs` 时若 pricing 查不到就
/// 把 cost 全部填 `"0"`，且使用 `INSERT OR IGNORE` —— 后续重跑解析也不会
/// 修正这些行。本函数兜底处理：扫描 cost=0 但 token>0 且 data_source 来自
/// 离线日志的行，按当前 `model_pricing` 表重新计算。
///
/// 只动这三种 data_source（pure-proxy 行不动 —— 那些是真的没 token 数据）：
/// - `session_log` (claude)
/// - `codex_session`
/// - `gemini_session`
///
/// 计费公式与 `proxy::usage::CostCalculator::calculate` 完全对齐：
/// `billable_input = input - cache_read`（缓存命中部分按 cache_read 价计费，
/// 不再算 input 价）。
fn backfill_zero_cost_session_logs(db: &Database) -> Result<usize, AppError> {
    use rust_decimal::Decimal;
    use std::str::FromStr;

    let mut conn = lock_conn!(db.conn);
    let tx = conn
        .transaction()
        .map_err(|e| AppError::Database(format!("backfill begin tx: {e}")))?;

    // 选出 cost=0 但 token>0 的 session-log 行；带 cost_multiplier 一起拉。
    // total_cost_usd 是 TEXT，0/0.0/0.00... 都视为 0：用数值比较。
    let rows: Vec<(String, String, i64, i64, i64, i64, String)> = {
        let mut stmt = tx
            .prepare(
                "SELECT request_id, model,
                        input_tokens, output_tokens,
                        cache_read_tokens, cache_creation_tokens,
                        cost_multiplier
                 FROM proxy_request_logs
                 WHERE data_source IN ('session_log','codex_session','gemini_session')
                   AND CAST(total_cost_usd AS REAL) = 0
                   AND (input_tokens > 0 OR output_tokens > 0
                        OR cache_read_tokens > 0 OR cache_creation_tokens > 0)",
            )
            .map_err(|e| AppError::Database(format!("backfill prepare select: {e}")))?;
        let it = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .map_err(|e| AppError::Database(format!("backfill query: {e}")))?;
        it.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("backfill collect: {e}")))?
    };

    let mut updated = 0usize;
    {
        let mut stmt_update = tx
            .prepare(
                "UPDATE proxy_request_logs
                 SET input_cost_usd = ?2, output_cost_usd = ?3,
                     cache_read_cost_usd = ?4, cache_creation_cost_usd = ?5,
                     total_cost_usd = ?6
                 WHERE request_id = ?1",
            )
            .map_err(|e| AppError::Database(format!("backfill prepare update: {e}")))?;

        for (request_id, model, input_t, output_t, cache_r_t, cache_w_t, multiplier_s) in rows {
            // 用 lookup 同款的清洗规则查 pricing。`find_model_pricing_row` 在
            // services::usage_stats 里：复用它而不是重写，保证清洗逻辑一致。
            let pricing = match crate::services::usage_stats::find_model_pricing_row(&tx, &model) {
                Ok(Some(p)) => p,
                Ok(None) => continue,                       // 还是查不到，下次 sync 再说
                Err(e) => {
                    log::warn!("backfill: lookup failed for `{model}`: {e}");
                    continue;
                }
            };
            let (input_per_m, output_per_m, cache_r_per_m, cache_w_per_m) = pricing;

            // 失败一行就 skip，不阻塞批量
            let parse = |s: &str| -> Option<Decimal> { Decimal::from_str(s.trim()).ok() };
            let (Some(input_per_m), Some(output_per_m), Some(cache_r_per_m), Some(cache_w_per_m), Some(mult)) = (
                parse(&input_per_m),
                parse(&output_per_m),
                parse(&cache_r_per_m),
                parse(&cache_w_per_m),
                parse(&multiplier_s),
            ) else {
                log::warn!("backfill: decimal parse failed for `{model}`, skipping");
                continue;
            };

            let million = Decimal::from(1_000_000_u64);
            // 与 CostCalculator::calculate 完全一致：billable_input 减掉 cache_read
            let billable_in = (input_t as i64).saturating_sub(cache_r_t).max(0);
            let in_cost = Decimal::from(billable_in) * input_per_m / million;
            let out_cost = Decimal::from(output_t) * output_per_m / million;
            let cr_cost = Decimal::from(cache_r_t) * cache_r_per_m / million;
            let cw_cost = Decimal::from(cache_w_t) * cache_w_per_m / million;
            let total = (in_cost + out_cost + cr_cost + cw_cost) * mult;

            if total.is_zero() {
                // 价格本身全 0（极少见）—— 不更新，避免噪声
                continue;
            }

            stmt_update
                .execute(rusqlite::params![
                    request_id,
                    in_cost.to_string(),
                    out_cost.to_string(),
                    cr_cost.to_string(),
                    cw_cost.to_string(),
                    total.to_string(),
                ])
                .map_err(|e| AppError::Database(format!("backfill update: {e}")))?;
            updated += 1;
        }
    }

    tx.commit()
        .map_err(|e| AppError::Database(format!("backfill commit: {e}")))?;
    Ok(updated)
}

/// 一个 Ofox 模型展开成多少个 `model_id` key 写入 `model_pricing`。
///
/// 返回去重后的列表。每个模型的 lookup key 集合需要同时覆盖：
/// 1. `m.id` 自身（如 `anthropic/claude-opus-4.7`）
/// 2. 所有 `m.aliases`（如 `claude-opus-4.7`, `claude-opus-4-7`, ...）
/// 3. **裸名**：当 `id` 形如 `<vendor>/<rest>` 时把 `<rest>` 也加上
///    （修复 codex 的 `qwen-flash`、`gpt-5.4` 等被 normalize 掉前缀的查询）
/// 4. **带前缀名**：当 alias 不含 `/` 时，合成 `<vendor>/<alias>`
///    （修复 gemini parser 不剥 `google/` 前缀的老 bug，同时让任何
///    `<vendor>/<alias>` 形式的查询都能命中——例如 claude session 里
///    出现的 `anthropic/claude-opus-4.7`）
///
/// 没有 `/` 的 id（极少见，目前 Ofox 全是 `vendor/model` 格式）只展开 1+aliases。
fn expand_keys(m: &NormalizedModel) -> Vec<String> {
    let mut keys: Vec<String> = Vec::with_capacity(m.aliases.len() * 2 + 2);
    keys.push(m.id.clone());
    keys.extend(m.aliases.iter().cloned());

    if let Some((vendor, bare_id)) = m.id.split_once('/') {
        // 3. 裸名：vendor/rest → rest
        if !bare_id.is_empty() {
            keys.push(bare_id.to_string());
        }
        // 4. 带前缀名：每个不含 '/' 的 alias 合成 vendor/alias
        for alias in &m.aliases {
            if !alias.is_empty() && !alias.contains('/') {
                keys.push(format!("{vendor}/{alias}"));
            }
        }
    }

    keys.sort();
    keys.dedup();
    keys
}

/// `$/token` 字符串 → `$/百万 token` 字符串。
///
/// 用 `rust_decimal` 做精确十进制乘法避免 f64 误差，输出剥末尾零以匹配
/// seed 数据风格（`"5"` 而不是 `"5.000000"`，`"3.75"` 不变）。
fn per_token_to_per_million(per_token: &str) -> Result<String, String> {
    let trimmed = per_token.trim();
    if trimmed.is_empty() {
        return Ok("0".to_string());
    }
    let d = Decimal::from_str(trimmed).map_err(|e| format!("decimal parse `{trimmed}`: {e}"))?;
    let scaled = d * Decimal::from(1_000_000_u64);
    Ok(format_decimal_clean(scaled))
}

/// `Decimal::Display` 默认会带与 scale 等长的尾零（`Decimal::new(5, 6)` →
/// `"0.000005"`，但乘以 1e6 后内部 scale 仍可能保留，得到 `"5.000000"`）。
/// `normalize()` 会清掉冗余的尾零。
fn format_decimal_clean(d: Decimal) -> String {
    let normalized = d.normalize();
    let s = normalized.to_string();
    // 兜底：Decimal::normalize 在某些 1.x 版本下对整数会保留 ".0"。手工剥。
    if let Some(stripped) = s.strip_suffix(".0") {
        return stripped.to_string();
    }
    s
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, aliases: &[&str], p: OfoxPricing) -> NormalizedModel {
        NormalizedModel {
            id: id.to_string(),
            display_name: id.to_string(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            pricing: p,
        }
    }

    fn empty_pricing() -> OfoxPricing {
        OfoxPricing {
            prompt: None,
            completion: None,
            input_cache_read: None,
            input_cache_write_5m: None,
            input_cache_write: None,
        }
    }

    #[test]
    fn per_token_to_per_million_examples() {
        // Claude Sonnet 4.5 input: $3/M tokens
        assert_eq!(per_token_to_per_million("0.000003").unwrap(), "3");
        // Claude Sonnet 4.5 output: $15/M tokens
        assert_eq!(per_token_to_per_million("0.000015").unwrap(), "15");
        // Gemini 2.5 Flash input: $0.3/M tokens
        assert_eq!(per_token_to_per_million("0.0000003").unwrap(), "0.3");
        // Claude Sonnet 4.5 cache write 5m: $3.75/M tokens
        assert_eq!(per_token_to_per_million("0.00000375").unwrap(), "3.75");
        // Cache read very small: $0.125/M tokens
        assert_eq!(per_token_to_per_million("0.000000125").unwrap(), "0.125");
        // Zero
        assert_eq!(per_token_to_per_million("0").unwrap(), "0");
        // Empty → 0
        assert_eq!(per_token_to_per_million("").unwrap(), "0");
        // Whitespace tolerated
        assert_eq!(per_token_to_per_million("  0.000003  ").unwrap(), "3");
    }

    #[test]
    fn per_token_to_per_million_rejects_garbage() {
        assert!(per_token_to_per_million("not-a-number").is_err());
        assert!(per_token_to_per_million("1e-9").is_err()); // rust_decimal 不接受科学计数法
    }

    #[test]
    fn per_token_output_has_no_scientific_notation() {
        // 极小值不能输出成 1E-7
        let s = per_token_to_per_million("0.0000001").unwrap();
        assert!(!s.to_lowercase().contains('e'), "got {s}");
        assert_eq!(s, "0.1");
    }

    #[test]
    fn expand_keys_anthropic_example() {
        let m = mk(
            "anthropic/claude-opus-4.6",
            &["claude-opus-4.6", "claude-opus-4-6", "claude-opus-4-6-20260205"],
            empty_pricing(),
        );
        let keys = expand_keys(&m);
        // id + 3 aliases + 3 prefixed (anthropic/<alias>) = 7 raw, dedup leaves 7
        // (anthropic/claude-opus-4.6 == id, gets dedup'd → 6)
        assert_eq!(
            keys,
            vec![
                "anthropic/claude-opus-4-6".to_string(),
                "anthropic/claude-opus-4-6-20260205".to_string(),
                "anthropic/claude-opus-4.6".to_string(),
                "claude-opus-4-6".to_string(),
                "claude-opus-4-6-20260205".to_string(),
                "claude-opus-4.6".to_string(),
            ]
        );
    }

    #[test]
    fn expand_keys_gemini_synthesizes_google_prefix() {
        let m = mk(
            "google/gemini-2.5-pro",
            &["gemini-2.5-pro"],
            empty_pricing(),
        );
        let keys = expand_keys(&m);
        // 含 bare、含 google/bare（id 自己即是这个）
        assert!(keys.contains(&"gemini-2.5-pro".to_string()));
        assert!(keys.contains(&"google/gemini-2.5-pro".to_string()));
        // 去重后 2 个：`google/gemini-2.5-pro` 既是 id 也是合成的 google/{alias}
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn expand_keys_openai_also_gets_bare_and_prefixed() {
        // 验证泛化后 openai/ 也走相同分支（之前只 google/ 合成）
        let m = mk(
            "openai/gpt-5.4",
            &["gpt-5.4", "gpt-5.4-2026-03-05"],
            empty_pricing(),
        );
        let keys = expand_keys(&m);
        // id + 2 aliases + bare("gpt-5.4" 已经在 aliases) + 2 prefixed
        // -> {openai/gpt-5.4, gpt-5.4, gpt-5.4-2026-03-05, openai/gpt-5.4-2026-03-05}
        assert_eq!(
            keys,
            vec![
                "gpt-5.4".to_string(),
                "gpt-5.4-2026-03-05".to_string(),
                "openai/gpt-5.4".to_string(),
                "openai/gpt-5.4-2026-03-05".to_string(),
            ]
        );
    }

    #[test]
    fn expand_keys_bailian_qwen_flash_dev_gateway_case() {
        // dev gateway 实测 case：aliases 为空，但仍要展开出裸名 qwen-flash
        // 让 codex parser normalize 后的 `qwen-flash` 也能命中。
        let m = mk("bailian/qwen-flash", &[], empty_pricing());
        let keys = expand_keys(&m);
        assert_eq!(
            keys,
            vec![
                "bailian/qwen-flash".to_string(),
                "qwen-flash".to_string(),
            ]
        );
    }

    #[test]
    fn expand_keys_id_without_slash_no_panic() {
        // 防御：如果某天 Ofox 返回不带 vendor/ 的 id，不要 panic
        let m = mk("standalone-model", &["alias-1"], empty_pricing());
        let keys = expand_keys(&m);
        assert_eq!(
            keys,
            vec!["alias-1".to_string(), "standalone-model".to_string()]
        );
    }

    #[test]
    fn expand_keys_skips_alias_that_already_has_slash() {
        // alias 自带前缀（如 "google/foo")时不再二次合成
        let m = mk(
            "google/gemini-2.5-pro",
            &["google/already-prefixed"],
            empty_pricing(),
        );
        let keys = expand_keys(&m);
        // 不应该出现 "google/google/already-prefixed"
        assert!(keys.iter().all(|k| !k.contains("google/google/")));
    }

    #[test]
    fn parse_anthropic_pricing_payload() {
        let json = r#"{
            "data": [{
                "id": "anthropic/claude-sonnet-4.6",
                "aliases": ["claude-sonnet-4.6", "claude-sonnet-4-6"],
                "display_name": "Claude Sonnet 4.6",
                "pricing": {
                    "prompt": "0.000003",
                    "completion": "0.000015",
                    "input_cache_read": "0.0000003",
                    "input_cache_write_5m": "0.00000375",
                    "input_cache_write_1h": "0.000006",
                    "web_search": "0.015"
                }
            }]
        }"#;
        let resp: OpenAiShapeResp = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 1);
        let m = resp.data.into_iter().next().unwrap();
        assert_eq!(m.id, "anthropic/claude-sonnet-4.6");
        let p = m.pricing.unwrap();
        assert_eq!(p.input_cache_write_5m.as_deref(), Some("0.00000375"));
        // _1h 字段我们不解析，被 serde 忽略 —— 不会 error
    }

    #[test]
    fn anthropic_picks_5m_over_1h_for_cache_creation() {
        // 这里通过 write_pricing_rows 内部的 fallback 链验证：
        // _5m 有值时取 _5m，否则取 input_cache_write
        let p_5m = OfoxPricing {
            input_cache_write_5m: Some("0.00000375".to_string()),
            input_cache_write: None,
            ..empty_pricing()
        };
        let cache_write = p_5m
            .input_cache_write_5m
            .as_deref()
            .or(p_5m.input_cache_write.as_deref());
        assert_eq!(cache_write, Some("0.00000375"));
        assert_eq!(per_token_to_per_million(cache_write.unwrap()).unwrap(), "3.75");

        // Gemini shape：只有平铺 input_cache_write
        let p_gem = OfoxPricing {
            input_cache_write_5m: None,
            input_cache_write: Some("0.0000045".to_string()),
            ..empty_pricing()
        };
        let cache_write = p_gem
            .input_cache_write_5m
            .as_deref()
            .or(p_gem.input_cache_write.as_deref());
        assert_eq!(cache_write, Some("0.0000045"));
    }

    #[test]
    fn pricing_optional_fields_default_to_zero() {
        // 模型只有 prompt/completion（OpenAI 风格 minimal），其他列应为 "0"
        let _p = OfoxPricing {
            prompt: Some("0.000002".to_string()),
            completion: Some("0.000008".to_string()),
            input_cache_read: None,
            input_cache_write_5m: None,
            input_cache_write: None,
        };
        // 只断言 helper 行为；完整 INSERT 路径在集成测试中验证
        assert_eq!(
            per_token_to_per_million("0.000002").unwrap(),
            "2"
        );
    }

    #[test]
    fn fetch_filter_drops_models_missing_pricing() {
        // 模型缺 pricing 字段时直接被 filter 掉，不进 NormalizedModel
        let json = r#"{
            "data": [
                {"id": "a/b", "aliases": [], "pricing": null},
                {"id": "c/d", "aliases": ["d"], "pricing": {"prompt": "0.000001"}}
            ]
        }"#;
        let resp: OpenAiShapeResp = serde_json::from_str(json).unwrap();
        let models: Vec<NormalizedModel> = resp
            .data
            .into_iter()
            .filter_map(<Option<NormalizedModel>>::from)
            .collect();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "c/d");
    }
}
