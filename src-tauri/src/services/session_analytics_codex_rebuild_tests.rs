use super::*;
use serde_json::json;

fn setup() -> (tempfile::TempDir, Database, SourceSpec) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout-child.jsonl");
    let rows = [
        json!({"type":"session_meta","timestamp":"2026-01-01T00:00:00Z","payload":{"id":"child","session_id":"billing","source":"cli","cwd":"/fixture"}}),
        json!({"type":"session_meta","timestamp":"2025-12-31T23:00:00Z","payload":{"id":"parent","session_id":"billing","source":"cli","cwd":"/fixture"}}),
        json!({"type":"turn_context","timestamp":"2026-01-01T00:00:01Z","payload":{"turn_id":"t","model":"fixture-model"}}),
        json!({"type":"token_usage_record","timestamp":"2026-01-01T00:00:02Z","payload":{"thread_id":"child","session_id":"billing","turn_id":"t","response_id":"response-a","usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":20,"cache_write_input_tokens":0},"thread_token_usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":20,"cache_write_input_tokens":0}}}),
        json!({"type":"event_msg","timestamp":"2026-01-01T00:00:02Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":20,"cache_write_input_tokens":0},"last_token_usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":20,"cache_write_input_tokens":0}}}}),
    ];
    std::fs::write(
        &path,
        rows.iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let spec = SourceSpec {
        provider: "codex".into(),
        path,
        subagent: false,
        project_file: None,
        title_file: None,
    };
    (dir, Database::memory().unwrap(), spec)
}
fn query() -> super::super::usage_query::UsageQueryV1 {
    super::super::usage_query::UsageQueryV1 {
        version: 1,
        start_ms: Some(1767225600000),
        end_ms: 1767312000000,
        tool_ids: vec!["codex".into()],
        model_ids: vec![],
        project_ids: vec![],
        source_ids: vec![],
    }
}

#[test]
fn codex_rebuild_analytics_accepts_canonical_multihead_and_uses_native_thread() {
    let (_dir, db, spec) = setup();
    let original = std::fs::read(&spec.path).unwrap();
    refresh_sources(&db, std::slice::from_ref(&spec)).unwrap();
    let input = read_input(&db).unwrap();
    assert_eq!(input.sessions.len(), 1);
    assert_eq!(input.sessions[0].native_session_id, "child");
    assert_eq!(input.facts.len(), 1);
    assert_eq!(input.facts[0].session_key, Some(key("codex", "child")));
    assert_eq!(input.facts[0].buckets, [80, 10, 20, 0]);
    assert_eq!(std::fs::read(&spec.path).unwrap(), original);
    assert_eq!(refresh_sources(&db, &[spec]).unwrap().parsed, 0);
}

#[test]
fn codex_rebuild_effective_view_replaces_legacy_import_without_changing_saved_ledger() {
    let (_dir, db, spec) = setup();
    db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,created_at,data_source,session_id,total_cost_usd) VALUES('legacy-original','_codex_session','codex','fixture-model',999,1,0,200,1767225602,'codex_session','billing','7.77')",[]).unwrap();
    refresh_sources(&db, &[spec]).unwrap();
    let data = super::super::usage_data::read_dataset(
        &db,
        query(),
        None,
        "ofox.ai",
        1767312000000,
        &chrono::Utc,
    )
    .unwrap();
    assert_eq!(
        data.atoms
            .iter()
            .map(|a| a.tokens[0] + a.tokens[1])
            .sum::<u64>(),
        90
    );
    let saved:(i64,String,String)=db.conn.lock().unwrap().query_row("SELECT input_tokens,total_cost_usd,session_id FROM proxy_request_logs WHERE request_id='legacy-original'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(saved, (999, "7.77".into(), "billing".into()));
}

#[test]
fn codex_rebuild_failed_generation_never_publishes_a_partial_file_set() {
    let (_dir, db, spec) = setup();
    refresh_sources(&db, std::slice::from_ref(&spec)).unwrap();
    let before = read_input(&db).unwrap();
    let body = std::fs::read_to_string(&spec.path).unwrap();
    std::fs::write(&spec.path, body.replace("100", "200")).unwrap();
    let second = SourceSpec {
        path: spec.path.with_file_name("rollout-second.jsonl"),
        ..spec.clone()
    };
    std::fs::write(&second.path, body.replace("child", "second")).unwrap();
    db.conn.lock().unwrap().execute_batch("CREATE TRIGGER reject_second BEFORE INSERT ON session_meta_cache WHEN NEW.source_path LIKE '%rollout-second.jsonl' BEGIN SELECT RAISE(ABORT,'sandbox rejected'); END;").unwrap();
    let result = refresh_sources(&db, &[spec, second]);
    assert!(result.is_err() || result.unwrap().failed > 0);
    let after = read_input(&db).unwrap();
    assert_eq!(
        after.facts, before.facts,
        "no file of a failed generation may replace its accepted predecessor"
    );
}

