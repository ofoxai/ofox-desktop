//! Fixed test receipts, never a runtime admission policy.
use serde_json::Value;

/// Production CreateProcessW flags: CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT
/// | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED. A Windows test binds both
/// values to the windows crate constants used by `ConsoleMode`.
pub(super) const HIDDEN_FLAGS: u32 = 0x0808_0404;
/// The same launch with DETACHED_PROCESS in place of CREATE_NO_WINDOW.
pub(super) const DETACHED_FLAGS: u32 = 0x0008_040c;

/// One launch condition changed in B; A1/A2 keep the production launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Candidate {
    /// Adds the fixed registryRead capability (run 44027445443: insufficient).
    RegistryRead,
    /// DETACHED_PROCESS instead of CREATE_NO_WINDOW: no console to initialize.
    DetachedConsole,
}
impl Candidate {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::RegistryRead => "registryRead",
            Self::DetachedConsole => "detachedConsole",
        }
    }
}

pub(super) fn requested(value: Option<&str>) -> Option<Candidate> {
    match value {
        Some("registryRead") => Some(Candidate::RegistryRead),
        Some("detachedConsole") => Some(Candidate::DetachedConsole),
        _ => None,
    }
}

pub(super) fn eligible(error: &str) -> bool {
    let Some((_, data)) = error.split_once(":probe ") else {
        return false;
    };
    if data.len() > 65536 {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return false;
    };
    let e = &value["worker"]["evidence"];
    let d = &e["loaderDiagnostic"];
    let Some(before) = d["beforeFlags"].as_u64().filter(|v| *v <= u32::MAX as u64) else {
        return false;
    };
    e["processStatus"] == 0xc0000142u32
        && e["resumeResult"] == 1
        && d["diagnosticOnly"] == true
        && d["configConfirmed"] == true
        && d["afterFlags"].as_u64() == Some(before | 2)
        && d["preAttachSuspendPrevious"] == 1
        && d["preAttachPairedResumePrevious"] == 2
        && d["suspendPrevious"] == 1
        && d["pairedResumePrevious"] == 2
        && d["hostResumePrevious"] == 1
        && d["debuggerClosed"] == true
}

fn verified(row: &Value, count: u64) -> bool {
    row["completed"] == true
        && row["tokenVerified"] == true
        && row["cleanupConfirmed"] == true
        && row["resumePrevious"] == 1
        && row["timedOut"] == false
        && row["capabilityCount"].as_u64() == Some(count)
        && row["lpacAccessMatched"] == true
        && row["integrityRid"].as_u64().is_some_and(|v| v <= 4096)
        && row["childPolicy"].as_u64().is_some_and(|v| v & 1 == 1)
        && row["imagePolicy"].as_u64().is_some_and(|v| v & 3 == 3)
        && row["capabilityAttributes"]
            .as_array()
            .is_some_and(|v| v.len() == count as usize && v.iter().all(|v| v == 4))
        && row["stderrBytes"] == 0
}
pub(super) fn baseline(row: &Value) -> bool {
    verified(row, 1)
        && row["extraRegistryRead"] == false
        && row["consoleMode"] == "hidden"
        && row["creationFlags"] == HIDDEN_FLAGS
        && row["exitStatus"] == 0xc0000142u32
        && row["markerMatched"] == false
        && row["stdoutBytes"] == 0
}
pub(super) fn supported(rows: &[Value], candidate: Candidate) -> bool {
    let (count, extra, console, flags) = match candidate {
        Candidate::RegistryRead => (2, true, "hidden", HIDDEN_FLAGS),
        Candidate::DetachedConsole => (1, false, "detached", DETACHED_FLAGS),
    };
    rows.len() == 3
        && baseline(&rows[0])
        && baseline(&rows[2])
        && verified(&rows[1], count)
        && rows[1]["extraRegistryRead"] == extra
        && rows[1]["consoleMode"] == console
        && rows[1]["creationFlags"] == flags
        && rows[1]["exitStatus"] == 0
        && rows[1]["markerMatched"] == true
        && rows[1]["stdoutBytes"] == "OFOX_SYSTEM_INIT_V1".len()
        && [1, 2].into_iter().all(|index| {
            ["childPolicy", "imagePolicy", "integrityRid"]
                .into_iter()
                .all(|key| rows[index][key] == rows[0][key])
        })
}

