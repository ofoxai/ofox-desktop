//! "打开" 按钮的后端：在系统终端里拉起工具的 CLI（`codex` / `claude` /
//! `gemini` / `opencode`），进程归属新开的 Terminal.app（或用户首选终端）
//! 窗口，独立于 Ofox 主进程——即使用户 quit 掉 Ofox，那个终端窗口和 CLI
//! 仍在运行，直到用户主动关闭。
//!
//! 实现走 [`crate::commands::misc::launch_terminal_running_with_env`]。这里的关键
//! 是"用户环境续接"：GUI 起的 Ofox 继承的是精简 PATH（`/usr/bin`、
//! `/bin`…），nvm / asdf / homebrew 装的 `codex` 通常不在里面。
//!
//! 直觉方案是"在 bash 脚本里 source ~/.zshrc"——但会引入 shell 语法冲突：
//! 用户的 zshrc 里常常有 zsh-only 语法（`~/.bun/_bun` 的 `(N)` glob
//! qualifier、`${(%):-%x}` prompt 展开等），被 bash 解析就炸。所以这里
//! 反过来：`launch_terminal_running` 的外壳是 bash（临时脚本用 `#!/bin/bash`
//! 和 `read -n`），但**真正跑 CLI 交给用户的 login shell**
//! （macOS 一般是 zsh，通过 `$SHELL` 检出），让它自己读自己的 rc、拿到
//! 用户平时手敲 `codex` 时那一整套 PATH / alias / completion 环境。

use tauri::{AppHandle, Manager};

use crate::app_config::AppType;
use crate::commands::misc::launch_terminal_running_with_env;

#[derive(Default)]
struct CliSystemProxy {
    http: Option<String>,
    https: Option<String>,
    no_proxy: Option<String>,
}

