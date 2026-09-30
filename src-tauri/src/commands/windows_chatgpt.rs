//! Windows ChatGPT / OpenAI Codex App 生命周期。
//!
//! Windows 上官方分发形态是 Microsoft Store MSIX 包，Package Name `OpenAI.Codex`，
//! Product ID `9PLM9XGG6VKS`，Family `OpenAI.Codex_2p2nqsd0c76g0`。detect / install /
//! launch 都靠系统自带 PowerShell 触发——不要求用户自己装 winget/MSIX tooling。
//!
//! 与 macOS 侧的 `chatgpt_app` 对齐：所有对外入口都收 `Fn(&str) + Sync` 回调
//! 而不是 `AppHandle`，方便 `verify_chatgpt_install` 探针 bin 在干净 Windows 上
//! 走同一套代码。

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::process::Command;
#[cfg(target_os = "windows")]
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use serde_json::json;

// 包身份常量与更新检测共用，定义在 chatgpt_updates（全平台编译）。
#[cfg(target_os = "windows")]
use super::chatgpt_updates::{
    classify_winget_exit, close_script, compare_dotted, desktop_update_result,
    fetch_windows_latest, parse_process_list, stop_script, windows_launch_target, WingetOutcome,
    FETCH_TIMEOUT, PACKAGE_NAME, STORE_PRODUCT_ID, WINDOWS_MANIFEST_URL,
};
#[cfg(target_os = "windows")]
use super::tool_update::{run_logged_process, UpdateResult};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 一共 3 步：兼容性 → 安装 → 复查。macOS 侧是 4 步（多一步 codesign），Windows
/// 上签名校验由 Store / MSIX 系统链路托管，我们只观测 AppxPackage 是否落地。
#[cfg(target_os = "windows")]
const PROGRESS_TOTAL: u32 = 3;

/// Store 安装总时长上限。winget 静默装 + Store 兜底轮询都套在这里面。
#[cfg(target_os = "windows")]
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// 轮询检测 AppxPackage 是否已落地的间隔。太短会打爆 PowerShell spawn；太长
/// 让"装完了但还没被识别"的窗口拉长，3s 是可接受折中。
#[cfg(target_os = "windows")]
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// winget upgrade 自身的超时。
#[cfg(target_os = "windows")]
const WINGET_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// winget 报成功后，等 AppxPackage 换成新版本的时间。
#[cfg(target_os = "windows")]
const WINGET_SETTLE: Duration = Duration::from_secs(90);

/// 回退到 Store 页面后等用户点「更新」的时间。比安装短，免得面板长时间忙碌。
#[cfg(target_os = "windows")]
const STORE_UPGRADE_WAIT: Duration = Duration::from_secs(5 * 60);

/// CloseMainWindow 之后等进程退出的时间；超时再强制结束。
#[cfg(target_os = "windows")]
const CLOSE_GRACE: Duration = Duration::from_secs(10);

/// 强制结束后等进程消失的时间。
#[cfg(target_os = "windows")]
const STOP_GRACE: Duration = Duration::from_secs(5);

/// 生产入口（`launch_tool`）——同步返回。
#[cfg(target_os = "windows")]
pub(crate) fn launch_chatgpt_desktop_app() -> Result<bool, String> {
    if detect_chatgpt_desktop_app()?.is_none() {
        return Ok(false);
    }
    let target = windows_launch_target();
    let script = format!("Start-Process '{}'", target.replace('\'', "''"));
    match run_powershell(&script) {
        Ok(_) => Ok(true),
        Err(err) => Err(format!("启动 ChatGPT App 失败: {err}")),
    }
}

