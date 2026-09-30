//! 旧版把 Ofox 绑定数据和接管模式的整份配置备份混存在 `proxy_live_backup`。
//! 接管模式的退出/崩溃清理会整表删除，把绑定数据一起删掉（退出一次再解绑就
//! 什么都删不掉）。启动时把属于 Ofox 绑定的行搬到 `ofox_bind_snapshot`。
//!
//! 判断不了的行一律留在原处：误把一份接管备份（用户的整份配置）当成 Ofox 补丁
//! 搬走，解绑时会把用户配置当作 Ofox 字段减掉。

use std::str::FromStr;

use crate::app_config::AppType;
use crate::database::Database;
use crate::error::AppError;
use crate::ofox_apex::mentions_ofox_gateway;

/// OpenClaw/Hermes 的绑定备份外层标记（见 `ProxyService::ofox_backup_live_config`）。
const DIRECT_BACKUP_MARKER: &str = "__ofoxDirectBackupVersion";

/// 这一行是不是 Ofox 绑定写的。`taken_over` 报告该工具的配置文件里现在是否有
/// 接管占位符——有就说明这行是接管备份，不能动。
fn is_ofox_bind_row(app_type: &str, payload: &str, taken_over: &impl Fn(&AppType) -> bool) -> bool {
    if app_type == "workbuddy" {
        return true;
    }
    let Ok(app) = AppType::from_str(app_type) else {
        return false;
    };
    match app {
        // 这几个工具不支持接管模式，表里只可能是 Ofox 的绑定数据。
        AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => true,
        AppType::Claude | AppType::Codex | AppType::Gemini => {
            !taken_over(&app)
                && (payload.contains(DIRECT_BACKUP_MARKER) || mentions_ofox_gateway(payload))
        }
    }
}

/// 搬走所有 Ofox 绑定行，返回被搬走的工具名。可重复执行。
pub(crate) fn relocate_legacy_rows(
    db: &Database,
    taken_over: impl Fn(&AppType) -> bool,
) -> Result<Vec<String>, AppError> {
    let mut moved = Vec::new();
    for (app_type, payload) in db.list_live_backups()? {
        if is_ofox_bind_row(&app_type, &payload, &taken_over) {
            db.move_live_backup_to_bind_record(&app_type)?;
            moved.push(app_type);
        }
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE_PATCH: &str = r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.ofox.ai/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-of-x"}}"#;
    const CODEX_PATCH: &str = r#"{"auth":{"OPENAI_API_KEY":"sk-of-x"},"config":"model_provider = \"ofox\"\n[model_providers.ofox]\nbase_url = \"https://api.ofox.io/v1\"\n"}"#;
    const CLAUDE_TAKEOVER_BACKUP: &str = r#"{"env":{"ANTHROPIC_BASE_URL":"https://relay.example.com","ANTHROPIC_AUTH_TOKEN":"user"}}"#;

    #[tokio::test]
    async fn moves_ofox_rows_and_keeps_takeover_backups() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.save_live_backup("claude", CLAUDE_PATCH).await?;
        db.save_live_backup("codex", CODEX_PATCH).await?;
        db.save_live_backup("gemini", CLAUDE_TAKEOVER_BACKUP)
            .await?;
        db.save_live_backup("opencode", r#"{"provider":{}}"#)
            .await?;
        db.save_live_backup("openclaw", r#"{"__ofoxDirectBackupVersion":1,"patch":{}}"#)
            .await?;
        db.save_live_backup("workbuddy", r#"{"version":2}"#).await?;

        let moved = relocate_legacy_rows(&db, |_| false)?;

        assert_eq!(
            moved,
            ["claude", "codex", "openclaw", "opencode", "workbuddy"]
        );
        for tool in ["claude", "codex", "openclaw", "opencode", "workbuddy"] {
            assert!(db.get_bind_record(tool)?.is_some(), "{tool} moved");
            assert!(
                db.get_live_backup(tool).await?.is_none(),
                "{tool} old row gone"
            );
        }
        assert!(db.get_live_backup("gemini").await?.is_some());
        assert!(db.get_bind_record("gemini")?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn keeps_rows_of_tools_still_taken_over() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.save_live_backup("claude", CLAUDE_PATCH).await?;
        let moved = relocate_legacy_rows(&db, |app| matches!(app, AppType::Claude))?;
        assert!(moved.is_empty());
        assert!(db.get_live_backup("claude").await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn relocation_is_idempotent() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.save_live_backup("hermes", r#"{"patch":{}}"#).await?;
        assert_eq!(relocate_legacy_rows(&db, |_| false)?, ["hermes"]);
        assert!(relocate_legacy_rows(&db, |_| false)?.is_empty());
        assert!(db.get_bind_record("hermes")?.is_some());
        Ok(())
    }
}
