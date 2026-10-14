//! Usage sync: every session source (Claude, Codex, Gemini, OpenCode) read into
//! `proxy_request_logs`, then the pending one-shot maintenance.
//!
//! One run at a time (a tokio mutex); a run within 60 s of the last one is
//! skipped unless forced. It runs at startup (after the pricing sync, since rows
//! are priced as they are written) and every 10 minutes, and when the user asks
//! (`sync_session_usage`, forced). A run that happened emits `usage-synced`.

use crate::database::Database;
use crate::error::AppError;
use crate::services::session_usage::SessionSyncResult;
use crate::services::{
    pricing_sync, session_usage, session_usage_codex, session_usage_gemini, session_usage_opencode,
    usage_maintenance,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// A run this soon after the last one is skipped unless forced.
const DEBOUNCE: Duration = Duration::from_secs(60);
/// How often the background loop syncs.
const PERIODIC_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// Emitted with the run's `SessionSyncResult` after every run that happened.
pub const USAGE_SYNCED_EVENT: &str = "usage-synced";

/// Serializes runs and remembers when the last one finished.
struct UsageSync {
    last_run: tokio::sync::Mutex<Option<Instant>>,
}

impl UsageSync {
    const fn new() -> Self {
        Self {
            last_run: tokio::sync::Mutex::const_new(None),
        }
    }

    async fn run(
        &self,
        db: &Arc<Database>,
        force: bool,
    ) -> Result<Option<SessionSyncResult>, AppError> {
        let mut last_run = self.last_run.lock().await;
        if !force {
            if let Some(at) = *last_run {
                if at.elapsed() < DEBOUNCE {
                    log::debug!("[USAGE-SYNC] skipped: last run {:?} ago", at.elapsed());
                    return Ok(None);
                }
            }
        }
        let db = Arc::clone(db);
        let result = tokio::task::spawn_blocking(move || sync_sources(&db))
            .await
            .map_err(|e| AppError::Message(format!("usage sync task failed: {e}")))?;
        *last_run = Some(Instant::now());
        Ok(Some(result))
    }
}

static USAGE_SYNC: UsageSync = UsageSync::new();

/// Sync every session source, then run pending maintenance. Returns `None` when
/// skipped because a run finished less than 60 s ago (`force` never skips).
pub async fn sync_all(
    db: &Arc<Database>,
    force: bool,
) -> Result<Option<SessionSyncResult>, AppError> {
    USAGE_SYNC.run(db, force).await
}

/// `sync_all`, then `usage-synced` with the result when a run happened.
pub async fn sync_all_and_emit(
    app: &AppHandle,
    db: &Arc<Database>,
    force: bool,
) -> Result<Option<SessionSyncResult>, AppError> {
    let result = sync_all(db, force).await?;
    if let Some(result) = &result {
        if let Err(e) = app.emit(USAGE_SYNCED_EVENT, result) {
            log::debug!("[USAGE-SYNC] emit {USAGE_SYNCED_EVENT} failed: {e}");
        }
    }
    Ok(result)
}

/// The startup sync (after the pricing sync) and one every 10 minutes after.
pub fn spawn_periodic(app: AppHandle, db: Arc<Database>) {
    tauri::async_runtime::spawn(async move {
        // rows are priced as they are written: refresh prices first (24h cooldown)
        if let Err(e) = pricing_sync::sync_pricing_if_needed(&db).await {
            log::warn!("Pricing sync initial tick failed: {e}");
        }
        let mut interval = tokio::time::interval(PERIODIC_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await; // the first tick is immediate: the startup run
            if let Err(e) = sync_all_and_emit(&app, &db, false).await {
                log::debug!("[USAGE-SYNC] periodic run failed: {e}");
            }
        }
    });
}

type SourceSync = fn(&Database) -> Result<SessionSyncResult, AppError>;

/// Every source in turn (one failing does not stop the others), then the
/// pending maintenance; failures are collected in `errors`.
fn sync_sources(db: &Database) -> SessionSyncResult {
    let sources: [(&str, SourceSync); 4] = [
        ("Claude", session_usage::sync_claude_session_logs),
        ("Codex", session_usage_codex::sync_codex_usage),
        ("Gemini", session_usage_gemini::sync_gemini_usage),
        ("OpenCode", session_usage_opencode::sync_opencode_usage),
    ];
    let mut total = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: 0,
        errors: vec![],
    };
    for (name, sync) in sources {
        match sync(db) {
            Ok(result) => {
                total.imported += result.imported;
                total.skipped += result.skipped;
                total.files_scanned += result.files_scanned;
                total.errors.extend(result.errors);
            }
            Err(e) => total.errors.push(format!("{name} 同步失败: {e}")),
        }
    }
    if let Err(e) = usage_maintenance::run_pending(db) {
        total.errors.push(format!("用量维护失败: {e}"));
    }
    log::debug!(
        "[USAGE-SYNC] done: {} written, {} unchanged, {} files, {} errors",
        total.imported,
        total.skipped,
        total.files_scanned,
        total.errors.len()
    );
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_sandbox::sandbox;
    use serial_test::serial;
    use std::path::Path;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn data_sources(db: &Database) -> Vec<String> {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT DISTINCT data_source FROM proxy_request_logs ORDER BY data_source")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    /// One call of each source, written where each agent keeps its sessions.
    fn write_one_call_per_source(home: &Path) {
        write(
            &home.join(".claude/projects/-work/s1.jsonl"),
            &serde_json::json!({"type": "assistant", "sessionId": "s1",
                "timestamp": "2026-10-10T10:00:00Z",
                "message": {"id": "msg_1", "model": "claude-opus-4-6", "stop_reason": "end_turn",
                    "usage": {"input_tokens": 3, "output_tokens": 10}}})
            .to_string(),
        );
        write(
            &home.join(".codex/sessions/2026/10/10/rollout-2026-10-10T10-00-00-t1.jsonl"),
            &[
                serde_json::json!({"timestamp": "2026-10-10T10:00:00Z", "type": "session_meta",
                    "payload": {"id": "t1"}}),
                serde_json::json!({"timestamp": "2026-10-10T10:00:01Z", "type": "event_msg",
                    "payload": {"type": "token_count", "info": {"total_token_usage":
                        {"input_tokens": 100, "cached_input_tokens": 0, "output_tokens": 5}}}}),
            ]
            .map(|l| l.to_string())
            .join("\n"),
        );
        write(
            &home.join(".gemini/tmp/h/chats/session-1.json"),
            &serde_json::json!({"sessionId": "g1", "messages": [{"id": "m1", "type": "gemini",
                "model": "gemini-2.5-pro", "timestamp": "2026-10-10T10:00:00Z",
                "tokens": {"input": 10, "output": 5}}]})
            .to_string(),
        );
        write(
            &home.join(".local/share/opencode/storage/message/ses_1/msg_1.json"),
            &serde_json::json!({"id": "msg_1", "sessionID": "ses_1", "role": "assistant",
                "modelID": "gpt-5.4", "tokens": {"input": 10, "output": 5, "reasoning": 0,
                    "cache": {"read": 0, "write": 0}},
                "time": {"created": 1791800000000_i64, "completed": 1791800001000_i64}})
            .to_string(),
        );
    }

    #[tokio::test]
    #[serial]
    async fn a_run_reads_every_source_and_runs_pending_maintenance() {
        let sandbox = sandbox();
        write_one_call_per_source(sandbox.home());
        let db = Arc::new(Database::memory().unwrap());
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO maintenance_flags VALUES ('backfill_session_archive', 'pending')",
                [],
            )
            .unwrap();
        }

        let result = UsageSync::new().run(&db, false).await.unwrap().unwrap();

        assert_eq!(result.imported, 4, "{result:?}");
        assert_eq!(
            data_sources(&db),
            [
                "codex_session",
                "gemini_session",
                "opencode_session",
                "session_log"
            ]
        );
        let conn = db.conn.lock().unwrap();
        let flag: String = conn
            .query_row(
                "SELECT value FROM maintenance_flags WHERE key = 'backfill_session_archive'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(flag, "done");
    }

    #[tokio::test]
    #[serial]
    async fn a_run_within_a_minute_is_skipped_unless_forced() {
        let _sandbox = sandbox();
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();

        assert!(sync.run(&db, false).await.unwrap().is_some());
        assert!(sync.run(&db, false).await.unwrap().is_none());
        assert!(sync.run(&db, true).await.unwrap().is_some());
    }

    #[tokio::test]
    #[serial]
    async fn concurrent_runs_take_turns_and_the_second_is_skipped() {
        let _sandbox = sandbox();
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();

        let (a, b) = tokio::join!(sync.run(&db, false), sync.run(&db, false));

        let ran = [a.unwrap(), b.unwrap()]
            .iter()
            .filter(|r| r.is_some())
            .count();
        assert_eq!(ran, 1);
    }
}
