use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::AppError;

/// 各 agent CLI 用来改写自己配置目录的环境变量。
///
/// 测试沙箱（`test_sandbox::sandbox()`，单元测试与集成测试共用）会清空这些变量：开发者 shell 里残留的任何一个都会让测试读写真实 agent
/// 的目录。只列出确实会被读到的变量：`HERMES_HOME`、`PI_CODING_AGENT_DIR`、`DSH_HOME`
/// 由本应用直接读取；`CODEX_HOME`、`CLAUDE_CONFIG_DIR` 由应用启动 / 探测的 Codex、
/// Claude Code CLI 解析（也是应用要遵循的目录）。`GEMINI_CLI_HOME`、`OPENCODE_CONFIG_DIR`、
/// `OPENCODE_CONFIG`、`WORKBUDDY_CONFIG_DIR` 目前没有任何代码读取，故不在列；
/// 新增读取某个 agent 目录变量时，在这里补上。
pub const AGENT_DIR_VARS: &[&str] = &[
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "HERMES_HOME",
    "PI_CODING_AGENT_DIR",
    "DSH_HOME",
];

/// 读取 `CC_SWITCH_TEST_HOME` 显式覆盖；空白视为未设置。
fn home_override() -> Option<PathBuf> {
    let home = std::env::var("CC_SWITCH_TEST_HOME").ok()?;
    let trimmed = home.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

#[cfg(not(test))]
fn real_home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// 单元测试里没有显式覆盖时，永远不碰真实 home：整个进程共用一个一次性临时目录。
#[cfg(test)]
fn real_home_dir() -> Option<PathBuf> {
    use std::sync::OnceLock;

    static PROCESS_TEST_HOME: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = PROCESS_TEST_HOME.get_or_init(|| {
        tempfile::Builder::new()
            .prefix("ofox-test-home-")
            .tempdir()
            .expect("create the per-process test home")
    });
    Some(dir.path().to_path_buf())
}

/// 获取用户主目录；取不到时返回 `None`（调用方需要区分“没有 home”时用它）。
///
/// 这是整个 crate 里唯一允许向系统询问 home 的地方（`tests/home_dir_guard.rs` 会检查）。
/// 其余代码一律走 [`get_home_dir`] 或本函数。
pub fn try_get_home_dir() -> Option<PathBuf> {
    home_override().or_else(real_home_dir)
}

/// 获取用户主目录，带回退和日志
///
/// ## Windows 注意事项
///
/// - `dirs::home_dir()` 在 Windows 上使用 `SHGetKnownFolderPath(FOLDERID_Profile)`，
///   返回的是真实用户目录（类似 `C:\\Users\\Alice`），与 v3.10.2 行为一致。
/// - 不要直接使用 `HOME` 环境变量：它可能由 Git/Cygwin/MSYS 等第三方工具注入，
///   且不一定等于用户目录，可能导致 `.ofox-desktop/ofox-desktop.db` 路径变化，从而”看起来像数据丢失”。
///
/// ## 测试隔离
///
/// 为了让 Windows CI/本地测试能稳定隔离真实用户数据，可通过 `CC_SWITCH_TEST_HOME`
/// 显式覆盖 home dir（仅用于测试/调试场景）。单元测试（`cfg(test)`）里没有设置该变量时，
/// 返回本进程的一次性临时目录而不是真实 home。
pub fn get_home_dir() -> PathBuf {
    try_get_home_dir().unwrap_or_else(|| {
        log::warn!("无法获取用户主目录，回退到当前目录");
        PathBuf::from(".")
    })
}

/// 获取 Claude Code 配置目录路径
pub fn get_claude_config_dir() -> PathBuf {
    if let Some(custom) = crate::settings::get_claude_override_dir() {
        return custom;
    }

    get_home_dir().join(".claude")
}

/// 默认 Claude MCP 配置文件路径 (~/.claude.json)
pub fn get_default_claude_mcp_path() -> PathBuf {
    get_home_dir().join(".claude.json")
}

fn derive_mcp_path_from_override(dir: &Path) -> Option<PathBuf> {
    let file_name = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())?
        .trim()
        .to_string();
    if file_name.is_empty() {
        return None;
    }
    let parent = dir.parent().unwrap_or_else(|| Path::new(""));
    Some(parent.join(format!("{file_name}.json")))
}

