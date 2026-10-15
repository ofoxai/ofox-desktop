//! Public reader fixtures; all files and databases belong to test_sandbox.
use super::{
    session_usage as claude, session_usage_codex as codex, session_usage_gemini as gemini,
    session_usage_opencode as opencode,
};
use crate::{database::Database, error::AppError, test_sandbox::sandbox};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use serial_test::serial;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

type Buckets = [i64; 4];
fn write(path: &Path, body: &str, revision: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(10 + revision))
        .unwrap();
}
fn jsonl(lines: &[Value]) -> String {
    lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
fn facts(db: &Database) -> Vec<(String, Buckets)> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT request_id,input_tokens,output_tokens,cache_read_tokens,
        cache_creation_tokens FROM proxy_request_logs ORDER BY created_at,request_id",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((r.get(0)?, [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?]))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}
fn checkpoint(db: &Database) -> Vec<(String, i64, i64)> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare("SELECT file_path,last_modified,last_line_offset FROM session_log_sync ORDER BY file_path").unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
fn good(result: Result<claude::SessionSyncResult, AppError>) -> claude::SessionSyncResult {
    let r = result.unwrap();
    assert!(r.errors.is_empty(), "{r:?}");
    r
}
fn buckets(rows: &[(String, Buckets)]) -> Vec<Buckets> {
    rows.iter().map(|r| r.1).collect()
}
fn entries(rows: Vec<claude::SessionUsageEntry>) -> Vec<Buckets> {
    rows.into_iter()
        .map(|r| {
            [
                r.usage.input_tokens as i64,
                r.usage.output_tokens as i64,
                r.usage.cache_read_tokens as i64,
                r.usage.cache_creation_tokens as i64,
            ]
        })
        .collect()
}
fn c(id: &str, tokens: [u64; 4]) -> Value {
    json!({"type":"assistant","sessionId":"s","timestamp":"2026-10-15T00:00:01Z",
        "message":{"id":id,"model":"fixture","stop_reason":"end_turn","usage":{
            "input_tokens":tokens[0],"output_tokens":tokens[1],"cache_read_input_tokens":tokens[2],
            "cache_creation_input_tokens":tokens[3]}}})
}
fn claude_path(home: &Path) -> PathBuf {
    home.join(".claude/projects/p/s.jsonl")
}
fn x(n: u32, info: Value) -> Value {
    json!({"type":"event_msg","timestamp":format!("2026-10-15T00:00:{n:02}Z"),
        "payload":{"type":"token_count","info":info}})
}
fn total(input: u64, output: u64, cached: u64) -> Value {
    json!({"input_tokens":input,"output_tokens":output,"cached_input_tokens":cached})
}
fn t(n: u32, input: u64, output: u64, cached: u64) -> Value {
    x(n, json!({"total_token_usage":total(input,output,cached)}))
}
fn l(n: u32, input: u64, output: u64, cached: u64) -> Value {
    x(n, json!({"last_token_usage":total(input,output,cached)}))
}
fn codex_body(events: &[Value]) -> String {
    let mut lines = vec![json!({"type":"session_meta","payload":{"id":"s"}})];
    lines.extend_from_slice(events);
    jsonl(&lines)
}
fn codex_path(home: &Path) -> PathBuf {
    home.join(".codex/sessions/2026/10/15/rollout-s.jsonl")
}

#[test]
#[serial]
fn semantics_claude_zero_output_consumption_is_not_empty() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    write(
        &claude_path(home.home()),
        &jsonl(&[
            c("a", [0, 0, 500, 0]),
            c("b", [0, 0, 0, 70]),
            c("c", [4, 0, 0, 0]),
        ]),
        0,
    );
    assert_eq!(good(claude::sync_claude_session_logs(&db)).imported, 3);
    let expected = vec![[0, 0, 500, 0], [0, 0, 0, 70], [4, 0, 0, 0]];
    assert_eq!(buckets(&facts(&db)), expected);
    assert_eq!(entries(claude::session_entries()), expected);
    assert_eq!(good(claude::sync_claude_session_logs(&db)).imported, 0);
}

#[test]
#[serial]
fn semantics_claude_same_output_revisions_replace_all_buckets() {
    let home = sandbox();
    for (first, last) in [
        ([2, 10, 100, 20], [7, 10, 300, 40]),
        ([5, 50, 100, 80], [3, 50, 80, 60]),
    ] {
        for incremental in [false, true] {
            let db = Database::memory().unwrap();
            let path = claude_path(home.home());
            if incremental {
                write(&path, &jsonl(&[c("m", first)]), 0);
                good(claude::sync_claude_session_logs(&db));
            }
            write(&path, &jsonl(&[c("m", first), c("m", last)]), 1);
            good(claude::sync_claude_session_logs(&db));
            let expected = vec![last.map(|n| n as i64)];
            assert_eq!(buckets(&facts(&db)), expected, "incremental={incremental}");
            assert_eq!(entries(claude::session_entries()), expected);
        }
    }
}

fn claude_rewritten_source(mode: &str) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = claude_path(home.home());
    let initial = if mode == "truncate" {
        vec![c("a", [10, 10, 0, 0]), c("b", [20, 20, 0, 0])]
    } else {
        vec![c("a", [10, 10, 0, 0])]
    };
    write(&path, &jsonl(&initial), 0);
    good(claude::sync_claude_session_logs(&db));
    let stamp = fs::metadata(&path).unwrap().modified().unwrap();
    let revised = if mode == "inline" {
        c("a", [30, 10, 0, 0])
    } else {
        c("new", [30, 3, 0, 0])
    };
    if mode == "replace" {
        let replacement = path.with_extension("replacement");
        write(&replacement, &jsonl(&[revised]), 1);
        fs::rename(replacement, &path).unwrap();
    } else {
        write(&path, &jsonl(&[revised]), 1);
    }
    if mode == "backdated" {
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp - Duration::from_secs(100))
            .unwrap();
    }
    assert_eq!(
        good(claude::sync_claude_session_logs(&db)).imported,
        1,
        "{mode} skipped changed content before the old offset"
    );
    let rows = facts(&db);
    if mode == "inline" {
        assert_eq!(buckets(&rows), vec![[30, 10, 0, 0]]);
    } else {
        assert!(rows.iter().any(|r| r.0 == "session:new"));
    }
}
macro_rules! rewrite_case {
    ($name:ident,$mode:literal) => {
        #[test]
        #[serial]
        fn $name() {
            claude_rewritten_source($mode);
        }
    };
}
rewrite_case!(semantics_rewrite_claude_replaced_inode, "replace");
rewrite_case!(semantics_rewrite_claude_truncated_file, "truncate");
rewrite_case!(semantics_rewrite_claude_backdated_content, "backdated");
rewrite_case!(semantics_rewrite_claude_in_place_revision, "inline");

fn claude_model_correction(incremental: bool) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = claude_path(home.home());
    let mut first = c("model", [2, 10, 100, 20]);
    first["message"].as_object_mut().unwrap().remove("model");
    let mut final_line = first.clone();
    final_line["message"]["model"] = json!("native-model");
    if incremental {
        write(&path, &jsonl(&[first.clone()]), 0);
        good(claude::sync_claude_session_logs(&db));
    }
    write(&path, &jsonl(&[first, final_line]), 1);
    good(claude::sync_claude_session_logs(&db));
    let row: (String, String) = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT model,request_model FROM proxy_request_logs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("native-model".into(), "native-model".into()));
}
#[test]
#[serial]
fn semantics_model_claude_same_file_correction() {
    claude_model_correction(false);
}
#[test]
#[serial]
fn semantics_model_claude_incremental_correction() {
    claude_model_correction(true);
}

