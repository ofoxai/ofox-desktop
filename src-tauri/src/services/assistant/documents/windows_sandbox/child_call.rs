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

#[derive(Default)]
struct ControllerReplies {
    armed: bool,
    paired: Option<&'static str>,
    terminal: Option<Result<&'static str, &'static str>>,
}

impl ControllerReplies {
    // Only ARMED releases the fixture. A paired return is provisional until
    // the controller exits successfully and the original Node test finishes.
    fn accept(&mut self, line: &str) -> Result<(), ()> {
        if self.terminal.is_some() {
            return Err(());
        }
        if let Some(stage) = line.strip_prefix("INCOMPLETE ") {
            let stages = [
                "target",
                "import",
                "symbols",
                "arm",
                "observation",
                "request",
                "return-pair",
                "return-conflict",
                "return-observer-error",
                "last-error",
                "last-error-zero",
                "last-error-unreadable",
                "exit",
                "exit-marker",
                "exit-wait",
                "exit-status",
                "exit-stderr",
            ];
            let stage = stages.into_iter().find(|v| *v == stage).ok_or(())?;
            if matches!(stage, "return-conflict" | "return-observer-error") {
                self.paired.take().ok_or(())?;
            } else if self.paired.is_some() && !stage.starts_with("exit") {
                return Err(());
            }
            self.terminal = Some(Err(stage));
        } else if line == "ARMED" && !self.armed {
            self.armed = true;
        } else if !self.armed {
            return Err(());
        } else if let Some(value) = line.strip_prefix("PAIRED ") {
            if self.paired.is_some() {
                return Err(());
            }
            self.paired = Some(verdict(value).filter(|v| *v != "not-reached").ok_or(())?);
        } else {
            let value = verdict(line).ok_or(())?;
            if self.paired != (value != "not-reached").then_some(value) {
                return Err(());
            }
            self.terminal = Some(Ok(value));
        }
        Ok(())
    }
}

#[test]
fn fixed_child_paired_return_requires_an_armed_consistent_protocol() {
    let mut r = ControllerReplies::default();
    assert!(r.accept("PAIRED access-denied").is_err());
    assert!(r.accept("INCOMPLETE return-conflict").is_err());
    r.accept("ARMED").unwrap();
    assert!(r.accept("PAIRED not-reached").is_err());
    assert!(r.accept("PAIRED private-path").is_err());
    r.accept("PAIRED access-denied").unwrap();
    assert_eq!(r.paired, Some("access-denied"));
    assert_eq!(r.terminal, None);
    assert!(r.accept("PAIRED access-denied").is_err());
    assert!(r.accept("child-process-blocked").is_err());
    assert!(r.accept("INCOMPLETE symbols").is_err());
    r.accept("INCOMPLETE exit-marker").unwrap();
    assert_eq!(r.terminal, Some(Err("exit-marker")));
    assert!(r.accept("access-denied").is_err());
    for stage in ["return-conflict", "return-observer-error"] {
        let mut invalidated = ControllerReplies::default();
        invalidated.accept("ARMED").unwrap();
        invalidated.accept("PAIRED access-denied").unwrap();
        invalidated.accept(&format!("INCOMPLETE {stage}")).unwrap();
        assert_eq!(invalidated.paired, None);
        assert_eq!(invalidated.terminal, Some(Err(stage)));
    }
    for value in ["child-process-blocked", "not-reached"] {
        let mut r = ControllerReplies::default();
        r.accept("ARMED").unwrap();
        if value != "not-reached" {
            assert!(r.accept(value).is_err());
            r.accept(&format!("PAIRED {value}")).unwrap();
        }
        r.accept(value).unwrap();
        assert_eq!(r.terminal, Some(Ok(value)));
    }
}

fn remove_controller_scratch(scratch: tempfile::TempDir) -> &'static str {
    // TempDir::close wraps the native error with its path. Perform its one
    // removal operation directly so only the native classification is kept.
    match fs::remove_dir_all(scratch.keep()) {
        Ok(()) => "removed",
        Err(error) => match error.raw_os_error() {
            Some(5) => "access-denied",
            Some(32) => "sharing-violation",
            Some(33) => "lock-violation",
            Some(145) => "directory-not-empty",
            Some(2 | 3) => "not-found",
            _ => "other",
        },
    }
}

#[test]
fn fixed_child_scratch_removal_classifies_an_exclusive_file_handle() {
    use std::os::windows::fs::OpenOptionsExt;
    let parent = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir_in(parent.path()).unwrap();
    assert_eq!(remove_controller_scratch(scratch), "removed");
    let scratch = tempfile::tempdir_in(parent.path()).unwrap();
    let path = scratch.path().join("fixture");
    let held = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let reason = remove_controller_scratch(scratch);
    drop(held);
    parent.close().unwrap();
    assert_eq!(reason, "sharing-violation");
}

