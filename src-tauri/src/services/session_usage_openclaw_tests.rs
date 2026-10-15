use super::*;
use crate::{database::Database, session_manager::providers::openclaw, test_sandbox::sandbox};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use serial_test::serial;
use std::{fs, path::PathBuf};

fn header(sid: &str) -> Value {
    json!({"type":"session","version":4,"id":sid,"cwd":"/fixture/project","timestamp":"2026-10-15T00:00:00.000Z"})
}
fn message(id: &str) -> Value {
    json!({"type":"message","id":id,"parentId":null,"timestamp":"2026-10-15T00:00:01.123Z","message":{
        "role":"assistant","model":"fixture-model","provider":"fixture-provider","api":"openai-responses",
        "content":[{"type":"text","text":"native fixture body"}],
        "usage":{"input":40,"output":10,"cacheRead":100,"cacheWrite":20,"totalTokens":170,"reasoningTokens":3},
        "timestamp":1792022401123_i64,"stopReason":"stop"}})
}
fn legacy(root: &Path, agent: &str, sid: &str, events: &[Value]) -> PathBuf {
    let path = root
        .join("agents")
        .join(agent)
        .join("sessions")
        .join(format!("{sid}.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_events(
        &path,
        &std::iter::once(header(sid))
            .chain(events.iter().cloned())
            .collect::<Vec<_>>(),
    );
    path
}
fn write_events(path: &Path, events: &[Value]) {
    fs::write(
        path,
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
}
fn native_db(root: &Path, agent: &str, sid: &str, events: &[Value], compressed: bool) -> PathBuf {
    let path = root
        .join("agents")
        .join(agent)
        .join("agent/openclaw-agent.sqlite");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA user_version=24;
        CREATE TABLE schema_meta(meta_key TEXT PRIMARY KEY,role TEXT,schema_version INTEGER,agent_id TEXT,app_version TEXT,created_at INTEGER,updated_at INTEGER);
        CREATE TABLE session_windows(session_id TEXT PRIMARY KEY,reason TEXT,previous_session_id TEXT);
        CREATE TABLE transcript_events(session_id TEXT,seq INTEGER,event_json TEXT,created_at INTEGER,event_zstd BLOB,event_utf8_bytes INTEGER,navigation_json TEXT,PRIMARY KEY(session_id,seq));
        CREATE TABLE session_transcript_archives(session_id TEXT,generation TEXT);
        CREATE TABLE session_transcript_cold_archives(session_id TEXT,generation TEXT);").unwrap();
    conn.execute(
        "INSERT INTO schema_meta VALUES('primary','agent',24,?1,'2026.10.3',1,1)",
        [agent],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_windows VALUES(?1,'initial',NULL)",
        [sid],
    )
    .unwrap();
    for (n, event) in std::iter::once(header(sid))
        .chain(events.iter().cloned())
        .enumerate()
    {
        let raw = event.to_string();
        let (text, blob) = if compressed && n > 0 {
            (
                None,
                Some(zstd::stream::encode_all(raw.as_bytes(), 1).unwrap()),
            )
        } else {
            (Some(raw.clone()), None)
        };
        conn.execute(
            "INSERT INTO transcript_events VALUES(?1,?2,?3,1,?4,?5,NULL)",
            params![sid, n as i64, text, blob, raw.len() as i64],
        )
        .unwrap();
    }
    path
}
fn counts(fact: &NormalizedUsageFact) -> [u32; 4] {
    let u = &fact.usage;
    [
        u.input_tokens,
        u.output_tokens,
        u.cache_read_tokens,
        u.cache_creation_tokens,
    ]
}
fn ledger(db: &Database) -> Vec<(String, String, [u64; 4])> {
    db.conn.lock().unwrap().prepare("SELECT request_id,session_id,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens FROM proxy_request_logs WHERE app_type='openclaw' ORDER BY request_id").unwrap()
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?,[r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?]))).unwrap().collect::<Result<_,_>>().unwrap()
}
fn assert_rejected(path: &Path) {
    assert!(
        read_openclaw(path).is_err(),
        "invalid native source must not become empty successful coverage"
    );
}

