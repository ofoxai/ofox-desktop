//! One local refresh coordinates native usage, analytics, and source health.
//! A worker owns the gate until the real blocking job ends, even if an IPC
//! waiter disappears. Unchanged native/DB revisions skip parsing; manual bursts
//! can request one followup, and requests during that followup join it.
use crate::database::Database;
use crate::error::AppError;
use crate::services::session_usage::SessionSyncResult;
use crate::services::{
    pricing_sync, session_usage, session_usage_codex, session_usage_gemini, session_usage_opencode,
    session_usage_workbuddy, usage_maintenance,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const PERIODIC_INTERVAL: Duration = Duration::from_secs(10 * 60);
pub const USAGE_SYNCED_EVENT: &str = "usage-synced";

#[derive(Default)]
struct Requests {
    completed: u64,
    followup: bool,
    running: bool,
}
struct Shared {
    gate: Arc<tokio::sync::Mutex<()>>,
    requests: Mutex<Requests>,
    probe: Mutex<Option<Probe>>,
    last_synced_ms: AtomicI64,
}
struct UsageSync {
    shared: Arc<Shared>,
}
struct Completion {
    shared: Arc<Shared>,
    id: u64,
}
impl Drop for Completion {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.requests.lock() {
            state.completed = self.id;
            state.running = false;
            state.followup = false;
        }
    }
}
struct JobOutcome(Option<SessionSyncResult>);
impl From<SessionSyncResult> for JobOutcome {
    fn from(value: SessionSyncResult) -> Self {
        Self(Some(value))
    }
}
impl From<Option<SessionSyncResult>> for JobOutcome {
    fn from(value: Option<SessionSyncResult>) -> Self {
        Self(value)
    }
}
fn scheduler_error() -> AppError {
    AppError::Message("usage.sync.schedulerUnavailable".into())
}
impl UsageSync {
    fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                gate: Arc::new(tokio::sync::Mutex::new(())),
                requests: Mutex::new(Requests::default()),
                probe: Mutex::new(None),
                last_synced_ms: AtomicI64::new(0),
            }),
        }
    }
    fn last_synced_at(&self) -> Option<i64> {
        match self.shared.last_synced_ms.load(Ordering::Relaxed) {
            0 => None,
            ms => Some(ms),
        }
    }
    async fn run(
        &self,
        db: &Arc<Database>,
        force: bool,
    ) -> Result<Option<SessionSyncResult>, AppError> {
        let db = Arc::clone(db);
        let shared = Arc::clone(&self.shared);
        self.run_job(force, move || {
            let Ok(mut probe) = shared.probe.lock() else {
                return Some(error_result("usage.sync.probeUnavailable"));
            };
            refresh_local(&db, force, &mut probe)
        })
        .await
    }
    async fn run_job<F, R>(
        &self,
        force: bool,
        job: F,
    ) -> Result<Option<SessionSyncResult>, AppError>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Into<JobOutcome> + Send + 'static,
    {
        let (requested_at, join_followup) = {
            let state = self.shared.requests.lock().map_err(|_| scheduler_error())?;
            (state.completed, state.running && state.followup)
        };
        let gate = Arc::clone(&self.shared.gate).lock_owned().await;
        // No worker has started yet: cancellation here can safely release gate.
        crate::agent_env::ready().await;
        let id = {
            let mut state = self.shared.requests.lock().map_err(|_| scheduler_error())?;
            let advanced = state.completed > requested_at;
            if advanced
                && (!force || join_followup || state.completed > requested_at.saturating_add(1))
            {
                return Ok(None);
            }
            let id = state.completed.checked_add(1).ok_or_else(scheduler_error)?;
            state.running = true;
            state.followup = advanced;
            id
        };
        let shared = Arc::clone(&self.shared);
        let completion = Completion {
            shared: Arc::clone(&shared),
            id,
        };
        tokio::task::spawn_blocking(move || {
            // Drop completion before gate, including while unwinding a panic.
            let _gate = gate;
            let _completion = completion;
            let result = job().into().0;
            if result.is_some() {
                shared
                    .last_synced_ms
                    .store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
            }
            result
        })
        .await
        .map_err(|_| AppError::Message("usage.sync.taskFailed".into()))
    }
}
static USAGE_SYNC: LazyLock<UsageSync> = LazyLock::new(UsageSync::new);

