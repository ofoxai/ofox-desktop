//! Warm projection retention must not turn fresh SQLite state into stale data.
use super::*;
use crate::{
    services::{
        session_analytics_index::{query_decode_count, reset_query_decode_count, ProjectionCache},
        usage_health::{self, Channel, Outcome},
    },
    session_manager::source_cache::{self, Part},
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const NOW: i64 = 1_792_108_800_000;

fn selected() -> UsageQueryV1 {
    UsageQueryV1 {
        version: 1,
        start_ms: Some(NOW - 86_400_000),
        end_ms: NOW,
        tool_ids: vec!["claude".into()],
        model_ids: vec![],
        project_ids: vec![],
        source_ids: vec![],
    }
}

fn seed(conn: &Connection, path: &str, tokens: u64) {
    source_cache::merge_on(
        conn,
        path,
        "claude",
        None,
        Part::Analytics,
        &None,
        Some(json!({
            "version":1,"stamp":null,
            "sessions":[{
                "identity":{"key":path,"tool_id":"claude","project":"Unknown","verified_root_key":null},
                "native_session_id":path,"title":"Safe title","project_label":null,
                "project_path":null,"manager_key":"manager","source_available":true,
                "messages_supported":true,"message_reasons":[]
            }],
            "facts":[{"event_key":path,"session_key":path,"tool_id":"claude",
                "source_id":"session_log","model":"m","at_ms":NOW-1000,
                "buckets":[tokens,2,0,0],"complete":true}],
            "messages":[{"session_key":path,"native_id":"m","at_ms":NOW-1000,"role":"User","main":true}],
            "reasons":[],"available":true,"status":"ok","priority":0,
            "attemptedAtMs":1,"succeededAtMs":1
        })),
    )
    .unwrap();
}

fn ready(conn: &Connection) {
    for channel in [Channel::Ledger, Channel::Analytics] {
        let attempt = usage_health::begin_on(conn, "claude", "session_log", channel, 1).unwrap();
        assert!(usage_health::finish_on(conn, &attempt, Outcome::Ok, 1).unwrap());
    }
}

fn fixture() -> (Database, ProjectionCache) {
    let db = Database::memory().unwrap();
    {
        let conn = db.conn.lock().unwrap();
        seed(&conn, "source", 3);
        ready(&conn);
    }
    (db, ProjectionCache::default())
}

fn run(db: &Database, cache: &ProjectionCache) -> SessionAnalytics {
    query_with_cache(db, selected(), "ofox.ai", NOW, &chrono::Utc, cache).unwrap()
}

fn source(result: &SessionAnalytics) -> &super::super::usage_query::SourceHealth {
    result
        .sources
        .iter()
        .find(|source| source.source_id == "session_log")
        .unwrap()
}

fn change_raw(conn: &Connection, f: impl FnOnce(&mut Value)) {
    let raw: String = conn
        .query_row(
            "SELECT meta_json FROM session_meta_cache LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut value: Value = serde_json::from_str(&raw).unwrap();
    f(&mut value);
    conn.execute(
        "UPDATE session_meta_cache SET meta_json=?1",
        [value.to_string()],
    )
    .unwrap();
}

#[test]
fn analytics_projection_warm_query_reuses_verified_input_without_decode_or_reason_pollution() {
    let (db, cache) = fixture();
    reset_query_decode_count();
    let cold = run(&db, &cache);
    assert_eq!(query_decode_count(), 1);
    reset_query_decode_count();
    let warm = run(&db, &cache);
    assert_eq!(cold.receipt, warm.receipt);
    assert_eq!(warm.totals.tokens.total_tokens, 5);
    assert_eq!(
        query_decode_count(),
        0,
        "unchanged warm API must not decode cache envelopes"
    );

    let mut altered = selected();
    altered.start_ms = Some(NOW - 2000);
    altered.model_ids = vec!["m".into()];
    query_with_cache(&db, altered, "ofox.ai", NOW, &chrono::Utc, &cache).unwrap();
    let mut unsupported = selected();
    unsupported.tool_ids.push("unbridged".into());
    let partial = query_with_cache(&db, unsupported, "ofox.ai", NOW, &chrono::Utc, &cache).unwrap();
    assert!(partial
        .coverage
        .reasons
        .iter()
        .any(|reason| reason == "unsupportedTool:unbridged"));
    assert_eq!(run(&db, &cache).receipt, cold.receipt);
    assert!(!run(&db, &cache)
        .coverage
        .reasons
        .iter()
        .any(|reason| reason.contains("unbridged")));
    assert_eq!(
        query_decode_count(),
        0,
        "presentation/query changes must borrow the same input"
    );
}

#[test]
fn analytics_projection_every_raw_column_invalidates_and_preserves_original_receipt_hash() {
    let (db, cache) = fixture();
    let initial = run(&db, &cache);
    for (sql, value) in [
        ("UPDATE session_meta_cache SET stamp=?1", "different-stamp"),
        ("UPDATE session_meta_cache SET parser_version=?1", "4321"),
        ("UPDATE session_meta_cache SET provider_id=?1", "gemini"),
        (
            "UPDATE session_meta_cache SET source_path=?1",
            "moved-source",
        ),
    ] {
        db.conn.lock().unwrap().execute(sql, [value]).unwrap();
        reset_query_decode_count();
        let fresh = run(&db, &cache);
        assert_eq!(
            query_decode_count(),
            1,
            "changed column must invalidate: {sql}"
        );
        assert_eq!(fresh.totals.tokens.total_tokens, 5);
        if !sql.contains("source_path") {
            // These columns were never included in the public input hash.
            // Invalidation must not silently redefine snapshotVersion.
            assert_eq!(
                fresh.receipt.snapshot_version,
                initial.receipt.snapshot_version
            );
        }
        reset_query_decode_count();
        run(&db, &cache);
        assert_eq!(
            query_decode_count(),
            0,
            "newly verified input should become warm"
        );
    }
    {
        let conn = db.conn.lock().unwrap();
        change_raw(&conn, |v| {
            v["parts"]["analytics"]["value"]["facts"][0]["buckets"][0] = json!(9)
        });
    }
    let changed = run(&db, &cache);
    assert_eq!(changed.totals.tokens.total_tokens, 11);
    assert_ne!(
        changed.receipt.snapshot_version,
        initial.receipt.snapshot_version
    );
    let reference = {
        let conn = db.conn.lock().unwrap();
        session_analytics_index::read_input_on(&conn).unwrap()
    };
    let cached = {
        let conn = db.conn.lock().unwrap();
        cache.read_on(&conn).unwrap()
    };
    assert_eq!(
        serde_json::to_value(&*cached).unwrap(),
        serde_json::to_value(reference).unwrap()
    );
}

#[test]
fn analytics_projection_external_connection_updates_and_different_databases_never_cross_contaminate(
) {
    let (original, cache) = fixture();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("cache.sqlite");
    original
        .conn
        .lock()
        .unwrap()
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    let db = Database {
        conn: std::sync::Mutex::new(Connection::open(&path).unwrap()),
        analytics_identity: std::sync::atomic::AtomicU64::new(
            crate::database::next_analytics_identity(),
        ),
    };
    let old = run(&db, &cache);
    let external = Connection::open(&path).unwrap();
    change_raw(&external, |v| {
        v["parts"]["analytics"]["value"]["facts"][0]["buckets"][0] = json!(17)
    });
    let updated = run(&db, &cache);
    assert_eq!(updated.totals.tokens.total_tokens, 19);
    assert_ne!(
        updated.receipt.snapshot_version,
        old.receipt.snapshot_version
    );
    assert_eq!(run(&original, &cache).totals.tokens.total_tokens, 5);
    assert_eq!(run(&db, &cache).totals.tokens.total_tokens, 19);
    reset_query_decode_count();
    run(&db, &cache);
    assert_eq!(query_decode_count(), 0);
}

#[test]
fn analytics_projection_delete_clear_corrupt_and_repair_are_observed_without_native_fallback() {
    let (db, cache) = fixture();
    seed(&db.conn.lock().unwrap(), "second", 7);
    assert_eq!(run(&db, &cache).totals.tokens.total_tokens, 14);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM session_meta_cache WHERE source_path='second'",
            [],
        )
        .unwrap();
    assert_eq!(run(&db, &cache).totals.tokens.total_tokens, 5);
    db.conn
        .lock()
        .unwrap()
        .execute("UPDATE session_meta_cache SET meta_json='broken'", [])
        .unwrap();
    let damaged = run(&db, &cache);
    assert_eq!(damaged.totals.tokens.total_tokens, 0);
    assert!(damaged
        .coverage
        .reasons
        .iter()
        .any(|reason| reason == "analyticsCacheUnavailable"));
    reset_query_decode_count();
    run(&db, &cache);
    assert_eq!(
        query_decode_count(),
        0,
        "a verified unavailable projection may also be reused"
    );
    db.conn
        .lock()
        .unwrap()
        .execute("DELETE FROM session_meta_cache", [])
        .unwrap();
    assert_eq!(run(&db, &cache).totals.session_count, 0);
    seed(&db.conn.lock().unwrap(), "repaired", 23);
    assert_eq!(run(&db, &cache).totals.tokens.total_tokens, 25);
    db.conn
        .lock()
        .unwrap()
        .execute("DROP TABLE session_meta_cache", [])
        .unwrap();
    assert!(
        query_with_cache(&db, selected(), "ofox.ai", NOW, &chrono::Utc, &cache).is_err(),
        "warm retention must not hide SQLite read failures"
    );
}

#[test]
fn analytics_projection_common_health_epoch_and_retained_range_changes_are_fresh() {
    let (db, cache) = fixture();
    assert_eq!(source(&run(&db, &cache)).status, "ok");
    {
        let conn = db.conn.lock().unwrap();
        let attempt =
            usage_health::begin_on(&conn, "claude", "session_log", Channel::Analytics, 2).unwrap();
        assert_eq!(source(&run_without_lock(&conn, &cache)).status, "unknown");
        usage_health::finish_on(
            &conn,
            &attempt,
            Outcome::Failed("usage.sources.readFailed"),
            3,
        )
        .unwrap();
    }
    assert_eq!(source(&run(&db, &cache)).status, "failed");
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO maintenance_flags(key,value) VALUES('usage_ledger_epoch_v1','restored')",
            [],
        )
        .unwrap();
    }
    assert_eq!(source(&run(&db, &cache)).status, "unknown");
    db.set_setting(usage_health::SETTING_KEY, "not valid health")
        .unwrap();
    assert_eq!(
        source(&run(&db, &cache)).error_code.as_deref(),
        Some("usage.sources.invalidHealth")
    );
    {
        let conn = db.conn.lock().unwrap();
        conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at,data_source) VALUES('detail','fixture','claude','m',1,0,0,1,'session_log')", []).unwrap();
    }
    let updated = run(&db, &cache);
    assert_eq!(
        source(&updated).retained_detail_range,
        Some(super::super::usage_query::RetainedDetailRange {
            start_ms: 1000,
            end_ms: 2000
        })
    );
    assert_eq!(updated.totals.tokens.total_tokens, 5);
    reset_query_decode_count();
    run(&db, &cache);
    assert_eq!(query_decode_count(), 0);
}