fn claude_archived_copy(in_place: bool) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let a = claude_path(home.home());
    let b = a.with_file_name("other.jsonl");
    let stamp = (chrono::Utc::now() - chrono::Duration::days(40)).to_rfc3339();
    for (path, id) in [(&a, "ma"), (&b, "mb")] {
        let mut line = c(id, [100, 10, 0, 0]);
        line["timestamp"] = json!(stamp);
        write(path, &jsonl(&[line]), 0);
    }
    good(claude::sync_claude_session_logs(&db));
    assert_eq!(facts(&db).len(), 2);
    db.rollup_and_prune(30).unwrap();
    assert_eq!(archived_total(&db), (200, 20, 2));
    if in_place {
        fs::write(&a, fs::read(&b).unwrap()).unwrap();
    } else {
        let replacement = a.with_extension("replacement");
        fs::copy(&b, &replacement).unwrap();
        fs::rename(replacement, &a).unwrap();
    }
    fs::remove_file(&b).unwrap();
    good(claude::sync_claude_session_logs(&db));
    assert_eq!(
        archived_total(&db),
        (200, 20, 2),
        "copied archived native event was counted again, in_place={in_place}"
    );
    assert!(facts(&db).is_empty());
}
#[test]
#[serial]
fn semantics_alias_claude_archived_copy_with_new_inode() {
    claude_archived_copy(false);
}
#[test]
#[serial]
fn semantics_alias_claude_archived_copy_in_place() {
    claude_archived_copy(true);
}

fn check_codex(home: &Path, events: &[Value], expected: &[Buckets], ordinals: &[u32]) {
    let db = Database::memory().unwrap();
    write(&codex_path(home), &codex_body(events), 0);
    good(codex::sync_codex_usage(&db));
    let rows = facts(&db);
    assert_eq!(buckets(&rows), expected);
    assert_eq!(
        rows.iter().map(|r| r.0.clone()).collect::<Vec<_>>(),
        ordinals
            .iter()
            .map(|n| format!("codex_session:s:rollout-s:{n}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(entries(codex::session_entries()), expected);
    assert_eq!(good(codex::sync_codex_usage(&db)).imported, 0);
}
#[test]
#[serial]
fn semantics_codex_total_last_total_advances_the_accounted_baseline() {
    let home = sandbox();
    check_codex(
        home.home(),
        &[t(1, 100, 10, 60), l(2, 50, 5, 30), t(3, 180, 18, 100)],
        &[[40, 10, 60, 0], [20, 5, 30, 0], [20, 3, 10, 0]],
        &[1, 2, 3],
    );
}
#[test]
#[serial]
fn semantics_codex_corrected_zero_keeps_the_legacy_ordinal_slot() {
    let home = sandbox();
    check_codex(
        home.home(),
        &[
            t(1, 100, 10, 0),
            l(2, 20, 2, 0),
            t(3, 120, 12, 0),
            t(4, 150, 15, 0),
        ],
        &[[100, 10, 0, 0], [20, 2, 0, 0], [30, 3, 0, 0]],
        &[1, 2, 4],
    );
}
#[test]
#[serial]
fn semantics_codex_true_old_zero_and_null_info_do_not_take_slots() {
    let home = sandbox();
    check_codex(
        home.home(),
        &[
            t(1, 0, 0, 0),
            t(2, 10, 1, 0),
            t(3, 10, 1, 0),
            x(4, Value::Null),
            t(5, 20, 3, 0),
        ],
        &[[10, 1, 0, 0], [10, 2, 0, 0]],
        &[1, 2],
    );
}
#[test]
#[serial]
fn semantics_codex_null_total_falls_back_to_last() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = codex_path(home.home());
    write(
        &path,
        &codex_body(&[
            x(
                1,
                json!({"total_token_usage":null,"last_token_usage":total(30,3,0)}),
            ),
            l(2, 20, 2, 0),
        ]),
        0,
    );
    good(codex::sync_codex_usage(&db));
    let before = facts(&db);
    assert_eq!(buckets(&before), vec![[30, 3, 0, 0], [20, 2, 0, 0]]);
    assert!(before[0]
        .0
        .starts_with("codex_session:s:rollout-s:recovered-v2:"));
    assert_eq!(before[1].0, "codex_session:s:rollout-s:1");
    assert_eq!(
        entries(codex::session_entries()),
        vec![[30, 3, 0, 0], [20, 2, 0, 0]]
    );
    let archived = home.home().join(".codex/archived_sessions/rollout-s.jsonl");
    fs::create_dir_all(archived.parent().unwrap()).unwrap();
    fs::rename(&path, &archived).unwrap();
    good(codex::sync_codex_usage(&db));
    assert_eq!(facts(&db), before);
    db.conn
        .lock()
        .unwrap()
        .execute("DELETE FROM session_log_sync", [])
        .unwrap();
    good(codex::sync_codex_usage(&db));
    assert_eq!(facts(&db), before);
}
#[test]
#[serial]
fn semantics_codex_continued_first_call_then_last_then_total() {
    let home = sandbox();
    check_codex(
        home.home(),
        &[
            x(
                1,
                json!({"total_token_usage":total(400,30,0),"last_token_usage":total(300,20,0)}),
            ),
            l(2, 50, 5, 0),
            t(3, 900, 70, 0),
        ],
        &[[300, 20, 0, 0], [50, 5, 0, 0], [450, 35, 0, 0]],
        &[1, 2, 3],
    );
}
#[test]
#[serial]
fn semantics_codex_large_cumulative_keeps_small_deltas() {
    let home = sandbox();
    check_codex(
        home.home(),
        &[
            x(
                1,
                json!({"total_token_usage":total(4294967396,10,0),
        "last_token_usage":total(40,4,0)}),
            ),
            t(2, 4294967421, 12, 0),
        ],
        &[[40, 4, 0, 0], [25, 2, 0, 0]],
        &[1, 2],
    );
}

#[test]
#[serial]
fn semantics_codex_unrecognized_utf8_model_does_not_panic() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    write(
        &codex_path(home.home()),
        &codex_body(&[
            json!({"type":"turn_context","payload":{"model":"éééééé"}}),
            t(1, 10, 1, 0),
        ]),
        0,
    );
    assert_eq!(good(codex::sync_codex_usage(&db)).imported, 1);
    let model: String = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT model FROM proxy_request_logs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(model, "éééééé");
}

fn semantics_jsonl_incomplete_tail_retries_the_same_physical_line(is_codex: bool) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = if is_codex {
        codex_path(home.home())
    } else {
        claude_path(home.home())
    };
    let (prefix, next, third) = if is_codex {
        (
            codex_body(&[t(1, 100, 10, 0)]),
            t(2, 120, 12, 0),
            t(3, 150, 15, 0),
        )
    } else {
        (
            jsonl(&[c("a", [100, 10, 0, 0])]),
            c("b", [20, 2, 0, 0]),
            c("c", [30, 3, 0, 0]),
        )
    };
    let next = next.to_string();
    let cut = next.len() / 2;
    write(&path, &(prefix.clone() + &next[..cut]), 0);
    let sync = |db: &Database| {
        if is_codex {
            codex::sync_codex_usage(db)
        } else {
            claude::sync_claude_session_logs(db)
        }
    };
    assert_eq!(good(sync(&db)).imported, 1);
    assert_eq!(
        checkpoint(&db)[0].2,
        if is_codex { 2 } else { 1 },
        "incomplete tail cannot be checkpointed"
    );
    write(
        &path,
        &(prefix + &next + "\n" + &third.to_string() + "\n"),
        1,
    );
    assert_eq!(good(sync(&db)).imported, 2);
    assert_eq!(facts(&db).len(), 3);
}
#[test]
#[serial]
fn semantics_jsonl_incomplete_tail_retries_the_same_physical_line_claude() {
    semantics_jsonl_incomplete_tail_retries_the_same_physical_line(false);
}
#[test]
#[serial]
fn semantics_jsonl_incomplete_tail_retries_the_same_physical_line_codex() {
    semantics_jsonl_incomplete_tail_retries_the_same_physical_line(true);
}

