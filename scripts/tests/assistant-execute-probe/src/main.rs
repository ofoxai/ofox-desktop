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
    let builtin = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/Modules");
    let paths: Vec<_> = std::env::split_paths(original.as_deref().unwrap_or_default()).collect();
    for (label, first) in [
        (
            "native_builtin_first",
            PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("Modules"),
        ),
        ("mixed_builtin_first", builtin),
    ] {
        let mut modules = vec![first];
        modules.extend(paths.clone());
        std::env::set_var("PSModulePath", std::env::join_paths(modules).unwrap());
        let directory = home.join(format!("{label} 中文"));
        std::fs::create_dir_all(&directory).unwrap();
        for command in ["Get-Location", "Get-Location | Select-Object -ExpandProperty Path; Write-Output '中文'; Write-Output ('x' * 200000)"] {
            println!("CASE {label} unicode_large={}",command.len()>20);
            let result = rt.block_on(execute::run(command, &directory, &directory, 8));
            println!("actual_execute_result={:?}", result.as_ref().map(|v| (v["exitCode"].clone(),v["output"].as_str().unwrap().len(),v["truncated"].clone())));
        }
    }
    std::fs::remove_dir_all(home).unwrap();
}
#[cfg(not(windows))]
fn main() {
    println!("Windows-only execute diagnostic");
}
