use super::*;
use crate::services::{session_analytics_index, usage_requests};
use serde_json::json;

fn query() -> UsageQueryV1 {
    UsageQueryV1 {
        version: 1,
        start_ms: Some(0),
        end_ms: 86_400_000,
        tool_ids: vec![],
        model_ids: vec![],
        project_ids: vec![],
        source_ids: vec![],
    }
}

fn fixture() -> (crate::test_sandbox::Sandbox, Database) {
    fixture_complete(false)
}

fn fixture_complete(complete: bool) -> (crate::test_sandbox::Sandbox, Database) {
    fixture_usage(complete, [80, 10, 20, 0])
}

fn fixture_usage(complete: bool, buckets: [u64; 4]) -> (crate::test_sandbox::Sandbox, Database) {
    let sandbox = crate::test_sandbox::sandbox();
    let path = sandbox.home().join("rollout-thread.jsonl");
    let mut events = [
        json!({"type":"session_meta","timestamp":"1970-01-01T00:00:00Z","payload":{"id":"thread","session_id":"shared-billing","source":"cli","cwd":"/fixture/project"}}),
        json!({"type":"turn_context","timestamp":"1970-01-01T00:00:01Z","payload":{"turn_id":"turn","model":"native-model"}}),
        json!({"type":"token_usage_record","timestamp":"1970-01-01T00:00:02.001Z","payload":{"thread_id":"thread","session_id":"shared-billing","turn_id":"turn","response_id":"response","usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":20},"thread_token_usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":20}}}),
    ];
    for field in ["usage", "thread_token_usage"] {
        events[2]["payload"][field]["input_tokens"] = json!(buckets[0] + buckets[2] + buckets[3]);
        events[2]["payload"][field]["output_tokens"] = json!(buckets[1]);
        events[2]["payload"][field]["cached_input_tokens"] = json!(buckets[2]);
    }
    if complete {
        events[2]["payload"]["usage"]["cache_write_input_tokens"] = json!(buckets[3]);
        events[2]["payload"]["thread_token_usage"]["cache_write_input_tokens"] = json!(buckets[3]);
    }
    std::fs::write(
        &path,
        events
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let db = Database::memory().unwrap();
    session_analytics_index::refresh_sources(
        &db,
        &[session_analytics_index::SourceSpec {
            provider: "codex".into(),
            path,
            subagent: false,
            project_file: None,
            title_file: None,
        }],
    )
    .unwrap();
    let conn = db.conn.lock().unwrap();
    conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,created_at,data_source,session_id,total_cost_usd) VALUES('legacy-original','_codex_session','codex','legacy-model',999,1,0,200,2,'codex_session','shared-billing','7.77')", []).unwrap();
    conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at,data_source,total_cost_usd) VALUES('real-proxy','real-provider','codex','proxy-model',5,40,429,2,'proxy','1.25')", []).unwrap();
    conn.execute("INSERT INTO usage_daily_rollups(date,app_type,provider_id,model,request_count,input_tokens,total_cost_usd) VALUES('1970-01-01','codex','_codex_session','legacy-model',1,100,'8.88')", []).unwrap();
    drop(conn);
    (sandbox, db)
}