#[test]
fn native_runtime_claude_projects_all_timed_records_without_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claude.jsonl");
    let rows = [
        json!({"type":"user","sessionId":"s","cwd":"/fixture","timestamp":"2026-01-01T00:00:00Z","uuid":"u","message":{"role":"user","content":"fake prompt"}}),
        json!({"type":"progress","sessionId":"s","timestamp":"2026-01-01T00:04:00Z","data":{"type":"tool_progress"}}),
        json!({"type":"progress","sessionId":"s","timestamp":"2026-01-01T00:08:00Z","data":{"type":"tool_progress"}}),
        json!({"type":"assistant","sessionId":"s","timestamp":"2026-01-01T00:09:00Z","uuid":"a","message":{"id":"message-a","role":"assistant","model":"fixture-model","content":[{"type":"text","text":"fake output"}],"usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
    ];
    std::fs::write(
        &path,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let db = Database::memory().unwrap();
    refresh_sources(
        &db,
        &[SourceSpec {
            provider: "claude".into(),
            path,
            subagent: false,
            project_file: None,
            title_file: None,
        }],
    )
    .unwrap();
    let input = read_input(&db).unwrap();
    assert_eq!(input.native_activity.len(), 4);
    let activity = super::super::session_native_activity::activity(
        &input.native_activity,
        &super::super::session_analytics_core::Window {
            start_ms: query().start_ms,
            end_ms: query().end_ms,
        },
        &[super::super::session_analytics_core::DayBoundary {
            date: "2026-01-01".into(),
            start_ms: 1767225600000,
            end_ms: 1767312000000,
        }],
    )
    .unwrap();
    assert_eq!(activity.active_ms, 540_000);
    assert_eq!(input.messages.len(), 2);
    assert!(!serde_json::to_string(&input.native_activity)
        .unwrap()
        .contains("fake prompt"));
}

#[test]
fn native_reconcile_legacy_equal_tokens_in_different_histories_are_not_a_copy_proof() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("one.jsonl");
    let rows = [
        json!({"type":"session_meta","timestamp":"2026-01-01T00:00:00Z","payload":{"id":"thread","source":"cli","cwd":"/fixture"}}),
        json!({"type":"turn_context","timestamp":"2026-01-01T00:00:01Z","payload":{"turn_id":"t","model":"fixture-model"}}),
        json!({"type":"response_item","timestamp":"2026-01-01T00:00:02Z","payload":{"type":"message","id":"m","role":"user","content":[{"type":"input_text","text":"different-one"}]}}),
        json!({"type":"event_msg","timestamp":"2026-01-01T00:00:03Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":0,"cache_write_input_tokens":0},"last_token_usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":0,"cache_write_input_tokens":0}}}}),
    ];
    let body = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, &body).unwrap();
    let second = path.with_file_name("two.jsonl");
    std::fs::write(&second, body.replace("different-one", "different-two")).unwrap();
    let db = Database::memory().unwrap();
    let specs = [path, second]
        .into_iter()
        .map(|path| SourceSpec {
            provider: "codex".into(),
            path,
            subagent: false,
            project_file: None,
            title_file: None,
        })
        .collect::<Vec<_>>();
    refresh_sources(&db, &specs).unwrap();
    let input = read_input(&db).unwrap();
    assert!(input
        .source_reasons
        .iter()
        .any(|r| r.reason == "unprovenLegacyReplica"));
    assert!(
        input.facts.is_empty(),
        "ambiguous old interval is unknown, not a proven duplicate nor two calls"
    );
    assert!(
        input
            .source_observations
            .iter()
            .all(|source| source.status == "partial"),
        "coverage must describe reconciled events rather than pre-reconciliation files"
    );
}

