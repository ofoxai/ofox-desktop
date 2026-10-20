//! 使用统计相关命令

use crate::error::AppError;
use crate::services::usage_stats::*;
use crate::store::AppState;
use tauri::State;

/// 获取使用量汇总
///
/// 范围 = cc-switch 数据库 `proxy_request_logs` 全部数据，包括 proxy 实拦
/// 与三种离线日志（claude/codex/gemini session）。详见
/// [`crate::services::usage_stats::Database::get_usage_summary`] 注释。
#[tauri::command]
pub fn get_usage_summary(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
) -> Result<UsageSummary, AppError> {
    state
        .db
        .get_usage_summary(start_date, end_date, app_type.as_deref())
}

/// 获取每日趋势
#[tauri::command]
pub fn get_usage_trends(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
) -> Result<Vec<DailyStats>, AppError> {
    state
        .db
        .get_daily_trends(start_date, end_date, app_type.as_deref())
}

/// 获取 Provider 统计
#[tauri::command]
pub fn get_provider_stats(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
) -> Result<Vec<ProviderStats>, AppError> {
    state
        .db
        .get_provider_stats(start_date, end_date, app_type.as_deref())
}

/// 获取模型统计
#[tauri::command]
pub fn get_model_stats(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
) -> Result<Vec<ModelStats>, AppError> {
    state
        .db
        .get_model_stats(start_date, end_date, app_type.as_deref())
}

/// 获取请求日志列表
#[tauri::command]
pub fn get_request_logs(
    state: State<'_, AppState>,
    filters: Option<LogFilters>,
    page: u32,
    page_size: u32,
    query: Option<crate::services::usage_query::UsageQueryV1>,
    request_filters: Option<crate::services::usage_data::RequestFilters>,
    snapshot_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    if let Some(query) = query {
        if filters.as_ref().is_some_and(|f| {
            f.app_type.is_some()
                || f.provider_name.is_some()
                || f.model.is_some()
                || f.status_code.is_some()
                || f.start_date.is_some()
                || f.end_date.is_some()
        }) {
            return Err(AppError::InvalidInput(
                "usage.requests.conflictingLegacyFilters".into(),
            ));
        }
        let result = crate::services::usage_requests::request_page(
            &state.db,
            query,
            request_filters.unwrap_or_default(),
            crate::services::usage_requests::Pagination { page, page_size },
            snapshot_id.as_deref(),
            crate::ofox_apex::current_apex(),
            chrono::Utc::now().timestamp_millis(),
        )?;
        serde_json::to_value(result).map_err(|e| AppError::Database(e.to_string()))
    } else {
        if request_filters.is_some() || snapshot_id.is_some() {
            return Err(AppError::InvalidInput(
                "usage.requests.queryRequired".into(),
            ));
        }
        serde_json::to_value(state.db.get_request_logs(
            &filters.unwrap_or_default(),
            page,
            page_size,
        )?)
        .map_err(|e| AppError::Database(e.to_string()))
    }
}

/// Existing callers keep the legacy row shape; a snapshot pins both data and prices.
#[tauri::command]
pub fn get_request_detail(
    state: State<'_, AppState>,
    request_id: String,
    snapshot_id: Option<String>,
) -> Result<serde_json::Value, AppError> {
    if let Some(snapshot_id) = snapshot_id {
        serde_json::to_value(crate::services::usage_requests::request_detail(
            &request_id,
            &snapshot_id,
        )?)
        .map_err(|e| AppError::Database(e.to_string()))
    } else {
        serde_json::to_value(state.db.get_request_detail(&request_id)?)
            .map_err(|e| AppError::Database(e.to_string()))
    }
}

#[tauri::command]
pub fn export_request_logs_csv(
    snapshot_id: String,
    destination: Option<String>,
) -> Result<crate::services::usage_requests::CsvExport, AppError> {
    crate::services::usage_requests::export_csv(&snapshot_id, destination.as_deref())
}

#[tauri::command]
pub fn get_usage_capabilities() -> serde_json::Value {
    serde_json::json!({"queryVersion":1,"requestSnapshots":true,"csv":true,"projectFilter":true,
        "localRefresh":true,"sessionAnalytics":true,"sessionProjectFilter":true,"nativeRebuild":true,
        "sources":crate::services::usage_query::source_capabilities()})
}

#[cfg(test)]
mod workbench_capability_tests {
    #[test]
    fn workbench_capability_exposes_the_integrated_local_and_cached_query_paths() {
        let value = super::get_usage_capabilities();
        for key in [
            "localRefresh",
            "sessionAnalytics",
            "sessionProjectFilter",
            "projectFilter",
        ] {
            assert_eq!(value[key], true, "missing integrated capability {key}");
        }
    }
    #[test]
    fn workbench_capability_registry_only_claims_native_analytics_for_bridged_readers() {
        for capability in crate::services::usage_query::source_capabilities() {
            let supported = [
                "claude",
                "codex",
                "gemini",
                "opencode",
                "workbuddy",
                "pi",
                "dsh",
                "openclaw",
                "hermes",
            ]
            .contains(&capability.tool_id.as_str());
            assert_eq!(
                capability.session_analytics_supported, supported,
                "{} analytics bridge",
                capability.tool_id
            );
            assert_eq!(
                capability.project_filter_supported, supported,
                "{} project bridge",
                capability.tool_id
            );
        }
    }
}