#[test]
#[serial]
fn semantics_jsonl_complete_tail_without_newline_is_consumed() {
    let home = sandbox();
    for is_codex in [false, true] {
        let db = Database::memory().unwrap();
        let path = if is_codex {
            codex_path(home.home())
        } else {
            claude_path(home.home())
        };
        let body = if is_codex {
            codex_body(&[t(1, 10, 1, 0)])
        } else {
            jsonl(&[c("a", [10, 1, 0, 0])])
        };
        write(&path, body.trim_end(), 0);
        good(if is_codex {
            codex::sync_codex_usage(&db)
        } else {
            claude::sync_claude_session_logs(&db)
        });
        assert_eq!(facts(&db).len(), 1);
    }
}

#[derive(Clone, Copy, Debug)]
enum Source {
    Claude,
    Codex,
    Gemini,
    OpenCodeDb,
    OpenCodeJson,
}
impl Source {
    fn sync(self, db: &Database) -> Result<claude::SessionSyncResult, AppError> {
        match self {
            Self::Claude => claude::sync_claude_session_logs(db),
            Self::Codex => codex::sync_codex_usage(db),
            Self::Gemini => gemini::sync_gemini_usage(db),
            _ => opencode::sync_opencode_usage(db),
        }
    }
    fn write(self, home: &Path, tokens: [u64; 5], revision: u64) {
        let old = chrono::Utc::now() - chrono::Duration::days(40);
        let stamp = old.to_rfc3339();
        let at = old.timestamp_millis();
        let gm = |id: &str, ts: [u64; 5]| {
            json!({"id":id,"type":"gemini","model":"fixture","timestamp":stamp,
            "tokens":{"input":ts[0],"output":ts[1],"cached":ts[2],"thoughts":ts[4]}})
        };
        let oc = |id: &str, ts: [u64; 5]| {
            json!({"id":id,"sessionID":"s","role":"assistant","modelID":"fixture",
            "tokens":{"input":ts[0],"output":ts[1],"cache":{"read":ts[2],"write":ts[3]},"reasoning":ts[4]},
            "time":{"created":at,"completed":at+1}})
        };
        let base = [100, 10, 0, 0, 0];
        match self {
            Self::Claude => write(
                &claude_path(home),
                &jsonl(&[
                    c("a", [100, 10, 0, 0]),
                    c("b", [tokens[0], tokens[1], tokens[2], tokens[3]]),
                ]),
                revision,
            ),
            Self::Codex => write(
                &codex_path(home),
                &codex_body(&[
                    t(1, 100, 10, 0),
                    x(
                        2,
                        json!({"last_token_usage":{
                "input_tokens":tokens[0],"output_tokens":tokens[1],"cached_input_tokens":tokens[2],"cache_write_input_tokens":tokens[3]}}),
                    ),
                ]),
                revision,
            ),
            Self::Gemini => write(
                &home.join(".gemini/tmp/p/chats/session-s.json"),
                &json!({"sessionId":"s","messages":[gm("a",base),gm("b",tokens)]}).to_string(),
                revision,
            ),
            Self::OpenCodeJson => {
                for (id, ts) in [("a", base), ("b", tokens)] {
                    write(
                        &home.join(format!(".local/share/opencode/storage/message/s/{id}.json")),
                        &oc(id, ts).to_string(),
                        revision,
                    );
                }
            }
            Self::OpenCodeDb => {
                let path = home.join(".local/share/opencode/opencode.db");
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                let conn = Connection::open(path).unwrap();
                conn.execute_batch("CREATE TABLE IF NOT EXISTS session(id TEXT PRIMARY KEY,parent_id TEXT); CREATE TABLE IF NOT EXISTS message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT); DELETE FROM session; DELETE FROM message; INSERT INTO session VALUES('s',NULL);").unwrap();
                for (id, ts) in [("a", base), ("b", tokens)] {
                    conn.execute(
                        "INSERT INTO message VALUES(?1,'s',?2,?3,?4)",
                        params![id, at, at + revision as i64, oc(id, ts).to_string()],
                    )
                    .unwrap();
                }
            }
        }
    }
}
fn overflow_is_visible(source: Source) {
    let home = sandbox();
    let overflow = u32::MAX as u64 + 1;
    let mut cases = vec![
        [overflow, 1, 0, 0, 0],
        [1, overflow, 0, 0, 0],
        [overflow + 10, 1, overflow, 0, 0],
    ];
    if !matches!(source, Source::Gemini) {
        cases.push([overflow + 10, 1, 0, overflow, 0]);
    }
    if matches!(
        source,
        Source::Gemini | Source::OpenCodeDb | Source::OpenCodeJson
    ) {
        cases.extend([[1, u32::MAX as u64, 0, 0, 1], [1, u64::MAX, 0, 0, 1]]);
    }
    for (i, ts) in cases.into_iter().enumerate() {
        let db = Database::memory().unwrap();
        source.write(home.home(), ts, i as u64 * 5);
        match source.sync(&db) {
            Err(_) => {}
            Ok(r) => assert!(
                !r.errors.is_empty(),
                "{source:?} accepted a fresh overflow {ts:?}: {r:?}"
            ),
        }
        assert!(facts(&db).is_empty());
        assert!(checkpoint(&db).is_empty());
        source.write(home.home(), [20, 2, 0, 0, 0], i as u64 * 5 + 1);
        good(source.sync(&db));
        let before = facts(&db);
        let state = checkpoint(&db);
        source.write(home.home(), ts, i as u64 * 5 + 2);
        if matches!(source, Source::Claude) {
            write(
                &claude_path(home.home()),
                &jsonl(&[
                    c("a", [100, 10, 0, 0]),
                    c("b", [20, 2, 0, 0]),
                    c("poison", [ts[0], ts[1], ts[2], ts[3]]),
                ]),
                i as u64 * 5 + 2,
            );
        }
        match source.sync(&db) {
            Err(_) => {}
            Ok(r) => assert!(!r.errors.is_empty(), "{source:?} accepted {ts:?}: {r:?}"),
        }
        assert_eq!(facts(&db), before, "{source:?} corrupted a committed row");
        assert_eq!(checkpoint(&db), state);
        source.write(home.home(), [30, 3, 0, 0, 0], i as u64 * 5 + 3);
        if matches!(source, Source::Claude) {
            write(
                &claude_path(home.home()),
                &jsonl(&[
                    c("a", [100, 10, 0, 0]),
                    c("b", [20, 2, 0, 0]),
                    c("poison", [30, 3, 0, 0]),
                ]),
                i as u64 * 5 + 3,
            );
        }
        good(source.sync(&db));
    }
}
macro_rules! overflow_case {
    ($name:ident,$source:expr) => {
        #[test]
        #[serial]
        fn $name() {
            overflow_is_visible($source);
        }
    };
}
overflow_case!(semantics_claude_checked_buckets, Source::Claude);
overflow_case!(semantics_codex_checked_buckets, Source::Codex);
overflow_case!(semantics_gemini_checked_buckets, Source::Gemini);
overflow_case!(semantics_opencode_db_checked_buckets, Source::OpenCodeDb);
overflow_case!(
    semantics_opencode_json_checked_buckets,
    Source::OpenCodeJson
);