/// 获取 Claude MCP 配置文件路径，若设置了目录覆盖则与覆盖目录同级
pub fn get_claude_mcp_path() -> PathBuf {
    if let Some(custom_dir) = crate::settings::get_claude_override_dir() {
        if let Some(path) = derive_mcp_path_from_override(&custom_dir) {
            return path;
        }
    }
    get_default_claude_mcp_path()
}

/// 获取 Claude Code 主配置文件路径
pub fn get_claude_settings_path() -> PathBuf {
    let dir = get_claude_config_dir();
    let settings = dir.join("settings.json");
    if settings.exists() {
        return settings;
    }
    // 兼容旧版命名：若存在旧文件则继续使用
    let legacy = dir.join("claude.json");
    if legacy.exists() {
        return legacy;
    }
    // 默认新建：回落到标准文件名 settings.json（不再生成 claude.json）
    settings
}

/// 获取应用配置目录路径 (~/.ofox-desktop)
pub fn get_app_config_dir() -> PathBuf {
    if let Some(custom) = crate::app_store::get_app_config_dir_override() {
        return custom;
    }

    get_home_dir().join(".ofox-desktop")
}

/// 获取应用配置文件路径
pub fn get_app_config_path() -> PathBuf {
    get_app_config_dir().join("config.json")
}

/// 清理供应商名称，确保文件名安全
#[allow(dead_code)]
pub fn sanitize_provider_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            _ => c,
        })
        .collect::<String>()
        .to_lowercase()
}

/// 获取供应商配置文件路径
#[allow(dead_code)]
pub fn get_provider_config_path(provider_id: &str, provider_name: Option<&str>) -> PathBuf {
    let base_name = provider_name
        .map(sanitize_provider_name)
        .unwrap_or_else(|| sanitize_provider_name(provider_id));

    get_claude_config_dir().join(format!("settings-{base_name}.json"))
}

/// 读取 JSON 配置文件
pub fn read_json_file<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T, AppError> {
    if !path.exists() {
        return Err(AppError::Config(format!("文件不存在: {}", path.display())));
    }

    let content = fs::read_to_string(path).map_err(|e| AppError::io(path, e))?;

    serde_json::from_str(&content).map_err(|e| AppError::json(path, e))
}

/// 写入 JSON 配置文件
pub fn write_json_file<T: Serialize>(path: &Path, data: &T) -> Result<(), AppError> {
    // 确保目录存在
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let json =
        serde_json::to_string_pretty(data).map_err(|e| AppError::JsonSerialize { source: e })?;

    atomic_write(path, json.as_bytes())
}

/// 原子写入文本文件（用于 TOML/纯文本）
pub fn write_text_file(path: &Path, data: &str) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    atomic_write(path, data.as_bytes())
}

/// [`atomic_write_with`] 的可选项。
#[derive(Clone, Copy, Debug, Default)]
pub struct AtomicOpts {
    /// 写出文件的 Unix 权限位（Windows 上忽略）。`None` 时沿用被替换文件的权限，
    /// 文件还不存在则用 [`NEW_FILE_MODE`]。
    pub mode: Option<u32>,
}

/// 新建文件的默认权限：agent 的配置里常有 API key，只给自己读写。
const NEW_FILE_MODE: u32 = 0o600;

/// 跟随符号链接时最多走几步，防止环。
const MAX_SYMLINK_HOPS: usize = 40;

/// 临时文件重名（同一纳秒内的并发写）时最多换几次名字。
const MAX_TEMP_NAME_ATTEMPTS: usize = 16;

/// Windows 上 rename 被别的进程短暂占用（杀毒、索引、agent 正在读）时，每次重试前的等待，
/// 一共十次尝试、约 1 秒。
#[cfg(windows)]
const RENAME_BACKOFF: [std::time::Duration; 9] = [
    std::time::Duration::from_millis(10),
    std::time::Duration::from_millis(20),
    std::time::Duration::from_millis(40),
    std::time::Duration::from_millis(80),
    std::time::Duration::from_millis(160),
    std::time::Duration::from_millis(200),
    std::time::Duration::from_millis(200),
    std::time::Duration::from_millis(200),
    std::time::Duration::from_millis(200),
];

/// 原子写入：写入临时文件后 rename 替换，避免半写状态
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), AppError> {
    atomic_write_with(path, data, AtomicOpts::default())
}