#[test]
#[serial]
fn openclaw_usage_canonical_four_buckets_ignore_context_total_and_reasoning() {
    let home = sandbox();
    let path = legacy(&home.home().join(".openclaw"), "main", "s", &[message("m")]);
    let read = read_openclaw(&path).unwrap();
    assert_eq!(read.facts.len(), 1);
    assert_eq!(counts(&read.facts[0]), [40, 10, 100, 20]);
    assert_eq!(read.facts[0].at_ms, Some(1792022401123));
    assert!(read.reasons.is_empty());
}
#[test]
#[serial]
fn openclaw_usage_responses_shape_subtracts_cache_once() {
    let home = sandbox();
    let mut m = message("m");
    m["message"]["usage"] = json!({"input_tokens":160,"output_tokens":10,"input_tokens_details":{"cached_tokens":100,"cache_write_tokens":20},"output_tokens_details":{"reasoning_tokens":3}});
    let read = read_openclaw(&legacy(&home.home().join(".openclaw"), "main", "s", &[m])).unwrap();
    assert_eq!(read.facts.len(), 1);
    assert_eq!(counts(&read.facts[0]), [40, 10, 100, 20]);
}
#[test]
#[serial]
fn openclaw_usage_cache_only_is_a_real_event() {
    let home = sandbox();
    let mut m = message("m");
    m["message"]["usage"] = json!({"input":0,"output":0,"cacheRead":100,"cacheWrite":20});
    let read = read_openclaw(&legacy(&home.home().join(".openclaw"), "main", "s", &[m])).unwrap();
    assert_eq!(read.facts.len(), 1);
    assert_eq!(counts(&read.facts[0]), [0, 0, 100, 20]);
}
#[test]
#[serial]
fn openclaw_usage_missing_bucket_is_unknown_not_synthetic_zero() {
    let home = sandbox();
    let mut m = message("m");
    m["message"]["usage"] = json!({"input":40,"output":10});
    let read = read_openclaw(&legacy(&home.home().join(".openclaw"), "main", "s", &[m])).unwrap();
    assert!(read.reasons.contains("missingUsageBuckets"));
}
#[test]
#[serial]
fn openclaw_usage_context_only_snapshot_is_not_a_call() {
    let home = sandbox();
    let mut m = message("m");
    m["message"]["usage"] = json!({"totalTokens":999,"contextUsage":{"state":"available","promptTokens":900,"totalTokens":999}});
    let read = read_openclaw(&legacy(&home.home().join(".openclaw"), "main", "s", &[m])).unwrap();
    assert!(read.facts.is_empty());
    assert!(read.reasons.contains("missingUsageBuckets"));
}
#[test]
#[serial]
fn openclaw_usage_missing_model_is_partial_without_current_route_guessing() {
    let home = sandbox();
    let mut m = message("m");
    m["message"].as_object_mut().unwrap().remove("model");
    let read = read_openclaw(&legacy(&home.home().join(".openclaw"), "main", "s", &[m])).unwrap();
    assert_eq!(read.facts.len(), 1);
    assert_eq!(read.facts[0].model, "unknown");
    assert!(read.reasons.contains("missingModel"));
}
#[test]
#[serial]
fn openclaw_usage_missing_time_is_preserved_but_never_fabricated() {
    let home = sandbox();
    let mut m = message("m");
    m.as_object_mut().unwrap().remove("timestamp");
    m["message"].as_object_mut().unwrap().remove("timestamp");
    let read = read_openclaw(&legacy(&home.home().join(".openclaw"), "main", "s", &[m])).unwrap();
    assert_eq!(read.facts.len(), 1);
    assert_eq!(read.facts[0].at_ms, None);
    assert!(read.reasons.contains("missingTimestamp"));
}
#[test]
#[serial]
fn openclaw_usage_missing_native_id_does_not_ack_an_ordinal() {
    let home = sandbox();
    let mut m = message("m");
    m.as_object_mut().unwrap().remove("id");
    let read = read_openclaw(&legacy(&home.home().join(".openclaw"), "main", "s", &[m])).unwrap();
    assert!(read.reasons.contains("missingNativeEventIdentity"));
    assert!(read.facts.iter().all(|f| f.native_key.is_none()));
}
#[test]
#[serial]
fn openclaw_usage_missing_header_does_not_use_filename_as_native_sid() {
    let home = sandbox();
    let path = legacy(&home.home().join(".openclaw"), "main", "s", &[]);
    write_events(&path, &[message("m")]);
    let read = read_openclaw(&path).unwrap();
    assert!(read.reasons.contains("missingNativeSessionIdentity"));
    assert!(read.facts.iter().all(|f| f.native_key.is_none()));
}
#[test]
#[serial]
fn openclaw_usage_checked_counts_reject_overflow_negative_and_fractional() {
    let home = sandbox();
    for value in [json!(u32::MAX as u64 + 1), json!(-1), json!(1.5)] {
        let mut m = message("m");
        m["message"]["usage"]["input"] = value;
        assert_rejected(&legacy(&home.home().join(".openclaw"), "main", "s", &[m]));
    }
}
#[test]
#[serial]
fn openclaw_usage_responses_inconsistent_cache_is_an_error() {
    let home = sandbox();
    let mut m = message("m");
    m["message"]["usage"] = json!({"input_tokens":10,"output_tokens":1,"input_tokens_details":{"cached_tokens":100,"cache_write_tokens":20}});
    assert_rejected(&legacy(&home.home().join(".openclaw"), "main", "s", &[m]));
}
#[test]
#[serial]
fn openclaw_usage_conflicting_duplicate_native_event_is_not_a_revision() {
    let home = sandbox();
    let a = message("m");
    let mut b = a.clone();
    b["message"]["usage"]["input"] = json!(45);
    assert_rejected(&legacy(
        &home.home().join(".openclaw"),
        "main",
        "s",
        &[a, b],
    ));
}

