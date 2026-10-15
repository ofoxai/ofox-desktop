//! Real SQLite failures at the public source-sync boundary. Every source lives
//! under test_sandbox; neither native clients nor user databases are opened.
use super::{session_usage, session_usage_codex, session_usage_gemini, session_usage_opencode};
use crate::{database::Database, error::AppError, test_sandbox::sandbox};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use serial_test::serial;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

#[derive(Clone, Copy, Debug)]
enum Source {
    Claude,
    Codex,
    Gemini,
    OpenCodeDb,
    OpenCodeJson,
}
impl Source {
    fn sync(self, db: &Database) -> Result<session_usage::SessionSyncResult, AppError> {
        match self {
            Self::Claude => session_usage::sync_claude_session_logs(db),
            Self::Codex => session_usage_codex::sync_codex_usage(db),
            Self::Gemini => session_usage_gemini::sync_gemini_usage(db),
            Self::OpenCodeDb | Self::OpenCodeJson => {
                session_usage_opencode::sync_opencode_usage(db)
            }
        }
    }
    fn expected(self) -> u32 {
        if matches!(self, Self::OpenCodeDb | Self::OpenCodeJson) {
            3
        } else {
            2
        }
    }
    fn write(self, home: &Path, days: i64, revised: bool) {
        let at = chrono::Utc::now() - chrono::Duration::days(days);
        let stamp = at.to_rfc3339();
        let mut calls = vec![("a", if revised { 44 } else { 11 }), ("b", 22)];
        if revised {
            calls.push(("poison", 55));
        }
        match self {
            Self::Claude => {
                let line = |id: &str, output| {
                    json!({"type":"assistant","sessionId":"atomic",
                    "timestamp":stamp,"message":{"id":id,"model":"claude-opus-4-6",
                        "stop_reason":"end_turn","usage":{"input_tokens":100,"output_tokens":output}}})
                };
                let mut lines = vec![line("a", 11), line("b", 22)];
                if revised {
                    lines.extend([line("a", 44), line("poison", 55)]);
                }
                write(
                    &home.join(".claude/projects/p/atomic.jsonl"),
                    &jsonl(&lines),
                    revised,
                );
            }
            Self::Codex => {
                let mut lines = vec![
                    json!({"type":"session_meta","payload":{"id":"atomic"}}),
                    json!({"type":"turn_context","payload":{"model":"gpt-5.4"}}),
                    json!({"timestamp":stamp,"type":"event_msg","payload":{"type":"token_count",
                        "info":{"total_token_usage":{"input_tokens":100,"output_tokens":11}}}}),
                    json!({"timestamp":stamp,"type":"event_msg","payload":{"type":"token_count",
                        "info":{"total_token_usage":{"input_tokens":300,"output_tokens":33}}}}),
                ];
                if revised {
                    lines.push(
                        json!({"timestamp":stamp,"type":"event_msg","payload":{"type":"token_count",
                        "info":{"total_token_usage":{"input_tokens":600,"output_tokens":88}}}}),
                    );
                }
                write(
                    &home.join(".codex/sessions/2026/10/01/rollout-atomic.jsonl"),
                    &jsonl(&lines),
                    revised,
                );
            }
            Self::Gemini => {
                let messages: Vec<_> = calls.iter().map(|(id,output)| json!({"id":id,"type":"gemini",
                    "model":"gemini-2.5-pro","timestamp":stamp,"tokens":{"input":100,"output":output}})).collect();
                write(
                    &home.join(".gemini/tmp/p/chats/session-atomic.json"),
                    &json!({"sessionId":"atomic","messages":messages}).to_string(),
                    revised,
                );
            }
            Self::OpenCodeDb => {
                // The failed b is strictly below the next watermark: an old
                // implementation cannot recover it merely by re-reading >= max.
                calls.push(("c", 33));
                let path = home.join(".local/share/opencode/opencode.db");
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                let conn = Connection::open(path).unwrap();
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS session (id TEXT PRIMARY KEY, parent_id TEXT);
                    CREATE TABLE IF NOT EXISTS message (id TEXT PRIMARY KEY, session_id TEXT,
                        time_created INTEGER, time_updated INTEGER, data TEXT);
                    DELETE FROM message; DELETE FROM session;
                    INSERT INTO session VALUES ('atomic',NULL);",
                )
                .unwrap();
                for (index, (id, output)) in calls.iter().enumerate() {
                    let created = at.timestamp_millis() + index as i64;
                    let updated = if revised {
                        chrono::Utc::now().timestamp_millis() + index as i64
                    } else {
                        created
                    };
                    conn.execute(
                        "INSERT INTO message VALUES (?1,'atomic',?2,?3,?4)",
                        params![
                            id,
                            created,
                            updated,
                            oc_message(id, "atomic", *output, created).to_string()
                        ],
                    )
                    .unwrap();
                }
            }
            Self::OpenCodeJson => {
                for (id, output) in calls {
                    write(
                        &home.join(format!(
                            ".local/share/opencode/storage/message/atomic/{id}.json"
                        )),
                        &oc_message(id, "atomic", output, at.timestamp_millis()).to_string(),
                        revised,
                    );
                }
                // A second checkpoint must roll back with the first directory.
                write(
                    &home.join(".local/share/opencode/storage/message/other/c.json"),
                    &oc_message("c", "other", 33, at.timestamp_millis()).to_string(),
                    revised,
                );
            }
        }
    }
}
fn oc_message(id: &str, sid: &str, output: u32, at: i64) -> Value {
    json!({"id":id,"sessionID":sid,"role":"assistant","modelID":"gpt-5.4",
        "tokens":{"input":100,"output":output,"reasoning":0,"cache":{"read":0,"write":0}},
        "time":{"created":at,"completed":at+100}})
}
fn jsonl(lines: &[Value]) -> String {
    lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
fn write(path: &Path, body: &str, revised: bool) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(if revised { 10 } else { 1 }))
        .unwrap();
}
fn sql(db: &Database, sql: &str) {
    db.conn.lock().unwrap().execute_batch(sql).unwrap();
}
fn facts(db: &Database) -> Vec<(String, i64, String)> {
    let conn = db.conn.lock().unwrap();
    let mut stmt=conn.prepare("SELECT request_id,output_tokens,total_cost_usd FROM proxy_request_logs ORDER BY request_id").unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
fn checkpoints(db: &Database) -> Vec<(String, i64, i64, i64)> {
    let conn = db.conn.lock().unwrap();
    let mut stmt=conn.prepare("SELECT file_path,last_modified,last_line_offset,last_synced_at FROM session_log_sync ORDER BY file_path").unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
fn failure(result: Result<session_usage::SessionSyncResult, AppError>) {
    match result {
        Err(_) => {}
        Ok(result) => {
            assert!(
                !result.errors.is_empty(),
                "database failure was swallowed: {result:?}"
            );
            assert_eq!(
                (result.imported, result.skipped),
                (0, 0),
                "rolled-back work must not count"
            );
        }
    }
}
fn fact_failure(source: Source) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    source.write(home.home(), 1, false);
    sql(&db,"CREATE TEMP TRIGGER fail_fact BEFORE INSERT ON proxy_request_logs WHEN NEW.output_tokens=22
        BEGIN SELECT RAISE(ABORT,'fixture fact failure'); END;");
    failure(source.sync(&db));
    assert!(facts(&db).is_empty());
    assert!(checkpoints(&db).is_empty());
    sql(&db, "DROP TRIGGER fail_fact;");
    let retried = source.sync(&db).unwrap();
    assert!(retried.errors.is_empty(), "{retried:?}");
    assert_eq!(retried.imported, source.expected());
    let before = facts(&db);
    source.sync(&db).unwrap();
    assert_eq!(facts(&db), before);
}
fn checkpoint_failure(source: Source) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    source.write(home.home(), 1, false);
    sql(
        &db,
        "CREATE TEMP TRIGGER fail_checkpoint BEFORE INSERT ON session_log_sync
        BEGIN SELECT RAISE(ABORT,'fixture checkpoint failure'); END;",
    );
    failure(source.sync(&db));
    assert!(
        facts(&db).is_empty(),
        "facts survived the checkpoint failure"
    );
    assert!(checkpoints(&db).is_empty());
    sql(&db, "DROP TRIGGER fail_checkpoint;");
    assert_eq!(source.sync(&db).unwrap().imported, source.expected());
}
fn read_failure(source: Source, table: &str) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    source.write(home.home(), 1, false);
    sql(
        &db,
        &format!("ALTER TABLE {table} RENAME TO hidden_{table}"),
    );
    failure(source.sync(&db));
    assert!(facts(&db).is_empty(), "read error allowed facts to commit");
    sql(
        &db,
        &format!("ALTER TABLE hidden_{table} RENAME TO {table}"),
    );
    assert!(checkpoints(&db).is_empty());
    assert_eq!(source.sync(&db).unwrap().imported, source.expected());
}
fn updated_fact_failure(source: Source) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    source.write(home.home(), 1, false);
    assert_eq!(source.sync(&db).unwrap().imported, source.expected());
    let old_facts = facts(&db);
    let old_checkpoints = checkpoints(&db);
    source.write(home.home(), 1, true);
    sql(&db,"CREATE TEMP TRIGGER fail_update_batch BEFORE INSERT ON proxy_request_logs WHEN NEW.output_tokens=55
        BEGIN SELECT RAISE(ABORT,'fixture later failure'); END;");
    failure(source.sync(&db));
    assert_eq!(facts(&db), old_facts);
    assert_eq!(checkpoints(&db), old_checkpoints);
    sql(&db, "DROP TRIGGER fail_update_batch;");
    let result = source.sync(&db).unwrap();
    assert!(result.errors.is_empty());
    assert!(facts(&db).iter().any(|(_, output, _)| *output == 44));
}
macro_rules! source_tests {
    ($source:ident, $fact:ident, $checkpoint:ident, $read_checkpoint:ident, $pricing:ident) => {
        #[test]
        #[serial]
        fn $fact() {
            fact_failure(Source::$source);
        }
        #[test]
        #[serial]
        fn $checkpoint() {
            checkpoint_failure(Source::$source);
        }
        #[test]
        #[serial]
        fn $read_checkpoint() {
            read_failure(Source::$source, "session_log_sync");
        }
        #[test]
        #[serial]
        fn $pricing() {
            read_failure(Source::$source, "model_pricing");
        }
    };
}
source_tests!(
    Claude,
    atomic_sync_claude_fact,
    atomic_sync_claude_checkpoint,
    atomic_sync_claude_checkpoint_read,
    atomic_sync_claude_pricing_read
);
source_tests!(
    Codex,
    atomic_sync_codex_fact,
    atomic_sync_codex_checkpoint,
    atomic_sync_codex_checkpoint_read,
    atomic_sync_codex_pricing_read
);
source_tests!(
    Gemini,
    atomic_sync_gemini_fact,
    atomic_sync_gemini_checkpoint,
    atomic_sync_gemini_checkpoint_read,
    atomic_sync_gemini_pricing_read
);
source_tests!(
    OpenCodeDb,
    atomic_sync_opencode_db_fact,
    atomic_sync_opencode_db_checkpoint,
    atomic_sync_opencode_db_checkpoint_read,
    atomic_sync_opencode_db_pricing_read
);
source_tests!(
    OpenCodeJson,
    atomic_sync_opencode_json_fact,
    atomic_sync_opencode_json_checkpoint,
    atomic_sync_opencode_json_checkpoint_read,
    atomic_sync_opencode_json_pricing_read
);
#[test]
#[serial]
fn atomic_sync_claude_update_rollback() {
    updated_fact_failure(Source::Claude);
}
#[test]
#[serial]
fn atomic_sync_gemini_update_rollback() {
    updated_fact_failure(Source::Gemini);
}
#[test]
#[serial]
fn atomic_sync_opencode_db_update_rollback() {
    updated_fact_failure(Source::OpenCodeDb);
}
#[test]
#[serial]
fn atomic_sync_opencode_json_update_rollback() {
    updated_fact_failure(Source::OpenCodeJson);
}

