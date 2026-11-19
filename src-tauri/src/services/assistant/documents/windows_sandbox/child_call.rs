//! Dedicated CI diagnostic for the single, embedded checkpoint-7 Node request.
//! The target process and primary thread come directly from create_verified.
use super::*;
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
};
use windows::Win32::System::Pipes::PeekNamedPipe;

const BEGIN: &str = "OFOX_CHILD_BEGIN\n";
pub(super) const COMPLETE: &str = "OFOX_CHILD_BEGIN\nOFOX_CHILD_END\n";
const VERDICTS: &[&str] = &[
    "not-reached",
    "succeeded",
    "child-process-blocked",
    "access-denied",
    "policy-disabled",
    "appcontainer-unsupported",
    "privilege-not-held",
    "file-not-found",
    "path-not-found",
    "invalid-handle",
    "other-win32",
];

pub(super) fn requested() -> bool {
    [
        ("GITHUB_ACTIONS", "true"),
        ("GITHUB_WORKFLOW", "Document Engine Child Call Diagnostic"),
        ("GITHUB_REF", "refs/heads/codex/desktop-next"),
        ("OFOX_DOCUMENT_CHILD_CALL_TRACE", "1"),
    ]
    .iter()
    .all(|(key, value)| std::env::var(key).as_deref() == Ok(*value))
        && matches!(
            std::env::var("GITHUB_EVENT_NAME").as_deref(),
            Ok("push" | "workflow_dispatch")
        )
        && [
            "OFOX_DOCUMENT_STARTUP_COMPARISON",
            "OFOX_DOCUMENT_STARTUP_TRACE",
            cdb::ENV_ROOT,
        ]
        .iter()
        .all(|key| std::env::var_os(key).is_none())
}

pub(super) fn verdict(line: &str) -> Option<&'static str> {
    VERDICTS.iter().copied().find(|value| *value == line)
}

pub(super) fn instrument(script: &str) -> Result<String, String> {
    const START: &str = "checkpoint(7);";
    const END: &str = r#"console.error("OFOX_LPAC_CHILD_RESULT:" + childResult);"#;
    if script.matches(START).count() != 1 || script.matches(END).count() != 1 {
        return Err(UNAVAILABLE.into());
    }
    // Both writes are synchronous. At the actual spawn stdin is again the
    // inherited pipe with no writer, just like the unassisted production probe.
    Ok(script
        .replace(
            START,
            r#"checkpoint(7);
fs.writeSync(1, "OFOX_CHILD_BEGIN\n");
const permit = Buffer.alloc(1);
assert.equal(fs.readSync(0, permit, 0, 1), 1);
assert.equal(permit[0], 71);
assert.equal(fs.readSync(0, permit, 0, 1), 0);"#,
        )
        .replace(
            END,
            &format!(
                "{END}\n{}",
                r#"fs.writeSync(1, "OFOX_CHILD_END\n");
if (childResult !== "UNKNOWN") process.exit(2);"#
            ),
        ))
}

