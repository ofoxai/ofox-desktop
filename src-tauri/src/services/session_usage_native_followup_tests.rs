//! Native-format regressions; all payloads are synthetic and sandboxed.
use super::{session_analytics_index as index, session_usage_gemini as gemini, usage_source};
use crate::{database::Database, test_sandbox::sandbox};
use serde_json::{json, Value};
use serial_test::serial;
use std::path::{Path, PathBuf};

fn lines(path: &Path, rows: &[Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
}
fn header() -> Value {
    json!({"sessionId":"s","projectHash":"p","startTime":"2026-10-15T00:00:00Z","kind":"main"})
}
fn reply(id: &str, input: u64, output: u64) -> Value {
    json!({"id":id,"type":"gemini","model":"fixture","timestamp":"2026-10-15T00:00:01Z","content":"synthetic reply","tokens":{"input":input,"output":output,"cached":0,"thoughts":0,"tool":0,"total":input+output}})
}
fn chat(home: &Path, name: &str) -> PathBuf {
    home.join(".gemini/tmp/p/chats").join(name)
}
fn buckets(db: &Database) -> [i64; 4] {
    db.conn.lock().unwrap().query_row("SELECT COALESCE(SUM(input_tokens),0),COALESCE(SUM(output_tokens),0),COALESCE(SUM(cache_read_tokens),0),COALESCE(SUM(cache_creation_tokens),0) FROM proxy_request_logs",[],|r|Ok([r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?])).unwrap()
}
fn spec(path: &Path, provider: &str) -> index::SourceSpec {
    index::SourceSpec {
        provider: provider.into(),
        path: path.to_owned(),
        subagent: false,
        project_file: None,
        title_file: None,
    }
}

#[test]
#[serial]
fn native_followup_gemini_jsonl_counts_spent_before_native_rewind() {
    let home = sandbox();
    let path = chat(home.home(), "session-a.jsonl");
    let mut first = reply("a", 140, 8);
    first["tokens"]["cached"] = json!(100);
    first["tokens"]["thoughts"] = json!(2);
    let mut initial = first.clone();
    initial.as_object_mut().unwrap().remove("tokens");
    let mut final_snapshot = first.clone();
    final_snapshot["tokens"] = json!({"input":150,"output":12,"cached":110,"thoughts":3});
    lines(
        &path,
        &[
            header(),
            initial,
            first,
            final_snapshot,
            reply("b", 30, 3),
            json!({"$rewindTo":"b"}),
            reply("c", 20, 2),
        ],
    );
    let read = usage_source::read_normalized("gemini", &path).unwrap();
    assert_eq!(read.facts.len(), 3);
    assert_eq!(
        read.facts
            .iter()
            .map(|f| u64::from(f.usage.input_tokens))
            .sum::<u64>(),
        90
    );
    assert_eq!(
        read.facts
            .iter()
            .map(|f| u64::from(f.usage.output_tokens))
            .sum::<u64>(),
        20
    );
    assert_eq!(
        read.facts
            .iter()
            .map(|f| u64::from(f.usage.cache_read_tokens))
            .sum::<u64>(),
        110
    );
    let current = crate::session_manager::providers::gemini::read_conversation(&path).unwrap();
    assert_eq!(
        current["messages"].as_array().unwrap().len(),
        2,
        "conversation rewind must remain separate from physical spending"
    );
}

#[test]
#[serial]
fn native_followup_gemini_jsonl_and_legacy_json_share_native_identity() {
    let home = sandbox();
    let path = chat(home.home(), "native-subagent-id.jsonl");
    lines(&path, &[header(), reply("a", 10, 1), reply("b", 20, 2)]);
    let old = chat(home.home(), "session-mirror.json");
    std::fs::write(
        old,
        json!({"sessionId":"s","messages":[reply("a",5,1),reply("a",10,1)]}).to_string(),
    )
    .unwrap();
    let db = Database::memory().unwrap();
    let scan = gemini::sync_gemini_usage_at(&db, &home.home().join(".gemini")).unwrap();
    assert!(scan.errors.is_empty(), "{scan:?}");
    assert_eq!(scan.files_scanned, 2);
    assert_eq!(buckets(&db), [30, 3, 0, 0]);
    let again = gemini::sync_gemini_usage_at(&db, &home.home().join(".gemini")).unwrap();
    assert!(again.errors.is_empty());
    assert_eq!(again.imported, 0);
    assert_eq!(buckets(&db), [30, 3, 0, 0]);
    let moved = chat(home.home(), "session-moved.jsonl");
    std::fs::rename(path, moved).unwrap();
    assert!(
        gemini::sync_gemini_usage_at(&db, &home.home().join(".gemini"))
            .unwrap()
            .errors
            .is_empty()
    );
    assert_eq!(buckets(&db), [30, 3, 0, 0]);
}

#[test]
#[serial]
fn native_followup_gemini_jsonl_checkpoint_failure_is_atomic_and_retryable() {
    let home = sandbox();
    let path = chat(home.home(), "session-a.jsonl");
    lines(&path, &[header(), reply("a", 10, 1), reply("b", 20, 2)]);
    let db = Database::memory().unwrap();
    db.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_native_jsonl_checkpoint BEFORE INSERT ON session_log_sync BEGIN SELECT RAISE(ABORT,'fixture checkpoint failure'); END;").unwrap();
    let failed = gemini::sync_gemini_usage_at(&db, &home.home().join(".gemini")).unwrap();
    assert!(
        !failed.errors.is_empty(),
        "the discovered JSONL must actually reach the transaction"
    );
    assert_eq!(buckets(&db), [0; 4]);
    assert_eq!(
        db.conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM session_log_sync", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_native_jsonl_checkpoint;")
        .unwrap();
    let retry = gemini::sync_gemini_usage_at(&db, &home.home().join(".gemini")).unwrap();
    assert!(retry.errors.is_empty(), "{retry:?}");
    assert_eq!(retry.imported, 2);
    assert_eq!(buckets(&db), [30, 3, 0, 0]);
}

#[test]
#[serial]
fn native_followup_gemini_jsonl_torn_tail_is_partial_and_later_consumed() {
    let home = sandbox();
    let path = chat(home.home(), "session-a.jsonl");
    lines(&path, &[header(), reply("a", 10, 1)]);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"id\":")
        .unwrap();
    let read = usage_source::read_normalized("gemini", &path).unwrap();
    assert_eq!(read.facts.len(), 1);
    assert!(!read.complete);
    assert!(read.reasons.contains("incompleteEnumeration"));
    let db = Database::memory().unwrap();
    assert_eq!(
        gemini::sync_gemini_usage_at(&db, &home.home().join(".gemini"))
            .unwrap()
            .imported,
        1
    );
    lines(&path, &[header(), reply("a", 10, 1), reply("b", 20, 2)]);
    assert_eq!(
        gemini::sync_gemini_usage_at(&db, &home.home().join(".gemini"))
            .unwrap()
            .imported,
        1
    );
    assert_eq!(buckets(&db), [30, 3, 0, 0]);
}

