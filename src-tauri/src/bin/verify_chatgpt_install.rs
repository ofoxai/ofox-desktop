//! 干净 macOS / Windows 机器（一般是 VM）上验收 ChatGPT App 安装链路的探针。
//!
//! 平台分派通过 `commands/mod.rs` 的 `install_chatgpt_desktop_app_with` 重导出：
//!   - macOS：chatgpt_app 的 detect → download → hdiutil → codesign → ditto
//!   - Windows：windows_chatgpt 的 detect → winget/Store → Get-AppxPackage 轮询
//!
//! 进度事件用 println! emit 到 stdout（每行一条 JSON），不依赖 Tauri IPC。
//!
//! 用法（假定编译产物 scp 到 VM 上）：
//!   ./verify_chatgpt_install
//!
//! 判定：进程 exit code 0 且最后一行是 `RESULT: ok=0`，且系统里能查到
//! ChatGPT App（macOS 下 `~/Applications/ChatGPT.app`；Windows 下
//! `Get-AppxPackage OpenAI.Codex`）。

#[cfg(any(target_os = "macos", target_os = "windows"))]
use cc_switch_lib::install_chatgpt_desktop_app_with;

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
