//! Owned process groups/jobs, bounded output and host-side redaction.
use crate::services::assistant::process::{self, Child};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

pub async fn run(command: &str, folder: &Path, home: &Path, timeout: u64) -> Result<Value, String> {
    #[cfg(unix)]
    let mut process = {
        let mut p = tokio::process::Command::new("/bin/bash");
        p.args(["-l", "-c", command]);
        p
    };
    #[cfg(windows)]
    let mut process = {
        let root = std::env::var_os("SystemRoot").ok_or("assistant_process_start_failed")?;
        let mut p = tokio::process::Command::new(
            std::path::PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        );
        let script=format!("[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding; {command}");
        p.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ]);
        p
    };
    process::clean_environment(&mut process);
    process
        .current_dir(folder)
        .env("HOME", home)
        .env("USERPROFILE", home);
    let mut child = Child::spawn(process)?;
    drop(child.take_stdin());
    let stdout = child
        .take_stdout()
        .ok_or("assistant_command_outcome_unknown")?;
    let stderr = child
        .take_stderr()
        .ok_or("assistant_command_outcome_unknown")?;
    let operation = async {
        let (out, err, status) = tokio::join!(
            process::bounded_output(stdout, 24576),
            process::bounded_output(stderr, 24576),
            child.child.wait()
        );
        let out = out.map_err(|_| "assistant_command_outcome_unknown")?;
        let err = err.map_err(|_| "assistant_command_outcome_unknown")?;
        let status = status.map_err(|_| "assistant_command_outcome_unknown")?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        );
        if text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----") {
            return Err("assistant_command_outcome_unknown".into());
        }
        let clean = crate::redact::scrub(&text);
        let mut output = String::new();
        for line in clean.split_inclusive('\n').take(800) {
            for ch in line.chars() {
                if output.len() + ch.len_utf8() > 32768 {
                    break;
                }
                output.push(ch);
            }
            if output.len() >= 32765 {
                break;
            }
        }
        Ok::<_, String>(
            json!({"output":output,"exitCode":status.code(),"truncated":output.len()<text.len()||out.len()==24576||err.len()==24576,
                // Arbitrary commands can change configuration even when they
                // exit unsuccessfully. Do not let them bypass the folder gate.
                "requiresVerification":!super::commands::read_only(command,cfg!(windows))}),
        )
    };
    match tokio::time::timeout(Duration::from_secs(timeout.clamp(1, 600)), operation).await {
        Ok(result) => result,
        Err(_) => {
            child.stop().await;
            Err("assistant_command_outcome_unknown".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn only_read_only_commands_can_preserve_previous_verification() {
        let root = tempfile::tempdir().unwrap();
        let read = if cfg!(windows) { "Get-Location" } else { "pwd" };
        let result = run(read, root.path(), root.path(), 20).await.unwrap();
        assert_eq!(result["requiresVerification"], false);
        let result = run("exit 1", root.path(), root.path(), 20).await.unwrap();
        assert_eq!(result["exitCode"], 1);
        assert_eq!(result["requiresVerification"], true);
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn powershell_execution_uses_unicode_folder_and_redacts_bounded_output() {
        let root = tempfile::Builder::new()
            .prefix("ofox 中文 ")
            .tempdir()
            .unwrap();
        let result=run("Get-Location | Select-Object -ExpandProperty Path; Write-Output '中文'; Write-Output 'api_key=do-not-return-this'; Write-Output ('x' * 200000)",root.path(),root.path(),20).await.unwrap();
        let output = result["output"].as_str().unwrap();
        assert!(output.contains(root.path().to_str().unwrap()));
        assert!(output.contains("中文"));
        assert!(!output.contains("do-not-return-this"));
        assert!(output.len() <= 32768);
        assert_eq!(result["exitCode"], 0);
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn powershell_timeout_stops_before_late_write() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            run(
                "Start-Sleep -Seconds 5; Set-Content -LiteralPath late -Value bad",
                root.path(),
                root.path(),
                1
            )
            .await
            .unwrap_err(),
            "assistant_command_outcome_unknown"
        );
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(!root.path().join("late").exists());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn execution_uses_requested_folder_and_bounds_redacted_output() {
        let root = tempfile::tempdir().unwrap();
        let result = run(
            "pwd; printf 'api_key=do-not-return-this\\n'; head -c 200000 /dev/zero | tr '\\0' x",
            root.path(),
            root.path(),
            5,
        )
        .await
        .unwrap();
        let output = result["output"].as_str().unwrap();
        assert!(output.contains(root.path().file_name().unwrap().to_str().unwrap()));
        assert!(!output.contains("do-not-return-this"));
        assert!(output.len() <= 32768);
        assert_eq!(result["exitCode"], 0);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn execution_timeout_kills_owned_descendants() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("late");
        assert!(
            run("(sleep 2; touch late) & wait", root.path(), root.path(), 1)
                .await
                .is_err()
        );
        tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
        assert!(!marker.exists());
    }
}
