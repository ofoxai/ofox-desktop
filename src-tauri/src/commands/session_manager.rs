#![allow(non_snake_case)]

use crate::session_manager;

#[tauri::command]
pub async fn list_sessions() -> Result<Vec<session_manager::managed::ManagedSession>, String> {
    // Codex and Claude sessions live under CODEX_HOME / CLAUDE_CONFIG_DIR when set.
    crate::agent_env::ready().await;
    let sessions = tauri::async_runtime::spawn_blocking(session_manager::managed::list)
        .await
        .map_err(|e| format!("Failed to scan sessions: {e}"))?;
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
