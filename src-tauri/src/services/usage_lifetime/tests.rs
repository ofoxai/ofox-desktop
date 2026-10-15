use super::*;
use crate::services::{ofox_pricing::PriceCatalog, session_usage_hermes::read_lifetime};
use crate::session_manager::source_cache::{self, Part};
use rusqlite::{params, Connection};
use serde_json::json;
use std::{fs, path::PathBuf};

const NATIVE_SQL: &str = include_str!("../fixtures/hermes-native-0.21.7-usage.sql");

fn native_fixture(home: &Path, session: &str) -> PathBuf {
    let root = home.join("hermes-fixture");
    fs::create_dir_all(&root).unwrap();
    let conn = Connection::open(root.join("state.db")).unwrap();
    conn.execute_batch(NATIVE_SQL).unwrap();
    conn.execute("DELETE FROM sessions WHERE id != ?1", [session])
        .unwrap();
    conn.execute(
        "DELETE FROM session_model_usage WHERE session_id != ?1",
        [session],
    )
    .unwrap();
    root
}

fn default_native_fixture() -> PathBuf {
    let root = crate::hermes_config::get_hermes_dir();
    fs::create_dir_all(&root).unwrap();
    Connection::open(root.join("state.db"))
        .unwrap()
        .execute_batch(NATIVE_SQL)
        .unwrap();
    root
}

#[test]
#[serial_test::serial]
fn hermes_root_missing_settings_reads_the_native_default() {
    let _sandbox = crate::test_sandbox::sandbox();
    let root = default_native_fixture();
    assert_eq!(resolve_hermes_root().unwrap(), root);
    assert!(read_lifetime(&root).unwrap().session_records > 0);
}

#[test]
#[serial_test::serial]
fn hermes_root_recovered_settings_keeps_the_recovered_override() {
    let sandbox = crate::test_sandbox::sandbox();
    let custom = native_fixture(sandbox.home(), "main-aux");
    let folder = sandbox.home().join(".ofox-desktop");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("settings.json.bak"),
        json!({"hermesConfigDir": custom.to_str().unwrap()}).to_string(),
    )
    .unwrap();
    fs::write(folder.join("settings.json"), "invalid fixture").unwrap();
    crate::settings::reload_settings().unwrap();
    assert_eq!(
        crate::settings::settings_health().state,
        crate::settings::SettingsHealthState::Recovered
    );
    assert_eq!(resolve_hermes_root().unwrap(), custom);
}

fn assert_unknown_settings_refused(environment_override: bool) {
    let sandbox = crate::test_sandbox::sandbox();
    let root = default_native_fixture();
    if environment_override {
        std::env::set_var("HERMES_HOME", &root);
    }
    let folder = sandbox.home().join(".ofox-desktop");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("settings.json"), "invalid fixture SECRET").unwrap();
    fs::write(folder.join("settings.json.bak"), "invalid backup").unwrap();
    crate::settings::reload_settings().unwrap();
    assert_eq!(
        crate::settings::settings_health().state,
        crate::settings::SettingsHealthState::Unreadable
    );
    let db = Database::memory().unwrap();
    let result = resolve_hermes_root().and_then(|root| hermes_summary(&db, &root, "ofox.ai", 10));
    let error = result.expect_err("unknown settings fell through to readable default statistics");
    assert_eq!(error.to_string(), "usage.lifetime.settingsUnreadable");
    let cache_rows: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM session_meta_cache", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        cache_rows, 0,
        "unknown settings must not publish default facts"
    );
}

#[test]
#[serial_test::serial]
fn hermes_root_unreadable_settings_cannot_read_the_default_database() {
    assert_unknown_settings_refused(false);
}

#[test]
#[serial_test::serial]
fn hermes_root_unreadable_settings_cannot_fall_through_to_environment() {
    assert_unknown_settings_refused(true);
}

fn assert_amount(snapshot: &LifetimeNativeSnapshot, input: u64, output: u64) {
    assert_eq!(snapshot.totals.input_tokens, Some(input));
    assert_eq!(snapshot.totals.output_tokens, Some(output));
    assert_eq!(snapshot.totals.total_tokens, Some(input + output));
    assert_eq!(snapshot.session_records, 1);
}

