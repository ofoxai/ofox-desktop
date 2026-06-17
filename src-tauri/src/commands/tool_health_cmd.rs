//! Tauri commands for the tool-health subsystem.
//!
//! Two commands:
//!   - `get_tool_health_snapshot` — UI reads this on mount to render
//!     pills before the next event arrives.
//!   - `trigger_tool_health_check_now` — manual "check now" hook for
//!     the settings dialog and DevTools. Runs the same code path the
//!     periodic loop uses, on the same `Sleeper`-less fast path.
//!
//! Both commands are intentionally thin wrappers around
//! `services::tool_health::run_health_check_round` — the cache and
//! event semantics live there.

use std::collections::HashMap;

use tauri::{AppHandle, State};

use crate::commands::ofox_auth::OfoxAuthState;
use crate::services::tool_health::{
    run_health_check_round, ToolHealthSnapshot,
};
use crate::store::AppState;

/// Snapshot of every tool's last probe result. Empty map until the loop
/// has run at least once (or until the user has bound any tools).
#[tauri::command]
pub async fn get_tool_health_snapshot(
    state: State<'_, AppState>,
) -> Result<HashMap<String, ToolHealthSnapshot>, String> {
    Ok(state.tool_health_cache.snapshot().await)
}

/// Run one health-check round immediately, ignoring the timer. Returns
/// the resulting snapshot so the caller doesn't have to also subscribe
/// to the event for the trigger -> result round-trip.
#[tauri::command]
pub async fn trigger_tool_health_check_now(
    app: AppHandle,
    state: State<'_, AppState>,
    ofox_state: State<'_, OfoxAuthState>,
) -> Result<HashMap<String, ToolHealthSnapshot>, String> {
    let bound_tools = crate::settings::get_bound_tools();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let snap = run_health_check_round(
        &app,
        &state,
        &ofox_state.0,
        &state.tool_health_cache,
        &bound_tools,
        now_ms,
    )
    .await;
    Ok(snap)
}
