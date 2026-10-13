//! 已装的 Codex 认不认 `disable_response_storage`。
//!
//! 这个键是 Codex 给零数据保留（ZDR）账号用的开关：老版本认它，没写的话 ZDR 账号会报
//! 「Previous response cannot be used」；新版本（0.162 起）已经不认，每次启动都警告
//! 「忽略了无法识别的设置」。我们不靠版本号猜，直接问 Codex 自己：在临时目录里放一份只有
//! 这个键的配置，用 `codex app-server --strict-config` 启动，遇到不认识的字段它会立刻报错。
//!
//! 只有明确查出「不认」才不写、才清理；认、查不出（没装、版本太老没有这个开关、超时之外
//! 的别的错误）都照旧写，保证老版本的 ZDR 用户不受影响。

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) const FLAG: &str = "disable_response_storage";
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const CACHE_FOR: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlagSupport {
    /// Codex 认这个键。
    Known,
    /// Codex 明确不认这个键。
    Unknown,
    /// 没装 Codex、或查不出来。
    Undetermined,
}

/// 一次探测的结果，和怎么得到它无关，方便单测。
#[derive(Debug)]
enum Outcome {
    /// 严格模式下正常启动。
    Accepted,
    /// 5 秒内没退出：配置已经加载完，在等输入。
    TimedOut,
    /// 退出失败，附带它的错误输出。
    Rejected(String),
    /// 连 Codex 都没跑起来。
    CouldNotRun,
}

fn classify(outcome: &Outcome) -> FlagSupport {
    match outcome {
        Outcome::Accepted | Outcome::TimedOut => FlagSupport::Known,
        Outcome::Rejected(stderr)
            if stderr.contains("unknown configuration field") && stderr.contains(FLAG) =>
        {
            FlagSupport::Unknown
        }
        // 别的错误（没有 --strict-config、没有 app-server 子命令……）说明版本很老，不能断言。
        Outcome::Rejected(_) | Outcome::CouldNotRun => FlagSupport::Undetermined,
    }
}

/// 要试的 Codex，按顺序：PATH 里的，再是 macOS 上 ChatGPT App 自带的。
fn candidates() -> Vec<Command> {
    let mut list = Vec::new();
    if cfg!(windows) {
        // npm 装的是 `codex.cmd`，`Command::new("codex")` 找不到，要经过 cmd。
        let mut command = Command::new("cmd");
        command.args(["/C", "codex"]);
        list.push(command);
    } else {
        list.push(Command::new("codex"));
    }
    if cfg!(target_os = "macos") {
        let bundled =
            PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex");
        if bundled.exists() {
            list.push(Command::new(bundled));
        }
    }
    list
}

fn run(mut command: Command, home: &std::path::Path) -> Outcome {
    let spawned = command
        .args(["app-server", "--strict-config"])
        .env("CODEX_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = spawned else {
        return Outcome::CouldNotRun;
    };
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Outcome::Accepted,
            Ok(Some(_)) => {
                let mut message = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    use std::io::Read;
                    let _ = stderr.read_to_string(&mut message);
                }
                return Outcome::Rejected(message);
            }
            Ok(None) if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::TimedOut;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return Outcome::CouldNotRun,
        }
    }
}

/// 现场探测一次（不看缓存）。
pub(crate) fn probe_now() -> FlagSupport {
    let Ok(dir) = tempfile::Builder::new()
        .prefix("ofox-codex-probe")
        .tempdir()
    else {
        return FlagSupport::Undetermined;
    };
    if std::fs::write(dir.path().join("config.toml"), format!("{FLAG} = true\n")).is_err() {
        return FlagSupport::Undetermined;
    }
    for command in candidates() {
        match classify(&run(command, dir.path())) {
            // 这个 Codex 没法判断，换下一个。
            FlagSupport::Undetermined => continue,
            decided => return decided,
        }
    }
    FlagSupport::Undetermined
}

static CACHE: Mutex<Option<(Instant, FlagSupport)>> = Mutex::new(None);

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<FlagSupport>> = const { std::cell::Cell::new(None) };
}

/// 测试里默认不去跑真的 Codex：当作「查不出」，保持旧行为。开关只对当前测试线程生效，
/// 并行的别的测试不受影响。
#[cfg(test)]
pub(crate) fn set_override(support: Option<FlagSupport>) {
    OVERRIDE.with(|cell| cell.set(support));
}

/// 测试里不跑真的 Codex：默认当作「查不出」（保持旧行为），除非测试自己指定。
#[cfg(test)]
fn forced() -> Option<FlagSupport> {
    Some(
        OVERRIDE
            .with(|cell| cell.get())
            .unwrap_or(FlagSupport::Undetermined),
    )
}

#[cfg(not(test))]
fn forced() -> Option<FlagSupport> {
    None
}

pub(crate) fn flag_support() -> FlagSupport {
    if let Some(forced) = forced() {
        return forced;
    }
    if let Some((at, support)) = *CACHE.lock().unwrap() {
        if at.elapsed() < CACHE_FOR {
            return support;
        }
    }
    let support = probe_now();
    *CACHE.lock().unwrap() = Some((Instant::now(), support));
    support
}

/// 要不要把这个键写进配置：只有 Codex 明确不认时才不写。
pub(crate) fn should_write_flag() -> bool {
    flag_support() != FlagSupport::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejected(text: &str) -> Outcome {
        Outcome::Rejected(text.to_string())
    }

    #[test]
    fn a_codex_that_starts_or_waits_knows_the_flag() {
        assert_eq!(classify(&Outcome::Accepted), FlagSupport::Known);
        assert_eq!(classify(&Outcome::TimedOut), FlagSupport::Known);
    }

    #[test]
    fn only_an_unknown_field_error_naming_the_flag_means_codex_ignores_it() {
        let text =
            "Error: /tmp/x/config.toml:1:1: unknown configuration field `disable_response_storage`";
        assert_eq!(classify(&rejected(text)), FlagSupport::Unknown);
    }

    #[test]
    fn any_other_failure_cannot_be_told_apart_and_keeps_the_old_behavior() {
        let other_field = "Error: x:1:1: unknown configuration field `something_else`";
        assert_eq!(classify(&rejected(other_field)), FlagSupport::Undetermined);
        let old_codex = "error: unexpected argument '--strict-config' found";
        assert_eq!(classify(&rejected(old_codex)), FlagSupport::Undetermined);
        assert_eq!(classify(&rejected("")), FlagSupport::Undetermined);
        assert_eq!(classify(&Outcome::CouldNotRun), FlagSupport::Undetermined);
    }

    #[test]
    fn the_flag_is_written_unless_codex_is_known_to_ignore_it() {
        for (support, expected) in [
            (FlagSupport::Known, true),
            (FlagSupport::Undetermined, true),
            (FlagSupport::Unknown, false),
        ] {
            set_override(Some(support));
            assert_eq!(should_write_flag(), expected, "{support:?}");
        }
        set_override(None);
        assert!(should_write_flag(), "tests default to the old behavior");
    }
}