fn priced_models(db: &Database, amount: &str, at: i64) {
    let payload = json!({"data":[
        {"id":"main-model","pricing":{"prompt":amount,"completion":amount,"input_cache_read":"0","input_cache_write":"0"}},
        {"id":"voice-model","pricing":{"prompt":amount,"completion":amount,"input_cache_read":"0","input_cache_write":"0"}},
        {"id":"aux-model","pricing":{"prompt":amount,"completion":amount,"input_cache_read":"0","input_cache_write":"0"}}
    ]});
    PriceCatalog::ingest_on(&db.conn.lock().unwrap(), "ofox.ai", &payload, at).unwrap();
}

#[test]
#[serial_test::serial]
fn hermes_native_main_and_auxiliary_are_added_once() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    let snapshot = read_lifetime(&root).unwrap();
    assert_amount(&snapshot, 130, 13);
    let main = snapshot
        .models
        .iter()
        .find(|m| m.model_id.as_deref() == Some("main-model"))
        .unwrap();
    assert_eq!(main.category, LifetimeCategory::Main);
    assert_eq!(main.buckets.input_tokens, Some(100));
    assert_eq!(snapshot.totals.api_call_count, Some(2));
}

#[test]
#[serial_test::serial]
fn hermes_native_voice_is_already_in_the_session_pool() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-voice-aux");
    let snapshot = read_lifetime(&root).unwrap();
    assert_amount(&snapshot, 150, 15);
    let voice = snapshot
        .models
        .iter()
        .find(|m| m.model_id.as_deref() == Some("voice-model"))
        .unwrap();
    assert_eq!(voice.category, LifetimeCategory::Voice);
    assert_eq!(voice.buckets.input_tokens, Some(20));
    assert_eq!(snapshot.totals.api_call_count, Some(3));
}

#[test]
#[serial_test::serial]
fn hermes_native_absolute_pool_has_unknown_model_residual() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "absolute-aux");
    let snapshot = read_lifetime(&root).unwrap();
    assert_amount(&snapshot, 530, 53);
    let residual = snapshot
        .models
        .iter()
        .find(|m| m.model_id.is_none())
        .unwrap();
    assert_eq!(residual.category, LifetimeCategory::Unattributed);
    assert_eq!(residual.buckets.input_tokens, Some(500));
    assert_eq!(snapshot.totals.api_call_count, Some(6));
    let db = Database::memory().unwrap();
    priced_models(&db, "0.001", 10);
    let summary = hermes_summary(&db, &root, "ofox.ai", 20).unwrap();
    assert_eq!(summary.estimate.known_cost_usd.as_deref(), Some("0.033"));
    assert_eq!(summary.estimate.availability, "partial");
    assert!(summary
        .by_model
        .iter()
        .any(|m| m.fact.model_id.is_none() && m.estimate.availability == "unknown"));
}

#[test]
#[serial_test::serial]
fn hermes_unknown_task_does_not_double_count_an_uncertain_pool() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    Connection::open(root.join("state.db"))
        .unwrap()
        .execute(
            "UPDATE session_model_usage SET task='future_native_task' WHERE task='vision'",
            [],
        )
        .unwrap();
    let snapshot = read_lifetime(&root).unwrap();
    assert_amount(&snapshot, 100, 10);
    assert_eq!(snapshot.totals.amount_kind, LifetimeAmountKind::LowerBound);
    assert!(!snapshot.reasons.is_empty());
}

#[test]
#[serial_test::serial]
fn hermes_inconsistent_included_rows_keep_the_authoritative_pool() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-voice-aux");
    Connection::open(root.join("state.db"))
        .unwrap()
        .execute("UPDATE sessions SET input_tokens=50,output_tokens=5", [])
        .unwrap();
    let snapshot = read_lifetime(&root).unwrap();
    assert_amount(&snapshot, 80, 8);
    assert!(!snapshot.reasons.is_empty());
    assert!(!snapshot
        .models
        .iter()
        .any(|m| m.model_id.as_deref() == Some("main-model")));
    assert!(snapshot
        .models
        .iter()
        .any(|m| m.model_id.is_none() && m.buckets.input_tokens == Some(50)));
}

