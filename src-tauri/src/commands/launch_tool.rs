//! "打开" 按钮的后端：在系统终端里拉起工具的 CLI（`codex` / `claude` /
//! `gemini` / `opencode`），进程归属新开的 Terminal.app（或用户首选终端）
//! 窗口，独立于 Ofox 主进程——即使用户 quit 掉 Ofox，那个终端窗口和 CLI
//! 仍在运行，直到用户主动关闭。
//!
//! 实现走 [`crate::commands::misc::launch_terminal_running`]。这里的关键
//! 是"用户环境续接"：GUI 起的 Ofox 继承的是精简 PATH（`/usr/bin`、
//! `/bin`…），nvm / asdf / homebrew 装的 `codex` 通常不在里面。
//!
//! 直觉方案是"在 bash 脚本里 source ~/.zshrc"——但会引入 shell 语法冲突：
//! 用户的 zshrc 里常常有 zsh-only 语法（`~/.bun/_bun` 的 `(N)` glob
//! qualifier、`${(%):-%x}` prompt 展开等），被 bash 解析就炸。所以这里
//! 反过来：`launch_terminal_running` 的外壳是 bash（临时脚本用 `#!/bin/bash`
//! 和 `read -n`），但**真正跑 CLI 用 `exec` 换到用户的 login shell**
//! （macOS 一般是 zsh，通过 `$SHELL` 检出），让它自己读自己的 rc、拿到
//! 用户平时手敲 `codex` 时那一整套 PATH / alias / completion 环境。

use tauri::AppHandle;

use crate::commands::misc::launch_terminal_running;

/// 与 `TOOL_META` 的 `cliBin` 字段保持一致。openclaw/hermes 不在此列——
/// openclaw 走 gateway 不需要交互式 CLI，hermes 是 dashboard 服务，另有
/// `launch_hermes_dashboard` 命令。
fn resolve_cli_bin(tool_id: &str) -> Option<&'static str> {
    match tool_id {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        "gemini" => Some("gemini"),
        "opencode" => Some("opencode"),
        _ => None,
    }
}

#[tauri::command]
pub async fn launch_tool_cli(_app: AppHandle, tool_id: String) -> Result<(), String> {
    let bin = resolve_cli_bin(&tool_id)
        .ok_or_else(|| format!("工具 {tool_id} 不支持一键打开"))?;

    // `exec "$SHELL" -l -i -c "…"`：交给用户的 login shell 处理 rc 加载，
    // 避开 bash 硬 source zshrc 会踩到 zsh 语法（`(N)` glob 修饰符、
    // `${(%):-%x}` prompt 展开）的坑。`-l` 触发 profile 系列，`-i` 触发
    // 交互 rc（zshrc/bashrc），跟用户手动打开终端敲命令时的环境一致。
    //
    // 里层用单引号包裹 bin 名简化转义——bin 是本模块内的白名单常量，
    // 不涉及用户输入，注入面为零。
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let command_line = format!(
        r#"exec "${{SHELL:-/bin/zsh}}" -l -i -c 'command -v {bin} >/dev/null 2>&1 || {{ echo "[ofox-switch] {bin} 未安装或不在 PATH 中"; exit 127; }}; exec {bin}'"#,
        bin = bin,
    );

    #[cfg(target_os = "windows")]
    let command_line = bin.to_string();

    launch_terminal_running(&command_line, &format!("launch_{tool_id}"))
}