/// 通用 install 入口——探针 bin 与生产 AppHandle 包装层都走这里。返回 exit
/// code（0 = 成功）。
#[cfg(target_os = "windows")]
pub async fn install_chatgpt_desktop_app_with<E>(emit: &E) -> Result<i32, String>
where
    E: Fn(&str) + Sync,
{
    match install_inner(emit).await {
        Ok(()) => Ok(0),
        Err(err) => {
            emit_progress(emit, PROGRESS_TOTAL, "ChatGPT App", "failed", Some(&err));
            Err(err)
        }
    }
}

#[cfg(target_os = "windows")]
async fn install_inner<E>(emit: &E) -> Result<(), String>
where
    E: Fn(&str) + Sync,
{
    emit_progress(emit, 1, "Windows 兼容性", "start", None);
    // MSIX 要 Windows 10 1809+ 才有 Get-AppxPackage 的稳定行为。这里不去
    // 精确 gate 版本，交给 PowerShell 自己报错——真踩到才有必要维护表。
    if !cfg!(target_pointer_width = "64") {
        return Err("ChatGPT App 需要 64 位 Windows".into());
    }
    emit_progress(emit, 1, "Windows 兼容性", "done", None);

    if let Some(version) = detect_chatgpt_desktop_app()? {
        emit_progress(
            emit,
            PROGRESS_TOTAL,
            "ChatGPT App",
            "skipped",
            Some(&format!("已安装 {version}")),
        );
        return Ok(());
    }

    emit_progress(emit, 2, "调用 winget/Store", "start", None);
    let started = Instant::now();

    // Path A：winget 静默装。用户 PATH 里没有 winget 或调用失败（比如 msstore
    // source 需要 MS Account 登录）就走 Path B。
    let winget_status = try_winget_install(emit);
    let mut waiting_for_store = false;
    if let Err(err) = winget_status {
        log::warn!("winget install failed, falling back to Store URI: {err}");
        emit_progress(
            emit,
            2,
            "打开 Microsoft Store",
            "waiting",
            Some(&format!("winget 不可用（{err}），改用 Store 手动完成")),
        );
        open_store_page()?;
        waiting_for_store = true;
    }

    // Path A/B 汇合：轮询 AppxPackage 直到出现或超时。winget 成功也要轮询，
    // 因为 Get-AppxPackage 有一小段延迟才 index 上新装的包。
    loop {
        if let Some(version) = detect_chatgpt_desktop_app()? {
            emit_progress(
                emit,
                PROGRESS_TOTAL,
                "ChatGPT App",
                "done",
                Some(&format!("已安装 {version}")),
            );
            return Ok(());
        }
        let elapsed = started.elapsed();
        if elapsed >= INSTALL_TIMEOUT {
            let msg = if waiting_for_store {
                format!(
                    "等待 Microsoft Store 完成安装超时（{}秒）；请在 Store 里手动完成 ChatGPT App 安装后重试。",
                    INSTALL_TIMEOUT.as_secs()
                )
            } else {
                "winget 触发的安装未在预期时间内完成，请检查 Windows Update / MS Account 状态后重试"
                    .into()
            };
            return Err(msg);
        }
        let stage = if waiting_for_store {
            "等待 Microsoft Store"
        } else {
            "等待 winget 安装"
        };
        emit_progress(
            emit,
            2,
            stage,
            "waiting",
            Some(&format!("已等待 {} 秒", elapsed.as_secs())),
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// 升级入口：检查 → 关闭运行中的 App → winget/Store → 等新版本出现 → 重开。
/// `emit(stage, detail)` 与 `update_tool` 的 tool-update-progress 事件一致。
#[cfg(target_os = "windows")]
pub async fn upgrade_chatgpt_desktop_app_with<E>(emit: &E) -> Result<UpdateResult, String>
where
    E: Fn(&str, &str) + Sync,
{
    emit("checking", "");
    let before = blocking(detect_chatgpt_desktop_app)
        .await?
        .ok_or("ChatGPT App 尚未安装")?;
    let client = crate::proxy::http_client::get();
    let latest = fetch_windows_latest(&client, WINDOWS_MANIFEST_URL, FETCH_TIMEOUT).await?;
    if compare_dotted(&latest, &before) != Some(std::cmp::Ordering::Greater) {
        return Ok(UpdateResult {
            status: "current".into(),
            before: before.clone(),
            after: before,
        });
    }
    let was_running = !blocking(chatgpt_process_ids).await?.is_empty();
    // 用户原本开着的 App 一定要回来：关闭失败、升级失败都一样。回退到 Store 时
    // upgrade_inner 会提前重开并清掉这个标记，免得 App 在等待期间一直关着。
    let mut reopen = was_running;
    let closed = if was_running {
        emit("log", "正在关闭 ChatGPT…");
        close_chatgpt().await
    } else {
        Ok(())
    };
    let result = match closed {
        Ok(()) => upgrade_inner(emit, &before, &mut reopen).await,
        Err(err) => Err(err),
    };
    if reopen {
        reopen_chatgpt(emit).await;
    }
    result
}

/// 重开失败只记日志，不影响升级结果。
#[cfg(target_os = "windows")]
async fn reopen_chatgpt<E>(emit: &E)
where
    E: Fn(&str, &str) + Sync,
{
    match blocking(launch_chatgpt_desktop_app).await {
        Ok(true) => emit("log", "已重新打开 ChatGPT"),
        Ok(false) => emit("log", "未检测到 ChatGPT，无法重新打开"),
        Err(err) => emit("log", &format!("重新打开 ChatGPT 失败：{err}")),
    }
}

#[cfg(target_os = "windows")]
async fn upgrade_inner<E>(emit: &E, before: &str, reopen: &mut bool) -> Result<UpdateResult, String>
where
    E: Fn(&str, &str) + Sync,
{
    emit("updating", "msstore");
    let mut command = tokio::process::Command::new("winget");
    command
        .args([
            "upgrade",
            "--id",
            STORE_PRODUCT_ID,
            "--source",
            "msstore",
            "--silent",
            "--accept-source-agreements",
            "--accept-package-agreements",
        ])
        .creation_flags(CREATE_NO_WINDOW);
    let outcome = match run_logged_process(command, |line| emit("log", line), WINGET_TIMEOUT).await
    {
        Ok(status) => classify_winget_exit(status.code()),
        Err(err) => WingetOutcome::Failed(err),
    };
    let via_store = match outcome {
        WingetOutcome::Upgraded => false,
        WingetOutcome::NotApplicable => {
            emit(
                "log",
                "winget 暂未拿到这个版本（Store 可能还没推送），改为打开 Microsoft Store",
            );
            true
        }
        WingetOutcome::Failed(err) => {
            emit(
                "log",
                &format!("winget 升级失败（{err}），改为打开 Microsoft Store"),
            );
            true
        }
    };
    if via_store {
        // Store 更新 MSIX 时自己处理运行中的 App，不必让 ChatGPT 在等待期间一直关着。
        if std::mem::take(reopen) {
            reopen_chatgpt(emit).await;
        }
        blocking(open_store_page).await?;
        emit(
            "log",
            "请在 Microsoft Store 中点击「更新」，完成后这里会自动识别",
        );
    }

    emit("verifying", "");
    let wait = if via_store {
        STORE_UPGRADE_WAIT
    } else {
        WINGET_SETTLE
    };
    let started = Instant::now();
    let mut last_report = Instant::now();
    loop {
        // MSIX 换包时可能短暂查不到，None 继续等。
        if let Some(after) = blocking(detect_chatgpt_desktop_app).await? {
            if compare_dotted(&after, before) == Some(std::cmp::Ordering::Greater) {
                return Ok(desktop_update_result(before, &after));
            }
        }
        if started.elapsed() >= wait {
            break;
        }
        if last_report.elapsed() >= Duration::from_secs(30) {
            emit(
                "log",
                &format!(
                    "等待新版本安装完成（已等待 {} 秒）",
                    started.elapsed().as_secs()
                ),
            );
            last_report = Instant::now();
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    if via_store {
        return Err(format!(
            "Microsoft Store 未在 {} 分钟内完成更新；请在 Store 中完成后回到 Ofox 点击「检查更新」",
            STORE_UPGRADE_WAIT.as_secs() / 60
        ));
    }
    // winget 报成功但版本没变：如实返回 unchanged，不冒充 updated。
    let after = blocking(detect_chatgpt_desktop_app)
        .await?
        .unwrap_or_else(|| before.to_string());
    Ok(desktop_update_result(before, &after))
}

/// 先请求正常关闭，等不到再强制结束；仍有残留就报错，不带着运行中的 App 去升级。
#[cfg(target_os = "windows")]
async fn close_chatgpt() -> Result<(), String> {
    let ids = blocking(chatgpt_process_ids).await?;
    if ids.is_empty() {
        return Ok(());
    }
    let script = close_script(&ids);
    blocking(move || run_powershell(&script)).await?;
    if wait_until_closed(CLOSE_GRACE).await? {
        return Ok(());
    }
    let ids = blocking(chatgpt_process_ids).await?;
    if !ids.is_empty() {
        let script = stop_script(&ids);
        blocking(move || run_powershell(&script)).await?;
    }
    if wait_until_closed(STOP_GRACE).await? {
        return Ok(());
    }
    Err("无法关闭 ChatGPT，请手动退出后重试".into())
}

#[cfg(target_os = "windows")]
async fn wait_until_closed(limit: Duration) -> Result<bool, String> {
    let started = Instant::now();
    loop {
        if blocking(chatgpt_process_ids).await?.is_empty() {
            return Ok(true);
        }
        if started.elapsed() >= limit {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// 运行中的 ChatGPT 包内进程（主程序以及它拉起的 codex 等子进程）。
#[cfg(target_os = "windows")]
pub(crate) fn chatgpt_process_ids() -> Result<Vec<u32>, String> {
    let script = "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);\
                  Get-Process -ErrorAction SilentlyContinue | \
                  Where-Object { $_.Path -like '*\\WindowsApps\\OpenAI.Codex_*' } | \
                  ForEach-Object { [Console]::Out.WriteLine(('{0}{1}{2}' -f $_.Id,[char]9,$_.Path)) }";
    Ok(parse_process_list(&powershell_stdout(script)?))
}

#[cfg(target_os = "windows")]
fn open_store_page() -> Result<(), String> {
    let uri = format!("ms-windows-store://pdp/?ProductId={STORE_PRODUCT_ID}");
    let script = format!("Start-Process '{}'", uri.replace('\'', "''"));
    run_powershell(&script).map_err(|e| format!("打开 Microsoft Store 失败: {e}"))
}

/// PowerShell / AppxPackage 调用都是阻塞的，放到 blocking 线程执行。
#[cfg(target_os = "windows")]
async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| format!("后台任务失败: {err}"))?
}

/// 用 winget 尝试装。返回 Ok(()) 表示 winget 至少 spawn 且退出码 0；不代表
/// AppxPackage 已经落地——那需要 detect 轮询确认。
#[cfg(target_os = "windows")]
fn try_winget_install<E>(emit: &E) -> Result<(), String>
where
    E: Fn(&str) + Sync,
{
    // 先探 winget 是否存在。cargo-installed CommandExt 上跑 --version 是安全的
    // 快速探针，避免走到 install 才发现 winget 缺失。
    let probe = Command::new("winget")
        .arg("--version")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("winget 未找到: {err}"))?;
    if !probe.status.success() {
        return Err(format!(
            "winget --version 失败: {}",
            String::from_utf8_lossy(&probe.stderr).trim()
        ));
    }

    emit_progress(emit, 2, "winget install", "waiting", Some("静默安装中"));
    let output = Command::new("winget")
        .args([
            "install",
            "--id",
            STORE_PRODUCT_ID,
            "--source",
            "msstore",
            "--silent",
            "--accept-source-agreements",
            "--accept-package-agreements",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("winget install spawn 失败: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "winget install 退出码 {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// PowerShell 探 AppxPackage 是否装了 OpenAI.Codex。返回 Some(version) 或 None。
#[cfg(target_os = "windows")]
pub(crate) fn detect_chatgpt_desktop_app() -> Result<Option<String>, String> {
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let powershell = system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    // Get-AppxPackage 返回多版本时按 version desc 排、取第一条，与用户"最新版
    // 即当前版"的直觉对齐。tab 分隔字段避免和空格路径打架。
    // PACKAGE_NAME 只允许字母数字点，脚本注入攻击面为零；这里 format! 拼进去
    // 让 PACKAGE_NAME 保持是"改名唯一入口"的角色。
    let script = format!(
        "$ErrorActionPreference='Stop';\
         [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);\
         $p=Get-AppxPackage -Name {PACKAGE_NAME} -ErrorAction SilentlyContinue | \
         Sort-Object {{[version]$_.Version}} -Descending | Select-Object -First 1;\
         if($p){{[Console]::Out.WriteLine(('{{0}}{{1}}{{2}}' -f $p.Version,[char]9,$p.PackageFamilyName))}}"
    );
    let output = Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("执行 PowerShell 失败: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "Get-AppxPackage 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_appx_identity(&String::from_utf8_lossy(&output.stdout))
}

/// 抽出来的纯函数——单测能直接喂假 stdout 打回归。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_appx_identity(output: &str) -> Result<Option<String>, String> {
    let line = output.lines().find(|l| !l.trim().is_empty()).map(str::trim);
    let Some(line) = line else { return Ok(None) };
    let mut parts = line.split('\t');
    let version = parts.next().unwrap_or("").trim();
    if version.is_empty() {
        return Err(format!("Get-AppxPackage 输出无法解析: {output}"));
    }
    Ok(Some(version.to_string()))
}

#[cfg(target_os = "windows")]
fn run_powershell(script: &str) -> Result<(), String> {
    powershell_stdout(script).map(|_| ())
}

#[cfg(target_os = "windows")]
fn powershell_stdout(script: &str) -> Result<String, String> {
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let powershell = system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let output = Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("spawn PowerShell 失败: {err}"))?;
    if !output.status.success() {
        // 这里返回类型是 String——&str 不能自动进 String，to_string() 必需。
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("PowerShell 退出码 {}", output.status)
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 与 `chatgpt_app::emit_progress` 输出格式对齐——前端解 JSON 一份 parser 通吃
/// macOS 和 Windows。区别仅 total（Windows 3 步，macOS 4 步）。
#[cfg(target_os = "windows")]
fn emit_progress<E>(emit: &E, step: u32, name: &str, phase: &str, detail: Option<&str>)
where
    E: Fn(&str) + Sync,
{
    let mut payload = json!({
        "type": "ofox-install-progress",
        "step": step,
        "total": PROGRESS_TOTAL,
        "name": name,
        "phase": phase,
    });
    if let Some(detail) = detail {
        payload
            .as_object_mut()
            .expect("progress payload is object")
            .insert("detail".into(), json!(detail));
    }
    emit(&payload.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_from_appx_output() {
        let out = "26.924.22138\tOpenAI.Codex_2p2nqsd0c76g0";
        assert_eq!(
            parse_appx_identity(out).unwrap(),
            Some("26.924.22138".into())
        );
    }

    #[test]
    fn empty_output_means_not_installed() {
        assert_eq!(parse_appx_identity("").unwrap(), None);
        assert_eq!(parse_appx_identity("   \n").unwrap(), None);
    }

    #[test]
    fn missing_version_is_error() {
        assert!(parse_appx_identity("\tOpenAI.Codex_2p2nqsd0c76g0").is_err());
    }
}