#[test]
#[serial]
fn semantics_gemini_duplicate_id_has_one_latest_snapshot_in_both_readers() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let msg = |input, cached| {
        json!({"id":"m","type":"gemini","timestamp":"2026-10-15T00:00:01Z","model":"fixture",
        "tokens":{"input":input,"cached":cached,"output":8,"thoughts":2}})
    };
    let path = home.home().join(".gemini/tmp/p/chats/session-s.json");
    write(
        &path,
        &json!({"sessionId":"s","messages":[msg(140,100),msg(150,110)]}).to_string(),
        0,
    );
    good(gemini::sync_gemini_usage(&db));
    assert_eq!(buckets(&facts(&db)), vec![[40, 10, 110, 0]]);
    assert_eq!(entries(gemini::session_entries()), vec![[40, 10, 110, 0]]);
}

fn semantics_jsonl_tail_completion_in_same_second_is_detected(is_codex: bool) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = if is_codex {
        codex_path(home.home())
    } else {
        claude_path(home.home())
    };
    let prefix = if is_codex {
        codex_body(&[t(1, 100, 10, 0)])
    } else {
        jsonl(&[c("a", [100, 10, 0, 0])])
    };
    let next = if is_codex {
        t(2, 120, 12, 0)
    } else {
        c("b", [20, 2, 0, 0])
    };
    let next = next.to_string();
    let second = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 100;
    let stamp = |nanos| SystemTime::UNIX_EPOCH + Duration::new(second, nanos);
    let set_stamp = |nanos| {
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp(nanos))
            .unwrap()
    };
    let sync = |db: &Database| {
        if is_codex {
            codex::sync_codex_usage(db)
        } else {
            claude::sync_claude_session_logs(db)
        }
    };
    write(&path, &(prefix.clone() + &next[..next.len() / 2]), 0);
    set_stamp(100_000_000);
    good(sync(&db));
    assert_eq!(facts(&db).len(), 1);
    let before = fs::metadata(&path).unwrap();
    write(&path, &(prefix + &next + "\n"), 0);
    set_stamp(200_000_000);
    let after = fs::metadata(&path).unwrap();
    assert_eq!(
        before
            .modified()
            .unwrap()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        after
            .modified()
            .unwrap()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    assert_ne!(before.len(), after.len());
    assert_eq!(
        good(sync(&db)).imported,
        1,
        "same-second completion was permanently skipped: codex={is_codex}"
    );
    assert_eq!(facts(&db).len(), 2);
}

#[test]
#[serial]
fn semantics_jsonl_tail_completion_in_same_second_is_detected_claude() {
    semantics_jsonl_tail_completion_in_same_second_is_detected(false);
}
#[test]
#[serial]
fn semantics_jsonl_tail_completion_in_same_second_is_detected_codex() {
    semantics_jsonl_tail_completion_in_same_second_is_detected(true);
}

fn archived_total(db: &Database) -> (i64, i64, i64) {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT COALESCE(SUM(i),0),COALESCE(SUM(o),0),COALESCE(SUM(n),0) FROM (
        SELECT input_tokens i,output_tokens o,1 n FROM proxy_request_logs
        UNION ALL SELECT input_tokens,output_tokens,request_count FROM session_usage_archive)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}
#[test]
#[serial]
fn semantics_history_gemini_scanned_zero_is_not_proof_of_archival() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let at = (chrono::Utc::now() - chrono::Duration::days(40)).to_rfc3339();
    let path = home.home().join(".gemini/tmp/p/chats/session-s.json");
    let body = |input, output| {
        json!({"sessionId":"s","messages":[
        {"id":"m0","type":"gemini","model":"fixture","timestamp":at,"tokens":{"input":input,"output":output}},
        {"id":"m1","type":"gemini","model":"fixture","timestamp":at,"tokens":{"input":100,"output":10}}]}).to_string()
    };
    write(&path, &body(0, 0), 0);
    good(gemini::sync_gemini_usage(&db));
    db.rollup_and_prune(30).unwrap();
    assert_eq!(archived_total(&db), (100, 10, 1));
    assert!(facts(&db).is_empty());
    write(&path, &body(30, 3), 1);
    good(gemini::sync_gemini_usage(&db));
    assert_eq!(archived_total(&db), (130, 13, 2));
    assert_eq!(
        facts(&db),
        vec![("gemini_session:s:m0".into(), [30, 3, 0, 0])]
    );
    assert_native_proof(&db, "gemini", &["m0", "m1"]);
    db.rollup_and_prune(30).unwrap();
    write(&path, &body(30, 3), 2);
    good(gemini::sync_gemini_usage(&db));
    assert!(facts(&db).is_empty());
    assert_eq!(archived_total(&db), (130, 13, 2));
}
fn opencode_old_zero(use_db: bool, migrate: bool) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let base = home.home().join(".local/share/opencode");
    let old = (chrono::Utc::now() - chrono::Duration::days(40)).timestamp_millis();
    let message = |id: &str, input, output| {
        json!({"id":id,"sessionID":"s","role":"assistant","modelID":"fixture",
        "tokens":{"input":input,"output":output},"time":{"created":old,"completed":old+1}})
    };
    let store = |database: bool, revised: bool| {
        let messages = [
            message(
                "m0",
                if revised { 30 } else { 0 },
                if revised { 3 } else { 0 },
            ),
            message("m1", 100, 10),
        ];
        if database {
            fs::create_dir_all(&base).unwrap();
            let conn = Connection::open(base.join("opencode.db")).unwrap();
            conn.execute_batch("CREATE TABLE IF NOT EXISTS session(id TEXT PRIMARY KEY,parent_id TEXT); CREATE TABLE IF NOT EXISTS message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT); DELETE FROM session; DELETE FROM message; INSERT INTO session VALUES('s',NULL);").unwrap();
            for message in messages {
                conn.execute(
                    "INSERT INTO message VALUES(?1,'s',?2,?3,?4)",
                    params![
                        message["id"].as_str().unwrap(),
                        old,
                        if revised {
                            chrono::Utc::now().timestamp_millis()
                        } else {
                            old
                        },
                        message.to_string()
                    ],
                )
                .unwrap();
            }
        } else {
            for message in messages {
                write(
                    &base.join(format!(
                        "storage/message/s/{}.json",
                        message["id"].as_str().unwrap()
                    )),
                    &message.to_string(),
                    u64::from(revised),
                );
            }
        }
    };
    store(use_db, false);
    good(opencode::sync_opencode_usage(&db));
    db.rollup_and_prune(30).unwrap();
    assert_eq!(archived_total(&db), (100, 10, 1));
    store(use_db || migrate, true);
    good(opencode::sync_opencode_usage(&db));
    assert_eq!(archived_total(&db), (130, 13, 2));
    assert_eq!(facts(&db), vec![("opencode:m0".into(), [30, 3, 0, 0])]);
    assert_native_proof(&db, "opencode", &["m0", "m1"]);
    db.rollup_and_prune(30).unwrap();
    store(use_db || migrate, true);
    good(opencode::sync_opencode_usage(&db));
    assert!(facts(&db).is_empty());
    assert_eq!(archived_total(&db), (130, 13, 2));
}
#[test]
#[serial]
fn semantics_history_opencode_db_scanned_zero_is_not_proof_of_archival() {
    opencode_old_zero(true, false);
}
#[test]
#[serial]
fn semantics_history_opencode_json_scanned_zero_is_not_proof_of_archival() {
    opencode_old_zero(false, false);
}
#[test]
#[serial]
fn semantics_history_opencode_json_migration_does_not_invent_receipts() {
    opencode_old_zero(false, true);
}

