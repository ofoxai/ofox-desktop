//! What only a whole session file tells — every title candidate and the
//! message count — read once per change of the file. The result is kept in
//! `session_meta_cache`, keyed by the file's path and stamp (size and
//! modification time in nanoseconds); a listing reads again only the files
//! whose stamp changed, or that an older parser read (`PARSER_VERSION`).
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::providers;
use super::providers::codex::CodexSpawns;
use super::source_cache::{self, Part, SourceStamp};
use super::title::{titled, TitleCandidates};
use super::SessionMeta;
use crate::database::Database;
use crate::error::AppError;

/// Bump to read every cached file again after a change to what is read.
pub const PARSER_VERSION: i64 = 4;

/// Providers whose files are read whole for their details.
const PROVIDERS: [&str; 2] = ["claude", "codex"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionDetails {
    pub candidates: TitleCandidates,
    pub message_count: u32,
    /// The subagents a Codex rollout's own calls spawned.
    #[serde(skip_serializing_if = "CodexSpawns::is_empty")]
    pub spawns: CodexSpawns,
}

/// A file's stamp: its size and modification time in nanoseconds.
pub(crate) fn stamp(path: &Path) -> Option<String> {
    serde_json::to_string(&SourceStamp::at(path).ok()?).ok()
}

/// Read a whole session file for its details.
fn parse(provider: &str, path: &Path) -> Option<SessionDetails> {
    match provider {
        "claude" => providers::claude::details(path),
        "codex" => providers::codex::details(path),
        _ => None,
    }
}

struct File {
    provider: String,
    session_id: String,
    path: PathBuf,
    stamp: String,
    expected: Option<String>,
}

/// Title every Claude and Codex session of a listing from its whole files and
/// count its messages: from the cache where a file is unchanged, else read
/// (and cached when there is a database).
pub(crate) fn enrich(listing: &mut [SessionMeta], db: Option<&Database>) {
    let mut files = listing
        .iter()
        .filter(|meta| PROVIDERS.contains(&meta.provider_id.as_str()))
        .filter_map(|meta| {
            let path = PathBuf::from(meta.source_path.as_deref()?);
            Some(File {
                provider: meta.provider_id.clone(),
                session_id: meta.session_id.clone(),
                stamp: stamp(&path)?,
                path,
                expected: None,
            })
        })
        .collect::<Vec<_>>();

    let mut cached = db.map(load).unwrap_or_default();
    let mut details: HashMap<PathBuf, SessionDetails> = HashMap::new();
    let mut missing = Vec::new();
    for file in &mut files {
        let key = file.path.to_string_lossy();
        let cell = cached.remove(key.as_ref());
        file.expected = cell.as_ref().and_then(|cell| cell.token.clone());
        match cell.and_then(|cell| cell.valid) {
            Some((stamp, found)) if stamp == file.stamp => {
                details.insert(file.path.clone(), found);
            }
            _ => missing.push(&*file),
        }
    }

    let parsed = parse_all(&missing);
    let fresh = missing
        .iter()
        .zip(parsed)
        .filter_map(|(file, found)| Some((*file, found?)))
        .collect::<Vec<_>>();
    if let Some(db) = db {
        if let Err(e) = store(db, &fresh) {
            log::warn!("[SESSIONS] meta cache write failed: {e}");
        }
    }
    for (file, found) in fresh {
        details.insert(file.path.clone(), found);
    }

    apply(listing, &files, &details);
}

/// Merge each session's files (a Codex thread may run on in later rollouts:
/// the earliest names it, every one counts) and set its title and count.
fn apply(listing: &mut [SessionMeta], files: &[File], details: &HashMap<PathBuf, SessionDetails>) {
    let mut sessions: HashMap<(&str, &str), Vec<&File>> = HashMap::new();
    for file in files {
        sessions
            .entry((&file.provider, &file.session_id))
            .or_default()
            .push(file);
    }
    let names = providers::codex::current_thread_names();
    let mut merged = HashMap::new();
    for ((provider, session_id), mut group) in sessions {
        group.sort_by_key(|f| f.path.file_name().map(|n| n.to_os_string()));
        let plain = group.iter().map(|f| f.path.clone()).collect::<HashSet<_>>();
        // Metadata receipts contain counts, not native-byte prefix proofs. When
        // an archive and another rollout coexist, summing could count a copy
        // twice. The explicit transcript reader verifies prefixes when opened.
        let archive_root = crate::codex_config::get_codex_config_dir().join("archived_sessions");
        let effective: Vec<_> = group
            .iter()
            .filter(|file| {
                !file
                    .path
                    .extension()
                    .is_some_and(|extension| extension == "zst" || extension == "zstd")
                    || !plain.contains(&file.path.with_extension(""))
            })
            .collect();
        let count_available = !(provider == "codex"
            && effective.len() > 1
            && effective
                .iter()
                .any(|file| file.path.starts_with(&archive_root)));
        let mut total = SessionDetails::default();
        let mut found = false;
        for file in group {
            // a compressed twin repeats its uncompressed rollout
            if file
                .path
                .extension()
                .is_some_and(|e| e == "zst" || e == "zstd")
                && plain.contains(&file.path.with_extension(""))
            {
                continue;
            }
            let Some(part) = details.get(&file.path) else {
                continue;
            };
            found = true;
            total.candidates.fill_from(&part.candidates);
            total.message_count = total.message_count.saturating_add(part.message_count);
            total.spawns.merge(&part.spawns);
        }
        if found {
            if provider == "codex" {
                total.candidates.agent = names.get(session_id).cloned();
            }
            merged.insert(
                (provider.to_string(), session_id.to_string()),
                (total, count_available),
            );
        }
    }
    let tasks = task_titles(&merged);
    for meta in listing.iter_mut() {
        let Some((found, count_available)) =
            merged.get(&(meta.provider_id.clone(), meta.session_id.clone()))
        else {
            continue;
        };
        let mut candidates = found
            .candidates
            .clone()
            .with_directory(meta.project_dir.as_deref());
        if meta.provider_id == "codex" {
            if let Some(parent) = &meta.parent_session_id {
                candidates.task = tasks
                    .get(&(parent.as_str(), meta.session_id.as_str()))
                    .cloned()
                    .flatten();
            }
        }
        (meta.title, meta.title_source) = titled(&candidates);
        meta.message_count = count_available.then_some(found.message_count);
    }
}

/// Each Codex subagent's task name, by (parent, child): only its own parent's
/// call names a child, so a fork's copy of its parent's calls names none of
/// its siblings. A link two calls name differently is left unnamed.
fn task_titles(
    merged: &HashMap<(String, String), (SessionDetails, bool)>,
) -> HashMap<(&str, &str), Option<String>> {
    let mut tasks: HashMap<(&str, &str), Option<String>> = HashMap::new();
    for ((provider, parent), (details, _)) in merged {
        if provider != "codex" {
            continue;
        }
        for (child, task) in details.spawns.titles() {
            tasks
                .entry((parent.as_str(), child))
                .and_modify(|old| {
                    if old.as_deref() != Some(task) {
                        *old = None;
                    }
                })
                .or_insert_with(|| Some(task.to_owned()));
        }
    }
    tasks
}

/// Read the missing files' details, a few files at a time.
fn parse_all(files: &[&File]) -> Vec<Option<SessionDetails>> {
    let results = Mutex::new(vec![None; files.len()]);
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8)
        .min(files.len());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(file) = files.get(index) else {
                    break;
                };
                let found = parse(&file.provider, &file.path);
                results.lock().unwrap_or_else(|e| e.into_inner())[index] = found;
            });
        }
    });
    results.into_inner().unwrap_or_else(|e| e.into_inner())
}

