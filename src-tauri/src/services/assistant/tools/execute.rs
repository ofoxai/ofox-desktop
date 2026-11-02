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
        p.args(["-NoLogo", "-NonInteractive", "-Command", &script]);
        p
    };
    process::clean_environment(&mut process);
    process
        .current_dir(folder)
        .env("HOME", home)
        .env("USERPROFILE", home);
    let mut child = Child::spawn(process)?;
    drop(child.take_stdin());
    let stdout = child.take_stdout().ok_or("assistant_runtime_stdio")?;
    let stderr = child.take_stderr().ok_or("assistant_runtime_stdio")?;
    let operation = async {
        let (out, err, status) = tokio::join!(
            process::bounded_output(stdout, 24576),
            process::bounded_output(stderr, 24576),
            child.child.wait()
        );
        let out = out.map_err(|_| "assistant_runtime_stdio")?;
        let err = err.map_err(|_| "assistant_runtime_stdio")?;
        let status = status.map_err(|_| "assistant_runtime_stdio")?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        );
        if text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----") {
            return Err("assistant_path_denied".into());
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
            json!({"output":output,"exitCode":status.code(),"truncated":output.len()<text.len()||out.len()==24576||err.len()==24576}),
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
