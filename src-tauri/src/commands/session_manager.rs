#![allow(non_snake_case)]

use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::database::Database;
use crate::services::session_usage::SessionSyncResult;
use crate::services::usage_sync;
use crate::session_manager;
use crate::session_manager::managed::ManagedSession;
use crate::store::AppState;

/// Emitted when the session listing may have changed (titles, counts, usage):
/// after `refresh_session_index`, and after a usage sync the listing started
/// wrote new rows.
pub const SESSIONS_INDEX_UPDATED_EVENT: &str = "sessions-index-updated";

async fn listing(db: Arc<Database>) -> Result<Vec<ManagedSession>, String> {
    tauri::async_runtime::spawn_blocking(move || session_manager::managed::list_with(Some(&db)))
        .await
        .map_err(|e| format!("Failed to scan sessions: {e}"))
}

/// A sync that wrote rows changed what the listing shows.
fn sync_changed_usage(result: &Option<SessionSyncResult>) -> bool {
    result.as_ref().is_some_and(|r| r.imported > 0)
}

/// The session listing: titles and message counts from the meta cache (only
/// changed files are read), usage from the database. It also starts a usage
/// sync, skipped when one ran in the last minute.
#[tauri::command]
pub async fn list_sessions(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ManagedSession>, String> {
    let db = Arc::clone(&state.db);
    let sync_db = Arc::clone(&db);
    tauri::async_runtime::spawn(async move {
        match usage_sync::sync_all_and_emit(&app, &sync_db, false).await {
            Ok(result) if sync_changed_usage(&result) => {
                if let Err(e) = app.emit(SESSIONS_INDEX_UPDATED_EVENT, ()) {
                    log::debug!("[SESSIONS] emit {SESSIONS_INDEX_UPDATED_EVENT} failed: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => log::debug!("[SESSIONS] usage sync from the listing failed: {e}"),
        }
    });
    listing(db).await
}

/// Sync usage now (never skipped), read the changed session files again and
/// return the new listing; emits `sessions-index-updated`.
#[tauri::command]
pub async fn refresh_session_index(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ManagedSession>, String> {
    let db = Arc::clone(&state.db);
    if let Err(e) = usage_sync::sync_all_and_emit(&app, &db, true).await {
        log::warn!("[SESSIONS] usage sync for the session index failed: {e}");
    }
    let sessions = listing(db).await?;
    if let Err(e) = app.emit(SESSIONS_INDEX_UPDATED_EVENT, ()) {
        log::debug!("[SESSIONS] emit {SESSIONS_INDEX_UPDATED_EVENT} failed: {e}");
    }
    Ok(sessions)
}

#[tauri::command]
pub async fn get_session_messages(
    sessionKey: String,
) -> Result<Vec<session_manager::SessionMessage>, String> {
    tauri::async_runtime::spawn_blocking(move || session_manager::managed::messages(&sessionKey))
        .await
        .map_err(|e| format!("Failed to load session messages: {e}"))?
}

#[tauri::command]
pub async fn launch_session_terminal(
    command: String,
    cwd: Option<String>,
    custom_config: Option<String>,
) -> Result<bool, String> {
    let _ = (command, cwd, custom_config);
    Err("Raw terminal commands are disabled; use the server-owned resume_session action".into())
}

#[tauri::command]
pub async fn delete_session(
    providerId: String,
    sessionId: String,
    sourcePath: String,
) -> Result<bool, String> {
    let _ = (providerId, sessionId, sourcePath);
    Err("Permanent session deletion is disabled; use the reversible session trash".into())
}

#[tauri::command]
pub async fn export_session_markdown(
    sessionKey: String,
    destination: Option<String>,
) -> Result<session_manager::managed::MarkdownExport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        session_manager::managed::export_markdown(
            &sessionKey,
            destination.as_deref().map(std::path::Path::new),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn resume_session(
    sessionKey: String,
    launch: bool,
) -> Result<session_manager::managed::ResumeResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        session_manager::managed::resume(&sessionKey, launch)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn trash_sessions(
    items: Vec<session_manager::trash::TrashRequest>,
) -> Result<Vec<session_manager::trash::TrashOutcome>, String> {
    tauri::async_runtime::spawn_blocking(move || session_manager::trash::trash_many(&items))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_session_trash() -> Result<Vec<session_manager::trash::TrashEntry>, String> {
    tauri::async_runtime::spawn_blocking(session_manager::trash::list)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn restore_session_trash(
    key: String,
) -> Result<session_manager::trash::TrashEntry, String> {
    tauri::async_runtime::spawn_blocking(move || session_manager::trash::restore(&key))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn purge_session_trash(key: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || session_manager::trash::purge(&key))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_sessions(
    items: Vec<session_manager::DeleteSessionRequest>,
) -> Result<Vec<session_manager::DeleteSessionOutcome>, String> {
    Ok(items
        .into_iter()
        .map(|item| session_manager::DeleteSessionOutcome {
            provider_id: item.provider_id,
            session_id: item.session_id,
            source_path: item.source_path,
            success: false,
            error: Some(
                "Permanent session deletion is disabled; use the reversible session trash".into(),
            ),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synced(imported: u32) -> Option<SessionSyncResult> {
        Some(SessionSyncResult {
            imported,
            skipped: 0,
            files_scanned: 1,
            errors: vec![],
        })
    }

    #[test]
    fn only_a_sync_that_wrote_rows_changes_the_listing() {
        assert!(sync_changed_usage(&synced(2)));
        assert!(!sync_changed_usage(&synced(0)));
        // skipped: a sync ran in the last minute
        assert!(!sync_changed_usage(&None));
    }
}