#[test]
#[serial]
fn openclaw_usage_incomplete_tail_keeps_prefix_and_marks_partial() {
    let home = sandbox();
    let path = legacy(&home.home().join(".openclaw"), "main", "s", &[message("m")]);
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"type\":\"message\"")
        .unwrap();
    let read = read_openclaw(&path).unwrap();
    assert_eq!(read.facts.len(), 1);
    assert_eq!(read.event_count, 2);
    assert!(read.reasons.contains("incompleteEnumeration"));
}
#[test]
#[serial]
fn openclaw_usage_valid_unterminated_last_line_is_consumed() {
    let home = sandbox();
    let path = legacy(&home.home().join(".openclaw"), "main", "s", &[message("m")]);
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.trim_end()).unwrap();
    assert_eq!(read_openclaw(&path).unwrap().facts.len(), 1);
}
#[test]
#[serial]
fn openclaw_usage_sqlite_text_and_zstd_match_native_v4_fixture() {
    let home = sandbox();
    for (agent, compressed) in [("text", false), ("compressed", true)] {
        let path = native_db(
            &home.home().join(".openclaw"),
            agent,
            "s",
            &[message("m")],
            compressed,
        );
        let read = read_openclaw(&path).unwrap();
        assert_eq!(read.facts.len(), 1);
        assert_eq!(counts(&read.facts[0]), [40, 10, 100, 20]);
        assert!(read.sessions.contains("s"));
    }
}
#[test]
#[serial]
fn openclaw_usage_database_schema_and_agent_owner_are_verified() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let path = native_db(&root, "main", "s", &[message("m")], false);
    let conn = Connection::open(&path).unwrap();
    conn.execute("UPDATE schema_meta SET agent_id='other'", [])
        .unwrap();
    assert_rejected(&path);
    conn.execute(
        "UPDATE schema_meta SET agent_id='main',schema_version=25",
        [],
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 25).unwrap();
    assert_rejected(&path);
}
#[test]
#[serial]
fn openclaw_usage_unknown_transcript_version_is_not_empty_success() {
    let home = sandbox();
    let path = legacy(&home.home().join(".openclaw"), "main", "s", &[message("m")]);
    let mut h = header("s");
    h["version"] = json!(99);
    write_events(&path, &[h, message("m")]);
    assert_rejected(&path);
}
#[test]
#[serial]
fn openclaw_usage_compressed_payload_limits_and_recorded_length_are_enforced() {
    let home = sandbox();
    let path = native_db(
        &home.home().join(".openclaw"),
        "main",
        "s",
        &[message("m")],
        true,
    );
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE transcript_events SET event_utf8_bytes=4194305 WHERE seq=1",
        [],
    )
    .unwrap();
    assert_rejected(&path);
    conn.execute(
        "UPDATE transcript_events SET event_utf8_bytes=1 WHERE seq=1",
        [],
    )
    .unwrap();
    assert_rejected(&path);
    conn.execute(
        "UPDATE transcript_events SET event_zstd=x'000102',event_utf8_bytes=10 WHERE seq=1",
        [],
    )
    .unwrap();
    assert_rejected(&path);
}
#[test]
#[serial]
fn openclaw_usage_archive_or_fork_is_explicitly_partial() {
    let home = sandbox();
    let path = native_db(
        &home.home().join(".openclaw"),
        "main",
        "s",
        &[message("m")],
        false,
    );
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO session_transcript_archives VALUES('archived','g')",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE session_windows SET reason='fork',previous_session_id='parent'",
        [],
    )
    .unwrap();
    let read = read_openclaw(&path).unwrap();
    assert!(read.reasons.contains("nativeArchiveNotEnumerated"));
    assert!(read.reasons.contains("forkProvenanceUnverified"));
}
#[test]
#[serial]
fn openclaw_usage_stat_roster_includes_all_agents_primary_and_legacy() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let a = legacy(&root, "a", "s", &[message("m")]);
    let b = native_db(&root, "b", "s", &[message("m")], false);
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(openclaw::usage_files(&root).unwrap(), expected);
}
#[test]
#[serial]
fn openclaw_usage_existing_non_directory_root_is_an_error() {
    let home = sandbox();
    let root = home.home().join("root-file");
    fs::write(&root, "x").unwrap();
    assert!(openclaw::usage_files(&root).is_err());
}
#[cfg(unix)]
#[test]
#[serial]
fn openclaw_usage_roster_does_not_follow_agent_or_database_symlinks() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    fs::create_dir_all(root.join("agents")).unwrap();
    std::os::unix::fs::symlink(home.home(), root.join("agents/main")).unwrap();
    assert!(openclaw::usage_files(&root).is_err());
}
#[test]
#[serial]
fn openclaw_usage_resolves_state_dir_independently_of_config_path() {
    let home = sandbox();
    let state = home.home().join("native-state");
    std::env::set_var("OPENCLAW_STATE_DIR", &state);
    std::env::set_var(
        "OPENCLAW_CONFIG_PATH",
        home.home().join("elsewhere/config.json"),
    );
    assert_eq!(openclaw::usage_root().unwrap(), state);
}
#[test]
#[serial]
fn openclaw_usage_resolves_openclaw_home_and_legacy_default() {
    let home = sandbox();
    let native_home = home.home().join("native-home");
    std::env::set_var("OPENCLAW_HOME", &native_home);
    assert_eq!(
        openclaw::usage_root().unwrap(),
        native_home.join(".openclaw")
    );
    std::env::remove_var("OPENCLAW_HOME");
    fs::create_dir_all(home.home().join(".clawdbot")).unwrap();
    assert_eq!(
        openclaw::usage_root().unwrap(),
        home.home().join(".clawdbot")
    );
}
#[test]
#[serial]
fn openclaw_usage_sandbox_clears_native_resolver_variables() {
    let _home = sandbox();
    for name in [
        "OPENCLAW_HOME",
        "OPENCLAW_STATE_DIR",
        "OPENCLAW_CONFIG_PATH",
    ] {
        assert!(
            std::env::var_os(name).is_none(),
            "sandbox must clear {name}"
        );
    }
    for name in [
        "OPENCLAW_HOME",
        "OPENCLAW_STATE_DIR",
        "OPENCLAW_CONFIG_PATH",
    ] {
        assert!(crate::config::AGENT_DIR_VARS.contains(&name));
    }
}
#[test]
#[serial]
fn openclaw_usage_unreadable_settings_do_not_select_default_state() {
    let home = sandbox();
    let config = home.home().join(".ofox-desktop/settings.json");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, b"{").unwrap();
    crate::settings::reload_settings().unwrap();
    assert!(openclaw::usage_root().is_err());
}
#[test]
#[serial]
fn openclaw_usage_sync_is_idempotent_and_agent_scoped() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    legacy(&root, "a", "same-sid", &[message("same-id")]);
    legacy(&root, "b", "same-sid", &[message("same-id")]);
    let db = Database::memory().unwrap();
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 2);
    assert_eq!(ledger(&db).len(), 2);
    assert_ne!(ledger(&db)[0].1, ledger(&db)[1].1);
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 0);
}
#[test]
#[serial]
fn openclaw_usage_primary_database_suppresses_stale_jsonl_for_known_sid() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let mut stale = message("legacy-only");
    stale["message"]["usage"]["input"] = json!(999);
    legacy(&root, "main", "s", &[stale]);
    native_db(&root, "main", "s", &[message("native")], true);
    let db = Database::memory().unwrap();
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 1);
    assert_eq!(ledger(&db)[0].2, [40, 10, 100, 20]);
}
#[test]
#[serial]
fn openclaw_usage_broken_primary_never_falls_back_to_jsonl() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    legacy(&root, "main", "s", &[message("m")]);
    let path = native_db(&root, "main", "s", &[], false);
    fs::write(path, "broken SQLite").unwrap();
    let db = Database::memory().unwrap();
    assert!(sync_openclaw_usage_at(&db, &root).is_err());
    assert!(ledger(&db).is_empty());
}
#[test]
#[serial]
fn openclaw_usage_missing_time_can_be_completed_without_fake_today_row() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let mut undated = message("m");
    undated.as_object_mut().unwrap().remove("timestamp");
    undated["message"]
        .as_object_mut()
        .unwrap()
        .remove("timestamp");
    let path = legacy(&root, "main", "s", &[undated]);
    let db = Database::memory().unwrap();
    sync_openclaw_usage_at(&db, &root).unwrap();
    assert!(ledger(&db).is_empty());
    write_events(&path, &[header("s"), message("m")]);
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 1);
    assert_eq!(ledger(&db).len(), 1);
}
#[test]
#[serial]
fn openclaw_usage_native_live_revision_updates_in_place() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let path = legacy(&root, "main", "s", &[message("m")]);
    let db = Database::memory().unwrap();
    sync_openclaw_usage_at(&db, &root).unwrap();
    let mut revised = message("m");
    revised["message"]["usage"]["input"] = json!(45);
    write_events(&path, &[header("s"), revised]);
    sync_openclaw_usage_at(&db, &root).unwrap();
    assert_eq!(ledger(&db).len(), 1);
    assert_eq!(ledger(&db)[0].2, [45, 10, 100, 20]);
}
#[test]
#[serial]
fn openclaw_usage_fact_failure_rolls_back_all_agents_and_evidence() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    legacy(&root, "a", "s", &[message("a")]);
    legacy(&root, "b", "s", &[message("b")]);
    let db = Database::memory().unwrap();
    db.conn.lock().unwrap().execute_batch("CREATE TRIGGER reject_second BEFORE INSERT ON proxy_request_logs WHEN (SELECT COUNT(*) FROM proxy_request_logs)>0 BEGIN SELECT RAISE(ABORT,'fixture second'); END;").unwrap();
    assert!(sync_openclaw_usage_at(&db, &root).is_err());
    assert!(ledger(&db).is_empty());
    let c = db.conn.lock().unwrap();
    for table in ["session_log_sync", "session_meta_cache"] {
        assert_eq!(
            c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
#[serial]
fn openclaw_usage_checkpoint_failure_rolls_back_facts_and_retries_unchanged() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    legacy(&root, "main", "s", &[message("m")]);
    let db = Database::memory().unwrap();
    db.conn.lock().unwrap().execute_batch("CREATE TRIGGER reject_checkpoint BEFORE INSERT ON session_log_sync BEGIN SELECT RAISE(ABORT,'fixture checkpoint'); END;").unwrap();
    assert!(sync_openclaw_usage_at(&db, &root).is_err());
    assert!(ledger(&db).is_empty());
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_checkpoint")
        .unwrap();
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 1);
}
#[test]
#[serial]
fn openclaw_usage_prune_then_copy_and_rescan_does_not_recount_native_event() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let mut m = message("m");
    let old = (chrono::Utc::now() - chrono::Duration::days(40)).to_rfc3339();
    m["timestamp"] = json!(old);
    let path = legacy(&root, "main", "s", &[m]);
    let db = Database::memory().unwrap();
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 1);
    db.rollup_and_prune(30).unwrap();
    fs::copy(&path, path.with_file_name("copy.jsonl")).unwrap();
    sync_openclaw_usage_at(&db, &root).unwrap();
    assert!(ledger(&db).is_empty());
    assert_eq!(
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT SUM(request_count) FROM usage_daily_rollups WHERE app_type='openclaw'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
#[serial]
fn openclaw_usage_sqlite_wal_is_read_without_creating_source_shm() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let original = native_db(&home.home().join("original"), "main", "s", &[], false);
    let conn = Connection::open(&original).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    let raw = message("m").to_string();
    conn.execute(
        "INSERT INTO transcript_events VALUES('s',1,?1,1,NULL,?2,NULL)",
        params![raw, raw.len()],
    )
    .unwrap();
    let path = root.join("agents/main/agent/openclaw-agent.sqlite");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::copy(&original, &path).unwrap();
    fs::copy(
        format!("{}-wal", original.display()),
        format!("{}-wal", path.display()),
    )
    .unwrap();
    let before = fs::read(format!("{}-wal", path.display())).unwrap();
    assert!(!PathBuf::from(format!("{}-shm", path.display())).exists());
    let read = read_openclaw(&path).unwrap();
    assert_eq!(read.facts.len(), 1);
    assert!(!PathBuf::from(format!("{}-shm", path.display())).exists());
    assert_eq!(fs::read(format!("{}-wal", path.display())).unwrap(), before);
}

