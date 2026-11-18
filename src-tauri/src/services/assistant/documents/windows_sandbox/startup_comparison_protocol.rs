//! Fixed test receipts, never a runtime admission policy.
use serde_json::Value;

/// The pre-fix production flags, CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT
/// | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED, now used only by the A1/A2
/// baselines. A Windows test binds both values to the `ConsoleMode` constants.
pub(super) const HIDDEN_FLAGS: u32 = 0x0808_0404;
/// The same launch with DETACHED_PROCESS in place of CREATE_NO_WINDOW; production.
pub(super) const DETACHED_FLAGS: u32 = 0x0008_040c;

/// One launch condition changed in B relative to A1/A2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Candidate {
    /// Adds the fixed registryRead capability (run 44027445443: insufficient).
    RegistryRead,
    /// DETACHED_PROCESS instead of CREATE_NO_WINDOW: no console to initialize.
    /// Run 44068121826 got past KernelBase this way.
    DetachedConsole,
    /// Both A rows already run without a console and must reproduce libuv's
    /// WSAStartup 10107 abort (run 44071671054); B adds only registryRead.
    DetachedRegistryRead,
    /// Same baseline; B keeps the single runtime capability but runs while a
    /// temporary, CI-only ACE lets that capability read the Winsock catalog.
    /// registryRead also exposed HKCU (run 44133006077).
    DetachedWinsockGrant,
}
/// The launch shared by A1/A2 and the one condition set B changes.
#[derive(Clone, Copy)]
pub(super) struct Plan {
    pub(super) base_detached: bool,
    pub(super) extra: bool,
    pub(super) detached: bool,
    pub(super) winsock_grant: bool,
}
impl Candidate {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::RegistryRead => "registryRead",
            Self::DetachedConsole => "detachedConsole",
            Self::DetachedRegistryRead => "detachedRegistryRead",
            Self::DetachedWinsockGrant => "detachedWinsockGrant",
        }
    }
    pub(super) fn plan(self) -> Plan {
        let (base_detached, extra, detached, winsock_grant) = match self {
            Self::RegistryRead => (false, true, false, false),
            Self::DetachedConsole => (false, false, true, false),
            Self::DetachedRegistryRead => (true, true, true, false),
            Self::DetachedWinsockGrant => (true, false, true, true),
        };
        Plan {
            base_detached,
            extra,
            detached,
            winsock_grant,
        }
    }
}

