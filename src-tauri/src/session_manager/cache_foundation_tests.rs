//! End-to-end cache writers and restore boundaries. No real session/config reads.
use super::{meta_cache, SessionMeta};
use crate::{
    config::get_app_config_dir, database::Database, services::usage_maintenance,
    test_sandbox::sandbox,
};
use serde_json::{json, Value};
use serial_test::serial;
use std::{fs, path::Path};

fn seed(db: &Database, path: &Path, body: &Value, version: i64) {
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO session_meta_cache VALUES(?1,'claude','s',?2,?3,?4,1)",
            rusqlite::params![
                path.to_string_lossy(),
                meta_cache::stamp(path),
                version,
                body.to_string()
            ],
        )
        .unwrap();
}
fn cached(db: &Database, path: &Path) -> Value {
    let raw: String = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT meta_json FROM session_meta_cache WHERE source_path=?1",
            [path.to_string_lossy()],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}
fn usage_only() -> Value {
    json!({"formatVersion":1,"futureTop":{"keep":true},"parts":{
        "usageSync":{"revision":"usage-1","value":{"version":1,"trusted":true,
            "coverage":"completeFromStart","acknowledgedEventIds":["native-id-hash"],"futureUsage":17}},
        "futurePart":{"revision":"future-1","value":{"keep":42}}}})
}
fn transcript(home: &Path) -> std::path::PathBuf {
    let path = home.join(".claude/projects/p/s.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        json!({"type":"user","sessionId":"s","message":{"role":"user","content":"fixture prompt"}})
            .to_string()
            + "\n",
    )
    .unwrap();
    path
}
fn listing(path: &Path) -> Vec<SessionMeta> {
    vec![SessionMeta {
        provider_id: "claude".into(),
        session_id: "s".into(),
        source_path: Some(path.to_string_lossy().into()),
        ..Default::default()
    }]
}

