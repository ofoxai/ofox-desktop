//! macOS ChatGPT desktop-app lifecycle (independent tool from the `codex` CLI).
//!
//! `chatgpt` is a separate tool_id from `codex`. codex CLI stays installable via
//! npm; chatgpt installs the official signed ChatGPT.app (Bundle ID
//! `com.openai.codex`, Team ID `2DC432GLL2`). The two share `~/.codex/config.toml`
//! so a user who bound `codex` will find the app already usable without a second
//! bind — that's why `chatgpt` is intentionally left out of `OFOX_AUTO_BIND_TOOLS`
//! on the frontend.

#[cfg(target_os = "macos")]
use std::io::{Seek, SeekFrom, Write};
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::{Command, Output};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

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
const TOOL_ID: &str = "chatgpt";
/// 尝试次数上限。国内网络最常见的失败模式是 TCP reset / TLS handshake 半路
/// 断，直连一次不成功再试往往就通了；三次没通基本就是这台机器/时段真的
/// 到不了，用户手动去官网下更靠谱。
#[cfg(target_os = "macos")]
const DOWNLOAD_ATTEMPTS: u32 = 3;
/// 每次尝试的 wall-clock 上限。原代码用 15 min 是"整个下载"的上限；引入
/// resume 之后每次尝试只需要覆盖剩余部分，10 min 已经宽裕。
#[cfg(target_os = "macos")]
const DOWNLOAD_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// 连续 60 秒读不到任何字节，就认为链路 stall——直接中断本次尝试进入 resume
/// 分支。CDN 命中普遍 3 MB/s，stall 60s 意味着掉线，重试通常就恢复。
#[cfg(target_os = "macos")]
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatGptDesktopApp {
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
pub(crate) fn detect_chatgpt_desktop_app() -> Result<Option<ChatGptDesktopApp>, String> {
    for path in candidate_paths() {
        if !path.exists() {
            continue;
        }
        let version = verify_signed_bundle(&path)?;
        return Ok(Some(ChatGptDesktopApp { path, version }));
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
pub(crate) fn launch_chatgpt_desktop_app() -> Result<bool, String> {
    let Some(installed) = detect_chatgpt_desktop_app()? else {
        return Ok(false);
    };

    let bundle_launch = Command::new("/usr/bin/open")
        .args(["-b", BUNDLE_ID])
        .output()
        .map_err(|err| format!("启动 ChatGPT App 失败: {err}"))?;
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

/// AppHandle 入口——生产路径。构建一个把 progress payload 转发到前端
/// `install-tool-log` 事件的 emit 闭包，然后走通用 install 流程。
#[cfg(target_os = "macos")]
pub(crate) async fn install_chatgpt_desktop_app(app: &AppHandle) -> Result<i32, String> {
    let emit = |line: &str| {
        let _ = app.emit(
            "install-tool-log",
            json!({ "tool": TOOL_ID, "stream": "stdout", "line": line }),
        );
    };
    install_chatgpt_desktop_app_with(&emit).await
}

/// 通用 install 入口——bin/verify_chatgpt_install 用这条走 println 触发实际
/// 下载/校验/安装，绕开 Tauri AppHandle。返回 exit code；0 = 成功，其它 = 失败。
#[cfg(target_os = "macos")]
pub async fn install_chatgpt_desktop_app_with<E>(emit: &E) -> Result<i32, String>
where
    E: Fn(&str) + Sync,
{
    match install_chatgpt_desktop_app_inner(emit).await {
        Ok(()) => Ok(0),
        Err(err) => {
            emit_progress(emit, 4, "ChatGPT App", "failed", None, Some(&err));
            Err(err)
        }
    }
}

#[cfg(target_os = "macos")]
async fn install_chatgpt_desktop_app_inner<E>(emit: &E) -> Result<(), String>
where
    E: Fn(&str) + Sync,
{
    emit_progress(emit, 1, "系统兼容性", "start", None, None);
    let product_version = macos_product_version()?;
    if std::env::consts::ARCH != "aarch64" {
        return Err(format!(
            "ChatGPT App 自动安装目前仅支持 Apple Silicon；当前架构为 {}",
            std::env::consts::ARCH
        ));
    }
    if !macos_version_supported(&product_version) {
        return Err(format!(
            "ChatGPT App 需要 macOS {MIN_MACOS_MAJOR} 或更高版本；当前为 macOS {product_version}"
        ));
    }
    emit_progress(
        emit,
        1,
        "系统兼容性",
        "done",
        None,
        Some(&format!("macOS {product_version}")),
    );

    if let Some(installed) = detect_chatgpt_desktop_app()? {
        emit_progress(
            emit,
            4,
            "ChatGPT App",
            "skipped",
            None,
            Some(&format!("已安装 {}", installed.version)),
        );
        return Ok(());
    }

    let temp = tempfile::tempdir().map_err(|err| format!("创建临时目录失败: {err}"))?;
    let dmg_path = temp.path().join("ChatGPT.dmg");
    download_dmg(emit, &dmg_path).await?;

    let mount_path = temp.path().join("mount");
    std::fs::create_dir(&mount_path).map_err(|err| format!("创建挂载目录失败: {err}"))?;
    emit_progress(emit, 3, "校验官方安装包", "start", None, None);
    let attach = Command::new("/usr/bin/hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
        .arg(&mount_path)
        .arg(&dmg_path)
        .output()
        .map_err(|err| format!("挂载 ChatGPT DMG 失败: {err}"))?;
    if !attach.status.success() {
        return Err(format!("挂载 ChatGPT DMG 失败: {}", command_error(&attach)));
    }

    let install_result = install_from_mount(emit, &mount_path);
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

/// 下载协调层：三次尝试 + resume 兜底。所有失败最终转成"用户可以自己
/// 修的"错误消息（含官方 URL 供手动下载），不再抛裸的 reqwest 错误。
#[cfg(target_os = "macos")]
async fn download_dmg<E>(emit: &E, destination: &Path) -> Result<(), String>
where
    E: Fn(&str) + Sync,
{
    emit_progress(emit, 2, "下载官方安装包", "start", None, None);

    // hasher 必须跨 attempt 保持——resume 是从磁盘上已存字节继续算，
    // 每次尝试都新建 hasher 会导致最终 SHA-256 只覆盖最后一段。
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let mut last_error: Option<String> = None;

    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        match download_attempt(emit, destination, &mut hasher, downloaded, attempt).await {
            Ok((final_downloaded, total)) => {
                let sha256 = format!("{:x}", hasher.finalize());
                emit_progress(
                    emit,
                    2,
                    "下载官方安装包",
                    "done",
                    Some((final_downloaded, total)),
                    Some(&format!("SHA-256 {sha256}")),
                );
                return Ok(());
            }
            Err(DownloadFailure { transferred, error }) => {
                downloaded = transferred;
                log::warn!(
                    "ChatGPT DMG 第 {attempt}/{DOWNLOAD_ATTEMPTS} 次下载失败（已收 {downloaded} 字节）：{error}"
                );
                last_error = Some(error);
                if attempt < DOWNLOAD_ATTEMPTS {
                    // 指数退避：1s → 3s → 5s。别太长——用户在盯着看。
                    let backoff = Duration::from_secs(2u64.saturating_mul(attempt as u64) - 1);
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }

    Err(format!(
        "ChatGPT DMG 下载失败（已重试 {DOWNLOAD_ATTEMPTS} 次）：{}\n\n手动下载入口：{DOWNLOAD_URL}\n\
         或访问官方页面 https://chatgpt.com/download/ 选 macOS 版，把 ChatGPT.app 拖到 ~/Applications 即可。",
        last_error.unwrap_or_else(|| "未知错误".to_string())
    ))
}

/// 单次下载尝试的返回值——`transferred` 是本轮结束时磁盘上确认写入的字节
/// 数，无论成功还是失败都要报，好让上层 resume 从这里接着来。
#[cfg(target_os = "macos")]
struct DownloadFailure {
    transferred: u64,
    error: String,
}

#[cfg(target_os = "macos")]
async fn download_attempt<E>(
    emit: &E,
    destination: &Path,
    hasher: &mut Sha256,
    resume_from: u64,
    attempt: u32,
) -> Result<(u64, Option<u64>), DownloadFailure>
where
    E: Fn(&str) + Sync,
{
    // 每次尝试都用 Range: bytes=N-；首次 N=0 服务端返回 200 + full body，
    // 之后 N>0 返回 206 + partial，两条路径都被 reqwest 正常收流。
    let mut request = crate::proxy::http_client::get()
        .get(DOWNLOAD_URL)
        .timeout(DOWNLOAD_ATTEMPT_TIMEOUT);
    if resume_from > 0 {
        request = request.header("Range", format!("bytes={resume_from}-"));
    }

    let response = match request.send().await {
        Ok(resp) => resp,
        Err(err) => {
            return Err(DownloadFailure {
                transferred: resume_from,
                error: format!("发起请求失败: {err}"),
            });
        }
    };
    let response = match response.error_for_status() {
        Ok(resp) => resp,
        Err(err) => {
            return Err(DownloadFailure {
                transferred: resume_from,
                error: format!("HTTP 状态错误: {err}"),
            });
        }
    };

    // content-length 对 206 表示的是本轮 body 长度，不是整个文件；换算到"整体
    // 大小"用来算 progress 百分比。首次响应（resume_from=0）直接就是 total。
    let remaining_len = response.content_length();
    let total = remaining_len.map(|len| len.saturating_add(resume_from));

    let mut file = if resume_from > 0 {
        let mut f = match std::fs::OpenOptions::new().write(true).open(destination) {
            Ok(f) => f,
            Err(err) => {
                return Err(DownloadFailure {
                    transferred: resume_from,
                    error: format!("打开已下载的临时文件失败: {err}"),
                });
            }
        };
        // seek 到已下载末尾续写；不 truncate，避免抹掉前面的 partial。
        if let Err(err) = f.seek(SeekFrom::Start(resume_from)) {
            return Err(DownloadFailure {
                transferred: resume_from,
                error: format!("续传定位失败: {err}"),
            });
        }
        f
    } else {
        match std::fs::File::create(destination) {
            Ok(f) => f,
            Err(err) => {
                return Err(DownloadFailure {
                    transferred: 0,
                    error: format!("创建临时文件失败: {err}"),
                });
            }
        }
    };

    let mut stream = response.bytes_stream();
    let mut downloaded = resume_from;
    let mut last_emitted_percent = None;
    let mut last_emitted_bytes = downloaded;
    let mut last_progress_at = Instant::now();

    let stall_label = if attempt == 1 {
        "下载官方安装包".to_string()
    } else {
        format!("续传官方安装包（第 {attempt} 次）")
    };

    loop {
        // 每 chunk 上一层套 stall timeout——单个 chunk 之间静默超过 STALL_TIMEOUT
        // 就把这次尝试算失败，让 resume 分支接手。
        let chunk = match tokio::time::timeout(STALL_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(chunk))) => chunk,
            Ok(Some(Err(err))) => {
                return Err(DownloadFailure {
                    transferred: downloaded,
                    error: format!("下载中断: {err}"),
                });
            }
            Ok(None) => break,
            Err(_) => {
                return Err(DownloadFailure {
                    transferred: downloaded,
                    error: format!(
                        "{} 秒没有新数据到达，判定链路 stall",
                        STALL_TIMEOUT.as_secs()
                    ),
                });
            }
        };

        if let Err(err) = file.write_all(&chunk) {
            return Err(DownloadFailure {
                transferred: downloaded,
                error: format!("写入临时文件失败: {err}"),
            });
        }
        hasher.update(&chunk);
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        last_progress_at = Instant::now();

        let percent = total
            .filter(|value| *value > 0)
            .map(|value| downloaded.saturating_mul(100) / value);
        let bytes_checkpoint =
            percent.is_none() && downloaded.saturating_sub(last_emitted_bytes) >= 1024 * 1024;
        if percent != last_emitted_percent || bytes_checkpoint {
            emit_progress(
                emit,
                2,
                &stall_label,
                "waiting",
                Some((downloaded, total)),
                None,
            );
            last_emitted_percent = percent;
            last_emitted_bytes = downloaded;
        }
    }

    // 防御性——防止 sync_all 失败时把上面已成功的字节丢失。
    if let Err(err) = file.sync_all() {
        return Err(DownloadFailure {
            transferred: downloaded,
            error: format!("同步临时文件失败: {err}"),
        });
    }
    if !download_length_matches(downloaded, total) {
        return Err(DownloadFailure {
            transferred: downloaded,
            error: format!(
                "下载不完整：期望 {} 字节，实际 {downloaded} 字节",
                total.unwrap_or_default()
            ),
        });
    }
    let _ = last_progress_at; // silence unused warning; kept for potential future stall tuning
    Ok((downloaded, total))
}

#[cfg(target_os = "macos")]
fn install_from_mount<E>(emit: &E, mount_path: &Path) -> Result<(), String>
where
    E: Fn(&str) + Sync,
{
    let source = ["ChatGPT.app", "Codex.app"]
        .into_iter()
        .map(|name| mount_path.join(name))
        .find(|path| path.exists())
        .ok_or_else(|| "官方 DMG 中未找到 ChatGPT.app".to_string())?;

    let version = verify_trusted_bundle(&source)?;
    emit_progress(
        emit,
        3,
        "校验官方安装包",
        "done",
        None,
        Some(&format!("签名有效，版本 {version}")),
    );

    emit_progress(emit, 4, "安装 ChatGPT App", "start", None, None);
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
        emit,
        4,
        "安装 ChatGPT App",
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
pub(crate) fn macos_product_version() -> Result<String, String> {
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

/// 把一条 progress 事件封成 JSON payload 字符串，交给 caller 传进来的
/// emit 闭包。生产路径下 emit 会调 `app.emit("install-tool-log", ...)`；
/// 探针路径下 emit 就是 println!，输出给 stdout 便于 tail -f。
#[cfg(target_os = "macos")]
fn emit_progress<E>(
    emit: &E,
    step: u32,
    name: &str,
    phase: &str,
    bytes: Option<(u64, Option<u64>)>,
    detail: Option<&str>,
) where
    E: Fn(&str) + Sync,
{
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

    emit(&payload.to_string());
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// 真机探测手册测试：不是 CI 单测，也不是 mock 校验——直接对当前用户
    /// 的 filesystem 跑 detect + 校验，作为"新增 chatgpt 工具"落地时验证
    /// candidate_paths / codesign / Team ID 三条约束在本机上确实能命中的
    /// 简单办法。默认 ignore；`cargo test --lib -- --ignored manual_detect_chatgpt`
    /// 触发。
    #[test]
    #[ignore = "Real-machine acceptance: reads /Applications/ChatGPT.app on this host"]
    fn manual_detect_chatgpt() {
        match detect_chatgpt_desktop_app() {
            Ok(Some(app)) => println!(
                "[manual_detect_chatgpt] found path={} version={}",
                app.path.display(),
                app.version
            ),
            Ok(None) => println!("[manual_detect_chatgpt] not installed"),
            Err(err) => panic!("[manual_detect_chatgpt] error: {err}"),
        }
    }

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
