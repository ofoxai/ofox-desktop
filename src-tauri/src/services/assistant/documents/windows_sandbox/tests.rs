//! These tests must run on native Windows. A macOS compilation is not evidence
//! that a lowbox token, image policy, ACL, or Job is enforced.
use super::*;

fn runtime() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime/resources")
}

#[tokio::test]
#[serial_test::serial]
async fn document_lpac_proves_python_office_and_escape_denials() {
    assert_suspended_lpac_access_controls();
    let root = runtime();
    let office = root.join("engines/officecli/officecli.exe");
    let result = probe_windows_document_engines(&root, &office).await;
    let candidate = startup_comparison_protocol::requested(
        std::env::var("OFOX_DOCUMENT_STARTUP_COMPARISON")
            .ok()
            .as_deref(),
    );
    if let Some(candidate) = candidate.filter(|_| {
        std::env::var_os(cdb::ENV_ROOT).is_some()
            && result
                .as_ref()
                .err()
                .is_some_and(|error| startup_comparison_protocol::eligible(error))
    }) {
        match tokio::task::spawn_blocking(move || startup_comparison::compare(&root, candidate))
            .await
        {
            Ok(receipt) => eprintln!("OFOX_STARTUP_COMPARISON_V1 {receipt}"),
            Err(_) => eprintln!(
                "OFOX_STARTUP_COMPARISON_V1 {{\"status\":\"inconclusive\",\"joinFailure\":true}}"
            ),
        }
    }
    result.expect("native Python + OfficeCLI and all LPAC negative probes must pass");
}

#[test]
#[serial_test::serial]
fn document_lpac_verifier_rejects_real_ordinary_appcontainer_before_resume() {
    assert_suspended_lpac_access_controls();
}

