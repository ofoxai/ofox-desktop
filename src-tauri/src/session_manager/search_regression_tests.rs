use super::{managed, providers, read_context, search};
use crate::test_sandbox::sandbox;

#[test]
#[serial_test::serial]
fn archived_codex_history_is_readable_without_mutating_the_native_archive() {
    let home = sandbox();
    let archive = home.home().join(".codex/archived_sessions");
    std::fs::create_dir_all(&archive).unwrap();
    let path = archive.join("rollout-11111111-2222-3333-4444-555555555555.jsonl");
    let body = concat!(
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"11111111-2222-3333-4444-555555555555\",\"cwd\":\"/work/archive\"}}\n",
        "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Find the archived decision\"}]}}\n"
    );
    std::fs::write(&path, body).unwrap();
    managed::forget_listing();
    let rows = managed::list();
    let row = rows
        .iter()
        .find(|row| row.meta.session_id == "11111111-2222-3333-4444-555555555555")
        .expect("native archived history must appear in the session manager");
    assert!(row.capabilities.can_read_messages);
    assert!(row.capabilities.can_export);
    assert!(!row.capabilities.can_resume);
    assert!(!row.capabilities.can_trash);
    let exported = managed::export_markdown(&row.session_key, None).unwrap();
    assert!(exported.content.contains("Find the archived decision"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
    assert_eq!(providers::codex::scan_sessions().len(), 1);
}

#[test]
fn opencode_equal_timestamp_messages_and_parts_have_stable_native_order() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("opencode.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE message (id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,data TEXT); CREATE TABLE part (id TEXT PRIMARY KEY,session_id TEXT,message_id TEXT,time_created INTEGER,data TEXT);").unwrap();
    for (id, text) in [("msg_z", "later"), ("msg_a", "first")] {
        conn.execute(
            "INSERT INTO message VALUES (?1,'ses_a',1,'{\"role\":\"user\"}')",
            [id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES (?1,'ses_a',?2,1,?3)",
            rusqlite::params![
                format!("{id}_z"),
                id,
                serde_json::json!({"type":"text","text":format!("{text}-z")}).to_string()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES (?1,'ses_a',?2,1,?3)",
            rusqlite::params![
                format!("{id}_a"),
                id,
                serde_json::json!({"type":"text","text":format!("{text}-a")}).to_string()
            ],
        )
        .unwrap();
    }
    drop(conn);
    let rows =
        providers::opencode::load_messages_sqlite(&format!("sqlite:{}:ses_a", path.display()))
            .unwrap();
    assert_eq!(rows[0].content, "first-a\nfirst-z");
    assert_eq!(rows[1].content, "later-a\nlater-z");
}

fn claude(home: &std::path::Path, number: usize, extra: &str) -> std::path::PathBuf {
    let directory = home.join(".claude/projects/-work-search");
    std::fs::create_dir_all(&directory).unwrap();
    let id = format!("11111111-2222-3333-4444-{number:012}");
    let path = directory.join(format!("{id}.jsonl"));
    let body = format!(
        "{}\n{}\n{extra}",
        serde_json::json!({"type":"user","sessionId":id,"cwd":"/work/search","message":{"role":"user","content":"A harmless session title"}}),
        serde_json::json!({"type":"assistant","message":{"role":"assistant","content":"The body contains NEEDLE and 中文 evidence."}})
    );
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
#[serial_test::serial]
fn body_search_finds_normalized_content_and_rejects_changed_hit_locations() {
    let home = sandbox();
    let path = claude(home.home(), 1, "");
    managed::forget_listing();
    let rows = managed::list();
    let key = rows[0].session_key.clone();
    assert!(!rows[0].meta.title.as_deref().unwrap().contains("NEEDLE"));
    let prepared = search::prepare(
        "request-body".into(),
        "needle".into(),
        vec![key.clone()],
        false,
    )
    .unwrap();
    let result = search::run(&prepared.operation_id, None, |_| {}).unwrap();
    assert_eq!(result.state, "complete");
    assert_eq!(result.progress.request_id, "request-body");
    let hit = &result.progress.hits[0];
    assert_eq!(hit.message_ordinal, 1);
    assert!(hit.snippet.contains("NEEDLE"));
    search::transcript(
        &key,
        Some(&hit.read_revision),
        Some(hit.message_ordinal),
        Some(&hit.content_hash),
    )
    .unwrap();
    assert!(search::transcript(
        &key,
        Some(&hit.read_revision),
        Some(0),
        Some(&hit.content_hash)
    )
    .unwrap_err()
    .contains("session.search.changed"));
    use std::io::Write;
    writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(),"{}",serde_json::json!({"type":"assistant","message":{"role":"assistant","content":"new material"}})).unwrap();
    assert!(search::transcript(
        &key,
        Some(&hit.read_revision),
        Some(hit.message_ordinal),
        Some(&hit.content_hash)
    )
    .unwrap_err()
    .contains("session.search.changed"));
}

#[test]
#[serial_test::serial]
fn body_search_is_explicit_cancellable_and_excludes_process_output_by_default() {
    let home = sandbox();
    let tool = format!(
        "{}\n",
        serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"process-secret-marker"}]}})
    );
    claude(home.home(), 1, &tool);
    managed::forget_listing();
    let key = managed::list()[0].session_key.clone();
    let prepared = search::prepare(
        "dialogue".into(),
        "process-secret-marker".into(),
        vec![key.clone()],
        false,
    )
    .unwrap();
    let result = search::run(&prepared.operation_id, None, |_| {}).unwrap();
    assert!(result.progress.hits.is_empty());
    let prepared = search::prepare(
        "process".into(),
        "process-secret-marker".into(),
        vec![key.clone()],
        true,
    )
    .unwrap();
    let events = std::sync::Mutex::new(Vec::new());
    // The event callback is deliberately ordinary Fn, as Tauri emission is.
    let result = search::run(&prepared.operation_id, None, |event| {
        events.lock().unwrap().push(event)
    })
    .unwrap();
    assert_eq!(result.progress.hits.len(), 1);
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .all(|event| event.request_id == "process" && event.operation_id == prepared.operation_id));
    let cancelled = search::prepare("cancel".into(), "needle".into(), vec![key], false).unwrap();
    assert!(search::cancel(&cancelled.operation_id));
    let result = search::run(&cancelled.operation_id, None, |_| {}).unwrap();
    assert_eq!(result.state, "cancelled");
    assert_eq!(result.progress.scanned, 0);
    assert!(!search::cancel(&cancelled.operation_id));
}