#[test]
#[serial]
fn atomic_sync_second_json_checkpoint_failure_rolls_back_both_directories() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    Source::OpenCodeJson.write(home.home(), 1, false);
    sql(
        &db,
        "CREATE TEMP TABLE checkpoint_attempts (n INTEGER);
        INSERT INTO checkpoint_attempts VALUES (0);
        CREATE TEMP TRIGGER fail_late_checkpoint BEFORE INSERT ON session_log_sync BEGIN
            UPDATE checkpoint_attempts SET n=n+1;
            SELECT CASE WHEN (SELECT n FROM checkpoint_attempts)=2
                THEN RAISE(ABORT,'fixture second checkpoint failure') END;
        END;",
    );
    failure(Source::OpenCodeJson.sync(&db));
    assert!(
        checkpoints(&db).is_empty(),
        "first directory watermark survived second failure"
    );
    assert!(facts(&db).is_empty());
    sql(&db, "DROP TRIGGER fail_late_checkpoint;");
    assert_eq!(Source::OpenCodeJson.sync(&db).unwrap().imported, 3);
    assert_eq!(checkpoints(&db).len(), 2);
}

#[test]
#[serial]
fn atomic_sync_missing_prices_are_not_database_failures() {
    for source in [
        Source::Claude,
        Source::Codex,
        Source::Gemini,
        Source::OpenCodeDb,
        Source::OpenCodeJson,
    ] {
        let home = sandbox();
        let db = Database::memory().unwrap();
        source.write(home.home(), 1, false);
        sql(&db, "DELETE FROM model_pricing;");
        let result = source.sync(&db).unwrap();
        assert!(result.errors.is_empty(), "{source:?}: {result:?}");
        assert_eq!(result.imported, source.expected());
        assert!(facts(&db).iter().all(|(_, _, cost)| cost == "0"));
        assert!(!checkpoints(&db).is_empty());
    }
}