#[test]
#[serial]
fn foundation_metadata_refresh_preserves_usage_and_unknown_parts() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = transcript(home.home());
    let original = usage_only();
    seed(&db, &path, &original, 0);
    let mut rows = listing(&path);
    meta_cache::enrich(&mut rows, Some(&db));
    assert_eq!(rows[0].message_count, Some(1));
    let after = cached(&db, &path);
    assert_eq!(after["parts"]["usageSync"], original["parts"]["usageSync"]);
    assert_eq!(
        after["parts"]["futurePart"],
        original["parts"]["futurePart"]
    );
    assert_eq!(after["futureTop"], original["futureTop"]);
    assert!(after["parts"]["metadata"]["revision"].is_string());
}
#[test]
#[serial]
fn foundation_usage_only_cache_is_not_a_zero_message_metadata_hit() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = transcript(home.home());
    seed(&db, &path, &usage_only(), meta_cache::PARSER_VERSION);
    let mut rows = listing(&path);
    meta_cache::enrich(&mut rows, Some(&db));
    assert_eq!(rows[0].message_count, Some(1));
}
#[test]
#[serial]
fn foundation_filtered_or_missing_listing_does_not_delete_usage_proof() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = transcript(home.home());
    let original = usage_only();
    seed(&db, &path, &original, meta_cache::PARSER_VERSION);
    meta_cache::enrich(&mut [], Some(&db));
    assert_eq!(cached(&db, &path), original);
}
#[test]
#[serial]
fn foundation_unknown_envelope_version_is_not_overwritten_by_metadata() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = transcript(home.home());
    let mut original = usage_only();
    original["formatVersion"] = json!(999);
    seed(&db, &path, &original, 0);
    let mut rows = listing(&path);
    meta_cache::enrich(&mut rows, Some(&db));
    assert_eq!(rows[0].message_count, Some(1));
    assert_eq!(cached(&db, &path), original);
}
#[test]
#[serial]
fn foundation_corrupt_cache_is_not_replaced_with_an_empty_envelope() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = transcript(home.home());
    seed(&db, &path, &usage_only(), 0);
    db.conn
        .lock()
        .unwrap()
        .execute("UPDATE session_meta_cache SET meta_json='{broken'", [])
        .unwrap();
    let mut rows = listing(&path);
    meta_cache::enrich(&mut rows, Some(&db));
    let after: String = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT meta_json FROM session_meta_cache", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, "{broken");
    assert_eq!(rows[0].message_count, Some(1));
}
fn metadata_and_usage() -> Value {
    let mut v = usage_only();
    v["parts"]["metadata"] = json!({"revision":"meta-1","value":{
        "parserVersion":2,"sourceStamp":{"legacy":"stamp"},"details":{"candidates":{"prompt":"local"},"messageCount":1}}});
    v
}
fn importable(db: &Database) {
    db.conn.lock().unwrap().execute("INSERT INTO providers(id,app_type,name,settings_config,meta) VALUES ('fixture','claude','Fixture','{}','{}')",[]).unwrap();
}
fn assert_revoked(db: &Database, path: &Path, original: &Value) {
    let after = cached(db, path);
    assert_eq!(after["parts"]["metadata"], original["parts"]["metadata"]);
    assert_eq!(
        after["parts"]["futurePart"],
        original["parts"]["futurePart"]
    );
    assert_eq!(after["futureTop"], original["futureTop"]);
    assert_eq!(after["parts"]["usageSync"]["value"]["trusted"], false);
    assert_eq!(
        after["parts"]["usageSync"]["value"]["coverage"],
        "legacyUnknownBoundary"
    );
    assert_ne!(
        after["parts"]["usageSync"]["revision"],
        original["parts"]["usageSync"]["revision"]
    );
}
#[test]
#[serial]
fn foundation_full_import_revokes_usage_even_when_checkpoint_numbers_match() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let old = Database::memory().unwrap();
    let path = transcript(home.home());
    let original = metadata_and_usage();
    seed(&db, &path, &original, 2);
    importable(&old);
    for target in [&db, &old] {
        target
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO session_log_sync VALUES(?1,7,8,9)",
                [path.to_string_lossy()],
            )
            .unwrap();
    }
    db.import_sql_string(&old.export_sql_string().unwrap())
        .unwrap();
    assert_revoked(&db, &path, &original);
}
#[test]
#[serial]
fn foundation_db_restore_revokes_usage_but_keeps_other_parts() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let old = Database::memory().unwrap();
    let path = transcript(home.home());
    let original = metadata_and_usage();
    seed(&db, &path, &original, 2);
    let backups = get_app_config_dir().join("backups");
    fs::create_dir_all(&backups).unwrap();
    old.conn
        .lock()
        .unwrap()
        .execute(
            "VACUUM INTO ?1",
            [backups.join("fixture.db").to_string_lossy()],
        )
        .unwrap();
    db.restore_from_backup("fixture.db").unwrap();
    assert_revoked(&db, &path, &original);
}
#[test]
#[serial]
fn foundation_webdav_preserves_proof_only_with_its_local_ledger_and_checkpoint() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let old = Database::memory().unwrap();
    let path = transcript(home.home());
    let original = metadata_and_usage();
    seed(&db, &path, &original, 2);
    importable(&old);
    db.conn.lock().unwrap().execute_batch("INSERT INTO maintenance_flags VALUES('usage_ledger_epoch_v1','local-epoch');
        INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,latency_ms,status_code,created_at) VALUES('kept','fixture','claude','fixture',0,200,1);").unwrap();
    let before_token =
        super::source_cache::read_on(&db.conn.lock().unwrap(), &path.to_string_lossy(), "claude")
            .unwrap()
            .token(super::source_cache::Part::UsageSync);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO session_log_sync VALUES(?1,7,8,9)",
            [path.to_string_lossy()],
        )
        .unwrap();
    db.import_sql_string_for_sync(&old.export_sql_string_for_sync().unwrap())
        .unwrap();
    assert_eq!(cached(&db, &path), original);
    let offset: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT last_line_offset FROM session_log_sync", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(offset, 8);
    let conn = db.conn.lock().unwrap();
    assert_eq!(
        super::source_cache::read_on(&conn, &path.to_string_lossy(), "claude")
            .unwrap()
            .token(super::source_cache::Part::UsageSync),
        before_token
    );
    let fact: String = conn
        .query_row("SELECT request_id FROM proxy_request_logs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(fact, "kept");
}
fn maintenance_fixture(home: &Path, db: &Database, with_archive: bool) {
    let path = home.join(".claude/projects/p/s.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let at = (chrono::Utc::now() - chrono::Duration::days(40)).to_rfc3339();
    fs::write(&path,json!({"type":"assistant","sessionId":"s","timestamp":at,
        "message":{"id":"new-old-event","model":"fixture","stop_reason":"end_turn","usage":{"input_tokens":100,"output_tokens":10}}}).to_string()+"\n").unwrap();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO maintenance_flags VALUES('backfill_session_archive','pending')",
        [],
    )
    .unwrap();
    if with_archive {
        conn.execute("INSERT INTO session_usage_archive VALUES('claude','s','fixture',999,77,0,0,5,'7.7777',1,2)",[]).unwrap();
    }
}
fn archive(db: &Database) -> Vec<(i64, i64, i64, String)> {
    let conn = db.conn.lock().unwrap();
    let mut stmt=conn.prepare("SELECT input_tokens,output_tokens,request_count,cost_usd FROM session_usage_archive ORDER BY session_id").unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
fn maintenance_state(db: &Database) -> String {
    db.conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT value FROM maintenance_flags WHERE key='backfill_session_archive'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}
#[test]
#[serial]
fn foundation_unproven_backfill_preserves_archive_and_persists_partial() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    maintenance_fixture(home.home(), &db, true);
    let before = archive(&db);
    usage_maintenance::run_pending(&db).unwrap();
    assert_eq!(archive(&db), before);
    assert_eq!(maintenance_state(&db), "partial");
    db.conn.lock().unwrap().execute_batch("CREATE TEMP TRIGGER reject_archive BEFORE INSERT ON session_usage_archive BEGIN SELECT RAISE(ABORT,'must not rescan partial'); END;").unwrap();
    usage_maintenance::run_pending(&db).unwrap();
    assert_eq!(archive(&db), before);
}
#[test]
#[serial]
fn foundation_unsynced_old_append_is_not_backfilled_before_its_live_import() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    maintenance_fixture(home.home(), &db, false);
    usage_maintenance::run_pending(&db).unwrap();
    assert!(archive(&db).is_empty());
    assert_eq!(maintenance_state(&db), "partial");
}
#[test]
#[serial]
fn foundation_backfill_state_failure_does_not_publish_archive_changes() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    maintenance_fixture(home.home(), &db, true);
    let before = archive(&db);
    db.conn.lock().unwrap().execute_batch("CREATE TEMP TRIGGER reject_state BEFORE INSERT ON maintenance_flags WHEN NEW.value IN ('done','partial') BEGIN SELECT RAISE(ABORT,'fixture state failure'); END;").unwrap();
    assert!(usage_maintenance::run_pending(&db).is_err());
    assert_eq!(archive(&db), before);
    assert_eq!(maintenance_state(&db), "pending");
}