#[test]
#[serial_test::serial]
fn a_file_changed_after_search_preparation_is_partial_not_an_empty_success() {
    let home = sandbox();
    let path = claude(home.home(), 1, "");
    managed::forget_listing();
    let key = managed::list()[0].session_key.clone();
    let prepared = search::prepare("changed".into(), "needle".into(), vec![key], false).unwrap();
    std::fs::write(path, "a different native file").unwrap();
    let result = search::run(&prepared.operation_id, None, |_| {}).unwrap();
    assert_eq!(result.state, "limited");
    assert_eq!(result.progress.changed, 1);
    assert!(result.progress.hits.is_empty());
}

#[test]
fn decoded_and_line_budgets_and_mid_read_replacement_fail_closed() {
    use std::{io::Read, sync::atomic::Ordering};
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("rollout.jsonl.zst");
    let content = "x".repeat(1024);
    std::fs::write(
        &path,
        zstd::stream::encode_all(content.as_bytes(), 1).unwrap(),
    )
    .unwrap();
    let context = read_context::ReadContext::limited(vec![home.path().to_owned()], 64, 4096);
    let failure = read_context::scope(context, || read_context::read_text(&path)).unwrap_err();
    assert!(failure.contains("sessionLimit"));
    let context = read_context::ReadContext::limited(vec![home.path().to_owned()], 4096, 16);
    assert!(
        read_context::scope(context, || read_context::read_text(&path))
            .unwrap_err()
            .contains("lineLimit")
    );
    let context = read_context::ReadContext::limited(vec![home.path().to_owned()], 4096, 4096);
    let failure = read_context::scope(context.clone(), || {
        let mut reader = read_context::open(&path)?;
        let mut byte = [0];
        reader
            .read_exact(&mut byte)
            .map_err(|error| error.to_string())?;
        context.cancel.store(true, Ordering::Release);
        reader
            .read_exact(&mut byte)
            .map_err(|error| error.to_string())
    })
    .unwrap_err();
    assert!(failure.contains("cancelled"));
    let plain = home.path().join("plain.jsonl");
    std::fs::write(&plain, "before").unwrap();
    let context = read_context::ReadContext::limited(vec![home.path().to_owned()], 4096, 4096);
    read_context::scope(context.clone(), || read_context::read_text(&plain)).unwrap();
    std::fs::write(&plain, "after").unwrap();
    assert!(context.verify().unwrap_err().contains("changed"));
}

