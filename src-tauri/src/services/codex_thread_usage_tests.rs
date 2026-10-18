use super::*;

fn source(path: &Path) -> SourceSpec {
    SourceSpec {
        provider: "codex".into(),
        path: path.into(),
        subagent: false,
        project_file: None,
        title_file: None,
    }
}
fn native(path: &Path, extra: &[Value], usage: bool) {
    let mut rows = vec![
        json!({"timestamp":"2026-10-16T00:00:00Z","type":"session_meta","payload":{"id":"child","session_id":"root","cwd":"/fixture","source":{"subagent":{"thread_spawn":{"parent_thread_id":"root"}}}}}),
    ];
    rows.extend_from_slice(extra);
    rows.push(json!({"type":"turn_context","payload":{"model":"gpt-fixture"}}));
    if usage {
        rows.push(json!({"timestamp":"2026-10-16T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":20},"last_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":20}}}}));
    }
    std::fs::write(
        path,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
}
fn projection(db: &Database, path: &Path) -> Value {
    let conn = db.conn.lock().unwrap();
    let raw: String = conn
        .query_row(
            "SELECT meta_json FROM session_meta_cache WHERE source_path=?1",
            [path.to_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str::<Value>(&raw).unwrap()["parts"]["analytics"]["value"]["threadUsage"]
        .clone()
}
#[test]
fn codex_thread_display_single_native_id_keeps_legacy_root_ledger_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("child.jsonl");
    native(&path, &[], true);
    let db = Database::memory().unwrap();
    db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES('codex_session:root:old:1','_session','codex','gpt-fixture',900,100,0,200,'root',1,'codex_session')",[]).unwrap();
    refresh_sources(&db, &[source(&path)]).unwrap();
    let value = projection(&db, &path);
    assert_eq!(
        value["state"], "available",
        "single thread evidence must not be lost behind its billing/root session ID"
    );
    assert_eq!(value["nativeThreadId"], "child");
    assert_eq!(value["billingSessionId"], "root");
    assert_eq!(value["models"][0]["buckets"], json!([40, 20, 60, 0]));
    assert_eq!(db.conn.lock().unwrap().query_row("SELECT input_tokens FROM proxy_request_logs WHERE request_id='codex_session:root:old:1'",[],|r|r.get::<_,i64>(0)).unwrap(),900);
    assert_eq!(refresh_sources(&db, &[source(&path)]).unwrap().parsed, 0);
}
#[test]
fn codex_thread_display_multiple_headers_never_attributes_copied_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("child.jsonl");
    native(
        &path,
        &[
            json!({"type":"session_meta","payload":{"id":"root","cwd":"/fixture","source":"vscode"}}),
        ],
        true,
    );
    let db = Database::memory().unwrap();
    refresh_sources(&db, &[source(&path)]).unwrap();
    let value = projection(&db, &path);
    assert_eq!(value["state"], "unavailable");
    assert!(value["reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("unprovenInheritedBoundary")));
    assert!(
        read_input(&db).unwrap().facts.is_empty(),
        "existing analytics rejection must remain intact"
    );
}
#[test]
fn codex_thread_display_single_header_fork_stays_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("child.jsonl");
    native(&path, &[], true);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replace(
            "\"id\":\"child\"",
            "\"id\":\"child\",\"forked_from_id\":\"parent\"",
        ),
    )
    .unwrap();
    let db = Database::memory().unwrap();
    refresh_sources(&db, &[source(&path)]).unwrap();
    let value = projection(&db, &path);
    assert_eq!(value["state"], "unavailable");
    assert!(value["reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("unprovenInheritedBoundary")));
}
#[test]
fn codex_thread_display_missing_native_usage_is_not_known_zero() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("child.jsonl");
    native(&path, &[], false);
    let db = Database::memory().unwrap();
    refresh_sources(&db, &[source(&path)]).unwrap();
    let value = projection(&db, &path);
    assert_eq!(value["state"], "unavailable");
    assert!(value["reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("nativeUsageUnavailable")));
}

fn prove_rows(rows: &[Value]) -> codex_thread_usage::Projection {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("proof.jsonl");
    std::fs::write(
        &path,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let mut normalized = usage_source::read_normalized("codex", &path).unwrap();
    // Isolate proof checks from the expected missingNativeEventIdentity reason.
    normalized.reasons.remove("missingNativeEventIdentity");
    normalized.complete = normalized.reasons.is_empty();
    let mut proof = codex_thread_usage::Proof::default();
    proof.observe_source(&std::fs::read_to_string(&path).unwrap());
    for row in rows {
        proof.observe(row);
    }
    proof.finish(Some(&normalized))
}
fn proof_rows() -> Vec<Value> {
    vec![
        json!({"timestamp":"2026-10-16T00:00:00Z","type":"session_meta","payload":{"id":"child","session_id":"root","source":{"subagent":{"thread_spawn":{"parent_thread_id":"root"}}}}}),
        json!({"type":"turn_context","payload":{"model":"gpt-fixture"}}),
        json!({"timestamp":"2026-10-16T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":20},"last_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":20}}}}),
    ]
}
#[test]
fn codex_thread_display_rejects_usage_before_header_and_copied_earlier_time() {
    let mut rows = proof_rows();
    rows.swap(0, 2);
    assert_eq!(
        prove_rows(&rows).state,
        "unavailable",
        "pre-header usage has no thread ownership proof"
    );
    let mut rows = proof_rows();
    rows[2]["timestamp"] = json!("2026-10-15T23:59:59Z");
    assert_eq!(
        prove_rows(&rows).state,
        "unavailable",
        "copied history predates thread creation"
    );
}
#[test]
fn codex_thread_display_rejects_unknown_source_and_missing_header_time() {
    for source in [
        Value::Null,
        json!("unknown-new-source"),
        json!({"subagent":{"thread_spawn":{}}}),
    ] {
        let mut rows = proof_rows();
        rows[0]["payload"]["source"] = source;
        assert_eq!(prove_rows(&rows).state, "unavailable");
    }
    let mut rows = proof_rows();
    rows[0].as_object_mut().unwrap().remove("timestamp");
    assert_eq!(prove_rows(&rows).state, "unavailable");
}
#[test]
fn codex_thread_display_rejects_incomplete_last_even_when_total_is_complete() {
    let mut rows = proof_rows();
    rows[2]["payload"]["info"]["last_token_usage"] = json!({"input_tokens":20});
    assert_eq!(
        prove_rows(&rows).state,
        "unavailable",
        "Accounting consumes first last, never infer missing output/cache as zero"
    );
}
#[test]
fn codex_thread_display_accepts_recognized_app_server_and_explicit_zero() {
    let mut rows = proof_rows();
    rows[0]["payload"]["source"] = json!("appServer");
    for k in ["total_token_usage", "last_token_usage"] {
        rows[2]["payload"]["info"][k] =
            json!({"input_tokens":0,"cached_input_tokens":0,"output_tokens":0});
    }
    let value = prove_rows(&rows);
    assert_eq!(value.state, "available");
    assert_eq!(value.models[0].buckets, [0; 4]);
}
#[test]
fn codex_thread_display_warm_601_sources_reuses_decoded_projection() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::memory().unwrap();
    let mut specs = vec![];
    let mut listing = vec![];
    for i in 0..601 {
        let path = dir.path().join(format!("{i}.jsonl"));
        native(&path, &[], true);
        specs.push(source(&path));
        listing.push(crate::session_manager::SessionMeta {
            provider_id: "codex".into(),
            session_id: "child".into(),
            title: None,
            summary: None,
            project_dir: None,
            created_at: None,
            last_active_at: None,
            source_path: Some(path.to_string_lossy().into()),
            resume_command: None,
            title_source: None,
            message_count: None,
        });
    }
    refresh_sources(&db, &specs).unwrap();
    let conn = db.conn.lock().unwrap();
    let cache = ProjectionCache::default();
    thread_usage_cached_on(&conn, &listing, &cache).unwrap();
    reset_query_decode_count();
    let reads = usage_source::body_reads_under(dir.path());
    let first = thread_usage_cached_on(&conn, &listing, &cache).unwrap();
    let second = thread_usage_cached_on(&conn, &listing, &cache).unwrap();
    assert_eq!(
        usage_source::body_reads_under(dir.path()),
        reads,
        "warm display never rereads native body"
    );
    assert_eq!(first, second);
    assert_eq!(
        query_decode_count(),
        0,
        "warm listings must not decode 601 envelopes again"
    );
    drop(conn);
    assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 0);
}

