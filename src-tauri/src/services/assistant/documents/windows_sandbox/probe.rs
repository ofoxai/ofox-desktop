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
    TokenLpacDuplicate,
    TokenLpacDescriptors,
    TokenLpacWorld,
    TokenLpacAllPackages,
    TokenLpacCapability,
    TokenLpacForeign,
    TokenRegistryIsolation,
    #[cfg(test)]
    CdbHandshake,
    #[cfg(test)]
    CdbInitialHold,
    #[cfg(test)]
    CdbContinue,
    #[cfg(test)]
    CdbHold,
    #[cfg(test)]
    CdbStop,
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
    pub token_comparisons: Vec<TokenComparison>,
    pub lpac_access_proof: Option<LpacAccessProof>,
    pub registry_isolation: Vec<RegistryAccess>,
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
    pub child_pid: Option<u32>,
    pub primary_tid: Option<u32>,
    #[cfg(test)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_creation_filetime: Option<String>,
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
    #[cfg(test)]
    pub loader_diagnostic: Option<LoaderDiagnostic>,
}

#[cfg(test)]
#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LoaderDiagnostic {
    pub diagnostic_only: bool,
    pub stage: Option<&'static str>,
    pub config_confirmed: Option<bool>,
    pub before_flags: Option<u32>,
    pub after_flags: Option<u32>,
    pub pre_attach_suspend_previous: Option<u32>,
    pub pre_attach_paired_resume_previous: Option<u32>,
    pub release_probe_count: u32,
    pub release_suspend_previous: Option<u32>,
    pub release_paired_resume_previous: Option<u32>,
    pub suspend_previous: Option<u32>,
    pub paired_resume_previous: Option<u32>,
    pub host_resume_previous: Option<u32>,
    pub debugger_closed: Option<bool>,
    /// The traced launch's fixed configuration label and actual flags.
    pub launch_profile: Option<&'static str>,
    pub creation_flags: Option<u32>,
    /// CI-only Winsock catalog ACE: set before the launch, and whether every
    /// key of the subtree was proven back to its original DACL afterwards.
    pub winsock_grant_applied: Option<bool>,
    pub winsock_grant_restored: Option<bool>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TokenQuery {
    pub kind: i32,
    pub required_bytes: u32,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AccessCase {
    WorldOnly,
    AllApplicationPackages,
    RuntimeCapability,
    ForeignCapability,
}

/// Fixed registry keys checked against the actual suspended token's DACL
/// intersection. Names are a closed set; no path, SID or value leaves.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RegistryKey {
    UserEnvironment,
    UserSoftware,
    WinsockCatalog,
    /// HKLM itself: WSAStartup opens it before the catalog (trace 44231498872).
    MachineRoot,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RegistryAccess {
    pub key: RegistryKey,
    pub opened: bool,
    pub descriptor_read: bool,
    pub access_checked: bool,
    pub read_granted: Option<bool>,
}

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LpacAccessProof {
    pub duplicated: bool,
    pub checks: Vec<AccessObservation>,
    pub matched: Option<bool>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AccessObservation {
    pub case: AccessCase,
    pub api_success: bool,
    pub status: u32,
    pub access_status: Option<bool>,
    pub granted_access: Option<u32>,
    pub returned_privilege_bytes: Option<u32>,
    pub privilege_count: Option<u32>,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TokenApi {
    GetTokenInformation,
    NtQueryInformationToken,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TokenComparison {
    api: TokenApi,
    kind: i32,
    buffer_bytes: u32,
    success: bool,
    // HRESULT for GetTokenInformation, NTSTATUS for NtQueryInformationToken.
    status: u32,
    returned_bytes: u32,
    length_written: bool,
    buffer_written: bool,
    first_dword: Option<u32>,
}

impl TokenComparison {
    fn capture(
        api: TokenApi,
        kind: i32,
        buffer_bytes: u32,
        query: impl FnOnce(Option<&mut [u32; 2]>, u32, &mut u32) -> (bool, u32),
    ) -> Self {
        const BUFFER_SENTINELS: [u32; 2] = [0xa5a5_5a5a, 0x5a5a_a5a5];
        const LENGTH_SENTINEL: u32 = 0x5a5a_5a5a;
        let mut buffer = BUFFER_SENTINELS;
        let mut length = LENGTH_SENTINEL;
        let (success, status) = query(
            (buffer_bytes != 0).then_some(&mut buffer),
            buffer_bytes,
            &mut length,
        );
        Self {
            api,
            kind,
            buffer_bytes,
            success,
            status,
            returned_bytes: length,
            length_written: length != LENGTH_SENTINEL,
            buffer_written: buffer != BUFFER_SENTINELS,
            // A failed API call never makes any output-buffer value evidence.
            first_dword: if success && buffer_bytes != 0 {
                Some(buffer[0])
            } else {
                None
            },
        }
    }
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
    #[cfg(test)]
    loader_requested: bool,
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
            #[cfg(test)]
            loader_requested: false,
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

    #[cfg(test)]
    pub fn request_loader(&self) {
        self.update(|state| state.loader_requested = true);
    }
    #[cfg(test)]
    pub fn loader_requested(&self) -> bool {
        let mut requested = false;
        self.update(|state| requested = state.loader_requested);
        requested
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

    pub fn compare_token_queries(
        &self,
        mut query: impl FnMut(TokenApi, i32, Option<&mut [u32; 2]>, u32, &mut u32) -> (bool, u32),
    ) {
        self.evidence(|evidence| {
            if !evidence.token_comparisons.is_empty() {
                return;
            }
            // This fixed matrix runs against the same suspended token. Its
            // observations are never used by the original verification below.
            for (api, kind) in [
                (TokenApi::GetTokenInformation, 29),
                (TokenApi::GetTokenInformation, 46),
                (TokenApi::NtQueryInformationToken, 46),
            ] {
                for bytes in [0, 4, 8] {
                    evidence.token_comparisons.push(TokenComparison::capture(
                        api,
                        kind,
                        bytes,
                        |buffer, bytes, length| query(api, kind, buffer, bytes, length),
                    ));
                }
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
        ordinary.compare_token_queries(|_, _, _, _, _| {
            panic!("ordinary runner issued an additional token query")
        });
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

    #[test]
    fn fixed_token_matrix_retains_errors_without_failed_buffer_values() {
        let observation = Observer::fixed_probe();
        let mut calls = Vec::new();
        observation.compare_token_queries(|api, kind, buffer, bytes, length| {
            calls.push((serde_json::to_value(api).unwrap(), kind, bytes));
            if let Some(buffer) = buffer {
                buffer[0] = 0xdead_beef;
                buffer[1] = 0xcafe_babe;
            }
            *length = 8;
            (false, 0x8007_0057)
        });
        assert_eq!(
            calls,
            [
                ("get_token_information", 29, 0),
                ("get_token_information", 29, 4),
                ("get_token_information", 29, 8),
                ("get_token_information", 46, 0),
                ("get_token_information", 46, 4),
                ("get_token_information", 46, 8),
                ("nt_query_information_token", 46, 0),
                ("nt_query_information_token", 46, 4),
                ("nt_query_information_token", 46, 8),
            ]
            .map(|(api, kind, bytes)| (serde_json::json!(api), kind, bytes))
        );
        // Repeated observation cannot create an unbounded query loop.
        observation.compare_token_queries(|_, _, _, _, _| panic!("duplicate matrix"));
        let report = observation.snapshot().unwrap();
        let comparisons = report["evidence"]["tokenComparisons"].as_array().unwrap();
        assert_eq!(comparisons.len(), 9);
        for result in comparisons {
            assert_eq!(result["success"], false);
            assert_eq!(result["status"], 0x8007_0057u32);
            assert_eq!(result["returnedBytes"], 8);
            assert_eq!(result["lengthWritten"], true);
            assert!(result["firstDword"].is_null());
            assert_eq!(result["bufferWritten"], result["bufferBytes"] != 0);
        }
        let encoded = report.to_string();
        assert!(!encoded.contains(&0xdead_beefu32.to_string()));
        assert!(!encoded.contains(&0xcafe_babeu32.to_string()));
    }

    #[test]
    fn successful_token_diagnostic_keeps_only_first_dword() {
        let result =
            TokenComparison::capture(TokenApi::GetTokenInformation, 46, 8, |buffer, _, length| {
                let buffer = buffer.unwrap();
                buffer[0] = 1;
                buffer[1] = 0xcafe_babe;
                *length = 4;
                (true, 0)
            });
        let report = serde_json::to_value(result).unwrap();
        assert_eq!(report["firstDword"], 1);
        assert_eq!(report["returnedBytes"], 4);
        assert_eq!(report["bufferWritten"], true);
        assert!(!report.to_string().contains(&0xcafe_babeu32.to_string()));
        let null_result =
            TokenComparison::capture(TokenApi::NtQueryInformationToken, 46, 0, |buffer, _, _| {
                assert!(buffer.is_none());
                (true, 0)
            });
        let report = serde_json::to_value(null_result).unwrap();
        assert!(report["firstDword"].is_null());
        assert_eq!(report["bufferWritten"], false);
        assert_eq!(report["lengthWritten"], false);
    }
}