#[derive(Clone, PartialEq, Eq)]
struct DbRevision {
    epoch: Option<String>,
    changes: i64,
    data_version: i64,
}
impl DbRevision {
    fn read(db: &Database) -> Result<Self, AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        Ok(Self {
            epoch: crate::session_manager::source_cache::ledger_epoch_on(&conn)?,
            changes: conn.query_row("SELECT total_changes()", [], |r| r.get(0))?,
            data_version: conn.query_row("PRAGMA data_version", [], |r| r.get(0))?,
        })
    }
}
type Observations = BTreeMap<String, BTreeMap<PathBuf, serde_json::Value>>;
struct Probe {
    observations: Observations,
    db_revision: DbRevision,
    retry: BTreeSet<String>,
    maintenance_failed: bool,
}
struct Observed {
    discovery: super::session_analytics_index::Discovery,
    observations: Observations,
    failed: BTreeSet<String>,
}
fn observes_analytics(provider: &str) -> bool {
    matches!(provider, "claude" | "codex" | "gemini" | "opencode")
}
fn observe_sources() -> Observed {
    use super::session_analytics_index::{discover_sources, source_revision};
    let discovery = discover_sources();
    let mut observations: Observations = SOURCES
        .iter()
        .map(|s| (s.provider.to_owned(), BTreeMap::new()))
        .collect();
    let mut failed: BTreeSet<_> = discovery
        .failures
        .iter()
        .map(|failure| failure.provider.clone())
        .collect();
    for spec in &discovery.specs {
        match source_revision(spec) {
            Ok(stamp) => {
                observations
                    .entry(spec.provider.clone())
                    .or_default()
                    .insert(spec.path.clone(), stamp);
            }
            Err(_) => {
                failed.insert(spec.provider.clone());
            }
        }
    }
    let workbuddy = (|| {
        let root = crate::session_manager::providers::workbuddy::session_root()?;
        let mut stamps = BTreeMap::new();
        for path in session_usage_workbuddy::collect_files(&root)? {
            let stamp = super::usage_source::normalized_stamp("workbuddy", &path)?;
            stamps.insert(path, stamp);
        }
        Ok::<_, AppError>(stamps)
    })();
    match workbuddy {
        Ok(stamps) => {
            observations.insert("workbuddy".into(), stamps);
        }
        Err(_) => {
            failed.insert("workbuddy".into());
        }
    }
    Observed {
        discovery,
        observations,
        failed,
    }
}
fn error_result(code: &str) -> SessionSyncResult {
    SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: 0,
        errors: vec![code.into()],
    }
}
fn refresh_local(
    db: &Database,
    force: bool,
    saved: &mut Option<Probe>,
) -> Option<SessionSyncResult> {
    let before_db = match DbRevision::read(db) {
        Ok(revision) => revision,
        Err(_) => return Some(error_result("usage.sync.revisionUnavailable")),
    };
    // Capture observations before parsing. A concurrent append must remain a
    // change next time, even when a native reader happens to see the new bytes.
    let observed = observe_sources();
    let all = force
        || saved
            .as_ref()
            .is_none_or(|old| old.db_revision != before_db);
    let selected: BTreeSet<_> = SOURCES
        .iter()
        .filter(|source| {
            all || observed.failed.contains(source.provider)
                || saved.as_ref().is_some_and(|old| {
                    old.retry.contains(source.provider)
                        || old.observations.get(source.provider)
                            != observed.observations.get(source.provider)
                })
        })
        .map(|source| source.provider)
        .collect();
    if selected.is_empty() && !saved.as_ref().is_some_and(|old| old.maintenance_failed) {
        return None;
    }
    let (mut total, mut failed) = sync_selected_sources(db, &selected);
    failed.extend(observed.failed.iter().cloned());
    if selected.iter().any(|provider| observes_analytics(provider)) {
        // Refresh expects a complete discovery: a subset would falsely mark
        // unselected providers' cached paths as missing. Warm peers do no body reads.
        refresh_analytics(db, &observed.discovery, &mut total, &mut failed);
    }
    let maintenance_failed = !run_maintenance(db, &mut total);
    let after_db = match DbRevision::read(db) {
        Ok(revision) => revision,
        Err(_) => return Some(error_result("usage.sync.revisionUnavailable")),
    };
    if before_db.epoch != after_db.epoch {
        // A restore between provider batches may have replaced an earlier
        // provider's committed ledger. Never freeze those observations as read.
        *saved = None;
        total.errors.push("usage.sync.ledgerChangedRetry".into());
        return Some(total);
    }
    let mut observations = saved.take().map(|old| old.observations).unwrap_or_default();
    for (provider, stamps) in observed.observations {
        if !failed.contains(&provider) {
            observations.insert(provider, stamps);
        }
    }
    // This records our own final writes too. Changes from another operation
    // invalidate the probe but confer no acknowledgement or coverage proof.
    *saved = Some(Probe {
        observations,
        db_revision: after_db,
        retry: failed,
        maintenance_failed,
    });
    Some(total)
}

/// Sync every session source, then run pending maintenance. Returns `None` when
/// the unchanged revision probe skips work. Queued manual calls coalesce.
pub async fn sync_all(
    db: &Arc<Database>,
    force: bool,
) -> Result<Option<SessionSyncResult>, AppError> {
    USAGE_SYNC.run(db, force).await
}