// Both callers already hold the default serial lock. Do not annotate this
// helper or call another serial test: nested locks would deadlock the gate.
fn assert_suspended_lpac_access_controls() {
    let cap = RuntimeCapability::new().unwrap();
    let user = current_user().unwrap();
    // This fixed Windows image is never resumed. Token controls run before
    // expensive runtime hashing; bundled engine behavior is proved afterward.
    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
    assert!(system.is_absolute());
    let exe = system.join("System32").join("cmd.exe");
    let mut profile = Profile::new(&cap).unwrap();
    profile.make_low_profile().unwrap();
    let mut task = task_directory(&user, profile.sid.0).unwrap();
    let home = task.0.join("home");
    fs::create_dir(&home).unwrap();
    let marker = task.0.join("must-not-run");
    for (policy, lpac) in [
        (ContainerPolicy::Lpac, true),
        (ContainerPolicy::OrdinaryAppContainer, false),
    ] {
        let mut trace = ProbeTrace::new();
        trace.engine(ProbeStage::NodeIsolation);
        let observation = trace.observation.clone();
        let child = create_suspended(
            &exe,
            &["/D".into(), "/C".into(), "echo bad>must-not-run".into()],
            &task.0,
            &home,
            &profile,
            &cap,
            policy,
            &observation,
        )
        .unwrap_or_else(|error| {
            observation.failed();
            panic!(
                "SUSPENDED_CONTROL_CREATE lpac={lpac}: {}",
                trace.failure(&error, &[])
            );
        });
        let verified = verify_token(
            raw(&child.process.process),
            profile.sid.0,
            cap.sid,
            &observation,
        );
        if verified.is_err() {
            observation.failed();
        }
        let report = observation.snapshot().unwrap();
        let diagnostic = verified
            .as_ref()
            .err()
            .map(|error| trace.failure(error, &[]));
        if lpac {
            assert!(
                verified.is_ok(),
                "actual suspended LPAC must pass the complete production verifier: diagnostic={diagnostic:?}; report={report}"
            );
        } else {
            assert!(
                verified.as_ref().err().is_some_and(|error| error == UNAVAILABLE),
                "ordinary AC must receive the expected production refusal: accepted={}; diagnostic={diagnostic:?}; report={report}",
                verified.is_ok()
            );
        }
        let evidence = &report["evidence"];
        assert_eq!(evidence["appContainer"], 1);
        assert_eq!(evidence["packageMatches"], true);
        // Production: exactly the private runtime capability plus registryRead.
        assert_eq!(evidence["capabilityGroups"], 2);
        assert_eq!(evidence["capabilityMatches"], true);
        assert!(evidence["integrityRid"].as_u64().unwrap() <= 4096);
        assert_eq!(evidence["childPolicy"].as_u64().unwrap() & 1, 1);
        assert_eq!(evidence["imagePolicy"].as_u64().unwrap() & 3, 3);
        assert!(
            evidence["lessPrivileged"].is_null(),
            "AccessCheck proof must not impersonate a class46 gate result"
        );
        assert!(evidence["resumeResult"].is_null());
        assert_eq!(evidence["lpacAccessProof"]["matched"], lpac);
        let checks = evidence["lpacAccessProof"]["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 4);
        for (check, (case, granted)) in checks.iter().zip([
            ("world_only", false),
            ("all_application_packages", !lpac),
            ("runtime_capability", true),
            ("foreign_capability", false),
        ]) {
            assert_eq!(check["case"], case);
            assert_eq!(check["apiSuccess"], true);
            assert_eq!(check["privilegeCount"], 0);
            assert!(valid_specific_privilege_output(
                check["returnedPrivilegeBytes"].as_u64().unwrap() as u32,
                0
            ));
            assert_eq!(check["accessStatus"], granted);
            assert_eq!(check["grantedAccess"], u32::from(granted));
        }
        // The registry gate runs only after the LPAC matrix matched: the Winsock
        // catalog positive control, then the user's private keys refused.
        let registry = evidence["registryIsolation"].as_array().unwrap();
        if lpac {
            let keys: Vec<_> = registry.iter().map(|row| row["key"].clone()).collect();
            assert_eq!(
                keys,
                ["winsock_catalog", "user_environment", "user_software"]
            );
            for (row, granted) in registry.iter().zip([true, false, false]) {
                assert_eq!(row["opened"], true);
                assert_eq!(row["descriptorRead"], true);
                assert_eq!(row["accessChecked"], true);
                assert_eq!(row["readGranted"], granted);
            }
        } else {
            assert!(registry.is_empty());
        }
        assert!(!marker.exists(), "suspended control must never execute");
        unsafe {
            win(TerminateJobObject(raw(&child.process.job), 1)).unwrap();
            assert_eq!(
                WaitForSingleObject(raw(&child.process.process), 5000),
                WAIT_OBJECT_0
            );
        }
        drop(child);
        assert!(
            !marker.exists(),
            "rejected ordinary AC must never be resumed"
        );
    }
    task.dispose().unwrap();
    profile.dispose().unwrap();
}

/// registryRead must grant exactly what libuv's WSAStartup needs and nothing
/// private: with it the Winsock catalog key is readable, without it it is not,
/// the user's Environment and Software keys stay refused either way, and no
/// key anywhere in HKCU becomes readable because of it.
#[test]
#[serial_test::serial]
fn document_registry_read_opens_the_winsock_catalog_but_not_user_keys() {
    let cap = RuntimeCapability::new().unwrap();
    let user = current_user().unwrap();
    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
    let exe = system.join("System32").join("cmd.exe");
    let mut profile = Profile::new(&cap).unwrap();
    profile.make_low_profile().unwrap();
    let mut task = task_directory(&user, profile.sid.0).unwrap();
    let home = task.0.join("home");
    fs::create_dir(&home).unwrap();
    let marker = task.0.join("must-not-run");
    // Never resumed: only each suspended child's actual token is examined.
    let suspended_token = |caps: &ProcessCapabilities<'_>| {
        let observation = Observer::fixed_probe();
        let child = create_suspended_capabilities(
            &exe,
            &["/D".into(), "/C".into(), "echo bad>must-not-run".into()],
            &task.0,
            &home,
            &profile,
            caps,
            ContainerPolicy::Lpac,
            ConsoleMode::Detached,
            &observation,
        )
        .unwrap();
        // The production verifier accepts only the production set.
        let verified = verify_token(
            raw(&child.process.process),
            profile.sid.0,
            cap.sid,
            &observation,
        );
        let mut token = HANDLE::default();
        unsafe {
            win(OpenProcessToken(
                raw(&child.process.process),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                &mut token,
            ))
            .unwrap();
        }
        let token = handle(token);
        let duplicate = access_impersonation_token(raw(&token), &observation).unwrap();
        unsafe {
            win(TerminateJobObject(raw(&child.process.job), 1)).unwrap();
            assert_eq!(
                WaitForSingleObject(raw(&child.process.process), 5000),
                WAIT_OBJECT_0
            );
        }
        drop(child);
        (verified.is_ok(), token, duplicate)
    };
    let (production_verified, production_primary, production) =
        suspended_token(&ProcessCapabilities::production(&cap).unwrap());
    let (runtime_verified, runtime_primary, runtime_only) =
        suspended_token(&ProcessCapabilities::runtime_only(&cap));
    // The positive control must refuse whenever its expectation is wrong.
    let gate = |token: &OwnedHandle, registry_read| {
        verify_lpac_access(raw(token), cap.sid, registry_read, &Observer::fixed_probe()).is_ok()
    };
    let gates = [
        gate(&production_primary, true),
        gate(&production_primary, false),
        gate(&runtime_primary, false),
        gate(&runtime_primary, true),
    ];
    let keys = [
        RegistryKey::WinsockCatalog,
        RegistryKey::UserEnvironment,
        RegistryKey::UserSoftware,
    ];
    let observation = Observer::fixed_probe();
    let reads = |token: &OwnedHandle| {
        keys.map(|key| registry_read_granted(raw(token), key, &observation).unwrap())
    };
    let (production_reads, runtime_reads) = (reads(&production), reads(&runtime_only));
    let scan = scan_user_hive([raw(&production), raw(&runtime_only)]);
    assert!(
        !marker.exists(),
        "suspended registry controls must never execute"
    );
    task.dispose().unwrap();
    profile.dispose().unwrap();
    // Every observation is logged before any assertion, as closed-set values
    // and counts only, so a failure still reports both tokens and the walk.
    eprintln!(
        "OFOX_REGISTRY_ISOLATION_V1 {}",
        serde_json::json!({
            "keys": ["winsock_catalog", "user_environment", "user_software"],
            "production": production_reads,
            "runtimeOnly": runtime_reads,
            "productionVerified": production_verified,
            "runtimeOnlyVerified": runtime_verified,
            "gates": gates,
            "scan": {
                "visited": scan.visited,
                "vanished": scan.vanished,
                "hostUnopenable": scan.host_unopenable,
                "enumerationStopped": scan.enumeration_stopped,
                "checkErrors": scan.check_errors,
                "readableByRuntimeOnly": scan.readable_by_runtime_only,
                "readableByProduction": scan.readable_by_production,
                "addedByRegistryRead": scan.added_by_registry_read,
            },
        })
    );
    assert_eq!(
        gates,
        [true, false, true, false],
        "registry gate expectations"
    );
    assert!(production_verified, "production set must pass verify_token");
    assert!(!runtime_verified, "runtime-only set must fail verify_token");
    assert_eq!(
        production_reads,
        [true, false, false],
        "production capability set"
    );
    assert_eq!(runtime_reads, [false, false, false], "runtime-only control");
    assert!(scan.visited >= 100, "HKCU walk too small: {scan:?}");
    assert_eq!(
        scan.check_errors, 0,
        "HKCU keys without a decision: {scan:?}"
    );
    assert_eq!(
        scan.added_by_registry_read, 0,
        "registryRead made HKCU keys readable: {scan:?}"
    );
}

/// Counts only; no key name, path or value is recorded.
#[derive(Debug, Default)]
struct HiveScan {
    visited: usize,
    vanished: usize,
    host_unopenable: usize,
    enumeration_stopped: usize,
    check_errors: usize,
    readable_by_runtime_only: usize,
    readable_by_production: usize,
    added_by_registry_read: usize,
}