fn normalized_proxy_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    let url = if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    };
    let parsed = url::Url::parse(&url).ok()?;
    if !["http", "https", "socks5", "socks5h"].contains(&parsed.scheme())
        || parsed.host_str().is_none()
        || parsed.fragment().is_some()
    {
        return None;
    }
    Some(url)
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_macos_system_proxy(output: &str) -> CliSystemProxy {
    let mut values = std::collections::HashMap::new();
    let mut exceptions = Vec::new();
    let mut in_exceptions = false;
    for line in output.lines().map(str::trim) {
        if line.starts_with("ExceptionsList : <array> {") {
            in_exceptions = true;
            continue;
        }
        if in_exceptions {
            if line == "}" {
                in_exceptions = false;
            } else if let Some((index, value)) = line.split_once(" : ") {
                if index.parse::<usize>().is_ok() {
                    exceptions.push(value.to_string());
                }
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(" : ") {
            values.insert(key, value);
        }
    }
    let endpoint = |kind: &str| {
        let enabled_key = format!("{kind}Enable");
        if values.get(enabled_key.as_str()) != Some(&"1") {
            return None;
        }
        let host_key = format!("{kind}Proxy");
        let port_key = format!("{kind}Port");
        let host = values.get(host_key.as_str())?;
        let port = values.get(port_key.as_str())?;
        normalized_proxy_url(&format!("{host}:{port}"))
    };
    CliSystemProxy {
        http: endpoint("HTTP"),
        https: endpoint("HTTPS"),
        no_proxy: (!exceptions.is_empty()).then(|| exceptions.join(",")),
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_windows_system_proxy(server: &str, bypass: Option<&str>) -> CliSystemProxy {
    let mut http = None;
    let mut https = None;
    if server.contains('=') {
        for entry in server.split(';') {
            if let Some((kind, address)) = entry.split_once('=') {
                match kind.trim().to_ascii_lowercase().as_str() {
                    "http" => http = normalized_proxy_url(address),
                    "https" => https = normalized_proxy_url(address),
                    _ => {}
                }
            }
        }
    } else {
        http = normalized_proxy_url(server);
        https = http.clone();
    }
    CliSystemProxy {
        http,
        https,
        no_proxy: bypass
            .map(|value| {
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|part| !part.is_empty() && *part != "<local>")
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .filter(|value| !value.is_empty()),
    }
}

#[cfg(target_os = "macos")]
fn system_proxy() -> CliSystemProxy {
    std::process::Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_macos_system_proxy(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn system_proxy() -> CliSystemProxy {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let Ok(settings) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
    else {
        return CliSystemProxy::default();
    };
    if settings.get_value::<u32, _>("ProxyEnable").ok() != Some(1) {
        return CliSystemProxy::default();
    }
    let Ok(server) = settings.get_value::<String, _>("ProxyServer") else {
        return CliSystemProxy::default();
    };
    let bypass = settings.get_value::<String, _>("ProxyOverride").ok();
    parse_windows_system_proxy(&server, bypass.as_deref())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_proxy() -> CliSystemProxy {
    CliSystemProxy::default()
}

fn launch_proxy_env(
    configured: Option<&str>,
    system: CliSystemProxy,
    mut getenv: impl FnMut(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let configured = configured.and_then(normalized_proxy_url);
    let inherited = |upper: &str, lower: &str, getenv: &mut dyn FnMut(&str) -> Option<String>| {
        getenv(upper)
            .or_else(|| getenv(lower))
            .and_then(|value| normalized_proxy_url(&value))
    };
    let http = configured
        .clone()
        .or_else(|| inherited("HTTP_PROXY", "http_proxy", &mut getenv))
        .or(system.http);
    let https = configured
        .clone()
        .or_else(|| inherited("HTTPS_PROXY", "https_proxy", &mut getenv))
        .or(system.https);
    let all = configured
        .clone()
        .or_else(|| inherited("ALL_PROXY", "all_proxy", &mut getenv));
    let no_proxy = getenv("NO_PROXY")
        .or_else(|| getenv("no_proxy"))
        .or(system.no_proxy)
        .unwrap_or_default();
    let no_proxy = if http.is_some() || https.is_some() || all.is_some() {
        Some(
            [no_proxy.as_str(), "localhost,127.0.0.1,::1"]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(","),
        )
    } else {
        None
    };

    let mut result = Vec::new();
    for (key, value) in [
        ("HTTP_PROXY", http),
        ("HTTPS_PROXY", https),
        ("ALL_PROXY", all),
        ("NO_PROXY", no_proxy),
    ] {
        if let Some(value) = value {
            result.push((key.to_string(), value));
        }
    }
    result
}

/// Check only local proxy listeners. Remote proxies are left alone: a short
/// TCP probe is not a reliable health check for them.
fn unavailable_local_proxy(env_vars: &[(String, String)]) -> Option<String> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
    use std::time::Duration;

    let mut checked = std::collections::HashSet::new();
    for (name, value) in env_vars {
        if !matches!(name.as_str(), "HTTPS_PROXY" | "HTTP_PROXY" | "ALL_PROXY") {
            continue;
        }
        let Ok(url) = url::Url::parse(value) else {
            continue;
        };
        let Some(host) = url.host_str() else {
            continue;
        };
        let host = host.trim_matches(['[', ']']);
        let Some(port) = url.port_or_known_default() else {
            continue;
        };
        let addresses = if host == "localhost" {
            vec![
                SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
            ]
        } else if let Ok(ip) = host.parse::<IpAddr>() {
            if !ip.is_loopback() {
                continue;
            }
            vec![SocketAddr::from((ip, port))]
        } else {
            continue;
        };
        if !checked.insert((host.to_string(), addresses[0].port())) {
            continue;
        }
        if addresses
            .iter()
            .any(|address| TcpStream::connect_timeout(address, Duration::from_millis(200)).is_ok())
        {
            continue;
        }
        return Some(format!("{host}:{}", addresses[0].port()));
    }
    None
}

/// An explicitly configured app proxy should surface a broken local listener.
/// Automatically discovered process/OS proxies may be stale (especially in a
/// VM), so omit those endpoints and let the CLI connect directly. Always pass
/// both cases of each variable to the terminal: its login shell may otherwise
/// restore an old lowercase proxy after the outer launcher has selected direct.
fn resolved_launch_proxy_env(
    configured: Option<&str>,
    system: CliSystemProxy,
    getenv: impl FnMut(&str) -> Option<String>,
) -> Result<Vec<(String, String)>, String> {
    let explicitly_configured = configured.and_then(normalized_proxy_url).is_some();
    let mut selected = launch_proxy_env(configured, system, getenv);
    let mut skipped_local_proxy = false;

    for (name, value) in &mut selected {
        if !matches!(name.as_str(), "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY") {
            continue;
        }
        if let Some(endpoint) = unavailable_local_proxy(&[(name.clone(), value.clone())]) {
            if explicitly_configured {
                return Err(format!("LOCAL_PROXY_UNAVAILABLE|{endpoint}"));
            }
            log::info!(
                "CLI auto proxy {endpoint} is unavailable; using direct connection for {name}"
            );
            value.clear();
            skipped_local_proxy = true;
        }
    }

    let has_proxy = selected.iter().any(|(name, value)| {
        matches!(name.as_str(), "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY") && !value.is_empty()
    });
    if skipped_local_proxy && !has_proxy {
        if let Some((_, value)) = selected.iter_mut().find(|(name, _)| name == "NO_PROXY") {
            *value = "*".to_string();
        } else {
            selected.push(("NO_PROXY".to_string(), "*".to_string()));
        }
    }

    let mut result = Vec::with_capacity(8);
    for (upper, lower) in [
        ("HTTP_PROXY", "http_proxy"),
        ("HTTPS_PROXY", "https_proxy"),
        ("ALL_PROXY", "all_proxy"),
        ("NO_PROXY", "no_proxy"),
    ] {
        let value = selected
            .iter()
            .find(|(name, _)| name == upper)
            .map(|(_, value)| value.as_str())
            .unwrap_or_default()
            .to_string();
        result.push((upper.to_string(), value.clone()));
        result.push((lower.to_string(), value));
    }
    Ok(result)
}

/// The shared HTTP client also follows OS proxy settings when no in-app
/// proxy is configured. Reuse the CLI's platform-aware detection so a dead
/// loopback proxy cannot break the model catalog or account requests.
pub(crate) fn unavailable_system_proxy() -> Option<String> {
    let env_vars = launch_proxy_env(None, system_proxy(), |key| std::env::var(key).ok());
    unavailable_local_proxy(&env_vars)
}

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

#[cfg(any(target_os = "macos", target_os = "linux"))]
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
    let restore_proxy = [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ]
        .iter()
        .map(|name| {
            format!(
                "if [ \"${{OFOX_LAUNCH_{name}+x}}\" ]; then if [ -n \"$OFOX_LAUNCH_{name}\" ]; then export {name}=\"$OFOX_LAUNCH_{name}\"; else unset {name}; fi; unset OFOX_LAUNCH_{name}; fi;"
            )
        })
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        r#""${{SHELL:-/bin/zsh}}" -l -i -c '{restore_proxy} command -v "$1" >/dev/null 2>&1 || {{ echo "[ofox-switch] $1 未安装或不在 PATH 中"; exit 127; }}; exec "$@"' ofox-launch {quoted_args}"#,
    )
}

#[tauri::command]
pub async fn launch_tool_cli(app: AppHandle, tool_id: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if tool_id == "codex" && super::tool_update::probe("codex").await.is_err() {
        if let Some((path, _)) = super::tool_update::codex_desktop_version().await {
            let status = tokio::process::Command::new("/usr/bin/open")
                .arg(path)
                .status()
                .await
                .map_err(|e| e.to_string())?;
            return if status.success() {
                Ok(())
            } else {
                Err("Could not open Codex desktop app".into())
            };
        }
    }
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

    // `$SHELL -l -i -c "…"`：交给用户的 login shell 处理 rc 加载，
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

    let state = app.state::<crate::store::AppState>();
    let configured_proxy = state.db.get_global_proxy_url().ok().flatten();
    let proxy_env =
        resolved_launch_proxy_env(configured_proxy.as_deref(), system_proxy(), |key| {
            std::env::var(key).ok()
        })?;

    launch_terminal_running_with_env(&command_line, &format!("launch_{tool_id}"), &proxy_env)
}

/// Unified launcher: CLI tools continue to open in a terminal, while
/// desktop-app tools are started natively without shelling through a CLI.
#[tauri::command]
pub async fn launch_tool(app: AppHandle, tool_id: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if tool_id == "chatgpt" {
        return match super::chatgpt_app::launch_chatgpt_desktop_app() {
            Ok(true) => Ok(()),
            Ok(false) => Err("ChatGPT App 尚未安装".to_string()),
            Err(err) => Err(err),
        };
    }
    #[cfg(target_os = "windows")]
    if tool_id == "chatgpt" {
        return match super::windows_chatgpt::launch_chatgpt_desktop_app() {
            Ok(true) => Ok(()),
            Ok(false) => Err("ChatGPT App 尚未安装".to_string()),
            Err(err) => Err(err),
        };
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if tool_id == "chatgpt" {
        return Err("ChatGPT App 目前仅支持 macOS / Windows".to_string());
    }

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
    fn macos_system_proxy_parser_reads_https_and_bypass_hosts() {
        let proxy = parse_macos_system_proxy(
            "<dictionary> {\n  ExceptionsList : <array> {\n    0 : *.local\n  }\n  HTTPSEnable : 1\n  HTTPSPort : 7890\n  HTTPSProxy : 127.0.0.1\n}\n",
        );
        assert_eq!(proxy.https.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(proxy.no_proxy.as_deref(), Some("*.local"));
        assert!(proxy.http.is_none());
    }

    #[test]
    fn windows_system_proxy_parser_handles_per_protocol_endpoints() {
        let proxy = parse_windows_system_proxy(
            "http=127.0.0.1:8080;https=127.0.0.1:7890",
            Some("<local>;*.internal"),
        );
        assert_eq!(proxy.http.as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(proxy.https.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(proxy.no_proxy.as_deref(), Some("*.internal"));
    }

    #[test]
    fn launch_proxy_uses_configured_proxy_then_inherited_then_system() {
        let system = CliSystemProxy {
            http: Some("http://system:8080".to_string()),
            https: Some("http://system:7890".to_string()),
            no_proxy: Some("*.local".to_string()),
        };
        let inherited =
            |key: &str| (key == "HTTPS_PROXY").then(|| "http://process:7890".to_string());
        let env = launch_proxy_env(None, system, inherited);
        assert!(env.contains(&("HTTPS_PROXY".to_string(), "http://process:7890".to_string())));
        assert!(env.contains(&("HTTP_PROXY".to_string(), "http://system:8080".to_string())));
        assert!(env.iter().any(|(key, value)| key == "NO_PROXY"
            && value.contains("*.local")
            && value.contains("127.0.0.1")));

        let configured = launch_proxy_env(
            Some("http://configured:9000"),
            CliSystemProxy::default(),
            inherited,
        );
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            assert!(configured.contains(&(key.to_string(), "http://configured:9000".to_string())));
        }
    }

    #[test]
    fn local_proxy_preflight_reports_missing_listener_without_credentials() {
        use std::net::{Ipv4Addr, TcpListener};

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let env = vec![(
            "HTTPS_PROXY".to_string(),
            format!("http://user:secret@127.0.0.1:{port}"),
        )];
        assert_eq!(unavailable_local_proxy(&env), None);
        drop(listener);
        assert_eq!(
            unavailable_local_proxy(&env),
            Some(format!("127.0.0.1:{port}"))
        );
    }

    #[test]
    fn stale_automatic_proxy_silently_selects_direct_for_both_env_cases() {
        use std::net::{Ipv4Addr, TcpListener};

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let proxy = format!("http://127.0.0.1:{port}");
        let env = resolved_launch_proxy_env(
            None,
            CliSystemProxy {
                http: Some(proxy.clone()),
                https: Some(proxy),
                no_proxy: None,
            },
            |_| None,
        )
        .unwrap();

        for key in [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            assert_eq!(env.iter().find(|(name, _)| name == key).unwrap().1, "");
        }
        for key in ["NO_PROXY", "no_proxy"] {
            assert_eq!(env.iter().find(|(name, _)| name == key).unwrap().1, "*");
        }
    }

    #[test]
    fn stale_explicit_proxy_still_reports_the_listener() {
        use std::net::{Ipv4Addr, TcpListener};

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let error = resolved_launch_proxy_env(
            Some(&format!("http://127.0.0.1:{port}")),
            CliSystemProxy::default(),
            |_| None,
        )
        .unwrap_err();
        assert_eq!(error, format!("LOCAL_PROXY_UNAVAILABLE|127.0.0.1:{port}"));
    }

    #[test]
    fn live_automatic_proxy_is_preserved_and_lowercase_cannot_override_it() {
        use std::net::{Ipv4Addr, TcpListener};

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let proxy = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let env = resolved_launch_proxy_env(None, CliSystemProxy::default(), |key| {
            (key == "HTTPS_PROXY").then(|| proxy.clone())
        })
        .unwrap();
        for key in ["HTTPS_PROXY", "https_proxy"] {
            assert_eq!(env.iter().find(|(name, _)| name == key).unwrap().1, proxy);
        }
        assert_eq!(
            env.iter().find(|(name, _)| name == "HTTP_PROXY").unwrap().1,
            ""
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unix_login_shell_cannot_restore_a_stale_proxy_after_direct_selection() {
        let profile = tempfile::tempdir().unwrap();
        std::fs::write(
            profile.path().join(".zshrc"),
            "export HTTPS_PROXY=http://127.0.0.1:7890\nexport https_proxy=http://127.0.0.1:7890\n",
        )
        .unwrap();
        let command = build_unix_command(&["/usr/bin/env".to_string()]);
        let mut process = std::process::Command::new("/bin/bash");
        process
            .arg("-c")
            .arg(command)
            .env("SHELL", "/bin/zsh")
            .env("ZDOTDIR", profile.path());
        for (name, value) in
            resolved_launch_proxy_env(None, CliSystemProxy::default(), |_| None).unwrap()
        {
            process
                .env(&name, &value)
                .env(format!("OFOX_LAUNCH_{name}"), value);
        }
        let output = process.output().unwrap();
        assert!(output.status.success());
        let vars = String::from_utf8(output.stdout).unwrap();
        assert!(!vars.lines().any(|line| line.starts_with("HTTPS_PROXY=")));
        assert!(!vars.lines().any(|line| line.starts_with("https_proxy=")));
    }

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
        assert!(command.contains("export HTTPS_PROXY=\"$OFOX_LAUNCH_HTTPS_PROXY\""));
    }
}
