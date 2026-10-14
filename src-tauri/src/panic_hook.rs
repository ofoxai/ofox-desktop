//! Panic Hook 模块
//!
//! 在应用崩溃时捕获 panic 信息并记录到 `<app_config_dir>/crash.log` 文件中（默认 `~/.ofox-desktop/crash.log`）。
//! 便于用户和开发者诊断闪退问题。

use std::fs::OpenOptions;
use std::io::Write;
use std::panic;
use std::path::PathBuf;
use std::sync::OnceLock;

/// 应用版本号（从 Cargo.toml 读取）
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

static APP_CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn init_app_config_dir(dir: PathBuf) {
    let _ = APP_CONFIG_DIR.set(dir);
}

/// 获取默认应用配置目录（不会 panic）
fn default_app_config_dir() -> PathBuf {
    crate::config::get_home_dir().join(".ofox-desktop")
}

/// 获取应用配置目录（优先使用初始化时写入的值；不会 panic）
fn get_app_config_dir() -> PathBuf {
    APP_CONFIG_DIR
        .get()
        .cloned()
        .unwrap_or_else(default_app_config_dir)
}

/// 获取崩溃日志文件路径
fn get_crash_log_path() -> PathBuf {
    get_app_config_dir().join("crash.log")
}

/// 获取日志目录路径
pub fn get_log_dir() -> PathBuf {
    get_app_config_dir().join("logs")
}

/// 安全获取环境信息（不会 panic）
fn get_system_info() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let family = std::env::consts::FAMILY;

    // 安全获取当前工作目录
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // 安全获取当前线程信息
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    let thread_id = format!("{:?}", thread.id());

    format!(
        "OS: {os} ({family})\n\
         Arch: {arch}\n\
         App Version: {APP_VERSION}\n\
         Working Dir: {cwd}\n\
         Thread: {thread_name} (ID: {thread_id})"
    )
}

/// 把一条崩溃记录追加到崩溃日志。
fn append_crash_entry(log_path: &std::path::Path, entry: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    file.write_all(entry.as_bytes())?;
    file.flush()
}

/// 拼出一条崩溃记录。panic 消息里可能带着 key（比如对一个含 key 的 `Err` 做 `unwrap`），
/// 系统信息里也可能有，这两段先脱敏。位置和调用栈**不脱敏**：函数名、文件名里常有
/// `secret` / `token`（`ofox_secret::KeychainStore::load`，`src/ofox_secret.rs:44:9`），
/// 按键值规则处理会把帧抹成 `ofox_secret:«redacted»`，崩溃日志就没法用了；它们是代码
/// 位置，不带运行时数据。
fn format_crash_entry(
    timestamp: &str,
    system_info: &str,
    message: &str,
    location: &str,
    backtrace: &str,
) -> String {
    let separator = "=".repeat(80);
    let sub_separator = "-".repeat(40);
    let system_info = crate::redact::scrub(system_info);
    let message = crate::redact::scrub(message);
    format!(
        r#"
{separator}
[CRASH REPORT] {timestamp}
{separator}

{sub_separator}
System Information
{sub_separator}
{system_info}

{sub_separator}
Error Details
{sub_separator}
Message: {message}

Location: {location}

{sub_separator}
Stack Trace (Backtrace)
{sub_separator}
{backtrace}

{separator}
"#
    )
}

