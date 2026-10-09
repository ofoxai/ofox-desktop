//! Windows 一键安装 CLI：在一个可见的 PowerShell 窗口里跑厂商的安装器或 npm，
//! 对齐 macOS 在 Terminal 里跑安装脚本。
//!
//! Ported from magpie (https://github.com/yetone/magpie) internal/agent/install.go
//! — MIT License, Copyright (c) 2026 yetone.
//!
//! 有官方安装器的用官方的（Claude、Codex、Hermes，不需要 Node）；其余走 npm，
//! 没有 npm 时先用 winget 装 Node.js LTS，再从注册表重读 PATH（安装器写进注册表，
//! 当前窗口看不到），最后用 `npm.cmd`（`npm.ps1` 会被默认执行策略拦下）。
//! 中国大陆网络（区域 ofox.io）下官方安装器的下载源不稳定，Claude/Codex 也改走
//! npm，并对这条命令指定 npmmirror。

/// 国内 npm 镜像，只用于这一条安装命令，不写进用户的 `.npmrc`。
const NPM_MIRROR: &str = "https://registry.npmmirror.com";

/// winget 对"已经装过"返回的退出码（APPINSTALLER_CLI_ERROR_PACKAGE_ALREADY_INSTALLED）。
const WINGET_ALREADY_INSTALLED: i64 = -1978335189;

/// 安装前的环境：网络区域、有没有 npm、有没有 winget。
#[derive(Clone, Copy, Debug)]
pub(crate) struct InstallHost {
    pub mainland: bool,
    pub node_here: bool,
    pub winget_here: bool,
}

/// 各 CLI 在 npm 上的包。
fn npm_package(tool: &str) -> Option<&'static str> {
    super::tool_update::npm_package(tool)
}

/// `tool` 的安装脚本（在一个独立的 PowerShell 5.1 进程里运行）。
pub(crate) fn install_script(tool: &str, host: InstallHost) -> Result<String, String> {
    let body = match tool {
        "claude" if !host.mainland => "irm https://claude.ai/install.ps1 | iex\n".to_string(),
        "codex" if !host.mainland => concat!(
            "$env:CODEX_NON_INTERACTIVE = '1'\n",
            "irm https://chatgpt.com/codex/install.ps1 | iex\n",
        )
        .to_string(),
        "hermes" => concat!(
            "& ([scriptblock]::Create((irm https://hermes-agent.nousresearch.com/install.ps1)))",
            " -NonInteractive -SkipBrowser\n",
        )
        .to_string(),
        other => {
            let package = npm_package(other).ok_or_else(|| format!("不支持的工具: {other}"))?;
            npm_install(package, host)?
        }
    };
    Ok(format!(
        concat!(
            "$ErrorActionPreference = 'Stop'\n",
            "$ProgressPreference = 'SilentlyContinue'\n",
            "[Net.ServicePointManager]::SecurityProtocol = ",
            "[Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12\n",
            "{body}",
        ),
        body = body
    ))
}

/// `npm.cmd install -g <package>@latest`；没有 npm 时先用 winget 装 Node.js LTS。
fn npm_install(package: &str, host: InstallHost) -> Result<String, String> {
    let mut script = String::new();
    if !host.node_here {
        if !host.winget_here {
            return Err(
                "没有找到 Node.js（npm），这台电脑也没有 winget，无法自动安装。请先从 https://nodejs.org 安装 Node.js LTS，再重试。"
                    .into(),
            );
        }
        script.push_str(&format!(
            concat!(
                "winget install -e --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements\n",
                "if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne {already}) {{ throw \"winget 安装 Node.js 失败（退出码 $LASTEXITCODE）\" }}\n",
                "$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')\n",
            ),
            already = WINGET_ALREADY_INSTALLED
        ));
    }
    let registry = if host.mainland {
        format!(" --registry={NPM_MIRROR}")
    } else {
        String::new()
    };
    script.push_str(&format!(
        concat!(
            "npm.cmd install -g {package}@latest{registry}\n",
            "if ($LASTEXITCODE -ne 0) {{ throw \"npm 安装 {package} 失败（退出码 $LASTEXITCODE）\" }}\n",
        ),
        package = package,
        registry = registry
    ));
    Ok(script)
}