/// Walks the whole current-user hive and compares, key by key, the read
/// decision for [production token, runtime-only token]. Bounded: exceeding the
/// key limit is a test failure rather than silently partial evidence.
fn scan_user_hive(tokens: [HANDLE; 2]) -> HiveScan {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    const KEY_LIMIT: usize = 300_000;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const KEY_ENUMERATE_SUB_KEYS: u32 = 0x0008;
    // RegQueryInfoKey (the vanished-key check) needs KEY_QUERY_VALUE; run
    // 44133006077 counted 12 of 13 keys as vanished without it.
    let flags = REGISTRY_READ_CONTROL | KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS;
    let mut scan = HiveScan::default();
    let root = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("", flags)
        .unwrap();
    let mut pending = vec![root];
    while let Some(key) = pending.pop() {
        scan.visited += 1;
        assert!(scan.visited <= KEY_LIMIT, "HKCU walk exceeded its bound");
        // A key deleted while queued is not evidence either way.
        if key.query_info().is_err() {
            scan.vanished += 1;
            continue;
        }
        let handle = HANDLE(key.raw_handle() as _);
        let decide = |token| {
            let mut record = RegistryAccess {
                key: RegistryKey::UserSoftware,
                opened: true,
                descriptor_read: false,
                access_checked: false,
                read_granted: None,
            };
            registry_key_read_bits(handle, token, &mut record)
        };
        match (decide(tokens[0]), decide(tokens[1])) {
            // Any read right registryRead adds counts, bit by bit.
            (Ok(production), Ok(runtime_only)) => {
                scan.readable_by_runtime_only += usize::from(runtime_only != 0);
                scan.readable_by_production += usize::from(production != 0);
                if production & !runtime_only != 0 {
                    scan.added_by_registry_read += 1;
                }
            }
            _ => scan.check_errors += 1,
        }
        // winreg keeps advancing past errors (e.g. ERROR_KEY_DELETED); stop at
        // the first one instead of iterating the whole index space.
        for name in key.enum_keys() {
            let Ok(name) = name else {
                scan.enumeration_stopped += 1;
                break;
            };
            match key.open_subkey_with_flags(name, flags) {
                Ok(child) => pending.push(child),
                Err(_) => scan.host_unopenable += 1,
            }
        }
    }
    scan
}

#[test]
fn document_lpac_accesscheck_api_error_is_never_a_denial() {
    let mut primary = HANDLE::default();
    unsafe {
        win(OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_DUPLICATE,
            &mut primary,
        ))
        .unwrap();
    }
    let primary = handle(primary);
    let observation = Observer::fixed_probe();
    let duplicate = access_impersonation_token(raw(&primary), &observation).unwrap();
    // AccessCheck explicitly requires both owner and group in the descriptor.
    // A malformed descriptor must fail the API, never become a valid deny.
    assert!(check_specific_access(
        raw(&duplicate),
        "D:P(A;;0x1;;;WD)",
        AccessCase::WorldOnly,
        &observation,
    )
    .is_err());
    let report = observation.snapshot().unwrap();
    let result = &report["evidence"]["lpacAccessProof"]["checks"][0];
    assert_eq!(result["apiSuccess"], false);
    assert!(result["status"].as_u64().unwrap() != 0);
    assert!(result["accessStatus"].is_null());
    assert!(result["grantedAccess"].is_null());
    assert!(result["returnedPrivilegeBytes"].is_null());
    assert!(result["privilegeCount"].is_null());
    assert!(report["evidence"]["lpacAccessProof"]["matched"].is_null());
    // The same helper must grant the ordinary user side on a valid SD. Its
    // result is not sufficient for LPAC: the real paired test covers that gate.
    assert_eq!(
        check_specific_access(
            raw(&duplicate),
            "O:SYG:SYD:P(A;;0x1;;;WD)",
            AccessCase::WorldOnly,
            &Observer::default(),
        )
        .unwrap(),
        AccessVerdict {
            allowed: true,
            granted: 1
        }
    );
}

#[test]
fn document_lpac_access_matrix_rejects_missing_positive_or_extra_grants() {
    let deny = AccessVerdict {
        allowed: false,
        granted: 0,
    };
    let grant = AccessVerdict {
        allowed: true,
        granted: 1,
    };
    assert!(lpac_access_matches(&[deny, deny, grant, deny]));
    for unsafe_matrix in [
        [deny; 4],                  // Bad SD / all-deny cannot prove LPAC.
        [grant; 4],                 // An unrestricted host token cannot prove LPAC.
        [deny, grant, grant, deny], // The actual ordinary-AC negative control.
        [deny, deny, grant, grant], // An unrelated capability must stay denied.
        [
            deny,
            deny,
            AccessVerdict {
                allowed: true,
                granted: 3,
            },
            deny,
        ],
        [
            AccessVerdict {
                allowed: false,
                granted: 1,
            },
            deny,
            grant,
            deny,
        ],
    ] {
        assert!(!lpac_access_matches(&unsafe_matrix));
    }
}

#[test]
fn document_lpac_accesscheck_rejects_unknown_privilege_output() {
    let header = std::mem::offset_of!(PRIVILEGE_SET, Privilege) as u32;
    let capacity = size_of::<PRIVILEGE_SET>() as u32;
    assert!(valid_specific_privilege_output(header, 0));
    assert!(valid_specific_privilege_output(capacity, 0));
    for (bytes, count) in [
        (0, 0),
        (header - 1, 0),
        (capacity + 1, 0),
        (capacity, 1),
        (capacity, u32::MAX),
    ] {
        assert!(!valid_specific_privilege_output(bytes, count));
    }
}