fn assert_native_proof(db: &Database, provider: &str, ids: &[&str]) {
    let conn = db.conn.lock().unwrap();
    let raw: Vec<String> = conn
        .prepare("SELECT meta_json FROM session_meta_cache WHERE provider_id=?1")
        .unwrap()
        .query_map([provider], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut acks = std::collections::BTreeSet::new();
    for raw in raw {
        let value: Value = serde_json::from_str(&raw).unwrap();
        let evidence = &value["parts"]["usageSync"]["value"];
        assert_eq!(evidence["trusted"], true);
        acks.extend(
            evidence["acknowledgedEventIds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned()),
        );
    }
    for id in ids {
        let native = super::usage_source::native_key(
            provider,
            if provider == "opencode" { "" } else { "s" },
            id,
        );
        assert!(
            acks.contains(&super::usage_source::native_digest(&native)),
            "{provider} {id} missing committed proof"
        );
    }
}
fn warm_jsonl(no_codex: bool) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    for n in 0..600 {
        let path = if no_codex {
            claude_path(home.home()).with_file_name(format!("s{n}.jsonl"))
        } else {
            codex_path(home.home()).with_file_name(format!("rollout-{n}.jsonl"))
        };
        let body = if no_codex {
            jsonl(&[c(&format!("m{n}"), [2, 1, 0, 0])])
        } else {
            codex_body(&[t(1, 2, 1, 0)])
        };
        write(&path, &body, 0);
    }
    let sync = |db: &Database| {
        if no_codex {
            claude::sync_claude_session_logs(db)
        } else {
            codex::sync_codex_usage(db)
        }
    };
    assert_eq!(good(sync(&db)).imported, 600);
    let before = super::usage_source::body_reads_under(home.home());
    assert!(before >= 600);
    assert_eq!(good(sync(&db)).imported, 0);
    assert_eq!(
        super::usage_source::body_reads_under(home.home()),
        before,
        "unchanged files reread their bodies"
    );
}
#[test]
#[serial]
fn semantics_warm_600_claude_files_do_not_read_bodies() {
    warm_jsonl(true);
}
#[test]
#[serial]
fn semantics_warm_600_codex_files_do_not_read_bodies() {
    warm_jsonl(false);
}
#[test]
#[serial]
fn semantics_codex_empty_total_falls_back_without_taking_a_legacy_slot() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    write(
        &codex_path(home.home()),
        &codex_body(&[
            x(
                1,
                json!({"total_token_usage":{},"last_token_usage":total(30,3,0)}),
            ),
            l(2, 20, 2, 0),
        ]),
        0,
    );
    good(codex::sync_codex_usage(&db));
    let rows = facts(&db);
    assert_eq!(buckets(&rows), vec![[30, 3, 0, 0], [20, 2, 0, 0]]);
    // The old first-total path *did* use last even though total was empty,
    // hence these two events already owned the old N1/N2 slots.
    assert_eq!(
        rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        vec!["codex_session:s:rollout-s:1", "codex_session:s:rollout-s:2"]
    );
}
#[test]
#[serial]
fn semantics_codex_repeated_total_does_not_rewind_last_only_contributions() {
    let home = sandbox();
    check_codex(
        home.home(),
        &[
            t(1, 100, 10, 0),
            l(2, 20, 2, 0),
            t(3, 100, 10, 0),
            t(4, 150, 15, 0),
        ],
        &[[100, 10, 0, 0], [20, 2, 0, 0], [30, 3, 0, 0]],
        &[1, 2, 3],
    );
}
#[test]
#[serial]
fn semantics_codex_missing_anchor_or_reset_is_visible_and_keeps_checkpoint() {
    for events in [
        vec![l(1, 20, 2, 0), t(2, 100, 10, 0)],
        vec![t(1, 100, 10, 0), t(2, 90, 9, 0)],
    ] {
        let home = sandbox();
        let db = Database::memory().unwrap();
        write(&codex_path(home.home()), &codex_body(&events), 0);
        assert!(!codex::sync_codex_usage(&db).unwrap().errors.is_empty());
        assert!(facts(&db).is_empty());
        assert!(checkpoint(&db).is_empty());
    }
}

