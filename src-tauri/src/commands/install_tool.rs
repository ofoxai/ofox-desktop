//! 工具自动化安装：Tauri 调用 `scripts/installer/init.sh`（→ init.py），让 6 个
//! AI CLI 工具的安装变成 UI 上一个按钮的事情。
//!
//! 设计：
//!   - Rust 这边只做 spawn + stdout/stderr 中继。真正的安装步骤、断点续装、
//!     osascript 弹 Terminal 都在 Python 脚本里（搬自 openclaw-launcher）。
//!   - 子进程 stdout 流式通过 `install-tool-log` 事件推到前端——只是 init.py
//!     的状态行，详细 npm 日志在那个新开的 Terminal 窗口里给用户看。
//!   - 结束时 emit `install-tool-done`，前端 hook 据此触发重新扫描工具版本。
//!
//! macOS（Apple Silicon）：脚本入口 `init.sh` 第一行就 `uname -m != arm64 → exit 1`，
//! Rust 这边也用 cfg 提前拦——Intel Mac 用户得到清晰错误。
//! Windows：`windows_install` 在可见的 PowerShell 窗口里跑厂商安装器或 npm，
//! 进度与结束事件和 macOS 同格式。

#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Stdio;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use serde_json::json;
use tauri::AppHandle;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::Emitter;
#[cfg(target_os = "macos")]
use tauri::Manager;

/// 与 `scripts/installer/app/steps.py::TOOL_STEPS` 字典 key 对齐。
const ALLOWED_TOOLS: &[&str] = &[
    "claude", "codex", "gemini", "opencode", "openclaw", "hermes",
];

/// 走 Rust 原生下载 + DMG 安装，不经过 Python installer / osascript Terminal。
/// 与 `ALLOWED_TOOLS` 互斥——同一个 tool_id 不会两条路径都命中。
const NATIVE_INSTALL_TOOLS: &[&str] = &["chatgpt"];

/// Describe actual host support before offering an automatic install action.
#[tauri::command]
pub fn get_tool_install_capabilities() -> Vec<String> {
    let mut tools = Vec::new();
    if cfg!(any(
        all(target_os = "macos", target_arch = "aarch64"),
        target_os = "windows"
    )) {
        tools.extend(ALLOWED_TOOLS.iter().map(|tool| (*tool).to_string()));
    }
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) || cfg!(target_os = "windows") {
        tools.extend(NATIVE_INSTALL_TOOLS.iter().map(|tool| (*tool).to_string()));
    }
    tools
}

#[tauri::command]
pub async fn install_tool(
    app: AppHandle,
    tool_id: String,
    skip_env: Option<bool>,
) -> Result<i32, String> {
    let is_script_tool = ALLOWED_TOOLS.contains(&tool_id.as_str());
    let is_native_tool = NATIVE_INSTALL_TOOLS.contains(&tool_id.as_str());
    if !is_script_tool && !is_native_tool {
        return Err(format!("不支持的工具: {tool_id}"));
    }
    let _guard = super::tool_update::ToolOperationGuard::acquire(&tool_id)?;
    install_tool_impl(app, tool_id, skip_env, is_native_tool).await
}

/// 拆分入口以避免主命令函数在 Linux CI 下踩 clippy 的 needless_return
/// —— cfg-gate 完整覆盖每个平台下的最终表达式。
#[cfg(target_os = "macos")]
async fn install_tool_impl(
    app: AppHandle,
    tool_id: String,
    skip_env: Option<bool>,
    is_native_tool: bool,
) -> Result<i32, String> {
    if is_native_tool {
        let _ = skip_env;
        native_install(app, &tool_id).await
    } else {
        run_installer(app, tool_id, skip_env.unwrap_or(false)).await
    }
}

