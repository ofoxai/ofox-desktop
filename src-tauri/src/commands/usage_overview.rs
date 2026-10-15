//! The usage page's single command.

use crate::error::AppError;
use crate::services::usage_overview::{build_overview, UsageOverview, UsagePeriod};
use crate::services::usage_sync;
use crate::store::AppState;
use tauri::State;

/// Everything the usage page shows for `period`: totals, series, per-tool and
/// per-model breakdowns. Syncs the session sources first (debounced to once a
/// minute); a failing sync is logged and the overview is built from what the
/// database already has.
#[tauri::command(rename_all = "camelCase")]
pub async fn get_usage_overview(
    state: State<'_, AppState>,
    period: UsagePeriod,
    query: Option<crate::services::usage_query::UsageQueryV1>,
) -> Result<UsageOverview, AppError> {
    if let Some(query) = query {
        let db = state.db.clone();
        let apex = crate::ofox_apex::current_apex();
        let now_ms = chrono::Utc::now().timestamp_millis();
        return tokio::task::spawn_blocking(move || {
            crate::services::usage_analytics::build_query_overview(
                &db,
                period,
                query,
                apex,
                now_ms,
                &chrono::Local,
                usage_sync::last_synced_at_ms(),
            )
        })
        .await
        .map_err(|e| AppError::Message(format!("usage overview task failed: {e}")))?;
    }
    if let Err(e) = usage_sync::sync_all(&state.db, false).await {
        log::warn!("[USAGE-OVERVIEW] sync before the overview failed: {e}");
    }

    let db = state.db.clone();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let last_synced_at = usage_sync::last_synced_at_ms();
    tokio::task::spawn_blocking(move || {
        build_overview(&db, period, now_ms, &chrono::Local, last_synced_at)
    })
    .await
    .map_err(|e| AppError::Message(format!("usage overview task failed: {e}")))?
}
