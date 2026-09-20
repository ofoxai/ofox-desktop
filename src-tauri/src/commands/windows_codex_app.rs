//! Windows ChatGPT/Codex Store-app detection.
//!
//! The unified desktop app is the `OpenAI.Codex` MSIX package. The older
//! `OpenAI.ChatGPT-Desktop` package is ChatGPT Classic and is intentionally not
//! treated as a Codex desktop installation.

#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[allow(dead_code)]
pub(crate) const STORE_PRODUCT_ID: &str = "9PLM9XGG6VKS";
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const PACKAGE_FAMILY: &str = "OpenAI.Codex_2p2nqsd0c76g0";

#[cfg(target_os = "windows")]
pub(crate) fn detect_codex_desktop_app() -> Result<Option<String>, String> {
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let powershell = system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let script = concat!(
        "$ErrorActionPreference='Stop';",
        "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);",
        "$p=Get-AppxPackage -Name OpenAI.Codex -ErrorAction SilentlyContinue | ",
        "Sort-Object {[version]$_.Version} -Descending | Select-Object -First 1;",
        "if($p){[Console]::Out.WriteLine(('{0}{1}{2}' -f $p.Version,[char]9,$p.PackageFamilyName))}"
    );
    let output = Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("查询 ChatGPT/Codex Store App 失败: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "查询 ChatGPT/Codex Store App 失败".to_string()
        } else {
            format!("查询 ChatGPT/Codex Store App 失败: {stderr}")
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_appx_identity(&stdout)
}

#[cfg(target_os = "windows")]
pub(crate) fn launch_codex_desktop_app() -> Result<bool, String> {
    if detect_codex_desktop_app()?.is_none() {
        return Ok(false);
    }

    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let powershell = system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let script = concat!(
        "$ErrorActionPreference='Stop';",
        "try { Start-Process 'codex://' } ",
        "catch { Start-Process 'shell:AppsFolder\\OpenAI.Codex_2p2nqsd0c76g0!App' }"
    );
    let output = Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("启动 ChatGPT/Codex Store App 失败: {err}"))?;

    if output.status.success() {
        Ok(true)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            "启动 ChatGPT/Codex Store App 失败".to_string()
        } else {
            format!("启动 ChatGPT/Codex Store App 失败: {stderr}")
        })
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_appx_identity(output: &str) -> Result<Option<String>, String> {
    let line = output.lines().map(str::trim).find(|line| !line.is_empty());
    let Some(line) = line else {
        return Ok(None);
    };
    let Some((version, family)) = line.split_once('\t') else {
        return Err("ChatGPT/Codex Store App 返回了无法识别的包信息".to_string());
    };
    let family = family.trim();
    if family != PACKAGE_FAMILY {
        return Err(format!(
            "ChatGPT/Codex Store App 包身份不匹配：期望 {PACKAGE_FAMILY}，实际 {family}"
        ));
    }
    let version = version.trim();
    if version.is_empty() || !version.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
        return Err(format!("ChatGPT/Codex Store App 版本无效: {version}"));
    }
    Ok(Some(version.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unified_store_app_identity() {
        assert_eq!(
            parse_appx_identity("26.903.9818.0\tOpenAI.Codex_2p2nqsd0c76g0\r\n").unwrap(),
            Some("26.903.9818.0".to_string())
        );
    }

    #[test]
    fn rejects_chatgpt_classic_package() {
        let error = parse_appx_identity("1.2026.133.0\tOpenAI.ChatGPT-Desktop_2p2nqsd0c76g0\r\n")
            .unwrap_err();
        assert!(error.contains("包身份不匹配"));
    }

    #[test]
    fn exposes_current_store_product_id() {
        assert_eq!(STORE_PRODUCT_ID, "9PLM9XGG6VKS");
    }
}