pub(super) fn requested(value: Option<&str>) -> Option<Candidate> {
    [
        Candidate::RegistryRead,
        Candidate::DetachedConsole,
        Candidate::DetachedRegistryRead,
        Candidate::DetachedWinsockGrant,
    ]
    .into_iter()
    .find(|candidate| value == Some(candidate.name()))
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
}
/// Every isolation proof plus the exact capability set and console flags.
fn launched(row: &Value, extra: bool, detached: bool) -> bool {
    let (console, flags) = if detached {
        ("detached", DETACHED_FLAGS)
    } else {
        ("hidden", HIDDEN_FLAGS)
    };
    verified(row, if extra { 2 } else { 1 })
        && row["extraRegistryRead"] == extra
        && row["consoleMode"] == console
        && row["creationFlags"] == flags
}
/// A1/A2 must reproduce the failure the candidate addresses, exactly.
pub(super) fn baseline(row: &Value, candidate: Candidate) -> bool {
    let detached = candidate.plan().base_detached;
    let failed = if detached {
        row["exitStatus"] == 0x8000_0003u32
            // Below the 1024-byte drain limit, so the shape saw the whole text.
            && row["stderrBytes"]
                .as_u64()
                .is_some_and(|bytes| (1..1024).contains(&bytes))
            && row["stderrShape"]
                == serde_json::json!({"kind":"libuv-fatal","call":"WSAStartup","code":10107})
    } else {
        row["exitStatus"] == 0xc000_0142u32
            && row["stderrBytes"] == 0
            && row["stderrShape"] == serde_json::json!({"kind":"empty"})
    };
    launched(row, false, detached)
        && row["winsockGrant"] == false
        && failed
        && row["markerMatched"] == false
        && row["stdoutBytes"] == 0
}
pub(super) fn supported(rows: &[Value], candidate: Candidate) -> bool {
    let plan = candidate.plan();
    rows.len() == 3
        && baseline(&rows[0], candidate)
        && baseline(&rows[2], candidate)
        && launched(&rows[1], plan.extra, plan.detached)
        && rows[1]["winsockGrant"] == plan.winsock_grant
        // A temporary system ACL counts only if its exact restore was proven.
        && (!plan.winsock_grant || rows[1]["grantRestored"] == true)
        && rows[1]["exitStatus"] == 0
        && rows[1]["markerMatched"] == true
        && rows[1]["stdoutBytes"] == "OFOX_SYSTEM_INIT_V1".len()
        && rows[1]["stderrBytes"] == 0
        && rows[1]["stderrShape"] == serde_json::json!({"kind":"empty"})
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
    const ALL: [Candidate; 4] = [
        Candidate::RegistryRead,
        Candidate::DetachedConsole,
        Candidate::DetachedRegistryRead,
        Candidate::DetachedWinsockGrant,
    ];
    #[test]
    fn a_disproved_candidate_is_not_repeated_without_exact_opt_in() {
        for candidate in ALL {
            assert_eq!(requested(Some(candidate.name())), Some(candidate));
        }
        assert_eq!(Candidate::RegistryRead.name(), "registryRead");
        assert_eq!(Candidate::DetachedConsole.name(), "detachedConsole");
        assert_eq!(
            Candidate::DetachedRegistryRead.name(),
            "detachedRegistryRead"
        );
        assert_eq!(
            Candidate::DetachedWinsockGrant.name(),
            "detachedWinsockGrant"
        );
        for value in [
            None,
            Some(""),
            Some("true"),
            Some("RegistryRead"),
            Some("registryRead "),
            Some("DetachedConsole"),
            Some("detached"),
            Some("DetachedRegistryRead"),
            Some("detachedWinsock"),
        ] {
            assert_eq!(requested(value), None);
        }
    }
    /// outcome: "ok", "dll" (0xc0000142) or "winsock" (libuv WSAStartup abort).
    fn row(extra: bool, console: &str, outcome: &str) -> Value {
        let (exit, stdout, stderr, shape) = match outcome {
            "ok" => (0u32, 19, 0, serde_json::json!({"kind":"empty"})),
            "dll" => (0xc000_0142, 0, 0, serde_json::json!({"kind":"empty"})),
            "winsock" => (
                0x8000_0003,
                0,
                48,
                serde_json::json!({"kind":"libuv-fatal","call":"WSAStartup","code":10107}),
            ),
            _ => unreachable!(),
        };
        serde_json::json!({"completed":true,"tokenVerified":true,"cleanupConfirmed":true,"resumePrevious":1,
            "timedOut":false,"capabilityCount":if extra{2}else{1},"lpacAccessMatched":true,"childPolicy":1,"imagePolicy":7,"integrityRid":4096,
            "capabilityAttributes":if extra{vec![4,4]}else{vec![4]},"stderrBytes":stderr,"stderrShape":shape,"extraRegistryRead":extra,"consoleMode":console,
            "creationFlags":if console == "detached" {DETACHED_FLAGS} else {HIDDEN_FLAGS},
            "exitStatus":exit,"markerMatched":outcome == "ok","stdoutBytes":stdout,
            "winsockGrant":false,"grantRestored":null})
    }
    /// B row of the CI-only candidate: the temporary catalog ACE was applied
    /// for the launch and the exact original DACL was restored afterwards.
    fn granted(mut row: Value) -> Value {
        row["winsockGrant"] = serde_json::json!(true);
        row["grantRestored"] = serde_json::json!(true);
        row
    }
    /// The A1/A2 row and the B row each candidate expects.
    fn rows_for(candidate: Candidate) -> (Value, Value) {
        match candidate {
            Candidate::RegistryRead => (row(false, "hidden", "dll"), row(true, "hidden", "ok")),
            Candidate::DetachedConsole => {
                (row(false, "hidden", "dll"), row(false, "detached", "ok"))
            }
            Candidate::DetachedRegistryRead => (
                row(false, "detached", "winsock"),
                row(true, "detached", "ok"),
            ),
            Candidate::DetachedWinsockGrant => (
                row(false, "detached", "winsock"),
                granted(row(false, "detached", "ok")),
            ),
        }
    }
    const FIELDS: [&str; 20] = [
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
        "stderrShape",
        "extraRegistryRead",
        "consoleMode",
        "creationFlags",
        "exitStatus",
        "markerMatched",
        "stdoutBytes",
        "winsockGrant",
    ];
    #[test]
    fn all_three_actual_verified_results_are_required() {
        for candidate in ALL {
            let (base, middle) = rows_for(candidate);
            let rows = [base.clone(), middle, base];
            assert!(supported(&rows, candidate), "{}", candidate.name());
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
        // Neither another candidate's B row nor its baseline can certify one.
        for candidate in ALL {
            let (base, middle) = rows_for(candidate);
            for other in ALL.into_iter().filter(|other| *other != candidate) {
                let (other_base, other_middle) = rows_for(other);
                assert!(!supported(
                    &[base.clone(), other_middle, base.clone()],
                    candidate
                ));
                if other_base != base {
                    assert!(!supported(
                        &[other_base.clone(), middle.clone(), other_base],
                        candidate
                    ));
                }
            }
        }
        // A B row that changes both conditions only fits the staged candidate.
        let both = row(true, "detached", "ok");
        for candidate in [Candidate::RegistryRead, Candidate::DetachedConsole] {
            let (base, _) = rows_for(candidate);
            assert!(!supported(&[base.clone(), both.clone(), base], candidate));
        }
        // The console-free baseline must reproduce exactly WSAStartup 10107.
        for (key, value) in [
            ("code", serde_json::json!(10091)),
            ("call", serde_json::json!("CreateEvent")),
        ] {
            let mut other = row(false, "detached", "winsock");
            other["stderrShape"][key] = value;
            assert!(!baseline(&other, Candidate::DetachedRegistryRead));
        }
        let mut silent = row(false, "detached", "winsock");
        silent["stderrBytes"] = serde_json::json!(0);
        assert!(!baseline(&silent, Candidate::DetachedRegistryRead));
        assert!(!baseline(
            &row(false, "detached", "dll"),
            Candidate::DetachedRegistryRead
        ));
        assert!(!baseline(
            &row(false, "other", "dll"),
            Candidate::RegistryRead
        ));
        // The grant candidate needs a restored grant on B and none on A1/A2.
        let (base, middle) = rows_for(Candidate::DetachedWinsockGrant);
        let mut unrestored = middle.clone();
        unrestored["grantRestored"] = serde_json::json!(false);
        assert!(!supported(
            &[base.clone(), unrestored, base.clone()],
            Candidate::DetachedWinsockGrant
        ));
        assert!(!baseline(
            &granted(base.clone()),
            Candidate::DetachedWinsockGrant
        ));
        let mut missing = middle;
        missing.as_object_mut().unwrap().remove("grantRestored");
        assert!(!supported(
            &[base.clone(), missing, base],
            Candidate::DetachedWinsockGrant
        ));
    }
    #[test]
    fn each_plan_changes_exactly_one_launch_condition() {
        for candidate in ALL {
            let plan = candidate.plan();
            let changes = u8::from(plan.extra)
                + u8::from(plan.base_detached != plan.detached)
                + u8::from(plan.winsock_grant);
            assert_eq!(changes, 1, "{}", candidate.name());
        }
    }
    #[test]
    fn unexpected_stderr_on_a_silent_row_is_refused() {
        for candidate in ALL {
            let (base, middle) = rows_for(candidate);
            let mut noisy = middle.clone();
            noisy["stderrBytes"] = serde_json::json!(5);
            assert!(!supported(&[base.clone(), noisy, base.clone()], candidate));
            let mut shaped = middle;
            shaped["stderrShape"] = serde_json::json!({"kind":"unknown"});
            assert!(!supported(&[base.clone(), shaped, base], candidate));
        }
        let mut noisy = row(false, "hidden", "dll");
        noisy["stderrBytes"] = serde_json::json!(5);
        assert!(!baseline(&noisy, Candidate::RegistryRead));
        let mut shaped = row(false, "hidden", "dll");
        shaped["stderrShape"] = serde_json::json!({"kind":"unknown"});
        assert!(!baseline(&shaped, Candidate::RegistryRead));
        // At the drain limit the text may be truncated, so it cannot reproduce.
        let mut truncated = row(false, "detached", "winsock");
        truncated["stderrBytes"] = serde_json::json!(1024);
        assert!(!baseline(&truncated, Candidate::DetachedRegistryRead));
    }
    #[test]
    fn the_receipt_binds_each_label_to_the_actual_creation_flags() {
        assert_eq!(HIDDEN_FLAGS, 0x0808_0404);
        assert_eq!(DETACHED_FLAGS, 0x0008_040c);
        assert_eq!(HIDDEN_FLAGS ^ DETACHED_FLAGS, 0x0800_0008);
        let hidden = row(false, "hidden", "dll");
        let mut mislabeled = row(false, "detached", "ok");
        mislabeled["creationFlags"] = serde_json::json!(HIDDEN_FLAGS);
        let rows = [hidden.clone(), mislabeled, hidden.clone()];
        assert!(!supported(&rows, Candidate::DetachedConsole));
        let mut both = row(false, "detached", "ok");
        both["creationFlags"] = serde_json::json!(HIDDEN_FLAGS | DETACHED_FLAGS);
        assert!(!supported(
            &[hidden.clone(), both, hidden.clone()],
            Candidate::DetachedConsole
        ));
        let mut shifted = hidden.clone();
        shifted["creationFlags"] = serde_json::json!(DETACHED_FLAGS);
        assert!(!baseline(&shifted, Candidate::DetachedConsole));
        let mut detached = row(false, "detached", "winsock");
        detached["creationFlags"] = serde_json::json!(HIDDEN_FLAGS);
        assert!(!baseline(&detached, Candidate::DetachedRegistryRead));
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
}
