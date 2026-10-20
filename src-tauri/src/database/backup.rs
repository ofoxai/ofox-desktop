//! 数据库备份和恢复
//!
//! 提供 SQL 导出/导入和二进制快照备份功能。

use super::{lock_conn, Database};
use crate::config::{
    create_private_dir_all, ensure_private_file, get_app_config_dir, set_file_mode,
};
use crate::error::AppError;
use chrono::{Local, Utc};
use rusqlite::backup::Backup;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const CC_SWITCH_SQL_EXPORT_HEADER: &str = "-- Ofox Switch SQLite 导出";

/// Tables that describe this machine's files and never leave it: skipped by every
/// export, and kept from the local database by every import or restore. A bind
/// snapshot from another machine or an older backup would restore the wrong files.
/// The session-file metadata cache and balance samples are this machine's too.
///
/// `session_usage_archive` and `maintenance_flags` are not: they describe the
/// request logs, so a full export, import or restore carries them with
/// `proxy_request_logs` (an old backup's logs with this machine's archive would
/// count the archived calls twice, and its unrepriced Claude rows would never be
/// repriced under this machine's `done` flags). WebDAV sync skips and keeps them
/// along with the logs.
const MACHINE_LOCAL_TABLES: &[&str] = &[
    "ofox_bind_snapshot",
    "session_meta_cache",
    "balance_samples",
];

/// Tables whose data rows are skipped when exporting for WebDAV sync.
/// `session_log_sync` is how far this machine read its own transcripts: another
/// machine's progress would make every local transcript be read from the start
/// again and its old, already archived calls stored a second time.
const SYNC_SKIP_TABLES: &[&str] = &[
    "proxy_request_logs",
    "stream_check_logs",
    "provider_health",
    "proxy_live_backup",
    "usage_daily_rollups",
    "ofox_bind_snapshot",
    "session_usage_archive",
    "session_meta_cache",
    "balance_samples",
    "maintenance_flags",
    "session_log_sync",
];

/// Tables whose local data is preserved (restored from local snapshot) during WebDAV import.
/// Excludes ephemeral tables like provider_health that can safely rebuild at runtime.
const SYNC_PRESERVE_TABLES: &[&str] = &[
    "proxy_request_logs",
    "stream_check_logs",
    "proxy_live_backup",
    "usage_daily_rollups",
    "ofox_bind_snapshot",
    "session_usage_archive",
    "session_meta_cache",
    "balance_samples",
    "maintenance_flags",
    "session_log_sync",
];

/// A database backup entry for the UI
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub filename: String,
    pub size_bytes: u64,
    pub created_at: String, // ISO 8601
}

impl Database {
    /// 导出为 SQLite 兼容的 SQL 文本（内存字符串，完整导出）
    pub fn export_sql_string(&self) -> Result<String, AppError> {
        let snapshot = self.snapshot_to_memory()?;
        Self::dump_sql(&snapshot, MACHINE_LOCAL_TABLES)
    }

    /// Export SQL for sync (WebDAV), skipping local-only tables' data
    pub fn export_sql_string_for_sync(&self) -> Result<String, AppError> {
        let snapshot = self.snapshot_to_memory()?;
        Self::dump_sql(&snapshot, SYNC_SKIP_TABLES)
    }
    pub(crate) fn export_sync_snapshot(conn: &Connection) -> Result<String, AppError> {
        Self::dump_sql(conn, SYNC_SKIP_TABLES)
    }

    /// 导出为 SQLite 兼容的 SQL 文本
    pub fn export_sql(&self, target_path: &Path) -> Result<(), AppError> {
        let dump = self.export_sql_string()?;

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }

