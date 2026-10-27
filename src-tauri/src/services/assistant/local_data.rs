//! Local-only settings. The flag is thread scoped and used only for synchronous
//! assistant statements; unrelated configuration writers keep their notifications.
use crate::error::AppError;
use rusqlite::Connection;
use std::cell::Cell;

pub const PREFIXES: &[&str] = &[
    "assistant_v1:",
    "assistant_memory_v1:",
    "assistant_model_v1:",
    "assistant_restore_v1:",
];
pub fn is_private_key(key: &str) -> bool {
    PREFIXES.iter().any(|p| key.starts_with(p))
}
thread_local! { static PRIVATE_WRITE: Cell<u32> = const { Cell::new(0) }; }
pub fn should_notify(table: &str) -> bool {
    table != "settings" || PRIVATE_WRITE.with(|v| v.get() == 0)
}
pub fn write<T>(action: impl FnOnce() -> T) -> T {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            PRIVATE_WRITE.with(|v| v.set(v.get() - 1));
        }
    }
    PRIVATE_WRITE.with(|v| v.set(v.get() + 1));
    let _guard = Guard;
    action()
}

pub fn ensure_passive(conn: &Connection) -> Result<(), String> {
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM main.sqlite_master WHERE type IN ('trigger','view')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "assistant_storage_active_schema")?;
    if active != 0 {
        return Err("assistant_storage_active_schema".into());
    }
    // Use the authoritative PRAGMA, not its table-valued alias: an imported
    // ordinary table named pragma_table_list can shadow that alias.
    let mut statement = conn
        .prepare("PRAGMA main.table_list")
        .map_err(|_| "assistant_storage_active_schema")?;
    let kinds = statement
        .query_map([], |row| row.get::<_, String>(2))
        .map_err(|_| "assistant_storage_active_schema")?;
    for kind in kinds {
        if matches!(
            kind.map_err(|_| "assistant_storage_active_schema")?
                .as_str(),
            "virtual" | "shadow"
        ) {
            return Err("assistant_storage_active_schema".into());
        }
    }
    Ok(())
}
pub fn set(db: &crate::database::Database, key: &str, value: &str) -> Result<(), String> {
    let conn = db
        .conn
        .lock()
        .map_err(|_| "assistant_storage_unavailable")?;
    ensure_passive(&conn)?;
    write(|| {
        conn.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
            [key, value],
        )
    })
    .map_err(|_| "assistant_storage_unavailable")?;
    Ok(())
}

pub fn restore_local(source: &Connection, target: &Connection) -> Result<(), AppError> {
    ensure_passive(source).map_err(AppError::Config)?;
    ensure_passive(target).map_err(AppError::Config)?;
    // Called while holding the live connection lock, immediately before replacement.
    let error = |e: rusqlite::Error| AppError::Database(e.to_string());
    for prefix in PREFIXES {
        target
            .execute(
                "DELETE FROM settings WHERE substr(key,1,length(?1))=?1",
                [prefix],
            )
            .map_err(error)?;
        let mut query = source
            .prepare("SELECT key,value FROM settings WHERE substr(key,1,length(?1))=?1")
            .map_err(error)?;
        let rows = query
            .query_map([prefix], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(error)?;
        for row in rows {
            let (key, value) = row.map_err(error)?;
            target
                .execute(
                    "INSERT INTO settings(key,value) VALUES(?1,?2)",
                    [&key, &value],
                )
                .map_err(error)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_notifications_are_thread_scoped_and_do_not_hide_other_tables() {
        assert!(should_notify("settings"));
        write(|| {
            assert!(!should_notify("settings"));
            assert!(should_notify("providers"));
            assert!(std::thread::spawn(|| should_notify("settings"))
                .join()
                .unwrap());
            write(|| assert!(!should_notify("settings")));
        });
        assert!(should_notify("settings"));
        assert!(!is_private_key("assistant_theme"));
    }
}
