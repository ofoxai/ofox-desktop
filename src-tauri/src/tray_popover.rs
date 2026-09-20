//! Tray popover 窗口管理模块
//!
//! 负责在点击托盘图标时显示/隐藏一个自定义的 popover 窗口。

use tauri::Manager;

const POPOVER_LABEL: &str = "tray-popover";
const POPOVER_WIDTH: f64 = 360.0;
const POPOVER_HEIGHT: f64 = 620.0;

/// 切换 popover 窗口的显示/隐藏。
///
/// - 窗口已存在且可见 → 隐藏
/// - 窗口已存在但隐藏 → 重新定位并显示
/// - 窗口不存在 → 创建并显示
pub fn toggle_popover(app: &tauri::AppHandle, tray_rect: tauri::Rect) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(POPOVER_LABEL) {
        // 窗口已存在，切换可见性
        let visible = window.is_visible().unwrap_or(false);
        if visible {
            window
                .hide()
                .map_err(|e| format!("隐藏 popover 失败: {e}"))?;
        } else {
            position_window(&window, &tray_rect)?;
            window
                .show()
                .map_err(|e| format!("显示 popover 失败: {e}"))?;
            window
                .set_focus()
                .map_err(|e| format!("聚焦 popover 失败: {e}"))?;
        }
    } else {
        // 窗口不存在，创建
        create_popover(app, &tray_rect)?;
    }
    Ok(())
}

/// 创建 popover 窗口
fn create_popover(app: &tauri::AppHandle, tray_rect: &tauri::Rect) -> Result<(), String> {
    use tauri::WebviewUrl;
    use tauri::WebviewWindowBuilder;

    let url = WebviewUrl::App("index.html#/tray-popover".into());

    let window = WebviewWindowBuilder::new(app, POPOVER_LABEL, url)
        .title("")
        .inner_size(POPOVER_WIDTH, POPOVER_HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(true)
        .build()
        .map_err(|e| format!("创建 popover 窗口失败: {e}"))?;

    // 失焦自动隐藏（Rust 侧处理，比前端 onFocusChanged 更可靠）
    let window_clone = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            let _ = window_clone.hide();
        }
    });

    // macOS: 通过同步 ns_window() API 修正 NSWindow 属性
    // 必须在 show() 之前完成，确保不会闪烁
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSColor, NSWindow};

        let ptr = window
            .ns_window()
            .map_err(|e| format!("获取 NSWindow 失败: {e}"))?;
        unsafe {
            let ns_window: &NSWindow = &*(ptr as *const NSWindow);
            ns_window.setHasShadow(false);
            ns_window.setOpaque(false);
            let clear = NSColor::clearColor();
            ns_window.setBackgroundColor(Some(&clear));
        }
    }

    position_window(&window, tray_rect)?;

    window
        .show()
        .map_err(|e| format!("显示 popover 失败: {e}"))?;
    window
        .set_focus()
        .map_err(|e| format!("聚焦 popover 失败: {e}"))?;

    Ok(())
}

/// 从 `tauri::Rect` 中提取逻辑坐标（x, y, width, height）
fn extract_logical_rect(rect: &tauri::Rect, scale_factor: f64) -> (f64, f64, f64, f64) {
    let (x, y) = match &rect.position {
        tauri::Position::Physical(p) => (p.x as f64 / scale_factor, p.y as f64 / scale_factor),
        tauri::Position::Logical(l) => (l.x, l.y),
    };
    let (w, h) = match &rect.size {
        tauri::Size::Physical(p) => (
            p.width as f64 / scale_factor,
            p.height as f64 / scale_factor,
        ),
        tauri::Size::Logical(l) => (l.width, l.height),
    };
    (x, y, w, h)
}

/// 将 popover 窗口定位到 tray icon 正下方
fn position_window(window: &tauri::WebviewWindow, tray_rect: &tauri::Rect) -> Result<(), String> {
    let scale_factor = window.scale_factor().unwrap_or(1.0);
    let (tray_x, tray_y, tray_w, tray_h) = extract_logical_rect(tray_rect, scale_factor);

    // popover 居中于 tray icon 下方
    let tray_center_x = tray_x + tray_w / 2.0;
    let popover_x = tray_center_x - POPOVER_WIDTH / 2.0;
    let popover_y = tray_y + tray_h + 4.0;

    // 获取屏幕尺寸做边界限制
    let screen_width = if let Ok(Some(monitor)) = window.current_monitor() {
        let size = monitor.size();
        size.width as f64 / scale_factor
    } else {
        f64::MAX
    };

    // X 轴边界 clamp：不超出屏幕右侧，不小于 0
    let clamped_x = popover_x.max(4.0).min(screen_width - POPOVER_WIDTH - 4.0);

    // 转为物理坐标设置位置
    let physical_x = (clamped_x * scale_factor) as i32;
    let physical_y = (popover_y * scale_factor) as i32;

    window
        .set_position(tauri::PhysicalPosition::new(physical_x, physical_y))
        .map_err(|e| format!("设置 popover 位置失败: {e}"))?;

    Ok(())
}
