//! Native Windows tool installation.
//!
//! The installer never opens a shell window and does not require a preinstalled
//! Python or Node runtime. Claude and Hermes use their official PowerShell
//! installers. npm-based tools use a checksum-verified Node.js LTS runtime under
//! `%LOCALAPPDATA%\Ofox Desktop\toolchain`.

#![cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use futures::StreamExt;
#[cfg(target_os = "windows")]
use sha2::{Digest, Sha256};
#[cfg(target_os = "windows")]
use std::io::Write as _;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
use std::process::Stdio;
#[cfg(target_os = "windows")]
use tauri::{AppHandle, Emitter};
#[cfg(target_os = "windows")]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(target_os = "windows")]
const NODE_INDEX_URL: &str = "https://nodejs.org/dist/index.json";
#[cfg(target_os = "windows")]
const CLAUDE_INSTALLER_URL: &str = "https://claude.ai/install.ps1";
#[cfg(target_os = "windows")]
const HERMES_INSTALLER_URL: &str = "https://hermes-agent.nousresearch.com/install.ps1";
#[cfg(target_os = "windows")]
const CODEX_STORE_TIMEOUT_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeRelease {
    version: String,
    archive_name: String,
}

#[derive(Debug, Deserialize)]
struct NodeIndexEntry {
    version: String,
    lts: serde_json::Value,
    #[serde(default)]
    files: Vec<String>,
}

fn select_windows_x64_lts(index: &str) -> Result<NodeRelease, String> {
    let entries: Vec<NodeIndexEntry> =
        serde_json::from_str(index).map_err(|err| format!("Node.js 清单无效: {err}"))?;
    let entry = entries
        .into_iter()
        .find(|entry| {
            entry.lts.as_str().is_some() && entry.files.iter().any(|f| f == "win-x64-zip")
        })
        .ok_or_else(|| "Node.js 清单中没有可用的 Windows x64 LTS".to_string())?;
    let archive_name = format!("node-{}-win-x64.zip", entry.version);
    Ok(NodeRelease {
        version: entry.version,
        archive_name,
    })
}

fn checksum_for_archive(shasums: &str, archive_name: &str) -> Result<String, String> {
    for line in shasums.lines() {
        let mut fields = line.split_whitespace();
        let Some(hash) = fields.next() else { continue };
        let Some(name) = fields.next() else { continue };
        if name.trim_start_matches('*') == archive_name {
            let hash = hash.to_ascii_lowercase();
            if hash.len() == 64 && hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Ok(hash);
            }
            return Err(format!("Node.js 校验和格式无效: {hash}"));
        }
    }
    Err(format!("Node.js SHASUMS256.txt 缺少 {archive_name}"))
}

fn npm_requires_allow_scripts(version: &str) -> bool {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let major = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    major >= 12 || (major == 11 && minor >= 16)
}

fn toolchain_root_for(local_app_data: &Path) -> PathBuf {
    local_app_data.join("Ofox Desktop").join("toolchain")
}

fn managed_node_dir_for(local_app_data: &Path) -> PathBuf {
    toolchain_root_for(local_app_data).join("node")
}

fn managed_npm_prefix_for(local_app_data: &Path) -> PathBuf {
    toolchain_root_for(local_app_data).join("npm")
}

fn npm_package_for(tool: &str) -> Option<&'static str> {
    match tool {
        "gemini" => Some("@google/gemini-cli@latest"),
        "opencode" => Some("opencode-ai@latest"),
        "openclaw" => Some("openclaw@latest"),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn local_app_data_dir() -> Result<PathBuf, String> {
    dirs::data_local_dir().ok_or_else(|| "无法确定 LOCALAPPDATA 目录".to_string())
}

#[cfg(target_os = "windows")]
pub(crate) fn managed_npm_prefix() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| managed_npm_prefix_for(&dir))
}

#[cfg(target_os = "windows")]
pub(crate) fn managed_node_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| managed_node_dir_for(&dir))
}