#[test]
#[serial]
fn openclaw_usage_sqlite_enumeration_budget_is_not_unbounded() {
    let home = sandbox();
    let path = native_db(&home.home().join(".openclaw"), "main", "s", &[], false);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100001) INSERT INTO transcript_events SELECT 's',x,'{}',1,NULL,2,NULL FROM n;").unwrap();
    assert_rejected(&path);
}

#[test]
#[serial]
fn openclaw_usage_app_override_is_frozen_and_precedes_native_environment() {
    let home = sandbox();
    let mut settings = crate::settings::get_settings();
    let explicit = home.home().join("app-selected");
    settings.openclaw_config_dir = Some(explicit.to_string_lossy().into_owned());
    crate::settings::update_settings(settings).unwrap();
    std::env::set_var("OPENCLAW_STATE_DIR", home.home().join("environment"));
    assert_eq!(openclaw::usage_root().unwrap(), explicit);
}

#[test]
#[serial]
fn openclaw_usage_empty_database_sid_prevents_legacy_fallback() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    legacy(&root, "main", "s", &[message("m")]);
    let path = native_db(&root, "main", "s", &[], false);
    let conn = Connection::open(path).unwrap();
    conn.execute("DELETE FROM transcript_events", []).unwrap();
    let db = Database::memory().unwrap();
    let result = sync_openclaw_usage_at(&db, &root).unwrap();
    assert!(ledger(&db).is_empty());
    assert!(
        result.files_scanned >= 2,
        "both primary and legacy sources must be observed"
    );
}