#[test]
fn search_projections_preserve_gemini_rewind_and_pi_active_branch() {
    let home = tempfile::tempdir().unwrap();
    let gemini = home.path().join("gemini.jsonl");
    let records = [
        serde_json::json!({"sessionId":"s"}),
        serde_json::json!({"id":"a","type":"user","content":"kept"}),
        serde_json::json!({"id":"b","type":"gemini","content":"discarded-needle"}),
        serde_json::json!({"$rewindTo":"b"}),
    ];
    std::fs::write(
        &gemini,
        records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let context = read_context::ReadContext::limited(vec![home.path().to_owned()], 4096, 4096);
    let messages =
        read_context::scope(context, || providers::gemini::load_messages(&gemini)).unwrap();
    assert!(!messages
        .iter()
        .any(|message| message.content.contains("discarded-needle")));
    let pi = home.path().join("pi.jsonl");
    let records = [
        serde_json::json!({"type":"session","id":"s"}),
        serde_json::json!({"type":"message","id":"a","message":{"role":"user","content":"kept"}}),
        serde_json::json!({"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":"discarded-needle"}}),
        serde_json::json!({"type":"message","id":"c","parentId":"a","message":{"role":"assistant","content":"current branch"}}),
    ];
    std::fs::write(
        &pi,
        records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let context = read_context::ReadContext::limited(vec![home.path().to_owned()], 4096, 4096);
    let messages = read_context::scope(context, || providers::pi::load_messages(&pi)).unwrap();
    assert!(!messages
        .iter()
        .any(|message| message.content.contains("discarded-needle")));
}

#[test]
#[serial_test::serial]
fn native_archive_prefix_copies_export_once_and_block_mutation_of_both_copies() {
    let home = sandbox();
    let root = home.home().join(".codex");
    std::fs::create_dir_all(root.join("sessions")).unwrap();
    std::fs::create_dir_all(root.join("archived_sessions")).unwrap();
    let body = concat!("{\"type\":\"session_meta\",\"payload\":{\"id\":\"11111111-2222-3333-4444-555555555555\",\"cwd\":\"/work/archive\"}}\n", "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"only-once\"}]}}\n");
    let name = "rollout-11111111-2222-3333-4444-555555555555.jsonl";
    std::fs::write(root.join("sessions").join(name), body).unwrap();
    std::fs::write(root.join("archived_sessions").join(name),format!("{body}{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"archive tail\"}}]}}}}\n")).unwrap();
    managed::forget_listing();
    let rows = managed::list();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].capabilities.can_trash);
    let exported = managed::export_markdown(&rows[0].session_key, None).unwrap();
    assert_eq!(exported.content.matches("\nonly-once\n").count(), 1);
    assert!(exported.content.contains("archive tail"));
    assert!(super::trash::plan(&rows[0].meta).is_err());
}

#[test]
fn completed_legacy_file_reads_release_handles_before_the_session_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let context = read_context::ReadContext::limited(vec![directory.path().to_owned()], 4096, 4096);
    for number in 0..32 {
        let file = directory.path().join(format!("{number}.json"));
        std::fs::write(&file, "small native part").unwrap();
        assert_eq!(
            read_context::scope(context.clone(), || read_context::read_text(&file)).unwrap(),
            "small native part"
        );
    }
    assert!(
        context.retained_file_handles() <= 2,
        "a large legacy session must not retain a file descriptor for every completed part"
    );
    context.verify().unwrap();
}

#[test]
#[serial_test::serial]
fn copied_archive_metadata_does_not_present_unverified_message_counts() {
    let home = sandbox();
    let root = home.home().join(".codex");
    std::fs::create_dir_all(root.join("sessions")).unwrap();
    std::fs::create_dir_all(root.join("archived_sessions")).unwrap();
    let body = concat!("{\"type\":\"session_meta\",\"payload\":{\"id\":\"11111111-2222-3333-4444-555555555555\"}}\n", "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"one native prompt\"}]}}\n");
    let name = "rollout-11111111-2222-3333-4444-555555555555.jsonl";
    std::fs::write(root.join("sessions").join(name), body).unwrap();
    std::fs::write(root.join("archived_sessions").join(name), body).unwrap();
    managed::forget_listing();
    let rows = managed::list();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].meta.message_count, None,
        "metadata cache has no prefix proof; it must not add two copies of one prompt"
    );
}