struct CachedPart {
    token: Option<String>,
    valid: Option<(String, SessionDetails)>,
}
type Cached = HashMap<String, CachedPart>;

/// Every cached row of the current parser, by source path.
fn load(db: &Database) -> Cached {
    let rows = (|| -> Result<Cached, AppError> {
        let conn = db.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT source_path, provider_id, stamp, parser_version, meta_json FROM session_meta_cache
             WHERE provider_id IN ('claude', 'codex')",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ),
            ))
        })?;
        let mut cached = HashMap::new();
        for row in rows {
            let (path, row) = row?;
            let provider = row.0.clone().unwrap_or_default();
            let parts = match source_cache::decode_row(row, &provider) {
                Ok(parts) => parts,
                Err(e) => {
                    log::warn!("[SESSIONS] cache part unavailable: {e}");
                    continue;
                }
            };
            let valid = parts.value(Part::Metadata).and_then(|v| {
                if v["parserVersion"].as_i64() != Some(PARSER_VERSION) {
                    return None;
                }
                let details = v.get("details")?;
                if details.get("candidates").is_none() && details.get("messageCount").is_none() {
                    return None;
                }
                Some((
                    v["stamp"].as_str()?.to_owned(),
                    serde_json::from_value(details.clone()).ok()?,
                ))
            });
            cached.insert(
                path,
                CachedPart {
                    token: parts.token(Part::Metadata),
                    valid,
                },
            );
        }
        Ok(cached)
    })();
    rows.unwrap_or_else(|e| {
        log::warn!("[SESSIONS] meta cache read failed: {e}");
        HashMap::new()
    })
}