#[test]
fn native_rebuild_initial_all_unreadable_keeps_original_view_until_verified() {
    let (_dir, db, spec) = setup();
    db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,created_at,data_source,total_cost_usd) VALUES('old','_codex_session','codex','fixture-model',1000,0,0,200,1767225602,'codex_session','0')", []).unwrap();
    std::fs::write(&spec.path, "not readable JSON").unwrap();
    let result = refresh_sources(&db, &[spec]).unwrap();
    assert!(result.failed > 0);
    let input = read_input(&db).unwrap();
    assert!(
        !input.codex_native_ready,
        "a wholly failed initial rebuild must not publish an empty replacement"
    );
    let data = super::super::usage_data::read_dataset(
        &db,
        query(),
        None,
        "ofox.ai",
        1767312000000,
        &chrono::Utc,
    )
    .unwrap();
    assert_eq!(
        data.atoms
            .iter()
            .map(|a| a.tokens[0] + a.tokens[1])
            .sum::<u64>(),
        1000
    );
}

#[test]
fn native_rebuild_commit_checks_epoch_and_complete_source_roster() {
    let (dir, db, spec) = setup();
    refresh_sources(&db, std::slice::from_ref(&spec)).unwrap();
    let snapshot = read_input(&db).unwrap();
    let rows = {
        let conn = db.conn.lock().unwrap();
        rows_on(&conn).unwrap()
    };
    let roots = vec![("codex".into(), dir.path().to_path_buf())];
    let mut generation =
        rebuild::Generation::new(&db, &rows, std::slice::from_ref(&spec), &roots, false, None)
            .unwrap();
    generation
        .observed(&spec, Some(source_revision(&spec).unwrap()))
        .unwrap();
    std::fs::write(
        dir.path().join("rollout-added.jsonl"),
        std::fs::read(&spec.path).unwrap(),
    )
    .unwrap();
    assert!(generation.commit(&db).is_err());
    assert_eq!(
        read_input(&db).unwrap().snapshot_version,
        snapshot.snapshot_version
    );
    let mut generation = rebuild::Generation::new(&db, &rows, &[spec], &[], false, None).unwrap();
    db.conn.lock().unwrap().execute("INSERT OR REPLACE INTO maintenance_flags(key,value) VALUES('usage_ledger_epoch_v1','sandbox-restored')",[]).unwrap();
    assert!(
        generation.commit(&db).is_err(),
        "even a warm generation is fenced by epoch"
    );
}

#[test]
fn native_rebuild_changed_source_and_cancel_never_publish_staged_entries() {
    let (_dir, db, spec) = setup();
    refresh_sources(&db, std::slice::from_ref(&spec)).unwrap();
    let before = read_input(&db).unwrap();
    let rows = {
        let conn = db.conn.lock().unwrap();
        rows_on(&conn).unwrap()
    };
    let mut generation =
        rebuild::Generation::new(&db, &rows, std::slice::from_ref(&spec), &[], false, None)
            .unwrap();
    generation
        .observed(&spec, Some(source_revision(&spec).unwrap()))
        .unwrap();
    std::fs::write(&spec.path, "changed").unwrap();
    assert!(generation.commit(&db).is_err());
    assert_eq!(
        read_input(&db).unwrap().snapshot_version,
        before.snapshot_version
    );
    let control = super::super::usage_rebuild::Control::new(1);
    let mut generation =
        rebuild::Generation::new(&db, &rows, &[spec], &[], false, Some(control.clone())).unwrap();
    control.request_cancel();
    assert!(generation.commit(&db).is_err());
    assert_eq!(
        read_input(&db).unwrap().snapshot_version,
        before.snapshot_version
    );
}