/// When the last sync run finished (epoch milliseconds); `None` before the first
/// run of this process.
pub fn last_synced_at_ms() -> Option<i64> {
    USAGE_SYNC.last_synced_at()
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

struct Source {
    provider: &'static str,
    source_id: &'static str,
    name: &'static str,
    sync: SourceSync,
}

const SOURCES: &[Source] = &[
    Source {
        provider: "claude",
        source_id: "session_log",
        name: "Claude",
        sync: session_usage::sync_claude_session_logs,
    },
    Source {
        provider: "codex",
        source_id: "codex_session",
        name: "Codex",
        sync: session_usage_codex::sync_codex_usage,
    },
    Source {
        provider: "gemini",
        source_id: "gemini_session",
        name: "Gemini",
        sync: session_usage_gemini::sync_gemini_usage,
    },
    Source {
        provider: "opencode",
        source_id: "opencode_session",
        name: "OpenCode",
        sync: session_usage_opencode::sync_opencode_usage,
    },
    Source {
        provider: "workbuddy",
        source_id: "workbuddy_session",
        name: "WorkBuddy",
        sync: session_usage_workbuddy::sync_workbuddy_usage,
    },
];

fn begin_source(db: &Database, source: &Source) -> Result<super::usage_health::Attempt, AppError> {
    begin_source_channel(db, source, super::usage_health::Channel::Ledger)
}
fn begin_source_channel(
    db: &Database,
    source: &Source,
    channel: super::usage_health::Channel,
) -> Result<super::usage_health::Attempt, AppError> {
    let mut conn = crate::database::lock_conn!(db.conn);
    let tx = conn.transaction()?;
    let attempt = super::usage_health::begin_on(
        &tx,
        source.provider,
        source.source_id,
        channel,
        chrono::Utc::now().timestamp_millis(),
    )?;
    tx.commit()?;
    Ok(attempt)
}

fn refresh_analytics(
    db: &Database,
    discovery: &super::session_analytics_index::Discovery,
    total: &mut SessionSyncResult,
    failed: &mut BTreeSet<String>,
) {
    use super::{
        session_analytics_index::{refresh_discovered, RefreshState},
        usage_health,
    };
    let sources: Vec<_> = SOURCES
        .iter()
        .filter(|s| observes_analytics(s.provider))
        .collect();
    let attempts: Vec<_> = sources
        .iter()
        .map(
            |source| match begin_source_channel(db, source, usage_health::Channel::Analytics) {
                Ok(attempt) => Some(attempt),
                Err(_) => {
                    failed.insert(source.provider.into());
                    total
                        .errors
                        .push(format!("{}: usage.sources.healthWriteFailed", source.name));
                    None
                }
            },
        )
        .collect();
    let report = refresh_discovered(db, discovery);
    for (source, attempt) in sources.into_iter().zip(attempts) {
        let state = match &report {
            Ok(report) => report
                .provider_states
                .get(source.provider)
                .copied()
                .unwrap_or(RefreshState::Partial("usage.sources.noFiles")),
            Err(_) => RefreshState::Failed("usage.sources.analyticsReadFailed"),
        };
        let outcome = match state {
            RefreshState::Ok => usage_health::Outcome::Ok,
            RefreshState::Partial(code) => usage_health::Outcome::Partial(code),
            RefreshState::Failed(code) => {
                failed.insert(source.provider.into());
                total.errors.push(format!("{}: {code}", source.name));
                usage_health::Outcome::Failed(code)
            }
        };
        if let Some(attempt) = attempt {
            let saved = (|| {
                let mut conn = crate::database::lock_conn!(db.conn);
                let tx = conn.transaction()?;
                if !usage_health::finish_on(
                    &tx,
                    &attempt,
                    outcome,
                    chrono::Utc::now().timestamp_millis(),
                )? {
                    return Err(AppError::Database("usage.sources.staleAttempt".into()));
                }
                tx.commit()?;
                Ok::<_, AppError>(())
            })();
            if saved.is_err() {
                failed.insert(source.provider.into());
                total
                    .errors
                    .push(format!("{}: usage.sources.healthWriteFailed", source.name));
            }
        }
    }
}

fn finish_source(
    db: &Database,
    source: &Source,
    attempt: &super::usage_health::Attempt,
    failed: bool,
    empty: bool,
) -> Result<(), AppError> {
    use super::usage_health::{finish_on, Outcome};
    let mut conn = crate::database::lock_conn!(db.conn);
    let tx = conn.transaction()?;
    let outcome = if failed {
        Outcome::Failed("usage.sources.syncFailed")
    } else if empty {
        Outcome::Partial("usage.sources.noFiles")
    } else if super::usage_source::has_complete_coverage_on(&tx, source.provider)? {
        Outcome::Ok
    } else {
        Outcome::Partial("usage.sources.incompleteCoverage")
    };
    if !finish_on(&tx, attempt, outcome, chrono::Utc::now().timestamp_millis())? {
        return Err(AppError::Database("usage.sources.staleAttempt".into()));
    }
    tx.commit()?;
    Ok(())
}

/// Every source in turn (one failing does not stop the others), then the
/// pending maintenance; failures are collected in `errors`.
#[cfg(test)]
fn sync_sources(db: &Database) -> SessionSyncResult {
    let selected = SOURCES.iter().map(|source| source.provider).collect();
    let (mut total, _) = sync_selected_sources(db, &selected);
    run_maintenance(db, &mut total);
    total
}
fn sync_selected_sources(
    db: &Database,
    selected: &BTreeSet<&str>,
) -> (SessionSyncResult, BTreeSet<String>) {
    let mut total = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: 0,
        errors: vec![],
    };
    let mut failed_providers = BTreeSet::new();
    for source in SOURCES
        .iter()
        .filter(|source| selected.contains(source.provider))
    {
        let attempt = match begin_source(db, source) {
            Ok(attempt) => Some(attempt),
            Err(_) => {
                failed_providers.insert(source.provider.into());
                total
                    .errors
                    .push(format!("{}: usage.sources.healthWriteFailed", source.name));
                None
            }
        };
        let (failed, empty) = match (source.sync)(db) {
            Ok(result) => {
                let failed = !result.errors.is_empty();
                let empty = result.files_scanned == 0;
                total.imported += result.imported;
                total.skipped += result.skipped;
                total.files_scanned += result.files_scanned;
                total.errors.extend(result.errors);
                (failed, empty)
            }
            Err(e) => {
                total.errors.push(format!("{} 同步失败: {e}", source.name));
                (true, false)
            }
        };
        if failed {
            failed_providers.insert(source.provider.into());
        }
        if let Some(attempt) = attempt {
            if finish_source(db, source, &attempt, failed, empty).is_err() {
                failed_providers.insert(source.provider.into());
                total
                    .errors
                    .push(format!("{}: usage.sources.healthWriteFailed", source.name));
            }
        }
    }
    (total, failed_providers)
}
/// False only for an attempted maintenance failure. A source-caused deferral
/// already has its own provider retry; stable successful providers stay warm.
fn run_maintenance(db: &Database, total: &mut SessionSyncResult) -> bool {
    // Backfill reparses old files. Publishing it after a rolled-back source
    // batch would archive those calls before their unchanged-file retry writes
    // them live, counting them twice. Leave pending flags for a successful run.
    if total.errors.is_empty() {
        if let Err(e) = usage_maintenance::run_pending(db) {
            total.errors.push(format!("用量维护失败: {e}"));
            return false;
        }
    }
    log::debug!(
        "[USAGE-SYNC] done: {} written, {} unchanged, {} files, {} errors",
        total.imported,
        total.skipped,
        total.files_scanned,
        total.errors.len()
    );
    true
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

    fn workbuddy_partial_fixture(
        home: &Path,
        missing_model: bool,
        incomplete: bool,
    ) -> std::path::PathBuf {
        let path = home.join(".workbuddy/projects/native/s.jsonl");
        let mut row = serde_json::json!({"type":"message","role":"assistant","sessionId":"native-wb","timestamp":1791800000123_i64,"providerData":{"messageId":"m","model":"native","usage":{"requests":1,"inputTokens":3,"outputTokens":2}}});
        if missing_model {
            row["providerData"].as_object_mut().unwrap().remove("model");
        }
        let mut body = row.to_string() + "\n";
        if incomplete {
            body.push_str("{\"providerData\":{\"messageId\":\"unfinished\"");
        }
        write(&path, &body);
        path
    }

    fn status(db: &Database, tool: &str) -> super::super::usage_query::SourceHealth {
        // These tests exercise the ledger writer without analytics. Assert its
        // owned channel explicitly; the common reader separately requires all
        // advertised channels and must remain unknown without analytics.
        let raw: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM settings WHERE key='usage_source_health_v1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let source = value["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["toolId"] == tool)
            .unwrap();
        let mut ledger = source["ledger"].clone();
        ledger["toolId"] = source["toolId"].clone();
        ledger["sourceId"] = source["sourceId"].clone();
        serde_json::from_value(ledger).unwrap()
    }

    #[test]
    #[serial]
    fn source_status_failure_is_scoped_and_unchanged_retry_is_visible() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        let db = Database::memory().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_gemini BEFORE INSERT ON proxy_request_logs
             WHEN NEW.app_type='gemini' BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;",
            )
            .unwrap();
        let failed = sync_sources(&db);
        assert!(!failed.errors.is_empty());
        let row = status(&db, "gemini");
        assert_eq!(row.status, "failed");
        assert!(row.last_attempt_at_ms.is_some());
        assert!(row.last_success_at_ms.is_none());
        assert_eq!(row.error_code.as_deref(), Some("usage.sources.syncFailed"));
        assert_eq!(status(&db, "claude").status, "ok");
        assert_eq!(status(&db, "opencode").status, "ok");
        assert_eq!(status(&db, "codex").status, "partial");
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_gemini")
            .unwrap();
        let retried = sync_sources(&db);
        assert!(retried.errors.is_empty(), "{retried:?}");
        assert_eq!(retried.imported, 1);
        let row = status(&db, "gemini");
        assert_eq!(row.status, "ok");
        assert!(row.last_success_at_ms.is_some());
    }

    #[test]
    #[serial]
    fn source_status_partial_is_retained_on_an_unchanged_native_scan() {
        let home = sandbox();
        workbuddy_partial_fixture(home.home(), true, false);
        let db = Database::memory().unwrap();
        for _ in 0..2 {
            let result = sync_sources(&db);
            assert!(
                result.errors.is_empty(),
                "partial is not transport failure: {result:?}"
            );
            let row = status(&db, "workbuddy");
            assert_eq!(row.status, "partial");
            assert!(row.last_attempt_at_ms.is_some());
            assert!(row.last_success_at_ms.is_none());
        }
    }

    #[test]
    #[serial]
    fn source_status_receipt_loss_is_partial_even_when_remaining_files_are_unchanged() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        let db = Database::memory().unwrap();
        assert!(sync_sources(&db).errors.is_empty());
        assert_eq!(status(&db, "claude").status, "ok");
        db.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM session_meta_cache WHERE provider_id='claude'",
                [],
            )
            .unwrap();
        let result = sync_sources(&db);
        assert!(
            result.errors.is_empty(),
            "lost proof is partial: {result:?}"
        );
        assert_eq!(status(&db, "claude").status, "partial");
    }

    #[test]
    #[serial]
    fn source_status_health_write_failure_does_not_stop_native_facts_or_siblings() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        let db = Database::memory().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_health BEFORE INSERT ON settings
             WHEN NEW.key='usage_source_health_v1'
             BEGIN SELECT RAISE(ABORT,'fixture health write failure'); END;",
            )
            .unwrap();
        let result = sync_sources(&db);
        assert_eq!(
            result.imported, 4,
            "health persistence failure must not cancel native readers"
        );
        assert_eq!(data_sources(&db).len(), 4);
        assert!(
            !result.errors.is_empty(),
            "the health DB failure was swallowed"
        );
    }

    #[test]
    #[serial]
    fn source_status_complete_manifest_without_its_receipt_is_not_complete_health() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        let db = Database::memory().unwrap();
        session_usage::sync_claude_session_logs(&db).unwrap();
        let conn = db.conn.lock().unwrap();
        assert!(super::super::usage_source::has_complete_coverage_on(&conn, "claude").unwrap());
        conn.execute(
            "DELETE FROM session_meta_cache WHERE provider_id='claude'",
            [],
        )
        .unwrap();
        assert!(!super::super::usage_source::has_complete_coverage_on(&conn, "claude").unwrap());
    }

    #[test]
    #[serial]
    fn source_status_typed_partial_coverage_is_not_hidden_by_identity_completeness() {
        use crate::session_manager::source_cache::{self, Part};
        let home = sandbox();
        write_one_call_per_source(home.home());
        let db = Database::memory().unwrap();
        session_usage::sync_claude_session_logs(&db).unwrap();
        let conn = db.conn.lock().unwrap();
        let path = home.home().join(".claude/projects/-work/s1.jsonl");
        let parts = source_cache::read_on(&conn, path.to_str().unwrap(), "claude").unwrap();
        let mut value = parts.value(Part::UsageSync).unwrap().clone();
        value["coverage"] = serde_json::json!("partial");
        value["reasons"] = serde_json::json!(["historicalSessionAncestryUnverified"]);
        source_cache::merge_on(
            &conn,
            path.to_str().unwrap(),
            "claude",
            None,
            Part::UsageSync,
            &parts.token(Part::UsageSync),
            Some(value),
        )
        .unwrap();
        assert_eq!(
            source_cache::provider_on(&conn, "claude").unwrap().unwrap()["complete"],
            true
        );
        assert!(!super::super::usage_source::has_complete_coverage_on(&conn, "claude").unwrap());
    }

    #[test]
    fn source_status_coverage_database_failure_is_not_reported_as_complete() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute_batch("DROP TABLE maintenance_flags").unwrap();
        assert!(super::super::usage_source::has_complete_coverage_on(&conn, "claude").is_err());
    }

    fn source_disappears(tool: &str, relative: &str) {
        let home = sandbox();
        write_one_call_per_source(home.home());
        workbuddy_partial_fixture(home.home(), false, false);
        let db = Database::memory().unwrap();
        assert!(sync_sources(&db).errors.is_empty());
        let before = status(&db, tool);
        std::fs::remove_dir_all(home.home().join(relative)).unwrap();
        assert!(sync_sources(&db).errors.is_empty());
        let after = status(&db, tool);
        assert_eq!(
            after.status, "partial",
            "cached proof must not claim that a vanished native source was read"
        );
        assert_eq!(after.error_code.as_deref(), Some("usage.sources.noFiles"));
        assert_eq!(after.last_success_at_ms, before.last_success_at_ms);
        assert!(after.last_attempt_at_ms >= before.last_attempt_at_ms);
    }

    #[test]
    #[serial]
    fn source_status_disappeared_claude_does_not_advance_prior_success() {
        source_disappears("claude", ".claude/projects");
    }
    #[test]
    #[serial]
    fn source_status_disappeared_codex_does_not_advance_prior_success() {
        source_disappears("codex", ".codex/sessions");
    }
    #[test]
    #[serial]
    fn source_status_disappeared_gemini_does_not_advance_prior_success() {
        source_disappears("gemini", ".gemini/tmp");
    }
    #[test]
    #[serial]
    fn source_status_disappeared_opencode_does_not_advance_prior_success() {
        source_disappears("opencode", ".local/share/opencode");
    }
    #[test]
    #[serial]
    fn source_status_disappeared_workbuddy_does_not_advance_prior_success() {
        source_disappears("workbuddy", ".workbuddy/projects");
    }

    fn claude_call(id: &str) -> String {
        serde_json::json!({"type":"assistant","sessionId":"refresh-s",
            "timestamp":"2026-10-15T00:00:00Z","message":{"id":id,
            "model":"claude-opus-4-6","stop_reason":"end_turn",
            "usage":{"input_tokens":3,"output_tokens":2}}})
        .to_string()
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_changed_file_is_imported_within_the_old_minute_debounce() {
        let home = sandbox();
        let path = home.home().join(".claude/projects/p/s.jsonl");
        write(&path, &claude_call("m1"));
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        assert_eq!(sync.run(&db, false).await.unwrap().unwrap().imported, 1);
        write(&path, &(claude_call("m1") + "\n" + &claude_call("m2")));
        let next = sync.run(&db, false).await.unwrap();
        assert_eq!(
            next.map(|r| r.imported),
            Some(1),
            "a changed source must not wait 60 seconds"
        );
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_failed_unchanged_file_can_retry_within_the_old_debounce() {
        let home = sandbox();
        write(
            &home.home().join(".claude/projects/p/s.jsonl"),
            &claude_call("m1"),
        );
        let db = Arc::new(Database::memory().unwrap());
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_first BEFORE INSERT ON proxy_request_logs
             BEGIN SELECT RAISE(ABORT,'fixture first attempt failure'); END;",
            )
            .unwrap();
        let sync = UsageSync::new();
        let first = sync.run(&db, false).await.unwrap().unwrap();
        assert!(!first.errors.is_empty());
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_first")
            .unwrap();
        let next = sync.run(&db, false).await.unwrap();
        assert_eq!(
            next.map(|r| r.imported),
            Some(1),
            "failure must not debounce the unchanged retry"
        );
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_failed_maintenance_retries_without_a_native_or_row_change() {
        let _home = sandbox();
        let db = Arc::new(Database::memory().unwrap());
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO maintenance_flags VALUES('recompute_claude_costs','pending');
             CREATE TRIGGER fail_maintenance BEFORE INSERT ON maintenance_flags
             WHEN NEW.key='recompute_claude_costs' AND NEW.value='done'
             BEGIN SELECT RAISE(ABORT,'fixture maintenance failure'); END;",
            )
            .unwrap();
        let sync = UsageSync::new();
        let first = sync.run(&db, false).await.unwrap().unwrap();
        assert!(!first.errors.is_empty());
        // DDL does not change SQLite total_changes() on this connection. An
        // unfinished step must itself remain retryable when its input is stable.
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_maintenance")
            .unwrap();
        let retry = sync
            .run(&db, false)
            .await
            .unwrap()
            .expect("unfinished maintenance was hidden by the warm probe");
        assert!(retry.errors.is_empty(), "{retry:?}");
        let state: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM maintenance_flags WHERE key='recompute_claude_costs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "done");
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_a_new_source_file_is_not_hidden_by_the_old_debounce() {
        let home = sandbox();
        write(
            &home.home().join(".claude/projects/p/one.jsonl"),
            &claude_call("m1"),
        );
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        assert_eq!(sync.run(&db, false).await.unwrap().unwrap().imported, 1);
        write(
            &home.home().join(".claude/projects/p/two.jsonl"),
            &claude_call("m2"),
        );
        let next = sync.run(&db, false).await.unwrap();
        assert_eq!(next.map(|r| r.imported), Some(1));
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_six_hundred_unchanged_jsonl_files_have_no_second_body_reads() {
        let home = sandbox();
        for i in 0..600 {
            write(
                &home.home().join(format!(".claude/projects/p/{i}.jsonl")),
                &claude_call(&format!("m{i}")),
            );
        }
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        assert_eq!(sync.run(&db, false).await.unwrap().unwrap().imported, 600);
        let before = super::super::usage_source::body_reads_under(home.home());
        assert!(sync.run(&db, false).await.unwrap().is_none());
        assert_eq!(
            super::super::usage_source::body_reads_under(home.home()),
            before
        );
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_finishes_analytics_before_returning_the_local_result() {
        let home = sandbox();
        write(
            &home.home().join(".claude/projects/p/s.jsonl"),
            &claude_call("m1"),
        );
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        assert_eq!(sync.run(&db, false).await.unwrap().unwrap().imported, 1);
        let input = super::super::session_analytics_index::read_input(&db).unwrap();
        assert_eq!(
            input.facts.len(),
            1,
            "local refresh returned before analytics was refreshed"
        );
        assert_eq!(input.sessions.len(), 1);
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_metadata_only_revision_updates_the_cached_analysis() {
        use super::super::session_analytics_index::{
            discover_sources, read_input, refresh_discovered,
        };
        let home = sandbox();
        let path = home.home().join(".codex/sessions/2026/10/15/refresh.jsonl");
        write(&path, &[
            serde_json::json!({"type":"session_meta","payload":{"id":"refresh-x","cwd":"/fixture-project","source":"cli"}}),
            serde_json::json!({"type":"event_msg","timestamp":"2026-10-15T00:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":3,"output_tokens":2}}}})
        ].iter().map(serde_json::Value::to_string).collect::<Vec<_>>().join("\n"));
        let names = home.home().join(".codex/session_index.jsonl");
        write(
            &names,
            &serde_json::json!({"id":"refresh-x","thread_name":"Before"}).to_string(),
        );
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        sync.run(&db, false).await.unwrap();
        refresh_discovered(&db, &discover_sources()).unwrap();
        assert_eq!(read_input(&db).unwrap().sessions[0].title, "Before");
        write(
            &names,
            &serde_json::json!({"id":"refresh-x","thread_name":"After"}).to_string(),
        );
        assert!(sync.run(&db, false).await.unwrap().is_some());
        assert_eq!(read_input(&db).unwrap().sessions[0].title, "After");
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_database_receipt_loss_invalidates_an_unchanged_native_probe() {
        let home = sandbox();
        write(
            &home.home().join(".claude/projects/p/s.jsonl"),
            &claude_call("m1"),
        );
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        assert_eq!(sync.run(&db, false).await.unwrap().unwrap().imported, 1);
        db.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM session_meta_cache WHERE provider_id='claude'",
                [],
            )
            .unwrap();
        let result = sync
            .run(&db, false)
            .await
            .unwrap()
            .expect("DB changes must invalidate the warm probe");
        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(result.imported, 0);
        assert_eq!(status(&db, "claude").status, "partial");
        assert_eq!(
            super::super::session_analytics_index::read_input(&db)
                .unwrap()
                .facts
                .len(),
            1
        );
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_changed_provider_preserves_unchanged_analytics_peers() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        write(
            &home
                .home()
                .join(".local/share/opencode/storage/session/project/ses_1.json"),
            &serde_json::json!({"id":"ses_1","title":"Native peer","directory":"/fixture-project"})
                .to_string(),
        );
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        sync.run(&db, false).await.unwrap();
        let before = super::super::session_analytics_index::read_input(&db).unwrap();
        for provider in ["codex", "gemini", "opencode"] {
            assert!(
                before
                    .sessions
                    .iter()
                    .any(|s| s.identity.tool_id == provider && s.source_available),
                "fixture must first provide an available {provider} session"
            );
        }
        write(
            &home.home().join(".claude/projects/other/new.jsonl"),
            &claude_call("new"),
        );
        assert!(sync.run(&db, false).await.unwrap().is_some());
        let input = super::super::session_analytics_index::read_input(&db).unwrap();
        for provider in ["codex", "gemini", "opencode"] {
            assert!(
                input
                    .sessions
                    .iter()
                    .any(|s| s.identity.tool_id == provider && s.source_available),
                "unchanged {provider} became a missing source"
            );
        }
    }

    fn empty_result() -> SessionSyncResult {
        SessionSyncResult {
            imported: 0,
            skipped: 0,
            files_scanned: 0,
            errors: vec![],
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn source_scheduler_cancelling_a_waiter_keeps_the_running_job_serialized() {
        let _home = sandbox();
        let sync = Arc::new(UsageSync::new());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first_sync = Arc::clone(&sync);
        let first = tokio::spawn(async move {
            first_sync
                .run_job(true, move || {
                    let _ = started_tx.send(());
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    empty_result()
                })
                .await
        });
        started_rx.await.unwrap();
        first.abort();
        let _ = first.await;
        let (second_tx, second_rx) = tokio::sync::oneshot::channel();
        let next_sync = Arc::clone(&sync);
        let second = tokio::spawn(async move {
            next_sync
                .run_job(true, move || {
                    let _ = second_tx.send(());
                    empty_result()
                })
                .await
        });
        let overlapped = tokio::time::timeout(Duration::from_millis(250), second_rx)
            .await
            .is_ok();
        let _ = release_tx.send(());
        second.await.unwrap().unwrap();
        assert!(
            !overlapped,
            "dropping the IPC waiter released a still-running blocking job's gate"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn source_scheduler_manual_burst_queues_at_most_one_followup() {
        use std::future::Future;
        use std::task::Poll;
        let _home = sandbox();
        let sync = Arc::new(UsageSync::new());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first_sync = Arc::clone(&sync);
        let first_calls = Arc::clone(&calls);
        let first = tokio::spawn(async move {
            first_sync
                .run_job(true, move || {
                    first_calls.fetch_add(1, Ordering::Relaxed);
                    let _ = started_tx.send(());
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    empty_result()
                })
                .await
        });
        started_rx.await.unwrap();
        let mut waiters: Vec<_> = (0..4)
            .map(|_| {
                let calls = Arc::clone(&calls);
                Box::pin(sync.run_job(true, move || {
                    calls.fetch_add(1, Ordering::Relaxed);
                    empty_result()
                }))
            })
            .collect();
        // Poll every request while the first worker still holds the gate, so
        // these are one deterministic burst rather than scheduler timing luck.
        std::future::poll_fn(|cx| {
            for waiter in &mut waiters {
                assert!(waiter.as_mut().poll(cx).is_pending());
            }
            Poll::Ready(())
        })
        .await;
        release_tx.send(()).unwrap();
        first.await.unwrap().unwrap();
        for waiter in waiters {
            waiter.await.unwrap();
        }
        assert_eq!(
            calls.load(Ordering::Relaxed),
            2,
            "four queued clicks replayed the whole scan four times"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn source_scheduler_manual_during_forced_followup_joins_without_a_third_run() {
        use std::future::Future;
        use std::task::Poll;
        let _home = sandbox();
        let sync = Arc::new(UsageSync::new());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (first_started_tx, first_started_rx) = tokio::sync::oneshot::channel();
        let (first_release_tx, first_release_rx) = std::sync::mpsc::channel();
        let first_sync = Arc::clone(&sync);
        let first_calls = Arc::clone(&calls);
        let first = tokio::spawn(async move {
            first_sync
                .run_job(true, move || {
                    first_calls.fetch_add(1, Ordering::Relaxed);
                    let _ = first_started_tx.send(());
                    first_release_rx
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap();
                    empty_result()
                })
                .await
        });
        first_started_rx.await.unwrap();
        let (follow_started_tx, follow_started_rx) = tokio::sync::oneshot::channel();
        let (follow_release_tx, follow_release_rx) = std::sync::mpsc::channel();
        let follow_sync = Arc::clone(&sync);
        let follow_calls = Arc::clone(&calls);
        let mut followup = Box::pin(async move {
            follow_sync
                .run_job(true, move || {
                    follow_calls.fetch_add(1, Ordering::Relaxed);
                    let _ = follow_started_tx.send(());
                    follow_release_rx
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap();
                    empty_result()
                })
                .await
        });
        std::future::poll_fn(|cx| {
            assert!(followup.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        first_release_tx.send(()).unwrap();
        first.await.unwrap().unwrap();
        std::future::poll_fn(|cx| {
            assert!(followup.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        follow_started_rx.await.unwrap();
        let last_calls = Arc::clone(&calls);
        let mut late = Box::pin(sync.run_job(true, move || {
            last_calls.fetch_add(1, Ordering::Relaxed);
            empty_result()
        }));
        std::future::poll_fn(|cx| {
            assert!(late.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        follow_release_tx.send(()).unwrap();
        followup.await.unwrap();
        late.await.unwrap();
        assert_eq!(
            calls.load(Ordering::Relaxed),
            2,
            "clicks during the followup must join it, not create a new generation"
        );
    }

    #[test]
    #[serial]
    fn workbuddy_partial_model_does_not_block_other_sources_pending_maintenance() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        let path = workbuddy_partial_fixture(home.home(), true, false);
        let db = Database::memory().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO maintenance_flags VALUES('recompute_claude_costs','pending')",
                [],
            )
            .unwrap();
        let result = sync_sources(&db);
        let conn = db.conn.lock().unwrap();
        let flag: String = conn
            .query_row(
                "SELECT value FROM maintenance_flags WHERE key='recompute_claude_costs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            flag, "done",
            "WorkBuddy coverage partial must not defer unrelated maintenance: {result:?}"
        );
        assert!(
            result.errors.is_empty(),
            "partial is not a transport failure: {result:?}"
        );
        let parts = crate::session_manager::source_cache::read_on(
            &conn,
            path.to_str().unwrap(),
            "workbuddy",
        )
        .unwrap();
        let evidence = parts
            .value(crate::session_manager::source_cache::Part::UsageSync)
            .unwrap();
        assert_eq!(evidence["coverage"], "partial");
        assert!(evidence["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "missingModel"));
    }

    #[test]
    #[serial]
    fn workbuddy_partial_tail_does_not_block_maintenance_or_acknowledge_an_unfinished_event() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        let path = workbuddy_partial_fixture(home.home(), false, true);
        let db = Database::memory().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO maintenance_flags VALUES('recompute_claude_costs','pending')",
                [],
            )
            .unwrap();
        let result = sync_sources(&db);
        let conn = db.conn.lock().unwrap();
        let flag: String = conn
            .query_row(
                "SELECT value FROM maintenance_flags WHERE key='recompute_claude_costs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            flag, "done",
            "A valid native prefix is partial coverage, not failed maintenance: {result:?}"
        );
        assert!(result.errors.is_empty());
        let (records,offset):(i64,i64)=conn.query_row("SELECT (SELECT COUNT(*) FROM proxy_request_logs WHERE app_type='workbuddy'),(SELECT last_line_offset FROM session_log_sync WHERE file_path=?1)",[path.to_str().unwrap()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((records, offset), (1, 1));
        let parts = crate::session_manager::source_cache::read_on(
            &conn,
            path.to_str().unwrap(),
            "workbuddy",
        )
        .unwrap();
        let evidence = parts
            .value(crate::session_manager::source_cache::Part::UsageSync)
            .unwrap();
        assert_eq!(evidence["completeEventIdentitySet"], false);
        assert_eq!(
            evidence["acknowledgedEventIds"].as_array().unwrap().len(),
            1
        );
        assert!(evidence["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "incompleteEnumeration"));
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
        assert_eq!(flag, "partial");
    }

    #[test]
    #[serial]
    fn atomic_sync_one_failed_source_keeps_the_other_three() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        let db = Database::memory().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TEMP TRIGGER fail_claude BEFORE INSERT ON proxy_request_logs
             WHEN NEW.data_source = 'session_log'
             BEGIN SELECT RAISE(ABORT, 'fixture Claude write failure'); END;",
            )
            .unwrap();
        let result = sync_sources(&db);
        assert!(!result.errors.is_empty(), "{result:?}");
        assert_eq!((result.imported, result.skipped), (3, 0));
        assert_eq!(
            data_sources(&db),
            ["codex_session", "gemini_session", "opencode_session"]
        );
    }

    #[test]
    #[serial]
    fn atomic_sync_failed_old_call_defers_backfill_until_an_unchanged_retry() {
        let home = sandbox();
        let timestamp = (chrono::Utc::now() - chrono::Duration::days(40)).to_rfc3339();
        write(
            &home.home().join(".claude/projects/p/old.jsonl"),
            &serde_json::json!({"type":"assistant","sessionId":"old-atomic","timestamp":timestamp,
                "message":{"id":"old-atomic","model":"claude-opus-4-6","stop_reason":"end_turn",
                    "usage":{"input_tokens":100,"output_tokens":10}}})
            .to_string(),
        );
        let db = Database::memory().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO maintenance_flags VALUES ('backfill_session_archive','pending');
             CREATE TEMP TRIGGER fail_old BEFORE INSERT ON proxy_request_logs
             BEGIN SELECT RAISE(ABORT, 'fixture old call failure'); END;
             CREATE TEMP TRIGGER fail_old_checkpoint BEFORE INSERT ON session_log_sync
             BEGIN SELECT RAISE(ABORT, 'fixture old checkpoint failure'); END;",
            )
            .unwrap();
        let failed = sync_sources(&db);
        assert!(!failed.errors.is_empty(), "{failed:?}");
        let (archive_after_failure, flag_after_failure) = {
            let conn = db.conn.lock().unwrap();
            let archive: i64 = conn
                .query_row("SELECT COUNT(*) FROM session_usage_archive", [], |r| {
                    r.get(0)
                })
                .unwrap();
            let flag: String = conn
                .query_row(
                    "SELECT value FROM maintenance_flags WHERE key='backfill_session_archive'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            conn.execute_batch("DROP TRIGGER fail_old; DROP TRIGGER fail_old_checkpoint;")
                .unwrap();
            (archive, flag)
        };
        let retried = sync_sources(&db);
        assert!(retried.errors.is_empty(), "{retried:?}");
        assert_eq!(retried.imported, 1);
        db.rollup_and_prune(30).unwrap();
        let conn = db.conn.lock().unwrap();
        let total: i64 = conn.query_row("SELECT
            (SELECT COUNT(*) FROM proxy_request_logs WHERE session_id='old-atomic') +
            (SELECT COALESCE(SUM(request_count),0) FROM session_usage_archive WHERE session_id='old-atomic')",
            [],|r|r.get(0)).unwrap();
        assert_eq!(total, 1, "retry must not count archive and live twice");
        assert_eq!(
            archive_after_failure, 0,
            "backfill must not publish rolled-back source facts"
        );
        assert_eq!(flag_after_failure, "pending");
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
    async fn a_run_records_when_it_finished_and_a_skipped_one_keeps_it() {
        let _sandbox = sandbox();
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        assert_eq!(sync.last_synced_at(), None);

        let before = chrono::Utc::now().timestamp_millis();
        sync.run(&db, false).await.unwrap();
        let first = sync.last_synced_at().expect("a run happened");
        sync.run(&db, false).await.unwrap(); // debounced

        assert!(first >= before);
        assert_eq!(sync.last_synced_at(), Some(first));
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
