//! Tauri commands for managing per-tool ofox LLM API keys.
//!
//! 前端通过这些 command 查看 / 创建 / 撤销 / 强制刷新每个工具的 ofox API key。
//! 本模块是 [`crate::ofox_api_keys`] 的 Tauri 绑定层——所有真正的 keychain /
//! settings / `/openapi/api-keys` 调用都在那一层做，这里只负责"转参数 + 把
//! ApiKeyError 转 String"。
//!
//! 错误统一返 `Result<_, String>`：Tauri 把它序列化给前端，前端按消息文本
//! 决定 UI 弹什么。`ApiKeyError` 的 `thiserror::Error` 派生让 to_string()
//! 拿到的就是人读的文案，足够 UI 直接 toast。

use std::sync::Arc;

use tauri::State;
use tokio::sync::RwLock;

use crate::app_config::AppType;
use crate::ofox_api_keys::{ApiKeyError, FetchMode};
use crate::ofox_auth::OfoxAuthManager;
use crate::settings::{self, ApiKeyMeta};

/// 把 ApiKeyError 转成给前端的 String。**集中一处分类**避免每个 command 都
/// 重复 match 一遍——文案变更点也收敛在这里。
fn explain(e: ApiKeyError) -> String {
    match e {
        ApiKeyError::Unauthorized(msg) => {
            format!("OFox 授权失效：{msg}。请退出登录后重新走 device flow。")
        }
        ApiKeyError::RemoteRejected(msg) => format!("OFox API key 服务端拒绝：{msg}"),
        ApiKeyError::Storage(msg) => format!("本地 keychain / settings 读写失败：{msg}"),
    }
}

/// 列出所有工具的 API key 元数据。
///
/// **不**返回 key 本体——本体在 keychain，前端从来用不到（只在 bind 流程
/// 中由后端取出注入工具配置）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_list_api_keys() -> Result<Vec<ApiKeyMeta>, String> {
    Ok(settings::list_api_key_metas())
}

/// 为指定工具创建（或获取已有的）API key。
///
/// 流程跟 bind 流程里调的是同一个 [`crate::ofox_api_keys::fetch_or_create_api_key`]，
/// 模式固定为 [`FetchMode::CachedOk`]：keychain 命中即返，未命中调端点签发。
///
/// 这个 command 用于"在 UI 单独管理界面里点'生成 key'"——产品语义跟 bind
/// 解耦：用户可以提前创建好 key，之后再点 bind。
///
/// `name` 参数当前**忽略**——服务端命名由 `default_key_name(tool)` 统一生成
/// （`<tool> on <host>`）以保证 ofox console 里的可读性。保留参数为将来"用户
/// 自定义 alias"的扩展位（届时同时写入 ofox 后台 name + 本地 ApiKeyMeta.alias）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_create_api_key_for_tool(
    tool: String,
    _name: Option<String>,
    ofox_manager: State<'_, crate::commands::ofox_auth::OfoxAuthState>,
) -> Result<ApiKeyMeta, String> {
    let tool: AppType = tool
        .parse()
        .map_err(|e: crate::error::AppError| format!("无效的工具类型: {e}"))?;

    let manager: Arc<RwLock<OfoxAuthManager>> = ofox_manager.0.clone();

    crate::ofox_api_keys::fetch_or_create_api_key(tool, FetchMode::CachedOk, &manager)
        .await
        .map_err(explain)?;

    // 成功后元数据必然已 upsert（除非走的是 keychain 命中分支——那时元数据已
    // 经存在）。若仍 None 说明协议被破坏。
    settings::get_api_key_meta(tool).ok_or_else(|| {
        format!(
            "key 已签发但元数据缺失（tool={}）——内部不变量被破坏",
            tool.as_str()
        )
    })
}

/// 强制刷新指定工具的 API key——跳过 keychain 命中、直接调端点签发新的，覆盖
/// 本地。前端"工具调用 401 → 用户点重建"路径用。
///
/// **不**主动 DELETE 旧 key——旧 key_id 在服务端可能已被撤销，也可能本地丢了。
/// 用户想清理旧 key 时走单独的 [`ofox_revoke_api_key_for_tool`] command。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_refresh_api_key_for_tool(
    tool: String,
    ofox_manager: State<'_, crate::commands::ofox_auth::OfoxAuthState>,
) -> Result<ApiKeyMeta, String> {
    let tool: AppType = tool
        .parse()
        .map_err(|e: crate::error::AppError| format!("无效的工具类型: {e}"))?;

    let manager: Arc<RwLock<OfoxAuthManager>> = ofox_manager.0.clone();

    crate::ofox_api_keys::fetch_or_create_api_key(tool, FetchMode::ForceRefresh, &manager)
        .await
        .map_err(explain)?;

    settings::get_api_key_meta(tool).ok_or_else(|| {
        format!(
            "key 已刷新但元数据缺失（tool={}）——内部不变量被破坏",
            tool.as_str()
        )
    })
}

/// 撤销指定工具的 API key。
///
/// 行为：
///   - `remote=true` → 调 `DELETE /openapi/api-keys/{id}` + 清本地。
///     404 也视为成功（服务端已撤销 / id 跨 org）。
///   - `remote=false` → 仅清本地 keychain + settings（不调端点）。
///
/// "仅清本地"用于用户在 ofox 后台手动撤销过、本地状态需要追平的场景。
#[tauri::command(rename_all = "camelCase")]
pub async fn ofox_revoke_api_key_for_tool(
    tool: String,
    remote: bool,
    ofox_manager: State<'_, crate::commands::ofox_auth::OfoxAuthState>,
) -> Result<(), String> {
    let tool: AppType = tool
        .parse()
        .map_err(|e: crate::error::AppError| format!("无效的工具类型: {e}"))?;

    if remote {
        let manager: Arc<RwLock<OfoxAuthManager>> = ofox_manager.0.clone();
        crate::ofox_api_keys::revoke_remote(tool, &manager)
            .await
            .map_err(explain)
    } else {
        crate::ofox_api_keys::revoke_local(tool)
            .await
            .map_err(explain)
    }
}
