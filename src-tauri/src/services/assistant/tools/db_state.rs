//! Tool-scoped DB and provider-selection state for encrypted journals.
use crate::{app_config::AppType, database::Database};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::str::FromStr;
const FAILED: &str = "assistant_journal_unavailable";
const TABLES: [(&str, &str); 3] = [
    ("providers", "app_type"),
    ("provider_endpoints", "app_type"),
    ("ofox_bind_snapshot", "tool"),
];
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
enum Cell {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}
impl Cell {
    fn sql(&self) -> SqlValue {
        match self {
            Self::Null => SqlValue::Null,
            Self::Integer(v) => SqlValue::Integer(*v),
            Self::Real(v) => SqlValue::Real(*v),
            Self::Text(v) => SqlValue::Text(v.clone()),
            Self::Blob(v) => SqlValue::Blob(v.clone()),
        }
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
struct Table {
    columns: Vec<String>,
    keys: Vec<usize>,
    rows: Vec<Vec<Cell>>,
}
fn table(conn: &Connection, name: &str, column: &str, tool: &str) -> Result<Table, String> {
    let mut schema = conn
        .prepare(&format!("PRAGMA table_info({name})"))
        .map_err(|_| FAILED)?;
    let fields = schema
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, u32>(5)?))
        })
        .map_err(|_| FAILED)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| FAILED)?;
    let columns: Vec<_> = fields.iter().map(|(n, _)| n.clone()).collect();
    let keys: Vec<_> = fields
        .iter()
        .enumerate()
        .filter_map(|(i, (_, pk))| (*pk > 0).then_some(i))
        .collect();
    if keys.is_empty() {
        return Err(FAILED.into());
    }
    let mut stmt = conn
        .prepare(&format!(
            "SELECT * FROM {name} WHERE {column}=?1 ORDER BY {}",
            keys.iter()
                .map(|i| format!("\"{}\"", columns[*i]))
                .collect::<Vec<_>>()
                .join(",")
        ))
        .map_err(|_| FAILED)?;
    let rows = stmt
        .query_map([tool], |row| {
            (0..columns.len())
                .map(|i| {
                    row.get::<_, SqlValue>(i).map(|v| match v {
                        SqlValue::Null => Cell::Null,
                        SqlValue::Integer(v) => Cell::Integer(v),
                        SqlValue::Real(v) => Cell::Real(v),
                        SqlValue::Text(v) => Cell::Text(v),
                        SqlValue::Blob(v) => Cell::Blob(v),
                    })
                })
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| FAILED)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| FAILED)?;
    Ok(Table {
        columns,
        keys,
        rows,
    })
}
fn capture_locked(conn: &Connection, tool: &str) -> Result<Value, String> {
    super::registry::supported_tool(tool)?;
    let tables = TABLES
        .iter()
        .map(|(name, col)| table(conn, name, col, tool))
        .collect::<Result<Vec<_>, _>>()?;
    let selection = AppType::from_str(tool)
        .ok()
        .and_then(|app| crate::settings::get_current_provider(&app));
    Ok(json!({"scope":tool,"tables":tables,"selection":selection}))
}
pub fn capture(db: &Database, tool: &str) -> Result<Value, String> {
    capture_locked(&*db.conn.lock().map_err(|_| FAILED)?, tool)
}
pub fn restore(db: &Database, before: &Value, after: &Value) -> Result<(), String> {
    let tool = before["scope"].as_str().ok_or(FAILED)?;
    if after["scope"] != before["scope"] {
        return Err(FAILED.into());
    }
    let mut conn = db.conn.lock().map_err(|_| FAILED)?;
    let tx = conn.transaction().map_err(|_| FAILED)?;
    let current = capture_locked(&tx, tool)?;
    if current == *before {
        return Ok(());
    }
    if current != *after {
        return Err("assistant_restore_conflict".into());
    }
    let originals: Vec<Table> =
        serde_json::from_value(before["tables"].clone()).map_err(|_| FAILED)?;
    if originals.len() != TABLES.len() {
        return Err(FAILED.into());
    }
    for ((name, col), original) in TABLES.iter().zip(originals) {
        let live = table(&tx, name, col, tool)?;
        if live.columns != original.columns || live.keys != original.keys {
            return Err(FAILED.into());
        }
        for row in &live.rows {
            if !original
                .rows
                .iter()
                .any(|old| live.keys.iter().all(|i| row.get(*i) == old.get(*i)))
            {
                let where_keys = live
                    .keys
                    .iter()
                    .map(|i| format!("\"{}\"=?", live.columns[*i]))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                tx.execute(
                    &format!("DELETE FROM {name} WHERE {where_keys}"),
                    params_from_iter(live.keys.iter().map(|i| row[*i].sql())),
                )
                .map_err(|_| FAILED)?;
            }
        }
        let columns = original
            .columns
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(",");
        let keys = original
            .keys
            .iter()
            .map(|i| format!("\"{}\"", original.columns[*i]))
            .collect::<Vec<_>>()
            .join(",");
        let update = original
            .columns
            .iter()
            .enumerate()
            .filter(|(i, _)| !original.keys.contains(i))
            .map(|(_, c)| format!("\"{c}\"=excluded.\"{c}\""))
            .collect::<Vec<_>>()
            .join(",");
        let placeholders = vec!["?"; original.columns.len()].join(",");
        for row in original.rows {
            if row.len() != original.columns.len() {
                return Err(FAILED.into());
            }
            tx.execute(&format!("INSERT INTO {name}({columns}) VALUES ({placeholders}) ON CONFLICT({keys}) DO UPDATE SET {update}"),params_from_iter(row.iter().map(Cell::sql))).map_err(|_|FAILED)?;
        }
    }
    if let Ok(app) = AppType::from_str(tool) {
        crate::settings::set_current_provider(&app, before["selection"].as_str())
            .map_err(|_| FAILED)?;
    }
    tx.commit().map_err(|_| FAILED.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[serial_test::serial]
    fn state_restore_keeps_other_tools_and_detects_changed_binding() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = crate::database::Database::memory().unwrap();
        db.upsert_bind_record("pi", "before").unwrap();
        db.upsert_bind_record("dsh", "unrelated").unwrap();
        let before = capture(&db, "pi").unwrap();
        db.upsert_bind_record("pi", "after").unwrap();
        let after = capture(&db, "pi").unwrap();
        db.upsert_bind_record("pi", "user edit").unwrap();
        assert!(restore(&db, &before, &after).is_err());
        // Capture an explicit expected state to simulate an uninterrupted rollback.
        let current = capture(&db, "pi").unwrap();
        restore(&db, &before, &current).unwrap();
        assert_eq!(db.get_bind_record("pi").unwrap().unwrap().record, "before");
        assert_eq!(
            db.get_bind_record("dsh").unwrap().unwrap().record,
            "unrelated"
        );
    }
}