#[cfg(target_os = "windows")]
pub(crate) fn managed_cli_path(tool: &str) -> Option<PathBuf> {
    let prefix = managed_npm_prefix()?;
    [
        prefix.join(format!("{tool}.cmd")),
        prefix.join(format!("{tool}.exe")),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

#[cfg(target_os = "windows")]
pub(crate) fn managed_cli_launch_command(tool: &str) -> Option<String> {
    let path = managed_cli_path(tool)?;
    let escaped = path
        .to_string_lossy()
        .replace('%', "%%")
        .replace('"', "\\\"");
    Some(
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
        {
            format!("call \"{escaped}\"")
        } else {
            format!("\"{escaped}\"")
        },
    )
}

#[cfg(target_os = "windows")]
fn emit_progress(
    app: &AppHandle,
    tool: &str,
    step: u64,
    total: u64,
    name: &str,
    phase: &str,
    detail: Option<&str>,
    downloaded: Option<u64>,
    total_bytes: Option<u64>,
    elapsed: Option<u64>,
) {
    let mut value = serde_json::json!({
        "type": "ofox-install-progress",
        "step": step,
        "total": total,
        "name": name,
        "phase": phase,
    });
    let object = value.as_object_mut().expect("progress is an object");
    if let Some(detail) = detail {
        object.insert("detail".into(), serde_json::json!(detail));
    }
    if let Some(downloaded) = downloaded {
        object.insert("downloadedBytes".into(), serde_json::json!(downloaded));
    }
    if let Some(total_bytes) = total_bytes {
        object.insert("totalBytes".into(), serde_json::json!(total_bytes));
        if total_bytes > 0 {
            object.insert(
                "percent".into(),
                serde_json::json!((downloaded.unwrap_or(0) * 100 / total_bytes).min(100)),
            );
        }
    }
    if let Some(elapsed) = elapsed {
        object.insert("elapsed".into(), serde_json::json!(elapsed));
    }
    let _ = app.emit(
        "install-tool-log",
        serde_json::json!({"tool": tool, "stream": "stdout", "line": value.to_string()}),
    );
}

#[cfg(target_os = "windows")]
fn powershell_path() -> PathBuf {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32/WindowsPowerShell/v1.0/powershell.exe")
}

#[cfg(target_os = "windows")]
async fn pump_stream<R>(app: AppHandle, pipe: R, tool: String, stream: &'static str)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(pipe).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let _ = app.emit(
            "install-tool-log",
            serde_json::json!({"tool": tool, "stream": stream, "line": line}),
        );
    }
}

#[cfg(target_os = "windows")]
async fn run_command(
    app: &AppHandle,
    tool: &str,
    command: &mut tokio::process::Command,
    description: &str,
) -> Result<(), String> {
    command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| format!("启动{description}失败: {err}"))?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stdout_task = tokio::spawn(pump_stream(app.clone(), stdout, tool.to_string(), "stdout"));
    let stderr_task = tokio::spawn(pump_stream(app.clone(), stderr, tool.to_string(), "stderr"));
    let status = child
        .wait()
        .await
        .map_err(|err| format!("等待{description}失败: {err}"))?;
    let _ = tokio::join!(stdout_task, stderr_task);
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{description}失败（退出码 {}）",
            status.code().unwrap_or(-1)
        ))
    }
}

#[cfg(target_os = "windows")]
async fn run_powershell(
    app: &AppHandle,
    tool: &str,
    script: &str,
    description: &str,
) -> Result<(), String> {
    let mut command = tokio::process::Command::new(powershell_path());
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    run_command(app, tool, &mut command, description).await
}

#[cfg(target_os = "windows")]
async fn install_official_powershell_tool(app: &AppHandle, tool: &str) -> Result<i32, String> {
    let (url, args) = match tool {
        "claude" => (CLAUDE_INSTALLER_URL, ""),
        "hermes" => (HERMES_INSTALLER_URL, " -SkipSetup -NonInteractive"),
        _ => return Err(format!("没有 {tool} 的官方 PowerShell 安装器")),
    };
    emit_progress(
        app,
        tool,
        1,
        2,
        "官方安装器",
        "start",
        Some(url),
        None,
        None,
        None,
    );
    let script = format!(
        "$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; \
         [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; \
         & ([scriptblock]::Create((Invoke-RestMethod -UseBasicParsing '{url}'))){args}"
    );
    if let Err(err) = run_powershell(app, tool, &script, "官方 PowerShell 安装器").await {
        emit_progress(
            app,
            tool,
            1,
            2,
            "官方安装器",
            "failed",
            Some(&err),
            None,
            None,
            None,
        );
        return Err(err);
    }
    emit_progress(
        app,
        tool,
        1,
        2,
        "官方安装器",
        "done",
        None,
        None,
        None,
        None,
    );

    emit_progress(app, tool, 2, 2, "验证安装", "start", None, None, None, None);
    let verify = format!(
        "$p=@([Environment]::GetEnvironmentVariable('Path','Process'), \
         [Environment]::GetEnvironmentVariable('Path','User'), \
         [Environment]::GetEnvironmentVariable('Path','Machine')) -join ';'; \
         $env:Path=$p; & {tool} --version"
    );
    if let Err(err) = run_powershell(app, tool, &verify, "安装验证").await {
        emit_progress(
            app,
            tool,
            2,
            2,
            "验证安装",
            "failed",
            Some(&err),
            None,
            None,
            None,
        );
        return Err(err);
    }
    emit_progress(app, tool, 2, 2, "验证安装", "done", None, None, None, None);
    Ok(0)
}