/// 原子写入，可指定文件权限。
///
/// - `path` 是符号链接（常见于放在 dotfiles 仓库里的配置）时写它指向的文件，链接本身保留。
/// - 临时文件建在目标同目录，创建时就带上最终权限（没有“先 0644 再改 0600”的空窗），
///   写完 `sync_all` 之后才 rename：断电不会留下长度对、内容全零的文件。
/// - 任何一步失败，临时文件都会被清掉，目标文件保持原样。
pub fn atomic_write_with(path: &Path, data: &[u8], opts: AtomicOpts) -> Result<(), AppError> {
    let target = write_target(path)?;
    let file_name = target
        .file_name()
        .ok_or_else(|| AppError::Config("无效的文件名".to_string()))?;
    let parent = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        Some(_) => Path::new("."),
        None => return Err(AppError::Config("无效的路径".to_string())),
    };
    fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;

    let (tmp, mut file) = create_temp_file(parent, file_name, &target, opts)?;
    let mut guard = TempFileGuard::new(tmp.clone());
    file.write_all(data).map_err(|e| AppError::io(&tmp, e))?;
    file.sync_all().map_err(|e| AppError::io(&tmp, e))?;
    // Windows 不能重命名还开着的文件。
    drop(file);

    rename_over(&tmp, &target).map_err(|e| AppError::IoContext {
        context: format!("原子替换失败: {} -> {}", tmp.display(), target.display()),
        source: e,
    })?;
    guard.disarm();
    sync_dir(parent);
    Ok(())
}

/// 写 `path` 时真正要替换的文件：`path` 不是符号链接就是它自己，否则是链接指向的文件
/// （目标还不存在的悬空链接也算，这样写入会建出目标而不是把链接换成普通文件）。
fn write_target(path: &Path) -> Result<PathBuf, AppError> {
    if !is_symlink(path) {
        return Ok(path.to_path_buf());
    }
    if let Ok(resolved) = fs::canonicalize(path) {
        return Ok(resolved);
    }
    // 悬空链接 canonicalize 会失败：一步步读链接，直到走到不是链接的路径。
    let mut current = path.to_path_buf();
    for _ in 0..MAX_SYMLINK_HOPS {
        if !is_symlink(&current) {
            return Ok(current);
        }
        let destination = fs::read_link(&current).map_err(|e| AppError::io(&current, e))?;
        current = if destination.is_absolute() {
            destination
        } else {
            current
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(destination)
        };
    }
    Err(AppError::Config(format!(
        "Too many levels of symbolic links: {}",
        path.display()
    )))
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}

/// 在目标同目录建临时文件 `.{name}.ofox-tmp-{pid}-{nanos}`（`create_new`，不会写到别人的文件上）。
#[cfg_attr(not(unix), allow(unused_variables))]
fn create_temp_file(
    dir: &Path,
    file_name: &std::ffi::OsStr,
    target: &Path,
    opts: AtomicOpts,
) -> Result<(PathBuf, fs::File), AppError> {
    #[cfg(unix)]
    let mode = opts
        .mode
        .or_else(|| existing_mode(target))
        .unwrap_or(NEW_FILE_MODE);

    for _ in 0..MAX_TEMP_NAME_ATTEMPTS {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut name = std::ffi::OsString::from(".");
        name.push(file_name);
        name.push(format!(".ofox-tmp-{}-{nanos}", std::process::id()));
        let tmp = dir.join(name);

        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        match options.open(&tmp) {
            Ok(file) => {
                #[cfg(unix)]
                if let Err(e) = apply_exact_mode(&file, mode) {
                    drop(file);
                    let _ = fs::remove_file(&tmp);
                    return Err(AppError::io(&tmp, e));
                }
                return Ok((tmp, file));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(AppError::io(&tmp, e)),
        }
    }
    Err(AppError::Config(format!(
        "Could not pick an unused temp file name next to {}",
        target.display()
    )))
}

/// 被替换文件现在的权限位（不含 setuid 等特殊位）；文件不存在返回 `None`。
#[cfg(unix)]
fn existing_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o777)
}

/// 创建时给的权限会被 umask 削掉（0o664 在 umask 022 下变 0o644）；再 `fchmod` 一次，
/// 让结果和要求的完全一致。收窄不受 umask 影响，所以不会在写入内容之前变宽。
#[cfg(unix)]
fn apply_exact_mode(file: &fs::File, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(mode))
}