#[test]
#[serial]
fn native_followup_gemini_jsonl_enters_cached_analytics() {
    let home = sandbox();
    let path = chat(home.home(), "session-a.jsonl");
    lines(
        &path,
        &[
            header(),
            json!({"id":"u","type":"user","timestamp":"2026-10-15T00:00:00Z","content":"synthetic user"}),
            reply("a", 10, 1),
        ],
    );
    let db = Database::memory().unwrap();
    let sources = [spec(&path, "gemini")];
    let report = index::refresh_sources(&db, &sources).unwrap();
    assert_eq!(report.failed, 0);
    let input = index::read_input(&db).unwrap();
    assert_eq!(input.facts.len(), 1);
    assert_eq!(input.sessions.len(), 1);
    assert_eq!(input.messages.len(), 2);
    assert_eq!(index::refresh_sources(&db, &sources).unwrap().unchanged, 1);
}

fn assert_unverified_gemini_role(path: &Path, reason: &str) {
    let db = Database::memory().unwrap();
    let sources = [spec(path, "gemini")];
    assert_eq!(index::refresh_sources(&db, &sources).unwrap().failed, 0);
    let input = index::read_input(&db).unwrap();
    assert_eq!(input.facts.len(), 1, "paid usage must remain available");
    assert_eq!(input.facts[0].buckets, [10, 1, 0, 0]);
    assert_eq!(input.messages.len(), 2);
    assert!(input.messages.iter().all(|m| !m.main));
    assert!(input.source_reasons.iter().any(|r| r.reason == reason));
    let query = super::usage_query::UsageQueryV1 {
        version: 1,
        start_ms: Some(1792022400000),
        end_ms: 1792108800000,
        tool_ids: vec!["gemini".into()],
        model_ids: vec![],
        project_ids: vec![],
        source_ids: vec![],
    };
    let result =
        super::session_analytics_api::query(&db, query, "ofox.ai", 1792108800000, &chrono::Utc)
            .unwrap();
    assert_eq!(result.totals.messages, None);
    assert_eq!(result.totals.active_ms, None);
    assert_eq!(index::refresh_sources(&db, &sources).unwrap().unchanged, 1);
}

