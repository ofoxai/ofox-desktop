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
        let mut builder = if threaded {
            tokio::runtime::Builder::new_multi_thread()
        } else {
            tokio::runtime::Builder::new_current_thread()
        };
        let rt = builder.enable_all().build().unwrap();
        for inherited in [true, false] {
            if let Some(value) = original.as_ref().filter(|_| inherited) {
                std::env::set_var("PSModulePath", value);
            } else {
                std::env::remove_var("PSModulePath");
            }
            let directory = home.join(format!("threaded-{threaded}-inherited-{inherited} 中文"));
            std::fs::create_dir_all(&directory).unwrap();
            for command in ["Get-Location", "Get-Location | Select-Object -ExpandProperty Path; Write-Output '中文'; Write-Output ('x' * 200000)"] {
                let result = rt.block_on(execute::run(command, &directory, &directory, 8)).unwrap();
                assert_eq!(result["exitCode"], 0);
                let output = result["output"].as_str().unwrap();
                assert!(output.contains("中文"));
                assert!(output.len() <= 32768);
                println!("PASS threaded={threaded} inherited={inherited} large={}", command.len()>20);
            }
            assert!(rt
                .block_on(execute::run(
                    "Start-Sleep -Seconds 5; Set-Content -LiteralPath late -Value bad",
                    &directory,
                    &directory,
                    1
                ))
                .is_err());
            std::thread::sleep(std::time::Duration::from_millis(1500));
            assert!(!directory.join("late").exists());
            println!("PASS timeout cleanup threaded={threaded} inherited={inherited}");
        }
    }
    std::fs::remove_dir_all(home).unwrap();
}
#[cfg(not(windows))]
fn main() {
    println!("Windows-only execute diagnostic");
}
