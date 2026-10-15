//! One owner retains the process group even when its IPC waiter disappears.
use super::Failure;
#[cfg(unix)]
use std::process::Stdio;
use std::time::Duration;
use tokio::{process::Command, sync::watch};

#[cfg(unix)]
struct OwnedGroup {
    child: tokio::process::Child,
    pid: i32,
}
#[cfg(unix)]
impl OwnedGroup {
    fn signal(&self) -> Result<(), Failure> {
        if self.pid == 0 {
            return Ok(());
        }
        if unsafe { libc::kill(-self.pid, libc::SIGKILL) } == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            Ok(())
        } else {
            Err(Failure::Cleanup)
        }
    }
    async fn stop(&mut self) -> Result<(), Failure> {
        let mut signal = self.signal();
        let waited = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        // Darwin can return EPERM for a group containing only an unreaped zombie.
        // Reaping our child then retrying distinguishes it from a denied live group.
        if signal.is_err() && matches!(waited, Ok(Ok(_))) {
            signal = self.signal();
        }
        if signal.is_ok() {
            self.pid = 0;
        }
        signal?;
        waited
            .map_err(|_| Failure::Cleanup)?
            .map_err(|_| Failure::Cleanup)?;
        Ok(())
    }
}
#[cfg(unix)]
impl Drop for OwnedGroup {
    fn drop(&mut self) {
        let _ = self.signal();
        let _ = self.child.start_kill();
    }
}
#[cfg(unix)]
pub(super) async fn run(
    command: Command,
    cancel: watch::Receiver<bool>,
    timeout: Duration,
) -> Result<Vec<u8>, Failure> {
    run_guarded(command, cancel, timeout, &std::sync::Mutex::new(())).await
}
#[cfg(unix)]
pub(super) async fn run_guarded(
    command: Command,
    cancel: watch::Receiver<bool>,
    timeout: Duration,
    gate: &std::sync::Mutex<()>,
) -> Result<Vec<u8>, Failure> {
    run_checked(command, cancel, timeout, gate, || Ok(())).await
}
#[cfg(unix)]
pub(super) async fn run_checked(
    mut command: Command,
    mut cancel: watch::Receiver<bool>,
    timeout: Duration,
    gate: &std::sync::Mutex<()>,
    check: impl FnOnce() -> Result<(), Failure> + Send,
) -> Result<Vec<u8>, Failure> {
    use tokio::io::AsyncReadExt;
    if *cancel.borrow() {
        return Err(Failure::Cancelled);
    }
    command
        .process_group(0)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = {
        let _gate = gate.lock().map_err(|_| Failure::Changed)?;
        if *cancel.borrow() {
            return Err(Failure::Cancelled);
        }
        check()?;
        command.spawn().map_err(|_| Failure::Process)?
    };
    let pid = child.id().ok_or(Failure::Process)? as i32;
    let mut owned = OwnedGroup { child, pid };
    let mut stdout = owned.child.stdout.take().ok_or(Failure::Process)?;
    let mut capture = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let count = stdout
                .read(&mut buffer)
                .await
                .map_err(|_| Failure::Process)?;
            if count == 0 {
                return Ok(bytes);
            }
            if bytes.len() + count > 65536 {
                return Err(Failure::Limit);
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
    });
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut output = None;
    let mut cancellation_open = true;
    let result = loop {
        tokio::select! {
            biased;
            changed=cancel.changed(), if cancellation_open => {
                if *cancel.borrow() { break Err(Failure::Cancelled); }
                // A dropped caller must not cancel its detached owner implicitly.
                if changed.is_err() { cancellation_open=false; }
            },
            _=&mut deadline => break Err(Failure::Timeout),
            status=owned.child.wait() => break status.map_err(|_|Failure::Process).and_then(|status|if status.success(){Ok(())}else{Err(Failure::Process)}),
            captured=&mut capture, if output.is_none() => {
                match captured {
                    Ok(Ok(bytes))=>output=Some(bytes),
                    Ok(Err(failure))=>{output=Some(vec![]);break Err(failure)},
                    Err(_)=>{output=Some(vec![]);break Err(Failure::Process)},
                }
            }
        }
    };
    let cleanup = owned.stop().await;
    let captured = if let Some(output) = output {
        Ok(output)
    } else {
        match tokio::time::timeout(Duration::from_secs(3), &mut capture).await {
            Ok(Ok(value)) => value,
            _ => {
                capture.abort();
                Err(Failure::Cleanup)
            }
        }
    };
    cleanup?;
    result?;
    captured
}
#[cfg(not(unix))]
pub(super) async fn run_checked(
    _command: Command,
    _cancel: watch::Receiver<bool>,
    _timeout: Duration,
    _gate: &std::sync::Mutex<()>,
    _check: impl FnOnce() -> Result<(), Failure> + Send,
) -> Result<Vec<u8>, Failure> {
    Err(Failure::Process)
}
#[cfg(not(unix))]
pub(super) async fn run_guarded(
    _command: Command,
    _cancel: watch::Receiver<bool>,
    _timeout: Duration,
    _gate: &std::sync::Mutex<()>,
) -> Result<Vec<u8>, Failure> {
    Err(Failure::Process)
}
#[cfg(not(unix))]
pub(super) async fn run(
    _command: Command,
    _cancel: watch::Receiver<bool>,
    _timeout: Duration,
) -> Result<Vec<u8>, Failure> {
    Err(Failure::Process)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn captures_bounded_output_and_reports_failure_without_stderr() {
        let (_sender, cancel) = watch::channel(false);
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf safe; printf private-secret >&2; exit 1"]);
        assert_eq!(
            run(command, cancel, Duration::from_secs(3)).await,
            Err(Failure::Process)
        );
        let (_sender, cancel) = watch::channel(false);
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "head -c 70000 /dev/zero"]);
        assert_eq!(
            run(command, cancel, Duration::from_secs(3)).await,
            Err(Failure::Limit)
        );
    }
    #[tokio::test]
    async fn a_dropped_cancel_sender_does_not_starve_completion_or_timeout() {
        let (sender, cancel) = watch::channel(false);
        drop(sender);
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep .02; printf finished"]);
        assert_eq!(
            run(command, cancel, Duration::from_secs(1)).await,
            Ok(b"finished".to_vec())
        );
        let (sender, cancel) = watch::channel(false);
        drop(sender);
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 10"]);
        assert_eq!(
            run(command, cancel, Duration::from_millis(30)).await,
            Err(Failure::Timeout)
        );
    }
    #[tokio::test]
    async fn normal_parent_exit_and_cancellation_both_kill_descendants_before_return() {
        for cancel_early in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let sentinel = dir.path().join("descendant");
            let (sender, cancel) = watch::channel(false);
            let mut command = Command::new("/bin/sh");
            command
                .args([
                    "-c",
                    "(sleep .4; printf escaped > \"$1\") & if [ \"$2\" = yes ]; then sleep 5; fi",
                    "fixture",
                ])
                .arg(&sentinel)
                .arg(if cancel_early { "yes" } else { "no" });
            let future = run(command, cancel, Duration::from_secs(3));
            let result = if cancel_early {
                tokio::join!(future, async {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    sender.send_replace(true);
                })
                .0
            } else {
                future.await
            };
            assert_eq!(
                result,
                if cancel_early {
                    Err(Failure::Cancelled)
                } else {
                    Ok(vec![])
                }
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(!sentinel.exists(), "descendant outlived owner");
        }
    }
}