#[test]
#[serial_test::serial]
fn a_bounded_search_can_continue_without_repeating_already_scanned_sessions() {
    let home = sandbox();
    let extra = format!(
        "{}\n{}\n",
        serde_json::json!({"type":"assistant","message":{"role":"assistant","content":"needle second"}}),
        serde_json::json!({"type":"assistant","message":{"role":"assistant","content":"needle third"}})
    );
    for number in 0..68 {
        claude(home.home(), number, &extra);
    }
    managed::forget_listing();
    let keys = managed::list()
        .into_iter()
        .map(|row| row.session_key)
        .collect();
    let prepared = search::prepare("pages".into(), "needle".into(), keys, false).unwrap();
    let first = search::run(&prepared.operation_id, None, |_| {}).unwrap();
    assert_eq!(first.state, "limited");
    assert!(first.progress.hits.len() <= 200);
    let next = first.next_cursor.unwrap();
    assert!(search::run(&prepared.operation_id, Some("untrusted cursor"), |_| {}).is_err());
    let second = search::run(&prepared.operation_id, Some(&next), |_| {}).unwrap();
    assert_eq!(second.state, "complete");
    assert_eq!(second.progress.scanned, 68);
    let keys: std::collections::HashSet<_> = first
        .progress
        .hits
        .iter()
        .map(|hit| (&hit.session_key, hit.message_ordinal))
        .collect();
    assert!(second
        .progress
        .hits
        .iter()
        .all(|hit| !keys.contains(&(&hit.session_key, hit.message_ordinal))));
    assert_eq!(first.progress.hits.len() + second.progress.hits.len(), 204);
}

#[test]
#[serial_test::serial]
fn sqlite_search_hits_have_repeatable_revisions_and_detect_native_updates() {
    let home = sandbox();
    let root = home.home().join(".local/share/opencode");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("opencode.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE session (id TEXT PRIMARY KEY,title TEXT,directory TEXT,time_created INTEGER,time_updated INTEGER); CREATE TABLE message (id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,data TEXT); CREATE TABLE part (id TEXT PRIMARY KEY,session_id TEXT,message_id TEXT,time_created INTEGER,data TEXT); INSERT INTO session VALUES('ses_a','Native SQLite','/work/project',1,1); INSERT INTO message VALUES('msg_a','ses_a',1,'{\"role\":\"assistant\"}'); INSERT INTO part VALUES('part_a','ses_a','msg_a',1,'{\"type\":\"text\",\"text\":\"needle sqlite body\"}');").unwrap();
    drop(conn);
    managed::forget_listing();
    let rows = managed::list();
    let row = rows
        .iter()
        .find(|row| row.meta.provider_id == "opencode")
        .unwrap();
    assert!(row.revision.is_none());
    let prepared = search::prepare(
        "sqlite".into(),
        "needle".into(),
        vec![row.session_key.clone()],
        false,
    )
    .unwrap();
    let result = search::run(&prepared.operation_id, None, |_| {}).unwrap();
    let hit = &result.progress.hits[0];
    search::transcript(
        &row.session_key,
        Some(&hit.read_revision),
        Some(hit.message_ordinal),
        Some(&hit.content_hash),
    )
    .unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE part SET data='{\"type\":\"text\",\"text\":\"replacement native material\"}'",
        [],
    )
    .unwrap();
    drop(conn);
    assert!(search::transcript(
        &row.session_key,
        Some(&hit.read_revision),
        Some(hit.message_ordinal),
        Some(&hit.content_hash)
    )
    .unwrap_err()
    .contains("session.search.changed"));
}

fn pi_fixture(home: &std::path::Path) -> std::path::PathBuf {
    let root = home.join(".pi/agent/sessions/project");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("native.jsonl");
    let rows = [
        serde_json::json!({"type":"session","id":"pi-native","cwd":"/work/pi"}),
        serde_json::json!({"type":"message","id":"a","message":{"role":"user","content":"needle in Pi"}}),
    ];
    std::fs::write(
        &path,
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
    path
}

#[test]
#[serial_test::serial]
fn search_never_prereads_pi_models_outside_the_budgeted_reader() {
    let home = sandbox();
    let path = pi_fixture(home.home());
    managed::forget_listing();
    let rows = managed::list();
    let row = rows
        .iter()
        .find(|row| row.meta.provider_id == "pi")
        .unwrap();
    use std::io::Write;
    writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(),"{}",serde_json::json!({"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":"x".repeat(read_context::LINE_BYTES+1)}})).unwrap();
    let key = row.session_key.clone();
    managed::age_listing_for_test();
    let result = read_context::with_open_observer(move |source,bounded| {
        if source == path { assert!(bounded,"Pi model discovery must not open transcript content before entering ReadContext"); }
    },|| {
        let prepared = search::prepare("pi-budget".into(),"needle".into(),vec![key],false).unwrap();
        search::run(&prepared.operation_id,None,|_|{})
    }).unwrap();
    assert_eq!(result.progress.failed, 1);
    assert!(result
        .limit_reasons
        .iter()
        .any(|reason| reason == "lineLimit"));
}

