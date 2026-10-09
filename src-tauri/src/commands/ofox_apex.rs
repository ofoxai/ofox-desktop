//! OFox apex（区域）切换命令。
//!
//! 暴露给前端的唯一写入入口——前端不能直接 mutate `ofoxApex` 字段后保存，
//! 因为切换需要完成"清 token + 迁移工具地址 + 持久化 + 通知 UI"
//! 这一组动作。前端只走 `ofox_set_apex`、`ofox_get_apex` 两个命令，状态机就
//! 单点收敛在这里。
//!
//! 流程（顺序敏感，错位会留下半残状态）：
//! 1. 校验目标 apex 是已知值（白名单 in [`crate::ofox_apex::is_known_apex`]）。
//! 2. 与当前 apex 相同 → no-op，避免不必要的 logout。
//! 3. **先**写 settings：把 `ofoxApex` / `ofoxApexResolved` 改为目标值。这样
//!    紧随其后的地址构造调用 `current_apex()` 已经返回新 apex。
//! 4. **再** logout：清掉旧 apex 域签发的 token + 删 ofox_auth.json，避免被
//!    误用到新域上。
//! 5. **最后**只迁移现存 OFox 接入地址，保留已保存模型、API key 和绑定快照。
//!    遇到用户地址冲突时保留该工具原配置，并返回同步警告。
//! 6. emit `ofox-apex-changed`（payload 是新 apex）+ `ofox-reauth-requested`
//!    （MainApp 已监听，自动跳到 LoginPage）。
//!
//! 失败处理：
//! - 第 3 步失败 → 直接 return Err，settings 没改，没有副作用。
//! - 第 5 步冲突 → settings 已落盘，token 已清，但冲突工具的 DB/live 地址均
//!   保持原样。绑定诊断会提示检查；启动和再次选择地区会重试安全迁移。

use tauri::Emitter;

use crate::commands::ofox_auth::OfoxAuthState;
use crate::store::AppState;

/// 读当前 apex 字符串。前端 hook (`useOfoxApex`) 在挂载时读一次，并监听
/// `ofox-apex-changed` 事件刷新。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_get_apex() -> Result<String, String> {
    Ok(crate::ofox_apex::current_apex().to_string())
}

/// 前端看到的区域状态：当前 apex，以及是不是用户手动锁定的。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApexState {
    pub apex: String,
    pub pinned: bool,
    /// `ofox_set_apex_auto` 专用：这次探测有没有拿到结果。
    pub detected: bool,
}

fn apex_state(detected: bool) -> ApexState {
    ApexState {
        apex: crate::ofox_apex::current_apex().to_string(),
        pinned: crate::ofox_apex::apex_pinned(),
        detected,
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_get_apex_state() -> Result<ApexState, String> {
    Ok(apex_state(true))
}

/// Attempt the direct-config tools and WorkBuddy independently so one conflict
/// cannot prevent safe migrations in another tool. The returned flag drives
/// the renderer's region synchronization warning.
pub(crate) async fn reconcile_tool_endpoints(db: &crate::database::Database) -> bool {
    let direct = crate::services::ofox_bind::endpoint::reconcile_managed_endpoints(db).await;
    let workbuddy = crate::workbuddy_config::reconcile_managed_endpoint(db).await;
    if let Err(error) = &direct {
        log::warn!("[OfoxApex] tool endpoint reconciliation failed: {error}");
    }
    if workbuddy.is_err() {
        log::warn!("[OfoxApex] WorkBuddy endpoint conflict; existing configuration retained");
    }
    direct.is_ok() && workbuddy.is_ok()
}

/// 切换 apex 并完成关联副作用（持久化 → 清旧域 token → 同步现存工具地址 →
/// 通知前端）。手动切换和启动探测共用。返回所有工具地址是否同步成功。
pub(crate) async fn apply_apex_switch(
    app: &tauri::AppHandle,
    next_apex: &str,
    pin: bool,
) -> Result<bool, String> {
    use tauri::Manager;
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "应用状态尚未就绪".to_string())?;
    let ofox_state = app
        .try_state::<OfoxAuthState>()
        .ok_or_else(|| "登录状态尚未就绪".to_string())?;
    let current = crate::ofox_apex::current_apex();
    log::info!("[OfoxApex] switching apex: {current} → {next_apex}");

    // 3. Persist before endpoint reconciliation so builders use the new apex.
    let next_for_settings = next_apex.to_string();
    crate::settings::mutate_settings(move |s| {
        s.ofox_apex = Some(next_for_settings);
        s.ofox_apex_resolved = Some(true);
        s.ofox_apex_pinned = Some(pin);
    })
    .map_err(|e| format!("写入 settings 失败: {e}"))?;

    // 4. Clear the old region's login before changing external tool addresses.
    {
        let mut manager = ofox_state.0.write().await;
        manager.logout();
    }

    // 5. Patch only URLs, never re-seed user model selections or deleted files.
    let tools_synced = reconcile_tool_endpoints(&state.db).await;

    // 6. 通知前端
    if let Err(e) = app.emit("ofox-apex-changed", next_apex) {
        log::warn!("[OfoxApex] emit ofox-apex-changed failed: {e}");
    }
    if let Err(e) = app.emit("ofox-reauth-requested", ()) {
        log::warn!("[OfoxApex] emit ofox-reauth-requested failed: {e}");
    }

    Ok(tools_synced)
}

/// 手动切换 OFox apex（并锁定）。详见模块文档。
///
/// `next_apex`: 必须是 `"ofox.ai"` 或 `"ofox.io"`，前端 Select 已经把字符串
/// 限制好了，但服务端再校验一次防止被绕过写入未知值。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_set_apex(next_apex: String, app: tauri::AppHandle) -> Result<bool, String> {
    // 1. 白名单校验
    if !crate::ofox_apex::is_known_apex(&next_apex) {
        return Err(format!("未知 apex: {next_apex}"));
    }

    // 2. 同值：只锁定，不触发 logout（从「自动」改成手动选当前这个）
    let current = crate::ofox_apex::current_apex();
    if current == next_apex.as_str() {
        if !crate::ofox_apex::apex_pinned() {
            crate::settings::mutate_settings(|s| s.ofox_apex_pinned = Some(true))
                .map_err(|e| format!("写入 settings 失败: {e}"))?;
        }
        log::info!("[OfoxApex] set_apex({next_apex}): same as current, pinned");
        use tauri::Manager;
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| "应用状态尚未就绪".to_string())?;
        let synced = reconcile_tool_endpoints(&state.db).await;
        if let Err(error) = app.emit("ofox-prefs-updated", ()) {
            log::warn!("[OfoxApex] emit configuration refresh failed: {error}");
        }
        return Ok(synced);
    }

    apply_apex_switch(&app, &next_apex, true).await
}

/// 改回跟随网络：解除锁定并立刻探测一次，结果和当前不同就切换。
/// 探测失败时保持当前值（`detected == false`），下次启动会再试。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_set_apex_auto(app: tauri::AppHandle) -> Result<ApexState, String> {
    crate::settings::mutate_settings(|s| s.ofox_apex_pinned = None)
        .map_err(|e| format!("写入 settings 失败: {e}"))?;
    let client = crate::proxy::http_client::get();
    let (detected, confident) = crate::ofox_apex::detect_apex_from_geo(&client).await;
    if confident && detected != crate::ofox_apex::current_apex() {
        apply_apex_switch(&app, detected, false).await?;
    }
    Ok(apex_state(confident))
}
