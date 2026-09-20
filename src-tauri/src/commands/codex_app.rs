//! macOS ChatGPT/Codex desktop-app lifecycle.
//!
//! The current OpenAI desktop app is installed as `ChatGPT.app`, exposes Codex,
//! and keeps the historical `com.openai.codex` bundle identifier. We prefer it
//! over the npm CLI, but retain CLI fallback in detection and launch callers.

#[cfg(target_os = "macos")]
use std::io::Write;
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::{Command, Output};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use futures::StreamExt;
#[cfg(target_os = "macos")]
use serde_json::json;
#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use tauri::{AppHandle, Emitter};

#[cfg(target_os = "macos")]
const DOWNLOAD_URL: &str = "https://persistent.oaistatic.com/codex-app-prod/Codex.dmg";
#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "com.openai.codex";
#[cfg(target_os = "macos")]
const TEAM_ID: &str = "2DC432GLL2";
#[cfg(target_os = "macos")]
const MIN_MACOS_MAJOR: u32 = 13;
#[cfg(target_os = "macos")]
const PROGRESS_TOTAL: u32 = 4;

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexDesktopApp {
    pub path: PathBuf,
    pub version: String,
}

#[cfg(target_os = "macos")]
fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(4);
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join("Applications/ChatGPT.app"));
        paths.push(home.join("Applications/Codex.app"));
    }
    paths.push(PathBuf::from("/Applications/ChatGPT.app"));
    paths.push(PathBuf::from("/Applications/Codex.app"));
    paths
}

