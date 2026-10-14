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
use std::time::UNIX_EPOCH;

use super::providers;
use super::title::{titled, TitleCandidates};
use super::SessionMeta;
use crate::database::Database;

/// Bump to read every cached file again after a change to what is read.
pub const PARSER_VERSION: i64 = 2;

/// Providers whose files are read whole for their details.
const PROVIDERS: [&str; 2] = ["claude", "codex"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionDetails {
    pub candidates: TitleCandidates,
    pub message_count: u32,
}

/// A file's stamp: its size and modification time in nanoseconds.
pub(crate) fn stamp(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(format!("{}:{}", meta.len(), modified.as_nanos()))
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
}

/// Title every Claude and Codex session of a listing from its whole files and
/// count its messages: from the cache where a file is unchanged, else read
/// (and cached when there is a database).
pub(crate) fn enrich(listing: &mut [SessionMeta], db: Option<&Database>) {
    let files = listing
        .iter()
        .filter(|meta| PROVIDERS.contains(&meta.provider_id.as_str()))
        .filter_map(|meta| {
            let path = PathBuf::from(meta.source_path.as_deref()?);
            Some(File {
                provider: meta.provider_id.clone(),
                session_id: meta.session_id.clone(),
                stamp: stamp(&path)?,
                path,
            })
        })
        .collect::<Vec<_>>();

    let mut cached = db.map(load).unwrap_or_default();
    let mut details: HashMap<PathBuf, SessionDetails> = HashMap::new();
    let mut missing = Vec::new();
    for file in &files {
        let key = file.path.to_string_lossy();
        match cached.remove(key.as_ref()) {
            Some((stamp, found)) if stamp == file.stamp => {
                details.insert(file.path.clone(), found);
            }
            _ => missing.push(file),
        }
    }

    let parsed = parse_all(&missing);
    let fresh = missing
        .iter()
        .zip(parsed)
        .filter_map(|(file, found)| Some((*file, found?)))
        .collect::<Vec<_>>();
    if let Some(db) = db {
        // rows left in `cached` are files no longer listed (or of an older parser)
        let gone = cached.into_keys().collect::<Vec<_>>();
        if let Err(e) = store(db, &fresh, &gone) {
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
        }
        if found {
            if provider == "codex" {
                total.candidates.agent = names.get(session_id).cloned();
            }
            merged.insert((provider.to_string(), session_id.to_string()), total);
        }
    }
    for meta in listing.iter_mut() {
        let Some(found) = merged.get(&(meta.provider_id.clone(), meta.session_id.clone())) else {
            continue;
        };
        let candidates = found
            .candidates
            .clone()
            .with_directory(meta.project_dir.as_deref());
        (meta.title, meta.title_source) = titled(&candidates);
        meta.message_count = Some(found.message_count);
    }
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

type Cached = HashMap<String, (String, SessionDetails)>;

/// Every cached row of the current parser, by source path.
fn load(db: &Database) -> Cached {
    let rows = (|| -> rusqlite::Result<Cached> {
        let conn = db.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT source_path, stamp, meta_json FROM session_meta_cache
             WHERE parser_version = ?1 AND provider_id IN ('claude', 'codex')",
        )?;
        let rows = stmt.query_map([PARSER_VERSION], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut cached = HashMap::new();
        for row in rows {
            let (path, stamp, json) = row?;
            let found = json.and_then(|json| serde_json::from_str(&json).ok());
            if let (Some(stamp), Some(found)) = (stamp, found) {
                cached.insert(path, (stamp, found));
            }
        }
        Ok(cached)
    })();
    rows.unwrap_or_else(|e| {
        log::warn!("[SESSIONS] meta cache read failed: {e}");
        HashMap::new()
    })
}

/// Write the files read again, and forget the rows of files no longer listed
/// or read by an older parser.
fn store(
    db: &Database,
    fresh: &[(&File, SessionDetails)],
    gone: &[String],
) -> rusqlite::Result<()> {
    if fresh.is_empty() && gone.is_empty() {
        return Ok(());
    }
    let mut conn = db.conn.lock().unwrap_or_else(|e| e.into_inner());
    let tx = conn.transaction()?;
    {
        let now = chrono::Utc::now().timestamp_millis();
        let mut upsert = tx.prepare(
            "INSERT OR REPLACE INTO session_meta_cache
                (source_path, provider_id, session_id, stamp, parser_version, meta_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for (file, found) in fresh {
            let json = serde_json::to_string(found).unwrap_or_default();
            upsert.execute(rusqlite::params![
                file.path.to_string_lossy(),
                file.provider,
                file.session_id,
                file.stamp,
                PARSER_VERSION,
                json,
                now,
            ])?;
        }
        let mut forget = tx.prepare("DELETE FROM session_meta_cache WHERE source_path = ?1")?;
        for path in gone {
            forget.execute([path])?;
        }
        tx.execute(
            "DELETE FROM session_meta_cache
             WHERE provider_id IN ('claude', 'codex') AND parser_version != ?1",
            [PARSER_VERSION],
        )?;
    }
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::managed::{list_with, ManagedSession};
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
    fn the_row_of_a_file_no_longer_listed_is_forgotten() {
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
        assert_eq!(rows, 0);
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
}