#[test]
#[serial_test::serial]
fn hermes_missing_counter_is_unknown_while_cache_only_is_real_usage() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "absolute-aux");
    Connection::open(root.join("state.db")).unwrap().execute_batch(
        "DELETE FROM session_model_usage;
         UPDATE sessions SET input_tokens=NULL,output_tokens=0,cache_read_tokens=80,cache_write_tokens=0;",
    ).unwrap();
    let snapshot = read_lifetime(&root).unwrap();
    assert_eq!(snapshot.totals.input_tokens, None);
    assert_eq!(snapshot.totals.cache_read_tokens, Some(80));
    assert_eq!(snapshot.totals.total_tokens, None);
    assert_ne!(snapshot.totals.amount_kind, LifetimeAmountKind::Known);
}

#[test]
#[serial_test::serial]
fn hermes_reader_is_read_only_and_never_requires_message_columns() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    let before = fs::read(root.join("state.db")).unwrap();
    read_lifetime(&root).unwrap();
    assert_eq!(fs::read(root.join("state.db")).unwrap(), before);
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
}

fn wal_only_fixture(home: &Path) -> PathBuf {
    let root = native_fixture(home, "main-aux");
    let conn = Connection::open(root.join("state.db")).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    let main = fs::read(root.join("state.db")).unwrap();
    conn.execute("UPDATE sessions SET input_tokens=200", [])
        .unwrap();
    let wal = fs::read(root.join("state.db-wal")).unwrap();
    drop(conn);
    // Reconstruct an offline native snapshot with committed WAL but no SHM.
    // Closing a fixture writer normally checkpoints/deletes the WAL.
    fs::write(root.join("state.db"), main).unwrap();
    fs::write(root.join("state.db-wal"), wal).unwrap();
    root
}

#[test]
#[serial_test::serial]
fn hermes_offline_wal_is_read_without_creating_native_shm_or_changing_bytes() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = wal_only_fixture(sandbox.home());
    let main = fs::read(root.join("state.db")).unwrap();
    let wal = fs::read(root.join("state.db-wal")).unwrap();
    let snapshot = read_lifetime(&root).unwrap();
    assert_amount(&snapshot, 230, 13);
    assert_eq!(fs::read(root.join("state.db")).unwrap(), main);
    assert_eq!(fs::read(root.join("state.db-wal")).unwrap(), wal);
    assert!(!root.join("state.db-shm").exists());
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}

#[test]
#[serial_test::serial]
fn hermes_empty_wal_does_not_make_a_plain_database_corrupt() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    fs::write(root.join("state.db-wal"), b"").unwrap();
    assert_amount(&read_lifetime(&root).unwrap(), 130, 13);
    assert_eq!(fs::read(root.join("state.db-wal")).unwrap(), b"");
    assert!(!root.join("state.db-shm").exists());
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn hermes_sqlite_leaf_alias_with_target_wal_never_reports_stale_success() {
    let sandbox = crate::test_sandbox::sandbox();
    let target = wal_only_fixture(sandbox.home());
    let alias = sandbox.home().join("alias");
    fs::create_dir_all(&alias).unwrap();
    std::os::unix::fs::symlink(target.join("state.db"), alias.join("state.db")).unwrap();
    assert!(
        read_lifetime(&alias).is_err(),
        "a main alias without its target WAL must fail closed"
    );
    assert!(crate::services::session_usage_hermes::current_stamp(&alias).is_err());
    assert_amount(&read_lifetime(&target).unwrap(), 230, 13);
}

#[test]
#[serial_test::serial]
fn hermes_source_snapshot_rejects_main_identity_change_and_wal_set_change() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    assert_amount(&read_lifetime(&root).unwrap(), 130, 13);
    let changed = crate::services::session_usage_hermes::read_lifetime_after_copy(&root, || {
        let original = root.join("state.db");
        let replacement = root.join("replacement.db");
        fs::write(&replacement, fs::read(&original).unwrap()).unwrap();
        fs::rename(replacement, original).unwrap();
    });
    assert!(changed.is_err());
    let added_wal = crate::services::session_usage_hermes::read_lifetime_after_copy(&root, || {
        fs::write(root.join("state.db-wal"), b"").unwrap();
    });
    assert!(added_wal.is_err());
    let removed_wal =
        crate::services::session_usage_hermes::read_lifetime_after_copy(&root, || {
            fs::remove_file(root.join("state.db-wal")).unwrap();
        });
    assert!(removed_wal.is_err());
    let changed_main =
        crate::services::session_usage_hermes::read_lifetime_after_copy(&root, || {
            Connection::open(root.join("state.db"))
                .unwrap()
                .execute("UPDATE sessions SET output_tokens=20", [])
                .unwrap();
        });
    assert!(changed_main.is_err());
}