#[test]
#[serial_test::serial]
fn search_never_recursively_fingerprints_unrelated_session_auxiliary_files() {
    let home = sandbox();
    let path = claude(home.home(), 1, "");
    let auxiliary = path.with_extension("").join("tool-results/unrelated");
    std::fs::create_dir_all(&auxiliary).unwrap();
    for number in 0..64 {
        std::fs::write(auxiliary.join(format!("{number}.txt")), "unrelated result").unwrap();
    }
    managed::forget_listing();
    let key = managed::list()[0].session_key.clone();
    let before = super::trash::fingerprint_calls();
    let prepared =
        search::prepare("no-trash-walk".into(), "needle".into(), vec![key], false).unwrap();
    let result = search::run(&prepared.operation_id, None, |_| {}).unwrap();
    assert_eq!(result.progress.hits.len(), 1);
    assert_eq!(
        super::trash::fingerprint_calls(),
        before,
        "search must not plan trash or walk auxiliary directories"
    );
}

#[test]
fn compressed_input_without_decoded_output_still_consumes_a_budget() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("skippable.jsonl.zst");
    let mut frame = 0x184D2A50u32.to_le_bytes().to_vec();
    frame.extend_from_slice(&1024u32.to_le_bytes());
    frame.extend_from_slice(&[0; 1024]);
    frame.extend_from_slice(&zstd::stream::encode_all(&b""[..], 1).unwrap());
    std::fs::write(&path, frame).unwrap();
    let context = read_context::ReadContext::limited(vec![directory.path().to_owned()], 64, 4096);
    assert!(
        read_context::scope(context, || read_context::read_text(&path)).is_err(),
        "skippable frames must be charged even when the decoder emits no text"
    );
}

#[test]
fn compressed_transcripts_refuse_an_excessive_declared_window_before_allocation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("window.jsonl.zst");
    // A valid empty zstd frame declaring a 128MiB window, with a final empty
    // raw block. The transcript reader permits windows up to 8MiB (log 23).
    std::fs::write(
        &path,
        [0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x88, 0x01, 0x00, 0x00],
    )
    .unwrap();
    let context = read_context::ReadContext::limited(vec![directory.path().to_owned()], 4096, 4096);
    assert!(
        read_context::scope(context, || read_context::read_text(&path)).is_err(),
        "an excessive window declaration must not be accepted, including empty frames"
    );
}

#[cfg(unix)]
#[test]
fn sqlite_proofs_cannot_be_captured_after_opening_a_different_native_inode() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("native.db");
    let replacement = directory.path().join("replacement.db");
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute_batch("CREATE TABLE item (value TEXT); INSERT INTO item VALUES ('inode A');")
        .unwrap();
    let new = rusqlite::Connection::open(&replacement).unwrap();
    new.execute_batch("CREATE TABLE item (value TEXT); INSERT INTO item VALUES ('inode B');")
        .unwrap();
    drop(new);
    std::fs::rename(&path, directory.path().join("old.db")).unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    let context = read_context::ReadContext::limited(vec![directory.path().to_owned()], 4096, 4096);
    let result = read_context::scope(context.clone(), || {
        read_context::sqlite(&path, &old)?;
        old.query_row("SELECT value FROM item", [], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())
    });
    assert!(
        result.is_err() || context.verify().is_err(),
        "the open connection reads A; a proof captured from pathname B cannot certify it"
    );
}

#[test]
#[serial_test::serial]
fn secret_snippets_are_scrubbed_before_progress_and_result_delivery() {
    let home = sandbox();
    let secret = "sk-of-fake0000000000000000";
    let extra = format!(
        "{}\n",
        serde_json::json!({"type":"assistant","message":{"role":"assistant","content":format!("needle credential {secret}")}})
    );
    claude(home.home(), 1, &extra);
    managed::forget_listing();
    let key = managed::list()[0].session_key.clone();
    let prepared =
        search::prepare("safe-snippet".into(), "needle".into(), vec![key], false).unwrap();
    let events = std::sync::Mutex::new(Vec::new());
    let result = search::run(&prepared.operation_id, None, |event| {
        events.lock().unwrap().push(event)
    })
    .unwrap();
    assert!(!serde_json::to_string(&result).unwrap().contains(secret));
    assert!(!serde_json::to_string(&*events.lock().unwrap())
        .unwrap()
        .contains(secret));
    assert!(result
        .progress
        .hits
        .iter()
        .any(|hit| hit.snippet.contains(crate::redact::REDACTED)));
}