#[cfg(target_os = "macos")]
pub(crate) fn detect_codex_desktop_app() -> Result<Option<CodexDesktopApp>, String> {
    for path in candidate_paths() {
        if !path.exists() {
            continue;
        }
        let version = verify_signed_bundle(&path)?;
        return Ok(Some(CodexDesktopApp { path, version }));
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
pub(crate) fn launch_codex_desktop_app() -> Result<bool, String> {
    let Some(installed) = detect_codex_desktop_app()? else {
        return Ok(false);
    };

    let bundle_launch = Command::new("/usr/bin/open")
        .args(["-b", BUNDLE_ID])
        .output()
        .map_err(|err| format!("启动 ChatGPT/Codex App 失败: {err}"))?;
    if bundle_launch.status.success() {
        return Ok(true);
    }

    let url_launch = Command::new("/usr/bin/open")
        .arg("codex://")
        .output()
        .map_err(|err| format!("通过 codex:// 启动失败: {err}"))?;
    if url_launch.status.success() {
        Ok(true)
    } else {
        Err(format!(
            "无法启动 {}: {}",
            installed.path.display(),
            command_error(&url_launch)
        ))
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn install_codex_desktop_app(app: &AppHandle) -> Result<i32, String> {
    match install_codex_desktop_app_inner(app).await {
        Ok(()) => Ok(0),
        Err(err) => {
            emit_progress(app, 4, "ChatGPT/Codex App", "failed", None, Some(&err));
            Err(err)
        }
    }
}

#[cfg(target_os = "macos")]
async fn install_codex_desktop_app_inner(app: &AppHandle) -> Result<(), String> {
    emit_progress(app, 1, "系统兼容性", "start", None, None);
    let product_version = macos_product_version()?;
    if std::env::consts::ARCH != "aarch64" {
        return Err(format!(
            "ChatGPT/Codex App 自动安装目前仅支持 Apple Silicon；当前架构为 {}",
            std::env::consts::ARCH
        ));
    }
    if !macos_version_supported(&product_version) {
        return Err(format!(
            "ChatGPT/Codex App 需要 macOS {MIN_MACOS_MAJOR} 或更高版本；当前为 macOS {product_version}"
        ));
    }
    emit_progress(
        app,
        1,
        "系统兼容性",
        "done",
        None,
        Some(&format!("macOS {product_version}")),
    );

    if let Some(installed) = detect_codex_desktop_app()? {
        emit_progress(
            app,
            4,
            "ChatGPT/Codex App",
            "skipped",
            None,
            Some(&format!("已安装 {}", installed.version)),
        );
        return Ok(());
    }

    let temp = tempfile::tempdir().map_err(|err| format!("创建临时目录失败: {err}"))?;
    let dmg_path = temp.path().join("ChatGPT.dmg");
    download_dmg(app, &dmg_path).await?;

    let mount_path = temp.path().join("mount");
    std::fs::create_dir(&mount_path).map_err(|err| format!("创建挂载目录失败: {err}"))?;
    emit_progress(app, 3, "校验官方安装包", "start", None, None);
    let attach = Command::new("/usr/bin/hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
        .arg(&mount_path)
        .arg(&dmg_path)
        .output()
        .map_err(|err| format!("挂载 ChatGPT DMG 失败: {err}"))?;
    if !attach.status.success() {
        return Err(format!("挂载 ChatGPT DMG 失败: {}", command_error(&attach)));
    }

    let install_result = install_from_mount(app, &mount_path);
    let detach_result = Command::new("/usr/bin/hdiutil")
        .arg("detach")
        .arg(&mount_path)
        .output()
        .map_err(|err| format!("卸载 ChatGPT DMG 失败: {err}"));

    install_result?;
    match detach_result {
        Ok(detach) if !detach.status.success() => {
            log::warn!("卸载 ChatGPT DMG 失败: {}", command_error(&detach));
        }
        Err(err) => log::warn!("{err}"),
        Ok(_) => {}
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn download_dmg(app: &AppHandle, destination: &Path) -> Result<(), String> {
    emit_progress(app, 2, "下载官方安装包", "start", None, None);
    let response = crate::proxy::http_client::get()
        .get(DOWNLOAD_URL)
        .timeout(Duration::from_secs(15 * 60))
        .send()
        .await
        .map_err(|err| format!("下载 ChatGPT DMG 失败: {err}"))?
        .error_for_status()
        .map_err(|err| format!("下载 ChatGPT DMG 失败: {err}"))?;
    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(destination)
        .map_err(|err| format!("创建 ChatGPT DMG 临时文件失败: {err}"))?;
    let mut downloaded = 0_u64;
    let mut last_emitted_percent = None;
    let mut last_emitted_bytes = 0_u64;
    let mut hasher = Sha256::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| format!("下载 ChatGPT DMG 中断: {err}"))?;
        file.write_all(&chunk)
            .map_err(|err| format!("写入 ChatGPT DMG 失败: {err}"))?;
        hasher.update(&chunk);
        downloaded = downloaded.saturating_add(chunk.len() as u64);

        let percent = total
            .filter(|value| *value > 0)
            .map(|value| downloaded.saturating_mul(100) / value);
        let bytes_checkpoint =
            percent.is_none() && downloaded.saturating_sub(last_emitted_bytes) >= 1024 * 1024;
        if percent != last_emitted_percent || bytes_checkpoint {
            emit_progress(
                app,
                2,
                "下载官方安装包",
                "waiting",
                Some((downloaded, total)),
                None,
            );
            last_emitted_percent = percent;
            last_emitted_bytes = downloaded;
        }
    }
    file.sync_all()
        .map_err(|err| format!("同步 ChatGPT DMG 失败: {err}"))?;
    if !download_length_matches(downloaded, total) {
        return Err(format!(
            "ChatGPT DMG 下载不完整：期望 {} 字节，实际 {downloaded} 字节",
            total.unwrap_or_default()
        ));
    }

    let sha256 = format!("{:x}", hasher.finalize());
    emit_progress(
        app,
        2,
        "下载官方安装包",
        "done",
        Some((downloaded, total)),
        Some(&format!("SHA-256 {sha256}")),
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_from_mount(app: &AppHandle, mount_path: &Path) -> Result<(), String> {
    let source = ["ChatGPT.app", "Codex.app"]
        .into_iter()
        .map(|name| mount_path.join(name))
        .find(|path| path.exists())
        .ok_or_else(|| "官方 DMG 中未找到 ChatGPT.app".to_string())?;

    let version = verify_trusted_bundle(&source)?;
    emit_progress(
        app,
        3,
        "校验官方安装包",
        "done",
        None,
        Some(&format!("签名有效，版本 {version}")),
    );

    emit_progress(app, 4, "安装 ChatGPT/Codex App", "start", None, None);
    let applications = dirs::home_dir()
        .ok_or_else(|| "无法获取用户主目录".to_string())?
        .join("Applications");
    std::fs::create_dir_all(&applications)
        .map_err(|err| format!("创建 ~/Applications 失败: {err}"))?;
    let destination = applications.join("ChatGPT.app");
    if destination.exists() {
        return Err(format!(
            "{} 已存在但未通过复用检测；为避免覆盖可疑应用，安装已中止",
            destination.display()
        ));
    }

    let staged = applications.join(format!(".ChatGPT.app.install-{}", uuid::Uuid::new_v4()));
    let copy = Command::new("/usr/bin/ditto")
        .arg(&source)
        .arg(&staged)
        .output()
        .map_err(|err| format!("复制 ChatGPT.app 失败: {err}"))?;
    if !copy.status.success() {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(format!("复制 ChatGPT.app 失败: {}", command_error(&copy)));
    }

    let staged_result = verify_trusted_bundle(&staged);
    if let Err(err) = staged_result {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(err);
    }
    if let Err(err) = std::fs::rename(&staged, &destination) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(format!("完成 ChatGPT.app 原子安装失败: {err}"));
    }

    emit_progress(
        app,
        4,
        "安装 ChatGPT/Codex App",
        "done",
        None,
        Some(&format!("已安装到 {}", destination.display())),
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn verify_trusted_bundle(path: &Path) -> Result<String, String> {
    let version = verify_signed_bundle(path)?;

    let assess = Command::new("/usr/sbin/spctl")
        .args(["--assess", "--type", "execute", "--verbose=4"])
        .arg(path)
        .output()
        .map_err(|err| format!("执行 Gatekeeper 校验失败: {err}"))?;
    if !assess.status.success() {
        return Err(format!(
            "ChatGPT.app 未通过 Gatekeeper/notarization 校验: {}",
            command_error(&assess)
        ));
    }
    Ok(version)
}

#[cfg(target_os = "macos")]
fn verify_signed_bundle(path: &Path) -> Result<String, String> {
    let version = verify_identity(path)?;

    let verify = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(path)
        .output()
        .map_err(|err| format!("执行 codesign 校验失败: {err}"))?;
    if !verify.status.success() {
        return Err(format!(
            "ChatGPT.app 代码签名无效: {}",
            command_error(&verify)
        ));
    }
    Ok(version)
}

#[cfg(target_os = "macos")]
fn verify_identity(path: &Path) -> Result<String, String> {
    let info_plist = path.join("Contents/Info.plist");
    let bundle_id = plist_value(&info_plist, "CFBundleIdentifier")?;
    if bundle_id != BUNDLE_ID {
        return Err(format!(
            "ChatGPT.app Bundle ID 不匹配：期望 {BUNDLE_ID}，实际 {bundle_id}"
        ));
    }

    let metadata = Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=4"])
        .arg(path)
        .output()
        .map_err(|err| format!("读取 ChatGPT.app 签名失败: {err}"))?;
    if !metadata.status.success() {
        return Err(format!(
            "读取 ChatGPT.app 签名失败: {}",
            command_error(&metadata)
        ));
    }
    let signature = format!(
        "{}\n{}",
        String::from_utf8_lossy(&metadata.stdout),
        String::from_utf8_lossy(&metadata.stderr)
    );
    let actual_team = parse_team_identifier(&signature)
        .ok_or_else(|| "ChatGPT.app 签名缺少 TeamIdentifier".to_string())?;
    if actual_team != TEAM_ID {
        return Err(format!(
            "ChatGPT.app Team ID 不匹配：期望 {TEAM_ID}，实际 {actual_team}"
        ));
    }

    plist_value(&info_plist, "CFBundleShortVersionString")
}

#[cfg(target_os = "macos")]
fn plist_value(path: &Path, key: &str) -> Result<String, String> {
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}")])
        .arg(path)
        .output()
        .map_err(|err| format!("读取 {} 失败: {err}", path.display()))?;
    if !output.status.success() {
        return Err(format!("读取 {key} 失败: {}", command_error(&output)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "macos")]
fn macos_product_version() -> Result<String, String> {
    let output = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .map_err(|err| format!("读取 macOS 版本失败: {err}"))?;
    if !output.status.success() {
        return Err(format!("读取 macOS 版本失败: {}", command_error(&output)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "macos")]
fn macos_version_supported(version: &str) -> bool {
    version
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major >= MIN_MACOS_MAJOR)
}

#[cfg(target_os = "macos")]
fn download_length_matches(downloaded: u64, total: Option<u64>) -> bool {
    total.is_none_or(|expected| expected == downloaded)
}

#[cfg(target_os = "macos")]
fn parse_team_identifier(output: &str) -> Option<&str> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("TeamIdentifier="))
        .filter(|value| !value.is_empty())
}

#[cfg(target_os = "macos")]
fn command_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        stderr
    }
}

#[cfg(target_os = "macos")]
fn emit_progress(
    app: &AppHandle,
    step: u32,
    name: &str,
    phase: &str,
    bytes: Option<(u64, Option<u64>)>,
    detail: Option<&str>,
) {
    let (downloaded, total) = bytes.unwrap_or((0, None));
    let percent = total
        .filter(|value| *value > 0)
        .map(|value| downloaded as f64 * 100.0 / value as f64);
    let mut payload = json!({
        "type": "ofox-install-progress",
        "step": step,
        "total": PROGRESS_TOTAL,
        "name": name,
        "phase": phase,
    });
    let object = payload
        .as_object_mut()
        .expect("progress payload is an object");
    if let Some((_, total)) = bytes {
        object.insert("downloadedBytes".into(), json!(downloaded));
        if let Some(total) = total {
            object.insert("totalBytes".into(), json!(total));
        }
    }
    if let Some(percent) = percent {
        object.insert("percent".into(), json!(percent));
    }
    if let Some(detail) = detail {
        object.insert("detail".into(), json!(detail));
    }

    let _ = app.emit(
        "install-tool-log",
        json!({ "tool": "codex", "stream": "stdout", "line": payload.to_string() }),
    );
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_team_identifier() {
        let output = "Identifier=com.openai.codex\nTeamIdentifier=2DC432GLL2\n";
        assert_eq!(parse_team_identifier(output), Some("2DC432GLL2"));
    }

    #[test]
    fn rejects_missing_team_identifier() {
        assert_eq!(parse_team_identifier("Identifier=com.openai.codex"), None);
    }

    #[test]
    fn macos_12_is_explicitly_unsupported() {
        assert!(!macos_version_supported("12.7.6"));
        assert!(macos_version_supported("13.0"));
        assert!(macos_version_supported("14.6.1"));
        assert!(!macos_version_supported("unknown"));
    }

    #[test]
    fn rejects_incomplete_download_when_length_is_known() {
        assert!(download_length_matches(10, Some(10)));
        assert!(!download_length_matches(9, Some(10)));
        assert!(download_length_matches(9, None));
    }
}