/// Win32/libuv calls named by libuv's `uv_fatal_error` ("%s: (%d) %s"), covering
/// every Windows call site in the bundled Node's libuv 1.51, plus nearby stdio
/// and console calls. Any other name is reported as "?".
const FATAL_CALLS: [&str; 48] = [
    "WSAStartup",
    "uv__malloc",
    "QueryPerformanceCounter",
    "GetEnvironmentVariableW",
    "UnregisterWaitEx",
    "GetModuleHandleA",
    "GetModuleHandleW",
    "GetProcAddress",
    "LoadLibraryExA",
    "CreateIoCompletionPort",
    "GetQueuedCompletionStatus",
    "GetQueuedCompletionStatusEx",
    "PostQueuedCompletionStatus",
    "CreateEvent",
    "CreateEventA",
    "CreateEventW",
    "CreateJobObjectW",
    "SetInformationJobObject",
    "AssignProcessToJobObject",
    "QueryPerformanceFrequency",
    "SetConsoleCtrlHandler",
    "DuplicateHandle",
    "RegisterWaitForSingleObject",
    "UnregisterWait",
    "CreateFileW",
    "ReadFile",
    "WriteFile",
    "CreateThread",
    "QueueUserWorkItem",
    "TlsAlloc",
    "GetFileType",
    "NtQueryInformationFile",
    "NtSetInformationFile",
    "NtQueryVolumeInformationFile",
    "NtDeviceIoControlFile",
    "socket",
    "WSAIoctl",
    "closesocket",
    "CreateNamedPipeW",
    "ConnectNamedPipe",
    "SetNamedPipeHandleState",
    "GetConsoleMode",
    "SetConsoleMode",
    "GetConsoleScreenBufferInfo",
    "CancelIoEx",
    "GetSystemInfo",
    "OpenProcessToken",
    "GetTokenInformation",
];

/// Closed-set shape of a failed child's stderr: a fixed kind, plus a
/// vocabulary call name and numeric Win32 code for libuv fatal errors. No
/// message text, path, PID or unknown word is ever returned.
pub(super) fn stderr_shape(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return serde_json::json!({"kind":"empty"});
    }
    let text = String::from_utf8_lossy(bytes);
    // libuv's fatal line may follow other output; a known call name wins over
    // an unknown one, and its fields come only from that line.
    let fatal = text
        .lines()
        .filter_map(|line| libuv_fatal(line.trim_end_matches('\r')));
    let mut unknown_call = false;
    for (call, code) in fatal {
        // The numeric code is reported only beside a known libuv call name.
        if let Some(known) = FATAL_CALLS.into_iter().find(|known| *known == call) {
            return serde_json::json!({"kind":"libuv-fatal","call":known,"code":code});
        }
        unknown_call = true;
    }
    if unknown_call {
        return serde_json::json!({"kind":"libuv-fatal","call":"?","code":null});
    }
    let kind = if text.contains("Assertion") && text.contains("failed") {
        "node-assertion"
    } else if text.contains("FATAL ERROR:") {
        "node-fatal"
    } else if text.contains("# Fatal error") || text.contains("Check failed") {
        "v8-fatal"
    } else if text.contains("ICU") {
        "icu"
    } else if text.contains("OpenSSL") || text.contains("openssl") {
        "openssl"
    } else {
        "unknown"
    };
    serde_json::json!({"kind":kind})
}

