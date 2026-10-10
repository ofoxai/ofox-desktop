//! Tray popover 窗口管理模块
//!
//! 负责在点击托盘图标时显示/隐藏一个自定义的 popover 窗口。

use tauri::Manager;

const POPOVER_LABEL: &str = "tray-popover";
/// macOS 的窗口透明，卡片四周留 16px 边距（前端 `PopoverFrame` 的 `p-4`）。
#[cfg(target_os = "macos")]
const POPOVER_WIDTH: f64 = 360.0;
#[cfg(target_os = "macos")]
const POPOVER_HEIGHT: f64 = 620.0;
/// 其它平台窗口本身就是不透明面板，取 macOS 卡片的大小。
#[cfg(not(target_os = "macos"))]
const POPOVER_WIDTH: f64 = 328.0;
#[cfg(not(target_os = "macos"))]
const POPOVER_HEIGHT: f64 = 588.0;

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

/// 创建并显示 popover 窗口
fn create_popover(app: &tauri::AppHandle, tray_rect: &tauri::Rect) -> Result<(), String> {
    let window = build_popover(app, true)?;
    position_window(&window, tray_rect)?;

    window
        .show()
        .map_err(|e| format!("显示 popover 失败: {e}"))?;
    window
        .set_focus()
        .map_err(|e| format!("聚焦 popover 失败: {e}"))?;

    Ok(())
}

/// Windows：启动后先把 popover 建好藏着，第一次点托盘图标就只是显示它。
///
/// 当场创建时，WebView2 初始化期间会把焦点移进网页，窗口刚显示就收到一次
/// `Focused(false)`，被下面的失焦隐藏立刻藏掉——第一次点击只闪一下，第二次
/// （窗口已存在）才正常。预先建好、建的时候不抢焦点，就绕开了这一步。
#[cfg(target_os = "windows")]
pub fn prewarm(app: &tauri::AppHandle) {
    if app.get_webview_window(POPOVER_LABEL).is_some() {
        return;
    }
    if let Err(e) = build_popover(app, false) {
        log::warn!("预先创建 tray popover 失败，首次点击时再创建: {e}");
    }
}

/// 建一个隐藏的 popover 窗口；`focused` 决定创建时是否获取焦点。
fn build_popover(app: &tauri::AppHandle, focused: bool) -> Result<tauri::WebviewWindow, String> {
    use tauri::WebviewUrl;
    use tauri::WebviewWindowBuilder;

    let url = WebviewUrl::App("index.html#/tray-popover".into());

    let window = WebviewWindowBuilder::new(app, POPOVER_LABEL, url)
        .title("")
        .inner_size(POPOVER_WIDTH, POPOVER_HEIGHT)
        .resizable(false)
        .decorations(false)
        // Windows 给无边框窗口加边框和阴影，透明边距会被描成一圈框，所以只有 macOS 透明。
        .transparent(cfg!(target_os = "macos"))
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(focused)
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

    Ok(window)
}

/// popover 与托盘图标、屏幕可用区域边缘之间的间距（逻辑像素）。
const EDGE_GAP: f64 = 4.0;

/// 物理像素下的矩形。
#[derive(Debug, Clone, Copy, PartialEq)]
struct PxRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl PxRect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// `tauri::Rect` 转物理像素；逻辑坐标按 `scale_factor` 换算。
fn physical_rect(rect: &tauri::Rect, scale_factor: f64) -> PxRect {
    let (x, y) = match &rect.position {
        tauri::Position::Physical(p) => (p.x as f64, p.y as f64),
        tauri::Position::Logical(l) => (l.x * scale_factor, l.y * scale_factor),
    };
    let (w, h) = match &rect.size {
        tauri::Size::Physical(p) => (p.width as f64, p.height as f64),
        tauri::Size::Logical(l) => (l.width * scale_factor, l.height * scale_factor),
    };
    PxRect { x, y, w, h }
}

/// popover 左上角：顶部菜单栏（macOS）的图标往下弹，底部任务栏（Windows）的
/// 图标往上弹；最后整体限制在显示器可用区域内，不让任何一部分落到屏幕外或任务栏下。
fn popover_origin(tray: PxRect, work: PxRect, size: (f64, f64), gap: f64) -> (f64, f64) {
    let (width, height) = size;
    // 放不下时取下限，至少保证左上角（标题和账户信息）可见。
    let clamp = |value: f64, low: f64, high: f64| value.min(high).max(low);
    let x = clamp(
        tray.x + tray.w / 2.0 - width / 2.0,
        work.x + gap,
        work.x + work.w - width - gap,
    );
    let below = tray.y + tray.h + gap;
    let y = if below + height <= work.y + work.h - gap {
        below
    } else {
        tray.y - height - gap
    };
    let y = clamp(y, work.y + gap, work.y + work.h - height - gap);
    (x, y)
}

