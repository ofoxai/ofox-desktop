//! Windows document-only LPAC launcher. This is deliberately separate from the
//! general assistant process runner: a Job alone is not a security boundary.
//!
//! No caller may advertise Python merely because this module compiles. The
//! native functional/negative probes must pass on the actual installed assets.
use super::runner::{EngineOutput, DEAD_PROXY};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    ffi::{c_void, OsStr},
    fs::{self, File},
    io::{Read, Write},
    mem::{offset_of, size_of},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use windows::{
    core::{BOOL, PCWSTR, PWSTR},
    Wdk::System::SystemServices::RtlGetVersion,
    Win32::{
        Foundation::{
            LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT, HLOCAL,
            WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Security::{
            Authorization::{
                ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
                ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
                SetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            DeriveCapabilitySidsFromName, EqualSid, FreeSid, GetSecurityDescriptorDacl,
            GetSecurityDescriptorSacl, GetSidSubAuthority, GetSidSubAuthorityCount,
            GetTokenInformation,
            Isolation::{
                CreateAppContainerProfile, DeleteAppContainerProfile, GetAppContainerFolderPath,
            },
            TokenAppContainerSid, TokenCapabilities, TokenIntegrityLevel, TokenIsAppContainer,
            TokenIsLessPrivilegedAppContainer, TokenUser, DACL_SECURITY_INFORMATION,
            LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
            SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
            TOKEN_APPCONTAINER_INFORMATION, TOKEN_GROUPS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
            TOKEN_USER,
        },
        Storage::FileSystem::{
            CreateDirectoryW, CreateFileW, FileDispositionInfo, FileRenameInfo,
            GetFileInformationByHandle, GetFinalPathNameByHandleW, SetFileInformationByHandle,
            BY_HANDLE_FILE_INFORMATION, CREATE_NEW, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
            FILE_NAME_NORMALIZED, FILE_RENAME_INFO, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        },
        System::{
            Com::CoTaskMemFree,
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
                JobObjectExtendedLimitInformation, QueryInformationJobObject,
                SetInformationJobObject, TerminateJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Pipes::CreatePipe,
            SystemInformation::OSVERSIONINFOW,
            SystemServices::SE_GROUP_ENABLED,
            Threading::{
                CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess,
                GetExitCodeProcess, GetProcessMitigationPolicy, InitializeProcThreadAttributeList,
                OpenProcessToken, ProcessChildProcessPolicy, ProcessImageLoadPolicy, ResumeThread,
                TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW,
                CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
                LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
                PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
                PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
            },
        },
    },
};

const UNAVAILABLE: &str = "assistant_document_sandbox_unavailable";
const UNSAFE_FILE: &str = "assistant_document_sandbox_unsafe_file";
const CANCELLED: &str = "assistant_document_sandbox_cancelled";
const MAX_TREE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_TASK_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TREE_FILES: usize = 30_000;
// These SDK constants are creation attributes, not post-start best effort.
const LPAC_OPT_OUT: u32 = 1;
const CHILD_RESTRICTED: u32 = 1;
const NO_REMOTE_IMAGES: u64 = 1 << 52;
const NO_LOW_LABEL_IMAGES: u64 = 1 << 56;
const PREFER_SYSTEM32: u64 = 1 << 60;
const RUNTIME_CAPABILITY: &str = "ofoxDocumentRuntimeReadV1";

#[track_caller]
fn win<T>(result: windows::core::Result<T>) -> Result<T, String> {
    let line = std::panic::Location::caller().line();
    result.map_err(|error| {
        format!(
            "assistant_document_sandbox_win32_{:08x}_at_{line}",
            error.code().0 as u32
        )
    })
}
fn io<T>(result: std::io::Result<T>) -> Result<T, String> {
    result.map_err(|_| UNSAFE_FILE.to_owned())
}
fn wide(value: impl AsRef<OsStr>) -> Result<Vec<u16>, String> {
    let value: Vec<_> = value.as_ref().encode_wide().collect();
    if value.contains(&0) {
        return Err(UNAVAILABLE.into());
    }
    Ok(value.into_iter().chain(Some(0)).collect())
}
fn handle(raw: HANDLE) -> OwnedHandle {
    // SAFETY: every caller transfers one newly created, owned kernel handle.
    unsafe { OwnedHandle::from_raw_handle(raw.0) }
}
fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0)));
        }
    }
}
struct Sid(PSID);
impl Drop for Sid {
    fn drop(&mut self) {
        unsafe {
            let _ = FreeSid(self.0);
        }
    }
}
fn sid_string(sid: PSID) -> Result<String, String> {
    let mut text = PWSTR::null();
    unsafe {
        win(ConvertSidToStringSidW(sid, &mut text))?;
        let allocation = Local(text.0.cast());
        let result = text.to_string().map_err(|_| UNAVAILABLE.to_owned());
        drop(allocation);
        result
    }
}
fn token_buffer(
    token: HANDLE,
    kind: windows::Win32::Security::TOKEN_INFORMATION_CLASS,
) -> Result<Vec<usize>, String> {
    let mut length = 0;
    unsafe {
        let _ = GetTokenInformation(token, kind, None, 0, &mut length);
    }
    if length == 0 || length > 64 * 1024 {
        return Err(UNAVAILABLE.into());
    }
    let mut data = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    unsafe {
        win(GetTokenInformation(
            token,
            kind,
            Some(data.as_mut_ptr().cast()),
            length,
            &mut length,
        ))?;
    }
    Ok(data)
}
fn current_user() -> Result<String, String> {
    let mut token = HANDLE::default();
    unsafe {
        win(OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY,
            &mut token,
        ))?;
    }
    let token = handle(token);
    let data = token_buffer(raw(&token), TokenUser)?;
    sid_string(unsafe { (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid })
}

/// One private custom capability authorizes only the immutable public cache.
/// It is not an internet, COM, registry, or user-library capability.
struct RuntimeCapability {
    sid: PSID,
    allocations: Vec<Local>,
}
impl RuntimeCapability {
    fn new() -> Result<Self, String> {
        let name = wide(RUNTIME_CAPABILITY)?;
        let (mut groups, mut count, mut sids, mut sid_count) =
            (std::ptr::null_mut(), 0, std::ptr::null_mut(), 0);
        unsafe {
            win(DeriveCapabilitySidsFromName(
                PCWSTR(name.as_ptr()),
                &mut groups,
                &mut count,
                &mut sids,
                &mut sid_count,
            ))?;
            let mut allocations = Vec::new();
            for i in 0..count {
                allocations.push(Local((*groups.add(i as usize)).0));
            }
            if !groups.is_null() {
                allocations.push(Local(groups.cast()));
            }
            for i in 0..sid_count {
                allocations.push(Local((*sids.add(i as usize)).0));
            }
            if sid_count != 1 || sids.is_null() {
                if !sids.is_null() {
                    allocations.push(Local(sids.cast()));
                }
                return Err(UNAVAILABLE.into());
            }
            let sid = *sids;
            allocations.push(Local(sids.cast()));
            Ok(Self { sid, allocations })
        }
    }
    fn entry(&self) -> SID_AND_ATTRIBUTES {
        // Keep the LocalAlloc allocations alive through CreateProcessW.
        debug_assert!(!self.allocations.is_empty());
        SID_AND_ATTRIBUTES {
            Sid: self.sid,
            Attributes: SE_GROUP_ENABLED as u32,
        }
    }
}

fn security_descriptor(sddl: &str) -> Result<Local, String> {
    let sddl = wide(sddl)?;
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        win(ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        ))?;
    }
    Ok(Local(descriptor.0))
}
fn create_directory(path: &Path, sddl: &str) -> Result<(), String> {
    let descriptor = security_descriptor(sddl)?;
    let attrs = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: BOOL(0),
    };
    let name = wide(path)?;
    unsafe { win(CreateDirectoryW(PCWSTR(name.as_ptr()), Some(&attrs))) }
}
fn private_sddl(user: &str) -> String {
    // Elevated/Server tokens may default newly created objects to an admin
    // group owner. Bind ownership to the actual caller SID explicitly.
    format!("O:{user}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{user})")
}
fn owned_directory(path: &Path, user: &str) -> Result<File, String> {
    let file = open_path(path, true)?;
    let mut owner = PSID::default();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        GetSecurityInfo(
            HANDLE(file.as_raw_handle()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            Some(&mut descriptor),
        )
        .ok()
        .map_err(|_| UNAVAILABLE.to_owned())?;
        let allocation = Local(descriptor.0);
        let matches = sid_string(owner)? == user;
        drop(allocation);
        if !matches {
            return Err(UNAVAILABLE.into());
        }
    }
    Ok(file)
}