#[tokio::test]
#[serial_test::serial]
async fn document_lpac_keeps_source_inputs_and_cannot_modify_runtime() {
    let root = runtime();
    ensure_windows_document_engine_ready(&root).await.unwrap();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::write(task.path().join("source.pptx"), b"frozen input").unwrap();
    let script = r#"
      const fs = require('node:fs');
      const assert = require('node:assert/strict');
      assert.equal(fs.readFileSync('source.pptx', 'utf8'), 'frozen input');
      assert.throws(() => fs.writeFileSync(process.execPath, 'changed'));
      fs.writeFileSync('receipt.txt', 'checked');
    "#;
    let out = run_windows_engine_limited(
        &root.join("node/node.exe"),
        &["-e", script],
        task.path(),
        &home,
        &root,
        Duration::from_secs(20),
        4096,
    )
    .await
    .unwrap();
    assert_eq!(out.status, Some(0), "{}", out.stderr);
    assert_eq!(
        fs::read(task.path().join("receipt.txt")).unwrap(),
        b"checked"
    );
    assert_eq!(
        fs::read(task.path().join("source.pptx")).unwrap(),
        b"frozen input"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn document_lpac_timeout_prevents_late_writes_and_bounds_output() {
    let root = runtime();
    ensure_windows_document_engine_ready(&root).await.unwrap();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    let out = run_windows_engine_limited(
        &root.join("node/node.exe"),
        &["-e", "process.stdout.write('x'.repeat(100000)); setTimeout(() => require('node:fs').writeFileSync('late', 'bad'), 4000);"],
        task.path(),
        &home,
        &root,
        Duration::from_secs(1),
        1024,
    )
    .await
    .unwrap();
    assert!(out.timed_out);
    assert!(out.stdout.len() <= 1024);
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!task.path().join("late").exists());
}

#[test]
fn document_output_rejects_hardlinks_and_reparse_points() {
    use std::os::windows::fs::symlink_dir;
    let root = tempfile::tempdir().unwrap();
    let task = root.path().join("task");
    let other = root.path().join("other");
    fs::create_dir(&task).unwrap();
    fs::create_dir(&other).unwrap();
    fs::write(other.join("secret"), b"outside").unwrap();
    fs::hard_link(other.join("secret"), task.join("linked")).unwrap();
    assert!(open_regular(&task, &task.join("linked"), 4096).is_err());
    // Junctions do not require developer mode or symlink privilege.
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(task.join("junction"))
        .arg(&other)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(open_regular(&task, &task.join("junction/secret"), 4096).is_err());
    // The apparent root itself can be reached through an earlier junction.
    // Checking only the final parent directory would accept this case.
    fs::create_dir(other.join("nested")).unwrap();
    fs::write(other.join("nested/secret"), b"outside nested").unwrap();
    assert!(open_regular(
        &task.join("junction/nested"),
        &task.join("junction/nested/secret"),
        4096
    )
    .is_err());
    assert!(publish_output(&task, Path::new("junction/new.txt"), b"must not escape").is_err());
    assert!(!other.join("new.txt").exists());
    // A junction whose target is still inside root must also be rejected.
    // In a verbatim path `/` would hide the junction inside one Normal
    // component; raw Win32 encoding must not reinterpret that after pinning.
    let inside = task.join("inside");
    fs::create_dir(&inside).unwrap();
    fs::write(inside.join("secret"), b"inside fixture").unwrap();
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(task.join("inside-junction"))
        .arg(&inside)
        .status()
        .unwrap();
    assert!(status.success());
    let verbatim = task.canonicalize().unwrap();
    let hidden_ancestor = PathBuf::from(format!(r"{}\inside-junction/secret", verbatim.display()));
    assert!(open_regular(&verbatim, &hidden_ancestor, 4096).is_err());
    if symlink_dir(&other, task.join("symlink")).is_ok() {
        assert!(open_regular(&task, &task.join("symlink/secret"), 4096).is_err());
    }
    assert_eq!(fs::read(other.join("secret")).unwrap(), b"outside");
}

#[test]
fn document_output_publishes_new_and_existing_files_by_parent_handle() {
    let root = tempfile::tempdir().unwrap();
    publish_output(root.path(), Path::new("a"), b"short name").unwrap();
    assert_eq!(fs::read(root.path().join("a")).unwrap(), b"short name");
    publish_output(root.path(), Path::new("nested/new.txt"), b"first").unwrap();
    assert_eq!(
        fs::read(root.path().join("nested/new.txt")).unwrap(),
        b"first"
    );
    publish_output(root.path(), Path::new("nested/new.txt"), b"second").unwrap();
    assert_eq!(
        fs::read(root.path().join("nested/new.txt")).unwrap(),
        b"second"
    );
}

#[test]
fn document_rename_rejects_pending_informational_and_failed_completion_statuses() {
    assert!(check_rename_completion(NTSTATUS(0), NTSTATUS(0)).is_ok());
    for other in [
        STATUS_PENDING,
        NTSTATUS(0x40000000),
        NTSTATUS(0xc000000d_u32 as i32),
    ] {
        assert!(check_rename_completion(other, NTSTATUS(0)).is_err());
        assert!(check_rename_completion(NTSTATUS(0), other).is_err());
    }
}

#[test]
fn document_long_unicode_paths_support_regular_reads_cache_and_atomic_publication() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("public resources 中文 日本語");
    let mut relative = PathBuf::new();
    for index in 0..5 {
        relative.push(format!("{index} 中文 空 格 {}", "n".repeat(45)));
    }
    relative.push("payload 中文 空 格.txt");
    let input = source.join(&relative);
    fs::create_dir_all(input.parent().unwrap()).unwrap();
    fs::write(&input, b"long path public fixture").unwrap();
    assert!(input.as_os_str().encode_wide().count() > 260);
    // std::fs creates this successfully using its own extended-path handling;
    // the raw Win32 no-reparse reader must address the very same object.
    let mut bytes = Vec::new();
    open_regular(&source, &input, 1024)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"long path public fixture");
    let capability = RuntimeCapability::new().unwrap();
    let cached = cache_runtime(&source, &capability, &current_user().unwrap()).unwrap();
    assert_eq!(
        tree_digest_at(&source, 1024).unwrap(),
        tree_digest_at(&cached.root, 1024).unwrap()
    );
    let output = temp.path().join("document output 中文 空 格");
    fs::create_dir(&output).unwrap();
    publish_output(&output, &relative, b"first version").unwrap();
    publish_output(&output, &relative, b"replacement version").unwrap();
    assert_eq!(
        fs::read(output.join(&relative)).unwrap(),
        b"replacement version"
    );
}

