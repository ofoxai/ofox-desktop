#![allow(dead_code)]
#[cfg(windows)]
#[path = "../../../../src-tauri/src/services/assistant/tools/commands.rs"]
mod commands;
#[cfg(windows)]
#[path = "../../../../src-tauri/src/services/assistant/tools/execute.rs"]
mod execute;
#[cfg(windows)]
#[path = "../../../../src-tauri/src/services/assistant/process.rs"]
pub mod process;
#[cfg(windows)]
#[path = "../../../../src-tauri/src/redact.rs"]
mod redact;
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
    for threaded in [false, true] {
        let rt = if threaded {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
        } else {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        };
        for builtin in [false, true] {
            if builtin {
                std::env::set_var(
                    "PSModulePath",
                    PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                        .join("System32/WindowsPowerShell/v1.0/Modules"),
                );
            } else if let Some(value) = original.as_ref() {
                std::env::set_var("PSModulePath", value)
            } else {
                std::env::remove_var("PSModulePath")
            }
            println!("CASE threaded={threaded} builtin_module_path={builtin}");
            let result = rt.block_on(execute::run("Get-Location", &home, &home, 8));
            println!("actual_execute_result={result:?}");
        }
    }
    std::fs::remove_dir_all(home).unwrap();
}
#[cfg(not(windows))]
fn main() {
    println!("Windows-only execute diagnostic");
}