#[derive(Clone, Copy)]
struct ControllerCleanup {
    job_terminated: bool,
    controller_reaped: bool,
    job_empty: bool,
    removal: &'static str,
}

impl ControllerCleanup {
    fn failure_receipt(
        self,
        result: &Result<&str, String>,
        failure_stage: &str,
        paired: Option<&str>,
    ) -> Option<String> {
        if result.is_ok()
            && self.job_terminated
            && self.controller_reaped
            && self.job_empty
            && self.removal == "removed"
        {
            return None;
        }
        // The controller result predates cleanup and is never a final receipt here.
        let before = result.as_ref().copied().unwrap_or(failure_stage);
        let paired = paired.unwrap_or("none");
        Some(format!(
            "OFOX_CHILD_CALL_FAILURE_V2 {before} {paired} {} {} {} {}",
            self.job_terminated, self.controller_reaped, self.job_empty, self.removal
        ))
    }
}

#[test]
fn fixed_child_cleanup_receipt_preserves_the_controller_result_and_each_operation() {
    let clean = ControllerCleanup {
        job_terminated: true,
        controller_reaped: true,
        job_empty: true,
        removal: "removed",
    };
    let observed = Ok("child-process-blocked");
    let failed = Err("private controller error".to_owned());
    assert_eq!(
        clean.failure_receipt(&observed, "exit", Some("child-process-blocked")),
        None
    );
    assert_eq!(
        clean
            .failure_receipt(&failed, "exit-marker", Some("access-denied"))
            .as_deref(),
        Some("OFOX_CHILD_CALL_FAILURE_V2 exit-marker access-denied true true true removed")
    );
    for (outcome, expected) in [
        (
            ControllerCleanup {
                job_terminated: false,
                ..clean
            },
            "false true true removed",
        ),
        (
            ControllerCleanup {
                controller_reaped: false,
                ..clean
            },
            "true false true removed",
        ),
        (
            ControllerCleanup {
                job_empty: false,
                ..clean
            },
            "true true false removed",
        ),
        (
            ControllerCleanup {
                removal: "sharing-violation",
                ..clean
            },
            "true true true sharing-violation",
        ),
    ] {
        assert_eq!(
            outcome.failure_receipt(&observed, "exit", Some("child-process-blocked")),
            Some(format!(
                "OFOX_CHILD_CALL_FAILURE_V2 child-process-blocked child-process-blocked {expected}"
            ))
        );
        assert_eq!(
            outcome.failure_receipt(&failed, "symbols", None),
            Some(format!(
                "OFOX_CHILD_CALL_FAILURE_V2 symbols none {expected}"
            ))
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
    let mut replies = ControllerReplies::default();
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
            // ARMED, optional PAIRED and a terminal line. Detect any fourth line.
            for line in BufReader::new(pipe.take(256)).lines().take(4) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut input = Some(input);
        loop {
            check_time(deadline, cancelled)?;
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(line)) => {
                    replies.accept(&line).map_err(|()| {
                        failure_stage = "native-test";
                        UNAVAILABLE.to_owned()
                    })?;
                    if line == "ARMED" {
                        failure_stage = "observation";
                        io(File::from(input.take().ok_or(UNAVAILABLE)?).write_all(b"G"))?;
                    } else if let Some(Err(stage)) = replies.terminal {
                        failure_stage = stage;
                    } else {
                        failure_stage = "exit";
                    }
                }
                Ok(Err(_)) => {
                    failure_stage = "native-test";
                    return Err(UNAVAILABLE.into());
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        reader.join().map_err(|_| UNAVAILABLE)?;
        loop {
            check_time(deadline, cancelled)?;
            if let Some(status) = io(child.try_wait())? {
                return match (replies.terminal, status.success()) {
                    (Some(Ok(value)), true) => Ok(value),
                    (Some(Err(_)), true) => {
                        failure_stage = "native-test";
                        Err(UNAVAILABLE.into())
                    }
                    _ => Err(UNAVAILABLE.into()),
                };
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
    let cleanup = ControllerCleanup {
        job_terminated: terminated.is_ok(),
        controller_reaped: reaped.is_ok(),
        job_empty: empty.is_ok(),
        removal: remove_controller_scratch(scratch),
    };
    let paired = replies
        .paired
        .filter(|_| result.is_ok() || failure_stage.starts_with("exit"));
    if let Some(receipt) = cleanup.failure_receipt(&result, failure_stage, paired) {
        eprintln!("{receipt}");
        return Err(UNAVAILABLE.into());
    }
    if let Ok(value) = &result {
        eprintln!("OFOX_CHILD_CALL_CONTROLLER_V1 {value}");
    }
    result
}