#[cfg(target_os = "windows")]
async fn install_codex_store_app(app: &AppHandle) -> Result<i32, String> {
    if super::windows_codex_app::detect_codex_desktop_app()?.is_some() {
        emit_progress(
            app,
            "codex",
            1,
            1,
            "Microsoft Store",
            "skipped",
            Some("ChatGPT/Codex App 已安装"),
            None,
            None,
            None,
        );
        return Ok(0);
    }

    let uri = format!(
        "ms-windows-store://pdp/?ProductId={}",
        super::windows_codex_app::STORE_PRODUCT_ID
    );
    let script = format!("Start-Process '{uri}'");
    run_powershell(app, "codex", &script, "打开 Microsoft Store").await?;

    let started = std::time::Instant::now();
    loop {
        if super::windows_codex_app::detect_codex_desktop_app()?.is_some() {
            emit_progress(
                app,
                "codex",
                1,
                1,
                "Microsoft Store",
                "done",
                Some("ChatGPT/Codex App 安装完成"),
                None,
                None,
                Some(started.elapsed().as_secs()),
            );
            return Ok(0);
        }
        let elapsed = started.elapsed().as_secs();
        if elapsed >= CODEX_STORE_TIMEOUT_SECS {
            let err = "等待 Microsoft Store 安装超时；请完成安装后重新扫描";
            emit_progress(
                app,
                "codex",
                1,
                1,
                "Microsoft Store",
                "failed",
                Some(err),
                None,
                None,
                Some(elapsed),
            );
            return Err(err.to_string());
        }
        emit_progress(
            app,
            "codex",
            1,
            1,
            "Microsoft Store",
            "waiting",
            Some("请在 Microsoft Store 中完成安装"),
            None,
            None,
            Some(elapsed),
        );
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

#[cfg(target_os = "windows")]
fn installed_node_version(node_exe: &Path) -> Option<String> {
    let output = std::process::Command::new(node_exe)
        .arg("--version")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "windows")]
fn extract_node_archive(archive_path: &Path, staging_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive_path)
        .map_err(|err| format!("打开 Node.js 压缩包失败: {err}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|err| format!("Node.js 压缩包无效: {err}"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("读取 Node.js 压缩包失败: {err}"))?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| format!("Node.js 压缩包包含非法路径: {}", entry.name()))?;
        let relative: PathBuf = enclosed.components().skip(1).collect();
        if relative.as_os_str().is_empty() {
            continue;
        }
        let output = staging_dir.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&output)
                .map_err(|err| format!("创建 Node.js 目录失败: {err}"))?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("创建 Node.js 目录失败: {err}"))?;
        }
        let mut target = std::fs::File::create(&output)
            .map_err(|err| format!("创建 Node.js 文件失败: {err}"))?;
        std::io::copy(&mut entry, &mut target)
            .map_err(|err| format!("解压 Node.js 文件失败: {err}"))?;
        target
            .flush()
            .map_err(|err| format!("写入 Node.js 文件失败: {err}"))?;
    }
    if !staging_dir.join("node.exe").is_file() || !staging_dir.join("npm.cmd").is_file() {
        return Err("Node.js 压缩包缺少 node.exe 或 npm.cmd".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
async fn ensure_managed_node(app: &AppHandle, tool: &str) -> Result<PathBuf, String> {
    if std::env::consts::ARCH != "x86_64" {
        return Err(format!(
            "Windows 原生安装目前仅支持 x64，当前架构为 {}",
            std::env::consts::ARCH
        ));
    }
    let client = crate::proxy::http_client::get();
    let index = client
        .get(NODE_INDEX_URL)
        .send()
        .await
        .map_err(|err| format!("下载 Node.js 清单失败: {err}"))?
        .error_for_status()
        .map_err(|err| format!("下载 Node.js 清单失败: {err}"))?
        .text()
        .await
        .map_err(|err| format!("读取 Node.js 清单失败: {err}"))?;
    let release = select_windows_x64_lts(&index)?;
    let local_data = local_app_data_dir()?;
    let root = toolchain_root_for(&local_data);
    let node_dir = managed_node_dir_for(&local_data);
    let node_exe = node_dir.join("node.exe");
    if installed_node_version(&node_exe).as_deref() == Some(release.version.as_str()) {
        emit_progress(
            app,
            tool,
            1,
            3,
            "Node.js LTS",
            "skipped",
            Some(&format!("{} 已就绪", release.version)),
            None,
            None,
            None,
        );
        return Ok(node_dir);
    }

    std::fs::create_dir_all(&root).map_err(|err| format!("创建工具链目录失败: {err}"))?;
    let nonce = uuid::Uuid::new_v4();
    let archive_path = root.join(format!(".node-{nonce}.zip"));
    let staging_dir = root.join(format!(".node-{nonce}.staging"));
    let backup_dir = root.join(".node.previous");
    let base_url = format!("https://nodejs.org/dist/{}", release.version);
    let shasums = client
        .get(format!("{base_url}/SHASUMS256.txt"))
        .send()
        .await
        .map_err(|err| format!("下载 Node.js 校验和失败: {err}"))?
        .error_for_status()
        .map_err(|err| format!("下载 Node.js 校验和失败: {err}"))?
        .text()
        .await
        .map_err(|err| format!("读取 Node.js 校验和失败: {err}"))?;
    let expected_hash = checksum_for_archive(&shasums, &release.archive_name)?;

    let result: Result<(), String> = async {
        let response = client
            .get(format!("{base_url}/{}", release.archive_name))
            .send()
            .await
            .map_err(|err| format!("下载 Node.js 失败: {err}"))?
            .error_for_status()
            .map_err(|err| format!("下载 Node.js 失败: {err}"))?;
        let total_bytes = response.content_length();
        let mut stream = response.bytes_stream();
        let mut file = tokio::fs::File::create(&archive_path)
            .await
            .map_err(|err| format!("创建 Node.js 下载文件失败: {err}"))?;
        let mut hasher = Sha256::new();
        let mut downloaded = 0_u64;
        let mut last_percent = u64::MAX;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| format!("下载 Node.js 失败: {err}"))?;
            file.write_all(&chunk)
                .await
                .map_err(|err| format!("写入 Node.js 下载文件失败: {err}"))?;
            hasher.update(&chunk);
            downloaded += chunk.len() as u64;
            let percent = total_bytes
                .filter(|total| *total > 0)
                .map(|total| downloaded * 100 / total)
                .unwrap_or(0);
            if percent != last_percent {
                emit_progress(
                    app,
                    tool,
                    1,
                    3,
                    "Node.js LTS",
                    "waiting",
                    Some(&release.version),
                    Some(downloaded),
                    total_bytes,
                    None,
                );
                last_percent = percent;
            }
        }
        file.flush()
            .await
            .map_err(|err| format!("写入 Node.js 下载文件失败: {err}"))?;
        drop(file);
        let actual_hash = format!("{:x}", hasher.finalize());
        if actual_hash != expected_hash {
            return Err(format!(
                "Node.js SHA256 校验失败：期望 {expected_hash}，实际 {actual_hash}"
            ));
        }

        let archive = archive_path.clone();
        let staging = staging_dir.clone();
        tokio::task::spawn_blocking(move || extract_node_archive(&archive, &staging))
            .await
            .map_err(|err| format!("Node.js 解压任务失败: {err}"))??;

        if backup_dir.exists() {
            std::fs::remove_dir_all(&backup_dir)
                .map_err(|err| format!("清理旧 Node.js 备份失败: {err}"))?;
        }
        if node_dir.exists() {
            std::fs::rename(&node_dir, &backup_dir)
                .map_err(|err| format!("备份旧 Node.js 失败: {err}"))?;
        }
        if let Err(err) = std::fs::rename(&staging_dir, &node_dir) {
            if backup_dir.exists() {
                let _ = std::fs::rename(&backup_dir, &node_dir);
            }
            return Err(format!("启用新 Node.js 失败: {err}"));
        }
        if backup_dir.exists() {
            let _ = std::fs::remove_dir_all(&backup_dir);
        }
        Ok(())
    }
    .await;

    let _ = tokio::fs::remove_file(&archive_path).await;
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&staging_dir).await;
    }
    result?;
    emit_progress(
        app,
        tool,
        1,
        3,
        "Node.js LTS",
        "done",
        Some(&release.version),
        None,
        None,
        None,
    );
    Ok(node_dir)
}