#[test]
fn native_rebuild_warm_640_sources_reuses_decoded_projection() {
    let (dir, db, original) = setup();
    let body = std::fs::read_to_string(&original.path).unwrap();
    let specs = (0..640)
        .map(|index| {
            let path = dir.path().join(format!("rollout-{index:04}.jsonl"));
            std::fs::write(
                &path,
                body.replace("child", &format!("native-{index:04}"))
                    .replace("response-a", &format!("response-{index:04}")),
            )
            .unwrap();
            SourceSpec {
                path,
                ..original.clone()
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 640);
    let first = read_input(&db).unwrap();
    assert_eq!(first.facts.len(), 640);
    let started = Instant::now();
    for _ in 0..2 {
        let warm = refresh_sources(&db, &specs).unwrap();
        assert_eq!(warm.parsed, 0);
        assert_eq!(warm.unchanged, 640);
        assert_eq!(read_input(&db).unwrap().facts, first.facts);
    }
    eprintln!(
        "native Codex warm 640 physical sources two refreshes: {:?}; parsed=0",
        started.elapsed()
    );
}

#[test]
fn native_reconcile_conflicting_response_reports_partial_physical_origins() {
    let (_dir, db, first) = setup();
    let body = std::fs::read_to_string(&first.path)
        .unwrap()
        .lines()
        .filter(|line| !line.contains("\"id\":\"parent\""))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&first.path, &body).unwrap();
    assert!(super::super::codex_native::read(&first.path)
        .unwrap()
        .reasons
        .is_empty());
    let second = SourceSpec {
        path: first.path.with_file_name("conflict.jsonl"),
        ..first.clone()
    };
    std::fs::write(
        &second.path,
        std::fs::read_to_string(&first.path)
            .unwrap()
            .replace("100", "200")
            .replace("00:00:", "01:00:"),
    )
    .unwrap();
    refresh_sources(&db, &[first, second]).unwrap();
    let input = read_input(&db).unwrap();
    assert!(input.facts.is_empty());
    assert!(input
        .source_reasons
        .iter()
        .any(|r| r.reason == "conflictingUsageReplica"));
    assert_eq!(input.source_observations.len(), 2);
    assert!(input
        .source_observations
        .iter()
        .all(|source| source.status == "partial"));
}

#[test]
fn native_coverage_no_samples_is_unknown_but_real_zero_sample_remains_available() {
    let unavailable = super::super::session_analytics::SourceCoverage {
        failed: 1,
        ..Default::default()
    }
    .with_usage_samples(0);
    assert_eq!(
        unavailable.usage_availability.as_deref(),
        Some("unavailable")
    );
    assert_eq!(unavailable.known_usage_records, Some(0));
    let known_zero = super::super::session_analytics::SourceCoverage {
        read: 1,
        ..Default::default()
    }
    .with_usage_samples(1);
    assert_eq!(known_zero.usage_availability.as_deref(), Some("available"));
    assert_eq!(known_zero.known_usage_records, Some(1));
    let partial = super::super::session_analytics::SourceCoverage {
        read: 1,
        failed: 1,
        ..Default::default()
    }
    .with_usage_samples(1);
    assert_eq!(partial.usage_availability.as_deref(), Some("partial"));
}

#[test]
fn native_rebuild_new_epoch_republishes_verified_warm_sources_atomically() {
    let (_dir, db, spec) = setup();
    refresh_sources(&db, std::slice::from_ref(&spec)).unwrap();
    let accepted = read_input(&db).unwrap();
    assert!(accepted.codex_native_ready);
    db.conn.lock().unwrap().execute("INSERT OR REPLACE INTO maintenance_flags(key,value) VALUES('usage_ledger_epoch_v1','restored-fixture')",[]).unwrap();
    assert!(!read_input(&db).unwrap().codex_native_ready);
    let refresh = refresh_sources(&db, std::slice::from_ref(&spec)).unwrap();
    assert_eq!(
        refresh.failed, 0,
        "unchanged verified sources must not permanently block current-epoch publication"
    );
    assert_eq!(
        refresh.parsed, 0,
        "epoch fencing does not require re-parsing unchanged native bodies"
    );
    let rebuilt = read_input(&db).unwrap();
    assert!(rebuilt.codex_native_ready);
    assert_eq!(rebuilt.facts, accepted.facts);
    assert_ne!(rebuilt.snapshot_version, accepted.snapshot_version);
    assert_eq!(refresh_sources(&db, &[spec]).unwrap().parsed, 0);
}

#[test]
fn native_rebuild_source_route_return_restores_verified_listing_and_location() {
    let (_dir, db, spec) = setup();
    refresh_sources(&db, std::slice::from_ref(&spec)).unwrap();
    let accepted = read_input(&db).unwrap();
    assert!(accepted.sessions[0].source_available);
    assert!(accepted.sessions[0].manager_key.is_some());
    refresh_sources(&db, &[]).unwrap();
    let missing = read_input(&db).unwrap();
    assert!(!missing.sessions[0].source_available);
    assert!(missing.sessions[0].manager_key.is_none());
    // A route change is not a native-file mutation. Returning to that same
    // source must recover availability rather than clone its missing receipt.
    let result = refresh_sources(&db, &[spec]).unwrap();
    assert_eq!(result.failed, 0);
    let restored = read_input(&db).unwrap();
    assert!(restored.sessions[0].source_available);
    assert!(restored.sessions[0].manager_key.is_some());
    assert_eq!(restored.facts, accepted.facts);
}