/// 获取模型定价列表
#[tauri::command]
pub fn get_model_pricing(state: State<'_, AppState>) -> Result<Vec<ModelPricingInfo>, AppError> {
    log::info!("获取模型定价列表");
    state.db.ensure_model_pricing_seeded()?;

    let db = state.db.clone();
    let conn = crate::database::lock_conn!(db.conn);

    // 检查表是否存在
    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='model_pricing'",
            [],
            |row| row.get::<_, i64>(0).map(|count| count > 0),
        )
        .unwrap_or(false);

    if !table_exists {
        log::error!("model_pricing 表不存在,可能需要重启应用以触发数据库迁移");
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
         FROM model_pricing
         ORDER BY display_name",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(ModelPricingInfo {
            model_id: row.get(0)?,
            display_name: row.get(1)?,
            input_cost_per_million: row.get(2)?,
            output_cost_per_million: row.get(3)?,
            cache_read_cost_per_million: row.get(4)?,
            cache_creation_cost_per_million: row.get(5)?,
        })
    })?;

    let mut pricing = Vec::new();
    for row in rows {
        pricing.push(row?);
    }

    log::info!("成功获取 {} 条模型定价数据", pricing.len());
    Ok(pricing)
}

/// 更新模型定价
#[tauri::command]
pub fn update_model_pricing(
    state: State<'_, AppState>,
    model_id: String,
    display_name: String,
    input_cost: String,
    output_cost: String,
    cache_read_cost: String,
    cache_creation_cost: String,
) -> Result<(), AppError> {
    let db = state.db.clone();
    let conn = crate::database::lock_conn!(db.conn);

    conn.execute(
        "INSERT OR REPLACE INTO model_pricing (
            model_id, display_name, input_cost_per_million, output_cost_per_million,
            cache_read_cost_per_million, cache_creation_cost_per_million
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            model_id,
            display_name,
            input_cost,
            output_cost,
            cache_read_cost,
            cache_creation_cost
        ],
    )
    .map_err(|e| AppError::Database(format!("更新模型定价失败: {e}")))?;

    Ok(())
}

/// 检查 Provider 使用限额
#[tauri::command]
pub fn check_provider_limits(
    state: State<'_, AppState>,
    provider_id: String,
    app_type: String,
) -> Result<crate::services::usage_stats::ProviderLimitStatus, AppError> {
    state.db.check_provider_limits(&provider_id, &app_type)
}

/// 删除模型定价
#[tauri::command]
pub fn delete_model_pricing(state: State<'_, AppState>, model_id: String) -> Result<(), AppError> {
    let db = state.db.clone();
    let conn = crate::database::lock_conn!(db.conn);

    conn.execute(
        "DELETE FROM model_pricing WHERE model_id = ?1",
        rusqlite::params![model_id],
    )
    .map_err(|e| AppError::Database(format!("删除模型定价失败: {e}")))?;

    log::info!("已删除模型定价: {model_id}");
    Ok(())
}

/// 手动触发会话日志同步
///
/// 兼带 Ofox pricing 强制刷新（无 cooldown）—— 用户点「刷新」就是想看到
/// 最新的成本估算，包括新上线模型的 pricing。pricing 失败不阻塞日志同步。
/// The sync itself is `usage_sync::sync_all` forced (never skipped), which reads
/// every session source, runs pending maintenance and emits `usage-synced`.
#[tauri::command]
pub async fn sync_session_usage(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::services::session_usage::SessionSyncResult, AppError> {
    // Codex / Claude sessions may live in CODEX_HOME / CLAUDE_CONFIG_DIR.
    crate::agent_env::ready().await;

    // Pricing 刷新放在 session 解析之前：session_usage_* 在写入每条
    // proxy_request_logs 时按 model 查 model_pricing 算费用，先把表更新
    // 到最新版本，刚刚还没收录的模型这次同步即可正确计费。
    if let Err(e) = crate::services::pricing_sync::sync_pricing(&state.db).await {
        log::warn!("Pricing sync during manual refresh failed: {e}");
    }

    let result = crate::services::usage_sync::sync_all_and_emit(&app, &state.db, true).await?;
    Ok(
        result.unwrap_or(crate::services::session_usage::SessionSyncResult {
            imported: 0,
            skipped: 0,
            files_scanned: 0,
            errors: vec![],
        }),
    )
}

/// Local polling refreshes native facts and analytics without pricing or
/// account requests. Unchanged revisions return the compatible empty result.
#[tauri::command]
pub async fn refresh_local_usage(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::services::session_usage::SessionSyncResult, AppError> {
    let result = crate::services::usage_sync::sync_all_and_emit(&app, &state.db, false).await?;
    Ok(
        result.unwrap_or(crate::services::session_usage::SessionSyncResult {
            imported: 0,
            skipped: 0,
            files_scanned: 0,
            errors: vec![],
        }),
    )
}

#[tauri::command]
pub fn get_usage_rebuild_progress() -> Option<crate::services::usage_rebuild::RebuildProgress> {
    crate::services::usage_rebuild::progress()
}

#[tauri::command(rename_all = "camelCase")]
pub fn cancel_usage_rebuild(
    operation_id: String,
) -> Result<crate::services::usage_rebuild::RebuildProgress, AppError> {
    crate::services::usage_rebuild::cancel(&operation_id)
}

/// 获取数据来源分布
#[tauri::command]
pub fn get_usage_data_sources(
    state: State<'_, AppState>,
) -> Result<Vec<crate::services::session_usage::DataSourceSummary>, AppError> {
    crate::services::session_usage::get_data_source_breakdown(&state.db)
}

/// 模型定价信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricingInfo {
    pub model_id: String,
    pub display_name: String,
    pub input_cost_per_million: String,
    pub output_cost_per_million: String,
    pub cache_read_cost_per_million: String,
    pub cache_creation_cost_per_million: String,
}