/// 设置 panic hook，捕获崩溃信息并写入日志文件
///
/// 在应用启动时调用此函数，确保任何 panic 都会被记录。
/// 日志格式包含：
/// - 时间戳
/// - 应用版本和系统信息
/// - Panic 信息
/// - 发生位置（文件:行号）
/// - Backtrace（完整调用栈）
pub fn setup_panic_hook() {
    // 启用 backtrace（确保 release 模式也能捕获）
    if std::env::var("RUST_BACKTRACE").is_err() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    let default_hook = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        let log_path = get_crash_log_path();

        // 确保目录存在
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // 构建崩溃信息（使用 catch_unwind 保护时间格式化，避免嵌套 panic）
        let timestamp = std::panic::catch_unwind(|| {
            chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S%.3f")
                .to_string()
        })
        .unwrap_or_else(|_| {
            // chrono panic 时回退到 unix timestamp
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| format!("unix:{}.{:03}", d.as_secs(), d.subsec_millis()))
                .unwrap_or_else(|_| "unknown".to_string())
        });

        // 获取系统信息
        let system_info = std::panic::catch_unwind(get_system_info)
            .unwrap_or_else(|_| "Failed to get system info".to_string());

        // 获取 panic 消息（尝试多种方式提取）
        let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            // 尝试使用 Display trait
            format!("{panic_info}")
        };

        // 获取位置信息
        let location = if let Some(loc) = panic_info.location() {
            format!(
                "File: {}\n         Line: {}\n         Column: {}",
                loc.file(),
                loc.line(),
                loc.column()
            )
        } else {
            "Unknown location".to_string()
        };

        // 捕获 backtrace（完整调用栈）
        let backtrace = std::backtrace::Backtrace::force_capture();
        let backtrace_str = format!("{backtrace}");

        // 格式化日志条目
        let crash_entry = format_crash_entry(
            &timestamp,
            &system_info,
            &message,
            &location,
            &backtrace_str,
        );

        // 写入文件（追加模式）
        if append_crash_entry(&log_path, &crash_entry).is_ok() {
            // 记录日志文件位置到 stderr
            eprintln!("\n[Ofox-Switch] Crash log saved to: {}", log_path.display());
        }

        // 同时输出到 stderr（便于开发调试）
        eprintln!("{crash_entry}");

        // 调用默认 hook
        default_hook(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crash_log_path() {
        let path = get_crash_log_path();
        assert!(path.ends_with("crash.log"));
        assert!(path.to_string_lossy().contains(".ofox-desktop"));
    }

    const FRAMES: &str = "   7: ofox_desktop::ofox_secret::KeychainStore::load\n             at ./src/ofox_secret.rs:44:9\n   8: ofox_desktop::ofox_auth::token_refresh::run\n             at ./src/ofox_auth.rs:210:17";

    fn entry(message: &str, location: &str, backtrace: &str) -> String {
        format_crash_entry(
            "2026-10-14 10:00:00.000",
            "OS: macos (unix)\nWorking Dir: /tmp/work",
            message,
            location,
            backtrace,
        )
    }

    #[test]
    fn crash_entry_never_holds_a_key_from_the_panic_message() {
        let written = entry(
            "called `Result::unwrap()` on an `Err` value: \"401 for Bearer abcdefgh12345678 (sk-of-abc123456789)\"",
            "File: src/x.rs",
            FRAMES,
        );

        assert!(!written.contains("abc123456789"), "{written}");
        assert!(!written.contains("abcdefgh12345678"), "{written}");
        assert!(written.contains("(«redacted»)"), "{written}");
    }

    #[test]
    fn crash_entry_scrubs_the_system_info() {
        let written = format_crash_entry(
            "2026-10-14 10:00:00.000",
            "Working Dir: /home/u/sk-of-abc123456789",
            "boom",
            "File: src/x.rs",
            FRAMES,
        );

        assert!(!written.contains("abc123456789"), "{written}");
    }

    #[test]
    fn crash_entry_keeps_the_location_and_backtrace_frames_intact() {
        let location = "File: src/ofox_secret.rs\n         Line: 44\n         Column: 9";

        let written = entry("boom", location, FRAMES);

        assert!(written.contains(FRAMES), "{written}");
        assert!(written.contains(location), "{written}");
        assert!(written.contains("Message: boom"), "{written}");
    }

    #[test]
    fn crash_log_appends_entries_as_given() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("crash.log");

        append_crash_entry(&log_path, "first\n").unwrap();
        append_crash_entry(&log_path, "second\n").unwrap();

        assert_eq!(
            std::fs::read_to_string(&log_path).unwrap(),
            "first\nsecond\n"
        );
    }

    #[test]
    fn test_system_info() {
        let info = get_system_info();
        assert!(info.contains("OS:"));
        assert!(info.contains("Arch:"));
        assert!(info.contains("App Version:"));
    }
}