fn codex_rewritten_source(mode: &str) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = codex_path(home.home());
    write(
        &path,
        &codex_body(&[t(1, 10, 1, 0), t(2, 20, 2, 0), t(3, 30, 3, 0)]),
        0,
    );
    good(codex::sync_codex_usage(&db));
    assert_eq!(facts(&db).len(), 3);
    let mut lines = vec![
        json!({"type":"session_meta","payload":{"id":"replacement"}}),
        t(1, 40, 4, 0),
    ];
    if mode == "inline" {
        lines.extend([t(2, 50, 5, 0), t(3, 60, 6, 0)]);
    }
    if mode == "replace" {
        let replacement = path.with_extension("replacement");
        write(&replacement, &jsonl(&lines), 1);
        fs::rename(replacement, &path).unwrap();
    } else {
        write(&path, &jsonl(&lines), 1);
    }
    good(codex::sync_codex_usage(&db));
    assert!(
        facts(&db)
            .iter()
            .any(|r| r.0 == "codex_session:replacement:rollout-s:1"),
        "{mode} skipped a recent new identity before the old checkpoint"
    );
    assert_codex_partial(&db, "legacyLiveReconciliationRequired");
    assert_eq!(good(codex::sync_codex_usage(&db)).imported, 0);
}
#[test]
#[serial]
fn semantics_rewrite_codex_replaced_inode() {
    codex_rewritten_source("replace");
}
#[test]
#[serial]
fn semantics_rewrite_codex_truncated_file() {
    codex_rewritten_source("truncate");
}
#[test]
#[serial]
fn semantics_rewrite_codex_in_place_new_identity() {
    codex_rewritten_source("inline");
}
fn assert_codex_partial(db: &Database, reason: &str) {
    let raw:String=db.conn.lock().unwrap().query_row("SELECT meta_json FROM session_meta_cache WHERE provider_id='codex' ORDER BY source_path LIMIT 1",[],|r|r.get(0)).unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    let evidence = &value["parts"]["usageSync"]["value"];
    assert_eq!(evidence["coverage"], "partial");
    assert_eq!(evidence["completeEventIdentitySet"], false);
    assert!(
        evidence["acknowledgedEventIds"]
            .as_array()
            .unwrap()
            .is_empty(),
        "legacy ordinals must not become native ack"
    );
    assert!(
        evidence["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == reason),
        "{value}"
    );
}
#[test]
#[serial]
fn semantics_rewrite_codex_same_n_keeps_old_fact_and_marks_reconciliation() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = codex_path(home.home());
    write(&path, &codex_body(&[t(1, 10, 1, 0)]), 0);
    good(codex::sync_codex_usage(&db));
    let old = facts(&db);
    write(&path, &codex_body(&[t(1, 50, 5, 0)]), 1);
    good(codex::sync_codex_usage(&db));
    assert_eq!(facts(&db), old);
    assert_codex_partial(&db, "legacyLiveReconciliationRequired");
}
#[test]
#[serial]
fn semantics_alias_claude_native_id_is_global_even_if_session_header_changes() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = claude_path(home.home());
    let mut line = c("global", [100, 10, 0, 0]);
    line["timestamp"] = json!((chrono::Utc::now() - chrono::Duration::days(40)).to_rfc3339());
    write(&path, &jsonl(&[line.clone()]), 0);
    good(claude::sync_claude_session_logs(&db));
    db.rollup_and_prune(30).unwrap();
    line["sessionId"] = json!("renamed-session");
    write(&path, &jsonl(&[line]), 1);
    good(claude::sync_claude_session_logs(&db));
    assert_eq!(archived_total(&db), (100, 10, 1));
    assert!(facts(&db).is_empty());
}
fn normalized_scope(source: Source, home: &Path) -> (&'static str, PathBuf) {
    match source {
        Source::Claude => ("claude", claude_path(home)),
        Source::Codex => ("codex", codex_path(home)),
        Source::Gemini => ("gemini", home.join(".gemini/tmp/p/chats/session-s.json")),
        Source::OpenCodeDb => ("opencode", home.join(".local/share/opencode/opencode.db")),
        Source::OpenCodeJson => ("opencode", home.join(".local/share/opencode/storage")),
    }
}
fn normalized_contract(source: Source) {
    let home = sandbox();
    source.write(home.home(), [200, 20, 30, 10, 2], 0);
    let (provider, path) = normalized_scope(source, home.home());
    let before = super::usage_source::body_reads_under(home.home());
    let stamp = super::usage_source::normalized_stamp(provider, &path).unwrap();
    assert_eq!(super::usage_source::body_reads_under(home.home()), before);
    let snapshot = super::usage_source::read_normalized(provider, &path).unwrap();
    assert_eq!(snapshot.source_stamp, stamp);
    assert_eq!(snapshot.facts.len(), 2);
    assert!(snapshot.facts.iter().all(|f| f.at_ms.is_some()));
    assert!(snapshot
        .facts
        .iter()
        .all(|f| !f.request_id.is_empty() && f.session_id.as_deref() == Some("s")));
    if matches!(source, Source::Codex) {
        assert!(!snapshot.complete);
        assert!(snapshot.facts.iter().all(|f| f.native_key.is_none()));
    } else {
        assert!(snapshot.complete);
        assert!(snapshot.facts.iter().all(|f| f.native_key.is_some()));
    }
    let read = super::usage_source::body_reads_under(home.home());
    assert_eq!(
        super::usage_source::normalized_stamp(provider, &path).unwrap(),
        stamp
    );
    assert_eq!(super::usage_source::body_reads_under(home.home()), read);
}
macro_rules! normalized_case {
    ($name:ident,$source:expr) => {
        #[test]
        #[serial]
        fn $name() {
            normalized_contract($source);
        }
    };
}
normalized_case!(semantics_normalized_claude, Source::Claude);
normalized_case!(semantics_normalized_codex, Source::Codex);
normalized_case!(semantics_normalized_gemini, Source::Gemini);
normalized_case!(semantics_normalized_opencode_db, Source::OpenCodeDb);
normalized_case!(semantics_normalized_opencode_json, Source::OpenCodeJson);
#[test]
#[serial]
fn semantics_normalized_missing_timestamp_remains_none_and_errors_are_visible() {
    let home = sandbox();
    let path = claude_path(home.home());
    let mut line = c("undated", [10, 1, 0, 0]);
    line.as_object_mut().unwrap().remove("timestamp");
    write(&path, &jsonl(&[line]), 0);
    let snapshot = super::usage_source::read_normalized("claude", &path).unwrap();
    assert_eq!(snapshot.facts[0].at_ms, None);
    assert!(!snapshot.complete);
    assert!(snapshot.reasons.contains("missingTimestamp"));
    write(
        &path,
        &jsonl(&[c("overflow", [u32::MAX as u64 + 1, 1, 0, 0])]),
        1,
    );
    assert!(super::usage_source::read_normalized("claude", &path).is_err());
}

#[test]
#[serial]
fn semantics_codex_legacy_seed_keeps_old_slots_and_never_native_acknowledges_them() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = codex_path(home.home());
    let old = (chrono::Utc::now() - chrono::Duration::days(40)).to_rfc3339();
    let mut first = t(1, 100, 10, 0);
    first["timestamp"] = json!(old);
    write(
        &path,
        &codex_body(&[first, l(2, 20, 2, 0), t(3, 120, 12, 0), t(4, 150, 15, 0)]),
        0,
    );
    {
        let conn = db.conn.lock().unwrap();
        conn.execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source)VALUES('codex_session:s:1','_codex_session','codex','fixture',100,10,0,200,'s',?1,'codex_session')",[chrono::DateTime::parse_from_rfc3339(&old).unwrap().timestamp()]).unwrap();
        conn.execute(
            "INSERT INTO session_log_sync VALUES(?1,1,2,1)",
            [path.to_string_lossy()],
        )
        .unwrap();
    }
    good(codex::sync_codex_usage(&db));
    assert_eq!(
        facts(&db).iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        vec![
            "codex_session:s:1",
            "codex_session:s:rollout-s:2",
            "codex_session:s:rollout-s:4"
        ]
    );
    assert_codex_partial(&db, "legacyLiveReconciliationRequired");
    db.rollup_and_prune(30).unwrap();
    let archive = home.home().join(".codex/archived_sessions/rollout-s.jsonl");
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    fs::rename(path, &archive).unwrap();
    good(codex::sync_codex_usage(&db));
    assert_eq!(archived_total(&db), (150, 15, 3));
    db.conn
        .lock()
        .unwrap()
        .execute("DELETE FROM session_log_sync", [])
        .unwrap();
    good(codex::sync_codex_usage(&db));
    assert_eq!(archived_total(&db), (150, 15, 3));
}
#[test]
#[serial]
fn semantics_codex_pruned_rewrite_does_not_reset_its_unknown_history_boundary_on_the_next_append() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = codex_path(home.home());
    let old = (chrono::Utc::now() - chrono::Duration::days(40)).to_rfc3339();
    let event = |n, input, output| {
        let mut v = t(n, input, output, 0);
        v["timestamp"] = json!(old);
        v
    };
    write(
        &path,
        &codex_body(&[event(1, 100, 10), event(2, 200, 20)]),
        0,
    );
    good(codex::sync_codex_usage(&db));
    db.rollup_and_prune(30).unwrap();
    let header = json!({"type":"session_meta","payload":{"id":"replacement"}});
    write(&path, &jsonl(&[header.clone(), event(1, 30, 3)]), 1);
    good(codex::sync_codex_usage(&db));
    write(
        &path,
        &jsonl(&[header, event(1, 30, 3), event(2, 60, 6)]),
        2,
    );
    good(codex::sync_codex_usage(&db));
    assert_eq!(archived_total(&db), (200, 20, 2));
    assert!(facts(&db).is_empty());
    assert_codex_partial(&db, "legacyLiveReconciliationRequired");
}

#[test]
#[serial]
fn evidence_missing_claude_message_id_cannot_be_complete_native_coverage() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let mut line = c("missing", [10, 1, 0, 0]);
    line["message"].as_object_mut().unwrap().remove("id");
    write(&claude_path(home.home()), &jsonl(&[line]), 0);
    good(claude::sync_claude_session_logs(&db));
    assert_partial_without_ack(&db, "claude");
}
#[test]
#[serial]
fn evidence_missing_gemini_session_id_cannot_acknowledge_the_unknown_namespace() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = home.home().join(".gemini/tmp/p/chats/session-s.json");
    write(&path,&json!({"messages":[{"id":"m","type":"gemini","model":"fixture","timestamp":"2026-10-15T00:00:01Z","tokens":{"input":10,"output":1}}]}).to_string(),0);
    good(gemini::sync_gemini_usage(&db));
    assert_partial_without_ack(&db, "gemini");
}
fn assert_partial_without_ack(db: &Database, provider: &str) {
    let raw: String = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT meta_json FROM session_meta_cache WHERE provider_id=?1",
            [provider],
            |r| r.get(0),
        )
        .unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    let evidence = &value["parts"]["usageSync"]["value"];
    assert_eq!(evidence["completeEventIdentitySet"], false);
    assert_eq!(evidence["coverage"], "partial");
    assert!(evidence["acknowledgedEventIds"]
        .as_array()
        .unwrap()
        .is_empty());
}