#[test]
#[serial]
fn native_followup_gemini_jsonl_subagent_keeps_spending_without_main_activity() {
    let home = sandbox();
    let path = chat(home.home(), "child.jsonl");
    let mut child = header();
    child["kind"] = json!("subagent");
    lines(
        &path,
        &[
            child,
            json!({"id":"u","type":"user","timestamp":"2026-10-15T00:00:00Z","content":"synthetic user"}),
            reply("a", 10, 1),
        ],
    );
    assert_unverified_gemini_role(&path, "sessionParentUnavailable");
}

#[test]
#[serial]
fn native_followup_gemini_jsonl_missing_or_future_kind_cannot_claim_main_role() {
    let home = sandbox();
    for kind in [None, Some("future-kind")] {
        let path = chat(home.home(), "unknown.jsonl");
        let mut metadata = header();
        match kind {
            Some(kind) => metadata["kind"] = json!(kind),
            None => {
                metadata.as_object_mut().unwrap().remove("kind");
            }
        }
        lines(
            &path,
            &[
                metadata,
                json!({"id":"u","type":"user","timestamp":"2026-10-15T00:00:00Z","content":"synthetic user"}),
                reply("a", 10, 1),
            ],
        );
        assert_unverified_gemini_role(&path, "sessionRoleNotRecorded");
    }
    let legacy = chat(home.home(), "session-legacy.json");
    std::fs::write(
        &legacy,
        json!({"sessionId":"s","messages":[reply("a",10,1)]}).to_string(),
    )
    .unwrap();
    let db = Database::memory().unwrap();
    assert_eq!(
        index::refresh_sources(&db, &[spec(&legacy, "gemini")])
            .unwrap()
            .failed,
        0
    );
    let input = index::read_input(&db).unwrap();
    assert_eq!(input.facts.len(), 1);
    assert_eq!(input.messages.len(), 1);
    assert!(
        input.messages[0].main,
        "legacy JSON retains its role contract"
    );
}

