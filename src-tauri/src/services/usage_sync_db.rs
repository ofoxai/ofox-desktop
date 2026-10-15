//! Commit one parsed source batch without exposing a partial watermark.
use crate::proxy::usage::calculator::ModelPricing;
use crate::{
    database::{lock_conn, Database},
    error::AppError,
};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncState {
    pub modified: i64,
    pub offset: i64,
}

pub(crate) struct CheckpointChange {
    pub key: String,
    pub expected: Option<SyncState>,
    /// A read dependency (e.g. a migrated JSON directory) has no next state.
    pub next: Option<SyncState>,
}

pub(crate) fn read_sync_state(db: &Database, key: &str) -> Result<Option<SyncState>, AppError> {
    let conn = lock_conn!(db.conn);
    read_sync_state_on(&conn, key)
}

fn read_sync_state_on(conn: &Connection, key: &str) -> Result<Option<SyncState>, AppError> {
    conn.query_row(
        "SELECT last_modified,last_line_offset FROM session_log_sync WHERE file_path=?1",
        [key],
        |r| {
            Ok(SyncState {
                modified: r.get(0)?,
                offset: r.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|e| AppError::Database(format!("read usage checkpoint: {e}")))
}

pub(crate) fn commit_sync_batch<T>(
    db: &Database,
    checkpoints: &[CheckpointChange],
    apply: impl FnOnce(&Connection) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut conn = lock_conn!(db.conn);
    let tx = conn
        .transaction()
        .map_err(|e| AppError::Database(format!("begin usage sync: {e}")))?;
    // Parsing happens outside the mutex. Refuse stale work before either its
    // facts or any of its watermarks are published, including read-only deps.
    for change in checkpoints {
        if read_sync_state_on(&tx, &change.key)? != change.expected {
            return Err(AppError::Database(
                "usage checkpoint changed; retry sync".into(),
            ));
        }
    }
    let result = apply(&tx)?;
    for change in checkpoints {
        if let Some(next) = change.next {
            tx.execute(
                "INSERT OR REPLACE INTO session_log_sync
                (file_path,last_modified,last_line_offset,last_synced_at) VALUES (?1,?2,?3,?4)",
                params![
                    change.key,
                    next.modified,
                    next.offset,
                    chrono::Utc::now().timestamp()
                ],
            )
            .map_err(|e| AppError::Database(format!("write usage checkpoint: {e}")))?;
        }
    }
    tx.commit()
        .map_err(|e| AppError::Database(format!("commit usage sync: {e}")))?;
    Ok(result)
}

/// Missing catalog rows are normal. Query/row/price corruption is not a
/// missing price and must abort the same batch as a failed facts write.
pub(crate) fn lookup_pricing(
    conn: &Connection,
    model: &str,
    like: bool,
) -> Result<Option<ModelPricing>, AppError> {
    let sql = if like {
        "SELECT input_cost_per_million,output_cost_per_million,
            cache_read_cost_per_million,cache_creation_cost_per_million
         FROM model_pricing WHERE model_id LIKE ?1 LIMIT 1"
    } else {
        "SELECT input_cost_per_million,output_cost_per_million,
            cache_read_cost_per_million,cache_creation_cost_per_million
         FROM model_pricing WHERE model_id=?1"
    };
    let values: Option<(String, String, String, String)> = conn
        .query_row(sql, [model], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()
        .map_err(|e| AppError::Database(format!("read usage pricing: {e}")))?;
    values
        .map(|(i, o, cr, cc)| {
            ModelPricing::from_strings(&i, &o, &cr, &cc)
                .map_err(|e| AppError::Database(format!("parse usage pricing: {e}")))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stale_batch(expected: Option<SyncState>, current: SyncState, next: Option<SyncState>) {
        let db = Database::memory().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO session_log_sync VALUES ('source',?1,?2,0)",
                params![current.modified, current.offset],
            )
            .unwrap();
        let applied = std::cell::Cell::new(false);
        let result = commit_sync_batch(
            &db,
            &[CheckpointChange {
                key: "source".into(),
                expected,
                next,
            }],
            |conn| {
                applied.set(true);
                conn.execute(
                    "INSERT INTO proxy_request_logs
                    (request_id,provider_id,app_type,model,latency_ms,status_code,created_at)
                    VALUES ('stale','fixture','claude','fixture',0,200,1)",
                    [],
                )
                .map_err(|e| AppError::Database(e.to_string()))?;
                Ok(())
            },
        );
        assert!(result.is_err(), "a stale prepared batch committed");
        assert!(!applied.get(), "stale work must be refused before apply");
        assert_eq!(read_sync_state(&db, "source").unwrap(), Some(current));
        let rows: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }
    #[test]
    fn atomic_sync_cas_rejects_a_newer_checkpoint_before_writing_facts() {
        stale_batch(
            Some(SyncState {
                modified: 1,
                offset: 2,
            }),
            SyncState {
                modified: 3,
                offset: 4,
            },
            Some(SyncState {
                modified: 5,
                offset: 6,
            }),
        );
    }
    #[test]
    fn atomic_sync_cas_distinguishes_a_zero_checkpoint_from_no_checkpoint() {
        stale_batch(
            None,
            SyncState::default(),
            Some(SyncState {
                modified: 5,
                offset: 6,
            }),
        );
    }
    #[test]
    fn atomic_sync_cas_checks_a_read_dependency_without_advancing_it() {
        stale_batch(
            Some(SyncState {
                modified: 1,
                offset: 2,
            }),
            SyncState {
                modified: 3,
                offset: 4,
            },
            None,
        );
    }

    #[test]
    fn atomic_sync_commit_failure_rolls_back_facts_and_checkpoint_and_can_retry() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute_batch("CREATE TABLE commit_parent (id INTEGER PRIMARY KEY);
            CREATE TABLE commit_child (owner INTEGER REFERENCES commit_parent(id) DEFERRABLE INITIALLY DEFERRED);").unwrap();
        let change = [CheckpointChange {
            key: "commit-source".into(),
            expected: None,
            next: Some(SyncState {
                modified: 1,
                offset: 2,
            }),
        }];
        let apply = |conn: &Connection| {
            conn.execute("INSERT INTO commit_child VALUES (99)", [])?;
            conn.execute(
                "INSERT INTO proxy_request_logs
                (request_id,provider_id,app_type,model,latency_ms,status_code,created_at)
                VALUES ('commit-fact','fixture','claude','fixture',0,200,1)",
                [],
            )?;
            Ok::<_, AppError>(1)
        };
        let error = commit_sync_batch(&db, &change, apply).unwrap_err();
        assert!(error.to_string().contains("commit usage sync"), "{error}");
        assert_eq!(read_sync_state(&db, "commit-source").unwrap(), None);
        {
            let conn = db.conn.lock().unwrap();
            assert!(conn.is_autocommit());
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
            conn.execute("INSERT INTO commit_parent VALUES (99)", [])
                .unwrap();
        }
        assert_eq!(commit_sync_batch(&db, &change, apply).unwrap(), 1);
        assert_eq!(
            read_sync_state(&db, "commit-source").unwrap(),
            change[0].next
        );
    }
}