fn open_path(path: &Path, directory: bool) -> Result<File, String> {
    let name = wide(path)?;
    let handle = unsafe {
        win(CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        ))?
    };
    let file = unsafe { File::from_raw_handle(handle.0) };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        win(GetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            &mut info,
        ))?;
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0) != directory
        || (!directory && info.nNumberOfLinks != 1)
    {
        return Err(UNSAFE_FILE.into());
    }
    Ok(file)
}
fn final_path(file: &File) -> Result<PathBuf, String> {
    let mut path = vec![0u16; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            HANDLE(file.as_raw_handle()),
            &mut path,
            FILE_NAME_NORMALIZED,
        )
    } as usize;
    if length == 0 || length >= path.len() {
        return Err(UNSAFE_FILE.into());
    }
    Ok(PathBuf::from(
        String::from_utf16(&path[..length]).map_err(|_| UNSAFE_FILE.to_owned())?,
    ))
}

/// Open an engine output with no following of the final reparse point, reject
/// multi-link files, and bind every ancestor and the opened handle to root.
/// Ancestor directory handles deny rename while the file is opened.
pub(crate) fn open_regular(root: &Path, path: &Path, max: u64) -> Result<File, String> {
    let mut directories = pin_ancestors(root)?;
    let root_handle = directories.last().ok_or(UNSAFE_FILE)?;
    let physical_root = final_path(root_handle)?;
    let relative = path
        .strip_prefix(root)
        .map_err(|_| UNSAFE_FILE.to_owned())?;
    let parts: Vec<_> = relative.components().collect();
    if parts.is_empty() || parts.iter().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(UNSAFE_FILE.into());
    }
    let mut cursor = root.to_path_buf();
    for part in &parts[..parts.len() - 1] {
        cursor.push(part.as_os_str());
        let dir = open_path(&cursor, true)?;
        if !final_path(&dir)?.starts_with(&physical_root) {
            return Err(UNSAFE_FILE.into());
        }
        directories.push(dir);
    }
    let file = open_path(path, false)?;
    if !final_path(&file)?.starts_with(&physical_root) || io(file.metadata())?.len() > max {
        return Err(UNSAFE_FILE.into());
    }
    drop(directories);
    Ok(file)
}

/// Do not trust a lexically normal parent reached through a junction higher
/// in the path. Keep every ancestor handle open through extraction/cleanup.
fn pin_ancestors(path: &Path) -> Result<Vec<File>, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(UNSAFE_FILE.into());
    }
    let ancestors: Vec<_> = path
        .ancestors()
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    ancestors
        .into_iter()
        .rev()
        .map(|p| open_path(p, true))
        .collect()
}

fn descriptor_string(descriptor: PSECURITY_DESCRIPTOR) -> Result<String, String> {
    let mut text = PWSTR::null();
    unsafe {
        win(ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION,
            &mut text,
            None,
        ))?;
        let allocation = Local(text.0.cast());
        let result = text.to_string().map_err(|_| UNAVAILABLE.to_owned());
        drop(allocation);
        result
    }
}
fn verify_cache_acl(root: &File, expected: &str) -> Result<(), String> {
    let mut actual = PSECURITY_DESCRIPTOR::default();
    unsafe {
        GetSecurityInfo(
            HANDLE(root.as_raw_handle()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            Some(&mut actual),
        )
        .ok()
        .map_err(|_| UNAVAILABLE.to_owned())?;
    }
    let allocation = Local(actual.0);
    let expected = security_descriptor(expected)?;
    let matches =
        descriptor_string(actual)? == descriptor_string(PSECURITY_DESCRIPTOR(expected.0))?;
    drop(allocation);
    if !matches {
        return Err(UNAVAILABLE.into());
    }
    Ok(())
}

fn files(root: &Path, max: u64) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let _pin = open_path(root, true)?;
    let mut pending = vec![root.to_path_buf()];
    let (mut result, mut total, mut entries) = (Vec::new(), 0u64, 0usize);
    while let Some(directory) = pending.pop() {
        let _directory = open_path(&directory, true)?;
        for item in io(fs::read_dir(&directory))? {
            let item = io(item)?;
            entries += 1;
            if entries > MAX_TREE_FILES {
                return Err(UNSAFE_FILE.into());
            }
            let metadata = io(fs::symlink_metadata(item.path()))?;
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                return Err(UNSAFE_FILE.into());
            }
            if metadata.is_dir() {
                pending.push(item.path());
            } else {
                if !metadata.is_file() || result.len() >= MAX_TREE_FILES {
                    return Err(UNSAFE_FILE.into());
                }
                let mut bytes = Vec::new();
                io(open_regular(root, &item.path(), max)?
                    .take(max + 1)
                    .read_to_end(&mut bytes))?;
                total = total.checked_add(bytes.len() as u64).ok_or(UNSAFE_FILE)?;
                if total > max {
                    return Err(UNSAFE_FILE.into());
                }
                result.push((
                    item.path()
                        .strip_prefix(root)
                        .map_err(|_| UNSAFE_FILE)?
                        .to_path_buf(),
                    bytes,
                ));
            }
        }
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}
fn tree_digest(files: &[(PathBuf, Vec<u8>)]) -> String {
    let mut hash = Sha256::new();
    for (name, bytes) in files {
        let name = name.to_string_lossy();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}

/// Hash warm public resources without retaining a second whole runtime in RAM.
fn tree_digest_at(root: &Path, max: u64) -> Result<String, String> {
    let _pins = pin_ancestors(root)?;
    let (mut pending, mut names, mut entries) = (vec![root.to_path_buf()], Vec::new(), 0usize);
    while let Some(directory) = pending.pop() {
        let _directory = open_path(&directory, true)?;
        for item in io(fs::read_dir(&directory))? {
            let item = io(item)?;
            entries += 1;
            if entries > MAX_TREE_FILES {
                return Err(UNSAFE_FILE.into());
            }
            let metadata = io(fs::symlink_metadata(item.path()))?;
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                return Err(UNSAFE_FILE.into());
            }
            if metadata.is_dir() {
                pending.push(item.path());
            } else if metadata.is_file() {
                names.push(
                    item.path()
                        .strip_prefix(root)
                        .map_err(|_| UNSAFE_FILE)?
                        .to_path_buf(),
                );
            } else {
                return Err(UNSAFE_FILE.into());
            }
        }
    }
    names.sort();
    let (mut hash, mut total, mut chunk) = (Sha256::new(), 0u64, [0u8; 64 * 1024]);
    for relative in names {
        let mut file = open_regular(root, &root.join(&relative), max)?;
        let length = io(file.metadata())?.len();
        total = total.checked_add(length).ok_or(UNSAFE_FILE)?;
        if total > max {
            return Err(UNSAFE_FILE.into());
        }
        let name = relative.to_string_lossy();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update(length.to_le_bytes());
        let mut read = 0u64;
        loop {
            let n = io(file.read(&mut chunk))?;
            if n == 0 {
                break;
            }
            read = read.checked_add(n as u64).ok_or(UNSAFE_FILE)?;
            if read > length {
                return Err(UNSAFE_FILE.into());
            }
            hash.update(&chunk[..n]);
        }
        if read != length {
            return Err(UNSAFE_FILE.into());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn write_files(root: &Path, contents: &[(PathBuf, Vec<u8>)]) -> Result<(), String> {
    for (relative, bytes) in contents {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            io(fs::create_dir_all(parent))?;
        }
        let mut file = io(fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path))?;
        io(file.write_all(bytes))?;
    }
    Ok(())
}

#[cfg(test)]
fn publish_output(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), String> {
    publish_output_cancellable(root, relative, bytes, &AtomicBool::new(false))
}

fn publish_output_cancellable(
    root: &Path,
    relative: &Path,
    bytes: &[u8],
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let parts: Vec<_> = relative.components().collect();
    if parts.is_empty() || parts.iter().any(|part| !matches!(part, Component::Normal(name) if !name.to_string_lossy().contains(':'))) {
        return Err(UNSAFE_FILE.into());
    }
    let mut pins = pin_ancestors(root)?;
    let physical_root = final_path(pins.last().ok_or(UNSAFE_FILE)?)?;
    // While path-based CREATE_NEW creates the fresh temporary sibling, do
    // not permit another writer to turn its parent into a reparse point.
    let mut write_locks = vec![lock_publish_directory(root)?];
    let mut parent = root.to_path_buf();
    for part in &parts[..parts.len() - 1] {
        parent.push(part.as_os_str());
        if !parent.exists() {
            io(fs::create_dir(&parent))?;
        }
        let pin = open_path(&parent, true)?;
        if !final_path(&pin)?.starts_with(&physical_root) {
            return Err(UNSAFE_FILE.into());
        }
        pins.push(pin);
        write_locks.push(lock_publish_directory(&parent)?);
    }
    let destination = root.join(relative);
    if destination.exists() {
        let _checked = open_regular(root, &destination, MAX_TASK_BYTES)?;
    }
    let temporary = parent.join(format!(".ofox-publish-{}", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let path = wide(&temporary)?;
        // The staging name is unpublished. Hold its one CREATE_NEW handle
        // through writing, fsync and the final atomic rename; using the general
        // atomic_write here would perform another parent-opening rename while
        // the temporary write-denying directory guards are still held.
        let created = unsafe {
            win(CreateFileW(
                PCWSTR(path.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0 | 0x10000,
                FILE_SHARE_READ,
                None,
                CREATE_NEW,
                FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            ))?
        };
        let mut owned = PendingOutput {
            file: unsafe { File::from_raw_handle(created.0) },
            committed: false,
        };
        io(owned.file.write_all(bytes))?;
        io(owned.file.sync_all())?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            win(GetFileInformationByHandle(
                HANDLE(owned.file.as_raw_handle()),
                &mut info,
            ))?;
        }
        if info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT.0 | FILE_ATTRIBUTE_DIRECTORY.0)
            != 0
            || info.nNumberOfLinks != 1
            || !final_path(&owned.file)?.starts_with(&physical_root)
        {
            return Err(UNSAFE_FILE.into());
        }
        let filename: Vec<u16> = parts
            .last()
            .ok_or(UNSAFE_FILE)?
            .as_os_str()
            .encode_wide()
            .collect();
        let length = (offset_of!(FILE_RENAME_INFO, FileName) + filename.len() * 2)
            .max(size_of::<FILE_RENAME_INFO>());
        let mut storage = vec![0usize; length.div_ceil(size_of::<usize>())];
        let rename = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        unsafe {
            (*rename).Anonymous.ReplaceIfExists = true;
            (*rename).RootDirectory = HANDLE(pins.last().ok_or(UNSAFE_FILE)?.as_raw_handle());
            (*rename).FileNameLength = (filename.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(
                filename.as_ptr(),
                std::ptr::addr_of_mut!((*rename).FileName).cast::<u16>(),
                filename.len(),
            );
            // The kernel opens the rename target parent for write. The fresh
            // staging file is now owned by handle, so release write-denying
            // guards; keep all no-delete directory handles and use their
            // physical RootDirectory for the final relative rename.
            write_locks.clear();
            #[cfg(test)]
            pause_copyback(root, CopybackPoint::BeforeRename)?;
            // Cancelling during a potentially large staging write/fsync must
            // delete this owned temporary, not publish it afterwards. Only
            // the single native rename itself is non-interruptible.
            if cancelled.load(Ordering::Acquire) {
                return Err(CANCELLED.into());
            }
            win(SetFileInformationByHandle(
                HANDLE(owned.file.as_raw_handle()),
                FileRenameInfo,
                rename.cast(),
                length as u32,
            ))?;
        }
        owned.committed = true;
        Ok(())
    })();
    // pins survive until the atomic, directory-relative final rename completes.
    drop(pins);
    result
}

struct PendingOutput {
    file: File,
    committed: bool,
}
impl Drop for PendingOutput {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // Cleanup remains bound to this CREATE_NEW object if a final rename
        // failed after the temporary directory write guards were released.
        let result = unsafe {
            SetFileInformationByHandle(
                HANDLE(self.file.as_raw_handle()),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        };
        if result.is_err() {
            log::warn!("[Assistant] document output temporary cleanup incomplete");
        }
    }
}

fn lock_publish_directory(path: &Path) -> Result<File, String> {
    let name = wide(path)?;
    let owned = unsafe {
        win(CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        ))?
    };
    let file = unsafe { File::from_raw_handle(owned.0) };
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        win(GetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            &mut information,
        ))?;
    }
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0
    {
        return Err(UNSAFE_FILE.into());
    }
    Ok(file)
}

