//! One local refresh coordinates native usage, analytics, and source health.
//! A worker owns the gate until the real blocking job ends, even if an IPC
//! waiter disappears. Unchanged native/DB revisions skip parsing; manual bursts
//! can request one followup, and requests during that followup join it.
use crate::database::Database;
use crate::error::AppError;
use crate::services::session_usage::SessionSyncResult;
use crate::services::{
    pricing_sync, session_usage, session_usage_codex, session_usage_dsh, session_usage_gemini,
    session_usage_openclaw, session_usage_opencode, session_usage_pi, session_usage_workbuddy,
    usage_maintenance,
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
    #[cfg(test)]
    before_scan: Mutex<Option<Box<dyn FnOnce() + Send>>>,
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
                #[cfg(test)]
                before_scan: Mutex::new(None),
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
            refresh_local(&db, force, &mut probe, || {
                #[cfg(test)]
                {
                    let hook = shared.before_scan.lock().unwrap().take();
                    if let Some(hook) = hook {
                        hook();
                    }
                }
            })
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
#[derive(PartialEq)]
struct Revision {
    roots: Vec<PathBuf>,
    files: BTreeMap<PathBuf, serde_json::Value>,
}
type Observations = BTreeMap<String, Revision>;
struct Probe {
    observations: Observations,
    db_revision: DbRevision,
    retry: BTreeSet<String>,
    maintenance_failed: bool,
}
struct FrozenRoots(BTreeMap<&'static str, Result<Vec<PathBuf>, ()>>);
impl FrozenRoots {
    fn capture() -> Result<Self, AppError> {
        // Freeze configuration only. Enumeration and native reads happen after
        // releasing this barrier, using these paths without resolving again.
        let _io = crate::config_io::lock();
        if crate::settings::settings_health().state
            == crate::settings::SettingsHealthState::Unreadable
        {
            return Err(AppError::Message("usage.sources.settingsUnreadable".into()));
        }
        Ok(Self(
            SOURCES
                .iter()
                .map(|source| {
                    let roots = (source.resolve)().map_err(|_| ()).and_then(|roots| {
                        if roots.is_empty() {
                            Err(())
                        } else {
                            Ok(roots)
                        }
                    });
                    (source.provider, roots)
                })
                .collect(),
        ))
    }
    fn get(&self, provider: &str) -> Result<&[PathBuf], AppError> {
        self.0
            .get(provider)
            .and_then(|roots| roots.as_ref().ok())
            .map(Vec::as_slice)
            .ok_or_else(root_unavailable)
    }
}
fn root_unavailable() -> AppError {
    AppError::Message("usage.sources.rootUnavailable".into())
}
fn single_root(roots: &[PathBuf]) -> Result<&std::path::Path, AppError> {
    match roots {
        [root] => Ok(root.as_path()),
        _ => Err(root_unavailable()),
    }
}
/// Compatibility wrappers also freeze root resolution before native I/O.
#[cfg(test)]
pub(crate) fn resolve_root(provider: &str) -> Result<PathBuf, AppError> {
    let roots = FrozenRoots::capture()?;
    Ok(single_root(roots.get(provider)?)?.to_owned())
}
struct Observed {
    discovery: super::session_analytics_index::Discovery,
    observations: Observations,
    failed: BTreeSet<String>,
    roots: FrozenRoots,
}
fn observes_analytics(provider: &str) -> bool {
    matches!(
        provider,
        "claude" | "codex" | "gemini" | "opencode" | "workbuddy" | "pi" | "dsh" | "openclaw"
    )
}
fn observe_sources(roots: FrozenRoots) -> Observed {
    use super::session_analytics_index::source_revision;
    let native_roots: Vec<_> = SOURCES
        .iter()
        .filter(|s| observes_analytics(s.provider))
        .flat_map(|source| {
            roots
                .get(source.provider)
                .unwrap_or_default()
                .iter()
                .map(move |root| (source.provider, root.clone()))
        })
        .collect();
    let mut discovery = super::session_analytics_discovery::discover_in(&native_roots);
    let mut observations: Observations = SOURCES
        .iter()
        .map(|source| {
            (
                source.provider.to_owned(),
                Revision {
                    roots: roots.get(source.provider).unwrap_or_default().to_vec(),
                    files: BTreeMap::new(),
                },
            )
        })
        .collect();
    let mut failed: BTreeSet<_> = discovery
        .failures
        .iter()
        .map(|failure| failure.provider.clone())
        .collect();
    for source in SOURCES {
        if roots.get(source.provider).is_err() {
            failed.insert(source.provider.into());
            if observes_analytics(source.provider) {
                discovery
                    .failures
                    .push(super::session_analytics_index::DiscoveryFailure {
                        provider: source.provider.into(),
                        code: "usage.sources.rootUnavailable",
                    });
            }
        }
    }
    for spec in &discovery.specs {
        match source_revision(spec) {
            Ok(stamp) => {
                observations
                    .get_mut(&spec.provider)
                    .expect("known provider")
                    .files
                    .insert(spec.path.clone(), stamp);
            }
            Err(_) => {
                failed.insert(spec.provider.clone());
            }
        }
    }
    for source in SOURCES {
        let Some(enumerate) = source.observe else {
            continue;
        };
        let result = (|| {
            let paths = enumerate(roots.get(source.provider)?)?;
            let mut stamps = BTreeMap::new();
            for path in paths {
                let stamp = super::usage_source::normalized_stamp(source.provider, &path)?;
                stamps.insert(path, stamp);
            }
            Ok::<_, AppError>(stamps)
        })();
        match result {
            Ok(stamps) => {
                observations
                    .get_mut(source.provider)
                    .expect("known provider")
                    .files = stamps;
            }
            Err(_) => {
                failed.insert(source.provider.into());
            }
        }
    }
    for batch in &discovery.batches {
        let result = super::session_analytics_index::batch_stamp(batch)
            .and_then(|stamp| Ok((super::session_analytics_index::batch_key(batch)?, stamp)));
        match result {
            Ok((path, stamp)) => {
                observations
                    .get_mut(&batch.provider)
                    .expect("known provider")
                    .files
                    .insert(PathBuf::from(path), stamp);
            }
            Err(_) => {
                failed.insert(batch.provider.clone());
                discovery
                    .failures
                    .push(super::session_analytics_index::DiscoveryFailure {
                        provider: batch.provider.clone(),
                        code: "sourceDiscoveryFailed",
                    });
            }
        }
    }
    Observed {
        discovery,
        observations,
        failed,
        roots,
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
fn settings_failed(db: &Database) -> SessionSyncResult {
    use super::usage_health::{self, Channel, Outcome};
    let mut result = error_result("usage.sources.settingsUnreadable");
    let saved = (|| {
        let mut conn = crate::database::lock_conn!(db.conn);
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().timestamp_millis();
        for source in SOURCES {
            for channel in [Channel::Ledger]
                .into_iter()
                .chain(observes_analytics(source.provider).then_some(Channel::Analytics))
            {
                let attempt =
                    usage_health::begin_on(&tx, source.provider, source.source_id, channel, now)?;
                if !usage_health::finish_on(
                    &tx,
                    &attempt,
                    Outcome::Failed("usage.sources.settingsUnreadable"),
                    now,
                )? {
                    return Err(AppError::Database("usage.sources.staleAttempt".into()));
                }
            }
        }
        tx.commit()?;
        Ok::<_, AppError>(())
    })();
    if saved.is_err() {
        result.errors.push("usage.sources.healthWriteFailed".into());
    }
    result
}
fn refresh_local(
    db: &Database,
    force: bool,
    saved: &mut Option<Probe>,
    before_scan: impl FnOnce(),
) -> Option<SessionSyncResult> {
    let roots = match FrozenRoots::capture() {
        Ok(roots) => roots,
        Err(_) => {
            *saved = None;
            return Some(settings_failed(db));
        }
    };
    let before_db = match DbRevision::read(db) {
        Ok(revision) => revision,
        Err(_) => return Some(error_result("usage.sync.revisionUnavailable")),
    };
    // Capture observations before parsing. A concurrent append must remain a
    // change next time, even when a native reader happens to see the new bytes.
    let observed = observe_sources(roots);
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
    before_scan();
    let (mut total, mut failed) = sync_selected_sources(db, &selected, &observed.roots);
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

type SourceSync = fn(&Database, &[PathBuf]) -> Result<SessionSyncResult, AppError>;
type ResolveRoots = fn() -> Result<Vec<PathBuf>, AppError>;
type ObserveFiles = fn(&[PathBuf]) -> Result<Vec<PathBuf>, AppError>;
struct Source {
    provider: &'static str,
    source_id: &'static str,
    name: &'static str,
    resolve: ResolveRoots,
    sync: SourceSync,
    observe: Option<ObserveFiles>,
}
const SOURCES: &[Source] = &[
    Source {
        provider: "openclaw",
        source_id: "openclaw_session",
        name: "OpenClaw",
        resolve: || {
            crate::session_manager::providers::openclaw::usage_root()
                .map(|root| vec![root])
                .map_err(|_| root_unavailable())
        },
        sync: |db, roots| session_usage_openclaw::sync_openclaw_usage_at(db, single_root(roots)?),
        observe: Some(|roots| {
            crate::session_manager::providers::openclaw::usage_files(single_root(roots)?)
                .map_err(|_| root_unavailable())
        }),
    },
    Source {
        provider: "claude",
        source_id: "session_log",
        name: "Claude",
        resolve: || Ok(vec![crate::config::get_claude_config_dir()]),
        sync: |db, roots| session_usage::sync_claude_session_logs_at(db, single_root(roots)?),
        observe: None,
    },
    Source {
        provider: "codex",
        source_id: "codex_session",
        name: "Codex",
        resolve: || Ok(vec![crate::codex_config::get_codex_config_dir()]),
        sync: |db, roots| session_usage_codex::sync_codex_usage_at(db, single_root(roots)?),
        observe: None,
    },
    Source {
        provider: "gemini",
        source_id: "gemini_session",
        name: "Gemini",
        resolve: || Ok(vec![crate::gemini_config::get_gemini_dir()]),
        sync: |db, roots| session_usage_gemini::sync_gemini_usage_at(db, single_root(roots)?),
        observe: None,
    },
    Source {
        provider: "opencode",
        source_id: "opencode_session",
        name: "OpenCode",
        resolve: || {
            Ok(vec![
                crate::session_manager::providers::opencode::get_opencode_base_dir(),
            ])
        },
        sync: |db, roots| session_usage_opencode::sync_opencode_usage_at(db, single_root(roots)?),
        observe: None,
    },
    Source {
        provider: "workbuddy",
        source_id: "workbuddy_session",
        name: "WorkBuddy",
        resolve: || {
            crate::session_manager::providers::workbuddy::session_root().map(|root| vec![root])
        },
        sync: |db, roots| session_usage_workbuddy::sync_workbuddy_usage_at(db, single_root(roots)?),
        observe: Some(|roots| session_usage_workbuddy::collect_files(single_root(roots)?)),
    },
    Source {
        provider: "dsh",
        source_id: "dsh_session",
        name: "dsh",
        resolve: || {
            crate::session_manager::providers::dsh::home_dir()
                .map(|root| vec![root])
                .map_err(|_| root_unavailable())
        },
        sync: |db, roots| session_usage_dsh::sync_dsh_usage_at(db, single_root(roots)?),
        observe: Some(|roots| {
            crate::session_manager::providers::dsh::usage_files(single_root(roots)?)
                .map(|files| files.into_iter().map(|(path, _)| path).collect())
                .map_err(|_| root_unavailable())
        }),
    },
    Source {
        provider: "pi",
        source_id: "pi_session",
        name: "Pi",
        resolve: || {
            crate::session_manager::providers::pi::usage_roots().map_err(|_| root_unavailable())
        },
        sync: session_usage_pi::sync_pi_usage_at,
        observe: Some(|roots| {
            crate::session_manager::providers::pi::usage_files(roots)
                .map_err(|_| root_unavailable())
        }),
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
    failure: Option<&'static str>,
    empty: bool,
    scan: Option<super::usage_query::SourceScan>,
) -> Result<(), AppError> {
    use super::usage_health::{finish_with_scan_on, Outcome};
    let mut conn = crate::database::lock_conn!(db.conn);
    let tx = conn.transaction()?;
    let outcome = if let Some(code) = failure {
        Outcome::Failed(code)
    } else if empty {
        Outcome::Partial("usage.sources.noFiles")
    } else if super::usage_source::has_complete_coverage_on(&tx, source.provider)? {
        Outcome::Ok
    } else {
        Outcome::Partial("usage.sources.incompleteCoverage")
    };
    if !finish_with_scan_on(
        &tx,
        attempt,
        outcome,
        chrono::Utc::now().timestamp_millis(),
        scan,
    )? {
        return Err(AppError::Database("usage.sources.staleAttempt".into()));
    }
    tx.commit()?;
    Ok(())
}

/// Every source in turn (one failing does not stop the others), then the
/// pending maintenance; failures are collected in `errors`.
#[cfg(test)]
fn sync_sources(db: &Database) -> SessionSyncResult {
    let roots = match FrozenRoots::capture() {
        Ok(roots) => roots,
        Err(_) => return settings_failed(db),
    };
    let selected = SOURCES.iter().map(|source| source.provider).collect();
    let (mut total, _) = sync_selected_sources(db, &selected, &roots);
    run_maintenance(db, &mut total);
    total
}
fn sync_selected_sources(
    db: &Database,
    selected: &BTreeSet<&str>,
    roots: &FrozenRoots,
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
        let root_failure = roots.get(source.provider).is_err();
        let (failure, empty, scan) = match roots
            .get(source.provider)
            .and_then(|roots| (source.sync)(db, roots))
        {
            Ok(result) => {
                let failed = !result.errors.is_empty();
                let empty = result.files_scanned == 0;
                let scan = (!failed).then_some(super::usage_query::SourceScan {
                    files_scanned: result.files_scanned,
                    imported: result.imported,
                    skipped: result.skipped,
                });
                total.imported += result.imported;
                total.skipped += result.skipped;
                total.files_scanned += result.files_scanned;
                total.errors.extend(result.errors);
                (failed.then_some("usage.sources.syncFailed"), empty, scan)
            }
            Err(e) => {
                total.errors.push(format!("{} 同步失败: {e}", source.name));
                (
                    Some(if root_failure {
                        "usage.sources.rootUnavailable"
                    } else {
                        "usage.sources.syncFailed"
                    }),
                    false,
                    None,
                )
            }
        };
        if failure.is_some() {
            failed_providers.insert(source.provider.into());
        }
        if let Some(attempt) = attempt {
            if finish_source(db, source, &attempt, failure, empty, scan).is_err() {
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

    fn observation(db: &Database, tool: &str) -> serde_json::Value {
        let conn = db.conn.lock().unwrap();
        serde_json::to_value(
            super::super::usage_health::source_health_on(&conn)
                .unwrap()
                .into_iter()
                .find(|source| source.tool_id == tool && !source.source_id.ends_with(":proxy"))
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    #[serial]
    fn source_observability_scan_counts_are_scoped_to_each_completed_source() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        workbuddy_partial_fixture(home.home(), false, false);
        dsh_fixture(home.home(), "obs-dsh", "session.jsonl", 40);
        write(&home.home().join(".pi/agent/sessions/project/obs.jsonl"), &[
            serde_json::json!({"type":"session","version":3,"id":"obs-pi","timestamp":"2026-10-15T00:00:00Z","cwd":"/fixture"}),
            serde_json::json!({"type":"message","id":"a1","parentId":null,"timestamp":"2026-10-15T00:00:01Z","message":{"role":"assistant","model":"fixture","timestamp":1792022401000_i64,"usage":{"input":4,"output":2,"cacheRead":0,"cacheWrite":0}}}),
        ].map(|row|row.to_string()).join("\n"));
        let db = Database::memory().unwrap();
        let result = sync_sources(&db);
        assert!(result.errors.is_empty(), "{result:?}");
        for source in SOURCES {
            let row = observation(&db, source.provider);
            let scan = row
                .get("lastScan")
                .expect("lastScan must be an explicit nullable field");
            let has_fixture = matches!(
                source.provider,
                "claude" | "codex" | "gemini" | "opencode" | "workbuddy" | "dsh" | "pi"
            );
            assert_eq!(
                scan["imported"],
                u64::from(has_fixture),
                "{}: {row}",
                source.provider
            );
            assert_eq!(scan["skipped"], 0, "{}: {row}", source.provider);
            assert_eq!(
                scan["filesScanned"].as_u64().is_some_and(|n| n > 0),
                has_fixture,
                "{}: {row}",
                source.provider
            );
        }
    }

    #[test]
    #[serial]
    fn source_observability_failed_attempt_clears_scan_but_keeps_success_time() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        let db = Database::memory().unwrap();
        assert!(sync_sources(&db).errors.is_empty());
        let succeeded = status(&db, "gemini").last_success_at_ms;
        assert!(succeeded.is_some());
        write(&home.home().join(".gemini/tmp/h/chats/session-1.json"), "{");
        assert!(!sync_sources(&db).errors.is_empty());
        let failed = observation(&db, "gemini");
        assert_eq!(status(&db, "gemini").last_success_at_ms, succeeded);
        assert_eq!(failed.get("lastScan"), Some(&serde_json::Value::Null));
        assert!(observation(&db, "claude")["lastScan"].is_object());
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

    fn dsh_fixture(home: &Path, sid: &str, name: &str, input: u64) -> std::path::PathBuf {
        let path = home.join(".dsh/sessions/project").join(sid).join(name);
        let rows = [
            serde_json::json!({"type":"session","version":0,"id":sid,"createdAt":1792022400000_i64,"delegationDepth":0}),
            serde_json::json!({"type":"assistant/message","seq":0,"time":1792022400123_i64,"surfaceOp":"append",
                "data":{"message":{"id":"m","role":"assistant","source":{"kind":"model","provider":"fixture","model":"native"}},
                    "usage":{"inputTokens":input,"outputTokens":2,"cacheReadTokens":0,"cacheWriteTokens":0}}}),
        ];
        write(&path, &(rows.map(|row| row.to_string()).join("\n") + "\n"));
        path
    }

    fn dsh_input(db: &Database) -> u64 {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COALESCE(SUM(input_tokens),0) FROM proxy_request_logs WHERE app_type='dsh'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn pi_fixture(home: &Path, sid: &str, name: &str, input: u64) -> std::path::PathBuf {
        let path = home.join(".pi/agent/sessions/project").join(name);
        let rows = [
            serde_json::json!({"type":"session","version":3,"id":sid,"timestamp":"2026-10-15T00:00:00Z","cwd":"/fixture/project"}),
            serde_json::json!({"type":"message","id":"a1","parentId":null,"timestamp":"2026-10-15T00:00:01Z","message":{"role":"assistant","model":"fixture","timestamp":1792022401000_i64,"usage":{"input":input,"output":2,"cacheRead":0,"cacheWrite":0}}}),
        ];
        write(&path, &(rows.map(|row| row.to_string()).join("\n") + "\n"));
        path
    }

    fn pi_input(db: &Database) -> u64 {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COALESCE(SUM(input_tokens),0) FROM proxy_request_logs WHERE app_type='pi'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn root_setting(settings: &mut crate::settings::AppSettings, provider: &str, root: &Path) {
        let value = Some(root.to_str().unwrap().to_owned());
        match provider {
            "claude" => settings.claude_config_dir = value,
            "codex" => settings.codex_config_dir = value,
            "gemini" => settings.gemini_config_dir = value,
            _ => panic!("fixture provider"),
        }
    }

    async fn root_is_frozen(provider: &'static str) {
        let home = sandbox();
        let known = home.home().join("known");
        write_one_call_per_source(&known);
        workbuddy_partial_fixture(&known, false, false);
        dsh_fixture(&known, "one", "session.jsonl", 40);
        if provider == "pi" {
            pi_fixture(&known, "pi-one", "session.jsonl", 40);
            std::env::set_var("PI_CODING_AGENT_DIR", known.join(".pi/agent"));
        }
        let mut settings = crate::settings::get_settings();
        for (tool, path) in [
            ("claude", ".claude"),
            ("codex", ".codex"),
            ("gemini", ".gemini"),
        ] {
            root_setting(&mut settings, tool, &known.join(path));
        }
        crate::settings::update_settings(settings).unwrap();
        // Native OpenCode state follows XDG_DATA_HOME, independently of its
        // configuration-directory override.
        std::env::set_var("XDG_DATA_HOME", known.join(".local/share"));
        std::env::set_var("WORKBUDDY_CONFIG_DIR", known.join(".workbuddy"));
        std::env::set_var("DSH_HOME", known.join(".dsh"));
        let next_root = home.home().join("changed-empty-root");
        std::fs::create_dir_all(&next_root).unwrap();
        let (start_tx, start_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            start_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            let _io = crate::config_io::lock();
            let result = match provider {
                "opencode" => {
                    std::env::set_var("XDG_DATA_HOME", next_root);
                    Ok(())
                }
                "workbuddy" => {
                    std::env::set_var("WORKBUDDY_CONFIG_DIR", next_root);
                    Ok(())
                }
                "dsh" => {
                    std::env::set_var("DSH_HOME", next_root);
                    Ok(())
                }
                "pi" => {
                    std::env::set_var("PI_CODING_AGENT_DIR", next_root);
                    Ok(())
                }
                _ => crate::settings::mutate_settings(|s| root_setting(s, provider, &next_root)),
            };
            let _ = done_tx.send(result.is_ok());
            result
        });
        let config_was_available = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observation = Arc::clone(&config_was_available);
        let sync = UsageSync::new();
        *sync.shared.before_scan.lock().unwrap() = Some(Box::new(move || {
            start_tx.send(()).unwrap();
            observation.store(
                done_rx
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap_or(false),
                Ordering::Relaxed,
            );
        }));
        let db = Arc::new(Database::memory().unwrap());
        let result = sync.run(&db, false).await;
        writer.join().unwrap().unwrap();
        assert!(
            config_was_available.load(Ordering::Relaxed),
            "a native scan must not hold the configuration IO barrier"
        );
        let result = result.unwrap().unwrap();
        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(
            result.imported,
            if provider == "pi" { 7 } else { 6 },
            "{provider} re-resolved a changed root after the observation boundary"
        );
        let count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM proxy_request_logs WHERE app_type=?1",
                [provider],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    #[serial]
    async fn source_roots_claude_are_frozen_before_native_scan() {
        root_is_frozen("claude").await;
    }
    #[tokio::test]
    #[serial]
    async fn source_roots_codex_are_frozen_before_native_scan() {
        root_is_frozen("codex").await;
    }
    #[tokio::test]
    #[serial]
    async fn source_roots_gemini_are_frozen_before_native_scan() {
        root_is_frozen("gemini").await;
    }
    #[tokio::test]
    #[serial]
    async fn source_roots_opencode_are_frozen_before_native_scan() {
        root_is_frozen("opencode").await;
    }
    #[tokio::test]
    #[serial]
    async fn source_roots_workbuddy_are_frozen_before_native_scan() {
        root_is_frozen("workbuddy").await;
    }
    #[tokio::test]
    #[serial]
    async fn source_roots_dsh_are_frozen_before_native_scan() {
        root_is_frozen("dsh").await;
    }

    #[tokio::test]
    #[serial]
    async fn source_pi_roots_are_frozen_before_native_scan() {
        root_is_frozen("pi").await;
    }

    #[tokio::test]
    #[serial]
    async fn source_pi_modified_file_wakes_warm_refresh_without_reparsing_unchanged_body() {
        let home = sandbox();
        pi_fixture(home.home(), "pi-one", "one.jsonl", 40);
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        assert_eq!(sync.run(&db, false).await.unwrap().unwrap().imported, 1);
        let reads = super::super::usage_source::body_reads_under(home.home());
        assert!(sync.run(&db, false).await.unwrap().is_none());
        assert_eq!(
            super::super::usage_source::body_reads_under(home.home()),
            reads
        );
        pi_fixture(home.home(), "pi-one", "one.jsonl", 41);
        let changed = sync
            .run(&db, false)
            .await
            .unwrap()
            .expect("Pi changed stamp must wake a warm scan");
        assert!(changed.errors.is_empty(), "{changed:?}");
        assert_eq!(pi_input(&db), 41);
    }

    #[tokio::test]
    #[serial]
    async fn source_pi_new_file_wakes_warm_refresh() {
        let home = sandbox();
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        sync.run(&db, false).await.unwrap();
        pi_fixture(home.home(), "pi-new", "new.jsonl", 7);
        let changed = sync
            .run(&db, false)
            .await
            .unwrap()
            .expect("new Pi native file must wake a warm scan");
        assert!(changed.errors.is_empty(), "{changed:?}");
        assert_eq!(pi_input(&db), 7);
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_dsh_new_native_file_wakes_the_warm_probe() {
        let home = sandbox();
        dsh_fixture(home.home(), "one", "session.jsonl", 40);
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        assert_eq!(sync.run(&db, false).await.unwrap().unwrap().imported, 1);
        dsh_fixture(home.home(), "two", "session.jsonl", 20);
        let next = sync
            .run(&db, false)
            .await
            .unwrap()
            .expect("new dsh source was hidden by the warm probe");
        assert_eq!(next.imported, 1);
        assert_eq!(dsh_input(&db), 60);
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_dsh_native_revision_wakes_the_warm_probe() {
        let home = sandbox();
        dsh_fixture(home.home(), "one", "session.jsonl", 40);
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        sync.run(&db, false).await.unwrap();
        dsh_fixture(home.home(), "one", "session.jsonl", 80);
        let next = sync
            .run(&db, false)
            .await
            .unwrap()
            .expect("changed dsh source was hidden by the warm probe");
        assert_eq!(next.imported, 1);
        assert_eq!(dsh_input(&db), 80);
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_dsh_removed_highest_generation_is_observed_without_rollback() {
        let home = sandbox();
        dsh_fixture(home.home(), "one", "session.jsonl", 40);
        let highest = dsh_fixture(home.home(), "one", "session.v1.jsonl", 80);
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        sync.run(&db, false).await.unwrap();
        assert_eq!(dsh_input(&db), 80);
        std::fs::remove_file(highest).unwrap();
        let next = sync
            .run(&db, false)
            .await
            .unwrap()
            .expect("removed generation was hidden by the warm probe");
        assert!(next.errors.is_empty(), "{next:?}");
        assert_eq!(
            dsh_input(&db),
            80,
            "the surviving lower generation must not replace native order"
        );
        assert_eq!(status(&db, "dsh").status, "partial");
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_unreadable_settings_never_scan_default_roots_or_hide_the_failure() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        workbuddy_partial_fixture(home.home(), false, false);
        dsh_fixture(home.home(), "one", "session.jsonl", 40);
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        assert_eq!(
            sync.run(&db, false).await.unwrap().unwrap().imported,
            6,
            "all native fixtures must first be importable"
        );
        let before = super::super::usage_source::body_reads_under(home.home());
        let cached = super::super::session_analytics_index::read_input(&db)
            .unwrap()
            .facts
            .len();
        write(
            &home.home().join(".claude/projects/extra/new.jsonl"),
            &claude_call("new-default"),
        );
        let settings = home.home().join(".ofox-desktop/settings.json");
        std::fs::create_dir_all(&settings).unwrap();
        struct UnreadableFixture(std::path::PathBuf);
        impl Drop for UnreadableFixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _settings_fixture = UnreadableFixture(settings.clone());
        crate::settings::reload_settings().unwrap();
        assert_eq!(
            crate::settings::settings_health().state,
            crate::settings::SettingsHealthState::Unreadable
        );
        let result = sync
            .run(&db, false)
            .await
            .unwrap()
            .expect("unknown settings must not be hidden as unchanged");
        assert_eq!(
            result.imported, 0,
            "unreadable settings are not permission to scan the default roots"
        );
        assert!(!result.errors.is_empty());
        assert!(!format!("{result:?}").contains(&home.home().to_string_lossy().to_string()));
        assert_eq!(
            super::super::usage_source::body_reads_under(home.home()),
            before
        );
        assert_eq!(
            super::super::session_analytics_index::read_input(&db)
                .unwrap()
                .facts
                .len(),
            cached
        );
        for source in SOURCES {
            let health = status(&db, source.provider);
            assert_eq!(
                health.status, "failed",
                "{} must retain a typed root-resolution failure",
                source.provider
            );
            assert_eq!(
                health.error_code.as_deref(),
                Some("usage.sources.settingsUnreadable")
            );
        }
        std::fs::remove_dir(&settings).unwrap();
        crate::settings::reload_settings().unwrap();
        assert_eq!(
            sync.run(&db, false).await.unwrap().unwrap().imported,
            1,
            "a repaired root must retry even in the same minute"
        );
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
    async fn source_refresh_workbuddy_pi_bridge_preserves_project_and_all_spending() {
        let home = sandbox();
        let wb = workbuddy_partial_fixture(home.home(), false, false);
        let body = std::fs::read_to_string(&wb).unwrap();
        let mut row: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        row["cwd"] = serde_json::json!("/fixture-project");
        row["content"] = serde_json::json!("safe synthetic answer");
        write(&wb, &(row.to_string() + "\n"));
        let pi = home.home().join(".pi/agent/sessions/project/native.jsonl");
        let title_prefix = "Pi bridge fixture title: ";
        let title_text = format!(
            "{title_prefix}{}",
            "x".repeat(crate::session_manager::title::TITLE_MAX_CHARS - title_prefix.len())
        );
        let user_body_canary = "PI_PRIVATE_USER_BODY_CANARY_26d967f9";
        let user_body = format!("{title_text}\n{user_body_canary}");
        write(&pi, &[
            serde_json::json!({"type":"session","version":3,"id":"native-pi","cwd":"/fixture-project","timestamp":"2026-10-15T00:00:00Z"}),
            serde_json::json!({"type":"message","id":"u1","parentId":null,"timestamp":"2026-10-15T00:00:00Z","message":{"role":"user","content":user_body,"timestamp":1792022400000_i64}}),
            serde_json::json!({"type":"message","id":"a1","parentId":"u1","timestamp":"2026-10-15T00:00:01Z","message":{"role":"assistant","content":"old branch","model":"fixture","timestamp":1792022401000_i64,"usage":{"input":4,"output":2,"cacheRead":0,"cacheWrite":0}}}),
            serde_json::json!({"type":"message","id":"a2","parentId":"u1","timestamp":"2026-10-15T00:00:02Z","message":{"role":"assistant","content":"current branch","model":"fixture","timestamp":1792022402000_i64,"usage":{"input":7,"output":3,"cacheRead":0,"cacheWrite":0}}}),
        ].map(|row|row.to_string()).join("\n"));
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        let result = sync.run(&db, false).await.unwrap().unwrap();
        assert!(result.errors.is_empty(), "{result:?}");
        let input = super::super::session_analytics_index::read_input(&db).unwrap();
        assert_eq!(
            input.sessions.len(),
            2,
            "both native tools must enter the same cached analysis"
        );
        assert_eq!(
            input.facts.len(),
            3,
            "abandoned Pi spending must remain included"
        );
        assert!(input.sessions.iter().all(|session| matches!(
            &session.identity.project,
            super::super::session_analytics_core::Project::Known(_)
        )));
        assert_eq!(
            input
                .messages
                .iter()
                .filter(|m| m.session_key
                    == super::super::session_analytics_index::key("pi", "native-pi"))
                .count(),
            2,
            "active branch alone supplies readable conversation activity"
        );
        let before = super::super::usage_source::body_reads_under(home.home());
        assert!(sync.run(&db, false).await.unwrap().is_none());
        assert_eq!(
            super::super::usage_source::body_reads_under(home.home()),
            before
        );
        let pi_title = &input
            .sessions
            .iter()
            .find(|session| session.identity.tool_id == "pi")
            .unwrap()
            .title;
        assert_eq!(pi_title, &format!("{title_text}…"));
        assert_eq!(
            pi_title.chars().count(),
            crate::session_manager::title::TITLE_MAX_CHARS + 1,
            "the first prompt is allowed only as bounded title metadata"
        );
        let encoded = serde_json::to_string(&input).unwrap();
        let cached = {
            let conn = db.conn.lock().unwrap();
            let mut statement = conn
                .prepare("SELECT meta_json FROM session_meta_cache WHERE provider_id IN ('workbuddy','pi') AND meta_json IS NOT NULL")
                .unwrap();
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.join("\n")
        };
        for payload in [&encoded, &cached] {
            assert!(payload.contains(title_prefix));
            for private_body in [
                user_body_canary,
                "current branch",
                "old branch",
                "safe synthetic answer",
            ] {
                assert!(
                    !payload.contains(private_body),
                    "a full message body must not survive in analysis or its raw cache"
                );
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn source_refresh_native_batch_keeps_missing_session_history_and_retries_failures() {
        let home = sandbox();
        let path = workbuddy_partial_fixture(home.home(), false, false);
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        let first = sync.run(&db, false).await.unwrap().unwrap();
        assert!(first.errors.is_empty(), "{first:?}");
        let input = super::super::session_analytics_index::read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        std::fs::remove_file(&path).unwrap();
        let next = sync.run(&db, false).await.unwrap().unwrap();
        assert!(next.errors.is_empty(), "{next:?}");
        let input = super::super::session_analytics_index::read_input(&db).unwrap();
        assert_eq!(
            input.facts.len(),
            1,
            "a missing native file does not erase previously observed spending"
        );
        assert!(!input.sessions[0].source_available);
        assert!(input.sessions[0].manager_key.is_none());
        assert!(input
            .source_reasons
            .iter()
            .any(|reason| reason.reason == "sourceMissing"));
        workbuddy_partial_fixture(home.home(), false, false);
        sync.run(&db, false).await.unwrap().unwrap();
        let restored = super::super::session_analytics_index::read_input(&db).unwrap();
        assert!(restored.sessions[0].source_available);
        assert_eq!(restored.facts.len(), 1);
        let native_root = home.home().join(".workbuddy/projects");
        std::fs::remove_dir_all(&native_root).unwrap();
        std::fs::write(native_root, "not a source directory").unwrap();
        let failed = sync.run(&db, false).await.unwrap().unwrap();
        assert!(!failed.errors.is_empty());
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
    async fn source_refresh_six_hundred_mixed_native_bridges_stay_warm_without_body_reads() {
        use serde_json::json;
        let home = sandbox();
        for i in 0..150 {
            let sid = format!("mixed-{i}");
            write(&home.home().join(format!(".workbuddy/projects/project/{sid}.jsonl")),&(json!({"type":"message","role":"assistant","id":"a","sessionId":sid,"cwd":"/fixture/project","timestamp":1792022400000_i64,"content":"synthetic answer","providerData":{"messageId":"a","model":"native","usage":{"requests":1,"inputTokens":3,"outputTokens":2}}}).to_string()+"\n"));
            pi_fixture(home.home(), &sid, &format!("{sid}.jsonl"), 3);
            dsh_fixture(home.home(), &sid, "session.jsonl", 3);
            write(&home.home().join(format!(".openclaw/agents/main/sessions/{sid}.jsonl")),&[
                json!({"type":"session","version":4,"id":sid,"cwd":"/fixture/project","timestamp":"2026-10-15T00:00:00Z"}),
                json!({"type":"message","id":"a","timestamp":"2026-10-15T00:00:01Z","message":{"role":"assistant","model":"native","usage":{"input":3,"output":2,"cacheRead":0,"cacheWrite":0}}}),
            ].map(|row|row.to_string()).join("\n"));
        }
        let db = Arc::new(Database::memory().unwrap());
        let sync = UsageSync::new();
        let result = sync.run(&db, false).await.unwrap().unwrap();
        assert!(result.errors.is_empty(), "{result:?}");
        let input = super::super::session_analytics_index::read_input(&db).unwrap();
        assert_eq!(input.sessions.len(), 600);
        assert_eq!(input.facts.len(), 600);
        let before = super::super::usage_source::body_reads_under(home.home());
        assert!(sync.run(&db, false).await.unwrap().is_none());
        assert_eq!(
            super::super::usage_source::body_reads_under(home.home()),
            before
        );
        // Manual refresh still verifies the frozen roster, without rereading
        // unchanged native bodies to rebuild either ledger or analytics.
        let manual = sync.run(&db, true).await.unwrap().unwrap();
        assert!(manual.errors.is_empty(), "{manual:?}");
        assert_eq!(
            super::super::usage_source::body_reads_under(home.home()),
            before
        );
        assert_eq!(
            super::super::session_analytics_index::read_input(&db)
                .unwrap()
                .facts
                .len(),
            600
        );
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

    #[test]
    #[serial]
    fn dsh_registered_partial_prefix_does_not_block_other_sources_maintenance() {
        let home = sandbox();
        write_one_call_per_source(home.home());
        let path = home.home().join(".dsh/sessions/_no-cwd/s/session.jsonl");
        let header = serde_json::json!({"type":"session","version":0,"id":"s","createdAt":1_792_053_987_000_i64,"delegationDepth":0});
        let call = serde_json::json!({"type":"assistant/message","seq":0,"time":1_792_053_987_262_i64,
            "data":{"message":{"id":"native-message","role":"assistant","source":{"kind":"model"}},"usage":{"inputTokens":1,"outputTokens":2}}});
        write(
            &path,
            &format!("{header}\n{call}\n{{\"type\":\"assistant/message\""),
        );
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
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM proxy_request_logs WHERE app_type='dsh'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "dsh must be registered with the coordinator: {result:?}"
        );
        assert!(
            result.errors.is_empty(),
            "typed coverage partial is not transport failure: {result:?}"
        );
        let flag: String = conn
            .query_row(
                "SELECT value FROM maintenance_flags WHERE key='recompute_claude_costs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(flag, "done");
        let offset: i64 = conn
            .query_row(
                "SELECT last_line_offset FROM session_log_sync WHERE file_path=?1",
                [path.to_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(offset, 2);
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