#[test]
fn document_cache_requires_exact_specific_file_read_execute_acl() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("immutable-public-cache");
    let user = current_user().unwrap();
    let capability = RuntimeCapability::new().unwrap();
    let cap = sid_string(capability.sid).unwrap();
    let exact = format!(
        "{}(A;OICI;FRFX;;;{cap})S:(ML;OICI;NW;;;ME)",
        private_sddl(&user)
    );
    create_directory(&root, &exact).unwrap();
    let pinned = owned_directory(&root, &user).unwrap();
    verify_cache_acl(&pinned, &exact).unwrap();
    let write_allowed = format!(
        "{}(A;OICI;FRFXFW;;;{cap})S:(ML;OICI;NW;;;ME)",
        private_sddl(&user)
    );
    let unsafe_root = temp.path().join("unsafe-writable-public-cache");
    create_directory(&unsafe_root, &write_allowed).unwrap();
    let unsafe_pinned = owned_directory(&unsafe_root, &user).unwrap();
    assert!(verify_cache_acl(&unsafe_pinned, &exact).is_err());
    for (name, changed) in [
        ("low-label", exact.replace(";;;ME)", ";;;LW)")),
        ("unprotected", exact.replace("D:P", "D:")),
        ("wrong-capability", exact.replace(&cap, "S-1-1-0")),
        ("extra-ace", exact.replace("S:", "(A;OICI;FRFX;;;WD)S:")),
    ] {
        let altered = temp.path().join(name);
        create_directory(&altered, &changed).unwrap();
        let altered_pinned = owned_directory(&altered, &user).unwrap();
        assert!(
            verify_cache_acl(&altered_pinned, &exact).is_err(),
            "an actual {name} cache must fail the exact ACL contract"
        );
    }
}

#[test]
fn document_acl_comparison_ignores_only_sacl_auto_inherited_on_owned_copies() {
    use windows::Win32::Security::SE_DACL_AUTO_INHERITED;
    let exact = "D:P(A;OICI;FA;;;SY)(A;OICI;FRFX;;;S-1-15-3-1)S:(ML;OICI;NW;;;ME)";
    let expected = security_descriptor(exact).unwrap();
    let expected_key = descriptor_comparison_key(PSECURITY_DESCRIPTOR(expected.0)).unwrap();
    let sacl_ai = security_descriptor(&exact.replace("S:", "S:AI")).unwrap();
    let original = descriptor_diagnostic(PSECURITY_DESCRIPTOR(sacl_ai.0)).unwrap();
    assert!(descriptor_comparison_key(PSECURITY_DESCRIPTOR(sacl_ai.0)).unwrap() == expected_key);
    assert_eq!(
        descriptor_diagnostic(PSECURITY_DESCRIPTOR(sacl_ai.0)).unwrap(),
        original,
        "canonicalization must not mutate the original snapshot"
    );
    let dacl_ai = security_descriptor(&exact.replace("D:P", "D:PAI")).unwrap();
    let dacl_key = descriptor_comparison_key(PSECURITY_DESCRIPTOR(dacl_ai.0)).unwrap();
    assert_eq!(dacl_key.0 ^ expected_key.0, SE_DACL_AUTO_INHERITED.0);
    assert!(
        dacl_key != expected_key,
        "DACL auto-inheritance is not normalized"
    );
    for changed in [
        exact.replace(";;;ME)", ";;;LW)"),
        exact.replace("D:P", "D:"),
        exact.replace("FRFX", "FRFXFW"),
        exact.replace("S-1-15-3-1", "S-1-15-3-2"),
        exact.replace("S:", "(A;OICI;FRFX;;;WD)S:"),
        exact.split("S:").next().unwrap().to_owned(),
    ] {
        let actual = security_descriptor(&changed).unwrap();
        assert!(
            descriptor_comparison_key(PSECURITY_DESCRIPTOR(actual.0)).unwrap() != expected_key,
            "permissions, identity, protection and label differences must remain exact"
        );
    }
}

#[test]
fn document_acl_diagnostics_keep_rights_and_flags_without_account_sid_values() {
    let text = "D:PAI(A;OICI;FRFX;;;S-1-5-21-111111-222222-333333-1001)S:AI(ML;OICI;NW;;;ME)";
    let shape = descriptor_shape(text);
    assert_eq!(shape["aces"][0]["rights"], "FRFX");
    assert_eq!(shape["aces"][0]["flags"], "OICI");
    assert_eq!(shape["aces"][0]["identityClass"]["kind"], "nt_account");
    assert_eq!(shape["aces"][1]["identityClass"]["wellKnownAlias"], "ME");
    let encoded = shape.to_string();
    for private in ["111111", "222222", "333333", "1001", "S-1-5-21-"] {
        assert!(!encoded.contains(private));
    }
}

