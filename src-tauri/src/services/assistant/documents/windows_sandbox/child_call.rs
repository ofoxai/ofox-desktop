//! Dedicated CI diagnostic for the single, embedded checkpoint-7 Node request.
//! The target process and primary thread come directly from create_verified.
use super::*;
use std::{
    io::{BufRead, BufReader},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    process::{Command, Stdio},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
};
use windows::Win32::Storage::FileSystem::{
    FileAttributeTagInfo, GetFileInformationByHandleEx, DELETE, FILE_ATTRIBUTE_TAG_INFO,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_TRAVERSE,
};
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::System::SystemServices::IO_REPARSE_TAG_MOUNT_POINT;

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

// These stages are valid only after PAIRED and always withdraw its value.
const PAIRED_FAILURES: &[&str] = &[
    "return-conflict",
    "return-observer-error",
    "return-error/before-exit-marker/breakpoint",
    "return-error/before-exit-marker/syntax",
    "return-error/before-exit-marker/command-file",
    "return-error/after-exit-marker/breakpoint",
    "return-error/after-exit-marker/syntax",
    "return-error/after-exit-marker/command-file",
];

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
                "last-error",
                "last-error-zero",
                "last-error-unreadable",
                "exit",
                "exit-marker",
                "exit-wait",
                "exit-status",
                "exit-stderr",
            ];
            let stage = stages
                .into_iter()
                .chain(PAIRED_FAILURES.iter().copied())
                .find(|v| *v == stage)
                .ok_or(())?;
            if PAIRED_FAILURES.contains(&stage) {
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
    for &stage in PAIRED_FAILURES {
        let mut invalidated = ControllerReplies::default();
        assert!(invalidated.accept(&format!("INCOMPLETE {stage}")).is_err());
        invalidated.accept("ARMED").unwrap();
        invalidated.accept("PAIRED access-denied").unwrap();
        assert!(invalidated
            .accept("INCOMPLETE return-error/after-exit-marker/private")
            .is_err());
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

fn scratch_error(error: std::io::Error) -> &'static str {
    match error.raw_os_error() {
        Some(5) => "access-denied",
        Some(32) => "sharing-violation",
        Some(33) => "lock-violation",
        Some(145) => "directory-not-empty",
        Some(2 | 3) => "not-found",
        _ => "other",
    }
}

fn remove_controller_scratch(scratch: &Path, recover: Option<ScratchIdentity>) -> &'static str {
    // TempDir::close wraps the native error with its path. Keep only a closed
    // classification, and preserve the original removal on all other paths.
    match fs::remove_dir_all(scratch) {
        Ok(()) => "removed",
        Err(error) => recover_controller_scratch(scratch, recover, error),
    }
}

fn recover_controller_scratch(
    scratch: &Path,
    recover: Option<ScratchIdentity>,
    error: std::io::Error,
) -> &'static str {
    if let Some(_ancestors) = recover
        .filter(|_| error.raw_os_error() == Some(5))
        .and_then(|identity| unlink_controller_profile_junction(scratch, identity))
    {
        return match fs::remove_dir_all(scratch) {
            Ok(()) => "removed",
            Err(error) => scratch_error(error),
        };
    }
    scratch_error(error)
}

const SCRATCH_ENTRY_LIMIT: usize = 32;
const SCRATCH_DEPTH_LIMIT: usize = 4;
type ScratchIdentity = (u32, u32, u32);

#[derive(Default)]
struct ScratchResidue {
    command_files: usize,
    symbol_entries: usize,
    other_entries: usize,
}

struct ScratchObstruction {
    location: &'static str,
    stage: &'static str,
    reason: &'static str,
}

impl ScratchObstruction {
    fn new(location: &'static str, stage: &'static str, reason: &'static str) -> Self {
        Self {
            location,
            stage,
            reason,
        }
    }
}

fn open_scratch_entry(path: &Path, access: u32, share: u32) -> std::io::Result<File> {
    fs::OpenOptions::new()
        .access_mode(access)
        .share_mode(share)
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
        .open(path)
}

fn scratch_handle_info(file: &File) -> Option<(ScratchIdentity, u32)> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.ok()?;
    Some((
        (
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ),
        info.dwFileAttributes,
    ))
}

