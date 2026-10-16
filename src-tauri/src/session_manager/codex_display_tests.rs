use super::*;
use crate::services::session_analytics_index::{refresh_sources, SourceSpec};
use serde_json::{json, Value};
use std::path::Path;
fn meta(path: &Path, sid: &str) -> SessionMeta {
    SessionMeta {
        provider_id: "codex".into(),
        session_id: sid.into(),
        title: None,
        summary: None,
        project_dir: None,
        created_at: None,
        last_active_at: None,
        source_path: Some(path.to_string_lossy().into()),
        resume_command: None,
        title_source: None,
        message_count: None,
    }
}
fn write(path: &Path, sid: &str, billing: &str, input: u64) {
    let source = if sid == billing {
        json!("appServer")
    } else {
        json!({"subagent":{"thread_spawn":{"parent_thread_id":billing}}})
    };
    let rows = [
        json!({"timestamp":"2026-10-16T00:00:00Z","type":"session_meta","payload":{"id":sid,"session_id":billing,"source":source}}),
        json!({"type":"turn_context","payload":{"model":"gpt-fixture"}}),
        json!({"timestamp":"2026-10-16T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":0,"output_tokens":20},"last_token_usage":{"input_tokens":input,"cached_input_tokens":0,"output_tokens":20}}}}),
    ];
    std::fs::write(
        path,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
}
fn refresh(db: &Database, paths: &[&Path]) -> usize {
    refresh_sources(
        db,
        &paths
            .iter()
            .map(|path| SourceSpec {
                provider: "codex".into(),
                path: (*path).into(),
                subagent: false,
                project_file: None,
                title_file: None,
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
    .parsed
}
fn catalog(conn: &rusqlite::Connection, apex: &str, price: &str) {
    PriceCatalog::ingest_on(conn,apex,&json!({"data":[{"id":"gpt-fixture","pricing":{"prompt":price,"completion":"0","input_cache_read":"0","input_cache_write":"0"}}]}),1).unwrap();
}
#[test]
fn codex_thread_display_prices_unknown_partial_and_apex_without_native_reparse() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("child.jsonl");
    write(&path, "child", "root", 100);
    let db = Database::memory().unwrap();
    assert_eq!(refresh(&db, &[&path]), 1);
    let listing = [meta(&path, "child")];
    let conn = db.conn.lock().unwrap();
    let unknown = display_on(&conn, &listing, "ofox.ai", 2)
        .unwrap()
        .remove("child")
        .unwrap();
    assert_eq!(unknown.0.as_ref().map(|u| u.total_tokens), Some(120));
    let estimate = unknown.1.estimate.unwrap();
    assert_eq!(estimate.known_cost_usd, None);
    assert_eq!(estimate.availability, "unknown");
    catalog(&conn, "ofox.ai", "0.001");
    catalog(&conn, "ofox.io", "0.002");
    let ai = display_on(&conn, &listing, "ofox.ai", 2)
        .unwrap()
        .remove("child")
        .unwrap()
        .1;
    let io = display_on(&conn, &listing, "ofox.io", 2)
        .unwrap()
        .remove("child")
        .unwrap()
        .1;
    assert_eq!(ai.pricing_coverage.as_ref().unwrap().apex, "ofox.ai");
    assert_eq!(io.pricing_coverage.as_ref().unwrap().apex, "ofox.io");
    assert_ne!(
        ai.pricing_coverage.as_ref().unwrap().catalog_version,
        io.pricing_coverage.as_ref().unwrap().catalog_version
    );
    assert_eq!(ai.estimate.unwrap().known_cost_usd.as_deref(), Some("0.1"));
    assert_eq!(io.estimate.unwrap().known_cost_usd.as_deref(), Some("0.2"));
    // A missing completion price is a real partial amount, not fully priced.
    PriceCatalog::ingest_on(
        &conn,
        "ofox.ai",
        &json!({"data":[{"id":"gpt-fixture","pricing":{"prompt":"0.003"}}]}),
        3,
    )
    .unwrap();
    let partial = display_on(&conn, &listing, "ofox.ai", 4)
        .unwrap()
        .remove("child")
        .unwrap()
        .1;
    assert_eq!(
        partial.model_estimates["gpt-fixture"].availability,
        "partial"
    );
    assert_eq!(partial.estimate.as_ref().unwrap().availability, "partial");
    assert_eq!(
        partial.estimate.unwrap().known_cost_usd.as_deref(),
        Some("0.3")
    );
    drop(conn);
    assert_eq!(refresh(&db, &[&path]), 0);
}
#[test]
fn codex_thread_display_parent_child_replicas_conflicts_and_source_change() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root.jsonl");
    let child = dir.path().join("child.jsonl");
    let copy = dir.path().join("copy.jsonl");
    write(&root, "root", "root", 300);
    write(&child, "child", "root", 100);
    std::fs::copy(&child, &copy).unwrap();
    let db = Database::memory().unwrap();
    refresh(&db, &[&root, &child, &copy]);
    let listing = [
        meta(&root, "root"),
        meta(&child, "child"),
        meta(&copy, "child"),
    ];
    let conn = db.conn.lock().unwrap();
    let values = display_on(&conn, &listing, "ofox.ai", 2).unwrap();
    assert_eq!(values["root"].0.as_ref().unwrap().total_tokens, 320);
    assert_eq!(values["child"].0.as_ref().unwrap().total_tokens, 120);
    drop(conn);
    write(&copy, "child", "root", 200);
    refresh(&db, &[&root, &child, &copy]);
    let conn = db.conn.lock().unwrap();
    let conflict = display_on(&conn, &listing, "ofox.ai", 2)
        .unwrap()
        .remove("child")
        .unwrap();
    assert!(conflict.0.is_none());
    assert!(conflict
        .1
        .reasons
        .contains(&"conflictingThreadSources".into()));
    // One changed physical source must not serve stale own values before refresh.
    write(&root, "root", "root", 301);
    let changed = display_on(&conn, &listing, "ofox.ai", 2)
        .unwrap()
        .remove("root")
        .unwrap();
    assert!(changed.0.is_none());
    assert!(changed
        .1
        .reasons
        .contains(&"threadUsageSourceChanged".into()));
}

#[test]
fn codex_thread_display_mixed_known_unknown_models_retains_known_amount_plus() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("child.jsonl");
    write(&path, "child", "root", 100);
    let mut file = std::fs::read_to_string(&path).unwrap();
    file.push('\n');
    file.push_str(&json!({"type":"turn_context","payload":{"model":"unknown-model"}}).to_string());
    file.push('\n');
    file.push_str(&json!({"timestamp":"2026-10-16T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":0,"output_tokens":30},"last_token_usage":{"input_tokens":50,"cached_input_tokens":0,"output_tokens":10}}}}).to_string());
    std::fs::write(&path, file).unwrap();
    let db = Database::memory().unwrap();
    refresh(&db, &[&path]);
    let conn = db.conn.lock().unwrap();
    catalog(&conn, "ofox.ai", "0.001");
    let result = display_on(&conn, &[meta(&path, "child")], "ofox.ai", 2)
        .unwrap()
        .remove("child")
        .unwrap();
    assert_eq!(result.0.unwrap().total_tokens, 180);
    let estimate = result.1.estimate.unwrap();
    assert_eq!(estimate.availability, "partial");
    assert_eq!(estimate.known_cost_usd.as_deref(), Some("0.1"));
    assert_eq!(
        result.1.model_estimates["unknown-model"].known_cost_usd,
        None
    );
    assert_eq!(
        result.1.model_estimates["unknown-model"].availability,
        "unknown"
    );
}

#[test]
fn codex_thread_display_same_usage_different_native_content_is_not_replica() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("one.jsonl");
    let two = dir.path().join("two.jsonl");
    write(&one, "child", "root", 100);
    std::fs::copy(&one, &two).unwrap();
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(&two).unwrap();
    writeln!(f,"\n{}",json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"distinct local segment"}]}})).unwrap();
    drop(f);
    let db = Database::memory().unwrap();
    refresh(&db, &[&one, &two]);
    let conn = db.conn.lock().unwrap();
    let value = display_on(
        &conn,
        &[meta(&one, "child"), meta(&two, "child")],
        "ofox.ai",
        2,
    )
    .unwrap()
    .remove("child")
    .unwrap();
    assert!(
        value.0.is_none(),
        "same time/model/tokens alone cannot prove equal native histories"
    );
    assert!(value.1.reasons.contains(&"conflictingThreadSources".into()));
}