#[test]
#[serial_test::serial]
fn native_effective_dataset_replaces_only_imports_and_preserves_ledger() {
    let (_sandbox, db) = fixture();
    let data = read_dataset(&db, query(), None, "ofox.ai", 86_400_000, &chrono::Utc).unwrap();
    assert_eq!(
        data.atoms
            .iter()
            .map(|a| a.tokens[0] + a.tokens[1])
            .sum::<u64>(),
        95
    );
    assert!(data
        .records
        .iter()
        .all(|r| r.request_id != "legacy-original"));
    assert!(data.records.iter().any(|r| r.request_id == "real-proxy"));
    let project = session_analytics_index::project_key("/fixture/project");
    assert!(data
        .atoms
        .iter()
        .any(|a| a.at_ms == 2001 && a.project_id.as_deref() == Some(project.as_str())));
    assert_eq!(data.options.model_ids, ["native-model", "proxy-model"]);
    let source = data
        .sources
        .iter()
        .find(|source| source.tool_id == "codex" && source.source_id == "codex_session")
        .unwrap();
    let range = source.retained_detail_range.unwrap();
    assert_eq!((range.start_ms, range.end_ms), (2001, 2002));
    let saved: (i64, String) = db.conn.lock().unwrap().query_row("SELECT input_tokens,total_cost_usd FROM proxy_request_logs WHERE request_id='legacy-original'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(saved, (999, "7.77".into()));
}

#[test]
#[serial_test::serial]
fn native_effective_requests_have_no_invented_measurement_or_saved_amount() {
    let (_sandbox, db) = fixture();
    let page = usage_requests::request_page(
        &db,
        query(),
        RequestFilters::default(),
        usage_requests::Pagination {
            page: 0,
            page_size: 20,
        },
        None,
        "ofox.ai",
        86_400_000,
    )
    .unwrap();
    let native = page
        .data
        .iter()
        .find(|r| r.record.request_id.starts_with("codex_native_v1:"))
        .expect("canonical event appears in effective requests");
    let encoded = serde_json::to_value(native).unwrap();
    assert_eq!(
        encoded.get("recordedCostUsd"),
        Some(&serde_json::Value::Null)
    );
    assert!(native.observed_status_code.is_none());
    assert!(native.observed_latency_ms.is_none());
    let proxy = page
        .data
        .iter()
        .find(|r| r.record.request_id == "real-proxy")
        .unwrap();
    assert_eq!(proxy.observed_status_code, Some(429));
    assert_eq!(proxy.observed_latency_ms, Some(40));
    let encoded = serde_json::to_value(&page).unwrap();
    assert_eq!(encoded["methodVersion"], "magpie-native-v1");
    assert_eq!(encoded["sourceCoverage"]["legacyHistoryAvailable"], true);
    let csv = usage_requests::export_csv(&page.snapshot_id, None).unwrap();
    let native_line = csv
        .csv
        .lines()
        .find(|l| l.contains("codex_native_v1:"))
        .unwrap();
    assert_eq!(native_line.split(',').nth(9), Some("\"\""));
}

#[test]
#[serial_test::serial]
fn native_saved_history_is_original_codex_only_and_cannot_reuse_effective_snapshot() {
    let (_sandbox, db) = fixture();
    let audit: RequestFilters = serde_json::from_value(json!({"recordBasis":"savedHistory"}))
        .expect("saved-history basis is supported");
    let first = usage_requests::request_page(
        &db,
        query(),
        audit.clone(),
        usage_requests::Pagination {
            page: 0,
            page_size: 20,
        },
        None,
        "ofox.ai",
        86_400_000,
    )
    .unwrap();
    assert_eq!(first.data.len(), 1);
    assert_eq!(first.data[0].record.request_id, "legacy-original");
    assert_eq!(
        serde_json::to_value(&first.data[0]).unwrap()["recordedCostUsd"],
        "7.77"
    );
    assert!(usage_requests::request_page(
        &db,
        query(),
        RequestFilters::default(),
        usage_requests::Pagination {
            page: 0,
            page_size: 20
        },
        Some(&first.snapshot_id),
        "ofox.ai",
        86_400_000
    )
    .is_err());
}

#[test]
#[serial_test::serial]
fn native_effective_millisecond_window_uses_fact_time_without_truncation() {
    let (_sandbox, db) = fixture();
    let mut q = query();
    q.start_ms = Some(2001);
    q.end_ms = 2002;
    let data = read_dataset(&db, q.clone(), None, "ofox.ai", 86_400_000, &chrono::Utc).unwrap();
    assert_eq!(data.atoms.len(), 1);
    assert_eq!(data.atoms[0].at_ms, 2001);
    q.end_ms = 2001;
    assert!(
        read_dataset(&db, q, None, "ofox.ai", 86_400_000, &chrono::Utc)
            .unwrap()
            .atoms
            .is_empty()
    );
}

#[test]
#[serial_test::serial]
fn native_old_imports_do_not_exhaust_effective_row_budget() {
    let (_sandbox, db) = fixture();
    db.conn.lock().unwrap().execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100001) INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at,data_source) SELECT 'old-'||x,'_codex_session','codex','legacy-model',1,0,200,2,'codex_session' FROM n", []).unwrap();
    let data = read_dataset(&db, query(), None, "ofox.ai", 86_400_000, &chrono::Utc)
        .expect("excluded old imports never fill the effective query budget");
    assert_eq!(data.records.len(), 2);
}

