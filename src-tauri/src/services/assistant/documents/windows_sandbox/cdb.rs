//! Only the first embedded Node fixture can request this diagnostic controller.
use super::{
    cdb_protocol::{Configured, Fence},
    *,
};
use windows::{
    Wdk::Foundation::OBJECT_ATTRIBUTES,
    Win32::{
        Foundation::UNICODE_STRING,
        System::{
            SystemInformation::IMAGE_FILE_MACHINE_UNKNOWN,
            Threading::{IsWow64Process2, SuspendThread},
        },
    },
};

pub(super) const ENV_ROOT: &str = "OFOX_DOCUMENT_CDB_DIAGNOSTIC_ROOT";

pub(super) struct Guard {
    root: PathBuf,
    pid: u32,
    tid: u32,
    final_hold: Option<(u32, u32)>,
    _pins: Vec<File>,
    fence: Fence,
    observation: Observer,
    closed: bool,
    closing_attempted: bool,
    preserve_failure_stage: bool,
}

fn json(root: &Path, name: &str) -> Result<serde_json::Value, String> {
    let mut bytes = Vec::new();
    io(open_regular(root, &root.join(name), 32768)?
        .take(32769)
        .read_to_end(&mut bytes))?;
    if bytes.len() > 32768 {
        return Err(UNAVAILABLE.into());
    }
    serde_json::from_slice(&bytes).map_err(|_| UNAVAILABLE.to_owned())
}
fn write(root: &Path, name: &str, value: &serde_json::Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|_| UNAVAILABLE.to_owned())?;
    crate::config::atomic_write(&root.join(name), &bytes).map_err(|_| UNAVAILABLE.to_owned())
}
fn number(value: &serde_json::Value, name: &str) -> Result<u32, String> {
    value[name]
        .as_u64()
        .and_then(|n| n.try_into().ok())
        .ok_or_else(|| UNAVAILABLE.to_owned())
}
fn stage(value: &serde_json::Value) -> &'static str {
    match value["stage"].as_str() {
        Some("operation_architecture_unverified") => "operation_architecture_unverified",
        Some("operation_layout_unverified") => "operation_layout_unverified",
        Some("operation_thread_unverified") => "operation_thread_unverified",
        Some("operation_install_unverified") => "operation_install_unverified",
        Some("operation_target_invalid") => "operation_target_invalid",
        Some("target_not_seen") => "target_not_seen",
        Some("target_mismatch") => "target_mismatch",
        Some("gflag_before_unreadable") => "gflag_before_unreadable",
        Some("gflag_after_unreadable") => "gflag_after_unreadable",
        Some("gflag_changed_unexpectedly") => "gflag_changed_unexpectedly",
        Some("continue_mismatch") => "continue_mismatch",
        Some("host_resume_mismatch") => "host_resume_mismatch",
        Some("init_tail_missing") => "init_tail_missing",
        Some("debugger_exited") => "debugger_exited",
        Some("target_exited") => "target_exited",
        Some("deadline") => "deadline",
        Some("cancelled") => "cancelled",
        Some("debugger_closed") => "debugger_closed",
        _ => "controller_unknown",
    }
}
impl Guard {
    fn note(&self, current: &'static str) {
        if self.preserve_failure_stage {
            return;
        }
        self.observation.evidence(|evidence| {
            let diagnostic = evidence.loader_diagnostic.get_or_insert_default();
            diagnostic.diagnostic_only = true;
            diagnostic.stage = Some(current);
        });
    }
    pub fn begin(
        process: HANDLE,
        thread: HANDLE,
        pid: u32,
        tid: u32,
        observation: &Observer,
        cancelled: &AtomicBool,
    ) -> Result<Option<Self>, String> {
        if !observation.loader_requested() {
            return Ok(None);
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        observation.enter(Phase::CdbInitialHold);
        let root = PathBuf::from(std::env::var_os(ENV_ROOT).ok_or(UNAVAILABLE)?);
        if !root.is_absolute() {
            return Err(UNAVAILABLE.into());
        }
        let pins = pin_ancestors(&root)?;
        let _owner = owned_directory(&root, &current_user()?)?;
        let mut guard = Self {
            root,
            pid,
            tid,
            final_hold: None,
            _pins: pins,
            fence: Fence::new(pid, tid),
            observation: observation.clone(),
            closed: false,
            closing_attempted: false,
            preserve_failure_stage: false,
        };
        guard.note("target_not_seen");
        let controller = json(&guard.root, "controller.json")?;
        if controller["diagnosticOnly"] != true || controller["controllerReady"] != true {
            return Err(UNAVAILABLE.into());
        }
        guard.note("operation_architecture_unverified");
        let mut process_machine = IMAGE_FILE_MACHINE_UNKNOWN;
        let mut native_machine = IMAGE_FILE_MACHINE_UNKNOWN;
        let architecture =
            unsafe { IsWow64Process2(process, &mut process_machine, Some(&mut native_machine)) };
        // Published Windows x64 C ABI, checked against the native bindings.
        // No dependency on private ntdll PDB object-type evaluation.
        let compiled_layout = (
            size_of::<OBJECT_ATTRIBUTES>(),
            std::mem::offset_of!(OBJECT_ATTRIBUTES, RootDirectory),
            std::mem::offset_of!(OBJECT_ATTRIBUTES, ObjectName),
            size_of::<UNICODE_STRING>(),
            std::mem::offset_of!(UNICODE_STRING, Length),
            std::mem::offset_of!(UNICODE_STRING, MaximumLength),
            std::mem::offset_of!(UNICODE_STRING, Buffer),
        ) == (0x30, 8, 0x10, 0x10, 0, 2, 8);
        if !compiled_layout
            || !cdb_protocol::native_x64(architecture.is_ok(), process_machine.0, native_machine.0)
        {
            return Err(UNAVAILABLE.into());
        }
        guard.note("pre_attach_hold_unverified");
        if cancelled.load(Ordering::Acquire) {
            guard.note("cancelled");
            return Err(CANCELLED.into());
        }
        let initial = unsafe { SuspendThread(thread) };
        let mut initial_paired = None;
        let initially_held = guard.fence.initial_hold(initial, || {
            let paired = unsafe { ResumeThread(thread) };
            initial_paired = Some(paired);
            paired
        });
        guard.observation.evidence(|evidence| {
            let diagnostic = evidence.loader_diagnostic.get_or_insert_default();
            diagnostic.pre_attach_suspend_previous = Some(initial);
            diagnostic.pre_attach_paired_resume_previous = initial_paired;
        });
        initially_held.map_err(|()| UNAVAILABLE.to_owned())?;
        if cancelled.load(Ordering::Acquire) {
            guard.note("cancelled");
            return Err(CANCELLED.into());
        }
        guard.observation.enter(Phase::CdbHandshake);
        write(
            &guard.root,
            "target.json",
            &serde_json::json!({"childPid":pid,"primaryTid":tid,"architecture":{
                "apiSucceeded":true,"processMachine":process_machine.0,"nativeMachine":native_machine.0,
                "compiledLayoutVerified":true}}),
        )?;
        let configured = loop {
            if cancelled.load(Ordering::Acquire) {
                guard.note("cancelled");
                return Err(CANCELLED.into());
            }
            if guard.root.join("failed.json").exists() {
                guard.note("failure_record_unreadable");
                let failure = json(&guard.root, "failed.json")?;
                guard.note(stage(&failure));
                return Err(UNAVAILABLE.into());
            }
            if guard.root.join("ready.json").exists() {
                guard.note("ready_record_unreadable");
                break json(&guard.root, "ready.json")?;
            }
            if unsafe { WaitForSingleObject(process, 0) } == WAIT_OBJECT_0 {
                guard.note("target_exited");
                return Err(UNAVAILABLE.into());
            }
            if Instant::now() >= deadline {
                guard.note("deadline");
                return Err(UNAVAILABLE.into());
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        guard.note("configuration_mismatch");
        let before = number(&configured, "beforeFlags")?;
        let after = number(&configured, "afterFlags")?;
        guard
            .fence
            .configure(Configured {
                pid: number(&configured, "childPid")?,
                tid: number(&configured, "primaryTid")?,
                before,
                after,
                confirmed: configured["configConfirmed"] == true,
            })
            .map_err(|()| UNAVAILABLE.to_owned())?;
        guard.observation.evidence(|evidence| {
            let diagnostic = evidence.loader_diagnostic.get_or_insert_default();
            diagnostic.config_confirmed = Some(true);
            diagnostic.before_flags = Some(before);
            diagnostic.after_flags = Some(after);
        });
        guard.note("debugger_release_wait");
        guard.observation.enter(Phase::CdbContinue);
        guard
            .fence
            .continuing()
            .map_err(|()| UNAVAILABLE.to_owned())?;
        if Instant::now() >= deadline {
            guard.note("deadline");
            return Err(UNAVAILABLE.into());
        }
        if cancelled.load(Ordering::Acquire) {
            guard.note("cancelled");
            return Err(CANCELLED.into());
        }
        write(
            &guard.root,
            "continue.json",
            &serde_json::json!({
                "childPid": pid, "primaryTid": tid, "continueRequested": true,
                "preSuspendPrevious": initial, "prePairedResumePrevious": initial_paired,
            }),
        )?;
        guard.observation.enter(Phase::CdbHold);
        loop {
            if cancelled.load(Ordering::Acquire) {
                guard.note("cancelled");
                return Err(CANCELLED.into());
            }
            if guard.root.join("failed.json").exists() {
                guard.note(stage(&json(&guard.root, "failed.json")?));
                return Err(UNAVAILABLE.into());
            }
            if unsafe { WaitForSingleObject(process, 0) } == WAIT_OBJECT_0 {
                guard.note("target_exited");
                return Err(UNAVAILABLE.into());
            }
            if Instant::now() >= deadline {
                guard.note("deadline");
                return Err(UNAVAILABLE.into());
            }
            let previous = unsafe { SuspendThread(thread) };
            let mut paired = None;
            let held = guard.fence.hold(previous, || {
                let n = unsafe { ResumeThread(thread) };
                paired = Some(n);
                n
            });
            guard.observation.evidence(|evidence| {
                let diagnostic = evidence.loader_diagnostic.get_or_insert_default();
                diagnostic.release_probe_count = diagnostic.release_probe_count.saturating_add(1);
                diagnostic.release_suspend_previous = Some(previous);
                diagnostic.release_paired_resume_previous = paired;
                if held == Ok(true) {
                    diagnostic.suspend_previous = Some(previous);
                    diagnostic.paired_resume_previous = paired;
                }
            });
            if held.map_err(|()| UNAVAILABLE.to_owned())? {
                guard.final_hold = paired.map(|value| (previous, value));
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if cancelled.load(Ordering::Acquire) {
            guard.note("cancelled");
            return Err(CANCELLED.into());
        }
        if Instant::now() >= deadline {
            guard.note("deadline");
            return Err(UNAVAILABLE.into());
        }
        guard.note("configured_and_held");
        Ok(Some(guard))
    }
    pub fn resumed(&mut self, previous: u32, cancelled: &AtomicBool) -> Result<(), String> {
        self.observation.evidence(|evidence| {
            evidence
                .loader_diagnostic
                .get_or_insert_default()
                .host_resume_previous = Some(previous)
        });
        if cancelled.load(Ordering::Acquire) {
            self.fence.abort();
            self.note("cancelled");
            return Err(CANCELLED.into());
        }
        self.fence
            .resumed(previous)
            .map_err(|()| UNAVAILABLE.to_owned())?;
        if !self.fence.may_go() {
            return Err(UNAVAILABLE.into());
        }
        if cancelled.load(Ordering::Acquire) {
            self.fence.abort();
            self.note("cancelled");
            return Err(CANCELLED.into());
        }
        let (suspend, paired) = self.final_hold.ok_or(UNAVAILABLE)?;
        write(
            &self.root,
            "go.json",
            &serde_json::json!({"hostResumeResult":previous,"childPid":self.pid,"primaryTid":self.tid,
                "finalSuspendPrevious":suspend,"finalPairedResumePrevious":paired}),
        )?;
        self.note("go_sent");
        Ok(())
    }
    pub fn finish(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        if self.closing_attempted {
            return Err(cleanup_failed());
        }
        self.closing_attempted = true;
        self.fence.abort();
        self.observation.enter(Phase::CdbStop);
        write(&self.root, "abort.json", &serde_json::json!({"abort":true}))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.root.join("finished.json").exists() {
            if Instant::now() >= deadline {
                self.note("debugger_stop_unconfirmed");
                return Err(cleanup_failed());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let result = json(&self.root, "finished.json")?;
        self.closed = result["debuggerClosed"] == true;
        self.observation.evidence(|evidence| {
            evidence
                .loader_diagnostic
                .get_or_insert_default()
                .debugger_closed = Some(self.closed)
        });
        if !self.closed {
            return Err(cleanup_failed());
        }
        self.note(stage(&result));
        Ok(())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.closed {
            self.observation.failed();
            self.preserve_failure_stage = true;
        }
        if self.finish().is_err() {
            invalidate_admission();
        }
    }
}