#[test]
fn document_fixed_probe_diagnostics_keep_progress_without_private_output() {
    let mut trace = ProbeTrace::new();
    trace.engine(ProbeStage::NodeIsolation);
    let private =
        r"C:\Users\private-account\credentials S-1-5-21-111111-222222-333333-1001 condition-secret";
    let output = EngineOutput {
        status: Some(7),
        stdout: private.into(),
        stderr: format!("{private}\nOFOX_LPAC_PROBE_STAGE:9\nOFOX_LPAC_PROBE_STAGE:99\nOFOX_LPAC_PROBE_STAGE:{private}\n"),
        timed_out: false,
    };
    assert!(trace.success(output).is_err());
    let error = trace.failure(
        private,
        &["windowsBuild", "nativeCanvas", "condition-secret"],
    );
    let detail: serde_json::Value =
        serde_json::from_str(error.split_once(":probe ").unwrap().1).unwrap();
    assert_eq!(detail["stage"], "node_isolation");
    assert_eq!(detail["reason"], "other");
    assert_eq!(detail["engine"]["status"], 7);
    assert_eq!(detail["engine"]["timedOut"], false);
    assert_eq!(detail["engine"]["nodeCheckpointBeforeStep"], 9);
    assert_eq!(
        detail["completedChecks"],
        serde_json::json!(["windowsBuild", "nativeCanvas"])
    );
    for secret in [
        "private-account",
        "credentials",
        "111111",
        "222222",
        "333333",
        "condition-secret",
        "C:\\Users",
    ] {
        assert!(!error.contains(secret));
    }
    trace.engine(ProbeStage::PythonCreate);
    assert!(trace
        .success(EngineOutput {
            status: None,
            stdout: String::new(),
            stderr: private.into(),
            timed_out: true
        })
        .is_err());
    let error = trace.failure(UNAVAILABLE, &["windowsBuild"]);
    let detail: serde_json::Value =
        serde_json::from_str(error.split_once(":probe ").unwrap().1).unwrap();
    assert_eq!(detail["stage"], "python_create");
    assert_eq!(detail["engine"]["timedOut"], true);
    assert_eq!(detail["engine"]["status"], serde_json::Value::Null);
    assert_eq!(detail["reason"], "process_timeout");
    assert!(detail["engine"]["nodeCheckpointBeforeStep"].is_null());
    assert_eq!(
        detail["completedChecks"],
        serde_json::json!(["windowsBuild"])
    );
}

#[test]
fn document_fixed_probe_errors_keep_only_numeric_native_statuses() {
    let trace = ProbeTrace::new();
    let error = trace.failure("assistant_document_sandbox_win32_80070005_at_123", &[]);
    let detail: serde_json::Value =
        serde_json::from_str(error.split_once(":probe ").unwrap().1).unwrap();
    assert_eq!(detail["native"]["code"], "80070005");
    assert_eq!(detail["native"]["sourceLine"], 123);
    let error = trace.failure(
        "assistant_document_sandbox_win32_80070005_at_123 C:\\private\\secret",
        &[],
    );
    assert!(!error.contains("secret"));
    assert!(!error.contains("80070005"));
    let raw = format!("{UNAVAILABLE}:cache_acl_mismatch {{\"actual\":{{\"control\":\"9814\",\"revision\":1,\"private\":\"S-1-5-21-private condition-secret\"}},\"expected\":{{\"control\":\"9014\",\"revision\":1}}}}");
    let error = trace.failure(&raw, &[]);
    let detail: serde_json::Value =
        serde_json::from_str(error.split_once(":probe ").unwrap().1).unwrap();
    assert_eq!(detail["native"]["actualControl"], "9814");
    assert_eq!(detail["native"]["expectedControl"], "9014");
    assert!(!error.contains("private"));
    assert!(!error.contains("condition-secret"));
}

#[test]
fn document_fixed_probe_worker_failure_keeps_numeric_evidence_after_cleanup() {
    let mut trace = ProbeTrace::new();
    trace.engine(ProbeStage::NodeIsolation);
    let observation = trace.observation.clone();
    observation.enter(Phase::TokenMitigations);
    observation.evidence(|evidence| {
        evidence.app_container = Some(1);
        evidence.less_privileged = Some(1);
        evidence.package_matches = Some(true);
        evidence.capability_groups = Some(1);
        evidence.capability_matches = Some(true);
        evidence.integrity_subauthorities = Some(1);
        evidence.integrity_rid = Some(4096);
        evidence.child_policy = Some(1);
        evidence.image_policy = Some(0);
        evidence.native_status = Some(5);
        evidence.stderr_receive = Some(Receive::Disconnected);
    });
    observation.failed();
    observation.enter(Phase::TaskCleanup);
    observation.evidence(|evidence| evidence.task_cleanup = Some(true));
    observation.enter(Phase::ProfileCleanup);
    observation.evidence(|evidence| evidence.profile_cleanup = Some(true));
    let private =
        r"C:\Users\private-account\credentials S-1-5-21-111111-222222-333333-1001 condition-secret";
    let error = trace.failure(private, &["windowsBuild", "condition-secret"]);
    let detail: serde_json::Value =
        serde_json::from_str(error.split_once(":probe ").unwrap().1).unwrap();
    assert_eq!(detail["worker"]["firstFailure"], "token_mitigations");
    assert_eq!(detail["worker"]["stage"], "profile_cleanup");
    assert_eq!(detail["worker"]["evidence"]["imagePolicy"], 0);
    assert_eq!(detail["worker"]["evidence"]["nativeStatus"], 5);
    assert_eq!(
        detail["worker"]["evidence"]["stderrReceive"],
        "disconnected"
    );
    assert_eq!(detail["worker"]["evidence"]["taskCleanup"], true);
    assert_eq!(detail["worker"]["evidence"]["profileCleanup"], true);
    assert!(detail["engine"].is_null());
    assert_eq!(
        detail["completedChecks"],
        serde_json::json!(["windowsBuild"])
    );
    for secret in [
        "private-account",
        "credentials",
        "111111",
        "condition-secret",
        "C:\\Users",
    ] {
        assert!(!error.contains(secret));
    }
}

#[test]
fn document_fixed_probe_owner_mismatch_keeps_original_failure() {
    let root = tempfile::tempdir().unwrap();
    let user = current_user().unwrap();
    let path = root.path().join("owned-cache-fixture");
    create_directory(&path, &private_sddl(&user)).unwrap();
    let observation = Observer::fixed_probe();
    let error =
        owned_directory_observed(&path, "synthetic mismatched owner", &observation).unwrap_err();
    observation.failed();
    assert_eq!(error, UNAVAILABLE);
    assert_eq!(
        owned_directory(&path, "synthetic mismatched owner").unwrap_err(),
        error
    );
    let report = observation.snapshot().unwrap();
    assert_eq!(report["firstFailure"], "cache_owner_match");
    assert_eq!(report["evidence"]["cacheOwnerMatches"], false);
    assert!(!report.to_string().contains(&user));
}