fn native_oc_message(id: &str, sid: &str, at: i64, input: u64, output: u64) -> Value {
    json!({"id":id,"sessionID":sid,"role":"assistant","modelID":"fixture",
        "tokens":{"input":input,"output":output},"time":{"created":at,"completed":at+1}})
}
fn native_oc_database(path: &Path) -> Connection {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY,parent_id TEXT); CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);").unwrap();
    conn
}
fn native_oc_insert(conn: &Connection, message: &Value, updated: i64) {
    conn.execute(
        "INSERT INTO message VALUES(?1,?2,?3,?4,?5)",
        params![
            message["id"].as_str().unwrap(),
            message["sessionID"].as_str().unwrap(),
            message["time"]["created"].as_i64().unwrap(),
            updated,
            message.to_string()
        ],
    )
    .unwrap();
}
#[test]
#[serial]
fn semantics_opencode_replaced_database_reads_native_ids_below_the_old_watermark() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = home.home().join(".local/share/opencode/opencode.db");
    let high = chrono::Utc::now().timestamp_millis();
    let conn = native_oc_database(&path);
    conn.execute("INSERT INTO session VALUES('s',NULL)", [])
        .unwrap();
    native_oc_insert(
        &conn,
        &native_oc_message("a", "s", high - 120_000, 100, 10),
        high,
    );
    drop(conn);
    good(opencode::sync_opencode_usage(&db));
    let replacement = path.with_extension("replacement");
    let conn = native_oc_database(&replacement);
    conn.execute("INSERT INTO session VALUES('s',NULL)", [])
        .unwrap();
    native_oc_insert(
        &conn,
        &native_oc_message("b", "s", high - 120_000, 30, 3),
        high - 60_000,
    );
    drop(conn);
    fs::rename(replacement, &path).unwrap();
    good(opencode::sync_opencode_usage(&db));
    assert!(
        facts(&db).iter().any(|r| r.0 == "opencode:b"),
        "changed inode kept a watermark from the replaced database"
    );
    assert_eq!(good(opencode::sync_opencode_usage(&db)).imported, 0);
}
#[test]
#[serial]
fn semantics_opencode_old_zero_below_the_watermark_is_not_permanently_skipped() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = home.home().join(".local/share/opencode/opencode.db");
    let high = chrono::Utc::now().timestamp_millis();
    let old = (chrono::Utc::now() - chrono::Duration::days(40)).timestamp_millis();
    let conn = native_oc_database(&path);
    conn.execute("INSERT INTO session VALUES('s',NULL)", [])
        .unwrap();
    native_oc_insert(
        &conn,
        &native_oc_message("m0", "s", old - 86_400_000, 0, 0),
        high - 60_000,
    );
    native_oc_insert(&conn, &native_oc_message("m1", "s", old, 100, 10), high);
    good(opencode::sync_opencode_usage(&db));
    db.rollup_and_prune(30).unwrap();
    conn.execute(
        "UPDATE message SET data=?1 WHERE id='m0'",
        [native_oc_message("m0", "s", old - 86_400_000, 30, 3).to_string()],
    )
    .unwrap();
    good(opencode::sync_opencode_usage(&db));
    assert_eq!(archived_total(&db), (130, 13, 2));
    db.rollup_and_prune(30).unwrap();
    conn.execute("UPDATE message SET data=data WHERE id='m0'", [])
        .unwrap();
    good(opencode::sync_opencode_usage(&db));
    assert_eq!(archived_total(&db), (130, 13, 2));
    assert!(facts(&db).is_empty());
}
fn opencode_parent_dependency(use_db: bool) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let base = home.home().join(".local/share/opencode");
    let old = (chrono::Utc::now() - chrono::Duration::days(40)).timestamp_millis();
    let high = chrono::Utc::now().timestamp_millis();
    let path = base.join("opencode.db");
    if use_db {
        let conn = native_oc_database(&path);
        conn.execute_batch(
            "INSERT INTO session VALUES('child',NULL),('root',NULL),('other',NULL);",
        )
        .unwrap();
        native_oc_insert(
            &conn,
            &native_oc_message("child-msg", "child", old, 10, 1),
            high - 60_000,
        );
        native_oc_insert(
            &conn,
            &native_oc_message("other-msg", "other", old, 20, 2),
            high,
        );
    } else {
        write(
            &base.join("storage/session/p/child.json"),
            &json!({"id":"child"}).to_string(),
            0,
        );
        write(
            &base.join("storage/message/child/child-msg.json"),
            &native_oc_message("child-msg", "child", old, 10, 1).to_string(),
            0,
        );
    }
    good(opencode::sync_opencode_usage(&db));
    let group = || {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT session_id FROM proxy_request_logs WHERE request_id='opencode:child-msg'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
    };
    assert_eq!(group(), "child");
    let reparent = |parent: &str| {
        if use_db {
            let conn = Connection::open(&path).unwrap();
            conn.execute("UPDATE session SET parent_id=?1 WHERE id='child'", [parent])
                .unwrap();
        } else {
            write(
                &base.join("storage/session/p/child.json"),
                &json!({"id":"child","parentID":parent}).to_string(),
                1,
            );
        }
    };
    reparent("root");
    good(opencode::sync_opencode_usage(&db));
    assert_eq!(
        group(),
        "root",
        "parent-only change was hidden by the message watermark/stamp"
    );
    db.rollup_and_prune(30).unwrap();
    reparent("another-root");
    good(opencode::sync_opencode_usage(&db));
    assert!(facts(&db).is_empty());
    let conn = db.conn.lock().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT request_count FROM session_usage_archive WHERE session_id='root'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "unproven archived attribution must remain in its original group"
    );
    let raw:String=conn.query_row("SELECT meta_json FROM session_meta_cache WHERE provider_id='opencode' ORDER BY source_path LIMIT 1",[],|r|r.get(0)).unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    let evidence = &value["parts"]["usageSync"]["value"];
    assert_eq!(evidence["coverage"], "partial");
    assert!(evidence["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r == "historicalSessionAncestryUnverified"));
}
#[test]
#[serial]
fn semantics_opencode_parent_only_database_change_refreshes_owned_live_attribution() {
    opencode_parent_dependency(true);
}
#[test]
#[serial]
fn semantics_opencode_parent_only_json_change_refreshes_owned_live_attribution() {
    opencode_parent_dependency(false);
}

fn opencode_legacy_dependency_receipt(use_db: bool) {
    use crate::session_manager::source_cache::{self, Part};
    let home = sandbox();
    let db = Database::memory().unwrap();
    let base = home.home().join(".local/share/opencode");
    let at = chrono::Utc::now().timestamp_millis();
    let source = if use_db {
        let path = base.join("opencode.db");
        let conn = native_oc_database(&path);
        conn.execute("INSERT INTO session VALUES('s',NULL)", [])
            .unwrap();
        native_oc_insert(&conn, &native_oc_message("m", "s", at, 10, 1), at);
        path
    } else {
        let dir = base.join("storage/message/s");
        write(
            &dir.join("m.json"),
            &native_oc_message("m", "s", at, 10, 1).to_string(),
            0,
        );
        dir
    };
    good(opencode::sync_opencode_usage(&db));
    let key = source.to_string_lossy();
    {
        let conn = db.conn.lock().unwrap();
        let parts = source_cache::read_on(&conn, &key, "opencode").unwrap();
        let mut value = parts.value(Part::UsageSync).unwrap().clone();
        // null and an absent legacy field both deserialize to None; explicitly
        // unset it because merge preserves fields omitted by an older writer.
        value["dependencyHash"] = Value::Null;
        // Simulate a valid older receipt, updating the roster through the same
        // merge API rather than making an invalid receipt digest by hand.
        source_cache::merge_on(
            &conn,
            &key,
            "opencode",
            None,
            Part::UsageSync,
            &parts.token(Part::UsageSync),
            Some(value),
        )
        .unwrap();
    }
    assert_eq!(good(opencode::sync_opencode_usage(&db)).imported, 0);
    let conn = db.conn.lock().unwrap();
    let parts = source_cache::read_on(&conn, &key, "opencode").unwrap();
    assert!(parts.value(Part::UsageSync).unwrap()["dependencyHash"].is_string());
    assert_eq!(facts_count(&conn), 1);
}
fn facts_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r.get(0))
        .unwrap()
}
#[test]
#[serial]
fn semantics_opencode_legacy_database_receipt_establishes_parent_dependency() {
    opencode_legacy_dependency_receipt(true);
}
#[test]
#[serial]
fn semantics_opencode_legacy_json_receipt_establishes_parent_dependency() {
    opencode_legacy_dependency_receipt(false);
}

