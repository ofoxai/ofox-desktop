//! 换账号时让工具跟着换 key。
//!
//! Ofox API key 按创建它的组织计费。退出登录时把本账号在这台电脑上用的 key
//! 停掉（见 [`stop_for_logout`]）；登录后逐个检查已绑定的工具，不是当前账号的
//! key 就换成当前账号新签发的（见 [`reconcile`]）。只换 key：先用旧 key 核对
//! 配置没被用户改过，模型等其它设置一律不动；改过就停下来告诉用户。
//!
//! 服务器上没删掉的旧 key（退出时没联网、切换区域时没停等）记在待删列表里，
//! 等它的账号在这台电脑上登录时删掉。

use std::future::Future;
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
    /// 服务器上没能吊销的（例如没联网）：这些 key 还能用，直到这个账号下次登录时
    /// 被自动删掉，要提醒用户。
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

const ALL_TOOLS: [BindableTool; 7] = [
    BindableTool::Claude,
    BindableTool::Codex,
    BindableTool::Gemini,
    BindableTool::OpenCode,
    BindableTool::OpenClaw,
    BindableTool::Hermes,
    BindableTool::WorkBuddy,
];

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
    crate::ofox_api_keys::delete_queued_keys(&owner, manager).await;
    let store = crate::ofox_secret::default_store();
    for meta in settings::list_api_key_metas() {
        let tool = meta.tool;
        match key_action(&meta, is_bound(db, tool).await, &owner) {
            KeyAction::Keep => {}
            KeyAction::DropStale => {
                match crate::ofox_api_keys::revoke_local_with_store(tool, store.clone()).await {
                    Ok(()) => crate::ofox_api_keys::queue_deletion_unless_stopped(&meta),
                    Err(e) => {
                        log::warn!("[account_keys] drop stale key for {tool:?} failed: {e:?}")
                    }
                }
            }
            KeyAction::Replace => {
                let issue = || issue_new_key(tool, &owner, manager);
                match replace(db, manager, &store, tool, Some(&meta), issue).await {
                    Ok(()) => report.switched.push(tool.as_str().to_string()),
                    Err(message) => report.failed.push(KeyReconcileFailure {
                        tool: tool.as_str().to_string(),
                        message,
                    }),
                }
            }
        }
    }
    rekey_unrecorded(
        db,
        manager,
        &store,
        |tool| {
            let owner = owner.clone();
            async move { issue_new_key(tool, &owner, manager).await }
        },
        &mut report,
    )
    .await;
    report
}

async fn issue_new_key(
    tool: BindableTool,
    owner: &KeyOwner,
    manager: &Arc<RwLock<OfoxAuthManager>>,
) -> Result<(String, ApiKeyMeta), String> {
    crate::ofox_api_keys::create_unsaved(tool, Some(owner.clone()), manager)
        .await
        .map_err(|e| format!("签发新 key 失败：{e}"))
}

