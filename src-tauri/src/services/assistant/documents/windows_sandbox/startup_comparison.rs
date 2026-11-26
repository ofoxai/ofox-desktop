//! Fixed test-only A-B-A. No model input, debugger, admission or user data.
use super::*;

const MARKER: &[u8] = b"OFOX_SYSTEM_INIT_V1";

fn stop(process: &Process) -> Result<(), String> {
    unsafe {
        win(TerminateJobObject(raw(&process.job), 1))?;
        if WaitForSingleObject(raw(&process.process), 5000) != WAIT_OBJECT_0 {
            return Err(UNAVAILABLE.into());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        unsafe {
            win(QueryInformationJobObject(
                Some(raw(&process.job)),
                JobObjectBasicAccountingInformation,
                (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            ))?;
        }
        if accounting.ActiveProcesses == 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(UNAVAILABLE.into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn case(
    node: &Path,
    task: &Path,
    home: &Path,
    profile: &Profile,
    caps: &ProcessCapabilities<'_>,
    console: ConsoleMode,
    winsock_grant: bool,
) -> serde_json::Value {
    let observation = Observer::fixed_probe(); // Fresh; never request_loader.
    let entries = caps.entries();
    let mut phase = "create";
    let mut cleanup = false;
    let mut token_verified = false;
    let mut attrs = Vec::new();
    let mut previous = None;
    let mut status = None;
    let mut timed_out = false;
    let mut marker = false;
    let mut stdout_bytes = None;
    let mut stderr_bytes = None;
    let mut stderr_shape = None;
    let result: Result<(), String> = (|| {
        let SuspendedLaunch {
            process,
            thread,
            stdin_read,
            stdout_read,
            stdout_write,
            stderr_read,
            stderr_write,
            ..
        } = create_suspended_capabilities(
            node,
            &[
                "-e".into(),
                "process.stdout.write('OFOX_SYSTEM_INIT_V1')".into(),
            ],
            task,
            home,
            profile,
            caps,
            ContainerPolicy::Lpac,
            console,
            &observation,
            None,
        )?;
        drop(stdin_read);
        drop(stdout_write);
        drop(stderr_write);
        let stdout = drain(stdout_read, 1024);
        let stderr = drain(stderr_read, 1024);
        let executed: Result<(), String> = (|| {
            phase = "token";
            verify_token_entries(
                raw(&process.process),
                profile.sid.0,
                caps.runtime.sid,
                &entries,
                if entries.len() == 2 {
                    RegistryScope::RegistryRead
                } else {
                    RegistryScope::RuntimeOnly {
                        winsock_readable: winsock_grant,
                    }
                },
                &observation,
            )?;
            let mut token = HANDLE::default();
            unsafe {
                win(OpenProcessToken(
                    raw(&process.process),
                    TOKEN_QUERY,
                    &mut token,
                ))?;
            }
            let token = handle(token);
            let (data, bytes) =
                token_buffer_sized_observed(raw(&token), TokenCapabilities, &Observer::default())?;
            let required = std::mem::offset_of!(TOKEN_GROUPS, Groups)
                + entries.len() * size_of::<SID_AND_ATTRIBUTES>();
            if bytes < size_of::<TOKEN_GROUPS>() || bytes < required {
                return Err(UNAVAILABLE.into());
            }
            let groups = unsafe { &*data.as_ptr().cast::<TOKEN_GROUPS>() };
            if groups.GroupCount as usize != entries.len() {
                return Err(UNAVAILABLE.into());
            }
            attrs = unsafe { std::slice::from_raw_parts(groups.Groups.as_ptr(), entries.len()) }
                .iter()
                .map(|g| g.Attributes)
                .collect();
            if attrs.iter().any(|value| *value != SE_GROUP_ENABLED as u32) {
                return Err(UNAVAILABLE.into());
            }
            token_verified = true;
            phase = "resume";
            let resumed = unsafe { ResumeThread(raw(&thread)) };
            previous = Some(resumed);
            if resumed != 1 {
                return Err(UNAVAILABLE.into());
            }
            phase = "wait";
            let waited = unsafe { WaitForSingleObject(raw(&process.process), 10000) };
            timed_out = waited == WAIT_TIMEOUT;
            if waited != WAIT_OBJECT_0 {
                return Err(UNAVAILABLE.into());
            }
            let mut code = 0;
            unsafe {
                win(GetExitCodeProcess(raw(&process.process), &mut code))?;
            }
            status = Some(code);
            Ok(())
        })();
        // Never trust Drop as a cleanup receipt, even when verification failed.
        cleanup = stop(&process).is_ok();
        let out = stdout.recv_timeout(Duration::from_secs(2));
        let err = stderr.recv_timeout(Duration::from_secs(2));
        cleanup &= out.is_ok() && err.is_ok();
        if let Ok(out) = out {
            marker = out == MARKER;
            stdout_bytes = Some(out.len());
        }
        if let Ok(err) = err {
            stderr_bytes = Some(err.len());
            // Closed-set kind/call/code only; the text never leaves the runner.
            stderr_shape = Some(startup_comparison_protocol::stderr_shape(&err));
        }
        if !cleanup {
            return Err(UNAVAILABLE.into());
        }
        executed
    })();
    let evidence = observation.snapshot().unwrap_or_default();
    // The exact flags passed to CreateProcessW, so a label cannot misstate them.
    let flags = console.creation_flags().0;
    let console = match console {
        ConsoleMode::Hidden => "hidden",
        ConsoleMode::Detached => "detached",
    };
    serde_json::json!({"extraRegistryRead": entries.len()==2, "consoleMode":console, "creationFlags":flags,
        "winsockGrant":winsock_grant, "grantRestored":serde_json::Value::Null, "completed":result.is_ok(),
        "stage": if result.is_ok() {"complete"} else {phase}, "tokenVerified":token_verified,
        "capabilityCount":evidence["evidence"]["capabilityGroups"], "capabilityAttributes":attrs,
        "lpacAccessMatched":evidence["evidence"]["lpacAccessProof"]["matched"],
        "childPolicy":evidence["evidence"]["childPolicy"], "imagePolicy":evidence["evidence"]["imagePolicy"], "integrityRid":evidence["evidence"]["integrityRid"],
        "resumePrevious":previous, "exitStatus":status, "timedOut":timed_out,
        "markerMatched":marker, "stdoutBytes":stdout_bytes, "stderrBytes":stderr_bytes, "stderrShape":stderr_shape, "cleanupConfirmed":cleanup})
}

pub(super) fn compare(
    root: &Path,
    candidate: startup_comparison_protocol::Candidate,
) -> serde_json::Value {
    let mut profile = None;
    let mut task = None;
    let mut rows = Vec::new();
    let mut runtime_stable = false;
    // None: not this candidate; Some(false): the CI-only ACE could not be set.
    let mut grant_applied = None;
    // Only when apply failed: whether the original catalog DACLs were proven.
    let mut grant_failure_restored = None;
    let result: Result<bool, String> = (|| {
        let cap = RuntimeCapability::new()?;
        let user = current_user()?;
        let runtime = cache_runtime(root, &cap, &user)?;
        profile = Some(Profile::new(&cap)?);
        let profile = profile.as_mut().ok_or(UNAVAILABLE)?;
        profile.make_low_profile()?;
        task = Some(task_directory(&user, profile.sid.0)?);
        let folder = &task.as_ref().ok_or(UNAVAILABLE)?.0;
        let home = folder.join("home");
        io(fs::create_dir(&home))?;
        let node = runtime.root.join("node/node.exe");
        let plan = candidate.plan();
        let console = |detached: bool| {
            if detached {
                ConsoleMode::Detached
            } else {
                ConsoleMode::Hidden
            }
        };
        rows.push(case(
            &node,
            folder,
            &home,
            profile,
            &ProcessCapabilities::runtime_only(&cap),
            console(plan.base_detached),
            false,
        ));
        if !startup_comparison_protocol::baseline(&rows[0], candidate) {
            return Ok(false);
        } // No candidate unless A1 reproduces the failure it addresses.
          // B changes exactly one launch condition from A1.
        let caps = if plan.extra {
            ProcessCapabilities::production(&cap)?
        } else {
            ProcessCapabilities::runtime_only(&cap)
        };
        if plan.winsock_grant {
            // The ACE exists only for B's launch; A2 runs on the restored DACL.
            grant_applied = Some(false);
            let mut grant = match winsock_grant::WinsockCatalogGrant::apply(cap.sid) {
                Ok(grant) => grant,
                Err(failure) => {
                    grant_failure_restored = Some(failure.restored);
                    return Err(UNAVAILABLE.into());
                }
            };
            grant_applied = Some(true);
            rows.push(case(
                &node,
                folder,
                &home,
                profile,
                &caps,
                console(plan.detached),
                true,
            ));
            rows[1]["grantRestored"] = serde_json::json!(grant.restore());
        } else {
            rows.push(case(
                &node,
                folder,
                &home,
                profile,
                &caps,
                console(plan.detached),
                false,
            ));
        }
        if rows[1]["completed"] != true || rows[1]["cleanupConfirmed"] != true {
            return Ok(false);
        }
        rows.push(case(
            &node,
            folder,
            &home,
            profile,
            &ProcessCapabilities::runtime_only(&cap),
            console(plan.base_detached),
            false,
        ));
        runtime_stable = tree_digest_at(&runtime.root, MAX_TREE_BYTES)? == runtime.digest;
        Ok(runtime_stable && startup_comparison_protocol::supported(&rows, candidate))
    })();
    let task_clean = task.as_mut().is_none_or(|task| task.dispose().is_ok());
    let profile_clean = profile
        .as_mut()
        .is_none_or(|profile| profile.dispose().is_ok());
    serde_json::json!({"schemaVersion":1,"scope":"fixed-node-startup-launch-comparison",
        "diagnosticOnly":true,"releaseGate":false,"runtimePolicyChanged":false,"admissionGranted":false,
        "candidate":candidate.name(),"winsockGrantApplied":grant_applied,"winsockGrantFailureRestored":grant_failure_restored,"sameProfileAndInputs":rows.len()==3,"runtimeStable":runtime_stable,"debuggerUsed":false,
        "status":if result == Ok(true) && task_clean && profile_clean {"supports-candidate"} else {"inconclusive"},
        "taskCleanup":task_clean,"profileCleanup":profile_clean,"rows":rows})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn console_modes_differ_only_in_the_console_flag() {
        let hidden = ConsoleMode::Hidden.creation_flags();
        let detached = ConsoleMode::Detached.creation_flags();
        assert_eq!(
            hidden,
            CREATE_NO_WINDOW
                | CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT
        );
        assert_eq!(
            hidden.0 ^ detached.0,
            (CREATE_NO_WINDOW | windows::Win32::System::Threading::DETACHED_PROCESS).0
        );
        assert_eq!(hidden.0, startup_comparison_protocol::HIDDEN_FLAGS);
        assert_eq!(detached.0, startup_comparison_protocol::DETACHED_FLAGS);
    }
}
