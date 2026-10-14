//! The login-shell snapshot as a value: seeded from the copy kept on disk, replaced
//! when a fresh probe succeeds, and probed by one caller at a time.
//!
//! Asking a login shell takes about a second, so the last successful answer is kept in
//! a small file next to the settings. A launch that finds it uses it at once and
//! refreshes in the background; only a launch without one (the first, or after the file
//! is gone) waits for the probe. A probe that fails never replaces a good value and is
//! never kept as an empty answer.

use std::future::Future;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{AgentTool, EnvMap};
use crate::config::{atomic_write_with, AtomicOpts};
use crate::error::AppError;

const CACHE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    vars: EnvMap,
}

/// The agent variables kept at `path`. A missing file is the normal first launch; a
/// file that cannot be read or parsed is logged and treated the same way.
pub(super) fn read_cache(path: &Path) -> Option<EnvMap> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!("[agent_env] cannot read {}: {error}", path.display());
            return None;
        }
    };
    match serde_json::from_str::<CacheFile>(&text) {
        Ok(file) if file.version == CACHE_VERSION => Some(only_agent_vars(file.vars)),
        Ok(file) => {
            log::warn!(
                "[agent_env] ignoring {}: unknown version {}",
                path.display(),
                file.version
            );
            None
        }
        Err(error) => {
            log::warn!("[agent_env] ignoring {}: {error}", path.display());
            None
        }
    }
}

fn only_agent_vars(vars: EnvMap) -> EnvMap {
    vars.into_iter()
        .filter(|(name, value)| {
            AgentTool::ALL.iter().any(|tool| tool.env_var() == name) && !value.trim().is_empty()
        })
        .collect()
}

pub(super) fn write_cache(path: &Path, env: &EnvMap) -> Result<(), AppError> {
    let file = CacheFile {
        version: CACHE_VERSION,
        vars: env.clone(),
    };
    let json =
        serde_json::to_string_pretty(&file).map_err(|source| AppError::JsonSerialize { source })?;
    atomic_write_with(path, json.as_bytes(), AtomicOpts { mode: Some(0o600) })
}

/// What a refresh did.
#[derive(Debug)]
pub(super) enum Refresh {
    /// The probe succeeded; `previous` is the value it replaced.
    Updated { previous: Option<Arc<EnvMap>> },
    /// The probe failed; the earlier value, if any, is untouched.
    Failed,
}

pub(super) struct Snapshot {
    /// `None` until a value (cached or fresh) is available.
    current: RwLock<Option<Arc<EnvMap>>>,
    /// Held while probing, so callers share one probe; holds when the last one failed.
    probe_gate: Mutex<Option<Instant>>,
    /// How long after a failed probe [`Snapshot::ready`] does not try again.
    retry_after: Duration,
}

impl Snapshot {
    pub(super) const fn new(retry_after: Duration) -> Self {
        Self {
            current: RwLock::new(None),
            probe_gate: Mutex::const_new(None),
            retry_after,
        }
    }