/// 已绑定、钥匙串里有 key，却没有 key 记录的工具：没有记录就没有 key id，主页没有
/// 数据统计，上面按记录遍历的对账也看不见它。换成当前账号新签发的 key 并补上记录。
/// 配置已被用户改成别的（不再是 Ofox 的接入方式）就不动，也不报错：这不是 key 的问题。
/// 旧 key 没有 id，没法排队删除，会留在服务器上。
pub(crate) async fn rekey_unrecorded<I, Fut>(
    db: &Database,
    manager: &Arc<RwLock<OfoxAuthManager>>,
    store: &Arc<dyn SecretStore>,
    issue: I,
    report: &mut KeyReconcileReport,
) where
    I: Fn(BindableTool) -> Fut,
    Fut: Future<Output = Result<(String, ApiKeyMeta), String>>,
{
    for tool in ALL_TOOLS {
        if settings::get_api_key_meta(tool).is_some() || !is_bound(db, tool).await {
            continue;
        }
        let key = match store.load(Slot::ApiKey { tool }) {
            Ok(Some(key)) if !key.is_empty() => key,
            _ => continue,
        };
        if binding_status(db, tool, &key).await != BindingStatus::Configured {
            log::info!(
                "[account_keys] {tool:?} has a key but no record and a changed config; skipped"
            );
            continue;
        }
        match replace(db, manager, store, tool, None, || issue(tool)).await {
            Ok(()) => report.switched.push(tool.as_str().to_string()),
            Err(message) => report.failed.push(KeyReconcileFailure {
                tool: tool.as_str().to_string(),
                message,
            }),
        }
    }
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

/// 把工具用的 key 换成 `issue` 新签发的。`old` 是原来的记录；没有记录（见
/// [`rekey_unrecorded`]）就没有旧 key 的 id，不排队删除。
async fn replace<I, Fut>(
    db: &Database,
    manager: &Arc<RwLock<OfoxAuthManager>>,
    store: &Arc<dyn SecretStore>,
    tool: BindableTool,
    old: Option<&ApiKeyMeta>,
    issue: I,
) -> Result<(), String>
where
    I: FnOnce() -> Fut,
    Fut: Future<Output = Result<(String, ApiKeyMeta), String>>,
{
    let old_key = store
        .load(Slot::ApiKey { tool })
        .map_err(|_| "读取原来的 key 失败。".to_string())?
        .ok_or_else(|| "找不到原来的 key，请在主页解除绑定后重新添加。".to_string())?;
    // 先核对再签发：配置被改过就不必在服务器上多建一把用不上的 key。
    if binding_status(db, tool, &old_key).await != BindingStatus::Configured {
        return Err("接入配置已被修改，没有自动更换。请在管理窗口检查或重新绑定。".into());
    }
    let (new_key, meta) = issue().await?;
    let written = match file_tool(tool) {
        Some(file_tool) => ofox_bind::replace_key(db, file_tool, &old_key, &new_key).await,
        None => crate::workbuddy_config::replace_key(db, &new_key).await,
    };
    if let Err(error) = written {
        crate::ofox_api_keys::discard_remote(&meta, manager).await;
        return Err(error);
    }
    crate::ofox_api_keys::save_key(store, &new_key, meta)
        .map_err(|e| format!("保存新 key 失败：{e}"))?;
    crate::ofox_api_keys::mark_key_used(tool);
    if let Some(old) = old {
        crate::ofox_api_keys::queue_deletion_unless_stopped(old);
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

    /// 切换区域会退出登录但不停 key，于是换账号登录时可能遇到别的账号还能用的 key。
    #[tokio::test]
    #[serial_test::serial]
    async fn signing_in_keeps_a_dropped_live_key_of_another_account_for_deletion() {
        let _home = crate::settings::TempSettingsHome::new();
        let db = Database::memory().expect("db");
        let user: crate::ofox_auth::OfoxUserInfo = serde_json::from_str(
            r#"{"id":"user_a","email":"a@example.com","name":"A","org_id":"org_a"}"#,
        )
        .unwrap();
        let manager = OfoxAuthManager::new_with_secret_store(
            crate::config::get_app_config_dir(),
            Arc::new(crate::ofox_secret::InMemoryStore::new()),
        );
        manager.set_signed_in_for_test(user).await;
        let manager = Arc::new(RwLock::new(manager));
        crate::ofox_secret::default_store()
            .save(
                Slot::ApiKey {
                    tool: BindableTool::Codex,
                },
                "sk-of-B",
            )
            .unwrap();
        settings::upsert_api_key_meta(ApiKeyMeta {
            tool: BindableTool::Codex,
            key_id: "key-b".into(),
            ..meta(Some(org("org_b")), false)
        })
        .unwrap();

        let report = reconcile(&db, &manager).await;

        assert!(report.switched.is_empty() && report.failed.is_empty());
        assert_eq!(settings::get_api_key_meta(BindableTool::Codex), None);
        assert_eq!(
            settings::list_keys_to_delete(),
            [crate::settings::KeyToDelete {
                key_id: "key-b".into(),
                owner: Some(org("org_b")),
            }]
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