#[test]
#[serial]
fn foundation_review_bad_metadata_file_does_not_roll_back_healthy_cache() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let bad = transcript(home.home());
    seed(&db, &bad, &usage_only(), 0);
    db.conn
        .lock()
        .unwrap()
        .execute("UPDATE session_meta_cache SET meta_json='{broken'", [])
        .unwrap();
    let healthy = bad.with_file_name("healthy.jsonl");
    fs::copy(&bad, &healthy).unwrap();
    let mut rows = listing(&bad);
    let mut good = listing(&healthy);
    good[0].session_id = "healthy".into();
    rows.extend(good);
    meta_cache::enrich(&mut rows, Some(&db));
    let mut saved = cached(&db, &healthy);
    saved["parts"]["metadata"]["value"]["details"]["messageCount"] = json!(77);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE session_meta_cache SET meta_json=?1 WHERE source_path=?2",
            rusqlite::params![saved.to_string(), healthy.to_string_lossy()],
        )
        .unwrap();
    meta_cache::enrich(&mut rows, Some(&db));
    assert_eq!(rows[1].message_count, Some(77));
}

#[test]
#[serial]
fn foundation_review_future_usage_is_opaque_to_metadata_but_not_usage_owned_writes() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = transcript(home.home());
    let mut rows = listing(&path);
    meta_cache::enrich(&mut rows, Some(&db));
    let mut saved = cached(&db, &path);
    let future = json!({"revision":"future","value":{"version":999,"trusted":true,"futureSecret":"preserve"}});
    saved["parts"]["usageSync"] = future.clone();
    saved["parts"]["metadata"]["value"]["details"]["messageCount"] = json!(77);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE session_meta_cache SET meta_json=?1",
            [saved.to_string()],
        )
        .unwrap();
    meta_cache::enrich(&mut rows, Some(&db));
    assert_eq!(rows[0].message_count, Some(77));
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(
            b"\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"second\"}}\n",
        )
        .unwrap();
    meta_cache::enrich(&mut rows, Some(&db));
    let after = cached(&db, &path);
    assert_eq!(
        after["parts"]["metadata"]["value"]["details"]["messageCount"],
        2
    );
    assert_eq!(after["parts"]["usageSync"], future);
    let conn = db.conn.lock().unwrap();
    let current = super::source_cache::read_on(&conn, &path.to_string_lossy(), "claude").unwrap();
    assert!(super::source_cache::merge_on(
        &conn,
        &path.to_string_lossy(),
        "claude",
        None,
        super::source_cache::Part::UsageSync,
        &current.token(super::source_cache::Part::UsageSync),
        Some(json!({"version":1}))
    )
    .is_err());
}