#[derive(Clone)]
struct CachedRuntime {
    source: PathBuf,
    root: PathBuf,
    digest: String,
}
static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedRuntime>>> = OnceLock::new();
static ADMITTED: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
static PROBE_TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PYTHON_TURN: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();
fn cache_runtime(
    source: &Path,
    capability: &RuntimeCapability,
    user: &str,
) -> Result<CachedRuntime, String> {
    let source = io(source.canonicalize())?;
    let digest = tree_digest_at(&source, MAX_TREE_BYTES)?;
    let cap = sid_string(capability.sid)?;
    let expected_acl = format!(
        "{}(A;OICI;GRGX;;;{cap})S:(ML;OICI;NW;;;ME)",
        private_sddl(user)
    );
    let mut cached = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| UNAVAILABLE)?;
    if let Some(runtime) = cached.get(&source) {
        if runtime.digest == digest {
            let pin = owned_directory(&runtime.root, user)?;
            verify_cache_acl(&pin, &expected_acl)?;
            if tree_digest_at(&runtime.root, MAX_TREE_BYTES)? != digest {
                return Err(UNSAFE_FILE.into());
            }
            return Ok(runtime.clone());
        }
    }
    let private_parent = std::env::temp_dir().join(format!(
        "ofox-document-runtime-{}",
        &format!("{:x}", Sha256::digest(user.as_bytes()))[..16]
    ));
    if !private_parent.exists() {
        create_directory(&private_parent, &private_sddl(user))?;
    }
    let _parent = owned_directory(&private_parent, user)?;
    let root = private_parent.join(&digest);
    if !root.exists() {
        let contents = files(&source, MAX_TREE_BYTES)?;
        if tree_digest(&contents) != digest {
            return Err("assistant_document_sandbox_runtime_changed".into());
        }
        let staging = private_parent.join(format!("staging-{}", uuid::Uuid::new_v4().simple()));
        create_directory(&staging, &expected_acl)?;
        let prepared = write_files(&staging, &contents).and_then(|()| {
            if tree_digest_at(&staging, MAX_TREE_BYTES)? != digest {
                return Err(UNSAFE_FILE.into());
            }
            Ok(())
        });
        if let Err(error) = prepared {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        if fs::rename(&staging, &root).is_err() {
            let _ = fs::remove_dir_all(&staging);
            // A concurrent complete publisher is acceptable only after all
            // the same ownership, ACL and content checks below.
            if !root.exists() {
                return Err(UNAVAILABLE.into());
            }
        }
    }
    let root_handle = owned_directory(&root, user)?;
    verify_cache_acl(&root_handle, &expected_acl)?;
    if tree_digest_at(&root, MAX_TREE_BYTES)? != digest {
        return Err(UNSAFE_FILE.into());
    }
    let runtime = CachedRuntime {
        source: source.clone(),
        root,
        digest,
    };
    cached.insert(source, runtime.clone());
    Ok(runtime)
}

