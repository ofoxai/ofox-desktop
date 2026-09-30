//! 干净 macOS / Windows 机器（一般是 VM）上验收 ChatGPT App 安装链路的探针。
//!
//! 平台分派通过 `commands/mod.rs` 的 `install_chatgpt_desktop_app_with` 重导出：
//!   - macOS：chatgpt_app 的 detect → download → hdiutil → codesign → ditto
//!   - Windows：windows_chatgpt 的 detect → winget/Store → Get-AppxPackage 轮询
//!
//! 进度事件用 println! emit 到 stdout（每行一条 JSON），不依赖 Tauri IPC。
//!
//! 用法（假定编译产物 scp 到 VM 上）：
//!   ./verify_chatgpt_install                 # 完整安装
//!   ./verify_chatgpt_install --check-latest  # 只读：打印 get_tool_versions 的 chatgpt 结果
//!   ./verify_chatgpt_install --running       # 只读：ChatGPT 是否在运行（仅 Windows 会检测）
//!   ./verify_chatgpt_install --upgrade       # 仅 Windows：关闭 → winget/Store 升级 → 重开
//!
//! 判定：进程 exit code 0 且最后一行是 `RESULT: ok=0`，且系统里能查到
//! ChatGPT App（macOS 下 `~/Applications/ChatGPT.app`；Windows 下
//! `Get-AppxPackage OpenAI.Codex`）。`--check-latest` 以打印的 JSON 为准
//! （version / latest_version / update_status / update_source）。

#[cfg(target_os = "windows")]
use cc_switch_lib::upgrade_chatgpt_desktop_app_with;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use cc_switch_lib::{get_tool_versions, install_chatgpt_desktop_app_with, is_tool_app_running};

fn main() {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        eprintln!("verify_chatgpt_install 只支持 macOS / Windows");
        std::process::exit(2);
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("start tokio runtime");

        if std::env::args().any(|arg| arg == "--check-latest") {
            let tools = Some(vec!["chatgpt".to_string()]);
            match runtime.block_on(get_tool_versions(tools, None, Some(true))) {
                Ok(versions) => {
                    let json = serde_json::to_string_pretty(&versions).expect("serialize");
                    println!("{json}");
                    std::process::exit(0);
                }
                Err(err) => {
                    eprintln!("ERROR: {err}");
                    std::process::exit(1);
                }
            }
        }

        if std::env::args().any(|arg| arg == "--running") {
            match runtime.block_on(is_tool_app_running("chatgpt".into())) {
                Ok(running) => {
                    println!("RUNNING: {running}");
                    std::process::exit(0);
                }
                Err(err) => {
                    eprintln!("ERROR: {err}");
                    std::process::exit(1);
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            if std::env::args().any(|arg| arg == "--upgrade") {
                let emit = |stage: &str, detail: &str| println!("PROGRESS: {stage} {detail}");
                match runtime.block_on(upgrade_chatgpt_desktop_app_with(&emit)) {
                    Ok(result) => {
                        let json = serde_json::to_string(&result).expect("serialize");
                        println!("RESULT: {json}");
                        std::process::exit(0);
                    }
                    Err(err) => {
                        println!("RESULT: err");
                        eprintln!("ERROR: {err}");
                        std::process::exit(1);
                    }
                }
            }
        }

        let emit = |line: &str| {
            println!("PROGRESS: {line}");
        };

        let result = runtime.block_on(install_chatgpt_desktop_app_with(&emit));
        match result {
            Ok(code) => {
                println!("RESULT: ok={code}");
                std::process::exit(code);
            }
            Err(err) => {
                println!("RESULT: err");
                eprintln!("ERROR: {err}");
                std::process::exit(1);
            }
        }
    }
}