#[test]
#[serial_test::serial]
fn cancelling_during_text_matching_is_not_blocked_by_the_job_position_lock() {
    let home = sandbox();
    claude(home.home(), 1, "");
    managed::forget_listing();
    let key = managed::list()[0].session_key.clone();
    let prepared =
        search::prepare("cancel-match".into(), "needle".into(), vec![key], false).unwrap();
    let id = prepared.operation_id.clone();
    let result = search::with_match_observer(
        move || {
            let (send, receive) = std::sync::mpsc::channel();
            let id = id.clone();
            std::thread::spawn(move || {
                send.send(search::cancel(&id)).unwrap();
            });
            assert!(receive
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("cancel must complete while the matching phase is paused"));
        },
        || search::run(&prepared.operation_id, None, |_| {}),
    )
    .unwrap();
    assert_eq!(result.state, "cancelled");
    assert!(result.progress.hits.is_empty());
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn pi_symlink_replacement_is_rejected_without_opening_external_content() {
    let home = sandbox();
    let path = pi_fixture(home.home());
    managed::forget_listing();
    let rows = managed::list();
    let key = rows
        .iter()
        .find(|row| row.meta.provider_id == "pi")
        .unwrap()
        .session_key
        .clone();
    let prepared = search::prepare(
        "pi-symlink".into(),
        "needle".into(),
        vec![key.clone()],
        false,
    )
    .unwrap();
    let outside = tempfile::tempdir().unwrap();
    let external = outside.path().join("private.jsonl");
    std::fs::write(&external, "external material must not be opened").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&external, &path).unwrap();
    let result = read_context::with_open_observer(
        |_, _| panic!("symlink source must be rejected before opening any transcript"),
        || search::run(&prepared.operation_id, None, |_| {}),
    )
    .unwrap();
    assert_eq!(result.progress.changed, 1);
    assert!(search::transcript(&key, None, None, None).is_err());
}

#[test]
#[serial_test::serial]
fn cancelling_pi_at_the_native_open_boundary_prevents_a_preread() {
    let home = sandbox();
    let path = pi_fixture(home.home());
    managed::forget_listing();
    let key = managed::list()
        .into_iter()
        .find(|row| row.meta.provider_id == "pi")
        .unwrap()
        .session_key;
    let prepared = search::prepare("cancel-pi".into(), "needle".into(), vec![key], false).unwrap();
    let id = prepared.operation_id.clone();
    let result = read_context::with_open_observer(
        move |source, bounded| {
            if source == path {
                assert!(
                    bounded,
                    "cancellation must precede every native Pi body read"
                );
                assert!(search::cancel(&id));
            }
        },
        || search::run(&prepared.operation_id, None, |_| {}),
    )
    .unwrap();
    assert_eq!(result.state, "cancelled");
    assert!(result.progress.hits.is_empty());
}

#[test]
fn sqlite_snapshots_reuse_large_shared_sources_reject_changes_and_clean_up() {
    use std::sync::{
        atomic::{AtomicBool, AtomicU64},
        Arc,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let native = rusqlite::Connection::open(&path).unwrap();
    native.execute_batch("CREATE TABLE message (id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,data TEXT); CREATE TABLE part (id TEXT PRIMARY KEY,session_id TEXT,message_id TEXT,time_created INTEGER,data TEXT); CREATE TABLE unrelated (payload BLOB); INSERT INTO unrelated VALUES(zeroblob(34603008)); INSERT INTO message VALUES('msg_a','ses_a',1,'{\"role\":\"assistant\"}'); INSERT INTO part VALUES('part_a','ses_a','msg_a',1,'{\"type\":\"text\",\"text\":\"small selected session\"}');").unwrap();
    drop(native);
    assert!(std::fs::metadata(&path).unwrap().len() > read_context::SESSION_BYTES);
    let batch = Arc::new(read_context::SqliteBatch::default());
    let context = || {
        read_context::ReadContext::with_sqlite_batch(
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU64::new(0)),
            std::time::Instant::now() + read_context::BATCH_DURATION,
            vec![directory.path().to_owned()],
            batch.clone(),
        )
    };
    let source = format!("sqlite:{}:ses_a", path.display());
    let first = context();
    let rows = read_context::scope(first.clone(), || {
        providers::opencode::load_messages_sqlite(&source)
    })
    .unwrap();
    assert_eq!(rows[0].content, "small selected session");
    first.verify().unwrap();
    let copied = batch.copied_bytes();
    assert!(copied > read_context::SESSION_BYTES);
    let second = context();
    read_context::scope(second.clone(), || {
        providers::opencode::load_messages_sqlite(&source)
    })
    .unwrap();
    second.verify().unwrap();
    assert_eq!(
        batch.copied_bytes(),
        copied,
        "one batch must not recopy a shared database for each session"
    );
    let native = rusqlite::Connection::open(&path).unwrap();
    native
        .execute(
            "UPDATE part SET data='{\"type\":\"text\",\"text\":\"changed native body\"}'",
            [],
        )
        .unwrap();
    drop(native);
    let third = context();
    assert!(
        read_context::scope(third.clone(), || providers::opencode::load_messages_sqlite(
            &source
        ))
        .unwrap_err()
        .contains("changed")
    );
    assert_eq!(batch.copied_bytes(), copied);
    let copies = batch.copy_paths();
    assert_eq!(copies.len(), 1);
    drop(third);
    drop(second);
    drop(first);
    drop(batch);
    assert!(
        copies.iter().all(|copy| !copy.exists()),
        "batch completion must remove the private native database copy"
    );
}