#[cfg(target_os = "windows")]
fn merge_path(parts: &[&str]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut output = Vec::new();
    for part in parts {
        for segment in part.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            if seen.insert(segment.to_ascii_lowercase()) {
                output.push(segment);
            }
        }
    }
    output.join(";")
}

#[cfg(target_os = "windows")]
fn effective_install_path(node_dir: &Path, npm_prefix: &Path) -> String {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let process = std::env::var("PATH").unwrap_or_default();
    let user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .and_then(|key| key.get_value::<String, &str>("Path"))
        .unwrap_or_default();
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
        .and_then(|key| key.get_value::<String, &str>("Path"))
        .unwrap_or_default();
    merge_path(&[
        &npm_prefix.to_string_lossy(),
        &node_dir.to_string_lossy(),
        &process,
        &user,
        &machine,
    ])
}

#[cfg(target_os = "windows")]
fn persist_user_path(node_dir: &Path, npm_prefix: &Path) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, REG_EXPAND_SZ, REG_SZ};
    use winreg::types::{FromRegValue, ToRegValue};
    use winreg::RegKey;

    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Environment")
        .map_err(|err| format!("打开用户 PATH 失败: {err}"))?;
    let original = key.get_raw_value("Path").ok();
    let current = original
        .as_ref()
        .and_then(|value| String::from_reg_value(value).ok())
        .unwrap_or_default();
    let merged = merge_path(&[
        &npm_prefix.to_string_lossy(),
        &node_dir.to_string_lossy(),
        &current,
    ]);
    let mut value = merged.to_reg_value();
    value.vtype = match original.as_ref().map(|value| &value.vtype) {
        Some(REG_EXPAND_SZ) => REG_EXPAND_SZ,
        Some(REG_SZ) => REG_SZ,
        _ if merged.contains('%') => REG_EXPAND_SZ,
        _ => REG_SZ,
    };
    key.set_raw_value("Path", &value)
        .map_err(|err| format!("更新用户 PATH 失败: {err}"))?;
    std::env::set_var("PATH", effective_install_path(node_dir, npm_prefix));
    Ok(())
}

