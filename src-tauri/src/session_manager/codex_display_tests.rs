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
        parent_session_id: None,
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
        .contains(&"conflictingUsageReplica".into()));
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
    let input = crate::services::session_analytics_index::read_input(&db).unwrap();
    let final_gap = &input.thread_usage_coverage
        [&crate::services::session_analytics_index::key("codex", "child")];
    assert_eq!(
        final_gap.reasons,
        std::collections::BTreeSet::from(["unprovenLegacyReplica".into()]),
        "the final thread reconciliation, rather than either physical file, owns the gap"
    );
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
    assert!(final_gap
        .reasons
        .iter()
        .all(|reason| value.1.reasons.contains(reason)));
}

fn listing_projection(
    sid: &str,
    models: &[(&str, [u64; 4])],
    incomplete: &[&str],
) -> crate::services::session_analytics_index::codex_thread_usage::Projection {
    serde_json::from_value(json!({
        "version":1,
        "nativeThreadId":sid,
        "billingSessionId":sid,
        "parentSessionId":null,
        "state":"available",
        "reasons":["missingNativeCounters"],
        "models":models.iter().map(|(model,buckets)|json!({"model":model,"buckets":buckets,"records":1})).collect::<Vec<_>>(),
        "incompleteModels":incomplete,
        "factsHash":"a".repeat(64),
        "sourceDigest":"b".repeat(64)
    }))
    .unwrap()
}

fn listing_catalog() -> PriceCatalog {
    let db = Database::memory().unwrap();
    let conn = db.conn.lock().unwrap();
    PriceCatalog::ingest_on(
        &conn,
        "ofox.ai",
        &json!({"data":[
            {"id":"priced","pricing":{"prompt":"0.001","completion":"0","input_cache_read":"0","input_cache_write":"0"}},
            {"id":"free","pricing":{"prompt":"0","completion":"0","input_cache_read":"0","input_cache_write":"0"}},
            {"id":"sibling","pricing":{"prompt":"0.002","completion":"0","input_cache_read":"0","input_cache_write":"0"}}
        ]}),
        1,
    )
    .unwrap()
}

#[test]
fn native_listing_partial_counters_keep_unknown_zero_zero_plus_and_positive_partial_distinct() {
    let catalog = listing_catalog();
    for (sid, model, incomplete, availability, amount, count) in [
        ("real-zero", "free", false, "free", Some("0"), Some(0)),
        ("zero-plus", "free", true, "partial", Some("0"), None),
        ("known-plus", "priced", true, "partial", Some("0.005"), None),
        ("unknown", "missing", true, "unknown", None, Some(1)),
    ] {
        let rows = [(model, [5, 2, 0, 0])];
        let incomplete = if incomplete { vec![model] } else { vec![] };
        let projection = listing_projection(sid, &rows, &incomplete);
        let cached = BTreeMap::from([(sid.into(), projection)]);
        let listing = [meta(Path::new("unread-native-fixture"), sid)];
        let result = display_from_threads(&listing, &cached, &catalog).unwrap();
        let (usage, display) = &result[sid];
        assert_eq!(usage.as_ref().unwrap().total_tokens, 7, "{sid}");
        assert_eq!(display.state, "available", "{sid}");
        let estimate = display.estimate.as_ref().unwrap();
        assert_eq!(estimate.availability, availability, "{sid}");
        assert_eq!(estimate.known_cost_usd.as_deref(), amount, "{sid}");
        assert_eq!(estimate.unpriced_record_count, count, "{sid}");
        assert_eq!(
            display.model_estimates[model].availability, availability,
            "{sid}"
        );
        assert_eq!(display.model_estimates[model].unpriced_record_count, count);
    }
}