#[test]
fn codex_thread_display_child_total_only_has_no_own_baseline() {
    let mut rows = proof_rows();
    rows[2]["payload"]["info"]
        .as_object_mut()
        .unwrap()
        .remove("last_token_usage");
    assert_eq!(prove_rows(&rows).state, "unavailable");
    rows[0]["payload"]["id"] = json!("root");
    rows[0]["payload"]["source"] = json!("cli");
    assert_eq!(
        prove_rows(&rows).state,
        "available",
        "root native total can provide its initial baseline"
    );
}
#[test]
fn codex_thread_display_checked_accounting_rejects_delta_overflow() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("overflow.jsonl");
    let mut rows = proof_rows();
    rows[2]["payload"]["info"]["last_token_usage"]["output_tokens"] =
        json!(u64::from(u32::MAX) + 1);
    std::fs::write(
        &path,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    assert!(
        usage_source::read_normalized("codex", &path).is_err(),
        "checked Accounting must reject truncation despite legacy identity casts"
    );
}

#[test]
fn codex_thread_display_old_cache_rebuilds_once_and_future_version_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("child.jsonl");
    native(&path, &[], true);
    let db = Database::memory().unwrap();
    let specs = [source(&path)];
    refresh_sources(&db, &specs).unwrap();
    let mutate = |edit: fn(&mut Value)| {
        let conn = db.conn.lock().unwrap();
        let raw: String = conn
            .query_row(
                "SELECT meta_json FROM session_meta_cache WHERE source_path=?1",
                [path.to_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        let mut envelope: Value = serde_json::from_str(&raw).unwrap();
        edit(&mut envelope["parts"]["analytics"]["value"]);
        conn.execute(
            "UPDATE session_meta_cache SET meta_json=?1 WHERE source_path=?2",
            rusqlite::params![envelope.to_string(), path.to_str().unwrap()],
        )
        .unwrap();
    };
    mutate(|entry| {
        entry.as_object_mut().unwrap().remove("threadUsage");
    });
    assert_eq!(
        refresh_sources(&db, &specs).unwrap().parsed,
        1,
        "pre-display cache needs one verified refresh"
    );
    assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 0);
    mutate(|entry| {
        entry["threadUsage"]["version"] = json!(99);
        entry["threadUsage"]["futureOwnedField"] = json!("preserve");
    });
    native(&path, &[], false);
    assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 0);
    assert_eq!(projection(&db, &path)["futureOwnedField"], "preserve");
}
#[test]
fn codex_thread_display_large_cumulative_small_last_uses_checked_own_delta() {
    let mut rows = proof_rows();
    let large = u64::from(u32::MAX) + 100;
    rows[2]["payload"]["info"]["total_token_usage"] =
        json!({"input_tokens":large,"cached_input_tokens":0,"output_tokens":20});
    rows[2]["payload"]["info"]["last_token_usage"] =
        json!({"input_tokens":100,"cached_input_tokens":0,"output_tokens":20});
    let result = prove_rows(&rows);
    assert_eq!(result.state, "available");
    assert_eq!(result.models[0].buckets, [100, 20, 0, 0]);
}

#[test]
fn codex_thread_display_preserves_fixed_legacy_positional_entry_reads() {
    let mut old = empty_entry(1);
    old.status = "ok".into();
    old.available = true;
    let array = json!([1, null, [], [], [], [], true, "ok", 2, 1, 1]);
    for provider in ["codex", "claude"] {
        let envelope = json!({"formatVersion":1,"parts":{"analytics":{"revision":"legacy-positional","value":array}}});
        let (_, entry) = decode_analytics_row(provider, None, None, &envelope.to_string());
        let decoded = entry
            .expect("known legacy Entry array must remain readable")
            .unwrap();
        assert_eq!(decoded.status, old.status);
        assert!(decoded.thread_usage.is_none());
    }
}