// Exercise the same projection+catalog transaction while a test owns the
// connection, without trying to reacquire Database's mutex.
fn run_without_lock(conn: &Connection, cache: &ProjectionCache) -> SessionAnalytics {
    let input = cache.read_on(conn).unwrap();
    let catalog = PriceCatalog::load_on(conn, "ofox.ai", NOW).unwrap();
    let query = selected();
    let day_bounds = days(&query, &input, &chrono::Utc).unwrap();
    build_analysis_in(query, &input, &catalog, &day_bounds, &chrono::Utc).unwrap()
}

#[test]
fn analytics_projection_prices_apex_and_freshness_are_reloaded_without_redecoding_input() {
    let (db, cache) = fixture();
    let missing = run(&db, &cache);
    assert!(missing.totals.estimate.known_cost_usd.is_none());
    for (apex, prompt, completion, expected) in [
        ("ofox.ai", "1", "2", "7"),
        ("ofox.ai", "2", "3", "12"),
        ("ofox.io", "4", "5", "22"),
    ] {
        PriceCatalog::ingest_on(
            &db.conn.lock().unwrap(),
            apex,
            &json!({"data":[{"id":"m","pricing":{"prompt":prompt,"completion":completion}}]}),
            NOW,
        )
        .unwrap();
        reset_query_decode_count();
        let result = query_with_cache(&db, selected(), apex, NOW, &chrono::Utc, &cache).unwrap();
        assert_eq!(
            result.totals.estimate.known_cost_usd.as_deref(),
            Some(expected)
        );
        assert_eq!(result.pricing_coverage.apex, apex);
        assert_ne!(
            result.receipt.snapshot_version,
            missing.receipt.snapshot_version
        );
        assert_eq!(
            query_decode_count(),
            0,
            "price refresh is independent of input decode"
        );
    }
    let stale = query_with_cache(
        &db,
        selected(),
        "ofox.io",
        NOW + 86_400_001,
        &chrono::Utc,
        &cache,
    )
    .unwrap();
    assert!(stale.pricing_coverage.stale);
    assert_eq!(stale.totals.estimate.known_cost_usd.as_deref(), Some("22"));
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE settings SET value='broken' WHERE key=?1",
            params!["ofox_usage_pricing_catalog_v1:ofox.io"],
        )
        .unwrap();
    let corrupt = query_with_cache(&db, selected(), "ofox.io", NOW, &chrono::Utc, &cache).unwrap();
    assert!(corrupt.totals.estimate.known_cost_usd.is_none());
    assert_eq!(corrupt.pricing_coverage.catalog_state, "invalid");
}

