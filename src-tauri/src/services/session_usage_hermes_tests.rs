use super::*;
use crate::{database::Database, test_sandbox::sandbox};
use rusqlite::Connection;
use serial_test::serial;
use std::{fs, path::PathBuf};

const NATIVE_SQL: &str = include_str!("fixtures/hermes-native-0.21.7-sessions.sql");
const T0: i64 = 1_790_820_000;

/// A Hermes home whose state.db holds only the given fixture sessions.
fn fixture(home: &Path, keep: &[&str]) -> PathBuf {
    let root = home.join("hermes");
    fs::create_dir_all(&root).unwrap();
    let conn = Connection::open(root.join("state.db")).unwrap();
    conn.execute_batch(NATIVE_SQL).unwrap();
    let keep = serde_json::to_string(keep).unwrap();
    for table in ["sessions", "session_model_usage", "messages"] {
        let column = if table == "sessions" {
            "id"
        } else {
            "session_id"
        };
        conn.execute(
            &format!("DELETE FROM {table} WHERE {column} NOT IN (SELECT value FROM json_each(?1))"),
            [&keep],
        )
        .unwrap();
    }
    root
}

fn update(root: &Path, sql: &str) {
    Connection::open(root.join("state.db"))
        .unwrap()
        .execute_batch(sql)
        .unwrap();
}

/// (request_id, session_id, model, [input, output, cache read, cache write], created_at)
type Row = (String, String, String, [i64; 4], i64);
fn ledger(db: &Database) -> Vec<Row> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT request_id,session_id,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,created_at \
             FROM proxy_request_logs WHERE app_type='hermes' AND provider_id='_hermes_session' AND data_source='hermes_session' \
             ORDER BY session_id,model",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            [r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?],
            r.get(7)?,
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn models(rows: &[Row]) -> Vec<(&str, &str, [i64; 4])> {
    rows.iter()
        .map(|(_, sid, model, tokens, _)| (sid.as_str(), model.as_str(), *tokens))
        .collect()
}

fn complete(db: &Database) -> bool {
    super::super::usage_source::has_complete_coverage_on(&db.conn.lock().unwrap(), "hermes")
        .unwrap()
}

#[test]
#[serial]
fn hermes_ledger_voice_is_in_pool_and_aux_is_added_once() {
    let sandbox = sandbox();
    let root = fixture(sandbox.home(), &["main-voice-aux"]);
    let db = Database::memory().unwrap();
    assert_eq!(sync_hermes_usage_at(&db, &root).unwrap().imported, 3);
    assert_eq!(
        models(&ledger(&db)),
        [
            ("main-voice-aux", "aux-model", [30, 3, 0, 0]),
            ("main-voice-aux", "main-model", [100, 10, 0, 0]),
            ("main-voice-aux", "voice-model", [20, 2, 0, 0]),
        ]
    );
    assert!(complete(&db));
}

#[test]
#[serial]
fn hermes_ledger_legacy_pool_gets_one_unknown_residual() {
    let sandbox = sandbox();
    let root = fixture(sandbox.home(), &["legacy", "partial"]);
    let db = Database::memory().unwrap();
    sync_hermes_usage_at(&db, &root).unwrap();
    let rows = ledger(&db);
    assert_eq!(
        models(&rows),
        [
            ("legacy", "unknown", [50, 5, 0, 0]),
            ("partial", "main-model", [60, 6, 0, 0]),
            ("partial", "unknown", [20, 2, 0, 0]),
        ]
    );
    assert_eq!(rows[0].4, T0 + 3700, "legacy residual uses ended_at");
    assert_eq!(rows[2].4, T0 + 300, "residual uses last_activity_at");
    let read = read_hermes(&root.join("state.db")).unwrap();
    assert!(read.reasons.contains("modelAttributionUnavailable"));
    assert_eq!(read.incomplete.len(), 2);
    assert!(!complete(&db));
}

#[test]
#[serial]
fn hermes_ledger_growth_upserts_in_place_and_moves_created_at() {
    let sandbox = sandbox();
    let root = fixture(sandbox.home(), &["main-aux"]);
    let db = Database::memory().unwrap();
    assert_eq!(sync_hermes_usage_at(&db, &root).unwrap().imported, 2);
    let before = ledger(&db);
    update(
        &root,
        &format!(
            "UPDATE sessions SET input_tokens=150; \
             UPDATE session_model_usage SET input_tokens=150,last_seen={} WHERE model='main-model';",
            T0 + 86_400
        ),
    );
    assert_eq!(sync_hermes_usage_at(&db, &root).unwrap().imported, 1);
    let after = ledger(&db);
    assert_eq!(after.len(), 2);
    assert_eq!(after[1].0, before[1].0, "same request id");
    assert_eq!(after[1].3, [150, 10, 5, 2]);
    assert_eq!(after[1].4, T0 + 86_400);
    assert_eq!(after[0], before[0], "the vision row is untouched");
    assert!(complete(&db));
}

