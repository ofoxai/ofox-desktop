//! Windows document-only LPAC launcher. This is deliberately separate from the
//! general assistant process runner: a Job alone is not a security boundary.
//!
//! No caller may advertise Python merely because this module compiles. The
//! native functional/negative probes must pass on the actual installed assets.
use super::runner::{EngineOutput, DEAD_PROXY};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
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
        atomic::{AtomicBool, Ordering},
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
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                GetSecurityInfo, SetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            DeriveCapabilitySidsFromName, FreeSid, GetSecurityDescriptorSacl, GetSidSubAuthority,
            GetSidSubAuthorityCount, GetTokenInformation,
            Isolation::{
                CreateAppContainerProfile, DeleteAppContainerProfile, GetAppContainerFolderPath,
            },
            TokenIntegrityLevel, TokenIsAppContainer, TokenIsLessPrivilegedAppContainer, TokenUser,
            LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
            SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES, SE_GROUP_ENABLED, SID_AND_ATTRIBUTES,
            TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER,
        },
        Storage::FileSystem::{
            CreateDirectoryW, CreateFileW, GetFileInformationByHandle, GetFinalPathNameByHandleW,
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
            FILE_NAME_NORMALIZED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
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

fn win<T>(result: windows::core::Result<T>) -> Result<T, String> {
    result.map_err(|_| UNAVAILABLE.to_owned())
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
    format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{user})")
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
    let root_handle = open_path(root, true)?;
    let physical_root = final_path(&root_handle)?;
    let relative = path
        .strip_prefix(root)
        .map_err(|_| UNSAFE_FILE.to_owned())?;
    let parts: Vec<_> = relative.components().collect();
    if parts.is_empty() || parts.iter().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(UNSAFE_FILE.into());
    }
    let mut directories = vec![root_handle];
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

fn files(root: &Path, max: u64) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let _pin = open_path(root, true)?;
    let mut pending = vec![root.to_path_buf()];
    let (mut result, mut total) = (Vec::new(), 0u64);
    while let Some(directory) = pending.pop() {
        let _directory = open_path(&directory, true)?;
        for item in io(fs::read_dir(&directory))? {
            let item = io(item)?;
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

#[derive(Clone)]
struct CachedRuntime {
    source: PathBuf,
    root: PathBuf,
    digest: String,
}
static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedRuntime>>> = OnceLock::new();
fn cache_runtime(
    source: &Path,
    capability: &RuntimeCapability,
    user: &str,
) -> Result<CachedRuntime, String> {
    let source = io(source.canonicalize())?;
    let mut cached = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| UNAVAILABLE)?;
    if let Some(runtime) = cached.get(&source) {
        let _pin = owned_directory(&runtime.root, user)?;
        return Ok(runtime.clone());
    }
    let contents = files(&source, MAX_TREE_BYTES)?;
    let digest = tree_digest(&contents);
    let private_parent = std::env::temp_dir().join(format!(
        "ofox-document-runtime-{}",
        &format!("{:x}", Sha256::digest(user.as_bytes()))[..16]
    ));
    if !private_parent.exists() {
        create_directory(&private_parent, &private_sddl(user))?;
    }
    let _parent = owned_directory(&private_parent, user)?;
    // A fresh immutable directory per process prevents adopting unknown ACLs
    // or concurrently replacing a cache another Desktop instance is using.
    let root = private_parent.join(format!("{digest}-{}", uuid::Uuid::new_v4().simple()));
    let cap = sid_string(capability.sid)?;
    create_directory(
        &root,
        &format!(
            "{}(A;OICI;GRGX;;;{cap})S:(ML;OICI;NW;;;ME)",
            private_sddl(user)
        ),
    )?;
    if let Err(error) = write_files(&root, &contents) {
        let _ = fs::remove_dir_all(&root);
        return Err(error);
    }
    if tree_digest(&files(&root, MAX_TREE_BYTES)?) != digest {
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
        })
    }
    fn make_low_profile(&self) -> Result<(), String> {
        let sid = wide(sid_string(self.sid.0)?)?;
        let folder = unsafe { win(GetAppContainerFolderPath(PCWSTR(sid.as_ptr())))? };
        let result = unsafe { folder.to_string().map_err(|_| UNAVAILABLE.to_owned()) };
        unsafe {
            CoTaskMemFree(Some(folder.0.cast()));
        }
        let path = PathBuf::from(result?);
        // Profile is freshly created, before untrusted code can plant links.
        let _pin = open_path(&path, true)?;
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
}
impl Drop for Profile {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteAppContainerProfile(PCWSTR(self.name.as_ptr()));
        }
    }
}