#[test]
#[serial_test::serial]
fn native_effective_project_filters_use_native_thread_and_exact_source() {
    let (_sandbox, db) = fixture();
    let mut q = query();
    q.project_ids = vec![session_analytics_index::project_key("/fixture/project")];
    q.source_ids = vec!["codex_session".into()];
    q.model_ids = vec!["native-model".into()];
    let data = read_dataset(&db, q, None, "ofox.ai", 86_400_000, &chrono::Utc).unwrap();
    assert_eq!(data.records.len(), 1);
    assert!(data.records[0].request_id.starts_with("codex_native_v1:"));
    assert_eq!(data.atoms[0].tokens, [80, 10, 20, 0]);
}

#[test]
#[serial_test::serial]
fn native_estimates_distinguish_missing_buckets_zero_and_unknown_price() {
    for complete in [false, true] {
        let (_sandbox, db) = fixture_complete(complete);
        PriceCatalog::ingest_on(&db.conn.lock().unwrap(), "ofox.ai", &json!({"data":[{"id":"native-model","pricing":{"prompt":"0","completion":"0","input_cache_read":"0","input_cache_write":"0"}}]}), 86_400_000).unwrap();
        let page = usage_requests::request_page(
            &db,
            query(),
            RequestFilters::default(),
            usage_requests::Pagination {
                page: 0,
                page_size: 20,
            },
            None,
            "ofox.ai",
            86_400_000,
        )
        .unwrap();
        let native = page
            .data
            .iter()
            .find(|row| row.record.request_id.starts_with("codex_native_v1:"))
            .unwrap();
        assert_eq!(native.current_estimate.known_cost_usd.as_deref(), Some("0"));
        assert_eq!(
            native.current_estimate.availability,
            if complete { "free" } else { "partial" }
        );
        assert!(
            native.current_estimate.unpriced_model_ids.is_empty(),
            "missing token buckets do not invent an unknown model price"
        );
        if !complete {
            assert_eq!(native.current_estimate.unpriced_record_count, None);
        }
        let proxy = page
            .data
            .iter()
            .find(|row| row.record.request_id == "real-proxy")
            .unwrap();
        assert_eq!(proxy.current_estimate.known_cost_usd, None);
        assert_eq!(proxy.current_estimate.availability, "unknown");
        assert_eq!(
            page.summary.estimate.as_ref().unwrap().availability,
            "partial"
        );
    }
}

#[test]
#[serial_test::serial]
fn native_u64_tokens_remain_exact_in_overview_requests_and_csv() {
    let large = 4_294_967_296_u64;
    let buckets = [large, large + 1, large + 2, large + 3];
    let (_sandbox, db) = fixture_usage(true, buckets);
    let mut q = query();
    q.source_ids = vec!["codex_session".into()];
    let overview = crate::services::usage_analytics::build_query_overview(
        &db,
        crate::services::usage_overview::UsagePeriod::All,
        q.clone(),
        "ofox.ai",
        86_400_000,
        &chrono::Utc,
        None,
    )
    .unwrap();
    assert_eq!(overview.totals.input_tokens, buckets[0]);
    assert_eq!(overview.totals.output_tokens, buckets[1]);
    assert_eq!(overview.totals.cache_read_tokens, buckets[2]);
    assert_eq!(overview.totals.cache_write_tokens, buckets[3]);
    assert_eq!(overview.totals.total_tokens, large * 2 + 1);
    let page = usage_requests::request_page(
        &db,
        q,
        RequestFilters::default(),
        usage_requests::Pagination {
            page: 0,
            page_size: 20,
        },
        None,
        "ofox.ai",
        86_400_000,
    )
    .unwrap();
    assert_eq!(page.data.len(), 1);
    let record = &page.data[0].record;
    assert_eq!(
        [
            record.input_tokens,
            record.output_tokens,
            record.cache_read_tokens,
            record.cache_creation_tokens
        ],
        buckets
    );
    assert_eq!(page.summary.total_tokens, overview.totals.total_tokens);
    assert_eq!(
        page.summary.cache_read_tokens,
        overview.totals.cache_read_tokens
    );
    assert_eq!(
        page.summary.cache_write_tokens,
        overview.totals.cache_write_tokens
    );
    let csv = usage_requests::export_csv(&page.snapshot_id, None).unwrap();
    let native_line = csv
        .csv
        .lines()
        .find(|line| line.contains("codex_native_v1:"))
        .unwrap();
    for value in buckets {
        assert!(
            native_line.contains(&format!("\"{value}\"")),
            "CSV must retain {value} exactly"
        );
    }
}