#[test]
#[serial_test::serial]
fn hermes_source_snapshot_rejects_wal_only_change() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = wal_only_fixture(sandbox.home());
    assert_amount(&read_lifetime(&root).unwrap(), 230, 13);
    let changed = crate::services::session_usage_hermes::read_lifetime_after_copy(&root, || {
        let wal_path = root.join("state.db-wal");
        let mut wal = fs::read(&wal_path).unwrap();
        wal.push(0);
        fs::write(wal_path, wal).unwrap();
    });
    assert!(changed.is_err());
}

#[test]
#[serial_test::serial]
fn hermes_missing_source_is_distinct_from_corrupt_source() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let root = sandbox.home().join("missing-hermes");
    let missing = hermes_summary(&db, &root, "ofox.ai", 10).unwrap();
    assert_eq!(missing.source_state, "missing");
    assert_eq!(missing.totals.input_tokens, None);
    assert!(!root.exists());
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("state.db"), b"secret-sentinel invalid SQLite").unwrap();
    let failed = hermes_summary(&db, &root, "ofox.ai", 20).unwrap();
    assert_eq!(failed.source_state, "error");
    assert!(!serde_json::to_string(&failed)
        .unwrap()
        .contains("secret-sentinel"));
    assert!(!serde_json::to_string(&failed)
        .unwrap()
        .contains(root.to_str().unwrap()));
}

#[test]
#[serial_test::serial]
fn hermes_last_success_survives_source_failure_and_a_new_service_call() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    let db = Database::memory().unwrap();
    let first = hermes_summary(&db, &root, "ofox.ai", 10).unwrap();
    assert_eq!(first.last_success_at_ms, Some(10));
    fs::write(root.join("state.db"), b"corrupt native source").unwrap();
    let failed = hermes_summary(&db, &root, "ofox.ai", 20).unwrap();
    assert_eq!(failed.source_state, "error");
    assert!(failed.stale);
    assert_eq!(failed.last_success_at_ms, Some(10));
    assert_eq!(failed.totals.input_tokens, first.totals.input_tokens);
    let conn = db.conn.lock().unwrap();
    let stored: String = conn
        .query_row(
            "SELECT meta_json FROM session_meta_cache WHERE provider_id='hermes'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let stored: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(
        stored["parts"]["lifetimeUsage"]["value"]["sourceState"],
        "error"
    );
    assert_eq!(
        stored["parts"]["lifetimeUsage"]["value"]["lastSuccessAtMs"],
        10
    );
}

#[test]
#[serial_test::serial]
fn hermes_current_catalog_reprices_cached_facts_and_missing_price_is_unknown() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    let db = Database::memory().unwrap();
    let unknown = hermes_summary(&db, &root, "ofox.ai", 10).unwrap();
    assert_eq!(unknown.estimate.known_cost_usd, None);
    priced_models(&db, "0.001", 20);
    let one = hermes_summary(&db, &root, "ofox.ai", 21).unwrap();
    assert_eq!(one.estimate.known_cost_usd.as_deref(), Some("0.143"));
    priced_models(&db, "0.002", 30);
    let two = hermes_summary(&db, &root, "ofox.ai", 31).unwrap();
    assert_eq!(two.estimate.known_cost_usd.as_deref(), Some("0.286"));
    assert_eq!(one.snapshot_version, two.snapshot_version);
    assert_eq!(one.last_success_at_ms, two.last_success_at_ms);
    PriceCatalog::ingest_on(
        &db.conn.lock().unwrap(),
        "ofox.io",
        &json!({"data":[
            {"id":"main-model","pricing":{"prompt":"0.003","completion":"0.003"}},
            {"id":"aux-model","pricing":{"prompt":"0.003","completion":"0.003"}}
        ]}),
        40,
    )
    .unwrap();
    let other_apex = hermes_summary(&db, &root, "ofox.io", 41).unwrap();
    assert_eq!(other_apex.estimate.known_cost_usd.as_deref(), Some("0.429"));
    assert_eq!(other_apex.pricing_coverage.apex, "ofox.io");
    assert_eq!(other_apex.snapshot_version, two.snapshot_version);
    assert_eq!(other_apex.last_success_at_ms, two.last_success_at_ms);
}

