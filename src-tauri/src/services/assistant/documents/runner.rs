//! Runs a bundled document engine as an isolated child process: a clean
//! environment, a private home, bounded output and a deadline.
use crate::services::assistant::process::{self, Child};
use std::{io, path::Path, time::Duration};
use tokio::task::JoinHandle;

/// How long one engine run may take before it is killed.
pub const ENGINE_TIMEOUT: Duration = Duration::from_secs(60);

/// Each of stdout and stderr keeps at most this many bytes.
pub const OUTPUT_LIMIT: usize = 256 * 1024;

/// The same for a run that prints one slide as SVG (`view svg`): officecli
/// inlines the slide's pictures (plots, figures) as base64, so a slide with
/// a few of them is several times [`OUTPUT_LIMIT`].
pub const SVG_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

/// Once the engine has exited or been killed, how long its pipes may stay
/// open (a process it left behind can hold them) before the output is given up.
const PIPE_GRACE: Duration = Duration::from_secs(2);

/// Where an engine's web requests go: a port nothing listens on.
pub const DEAD_PROXY: &str = "http://127.0.0.1:9";

/// The folder of `home` where Typst looks for packages and caches them.
pub const TYPST_PACKAGES: &str = "typst-packages";

#[derive(Debug)]
pub struct EngineOutput {
    /// `None` when the engine was killed, by the deadline or by a signal.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Run `exe` with `args` in `cwd`. The engine sees none of the app's
/// environment: `home` is its home and temp folder, it neither updates itself
/// nor leaves a resident process, it finds Typst packages only in an empty
/// folder of `home`, and its web requests go to [`DEAD_PROXY`]. Each of its
/// stdout and stderr keeps at most [`OUTPUT_LIMIT`] bytes.
pub async fn run_engine(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    home: &Path,
    timeout: Duration,
) -> Result<EngineOutput, String> {
    run_engine_limited(exe, args, cwd, home, timeout, OUTPUT_LIMIT).await
}

/// [`run_engine`], keeping at most `limit` bytes of each of stdout and
/// stderr.
pub async fn run_engine_limited(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    home: &Path,
    timeout: Duration,
    limit: usize,
) -> Result<EngineOutput, String> {
    let packages = home.join(TYPST_PACKAGES);
    std::fs::create_dir_all(&packages).map_err(|_| "assistant_process_start_failed")?;
    let mut command = tokio::process::Command::new(exe);
    command.args(args);
    process::clean_environment(&mut command);
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("TMPDIR", home)
        .env("TEMP", home)
        .env("TMP", home)
        .env("OFFICECLI_NO_AUTO_RESIDENT", "1")
        .env("OFFICECLI_SKIP_UPDATE", "1")
        .env("TYPST_PACKAGE_PATH", &packages)
        .env("TYPST_PACKAGE_CACHE_PATH", &packages);
    // No engine reaches the network: every request goes to a proxy that is
    // not there.
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env(name, DEAD_PROXY);
    }
    command.env("NO_PROXY", "").env("no_proxy", "");
    let mut child = Child::spawn(command)?;
    drop(child.take_stdin());
    let stdout = child
        .take_stdout()
        .ok_or("assistant_command_outcome_unknown")?;
    let stderr = child
        .take_stderr()
        .ok_or("assistant_command_outcome_unknown")?;
    let stdout = tokio::spawn(process::bounded_output(stdout, limit));
    let stderr = tokio::spawn(process::bounded_output(stderr, limit));
    let (status, timed_out) = match tokio::time::timeout(timeout, child.child.wait()).await {
        Ok(status) => (
            status
                .map_err(|_| "assistant_command_outcome_unknown")?
                .code(),
            false,
        ),
        Err(_) => {
            child.stop().await;
            (None, true)
        }
    };
    let (stdout, stderr) = tokio::join!(drained(stdout), drained(stderr));
    // Output that cannot be read after a normal exit leaves the outcome
    // unknown; after a kill it only means the engine said less.
    let (stdout, stderr) = if timed_out {
        (stdout.unwrap_or_default(), stderr.unwrap_or_default())
    } else {
        (
            stdout.ok_or("assistant_command_outcome_unknown")?,
            stderr.ok_or("assistant_command_outcome_unknown")?,
        )
    };
    Ok(EngineOutput {
        status,
        stdout: text(&stdout, limit),
        stderr: text(&stderr, limit),
        timed_out,
    })
}

/// What a pipe reader read, if its pipe closed in time.
async fn drained(mut reader: JoinHandle<io::Result<Vec<u8>>>) -> Option<Vec<u8>> {
    match tokio::time::timeout(PIPE_GRACE, &mut reader).await {
        Ok(Ok(Ok(bytes))) => Some(bytes),
        Ok(_) => None,
        Err(_) => {
            reader.abort();
            None
        }
    }
}