#[test]
fn native_listing_partial_counter_flags_apply_only_to_their_model_and_thread() {
    let catalog = listing_catalog();
    let cached = BTreeMap::from([
        (
            "partial".into(),
            listing_projection(
                "partial",
                &[("priced", [5, 2, 0, 0]), ("sibling", [7, 3, 0, 0])],
                &["priced"],
            ),
        ),
        (
            "complete".into(),
            listing_projection("complete", &[("priced", [1, 2, 0, 0])], &[]),
        ),
    ]);
    let listing = [
        meta(Path::new("partial-fixture"), "partial"),
        meta(Path::new("complete-fixture"), "complete"),
    ];
    let result = display_from_threads(&listing, &cached, &catalog).unwrap();
    let partial = &result["partial"].1;
    assert_eq!(partial.model_estimates["priced"].availability, "partial");
    assert_eq!(
        partial.model_estimates["priced"].unpriced_record_count,
        None
    );
    assert_eq!(partial.model_estimates["sibling"].availability, "known");
    assert_eq!(
        partial.model_estimates["sibling"].unpriced_record_count,
        Some(0)
    );
    let estimate = partial.estimate.as_ref().unwrap();
    assert_eq!(estimate.availability, "partial");
    assert_eq!(estimate.known_cost_usd.as_deref(), Some("0.019"));
    assert_eq!(estimate.unpriced_record_count, None);
    assert_eq!(
        partial
            .pricing_coverage
            .as_ref()
            .unwrap()
            .unpriced_record_count,
        None
    );
    // A source-wide warning cannot downgrade an unrelated model or thread.
    let complete = &result["complete"].1;
    assert_eq!(complete.model_estimates["priced"].availability, "known");
    assert_eq!(complete.estimate.as_ref().unwrap().availability, "known");
}

