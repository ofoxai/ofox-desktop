use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::AppError;

/// 各 agent CLI 用来改写自己配置目录的环境变量。
///
/// 测试沙箱会清空这些变量（见 `tests/support.rs` 的 `sandbox()` 与 crate 内的
/// `test_support::sandbox()`）：开发者 shell 里残留的任何一个都会让测试读写真实 agent
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

/// 原子写入：写入临时文件后 rename 替换，避免半写状态
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let parent = path
        .parent()
        .ok_or_else(|| AppError::Config("无效的路径".to_string()))?;
    let mut tmp = parent.to_path_buf();
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Config("无效的文件名".to_string()))?
        .to_string_lossy()
        .to_string();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    tmp.push(format!("{file_name}.tmp.{ts}"));

    {
        let mut f = fs::File::create(&tmp).map_err(|e| AppError::io(&tmp, e))?;
        f.write_all(data).map_err(|e| AppError::io(&tmp, e))?;
        f.flush().map_err(|e| AppError::io(&tmp, e))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let perm = meta.permissions().mode();
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(perm));
        }
    }

    #[cfg(windows)]
    {
        // Windows 上 rename 目标存在会失败，先移除再重命名（尽量接近原子性）
        if path.exists() {
            let _ = fs::remove_file(path);
        }
        fs::rename(&tmp, path).map_err(|e| AppError::IoContext {
            context: format!("原子替换失败: {} -> {}", tmp.display(), path.display()),
            source: e,
        })?;
    }

    #[cfg(not(windows))]
    {
        fs::rename(&tmp, path).map_err(|e| AppError::IoContext {
            context: format!("原子替换失败: {} -> {}", tmp.display(), path.display()),
            source: e,
        })?;
    }
    Ok(())
}

/// 读文件原始字节；文件不存在返回 `None`。
pub(crate) fn read_file_bytes(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AppError::io(path, e)),
    }
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
                Some(bytes) => atomic_write(&path, &bytes).and_then(|()| match mode {
                    Some(mode) => set_file_mode(&path, mode),
                    None => Ok(()),
                }),
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