struct Profile {
    name: Vec<u16>,
    sid: Sid,
    directory: Option<PathBuf>,
    pins: Vec<File>,
    root_pinned: bool,
    disposed: bool,
}
impl Profile {
    fn new(capability: &RuntimeCapability) -> Result<Self, String> {
        let name = wide(format!("Ofox.Document.{}", uuid::Uuid::new_v4().simple()))?;
        let capabilities = [capability.entry()];
        let sid = unsafe {
            win(CreateAppContainerProfile(
                PCWSTR(name.as_ptr()),
                PCWSTR(name.as_ptr()),
                PCWSTR(name.as_ptr()),
                Some(&capabilities),
            ))?
        };
        Ok(Self {
            name,
            sid: Sid(sid),
            directory: None,
            pins: Vec::new(),
            root_pinned: false,
            disposed: false,
        })
    }
    fn make_low_profile(&mut self) -> Result<(), String> {
        let sid = wide(sid_string(self.sid.0)?)?;
        let folder = unsafe { win(GetAppContainerFolderPath(PCWSTR(sid.as_ptr())))? };
        let result = unsafe { folder.to_string().map_err(|_| UNAVAILABLE.to_owned()) };
        unsafe {
            CoTaskMemFree(Some(folder.0.cast()));
        }
        let path = PathBuf::from(result?);
        if !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("AC"))
        {
            return Err(UNAVAILABLE.into());
        }
        let path = path.parent().ok_or(UNAVAILABLE)?.to_path_buf();
        // Profile is freshly created, before untrusted code can plant links.
        self.pins = pin_ancestors(&path)?;
        self.root_pinned = true;
        self.directory = Some(path.clone());
        let name = wide(&path)?;
        // LABEL_SECURITY_INFORMATION requires WRITE_OWNER, which is never
        // requested on an engine-created or caller-supplied object.
        let file = unsafe {
            handle(win(CreateFileW(
                PCWSTR(name.as_ptr()),
                FILE_GENERIC_READ.0 | 0x80000,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                None,
            ))?)
        };
        let descriptor = security_descriptor("S:(ML;OICI;NW;;;LW)")?;
        let (mut present, mut defaulted, mut acl) = (BOOL(0), BOOL(0), std::ptr::null_mut());
        unsafe {
            win(GetSecurityDescriptorSacl(
                PSECURITY_DESCRIPTOR(descriptor.0),
                &mut present,
                &mut acl,
                &mut defaulted,
            ))?;
            SetSecurityInfo(
                raw(&file),
                SE_FILE_OBJECT,
                LABEL_SECURITY_INFORMATION,
                None,
                None,
                None,
                Some(acl),
            )
            .ok()
            .map_err(|_| UNAVAILABLE.to_owned())?;
        }
        Ok(())
    }
    fn dispose(&mut self) -> Result<(), String> {
        if self.disposed {
            return Ok(());
        }
        if let Some(directory) = &self.directory {
            if self.root_pinned {
                self.pins.pop();
                self.root_pinned = false;
            }
            match fs::remove_dir_all(directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("assistant_document_sandbox_cleanup_failed".into()),
            }
        }
        unsafe {
            win(DeleteAppContainerProfile(PCWSTR(self.name.as_ptr())))?;
        }
        self.disposed = true;
        Ok(())
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        if self.dispose().is_err() {
            log::warn!("[Assistant] document sandbox profile cleanup incomplete");
        }
    }
}

struct TaskDirectory(PathBuf, Vec<File>, bool, bool);
impl TaskDirectory {
    fn dispose(&mut self) -> Result<(), String> {
        if self.3 {
            return Ok(());
        }
        if self.2 {
            self.1.pop();
            self.2 = false;
        }
        // Windows std::fs::remove_dir_all unlinks junctions/reparse points
        // rather than following them; ancestor handles survive this call.
        match fs::remove_dir_all(&self.0) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("assistant_document_sandbox_cleanup_failed".into()),
        }
        self.3 = true;
        Ok(())
    }
}
impl Drop for TaskDirectory {
    fn drop(&mut self) {
        if self.dispose().is_err() {
            log::warn!("[Assistant] document sandbox task cleanup incomplete");
        }
    }
}
fn task_directory(user: &str, sid: PSID) -> Result<TaskDirectory, String> {
    let parent = std::env::temp_dir();
    let path = parent.join(format!(
        "ofox-document-task-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let package = sid_string(sid)?;
    create_directory(
        &path,
        &format!(
            "{}(A;OICI;0x1301bf;;;{package})S:(ML;OICI;NW;;;LW)",
            private_sddl(user)
        ),
    )?;
    let pins = pin_ancestors(&path)?;
    Ok(TaskDirectory(path, pins, true, false))
}

fn os_version() -> Result<OSVERSIONINFOW, String> {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    unsafe {
        RtlGetVersion(&mut version)
            .ok()
            .map_err(|_| UNAVAILABLE.to_owned())?;
    }
    if version.dwMajorVersion < 10 || version.dwBuildNumber < 17763 {
        return Err(UNAVAILABLE.into());
    }
    Ok(version)
}
fn environment(home: &Path) -> Result<Vec<u16>, String> {
    let system = std::env::var_os("SystemRoot").ok_or(UNAVAILABLE)?;
    if !Path::new(&system).is_absolute() {
        return Err(UNAVAILABLE.into());
    }
    let mut values = vec![
        ("SystemRoot", system.clone()),
        ("WINDIR", system.clone()),
        (
            "PATH",
            PathBuf::from(&system).join("System32").into_os_string(),
        ),
    ];
    for key in [
        "HOME",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "TMP",
        "TEMP",
        "TMPDIR",
    ] {
        values.push((key, home.as_os_str().to_owned()));
    }
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        values.push((key, DEAD_PROXY.into()));
    }
    for (key, value) in [
        ("NO_PROXY", ""),
        ("no_proxy", ""),
        ("OFFICECLI_NO_AUTO_RESIDENT", "1"),
        ("OFFICECLI_SKIP_UPDATE", "1"),
    ] {
        values.push((key, value.into()));
    }
    values.sort_by_key(|(key, _)| key.to_ascii_uppercase());
    // Windows environment names are case-insensitive: retain one spelling.
    values.dedup_by(|a, b| a.0.eq_ignore_ascii_case(b.0));
    let mut block = Vec::new();
    for (key, value) in values {
        let mut entry = wide(format!("{key}={}", value.to_string_lossy()))?;
        block.append(&mut entry);
    }
    block.push(0);
    Ok(block)
}
fn quote(arg: &OsStr) -> Result<String, String> {
    let text = arg.to_str().ok_or(UNAVAILABLE)?;
    if text.contains('\0') {
        return Err(UNAVAILABLE.into());
    }
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in text.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n('\\', slashes));
        }
        slashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    Ok(out)
}

struct Attributes {
    storage: Vec<usize>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Attributes {
    fn new(count: u32) -> Result<Self, String> {
        let mut length = 0;
        unsafe {
            let _ = InitializeProcThreadAttributeList(None, count, None, &mut length);
        }
        if length == 0 {
            return Err(UNAVAILABLE.into());
        }
        let mut storage = vec![0usize; length.div_ceil(size_of::<usize>())];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        unsafe {
            win(InitializeProcThreadAttributeList(
                Some(list),
                count,
                None,
                &mut length,
            ))?;
        }
        Ok(Self { storage, list })
    }
    fn set<T>(&mut self, key: u32, value: &T) -> Result<(), String> {
        debug_assert!(!self.storage.is_empty());
        unsafe {
            win(UpdateProcThreadAttribute(
                self.list,
                0,
                key as usize,
                Some((value as *const T).cast()),
                size_of::<T>(),
                None,
                None,
            ))
        }
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.list);
        }
    }
}