/// 安装脚本出错时把原因写进 `error_file`，退出码 1；正常结束退出码 0。
/// 安装窗口是独立进程，Ofox 读不到它的输出，失败原因只能这样带回来。
pub(crate) fn guarded_script(script: &str, error_file: &str) -> String {
    format!(
        concat!(
            "try {{\n{script}}} catch {{\n",
            "  $message = $_.Exception.Message\n",
            "  Set-Content -LiteralPath {error_file} -Value $message -Encoding UTF8\n",
            "  Write-Host \"错误: $message\" -ForegroundColor Red\n",
            "  exit 1\n",
            "}}\n",
            "exit 0\n",
        ),
        script = script,
        error_file = ps_quote(error_file),
    )
}

/// PowerShell 单引号字符串：`'` 写成 `''`。
pub(crate) fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 外层脚本：在独立进程里跑安装脚本（厂商脚本里的 `exit` 只会结束那个进程），
/// 把退出码写进 `done` 文件（先写临时文件再改名，读的一方不会读到半截），
/// 成功几秒后自动关窗，失败等用户按回车再关。
pub(crate) fn wrapper_script(inner: &str, done: &str, label: &str) -> String {
    let title = ps_quote(&format!("Ofox Desktop - 安装 {label}"));
    let done_tmp = ps_quote(&format!("{done}.tmp"));
    format!(
        concat!(
            "$Host.UI.RawUI.WindowTitle = {title}\n",
            "Write-Host {starting}\n",
            "& \"$PSHOME\\powershell.exe\" -NoLogo -NoProfile -ExecutionPolicy Bypass -File {inner}\n",
            "$code = $LASTEXITCODE\n",
            "if ($null -eq $code) {{ $code = 1 }}\n",
            "Set-Content -LiteralPath {done_tmp} -Value $code -Encoding ASCII\n",
            "Move-Item -LiteralPath {done_tmp} -Destination {done} -Force\n",
            "if ($code -eq 0) {{\n",
            "  Write-Host {finished}\n",
            "  Start-Sleep -Seconds 5\n",
            "}} else {{\n",
            "  Write-Host \"[ofox] 安装失败（退出码 $code），按回车关闭窗口\" -ForegroundColor Red\n",
            "  [void](Read-Host)\n",
            "}}\n",
            "Remove-Item -LiteralPath (Split-Path -Parent $PSCommandPath) -Recurse -Force -ErrorAction SilentlyContinue\n",
        ),
        title = title,
        starting = ps_quote(&format!("[ofox] 正在安装 {label} …")),
        inner = ps_quote(inner),
        done_tmp = done_tmp,
        done = ps_quote(done),
        finished = ps_quote("[ofox] 安装完成，窗口 5 秒后关闭"),
    )
}

#[cfg(target_os = "windows")]
mod runner {
    use super::{guarded_script, install_script, wrapper_script, InstallHost};
    use crate::commands::windows_tools::{effective_path, find_tool};
    use serde_json::json;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use tauri::{AppHandle, Emitter};

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    const TIMEOUT: Duration = Duration::from_secs(15 * 60);
    const POLL: Duration = Duration::from_secs(2);
    const STEPS: u32 = 2;

    fn emit_line(app: &AppHandle, tool: &str, line: &str) {
        let _ = app.emit(
            "install-tool-log",
            json!({ "tool": tool, "stream": "stdout", "line": line }),
        );
    }

    /// 与 `useToolInstall` 解析的 `ofox-install-progress` 行同格式。
    fn emit_progress(app: &AppHandle, tool: &str, step: u32, name: &str, elapsed: Option<u64>) {
        let mut progress = json!({
            "type": "ofox-install-progress",
            "step": step,
            "total": STEPS,
            "name": name,
            "phase": if elapsed.is_some() { "waiting" } else { "start" },
        });
        if let Some(elapsed) = elapsed {
            progress["elapsed"] = json!(elapsed);
            progress["timeout"] = json!(TIMEOUT.as_secs());
        }
        emit_line(app, tool, &progress.to_string());
    }

    fn host() -> InstallHost {
        let program_files_npm = std::env::var_os("ProgramFiles")
            .map(|dir| PathBuf::from(dir).join("nodejs").join("npm.cmd"))
            .is_some_and(|npm| npm.is_file());
        InstallHost {
            mainland: crate::ofox_apex::current_apex() == "ofox.io",
            node_here: find_tool("npm").is_some() || program_files_npm,
            winget_here: find_tool("winget").is_some(),
        }
    }