fn initial_part(part: Part) -> Value {
    match part {
        Part::Metadata => json!({"parserVersion":1,"stamp":"meta","details":{"futureMetadata":42}}),
        Part::UsageSync => {
            json!({"version":1,"trusted":false,"completeEventIdentitySet":false,"acknowledgedEventIds":[]})
        }
        Part::Analytics => json!({"version":1,"futureAnalytics":43}),
        Part::Lifetime => {
            json!({"version":1,"sourceState":"ready","lastSuccess":{"futureNative":44}})
        }
    }
}

#[test]
#[serial_test::serial]
fn hermes_lifetime_metadata_usage_and_analytics_parts_never_erase_each_other() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let conn = db.conn.lock().unwrap();
    let path = "fixture-lifetime-native-db";
    let parts = [
        Part::Metadata,
        Part::UsageSync,
        Part::Analytics,
        Part::Lifetime,
    ];
    for part in parts {
        let before = source_cache::read_on(&conn, path, "hermes").unwrap();
        source_cache::merge_on(
            &conn,
            path,
            "hermes",
            None,
            part,
            &before.token(part),
            Some(initial_part(part)),
        )
        .unwrap();
    }
    for part in parts {
        let before = source_cache::read_on(&conn, path, "hermes").unwrap();
        source_cache::merge_on(
            &conn,
            path,
            "hermes",
            None,
            part,
            &before.token(part),
            Some(json!({"newField":true})),
        )
        .unwrap();
    }
    let after = source_cache::read_on(&conn, path, "hermes").unwrap();
    for part in parts {
        let value = after.value(part).unwrap();
        assert_eq!(value["newField"], true);
        for (key, expected) in initial_part(part).as_object().unwrap() {
            assert_eq!(&value[key], expected);
        }
    }
}

#[test]
#[serial_test::serial]
fn hermes_future_lifetime_is_opaque_to_peers_but_refuses_owned_writes_and_tombstones() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let conn = db.conn.lock().unwrap();
    let path = "fixture-future-native-db";
    let future = json!({"formatVersion":1,"futureEnvelope":[1,2],"parts":{
        "lifetimeUsage":{"revision":"future-revision","value":{"version":2,"privateFuture":"preserve"}}
    }});
    conn.execute("INSERT INTO session_meta_cache(source_path,provider_id,meta_json,parser_version,updated_at) VALUES(?1,'hermes',?2,0,0)", params![path, future.to_string()]).unwrap();
    for part in [Part::Metadata, Part::UsageSync, Part::Analytics] {
        let before = source_cache::read_on(&conn, path, "hermes").unwrap();
        source_cache::merge_on(
            &conn,
            path,
            "hermes",
            None,
            part,
            &before.token(part),
            Some(initial_part(part)),
        )
        .unwrap();
    }
    let before = source_cache::read_on(&conn, path, "hermes").unwrap();
    assert_eq!(before.value(Part::Lifetime).unwrap()["version"], 2);
    assert!(source_cache::merge_on(
        &conn,
        path,
        "hermes",
        None,
        Part::Lifetime,
        &before.token(Part::Lifetime),
        Some(json!({"version":1}))
    )
    .is_err());
    assert!(source_cache::merge_on(
        &conn,
        path,
        "hermes",
        None,
        Part::Lifetime,
        &before.token(Part::Lifetime),
        None
    )
    .is_err());
    let stored: String = conn
        .query_row(
            "SELECT meta_json FROM session_meta_cache WHERE source_path=?1",
            [path],
            |r| r.get(0),
        )
        .unwrap();
    let stored: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(stored["futureEnvelope"], future["futureEnvelope"]);
    assert_eq!(
        stored["parts"]["lifetimeUsage"],
        future["parts"]["lifetimeUsage"]
    );
}