fn check_time(deadline: Instant, cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        return Err(CANCELLED.into());
    }
    if Instant::now() >= deadline {
        return Err(UNAVAILABLE.into());
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ControllerCleanup {
    job_terminated: bool,
    controller_reaped: bool,
    job_empty: bool,
    scratch_removed: bool,
}

impl ControllerCleanup {
    fn failure_receipt(self, result: &Result<&str, String>, failure_stage: &str) -> Option<String> {
        if self.job_terminated && self.controller_reaped && self.job_empty && self.scratch_removed {
            return None;
        }
        // The controller result predates cleanup and is never a final receipt here.
        let before = result.as_ref().copied().unwrap_or(failure_stage);
        Some(format!(
            "OFOX_CHILD_CALL_CLEANUP_V1 {before} {} {} {} {}",
            self.job_terminated, self.controller_reaped, self.job_empty, self.scratch_removed
        ))
    }
}

#[test]
fn fixed_child_cleanup_receipt_preserves_the_controller_result_and_each_operation() {
    let clean = ControllerCleanup {
        job_terminated: true,
        controller_reaped: true,
        job_empty: true,
        scratch_removed: true,
    };
    let observed = Ok("child-process-blocked");
    let failed = Err("private controller error".to_owned());
    assert_eq!(clean.failure_receipt(&observed, "exit"), None);
    assert_eq!(clean.failure_receipt(&failed, "symbols"), None);
    for (outcome, expected) in [
        (
            ControllerCleanup {
                job_terminated: false,
                ..clean
            },
            "false true true true",
        ),
        (
            ControllerCleanup {
                controller_reaped: false,
                ..clean
            },
            "true false true true",
        ),
        (
            ControllerCleanup {
                job_empty: false,
                ..clean
            },
            "true true false true",
        ),
        (
            ControllerCleanup {
                scratch_removed: false,
                ..clean
            },
            "true true true false",
        ),
    ] {
        assert_eq!(
            outcome.failure_receipt(&observed, "exit"),
            Some(format!(
                "OFOX_CHILD_CALL_CLEANUP_V1 child-process-blocked {expected}"
            ))
        );
        assert_eq!(
            outcome.failure_receipt(&failed, "symbols"),
            Some(format!("OFOX_CHILD_CALL_CLEANUP_V1 symbols {expected}"))
        );
    }
}

pub(super) fn observe(
    pid: u32,
    tid: u32,
    exe: &Path,
    stdout: &OwnedHandle,
    input: OwnedHandle,
    cancelled: &AtomicBool,
) -> Result<&'static str, String> {
    // A controller inheriting the writer would keep the fixture's EOF pending.
    let mut flags = 0;
    unsafe {
        win(windows::Win32::Foundation::GetHandleInformation(
            raw(&input),
            &mut flags,
        ))?;
    }
    if flags & HANDLE_FLAG_INHERIT.0 != 0 {
        return Err(UNAVAILABLE.into());
    }
    // SymChk has a separate 30-second bound before CDB attaches.
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut prefix = [0; BEGIN.len()];
    loop {
        check_time(deadline, cancelled)?;
        let (mut read, mut available) = (0, 0);
        unsafe {
            win(PeekNamedPipe(
                raw(stdout),
                Some(prefix.as_mut_ptr().cast()),
                prefix.len() as u32,
                Some(&mut read),
                Some(&mut available),
                None,
            ))?;
        }
        if available as usize > prefix.len()
            || !BEGIN.as_bytes().starts_with(&prefix[..read as usize])
        {
            return Err(UNAVAILABLE.into());
        }
        if read as usize == prefix.len() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    // The controller waits for our private stdin before doing any work. Assign
    // it to a kill-on-close Job before supplying the identity of our own child.
    let job = unsafe { handle(win(CreateJobObjectW(None, PCWSTR::null()))?) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        win(SetInformationJobObject(
            raw(&job),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ))?;
    }
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/document-child-call-cdb.ps1");
    let pwsh = PathBuf::from(std::env::var_os("ProgramFiles").ok_or(UNAVAILABLE)?)
        .join("PowerShell/7/pwsh.exe");
    let scratch = io(tempfile::tempdir())?;
    let mut child = io(Command::new(pwsh)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(script)
        .current_dir(scratch.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn())?;
    let mut failure_stage = "arm";
    let result = (|| {
        unsafe {
            win(AssignProcessToJobObject(
                raw(&job),
                HANDLE(child.as_raw_handle()),
            ))?;
        }
        let identity =
            serde_json::json!({"process":pid, "thread":tid, "host":std::process::id(), "exe":exe});
        let mut channel = child.stdin.take().ok_or(UNAVAILABLE)?;
        io(writeln!(channel, "{identity}"))?;
        drop(channel);
        let pipe = child.stdout.take().ok_or(UNAVAILABLE)?;
        let (tx, rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            // Two short ASCII lines are the entire controller protocol.
            for line in BufReader::new(pipe.take(256)).lines().take(3) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut input = Some(input);
        let mut observed = None;
        loop {
            check_time(deadline, cancelled)?;
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(line)) if line.starts_with("INCOMPLETE ") => {
                    failure_stage = [
                        "target",
                        "import",
                        "symbols",
                        "arm",
                        "observation",
                        "request",
                        "return-pair",
                        "last-error",
                        "last-error-zero",
                        "last-error-unreadable",
                        "exit",
                    ]
                    .into_iter()
                    .find(|phase| line.strip_prefix("INCOMPLETE ") == Some(*phase))
                    .unwrap_or("native-test");
                    return Err(UNAVAILABLE.into());
                }
                Ok(Ok(line)) if line == "ARMED" && input.is_some() => {
                    failure_stage = "observation";
                    io(File::from(input.take().ok_or(UNAVAILABLE)?).write_all(b"G"))?;
                }
                Ok(Ok(line)) if input.is_none() && observed.is_none() => {
                    observed = Some(verdict(&line).ok_or(UNAVAILABLE)?);
                    failure_stage = "exit";
                }
                Ok(_) => return Err(UNAVAILABLE.into()),
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        reader.join().map_err(|_| UNAVAILABLE)?;
        loop {
            check_time(deadline, cancelled)?;
            if let Some(status) = io(child.try_wait())? {
                return observed
                    .filter(|_| status.success())
                    .ok_or_else(|| UNAVAILABLE.to_owned());
            }
            thread::sleep(Duration::from_millis(10));
        }
    })();
    // Assignment may have failed. Reap the controller in that case too. Keep
    // the Job handle until its descendants have released the private directory.
    let terminated = unsafe { win(TerminateJobObject(raw(&job), 1)) };
    let _ = child.kill();
    let reaped = io(child.wait());
    let empty = wait_for_empty_job(&job, |_| {});
    drop(job);
    let removed = io(scratch.close());
    let cleanup = ControllerCleanup {
        job_terminated: terminated.is_ok(),
        controller_reaped: reaped.is_ok(),
        job_empty: empty.is_ok(),
        scratch_removed: removed.is_ok(),
    };
    if let Some(receipt) = cleanup.failure_receipt(&result, failure_stage) {
        eprintln!("{receipt}");
        return Err(UNAVAILABLE.into());
    }
    if result.is_err() {
        eprintln!("OFOX_CHILD_CALL_INCOMPLETE_V1 {failure_stage}");
    }
    result
}