fn missing_timestamp_is_not_today(provider: &str, invalid: bool) {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let mut call = match provider {
        "claude" => c("undated", [10, 1, 0, 0]),
        "codex" => t(1, 10, 1, 0),
        "gemini" => json!({"id":"undated","type":"gemini","model":"fixture",
            "tokens":{"input":10,"output":1}}),
        _ => unreachable!(),
    };
    if invalid {
        call["timestamp"] = json!("not-an-original-date");
    } else {
        call.as_object_mut().unwrap().remove("timestamp");
    }
    let (path, sync): (
        PathBuf,
        fn(&Database) -> Result<claude::SessionSyncResult, AppError>,
    ) = match provider {
        "claude" => (claude_path(home.home()), claude::sync_claude_session_logs),
        "codex" => (codex_path(home.home()), codex::sync_codex_usage),
        "gemini" => (
            home.home().join(".gemini/tmp/p/chats/session-s.json"),
            gemini::sync_gemini_usage,
        ),
        _ => unreachable!(),
    };
    let body = |call: &Value| match provider {
        "codex" => codex_body(std::slice::from_ref(call)),
        "gemini" => json!({"sessionId":"s","messages":[call]}).to_string(),
        _ => jsonl(std::slice::from_ref(call)),
    };
    write(&path, &body(&call), 0);
    good(sync(&db));
    assert!(
        facts(&db).is_empty(),
        "undated native usage was given a fabricated current date"
    );
    let snapshot = super::usage_source::read_normalized(provider, &path).unwrap();
    assert_eq!(snapshot.facts.len(), 1);
    assert_eq!(snapshot.facts[0].at_ms, None);
    assert!(snapshot.reasons.contains("missingTimestamp"));
    {
        let conn = db.conn.lock().unwrap();
        let raw: String = conn
            .query_row(
                "SELECT meta_json FROM session_meta_cache WHERE provider_id=?1",
                [provider],
                |r| r.get(0),
            )
            .unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        let evidence = &value["parts"]["usageSync"]["value"];
        assert_eq!(evidence["coverage"], "partial");
        assert!(evidence["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "missingTimestamp"));
        assert!(evidence["acknowledgedEventIds"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    let valid = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let expected = chrono::DateTime::parse_from_rfc3339(&valid)
        .unwrap()
        .timestamp();
    call["timestamp"] = json!(valid);
    write(&path, &body(&call), 1);
    assert_eq!(
        good(sync(&db)).imported,
        1,
        "later original timestamp must allow a normal import"
    );
    assert_eq!(facts(&db).len(), 1);
    let stored: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT created_at FROM proxy_request_logs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stored, expected);
    assert_eq!(good(sync(&db)).imported, 0);
}

#[test]
#[serial]
fn semantics_missing_claude_timestamp_is_not_today() {
    missing_timestamp_is_not_today("claude", false);
}
#[test]
#[serial]
fn semantics_invalid_claude_timestamp_is_not_today() {
    missing_timestamp_is_not_today("claude", true);
}
#[test]
#[serial]
fn semantics_missing_codex_timestamp_is_not_today() {
    missing_timestamp_is_not_today("codex", false);
}
#[test]
#[serial]
fn semantics_invalid_codex_timestamp_is_not_today() {
    missing_timestamp_is_not_today("codex", true);
}
#[test]
#[serial]
fn semantics_missing_gemini_timestamp_is_not_today() {
    missing_timestamp_is_not_today("gemini", false);
}
#[test]
#[serial]
fn semantics_invalid_gemini_timestamp_is_not_today() {
    missing_timestamp_is_not_today("gemini", true);
}

#[test]
#[serial]
fn semantics_legacy_changed_native_time_remains_partial_without_identity_proof() {
    let home = sandbox();
    let db = Database::memory().unwrap();
    let path = home.home().join(".local/share/opencode/opencode.db");
    let old = (chrono::Utc::now() - chrono::Duration::days(40)).timestamp_millis();
    let now = chrono::Utc::now().timestamp_millis();
    let conn = native_oc_database(&path);
    conn.execute("INSERT INTO session VALUES('s',NULL)", [])
        .unwrap();
    let mut message = native_oc_message("m", "s", old, 100, 10);
    message["time"]["completed"] = json!(0);
    native_oc_insert(&conn, &message, old);
    // A legacy ledger has this native contribution, but no event receipt. Its
    // aggregate cannot prove which ID supplied the old tokens after pruning.
    db.conn.lock().unwrap().execute("INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,output_tokens,latency_ms,status_code,session_id,created_at,data_source) VALUES('opencode:m','_opencode_session','opencode','fixture',100,10,0,200,'s',?1,'opencode_session')", [old.div_euclid(1000)]).unwrap();
    db.rollup_and_prune(30).unwrap();
    message["time"]["completed"] = json!(now);
    conn.execute(
        "UPDATE message SET data=?1,time_updated=?2 WHERE id='m'",
        params![message.to_string(), now],
    )
    .unwrap();
    good(opencode::sync_opencode_usage(&db));
    // Compatibility admits a recent unknown event. This deliberately exposes
    // the legacy limitation instead of pretending a temporal bound deduped it.
    assert_eq!(archived_total(&db), (200, 20, 2));
    let conn = db.conn.lock().unwrap();
    let raw: String = conn
        .query_row(
            "SELECT meta_json FROM session_meta_cache WHERE provider_id='opencode'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let cache: Value = serde_json::from_str(&raw).unwrap();
    let evidence = &cache["parts"]["usageSync"]["value"];
    assert_eq!(evidence["coverage"], "partial");
    assert_eq!(evidence["completeEventIdentitySet"], false);
    let manifest = crate::session_manager::source_cache::provider_on(&conn, "opencode")
        .unwrap()
        .unwrap();
    assert_eq!(manifest["originEmpty"], false);
    assert_eq!(manifest["complete"], false);
}