    /// PowerShell 5.1 按系统代码页读没有 BOM 的脚本，中文会乱码。
    fn write_with_bom(path: &Path, text: &str) -> Result<(), String> {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(text.as_bytes());
        std::fs::write(path, bytes).map_err(|e| format!("写入安装脚本失败: {e}"))
    }

    fn read_text(path: &Path) -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        let text = text.trim_start_matches('\u{feff}').trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    /// 在可见的 PowerShell 窗口里安装 `tool`，返回退出码：0 表示装完后找得到 CLI。
    /// 失败原因以 `错误: …` 行发给前端（`useToolInstall` 据此展示）。
    pub(crate) async fn install_cli(
        app: &AppHandle,
        tool: &str,
        label: &str,
        proxy_env: Vec<(String, String)>,
    ) -> i32 {
        match run(app, tool, label, proxy_env).await {
            Ok(()) => 0,
            Err(message) => {
                emit_line(app, tool, &format!("错误: {message}"));
                1
            }
        }
    }

    async fn run(
        app: &AppHandle,
        tool: &str,
        label: &str,
        proxy_env: Vec<(String, String)>,
    ) -> Result<(), String> {
        let script = install_script(tool, host())?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("ofox-install-{tool}-{stamp}"));
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建安装目录失败: {e}"))?;
        let inner = dir.join("inner.ps1");
        let error_file = dir.join("error.txt");
        let done = dir.join("done");
        let wrapper = dir.join("wrapper.ps1");
        write_with_bom(
            &inner,
            &guarded_script(&script, &error_file.to_string_lossy()),
        )?;
        write_with_bom(
            &wrapper,
            &wrapper_script(&inner.to_string_lossy(), &done.to_string_lossy(), label),
        )?;

        let step = format!("安装 {label}");
        emit_progress(app, tool, 1, &step, None);
        let mut child = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&wrapper)
            .env("PATH", effective_path())
            .envs(proxy_env)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map_err(|e| format!("打开安装窗口失败: {e}"))?;

        let started = Instant::now();
        let finished = loop {
            tokio::time::sleep(POLL).await;
            if let Some(code) = read_text(&done) {
                break Some(code);
            }
            match child.try_wait() {
                // Without a done file the window was closed before the installer finished.
                Ok(Some(_)) => break read_text(&done),
                Ok(None) => {}
                Err(e) => return Err(format!("安装窗口状态未知: {e}")),
            }
            if started.elapsed() > TIMEOUT {
                return Err("安装超过 15 分钟仍未结束，请查看安装窗口".into());
            }
            emit_progress(app, tool, 1, &step, Some(started.elapsed().as_secs()));
        };

        emit_progress(app, tool, 2, "检查安装结果", None);
        // magpie's rule: the install worked when the CLI is there now,
        // whatever the installer's exit code says.
        if find_tool(tool).is_some() {
            return Ok(());
        }
        Err(match (read_text(&error_file), finished) {
            (Some(reason), _) => reason,
            (None, Some(code)) => format!("安装器已结束（退出码 {code}），但没有找到 {label}"),
            (None, None) => "安装窗口已关闭，安装没有完成".into(),
        })
    }
}

#[cfg(target_os = "windows")]
pub(crate) use runner::install_cli;

#[cfg(test)]
mod tests {
    use super::*;

    const ABROAD: InstallHost = InstallHost {
        mainland: false,
        node_here: true,
        winget_here: true,
    };

    #[test]
    fn claude_and_codex_use_their_official_installers_abroad() {
        let claude = install_script("claude", ABROAD).unwrap();
        assert!(
            claude.contains("irm https://claude.ai/install.ps1 | iex"),
            "{claude}"
        );
        assert!(!claude.contains("npm"), "{claude}");
        let codex = install_script("codex", ABROAD).unwrap();
        assert!(
            codex.contains("$env:CODEX_NON_INTERACTIVE = '1'"),
            "{codex}"
        );
        assert!(codex.contains("irm https://chatgpt.com/codex/install.ps1 | iex"));
    }

    #[test]
    fn claude_and_codex_install_from_the_npm_mirror_in_mainland_china() {
        let host = InstallHost {
            mainland: true,
            ..ABROAD
        };
        let claude = install_script("claude", host).unwrap();
        assert!(
            claude.contains(
                "npm.cmd install -g @anthropic-ai/claude-code@latest --registry=https://registry.npmmirror.com"
            ),
            "{claude}"
        );
        assert!(!claude.contains("claude.ai/install.ps1"));
        let codex = install_script("codex", host).unwrap();
        assert!(codex.contains("npm.cmd install -g @openai/codex@latest --registry="));
    }