        crate::config::atomic_write(target_path, dump.as_bytes())
    }

    /// 从 SQL 文件导入，返回生成的备份 ID（若无备份则为空字符串）
    pub fn import_sql(&self, source_path: &Path) -> Result<String, AppError> {
        if !source_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "SQL 文件不存在: {}",
                source_path.display()
            )));
        }

        let sql_raw = fs::read_to_string(source_path).map_err(|e| AppError::io(source_path, e))?;
        let sql_content = sql_raw.trim_start_matches('\u{feff}');
        self.import_sql_string(sql_content)
    }

    /// 从 SQL 字符串导入，返回生成的备份 ID（若无备份则为空字符串）
    pub fn import_sql_string(&self, sql_raw: &str) -> Result<String, AppError> {
        self.import_sql_string_inner(sql_raw, MACHINE_LOCAL_TABLES, |_| Ok(()))
    }

    /// Import SQL generated for sync, then restore local-only tables from the
    /// current device snapshot before replacing the main database.
    #[cfg(test)]
    pub(crate) fn import_sql_string_for_sync(&self, sql_raw: &str) -> Result<String, AppError> {
        self.import_sql_string_inner(sql_raw, SYNC_PRESERVE_TABLES, |_| Ok(()))
    }

    pub(crate) fn import_sync_with_resources(
        &self,
        sql_raw: &str,
        prepare: impl FnOnce(&Connection) -> Result<(), AppError>,
    ) -> Result<String, AppError> {
        self.import_sql_string_inner(sql_raw, SYNC_PRESERVE_TABLES, prepare)
    }

    fn import_sql_string_inner(
        &self,
        sql_raw: &str,
        preserve_tables: &[&str],
        prepare: impl FnOnce(&Connection) -> Result<(), AppError>,
    ) -> Result<String, AppError> {
        let _io = crate::config_io::lock();
        let sql_content = sql_raw.trim_start_matches('\u{feff}');
        Self::validate_cc_switch_sql_export(sql_content)?;

        // 导入前备份现有数据库
        let backup_path = self.backup_database_file()?;

        let local_snapshot = self.snapshot_to_memory()?;

        // 在临时数据库执行导入，确保失败不会污染主库
        let temp_file = NamedTempFile::new().map_err(|e| AppError::IoContext {
            context: "创建临时数据库文件失败".to_string(),
            source: e,
        })?;
        let temp_path = temp_file.path().to_path_buf();
        let temp_conn =
            Connection::open(&temp_path).map_err(|e| AppError::Database(e.to_string()))?;

        temp_conn
            .execute_batch(sql_content)
            .map_err(|e| AppError::Database(format!("执行 SQL 导入失败: {e}")))?;

        // 补齐缺失表/索引并进行基础校验
        Self::create_tables_on_conn(&temp_conn)?;
        Self::apply_schema_migrations_on_conn(&temp_conn)?;
        prepare(&temp_conn)?;
        Self::validate_basic_state(&temp_conn)?;
        Self::restore_tables(&local_snapshot, &temp_conn, preserve_tables)?;
        let preserves_usage = [
            "proxy_request_logs",
            "session_log_sync",
            "session_meta_cache",
        ]
        .iter()
        .all(|table| preserve_tables.contains(table));
        if !preserves_usage {
            crate::session_manager::source_cache::revoke_usage_on(&temp_conn)?;
        }
        crate::services::library::restore_machine_state(&local_snapshot, &temp_conn)?;

        // 使用 Backup 将临时库原子写回主库
        {
            let mut main_conn = lock_conn!(self.conn);
            if preserves_usage {
                // Usage writes do not take config_io; freeze their latest ledger
                // and evidence under the same lock as the final replacement.
                Self::restore_tables(&main_conn, &temp_conn, preserve_tables)?;
            }
            Self::restore_local_usage_health(&main_conn, &temp_conn)?;
            let backup = Backup::new(&temp_conn, &mut main_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        let backup_id = backup_path
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_default();

        Ok(backup_id)
    }

    /// 创建内存快照以避免长时间持有数据库锁
    pub(crate) fn snapshot_to_memory(&self) -> Result<Connection, AppError> {
        let conn = lock_conn!(self.conn);
        let mut snapshot =
            Connection::open_in_memory().map_err(|e| AppError::Database(e.to_string()))?;

        {
            let backup =
                Backup::new(&conn, &mut snapshot).map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        Ok(snapshot)
    }

    fn validate_cc_switch_sql_export(sql: &str) -> Result<(), AppError> {
        let trimmed = sql.trim_start();
        if trimmed.starts_with(CC_SWITCH_SQL_EXPORT_HEADER) {
            return Ok(());
        }

        Err(AppError::localized(
            "backup.sql.invalid_format",
            "仅支持导入由 Ofox Switch 导出的 SQL 备份文件。",
            "Only SQL backups exported by Ofox Switch are supported.",
        ))
    }

    fn restore_tables(
        source_conn: &Connection,
        target_conn: &Connection,
        tables: &[&str],
    ) -> Result<(), AppError> {
        for table in tables {
            if !Self::table_exists(source_conn, table)? || !Self::table_exists(target_conn, table)?
            {
                continue;
            }

            let columns = Self::get_table_columns(source_conn, table)?;
            if columns.is_empty() {
                continue;
            }

            target_conn
                .execute(&format!("DELETE FROM \"{table}\""), [])
                .map_err(|e| AppError::Database(format!("清空表 {table} 失败: {e}")))?;

            let placeholders = (1..=columns.len())
                .map(|idx| format!("?{idx}"))
                .collect::<Vec<_>>()
                .join(", ");
            let cols = columns
                .iter()
                .map(|column| format!("\"{column}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let insert_sql = format!("INSERT INTO \"{table}\" ({cols}) VALUES ({placeholders})");

            let mut stmt = source_conn
                .prepare(&format!("SELECT * FROM \"{table}\""))
                .map_err(|e| AppError::Database(format!("读取表 {table} 失败: {e}")))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| AppError::Database(format!("查询表 {table} 数据失败: {e}")))?;

            while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
                let mut values = Vec::with_capacity(columns.len());
                for idx in 0..columns.len() {
                    values.push(
                        row.get::<_, rusqlite::types::Value>(idx)
                            .map_err(|e| AppError::Database(e.to_string()))?,
                    );
                }

                target_conn
                    .execute(&insert_sql, rusqlite::params_from_iter(values.iter()))
                    .map_err(|e| AppError::Database(format!("恢复表 {table} 数据失败: {e}")))?;
            }
        }

        Ok(())
    }

    /// Periodic backup: create a new backup if the latest one is older than the configured interval
    pub(crate) fn periodic_backup_if_needed(&self) -> Result<(), AppError> {
        let interval_hours = crate::settings::effective_backup_interval_hours();
        if interval_hours > 0 {
            let backup_dir = get_app_config_dir().join("backups");
            if !backup_dir.exists() {
                self.backup_database_file()?;
            } else {
                let latest = fs::read_dir(&backup_dir).ok().and_then(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().map(|ext| ext == "db").unwrap_or(false))
                        .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
                        .max()
                });

                let interval_secs = u64::from(interval_hours) * 3600;
                let needs_backup = match latest {
                    None => true,
                    Some(last_modified) => {
                        last_modified.elapsed().unwrap_or_default()
                            > std::time::Duration::from_secs(interval_secs)
                    }
                };

                if needs_backup {
                    log::info!(
                        "Periodic backup: latest backup is older than {interval_hours} hours, creating new backup"
                    );
                    self.backup_database_file()?;
                }
            }
        }

        // Periodic maintenance is always enabled, regardless of auto-backup settings.
        let mut reclaimed_rows = 0u64;
        match self.cleanup_old_stream_check_logs(7) {
            Ok(deleted) => {
                reclaimed_rows += deleted;
            }
            Err(e) => {
                log::warn!("Periodic stream_check_logs cleanup failed: {e}");
            }
        }
        match self.rollup_and_prune(super::dao::usage_rollup::USAGE_DETAIL_RETAIN_DAYS) {
            Ok(deleted) => {
                reclaimed_rows += deleted;
            }
            Err(e) => {
                log::warn!("Periodic rollup_and_prune failed: {e}");
            }
        }
        if reclaimed_rows > 0 {
            let conn = lock_conn!(self.conn);
            if let Err(e) = conn.execute_batch("PRAGMA incremental_vacuum;") {
                log::warn!("Periodic incremental vacuum failed: {e}");
            }
        }

        Ok(())
    }

    /// 生成一致性快照备份，返回备份文件路径（不存在主库时返回 None）
    pub(crate) fn backup_database_file(&self) -> Result<Option<PathBuf>, AppError> {
        let db_path = get_app_config_dir().join("ofox-desktop.db");
        if !db_path.exists() {
            return Ok(None);
        }

        let backup_dir = db_path
            .parent()
            .ok_or_else(|| AppError::Config("无效的数据库路径".to_string()))?
            .join("backups");

        // 备份里有完整的库（含 API key）：目录只给自己；老版本建的 0o755 目录也收紧。
        create_private_dir_all(&backup_dir)?;
        set_file_mode(&backup_dir, 0o700)?;

        let base_id = format!("db_backup_{}", Local::now().format("%Y%m%d_%H%M%S"));
        let mut backup_id = base_id.clone();
        let mut backup_path = backup_dir.join(format!("{backup_id}.db"));
        let mut counter = 1;
        while backup_path.exists() {
            backup_id = format!("{base_id}_{counter}");
            backup_path = backup_dir.join(format!("{backup_id}.db"));
            counter += 1;
        }

        // 先建成 0o600 的空文件，备份内容写进去时就已经是私有的。
        ensure_private_file(&backup_path)?;
        {
            let conn = lock_conn!(self.conn);
            let mut dest_conn =
                Connection::open(&backup_path).map_err(|e| AppError::Database(e.to_string()))?;
            let backup = Backup::new(&conn, &mut dest_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        Self::cleanup_db_backups(&backup_dir)?;
        Ok(Some(backup_path))
    }

    /// 清理旧的数据库备份，保留最新的 N 个
    fn cleanup_db_backups(dir: &Path) -> Result<(), AppError> {
        let retain = crate::settings::effective_backup_retain_count();
        let entries = match fs::read_dir(dir) {
            Ok(iter) => iter
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .map(|ext| ext == "db")
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>(),
            Err(_) => return Ok(()),
        };

        if entries.len() <= retain {
            return Ok(());
        }

        let remove_count = entries.len().saturating_sub(retain);
        let mut sorted = entries;
        sorted.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());

        for entry in sorted.into_iter().take(remove_count) {
            if let Err(err) = fs::remove_file(entry.path()) {
                log::warn!("删除旧数据库备份失败 {}: {}", entry.path().display(), err);
            }
        }
        Ok(())
    }

    /// 基础状态校验
    fn validate_basic_state(conn: &Connection) -> Result<(), AppError> {
        let provider_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;
        let mcp_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mcp_servers", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;

        let skill_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM skills", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;
        let resources = crate::services::library::snapshot_from_connection(conn)?;

        if provider_count == 0
            && mcp_count == 0
            && skill_count == 0
            && resources.definitions.instructions.is_empty()
        {
            return Err(AppError::Config(
                "导入的 SQL 未包含有效的供应商或 MCP 数据".to_string(),
            ));
        }
        Ok(())
    }

    /// 导出数据库为 SQL 文本
    fn dump_sql(conn: &Connection, skip_tables: &[&str]) -> Result<String, AppError> {
        let mut output = String::new();
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let user_version: i64 = conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .unwrap_or(0);

        output.push_str(&format!(
            "-- Ofox Switch SQLite 导出\n-- 生成时间: {timestamp}\n-- user_version: {user_version}\n"
        ));
        output.push_str("PRAGMA foreign_keys=OFF;\n");
        output.push_str(&format!("PRAGMA user_version={user_version};\n"));
        output.push_str("BEGIN TRANSACTION;\n");

        // 导出 schema
        let mut stmt = conn
            .prepare(
                "SELECT type, name, tbl_name, sql
                 FROM sqlite_master
                 WHERE sql NOT NULL AND type IN ('table','index','trigger','view')
                 ORDER BY type='table' DESC, name",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut tables = Vec::new();
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Database(e.to_string()))?;
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let obj_type: String = row.get(0).map_err(|e| AppError::Database(e.to_string()))?;
            let name: String = row.get(1).map_err(|e| AppError::Database(e.to_string()))?;
            let sql: String = row.get(3).map_err(|e| AppError::Database(e.to_string()))?;

            // 跳过 SQLite 内部对象（如 sqlite_sequence）
            if name.starts_with("sqlite_") {
                continue;
            }

            output.push_str(&sql);
            output.push_str(";\n");

            if obj_type == "table" && !name.starts_with("sqlite_") {
                tables.push(name);
            }
        }

        // 导出数据
        for table in tables {
            if skip_tables.iter().any(|t| *t == table) {
                continue;
            }
            let columns = Self::get_table_columns(conn, &table)?;
            if columns.is_empty() {
                continue;
            }

            let mut stmt = conn
                .prepare(&format!("SELECT * FROM \"{table}\""))
                .map_err(|e| AppError::Database(e.to_string()))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| AppError::Database(e.to_string()))?;

            while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
                if table == "settings" {
                    let key_index = columns
                        .iter()
                        .position(|column| column == "key")
                        .ok_or_else(|| AppError::Database("settings has no key".into()))?;
                    let key: String = row
                        .get(key_index)
                        .map_err(|e| AppError::Database(e.to_string()))?;
                    if key == crate::services::library::MACHINE_KEY
                        || key == crate::services::library::batch::SETTING_KEY
                    {
                        continue;
                    }
                    if key == crate::services::usage_health::SETTING_KEY {
                        continue;
                    }
                }
                let mut values = Vec::with_capacity(columns.len());
                for idx in 0..columns.len() {
                    let value = row
                        .get_ref(idx)
                        .map_err(|e| AppError::Database(e.to_string()))?;
                    values.push(Self::format_sql_value(value)?);
                }

                let cols = columns
                    .iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                output.push_str(&format!(
                    "INSERT INTO \"{table}\" ({cols}) VALUES ({});\n",
                    values.join(", ")
                ));
            }
        }

        output.push_str("COMMIT;\nPRAGMA foreign_keys=ON;\n");
        Ok(output)
    }

    /// This single settings row describes local native sources. Copy its fresh
    /// value (including absence) under the final replacement lock; unrelated
    /// settings still come from the selected import or backup.
    fn restore_local_usage_health(
        source: &Connection,
        target: &Connection,
    ) -> Result<(), AppError> {
        use rusqlite::OptionalExtension;
        let key = crate::services::usage_health::SETTING_KEY;
        let value: Option<String> = source
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        target.execute("DELETE FROM settings WHERE key=?1", [key])?;
        if let Some(value) = value {
            target.execute(
                "INSERT INTO settings(key,value) VALUES(?1,?2)",
                [key, &value],
            )?;
        }
        Ok(())
    }

    /// 获取表的列名列表
    fn get_table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, AppError> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .map_err(|e| AppError::Database(e.to_string()))?;
        let iter = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut columns = Vec::new();
        for col in iter {
            columns.push(col.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(columns)
    }

    /// 格式化 SQL 值
    fn format_sql_value(value: ValueRef<'_>) -> Result<String, AppError> {
        match value {
            ValueRef::Null => Ok("NULL".to_string()),
            ValueRef::Integer(i) => Ok(i.to_string()),
            ValueRef::Real(f) => Ok(f.to_string()),
            ValueRef::Text(t) => {
                let text = std::str::from_utf8(t)
                    .map_err(|e| AppError::Database(format!("文本字段不是有效的 UTF-8: {e}")))?;
                let escaped = text.replace('\'', "''");
                Ok(format!("'{escaped}'"))
            }
            ValueRef::Blob(bytes) => {
                let mut s = String::from("X'");
                for b in bytes {
                    use std::fmt::Write;
                    let _ = write!(&mut s, "{b:02X}");
                }
                s.push('\'');
                Ok(s)
            }
        }
    }

    /// List all database backup files, sorted by creation time (newest first)
    pub fn list_backups() -> Result<Vec<BackupEntry>, AppError> {
        let backup_dir = get_app_config_dir().join("backups");
        if !backup_dir.exists() {
            return Ok(vec![]);
        }

        let mut entries: Vec<BackupEntry> = fs::read_dir(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "db").unwrap_or(false))
            .filter_map(|e| {
                let metadata = e.metadata().ok()?;
                let filename = e.file_name().to_string_lossy().to_string();
                let size_bytes = metadata.len();
                let created_at = metadata
                    .modified()
                    .ok()
                    .map(|t| {
                        let dt: chrono::DateTime<Utc> = t.into();
                        dt.to_rfc3339()
                    })
                    .unwrap_or_default();
                Some(BackupEntry {
                    filename,
                    size_bytes,
                    created_at,
                })
            })
            .collect();

        // Sort by created_at descending (newest first)
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(entries)
    }

    /// Restore database from a backup file. Returns the safety backup ID.
    pub fn restore_from_backup(&self, filename: &str) -> Result<String, AppError> {
        let _io = crate::config_io::lock();
        // Security: validate filename to prevent path traversal
        if filename.contains("..")
            || filename.contains('/')
            || filename.contains('\\')
            || !filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        let backup_dir = get_app_config_dir().join("backups");
        let backup_path = backup_dir.join(filename);

        if !backup_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {filename}"
            )));
        }

        // Open without CREATE and fully stage the selected source before a
        // safety backup's retention cleanup can unlink that source.
        let source_conn =
            Connection::open_with_flags(&backup_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| AppError::Config("backup.sourceUnreadable".into()))?;
        let managed_schema: bool = source_conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='providers')", [], |row| row.get(0))
            .map_err(|_| AppError::Config("backup.sourceUnreadable".into()))?;
        if !managed_schema {
            return Err(AppError::Config("backup.sourceInvalid".into()));
        }
        let local_snapshot = self.snapshot_to_memory()?;

        // Validate and preserve local proof in a staging database before replacing the main one.
        let mut staged =
            Connection::open_in_memory().map_err(|e| AppError::Database(e.to_string()))?;
        Backup::new(&source_conn, &mut staged)
            .map_err(|e| AppError::Database(e.to_string()))?
            .step(-1)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Self::create_tables_on_conn(&staged)?;
        Self::apply_schema_migrations_on_conn(&staged)?;
        Self::restore_tables(&local_snapshot, &staged, MACHINE_LOCAL_TABLES)?;
        crate::session_manager::source_cache::revoke_usage_on(&staged)?;
        crate::services::library::restore_machine_state(&local_snapshot, &staged)?;
        drop(source_conn);
        let safety_backup = self.backup_database_file()?;
        let safety_id = safety_backup
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_default();
        {
            let mut main_conn = lock_conn!(self.conn);
            Self::restore_local_usage_health(&main_conn, &staged)?;
            Backup::new(&staged, &mut main_conn)
                .map_err(|e| AppError::Database(e.to_string()))?
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        self.ensure_model_pricing_seeded()?;

        log::info!("Database restored from backup: {filename}, safety backup: {safety_id}");
        Ok(safety_id)
    }

    /// Rename a backup file. Returns the new filename.
    pub fn rename_backup(old_filename: &str, new_name: &str) -> Result<String, AppError> {
        // Validate old filename (path traversal + .db suffix)
        if old_filename.contains("..")
            || old_filename.contains('/')
            || old_filename.contains('\\')
            || !old_filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        // Clean new name
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return Err(AppError::InvalidInput(
                "New name cannot be empty".to_string(),
            ));
        }

        // Length limit (without .db suffix)
        let name_part = trimmed.strip_suffix(".db").unwrap_or(trimmed);
        if name_part.len() > 100 {
            return Err(AppError::InvalidInput(
                "Name too long (max 100 characters)".to_string(),
            ));
        }

        // Prevent path traversal in new name
        if name_part.contains("..")
            || name_part.contains('/')
            || name_part.contains('\\')
            || name_part.contains('\0')
        {
            return Err(AppError::InvalidInput(
                "Invalid characters in new name".to_string(),
            ));
        }

        let new_filename = format!("{name_part}.db");

        let backup_dir = get_app_config_dir().join("backups");
        let old_path = backup_dir.join(old_filename);
        let new_path = backup_dir.join(&new_filename);

        if !old_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {old_filename}"
            )));
        }

        if new_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "A backup named '{new_filename}' already exists"
            )));
        }

        fs::rename(&old_path, &new_path).map_err(|e| AppError::io(&old_path, e))?;
        log::info!("Renamed backup: {old_filename} -> {new_filename}");
        Ok(new_filename)
    }

    /// Delete a backup file permanently.
    pub fn delete_backup(filename: &str) -> Result<(), AppError> {
        // Validate filename (path traversal + .db suffix)
        if filename.contains("..")
            || filename.contains('/')
            || filename.contains('\\')
            || !filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        let backup_path = get_app_config_dir().join("backups").join(filename);
        if !backup_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {filename}"
            )));
        }

        fs::remove_file(&backup_path).map_err(|e| AppError::io(&backup_path, e))?;
        log::info!("Deleted backup: {filename}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Database;
    use crate::error::AppError;
    use crate::settings::{update_settings, AppSettings};
    use serial_test::serial;

    #[test]
    fn library_machine_receipts_and_journal_never_leave_sql_exports() {
        let db = Database::memory().unwrap();
        let state = crate::services::library::MachineState {
            epoch: 4,
            ..Default::default()
        };
        db.set_setting(
            crate::services::library::MACHINE_KEY,
            &serde_json::to_string(&state).unwrap(),
        )
        .unwrap();
        for dump in [
            db.export_sql_string().unwrap(),
            db.export_sql_string_for_sync().unwrap(),
        ] {
            assert!(!dump.contains(crate::services::library::MACHINE_KEY));
        }
    }

    #[test]
    #[serial]
    fn library_restore_keeps_local_receipts_and_invalidates_old_confirmation() {
        let sandbox = crate::test_sandbox::sandbox();
        let local = Database::memory().unwrap();
        let remote = Database::memory().unwrap();
        let definition = |id: &str| crate::services::library::InstructionSet {
            id: id.into(),
            name: id.into(),
            content: "shared".into(),
            revision: 0,
        };
        crate::services::library::save_instruction(&local, definition("local"), None).unwrap();
        crate::services::library::save_instruction(&remote, definition("remote"), None).unwrap();
        {
            let conn = remote.conn.lock().unwrap();
            conn.execute("INSERT INTO providers(id,app_type,name,settings_config,meta) VALUES ('fixture','claude','Fixture','{}','{}')",[]).unwrap();
        }
        let path = sandbox.home().join("AGENTS.md");
        std::fs::write(&path, "personal").unwrap();
        let applied = crate::services::library::coordinator::preview_at(
            &local,
            "local",
            &[("codex".into(), path.clone())],
        )
        .unwrap();
        crate::services::library::apply_operation(&local, &applied.id).unwrap();
        let pending =
            crate::services::library::coordinator::preview_at(&local, "local", &[]).unwrap();
        let before = local.library_snapshot().unwrap();
        let file = std::fs::read_to_string(&path).unwrap();
        let foreign_machine = crate::services::library::MachineState {
            epoch: 99,
            ..Default::default()
        };
        let machine = serde_json::to_string(&foreign_machine)
            .unwrap()
            .replace('\'', "''");
        let dump = format!(
            "{}\nINSERT OR REPLACE INTO settings(key,value) VALUES ('{}','{}');",
            remote.export_sql_string().unwrap(),
            crate::services::library::MACHINE_KEY,
            machine
        );
        local.import_sql_string(&dump).unwrap();
        let after = local.library_snapshot().unwrap();
        assert_eq!(after.machine.receipts, before.machine.receipts);
        assert_eq!(after.machine.epoch, before.machine.epoch + 1);
        assert!(after.definitions.instructions.contains_key("remote"));
        assert!(!after.definitions.instructions.contains_key("local"));
        assert!(crate::services::library::apply_operation(&local, &pending.id).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), file);
    }

    #[test]
    fn sync_import_preserves_local_only_tables() -> Result<(), AppError> {
        let remote_db = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(remote_db.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('remote-provider', 'claude', 'Remote Provider', '{}', '{}')",
                [],
            )?;
        }
        let remote_sql = remote_db.export_sql_string_for_sync()?;

        let local_db = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(local_db.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('local-provider', 'claude', 'Local Provider', '{}', '{}')",
                [],
            )?;
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES ('req-1', 'local-provider', 'claude', 'claude-3', 100, 50, '0.01', 120, 200, 1000)",
                [],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model, request_count, success_count,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, avg_latency_ms
                ) VALUES ('2026-03-01', 'claude', 'local-provider', 'claude-3', 7, 7, 700, 350, 0, 0, '0.07', 120)",
                [],
            )?;
            conn.execute(
                "INSERT INTO stream_check_logs (
                    provider_id, provider_name, app_type, status, success, message,
                    response_time_ms, http_status, model_used, retry_count, tested_at
                ) VALUES ('local-provider', 'Local Provider', 'claude', 'operational', 1, 'ok', 42, 200, 'claude-3', 0, 1000)",
                [],
            )?;
        }

        local_db.import_sql_string_for_sync(&remote_sql)?;

        let remote_provider_exists: i64 = {
            let conn = crate::database::lock_conn!(local_db.conn);
            conn.query_row(
                "SELECT COUNT(*) FROM providers WHERE id = 'remote-provider' AND app_type = 'claude'",
                [],
                |row| row.get(0),
            )?
        };
        assert_eq!(
            remote_provider_exists, 1,
            "remote config should be imported"
        );

        let (request_logs, rollups, stream_logs): (i64, i64, i64) = {
            let conn = crate::database::lock_conn!(local_db.conn);
            let request_logs =
                conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                    row.get(0)
                })?;
            let rollups =
                conn.query_row("SELECT COUNT(*) FROM usage_daily_rollups", [], |row| {
                    row.get(0)
                })?;
            let stream_logs =
                conn.query_row("SELECT COUNT(*) FROM stream_check_logs", [], |row| {
                    row.get(0)
                })?;
            (request_logs, rollups, stream_logs)
        };
        assert_eq!(request_logs, 1, "local request logs should be preserved");
        assert_eq!(rollups, 1, "local rollups should be preserved");
        assert_eq!(
            stream_logs, 1,
            "local stream check logs should be preserved"
        );

        Ok(())
    }

    #[test]
    #[serial]
    fn periodic_maintenance_runs_even_when_auto_backup_disabled() -> Result<(), AppError> {
        let old_test_home = std::env::var_os("CC_SWITCH_TEST_HOME");
        let test_home =
            std::env::temp_dir().join("cc-switch-periodic-maintenance-backup-disabled-test");
        let _ = std::fs::remove_dir_all(&test_home);
        std::fs::create_dir_all(&test_home).expect("create test home");
        std::env::set_var("CC_SWITCH_TEST_HOME", &test_home);

        let settings = AppSettings {
            backup_interval_hours: Some(0),
            ..AppSettings::default()
        };
        update_settings(settings).expect("disable auto backup");

        let db = Database::memory()?;
        let now = chrono::Utc::now().timestamp();
        let old_ts = now - 40 * 86400;
        let old_stream_ts = now - 8 * 86400;

        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES ('old-req', 'p1', 'claude', 'claude-3', 100, 50, '0.01', 100, 200, ?1)",
                [old_ts],
            )?;
            conn.execute(
                "INSERT INTO stream_check_logs (
                    provider_id, provider_name, app_type, status, success, message,
                    response_time_ms, http_status, model_used, retry_count, tested_at
                ) VALUES ('p1', 'Provider 1', 'claude', 'operational', 1, 'ok', 42, 200, 'claude-3', 0, ?1)",
                [old_stream_ts],
            )?;
        }

        db.periodic_backup_if_needed()?;

        let (remaining_request_logs, stream_logs, rollups): (i64, i64, i64) = {
            let conn = crate::database::lock_conn!(db.conn);
            let remaining_request_logs =
                conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                    row.get(0)
                })?;
            let stream_logs =
                conn.query_row("SELECT COUNT(*) FROM stream_check_logs", [], |row| {
                    row.get(0)
                })?;
            let rollups =
                conn.query_row("SELECT COUNT(*) FROM usage_daily_rollups", [], |row| {
                    row.get(0)
                })?;
            (remaining_request_logs, stream_logs, rollups)
        };

        assert_eq!(
            remaining_request_logs, 0,
            "old request logs should still be pruned when auto backup is disabled"
        );
        assert_eq!(
            stream_logs, 0,
            "old stream check logs should still be pruned when auto backup is disabled"
        );
        assert_eq!(rollups, 1, "old request logs should be rolled up");

        match old_test_home {
            Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
        }

        Ok(())
    }

    /// 临时把 `CC_SWITCH_TEST_HOME` 指向独立目录，Drop 时还原；配合 `#[serial]`。
    struct TestHome {
        previous: Option<std::ffi::OsString>,
        dir: std::path::PathBuf,
    }

    impl TestHome {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ofox-backup-test-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create test home");
            let previous = std::env::var_os("CC_SWITCH_TEST_HOME");
            std::env::set_var("CC_SWITCH_TEST_HOME", &dir);
            Self { previous, dir }
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
                None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    #[serial]
    fn selected_backup_is_staged_before_retention_cleanup_can_remove_it() -> Result<(), AppError> {
        let _sandbox = crate::test_sandbox::sandbox();
        let mut settings = crate::settings::get_settings();
        settings.backup_retain_count = Some(1);
        crate::settings::update_settings(settings)?;
        let db = Database::init()?;
        db.set_setting("selected-backup-content", "old-definition")?;
        let source = db.backup_database_file()?.unwrap();
        std::fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10),
            ))
            .unwrap();
        db.set_setting("selected-backup-content", "current-definition")?;
        db.upsert_bind_record("codex", "local-proof")?;
        db.restore_from_backup(source.file_name().unwrap().to_str().unwrap())?;
        assert_eq!(
            db.get_setting("selected-backup-content")?.as_deref(),
            Some("old-definition")
        );
        assert_eq!(db.get_bind_record("codex")?.unwrap().record, "local-proof");
        Ok(())
    }

    #[test]
    fn exports_never_include_bind_snapshots() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.upsert_bind_record("codex", "{\"secret\":\"local-only\"}")?;
        for dump in [db.export_sql_string()?, db.export_sql_string_for_sync()?] {
            assert!(
                !dump.contains("local-only"),
                "bind snapshot leaked into export"
            );
        }
        Ok(())
    }

    #[test]
    #[serial]
    fn imports_keep_local_bind_snapshots() -> Result<(), AppError> {
        let _home = TestHome::new("import-keeps-bind");
        let remote = Database::memory()?;
        {
            // Import only accepts dumps that contain providers or MCP data.
            let conn = crate::database::lock_conn!(remote.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('remote-provider', 'claude', 'Remote Provider', '{}', '{}')",
                [],
            )?;
        }
        remote.upsert_bind_record("codex", "remote")?;
        let full = remote.export_sql_string()?;
        let sync = remote.export_sql_string_for_sync()?;

        let local = Database::memory()?;
        local.upsert_bind_record("codex", "local")?;
        local.import_sql_string(&full)?;
        assert_eq!(local.get_bind_record("codex")?.unwrap().record, "local");
        local.import_sql_string_for_sync(&sync)?;
        assert_eq!(local.get_bind_record("codex")?.unwrap().record, "local");
        Ok(())
    }

    #[test]
    #[serial]
    fn restoring_a_backup_keeps_local_bind_snapshots() -> Result<(), AppError> {
        let home = TestHome::new("restore-keeps-bind");
        let backup_dir = crate::config::get_app_config_dir().join("backups");
        std::fs::create_dir_all(&backup_dir).expect("create backups dir");

        let old = Database::memory()?;
        old.upsert_bind_record("claude", "stale-from-backup")?;
        {
            let conn = crate::database::lock_conn!(old.conn);
            conn.execute(
                "VACUUM INTO ?1",
                [backup_dir.join("old.db").to_string_lossy().to_string()],
            )?;
        }

        let db = Database::memory()?;
        db.upsert_bind_record("codex", "current")?;
        db.restore_from_backup("old.db")?;
        assert!(db.get_bind_record("claude")?.is_none());
        assert_eq!(db.get_bind_record("codex")?.unwrap().record, "current");
        drop(home);
        Ok(())
    }

    /// One row in each v11 usage table, tagged with `tag`.
    fn insert_usage_local_rows(db: &Database, tag: &str) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT INTO session_usage_archive (app_type, session_id, model, input_tokens,
                output_tokens, cache_read_tokens, cache_creation_tokens, request_count,
                cost_usd, first_at, last_at)
             VALUES ('claude', ?1, 'm', 1, 1, 0, 0, 1, '0.1', 1, 2)",
            [tag],
        )?;
        conn.execute(
            "INSERT INTO session_meta_cache (source_path, provider_id, session_id, stamp,
                parser_version, meta_json, updated_at)
             VALUES (?1, 'claude', 's', 'st', 1, '{}', 1)",
            [tag],
        )?;
        conn.execute(
            "INSERT INTO balance_samples (org_id, at_ms, amount) VALUES (?1, 1, 2.5)",
            [tag],
        )?;
        conn.execute(
            "INSERT INTO maintenance_flags (key, value) VALUES ('backfill_session_archive', ?1)",
            [tag],
        )?;
        Ok(())
    }

    /// Machine-local: never exported, always kept.
    const USAGE_MACHINE_LOCAL: [(&str, &str); 2] = [
        ("session_meta_cache", "source_path"),
        ("balance_samples", "org_id"),
    ];
    /// Bound to the request logs: carried by full exports and imports, skipped and
    /// kept by WebDAV sync.
    const USAGE_LOG_BOUND: [(&str, &str); 2] = [
        ("session_usage_archive", "session_id"),
        ("maintenance_flags", "value"),
    ];

    fn column_values(db: &Database, table: &str, column: &str) -> Result<Vec<String>, AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        let condition = if table == "maintenance_flags" {
            " WHERE key != 'usage_ledger_epoch_v1'"
        } else {
            ""
        };
        let values = conn
            .prepare(&format!("SELECT {column} FROM {table}{condition}"))?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(values)
    }

    fn remote_with_usage_rows() -> Result<Database, AppError> {
        let remote = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(remote.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('remote-provider', 'claude', 'Remote Provider', '{}', '{}')",
                [],
            )?;
            // an export from a current install, so importing it migrates nothing
            Database::set_user_version(&conn, crate::database::SCHEMA_VERSION)?;
        }
        insert_usage_local_rows(&remote, "remote")?;
        Ok(remote)
    }

    #[test]
    fn exports_never_include_machine_local_usage_tables() -> Result<(), AppError> {
        let db = remote_with_usage_rows()?;
        let full = db.export_sql_string()?;
        let sync = db.export_sql_string_for_sync()?;
        for (table, _) in USAGE_MACHINE_LOCAL {
            for dump in [&full, &sync] {
                assert!(
                    !dump.contains(&format!("INSERT INTO \"{table}\"")),
                    "{table} leaked into an export"
                );
            }
        }
        for (table, _) in USAGE_LOG_BOUND {
            assert!(
                full.contains(&format!("INSERT INTO \"{table}\"")),
                "{table} full"
            );
            assert!(
                !sync.contains(&format!("INSERT INTO \"{table}\"")),
                "{table} sync"
            );
        }
        Ok(())
    }

    #[test]
    #[serial]
    fn a_full_import_brings_the_archive_with_the_logs_and_keeps_local_caches(
    ) -> Result<(), AppError> {
        let _home = TestHome::new("import-usage-archive");
        let full = remote_with_usage_rows()?.export_sql_string()?;
        let local = Database::memory()?;
        insert_usage_local_rows(&local, "local")?;

        local.import_sql_string(&full)?;

        for (table, column) in USAGE_MACHINE_LOCAL {
            assert_eq!(column_values(&local, table, column)?, ["local"], "{table}");
        }
        for (table, column) in USAGE_LOG_BOUND {
            assert_eq!(column_values(&local, table, column)?, ["remote"], "{table}");
        }
        Ok(())
    }

    #[test]
    #[serial]
    fn a_webdav_import_keeps_every_local_usage_table() -> Result<(), AppError> {
        let _home = TestHome::new("sync-import-usage-archive");
        let sync = remote_with_usage_rows()?.export_sql_string_for_sync()?;
        let local = Database::memory()?;
        insert_usage_local_rows(&local, "local")?;

        local.import_sql_string_for_sync(&sync)?;

        for (table, column) in USAGE_MACHINE_LOCAL.iter().chain(USAGE_LOG_BOUND.iter()) {
            assert_eq!(column_values(&local, table, column)?, ["local"], "{table}");
        }
        Ok(())
    }

    #[test]
    #[serial]
    fn webdav_sync_keeps_this_machines_session_sync_progress() -> Result<(), AppError> {
        let _home = TestHome::new("sync-keeps-session-log-sync");
        let remote = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(remote.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('remote-provider', 'claude', 'Remote Provider', '{}', '{}')",
                [],
            )?;
            conn.execute(
                "INSERT INTO session_log_sync VALUES ('/remote/transcript.jsonl', 1, 99, 1)",
                [],
            )?;
        }
        let sync = remote.export_sql_string_for_sync()?;
        assert!(
            !sync.contains("/remote/transcript.jsonl"),
            "sync progress leaked into a WebDAV export"
        );

        let local = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(local.conn);
            conn.execute(
                "INSERT INTO session_log_sync VALUES ('/local/transcript.jsonl', 1, 42, 1)",
                [],
            )?;
        }
        local.import_sql_string_for_sync(&sync)?;

        let conn = crate::database::lock_conn!(local.conn);
        let rows: Vec<(String, i64)> = conn
            .prepare("SELECT file_path, last_line_offset FROM session_log_sync")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        assert_eq!(rows, [("/local/transcript.jsonl".to_string(), 42)]);
        Ok(())
    }

    type LocalUsageState = Vec<(String, Vec<Vec<rusqlite::types::Value>>)>;

    fn local_usage_state(db: &Database) -> Result<LocalUsageState, AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        [
            "proxy_request_logs",
            "session_log_sync",
            "session_meta_cache",
            "maintenance_flags",
            "usage_daily_rollups",
            "session_usage_archive",
        ]
        .into_iter()
        .map(|table| {
            let mut statement = conn.prepare(&format!("SELECT * FROM {table} ORDER BY 1"))?;
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|index| row.get::<_, rusqlite::types::Value>(index))
                        .collect::<Result<Vec<_>, _>>()
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((table.to_string(), rows))
        })
        .collect()
    }

    fn commit_concurrent_usage(
        db: &Database,
        path: &std::path::Path,
        at: i64,
    ) -> Result<(), AppError> {
        use crate::services::usage_source::{native_key, Prepared, Snapshot};
        use crate::services::usage_sync_db::{commit_sync_batch, CheckpointChange, SyncState};
        use crate::session_manager::source_cache::{merge_on, Part};
        std::fs::write(path, "{}\n").map_err(|error| AppError::io(path, error))?;
        let mut source = Prepared::load(db, Snapshot::file(path)?, "claude", None, 2)?;
        let next = SyncState {
            modified: 1,
            offset: 1,
        };
        commit_sync_batch(
            db,
            &[CheckpointChange {
                key: path.to_string_lossy().into(),
                expected: None,
                next: Some(next),
            }],
            |conn| {
                source.check_on(conn)?;
                conn.execute(
                    "INSERT INTO proxy_request_logs
                    (request_id,provider_id,app_type,model,input_tokens,output_tokens,
                     total_cost_usd,latency_ms,status_code,session_id,created_at,data_source)
                    VALUES ('session:concurrent-event','_session','claude','fixture',100,10,
                            '0.123',0,200,'concurrent-session',?1,'session_log')",
                    [at],
                )?;
                source.evidence.acknowledge(&native_key(
                    "claude",
                    "concurrent-session",
                    "concurrent-event",
                ));
                source.save_on(conn, next)?;
                conn.execute(
                    "INSERT INTO maintenance_flags (key, value) VALUES
                     ('usage_prune_frontier_utc_v1:claude',
                      '{\"version\":1,\"cutoff\":1234000,\"legacyUnknown\":false}')",
                    [],
                )?;
                merge_on(
                    conn,
                    &path.to_string_lossy(),
                    "claude",
                    Some("concurrent-session"),
                    Part::Metadata,
                    &None,
                    Some(serde_json::json!({"parserVersion":2,
                    "stamp":"concurrent","details":{"candidates":{},"messageCount":1}})),
                )?;
                merge_on(
                    conn,
                    &path.to_string_lossy(),
                    "claude",
                    None,
                    Part::Analytics,
                    &None,
                    Some(serde_json::json!({"version":1,"futureAnalytics":{"preserve":true}})),
                )?;
                Ok(())
            },
        )
    }

    #[test]
    #[serial]
    fn webdav_sync_preserves_usage_committed_after_its_initial_snapshot() -> Result<(), AppError> {
        let sandbox = crate::test_sandbox::sandbox();
        let remote = Database::memory()?;
        remote.conn.lock().unwrap().execute(
            "INSERT INTO providers(id,app_type,name,settings_config,meta)
             VALUES('remote-provider','claude','Remote','{}','{}')",
            [],
        )?;
        let sql = remote.export_sql_string_for_sync()?;
        for prune in [false, true] {
            let local = Database::memory()?;
            local.conn.lock().unwrap().execute(
                "INSERT INTO maintenance_flags(key,value) VALUES('usage_ledger_epoch_v1','local-epoch')", [],)?;
            let path = sandbox
                .home()
                .join(if prune { "pruned.jsonl" } else { "live.jsonl" });
            let mut committed = None;
            local.import_sync_with_resources(&sql, |_| {
                let at = if prune {
                    (chrono::Utc::now() - chrono::Duration::days(40)).timestamp()
                } else {
                    chrono::Utc::now().timestamp()
                };
                commit_concurrent_usage(&local, &path, at)?;
                if prune {
                    local.rollup_and_prune(30)?;
                }
                committed = Some(local_usage_state(&local)?);
                Ok(())
            })?;
            assert_eq!(
                local_usage_state(&local)?,
                committed.unwrap(),
                "WebDAV replaced a successfully committed local usage batch (prune={prune})"
            );
            let remote_count: i64 = local.conn.lock().unwrap().query_row(
                "SELECT COUNT(*) FROM providers WHERE id='remote-provider'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(remote_count, 1, "remote configuration should still import");
        }
        Ok(())
    }

    #[test]
    #[serial]
    fn webdav_late_local_copy_failure_preserves_the_concurrent_main_commit() -> Result<(), AppError>
    {
        let sandbox = crate::test_sandbox::sandbox();
        let remote = Database::memory()?;
        remote.conn.lock().unwrap().execute_batch(
            "CREATE TRIGGER fail_late_cache BEFORE INSERT ON session_meta_cache
             WHEN NEW.session_id='concurrent-session'
             BEGIN SELECT RAISE(ABORT,'fixture late cache copy'); END;
             INSERT INTO providers(id,app_type,name,settings_config,meta)
             VALUES('remote-provider','claude','Remote','{}','{}');",
        )?;
        let sql = remote.export_sql_string_for_sync()?;
        let local = Database::memory()?;
        local.conn.lock().unwrap().execute(
            "INSERT INTO providers(id,app_type,name,settings_config,meta)
             VALUES('local-provider','claude','Local','{}','{}')",
            [],
        )?;
        let mut committed = None;
        let result = local.import_sync_with_resources(&sql, |_| {
            commit_concurrent_usage(
                &local,
                &sandbox.home().join("late.jsonl"),
                chrono::Utc::now().timestamp(),
            )?;
            committed = Some(local_usage_state(&local)?);
            Ok(())
        });
        assert!(
            result.is_err(),
            "the final copy never observed the new local cache row"
        );
        assert_eq!(local_usage_state(&local)?, committed.unwrap());
        let providers: Vec<String> = local
            .conn
            .lock()
            .unwrap()
            .prepare("SELECT id FROM providers ORDER BY id")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        assert_eq!(
            providers,
            ["local-provider"],
            "failed staging replaced main configuration"
        );
        Ok(())
    }

    const HEALTH_KEY: &str = "usage_source_health_v1";

    fn health_remote_sql(sync: bool) -> Result<String, AppError> {
        let remote = Database::memory()?;
        remote.conn.lock().unwrap().execute(
            "INSERT INTO providers(id,app_type,name,settings_config,meta)
             VALUES('health-remote','claude','Remote','{}','{}')",
            [],
        )?;
        // Old clients may have exported this machine-local key. Import must
        // remove the foreign value even when the local device has no row.
        let dump = if sync {
            remote.export_sql_string_for_sync()?
        } else {
            remote.export_sql_string()?
        };
        Ok(format!(
            "{dump}\nINSERT INTO settings(key,value) VALUES('{HEALTH_KEY}','foreign-health');"
        ))
    }

    #[test]
    fn health_exports_exclude_only_the_machine_local_health_setting() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.set_setting(HEALTH_KEY, "local-only-health")?;
        db.set_setting("usage_source_health_v1_extra", "unrelated-setting")?;
        let snapshot = db.snapshot_to_memory()?;
        for dump in [
            db.export_sql_string()?,
            db.export_sql_string_for_sync()?,
            Database::export_sync_snapshot(&snapshot)?,
        ] {
            assert!(
                !dump.contains("local-only-health"),
                "local health leaked into SQL"
            );
            assert!(
                dump.contains("unrelated-setting"),
                "filter expanded beyond the exact key"
            );
        }
        Ok(())
    }

    #[test]
    #[serial]
    fn health_full_sql_import_preserves_local_and_removes_foreign_when_absent(
    ) -> Result<(), AppError> {
        let _sandbox = crate::test_sandbox::sandbox();
        let sql = health_remote_sql(false)?;
        for local_value in [Some("local-health"), None] {
            let local = Database::memory()?;
            if let Some(value) = local_value {
                local.set_setting(HEALTH_KEY, value)?;
            }
            local.import_sql_string(&sql)?;
            assert_eq!(local.get_setting(HEALTH_KEY)?.as_deref(), local_value);
        }
        Ok(())
    }

    #[test]
    #[serial]
    fn health_webdav_import_uses_fresh_local_value_and_absence() -> Result<(), AppError> {
        let _sandbox = crate::test_sandbox::sandbox();
        let sql = health_remote_sql(true)?;
        for fresh_value in [Some("new-local-health"), None] {
            let local = Database::memory()?;
            local.set_setting(HEALTH_KEY, "stale-local-health")?;
            local.import_sync_with_resources(&sql, |_| {
                match fresh_value {
                    Some(value) => local.set_setting(HEALTH_KEY, value)?,
                    None => {
                        local
                            .conn
                            .lock()
                            .unwrap()
                            .execute("DELETE FROM settings WHERE key=?1", [HEALTH_KEY])?;
                    }
                }
                Ok(())
            })?;
            assert_eq!(local.get_setting(HEALTH_KEY)?.as_deref(), fresh_value);
        }
        Ok(())
    }

    #[test]
    #[serial]
    fn health_full_import_takes_the_fresh_local_value() -> Result<(), AppError> {
        let _sandbox = crate::test_sandbox::sandbox();
        let local = Database::memory()?;
        local.set_setting(HEALTH_KEY, "stale-local-health")?;
        local.import_sql_string_inner(
            &health_remote_sql(false)?,
            super::MACHINE_LOCAL_TABLES,
            |_| {
                local.set_setting(HEALTH_KEY, "fresh-local-health")?;
                Ok(())
            },
        )?;
        assert_eq!(
            local.get_setting(HEALTH_KEY)?.as_deref(),
            Some("fresh-local-health")
        );
        Ok(())
    }

    #[test]
    #[serial]
    fn health_database_restore_keeps_current_local_health() -> Result<(), AppError> {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::init()?;
        db.set_setting(HEALTH_KEY, "old-backup-health")?;
        let source = db.backup_database_file()?.unwrap();
        db.set_setting(HEALTH_KEY, "current-local-health")?;
        db.restore_from_backup(source.file_name().unwrap().to_str().unwrap())?;
        assert_eq!(
            db.get_setting(HEALTH_KEY)?.as_deref(),
            Some("current-local-health")
        );
        Ok(())
    }
}