#[test]
#[serial]
fn hermes_ledger_ids_never_contain_billing_urls() {
    let sandbox = sandbox();
    let root = fixture(sandbox.home(), &["main-aux"]);
    let db = Database::memory().unwrap();
    sync_hermes_usage_at(&db, &root).unwrap();
    let rows = ledger(&db);
    assert_eq!(rows.len(), 2);
    for (id, sid, model, _, _) in rows {
        assert!(id.starts_with("hermes_session:[\"main-aux\","), "{id}");
        assert_eq!(sid, "main-aux");
        for text in [id, model] {
            assert!(
                !text.contains("SECRET") && !text.contains("ofox.ai"),
                "{text}"
            );
        }
    }
}

#[test]
#[serial]
fn hermes_ledger_unknown_task_counted_but_partial() {
    let sandbox = sandbox();
    let root = fixture(sandbox.home(), &["unknown-task"]);
    let db = Database::memory().unwrap();
    sync_hermes_usage_at(&db, &root).unwrap();
    assert_eq!(
        models(&ledger(&db)),
        [
            ("unknown-task", "future-model", [7, 1, 0, 0]),
            ("unknown-task", "main-model", [10, 1, 0, 0]),
        ]
    );
    let read = read_hermes(&root.join("state.db")).unwrap();
    assert!(read.reasons.contains("unknownTask"));
    assert_eq!(read.incomplete.len(), 1);
    assert!(!complete(&db));
}

#[test]
#[serial]
fn hermes_ledger_orphan_rows_skipped() {
    let sandbox = sandbox();
    let root = fixture(sandbox.home(), &["main-aux", "ghost"]);
    let db = Database::memory().unwrap();
    sync_hermes_usage_at(&db, &root).unwrap();
    assert!(ledger(&db).iter().all(|row| row.1 == "main-aux"));
    let read = read_hermes(&root.join("state.db")).unwrap();
    assert!(read.reasons.contains("orphanModelRows"));
}

#[test]
#[serial]
fn hermes_ledger_counter_overflow_skips_fact_only() {
    let sandbox = sandbox();
    let root = fixture(sandbox.home(), &["main-aux"]);
    update(
        &root,
        "INSERT INTO session_model_usage(session_id,model,task,input_tokens,last_seen) \
         VALUES('main-aux','huge-model','vision',5000000000,1790820050.0);",
    );
    let db = Database::memory().unwrap();
    assert_eq!(sync_hermes_usage_at(&db, &root).unwrap().imported, 2);
    assert!(ledger(&db).iter().all(|row| row.2 != "huge-model"));
    let read = read_hermes(&root.join("state.db")).unwrap();
    assert!(read.reasons.contains("counterOverflow"));
}

#[test]
#[serial]
fn hermes_ledger_dates_rows_by_last_seen_else_session_end() {
    let sandbox = sandbox();
    let root = fixture(sandbox.home(), &["cross-day", "null-seen"]);
    let db = Database::memory().unwrap();
    sync_hermes_usage_at(&db, &root).unwrap();
    let rows = ledger(&db);
    assert_eq!(rows[0].1, "cross-day");
    assert_eq!(rows[0].4, 1_790_871_000);
    assert_eq!(rows[1].1, "null-seen");
    assert_eq!(rows[1].4, T0 + 530, "latest message beats ended_at");
}

#[test]
#[serial]
fn hermes_ledger_unchanged_stamp_returns_without_import() {
    let sandbox = sandbox();
    let root = fixture(sandbox.home(), &["main-aux"]);
    let db = Database::memory().unwrap();
    assert_eq!(sync_hermes_usage_at(&db, &root).unwrap().imported, 2);
    let again = sync_hermes_usage_at(&db, &root).unwrap();
    assert_eq!((again.imported, again.skipped), (0, 0));
}

