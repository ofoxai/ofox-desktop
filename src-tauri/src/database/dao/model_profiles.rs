//! Portable model recipes. Credentials, URLs and machine-local paths are excluded.

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::model_selection::{validate_targets, ProfileTarget};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfileRow {
    pub id: String,
    pub name: String,
    pub revision: String,
    pub targets: Vec<ProfileTarget>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn db_error(error: rusqlite::Error) -> AppError {
    AppError::Database(error.to_string())
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelProfileRow> {
    let text: String = row.get(3)?;
    let targets = serde_json::from_str(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(ModelProfileRow {
        id: row.get(0)?,
        name: row.get(1)?,
        revision: row.get(2)?,
        targets,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

impl Database {
    pub(crate) fn list_model_profiles(&self) -> Result<Vec<ModelProfileRow>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, name, revision, targets, created_at, updated_at
             FROM model_profiles ORDER BY updated_at DESC, name_key ASC, id ASC",
            )
            .map_err(db_error)?;
        let rows = stmt.query_map([], read_row).map_err(db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
    }

    pub(crate) fn get_model_profile(&self, id: &str) -> Result<Option<ModelProfileRow>, AppError> {
        use rusqlite::OptionalExtension;
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT id, name, revision, targets, created_at, updated_at FROM model_profiles WHERE id = ?1",
            [id], read_row,
        ).optional().map_err(db_error)
    }

    pub(crate) fn save_model_profile(
        &self,
        id: Option<&str>,
        name: &str,
        targets: Vec<ProfileTarget>,
        expected_revision: Option<&str>,
    ) -> Result<ModelProfileRow, AppError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err(AppError::Config("Profile 名称须为 1 到 80 个字符。".into()));
        }
        let targets = validate_targets(targets).map_err(AppError::Config)?;
        let text = serde_json::to_string(&targets)
            .map_err(|_| AppError::Config("无法保存模型组合。".into()))?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn.transaction().map_err(db_error)?;
        let name_key = name.to_lowercase();
        let id = match id {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => uuid::Uuid::new_v4().to_string(),
        };
        let existing: Option<(String, i64)> = {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "SELECT revision, created_at FROM model_profiles WHERE id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db_error)?
        };
        if let Some((previous, _)) = &existing {
            if expected_revision != Some(previous.as_str()) {
                return Err(AppError::Config(
                    "Profile 已在其他窗口更改，请刷新后重试。".into(),
                ));
            }
        } else if expected_revision.is_some() {
            return Err(AppError::Config("Profile 已删除，请刷新后重试。".into()));
        }
        let occupied: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM model_profiles WHERE name_key = ?1 AND id != ?2)",
                params![name_key, id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if occupied {
            return Err(AppError::Config(
                "已有同名 Profile，请使用其他名称。".into(),
            ));
        }
        let created_at = existing.map(|(_, at)| at).unwrap_or(now);
        let revision = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO model_profiles (id, name, name_key, revision, targets, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, name_key=excluded.name_key,
               revision=excluded.revision, targets=excluded.targets, updated_at=excluded.updated_at",
            params![id, name, name_key, revision, text, created_at, now],
        ).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(ModelProfileRow {
            id,
            name: name.into(),
            revision,
            targets,
            created_at,
            updated_at: now,
        })
    }

    pub(crate) fn delete_model_profile(
        &self,
        id: &str,
        expected_revision: &str,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        let affected = conn
            .execute(
                "DELETE FROM model_profiles WHERE id = ?1 AND revision = ?2",
                params![id, expected_revision],
            )
            .map_err(db_error)?;
        if affected == 0 {
            return Err(AppError::Config(
                "Profile 已更改或删除，请刷新后重试。".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_selection::ModelSelection;

    fn targets() -> Vec<ProfileTarget> {
        vec![ProfileTarget {
            target: "workbuddy".into(),
            selection: ModelSelection::Models {
                models: vec!["second".into(), "first".into()],
            },
        }]
    }

    #[test]
    fn storage_roundtrip_preserves_collection_order_and_shared_target() {
        let db = Database::memory().unwrap();
        let saved = db
            .save_model_profile(None, "工作", targets(), None)
            .unwrap();
        assert_eq!(
            db.get_model_profile(&saved.id).unwrap().unwrap().targets,
            targets()
        );
        assert_eq!(db.list_model_profiles().unwrap().len(), 1);
    }

    #[test]
    fn stale_edit_delete_and_duplicate_name_do_not_change_the_profile() {
        let db = Database::memory().unwrap();
        let saved = db
            .save_model_profile(None, "Work", targets(), None)
            .unwrap();
        assert!(db
            .save_model_profile(None, " work ", targets(), None)
            .is_err());
        assert!(db
            .save_model_profile(Some(&saved.id), "New", targets(), Some("stale"))
            .is_err());
        assert!(db.delete_model_profile(&saved.id, "stale").is_err());
        assert_eq!(
            db.get_model_profile(&saved.id).unwrap().unwrap().name,
            "Work"
        );
        let next = db
            .save_model_profile(Some(&saved.id), "New", targets(), Some(&saved.revision))
            .unwrap();
        assert_ne!(next.revision, saved.revision);
        assert_eq!(next.created_at, saved.created_at);
        assert!(db.delete_model_profile(&saved.id, &saved.revision).is_err());
        db.delete_model_profile(&saved.id, &next.revision).unwrap();
        assert!(db.get_model_profile(&saved.id).unwrap().is_none());
    }

    #[test]
    fn invalid_recipe_is_rejected_before_any_database_change() {
        let db = Database::memory().unwrap();
        assert!(db
            .save_model_profile(
                None,
                "Bad",
                vec![ProfileTarget {
                    target: "pi".into(),
                    selection: ModelSelection::Single {
                        model: "sk-of-secret".into()
                    },
                }],
                None
            )
            .is_err());
        assert!(db.list_model_profiles().unwrap().is_empty());
    }
}