#[test]
fn unchanged_managed_database_does_not_copy_raw_history_on_filter_change() {
    let (db, cache) = fixture();
    run(&db, &cache);
    reset_query_decode_count();
    let mut next = selected();
    next.start_ms = Some(NOW - 3_600_000);
    let answer = query_with_cache(&db, next, "ofox.ai", NOW, &chrono::Utc, &cache).unwrap();
    assert_eq!(answer.totals.tokens.input_tokens, 3);
    assert_eq!(
        super::super::session_analytics_index::query_raw_bytes(),
        0,
        "a verified unchanged SQLite snapshot must not copy every source JSON again"
    );
}

#[test]
#[ignore = "manual warm-filter read benchmark; synthetic 32 MiB cache"]
fn managed_projection_warm_read_benchmark() {
    let (db, cache) = fixture();
    let conn = db.conn.lock().unwrap();
    for i in 0..256 {
        seed(&conn, &format!("bench-{i}"), 3);
    }
    conn.execute(
        "UPDATE session_meta_cache SET meta_json=json_set(meta_json,'$.padding',?1)",
        ["x".repeat(128 * 1024)],
    )
    .unwrap();
    drop(conn);
    run(&db, &cache);
    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction().unwrap();
    let start = std::time::Instant::now();
    cache.read_on(&tx).unwrap();
    let full = start.elapsed();
    reset_query_decode_count();
    let start = std::time::Instant::now();
    cache.read_managed_on(&db, &tx).unwrap();
    let fast = start.elapsed();
    assert_eq!(super::super::session_analytics_index::query_raw_bytes(), 0);
    eprintln!("synthetic 32 MiB warm projection: full validation {full:?}, unchanged SQLite observation {fast:?}");
}