/// Windows 上 chatgpt 走 Microsoft Store（winget 静默 + Store URI 兜底）；CLI
/// 在可见的 PowerShell 窗口里跑厂商安装器或 npm（见 `windows_install`）。
#[cfg(target_os = "windows")]
async fn install_tool_impl(
    app: AppHandle,
    tool_id: String,
    _skip_env: Option<bool>,
    is_native_tool: bool,
) -> Result<i32, String> {
    if is_native_tool {
        return native_install_windows(app, &tool_id).await;
    }
    let code = match super::launch_tool::terminal_proxy_env(&app) {
        Ok(proxy_env) => {
            super::windows_install::install_cli(&app, &tool_id, cli_label(&tool_id), proxy_env)
                .await
        }
        Err(error) => {
            let _ = app.emit(
                "install-tool-log",
                json!({ "tool": &tool_id, "stream": "stdout", "line": format!("错误: {error}") }),
            );
            1
        }
    };
    let _ = app.emit(
        "install-tool-done",
        json!({ "tool": &tool_id, "code": code }),
    );
    Ok(code)
}

/// 安装窗口标题和提示里的工具名。
#[cfg(target_os = "windows")]
fn cli_label(tool_id: &str) -> &'static str {
    match tool_id {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "gemini" => "Gemini CLI",
        "opencode" => "OpenCode",
        "openclaw" => "OpenClaw",
        "hermes" => "Hermes",
        _ => "CLI",
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn install_tool_impl(
    _app: AppHandle,
    tool_id: String,
    _skip_env: Option<bool>,
    is_native_tool: bool,
) -> Result<i32, String> {
    if is_native_tool {
        Err(format!("{tool_id} 自动安装目前仅支持 macOS/Windows"))
    } else {
        Err("工具自动安装目前仅支持 macOS/Windows".into())
    }
}

#[cfg(target_os = "macos")]
async fn native_install(app: AppHandle, tool_id: &str) -> Result<i32, String> {
    let result = match tool_id {
        "chatgpt" => super::chatgpt_app::install_chatgpt_desktop_app(&app).await,
        other => Err(format!("原生安装未实现: {other}")),
    };
    // 与 osascript 安装分支保持一致：无论成功失败都 emit done，前端 hook 据此重扫。
    let code = result.as_ref().copied().unwrap_or(-1);
    let _ = app.emit(
        "install-tool-done",
        json!({ "tool": tool_id, "code": code }),
    );
    result
}

/// Windows 上的 native install 路由——包一层 emit 闭包，把 windows_chatgpt 的
/// progress JSON 转发到前端 `install-tool-log` 事件。行为跟 macOS 侧一致。
#[cfg(target_os = "windows")]
async fn native_install_windows(app: AppHandle, tool_id: &str) -> Result<i32, String> {
    let result = match tool_id {
        "chatgpt" => {
            let tool_id_owned = tool_id.to_string();
            let app_for_emit = app.clone();
            let emit = move |line: &str| {
                let _ = app_for_emit.emit(
                    "install-tool-log",
                    json!({ "tool": &tool_id_owned, "stream": "stdout", "line": line }),
                );
            };
            super::windows_chatgpt::install_chatgpt_desktop_app_with(&emit).await
        }
        other => Err(format!("Windows 原生安装未实现: {other}")),
    };
    let code = result.as_ref().copied().unwrap_or(-1);
    let _ = app.emit(
        "install-tool-done",
        json!({ "tool": tool_id, "code": code }),
    );
    result
}

#[cfg(target_os = "macos")]
async fn run_installer(app: AppHandle, tool_id: String, skip_env: bool) -> Result<i32, String> {
    use tokio::process::Command;

    let script_path = resolve_init_sh(&app)?;

    // 用 /bin/bash 显式启动——init.sh 头部虽然有 shebang，但走 bash 命令更
    // 稳：保证不会因为 macOS 默认 shell 切换（Catalina 之后是 zsh）导致歧义。
    let mut cmd = Command::new("/bin/bash");
    cmd.arg(&script_path)
        .arg("--tool")
        .arg(&tool_id)
        // cc-switch 不用 onboard-server（那是 openclaw-launcher 自己的配置后端）
        .arg("--no-onboard");
    if skip_env {
        cmd.arg("--skip-env");
    }
    // The GUI has no place to answer init.py's fallback "continue?" prompt.
    // In dev mode stdin can otherwise be inherited from the launching shell
    // and leave an install stuck indefinitely after a failed prerequisite.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("启动安装脚本失败: {e}"))?;

    // stdout / stderr 各起一个 task 转发到前端事件流。这里捕获的只是 init.py
    // 主进程的状态行——npm install 的真实日志在 osascript 弹出的那个 Terminal
    // 窗口里。前端按 tool_id 路由日志，不同卡片的安装并行也不会互相串。
    spawn_stream_pump(&app, child.stdout.take(), tool_id.clone(), "stdout");
    spawn_stream_pump(&app, child.stderr.take(), tool_id.clone(), "stderr");

    let status = child.wait().await.map_err(|e| e.to_string())?;
    let code = status.code().unwrap_or(-1);

    let _ = app.emit(
        "install-tool-done",
        json!({ "tool": tool_id, "code": code }),
    );

    Ok(code)
}

#[cfg(target_os = "macos")]
fn spawn_stream_pump<R>(app: &AppHandle, pipe: Option<R>, tool_id: String, stream: &'static str)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::{AsyncBufReadExt, BufReader};

    let Some(pipe) = pipe else { return };
    let app = app.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(pipe).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app.emit(
                "install-tool-log",
                json!({ "tool": tool_id, "stream": stream, "line": line }),
            );
        }
    });
}

