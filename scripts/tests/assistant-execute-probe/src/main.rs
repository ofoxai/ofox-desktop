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
    // Only module directory paths from this ephemeral CI runner; no full environment.
    println!("module_directories={paths:?}");
    let mut cases = vec![("original".to_string(), paths.clone())];
    let mut first = vec![builtin.clone()];
    first.extend(paths.clone());
    cases.push(("builtin_first".into(), first));
    let mut last = paths.clone();
    last.push(builtin.clone());
    cases.push(("builtin_last".into(), last));
    for (index, path) in paths.iter().enumerate() {
        cases.push((format!("entry_{index}_only"), vec![path.clone()]));
    }
    for (label, module_paths) in cases {
        std::env::set_var("PSModulePath", std::env::join_paths(module_paths).unwrap());
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