#[test]
#[serial_test::serial]
fn hermes_lifetime_tombstone_prevents_absent_aba() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let conn = db.conn.lock().unwrap();
    let path = "fixture-aba-native-db";
    source_cache::merge_on(
        &conn,
        path,
        "hermes",
        None,
        Part::Lifetime,
        &None,
        Some(initial_part(Part::Lifetime)),
    )
    .unwrap();
    let live = source_cache::read_on(&conn, path, "hermes").unwrap();
    source_cache::merge_on(
        &conn,
        path,
        "hermes",
        None,
        Part::Lifetime,
        &live.token(Part::Lifetime),
        None,
    )
    .unwrap();
    assert!(source_cache::merge_on(
        &conn,
        path,
        "hermes",
        None,
        Part::Lifetime,
        &None,
        Some(initial_part(Part::Lifetime))
    )
    .is_err());
    let tombstone = source_cache::read_on(&conn, path, "hermes").unwrap();
    assert!(tombstone.token(Part::Lifetime).is_some());
    assert!(tombstone.value(Part::Lifetime).is_none());
}

#[test]
#[serial_test::serial]
fn hermes_counter_coverage_is_independent_of_historical_models_and_current_prices() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    let db = Database::memory().unwrap();
    let summary = hermes_summary(&db, &root, "ofox.ai", 10).unwrap();
    assert_eq!(summary.source_state, "ready");
    assert_eq!(summary.counter_coverage, LifetimeCoverage::Complete);
    assert_eq!(summary.model_coverage, LifetimeCoverage::Partial);
    assert_eq!(summary.estimate.availability, "unknown");
    assert!(summary
        .reasons
        .iter()
        .any(|r| r == "historicalModelAttributionUnverified"));
    assert!(summary
        .by_model
        .iter()
        .any(|m| m.fact.model_id.as_deref() == Some("main-model")));
}

#[test]
#[serial_test::serial]
fn hermes_unknown_task_lowers_counter_coverage_even_when_known_models_have_prices() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    Connection::open(root.join("state.db"))
        .unwrap()
        .execute(
            "UPDATE session_model_usage SET task='unknown_future_task' WHERE task='vision'",
            [],
        )
        .unwrap();
    let db = Database::memory().unwrap();
    priced_models(&db, "0.001", 10);
    let summary = hermes_summary(&db, &root, "ofox.ai", 20).unwrap();
    assert_eq!(summary.source_state, "ready");
    assert_eq!(summary.counter_coverage, LifetimeCoverage::Partial);
    assert_eq!(summary.model_coverage, LifetimeCoverage::Partial);
    assert_eq!(summary.totals.amount_kind, LifetimeAmountKind::LowerBound);
    assert_eq!(summary.estimate.availability, "partial");
    assert_eq!(summary.estimate.known_cost_usd.as_deref(), Some("0.11"));
}

#[test]
#[serial_test::serial]
fn hermes_last_success_is_machine_local_and_survives_database_reopen() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    let db = Database::init().unwrap();
    let success = hermes_summary(&db, &root, "ofox.ai", 10).unwrap();
    assert_eq!(success.totals.input_tokens, Some(130));
    drop(db);
    fs::remove_dir_all(&root).unwrap();
    let reopened = Database::init().unwrap();
    let stale = hermes_summary(&reopened, &root, "ofox.ai", 20).unwrap();
    assert_eq!(stale.source_state, "missing");
    assert!(stale.stale);
    assert_eq!(stale.last_success_at_ms, Some(10));
    assert_eq!(stale.totals.input_tokens, Some(130));
}

