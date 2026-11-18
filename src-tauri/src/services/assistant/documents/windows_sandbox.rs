//! Windows document-only LPAC launcher. This is deliberately separate from the
//! general assistant process runner: a Job alone is not a security boundary.
//!
//! No caller may advertise Python merely because this module compiles. The
//! native functional/negative probes must pass on the actual installed assets.
use super::runner::{EngineOutput, DEAD_PROXY};
mod capability_set;
#[cfg(test)]
mod cdb;
#[cfg(test)]
mod cdb_protocol;
mod probe;
#[cfg(test)]
mod startup_comparison;
#[cfg(test)]
mod startup_comparison_protocol;
#[cfg(test)]
mod trace_launch;
#[cfg(test)]
mod winsock_grant;
use probe::{
    AccessCase, AccessObservation, Observer, Phase, Receive, RegistryAccess, RegistryKey, TokenApi,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    ffi::{c_void, OsStr},
    fs::{self, File},
    io::{Read, Write},
    mem::size_of,
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
    Wdk::{
        Storage::FileSystem::{
            FileRenameInformation, NtQueryInformationToken, NtSetInformationFile,
            FILE_RENAME_INFORMATION,
        },
        System::SystemServices::RtlGetVersion,
    },
    Win32::{
        Foundation::{
            GetLastError, LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAGS,
            HANDLE_FLAG_INHERIT, HLOCAL, NTSTATUS, STATUS_PENDING, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Security::{
            AccessCheck,
            Authorization::{
                ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
                ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
                SetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT, SE_REGISTRY_KEY,
            },
            DeriveCapabilitySidsFromName, DuplicateTokenEx, EqualSid, FreeSid,
            GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetSecurityDescriptorSacl,
            GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
            Isolation::{
                CreateAppContainerProfile, DeleteAppContainerProfile, GetAppContainerFolderPath,
            },
            SecurityImpersonation, SetSecurityDescriptorControl, TokenAppContainerSid,
            TokenCapabilities, TokenImpersonation, TokenIntegrityLevel, TokenIsAppContainer,
            TokenUser, DACL_SECURITY_INFORMATION, GENERIC_MAPPING, GROUP_SECURITY_INFORMATION,
            LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PRIVILEGE_SET,
            PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES,
            SECURITY_DESCRIPTOR_CONTROL, SE_SACL_AUTO_INHERITED, SID_AND_ATTRIBUTES,
            TOKEN_APPCONTAINER_INFORMATION, TOKEN_DUPLICATE, TOKEN_GROUPS, TOKEN_MANDATORY_LABEL,
            TOKEN_QUERY, TOKEN_USER,
        },
        Storage::FileSystem::{
            CreateDirectoryW, CreateFileW, FileDispositionInfo, GetFileInformationByHandle,
            GetFinalPathNameByHandleW, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
            CREATE_NEW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_NAME_NORMALIZED,
            FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
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
                TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_SUSPENDED,
                CREATE_UNICODE_ENVIRONMENT, DETACHED_PROCESS, EXTENDED_STARTUPINFO_PRESENT,
                LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
                PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
                PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
            },
            IO::IO_STATUS_BLOCK,
        },
    },
};

const UNAVAILABLE: &str = "assistant_document_sandbox_unavailable";
const UNSAFE_FILE: &str = "assistant_document_sandbox_unsafe_file";
const CANCELLED: &str = "assistant_document_sandbox_cancelled";
const RUNTIME_CHANGED: &str = "assistant_document_sandbox_runtime_changed";
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
const FOREIGN_CAPABILITY: &str = "ofoxDocumentForeignControlV1";

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

fn check_rename_completion(status: NTSTATUS, completion: NTSTATUS) -> Result<(), String> {
    // NT_SUCCESS also accepts pending/informational statuses. Publication is
    // complete only when both the call and its IO_STATUS_BLOCK say SUCCESS.
    if status.0 != 0 || completion.0 != 0 {
        return Err(format!(
            "assistant_document_sandbox_rename_status_{:08x}_completion_{:08x}",
            status.0 as u32, completion.0 as u32
        ));
    }
    Ok(())
}
fn wide(value: impl AsRef<OsStr>) -> Result<Vec<u16>, String> {
    let value: Vec<_> = value.as_ref().encode_wide().collect();
    if value.contains(&0) {
        return Err(UNAVAILABLE.into());
    }
    Ok(value.into_iter().chain(Some(0)).collect())
}

/// Raw Win32 filesystem calls do not receive Rust std's long-path conversion.
/// Add the extended prefix lexically: canonicalize would follow a junction
/// before the no-reparse handle checks. Keep SDDL and command text on `wide`.
fn wide_path(path: &Path) -> Result<Vec<u16>, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::Prefix;
    // Ancestor checks parse the original path too. Verbatim Windows paths
    // treat `/` as component data; do not turn it into new uninspected levels.
    if path
        .as_os_str()
        .encode_wide()
        .any(|unit| unit == b'/' as u16)
        && matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _)))
    {
        return Err(UNSAFE_FILE.into());
    }
    // DOS/UNC components already recognize both separators. Normalize them
    // lexically before adding the verbatim prefix, without resolving links.
    let units: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .map(|unit| {
            if unit == b'/' as u16 {
                b'\\' as u16
            } else {
                unit
            }
        })
        .collect();
    if units.contains(&0)
        || units
            .split(|unit| *unit == b'\\' as u16)
            .any(|part| part == [b'.' as u16] || part == [b'.' as u16, b'.' as u16])
    {
        return Err(UNSAFE_FILE.into());
    }
    let path = PathBuf::from(OsString::from_wide(&units));
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(UNSAFE_FILE.into());
    }
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err(UNSAFE_FILE.into());
    };
    let normalized: PathBuf = path.components().collect();
    let name: Vec<u16> = normalized.as_os_str().encode_wide().collect();
    let mut extended: Vec<u16> = match prefix.kind() {
        Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _) => name,
        Prefix::Disk(_) => r"\\?\".encode_utf16().chain(name).collect(),
        Prefix::UNC(_, _) => r"\\?\UNC\"
            .encode_utf16()
            .chain(name.into_iter().skip(2))
            .collect(),
        _ => return Err(UNSAFE_FILE.into()),
    };
    if extended.len() >= 32767 {
        return Err(UNSAFE_FILE.into());
    }
    extended.push(0);
    Ok(extended)
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
fn token_buffer_observed(
    token: HANDLE,
    kind: windows::Win32::Security::TOKEN_INFORMATION_CLASS,
    observation: &Observer,
) -> Result<Vec<usize>, String> {
    token_buffer_sized_observed(token, kind, observation).map(|(data, _)| data)
}
fn token_buffer_sized_observed(
    token: HANDLE,
    kind: windows::Win32::Security::TOKEN_INFORMATION_CLASS,
    observation: &Observer,
) -> Result<(Vec<usize>, usize), String> {
    let mut length = 0;
    unsafe {
        let _ = GetTokenInformation(token, kind, None, 0, &mut length);
    }
    observation.token_query(kind.0, length);
    if length == 0 || length > 64 * 1024 {
        return Err(UNAVAILABLE.into());
    }
    let mut data = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    let requested = length;
    unsafe {
        win(GetTokenInformation(
            token,
            kind,
            Some(data.as_mut_ptr().cast()),
            length,
            &mut length,
        ))?;
    }
    if length == 0 || length > requested {
        return Err(UNAVAILABLE.into());
    }
    Ok((data, length as usize))
}
fn current_user() -> Result<String, String> {
    current_user_observed(&Observer::default())
}
fn current_user_observed(observation: &Observer) -> Result<String, String> {
    let mut token = HANDLE::default();
    unsafe {
        win(OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY,
            &mut token,
        ))?;
    }
    let token = handle(token);
    let data = token_buffer_observed(raw(&token), TokenUser, observation)?;
    sid_string(unsafe { (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid })
}