fn pipe() -> Result<(OwnedHandle, OwnedHandle), String> {
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    let attrs = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        bInheritHandle: BOOL(1),
        ..Default::default()
    };
    unsafe {
        win(CreatePipe(&mut read, &mut write, Some(&attrs), 0))?;
    }
    Ok((handle(read), handle(write)))
}
struct Process {
    process: OwnedHandle,
    job: OwnedHandle,
}
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            let _ = TerminateJobObject(raw(&self.job), 1);
            let _ = TerminateProcess(raw(&self.process), 1);
            let _ = WaitForSingleObject(raw(&self.process), 5000);
        }
    }
}
fn verify_token(process: HANDLE, package: PSID, capability: PSID) -> Result<(), String> {
    let mut token = HANDLE::default();
    unsafe {
        win(OpenProcessToken(process, TOKEN_QUERY, &mut token))?;
    }
    let token = handle(token);
    for kind in [TokenIsAppContainer, TokenIsLessPrivilegedAppContainer] {
        if token_buffer(raw(&token), kind)?[0] as u32 != 1 {
            return Err(UNAVAILABLE.into());
        }
    }
    let package_data = token_buffer(raw(&token), TokenAppContainerSid)?;
    let actual_package = unsafe {
        &*package_data
            .as_ptr()
            .cast::<TOKEN_APPCONTAINER_INFORMATION>()
    };
    unsafe {
        win(EqualSid(actual_package.TokenAppContainer, package))?;
    }
    let capabilities = token_buffer(raw(&token), TokenCapabilities)?;
    let groups = unsafe { &*capabilities.as_ptr().cast::<TOKEN_GROUPS>() };
    if groups.GroupCount != 1 {
        return Err(UNAVAILABLE.into());
    }
    unsafe {
        win(EqualSid(groups.Groups[0].Sid, capability))?;
    }
    let data = token_buffer(raw(&token), TokenIntegrityLevel)?;
    let label = unsafe { &*data.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
    let integrity = unsafe {
        *GetSidSubAuthority(
            label.Label.Sid,
            (*GetSidSubAuthorityCount(label.Label.Sid) as u32)
                .checked_sub(1)
                .ok_or(UNAVAILABLE)?,
        )
    };
    if integrity > 4096 {
        return Err(UNAVAILABLE.into());
    }
    // Both policies have a DWORD flags layout. Verify actual kernel state,
    // not just successful construction of the startup attribute list.
    let (mut child, mut image) = (0u32, 0u32);
    unsafe {
        win(GetProcessMitigationPolicy(
            process,
            ProcessChildProcessPolicy,
            (&mut child as *mut u32).cast(),
            size_of::<u32>(),
        ))?;
        win(GetProcessMitigationPolicy(
            process,
            ProcessImageLoadPolicy,
            (&mut image as *mut u32).cast(),
            size_of::<u32>(),
        ))?;
    }
    if child & 1 == 0 || image & 3 != 3 {
        return Err(UNAVAILABLE.into());
    }
    Ok(())
}

struct Launch {
    process: Process,
    stdout: std::sync::mpsc::Receiver<Vec<u8>>,
    stderr: std::sync::mpsc::Receiver<Vec<u8>>,
}
fn drain(handle: OwnedHandle, limit: usize) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (send, recv) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut file = File::from(handle);
        let (mut bytes, mut chunk) = (Vec::new(), [0u8; 8192]);
        while let Ok(n) = file.read(&mut chunk) {
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n.min(limit.saturating_sub(bytes.len()))]);
        }
        let _ = send.send(bytes);
    });
    recv
}
#[allow(clippy::too_many_arguments)]
fn launch(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    home: &Path,
    profile: &Profile,
    cap: &RuntimeCapability,
    limit: usize,
) -> Result<Launch, String> {
    let app = wide(exe)?;
    let mut cmd = quote(exe.as_os_str())?;
    for arg in args {
        cmd.push(' ');
        cmd.push_str(&quote(OsStr::new(arg))?);
    }
    let mut command = wide(cmd)?;
    let cwd = wide(cwd)?;
    let env = environment(home)?;
    let (stdin_read, stdin_write) = pipe()?;
    let (stdout_read, stdout_write) = pipe()?;
    let (stderr_read, stderr_write) = pipe()?;
    unsafe {
        win(SetHandleInformation(
            raw(&stdout_read),
            HANDLE_FLAG_INHERIT.0,
            HANDLE_FLAGS(0),
        ))?;
        win(SetHandleInformation(
            raw(&stderr_read),
            HANDLE_FLAG_INHERIT.0,
            HANDLE_FLAGS(0),
        ))?;
    }
    drop(stdin_write);
    let inherited = [raw(&stdin_read), raw(&stdout_write), raw(&stderr_write)];
    let mut capability = [cap.entry()];
    let security = SECURITY_CAPABILITIES {
        AppContainerSid: profile.sid.0,
        Capabilities: capability.as_mut_ptr(),
        CapabilityCount: 1,
        Reserved: 0,
    };
    let mitigations = NO_REMOTE_IMAGES | NO_LOW_LABEL_IMAGES | PREFER_SYSTEM32;
    let mut attributes = Attributes::new(5)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &security)?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
        &LPAC_OPT_OUT,
    )?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY,
        &CHILD_RESTRICTED,
    )?;
    attributes.set(PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, &mitigations)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited)?;
    let mut startup = STARTUPINFOEXW {
        lpAttributeList: attributes.list,
        ..Default::default()
    };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = inherited[0];
    startup.StartupInfo.hStdOutput = inherited[1];
    startup.StartupInfo.hStdError = inherited[2];
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
    let mut info = PROCESS_INFORMATION::default();
    unsafe {
        win(CreateProcessW(
            PCWSTR(app.as_ptr()),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_NO_WINDOW
                | CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            Some(env.as_ptr().cast()),
            PCWSTR(cwd.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        ))?;
    }
    let process = Process {
        process: handle(info.hProcess),
        job,
    };
    let thread = handle(info.hThread);
    unsafe {
        win(AssignProcessToJobObject(
            raw(&process.job),
            raw(&process.process),
        ))?;
    }
    verify_token(raw(&process.process), profile.sid.0, cap.sid)?;
    if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
        return Err(UNAVAILABLE.into());
    }
    drop(thread);
    drop(stdin_read);
    drop(stdout_write);
    drop(stderr_write);
    Ok(Launch {
        process,
        stdout: drain(stdout_read, limit),
        stderr: drain(stderr_read, limit),
    })
}