    pub(super) fn get(&self) -> Option<Arc<EnvMap>> {
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Replace the value; returns the one it replaced.
    pub(super) fn set(&self, env: Arc<EnvMap>) -> Option<Arc<EnvMap>> {
        self.current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .replace(env)
    }

    /// Use the copy kept at `cache` if there is no value yet. True when a value is
    /// available afterwards.
    pub(super) fn seed_from_cache(&self, cache: &Path) -> bool {
        if self.get().is_some() {
            return true;
        }
        match read_cache(cache) {
            Some(env) => {
                self.set(Arc::new(env));
                true
            }
            None => false,
        }
    }

    /// Return once a value is available: at once with a cached or earlier one, else
    /// after one probe. A probe that fails leaves no value (not an empty one) and is not
    /// repeated for `retry_after`, so a broken shell does not stall every caller.
    pub(super) async fn ready<F>(&self, cache: &Path, probe: impl FnOnce() -> F)
    where
        F: Future<Output = Option<EnvMap>>,
    {
        if self.get().is_some() {
            return;
        }
        let mut failed_at = self.probe_gate.lock().await;
        if self.get().is_some() {
            return;
        }
        if failed_at.is_some_and(|at| at.elapsed() < self.retry_after) {
            return;
        }
        match probe().await {
            Some(env) => {
                *failed_at = None;
                self.install(env, cache);
            }
            None => *failed_at = Some(Instant::now()),
        }
    }

    /// Probe again and, on success, replace the value and the copy on disk.
    pub(super) async fn refresh<F>(&self, cache: &Path, probe: impl FnOnce() -> F) -> Refresh
    where
        F: Future<Output = Option<EnvMap>>,
    {
        let mut failed_at = self.probe_gate.lock().await;
        match probe().await {
            Some(env) => {
                *failed_at = None;
                Refresh::Updated {
                    previous: self.install(env, cache),
                }
            }
            None => {
                *failed_at = Some(Instant::now());
                Refresh::Failed
            }
        }
    }

    /// Make `env` the value, and keep it on disk unless it is what is already there.
    fn install(&self, env: EnvMap, cache: &Path) -> Option<Arc<EnvMap>> {
        let unchanged = self.get().is_some_and(|current| *current == env);
        if !unchanged {
            if let Err(error) = write_cache(cache, &env) {
                log::warn!("[agent_env] could not save the snapshot: {error}");
            }
        }
        self.set(Arc::new(env))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn env(pairs: &[(&str, &str)]) -> EnvMap {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn state() -> Snapshot {
        Snapshot::new(Duration::ZERO)
    }

    fn cache_in(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("agent-env.json")
    }

    /// A probe that takes `delay`, counts its runs, and answers `result`.
    fn probe(
        runs: &Arc<AtomicUsize>,
        delay: Duration,
        result: Option<EnvMap>,
    ) -> impl FnOnce() -> std::pin::Pin<Box<dyn Future<Output = Option<EnvMap>>>> {
        let runs = Arc::clone(runs);
        move || {
            runs.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                result
            })
        }
    }

    #[test]
    fn the_cache_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        let saved = env(&[("CODEX_HOME", "/data/codex")]);

        write_cache(&path, &saved).unwrap();

        assert_eq!(read_cache(&path), Some(saved));
    }