/// 将 popover 窗口定位到托盘图标旁，并保持在图标所在显示器的可用区域内
fn position_window(window: &tauri::WebviewWindow, tray_rect: &tauri::Rect) -> Result<(), String> {
    let tray = physical_rect(tray_rect, window.scale_factor().unwrap_or(1.0));
    let (center_x, center_y) = (tray.x + tray.w / 2.0, tray.y + tray.h / 2.0);
    let monitor = window
        .available_monitors()
        .ok()
        .and_then(|monitors| {
            monitors.into_iter().find(|monitor| {
                let (position, size) = (monitor.position(), monitor.size());
                PxRect {
                    x: position.x as f64,
                    y: position.y as f64,
                    w: size.width as f64,
                    h: size.height as f64,
                }
                .contains(center_x, center_y)
            })
        })
        .or_else(|| window.current_monitor().ok().flatten());

    let (x, y) = match monitor {
        Some(monitor) => {
            let scale = monitor.scale_factor();
            let area = monitor.work_area();
            let work = PxRect {
                x: area.position.x as f64,
                y: area.position.y as f64,
                w: area.size.width as f64,
                h: area.size.height as f64,
            };
            popover_origin(
                tray,
                work,
                (POPOVER_WIDTH * scale, POPOVER_HEIGHT * scale),
                EDGE_GAP * scale,
            )
        }
        // 拿不到显示器信息时退回原来的行为：图标正下方居中。
        None => {
            let scale = window.scale_factor().unwrap_or(1.0);
            (
                center_x - POPOVER_WIDTH * scale / 2.0,
                tray.y + tray.h + EDGE_GAP * scale,
            )
        }
    };

    window
        .set_position(tauri::PhysicalPosition::new(
            x.round() as i32,
            y.round() as i32,
        ))
        .map_err(|e| format!("设置 popover 位置失败: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f64, f64) = (360.0, 620.0);
    const GAP: f64 = 4.0;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> PxRect {
        PxRect { x, y, w, h }
    }

    fn assert_inside(origin: (f64, f64), work: PxRect) {
        let (x, y) = origin;
        assert!(
            x >= work.x && x + SIZE.0 <= work.x + work.w,
            "x {x} outside {work:?}"
        );
        assert!(
            y >= work.y && y + SIZE.1 <= work.y + work.h,
            "y {y} outside {work:?}"
        );
    }

    #[test]
    fn opens_below_an_icon_in_a_top_menu_bar() {
        let work = rect(0.0, 25.0, 1512.0, 957.0);
        let tray = rect(1200.0, 0.0, 30.0, 24.0);
        let (x, y) = popover_origin(tray, work, SIZE, GAP);
        assert!(
            y > tray.y + tray.h,
            "popover should open below the menu bar icon"
        );
        assert_eq!(x, 1215.0 - 180.0);
        assert_inside((x, y), work);
    }

    #[test]
    fn opens_above_an_icon_in_a_bottom_taskbar() {
        let work = rect(0.0, 0.0, 1920.0, 1032.0);
        let tray = rect(1800.0, 1040.0, 24.0, 32.0);
        let (x, y) = popover_origin(tray, work, SIZE, GAP);
        // The icon sits inside the taskbar, so the popover rests on the taskbar's top edge.
        assert_eq!(y, 1032.0 - 620.0 - GAP);
        assert_eq!(x, 1920.0 - 360.0 - GAP);
        assert_inside((x, y), work);
    }

    #[test]
    fn opens_above_an_icon_in_the_taskbar_overflow_flyout() {
        let work = rect(0.0, 0.0, 1920.0, 1032.0);
        let tray = rect(1700.0, 950.0, 32.0, 32.0);
        let origin = popover_origin(tray, work, SIZE, GAP);
        assert_eq!(origin.1, 950.0 - 620.0 - GAP);
        assert_inside(origin, work);
    }

    #[test]
    fn stays_inside_the_work_area_for_side_and_top_taskbars() {
        // Left taskbar: icon near the bottom-left corner.
        let left = rect(60.0, 0.0, 1860.0, 1080.0);
        let origin = popover_origin(rect(10.0, 1000.0, 40.0, 40.0), left, SIZE, GAP);
        assert_eq!(origin.0, 60.0 + GAP);
        assert_inside(origin, left);
        // Top taskbar: below the icon would still overlap the taskbar.
        let top = rect(0.0, 48.0, 1920.0, 1032.0);
        let origin = popover_origin(rect(1800.0, 8.0, 24.0, 32.0), top, SIZE, GAP);
        assert_eq!(origin.1, 48.0 + GAP);
        assert_inside(origin, top);
    }

    #[test]
    fn stays_on_a_secondary_monitor_left_of_the_primary() {
        let work = rect(-1920.0, 0.0, 1920.0, 1040.0);
        let origin = popover_origin(rect(-100.0, 1045.0, 24.0, 30.0), work, SIZE, GAP);
        assert_eq!(origin.0, -360.0 - GAP);
        assert_inside(origin, work);
    }

    #[test]
    fn keeps_the_top_visible_when_the_work_area_is_shorter_than_the_popover() {
        let work = rect(0.0, 0.0, 1366.0, 600.0);
        let (_, y) = popover_origin(rect(1300.0, 610.0, 24.0, 30.0), work, SIZE, GAP);
        assert_eq!(y, GAP);
    }
}