/// 解析 init.sh 在文件系统里的实际位置。
///
/// 两种模式：
///   - 打包后 (.app)：脚本在 `<bundle>/Contents/Resources/_up_/scripts/installer/`
///   - dev 模式：脚本在 `<CARGO_MANIFEST_DIR>/../scripts/installer/`
///
/// **`_up_` 前缀是必须的**：tauri.conf.json 里资源声明为
/// `"../scripts/installer/**/*"`，Tauri 打包时会把路径里的 `..` 转义成字面
/// 目录名 `_up_`（`tauri_utils::resources` 的 `resource_relpath` 规则），
/// 所以 bundle 里的真实布局是 `Resources/_up_/scripts/...` 而**不是**
/// `Resources/scripts/...`。历史上这里只查了不带 `_up_` 的路径，导致正式版
/// 点"安装"必定 Err("找不到 init.sh")，而前端 catch 只打 console —— 表现为
/// 按钮点了完全没反应。不带 `_up_` 的候选仍保留，兼容将来资源声明改成不带
/// `../` 的写法。
///
/// 所有候选都用 `.exists()` 校验后才返回——dev 模式下 Tauri 也能成功 resolve
/// 出 Resource 路径，但文件并不存在。
#[cfg(target_os = "macos")]
fn resolve_init_sh(app: &AppHandle) -> Result<PathBuf, String> {
    // prod 路径：通过 Tauri path API 找到 Resources 下的安装脚本
    for rel in [
        "_up_/scripts/installer/init.sh",
        "scripts/installer/init.sh",
    ] {
        if let Ok(p) = app
            .path()
            .resolve(rel, tauri::path::BaseDirectory::Resource)
        {
            if p.exists() {
                return Ok(p);
            }
        }
    }

    // dev 路径：从 src-tauri/ 退一级到仓库根
    let manifest = env!("CARGO_MANIFEST_DIR");
    let dev = std::path::Path::new(manifest)
        .parent()
        .ok_or_else(|| "CARGO_MANIFEST_DIR 无 parent".to_string())?
        .join("scripts/installer/init.sh");
    if dev.exists() {
        return Ok(dev);
    }

    Err(format!(
        "找不到 init.sh —— 既不在 Resources（_up_/scripts/installer/ 或 \
         scripts/installer/），也不在 {}",
        dev.display()
    ))
}