#[test]
#[serial_test::serial]
fn hermes_main_tokens_exclude_cache_in_both_totals_and_model_rows() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    Connection::open(root.join("state.db")).unwrap().execute_batch(
        "DELETE FROM session_model_usage WHERE task='vision';
         UPDATE sessions SET input_tokens=40,output_tokens=10,cache_read_tokens=100,cache_write_tokens=20;
         UPDATE session_model_usage SET input_tokens=40,output_tokens=10,cache_read_tokens=100,cache_write_tokens=20;",
    ).unwrap();
    let snapshot = read_lifetime(&root).unwrap();
    assert_eq!(snapshot.totals.total_tokens, Some(50));
    assert_eq!(snapshot.totals.cache_read_tokens, Some(100));
    assert_eq!(snapshot.totals.cache_write_tokens, Some(20));
    assert_eq!(snapshot.models[0].buckets.total_tokens, Some(50));
    assert_eq!(snapshot.models[0].buckets.cache_read_tokens, Some(100));
    assert_eq!(snapshot.models[0].buckets.cache_write_tokens, Some(20));
    let db = Database::memory().unwrap();
    PriceCatalog::ingest_on(
        &db.conn.lock().unwrap(),
        "ofox.ai",
        &json!({"data":[{
            "id":"main-model","pricing":{"prompt":"0.001","completion":"0.001",
            "input_cache_read":"0.002","input_cache_write":"0.003"}
        }]}),
        10,
    )
    .unwrap();
    let summary = hermes_summary(&db, &root, "ofox.ai", 20).unwrap();
    assert_eq!(summary.totals.total_tokens, Some(50));
    assert_eq!(summary.estimate.known_cost_usd.as_deref(), Some("0.31"));
}

#[test]
#[serial_test::serial]
fn hermes_unknown_cache_does_not_erase_known_main_tokens() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    Connection::open(root.join("state.db")).unwrap().execute_batch(
        "DELETE FROM session_model_usage WHERE task='vision';
         UPDATE sessions SET input_tokens=40,output_tokens=10,cache_read_tokens=NULL,cache_write_tokens=20;
         UPDATE session_model_usage SET input_tokens=40,output_tokens=10,cache_read_tokens=NULL,cache_write_tokens=20;",
    ).unwrap();
    let snapshot = read_lifetime(&root).unwrap();
    assert_eq!(snapshot.totals.total_tokens, Some(50));
    assert_eq!(snapshot.totals.cache_read_tokens, None);
    assert_eq!(snapshot.totals.amount_kind, LifetimeAmountKind::LowerBound);
    assert_eq!(snapshot.models[0].buckets.total_tokens, Some(50));
    assert_eq!(snapshot.models[0].buckets.cache_read_tokens, None);
    assert_eq!(
        snapshot.models[0].buckets.amount_kind,
        LifetimeAmountKind::LowerBound
    );
}

#[test]
#[serial_test::serial]
fn hermes_future_stamp_fields_do_not_reparse_an_unchanged_native_source() {
    let sandbox = crate::test_sandbox::sandbox();
    let root = native_fixture(sandbox.home(), "main-aux");
    let db = Database::memory().unwrap();
    assert_eq!(
        hermes_summary(&db, &root, "ofox.ai", 10)
            .unwrap()
            .last_success_at_ms,
        Some(10)
    );
    let path = root.join("state.db").to_string_lossy().into_owned();
    {
        let conn = db.conn.lock().unwrap();
        let cached = source_cache::read_on(&conn, &path, "hermes").unwrap();
        let mut value = cached.value(Part::Lifetime).unwrap().clone();
        value["lastSuccess"]["sourceStamp"]["futurePeer"] = json!({"preserve":true});
        value["lastSuccess"]["sourceStamp"]["database"]["futureIdentity"] = json!([1, 2, 3]);
        source_cache::merge_on(
            &conn,
            &path,
            "hermes",
            None,
            Part::Lifetime,
            &cached.token(Part::Lifetime),
            Some(value),
        )
        .unwrap();
    }
    let unchanged = hermes_summary(&db, &root, "ofox.ai", 20).unwrap();
    assert_eq!(unchanged.source_state, "ready");
    assert_eq!(unchanged.last_success_at_ms, Some(10));
    let conn = db.conn.lock().unwrap();
    let after = source_cache::read_on(&conn, &path, "hermes").unwrap();
    let value = after.value(Part::Lifetime).unwrap();
    assert_eq!(
        value["lastSuccess"]["sourceStamp"]["futurePeer"]["preserve"],
        true
    );
    assert_eq!(
        value["lastSuccess"]["sourceStamp"]["database"]["futureIdentity"],
        json!([1, 2, 3])
    );
}