    #[test]
    fn hermes_runs_its_installer_without_prompts() {
        let hermes = install_script("hermes", ABROAD).unwrap();
        assert!(
            hermes.contains("& ([scriptblock]::Create((irm https://hermes-agent.nousresearch.com/install.ps1))) -NonInteractive -SkipBrowser"),
            "{hermes}"
        );
    }

    #[test]
    fn npm_tools_install_with_npm_cmd_and_check_its_exit_code() {
        let gemini = install_script("gemini", ABROAD).unwrap();
        assert!(
            gemini.contains("npm.cmd install -g @google/gemini-cli@latest\n"),
            "{gemini}"
        );
        assert!(gemini.contains("if ($LASTEXITCODE -ne 0)"));
        assert!(!gemini.contains("winget"));
        assert!(!gemini.contains("--registry"));
    }

    #[test]
    fn npm_tools_install_node_with_winget_first_when_npm_is_missing() {
        let host = InstallHost {
            node_here: false,
            ..ABROAD
        };
        let script = install_script("openclaw", host).unwrap();
        let winget = script
            .find("winget install -e --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements")
            .expect("winget step");
        let refresh = script
            .find("[Environment]::GetEnvironmentVariable('Path','Machine')")
            .expect("PATH refresh");
        let npm = script
            .find("npm.cmd install -g openclaw@latest")
            .expect("npm step");
        assert!(winget < refresh && refresh < npm, "{script}");
        assert!(
            script.contains("-1978335189"),
            "winget's already-installed code is success"
        );
    }

    #[test]
    fn npm_tools_without_npm_or_winget_ask_for_node() {
        let host = InstallHost {
            node_here: false,
            winget_here: false,
            ..ABROAD
        };
        let error = install_script("opencode", host).unwrap_err();
        assert!(error.contains("Node.js"), "{error}");
        assert!(
            install_script("claude", host).is_ok(),
            "Claude's installer needs no Node"
        );
    }

    #[test]
    fn every_script_stops_on_errors_and_unknown_tools_are_refused() {
        for tool in [
            "claude", "codex", "hermes", "gemini", "opencode", "openclaw",
        ] {
            let script = install_script(tool, ABROAD).unwrap();
            assert!(
                script.starts_with("$ErrorActionPreference = 'Stop'\n"),
                "{tool}"
            );
            assert!(script.contains("Tls12"), "{tool}");
        }
        assert!(install_script("chatgpt", ABROAD).is_err());
    }

    #[test]
    fn guarded_script_records_why_an_install_failed() {
        let guarded = guarded_script("npm.cmd install -g x\n", r"C:\Temp\t\error.txt");
        assert!(
            guarded.starts_with("try {\nnpm.cmd install -g x\n} catch {"),
            "{guarded}"
        );
        assert!(guarded.contains(r"Set-Content -LiteralPath 'C:\Temp\t\error.txt' -Value $message"));
        assert!(guarded.contains("exit 1"));
        assert!(guarded.ends_with("exit 0\n"));
    }

    #[test]
    fn ps_quote_doubles_single_quotes() {
        assert_eq!(
            ps_quote(r"C:\Users\O'Brien\x.ps1"),
            r"'C:\Users\O''Brien\x.ps1'"
        );
    }

    #[test]
    fn wrapper_runs_the_installer_in_its_own_process_and_reports_its_exit_code() {
        let wrapper = wrapper_script(
            r"C:\Temp\in st\inner.ps1",
            r"C:\Temp\in st\done",
            "Claude Code",
        );
        assert!(
            wrapper.contains(r"-ExecutionPolicy Bypass -File 'C:\Temp\in st\inner.ps1'"),
            "{wrapper}"
        );
        assert!(wrapper.contains("$code = $LASTEXITCODE"));
        assert!(wrapper.contains(r"Move-Item -LiteralPath 'C:\Temp\in st\done.tmp' -Destination 'C:\Temp\in st\done' -Force"));
        assert!(wrapper.contains("Read-Host"), "failures wait for the user");
        assert!(
            wrapper.ends_with("Remove-Item -LiteralPath (Split-Path -Parent $PSCommandPath) -Recurse -Force -ErrorAction SilentlyContinue\n"),
            "{wrapper}"
        );
        assert!(wrapper.contains("Claude Code"));
    }
}