const SCRATCH_SHARED: u32 = FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0;

fn pin_scratch_directory(path: &Path) -> Option<(File, ScratchIdentity)> {
    // Attribute-only opens are exempt from these Windows sharing checks.
    let file = open_scratch_entry(
        path,
        (FILE_TRAVERSE | FILE_READ_ATTRIBUTES).0,
        FILE_SHARE_READ.0,
    )
    .ok()?;
    let (identity, attributes) = scratch_handle_info(&file)?;
    (attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT).0
        == FILE_ATTRIBUTE_DIRECTORY.0)
        .then_some((file, identity))
}

fn unlink_controller_profile_junction(
    scratch: &Path,
    expected: ScratchIdentity,
) -> Option<Vec<File>> {
    let ancestors = pin_ancestors(scratch.parent()?).ok()?;
    let (root, identity) = pin_scratch_directory(scratch)?;
    if identity != expected {
        return None;
    }
    let mut pins = vec![root];
    // The signed SymChk's WinINet profile contains this exact junction. Its
    // deny ACE prevents the directory-enumeration open used by remove_dir_all,
    // but allows opening the junction itself for DELETE.
    let mut directory = scratch.to_path_buf();
    for component in ["AppData", "Local", "Microsoft", "Windows", "INetCache"] {
        directory.push(component);
        pins.push(pin_scratch_directory(&directory)?.0);
    }
    let entry = pin_profile_junction(&directory.join("Content.IE5"))?;
    let handle = HANDLE(entry.as_raw_handle());
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // The no-follow handle binds this operation to the link itself. Its
    // destination was never opened. Keep every directory pinned through close.
    unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    }
    .ok()?;
    drop(entry);
    drop(pins);
    // The caller still needs the outer path to be anchored during the retry.
    Some(ancestors)
}

fn pin_profile_junction(path: &Path) -> Option<File> {
    let entry = open_scratch_entry(
        path,
        (DELETE | FILE_READ_ATTRIBUTES).0,
        (FILE_SHARE_READ | FILE_SHARE_WRITE).0,
    )
    .ok()?;
    let handle = HANDLE(entry.as_raw_handle());
    let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    }
    .ok()?;
    let junction_attributes = (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT).0;
    (tag.ReparseTag == IO_REPARSE_TAG_MOUNT_POINT
        && tag.FileAttributes & junction_attributes == junction_attributes)
        .then_some(entry)
}

fn check_scratch_delete_access(
    path: &Path,
    location: &'static str,
) -> Result<(), ScratchObstruction> {
    // Opening for DELETE never requests deletion or changes permissions. Close
    // before any listing so these probe handles do not cause sharing failures.
    open_scratch_entry(path, DELETE.0, SCRATCH_SHARED)
        .map(drop)
        .map_err(|error| ScratchObstruction::new(location, "open-delete", scratch_error(error)))
}

fn inspect_scratch_directory(
    path: &Path,
    location: &'static str,
    depth: usize,
    expected: Option<ScratchIdentity>,
    residue: &mut ScratchResidue,
) -> Result<(), ScratchObstruction> {
    if depth > SCRATCH_DEPTH_LIMIT {
        return Err(ScratchObstruction::new(location, "limit", "none"));
    }
    // Open without following reparses and deny replacement while read_dir
    // enumerates this path. The callers keep every ancestor alive.
    let pinned =
        open_scratch_entry(path, FILE_LIST_DIRECTORY.0, FILE_SHARE_READ.0).map_err(|error| {
            ScratchObstruction::new(location, "list-directory", scratch_error(error))
        })?;
    let (identity, attributes) = scratch_handle_info(&pinned)
        .ok_or_else(|| ScratchObstruction::new(location, "inspect", "other"))?;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(ScratchObstruction::new(location, "reparse-point", "none"));
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0
        || expected.is_some_and(|expected| expected != identity)
    {
        return Err(ScratchObstruction::new(location, "identity", "none"));
    }
    let list_error =
        |error| ScratchObstruction::new(location, "list-directory", scratch_error(error));
    for entry in fs::read_dir(path).map_err(list_error)? {
        let entry = entry.map_err(list_error)?;
        if residue.command_files + residue.symbol_entries + residue.other_entries
            >= SCRATCH_ENTRY_LIMIT
        {
            return Err(ScratchObstruction::new(location, "limit", "none"));
        }
        let name = entry.file_name();
        let child_location = if location == "symbol-cache" || (depth == 0 && name == "symbols") {
            residue.symbol_entries += 1;
            "symbol-cache"
        } else if depth == 0
            && ["init.cmd", "entry.cmd", "return.cmd", "exit.cmd"]
                .iter()
                .any(|fixed| name == *fixed)
        {
            residue.command_files += 1;
            "command-file"
        } else {
            residue.other_entries += 1;
            "other"
        };
        let attributes = entry
            .metadata()
            .map_err(|error| {
                ScratchObstruction::new(child_location, "inspect", scratch_error(error))
            })?
            .file_attributes();
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(ScratchObstruction::new(
                child_location,
                "reparse-point",
                "none",
            ));
        }
        let entry_path = entry.path();
        check_scratch_delete_access(&entry_path, child_location)?;
        if attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
            inspect_scratch_directory(&entry_path, child_location, depth + 1, None, residue)?;
        }
    }
    drop(pinned);
    Ok(())
}