#[test]
#[serial]
fn openclaw_usage_missing_remembered_primary_does_not_replay_legacy_or_block_other_agents() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let primary = native_db(&root, "main", "s", &[message("native")], false);
    let db = Database::memory().unwrap();
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 1);
    fs::remove_file(primary).unwrap();
    legacy(&root, "main", "s", &[message("stale-legacy-only")]);
    legacy(&root, "healthy", "other", &[message("other-message")]);
    let result = sync_openclaw_usage_at(&db, &root).unwrap();
    assert_eq!(result.imported, 1);
    assert_eq!(ledger(&db).len(), 2);
    assert!(ledger(&db)
        .iter()
        .all(|(id, _, _)| !id.contains("stale-legacy-only")));
    let conn = db.conn.lock().unwrap();
    assert!(!super::super::usage_source::has_complete_coverage_on(&conn, "openclaw").unwrap());
}

#[test]
#[serial]
fn openclaw_usage_exact_native_duplicate_counts_once() {
    let home = sandbox();
    let m = message("m");
    let path = legacy(&home.home().join(".openclaw"), "main", "s", &[m.clone(), m]);
    let read = read_openclaw(&path).unwrap();
    assert_eq!(read.facts.len(), 1);
    assert!(read.reasons.is_empty());
}
#[test]
#[serial]
fn openclaw_usage_spent_includes_physical_events_outside_active_context() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let path = native_db(
        &root,
        "main",
        "s",
        &[message("active"), message("abandoned")],
        false,
    );
    let c = Connection::open(path).unwrap();
    c.execute_batch("CREATE TABLE session_transcript_active_events(session_id TEXT,event_seq INTEGER);INSERT INTO session_transcript_active_events VALUES('s',1);").unwrap();
    let db = Database::memory().unwrap();
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 2);
    assert_eq!(ledger(&db).iter().map(|r| r.2[0]).sum::<u64>(), 80);
}
#[test]
#[serial]
fn openclaw_usage_fork_database_keeps_raw_facts_but_does_not_count_unknown_seed() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let path = native_db(&root, "main", "child", &[message("copied")], false);
    let c = Connection::open(&path).unwrap();
    c.execute(
        "UPDATE session_windows SET reason='fork',previous_session_id='parent'",
        [],
    )
    .unwrap();
    let raw = read_openclaw(&path).unwrap();
    assert_eq!(raw.facts.len(), 1);
    assert!(raw.reasons.contains("forkProvenanceUnverified"));
    let db = Database::memory().unwrap();
    sync_openclaw_usage_at(&db, &root).unwrap();
    assert!(ledger(&db).is_empty());
    assert!(!super::super::usage_source::has_complete_coverage_on(
        &db.conn.lock().unwrap(),
        "openclaw"
    )
    .unwrap());
}
#[test]
#[serial]
fn openclaw_usage_fork_jsonl_header_does_not_count_unknown_seed() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let path = legacy(&root, "main", "child", &[]);
    let mut h = header("child");
    h["parentSession"] = json!("parent.jsonl");
    write_events(&path, &[h, message("copied")]);
    let raw = read_openclaw(&path).unwrap();
    assert_eq!(raw.facts.len(), 1);
    assert!(raw.reasons.contains("forkProvenanceUnverified"));
    let db = Database::memory().unwrap();
    sync_openclaw_usage_at(&db, &root).unwrap();
    assert!(ledger(&db).is_empty());
}
#[test]
#[serial]
fn openclaw_usage_cold_only_sid_suppresses_legacy_and_remains_partial() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let path = native_db(&root, "main", "s", &[], false);
    let c = Connection::open(path).unwrap();
    c.execute_batch("DELETE FROM transcript_events;DELETE FROM session_windows;INSERT INTO session_transcript_cold_archives VALUES('s','g');").unwrap();
    legacy(&root, "main", "s", &[message("old-copy")]);
    let db = Database::memory().unwrap();
    sync_openclaw_usage_at(&db, &root).unwrap();
    assert!(ledger(&db).is_empty());
    assert!(!super::super::usage_source::has_complete_coverage_on(
        &db.conn.lock().unwrap(),
        "openclaw"
    )
    .unwrap());
}
#[cfg(unix)]
#[test]
#[serial]
fn openclaw_usage_roster_rejects_database_leaf_symlink() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let actual = native_db(
        &home.home().join("actual"),
        "main",
        "s",
        &[message("m")],
        false,
    );
    let link = root.join("agents/main/agent/openclaw-agent.sqlite");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(actual, link).unwrap();
    assert!(openclaw::usage_files(&root).is_err());
}
#[test]
#[serial]
fn openclaw_usage_capability_supports_event_time_without_claiming_analytics() {
    let capability = super::super::usage_query::source_capabilities()
        .into_iter()
        .find(|c| c.tool_id == "openclaw")
        .unwrap();
    assert!(capability.usage_supported);
    assert_eq!(capability.source_ids, vec!["openclaw_session"]);
    assert!(!capability.session_analytics_supported);
    assert!(!capability.project_filter_supported);
    assert!(!capability.lifetime_usage_supported);
}

