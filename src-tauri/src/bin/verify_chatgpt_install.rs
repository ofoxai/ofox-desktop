//! 干净 macOS 机器（一般是 VM）上验收 ChatGPT App 安装链路的探针。
//!
//! 直接调 `chatgpt_app::install_chatgpt_desktop_app_with(&emit)`——走通生产路径
//! 的 detect → download（含 resume/stall/retry）→ hdiutil 挂载 → codesign / Team
//! ID / Gatekeeper 校验 → ditto 到 ~/Applications/ChatGPT.app 全流程；进度事件
//! 用 println! emit 到 stdout 而不是走 Tauri IPC。
//!
//! 用法（假定编译产物 scp 到 VM 上）：
//!   ./verify_chatgpt_install
//!
//! 判定：进程 exit code 0 且最后一行是 `RESULT: ok=0`，且 ~/Applications 或
//! /Applications 下出现签名有效的 ChatGPT.app。
//!
//! 只在 macOS 编译；非 macOS 目标下 main 直接报错退出。

#[cfg(target_os = "macos")]
use cc_switch_lib::install_chatgpt_desktop_app_with;

fn main() {
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("verify_chatgpt_install 只支持 macOS");
        std::process::exit(2);
    }

    #[cfg(target_os = "macos")]
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
