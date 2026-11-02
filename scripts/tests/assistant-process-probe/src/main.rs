#[cfg(windows)]
#[path = "../../../../src-tauri/src/services/assistant/process.rs"]
mod process;

#[cfg(windows)]
#[tokio::main]
async fn main() {
    let home = std::env::temp_dir().join(format!("ofox-shell-probe-{}", std::process::id()));
    for suffix in ["AppData/Roaming", "AppData/Local"] {
        std::fs::create_dir_all(home.join(suffix)).unwrap();
    }
    let groups: &[(&str, &[&str])] = &[
        ("base", &[]),
        ("module-path", &["PSModulePath"]),
        (
            "cpu",
            &[
                "OS",
                "PROCESSOR_ARCHITECTURE",
                "PROCESSOR_IDENTIFIER",
                "PROCESSOR_LEVEL",
                "PROCESSOR_REVISION",
                "NUMBER_OF_PROCESSORS",
            ],
        ),
        (
            "identity",
            &[
                "PUBLIC",
                "ALLUSERSPROFILE",
                "USERNAME",
                "USERDOMAIN",
                "USERDOMAIN_ROAMINGPROFILE",
                "LOGONSERVER",
                "COMPUTERNAME",
                "HOMEDRIVE",
                "HOMEPATH",
            ],
        ),
    ];
    for (label, names) in groups {
        let passed = check(label, names, &home).await;
        if passed && names.len() > 1 {
            for name in *names {
                check(name, &[*name], &home).await;
            }
        }
    }
    std::fs::remove_dir_all(home).unwrap();
}

#[cfg(windows)]
async fn check(label: &str, names: &[&str], home: &std::path::Path) -> bool {
    use std::{path::PathBuf, time::Duration};
    let root = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
    let mut command =
        tokio::process::Command::new(root.join("System32/WindowsPowerShell/v1.0/powershell.exe"));
    command.args(["-NoLogo","-NoProfile","-NonInteractive","-Command","[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding; Get-Location; Write-Output 'shell-probe-ok'"]);
    process::clean_environment(&mut command);
    command
        .current_dir(home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("APPDATA", home.join("AppData/Roaming"))
        .env("LOCALAPPDATA", home.join("AppData/Local"));
    for name in names {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    println!("CASE {label}");
    let mut child = process::Child::spawn(command).unwrap();
    drop(child.take_stdin());
    let out = tokio::spawn(process::bounded_output(child.take_stdout().unwrap(), 4096));
    let err = tokio::spawn(process::bounded_output(child.take_stderr().unwrap(), 4096));
    let status = tokio::time::timeout(Duration::from_secs(8), child.child.wait()).await;
    let passed = matches!(status,Ok(Ok(s)) if s.success());
    println!(
        "job_wait={status:?}; stdout_done={}; stderr_done={}",
        out.is_finished(),
        err.is_finished()
    );
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
    passed
}

#[cfg(not(windows))]
fn main() {
    println!("This probe runs on Windows.");
}
