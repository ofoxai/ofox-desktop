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
                include_str!("../../../tests/fixtures/assistant-process.mjs"),
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
                    "fixture never became ready: {}",
                    path.display()
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
        pub(crate) async fn observe_owned(&mut self, managed: &mut Child) {
            let observed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
            }));
            if let Err(failure) = observed {
                // Preserve the failure while exposing startup errors from the
                // owned fixture instead of reporting only a readiness timeout.
                managed.stop().await;
                if let Some(stderr) = managed.take_stderr() {
                    if let Ok(Ok(bytes)) =
                        tokio::time::timeout(Duration::from_secs(1), bounded_output(stderr, 8192))
                            .await
                    {
                        eprintln!(
                            "Managed fixture stderr: {}",
                            String::from_utf8_lossy(&bytes)
                        );
                    }
                }
                std::panic::resume_unwind(failure);
            }
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
            // Do not let teardown make an assertion pass: callers inspect
            // retained handles before this fallback cleanup runs. Only the
            // managed Child owns permission to terminate its Job.
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
    #[cfg(unix)]
    use std::path::Path;

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

    /// A shell whose background descendant first writes `ready`, then, unless
    /// it is killed, writes `child-survived` 0.3 s later. `wait` keeps the
    /// parent alive; without it the parent exits and leaves the descendant.
    #[cfg(unix)]
    fn tree_command(home: &Path, parent_waits: bool) -> tokio::process::Command {
        let script = if parent_waits {
            "(touch ready; sleep 0.3; touch child-survived) & wait"
        } else {
            "(touch ready; sleep 0.3; touch child-survived) &"
        };
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", script]).current_dir(home);
        clean_environment(&mut command);
        command
    }

    /// Handshake instead of a fixed sleep: the descendant is known to run.
    #[cfg(unix)]
    async fn wait_until_ready(home: &Path) {
        let ready = home.join("ready");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "fixture never became ready"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Long enough for a surviving descendant to have written its marker.
    #[cfg(unix)]
    async fn assert_descendant_killed(home: &Path) {
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(!home.join("child-survived").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn bounded_output_keeps_draining_a_loud_child() {
        let sandbox = crate::test_sandbox::sandbox();
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "head -c 100000 /dev/zero; printf done >&2"])
            .current_dir(sandbox.home());
        clean_environment(&mut command);
        let mut child = Child::spawn(command).unwrap();
        drop(child.take_stdin());
        let stdout = tokio::spawn(bounded_output(child.take_stdout().unwrap(), 32 * 1024));
        let stderr = tokio::spawn(bounded_output(child.take_stderr().unwrap(), 32 * 1024));
        // More than a pipe buffer is written: the child only exits if the
        // bounded reader keeps draining after it stops keeping bytes.
        let status = tokio::time::timeout(Duration::from_secs(5), child.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success());
        assert_eq!(stdout.await.unwrap().unwrap().len(), 32 * 1024);
        assert_eq!(stderr.await.unwrap().unwrap(), b"done");
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn stop_ends_the_owned_tree_and_spares_a_same_name_process() {
        let sandbox = crate::test_sandbox::sandbox();
        let home = sandbox.home();
        // Same executable, not ours: it must outlive our cleanup.
        let mut sentinel = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .unwrap();
        let mut child = Child::spawn(tree_command(home, true)).unwrap();
        wait_until_ready(home).await;
        // The host's deadline passes while the tree still runs, then it stops it.
        assert!(
            tokio::time::timeout(Duration::from_millis(100), child.child.wait())
                .await
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(5), child.stop())
            .await
            .unwrap();
        // Checked while the Child is still held: stop() alone ended the tree.
        assert!(child.child.try_wait().unwrap().is_some());
        assert_descendant_killed(home).await;
        assert!(sentinel.try_wait().unwrap().is_none());
        drop(child);
        sentinel.kill().unwrap();
        sentinel.wait().unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn stop_ends_descendants_that_outlive_their_parent() {
        let sandbox = crate::test_sandbox::sandbox();
        let home = sandbox.home();
        let mut child = Child::spawn(tree_command(home, false)).unwrap();
        wait_until_ready(home).await;
        let status = tokio::time::timeout(Duration::from_secs(5), child.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success());
        tokio::time::timeout(Duration::from_secs(5), child.stop())
            .await
            .unwrap();
        assert_descendant_killed(home).await;
        drop(child);
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn dropping_a_managed_child_kills_its_own_descendants() {
        let sandbox = crate::test_sandbox::sandbox();
        let home = sandbox.home();
        let child = Child::spawn(tree_command(home, true)).unwrap();
        wait_until_ready(home).await;
        drop(child);
        assert_descendant_killed(home).await;
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn windows_stop_ends_owned_parent_and_child_without_killing_same_name_sentinel() {
        let sandbox = crate::test_sandbox::sandbox();
        let mut fixture = windows_fixture::Fixture::new(sandbox.home());
        let mut child = Child::spawn(fixture.command()).unwrap();
        fixture.observe_owned(&mut child).await;
        tokio::time::timeout(Duration::from_secs(10), child.stop())
            .await
            .unwrap();
        // Checked while the Child is still held: stop() alone ended the Job's
        // tree, and the same-name sentinel outside the Job is alive.
        fixture.assert_owned_stopped();
        drop(child);
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn windows_stop_after_a_missed_deadline_ends_the_running_tree() {
        let sandbox = crate::test_sandbox::sandbox();
        let mut fixture = windows_fixture::Fixture::new(sandbox.home());
        let mut child = Child::spawn(fixture.command()).unwrap();
        // Readiness and live native handles first, so "never started" cannot
        // pass for "stopped".
        fixture.observe_owned(&mut child).await;
        // The host's deadline passes while the tree still runs, then it stops it.
        assert!(
            tokio::time::timeout(Duration::from_millis(500), child.child.wait())
                .await
                .is_err()
        );
        fixture.assert_owned_alive();
        tokio::time::timeout(Duration::from_secs(10), child.stop())
            .await
            .unwrap();
        fixture.assert_owned_stopped();
        drop(child);
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn windows_dropping_managed_child_stops_its_owned_tree() {
        let sandbox = crate::test_sandbox::sandbox();
        let mut fixture = windows_fixture::Fixture::new(sandbox.home());
        let mut child = Child::spawn(fixture.command()).unwrap();
        fixture.observe_owned(&mut child).await;
        drop(child);
        fixture.assert_owned_stopped();
    }
}
