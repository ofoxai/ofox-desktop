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

use tauri::{AppHandle, Manager};

use crate::app_config::AppType;
use crate::commands::misc::launch_terminal_running;

#[cfg(target_os = "macos")]
fn workbuddy_macos_launch_command(path: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg(path);
    command
}

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

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn cli_args(tool_id: &str, bin: &str, selection: Option<(&str, &str)>) -> Vec<String> {
    let mut args = vec![bin.to_string()];

    if tool_id == "opencode" {
        if let Some((provider_id, model)) = selection {
            let provider_id = provider_id.trim();
            let model = model.trim();
            if provider_id.starts_with("ofox-") && !model.is_empty() {
                args.push("--model".to_string());
                args.push(format!("{provider_id}/{model}"));
            }
        }
    }

    args
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn build_unix_command(args: &[String]) -> String {
    let quoted_args = args
        .iter()
        .map(|arg| shell_single_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        r#"exec "${{SHELL:-/bin/zsh}}" -l -i -c 'command -v "$1" >/dev/null 2>&1 || {{ echo "[ofox-switch] $1 未安装或不在 PATH 中"; exit 127; }}; exec "$@"' ofox-launch {quoted_args}"#,
    )
}

#[tauri::command]
pub async fn launch_tool_cli(app: AppHandle, tool_id: String) -> Result<(), String> {
    let bin = resolve_cli_bin(&tool_id).ok_or_else(|| format!("工具 {tool_id} 不支持一键打开"))?;

    // Self-heal bindings created by older Ofox versions. Gemini CLI otherwise
    // stops at its first-run auth chooser despite GEMINI_API_KEY being present.
    if tool_id == "gemini" {
        crate::gemini_config::ensure_api_key_auth_selected()
            .map_err(|e| format!("同步 Gemini 认证方式失败: {e}"))?;
    }

    let selection = if tool_id == "opencode" {
        let state = app.state::<crate::store::AppState>();
        crate::commands::manage_tool::read_active_provider_and_model_for(&state, &AppType::OpenCode)
            .ok()
    } else {
        None
    };

    let args = cli_args(
        &tool_id,
        bin,
        selection
            .as_ref()
            .map(|(provider, model)| (provider.as_str(), model.as_str())),
    );

    // `exec "$SHELL" -l -i -c "…"`：交给用户的 login shell 处理 rc 加载，
    // 避开 bash 硬 source zshrc 会踩到 zsh 语法（`(N)` glob 修饰符、
    // `${(%):-%x}` prompt 展开）的坑。`-l` 触发 profile 系列，`-i` 触发
    // 交互 rc（zshrc/bashrc），跟用户手动打开终端敲命令时的环境一致。
    //
    // CLI 及其参数通过内层 shell 的位置参数传入，并由 `exec "$@"` 启动；
    // 不把 model 等值拼进 shell 程序文本，避免空格或特殊字符改变命令语义。
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let command_line = build_unix_command(&args);

    #[cfg(target_os = "windows")]
    let command_line = args.join(" ");

    launch_terminal_running(&command_line, &format!("launch_{tool_id}"))
}

/// Unified launcher: CLI tools continue to open in a terminal, while
/// desktop-app tools are started natively without shelling through a CLI.
#[tauri::command]
pub async fn launch_tool(app: AppHandle, tool_id: String) -> Result<(), String> {
    if tool_id != "workbuddy" {
        return launch_tool_cli(app, tool_id).await;
    }

    let (path, _) = crate::commands::misc::find_workbuddy_app()?;

    #[cfg(target_os = "macos")]
    {
        let status = workbuddy_macos_launch_command(&path)
            .status()
            .map_err(|error| format!("启动 WorkBuddy 失败：{error}"))?;
        if !status.success() {
            return Err(format!("启动 WorkBuddy 失败（{}）", path.display()));
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new(&path)
            .spawn()
            .map_err(|error| format!("启动 WorkBuddy 失败：{error}"))?;
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err("WorkBuddy 首版仅支持 macOS 和 Windows".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_launch_uses_active_ofox_model() {
        let args = cli_args(
            "opencode",
            "opencode",
            Some(("ofox-opencode", "openai/gpt-5.6-terra")),
        );
        assert_eq!(
            args,
            ["opencode", "--model", "ofox-opencode/openai/gpt-5.6-terra"]
        );
    }

    #[test]
    fn opencode_launch_does_not_override_non_ofox_provider() {
        let args = cli_args("opencode", "opencode", Some(("openai", "gpt-5.6-luna")));
        assert_eq!(args, ["opencode"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn workbuddy_launch_uses_the_verified_app_path() {
        use std::ffi::OsStr;

        let path = std::path::Path::new("/Applications/WorkBuddy.app");
        let command = workbuddy_macos_launch_command(path);

        assert_eq!(command.get_program(), OsStr::new("open"));
        assert_eq!(command.get_args().collect::<Vec<_>>(), [path.as_os_str()]);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn unix_command_passes_model_as_a_separate_argument() {
        let args = cli_args(
            "opencode",
            "opencode",
            Some(("ofox-opencode", "openai/gpt-5.6-terra")),
        );
        let command = build_unix_command(&args);
        assert!(command.contains("exec \"$@\""));
        assert!(command.contains("'--model' 'ofox-opencode/openai/gpt-5.6-terra'"));
    }
}
