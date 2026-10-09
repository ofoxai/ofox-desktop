//! Ofox 绑定记录 DAO（`ofox_bind_snapshot` 表）。
//!
//! 每个工具一行：`tool` 是 app 名（`codex` 一行同时服务 ChatGPT），`record`
//! 是版本化 JSON。这张表只属于本机——导出/同步跳过、导入/恢复保留本机行，
//! 接管模式的退出/崩溃清理也不会删它。

use crate::database::{lock_conn, Database};
use crate::error::AppError;

#[derive(Debug, Clone, PartialEq)]
pub struct BindRow {
    pub tool: String,
    pub record: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Database {
    pub fn get_bind_record(&self, tool: &str) -> Result<Option<BindRow>, AppError> {
        let conn = lock_conn!(self.conn);
        let result = conn.query_row(
            "SELECT tool, record, created_at, updated_at FROM ofox_bind_snapshot WHERE tool = ?1",
            rusqlite::params![tool],
            |row| {
                Ok(BindRow {
                    tool: row.get(0)?,
                    record: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            },
        );
        match result {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    /// 只在该工具还没有记录时写入，返回是否真的写入了。绑定前快照靠它保证
    /// 「只记一次」：重复绑定、自愈重绑都不会覆盖最初的状态。
    pub fn insert_bind_record_if_absent(&self, tool: &str, record: &str) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let now = chrono::Utc::now().to_rfc3339();
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO ofox_bind_snapshot (tool, record, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3)",
                rusqlite::params![tool, record, now],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(inserted > 0)
    }

    /// 写入或覆盖记录（保留原 `created_at`）。
    pub fn upsert_bind_record(&self, tool: &str, record: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO ofox_bind_snapshot (tool, record, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(tool) DO UPDATE SET record = excluded.record, updated_at = excluded.updated_at",
            rusqlite::params![tool, record, now],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn delete_bind_record(&self, tool: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "DELETE FROM ofox_bind_snapshot WHERE tool = ?1",
            rusqlite::params![tool],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// 把 `proxy_live_backup` 里的一行搬进绑定表（同一事务）：新表已有同名
    /// 记录时保留新表的，旧行照样删掉。
    pub fn move_live_backup_to_bind_record(&self, app_type: &str) -> Result<bool, AppError> {
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Database(e.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();
        let moved = tx
            .execute(
                "INSERT OR IGNORE INTO ofox_bind_snapshot (tool, record, created_at, updated_at)
                 SELECT app_type, original_config, backed_up_at, ?2
                 FROM proxy_live_backup WHERE app_type = ?1",
                rusqlite::params![app_type, now],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        tx.execute(
            "DELETE FROM proxy_live_backup WHERE app_type = ?1",
            rusqlite::params![app_type],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
        Ok(moved > 0)
    }

    /// `proxy_live_backup` 的全部行（`app_type`, `original_config`），供迁移判断。
    pub fn list_live_backups(&self) -> Result<Vec<(String, String)>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare("SELECT app_type, original_config FROM proxy_live_backup ORDER BY app_type")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| AppError::Database(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use crate::error::AppError;

    #[test]
    fn insert_if_absent_is_write_once() -> Result<(), AppError> {
        let db = Database::memory()?;
        assert!(db.insert_bind_record_if_absent("codex", "first")?);
        assert!(!db.insert_bind_record_if_absent("codex", "second")?);
        assert_eq!(db.get_bind_record("codex")?.unwrap().record, "first");
        Ok(())
    }

    #[test]
    fn upsert_replaces_record_and_keeps_created_at() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.upsert_bind_record("workbuddy", "v1")?;
        let created = db.get_bind_record("workbuddy")?.unwrap().created_at;
        db.upsert_bind_record("workbuddy", "v2")?;
        let row = db.get_bind_record("workbuddy")?.unwrap();
        assert_eq!(row.record, "v2");
        assert_eq!(row.created_at, created);
        db.delete_bind_record("workbuddy")?;
        assert!(db.get_bind_record("workbuddy")?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn move_live_backup_relocates_row_and_is_idempotent() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.save_live_backup("opencode", "{\"patch\":1}").await?;
        assert!(db.move_live_backup_to_bind_record("opencode")?);
        assert_eq!(
            db.get_bind_record("opencode")?.unwrap().record,
            "{\"patch\":1}"
        );
        assert!(db.get_live_backup("opencode").await?.is_none());
        assert!(!db.move_live_backup_to_bind_record("opencode")?);
        Ok(())
    }

    #[tokio::test]
    async fn move_keeps_existing_bind_record_but_drops_old_row() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.upsert_bind_record("hermes", "newer")?;
        db.save_live_backup("hermes", "older").await?;
        assert!(!db.move_live_backup_to_bind_record("hermes")?);
        assert_eq!(db.get_bind_record("hermes")?.unwrap().record, "newer");
        assert!(db.get_live_backup("hermes").await?.is_none());
        Ok(())
    }
}