struct TaskDirectory(PathBuf);
impl Drop for TaskDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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
    Ok(TaskDirectory(path))
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
fn verify_token(process: HANDLE) -> Result<(), String> {
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
    verify_token(raw(&process.process))?;
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
    for (from, to) in [(&runtime.source, &runtime.root), (source, task)] {
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

/// Same result/limits as the ordinary runner, with a mandatory LPAC boundary.
/// The sandbox never receives ACL access to the host's original document tree.
pub async fn run_windows_engine_limited(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    home: &Path,
    read_root: &Path,
    timeout: Duration,
    limit: usize,
) -> Result<EngineOutput, String> {
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
        run_blocking(&exe, &args, &cwd, &home, &root, timeout, limit, &cancelled)
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
) -> Result<EngineOutput, String> {
    os_version()?;
    let user = current_user()?;
    let capability = RuntimeCapability::new()?;
    let runtime = cache_runtime(read_root, &capability, &user)?;
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
    let profile = Profile::new(&capability)?;
    profile.make_low_profile()?;
    let task = task_directory(&user, profile.sid.0)?;
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
    let deadline = Instant::now() + timeout;
    let timed_out = loop {
        let state = unsafe { WaitForSingleObject(raw(&launched.process.process), 25) };
        if state == WAIT_OBJECT_0 {
            break false;
        }
        if state != WAIT_TIMEOUT {
            return Err(UNAVAILABLE.into());
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(CANCELLED.into());
        }
        if Instant::now() >= deadline {
            break true;
        }
    };
    let mut status = 0u32;
    if !timed_out {
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
    if accounting.ActiveProcesses != 0 {
        return Err(UNAVAILABLE.into());
    }
    let stdout = launched
        .stdout
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| UNAVAILABLE.to_owned())?;
    let stderr = launched
        .stderr
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| UNAVAILABLE.to_owned())?;
    if !timed_out {
        let outputs = files(&task.0, MAX_TASK_BYTES)?;
        // Original cwd is never granted to the LPAC. Keep its directory handle
        // pinned, and reject pre-existing aliases before atomic replacement.
        let _destination = open_path(&cwd, true)?;
        for (relative, bytes) in outputs {
            if relative.starts_with(home_relative) {
                continue;
            }
            let into = cwd.join(&relative);
            if into.exists() {
                let _ = open_regular(&cwd, &into, MAX_TASK_BYTES)?;
            }
            if let Some(parent) = into.parent() {
                io(fs::create_dir_all(parent))?;
            }
            crate::config::atomic_write(&into, &bytes).map_err(|_| UNSAFE_FILE.to_owned())?;
        }
    }
    drop(launched);
    drop(task);
    drop(profile);
    Ok(EngineOutput {
        status: (!timed_out).then_some(status as i32),
        stdout: bounded_text(stdout, limit),
        stderr: bounded_text(stderr, limit),
        timed_out,
    })
}

// Deliberately embedded trusted code. The acceptance binary never accepts
// arbitrary Python/JavaScript, host files, gateway URLs, or model credentials.
const NODE_PROBE: &str = r#"
const fs = require('node:fs');
const assert = require('node:assert/strict');
const net = require('node:net');
const cp = require('node:child_process');
const outside = OUTSIDE_PATH;
assert.equal(process.versions.node, '22.23.4');
assert.throws(() => fs.readFileSync(outside));
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
    probe_inner(runtime_root, officecli, &mut Vec::new()).await
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
        .replace("PROBE_PORT", &address.port().to_string());
    let native = runtime_root
        .join("runtime/node_modules/@napi-rs/canvas-win32-x64-msvc/skia.win32-x64-msvc.node");
    let canvas = runtime_root.join("runtime/node_modules/@napi-rs/canvas");
    let node = runtime_root.join("node/node.exe");
    success(
        run_windows_engine_limited(
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
        )
        .await?,
    )?;
    if listener.accept().is_ok() || io(fs::read(&outside))? != b"synthetic private data" {
        return Err(UNAVAILABLE.into());
    }
    checks.extend([
        "outsideReadWriteDenied",
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
        run_windows_engine_limited(
            &node,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            &cwd,
            &home,
            &runtime_root,
            Duration::from_secs(90),
            16 * 1024,
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
            run_windows_engine_limited(
                officecli,
                &args,
                &cwd,
                &home,
                &runtime_root,
                Duration::from_secs(60),
                4 * 1024 * 1024,
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
        run_windows_engine_limited(
            &node,
            &edit_args.iter().map(String::as_str).collect::<Vec<_>>(),
            &cwd,
            &home,
            &runtime_root,
            Duration::from_secs(90),
            16 * 1024,
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
    let result = probe_inner(
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