#[test]
fn document_win32_paths_are_lexical_extended_absolute_paths() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    for (input, expected) in [
        (
            r"C:\public resources\中文\file",
            r"\\?\C:\public resources\中文\file",
        ),
        (
            r"C:/public resources/中文/file",
            r"\\?\C:\public resources\中文\file",
        ),
        (
            r"\\server\share\中文\file",
            r"\\?\UNC\server\share\中文\file",
        ),
        (
            r"//server/share/中文/file",
            r"\\?\UNC\server\share\中文\file",
        ),
        (
            r"\\?\C:\public resources\中文\file",
            r"\\?\C:\public resources\中文\file",
        ),
    ] {
        let encoded = wide_path(Path::new(input)).unwrap();
        assert_eq!(encoded.last(), Some(&0));
        assert_eq!(OsString::from_wide(&encoded[..encoded.len() - 1]), expected);
    }
    for invalid in [
        r"relative\file",
        r"C:relative\file",
        r"C:\public\..\secret",
        r"C:\public\.\file",
        r"\\?\C:\public/../secret",
        r"\\?\C:\public/normal/file",
        r"\\?\UNC\server\share\public/../secret",
        r"\\.\PhysicalDrive0",
    ] {
        assert!(wide_path(Path::new(invalid)).is_err());
    }
}

#[test]
fn document_workspace_cleanup_unlinks_junction_without_reading_target() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel"), b"preserved").unwrap();
    let cap = RuntimeCapability::new().unwrap();
    let user = current_user().unwrap();
    let mut profile = Profile::new(&cap).unwrap();
    profile.make_low_profile().unwrap();
    let mut workspace = task_directory(&user, profile.sid.0).unwrap();
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(workspace.0.join("outside-junction"))
        .arg(outside.path())
        .status()
        .unwrap();
    assert!(status.success());
    let task_path = workspace.0.clone();
    workspace.dispose().unwrap();
    profile.dispose().unwrap();
    assert!(!task_path.exists());
    assert_eq!(
        fs::read(outside.path().join("sentinel")).unwrap(),
        b"preserved"
    );
}

#[test]
fn document_cleanup_failure_is_reported_and_can_be_retried() {
    let capability = RuntimeCapability::new().unwrap();
    let user = current_user().unwrap();
    let mut profile = Profile::new(&capability).unwrap();
    profile.make_low_profile().unwrap();
    let mut workspace = task_directory(&user, profile.sid.0).unwrap();
    fs::write(workspace.0.join("locked"), b"owned test data").unwrap();
    let locked = open_regular(&workspace.0, &workspace.0.join("locked"), 1024).unwrap();
    assert_eq!(
        workspace.dispose().unwrap_err(),
        "assistant_document_sandbox_cleanup_failed"
    );
    drop(locked);
    workspace.dispose().unwrap();
    profile.dispose().unwrap();
    assert!(!workspace.0.exists());
}

#[tokio::test]
#[serial_test::serial]
async fn failed_profile_cleanup_revokes_admission_before_another_engine_can_start() {
    // A synthetic executable must never be launched. Its deliberately seeded
    // proof lets this exercise cleanup revocation without a paid/model run.
    let public = tempfile::tempdir().unwrap();
    fs::create_dir(public.path().join("node")).unwrap();
    let node = public.path().join("node/node.exe");
    fs::write(&node, b"synthetic executable; must never launch").unwrap();
    let capability = RuntimeCapability::new().unwrap();
    let cached = cache_runtime(public.path(), &capability, &current_user().unwrap()).unwrap();
    ADMITTED
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(cached.source, cached.digest);
    assert!(windows_document_engine_proven(public.path()));
    let mut profile = Profile::new(&capability).unwrap();
    profile.make_low_profile().unwrap();
    let folder = profile.directory.as_ref().unwrap().clone();
    let marker = folder.join("held-open-test-file");
    fs::write(&marker, b"synthetic lock").unwrap();
    let locked = open_regular(&folder, &marker, 1024).unwrap();
    assert_eq!(
        dispose_profile(&mut profile).unwrap_err(),
        "assistant_document_sandbox_cleanup_failed"
    );
    assert!(!windows_document_engine_proven(public.path()));
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    assert_eq!(
        run_windows_engine_limited(
            &node,
            &[],
            task.path(),
            &home,
            public.path(),
            Duration::from_secs(1),
            1024
        )
        .await
        .unwrap_err(),
        "assistant_document_sandbox_unproven_runtime"
    );
    drop(locked);
    dispose_profile(&mut profile).unwrap();
}

#[tokio::test]
#[serial_test::serial]
async fn document_child_crash_cleans_task_and_profile_before_receipt() {
    let root = runtime();
    ensure_windows_document_engine_ready(&root).await.unwrap();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    let sequence = RUN_SEQUENCE.load(Ordering::Acquire);
    let out = run_windows_engine_limited(&root.join("node/node.exe"),
        &["-e", "setTimeout(() => require('node:fs').writeFileSync('late', 'bad'), 2000); process.abort();"],
        task.path(), &home, &root, Duration::from_secs(15), 4096).await.unwrap();
    assert!(out.status.is_some_and(|status| status != 0));
    let receipts = windows_document_sandbox_receipts(sequence);
    let receipt = receipts
        .iter()
        .find(|row| row["trustedProbe"] == false)
        .unwrap();
    assert_eq!(receipt["cleanupComplete"], true);
    assert!(!std::env::temp_dir()
        .join(receipt["taskId"].as_str().unwrap())
        .exists());
    assert!(!task.path().join("late").exists());
}

