#![allow(dead_code)]
#[cfg(windows)]
#[path = "../../../../src-tauri/src/services/assistant/tools/commands.rs"]
mod commands;
#[cfg(windows)]
#[path = "../../../../src-tauri/src/services/assistant/tools/execute.rs"]
mod execute;
#[cfg(windows)]
#[path = "../../../../src-tauri/src/services/assistant/process.rs"]
mod original_process;
#[cfg(windows)]
pub mod process {
    pub use crate::original_process::{bounded_output, Child};
    pub fn clean_environment(command: &mut tokio::process::Command) {
        crate::original_process::clean_environment(command);
        for name in std::env::var("OFOX_PROBE_SYSTEM_VARS")
            .unwrap_or_default()
            .split(';')
        {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
    }
}
#[cfg(windows)]
mod redact {
    // This startup diagnostic only runs Get-Location in its own temp directory.
    // Redaction happens after process exit and is covered by the app test suite.
    pub fn scrub(value: &str) -> String {
        value.to_owned()
    }
}
#[cfg(windows)]
#[path = "../../../../src-tauri/src/services/assistant/tools/scope.rs"]
mod scope;
#[cfg(windows)]
mod services {
    pub mod assistant {
        pub use crate::process;
    }
}

#[cfg(windows)]
fn main() {
    use std::path::PathBuf;
    let home = std::env::temp_dir().join(format!("ofox-execute-probe-{}", std::process::id()));
    for suffix in ["AppData/Roaming", "AppData/Local"] {
        std::fs::create_dir_all(home.join(suffix)).unwrap();
    }
    std::env::set_var("APPDATA", home.join("AppData/Roaming"));
    std::env::set_var("LOCALAPPDATA", home.join("AppData/Local"));
    let original = std::env::var_os("PSModulePath");
    println!(
        "PSModulePath present={}, has_variable_reference={}",
        original.is_some(),
        original
            .as_ref()
            .is_some_and(|v| v.to_string_lossy().contains('%'))
    );
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for (label, extra) in [
        ("inherited", ""),
        ("program_files", "ProgramFiles"),
        ("system_drive", "SystemDrive"),
        ("both", "ProgramFiles;SystemDrive"),
        ("program_files_family", "ProgramFiles;ProgramFiles(x86);ProgramW6432;CommonProgramFiles;CommonProgramFiles(x86);CommonProgramW6432"),
        ("system_paths", "SystemDrive;ProgramData;ProgramFiles;ProgramFiles(x86);ProgramW6432;CommonProgramFiles;CommonProgramFiles(x86);CommonProgramW6432"),
        ("builtin_only", ""),
        ("unset_module_path_system_paths", "SystemDrive;ProgramData;ProgramFiles;ProgramFiles(x86);ProgramW6432;CommonProgramFiles;CommonProgramFiles(x86);CommonProgramW6432"),
    ] {
        if label == "builtin_only" {
            std::env::set_var("PSModulePath", PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/WindowsPowerShell/v1.0/Modules"));
        } else if label == "unset_module_path_system_paths" {
            std::env::remove_var("PSModulePath");
        } else if let Some(value) = original.as_ref() {
            std::env::set_var("PSModulePath", value);
        } else {
            std::env::remove_var("PSModulePath");
        }
        std::env::set_var("OFOX_PROBE_SYSTEM_VARS", extra);
        println!("CASE {label}");
        let result = rt.block_on(execute::run("Get-Location", &home, &home, 8));
        println!("actual_execute_result={result:?}");
    }
    std::fs::remove_dir_all(home).unwrap();
}
#[cfg(not(windows))]
fn main() {
    println!("Windows-only execute diagnostic");
}