#[test]
fn managed_projection_does_not_publish_uncommitted_input_after_rollback() {
    let (db, cache) = fixture();
    run(&db, &cache);
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        change_raw(&tx, |v| {
            v["parts"]["analytics"]["value"]["facts"][0]["buckets"][0] = json!(99)
        });
        let dirty = cache.read_managed_on(&db, &tx).unwrap();
        assert_eq!(dirty.facts[0].buckets[0], 99);
        tx.rollback().unwrap();
    }
    assert_eq!(run(&db, &cache).totals.tokens.total_tokens, 5);
}

#[test]
fn managed_projection_pins_wal_snapshot_and_observes_the_next_external_commit() {
    let (original, cache) = fixture();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("wal.sqlite");
    original
        .conn
        .lock()
        .unwrap()
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    let db = Database {
        conn: std::sync::Mutex::new(conn),
        analytics_identity: std::sync::atomic::AtomicU64::new(
            crate::database::next_analytics_identity(),
        ),
    };
    run(&db, &cache);
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        let before = cache.read_managed_on(&db, &tx).unwrap();
        let external = Connection::open(&path).unwrap();
        change_raw(&external, |v| {
            v["parts"]["analytics"]["value"]["facts"][0]["buckets"][0] = json!(17)
        });
        assert!(std::sync::Arc::ptr_eq(
            &before,
            &cache.read_managed_on(&db, &tx).unwrap()
        ));
        tx.commit().unwrap();
    }
    assert_eq!(run(&db, &cache).totals.tokens.total_tokens, 19);
}

#[test]
#[serial_test::serial]
fn managed_projection_revalidates_after_sql_import_and_backup_restore() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::init().unwrap();
    {
        let conn = db.conn.lock().unwrap();
        seed(&conn, "source", 3);
        ready(&conn);
    }
    let cache = ProjectionCache::default();
    let other = Database::memory().unwrap();
    other.conn.lock().unwrap().execute("INSERT INTO providers(id,app_type,name,settings_config,meta) VALUES ('fixture','claude','Fixture','{}','{}')", []).unwrap();
    run(&db, &cache);
    let before = db
        .analytics_identity
        .load(std::sync::atomic::Ordering::Relaxed);
    db.import_sql_string(&other.export_sql_string().unwrap())
        .unwrap();
    assert_ne!(
        before,
        db.analytics_identity
            .load(std::sync::atomic::Ordering::Relaxed)
    );
    reset_query_decode_count();
    run(&db, &cache);
    assert!(super::super::session_analytics_index::query_raw_bytes() > 0);
    let backup = db.backup_database_file().unwrap().unwrap();
    let before = db
        .analytics_identity
        .load(std::sync::atomic::Ordering::Relaxed);
    db.restore_from_backup(backup.file_name().unwrap().to_str().unwrap())
        .unwrap();
    assert_ne!(
        before,
        db.analytics_identity
            .load(std::sync::atomic::Ordering::Relaxed)
    );
    reset_query_decode_count();
    run(&db, &cache);
    assert!(super::super::session_analytics_index::query_raw_bytes() > 0);
}