fn observe_scratch_residue(
    path: &Path,
    expected: Option<ScratchIdentity>,
    residue: &mut ScratchResidue,
) -> Result<(), ScratchObstruction> {
    let identity_failure = || ScratchObstruction::new("root", "identity", "none");
    let expected = expected.ok_or_else(identity_failure)?;
    let parent = path.parent().ok_or_else(identity_failure)?;
    let _ancestors = pin_ancestors(parent).map_err(|_| identity_failure())?;
    // The initial read-only handle and the listing must identify the same
    // directory we created. A reparse or replaced root stops inspection.
    let file = open_scratch_entry(path, FILE_READ_ATTRIBUTES.0, SCRATCH_SHARED)
        .map_err(|error| ScratchObstruction::new("root", "inspect", scratch_error(error)))?;
    let (identity, attributes) = scratch_handle_info(&file)
        .ok_or_else(|| ScratchObstruction::new("root", "inspect", "other"))?;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(ScratchObstruction::new("root", "reparse-point", "none"));
    }
    if identity != expected || attributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
        return Err(identity_failure());
    }
    drop(file);
    check_scratch_delete_access(path, "root")?;
    inspect_scratch_directory(path, "root", 0, Some(expected), residue)
}

fn scratch_residue_receipt(path: &Path, expected: Option<ScratchIdentity>) -> String {
    // A recursive removal may have partially succeeded. The surviving tree
    // can explain where to look next, but cannot identify the original failure.
    let mut residue = ScratchResidue::default();
    let obstruction = observe_scratch_residue(path, expected, &mut residue)
        .err()
        .unwrap_or_else(|| ScratchObstruction::new("none", "none", "none"));
    format!(
        "OFOX_CHILD_CALL_SCRATCH_V1 {} {} {} {} {} {}",
        residue.command_files,
        residue.symbol_entries,
        residue.other_entries,
        obstruction.location,
        obstruction.stage,
        obstruction.reason
    )
}

#[test]
fn fixed_child_scratch_removal_classifies_an_exclusive_file_handle() {
    let parent = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir_in(parent.path()).unwrap();
    assert_eq!(remove_controller_scratch(&scratch.keep(), None), "removed");
    let scratch = tempfile::tempdir_in(parent.path()).unwrap();
    let symbols = scratch.path().join("symbols");
    fs::create_dir(&symbols).unwrap();
    let path = symbols.join("fixture");
    let identity = open_scratch_entry(scratch.path(), FILE_READ_ATTRIBUTES.0, SCRATCH_SHARED)
        .ok()
        .and_then(|file| scratch_handle_info(&file))
        .map(|(identity, _)| identity);
    let held = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let scratch = scratch.keep();
    let reason = remove_controller_scratch(&scratch, None);
    let evidence = scratch_residue_receipt(&scratch, identity);
    drop(held);
    parent.close().unwrap();
    assert_eq!(reason, "sharing-violation");
    assert_eq!(
        evidence,
        "OFOX_CHILD_CALL_SCRATCH_V1 0 2 0 symbol-cache open-delete sharing-violation"
    );
}

