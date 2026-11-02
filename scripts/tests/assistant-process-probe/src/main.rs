#[cfg(windows)]
#[path = "../../../../src-tauri/src/services/assistant/process.rs"]
mod process;

#[cfg(windows)]
#[tokio::main]
async fn main() {
    use std::{path::PathBuf, process::Stdio, time::Duration};
    let home = std::env::temp_dir().join(format!("ofox-shell-probe-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    for suffix in ["AppData/Roaming", "AppData/Local"] {
        std::fs::create_dir_all(home.join(suffix)).unwrap();
    }
    for (label, managed, filtered, isolated, encoding) in [
        ("raw-inherited", false, false, false, true),
        ("raw-isolated", false, false, true, true),
        ("raw-filtered-system", false, true, true, true),
        ("job-filtered-system", true, true, true, true),
        ("raw-filtered-no-encoding", false, true, true, false),
        ("raw-filtered-real-home", false, true, false, true),
    ] {
        let root = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        let mut command = tokio::process::Command::new(
            root.join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        );
        let script = if encoding {
            "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding; Write-Output 'shell-probe-ok'"
        } else {
            "Write-Output 'shell-probe-ok'"
        };
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ]);
        if filtered {
            process::clean_environment(&mut command);
            for name in [
                "SystemDrive",
                "ProgramData",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "CommonProgramFiles",
                "CommonProgramFiles(x86)",
                "PUBLIC",
                "ALLUSERSPROFILE",
                "USERNAME",
                "USERDOMAIN",
                "USERDOMAIN_ROAMINGPROFILE",
                "LOGONSERVER",
                "COMPUTERNAME",
                "HOMEDRIVE",
                "HOMEPATH",
                "OS",
                "PROCESSOR_ARCHITECTURE",
                "PROCESSOR_IDENTIFIER",
                "PROCESSOR_LEVEL",
                "PROCESSOR_REVISION",
                "NUMBER_OF_PROCESSORS",
                "PSModulePath",
            ] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
        }
        command.current_dir(&home);
        if isolated {
            command
                .env("HOME", &home)
                .env("USERPROFILE", &home)
                .env("APPDATA", home.join("AppData/Roaming"))
                .env("LOCALAPPDATA", home.join("AppData/Local"));
        }
        println!("CASE {label}");
        if managed {
            let mut child = process::Child::spawn(command).unwrap();
            drop(child.take_stdin());
            let out = tokio::spawn(process::bounded_output(child.take_stdout().unwrap(), 4096));
            let err = tokio::spawn(process::bounded_output(child.take_stderr().unwrap(), 4096));
            let status = tokio::time::timeout(Duration::from_secs(8), child.child.wait()).await;
            println!(
                "job_wait={status:?}; stdout_done={}; stderr_done={}",
                out.is_finished(),
                err.is_finished()
            );
            if status.is_err() {
                use std::os::windows::io::AsRawHandle;
                let mut code = 0;
                if let Some(handle) = child.child.process_handle() {
                    let result = unsafe {
                        windows::Win32::System::Threading::GetExitCodeProcess(
                            windows::Win32::Foundation::HANDLE(handle.as_raw_handle()),
                            &mut code,
                        )
                    };
                    println!("parent_exit_code={code}; query={result:?}");
                }
            }
            child.stop().await;
            println!(
                "stdout={:?}; stderr={:?}",
                out.await
                    .unwrap()
                    .map(|b| String::from_utf8_lossy(&b).into_owned()),
                err.await
                    .unwrap()
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
            );
        } else {
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            command.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
            let mut child = command.spawn().unwrap();
            let out = tokio::spawn(process::bounded_output(child.stdout.take().unwrap(), 4096));
            let err = tokio::spawn(process::bounded_output(child.stderr.take().unwrap(), 4096));
            let status = tokio::time::timeout(Duration::from_secs(8), child.wait()).await;
            println!(
                "raw_wait={status:?}; stdout_done={}; stderr_done={}",
                out.is_finished(),
                err.is_finished()
            );
            let _ = child.kill().await;
            println!(
                "stdout={:?}; stderr={:?}",
                out.await
                    .unwrap()
                    .map(|b| String::from_utf8_lossy(&b).into_owned()),
                err.await
                    .unwrap()
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
            );
        }
    }
    std::fs::remove_dir_all(home).unwrap();
}

#[cfg(not(windows))]
fn main() {
    println!("This probe runs on Windows.");
}