/// 退出作用域时删掉临时文件，除非已经 [`disarm`](Self::disarm)：任何错误路径（含 `?`、panic）都不留垃圾。
struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// 把 `from` 改名成 `to`，替换已有的 `to`。
///
/// Unix 上 `rename` 本身是原子替换。Windows 上标准库的 `rename` 也会替换已有文件，所以不要先删目标
/// （先删再改名中间一断电，配置就没了）；目标恰好被别的进程打开时会得到 5/32/33，短暂重试。
#[cfg(windows)]
fn rename_over(from: &Path, to: &Path) -> std::io::Result<()> {
    retry_transient(
        || fs::rename(from, to),
        is_sharing_violation,
        &RENAME_BACKOFF,
    )
}

#[cfg(not(windows))]
fn rename_over(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

/// ERROR_ACCESS_DENIED / ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION。
#[cfg(windows)]
fn is_sharing_violation(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

/// 先试一次；失败且 `is_transient` 认为值得再试，就按 `backoff` 逐项等待后重试，
/// 最多共 `backoff.len() + 1` 次，最后一次的结果原样返回。
#[cfg(any(windows, test))]
fn retry_transient<T>(
    mut op: impl FnMut() -> std::io::Result<T>,
    is_transient: impl Fn(&std::io::Error) -> bool,
    backoff: &[std::time::Duration],
) -> std::io::Result<T> {
    let mut result = op();
    for pause in backoff {
        match &result {
            Err(error) if is_transient(error) => {
                std::thread::sleep(*pause);
                result = op();
            }
            _ => break,
        }
    }
    result
}

/// rename 之后把目录项也刷到盘上，断电后新文件才一定在。尽力而为：Windows 没有对应操作，
/// 个别文件系统也不支持，失败不影响已经完成的写入。
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Err(e) = fs::File::open(dir).and_then(|handle| handle.sync_all()) {
        log::debug!("fsync of {} failed: {e}", dir.display());
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// 读文件原始字节；文件不存在返回 `None`。
///
/// Windows 上别的进程正在用 rename 替换这个文件时，它会有一瞬间“不存在”（实测约 2 毫秒），
/// 读成 `None` 会让调用方以为用户没有配置；所以先短暂重试，真不存在的文件要多等 100 毫秒。
pub(crate) fn read_file_bytes(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match read_waiting_out_replace(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AppError::io(path, e)),
    }
}

#[cfg(windows)]
fn read_waiting_out_replace(path: &Path) -> std::io::Result<Vec<u8>> {
    retry_transient(
        || fs::read(path),
        |e| e.kind() == std::io::ErrorKind::NotFound,
        &[std::time::Duration::from_millis(20); 5],
    )
}

#[cfg(not(windows))]
fn read_waiting_out_replace(path: &Path) -> std::io::Result<Vec<u8>> {
    fs::read(path)
}

fn remove_file_if_exists(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::io(path, e)),
    }
}

/// Unix 权限位；其它平台没有这个概念，返回 `None`。
#[cfg(unix)]
pub(crate) fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
pub(crate) fn file_mode(_path: &Path) -> Option<u32> {
    None
}

#[cfg(unix)]
pub(crate) fn set_file_mode(path: &Path, mode: u32) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|e| AppError::io(path, e))
}

#[cfg(not(unix))]
pub(crate) fn set_file_mode(_path: &Path, _mode: u32) -> Result<(), AppError> {
    Ok(())
}

/// 多文件写入事务：每个文件第一次被改动前记下原来的字节和权限（不存在记为
/// `None`），任一步失败时 [`FileTxn::rollback`] 按相反顺序全部还原。
#[derive(Default)]
pub(crate) struct FileTxn {
    originals: Vec<(PathBuf, Option<Vec<u8>>, Option<u32>)>,
}

impl FileTxn {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn remember(&mut self, path: &Path) -> Result<(), AppError> {
        if self.originals.iter().any(|(seen, _, _)| seen == path) {
            return Ok(());
        }
        let original = read_file_bytes(path)?;
        self.originals
            .push((path.to_path_buf(), original, file_mode(path)));
        Ok(())
    }

    pub(crate) fn write(&mut self, path: &Path, data: &[u8]) -> Result<(), AppError> {
        self.remember(path)?;
        atomic_write(path, data)
    }