fn response_source(sid: &str, first_input: u64, survivor: bool) -> String {
    let usage = |input, output| json!({"input_tokens":input,"output_tokens":output,"cached_input_tokens":0,"cache_write_input_tokens":0});
    let mut rows = vec![
        json!({"timestamp":"2026-10-16T00:00:00Z","type":"session_meta","payload":{"id":sid,"session_id":"shared-billing","source":"cli","cwd":"/fixture"}}),
        json!({"timestamp":"2026-10-16T00:00:01Z","type":"turn_context","payload":{"turn_id":"turn","model":"priced"}}),
        json!({"timestamp":"2026-10-16T00:00:02Z","type":"token_usage_record","payload":{"thread_id":sid,"session_id":"shared-billing","turn_id":"turn","response_id":"r","usage":usage(first_input,10),"thread_token_usage":usage(first_input,10)}}),
    ];
    if survivor {
        rows.push(json!({"timestamp":"2026-10-16T00:00:03Z","type":"token_usage_record","payload":{"thread_id":sid,"session_id":"shared-billing","turn_id":"turn","response_id":"s","usage":usage(5,2),"thread_token_usage":usage(first_input+5,12)}}));
    }
    rows.iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[test]
#[serial_test::serial]
fn native_listing_reconciled_conflict_retains_survivor_and_marks_only_affected_thread_partial() {
    use crate::services::{
        session_analytics_api, session_analytics_index as index, usage_query::UsageQueryV1,
    };
    let sandbox = crate::test_sandbox::sandbox();
    let first = sandbox.home().join("one.jsonl");
    let second = sandbox.home().join("two.jsonl");
    let clean = sandbox.home().join("clean.jsonl");
    let originals = [
        (&first, response_source("partial-thread", 100, true)),
        (&second, response_source("partial-thread", 200, true)),
        (&clean, response_source("clean-thread", 7, false)),
    ];
    for (path, body) in &originals {
        std::fs::write(path, body).unwrap();
    }
    let db = Database::memory().unwrap();
    assert_eq!(refresh(&db, &[&first, &second, &clean]), 3);
    let input = index::read_input(&db).unwrap();
    assert_eq!(
        input.facts.len(),
        2,
        "only response r is conflicting; response s and the clean thread remain"
    );
    assert!(
        input.facts.iter().all(|fact| fact.complete),
        "a lost sibling response must not falsify the survivor's own counters"
    );
    let partial_key = index::key("codex", "partial-thread");
    assert_eq!(
        input
            .facts
            .iter()
            .filter(|fact| fact.session_key.as_deref() == Some(partial_key.as_str()))
            .map(|fact| fact.buckets)
            .collect::<Vec<_>>(),
        [[5, 2, 0, 0]]
    );
    assert!(input
        .source_reasons
        .iter()
        .any(|reason| reason.reason == "conflictingUsageReplica"));
    {
        let conn = db.conn.lock().unwrap();
        PriceCatalog::ingest_on(&conn,"ofox.ai",&json!({"data":[{"id":"priced","pricing":{"prompt":"0.001","completion":"0","input_cache_read":"0","input_cache_write":"0"}}]}),1).unwrap();
    }
    let query = UsageQueryV1 {
        version: 1,
        start_ms: Some(1792108800000),
        end_ms: 1792195200000,
        tool_ids: vec!["codex".into()],
        model_ids: vec![],
        project_ids: vec![],
        source_ids: vec![],
    };
    let analysis =
        session_analytics_api::query(&db, query, "ofox.ai", 1792195200000, &chrono::Utc).unwrap();
    assert_eq!(analysis.totals.tokens.total_tokens, 24);
    assert_eq!(analysis.coverage.state, "partial");
    let listing = [
        meta(&first, "partial-thread"),
        meta(&second, "partial-thread"),
        meta(&clean, "clean-thread"),
    ];
    let conn = db.conn.lock().unwrap();
    let display = display_on(&conn, &listing, "ofox.ai", 2).unwrap();
    let (own, partial) = &display["partial-thread"];
    assert_eq!(own.as_ref().unwrap().total_tokens, 7);
    assert_eq!(partial.state, "available");
    assert!(partial.reasons.contains(&"conflictingUsageReplica".into()));
    let estimate = partial.estimate.as_ref().unwrap();
    assert_eq!(estimate.known_cost_usd.as_deref(), Some("0.005"));
    assert_eq!(estimate.availability, "partial");
    assert_eq!(estimate.unpriced_record_count, None);
    assert_eq!(partial.model_estimates["priced"].availability, "partial");
    let (own, clean_display) = &display["clean-thread"];
    assert_eq!(own.as_ref().unwrap().total_tokens, 17);
    assert_eq!(
        clean_display.estimate.as_ref().unwrap().availability,
        "known"
    );
    assert_eq!(
        clean_display
            .estimate
            .as_ref()
            .unwrap()
            .known_cost_usd
            .as_deref(),
        Some("0.007")
    );
    assert!(!clean_display
        .reasons
        .contains(&"conflictingUsageReplica".into()));
    drop(conn);
    assert_eq!(refresh(&db, &[&first, &second, &clean]), 0);
    for (path, body) in originals {
        assert_eq!(std::fs::read_to_string(path).unwrap(), body);
    }
}

#[test]
fn native_listing_reconciled_thread_gap_keeps_zero_plus_unknown_and_unrelated_model_distinct() {
    let catalog = listing_catalog();
    for (model, availability, amount) in [
        ("priced", "partial", Some("0.005")),
        ("free", "partial", Some("0")),
        ("missing", "unknown", None),
    ] {
        let mut projection =
            listing_projection("thread", &[(model, [5, 2, 0, 0])], &["discarded-model"]);
        projection.reasons = vec!["conflictingUsageReplica".into()];
        let cached = BTreeMap::from([("thread".into(), projection)]);
        let result = display_from_threads(
            &[meta(Path::new("unread-partial-fixture"), "thread")],
            &cached,
            &catalog,
        )
        .unwrap();
        let (usage, display) = &result["thread"];
        assert_eq!(usage.as_ref().unwrap().total_tokens, 7);
        assert_eq!(
            display.estimate.as_ref().unwrap().availability,
            availability
        );
        assert_eq!(
            display.estimate.as_ref().unwrap().known_cost_usd.as_deref(),
            amount
        );
        assert_eq!(
            display.model_estimates[model].availability,
            if model == "priced" {
                "known"
            } else if model == "free" {
                "free"
            } else {
                "unknown"
            }
        );
        if amount.is_some() {
            assert_eq!(
                display.estimate.as_ref().unwrap().unpriced_record_count,
                None
            );
        }
    }
}