#[cfg(target_os = "windows")]
async fn install_npm_tool(app: &AppHandle, tool: &str) -> Result<i32, String> {
    let package = npm_package_for(tool).ok_or_else(|| format!("未知 npm 工具: {tool}"))?;
    emit_progress(
        app,
        tool,
        1,
        3,
        "Node.js LTS",
        "start",
        Some("查询官方 LTS 清单"),
        None,
        None,
        None,
    );
    let node_dir = ensure_managed_node(app, tool).await?;
    let local_data = local_app_data_dir()?;
    let npm_prefix = managed_npm_prefix_for(&local_data);
    std::fs::create_dir_all(&npm_prefix).map_err(|err| format!("创建 npm 目录失败: {err}"))?;
    let path = effective_install_path(&node_dir, &npm_prefix);

    emit_progress(
        app,
        tool,
        2,
        3,
        "npm 安装",
        "start",
        Some(package),
        None,
        None,
        None,
    );
    let npm = node_dir.join("npm.cmd");
    let npm_version_output = std::process::Command::new(&npm)
        .arg("--version")
        .env("PATH", &path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("读取 npm 版本失败: {err}"))?;
    if !npm_version_output.status.success() {
        return Err("读取 npm 版本失败".to_string());
    }
    let npm_version = String::from_utf8_lossy(&npm_version_output.stdout)
        .trim()
        .to_string();
    let mut command = tokio::process::Command::new(&npm);
    command.args([
        "install",
        "--global",
        "--prefix",
        npm_prefix.to_string_lossy().as_ref(),
        "--no-audit",
        "--no-fund",
        package,
    ]);
    if tool == "openclaw" && npm_requires_allow_scripts(&npm_version) {
        command.arg("--allow-scripts=openclaw");
    }
    command.env("PATH", &path);
    if let Err(err) = run_command(app, tool, &mut command, "npm 安装").await {
        emit_progress(
            app,
            tool,
            2,
            3,
            "npm 安装",
            "failed",
            Some(&err),
            None,
            None,
            None,
        );
        return Err(err);
    }
    emit_progress(
        app,
        tool,
        2,
        3,
        "npm 安装",
        "done",
        Some(package),
        None,
        None,
        None,
    );

    emit_progress(app, tool, 3, 3, "验证安装", "start", None, None, None, None);
    let verify_line = managed_cli_launch_command(tool)
        .map(|command| format!("{command} --version"))
        .ok_or_else(|| format!("npm 未生成 {tool} 启动器"))?;
    let mut verify = tokio::process::Command::new("cmd.exe");
    verify
        .args(["/D", "/S", "/C"])
        .raw_arg(verify_line)
        .env("PATH", &path);
    run_command(app, tool, &mut verify, "安装验证").await?;
    persist_user_path(&node_dir, &npm_prefix)?;
    emit_progress(app, tool, 3, 3, "验证安装", "done", None, None, None, None);
    Ok(0)
}