/// A capability SID derived by name. The private runtime capability authorizes
/// only the immutable public cache (not internet, COM, registry or user
/// libraries); the same type also derives Windows' read-only registryRead.
struct RuntimeCapability {
    sid: PSID,
    allocations: Vec<Local>,
}
impl RuntimeCapability {
    fn new() -> Result<Self, String> {
        Self::new_observed(&Observer::default())
    }
    fn new_observed(observation: &Observer) -> Result<Self, String> {
        Self::derive(RUNTIME_CAPABILITY, observation)
    }
    fn derive(name: &str, observation: &Observer) -> Result<Self, String> {
        let name = wide(name)?;
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
            observation.evidence(|evidence| {
                evidence.capability_count = Some(sid_count);
                evidence.capability_array_present = Some(!sids.is_null());
            });
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

/// Windows' read-only registry capability. libuv initializes Winsock on every
/// Node start; WSAStartup must open the HKLM root key, which Windows grants an
/// LPAC only through this capability (native trace 44231498872, A-B-A
/// 44076224990). On default ACLs it also reaches the user's HKCU (44133006077):
/// an accepted, documented residual risk (see docs/known-limitations.md).
const REGISTRY_READ_CAPABILITY: &str = "registryRead";

// Production: exactly the private runtime capability plus registryRead. Every
// launch re-derives both and measures the registry scope before resuming.
// Only the fixed test comparison can construct the runtime-only baseline.
struct ProcessCapabilities<'a> {
    runtime: &'a RuntimeCapability,
    registry_read: Option<RuntimeCapability>,
}
impl<'a> ProcessCapabilities<'a> {
    fn production(runtime: &'a RuntimeCapability) -> Result<Self, String> {
        let registry_read =
            RuntimeCapability::derive(REGISTRY_READ_CAPABILITY, &Observer::default())?;
        if unsafe { EqualSid(runtime.sid, registry_read.sid) }.is_ok() {
            return Err(UNAVAILABLE.into());
        }
        Ok(Self {
            runtime,
            registry_read: Some(registry_read),
        })
    }
    #[cfg(test)]
    fn runtime_only(runtime: &'a RuntimeCapability) -> Self {
        Self {
            runtime,
            registry_read: None,
        }
    }
    fn entries(&self) -> Vec<SID_AND_ATTRIBUTES> {
        let mut entries = vec![self.runtime.entry()];
        if let Some(registry_read) = &self.registry_read {
            entries.push(registry_read.entry());
        }
        entries
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
    let name = wide_path(path)?;
    unsafe { win(CreateDirectoryW(PCWSTR(name.as_ptr()), Some(&attrs))) }
}
fn private_sddl(user: &str) -> String {
    // Elevated/Server tokens may default newly created objects to an admin
    // group owner. Bind ownership to the actual caller SID explicitly.
    format!("O:{user}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{user})")
}
#[cfg(test)]
fn owned_directory(path: &Path, user: &str) -> Result<File, String> {
    owned_directory_observed(path, user, &Observer::default())
}
fn owned_directory_observed(
    path: &Path,
    user: &str,
    observation: &Observer,
) -> Result<File, String> {
    observation.enter(Phase::CacheOwnerOpen);
    let file = open_path(path, true)?;
    let mut owner = PSID::default();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    observation.enter(Phase::CacheOwnerQuery);
    unsafe {
        let queried = GetSecurityInfo(
            HANDLE(file.as_raw_handle()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            Some(&mut descriptor),
        );
        if queried.0 != 0 {
            observation.evidence(|evidence| evidence.native_status = Some(queried.0));
        }
        queried.ok().map_err(|_| UNAVAILABLE.to_owned())?;
        let allocation = Local(descriptor.0);
        observation.enter(Phase::CacheOwnerMatch);
        let matches = sid_string(owner)? == user;
        observation.evidence(|evidence| evidence.cache_owner_matches = Some(matches));
        drop(allocation);
        if !matches {
            return Err(UNAVAILABLE.into());
        }
    }
    Ok(file)
}

fn open_path(path: &Path, directory: bool) -> Result<File, String> {
    let name = wide_path(path)?;
    let handle = unsafe {
        win(CreateFileW(
            PCWSTR(name.as_ptr()),
            // Directory handles anchor relative rename operations. They need
            // traverse/read-attributes, not directory enumeration/read-data.
            if directory {
                FILE_GENERIC_EXECUTE.0
            } else {
                FILE_GENERIC_READ.0
            },
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

fn descriptor_comparison_key(
    descriptor: PSECURITY_DESCRIPTOR,
) -> Result<(u16, u32, String), String> {
    let (mut control, mut revision) = (0u16, 0u32);
    unsafe {
        win(GetSecurityDescriptorControl(
            descriptor,
            &mut control,
            &mut revision,
        ))?;
    }
    // Native Server 2022 reports only SE_SACL_AUTO_INHERITED in addition to
    // our exact template (0x9814 vs 0x9014). Normalize that bookkeeping bit
    // on a freshly allocated descriptor copy, never the filesystem object or
    // original snapshot. Every other control bit, ACE, mask, SID, protection
    // and mandatory-label field remains part of the exact comparison.
    let copy = security_descriptor(&descriptor_string(descriptor)?)?;
    unsafe {
        win(SetSecurityDescriptorControl(
            PSECURITY_DESCRIPTOR(copy.0),
            SE_SACL_AUTO_INHERITED,
            SECURITY_DESCRIPTOR_CONTROL(0),
        ))?;
    }
    Ok((
        control & !SE_SACL_AUTO_INHERITED.0,
        revision,
        descriptor_string(PSECURITY_DESCRIPTOR(copy.0))?,
    ))
}

/// Diagnostic shape only, never an ACL acceptance rule. Keep permission/control
/// differences visible without logging account SIDs or conditional ACE payloads.
fn descriptor_shape(text: &str) -> serde_json::Value {
    let aces: Vec<_> = text
        .split('(')
        .skip(1)
        .take(32)
        .filter_map(|part| {
            let body = part.split(')').next()?;
            let fields: Vec<_> = body.split(';').take(6).collect();
            if fields.len() != 6 {
                return None;
            }
            let tokens = |value: &str, allowed: &[&str]| {
                (value.len() <= 48
                    && value.len().is_multiple_of(2)
                    && value
                        .as_bytes()
                        .chunks(2)
                        .all(|pair| allowed.iter().any(|token| token.as_bytes() == pair)))
                .then(|| value.to_owned())
            };
            let kind = match fields[0] {
                "A" => "allow",
                "D" => "deny",
                "ML" => "mandatory_label",
                _ => "other",
            };
            if kind == "other" {
                return Some(serde_json::json!({"type":"other"}));
            }
            let sid = fields[5];
            let identity = if [
                "SY", "ME", "LW", "HI", "SI", "OW", "WD", "BA", "BU", "AC", "RA", "AN", "AU", "LS",
                "NS", "IU", "SU", "NU", "CO", "CG",
            ]
            .contains(&sid)
            {
                serde_json::json!({"wellKnownAlias":sid})
            } else {
                let kind = if sid.starts_with("S-1-5-21-") {
                    "nt_account"
                } else if sid.starts_with("S-1-15-3-") {
                    "capability"
                } else if sid.starts_with("S-1-15-2-") {
                    "app_container"
                } else if sid.starts_with("S-1-16-") {
                    "integrity_label"
                } else {
                    "other"
                };
                serde_json::json!({"kind":kind})
            };
            let flags = tokens(
                fields[1],
                &["OI", "CI", "NP", "IO", "ID", "SA", "FA", "TP", "CR"],
            );
            let rights = if fields[2].starts_with("0x")
                && fields[2].len() <= 10
                && fields[2][2..].bytes().all(|c| c.is_ascii_hexdigit())
            {
                Some(fields[2].to_owned())
            } else {
                tokens(
                    fields[2],
                    &[
                        "GA", "GR", "GW", "GX", "RC", "SD", "WD", "WO", "FA", "FR", "FW", "FX",
                        "KA", "KR", "KW", "KX", "NR", "NW", "NX",
                    ],
                )
            };
            Some(serde_json::json!({
                "type":kind, "flags":flags,
                "rights":rights, "identityClass":identity
            }))
        })
        .collect();
    serde_json::json!({"sddlBytes":text.len(),"aces":aces})
}

fn descriptor_diagnostic(descriptor: PSECURITY_DESCRIPTOR) -> Result<serde_json::Value, String> {
    let (mut control, mut revision) = (0u16, 0u32);
    unsafe {
        win(GetSecurityDescriptorControl(
            descriptor,
            &mut control,
            &mut revision,
        ))?;
    }
    Ok(serde_json::json!({
        "control":format!("{control:04x}"), "revision":revision,
        "shape":descriptor_shape(&descriptor_string(descriptor)?)
    }))
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
        .map_err(|error| {
            format!(
                "{UNAVAILABLE}:cache_acl_query_{:08x}",
                error.code().0 as u32
            )
        })?;
    }
    let allocation = Local(actual.0);
    let expected = security_descriptor(expected)?;
    let matches = descriptor_comparison_key(actual)?
        == descriptor_comparison_key(PSECURITY_DESCRIPTOR(expected.0))?;
    let diagnostic = if matches {
        None
    } else {
        Some(serde_json::json!({
            "actual":descriptor_diagnostic(actual)?,
            "expected":descriptor_diagnostic(PSECURITY_DESCRIPTOR(expected.0))?
        }))
    };
    drop(allocation);
    if let Some(diagnostic) = diagnostic {
        return Err(format!("{UNAVAILABLE}:cache_acl_mismatch {diagnostic}"));
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
        let path = wide_path(&temporary)?;
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
        // Satisfy the documented native structure+name buffer bound, keeping
        // zero terminator space without counting it in FileNameLength.
        let length = size_of::<FILE_RENAME_INFORMATION>() + (filename.len() + 1) * 2;
        let mut storage = vec![0usize; length.div_ceil(size_of::<usize>())];
        let rename = storage.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
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
            // CREATE_NEW above omitted FILE_FLAG_OVERLAPPED, so this file
            // handle has synchronous I/O and these buffers outlive the call.
            // Win32's class-3 wrapper rejects relative RootDirectory on the
            // tested Server 2022 host; native class 10 preserves our anchored
            // directory boundary without converting back to a pathname.
            let mut completion = IO_STATUS_BLOCK::default();
            completion.Anonymous.Status = STATUS_PENDING;
            let status = NtSetInformationFile(
                HANDLE(owned.file.as_raw_handle()),
                &mut completion,
                rename.cast(),
                length as u32,
                FileRenameInformation,
            );
            check_rename_completion(status, completion.Anonymous.Status)?;
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
    let name = wide_path(path)?;
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
    cache_runtime_observed(source, capability, user, &Observer::default())
}
fn cache_runtime_observed(
    source: &Path,
    capability: &RuntimeCapability,
    user: &str,
    observation: &Observer,
) -> Result<CachedRuntime, String> {
    observation.enter(Phase::CacheSource);
    let source = io(source.canonicalize())?;
    observation.enter(Phase::CacheSourceDigest);
    let digest = tree_digest_at(&source, MAX_TREE_BYTES)?;
    observation.enter(Phase::CacheDescriptor);
    let cap = sid_string(capability.sid)?;
    let expected_acl = format!(
        "{}(A;OICI;FRFX;;;{cap})S:(ML;OICI;NW;;;ME)",
        private_sddl(user)
    );
    observation.enter(Phase::CacheLock);
    let mut cached = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| UNAVAILABLE)?;
    if let Some(runtime) = cached.get(&source) {
        if runtime.digest == digest {
            observation.evidence(|evidence| evidence.cache_hit = Some(true));
            let pin = owned_directory_observed(&runtime.root, user, observation)?;
            observation.enter(Phase::CacheAcl);
            verify_cache_acl(&pin, &expected_acl)?;
            observation.enter(Phase::CacheDigest);
            if tree_digest_at(&runtime.root, MAX_TREE_BYTES)? != digest {
                return Err(UNSAFE_FILE.into());
            }
            return Ok(runtime.clone());
        }
    }
    observation.evidence(|evidence| evidence.cache_hit = Some(false));
    observation.enter(Phase::CacheParent);
    let private_parent = std::env::temp_dir().join(format!(
        "ofox-document-runtime-{}",
        &format!("{:x}", Sha256::digest(user.as_bytes()))[..16]
    ));
    if !private_parent.exists() {
        create_directory(&private_parent, &private_sddl(user))?;
    }
    let _parent = owned_directory_observed(&private_parent, user, observation)?;
    let root = private_parent.join(&digest);
    if !root.exists() {
        observation.enter(Phase::CacheSourceRead);
        let contents = files(&source, MAX_TREE_BYTES)?;
        if tree_digest(&contents) != digest {
            return Err(RUNTIME_CHANGED.into());
        }
        let staging = private_parent.join(format!("staging-{}", uuid::Uuid::new_v4().simple()));
        observation.enter(Phase::CacheStagingCreate);
        create_directory(&staging, &expected_acl)?;
        observation.enter(Phase::CacheStagingWrite);
        let prepared = write_files(&staging, &contents).and_then(|()| {
            observation.enter(Phase::CacheStagingDigest);
            if tree_digest_at(&staging, MAX_TREE_BYTES)? != digest {
                return Err(UNSAFE_FILE.into());
            }
            Ok(())
        });
        if let Err(error) = prepared {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        observation.enter(Phase::CachePublish);
        if fs::rename(&staging, &root).is_err() {
            let _ = fs::remove_dir_all(&staging);
            // A concurrent complete publisher is acceptable only after all
            // the same ownership, ACL and content checks below.
            if !root.exists() {
                return Err(UNAVAILABLE.into());
            }
        }
    }
    let root_handle = owned_directory_observed(&root, user, observation)?;
    observation.enter(Phase::CacheAcl);
    verify_cache_acl(&root_handle, &expected_acl)?;
    observation.enter(Phase::CacheDigest);
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
    #[cfg(test)]
    fn make_low_profile(&mut self) -> Result<(), String> {
        self.make_low_profile_observed(&Observer::default())
    }
    fn make_low_profile_observed(&mut self, observation: &Observer) -> Result<(), String> {
        observation.enter(Phase::ProfileFolder);
        let sid = wide(sid_string(self.sid.0)?)?;
        let folder = unsafe { win(GetAppContainerFolderPath(PCWSTR(sid.as_ptr())))? };
        observation.enter(Phase::ProfileFolderDecode);
        let result = unsafe { folder.to_string().map_err(|_| UNAVAILABLE.to_owned()) };
        unsafe {
            CoTaskMemFree(Some(folder.0.cast()));
        }
        let path = PathBuf::from(result?);
        observation.enter(Phase::ProfileFolderValidate);
        let folder_is_ac = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("AC"));
        observation.evidence(|evidence| {
            evidence.profile_folder_is_ac = Some(folder_is_ac);
            evidence.profile_folder_has_parent = Some(path.parent().is_some());
        });
        if !folder_is_ac {
            return Err(UNAVAILABLE.into());
        }
        let path = path.parent().ok_or(UNAVAILABLE)?.to_path_buf();
        // Profile is freshly created, before untrusted code can plant links.
        observation.enter(Phase::ProfilePin);
        self.pins = pin_ancestors(&path)?;
        observation.evidence(|evidence| evidence.profile_pin_complete = Some(true));
        self.root_pinned = true;
        self.directory = Some(path.clone());
        observation.enter(Phase::ProfileLabelOpen);
        let name = wide_path(&path)?;
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
        observation.enter(Phase::ProfileLabelDescriptor);
        let descriptor = security_descriptor("S:(ML;OICI;NW;;;LW)")?;
        let (mut present, mut defaulted, mut acl) = (BOOL(0), BOOL(0), std::ptr::null_mut());
        unsafe {
            win(GetSecurityDescriptorSacl(
                PSECURITY_DESCRIPTOR(descriptor.0),
                &mut present,
                &mut acl,
                &mut defaulted,
            ))?;
            observation.enter(Phase::ProfileLabelSet);
            let labelled = SetSecurityInfo(
                raw(&file),
                SE_FILE_OBJECT,
                LABEL_SECURITY_INFORMATION,
                None,
                None,
                None,
                Some(acl),
            );
            if labelled.0 != 0 {
                observation.evidence(|evidence| evidence.native_status = Some(labelled.0));
            }
            labelled.ok().map_err(|_| UNAVAILABLE.to_owned())?;
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
    os_version_observed(&Observer::default())
}
fn os_version_observed(observation: &Observer) -> Result<OSVERSIONINFOW, String> {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    unsafe {
        let status = RtlGetVersion(&mut version);
        if status.0 != 0 {
            observation.evidence(|evidence| evidence.native_status = Some(status.0 as u32));
        }
        status.ok().map_err(|_| UNAVAILABLE.to_owned())?;
    }
    observation.evidence(|evidence| {
        evidence.os_major = Some(version.dwMajorVersion);
        evidence.os_build = Some(version.dwBuildNumber);
    });
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AccessVerdict {
    allowed: bool,
    granted: u32,
}

fn valid_specific_privilege_output(bytes: u32, count: u32) -> bool {
    // AccessCheck may use the fixed header even when no privileges are needed.
    // The invented specific bit must not depend on a standard-right privilege.
    bytes >= std::mem::offset_of!(PRIVILEGE_SET, Privilege) as u32
        && bytes <= size_of::<PRIVILEGE_SET>() as u32
        && count == 0
}

fn access_impersonation_token(
    token: HANDLE,
    observation: &Observer,
) -> Result<OwnedHandle, String> {
    observation.enter(Phase::TokenLpacDuplicate);
    observation.evidence(|evidence| {
        evidence.lpac_access_proof = Some(Default::default());
    });
    let mut duplicate = HANDLE::default();
    unsafe {
        win(DuplicateTokenEx(
            token,
            TOKEN_QUERY,
            None,
            SecurityImpersonation,
            TokenImpersonation,
            &mut duplicate,
        ))?;
    }
    let duplicate = handle(duplicate);
    observation.evidence(|evidence| {
        evidence
            .lpac_access_proof
            .get_or_insert_default()
            .duplicated = true;
    });
    Ok(duplicate)
}

fn check_specific_access(
    token: HANDLE,
    sddl: &str,
    case: AccessCase,
    observation: &Observer,
) -> Result<AccessVerdict, String> {
    observation.enter(match case {
        AccessCase::WorldOnly => Phase::TokenLpacWorld,
        AccessCase::AllApplicationPackages => Phase::TokenLpacAllPackages,
        AccessCase::RuntimeCapability => Phase::TokenLpacCapability,
        AccessCase::ForeignCapability => Phase::TokenLpacForeign,
    });
    let descriptor = security_descriptor(sddl)?;
    // One invented object-specific bit: no generic/MAXIMUM_ALLOWED or standard
    // owner rights can turn a denied package-side check into a positive result.
    let mapping = GENERIC_MAPPING {
        GenericRead: 1,
        GenericWrite: 1,
        GenericExecute: 1,
        GenericAll: 1,
    };
    let mut privileges = PRIVILEGE_SET::default();
    let mut privilege_bytes = size_of::<PRIVILEGE_SET>() as u32;
    let mut granted = 0u32;
    let mut allowed = BOOL(0);
    let result = unsafe {
        AccessCheck(
            PSECURITY_DESCRIPTOR(descriptor.0),
            token,
            1,
            &mapping,
            Some(&mut privileges),
            &mut privilege_bytes,
            &mut granted,
            &mut allowed,
        )
    };
    let success = result.is_ok();
    let status = result
        .as_ref()
        .err()
        .map_or(0, |error| error.code().0 as u32);
    observation.evidence(|evidence| {
        let proof = evidence.lpac_access_proof.get_or_insert_default();
        if proof.checks.len() < 4 {
            proof.checks.push(AccessObservation {
                case,
                api_success: success,
                status,
                access_status: success.then(|| allowed.as_bool()),
                granted_access: success.then_some(granted),
                returned_privilege_bytes: success.then_some(privilege_bytes),
                privilege_count: success.then_some(privileges.PrivilegeCount),
            });
        }
    });
    // API failure (including invalid SD or privilege-buffer size) is never a
    // successful denial. Do not interpret its initialized output parameters.
    win(result)?;
    if !valid_specific_privilege_output(privilege_bytes, privileges.PrivilegeCount) {
        return Err(UNAVAILABLE.into());
    }
    Ok(AccessVerdict {
        allowed: allowed.as_bool(),
        granted,
    })
}

fn lpac_access_matches(verdicts: &[AccessVerdict; 4]) -> bool {
    *verdicts
        == [
            AccessVerdict {
                allowed: false,
                granted: 0,
            },
            AccessVerdict {
                allowed: false,
                granted: 0,
            },
            AccessVerdict {
                allowed: true,
                granted: 1,
            },
            AccessVerdict {
                allowed: false,
                granted: 0,
            },
        ]
}

/// What the registry gate demands of an actual suspended token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegistryScope {
    /// Production {runtime, registryRead}: the HKLM root and the Winsock
    /// catalog must be readable. The user's Environment and Software keys are
    /// measured on every launch (any failure refuses) but never decide it;
    /// readable is the accepted residual risk.
    RegistryRead,
    /// Test-only runtime-only tokens: the HKLM root and the user keys must stay
    /// denied, and the catalog is readable exactly under the CI-only grant.
    #[cfg(test)]
    RuntimeOnly { winsock_readable: bool },
}

/// The gate's decision from the measured reads: [HKLM root, Winsock catalog],
/// then the user's [Environment, Software].
fn registry_scope_admits(scope: RegistryScope, machine: [bool; 2], user_keys: [bool; 2]) -> bool {
    let [root, catalog] = machine;
    match scope {
        RegistryScope::RegistryRead => {
            // Accepted residual risk: the user-key reads never decide a
            // production launch.
            let _ = user_keys;
            root && catalog
        }
        #[cfg(test)]
        RegistryScope::RuntimeOnly { winsock_readable } => {
            !root && catalog == winsock_readable && !user_keys.iter().any(|read| *read)
        }
    }
}

fn verify_lpac_access(
    token: HANDLE,
    capability: PSID,
    scope: RegistryScope,
    observation: &Observer,
) -> Result<(), String> {
    let duplicate = access_impersonation_token(token, observation)?;
    observation.enter(Phase::TokenLpacDescriptors);
    let own = sid_string(capability)?;
    let foreign_capability = RuntimeCapability::derive(FOREIGN_CAPABILITY, &Observer::default())?;
    let foreign = sid_string(foreign_capability.sid)?;
    if own == foreign {
        return Err(UNAVAILABLE.into());
    }
    // AppContainer authorization intersects normal-user and package/capability
    // grants. WD supplies only the user side; every SD has an owner and group.
    let base = "O:SYG:SYD:P(A;;0x1;;;WD)";
    let descriptors = [
        (AccessCase::WorldOnly, base.to_owned()),
        (
            AccessCase::AllApplicationPackages,
            format!("{base}(A;;0x1;;;S-1-15-2-1)"),
        ),
        (
            AccessCase::RuntimeCapability,
            format!("{base}(A;;0x1;;;{own})"),
        ),
        (
            AccessCase::ForeignCapability,
            format!("{base}(A;;0x1;;;{foreign})"),
        ),
    ];
    let mut verdicts = [AccessVerdict {
        allowed: false,
        granted: 0,
    }; 4];
    for ((case, descriptor), verdict) in descriptors.iter().zip(&mut verdicts) {
        *verdict = check_specific_access(raw(&duplicate), descriptor, *case, observation)?;
    }
    let matched = lpac_access_matches(&verdicts);
    observation.evidence(|evidence| {
        evidence.lpac_access_proof.get_or_insert_default().matched = Some(matched);
    });
    if !matched {
        return Err(UNAVAILABLE.into());
    }
    // registryRead is a read grant on keys Windows marks for it. Every read is
    // measured on this actual token (and kept in probe evidence); any open,
    // descriptor or AccessCheck failure refuses the launch (fail closed).
    observation.enter(Phase::TokenRegistryIsolation);
    let machine = [
        registry_read_granted(raw(&duplicate), RegistryKey::MachineRoot, observation)?,
        registry_read_granted(raw(&duplicate), RegistryKey::WinsockCatalog, observation)?,
    ];
    let user_keys = [
        registry_read_granted(raw(&duplicate), RegistryKey::UserEnvironment, observation)?,
        registry_read_granted(raw(&duplicate), RegistryKey::UserSoftware, observation)?,
    ];
    if !registry_scope_admits(scope, machine, user_keys) {
        return Err(UNAVAILABLE.into());
    }
    Ok(())
}

/// KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS: the rights a read would need.
const REGISTRY_READ_BITS: u32 = 0x0001 | 0x0008;
/// READ_CONTROL, enough for the host to read the key's security descriptor.
const REGISTRY_READ_CONTROL: u32 = 0x0002_0000;

/// AccessCheck of the actual registry key DACL against `token` (an
/// impersonation duplicate of the suspended child). MAXIMUM_ALLOWED reports
/// every right the DACL and AppContainer intersection grant; only the read
/// bits decide. Any open, descriptor or AccessCheck failure is an error.
fn registry_read_granted(
    token: HANDLE,
    key: RegistryKey,
    observation: &Observer,
) -> Result<bool, String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let (root, path) = match key {
        RegistryKey::UserEnvironment => (HKEY_CURRENT_USER, "Environment"),
        RegistryKey::UserSoftware => (HKEY_CURRENT_USER, "Software"),
        RegistryKey::WinsockCatalog => (
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Services\WinSock2\Parameters",
        ),
        RegistryKey::MachineRoot => (HKEY_LOCAL_MACHINE, ""),
    };
    let mut record = RegistryAccess {
        key,
        opened: false,
        descriptor_read: false,
        access_checked: false,
        read_granted: None,
    };
    let result = registry_read_check(root, path, token, &mut record);
    observation.evidence(|evidence| {
        if evidence.registry_isolation.len() < 4 {
            evidence.registry_isolation.push(record);
        }
    });
    result
}

fn registry_read_check(
    root: winreg::HKEY,
    path: &str,
    token: HANDLE,
    record: &mut RegistryAccess,
) -> Result<bool, String> {
    let opened = winreg::RegKey::predef(root)
        .open_subkey_with_flags(path, REGISTRY_READ_CONTROL)
        .map_err(|_| UNAVAILABLE.to_owned())?;
    record.opened = true;
    Ok(registry_key_read_bits(HANDLE(opened.raw_handle() as _), token, record)? != 0)
}

/// The granted read bits (KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS) for one
/// already-open key handle (READ_CONTROL); 0 when nothing is granted.
fn registry_key_read_bits(
    key: HANDLE,
    token: HANDLE,
    record: &mut RegistryAccess,
) -> Result<u32, String> {
    Ok(registry_key_access(key, token, record)? & REGISTRY_READ_BITS)
}

/// Every right `token` is granted on one already-open key handle
/// (READ_CONTROL); 0 when nothing is granted.
fn registry_key_access(
    key: HANDLE,
    token: HANDLE,
    record: &mut RegistryAccess,
) -> Result<u32, String> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetSecurityInfo(
            key,
            SE_REGISTRY_KEY,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    };
    if status.0 != 0 || descriptor.0.is_null() {
        return Err(UNAVAILABLE.into());
    }
    let descriptor = Local(descriptor.0);
    record.descriptor_read = true;
    // KEY_READ, KEY_WRITE, KEY_EXECUTE (= KEY_READ) and KEY_ALL_ACCESS.
    let mapping = GENERIC_MAPPING {
        GenericRead: 0x0002_0019,
        GenericWrite: 0x0002_0006,
        GenericExecute: 0x0002_0019,
        GenericAll: 0x000f_003f,
    };
    let mut privileges = PRIVILEGE_SET::default();
    let mut privilege_bytes = size_of::<PRIVILEGE_SET>() as u32;
    let (mut granted, mut allowed) = (0u32, BOOL(0));
    unsafe {
        AccessCheck(
            PSECURITY_DESCRIPTOR(descriptor.0),
            token,
            0x0200_0000, // MAXIMUM_ALLOWED
            &mapping,
            Some(&mut privileges),
            &mut privilege_bytes,
            &mut granted,
            &mut allowed,
        )
    }
    .map_err(|_| UNAVAILABLE.to_owned())?;
    record.access_checked = true;
    let granted = if allowed.as_bool() { granted } else { 0 };
    record.read_granted = Some(granted & REGISTRY_READ_BITS != 0);
    Ok(granted)
}

fn verify_token(
    process: HANDLE,
    package: PSID,
    capability: PSID,
    observation: &Observer,
) -> Result<(), String> {
    // Derived independently of the launch so a wrong set cannot vouch for itself.
    let registry_read = RuntimeCapability::derive(REGISTRY_READ_CAPABILITY, &Observer::default())?;
    verify_token_entries(
        process,
        package,
        capability,
        &[
            SID_AND_ATTRIBUTES {
                Sid: capability,
                Attributes: SE_GROUP_ENABLED as u32,
            },
            registry_read.entry(),
        ],
        RegistryScope::RegistryRead,
        observation,
    )
}

/// `scope`: what the registry gate demands; it must match the expected set
/// (production {runtime, registryRead}; test-only runtime-only singleton).
fn verify_token_entries(
    process: HANDLE,
    package: PSID,
    capability: PSID,
    expected: &[SID_AND_ATTRIBUTES],
    scope: RegistryScope,
    observation: &Observer,
) -> Result<(), String> {
    let consistent = match scope {
        RegistryScope::RegistryRead => expected.len() == 2,
        #[cfg(test)]
        RegistryScope::RuntimeOnly { .. } => expected.len() == 1,
    };
    if !consistent {
        return Err(UNAVAILABLE.into());
    }
    if expected[0].Attributes != SE_GROUP_ENABLED as u32
        || unsafe { EqualSid(expected[0].Sid, capability) }.is_err()
    {
        return Err(UNAVAILABLE.into());
    }
    if expected.len() == 2 {
        let registry_read =
            RuntimeCapability::derive(REGISTRY_READ_CAPABILITY, &Observer::default())?;
        if expected[1].Attributes != SE_GROUP_ENABLED as u32
            || unsafe { EqualSid(expected[1].Sid, registry_read.sid) }.is_err()
            || unsafe { EqualSid(expected[0].Sid, expected[1].Sid) }.is_ok()
        {
            return Err(UNAVAILABLE.into());
        }
    }
    observation.enter(Phase::TokenOpen);
    let mut token = HANDLE::default();
    unsafe {
        win(OpenProcessToken(
            process,
            TOKEN_QUERY | TOKEN_DUPLICATE,
            &mut token,
        ))?;
    }
    let token = handle(token);
    observation.enter(Phase::TokenContainer);
    observation.compare_token_queries(|api, kind, buffer, bytes, length| {
        let buffer = buffer.map(|buffer| buffer.as_mut_ptr().cast());
        let kind = windows::Win32::Security::TOKEN_INFORMATION_CLASS(kind);
        match api {
            TokenApi::GetTokenInformation => {
                let result =
                    unsafe { GetTokenInformation(raw(&token), kind, buffer, bytes, length) };
                let status = result
                    .as_ref()
                    .err()
                    .map_or(0, |error| error.code().0 as u32);
                (result.is_ok(), status)
            }
            TokenApi::NtQueryInformationToken => {
                let status =
                    unsafe { NtQueryInformationToken(raw(&token), kind, buffer, bytes, length) };
                (status.0 == 0, status.0 as u32)
            }
        }
    });
    let container = token_buffer_observed(raw(&token), TokenIsAppContainer, observation)?[0] as u32;
    observation.evidence(|evidence| evidence.app_container = Some(container));
    if container != 1 {
        return Err(UNAVAILABLE.into());
    }
    observation.enter(Phase::TokenPackage);
    let package_data = token_buffer_observed(raw(&token), TokenAppContainerSid, observation)?;
    let actual_package = unsafe {
        &*package_data
            .as_ptr()
            .cast::<TOKEN_APPCONTAINER_INFORMATION>()
    };
    unsafe {
        let matched = EqualSid(actual_package.TokenAppContainer, package);
        observation.evidence(|evidence| evidence.package_matches = Some(matched.is_ok()));
        win(matched)?;
    }
    observation.enter(Phase::TokenCapabilities);
    let (capabilities, capability_bytes) =
        token_buffer_sized_observed(raw(&token), TokenCapabilities, observation)?;
    if capability_bytes < size_of::<TOKEN_GROUPS>() {
        return Err(UNAVAILABLE.into());
    }
    let groups = unsafe { &*capabilities.as_ptr().cast::<TOKEN_GROUPS>() };
    observation.evidence(|evidence| evidence.capability_groups = Some(groups.GroupCount));
    if groups.GroupCount as usize != expected.len() {
        return Err(UNAVAILABLE.into());
    }
    let offset = std::mem::offset_of!(TOKEN_GROUPS, Groups);
    let required = offset
        .checked_add(
            expected
                .len()
                .checked_mul(size_of::<SID_AND_ATTRIBUTES>())
                .ok_or(UNAVAILABLE)?,
        )
        .ok_or(UNAVAILABLE)?;
    if required > capability_bytes {
        return Err(UNAVAILABLE.into());
    }
    unsafe {
        let actual = std::slice::from_raw_parts(groups.Groups.as_ptr(), expected.len());
        let masks = actual
            .iter()
            .map(|entry| {
                expected
                    .iter()
                    .enumerate()
                    .fold(0u8, |mask, (index, want)| {
                        mask | if EqualSid(entry.Sid, want.Sid).is_ok() {
                            1 << index
                        } else {
                            0
                        }
                    })
            })
            .collect::<Vec<_>>();
        // Every actual capability must be present and enabled, not just named.
        let enabled = actual
            .iter()
            .all(|entry| entry.Attributes == SE_GROUP_ENABLED as u32);
        let exact = capability_set::exact(&masks) && enabled;
        observation.evidence(|evidence| evidence.capability_matches = Some(exact));
        if !exact {
            return Err(UNAVAILABLE.into());
        }
    }
    observation.enter(Phase::TokenIntegrity);
    let data = token_buffer_observed(raw(&token), TokenIntegrityLevel, observation)?;
    let label = unsafe { &*data.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
    let subauthorities = unsafe { *GetSidSubAuthorityCount(label.Label.Sid) };
    observation.evidence(|evidence| {
        evidence.integrity_subauthorities = Some(subauthorities);
    });
    let integrity = unsafe {
        *GetSidSubAuthority(
            label.Label.Sid,
            (subauthorities as u32).checked_sub(1).ok_or(UNAVAILABLE)?,
        )
    };
    observation.evidence(|evidence| evidence.integrity_rid = Some(integrity));
    if integrity > 4096 {
        return Err(UNAVAILABLE.into());
    }
    // Both policies have a DWORD flags layout. Verify actual kernel state,
    // not just successful construction of the startup attribute list.
    observation.enter(Phase::TokenMitigations);
    let (mut child, mut image) = (0u32, 0u32);
    unsafe {
        win(GetProcessMitigationPolicy(
            process,
            ProcessChildProcessPolicy,
            (&mut child as *mut u32).cast(),
            size_of::<u32>(),
        ))?;
        observation.evidence(|evidence| evidence.child_policy = Some(child));
        win(GetProcessMitigationPolicy(
            process,
            ProcessImageLoadPolicy,
            (&mut image as *mut u32).cast(),
            size_of::<u32>(),
        ))?;
        observation.evidence(|evidence| evidence.image_policy = Some(image));
    }
    if child & 1 == 0 || image & 3 != 3 {
        return Err(UNAVAILABLE.into());
    }
    // Class 46 is rejected with STATUS_INVALID_INFO_CLASS on native 20348.
    // Prove its documented LPAC authorization semantics on this actual token,
    // while it is still suspended, with both positive and negative controls.
    verify_lpac_access(raw(&token), capability, scope, observation)
}

struct Launch {
    process: Process,
    stdout: std::sync::mpsc::Receiver<Vec<u8>>,
    stderr: std::sync::mpsc::Receiver<Vec<u8>>,
    #[cfg(test)]
    debugger: Option<cdb::Guard>,
    /// Held only for its Drop. Last field: the catalog ACE is restored after
    /// the process and debugger are gone.
    #[cfg(test)]
    _grant: Option<trace_launch::HeldGrant>,
}

#[derive(Clone, Copy)]
enum ContainerPolicy {
    Lpac,
    // A native negative control can create an ordinary AC, but no production
    // caller can select it and no control ever resumes its primary thread.
    #[cfg(test)]
    OrdinaryAppContainer,
}

#[cfg(test)]
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

/// Console allocation for the sandboxed child. Production launches detached
/// with explicit pipe stdio: allocating a new hidden console (CREATE_NO_WINDOW)
/// fails KernelBase initialization in this LPAC sandbox with 0xc0000142
/// (native A-B-A 44068121826). Hidden remains only for that fixed comparison.
#[derive(Clone, Copy)]
enum ConsoleMode {
    #[cfg(test)]
    Hidden,
    Detached,
}
impl ConsoleMode {
    fn creation_flags(self) -> PROCESS_CREATION_FLAGS {
        let console = match self {
            #[cfg(test)]
            Self::Hidden => CREATE_NO_WINDOW,
            Self::Detached => DETACHED_PROCESS,
        };
        console | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT
    }
}

struct SuspendedLaunch {
    #[cfg(test)]
    pid: u32,
    #[cfg(test)]
    tid: u32,
    thread: OwnedHandle,
    process: Process,
    stdin_read: OwnedHandle,
    stdout_read: OwnedHandle,
    stdout_write: OwnedHandle,
    stderr_read: OwnedHandle,
    stderr_write: OwnedHandle,
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
    observation: &Observer,
    _cancelled: &AtomicBool,
) -> Result<Launch, String> {
    // Declared first so it outlives (is dropped after) the suspended process.
    #[cfg(test)]
    let mut grant = None;
    #[cfg(test)]
    let suspended = if observation.loader_requested() {
        let (suspended, held) =
            trace_launch::create(exe, args, cwd, home, profile, cap, observation)?;
        grant = Some(held);
        suspended
    } else {
        create_verified(exe, args, cwd, home, profile, cap, observation)?
    };
    #[cfg(not(test))]
    let suspended = create_verified(exe, args, cwd, home, profile, cap, observation)?;
    #[cfg(test)]
    let mut debugger = cdb::Guard::begin(
        raw(&suspended.process.process),
        raw(&suspended.thread),
        suspended.pid,
        suspended.tid,
        observation,
        _cancelled,
    )?;
    let SuspendedLaunch {
        thread,
        process,
        stdin_read,
        stdout_read,
        stdout_write,
        stderr_read,
        stderr_write,
        #[cfg(test)]
            pid: _,
        #[cfg(test)]
            tid: _,
    } = suspended;
    #[cfg(test)]
    if debugger.is_some() && _cancelled.load(Ordering::Acquire) {
        observation.failed();
        return Err(CANCELLED.into());
    }
    observation.enter(Phase::Resume);
    let resumed = unsafe { ResumeThread(raw(&thread)) };
    let resume_error = (resumed == u32::MAX).then(|| unsafe { GetLastError().0 });
    observation.evidence(|evidence| {
        evidence.resume_result = Some(resumed);
        if let Some(code) = resume_error {
            evidence.native_status = Some(code);
        }
    });
    if resumed == u32::MAX {
        return Err(UNAVAILABLE.into());
    }
    #[cfg(test)]
    if let Some(debugger) = &mut debugger {
        debugger.resumed(resumed, _cancelled)?;
    }
    drop(thread);
    drop(stdin_read);
    drop(stdout_write);
    drop(stderr_write);
    Ok(Launch {
        process,
        stdout: drain(stdout_read, limit),
        stderr: drain(stderr_read, limit),
        #[cfg(test)]
        debugger,
        #[cfg(test)]
        _grant: grant,
    })
}

/// The production launch: {runtime, registryRead}, console-free, and the token
/// (including the registry gate) verified while the process is still suspended.
fn create_verified(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    home: &Path,
    profile: &Profile,
    cap: &RuntimeCapability,
    observation: &Observer,
) -> Result<SuspendedLaunch, String> {
    let suspended = create_suspended(
        exe,
        args,
        cwd,
        home,
        profile,
        cap,
        ContainerPolicy::Lpac,
        observation,
    )?;
    verify_token(
        raw(&suspended.process.process),
        profile.sid.0,
        cap.sid,
        observation,
    )?;
    Ok(suspended)
}

#[allow(clippy::too_many_arguments)]
fn create_suspended(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    home: &Path,
    profile: &Profile,
    cap: &RuntimeCapability,
    policy: ContainerPolicy,
    observation: &Observer,
) -> Result<SuspendedLaunch, String> {
    create_suspended_capabilities(
        exe,
        args,
        cwd,
        home,
        profile,
        &ProcessCapabilities::production(cap)?,
        policy,
        ConsoleMode::Detached,
        observation,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_suspended_capabilities(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    home: &Path,
    profile: &Profile,
    caps: &ProcessCapabilities<'_>,
    policy: ContainerPolicy,
    console: ConsoleMode,
    observation: &Observer,
) -> Result<SuspendedLaunch, String> {
    observation.enter(Phase::Command);
    let app = wide(exe)?;
    let mut cmd = quote(exe.as_os_str())?;
    for arg in args {
        cmd.push(' ');
        cmd.push_str(&quote(OsStr::new(arg))?);
    }
    let mut command = wide(cmd)?;
    let cwd = wide(cwd)?;
    observation.enter(Phase::Environment);
    let env = environment(home)?;
    observation.enter(Phase::Pipes);
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
    let mut capability = caps.entries();
    let security = SECURITY_CAPABILITIES {
        AppContainerSid: profile.sid.0,
        Capabilities: capability.as_mut_ptr(),
        CapabilityCount: capability.len().try_into().map_err(|_| UNAVAILABLE)?,
        Reserved: 0,
    };
    let mitigations = NO_REMOTE_IMAGES | NO_LOW_LABEL_IMAGES | PREFER_SYSTEM32;
    observation.enter(Phase::Attributes);
    let mut attributes = Attributes::new(match policy {
        ContainerPolicy::Lpac => 5,
        #[cfg(test)]
        ContainerPolicy::OrdinaryAppContainer => 4,
    })?;
    attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &security)?;
    match policy {
        ContainerPolicy::Lpac => attributes.set(
            PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
            &LPAC_OPT_OUT,
        )?,
        #[cfg(test)]
        ContainerPolicy::OrdinaryAppContainer => {}
    }
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
    observation.enter(Phase::JobCreate);
    let job = unsafe { handle(win(CreateJobObjectW(None, PCWSTR::null()))?) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    observation.enter(Phase::JobLimits);
    unsafe {
        win(SetInformationJobObject(
            raw(&job),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ))?;
    }
    let mut info = PROCESS_INFORMATION::default();
    observation.enter(Phase::ProcessCreate);
    unsafe {
        win(CreateProcessW(
            PCWSTR(app.as_ptr()),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            true,
            console.creation_flags(),
            Some(env.as_ptr().cast()),
            PCWSTR(cwd.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        ))?;
    }
    observation.evidence(|evidence| {
        evidence.child_pid = Some(info.dwProcessId);
        evidence.primary_tid = Some(info.dwThreadId);
        #[cfg(test)]
        if std::env::var("OFOX_DOCUMENT_CHILD_BLOCK_EVENT").as_deref() == Ok("1")
            && std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("GITHUB_WORKFLOW").as_deref()
                == Ok("Document Engine Development Checks")
        {
            use windows::Win32::Foundation::FILETIME;
            use windows::Win32::System::Threading::GetProcessTimes;
            let mut created = FILETIME::default();
            let mut exit = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            if unsafe {
                GetProcessTimes(
                    info.hProcess,
                    &mut created,
                    &mut exit,
                    &mut kernel,
                    &mut user,
                )
            }
            .is_ok()
            {
                let ticks =
                    (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
                evidence.child_creation_filetime = Some(ticks.to_string());
            }
        }
    });
    let process = Process {
        process: handle(info.hProcess),
        job,
    };
    let thread = handle(info.hThread);
    observation.enter(Phase::JobAssign);
    unsafe {
        win(AssignProcessToJobObject(
            raw(&process.job),
            raw(&process.process),
        ))?;
    }
    Ok(SuspendedLaunch {
        #[cfg(test)]
        pid: info.dwProcessId,
        #[cfg(test)]
        tid: info.dwThreadId,
        thread,
        process,
        stdin_read,
        stdout_read,
        stdout_write,
        stderr_read,
        stderr_write,
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
    run_candidate_observed(
        exe,
        args,
        cwd,
        home,
        read_root,
        timeout,
        limit,
        trusted_probe,
        Observer::default(),
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn run_candidate_observed(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    home: &Path,
    read_root: &Path,
    timeout: Duration,
    limit: usize,
    trusted_probe: bool,
    observation: Observer,
) -> Result<EngineOutput, String> {
    let observation = if trusted_probe {
        observation
    } else {
        Observer::default()
    };
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
    let worker_observation = observation.clone();
    let answer = tokio::task::spawn_blocking(move || {
        // Keep this owned guard in the worker: cancelling the awaiting future
        // must not admit another Pyodide before its Job/disposal is complete.
        let _python_turn = python_turn;
        let result = run_blocking(
            &exe,
            &args,
            &cwd,
            &home,
            &root,
            timeout,
            limit,
            &cancelled,
            trusted_probe,
            &worker_observation,
        );
        if result.is_err() {
            worker_observation.failed();
        }
        result
    })
    .await
    .map_err(|error| {
        observation.evidence(|evidence| {
            evidence.worker_join_panicked = Some(error.is_panic());
            evidence.worker_join_cancelled = Some(error.is_cancelled());
        });
        observation.failed();
        UNAVAILABLE.to_owned()
    })?;
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
    observation: &Observer,
) -> Result<EngineOutput, String> {
    observation.enter(Phase::OsVersion);
    os_version_observed(observation)?;
    observation.enter(Phase::CurrentUser);
    let user = current_user_observed(observation)?;
    observation.enter(Phase::RuntimeCapability);
    let capability = RuntimeCapability::new_observed(observation)?;
    let runtime = cache_runtime_observed(read_root, &capability, &user, observation)?;
    if !trusted_probe {
        observation.enter(Phase::Admission);
        let admitted = ADMITTED
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| UNAVAILABLE)?;
        if admitted.get(&runtime.source) != Some(&runtime.digest) {
            return Err("assistant_document_sandbox_unproven_runtime".into());
        }
    }
    observation.enter(Phase::InputPaths);
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
    observation.enter(Phase::InputSnapshot);
    let inputs = files(&cwd, MAX_TASK_BYTES)?;
    observation.enter(Phase::ProfileCreate);
    let mut profile = Profile::new(&capability)?;
    if let Err(error) = profile.make_low_profile_observed(observation) {
        observation.failed();
        observation.enter(Phase::ProfileCleanup);
        let cleanup = dispose_profile(&mut profile);
        observation.evidence(|evidence| evidence.profile_cleanup = Some(cleanup.is_ok()));
        cleanup?;
        return Err(error);
    }
    observation.enter(Phase::TaskCreate);
    let mut task = match task_directory(&user, profile.sid.0) {
        Ok(task) => task,
        Err(error) => {
            observation.failed();
            observation.enter(Phase::ProfileCleanup);
            let cleanup = dispose_profile(&mut profile);
            observation.evidence(|evidence| evidence.profile_cleanup = Some(cleanup.is_ok()));
            cleanup?;
            return Err(error);
        }
    };
    #[cfg(test)]
    let mut active = None;
    let executed: Result<(EngineOutput, serde_json::Value, bool), String> = (|| {
        observation.enter(Phase::TaskCopy);
        write_files(&task.0, &inputs)?;
        observation.enter(Phase::TaskHome);
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
            observation,
            cancelled,
        )?;
        #[cfg(test)]
        let mut launched = launched;
        #[cfg(test)]
        {
            active = Some(ActiveTask::new(cwd.clone(), task.0.clone()));
        }
        let deadline = Instant::now() + timeout;
        let mut was_cancelled = false;
        observation.enter(Phase::ProcessWait);
        let timed_out = loop {
            let state = unsafe { WaitForSingleObject(raw(&launched.process.process), 25) };
            let wait_error = (state.0 == u32::MAX).then(|| unsafe { GetLastError().0 });
            observation.evidence(|evidence| {
                evidence.process_wait = Some(state.0);
                if let Some(code) = wait_error {
                    evidence.native_status = Some(code);
                }
            });
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
        observation.evidence(|evidence| {
            evidence.timed_out = Some(timed_out);
            evidence.cancelled = Some(was_cancelled);
        });
        let mut status = 0u32;
        if !timed_out && !was_cancelled {
            observation.enter(Phase::ProcessStatus);
            unsafe {
                win(GetExitCodeProcess(
                    raw(&launched.process.process),
                    &mut status,
                ))?;
            }
            observation.evidence(|evidence| evidence.process_status = Some(status));
        }
        // Stop/wait the whole owned Job before any privileged read or cleanup.
        observation.enter(Phase::JobStop);
        unsafe {
            win(TerminateJobObject(raw(&launched.process.job), 1))?;
        }
        #[cfg(test)]
        if let Some(debugger) = &mut launched.debugger {
            debugger.finish()?;
        }
        observation.enter(Phase::JobProcessWait);
        let stopped = unsafe { WaitForSingleObject(raw(&launched.process.process), 5000) };
        let wait_error = (stopped.0 == u32::MAX).then(|| unsafe { GetLastError().0 });
        observation.evidence(|evidence| {
            evidence.job_process_wait = Some(stopped.0);
            if let Some(code) = wait_error {
                evidence.native_status = Some(code);
            }
        });
        if stopped != WAIT_OBJECT_0 {
            return Err(UNAVAILABLE.into());
        }
        observation.enter(Phase::JobAccounting);
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
            observation.evidence(|evidence| {
                evidence.job_active_processes = Some(accounting.ActiveProcesses);
            });
            if accounting.ActiveProcesses == 0 {
                break;
            }
            if Instant::now() >= stopped_by {
                return Err(UNAVAILABLE.into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        observation.enter(Phase::StdoutReceive);
        let stdout = launched.stdout.recv_timeout(Duration::from_secs(2));
        observation.evidence(|evidence| evidence.stdout_receive = Some(receive_outcome(&stdout)));
        let stdout = stdout.map_err(|_| UNAVAILABLE.to_owned())?;
        observation.enter(Phase::StderrReceive);
        let stderr = launched.stderr.recv_timeout(Duration::from_secs(2));
        observation.evidence(|evidence| evidence.stderr_receive = Some(receive_outcome(&stderr)));
        let stderr = stderr.map_err(|_| UNAVAILABLE.to_owned())?;
        observation.enter(Phase::Copyback);
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
            "lpacProof":"access-check-v1",
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
    if executed.is_err() {
        observation.failed();
    }
    observation.enter(Phase::TaskCleanup);
    let task_cleanup = task.dispose();
    observation.evidence(|evidence| evidence.task_cleanup = Some(task_cleanup.is_ok()));
    if task_cleanup.is_err() {
        observation.failed();
    }
    observation.enter(Phase::ProfileCleanup);
    let profile_cleanup = profile.dispose();
    observation.evidence(|evidence| evidence.profile_cleanup = Some(profile_cleanup.is_ok()));
    if profile_cleanup.is_err() {
        observation.failed();
    }
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
    observation.enter(Phase::Complete);
    Ok(output)
}

fn receive_outcome<T>(result: &Result<T, std::sync::mpsc::RecvTimeoutError>) -> Receive {
    match result {
        Ok(_) => Receive::Ok,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Receive::Timeout,
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Receive::Disconnected,
    }
}

// Deliberately embedded trusted code. The acceptance binary never accepts
// arbitrary Python/JavaScript, host files, gateway URLs, or model credentials.
const NODE_PROBE: &str = r#"
const checkpoint = n => console.error('OFOX_LPAC_PROBE_STAGE:' + n);
checkpoint(1);
const fs = require('node:fs');
const assert = require('node:assert/strict');
const net = require('node:net');
const cp = require('node:child_process');
const outside = OUTSIDE_PATH;
const outsideAap = OUTSIDE_AAP_PATH;
checkpoint(2);
assert.equal(process.versions.node, '22.23.4');
checkpoint(3);
assert.throws(() => fs.readFileSync(outside));
checkpoint(4);
assert.throws(() => fs.readFileSync(outsideAap), 'LPAC read an ALL APPLICATION PACKAGES fixture');
checkpoint(5);
assert.throws(() => fs.writeFileSync(outside, 'changed'));
assert.throws(() => fs.writeFileSync(process.execPath, 'changed'));
checkpoint(6);
for (const name of ['NODE_OPTIONS', 'NODE_PATH', 'OFOX_DOCUMENT_PROBE_SECRET'])
  assert.equal(process.env[name], undefined);
checkpoint(7);
// libuv builds the default stdio named pipes before CreateProcessW. Inherit the
// three handles supplied by the verified host so the probe tests child creation.
const spawned = cp.spawnSync(
  process.execPath,
  ["-e", "require('node:fs').writeFileSync('child-marker', 'bad')"],
  {
    stdio: "inherit",
    timeout: 5000,
  },
);
const childCode = spawned.error?.code;
let childResult = "other-error";
if (fs.existsSync("child-marker")) childResult = "marker-present";
else if (
  !spawned.error ||
  spawned.pid !== 0 ||
  spawned.status !== null ||
  spawned.signal !== null
)
  childResult = "child-started";
else if (
  ["EPERM", "EACCES", "ETIMEDOUT", "ENOENT", "UNKNOWN"].includes(childCode)
)
  childResult = childCode;
console.error("OFOX_LPAC_CHILD_RESULT:" + childResult);
assert.ok(
  childResult === "EPERM" || childResult === "EACCES",
  "child process denial was not proven",
);
const native = process.argv[1];
checkpoint(8);
require(native); // Prove the original binary actually loads before the denial.
checkpoint(9);
fs.copyFileSync(native, 'untrusted.node');
checkpoint(10);
assert.throws(() => require(require('node:path').resolve('untrusted.node')),
  'a DLL copied into the writable low-label workspace loaded');
checkpoint(11);
fs.unlinkSync('untrusted.node');
checkpoint(12);
const canvas = require(process.argv[2]).createCanvas(320, 180);
canvas.getContext('2d').fillText('Windows 中文 日本語', 10, 40);
checkpoint(13);
fs.writeFileSync('probe.png', canvas.toBuffer('image/png'));
checkpoint(14);
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
    let name = wide_path(path)?;
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

#[derive(Clone, Copy)]
enum ProbeStage {
    RuntimeBefore,
    Setup,
    NodeIsolation,
    NodePostconditions,
    PythonCreate,
    PythonArtifact,
    OfficeValidate,
    OfficeSvg,
    PythonSourceEdit,
    SourcePreservation,
    RuntimeAfter,
}
impl ProbeStage {
    fn name(self) -> &'static str {
        match self {
            Self::RuntimeBefore => "runtime_before",
            Self::Setup => "setup",
            Self::NodeIsolation => "node_isolation",
            Self::NodePostconditions => "node_postconditions",
            Self::PythonCreate => "python_create",
            Self::PythonArtifact => "python_artifact",
            Self::OfficeValidate => "office_validate",
            Self::OfficeSvg => "office_svg",
            Self::PythonSourceEdit => "python_source_edit",
            Self::SourcePreservation => "source_preservation",
            Self::RuntimeAfter => "runtime_after",
        }
    }
}

struct ProbeTrace {
    stage: ProbeStage,
    started: Instant,
    stage_started: Instant,
    timings: Vec<serde_json::Value>,
    engine: Option<serde_json::Value>,
    observation: Observer,
}
impl ProbeTrace {
    fn new() -> Self {
        Self {
            stage: ProbeStage::RuntimeBefore,
            started: Instant::now(),
            stage_started: Instant::now(),
            timings: Vec::new(),
            engine: None,
            observation: Observer::default(),
        }
    }
    fn enter(&mut self, stage: ProbeStage) {
        self.timings.push(serde_json::json!({
            "stage":self.stage.name(), "elapsedMs":self.stage_started.elapsed().as_millis()
        }));
        self.stage = stage;
        self.stage_started = Instant::now();
    }
    fn engine(&mut self, stage: ProbeStage) {
        self.enter(stage);
        self.engine = None;
        self.observation = Observer::fixed_probe();
    }
    fn success(&mut self, output: EngineOutput) -> Result<EngineOutput, String> {
        // Only our fixed stderr checkpoint grammar is retained. No exception
        // message, stdout/stderr text, source path, account SID or Python code.
        let marker = output
            .stderr
            .lines()
            .filter_map(|line| line.strip_prefix("OFOX_LPAC_PROBE_STAGE:"))
            .filter_map(|value| value.parse::<u8>().ok())
            .rfind(|value| (1..=14).contains(value));
        let child_result = output.stderr.lines().find_map(|line| {
            let result = line.strip_prefix("OFOX_LPAC_CHILD_RESULT:")?;
            matches!(
                result,
                "EPERM"
                    | "EACCES"
                    | "ETIMEDOUT"
                    | "ENOENT"
                    | "UNKNOWN"
                    | "child-started"
                    | "marker-present"
                    | "other-error"
            )
            .then_some(result)
        });
        self.engine = Some(serde_json::json!({
            "status":output.status, "timedOut":output.timed_out,
            "stdoutBytes":output.stdout.len(), "stderrBytes":output.stderr.len(),
            "nodeCheckpointBeforeStep":marker, "childSpawnResult":child_result
        }));
        if output.status != Some(0) || output.timed_out {
            return Err(UNAVAILABLE.into());
        }
        Ok(output)
    }
    fn failure(&self, error: &str, checks: &[&'static str]) -> String {
        let mut reason = match error {
            UNAVAILABLE => "unavailable",
            UNSAFE_FILE => "unsafe_file_or_io",
            CANCELLED => "cancelled",
            RUNTIME_CHANGED => "runtime_changed",
            "assistant_document_sandbox_cleanup_failed" => "cleanup_failed",
            "assistant_document_sandbox_unproven_runtime" => "unproven_runtime",
            _ => "other",
        };
        let mut native = None;
        if error == UNAVAILABLE {
            if let Some(engine) = &self.engine {
                if engine["timedOut"] == true {
                    reason = "process_timeout";
                } else if engine["status"].as_i64().is_some_and(|status| status != 0) {
                    reason = "process_exit";
                }
            }
        }
        let hex = |value: &str| {
            (value.len() == 8 && value.bytes().all(|c| c.is_ascii_hexdigit()))
                .then(|| value.to_owned())
        };
        if let Some(tail) = error.strip_prefix("assistant_document_sandbox_win32_") {
            if let Some((code, line)) = tail.split_once("_at_") {
                if let (Some(code), Ok(line)) = (hex(code), line.parse::<u32>()) {
                    reason = "win32";
                    native = Some(serde_json::json!({"code":code,"sourceLine":line}));
                }
            }
        } else if let Some(tail) = error.strip_prefix("assistant_document_sandbox_rename_status_") {
            if let Some((status, completion)) = tail.split_once("_completion_") {
                if let (Some(status), Some(completion)) = (hex(status), hex(completion)) {
                    reason = "rename_status";
                    native = Some(serde_json::json!({"status":status,"completion":completion}));
                }
            }
        } else if let Some(code) = error.strip_prefix(&format!("{UNAVAILABLE}:cache_acl_query_")) {
            if let Some(code) = hex(code) {
                reason = "cache_acl_query";
                native = Some(serde_json::json!({"code":code}));
            }
        } else if let Some(raw) = error.strip_prefix(&format!("{UNAVAILABLE}:cache_acl_mismatch "))
        {
            reason = "cache_acl_mismatch";
            // Keep the original descriptor's numeric control evidence without
            // forwarding an arbitrary nested JSON/error payload.
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
                let control = |which: &str| {
                    value[which]["control"].as_str().filter(|code| {
                        code.len() == 4 && code.bytes().all(|c| c.is_ascii_hexdigit())
                    })
                };
                native = Some(serde_json::json!({
                    "actualControl":control("actual"), "expectedControl":control("expected"),
                    "actualRevision":value["actual"]["revision"].as_u64().filter(|n| *n <= u32::MAX as u64),
                    "expectedRevision":value["expected"]["revision"].as_u64().filter(|n| *n <= u32::MAX as u64)
                }));
            }
        }
        let allowed = [
            "windowsBuild",
            "outsideReadWriteDenied",
            "allApplicationPackagesReadDenied",
            "childProcessDenied",
            "lowImageDenied",
            "networkDenied",
            "nativeCanvas",
            "pythonPptx",
            "officeValidation",
            "officeSvg",
            "frozenSourceEdit",
        ];
        let completed: Vec<_> = checks
            .iter()
            .filter(|check| allowed.contains(check))
            .collect();
        let code = match error {
            UNSAFE_FILE
            | CANCELLED
            | RUNTIME_CHANGED
            | "assistant_document_sandbox_cleanup_failed" => error,
            _ => UNAVAILABLE,
        };
        let detail = serde_json::json!({
            "stage":self.stage.name(), "reason":reason, "native":native,
            "engine":self.engine, "completedChecks":completed,
            "worker":self.observation.snapshot(),
            "elapsedMs":self.started.elapsed().as_millis(),
            "stageElapsedMs":self.stage_started.elapsed().as_millis(),
            "completedStages":self.timings
        });
        format!("{code}:probe {detail}")
    }
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
    let mut trace = ProbeTrace::new();
    let result: Result<String, String> = async {
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
        probe_inner(runtime_root, officecli, checks, &mut trace).await?;
        trace.engine(ProbeStage::RuntimeAfter);
        let root = runtime_root.to_path_buf();
        let after = tokio::task::spawn_blocking(move || {
            cache_runtime(&root, &RuntimeCapability::new()?, &current_user()?)
        })
        .await
        .map_err(|_| UNAVAILABLE.to_owned())??;
        if before.digest != after.digest {
            return Err(RUNTIME_CHANGED.into());
        }
        ADMITTED
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| UNAVAILABLE)?
            .insert(after.source, after.digest.clone());
        Ok(after.digest)
    }
    .await;
    result.map_err(|error| trace.failure(&error, checks))
}

async fn probe_inner(
    runtime_root: &Path,
    officecli: &Path,
    checks: &mut Vec<&'static str>,
    trace: &mut ProbeTrace,
) -> Result<(), String> {
    trace.enter(ProbeStage::Setup);
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
    trace.engine(ProbeStage::NodeIsolation);
    #[cfg(test)]
    let loader_diagnostic = std::env::var_os(cdb::ENV_ROOT).is_some();
    #[cfg(test)]
    if loader_diagnostic {
        trace.observation.request_loader();
    }
    // Under the debugger every observed call costs two stops, and libuv's
    // abort holds the target until this timeout; give the trace more room.
    #[cfg(test)]
    let node_timeout = Duration::from_secs(if loader_diagnostic { 60 } else { 30 });
    #[cfg(not(test))]
    let node_timeout = Duration::from_secs(30);
    let observation = trace.observation.clone();
    trace.success(
        run_candidate_observed(
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
            node_timeout,
            16 * 1024,
            true,
            observation,
        )
        .await?,
    )?;
    #[cfg(test)]
    if loader_diagnostic {
        // Debugger execution is an observation, never engine admission. Stop
        // after this fixed Node fixture even if observing changes its outcome.
        return Err(UNAVAILABLE.into());
    }
    trace.enter(ProbeStage::NodePostconditions);
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
    trace.engine(ProbeStage::PythonCreate);
    let observation = trace.observation.clone();
    trace.success(
        run_candidate_observed(
            &node,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            &cwd,
            &home,
            &runtime_root,
            Duration::from_secs(90),
            16 * 1024,
            true,
            observation,
        )
        .await?,
    )?;
    trace.enter(ProbeStage::PythonArtifact);
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
        trace.engine(if args[0] == "validate" {
            ProbeStage::OfficeValidate
        } else {
            ProbeStage::OfficeSvg
        });
        let observation = trace.observation.clone();
        let output = trace.success(
            run_candidate_observed(
                officecli,
                &args,
                &cwd,
                &home,
                &runtime_root,
                Duration::from_secs(60),
                4 * 1024 * 1024,
                true,
                observation,
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
    trace.engine(ProbeStage::PythonSourceEdit);
    let observation = trace.observation.clone();
    trace.success(
        run_candidate_observed(
            &node,
            &edit_args.iter().map(String::as_str).collect::<Vec<_>>(),
            &cwd,
            &home,
            &runtime_root,
            Duration::from_secs(90),
            16 * 1024,
            true,
            observation,
        )
        .await?,
    )?;
    trace.enter(ProbeStage::SourcePreservation);
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
