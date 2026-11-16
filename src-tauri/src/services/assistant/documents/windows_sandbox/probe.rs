//! Numeric, per-candidate diagnostics for the embedded trusted probes only.
//! This observer never supplies an authorization or changes a runner result.
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    WorkerStart,
    OsVersion,
    CurrentUser,
    RuntimeCapability,
    CacheSource,
    CacheSourceDigest,
    CacheDescriptor,
    CacheLock,
    CacheOwnerOpen,
    CacheOwnerQuery,
    CacheOwnerMatch,
    CacheAcl,
    CacheDigest,
    CacheParent,
    CacheSourceRead,
    CacheStagingCreate,
    CacheStagingWrite,
    CacheStagingDigest,
    CachePublish,
    Admission,
    InputPaths,
    InputSnapshot,
    ProfileCreate,
    ProfileFolder,
    ProfileFolderDecode,
    ProfileFolderValidate,
    ProfilePin,
    ProfileLabelOpen,
    ProfileLabelDescriptor,
    ProfileLabelSet,
    TaskCreate,
    TaskCopy,
    TaskHome,
    Command,
    Environment,
    Pipes,
    Attributes,
    JobCreate,
    JobLimits,
    ProcessCreate,
    JobAssign,
    TokenOpen,
    TokenContainer,
    TokenPackage,
    TokenCapabilities,
    TokenIntegrity,
    TokenMitigations,
    Resume,
    ProcessWait,
    ProcessStatus,
    JobStop,
    JobProcessWait,
    JobAccounting,
    StdoutReceive,
    StderrReceive,
    Copyback,
    TaskCleanup,
    ProfileCleanup,
    Complete,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Receive {
    Ok,
    Timeout,
    Disconnected,
}

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Evidence {
    pub os_major: Option<u32>,
    pub os_build: Option<u32>,
    pub native_status: Option<u32>,
    pub capability_count: Option<u32>,
    pub capability_array_present: Option<bool>,
    pub cache_hit: Option<bool>,
    pub cache_owner_matches: Option<bool>,
    pub profile_folder_is_ac: Option<bool>,
    pub profile_folder_has_parent: Option<bool>,
    pub profile_pin_complete: Option<bool>,
    pub token_queries: Vec<TokenQuery>,
    pub app_container: Option<u32>,
    pub less_privileged: Option<u32>,
    pub package_matches: Option<bool>,
    pub capability_groups: Option<u32>,
    pub capability_matches: Option<bool>,
    pub integrity_subauthorities: Option<u8>,
    pub integrity_rid: Option<u32>,
    pub child_policy: Option<u32>,
    pub image_policy: Option<u32>,
    pub resume_result: Option<u32>,
    pub process_wait: Option<u32>,
    pub process_status: Option<u32>,
    pub timed_out: Option<bool>,
    pub cancelled: Option<bool>,
    pub job_process_wait: Option<u32>,
    pub job_active_processes: Option<u32>,
    pub stdout_receive: Option<Receive>,
    pub stderr_receive: Option<Receive>,
    pub task_cleanup: Option<bool>,
    pub profile_cleanup: Option<bool>,
    pub worker_join_panicked: Option<bool>,
    pub worker_join_cancelled: Option<bool>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TokenQuery {
    pub kind: i32,
    pub required_bytes: u32,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Timing {
    stage: Phase,
    elapsed_ms: u128,
}

struct State {
    stage: Phase,
    stage_started: Instant,
    first_failure: Option<Phase>,
    completed: Vec<Timing>,
    evidence: Evidence,
}

/// Default is inert. Only the private fixed-probe orchestration allocates state.
#[derive(Clone, Default)]
pub(super) struct Observer(Option<Arc<Mutex<State>>>);

impl Observer {
    pub fn fixed_probe() -> Self {
        Self(Some(Arc::new(Mutex::new(State {
            stage: Phase::WorkerStart,
            stage_started: Instant::now(),
            first_failure: None,
            completed: Vec::new(),
            evidence: Evidence::default(),
        }))))
    }

    fn update(&self, update: impl FnOnce(&mut State)) {
        if let Some(state) = &self.0 {
            // A diagnostic cannot make the runner fail or replace its error.
            let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
            update(&mut state);
        }
    }

    pub fn enter(&self, stage: Phase) {
        self.update(|state| {
            if state.completed.len() < 128 {
                state.completed.push(Timing {
                    stage: state.stage,
                    elapsed_ms: state.stage_started.elapsed().as_millis(),
                });
            }
            state.stage = stage;
            state.stage_started = Instant::now();
        });
    }

    pub fn evidence(&self, update: impl FnOnce(&mut Evidence)) {
        self.update(|state| update(&mut state.evidence));
    }

    pub fn failed(&self) {
        self.update(|state| {
            state.first_failure.get_or_insert(state.stage);
        });
    }

    pub fn token_query(&self, kind: i32, required_bytes: u32) {
        self.evidence(|evidence| {
            if evidence.token_queries.len() < 16 {
                evidence.token_queries.push(TokenQuery {
                    kind,
                    required_bytes,
                });
            }
        });
    }

    pub fn snapshot(&self) -> Option<serde_json::Value> {
        self.0.as_ref().map(|state| {
            let state = state.lock().unwrap_or_else(|error| error.into_inner());
            serde_json::json!({
                "stage":state.stage, "firstFailure":state.first_failure,
                "stageElapsedMs":state.stage_started.elapsed().as_millis(),
                "completedStages":state.completed, "evidence":state.evidence
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_worker_failure_survives_both_cleanup_stages() {
        let observation = Observer::fixed_probe();
        observation.enter(Phase::TokenContainer);
        observation.evidence(|evidence| {
            evidence.app_container = Some(1);
            evidence.less_privileged = Some(0);
        });
        observation.failed();
        observation.enter(Phase::TaskCleanup);
        observation.evidence(|evidence| evidence.task_cleanup = Some(true));
        observation.enter(Phase::ProfileCleanup);
        observation.evidence(|evidence| evidence.profile_cleanup = Some(false));
        observation.failed();
        let report = observation.snapshot().unwrap();
        assert_eq!(report["stage"], "profile_cleanup");
        assert_eq!(report["firstFailure"], "token_container");
        assert_eq!(report["evidence"]["appContainer"], 1);
        assert_eq!(report["evidence"]["lessPrivileged"], 0);
        assert_eq!(report["evidence"]["taskCleanup"], true);
        assert_eq!(report["evidence"]["profileCleanup"], false);
    }

    #[test]
    fn ordinary_observer_is_inert_and_probe_storage_is_bounded() {
        let ordinary = Observer::default();
        ordinary.enter(Phase::TokenIntegrity);
        ordinary.failed();
        // The closure is not called at all for an ordinary model execution.
        ordinary.evidence(|_| panic!("ordinary runner allocated diagnostic state"));
        assert!(ordinary.snapshot().is_none());
        let observation = Observer::fixed_probe();
        for _ in 0..1000 {
            observation.enter(Phase::ProcessWait);
            observation.token_query(29, 4);
        }
        let report = observation.snapshot().unwrap();
        assert_eq!(report["completedStages"].as_array().unwrap().len(), 128);
        assert_eq!(
            report["evidence"]["tokenQueries"].as_array().unwrap().len(),
            16
        );
    }
}