#[test]
fn sqlite_field_budgets_cover_opencode_ids_parts_and_hermes_roles() {
    for field in [
        "opencode-message-id",
        "opencode-part-reference",
        "hermes-role",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native.db");
        let native = rusqlite::Connection::open(&path).unwrap();
        let oversized = "x".repeat(256);
        let source = if field == "hermes-role" {
            native.execute_batch("CREATE TABLE messages (session_id TEXT,role TEXT,content TEXT,created_at INTEGER);").unwrap();
            native
                .execute(
                    "INSERT INTO messages VALUES('native',?1,'needle',1)",
                    [oversized.as_str()],
                )
                .unwrap();
            format!("sqlite:{}#native", path.display())
        } else {
            native.execute_batch("CREATE TABLE message (id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,data TEXT); CREATE TABLE part (id TEXT PRIMARY KEY,session_id TEXT,message_id TEXT,time_created INTEGER,data TEXT);").unwrap();
            let message_id = if field == "opencode-message-id" {
                oversized.as_str()
            } else {
                "msg_a"
            };
            let part_reference = if field == "opencode-part-reference" {
                oversized.as_str()
            } else {
                "msg_a"
            };
            native
                .execute(
                    "INSERT INTO message VALUES(?1,'ses_a',1,'{\"role\":\"assistant\"}')",
                    [message_id],
                )
                .unwrap();
            native.execute("INSERT INTO part VALUES('part_a','ses_a',?1,1,'{\"type\":\"text\",\"text\":\"needle\"}')",[part_reference]).unwrap();
            format!("sqlite:{}:ses_a", path.display())
        };
        drop(native);
        let context =
            read_context::ReadContext::limited(vec![directory.path().to_owned()], 4096, 128);
        let result = read_context::scope(context, || {
            if field == "hermes-role" {
                providers::hermes::load_messages_sqlite(&source)
            } else {
                providers::opencode::load_messages_sqlite(&source)
            }
        });
        assert!(
            result.is_err_and(|error| error.ends_with("lineLimit")),
            "{field} must be charged before allocating a selected SQLite string"
        );
    }
}

#[test]
fn proof_verification_stops_when_cancelled_between_source_receipts() {
    let directory = tempfile::tempdir().unwrap();
    let context = read_context::ReadContext::limited(vec![directory.path().to_owned()], 4096, 128);
    for number in 0..4 {
        let path = directory.path().join(format!("{number}.jsonl"));
        std::fs::write(&path, "native body\n").unwrap();
        read_context::scope(context.clone(), || read_context::read_text(&path)).unwrap();
    }
    let cancel = context.cancel.clone();
    let result = read_context::with_verify_observer(
        move |index| {
            if index == 1 {
                cancel.store(true, std::sync::atomic::Ordering::Release);
            }
        },
        || context.verify(),
    );
    assert!(
        result.is_err_and(|error| error.ends_with("cancelled")),
        "cancellation must be checked between individual source receipt validations"
    );
}

#[test]
#[serial_test::serial]
fn text_matching_cannot_emit_hits_after_the_run_deadline() {
    let home = sandbox();
    claude(home.home(), 1, "");
    managed::forget_listing();
    let key = managed::list()[0].session_key.clone();
    let prepared = search::prepare(
        "matching-deadline".into(),
        "needle".into(),
        vec![key],
        false,
    )
    .unwrap();
    let result = search::with_run_duration(std::time::Duration::from_millis(100), || {
        search::with_match_observer(
            || std::thread::sleep(std::time::Duration::from_millis(120)),
            || search::run(&prepared.operation_id, None, |_| {}),
        )
    })
    .unwrap();
    assert_eq!(result.state, "limited");
    assert!(
        result.progress.hits.is_empty(),
        "matching after the deadline must not emit a successful hit"
    );
    assert!(result
        .limit_reasons
        .iter()
        .any(|reason| reason == "deadline"));
    assert!(
        result.next_cursor.is_some(),
        "a timed-out candidate must remain available to a fresh bounded continuation"
    );
}