fn restored_epoch_refuses_old_prepare(state: &str, binary: bool) {
    use super::source_cache::{self, Part};
    use crate::services::usage_sync_db::{commit_sync_batch, CheckpointChange, SyncState};
    let home = sandbox();
    let db = Database::memory().unwrap();
    let remote = Database::memory().unwrap();
    let path = transcript(home.home());
    importable(&remote);
    if state != "missing" {
        let mut body = metadata_and_usage();
        if state == "metadata" {
            body["parts"].as_object_mut().unwrap().remove("usageSync");
        }
        if state == "tombstone" {
            body["parts"]["usageSync"] = json!({"revision":"cleared","value":null});
        }
        seed(&db, &path, &body, 2);
    }
    for target in [&db, &remote] {
        target
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO session_log_sync VALUES(?1,7,8,9)",
                [path.to_string_lossy()],
            )
            .unwrap();
    }
    let old_token =
        source_cache::read_on(&db.conn.lock().unwrap(), &path.to_string_lossy(), "claude")
            .unwrap()
            .token(Part::UsageSync);
    if binary {
        let backups = get_app_config_dir().join("backups");
        fs::create_dir_all(&backups).unwrap();
        remote
            .conn
            .lock()
            .unwrap()
            .execute(
                "VACUUM INTO ?1",
                [backups.join("epoch.db").to_string_lossy()],
            )
            .unwrap();
        db.restore_from_backup("epoch.db").unwrap();
    } else {
        db.import_sql_string(&remote.export_sql_string().unwrap())
            .unwrap();
    }
    let result = commit_sync_batch(
        &db,
        &[CheckpointChange {
            key: path.to_string_lossy().into(),
            expected: Some(SyncState {
                modified: 7,
                offset: 8,
            }),
            next: Some(SyncState {
                modified: 7,
                offset: 10,
            }),
        }],
        |conn| {
            source_cache::check_on(
                conn,
                &path.to_string_lossy(),
                "claude",
                Part::UsageSync,
                &old_token,
            )?;
            conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,latency_ms,status_code,created_at) VALUES('stale','fixture','claude','fixture',0,200,1)",[])?;
            Ok(())
        },
    );
    assert!(
        result.is_err(),
        "restore accepted pre-restore parse: {state}, binary={binary}"
    );
    let n: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}
macro_rules! epoch_case {
    ($name:ident,$state:literal,$binary:literal) => {
        #[test]
        #[serial]
        fn $name() {
            restored_epoch_refuses_old_prepare($state, $binary);
        }
    };
}
epoch_case!(foundation_review_import_epoch_missing, "missing", false);
epoch_case!(
    foundation_review_import_epoch_metadata_only,
    "metadata",
    false
);
epoch_case!(foundation_review_import_epoch_tombstone, "tombstone", false);
epoch_case!(foundation_review_import_epoch_live_usage, "live", false);
epoch_case!(foundation_review_restore_epoch_missing, "missing", true);
epoch_case!(
    foundation_review_restore_epoch_metadata_only,
    "metadata",
    true
);
epoch_case!(foundation_review_restore_epoch_tombstone, "tombstone", true);
epoch_case!(foundation_review_restore_epoch_live_usage, "live", true);

#[test]
#[serial]
fn foundation_unknown_usage_restore_fails_without_replacing_local_ledger() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let remote = Database::memory().unwrap();
    let path = transcript(home.home());
    let mut original = metadata_and_usage();
    original["parts"]["usageSync"]["value"]["version"] = json!(999);
    seed(&db, &path, &original, 2);
    importable(&remote);
    db.conn.lock().unwrap().execute("INSERT INTO providers(id,app_type,name,settings_config,meta) VALUES('local','claude','Local','{}','{}')",[]).unwrap();
    assert!(db
        .import_sql_string(&remote.export_sql_string().unwrap())
        .is_err());
    assert_eq!(cached(&db, &path), original);
    let id: String = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT id FROM providers", [], |r| r.get(0))
        .unwrap();
    assert_eq!(id, "local");
}