fn codex_rows(fork: bool) -> Vec<Value> {
    let mut rows = vec![
        json!({"type":"session_meta","payload":{"id":"parent","cwd":"/fixture","source":"cli"}}),
        json!({"type":"event_msg","timestamp":"2026-10-15T00:00:01Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":1}}}}),
    ];
    if fork {
        rows.push(json!({"type":"session_meta","payload":{"id":"child","forked_from_id":"parent","parent_thread_id":"parent","cwd":"/fixture","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}}}));
    }
    rows.push(json!({"type":"event_msg","timestamp":"2026-10-15T00:00:02Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"output_tokens":2}}}}));
    rows
}

#[test]
#[serial]
fn native_followup_codex_replayed_headers_are_unknown_partial_and_warm_cached() {
    let home = sandbox();
    let path = home.home().join("codex-fork.jsonl");
    lines(&path, &codex_rows(true));
    let db = Database::memory().unwrap();
    db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,created_at,data_source,session_id) VALUES('legacy-fixture','_codex_session','codex','fixture',7,2,0,0,1,'codex_session','parent')",[]).unwrap();
    let sources = [spec(&path, "codex")];
    let report = index::refresh_sources(&db, &sources).unwrap();
    assert_eq!(report.failed, 0);
    let input = index::read_input(&db).unwrap();
    assert!(input.sessions.is_empty());
    assert!(input.facts.is_empty());
    assert!(input.messages.is_empty());
    assert!(input
        .source_reasons
        .iter()
        .any(|r| r.reason == "sessionIdentityUnavailable"));
    assert_eq!(
        buckets(&db),
        [7, 2, 0, 0],
        "C must not reattribute the legacy ledger"
    );
    let warm = index::refresh_sources(&db, &sources).unwrap();
    assert_eq!(warm.unchanged, 1);
    assert_eq!(warm.parsed, 0);
    assert_eq!(warm.failed, 0);
    let query = super::usage_query::UsageQueryV1 {
        version: 1,
        start_ms: Some(1792022400000),
        end_ms: 1792108800000,
        tool_ids: vec!["codex".into()],
        model_ids: vec![],
        project_ids: vec![],
        source_ids: vec![],
    };
    let result =
        super::session_analytics_api::query(&db, query, "ofox.ai", 1792108800000, &chrono::Utc)
            .unwrap();
    assert_eq!(result.totals.messages, None);
    assert_eq!(result.totals.active_ms, None);
}

#[test]
#[serial]
fn native_followup_codex_single_header_still_produces_known_facts() {
    let home = sandbox();
    let path = home.home().join("codex.jsonl");
    lines(&path, &codex_rows(false));
    let db = Database::memory().unwrap();
    let report = index::refresh_sources(&db, &[spec(&path, "codex")]).unwrap();
    assert_eq!(report.failed, 0);
    let input = index::read_input(&db).unwrap();
    assert_eq!(input.sessions.len(), 1);
    assert_eq!(input.facts.len(), 2);
}

#[test]
#[serial]
fn native_followup_codex_actual_malformed_or_missing_source_still_fails() {
    let home = sandbox();
    let path = home.home().join("codex.jsonl");
    lines(&path, &codex_rows(true));
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{broken native fixture\n")
        .unwrap();
    let db = Database::memory().unwrap();
    let sources = [spec(&path, "codex")];
    assert_eq!(index::refresh_sources(&db, &sources).unwrap().failed, 1);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(index::refresh_sources(&db, &sources).unwrap().failed, 1);
}

#[test]
#[serial]
fn native_followup_codex_failed_replay_preserves_cache_until_identity_is_known_partial() {
    let home = sandbox();
    let path = home.home().join("codex.jsonl");
    lines(&path, &codex_rows(false));
    let db = Database::memory().unwrap();
    let sources = [spec(&path, "codex")];
    assert_eq!(index::refresh_sources(&db, &sources).unwrap().failed, 0);
    assert_eq!(index::read_input(&db).unwrap().facts.len(), 2);
    lines(&path, &codex_rows(true));
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{broken fixture\n")
        .unwrap();
    assert_eq!(index::refresh_sources(&db, &sources).unwrap().failed, 1);
    assert_eq!(
        index::read_input(&db).unwrap().facts.len(),
        2,
        "real malformed input must retain last known cache"
    );
    lines(&path, &codex_rows(true));
    assert_eq!(index::refresh_sources(&db, &sources).unwrap().failed, 0);
    let partial = index::read_input(&db).unwrap();
    assert!(partial.facts.is_empty());
    assert!(partial.sessions.is_empty());
}

#[test]
#[serial]
fn native_followup_gemini_jsonl_identity_change_cannot_rebind_previous_spending() {
    let home = sandbox();
    let path = chat(home.home(), "session-a.jsonl");
    lines(&path, &[header(), reply("a", 10, 1)]);
    let db = Database::memory().unwrap();
    let root = home.home().join(".gemini");
    assert_eq!(
        gemini::sync_gemini_usage_at(&db, &root).unwrap().imported,
        1
    );
    let before: Vec<(String, i64, i64)> = db
        .conn
        .lock()
        .unwrap()
        .prepare("SELECT file_path,last_modified,last_line_offset FROM session_log_sync")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    lines(
        &path,
        &[
            header(),
            reply("a", 10, 1),
            json!({"$set":{"sessionId":"different"}}),
            reply("b", 20, 2),
        ],
    );
    assert!(usage_source::read_normalized("gemini", &path).is_err());
    assert!(!gemini::sync_gemini_usage_at(&db, &root)
        .unwrap()
        .errors
        .is_empty());
    assert_eq!(buckets(&db), [10, 1, 0, 0]);
    let after: Vec<(String, i64, i64)> = db
        .conn
        .lock()
        .unwrap()
        .prepare("SELECT file_path,last_modified,last_line_offset FROM session_log_sync")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(after, before);
}

#[test]
#[serial]
fn native_followup_gemini_jsonl_context_set_and_patch_do_not_delete_spending() {
    let home = sandbox();
    let path = chat(home.home(), "session-a.jsonl");
    lines(
        &path,
        &[
            header(),
            reply("a", 10, 1),
            json!({"$set":{"messages":[reply("b",20,2)]}}),
            json!({"$patch":{"removeIds":["b"]}}),
        ],
    );
    let read = usage_source::read_normalized("gemini", &path).unwrap();
    assert_eq!(read.facts.len(), 2);
    assert_eq!(
        read.facts
            .iter()
            .map(|f| u64::from(f.usage.input_tokens))
            .sum::<u64>(),
        30
    );
    assert!(
        crate::session_manager::providers::gemini::read_conversation(&path).unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
#[serial]
fn native_followup_gemini_committed_malformed_line_keeps_last_analytics_facts() {
    let home = sandbox();
    let path = chat(home.home(), "session-a.jsonl");
    lines(&path, &[header(), reply("a", 10, 1)]);
    let db = Database::memory().unwrap();
    let sources = [spec(&path, "gemini")];
    assert_eq!(index::refresh_sources(&db, &sources).unwrap().failed, 0);
    assert_eq!(index::read_input(&db).unwrap().facts.len(), 1);
    std::fs::write(&path, format!("{}\n{{broken}}\n", header())).unwrap();
    assert_eq!(index::refresh_sources(&db, &sources).unwrap().failed, 1);
    assert_eq!(index::read_input(&db).unwrap().facts.len(), 1);
    assert!(index::read_input(&db)
        .unwrap()
        .source_reasons
        .iter()
        .any(|r| r.reason == "sourceReadFailed"));
}