fn libuv_fatal(text: &str) -> Option<(&str, Option<i64>)> {
    let (call, rest) = text.split_once(": (")?;
    let identifier = call.len() <= 64
        && call.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && call.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let (digits, _) = rest.split_once(')')?;
    let magnitude = digits.strip_prefix('-').unwrap_or(digits);
    let numeric =
        (1..=11).contains(&magnitude.len()) && magnitude.bytes().all(|b| b.is_ascii_digit());
    if !identifier || !numeric {
        return None;
    }
    // libuv prints the code with %d: only an i32 without leading zeros counts.
    let canonical = magnitude == "0" || !magnitude.starts_with('0');
    let code = digits
        .parse::<i64>()
        .ok()
        .filter(|value| canonical && i32::try_from(*value).is_ok());
    Some((call, code))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_disproved_candidate_is_not_repeated_without_exact_opt_in() {
        assert_eq!(
            requested(Some("registryRead")),
            Some(Candidate::RegistryRead)
        );
        assert_eq!(
            requested(Some("detachedConsole")),
            Some(Candidate::DetachedConsole)
        );
        for value in [
            None,
            Some(""),
            Some("true"),
            Some("RegistryRead"),
            Some("registryRead "),
            Some("DetachedConsole"),
            Some("detached"),
        ] {
            assert_eq!(requested(value), None);
        }
        assert_eq!(Candidate::RegistryRead.name(), "registryRead");
        assert_eq!(Candidate::DetachedConsole.name(), "detachedConsole");
    }
    fn row(extra: bool, console: &str, success: bool) -> Value {
        serde_json::json!({"completed":true,"tokenVerified":true,"cleanupConfirmed":true,"resumePrevious":1,
            "timedOut":false,"capabilityCount":if extra{2}else{1},"lpacAccessMatched":true,"childPolicy":1,"imagePolicy":7,"integrityRid":4096,
            "capabilityAttributes":if extra{vec![4,4]}else{vec![4]},"stderrBytes":0,"extraRegistryRead":extra,"consoleMode":console,
            "creationFlags":if console == "detached" {DETACHED_FLAGS} else {HIDDEN_FLAGS},
            "exitStatus":if success{0}else{0xc0000142u32},"markerMatched":success,"stdoutBytes":if success{19}else{0}})
    }
    const FIELDS: [&str; 18] = [
        "completed",
        "tokenVerified",
        "cleanupConfirmed",
        "resumePrevious",
        "timedOut",
        "capabilityCount",
        "lpacAccessMatched",
        "childPolicy",
        "imagePolicy",
        "integrityRid",
        "capabilityAttributes",
        "stderrBytes",
        "extraRegistryRead",
        "consoleMode",
        "creationFlags",
        "exitStatus",
        "markerMatched",
        "stdoutBytes",
    ];
    #[test]
    fn all_three_actual_verified_results_are_required() {
        for (candidate, middle) in [
            (Candidate::RegistryRead, row(true, "hidden", true)),
            (Candidate::DetachedConsole, row(false, "detached", true)),
        ] {
            let rows = [
                row(false, "hidden", false),
                middle,
                row(false, "hidden", false),
            ];
            assert!(supported(&rows, candidate));
            for length in 0..3 {
                assert!(!supported(&rows[..length], candidate));
            }
            for index in 0..3 {
                for field in FIELDS {
                    let mut bad = rows.clone();
                    bad[index].as_object_mut().unwrap().remove(field);
                    assert!(!supported(&bad, candidate), "{index}:{field}");
                }
            }
            let mut warm = rows.clone();
            warm[2] = rows[1].clone();
            assert!(!supported(&warm, candidate));
            for (field, value) in [("childPolicy", 3), ("imagePolicy", 3), ("integrityRid", 0)] {
                let mut confounded = rows.clone();
                confounded[1][field] = serde_json::json!(value);
                assert!(!supported(&confounded, candidate));
            }
        }
    }
    #[test]
    fn a_candidate_varies_exactly_one_launch_condition() {
        let hidden = row(false, "hidden", false);
        // The console candidate keeps the single runtime capability.
        let both = [hidden.clone(), row(true, "detached", true), hidden.clone()];
        assert!(!supported(&both, Candidate::DetachedConsole));
        assert!(!supported(&both, Candidate::RegistryRead));
        // Each candidate's row cannot certify the other candidate.
        let detached = [hidden.clone(), row(false, "detached", true), hidden.clone()];
        assert!(!supported(&detached, Candidate::RegistryRead));
        let registry = [hidden.clone(), row(true, "hidden", true), hidden.clone()];
        assert!(!supported(&registry, Candidate::DetachedConsole));
        // Baselines must use the production console mode.
        let shifted = [
            row(false, "detached", false),
            row(false, "detached", true),
            hidden,
        ];
        assert!(!supported(&shifted, Candidate::DetachedConsole));
        assert!(!baseline(&row(false, "other", false)));
    }
    #[test]
    fn the_receipt_binds_each_label_to_the_actual_creation_flags() {
        assert_eq!(HIDDEN_FLAGS, 0x0808_0404);
        assert_eq!(DETACHED_FLAGS, 0x0008_040c);
        assert_eq!(HIDDEN_FLAGS ^ DETACHED_FLAGS, 0x0800_0008);
        let hidden = row(false, "hidden", false);
        let mut mislabeled = row(false, "detached", true);
        mislabeled["creationFlags"] = serde_json::json!(HIDDEN_FLAGS);
        let rows = [hidden.clone(), mislabeled, hidden.clone()];
        assert!(!supported(&rows, Candidate::DetachedConsole));
        let mut both = row(false, "detached", true);
        both["creationFlags"] = serde_json::json!(HIDDEN_FLAGS | DETACHED_FLAGS);
        assert!(!supported(
            &[hidden.clone(), both, hidden.clone()],
            Candidate::DetachedConsole
        ));
        let mut shifted = hidden.clone();
        shifted["creationFlags"] = serde_json::json!(DETACHED_FLAGS);
        assert!(!baseline(&shifted));
    }
    #[test]
    fn stderr_is_reported_only_as_a_closed_shape() {
        let shape = |text: &[u8]| stderr_shape(text);
        assert_eq!(shape(b""), serde_json::json!({"kind":"empty"}));
        assert_eq!(
            shape(b"CreateIoCompletionPort: (5) Access is denied.\r\r\n"),
            serde_json::json!({"kind":"libuv-fatal","call":"CreateIoCompletionPort","code":5})
        );
        assert_eq!(
            shape(b"WSAStartup: (10091) WSAStartup cannot function at this time.\r\r\n"),
            serde_json::json!({"kind":"libuv-fatal","call":"WSAStartup","code":10091})
        );
        // A call outside the vocabulary or an out-of-range code is masked.
        assert_eq!(
            shape(b"PrivateUserName: (5) Access is denied."),
            serde_json::json!({"kind":"libuv-fatal","call":"?","code":null})
        );
        assert_eq!(
            shape(b"WSAStartup: (99999999999) x"),
            serde_json::json!({"kind":"libuv-fatal","call":"WSAStartup","code":null})
        );
        // The fatal line wins wherever it appears, and only its own fields count.
        assert_eq!(
            shape(b"\r\nwarning: private ICU note\r\nWSAStartup: (10091) x\r\r\n"),
            serde_json::json!({"kind":"libuv-fatal","call":"WSAStartup","code":10091})
        );
        assert_eq!(
            shape(b"uv__malloc: (14) Not enough memory.\r\r\n"),
            serde_json::json!({"kind":"libuv-fatal","call":"uv__malloc","code":14})
        );
        for (text, code) in [
            (
                &b"WSAStartup: (-2147483648) x"[..],
                Some(i64::from(i32::MIN)),
            ),
            (&b"WSAStartup: (2147483648) x"[..], None),
            (&b"WSAStartup: (00005) x"[..], None),
            (&b"WSAStartup: (0) x"[..], Some(0)),
        ] {
            assert_eq!(
                shape(text),
                serde_json::json!({"kind":"libuv-fatal","call":"WSAStartup","code":code})
            );
        }
        for text in [
            &b"WSAStartup: (-) x"[..],
            b"WSAStartup: () x",
            b"WSAStartup: (5 x",
        ] {
            assert_eq!(shape(text), serde_json::json!({"kind":"unknown"}));
        }
        for (text, kind) in [
            (
                &b"node[1234]: C:\\Users\\private\\src\\node.cc:1: Assertion `x' failed."[..],
                "node-assertion",
            ),
            (
                &b"FATAL ERROR: v8::ToLocalChecked Empty MaybeLocal"[..],
                "node-fatal",
            ),
            (
                &b"\n\n#\n# Fatal error in , line 0\n# Check failed: x.\n"[..],
                "v8-fatal",
            ),
            (
                &b"node: could not initialize ICU (check NODE_ICU_DATA)"[..],
                "icu",
            ),
            (&b"OpenSSL configuration error: private-path"[..], "openssl"),
            (&b"C:\\Users\\private: (5) Access is denied."[..], "unknown"),
            (&b"\xff\xfe private-user"[..], "unknown"),
        ] {
            let value = shape(text);
            assert_eq!(value, serde_json::json!({"kind":kind}), "{kind}");
            let encoded = value.to_string();
            for secret in ["private", "Users", "1234", "node.cc", "MaybeLocal"] {
                assert!(!encoded.contains(secret), "{kind}:{secret}");
            }
        }
    }
    #[test]
    fn every_libuv_1_51_windows_fatal_call_is_named() {
        // All uv_fatal_error call names in Node v22.23.4's deps/uv/src/win.
        for call in [
            "GetQueuedCompletionStatusEx",
            "uv__malloc",
            "CreateIoCompletionPort",
            "PostQueuedCompletionStatus",
            "CreateEvent",
            "QueueUserWorkItem",
            "CreateJobObjectW",
            "SetInformationJobObject",
            "AssignProcessToJobObject",
            "GetEnvironmentVariableW",
            "UnregisterWaitEx",
            "RegisterWaitForSingleObject",
            "DuplicateHandle",
            "QueryPerformanceFrequency",
            "QueryPerformanceCounter",
            "GetModuleHandleA",
            "GetProcAddress",
            "WSAStartup",
        ] {
            let line = format!("{call}: (5) Access is denied.\r\r\n");
            assert_eq!(
                stderr_shape(line.as_bytes()),
                serde_json::json!({"kind":"libuv-fatal","call":call,"code":5}),
                "{call}"
            );
        }
    }
    #[test]
    fn incomplete_or_wrong_original_failure_never_arms_comparison() {
        let mut e = serde_json::json!({"processStatus":0xc0000142u32,"resumeResult":1,"loaderDiagnostic":{
            "diagnosticOnly":true,"configConfirmed":true,"beforeFlags":0,"afterFlags":2,"preAttachSuspendPrevious":1,
            "preAttachPairedResumePrevious":2,"suspendPrevious":1,"pairedResumePrevious":2,"hostResumePrevious":1,"debuggerClosed":true}});
        let message = |e: &Value| {
            format!(
                "unavailable:probe {}",
                serde_json::json!({"worker":{"evidence":e}})
            )
        };
        assert!(eligible(&message(&e)));
        for field in [
            "diagnosticOnly",
            "configConfirmed",
            "beforeFlags",
            "afterFlags",
            "preAttachSuspendPrevious",
            "preAttachPairedResumePrevious",
            "suspendPrevious",
            "pairedResumePrevious",
            "hostResumePrevious",
            "debuggerClosed",
        ] {
            let mut bad = e.clone();
            bad["loaderDiagnostic"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(!eligible(&message(&bad)), "{field}");
        }
        e["processStatus"] = serde_json::json!(0);
        assert!(!eligible(&message(&e)));
        assert!(!eligible("unavailable"));
        assert!(!eligible("unavailable:probe broken"));
    }
}