#[test]
fn fixed_child_scratch_pins_block_root_and_nested_parent_renames() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("scratch");
    let nested = root.join("profile");
    fs::create_dir_all(&nested).unwrap();
    let attempts = [
        (&root, fixture.path().join("moved-scratch")),
        (&nested, root.join("moved-profile")),
    ]
    .map(|(directory, renamed)| {
        let (pinned, _) = pin_scratch_directory(directory).unwrap();
        let attempt = fs::rename(directory, &renamed);
        drop(pinned);
        if attempt.is_ok() {
            fs::rename(&renamed, directory).unwrap();
        }
        attempt.err().and_then(|error| error.raw_os_error())
    });
    assert_eq!(attempts, [Some(32), Some(32)]);
}

#[test]
fn fixed_child_scratch_unlinks_the_profile_junction_without_touching_its_target() {
    let parent = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir_in(parent.path()).unwrap();
    let outside = parent.path().join("target");
    fs::create_dir(&outside).unwrap();
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, b"outside scratch").unwrap();
    let junction = scratch
        .path()
        .join(r"AppData\Local\Microsoft\Windows\INetCache\Content.IE5");
    let (_, original) = pin_scratch_directory(scratch.path()).unwrap();
    let (_, replacement) = pin_scratch_directory(&outside).unwrap();
    let attempt = |identity, win32| {
        recover_controller_scratch(
            scratch.path(),
            identity,
            std::io::Error::from_raw_os_error(win32),
        )
    };
    fs::create_dir_all(&junction).unwrap();
    let ordinary = junction.join("sentinel");
    fs::write(&ordinary, b"ordinary directory").unwrap();
    assert_eq!(attempt(Some(original), 5), "access-denied");
    assert_eq!(fs::read(ordinary).unwrap(), b"ordinary directory");
    fs::remove_file(junction.join("sentinel")).unwrap();
    fs::remove_dir(&junction).unwrap();
    let result = Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .current_dir(parent.path())
        .arg(junction.strip_prefix(parent.path()).unwrap())
        .arg("target")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(attempt(Some(original), 32), "sharing-violation");
    assert_eq!(attempt(None, 5), "access-denied");
    assert_eq!(attempt(Some(replacement), 5), "access-denied");
    assert!(fs::symlink_metadata(&junction).is_ok());
    let pinned = pin_profile_junction(&junction).unwrap();
    let renamed = junction.with_file_name("moved-junction");
    let rename = fs::rename(&junction, &renamed);
    drop(pinned);
    if rename.is_ok() {
        fs::rename(&renamed, &junction).unwrap();
    }
    assert_eq!(
        rename.err().and_then(|error| error.raw_os_error()),
        Some(32)
    );
    assert_eq!(attempt(Some(original), 5), "removed");
    assert_eq!(
        fs::symlink_metadata(scratch.path()).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"outside scratch");
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
    let scratch_identity =
        open_scratch_entry(scratch.path(), FILE_READ_ATTRIBUTES.0, SCRATCH_SHARED)
            .ok()
            .and_then(|file| scratch_handle_info(&file))
            .map(|(identity, _)| identity);
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
    let scratch = scratch.keep();
    let recover =
        scratch_identity.filter(|_| terminated.is_ok() && reaped.is_ok() && empty.is_ok());
    let cleanup = ControllerCleanup {
        job_terminated: terminated.is_ok(),
        controller_reaped: reaped.is_ok(),
        job_empty: empty.is_ok(),
        removal: remove_controller_scratch(&scratch, recover),
    };
    let paired = replies
        .paired
        .filter(|_| result.is_ok() || failure_stage.starts_with("exit"));
    if let Some(receipt) = cleanup.failure_receipt(&result, failure_stage, paired) {
        eprintln!("{receipt}");
        if cleanup.job_terminated
            && cleanup.controller_reaped
            && cleanup.job_empty
            && cleanup.removal != "removed"
        {
            eprintln!("{}", scratch_residue_receipt(&scratch, scratch_identity));
        }
        return Err(UNAVAILABLE.into());
    }
    if let Ok(value) = &result {
        eprintln!("OFOX_CHILD_CALL_CONTROLLER_V1 {value}");
    }
    result
}
