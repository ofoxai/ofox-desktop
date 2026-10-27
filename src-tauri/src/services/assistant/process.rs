use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};

/// Every child belongs to this process group / Windows Job. Never search the
/// machine by process name or kill an executable that the user started.
pub struct Child {
    #[cfg(unix)]
    pub child: tokio::process::Child,
    #[cfg(unix)]
    group: u32,
    #[cfg(windows)]
    pub child: Box<dyn process_wrap::tokio::ChildWrapper>,
    stopped: bool,
}
impl Child {
    pub fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        #[cfg(unix)]
        {
            self.child.stdin.take()
        }
        #[cfg(windows)]
        {
            self.child.stdin().take()
        }
    }
    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        #[cfg(unix)]
        {
            self.child.stdout.take()
        }
        #[cfg(windows)]
        {
            self.child.stdout().take()
        }
    }
    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        #[cfg(unix)]
        {
            self.child.stderr.take()
        }
        #[cfg(windows)]
        {
            self.child.stderr().take()
        }
    }
    pub fn spawn(mut command: tokio::process::Command) -> Result<Self, String> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
            let child = command
                .spawn()
                .map_err(|_| "assistant_process_start_failed")?;
            let group = child.id().ok_or("assistant_process_start_failed")?;
            Ok(Self {
                child,
                group,
                stopped: false,
            })
        }
        #[cfg(windows)]
        {
            use process_wrap::tokio::{CommandWrap, CreationFlags, JobObject, KillOnDrop};
            use windows::Win32::System::Threading::CREATE_NO_WINDOW;
            let mut wrapper = CommandWrap::from(command);
            wrapper
                .wrap(KillOnDrop)
                .wrap(CreationFlags(CREATE_NO_WINDOW))
                .wrap(JobObject);
            Ok(Self {
                child: wrapper
                    .spawn()
                    .map_err(|_| "assistant_process_start_failed")?,
                stopped: false,
            })
        }
    }
    pub fn kill(&mut self) {
        if self.stopped {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.group as i32), libc::SIGKILL);
        }
        let _ = self.child.start_kill();
        self.stopped = true;
    }
    pub async fn stop(&mut self) {
        self.kill();
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        self.kill();
    }
}

pub async fn bounded_output(
    mut reader: impl AsyncRead + Unpin,
    limit: usize,
) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        let keep = n.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&chunk[..keep]);
    }
    Ok(bytes)
}

pub fn clean_environment(command: &mut tokio::process::Command) {
    let allowed = [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LOCALAPPDATA",
        "APPDATA",
        "LANG",
        "LC_ALL",
        "PATHEXT",
        "COMSPEC",
    ];
    let env: Vec<_> = allowed
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| ((*name).to_string(), value)))
        .collect();
    command.env_clear().envs(env);
}

pub async fn run(
    command: tokio::process::Command,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    if *cancel.borrow() {
        return Err("assistant_cancelled".into());
    }
    let mut child = Child::spawn(command)?;
    // No managed command may wait for input from an invisible terminal.
    drop(child.take_stdin());
    let stdout = child.take_stdout().ok_or("assistant_process_stdio")?;
    let stderr = child.take_stderr().ok_or("assistant_process_stdio")?;
    let mut readers = Readers {
        stdout: tokio::spawn(bounded_output(stdout, 32 * 1024)),
        stderr: tokio::spawn(bounded_output(stderr, 32 * 1024)),
    };
    let status = tokio::select! {
        biased;
        _=cancel.changed()=>Err("assistant_cancelled"),
        _=tokio::time::sleep(timeout)=>Err("assistant_process_timeout"),
        result=child.child.wait()=>result.map_err(|_|"assistant_process_wait_failed"),
    };
    // Also end grandchildren which inherited the pipes after their parent exited.
    child.stop().await;
    let status = status.map_err(str::to_string)?;
    let (stdout, stderr) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(&mut readers.stdout, &mut readers.stderr)
    })
    .await
    .map_err(|_| "assistant_process_drain_timeout")?;
    let stdout = stdout
        .map_err(|_| "assistant_process_read_failed")?
        .map_err(|_| "assistant_process_read_failed")?;
    let stderr = stderr
        .map_err(|_| "assistant_process_read_failed")?
        .map_err(|_| "assistant_process_read_failed")?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

struct Readers {
    stdout: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
}
impl Drop for Readers {
    fn drop(&mut self) {
        self.stdout.abort();
        self.stderr.abort();
    }
}