#[test]
#[serial]
fn openclaw_usage_nonpaid_event_cannot_reuse_paid_native_identity() {
    let home = sandbox();
    let mut user = message("m");
    user["message"]["role"] = json!("user");
    user["message"].as_object_mut().unwrap().remove("usage");
    assert_rejected(&legacy(
        &home.home().join(".openclaw"),
        "main",
        "s",
        &[user, message("m")],
    ));
}
#[test]
#[serial]
fn openclaw_usage_identical_paid_buckets_do_not_hide_native_payload_conflict() {
    let home = sandbox();
    let a = message("m");
    let mut b = a.clone();
    b["message"]["content"] = json!([{"type":"text","text":"conflicting native content"}]);
    assert_rejected(&legacy(
        &home.home().join(".openclaw"),
        "main",
        "s",
        &[a, b],
    ));
}
#[test]
#[serial]
fn openclaw_usage_nested_old_primary_namespace_does_not_poison_direct_agent() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let nested = root.join("agents/child");
    let path = native_db(&nested, "main", "old", &[message("old-message")], false);
    let db = Database::memory().unwrap();
    assert_eq!(sync_openclaw_usage_at(&db, &nested).unwrap().imported, 1);
    fs::remove_file(path).unwrap();
    legacy(&root, "main", "current", &[message("current-message")]);
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 1);
    assert_eq!(ledger(&db).len(), 2);
}
#[cfg(unix)]
#[test]
#[serial]
fn openclaw_usage_direct_database_alias_must_not_lose_the_target_wal() {
    let home = sandbox();
    let original = native_db(&home.home().join("original"), "main", "s", &[], false);
    let c = Connection::open(&original).unwrap();
    c.pragma_update(None, "journal_mode", "WAL").unwrap();
    c.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    let raw = message("m").to_string();
    c.execute(
        "INSERT INTO transcript_events VALUES('s',1,?1,1,NULL,?2,NULL)",
        params![raw, raw.len()],
    )
    .unwrap();
    let alias = home
        .home()
        .join("alias/agents/main/agent/openclaw-agent.sqlite");
    fs::create_dir_all(alias.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(original, &alias).unwrap();
    assert_rejected(&alias);
}
#[test]
#[serial]
fn openclaw_usage_total_decoded_budget_rejects_many_small_compressed_rows() {
    let home = sandbox();
    let path = native_db(&home.home().join(".openclaw"), "main", "s", &[], false);
    let c = Connection::open(&path).unwrap();
    let mut m = message("m");
    m["message"]["content"] = json!([{"type":"text","text":"x".repeat(4*1024*1024-2048)}]);
    let raw = m.to_string();
    let blob = zstd::stream::encode_all(raw.as_bytes(), 1).unwrap();
    for seq in 1..=33 {
        c.execute(
            "INSERT INTO transcript_events VALUES('s',?1,NULL,1,?2,?3,NULL)",
            params![seq, &blob, raw.len()],
        )
        .unwrap();
    }
    let error = read_openclaw(&path)
        .err()
        .expect("aggregate decoded bytes must be bounded");
    assert!(error.to_string().contains("decodedBudgetExceeded"));
}

#[cfg(unix)]
#[test]
#[serial]
fn openclaw_usage_warm_stamp_cannot_trust_database_leaf_alias() {
    let home = sandbox();
    let target = native_db(
        &home.home().join("target"),
        "main",
        "s",
        &[message("m")],
        false,
    );
    let alias = home
        .home()
        .join("alias/agents/main/agent/openclaw-agent.sqlite");
    fs::create_dir_all(alias.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, &alias).unwrap();
    assert!(super::super::usage_source::normalized_stamp("openclaw", &alias).is_err());
}
#[cfg(unix)]
#[test]
#[serial]
fn openclaw_usage_shared_opencode_stamp_also_rejects_database_leaf_alias() {
    let home = sandbox();
    let target = native_db(
        &home.home().join("target"),
        "main",
        "s",
        &[message("m")],
        false,
    );
    let alias = home.home().join("opencode.db");
    std::os::unix::fs::symlink(target, &alias).unwrap();
    assert!(super::super::usage_source::normalized_stamp("opencode", &alias).is_err());
}

#[test]
#[serial]
fn openclaw_usage_native_event_identity_follows_native_trim_without_changing_model() {
    let home = sandbox();
    let mut m = message("  m  ");
    m["message"]["model"] = json!(" model with spaces ");
    let path = legacy(&home.home().join(".openclaw"), "main", "s", &[m.clone()]);
    let raw = read_openclaw(&path).unwrap();
    assert_eq!(
        raw.facts[0].request_id,
        format!("openclaw_session:{}", json!(["main", "s", "m"]))
    );
    assert_eq!(raw.facts[0].model, " model with spaces ");
    write_events(&path, &[header("s"), m, message("m")]);
    assert_rejected(&path);
}

#[test]
#[serial]
fn openclaw_usage_installed_native_sqlite_payload_fixture_is_consumed_exactly() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/openclaw-native-2026.10.3-events.json"
    ))
    .unwrap();
    let sid = fixture["sessionId"].as_str().unwrap();
    let path = native_db(&root, "main", sid, &[], false);
    let c = Connection::open(&path).unwrap();
    c.execute("DELETE FROM transcript_events", []).unwrap();
    for event in fixture["events"].as_array().unwrap() {
        let text = event.get("event").map(Value::to_string);
        let blob = event.get("zstdHex").and_then(Value::as_str).map(|hex| {
            hex.as_bytes()
                .chunks_exact(2)
                .map(|part| u8::from_str_radix(std::str::from_utf8(part).unwrap(), 16).unwrap())
                .collect::<Vec<_>>()
        });
        c.execute(
            "INSERT INTO transcript_events VALUES(?1,?2,?3,1,?4,?5,NULL)",
            params![
                sid,
                event["seq"].as_i64(),
                text,
                blob,
                event["utf8Bytes"].as_i64()
            ],
        )
        .unwrap();
    }
    let read = read_openclaw(&path).unwrap();
    assert_eq!(read.facts.len(), 2);
    assert!(read.reasons.is_empty());
    assert!(read
        .facts
        .iter()
        .all(|f| counts(f) == [40, 10, 100, 20] && f.at_ms == Some(1792022401123)));
    let db = Database::memory().unwrap();
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 2);
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 0);
}

#[test]
#[serial]
fn openclaw_usage_primary_known_sid_does_not_parse_an_obsolete_legacy_body() {
    let home = sandbox();
    let root = home.home().join(".openclaw");
    native_db(&root, "main", "s", &[message("native")], false);
    let path = legacy(&root, "main", "s", &[]);
    let mut old = header("s");
    old["version"] = json!(1);
    let mut malformed = message("obsolete");
    malformed["message"]["usage"]["input"] = json!(-1);
    write_events(&path, &[old, malformed]);
    let db = Database::memory().unwrap();
    assert_eq!(sync_openclaw_usage_at(&db, &root).unwrap().imported, 1);
    assert_eq!(ledger(&db)[0].2, [40, 10, 100, 20]);
    assert!(!super::super::usage_source::has_complete_coverage_on(
        &db.conn.lock().unwrap(),
        "openclaw"
    )
    .unwrap());
}