/// UTF-8 text of at most `limit` bytes: a character cut by the limit
/// becomes a replacement character, which is longer than what it replaces.
fn text(bytes: &[u8], limit: usize) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if text.len() > limit {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::time::Instant;

    #[test]
    fn text_never_exceeds_the_limit_in_bytes() {
        // The limit cuts the two-byte "é": its first byte alone would become
        // a three-byte replacement character.
        let mut bytes = vec![b'a'; OUTPUT_LIMIT - 1];
        bytes.extend("é".as_bytes());
        bytes.truncate(OUTPUT_LIMIT);
        let text = text(&bytes, OUTPUT_LIMIT);
        assert!(text.len() <= OUTPUT_LIMIT);
        assert!(text.chars().all(|c| c == 'a'));
    }

    /// A stand-in engine: `script` is the shell body of an executable.
    #[cfg(unix)]
    fn engine(dir: &Path, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("engine");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    fn folders() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        let home = root.path().join("home");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        (root, work, home)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_engine_passes_isolated_environment() {
        let (root, work, home) = folders();
        std::env::set_var("OFOX_RUNNER_TEST_SECRET", "must-not-reach-the-engine");
        let exe = engine(root.path(), r#"env > "$1""#);
        let dump = root.path().join("env.txt");
        let output = run_engine(
            &exe,
            &[dump.to_str().unwrap()],
            &work,
            &home,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert_eq!(output.status, Some(0));
        let env = std::fs::read_to_string(&dump).unwrap();
        let lines: Vec<&str> = env.lines().collect();
        let home_text = home.display().to_string();
        for name in ["HOME", "USERPROFILE", "TMPDIR", "TEMP", "TMP"] {
            assert!(
                lines.contains(&format!("{name}={home_text}").as_str()),
                "{name}"
            );
        }
        let packages = format!("{home_text}/typst-packages");
        assert!(lines.contains(&format!("TYPST_PACKAGE_PATH={packages}").as_str()));
        assert!(lines.contains(&format!("TYPST_PACKAGE_CACHE_PATH={packages}").as_str()));
        assert!(home.join("typst-packages").is_dir());
        assert!(lines.contains(&"OFFICECLI_SKIP_UPDATE=1"));
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            assert!(
                lines.contains(&format!("{name}=http://127.0.0.1:9").as_str()),
                "{name}"
            );
        }
        assert!(lines.contains(&"NO_PROXY=") && lines.contains(&"no_proxy="));
        assert!(lines.contains(&"OFFICECLI_NO_AUTO_RESIDENT=1"));
        assert!(!lines.iter().any(|line| line.starts_with("OFOX_")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_engine_reports_status_streams_and_folder() {
        let (root, work, home) = folders();
        let exe = engine(root.path(), "pwd; echo failed >&2; exit 3");
        let output = run_engine(&exe, &[], &work, &home, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(output.status, Some(3));
        assert!(!output.timed_out);
        assert_eq!(output.stderr, "failed\n");
        let reported = PathBuf::from(output.stdout.trim());
        assert_eq!(
            reported.canonicalize().unwrap(),
            work.canonicalize().unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_engine_bounds_output() {
        let (root, work, home) = folders();
        let exe = engine(
            root.path(),
            "head -c 1048576 /dev/zero | tr '\\0' 'a'\nhead -c 1048576 /dev/zero | tr '\\0' 'b' >&2",
        );
        let output = run_engine(&exe, &[], &work, &home, Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(output.status, Some(0));
        assert!(output.stdout.len() <= 256 * 1024);
        assert!(output.stderr.len() <= 256 * 1024);
        assert!(output.stdout.starts_with("aaaa"));
        assert!(output.stderr.starts_with("bbbb"));
    }

    #[cfg(unix)]
    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_engine_times_out_and_kills() {
        let (root, work, home) = folders();
        let exe = engine(root.path(), "sleep 30");
        let started = Instant::now();
        let output = run_engine(&exe, &[], &work, &home, Duration::from_secs(1))
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(output.timed_out);
        assert_eq!(output.status, None);
    }

    /// A deadline that kills the engine kills what it started too, and keeps
    /// what it had printed. macOS checks a new executable the first time it
    /// runs, which can take seconds while a test run writes many of them, so
    /// the script runs once before its deadline starts.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_engine_kills_what_the_engine_started_and_keeps_its_output() {
        let (root, work, home) = folders();
        let pids = root.path().join("pids.txt");
        let exe = engine(
            root.path(),
            r#"[ "$1" = warm ] && exit 0; echo partial; echo $$ > "$1"; sleep 30 & echo $! >> "$1"; wait"#,
        );
        assert!(std::process::Command::new(&exe)
            .arg("warm")
            .status()
            .unwrap()
            .success());
        let output = run_engine(
            &exe,
            &[pids.to_str().unwrap()],
            &work,
            &home,
            Duration::from_secs(3),
        )
        .await
        .unwrap();
        assert!(output.timed_out);
        assert_eq!(output.stdout, "partial\n");
        let pids: Vec<i32> = std::fs::read_to_string(&pids)
            .unwrap()
            .lines()
            .map(|line| line.parse().unwrap())
            .collect();
        assert_eq!(pids.len(), 2);
        for pid in pids {
            for _ in 0..50 {
                if !alive(pid) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert!(!alive(pid), "process {pid} survived the timeout");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_engine_fails_when_the_engine_cannot_start() {
        let (root, work, home) = folders();
        let missing = root.path().join("missing");
        let error = run_engine(&missing, &[], &work, &home, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert_eq!(error, "assistant_process_start_failed");
    }
}