#[tokio::test]
#[serial_test::serial]
async fn cancelling_an_active_document_job_cleans_before_receipt_and_no_late_write() {
    let root = runtime();
    ensure_windows_document_engine_ready(&root).await.unwrap();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    let canonical = task.path().canonicalize().unwrap();
    let original = task.path().to_path_buf();
    let sequence = RUN_SEQUENCE.load(Ordering::Acquire);
    let running = tokio::spawn(async move {
        run_windows_engine_limited(&root.join("node/node.exe"),
            &["-e", "setTimeout(() => require('node:fs').writeFileSync('late', 'bad'), 4000); setInterval(() => {}, 1000);"],
            &original, &home, &root, Duration::from_secs(30), 4096).await
    });
    let sandbox = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let active = ACTIVE_TASKS
                .get_or_init(Default::default)
                .lock()
                .unwrap()
                .get(&canonical)
                .cloned();
            if let Some(path) = active {
                break path;
            }
            assert!(
                !running.is_finished(),
                "the controlled job failed before starting"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    running.abort();
    let _ = running.await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while ACTIVE_TASKS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .contains_key(&canonical)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!sandbox.exists());
    assert!(!task.path().join("late").exists());
    let receipts = windows_document_sandbox_receipts(sequence);
    let row = receipts
        .iter()
        .find(|row| row["taskId"].as_str() == sandbox.file_name().and_then(OsStr::to_str))
        .unwrap();
    assert_eq!(row["cancelled"], true);
    assert_eq!(row["cleanupComplete"], true);
}

#[tokio::test]
#[serial_test::serial]
async fn cancelling_after_exit_stops_all_copyback_publication() {
    cancel_at_copyback(CopybackPoint::Drained).await;
}

#[tokio::test]
#[serial_test::serial]
async fn cancelling_between_copyback_files_stops_remaining_publication() {
    cancel_at_copyback(CopybackPoint::Published(1)).await;
}

#[tokio::test]
#[serial_test::serial]
async fn cancelling_after_staging_fsync_does_not_publish_the_staged_file() {
    cancel_at_copyback(CopybackPoint::BeforeRename).await;
}

async fn cancel_at_copyback(point: CopybackPoint) {
    let regression = match point {
        CopybackPoint::Drained => "COPYBACK_CANCEL_REGRESSION_AFTER_EXIT",
        CopybackPoint::Published(_) => "COPYBACK_CANCEL_REGRESSION_BETWEEN_FILES",
        CopybackPoint::BeforeRename => "COPYBACK_CANCEL_REGRESSION_BEFORE_RENAME",
    };
    let root = runtime();
    ensure_windows_document_engine_ready(&root).await.unwrap();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    let canonical = task.path().canonicalize().unwrap();
    let (reached, started) = std::sync::mpsc::sync_channel(1);
    let (resume, resumed) = std::sync::mpsc::sync_channel(1);
    COPYBACK_GATES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(
            canonical.clone(),
            Arc::new(CopybackGate {
                point,
                reached,
                resume: Mutex::new(resumed),
            }),
        );
    let original = task.path().to_path_buf();
    let runtime = root.clone();
    let sequence = RUN_SEQUENCE.load(Ordering::Acquire);
    let running = tokio::spawn(async move {
        run_windows_engine_limited(&runtime.join("node/node.exe"),
                &["-e", "const f=require('node:fs'); f.writeFileSync('a.txt','first'); f.writeFileSync('b.txt','second');"],
                &original, &home, &runtime, Duration::from_secs(30), 4096).await
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(Duration::from_secs(60)))
        .await
        .unwrap()
        .expect("controlled successful child must reach the copyback handshake");
    let sandbox = ACTIVE_TASKS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .get(&canonical)
        .cloned()
        .unwrap();
    running.abort();
    let _ = running.await;
    resume.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while ACTIVE_TASKS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .contains_key(&canonical)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    COPYBACK_GATES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .remove(&canonical);
    assert_eq!(
        task.path().join("a.txt").exists(),
        point == CopybackPoint::Published(1),
        "{regression}: first output was published after cancellation"
    );
    assert!(
        !task.path().join("b.txt").exists(),
        "{regression}: remaining output was published after cancellation"
    );
    assert!(fs::read_dir(task.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".ofox-publish-")));
    assert!(!sandbox.exists());
    let receipts = windows_document_sandbox_receipts(sequence);
    let row = receipts
        .iter()
        .find(|row| row["taskId"].as_str() == sandbox.file_name().and_then(OsStr::to_str))
        .unwrap();
    assert_eq!(
        row["cancelled"], true,
        "{regression}: cancellation was lost"
    );
    assert_eq!(
        row["status"],
        serde_json::Value::Null,
        "{regression}: cancelled copyback reported successful completion"
    );
    assert_eq!(row["cleanupComplete"], true);
}

#[tokio::test]
#[serial_test::serial]
async fn public_runner_refuses_unproven_or_changed_runtime_before_launch() {
    let public = tempfile::tempdir().unwrap();
    fs::create_dir(public.path().join("node")).unwrap();
    let node = public.path().join("node/node.exe");
    fs::write(&node, b"synthetic executable; must never launch").unwrap();
    let task = tempfile::tempdir().unwrap();
    let home = task.path().join("home");
    fs::create_dir(&home).unwrap();
    let run = || {
        run_windows_engine_limited(
            &node,
            &[],
            task.path(),
            &home,
            public.path(),
            Duration::from_secs(1),
            1024,
        )
    };
    assert_eq!(
        run().await.unwrap_err(),
        "assistant_document_sandbox_unproven_runtime"
    );
    let cap = RuntimeCapability::new().unwrap();
    let cached = cache_runtime(public.path(), &cap, &current_user().unwrap()).unwrap();
    ADMITTED
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(cached.source.clone(), cached.digest);
    fs::write(
        &node,
        b"changed synthetic executable; must still never launch",
    )
    .unwrap();
    assert_eq!(
        run().await.unwrap_err(),
        "assistant_document_sandbox_unproven_runtime"
    );
    ADMITTED
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .remove(&cached.source);
}

#[test]
fn warm_public_cache_rejects_changed_cached_bytes() {
    let public = tempfile::tempdir().unwrap();
    fs::write(
        public.path().join("resource"),
        format!("original public asset {}", uuid::Uuid::new_v4()),
    )
    .unwrap();
    let capability = RuntimeCapability::new().unwrap();
    let user = current_user().unwrap();
    let cached = cache_runtime(public.path(), &capability, &user).unwrap();
    fs::write(cached.root.join("resource"), b"corrupt cached public asset").unwrap();
    assert!(cache_runtime(public.path(), &capability, &user).is_err());
    // Clean only this test's known complete public cache, not another task.
    fs::remove_dir_all(&cached.root).unwrap();
}