    #[test]
    fn an_empty_snapshot_is_a_cache_too() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);

        write_cache(&path, &EnvMap::new()).unwrap();

        assert_eq!(read_cache(&path), Some(EnvMap::new()));
    }

    #[test]
    fn a_missing_or_foreign_cache_is_no_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        assert_eq!(read_cache(&path), None, "missing");

        std::fs::write(&path, "not json").unwrap();
        assert_eq!(read_cache(&path), None, "garbage");

        std::fs::write(&path, r#"{"version":99,"vars":{"CODEX_HOME":"/x"}}"#).unwrap();
        assert_eq!(read_cache(&path), None, "unknown version");
    }

    #[test]
    fn only_non_empty_agent_variables_are_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        std::fs::write(
            &path,
            r#"{"version":1,"vars":{"CODEX_HOME":"/c","CLAUDE_CONFIG_DIR":" ","PATH":"/bin"}}"#,
        )
        .unwrap();

        assert_eq!(read_cache(&path), Some(env(&[("CODEX_HOME", "/c")])));
    }

    #[cfg(unix)]
    #[test]
    fn the_cache_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);

        write_cache(&path, &env(&[("CODEX_HOME", "/c")])).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn a_cached_snapshot_is_used_at_once_without_waiting_for_a_slow_probe() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        write_cache(&path, &env(&[("CODEX_HOME", "/cached")])).unwrap();
        let (state, runs) = (state(), Arc::new(AtomicUsize::new(0)));

        assert!(state.seed_from_cache(&path));
        let started = Instant::now();
        state
            .ready(
                &path,
                probe(
                    &runs,
                    Duration::from_secs(3),
                    Some(env(&[("CODEX_HOME", "/slow")])),
                ),
            )
            .await;

        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(runs.load(Ordering::SeqCst), 0, "no probe was needed");
        assert_eq!(*state.get().unwrap(), env(&[("CODEX_HOME", "/cached")]));
    }

    #[tokio::test]
    async fn without_a_cache_the_first_ready_waits_for_the_probe_and_keeps_its_answer() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        let (state, runs) = (state(), Arc::new(AtomicUsize::new(0)));
        let answer = env(&[("CODEX_HOME", "/fresh")]);

        assert!(!state.seed_from_cache(&path));
        let started = Instant::now();
        state
            .ready(
                &path,
                probe(&runs, Duration::from_millis(300), Some(answer.clone())),
            )
            .await;

        assert!(started.elapsed() >= Duration::from_millis(300));
        assert_eq!(*state.get().unwrap(), answer);
        assert_eq!(read_cache(&path), Some(answer), "kept for the next launch");
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_probe() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        let (state, runs) = (state(), Arc::new(AtomicUsize::new(0)));
        let answer = env(&[("CODEX_HOME", "/fresh")]);

        tokio::join!(
            state.ready(
                &path,
                probe(&runs, Duration::from_millis(200), Some(answer.clone()))
            ),
            state.ready(
                &path,
                probe(&runs, Duration::from_millis(200), Some(answer.clone()))
            ),
            state.ready(
                &path,
                probe(&runs, Duration::from_millis(200), Some(answer.clone()))
            ),
        );

        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(*state.get().unwrap(), answer);
    }

    #[tokio::test]
    async fn a_failed_first_probe_is_not_kept_as_an_empty_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        let (state, runs) = (state(), Arc::new(AtomicUsize::new(0)));

        state.ready(&path, probe(&runs, Duration::ZERO, None)).await;

        assert!(state.get().is_none(), "no value, not an empty one");
        assert!(!path.exists(), "nothing is cached");
    }

    #[tokio::test]
    async fn a_later_ready_probes_again_after_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        let (state, runs) = (state(), Arc::new(AtomicUsize::new(0)));
        let answer = env(&[("CLAUDE_CONFIG_DIR", "/late")]);

        state.ready(&path, probe(&runs, Duration::ZERO, None)).await;
        state
            .ready(&path, probe(&runs, Duration::ZERO, Some(answer.clone())))
            .await;

        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert_eq!(*state.get().unwrap(), answer);
    }

    #[tokio::test]
    async fn a_failed_probe_is_not_repeated_within_the_retry_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        let state = Snapshot::new(Duration::from_secs(3600));
        let runs = Arc::new(AtomicUsize::new(0));

        for _ in 0..3 {
            state.ready(&path, probe(&runs, Duration::ZERO, None)).await;
        }

        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_refresh_replaces_the_value_and_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        let old = env(&[("CODEX_HOME", "/old")]);
        let new = env(&[("CODEX_HOME", "/new")]);
        write_cache(&path, &old).unwrap();
        let (state, runs) = (state(), Arc::new(AtomicUsize::new(0)));
        assert!(state.seed_from_cache(&path));

        let outcome = state
            .refresh(&path, probe(&runs, Duration::ZERO, Some(new.clone())))
            .await;

        match outcome {
            Refresh::Updated { previous } => assert_eq!(*previous.unwrap(), old),
            Refresh::Failed => panic!("the probe succeeded"),
        }
        assert_eq!(*state.get().unwrap(), new);
        assert_eq!(read_cache(&path), Some(new));
    }

    #[tokio::test]
    async fn a_failed_refresh_keeps_the_cached_value_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        let old = env(&[("CODEX_HOME", "/old")]);
        write_cache(&path, &old).unwrap();
        let (state, runs) = (state(), Arc::new(AtomicUsize::new(0)));
        assert!(state.seed_from_cache(&path));

        let outcome = state
            .refresh(&path, probe(&runs, Duration::ZERO, None))
            .await;

        assert!(matches!(outcome, Refresh::Failed));
        assert_eq!(*state.get().unwrap(), old);
        assert_eq!(read_cache(&path), Some(old));
    }

    #[tokio::test]
    async fn a_refresh_that_finds_the_same_value_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_in(&dir);
        let same = env(&[("CODEX_HOME", "/same")]);
        write_cache(&path, &same).unwrap();
        let (state, runs) = (state(), Arc::new(AtomicUsize::new(0)));
        assert!(state.seed_from_cache(&path));
        std::fs::remove_file(&path).unwrap();

        state
            .refresh(&path, probe(&runs, Duration::ZERO, Some(same.clone())))
            .await;

        assert!(!path.exists(), "unchanged values are not rewritten");
        assert_eq!(*state.get().unwrap(), same);
    }
}