#[test]
#[serial]
fn hermes_ledger_missing_database_scans_nothing() {
    let sandbox = sandbox();
    let db = Database::memory().unwrap();
    let result = sync_hermes_usage_at(&db, &sandbox.home().join("no-hermes")).unwrap();
    assert_eq!(result.files_scanned, 0);
    assert!(ledger(&db).is_empty());
}

#[test]
#[serial]
fn hermes_reader_keeps_message_bodies_in_sqlite() {
    let sandbox = sandbox();
    let root = fixture(sandbox.home(), &["main-aux"]);
    let read = read_hermes(&root.join("state.db")).unwrap();
    let messages = read.messages.unwrap();
    assert_eq!(
        messages
            .iter()
            .map(|m| (m.id, m.role.as_str(), m.at_ms))
            .collect::<Vec<_>>(),
        [
            (1, "user", (T0 + 1) * 1000),
            (2, "assistant", (T0 + 4) * 1000),
            (3, "tool", (T0 + 6) * 1000),
            (4, "assistant", (T0 + 9) * 1000),
        ]
    );
    let session = &read.sessions[0];
    assert_eq!(session.cwd.as_deref(), Some("/work/alpha"));
    assert_eq!(session.title.as_deref(), Some("Alpha"));
    assert_eq!(session.started_ms, T0 * 1000);
}

fn wal_only_fixture(home: &Path) -> PathBuf {
    let root = fixture(home, &["main-aux"]);
    let conn = Connection::open(root.join("state.db")).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    let main = fs::read(root.join("state.db")).unwrap();
    conn.execute_batch(
        "UPDATE sessions SET input_tokens=200; UPDATE session_model_usage SET input_tokens=200 WHERE model='main-model';",
    )
    .unwrap();
    let wal = fs::read(root.join("state.db-wal")).unwrap();
    drop(conn);
    // An offline native snapshot with a committed WAL and no SHM. Closing the
    // fixture writer normally checkpoints and deletes the WAL.
    fs::write(root.join("state.db"), main).unwrap();
    fs::write(root.join("state.db-wal"), wal).unwrap();
    root
}

fn main_input(read: &HermesRead) -> u32 {
    read.facts
        .iter()
        .find(|f| f.model == "main-model")
        .unwrap()
        .usage
        .input_tokens
}

#[test]
#[serial]
fn hermes_offline_wal_is_read_without_creating_native_shm_or_changing_bytes() {
    let sandbox = sandbox();
    let root = wal_only_fixture(sandbox.home());
    let main = fs::read(root.join("state.db")).unwrap();
    let wal = fs::read(root.join("state.db-wal")).unwrap();
    assert_eq!(
        main_input(&read_hermes(&root.join("state.db")).unwrap()),
        200
    );
    assert_eq!(fs::read(root.join("state.db")).unwrap(), main);
    assert_eq!(fs::read(root.join("state.db-wal")).unwrap(), wal);
    assert!(!root.join("state.db-shm").exists());
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
#[serial]
fn hermes_sqlite_leaf_alias_fails_closed() {
    let sandbox = sandbox();
    let target = wal_only_fixture(sandbox.home());
    let alias = sandbox.home().join("alias");
    fs::create_dir_all(&alias).unwrap();
    std::os::unix::fs::symlink(target.join("state.db"), alias.join("state.db")).unwrap();
    assert!(read_hermes(&alias.join("state.db")).is_err());
    assert!(sync_hermes_usage_at(&Database::memory().unwrap(), &alias).is_err());
    assert_eq!(
        main_input(&read_hermes(&target.join("state.db")).unwrap()),
        200
    );
}

#[test]
#[serial]
fn hermes_snapshot_rejects_changes_during_the_copy() {
    let sandbox = sandbox();
    let root = wal_only_fixture(sandbox.home());
    let db_path = root.join("state.db");
    let wal_change = read_after_copy(&db_path, || {
        let mut wal = fs::read(root.join("state.db-wal")).unwrap();
        wal.push(0);
        fs::write(root.join("state.db-wal"), wal).unwrap();
    });
    assert!(wal_change.is_err());
    let removed_wal = read_after_copy(&db_path, || {
        fs::remove_file(root.join("state.db-wal")).unwrap();
    });
    assert!(removed_wal.is_err());
    let replaced = read_after_copy(&db_path, || {
        let replacement = root.join("replacement.db");
        fs::write(&replacement, fs::read(&db_path).unwrap()).unwrap();
        fs::rename(replacement, &db_path).unwrap();
    });
    assert!(replaced.is_err());
}
