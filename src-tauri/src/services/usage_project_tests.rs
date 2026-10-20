//! The same native project condition must constrain overview, frozen requests
//! and CSV without treating a coincident proxy SID or an old rollup as proof.
use super::{
    session_analytics_index::{self, SourceSpec},
    usage_analytics::build_query_overview,
    usage_data::RequestFilters,
    usage_overview::UsagePeriod,
    usage_query::UsageQueryV1,
    usage_requests::{self, Pagination},
};
use crate::database::Database;
use rusqlite::params;
use serde_json::{json, Value};
use std::path::Path;

const AT: i64 = 1_792_022_400_000;
fn query(projects: Vec<String>) -> UsageQueryV1 {
    UsageQueryV1 {
        version: 1,
        start_ms: Some(AT),
        end_ms: AT + 86_400_000,
        tool_ids: vec![],
        model_ids: vec![],
        project_ids: projects,
        source_ids: vec![],
    }
}
fn write(path: &Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("{value}\n")).unwrap();
}
fn spec(root: &Path, tool: &str, sid: &str, project: Option<&str>) -> SourceSpec {
    let path = root.join(format!("{tool}-{sid}.jsonl"));
    let mut spec = SourceSpec {
        provider: tool.into(),
        path: path.clone(),
        subagent: false,
        project_file: None,
        title_file: None,
    };
    match tool {
        "claude" => write(
            &path,
            &json!({"type":"assistant","sessionId":sid,"cwd":project,"uuid":"m","timestamp":"2026-10-15T00:00:00Z","message":{"id":"m","role":"assistant","model":"m","content":"answer","usage":{"input_tokens":10,"output_tokens":2}}}),
        ),
        "codex" => {
            let input = if sid == "other" { 20 } else { 10 };
            std::fs::write(&path, [json!({"type":"session_meta","payload":{"id":sid,"cwd":project,"source":"cli"}}),json!({"type":"turn_context","payload":{"model":"m"}}),json!({"type":"event_msg","timestamp":"2026-10-15T00:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"output_tokens":2,"cached_input_tokens":0,"cache_write_input_tokens":0}}}})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        }
        "gemini" => {
            write(
                &path,
                &json!({"sessionId":sid,"messages":[{"id":"m","type":"gemini","model":"m","timestamp":"2026-10-15T00:00:00Z","content":"answer","tokens":{"input":10,"output":2}}]}),
            );
            let side = root.join(format!("{sid}.project"));
            std::fs::write(&side, project.unwrap_or("")).unwrap();
            spec.project_file = Some(side);
        }
        "opencode" => {
            spec.path = root.join(format!("oc-{sid}"));
            write(
                &spec.path.join(format!("session/project/{sid}.json")),
                &json!({"id":sid,"directory":project,"title":"answer"}),
            );
            write(
                &spec.path.join(format!("message/{sid}/m.json")),
                &json!({"id":"m","sessionID":sid,"role":"assistant","modelID":"m","tokens":{"input":10,"output":2},"time":{"created":AT}}),
            );
        }
        _ => panic!("test source"),
    }
    spec
}
fn row(db: &Database, id: &str, tool: &str, sid: Option<&str>, tokens: i64, proxy: bool) {
    let (provider, source) = match tool {
        "claude" => ("_session", "session_log"),
        "codex" => ("_codex_session", "codex_session"),
        "gemini" => ("_gemini_session", "gemini_session"),
        "opencode" => ("_opencode_session", "opencode_session"),
        _ => panic!("test source"),
    };
    db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES(?1,?2,?3,'m',?4,2,0,200,?5,?6,?7)", params![id, if proxy {"user-provider"} else {provider},tool,tokens,sid,AT/1000,if proxy {"proxy"} else {source}]).unwrap();
}
fn overview(db: &Database, q: UsageQueryV1) -> Value {
    serde_json::to_value(
        build_query_overview(
            db,
            UsagePeriod::All,
            q,
            "ofox.ai",
            AT + 86_400_000,
            &chrono::Utc,
            None,
        )
        .unwrap(),
    )
    .unwrap()
}
fn shared_project(tool: &str) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::memory().unwrap();
    let a = spec(dir.path(), tool, "same-native-sid", Some("/one/project"));
    let b = spec(dir.path(), tool, "other", Some("/two/project"));
    assert_eq!(
        session_analytics_index::refresh_sources(&db, &[a, b])
            .unwrap()
            .parsed,
        2
    );
    row(&db, "a", tool, Some("same-native-sid"), 10, false);
    row(&db, "b", tool, Some("other"), 20, false);
    row(
        &db,
        "coincident-proxy",
        tool,
        Some("same-native-sid"),
        100,
        true,
    );
    let q = query(vec![session_analytics_index::project_key("/one/project")]);
    let all = overview(&db, query(vec![]));
    assert_eq!(all["totals"]["totalTokens"], 136);
    let filtered = overview(&db, q.clone());
    assert_eq!(filtered["totals"]["totalTokens"], 12);
    assert_eq!(filtered["appliedQuery"]["projectIds"], json!(q.project_ids));
    assert_eq!(filtered["byProject"][0]["totalTokens"], 12);
    assert!(filtered["coverage"]["reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("projectAttributionIncomplete")));
    let page = usage_requests::request_page(
        &db,
        q,
        RequestFilters::default(),
        Pagination {
            page: 0,
            page_size: 20,
        },
        None,
        "ofox.ai",
        AT + 86_400_000,
    )
    .unwrap();
    assert_eq!(page.total, 1);
    if tool == "codex" {
        assert!(page.data[0]
            .record
            .request_id
            .starts_with("codex_native_v1:"));
        assert!(page.data[0].recorded_cost_usd.is_none());
    } else {
        assert_eq!(page.data[0].record.request_id, "a");
    }
    assert_eq!(page.summary.total_tokens, 12);
    let csv = usage_requests::export_csv(&page.snapshot_id, None).unwrap();
    assert_eq!(csv.record_count, 1);
    assert!(!csv.csv.contains("coincident-proxy"));
}
#[test]
fn projects_claude_condition_is_shared_by_overview_requests_and_csv() {
    shared_project("claude")
}
#[test]
fn projects_codex_condition_is_shared_by_overview_requests_and_csv() {
    shared_project("codex")
}
#[test]
fn projects_gemini_condition_is_shared_by_overview_requests_and_csv() {
    shared_project("gemini")
}
#[test]
fn projects_opencode_condition_is_shared_by_overview_requests_and_csv() {
    shared_project("opencode")
}

#[test]
fn projects_unknown_is_not_unparsed_or_a_coincident_proxy_identity() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::memory().unwrap();
    let a = spec(dir.path(), "claude", "unknown", None);
    session_analytics_index::refresh_sources(&db, &[a]).unwrap();
    row(&db, "unknown", "claude", Some("unknown"), 10, false);
    row(&db, "unparsed", "claude", Some("not-indexed"), 20, false);
    row(&db, "unassociated", "claude", None, 30, false);
    row(&db, "proxy", "claude", Some("unknown"), 40, true);
    let result = overview(
        &db,
        query(vec![
            super::session_analytics_core::UNKNOWN_PROJECT_ID.into()
        ]),
    );
    assert_eq!(result["totals"]["totalTokens"], 12);
    assert_eq!(result["attribution"]["sessionCount"], 1);
    assert_eq!(result["attribution"]["projectCount"], 0);
}

#[test]
fn projects_overview_keeps_unattributed_and_legacy_usage_in_totals_but_not_project_counts() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::memory().unwrap();
    let a = spec(dir.path(), "claude", "s", Some("/one/project"));
    session_analytics_index::refresh_sources(&db, &[a]).unwrap();
    row(&db, "known", "claude", Some("s"), 10, false);
    row(&db, "unknown", "claude", None, 20, false);
    db.conn.lock().unwrap().execute("INSERT INTO usage_daily_rollups(date,app_type,provider_id,model,request_count,input_tokens,output_tokens,total_cost_usd) VALUES('2026-10-15','claude','_session','m',2,30,4,'0')",[]).unwrap();
    let all = overview(&db, query(vec![]));
    assert_eq!(all["totals"]["totalTokens"], 68);
    assert_eq!(all["attribution"]["sessionCount"], 1);
    assert_eq!(all["attribution"]["projectCount"], 1);
    assert_eq!(all["attribution"]["unassociatedRecords"], 3);
    assert_eq!(all["attribution"]["partial"], true);
    let filtered = overview(
        &db,
        query(vec![session_analytics_index::project_key("/one/project")]),
    );
    assert_eq!(filtered["totals"]["totalTokens"], 12);
    assert!(filtered["coverage"]["reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("legacyProjectUnavailable")));
    assert_eq!(filtered["coverage"]["excludedRollupDays"], 1);
}

#[test]
fn projects_request_snapshot_keeps_frozen_project_membership_after_metadata_changes() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::memory().unwrap();
    let a = spec(dir.path(), "claude", "s", Some("/before/project"));
    session_analytics_index::refresh_sources(&db, &[a]).unwrap();
    row(&db, "a", "claude", Some("s"), 10, false);
    let q = query(vec![session_analytics_index::project_key(
        "/before/project",
    )]);
    let page = usage_requests::request_page(
        &db,
        q.clone(),
        RequestFilters::default(),
        Pagination {
            page: 0,
            page_size: 20,
        },
        None,
        "ofox.ai",
        AT + 1,
    )
    .unwrap();
    let changed = spec(dir.path(), "claude", "s", Some("/after/project"));
    session_analytics_index::refresh_sources(&db, &[changed]).unwrap();
    let old = usage_requests::request_page(
        &db,
        q.clone(),
        RequestFilters::default(),
        Pagination {
            page: 0,
            page_size: 20,
        },
        Some(&page.snapshot_id),
        "ofox.ai",
        AT + 2,
    )
    .unwrap();
    assert_eq!(old.total, 1);
    assert_eq!(old.receipt.snapshot_version, page.receipt.snapshot_version);
    let new = usage_requests::request_page(
        &db,
        q,
        RequestFilters::default(),
        Pagination {
            page: 0,
            page_size: 20,
        },
        None,
        "ofox.ai",
        AT + 2,
    )
    .unwrap();
    assert_eq!(new.total, 0);
    assert_ne!(new.receipt.snapshot_version, page.receipt.snapshot_version);
}

#[test]
fn projects_verified_parent_and_child_use_one_root_session_and_the_root_project() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::memory().unwrap();
    let source = spec(dir.path(), "opencode", "root", Some("/root/project"));
    write(
        &source.path.join("session/project/child.json"),
        &json!({"id":"child","parentID":"root","directory":"/child/project","title":"child"}),
    );
    write(
        &source.path.join("message/child/cm.json"),
        &json!({"id":"cm","sessionID":"child","role":"assistant","modelID":"m","tokens":{"input":20,"output":2},"time":{"created":AT}}),
    );
    session_analytics_index::refresh_sources(&db, &[source]).unwrap();
    // Older imports can retain the child SID; current OpenCode imports can
    // already carry the verified root SID. Both resolve to the same project.
    row(&db, "root-call", "opencode", Some("root"), 10, false);
    row(&db, "child-call", "opencode", Some("child"), 20, false);
    let value = overview(
        &db,
        query(vec![session_analytics_index::project_key("/root/project")]),
    );
    assert_eq!(value["totals"]["totalTokens"], 34);
    assert_eq!(value["attribution"]["sessionCount"], 1);
    assert_eq!(value["attribution"]["projectCount"], 1);
    assert_eq!(value["byProject"].as_array().unwrap().len(), 1);
}

#[test]
fn projects_missing_native_parent_is_unparsed_instead_of_an_unknown_directory() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::memory().unwrap();
    let source = spec(dir.path(), "opencode", "child", None);
    write(
        &source.path.join("session/project/child.json"),
        &json!({"id":"child","parentID":"missing-parent","title":"child"}),
    );
    session_analytics_index::refresh_sources(&db, &[source]).unwrap();
    row(&db, "child-call", "opencode", Some("child"), 20, false);
    let value = overview(
        &db,
        query(vec![
            super::session_analytics_core::UNKNOWN_PROJECT_ID.into()
        ]),
    );
    assert_eq!(value["totals"]["requests"], 0);
    assert!(value["coverage"]["reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("projectAttributionIncomplete")));
}