#[cfg(target_os = "windows")]
pub(crate) async fn install_windows_tool(app: &AppHandle, tool: &str) -> Result<i32, String> {
    match tool {
        "codex" => install_codex_store_app(app).await,
        "claude" | "hermes" => install_official_powershell_tool(app, tool).await,
        "gemini" | "opencode" | "openclaw" => install_npm_tool(app, tool).await,
        _ => Err(format!("Windows 不支持安装工具: {tool}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_latest_windows_x64_lts_release() {
        let index = r#"[
          {"version":"v27.0.0","lts":false,"files":["win-x64-zip"]},
          {"version":"v24.19.1","lts":"Krypton","files":["linux-x64","win-x64-zip"]},
          {"version":"v22.22.0","lts":"Jod","files":["win-x64-zip"]}
        ]"#;
        assert_eq!(
            select_windows_x64_lts(index).unwrap(),
            NodeRelease {
                version: "v24.19.1".to_string(),
                archive_name: "node-v24.19.1-win-x64.zip".to_string(),
            }
        );
    }

    #[test]
    fn selects_exact_archive_checksum() {
        let sums = concat!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  node-v24.19.1-win-arm64.zip\n",
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB *node-v24.19.1-win-x64.zip\n"
        );
        assert_eq!(
            checksum_for_archive(sums, "node-v24.19.1-win-x64.zip").unwrap(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }

    #[test]
    fn rejects_missing_or_invalid_checksum() {
        assert!(checksum_for_archive("xyz file.zip", "file.zip").is_err());
        assert!(checksum_for_archive("", "file.zip").is_err());
    }

    #[test]
    fn recognizes_npm_lifecycle_approval_versions() {
        assert!(!npm_requires_allow_scripts("11.15.9"));
        assert!(npm_requires_allow_scripts("11.16.0"));
        assert!(npm_requires_allow_scripts("12.0.0"));
    }

    #[test]
    fn managed_paths_stay_under_ofox_toolchain() {
        let base = Path::new(r"C:\Users\Alice\AppData\Local");
        assert_eq!(
            managed_node_dir_for(base),
            base.join("Ofox Desktop/toolchain/node")
        );
        assert_eq!(
            managed_npm_prefix_for(base),
            base.join("Ofox Desktop/toolchain/npm")
        );
    }

    #[test]
    fn maps_only_supported_npm_tools() {
        assert_eq!(npm_package_for("gemini"), Some("@google/gemini-cli@latest"));
        assert_eq!(npm_package_for("opencode"), Some("opencode-ai@latest"));
        assert_eq!(npm_package_for("openclaw"), Some("openclaw@latest"));
        assert_eq!(npm_package_for("claude"), None);
        assert_eq!(npm_package_for("codex"), None);
    }
}
