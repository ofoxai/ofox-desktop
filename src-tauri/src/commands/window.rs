//! 窗口控制相关命令。
//!
//! 当前只暴露 `show_main_window`，供 tray popover 等独立 webview
//! 把主窗口拉前并恢复 dock 图标（macOS）/ taskbar 显示（Windows）。

/// 把主窗口前置。复用 `tray::bring_main_window_to_front` 的平台修正。
#[tauri::command]
pub async fn show_main_window(app: tauri::AppHandle) -> Result<(), String> {
    crate::tray::bring_main_window_to_front(&app);
    Ok(())
}