#[test]
fn native_request_basis_is_normalized_and_unknown_basis_is_rejected() {
    let default = RequestFilters::default().normalized().unwrap();
    let effective: RequestFilters =
        serde_json::from_value(json!({"recordBasis":"effective"})).unwrap();
    assert_eq!(default, effective.normalized().unwrap());
    let invalid: RequestFilters = serde_json::from_value(json!({"recordBasis":"other"})).unwrap();
    assert!(invalid.normalized().is_err());
}

#[test]
#[serial_test::serial]
fn native_import_shadowing_uses_source_evidence_not_provider_name() {
    let (_sandbox, db) = fixture();
    let conn = db.conn.lock().unwrap();
    for provider in ["_session", "retired-provider"] {
        conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at,data_source,total_cost_usd) VALUES(?1,?2,'codex','legacy-other-provider',1000,0,200,2,'codex_session','0.000007770000')", rusqlite::params![format!("import-{provider}"),provider]).unwrap();
    }
    conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,latency_ms,status_code,created_at,data_source,total_cost_usd) VALUES('real-http-alias','_session','codex','real-http-alias-model',3,5,200,2,'proxy','1.2300e-7')",[]).unwrap();
    drop(conn);
    let data = read_dataset(&db, query(), None, "ofox.ai", 86_400_000, &chrono::Utc).unwrap();
    assert_eq!(data.atoms.iter().map(|atom| atom.tokens[0]+atom.tokens[1]).sum::<u64>(),98,"all proven native Codex imports are replaced, but real HTTP records keep their contribution even with a reserved-looking provider");
    assert!(!data
        .options
        .model_ids
        .contains(&"legacy-other-provider".into()));
    let audit: RequestFilters =
        serde_json::from_value(json!({"recordBasis":"savedHistory"})).unwrap();
    let data = read_dataset(
        &db,
        query(),
        Some(&audit),
        "ofox.ai",
        86_400_000,
        &chrono::Utc,
    )
    .unwrap();
    assert_eq!(data.records.len(), 3);
    assert!(data
        .records
        .iter()
        .all(|row| row.data_source.as_deref() == Some("codex_session")));
    assert_eq!(
        data.record_evidence["import-_session"]
            .recorded_cost_usd
            .as_deref(),
        Some("0.000007770000")
    );
}

#[test]
#[serial_test::serial]
fn native_audit_available_for_imported_history_under_other_provider() {
    let (_sandbox, db) = fixture();
    let conn = db.conn.lock().unwrap();
    conn.execute("DELETE FROM usage_daily_rollups", []).unwrap();
    conn.execute(
        "UPDATE proxy_request_logs SET provider_id='_session' WHERE request_id='legacy-original'",
        [],
    )
    .unwrap();
    drop(conn);
    let data = read_dataset(&db, query(), None, "ofox.ai", 86_400_000, &chrono::Utc).unwrap();
    assert!(
        data.source_coverage.legacy_history_available,
        "source-proven old Codex imports remain discoverable regardless of provider alias"
    );
}

#[test]
#[serial_test::serial]
fn native_rollup_replacement_requires_evidenced_codex_import_provider() {
    let (_sandbox, db) = fixture();
    db.conn.lock().unwrap().execute("INSERT INTO usage_daily_rollups(date,app_type,provider_id,model,request_count,input_tokens,total_cost_usd) VALUES('1970-01-01','codex','_session','unproven-session-provider',1,7,'0'),('1970-01-01','codex','unrelated-provider','other-rollup',1,8,'0')",[]).unwrap();
    let data = read_dataset(&db, query(), None, "ofox.ai", 86_400_000, &chrono::Utc).unwrap();
    assert_eq!(
        data.atoms
            .iter()
            .map(|atom| atom.tokens[0] + atom.tokens[1])
            .sum::<u64>(),
        110
    );
    for model in ["unproven-session-provider", "other-rollup"] {
        assert!(
            data.atoms
                .iter()
                .any(|atom| atom.archived && atom.model == model),
            "without data_source evidence an unknown rollup provider must not be silently replaced"
        );
        assert!(data.options.model_ids.contains(&model.into()));
    }
}