#[test]
#[serial]
fn atomic_sync_checkpoint_failure_restores_existing_facts_and_watermarks() {
    for source in [
        Source::Claude,
        Source::Codex,
        Source::Gemini,
        Source::OpenCodeDb,
        Source::OpenCodeJson,
    ] {
        let home = sandbox();
        let db = Database::memory().unwrap();
        source.write(home.home(), 1, false);
        assert!(source.sync(&db).unwrap().errors.is_empty());
        let before = facts(&db);
        let state = checkpoints(&db);
        source.write(home.home(), 1, true);
        sql(
            &db,
            "CREATE TEMP TRIGGER fail_existing_checkpoint BEFORE INSERT ON session_log_sync
            BEGIN SELECT RAISE(ABORT,'fixture existing checkpoint failure'); END;",
        );
        failure(source.sync(&db));
        assert_eq!(facts(&db), before, "{source:?}");
        assert_eq!(checkpoints(&db), state, "{source:?}");
        sql(&db, "DROP TRIGGER fail_existing_checkpoint;");
        let retried = source.sync(&db).unwrap();
        assert!(retried.errors.is_empty());
        assert!(
            facts(&db).iter().any(|(_, output, _)| *output == 55),
            "{source:?}: retry lost the new call"
        );
        if matches!(source, Source::Codex) {
            assert_eq!(retried.imported, 1);
        }
    }
}