    pub(crate) fn set_mode(&mut self, path: &Path, mode: u32) -> Result<(), AppError> {
        self.remember(path)?;
        set_file_mode(path, mode)
    }

    pub(crate) fn remove(&mut self, path: &Path) -> Result<(), AppError> {
        self.remember(path)?;
        remove_file_if_exists(path)
    }

    /// 把改过的文件全部还原成事务开始前的样子；尽量全部还原，返回第一个错误。
    pub(crate) fn rollback(self) -> Result<(), AppError> {
        let mut first_error = None;
        for (path, original, mode) in self.originals.into_iter().rev() {
            let result = match original {
                Some(bytes) => atomic_write_with(&path, &bytes, AtomicOpts { mode }),
                None => remove_file_if_exists(&path),
            };
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn file_txn_rollback_restores_changed_created_and_removed_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let existing = dir.path().join("existing.toml");
        let created = dir.path().join("nested/created.json");
        let removed = dir.path().join("removed.env");
        fs::write(&existing, b"original = 1\n").unwrap();
        fs::write(&removed, b"KEEP=1\n").unwrap();

        let mut txn = FileTxn::new();
        txn.write(&existing, b"changed = 2\n").unwrap();
        txn.write(&existing, b"changed again = 3\n").unwrap();
        txn.write(&created, b"{}").unwrap();
        txn.remove(&removed).unwrap();
        txn.rollback().unwrap();

        assert_eq!(fs::read(&existing).unwrap(), b"original = 1\n");
        assert!(!created.exists());
        assert_eq!(fs::read(&removed).unwrap(), b"KEEP=1\n");
    }

    #[cfg(unix)]
    #[test]
    fn file_txn_rollback_restores_the_original_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        fs::write(&path, b"a = 1\n").unwrap();
        set_file_mode(&path, 0o644).unwrap();

        let mut txn = FileTxn::new();
        txn.write(&path, b"a = 2\n").unwrap();
        txn.set_mode(&path, 0o600).unwrap();
        txn.rollback().unwrap();

        assert_eq!(file_mode(&path), Some(0o644));
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn atomic_write_creates_parent_directories_and_replaces_the_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested/deeper/config.json");

        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(names_in(path.parent().unwrap()), ["config.json"]);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_through_a_symlink_writes_the_target_and_keeps_the_link() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("dotfiles/config.toml");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        fs::write(&real, b"old").unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        atomic_write(&link, b"new").unwrap();

        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&real).unwrap(), b"new");
        assert_eq!(names_in(dir.path()), ["config.toml", "dotfiles"]);
        assert_eq!(names_in(real.parent().unwrap()), ["config.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_follows_a_chain_of_relative_symlinks() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("store")).unwrap();
        let real = dir.path().join("store/real.json");
        fs::write(&real, b"old").unwrap();
        std::os::unix::fs::symlink("store/real.json", dir.path().join("middle.json")).unwrap();
        std::os::unix::fs::symlink("middle.json", dir.path().join("config.json")).unwrap();

        atomic_write(&dir.path().join("config.json"), b"new").unwrap();

        assert_eq!(fs::read(&real).unwrap(), b"new");
        for link in ["middle.json", "config.json"] {
            let meta = fs::symlink_metadata(dir.path().join(link)).unwrap();
            assert!(meta.file_type().is_symlink(), "{link} stopped being a link");
        }
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_through_a_dangling_symlink_creates_the_file_it_points_at() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("store/config.toml");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink("store/config.toml", &link).unwrap();

        atomic_write(&link, b"fresh").unwrap();

        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&real).unwrap(), b"fresh");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_gives_a_new_file_mode_0600() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");

        atomic_write(&path, b"{}").unwrap();

        assert_eq!(file_mode(&path), Some(0o600));
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_the_exact_mode_of_an_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        for mode in [0o644, 0o664, 0o640] {
            let path = dir.path().join(format!("config-{mode:o}.toml"));
            fs::write(&path, b"old").unwrap();
            set_file_mode(&path, mode).unwrap();

            atomic_write(&path, b"new").unwrap();

            assert_eq!(file_mode(&path), Some(mode), "mode {mode:o} was not kept");
            assert_eq!(fs::read(&path).unwrap(), b"new");
        }
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_with_an_explicit_mode_wins_over_the_existing_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let existing = dir.path().join("settings.json");
        fs::write(&existing, b"old").unwrap();
        set_file_mode(&existing, 0o644).unwrap();
        let created = dir.path().join("created.json");

        atomic_write_with(&existing, b"new", AtomicOpts { mode: Some(0o600) }).unwrap();
        atomic_write_with(&created, b"new", AtomicOpts { mode: Some(0o640) }).unwrap();

        assert_eq!(file_mode(&existing), Some(0o600));
        assert_eq!(file_mode(&created), Some(0o640));
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_to_a_symlink_keeps_the_mode_of_the_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("real.toml");
        fs::write(&real, b"old").unwrap();
        set_file_mode(&real, 0o640).unwrap();
        let link = dir.path().join("link.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        atomic_write(&link, b"new").unwrap();

        assert_eq!(file_mode(&real), Some(0o640));
    }

    #[test]
    fn a_failed_atomic_write_leaves_neither_a_temp_file_nor_a_changed_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A directory cannot be replaced by a file, so the rename at the very end fails
        // after the temp file has been fully written.
        let target = dir.path().join("config.toml");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep.txt"), b"keep").unwrap();

        let result = atomic_write(&target, b"new");

        assert!(result.is_err(), "replacing a directory must fail");
        assert_eq!(names_in(dir.path()), ["config.toml"]);
        assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"keep");
    }

    #[test]
    fn a_failed_atomic_write_with_a_mode_leaves_no_temp_file_either() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("settings.json");
        fs::create_dir(&target).unwrap();

        let result = atomic_write_with(&target, b"new", AtomicOpts { mode: Some(0o600) });

        assert!(result.is_err());
        assert_eq!(names_in(dir.path()), ["settings.json"]);
    }

    #[test]
    fn atomic_write_rejects_a_path_without_a_file_name() {
        let result = atomic_write(Path::new("/"), b"x");

        assert!(result.is_err());
    }

    #[test]
    fn retry_transient_succeeds_once_the_failure_clears() {
        let mut calls = 0;
        let result = retry_transient(
            || {
                calls += 1;
                if calls < 4 {
                    Err(std::io::Error::from_raw_os_error(32))
                } else {
                    Ok("done")
                }
            },
            |e| e.raw_os_error() == Some(32),
            &[std::time::Duration::ZERO; 9],
        );

        assert_eq!(result.unwrap(), "done");
        assert_eq!(calls, 4);
    }

    #[test]
    fn retry_transient_gives_up_after_the_backoff_is_used_up() {
        let mut calls = 0;
        let result: std::io::Result<()> = retry_transient(
            || {
                calls += 1;
                Err(std::io::Error::from_raw_os_error(32))
            },
            |e| e.raw_os_error() == Some(32),
            &[std::time::Duration::ZERO; 9],
        );

        assert_eq!(result.unwrap_err().raw_os_error(), Some(32));
        assert_eq!(calls, 10, "one try plus one per backoff step");
    }

    #[test]
    fn retry_transient_returns_other_errors_at_once() {
        let mut calls = 0;
        let result: std::io::Result<()> = retry_transient(
            || {
                calls += 1;
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            },
            |e| e.kind() == std::io::ErrorKind::NotFound,
            &[std::time::Duration::ZERO; 9],
        );

        assert!(result.is_err());
        assert_eq!(calls, 1);
    }

    /// 临时设置 `CC_SWITCH_TEST_HOME`，Drop 时还原。
    ///
    /// 环境变量是进程全局的，不还原会污染同一进程里的其他测试；配合 `#[serial]`
    /// 才能保证并发跑测试时不互相踩。
    struct TestHomeGuard(Option<std::ffi::OsString>);

    impl TestHomeGuard {
        fn set(value: &str) -> Self {
            let previous = std::env::var_os("CC_SWITCH_TEST_HOME");
            std::env::set_var("CC_SWITCH_TEST_HOME", value);
            Self(previous)
        }

        fn unset() -> Self {
            let previous = std::env::var_os("CC_SWITCH_TEST_HOME");
            std::env::remove_var("CC_SWITCH_TEST_HOME");
            Self(previous)
        }
    }

    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
                None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
            }
        }
    }

    #[test]
    #[serial]
    fn test_home_override_takes_precedence_over_real_home() {
        let _guard = TestHomeGuard::set("/tmp/ofox-fake-home");

        assert_eq!(get_home_dir(), PathBuf::from("/tmp/ofox-fake-home"));
    }

    #[test]
    #[serial]
    fn test_home_override_is_trimmed() {
        let _guard = TestHomeGuard::set("  /tmp/ofox-fake-home  ");

        assert_eq!(get_home_dir(), PathBuf::from("/tmp/ofox-fake-home"));
    }

    #[test]
    #[serial]
    fn blank_test_home_override_is_treated_as_unset() {
        let _guard = TestHomeGuard::set("   ");
        let blank = get_home_dir();

        drop(_guard);
        let _unset = TestHomeGuard::unset();
        assert_eq!(blank, get_home_dir());
        assert_ne!(blank, PathBuf::from("   "));
    }

    #[test]
    #[serial]
    fn unit_tests_without_an_override_never_resolve_to_the_real_home() {
        let _guard = TestHomeGuard::unset();

        let home = get_home_dir();

        // `dirs::home_dir()` 是真实 home；单元测试必须落在别处的一次性目录。
        assert_ne!(Some(home.clone()), dirs::home_dir());
        assert!(
            home.starts_with(std::env::temp_dir()),
            "{} is not under the temp dir",
            home.display()
        );
        assert!(home.is_dir());
        assert_eq!(
            home,
            get_home_dir(),
            "the process test home is created once"
        );
        assert_eq!(try_get_home_dir(), Some(home));
    }

    /// Windows 上 `CC_SWITCH_TEST_HOME` 会是 `C:\Users\Alice` 这种形态。这里用
    /// `PathBuf::join` 而不是硬编码分隔符做断言，所以同一个用例在 macOS 和
    /// Windows 上都成立——等 Windows CI 跑起来时，它能直接当回归保护网。
    #[test]
    #[serial]
    fn windows_style_home_is_preserved_when_joining() {
        let _guard = TestHomeGuard::set(r"C:\Users\Alice");

        assert_eq!(get_home_dir(), PathBuf::from(r"C:\Users\Alice"));
        assert_eq!(
            get_default_claude_mcp_path(),
            PathBuf::from(r"C:\Users\Alice").join(".claude.json")
        );
    }

    #[test]
    fn derive_mcp_path_from_override_preserves_folder_name() {
        let override_dir = PathBuf::from("/tmp/profile/.claude");
        let derived = derive_mcp_path_from_override(&override_dir)
            .expect("should derive path for nested dir");
        assert_eq!(derived, PathBuf::from("/tmp/profile/.claude.json"));
    }

    #[test]
    fn derive_mcp_path_from_override_handles_non_hidden_folder() {
        let override_dir = PathBuf::from("/data/claude-config");
        let derived = derive_mcp_path_from_override(&override_dir)
            .expect("should derive path for standard dir");
        assert_eq!(derived, PathBuf::from("/data/claude-config.json"));
    }

    #[test]
    fn derive_mcp_path_from_override_supports_relative_rootless_dir() {
        let override_dir = PathBuf::from("claude");
        let derived = derive_mcp_path_from_override(&override_dir)
            .expect("should derive path for single segment");
        assert_eq!(derived, PathBuf::from("claude.json"));
    }

    #[test]
    fn derive_mcp_path_from_root_like_dir_returns_none() {
        let override_dir = PathBuf::from("/");
        assert!(derive_mcp_path_from_override(&override_dir).is_none());
    }
}

/// 复制文件
pub fn copy_file(from: &Path, to: &Path) -> Result<(), AppError> {
    fs::copy(from, to).map_err(|e| AppError::IoContext {
        context: format!("复制文件失败 ({} -> {})", from.display(), to.display()),
        source: e,
    })?;
    Ok(())
}

/// 删除文件
pub fn delete_file(path: &Path) -> Result<(), AppError> {
    if path.exists() {
        fs::remove_file(path).map_err(|e| AppError::io(path, e))?;
    }
    Ok(())
}

/// 检查 Claude Code 配置状态
#[derive(Serialize, Deserialize)]
pub struct ConfigStatus {
    pub exists: bool,
    pub path: String,
}

/// 获取 Claude Code 配置状态
pub fn get_claude_config_status() -> ConfigStatus {
    let path = get_claude_settings_path();
    ConfigStatus {
        exists: path.exists(),
        path: path.to_string_lossy().to_string(),
    }
}