/// Only metadata is replaced. A filtered/incomplete listing is not evidence
/// that a source disappeared, and never deletes another part's usage proof.
fn store(db: &Database, fresh: &[(&File, SessionDetails)]) -> Result<(), AppError> {
    if fresh.is_empty() {
        return Ok(());
    }
    let mut conn = db.conn.lock().unwrap_or_else(|e| e.into_inner());
    let mut first_error = None;
    for (file, found) in fresh {
        let result = (|| -> Result<(), AppError> {
            let tx = conn.transaction()?;
            // A changed source cannot publish stale details after parsing.
            if stamp(&file.path).as_deref() != Some(&file.stamp) {
                return Err(AppError::Database("metadata source changed; retry".into()));
            }
            source_cache::merge_on(
                &tx,
                &file.path.to_string_lossy(),
                &file.provider,
                Some(&file.session_id),
                Part::Metadata,
                &file.expected,
                Some(serde_json::json!({"parserVersion":PARSER_VERSION,
                "stamp":file.stamp,"sourceStamp":serde_json::from_str::<serde_json::Value>(&file.stamp).ok(),"details":found})),
            )?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(e) = result {
            if first_error.is_none() {
                first_error = Some(e);
            }
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::managed::{list_with, ManagedSession};
    use crate::session_manager::title::TitleSource;
    use crate::test_sandbox::sandbox;

    const ID: &str = "11111111-2222-3333-4444-555555555555";

    fn claude_session(home: &Path) -> PathBuf {
        let project = home.join(".claude/projects/-work-app");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join(format!("{ID}.jsonl"));
        let line = serde_json::json!({"type": "user", "sessionId": ID, "cwd": "/work/app",
            "timestamp": "2026-10-14T10:00:00Z", "message": {"role": "user", "content": "fix the login"}});
        std::fs::write(&path, format!("{line}\n")).unwrap();
        path
    }

    fn listed<'a>(sessions: &'a [ManagedSession], id: &str) -> &'a ManagedSession {
        sessions
            .iter()
            .find(|s| s.meta.session_id == id)
            .expect("listed")
    }

    fn poison(db: &Database, version: i64) {
        let conn = db.conn.lock().unwrap();
        let changed = conn
            .execute(
                "UPDATE session_meta_cache SET parser_version = ?1,
                 meta_json = '{\"candidates\":{\"prompt\":\"from the cache\"},\"messageCount\":7}'",
                [version],
            )
            .unwrap();
        assert_eq!(changed, 1);
    }

    #[test]
    #[serial_test::serial]
    fn an_unchanged_file_is_not_read_again() {
        let sandbox = sandbox();
        let db = Database::memory().unwrap();
        let path = claude_session(sandbox.home());

        let first = list_with(Some(&db));
        assert_eq!(
            listed(&first, ID).meta.title.as_deref(),
            Some("fix the login")
        );
        assert_eq!(listed(&first, ID).meta.message_count, Some(1));

        // the row now says something the file does not: only a cache hit shows it
        poison(&db, PARSER_VERSION);
        let cached = list_with(Some(&db));
        assert_eq!(
            listed(&cached, ID).meta.title.as_deref(),
            Some("from the cache")
        );
        assert_eq!(listed(&cached, ID).meta.message_count, Some(7));

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(
            &mut file,
            b"{\"type\":\"ai-title\",\"aiTitle\":\"Fix login bug\"}\n",
        )
        .unwrap();
        let changed = list_with(Some(&db));
        assert_eq!(
            listed(&changed, ID).meta.title.as_deref(),
            Some("Fix login bug")
        );
        assert_eq!(listed(&changed, ID).meta.message_count, Some(1));
    }

    #[test]
    #[serial_test::serial]
    fn a_row_an_older_parser_wrote_is_read_again() {
        let sandbox = sandbox();
        let db = Database::memory().unwrap();
        claude_session(sandbox.home());
        list_with(Some(&db));

        poison(&db, PARSER_VERSION - 1);
        let listing = list_with(Some(&db));
        assert_eq!(
            listed(&listing, ID).meta.title.as_deref(),
            Some("fix the login")
        );
        let version: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT parser_version FROM session_meta_cache", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, PARSER_VERSION);
    }

    #[test]
    #[serial_test::serial]
    fn a_file_absent_from_a_listing_retains_its_cache_parts() {
        let sandbox = sandbox();
        let db = Database::memory().unwrap();
        let path = claude_session(sandbox.home());
        list_with(Some(&db));
        std::fs::remove_file(&path).unwrap();

        list_with(Some(&db));
        let rows: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM session_meta_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    #[serial_test::serial]
    fn a_codex_thread_is_named_by_its_earliest_rollout_and_counts_every_one() {
        let sandbox = sandbox();
        let thread = "01a0bdc9-b5fd-7e63-9658-68bc8dce5ecd";
        let day = sandbox.home().join(".codex/sessions/2026/10/14");
        std::fs::create_dir_all(&day).unwrap();
        let rollout = |time: &str, prompt: &str| {
            let lines = [
                serde_json::json!({"timestamp": "2026-10-14T10:00:00Z", "type": "session_meta",
                    "payload": {"id": thread, "cwd": "/work/app"}}),
                serde_json::json!({"timestamp": "2026-10-14T10:00:01Z", "type": "response_item",
                    "payload": {"type": "message", "role": "user", "content": [
                        {"type": "input_text", "text": "<environment_context>x</environment_context>"}]}}),
                serde_json::json!({"timestamp": "2026-10-14T10:00:02Z", "type": "response_item",
                    "payload": {"type": "message", "role": "user", "content": [
                        {"type": "input_text", "text": prompt}]}}),
                serde_json::json!({"timestamp": "2026-10-14T10:00:03Z", "type": "event_msg",
                    "payload": {"type": "user_message", "message": prompt}}),
                serde_json::json!({"timestamp": "2026-10-14T10:00:04Z", "type": "response_item",
                    "payload": {"type": "message", "role": "assistant", "content": [
                        {"type": "output_text", "text": "done"}]}}),
            ];
            let body = lines.iter().map(|l| format!("{l}\n")).collect::<String>();
            std::fs::write(
                day.join(format!("rollout-2026-10-14T{time}-{thread}.jsonl")),
                body,
            )
            .unwrap();
        };
        rollout("10-00-00", "first prompt");
        rollout("11-00-00", "second prompt");

        let listing = list_with(None);
        let session = listed(&listing, thread);
        assert_eq!(session.meta.title.as_deref(), Some("first prompt"));
        assert_eq!(session.meta.message_count, Some(4));

        std::fs::write(
            sandbox.home().join(".codex/session_index.jsonl"),
            format!(
                "{}\n",
                serde_json::json!({"id": thread, "thread_name": "Codex named it"})
            ),
        )
        .unwrap();
        let listing = list_with(None);
        assert_eq!(
            listed(&listing, thread).meta.title.as_deref(),
            Some("Codex named it")
        );
    }

    /// A Codex rollout of `id`, spawned by `parent` when there is one.
    fn codex_thread(home: &Path, id: &str, parent: Option<&str>, lines: &[serde_json::Value]) {
        let day = home.join(".codex/sessions/2026/10/14");
        std::fs::create_dir_all(&day).unwrap();
        let source = match parent {
            Some(parent) => {
                serde_json::json!({"subagent": {"thread_spawn": {"parent_thread_id": parent}}})
            }
            None => serde_json::json!("vscode"),
        };
        let mut body = format!(
            "{}\n",
            serde_json::json!({"timestamp": "2026-10-14T10:00:00Z", "type": "session_meta",
                "payload": {"id": id, "cwd": "/work/app", "source": source}})
        );
        for line in lines {
            body.push_str(&format!("{line}\n"));
        }
        std::fs::write(
            day.join(format!("rollout-2026-10-14T10-00-00-{id}.jsonl")),
            body,
        )
        .unwrap();
    }

    fn spawned(call: &str, task: &str, child: &str) -> [serde_json::Value; 2] {
        [
            serde_json::json!({"timestamp": "2026-10-14T10:00:01Z", "type": "response_item",
                "payload": {"type": "function_call", "name": "spawn_agent", "call_id": call,
                    "arguments": serde_json::json!({"task_name": task}).to_string()}}),
            serde_json::json!({"timestamp": "2026-10-14T10:00:02Z", "type": "event_msg",
                "payload": {"type": "item_completed", "item": {"type": "SubAgentActivity",
                    "id": call, "kind": "started", "agent_thread_id": child}}}),
        ]
    }

    fn typed(prompt: &str) -> serde_json::Value {
        serde_json::json!({"timestamp": "2026-10-14T10:00:03Z", "type": "response_item",
            "payload": {"type": "message", "role": "user", "content": [
                {"type": "input_text", "text": prompt}]}})
    }

    /// Magpie `TestCodexSubagentTaskTitleAndNickname`,
    /// `…ComesFromItsOwnParent` and `…ConflictingSpawnLinksDoNotNameChildren` (MIT).
    #[test]
    #[serial_test::serial]
    fn an_untitled_codex_subagent_takes_its_parents_task_name() {
        let sandbox = sandbox();
        let home = sandbox.home();
        let [spawn_a, start_a] = spawned("c1", "usage_audit", "child-a");
        let [spawn_b, start_b] = spawned("c2", "r0_final_review", "child-b");
        let [spawn_c, start_c] = spawned("c3", "named_but_prompted", "child-c");
        codex_thread(
            home,
            "parent-p",
            None,
            &[
                typed("合并集成分支改动"),
                spawn_a,
                start_a,
                spawn_b,
                start_b,
                spawn_c,
                start_c,
            ],
        );
        // child-a repeats its parent's history: its copy of the call that
        // started child-b says another name, and must not rename its sibling
        let [copied_spawn, copied_start] = spawned("c2", "copied_wrong_name", "child-b");
        codex_thread(
            home,
            "child-a",
            Some("parent-p"),
            &[copied_spawn, copied_start],
        );
        codex_thread(home, "child-b", Some("parent-p"), &[]);
        codex_thread(
            home,
            "child-c",
            Some("parent-p"),
            &[typed("读 HANDOFF-codex.md")],
        );
        // a thread no parent spawned keeps its folder name
        codex_thread(home, "child-d", Some("parent-p"), &[]);

        let listing = list_with(None);
        let title = |id: &str| {
            let meta = &listed(&listing, id).meta;
            (meta.title.clone().unwrap_or_default(), meta.title_source)
        };
        assert_eq!(
            title("child-a"),
            ("usage_audit".into(), Some(TitleSource::Task))
        );
        assert_eq!(
            title("child-b"),
            ("r0_final_review".into(), Some(TitleSource::Task))
        );
        assert_eq!(
            title("child-c"),
            ("读 HANDOFF-codex.md".into(), Some(TitleSource::Prompt))
        );
        assert_eq!(
            title("child-d"),
            ("app".into(), Some(TitleSource::Directory))
        );
        assert_eq!(
            listed(&listing, "child-a")
                .meta
                .parent_session_id
                .as_deref(),
            Some("parent-p")
        );
        assert_eq!(listed(&listing, "parent-p").meta.parent_session_id, None);
    }
}