#[cfg(all(test, windows))]
pub(crate) mod windows_fixture {
    use super::*;
    use serde_json::Value;
    use std::os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    };
    use std::path::{Path, PathBuf};
    use windows::Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            OpenProcess, WaitForSingleObject, CREATE_NO_WINDOW, PROCESS_SYNCHRONIZE,
        },
    };

    struct OwnedProcess(OwnedHandle);
    impl OwnedProcess {
        fn open(pid: u32) -> Self {
            // Capture an observation-only handle before the action. Teardown
            // uses the original managed task/Child, never a PID lookup.
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }
                .expect("open the handshaken fixture process");
            Self(unsafe { OwnedHandle::from_raw_handle(handle.0) })
        }
        fn handle(&self) -> HANDLE {
            HANDLE(self.0.as_raw_handle())
        }
        fn running(&self) -> bool {
            match unsafe { WaitForSingleObject(self.handle(), 0) } {
                WAIT_TIMEOUT => true,
                WAIT_OBJECT_0 => false,
                other => panic!("fixture process state unavailable: {other:?}"),
            }
        }
        fn assert_stopped(&self) {
            assert_eq!(
                unsafe { WaitForSingleObject(self.handle(), 5000) },
                WAIT_OBJECT_0,
                "owned fixture process survived managed cleanup"
            );
        }
    }
    pub(crate) struct Fixture {
        directory: PathBuf,
        node: PathBuf,
        script: PathBuf,
        nonce: String,
        ready: PathBuf,
        sentinel: std::process::Child,
        owned: Vec<OwnedProcess>,
        abort: Option<tokio::task::AbortHandle>,
    }
    impl Fixture {
        pub(crate) fn new(home: &Path) -> Self {
            let directory = home.join("owned process 空 格");
            std::fs::create_dir_all(&directory).unwrap();
            let node = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../assistant-runtime/resources/node/node.exe");
            assert!(
                node.is_file(),
                "prepare the bundled Windows Node before Rust tests"
            );
            let script = directory.join("process fixture.mjs");
            std::fs::write(
                &script,
                r#"
import { spawn } from 'node:child_process';
import { writeFileSync, renameSync } from 'node:fs';
const [role, ready, nonce] = process.argv.slice(2);
const publish = data => {
  writeFileSync(ready + '.partial', JSON.stringify({ nonce, ...data }));
  renameSync(ready + '.partial', ready);
};
setInterval(() => {}, 1000);
if (role === 'sentinel') publish({ role, pid: process.pid });
else if (role === 'child') process.send({ nonce, pid: process.pid, parent: process.ppid });
else if (role === 'parent') {
  const child = spawn(process.execPath, [import.meta.filename, 'child', ready, nonce], {
    windowsHide: true, stdio: ['ignore', 'ignore', 'ignore', 'ipc']
  });
  child.once('error', () => process.exit(2));
  child.once('message', message => {
    if (message.nonce !== nonce || message.pid !== child.pid || message.parent !== process.pid)
      process.exit(3);
    publish({ role, parent: process.pid, child: child.pid });
  });
} else process.exit(4);
"#,
            )
            .unwrap();
            let nonce = uuid::Uuid::new_v4().to_string();
            let sentinel_ready = directory.join("sentinel.json");
            let sentinel = std::process::Command::new(&node)
                .arg(&script)
                .arg("sentinel")
                .arg(&sentinel_ready)
                .arg(&nonce)
                .env_clear()
                .current_dir(&directory)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW.0)
                .spawn()
                .unwrap();
            let mut fixture = Self {
                ready: directory.join("owned.json"),
                directory,
                node,
                script,
                nonce,
                sentinel,
                owned: Vec::new(),
                abort: None,
            };
            let record = fixture.read_ready(&sentinel_ready);
            assert_eq!(record["role"], "sentinel");
            assert_eq!(record["pid"].as_u64(), Some(fixture.sentinel.id().into()));
            fixture.assert_sentinel_alive();
            fixture
        }
        fn read_ready(&self, path: &Path) -> Value {
            let end = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                match std::fs::read(path) {
                    Ok(bytes) => {
                        let record: Value = serde_json::from_slice(&bytes).unwrap();
                        assert_eq!(record["nonce"], self.nonce);
                        return record;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("fixture handshake unreadable: {error}"),
                }
                assert!(
                    std::time::Instant::now() < end,
                    "fixture never became ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        pub(crate) fn command(&self) -> tokio::process::Command {
            let mut command = tokio::process::Command::new(&self.node);
            clean_environment(&mut command);
            command
                .arg(&self.script)
                .arg("parent")
                .arg(&self.ready)
                .arg(&self.nonce)
                .env("PATH", "")
                .current_dir(&self.directory);
            command
        }
        pub(crate) fn set_abort(&mut self, abort: tokio::task::AbortHandle) {
            self.abort = Some(abort);
        }
        pub(crate) fn observe_owned(&mut self) {
            let record = self.read_ready(&self.ready);
            assert_eq!(record["role"], "parent");
            let parent = u32::try_from(record["parent"].as_u64().unwrap()).unwrap();
            let child = u32::try_from(record["child"].as_u64().unwrap()).unwrap();
            assert!(parent > 1 && child > 1);
            assert_ne!(parent, child);
            assert_ne!(parent, self.sentinel.id());
            assert_ne!(child, self.sentinel.id());
            self.owned.push(OwnedProcess::open(parent));
            self.owned.push(OwnedProcess::open(child));
            self.assert_owned_alive();
        }
        pub(crate) fn assert_owned_alive(&mut self) {
            assert_eq!(self.owned.len(), 2);
            assert!(self.owned.iter().all(OwnedProcess::running));
            self.assert_sentinel_alive();
        }
        pub(crate) fn assert_owned_stopped(&mut self) {
            assert_eq!(self.owned.len(), 2);
            for process in &self.owned {
                process.assert_stopped();
            }
            self.assert_sentinel_alive();
        }
        fn assert_sentinel_alive(&mut self) {
            assert!(self.sentinel.try_wait().unwrap().is_none());
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(abort) = self.abort.take() {
                abort.abort();
                let end = std::time::Instant::now() + Duration::from_secs(3);
                while !abort.is_finished() && std::time::Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            // Do not let teardown make an assertion pass: callers inspect
            // retained handles before this fallback cleanup runs. Only the
            // original task owns permission to terminate the managed Job.
            for process in &self.owned {
                let _ = unsafe { WaitForSingleObject(process.handle(), 1000) };
            }
            self.owned.clear();
            if matches!(self.sentinel.try_wait(), Ok(None)) {
                let _ = self.sentinel.kill();
            }
            let _ = self.sentinel.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_commands_remove_credential_environment_entries() {
        let mut command = tokio::process::Command::new("fixture");
        command
            .env("ANTHROPIC_API_KEY", "fake-key")
            .env("OFOX_ACCESS_TOKEN", "fake-token");
        clean_environment(&mut command);
        let keys = command
            .as_std()
            .get_envs()
            .filter_map(|(key, value)| value.map(|_| key.to_string_lossy().into_owned()))
            .collect::<Vec<_>>();
        assert!(!keys
            .iter()
            .any(|key| key == "ANTHROPIC_API_KEY" || key == "OFOX_ACCESS_TOKEN"));
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn windows_cancel_stops_owned_parent_and_child_without_killing_same_name_sentinel() {
        let sandbox = crate::test_sandbox::sandbox();
        let mut fixture = windows_fixture::Fixture::new(sandbox.home());
        let (sender, cancel) = tokio::sync::watch::channel(false);
        let work = tokio::spawn(run(fixture.command(), cancel, Duration::from_secs(60)));
        fixture.set_abort(work.abort_handle());
        fixture.observe_owned();
        sender.send_replace(true);
        let result = tokio::time::timeout(Duration::from_secs(10), work)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err(), "assistant_cancelled");
        fixture.assert_owned_stopped();
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn windows_timeout_stops_an_already_running_owned_tree() {
        let sandbox = crate::test_sandbox::sandbox();
        let mut fixture = windows_fixture::Fixture::new(sandbox.home());
        let (_sender, cancel) = tokio::sync::watch::channel(false);
        let work = tokio::spawn(run(fixture.command(), cancel, Duration::from_secs(15)));
        fixture.set_abort(work.abort_handle());
        // Readiness and live native handles are required before waiting for
        // the timeout, so "never started" cannot masquerade as cancellation.
        fixture.observe_owned();
        let result = tokio::time::timeout(Duration::from_secs(20), work)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err(), "assistant_process_timeout");
        fixture.assert_owned_stopped();
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn windows_dropping_managed_child_stops_its_owned_tree() {
        let sandbox = crate::test_sandbox::sandbox();
        let mut fixture = windows_fixture::Fixture::new(sandbox.home());
        let child = Child::spawn(fixture.command()).unwrap();
        fixture.observe_owned();
        drop(child);
        fixture.assert_owned_stopped();
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn bounded_output_does_not_stop_draining_a_loud_child() {
        let sandbox = crate::test_sandbox::sandbox();
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "head -c 100000 /dev/zero; printf done >&2"])
            .current_dir(sandbox.home());
        clean_environment(&mut command);
        let (_sender, cancel) = tokio::sync::watch::channel(false);
        let result = run(command, cancel, Duration::from_secs(3)).await.unwrap();
        assert!(result.status.success());
        assert!(result.stdout.len() + result.stderr.len() <= 65536);
        assert_eq!(result.stderr, b"done");
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn cancelling_a_managed_process_kills_its_own_descendants() {
        let sandbox = crate::test_sandbox::sandbox();
        let marker = sandbox.home().join("child-survived");
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "(sleep 0.3; touch child-survived) & wait"])
            .current_dir(sandbox.home());
        clean_environment(&mut command);
        let (sender, cancel) = tokio::sync::watch::channel(false);
        let work = run(command, cancel, Duration::from_secs(3));
        let signal = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            sender.send_replace(true);
        };
        let (result, ()) = tokio::join!(work, signal);
        assert_eq!(result.unwrap_err(), "assistant_cancelled");
        tokio::time::sleep(Duration::from_millis(450)).await;
        assert!(!marker.exists());
    }
}