fn remap(
    value: &str,
    runtime: &CachedRuntime,
    source: &Path,
    task: &Path,
) -> Result<String, String> {
    let path = Path::new(value);
    if !path.is_absolute() {
        return Ok(value.to_owned());
    }
    for (from, to) in [
        (runtime.source.as_path(), runtime.root.as_path()),
        (source, task),
    ] {
        if let Ok(relative) = path.strip_prefix(from) {
            return Ok(to.join(relative).to_string_lossy().into_owned());
        }
        // Host APIs often return verbatim canonical paths while CLI arguments
        // were built before canonicalization. Compare their canonical forms.
        if let Ok(real) = path.canonicalize() {
            if let Ok(relative) = real.strip_prefix(from) {
                return Ok(to.join(relative).to_string_lossy().into_owned());
            }
        }
    }
    Err(UNSAFE_FILE.into())
}
fn bounded_text(bytes: Vec<u8>, limit: usize) -> String {
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    while text.len() > limit {
        text.pop();
    }
    text
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static RUN_RECEIPTS: OnceLock<Mutex<VecDeque<serde_json::Value>>> = OnceLock::new();
#[cfg(test)]
static ACTIVE_TASKS: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum CopybackPoint {
    Drained,
    Published(usize),
    BeforeRename,
}
#[cfg(test)]
struct CopybackGate {
    point: CopybackPoint,
    reached: std::sync::mpsc::SyncSender<()>,
    resume: Mutex<std::sync::mpsc::Receiver<()>>,
}
#[cfg(test)]
static COPYBACK_GATES: OnceLock<Mutex<HashMap<PathBuf, Arc<CopybackGate>>>> = OnceLock::new();
#[cfg(test)]
fn pause_copyback(cwd: &Path, point: CopybackPoint) -> Result<(), String> {
    let gate = COPYBACK_GATES
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .get(cwd)
        .filter(|gate| gate.point == point)
        .cloned();
    if let Some(gate) = gate {
        gate.reached.try_send(()).map_err(|_| UNAVAILABLE)?;
        gate.resume
            .lock()
            .map_err(|_| UNAVAILABLE)?
            .recv_timeout(Duration::from_secs(30))
            .map_err(|_| UNAVAILABLE)?;
    }
    Ok(())
}
#[cfg(test)]
struct ActiveTask(PathBuf);
#[cfg(test)]
impl ActiveTask {
    fn new(original: PathBuf, sandbox: PathBuf) -> Self {
        ACTIVE_TASKS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(original.clone(), sandbox);
        Self(original)
    }
}
#[cfg(test)]
impl Drop for ActiveTask {
    fn drop(&mut self) {
        ACTIVE_TASKS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .remove(&self.0);
    }
}
/// In-memory evidence for native acceptance. A platform cfg or a successful
/// fallback cannot manufacture one of these receipts. Retain only 512 runs.
pub fn windows_document_sandbox_receipts(after_sequence: u64) -> Vec<serde_json::Value> {
    RUN_RECEIPTS
        .get_or_init(Default::default)
        .lock()
        .map(|items| {
            items
                .iter()
                .filter(|item| item["sequence"].as_u64().unwrap_or_default() > after_sequence)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Same result/limits as the ordinary runner, with a mandatory LPAC boundary.
/// The sandbox never receives ACL access to the host's original document tree.
#[allow(clippy::too_many_arguments)]
pub async fn run_windows_engine_limited(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    home: &Path,
    read_root: &Path,
    timeout: Duration,
    limit: usize,
) -> Result<EngineOutput, String> {
    run_candidate(exe, args, cwd, home, read_root, timeout, limit, false).await
}

#[allow(clippy::too_many_arguments)]
async fn run_candidate(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    home: &Path,
    read_root: &Path,
    timeout: Duration,
    limit: usize,
    trusted_probe: bool,
) -> Result<EngineOutput, String> {
    let python = args
        .first()
        .is_some_and(|arg| Path::new(arg).ends_with("python/runner.mjs"));
    let python_turn = if python {
        Some(
            PYTHON_TURN
                .get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
                .lock_owned()
                .await,
        )
    } else {
        None
    };
    let (exe, args, cwd, home, root) = (
        exe.to_path_buf(),
        args.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>(),
        cwd.to_path_buf(),
        home.to_path_buf(),
        read_root.to_path_buf(),
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    let guard = CancelOnDrop(cancelled.clone());
    let answer = tokio::task::spawn_blocking(move || {
        // Keep this owned guard in the worker: cancelling the awaiting future
        // must not admit another Pyodide before its Job/disposal is complete.
        let _python_turn = python_turn;
        run_blocking(
            &exe,
            &args,
            &cwd,
            &home,
            &root,
            timeout,
            limit,
            &cancelled,
            trusted_probe,
        )
    })
    .await
    .map_err(|_| UNAVAILABLE.to_owned())?;
    drop(guard);
    answer
}
#[allow(clippy::too_many_arguments)]
fn run_blocking(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    home: &Path,
    read_root: &Path,
    timeout: Duration,
    limit: usize,
    cancelled: &AtomicBool,
    trusted_probe: bool,
) -> Result<EngineOutput, String> {
    os_version()?;
    let user = current_user()?;
    let capability = RuntimeCapability::new()?;
    let runtime = cache_runtime(read_root, &capability, &user)?;
    if !trusted_probe {
        let admitted = ADMITTED
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| UNAVAILABLE)?;
        if admitted.get(&runtime.source) != Some(&runtime.digest) {
            return Err("assistant_document_sandbox_unproven_runtime".into());
        }
    }
    let cwd = io(cwd.canonicalize())?;
    let home = io(home.canonicalize())?;
    let home_relative = home
        .strip_prefix(&cwd)
        .map_err(|_| UNSAFE_FILE.to_owned())?;
    if home_relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(UNSAFE_FILE.into());
    }
    let inputs = files(&cwd, MAX_TASK_BYTES)?;
    let mut profile = Profile::new(&capability)?;
    if let Err(error) = profile.make_low_profile() {
        dispose_profile(&mut profile)?;
        return Err(error);
    }
    let mut task = match task_directory(&user, profile.sid.0) {
        Ok(task) => task,
        Err(error) => {
            dispose_profile(&mut profile)?;
            return Err(error);
        }
    };
    #[cfg(test)]
    let mut active = None;
    let executed: Result<(EngineOutput, serde_json::Value, bool), String> = (|| {
        write_files(&task.0, &inputs)?;
        let task_home = task.0.join(home_relative);
        io(fs::create_dir_all(&task_home))?;
        let actual_exe = PathBuf::from(remap(&exe.to_string_lossy(), &runtime, &cwd, &task.0)?);
        let actual_args = args
            .iter()
            .map(|arg| remap(arg, &runtime, &cwd, &task.0))
            .collect::<Result<Vec<_>, _>>()?;
        if cancelled.load(Ordering::Acquire) {
            return Err(CANCELLED.into());
        }
        let launched = launch(
            &actual_exe,
            &actual_args,
            &task.0,
            &task_home,
            &profile,
            &capability,
            limit,
        )?;
        #[cfg(test)]
        {
            active = Some(ActiveTask::new(cwd.clone(), task.0.clone()));
        }
        let deadline = Instant::now() + timeout;
        let mut was_cancelled = false;
        let timed_out = loop {
            let state = unsafe { WaitForSingleObject(raw(&launched.process.process), 25) };
            if state == WAIT_OBJECT_0 {
                break false;
            }
            if state != WAIT_TIMEOUT {
                return Err(UNAVAILABLE.into());
            }
            if cancelled.load(Ordering::Acquire) {
                was_cancelled = true;
                break false;
            }
            if Instant::now() >= deadline {
                break true;
            }
        };
        let mut status = 0u32;
        if !timed_out && !was_cancelled {
            unsafe {
                win(GetExitCodeProcess(
                    raw(&launched.process.process),
                    &mut status,
                ))?;
            }
        }
        // Stop/wait the whole owned Job before any privileged read or cleanup.
        unsafe {
            win(TerminateJobObject(raw(&launched.process.job), 1))?;
        }
        if unsafe { WaitForSingleObject(raw(&launched.process.process), 5000) } != WAIT_OBJECT_0 {
            return Err(UNAVAILABLE.into());
        }
        let stopped_by = Instant::now() + Duration::from_secs(5);
        loop {
            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            unsafe {
                win(QueryInformationJobObject(
                    Some(raw(&launched.process.job)),
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    None,
                ))?;
            }
            if accounting.ActiveProcesses == 0 {
                break;
            }
            if Instant::now() >= stopped_by {
                return Err(UNAVAILABLE.into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let stdout = launched
            .stdout
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| UNAVAILABLE.to_owned())?;
        let stderr = launched
            .stderr
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| UNAVAILABLE.to_owned())?;
        #[cfg(test)]
        pause_copyback(&cwd, CopybackPoint::Drained)?;
        was_cancelled |= cancelled.load(Ordering::Acquire);
        if !timed_out && !was_cancelled && status == 0 {
            let outputs = files(&task.0, MAX_TASK_BYTES)?;
            // Original cwd is never granted to the LPAC. Keep its directory handle
            // pinned, and reject pre-existing aliases before atomic replacement.
            let _destination = pin_ancestors(&cwd)?;
            #[cfg(test)]
            let mut published = 0;
            for (relative, bytes) in outputs {
                if cancelled.load(Ordering::Acquire) {
                    was_cancelled = true;
                    break;
                }
                if relative.starts_with(home_relative) {
                    continue;
                }
                match publish_output_cancellable(&cwd, &relative, &bytes, cancelled) {
                    Ok(()) => {}
                    Err(error) if error == CANCELLED => {
                        was_cancelled = true;
                        break;
                    }
                    Err(error) => return Err(error),
                }
                #[cfg(test)]
                {
                    published += 1;
                    pause_copyback(&cwd, CopybackPoint::Published(published))?;
                }
            }
        }
        let receipt = serde_json::json!({
            "runId":uuid::Uuid::new_v4().to_string(), "runtimeSha256":runtime.digest,
            "executable":actual_exe.strip_prefix(&runtime.root).map_err(|_| UNSAFE_FILE)?.to_string_lossy(),
            "policy":"lpac-v1", "tokenVerified":true, "childProcessDenied":true,
            "lowImageDenied":true, "jobStopped":true, "timedOut":timed_out,
            "trustedProbe":trusted_probe, "cancelled":was_cancelled,
            "taskId":task.0.file_name().ok_or(UNSAFE_FILE)?.to_string_lossy(),
            "status":(!timed_out && !was_cancelled).then_some(status as i32)
        });
        drop(launched);
        Ok((
            EngineOutput {
                status: (!timed_out && !was_cancelled).then_some(status as i32),
                stdout: bounded_text(stdout, limit),
                stderr: bounded_text(stderr, limit),
                timed_out,
            },
            receipt,
            was_cancelled,
        ))
    })();
    let task_cleanup = task.dispose();
    let profile_cleanup = profile.dispose();
    if task_cleanup.is_err() || profile_cleanup.is_err() {
        return Err(cleanup_failed());
    }
    let (mut output, mut receipt, was_cancelled) = executed?;
    let was_cancelled = was_cancelled || cancelled.load(Ordering::Acquire);
    if was_cancelled {
        output.status = None;
        receipt["status"] = serde_json::Value::Null;
        receipt["cancelled"] = serde_json::json!(true);
    }
    receipt["sequence"] = serde_json::json!(RUN_SEQUENCE.fetch_add(1, Ordering::AcqRel) + 1);
    receipt["cleanupComplete"] = serde_json::json!(true);
    receipt["taskRemoved"] = serde_json::json!(true);
    receipt["profileRemoved"] = serde_json::json!(true);
    if let Ok(mut receipts) = RUN_RECEIPTS.get_or_init(Default::default).lock() {
        receipts.push_back(receipt);
        if receipts.len() > 512 {
            receipts.pop_front();
        }
    }
    #[cfg(test)]
    drop(active);
    if was_cancelled {
        return Err(CANCELLED.into());
    }
    Ok(output)
}

// Deliberately embedded trusted code. The acceptance binary never accepts
// arbitrary Python/JavaScript, host files, gateway URLs, or model credentials.
const NODE_PROBE: &str = r#"
const fs = require('node:fs');
const assert = require('node:assert/strict');
const net = require('node:net');
const cp = require('node:child_process');
const outside = OUTSIDE_PATH;
const outsideAap = OUTSIDE_AAP_PATH;
assert.equal(process.versions.node, '22.23.4');
assert.throws(() => fs.readFileSync(outside));
assert.throws(() => fs.readFileSync(outsideAap), 'LPAC read an ALL APPLICATION PACKAGES fixture');
assert.throws(() => fs.writeFileSync(outside, 'changed'));
assert.throws(() => fs.writeFileSync(process.execPath, 'changed'));
for (const name of ['NODE_OPTIONS', 'NODE_PATH', 'OFOX_DOCUMENT_PROBE_SECRET'])
  assert.equal(process.env[name], undefined);
const spawned = cp.spawnSync(process.execPath, ['-e', "require('node:fs').writeFileSync('child-marker', 'bad')"]);
assert.ok(spawned.error, 'child process creation was allowed');
assert.equal(fs.existsSync('child-marker'), false);
const native = process.argv[1];
require(native); // Prove the original binary actually loads before the denial.
fs.copyFileSync(native, 'untrusted.node');
assert.throws(() => require(require('node:path').resolve('untrusted.node')),
  'a DLL copied into the writable low-label workspace loaded');
fs.unlinkSync('untrusted.node');
const canvas = require(process.argv[2]).createCanvas(320, 180);
canvas.getContext('2d').fillText('Windows 中文 日本語', 10, 40);
fs.writeFileSync('probe.png', canvas.toBuffer('image/png'));
const socket = net.connect({host: '127.0.0.1', port: PROBE_PORT});
socket.on('connect', () => { console.error('network was allowed'); process.exit(1); });
socket.on('error', () => { fs.writeFileSync('isolation.json', JSON.stringify({outsideRead:false,outsideWrite:false,child:false,lowImage:false,network:false,canvas:true})); process.exit(0); });
setTimeout(() => { socket.destroy(); console.error('network refusal was not observed'); process.exit(1); }, 3000);
"#;
const PYTHON_PROBE: &str = "prs = new_deck(title='Windows engine probe')\ncover_slide(prs, 'Windows engine probe', subtitle='中文 日本語')\n";

fn aap_read_fixture(path: &Path) -> Result<(), String> {
    let user = current_user()?;
    let descriptor = security_descriptor(&format!(
        "D:P(A;;FA;;;SY)(A;;FA;;;{user})(A;;GR;;;S-1-15-2-1)"
    ))?;
    let name = wide(path)?;
    let file = unsafe {
        handle(win(CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_GENERIC_READ.0 | 0x40000,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        ))?)
    };
    let (mut present, mut defaulted, mut acl) = (BOOL(0), BOOL(0), std::ptr::null_mut());
    unsafe {
        win(GetSecurityDescriptorDacl(
            PSECURITY_DESCRIPTOR(descriptor.0),
            &mut present,
            &mut acl,
            &mut defaulted,
        ))?;
        SetSecurityInfo(
            raw(&file),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(acl),
            None,
        )
        .ok()
        .map_err(|_| UNAVAILABLE.to_owned())?;
    }
    Ok(())
}

fn success(output: EngineOutput) -> Result<EngineOutput, String> {
    if output.status != Some(0) || output.timed_out {
        return Err(UNAVAILABLE.into());
    }
    Ok(output)
}

/// Prove both engines and the launcher's negative properties. This function
/// creates only temporary invented data, never reads account configuration.
pub async fn probe_windows_document_engines(
    runtime_root: &Path,
    officecli: &Path,
) -> Result<(), String> {
    prove_runtime(runtime_root, officecli, &mut Vec::new())
        .await
        .map(|_| ())
}

/// Call before bootstrap/send. The returned digest identifies the exact public
/// bytes whose positive and negative native probes passed.
pub async fn ensure_windows_document_engine_ready(runtime_root: &Path) -> Result<String, String> {
    let result = ensure_ready(runtime_root).await;
    if result.is_err() {
        invalidate_admission();
    }
    result
}

fn invalidate_admission() {
    // There is one production runtime. Clear every remembered hint on a failed
    // recheck, including disappearance/unreadability before canonicalization.
    if let Ok(mut proofs) = ADMITTED.get_or_init(Default::default).lock() {
        proofs.clear();
    }
}

fn cleanup_failed() -> String {
    invalidate_admission();
    "assistant_document_sandbox_cleanup_failed".into()
}

fn dispose_profile(profile: &mut Profile) -> Result<(), String> {
    profile.dispose().map_err(|_| cleanup_failed())
}

async fn ensure_ready(runtime_root: &Path) -> Result<String, String> {
    let root = runtime_root.to_path_buf();
    let runtime = tokio::task::spawn_blocking(move || {
        os_version()?;
        cache_runtime(&root, &RuntimeCapability::new()?, &current_user()?)
    })
    .await
    .map_err(|_| UNAVAILABLE.to_owned())??;
    let ready = ADMITTED
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .get(&runtime.source)
        == Some(&runtime.digest);
    if ready {
        return Ok(runtime.digest);
    }
    let _turn = PROBE_TURN.lock().await;
    // Another bootstrap/send may have completed all probes while we waited.
    let root = runtime_root.to_path_buf();
    let runtime = tokio::task::spawn_blocking(move || {
        cache_runtime(&root, &RuntimeCapability::new()?, &current_user()?)
    })
    .await
    .map_err(|_| UNAVAILABLE.to_owned())??;
    if ADMITTED
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .get(&runtime.source)
        == Some(&runtime.digest)
    {
        return Ok(runtime.digest);
    }
    prove_runtime_locked(
        runtime_root,
        &runtime_root.join("engines/officecli/officecli.exe"),
        &mut Vec::new(),
    )
    .await
}

/// Presentation hint only. Actual execution always hashes the bytes again and
/// requires a matching proof, even if this remembered hint is true.
pub fn windows_document_engine_proven(runtime_root: &Path) -> bool {
    let Ok(root) = runtime_root.canonicalize() else {
        return false;
    };
    let Some(caches) = CACHE.get() else {
        return false;
    };
    let Ok(caches) = caches.lock() else {
        return false;
    };
    let Some(runtime) = caches.get(&root) else {
        return false;
    };
    ADMITTED
        .get_or_init(Default::default)
        .lock()
        .is_ok_and(|proofs| proofs.get(&root) == Some(&runtime.digest))
}

async fn prove_runtime(
    runtime_root: &Path,
    officecli: &Path,
    checks: &mut Vec<&'static str>,
) -> Result<String, String> {
    let _turn = PROBE_TURN.lock().await;
    prove_runtime_locked(runtime_root, officecli, checks).await
}

async fn prove_runtime_locked(
    runtime_root: &Path,
    officecli: &Path,
    checks: &mut Vec<&'static str>,
) -> Result<String, String> {
    let root = runtime_root.to_path_buf();
    let before = tokio::task::spawn_blocking(move || {
        cache_runtime(&root, &RuntimeCapability::new()?, &current_user()?)
    })
    .await
    .map_err(|_| UNAVAILABLE.to_owned())??;
    ADMITTED
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .remove(&before.source);
    probe_inner(runtime_root, officecli, checks).await?;
    let root = runtime_root.to_path_buf();
    let after = tokio::task::spawn_blocking(move || {
        cache_runtime(&root, &RuntimeCapability::new()?, &current_user()?)
    })
    .await
    .map_err(|_| UNAVAILABLE.to_owned())??;
    if before.digest != after.digest {
        return Err("assistant_document_sandbox_runtime_changed".into());
    }
    ADMITTED
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .insert(after.source, after.digest.clone());
    Ok(after.digest)
}

async fn probe_inner(
    runtime_root: &Path,
    officecli: &Path,
    checks: &mut Vec<&'static str>,
) -> Result<(), String> {
    os_version()?;
    checks.push("windowsBuild");
    let runtime_root = io(runtime_root.canonicalize())?;
    let fixtures = io(tempfile::tempdir())?;
    let cwd = fixtures.path().join("document 空 格");
    let home = cwd.join("home");
    io(fs::create_dir_all(&home))?;
    let outside = fixtures.path().join("outside-credential-fixture.txt");
    io(fs::write(&outside, b"synthetic private data"))?;
    let outside_aap = fixtures.path().join("outside-aap-fixture.txt");
    io(fs::write(&outside_aap, b"synthetic package-readable data"))?;
    aap_read_fixture(&outside_aap)?;
    let listener = io(std::net::TcpListener::bind((
        std::net::Ipv4Addr::LOCALHOST,
        0,
    )))?;
    // Positive control: an ordinary process can reach the listener.
    let address = io(listener.local_addr())?;
    let control = io(std::net::TcpStream::connect(address))?;
    let _ = io(listener.accept())?;
    drop(control);
    io(listener.set_nonblocking(true))?;
    let script = NODE_PROBE
        .replace(
            "OUTSIDE_PATH",
            &serde_json::to_string(&outside).map_err(|_| UNAVAILABLE)?,
        )
        .replace(
            "OUTSIDE_AAP_PATH",
            &serde_json::to_string(&outside_aap).map_err(|_| UNAVAILABLE)?,
        )
        .replace("PROBE_PORT", &address.port().to_string());
    let native = runtime_root
        .join("runtime/node_modules/@napi-rs/canvas-win32-x64-msvc/skia.win32-x64-msvc.node");
    let canvas = runtime_root.join("runtime/node_modules/@napi-rs/canvas");
    let node = runtime_root.join("node/node.exe");
    success(
        run_candidate(
            &node,
            &[
                "-e",
                &script,
                &native.to_string_lossy(),
                &canvas.to_string_lossy(),
            ],
            &cwd,
            &home,
            &runtime_root,
            Duration::from_secs(30),
            16 * 1024,
            true,
        )
        .await?,
    )?;
    if listener.accept().is_ok() || io(fs::read(&outside))? != b"synthetic private data" {
        return Err(UNAVAILABLE.into());
    }
    checks.extend([
        "outsideReadWriteDenied",
        "allApplicationPackagesReadDenied",
        "childProcessDenied",
        "lowImageDenied",
        "networkDenied",
        "nativeCanvas",
    ]);
    io(fs::write(cwd.join("code.py"), PYTHON_PROBE))?;
    let runner = runtime_root.join("runtime/src/python/runner.mjs");
    let assets = runtime_root.join("engines/python");
    let args = [
        runner.to_string_lossy().into_owned(),
        "--assets".into(),
        assets.to_string_lossy().into_owned(),
        "--code".into(),
        "code.py".into(),
        "--out".into(),
        "out.pptx".into(),
        "--lint".into(),
        "lint.json".into(),
    ];
    success(
        run_candidate(
            &node,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            &cwd,
            &home,
            &runtime_root,
            Duration::from_secs(90),
            16 * 1024,
            true,
        )
        .await?,
    )?;
    let deck = cwd.join("out.pptx");
    let mut bytes = Vec::new();
    io(open_regular(&cwd, &deck, 25 * 1024 * 1024)?.read_to_end(&mut bytes))?;
    if !bytes.starts_with(b"PK") {
        return Err(UNAVAILABLE.into());
    }
    checks.push("pythonPptx");
    for args in [
        vec!["validate", "out.pptx", "--json"],
        vec!["view", "out.pptx", "svg", "--start", "1", "--end", "1"],
    ] {
        let output = success(
            run_candidate(
                officecli,
                &args,
                &cwd,
                &home,
                &runtime_root,
                Duration::from_secs(60),
                4 * 1024 * 1024,
                true,
            )
            .await?,
        )?;
        if args[0] == "validate" {
            let report: serde_json::Value =
                serde_json::from_str(&output.stdout).map_err(|_| UNAVAILABLE)?;
            if report["success"] == false || report["data"]["valid"] == false {
                return Err(UNAVAILABLE.into());
            }
            checks.push("officeValidation");
        } else {
            if !output.stdout.contains("<svg") {
                return Err(UNAVAILABLE.into());
            }
            checks.push("officeSvg");
        }
    }
    // Exercise the same runner's frozen source-deck/hash/target-page inputs.
    io(fs::write(cwd.join("source-deck.pptx"), &bytes))?;
    let meta = serde_json::json!({"version":format!("{:x}", Sha256::digest(&bytes)),"totalPages":1,"pageNumbers":[1]});
    io(fs::write(
        cwd.join("source-meta.json"),
        serde_json::to_vec(&meta).map_err(|_| UNAVAILABLE)?,
    ))?;
    io(fs::write(
        cwd.join("code.py"),
        "prs = source_deck()\ntarget_slides(prs)[0].shapes.title.text = 'Edited Windows probe'\n",
    ))?;
    let mut edit_args = args.to_vec();
    edit_args.extend([
        "--source-deck".into(),
        cwd.join("source-deck.pptx").to_string_lossy().into_owned(),
        "--source-meta".into(),
        cwd.join("source-meta.json").to_string_lossy().into_owned(),
    ]);
    success(
        run_candidate(
            &node,
            &edit_args.iter().map(String::as_str).collect::<Vec<_>>(),
            &cwd,
            &home,
            &runtime_root,
            Duration::from_secs(90),
            16 * 1024,
            true,
        )
        .await?,
    )?;
    if io(fs::read(cwd.join("source-deck.pptx")))? != bytes {
        return Err(UNAVAILABLE.into());
    }
    checks.push("frozenSourceEdit");
    Ok(())
}

/// A redacted report suitable for the one-click client acceptance package.
/// Successful fallback is deliberately not a successful Python acceptance.
pub async fn verify_windows_document_engines(runtime_root: &Path) -> serde_json::Value {
    let version = os_version().ok();
    let mut checks = Vec::new();
    let result = prove_runtime(
        runtime_root,
        &runtime_root.join("engines/officecli/officecli.exe"),
        &mut checks,
    )
    .await;
    let provenance = std::env::current_exe()
        .ok()
        .and_then(|path| fs::read(path).ok())
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
    let runtime_digest = runtime_root.canonicalize().ok().and_then(|root| {
        CACHE
            .get()?
            .lock()
            .ok()?
            .get(&root)
            .map(|entry| entry.digest.clone())
    });
    serde_json::json!({
        "schemaVersion": 1, "platform":"windows-x64", "success":result.is_ok(),
        "windowsBuild":version.map(|v| v.dwBuildNumber), "binarySha256":provenance,
        "runtimeSha256":runtime_digest, "nodeVersion":"22.23.4", "pyodideVersion":"314.0.7",
        "passedChecks":checks, "diagnostic":result.err(),
        "scope":"native-fixed-fixtures", "paidModelCalls":0,
        "liveModelAcceptance":false, "pythonCapabilityEnabled":false
    })
}

#[cfg(test)]
mod tests;