#[test]
#[serial_test::serial]
fn candidate_replacement_between_stamp_check_and_native_open_is_changed() {
    let home = sandbox();
    let path = claude(home.home(), 1, "");
    managed::forget_listing();
    let key = managed::list()[0].session_key.clone();
    let prepared =
        search::prepare("candidate-race".into(), "needle".into(), vec![key], false).unwrap();
    let replacement = path.with_extension("replacement");
    let body = std::fs::read_to_string(&path)
        .unwrap()
        .replace("NEEDLE", "needle replaced after candidate validation");
    std::fs::write(&replacement, body).unwrap();
    let replaced = std::cell::Cell::new(false);
    let result = read_context::with_open_observer(
        move |source, bounded| {
            if source == path && !replaced.replace(true) {
                assert!(bounded);
                std::fs::rename(&replacement, &path).unwrap();
            }
        },
        || search::run(&prepared.operation_id, None, |_| {}),
    )
    .unwrap();
    assert_eq!(
        result.progress.changed, 1,
        "the handle must match the prepared candidate, not merely a self-consistent replacement"
    );
    assert!(result.progress.hits.is_empty());
}

#[test]
#[serial_test::serial]
fn sqlite_candidate_replacement_before_snapshot_capture_is_changed() {
    let home = sandbox();
    let root = home.home().join(".local/share/opencode");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("opencode.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE session (id TEXT PRIMARY KEY,title TEXT,directory TEXT,time_created INTEGER,time_updated INTEGER); CREATE TABLE message (id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,data TEXT); CREATE TABLE part (id TEXT PRIMARY KEY,session_id TEXT,message_id TEXT,time_created INTEGER,data TEXT); INSERT INTO session VALUES('ses_a','Native SQLite','/work/project',1,1); INSERT INTO message VALUES('msg_a','ses_a',1,'{\"role\":\"assistant\"}'); INSERT INTO part VALUES('part_a','ses_a','msg_a',1,'{\"type\":\"text\",\"text\":\"needle original body\"}');").unwrap();
    drop(conn);
    managed::forget_listing();
    let key = managed::list()
        .into_iter()
        .find(|row| row.meta.provider_id == "opencode")
        .unwrap()
        .session_key;
    let prepared = search::prepare(
        "candidate-sqlite-race".into(),
        "needle".into(),
        vec![key],
        false,
    )
    .unwrap();
    let replacement = root.join("replacement.db");
    std::fs::copy(&path, &replacement).unwrap();
    let conn = rusqlite::Connection::open(&replacement).unwrap();
    conn.execute("UPDATE part SET data='{\"type\":\"text\",\"text\":\"needle replaced after candidate validation\"}'",[]).unwrap();
    drop(conn);
    let replaced = std::cell::Cell::new(false);
    let result = read_context::with_open_observer(
        move |source, bounded| {
            if source == path && !replaced.replace(true) {
                assert!(bounded);
                std::fs::rename(&replacement, &path).unwrap();
            }
        },
        || search::run(&prepared.operation_id, None, |_| {}),
    )
    .unwrap();
    assert_eq!(
        result.progress.changed, 1,
        "SQLite capture must retain the prepared candidate's expected identity"
    );
    assert!(result.progress.hits.is_empty());
}

#[test]
#[serial_test::serial]
fn preparing_a_pi_resume_command_does_not_open_bodies_or_refresh_an_aged_listing() {
    let home = sandbox();
    let path = pi_fixture(home.home());
    let native = std::fs::read_to_string(&path).unwrap().replace(
        "\"/work/pi\"",
        &serde_json::to_string(home.home().to_str().unwrap()).unwrap(),
    );
    std::fs::write(&path, native).unwrap();
    managed::forget_listing();
    let row = managed::list()
        .into_iter()
        .find(|row| row.meta.provider_id == "pi")
        .unwrap();
    assert!(row.capabilities.can_resume);
    managed::age_listing_for_test();
    let result = read_context::with_open_observer(
        |_, _| panic!("preparing a resume command must not read any session body"),
        || managed::resume(&row.session_key, false),
    )
    .unwrap();
    assert!(!result.launched);
    assert!(result.command.contains("'pi'"));
    assert!(result.command.contains("--session"));
}
