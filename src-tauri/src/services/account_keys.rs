//! 换账号时让工具跟着换 key。
//!
//! Ofox API key 按创建它的组织计费。退出登录时把本账号在这台电脑上用的 key
//! 停掉（见 [`stop_for_logout`]）；登录后逐个检查已绑定的工具，不是当前账号的
//! key 就换成当前账号新签发的（见 [`reconcile`]）。只换 key：先用旧 key 核对
//! 配置没被用户改过，模型等其它设置一律不动；改过就停下来告诉用户。

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::RwLock;

use crate::app_config::{AppType, BindableTool};
use crate::database::Database;
use crate::ofox_auth::OfoxAuthManager;
use crate::ofox_secret::{SecretStore, Slot};
use crate::services::ofox_bind::{self, status::BindingStatus, Tool};
use crate::settings::{self, ApiKeyMeta, KeyOwner};

/// 登录后检查和启动时检查可能同时触发，串行执行。
static RECONCILE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoutReport {
    /// 停用了 key 的工具（工具 id）。
    pub stopped: Vec<String>,
    /// 服务器上没能吊销的（例如没联网）：这些 key 还能用，要提醒用户去控制台删。
    pub not_revoked: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyReconcileReport {
    /// 已经换成当前账号 key 的工具 id。
    pub switched: Vec<String>,
    pub failed: Vec<KeyReconcileFailure>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyReconcileFailure {
    pub tool: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyAction {
    /// 当前账号能用，不动。
    Keep,
    /// 工具已经没绑定了（解绑时 key 留在本地）：丢掉，以后绑定时签新的。
    DropStale,
    /// 工具还绑着，key 却是别的账号的或已停用：换成当前账号的新 key。
    Replace,
}

pub(crate) fn key_action(meta: &ApiKeyMeta, bound: bool, current: &KeyOwner) -> KeyAction {
    if meta.usable_by(Some(current)) {
        KeyAction::Keep
    } else if bound {
        KeyAction::Replace
    } else {
        KeyAction::DropStale
    }
}

/// 改配置文件的工具；WorkBuddy 单独处理。
fn file_tool(tool: BindableTool) -> Option<Tool> {
    let app = match tool {
        BindableTool::Claude => AppType::Claude,
        BindableTool::Codex => AppType::Codex,
        BindableTool::Gemini => AppType::Gemini,
        BindableTool::OpenCode => AppType::OpenCode,
        BindableTool::OpenClaw => AppType::OpenClaw,
        BindableTool::Hermes => AppType::Hermes,
        BindableTool::WorkBuddy => return None,
    };
    Some(Tool::from_app(&app))
}

async fn is_bound(db: &Database, tool: BindableTool) -> bool {
    match file_tool(tool) {
        Some(tool) => ofox_bind::is_bound(db, tool),
        None => crate::workbuddy_config::is_bound(db).await,
    }
}

/// 退出登录前（登录还有效时）调用。
pub async fn stop_for_logout(manager: &Arc<RwLock<OfoxAuthManager>>) -> LogoutReport {
    let stopped = settings::list_api_key_metas()
        .into_iter()
        .filter(|meta| !meta.revoked)
        .map(|meta| meta.tool.as_str().to_string())
        .collect();
    let not_revoked = crate::ofox_api_keys::stop_keys_for_logout(manager)
        .await
        .into_iter()
        .map(|tool| tool.as_str().to_string())
        .collect();
    LogoutReport {
        stopped,
        not_revoked,
    }
}

/// 登录后、启动时调用：让已绑定的工具都用当前账号的 key。
pub async fn reconcile(
    db: &Database,
    manager: &Arc<RwLock<OfoxAuthManager>>,
) -> KeyReconcileReport {
    let _guard = RECONCILE_LOCK.lock().await;
    let mut report = KeyReconcileReport::default();
    let Some(owner) = manager.read().await.current_key_owner().await else {
        return report;
    };
    crate::ofox_api_keys::adopt_unowned_keys(&owner);
    let store = crate::ofox_secret::default_store();
    for meta in settings::list_api_key_metas() {
        let tool = meta.tool;
        match key_action(&meta, is_bound(db, tool).await, &owner) {
            KeyAction::Keep => {}
            KeyAction::DropStale => {
                if let Err(e) =
                    crate::ofox_api_keys::revoke_local_with_store(tool, store.clone()).await
                {
                    log::warn!("[account_keys] drop stale key for {tool:?} failed: {e:?}");
                }
            }
            KeyAction::Replace => match replace(db, manager, &store, &meta, &owner).await {
                Ok(()) => report.switched.push(tool.as_str().to_string()),
                Err(message) => report.failed.push(KeyReconcileFailure {
                    tool: tool.as_str().to_string(),
                    message,
                }),
            },
        }
    }
    report
}

async fn binding_status(db: &Database, tool: BindableTool, old_key: &str) -> BindingStatus {
    match file_tool(tool) {
        Some(file_tool) => {
            ofox_bind::status::binding_status(db, file_tool, Some(old_key))
                .await
                .status
        }
        None => crate::workbuddy_config::binding_status(db).await.status,
    }
}

async fn replace(
    db: &Database,
    manager: &Arc<RwLock<OfoxAuthManager>>,
    store: &Arc<dyn SecretStore>,
    old: &ApiKeyMeta,
    owner: &KeyOwner,
) -> Result<(), String> {
    let tool = old.tool;
    let old_key = store
        .load(Slot::ApiKey { tool })
        .map_err(|_| "读取原来的 key 失败。".to_string())?
        .ok_or_else(|| "找不到原来的 key，请在主页解除绑定后重新添加。".to_string())?;
    // 先核对再签发：配置被改过就不必在服务器上多建一把用不上的 key。
    if binding_status(db, tool, &old_key).await != BindingStatus::Configured {
        return Err("接入配置已被修改，没有自动更换。请在管理窗口检查或重新绑定。".into());
    }
    let (new_key, meta) = crate::ofox_api_keys::create_unsaved(tool, Some(owner.clone()), manager)
        .await
        .map_err(|e| format!("签发新 key 失败：{e}"))?;
    let written = match file_tool(tool) {
        Some(file_tool) => ofox_bind::replace_key(db, file_tool, &old_key, &new_key).await,
        None => crate::workbuddy_config::replace_key(db, &new_key).await,
    };
    if let Err(error) = written {
        crate::ofox_api_keys::discard_remote(&meta.key_id, manager).await;
        return Err(error);
    }
    crate::ofox_api_keys::save_key(store, &new_key, meta)
        .map_err(|e| format!("保存新 key 失败：{e}"))?;
    crate::ofox_api_keys::mark_key_used(tool);
    // 同一个账号重新登录：退出时没能在服务器上吊销的旧 key 这时可以删掉。
    // 别的账号的 key 删不了（没有它的登录），退出时已提示用户去控制台删。
    let same_account = old
        .owner
        .as_ref()
        .and_then(|previous| previous.same_account(owner));
    if same_account == Some(true) {
        crate::ofox_api_keys::discard_remote(&old.key_id, manager).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(owner: Option<KeyOwner>, revoked: bool) -> ApiKeyMeta {
        ApiKeyMeta {
            tool: BindableTool::Claude,
            key_id: "key".into(),
            name: None,
            alias: None,
            key_start: None,
            created_at: 1,
            last_used_at: None,
            owner,
            revoked,
        }
    }

    fn org(id: &str) -> KeyOwner {
        KeyOwner {
            org_id: Some(id.into()),
            user_id: None,
        }
    }

    #[test]
    fn a_bound_tool_gets_a_new_key_only_when_its_key_is_not_this_accounts() {
        let a = org("org_a");
        let b = org("org_b");
        assert_eq!(
            key_action(&meta(Some(a.clone()), false), true, &a),
            KeyAction::Keep
        );
        assert_eq!(
            key_action(&meta(Some(a.clone()), false), true, &b),
            KeyAction::Replace
        );
        // Logging out stops the key; logging back in as the same account still renews it.
        assert_eq!(
            key_action(&meta(Some(a.clone()), true), true, &a),
            KeyAction::Replace
        );
    }

    #[test]
    fn a_leftover_key_of_an_unbound_tool_is_dropped_not_reused() {
        let a = org("org_a");
        let b = org("org_b");
        assert_eq!(
            key_action(&meta(Some(a.clone()), false), false, &b),
            KeyAction::DropStale
        );
        assert_eq!(
            key_action(&meta(Some(a.clone()), true), false, &a),
            KeyAction::DropStale
        );
        assert_eq!(
            key_action(&meta(Some(a.clone()), false), false, &a),
            KeyAction::Keep
        );
    }
}
