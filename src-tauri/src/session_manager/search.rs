//! Explicit local transcript search. Jobs and snippets remain in memory only.
use super::{
    managed, providers,
    read_context::{self, ReadContext},
    source_cache::SourceStamp,
    SessionMessage, SessionMeta,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const MAX_HITS: usize = 200;
const HITS_PER_SESSION: usize = 3;
const JOB_TTL: Duration = Duration::from_secs(60);

#[cfg(test)]
type MatchObserver = Box<dyn Fn()>;
#[cfg(test)]
thread_local! {
    static MATCH_OBSERVER: std::cell::RefCell<Option<MatchObserver>> = const { std::cell::RefCell::new(None) };
    static RUN_DURATION: std::cell::Cell<Duration> = const { std::cell::Cell::new(read_context::BATCH_DURATION) };
}

#[cfg(test)]
pub(crate) fn with_run_duration<T>(duration: Duration, action: impl FnOnce() -> T) -> T {
    struct Reset(Duration);
    impl Drop for Reset {
        fn drop(&mut self) {
            RUN_DURATION.with(|current| current.set(self.0));
        }
    }
    let _reset = Reset(RUN_DURATION.with(|current| current.replace(duration)));
    action()
}

#[cfg(test)]
pub(crate) fn with_match_observer<T>(
    observer: impl Fn() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            MATCH_OBSERVER.with(|observer| {
                observer.replace(None);
            });
        }
    }
    MATCH_OBSERVER.with(|current| {
        current.replace(Some(Box::new(observer)));
    });
    let _reset = Reset;
    action()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub session_key: String,
    pub read_revision: String,
    pub message_ordinal: usize,
    pub content_hash: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub snippet: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub operation_id: String,
    pub request_id: String,
    pub hits: Vec<SearchHit>,
    pub scanned: usize,
    pub total: usize,
    pub changed: usize,
    pub failed: usize,
    pub unsupported: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub operation_id: String,
    pub request_id: String,
    pub candidate_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(flatten)]
    pub progress: Progress,
    pub state: String,
    pub limit_reasons: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub read_revision: String,
    pub messages: Vec<SessionMessage>,
}

struct Candidate {
    meta: SessionMeta,
    stamp: SourceStamp,
}
struct Position {
    next: usize,
    cursor: Option<String>,
    touched: Instant,
    scanned: usize,
    changed: usize,
    failed: usize,
    unsupported: usize,
    finished: bool,
}
struct Job {
    request_id: String,
    query: String,
    /// The query as a file's raw bytes would hold it, when they would.
    raw_needle: Option<String>,
    include_process: bool,
    candidates: Vec<Candidate>,
    cancel: Arc<AtomicBool>,
    running: AtomicBool,
    position: Mutex<Position>,
}

fn jobs() -> &'static Mutex<BTreeMap<String, Arc<Job>>> {
    static JOBS: OnceLock<Mutex<BTreeMap<String, Arc<Job>>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(BTreeMap::new()))
}
fn active() -> &'static Mutex<Option<String>> {
    static ACTIVE: Mutex<Option<String>> = Mutex::new(None);
    &ACTIVE
}

pub fn prepare(
    request_id: String,
    query: String,
    session_keys: Vec<String>,
    include_process: bool,
) -> Result<Prepared, String> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 256 {
        return Err("session.search.invalidQuery".into());
    }
    if request_id.is_empty() || request_id.len() > 128 || session_keys.len() > 20_000 {
        return Err("session.search.invalidRequest".into());
    }
    let rows = managed::freeze_references(&session_keys)?;
    let mut candidates = Vec::new();
    for meta in rows {
        let path = managed::source_file(&meta)?;
        let stamp = SourceStamp::at(&path).map_err(|_| "session.search.changed".to_owned())?;
        candidates.push(Candidate { meta, stamp });
    }
    let mut registry = jobs().lock().unwrap_or_else(|error| error.into_inner());
    registry.retain(|_, job| {
        job.running.load(Ordering::Acquire) || {
            let position = job
                .position
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            !position.finished && position.touched.elapsed() < JOB_TTL
        }
    });
    if registry.len() >= 4 {
        return Err("session.search.busy".into());
    }
    let operation_id = uuid::Uuid::new_v4().to_string();
    let count = candidates.len();
    registry.insert(
        operation_id.clone(),
        Arc::new(Job {
            request_id: request_id.clone(),
            raw_needle: raw_needle(&query.to_lowercase()),
            query: query.to_lowercase(),
            include_process,
            candidates,
            cancel: Arc::new(AtomicBool::new(false)),
            running: AtomicBool::new(false),
            position: Mutex::new(Position {
                next: 0,
                cursor: None,
                touched: Instant::now(),
                scanned: 0,
                changed: 0,
                failed: 0,
                unsupported: 0,
                finished: false,
            }),
        }),
    );
    Ok(Prepared {
        operation_id,
        request_id,
        candidate_count: count,
    })
}

pub fn cancel(operation_id: &str) -> bool {
    let registry = jobs().lock().unwrap_or_else(|error| error.into_inner());
    let Some(job) = registry.get(operation_id) else {
        return false;
    };
    let mut position = job
        .position
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if position.finished {
        return false;
    }
    job.cancel.store(true, Ordering::Release);
    position.finished = true;
    true
}

struct Running {
    id: String,
    job: Arc<Job>,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.job.running.store(false, Ordering::Release);
        let mut worker = active().lock().unwrap_or_else(|error| error.into_inner());
        if worker.as_deref() == Some(&self.id) {
            *worker = None;
        }
    }
}

fn progress(id: &str, job: &Job, hits: Vec<SearchHit>) -> Progress {
    let position = job
        .position
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    Progress {
        operation_id: id.into(),
        request_id: job.request_id.clone(),
        hits,
        scanned: position.scanned,
        total: job.candidates.len(),
        changed: position.changed,
        failed: position.failed,
        unsupported: position.unsupported,
    }
}

pub fn run(
    operation_id: &str,
    cursor: Option<&str>,
    emit: impl Fn(Progress),
) -> Result<SearchResult, String> {
    let job = jobs()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(operation_id)
        .cloned()
        .ok_or("session.search.expired")?;
    {
        let mut worker = active().lock().unwrap_or_else(|error| error.into_inner());
        if worker.is_some() || job.running.swap(true, Ordering::AcqRel) {
            return Err("session.search.busy".into());
        }
        *worker = Some(operation_id.into());
    }
    let _running = Running {
        id: operation_id.into(),
        job: job.clone(),
    };
    {
        let mut position = job
            .position
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if position.touched.elapsed() >= JOB_TTL {
            return Err("session.search.expired".into());
        }
        if position.finished && !job.cancel.load(Ordering::Acquire) {
            return Err("session.search.expired".into());
        }
        if position.cursor.as_deref() != cursor {
            return Err("session.search.invalidRequest".into());
        }
        position.cursor = None;
    }
    #[cfg(not(test))]
    let duration = read_context::BATCH_DURATION;
    #[cfg(test)]
    let duration = RUN_DURATION.with(|current| current.get());
    let deadline = Instant::now() + duration;
    let bytes = Arc::new(AtomicU64::new(0));
    let sqlite_batch = Arc::new(read_context::SqliteBatch::default());
    let mut hits = Vec::new();
    let mut pending = Vec::new();
    let mut reasons = Vec::new();
    let mut last_emit = Instant::now();
    emit(progress(operation_id, &job, vec![]));
    loop {
        if job.cancel.load(Ordering::Acquire) {
            break;
        }
        let next = job
            .position
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .next;
        let Some(candidate) = job.candidates.get(next) else {
            break;
        };
        if Instant::now() >= deadline {
            reasons.push("deadline".into());
            break;
        }
        if bytes.load(Ordering::Relaxed) >= read_context::BATCH_BYTES {
            reasons.push("batchLimit".into());
            break;
        }
        if hits.len() >= MAX_HITS - HITS_PER_SESSION {
            reasons.push("hitLimit".into());
            break;
        }
        let result = if candidate.meta.provider_id == "dsh" {
            Err("session.search.unsupported".into())
        } else {
            let path = managed::source_file(&candidate.meta)?;
            // a file changed since the search was prepared is reported as such,
            // never as one that holds no match
            let unchanged = SourceStamp::at(&path).is_ok_and(|stamp| stamp == candidate.stamp);
            let held = match &job.raw_needle {
                Some(needle) if unchanged => {
                    may_hold(&candidate.meta, needle, &job.cancel, deadline)
                }
                _ => Ok(true),
            };
            if held.as_ref().is_ok_and(|held| !held) {
                Ok(None)
            } else if let Err(error) = held {
                Err(error)
            } else if unchanged {
                let context = ReadContext::with_sqlite_batch(
                    job.cancel.clone(),
                    bytes.clone(),
                    deadline,
                    managed::roots(&candidate.meta.provider_id)?,
                    sqlite_batch.clone(),
                );
                context.expect_source(path, candidate.stamp.clone());
                read_record(&candidate.meta, context).map(Some)
            } else {
                Err("session.search.changed".into())
            }
        };
        match result {
            // its files do not hold the query: nothing to decode
            Ok(None) => {}
            Ok(Some(transcript)) => {
                #[cfg(test)]
                MATCH_OBSERVER.with(|observer| {
                    if let Some(observer) = observer.borrow().as_ref() {
                        observer();
                    }
                });
                let key = managed::key(&candidate.meta);
                let mut session_hits = Vec::new();
                let mut timed_out = false;
                for (ordinal, message) in transcript.messages.iter().enumerate() {
                    if job.cancel.load(Ordering::Acquire) {
                        break;
                    }
                    if Instant::now() >= deadline {
                        timed_out = true;
                        break;
                    }
                    if !job.include_process
                        && !matches!(message.role.as_str(), "user" | "assistant")
                    {
                        continue;
                    }
                    if !message.content.to_lowercase().contains(&job.query) {
                        continue;
                    }
                    let hit = SearchHit {
                        session_key: key.clone(),
                        read_revision: transcript.read_revision.clone(),
                        message_ordinal: ordinal,
                        content_hash: content_hash(message),
                        role: message.role.clone(),
                        name: message
                            .name
                            .as_ref()
                            .map(|name| crate::redact::scrub(name).into_owned()),
                        snippet: snippet(&message.content, &job.query),
                    };
                    session_hits.push(hit);
                    if session_hits.len() == HITS_PER_SESSION {
                        break;
                    }
                }
                if timed_out || Instant::now() >= deadline {
                    reasons.push("deadline".into());
                    break;
                }
                if job.cancel.load(Ordering::Acquire) {
                    break;
                }
                // Commit one candidate's samples together; a timed-out match
                // remains on the cursor without duplicating partial hits.
                pending.extend(session_hits.iter().cloned());
                hits.extend(session_hits);
            }
            Err(error) if error.ends_with("cancelled") => break,
            Err(error) if error.ends_with("deadline") || error.ends_with("batchLimit") => {
                reasons.push(error.trim_start_matches("session.search.").to_owned());
                break;
            }
            Err(error) if error.ends_with("changed") => {
                job.position
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .changed += 1
            }
            Err(error) if error.ends_with("unsupported") => {
                job.position
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .unsupported += 1
            }
            Err(error) => {
                job.position
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .failed += 1;
                if error.ends_with("sessionLimit")
                    || error.ends_with("lineLimit")
                    || error.ends_with("databaseLimit")
                {
                    let reason = error.trim_start_matches("session.search.").to_owned();
                    if !reasons.contains(&reason) {
                        reasons.push(reason);
                    }
                }
            }
        }
        let mut position = job
            .position
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        position.next += 1;
        position.scanned += 1;
        drop(position);
        if last_emit.elapsed() >= Duration::from_millis(100) {
            emit(progress(operation_id, &job, std::mem::take(&mut pending)));
            last_emit = Instant::now();
        }
    }
    emit(progress(operation_id, &job, pending));
    let cancelled = job.cancel.load(Ordering::Acquire);
    let mut position = job
        .position
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let remaining = position.next < job.candidates.len() && !cancelled;
    let next_cursor = remaining.then(|| uuid::Uuid::new_v4().to_string());
    position.cursor = next_cursor.clone();
    position.touched = Instant::now();
    position.finished = !remaining;
    drop(position);
    let progress = progress(operation_id, &job, hits);
    let partial = progress.changed > 0 || progress.failed > 0 || progress.unsupported > 0;
    Ok(SearchResult {
        progress,
        state: if cancelled {
            "cancelled"
        } else if remaining || partial || !reasons.is_empty() {
            "limited"
        } else {
            "complete"
        }
        .into(),
        limit_reasons: reasons,
        next_cursor,
    })
}

pub fn content_hash(message: &SessionMessage) -> String {
    let value = serde_json::to_vec(&(&message.role, &message.content, &message.name, message.ts))
        .unwrap_or_default();
    format!("{:x}", Sha256::digest(value))
}

fn read_record(meta: &SessionMeta, context: Arc<ReadContext>) -> Result<Transcript, String> {
    let messages =
        read_context::scope(context.clone(), || managed::messages_meta(meta)).map_err(|error| {
            if error.starts_with("session.search.") {
                error
            } else {
                "session.search.sourceUnavailable".into()
            }
        })?;
    if messages.len() > 100_000 {
        return Err("session.search.sessionLimit".into());
    }
    let mut hash = Sha256::new();
    hash.update(context.verify()?.as_bytes());
    for message in &messages {
        context.check()?;
        hash.update(content_hash(message).as_bytes());
    }
    Ok(Transcript {
        read_revision: format!("{:x}", hash.finalize()),
        messages,
    })
}

pub fn transcript(
    session_key: &str,
    expected_revision: Option<&str>,
    expected_ordinal: Option<usize>,
    expected_hash: Option<&str>,
) -> Result<Transcript, String> {
    if expected_ordinal.is_some() != expected_hash.is_some() {
        return Err("session.search.invalidRequest".into());
    }
    let meta = managed::freeze_references(&[session_key.to_owned()])?
        .into_iter()
        .next()
        .ok_or("session.search.changed")?;
    let context = ReadContext::new(
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicU64::new(0)),
        Instant::now() + read_context::BATCH_DURATION,
        managed::roots(&meta.provider_id)?,
    );
    let transcript = read_record(&meta, context)?;
    if expected_revision.is_some_and(|expected| expected != transcript.read_revision) {
        return Err("session.search.changed".into());
    }
    if let (Some(ordinal), Some(hash)) = (expected_ordinal, expected_hash) {
        if transcript
            .messages
            .get(ordinal)
            .is_none_or(|message| content_hash(message) != hash)
        {
            return Err("session.search.changed".into());
        }
    }
    Ok(transcript)
}

/// One of a listed session's subagents' conversation, read as a session's
/// is: from references the listing issued, within the reader's limits, and
/// only a subagent the session lists, never a path the renderer names.
pub fn subagent_transcript(session_key: &str, subagent_id: &str) -> Result<Transcript, String> {
    let meta = managed::freeze_references(&[session_key.to_owned()])?
        .into_iter()
        .next()
        .ok_or("session.search.changed")?;
    let listing = managed::cached_listing()?;
    let subagent = managed::subagents_of(&meta, &providers::opencode::children(&listing))
        .into_iter()
        .find(|subagent| subagent.id == subagent_id)
        .ok_or("session.search.changed")?;
    let file = SessionMeta {
        provider_id: meta.provider_id.clone(),
        session_id: subagent.id,
        source_path: Some(subagent.source),
        ..SessionMeta::default()
    };
    let context = ReadContext::new(
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicU64::new(0)),
        Instant::now() + read_context::BATCH_DURATION,
        managed::roots(&meta.provider_id)?,
    );
    read_record(&file, context)
}

/// The longest word of a lowered query, when a session file's raw bytes
/// hold it exactly as typed: ASCII but for quotes, escapes, markup and
/// brackets, and CJK, which Claude Code and Codex write unescaped. Else
/// none, and every file is decoded to be searched. Adapted from Magpie
/// internal/sessions/search.go (`rawNeedle`, MIT).
fn raw_needle(query: &str) -> Option<String> {
    let raw = |c: char| {
        c.is_whitespace()
            || c.is_ascii()
                && !c.is_ascii_control()
                && !matches!(c, '"' | '\\' | '<' | '>' | '&' | '[' | ']')
            || matches!(c as u32,
                0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
                    | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
    };
    if !query.chars().all(raw) {
        return None;
    }
    query
        .split_whitespace()
        .max_by_key(|word| word.len())
        .map(str::to_owned)
}

/// Whether a session's files may hold `needle`: a pass over their raw
/// (decompressed) bytes, ASCII case folded, decoding no line. Only Claude
/// and Codex transcripts are known to keep text unescaped; any other, or a
/// file that cannot be read here, is left to the decoding reader.
fn may_hold(
    meta: &SessionMeta,
    needle: &str,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<bool, String> {
    let paths = match meta.provider_id.as_str() {
        "claude" => vec![managed::source_file(meta)?],
        "codex" => match providers::codex::conversation_paths(meta) {
            Ok(paths) => paths,
            Err(_) => return Ok(true),
        },
        _ => return Ok(true),
    };
    let needle = needle.as_bytes();
    for path in paths {
        let Ok(file) = std::fs::File::open(&path) else {
            return Ok(true);
        };
        let compressed = path
            .extension()
            .is_some_and(|ext| ext == "zst" || ext == "zstd");
        let mut reader: Box<dyn std::io::Read> = if compressed {
            match zstd::stream::read::Decoder::new(file) {
                Ok(decoder) => Box::new(decoder),
                Err(_) => return Ok(true),
            }
        } else {
            Box::new(file)
        };
        let keep = needle.len().saturating_sub(1);
        let mut buffer = vec![0u8; (1 << 20) + keep];
        let mut have = 0;
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("session.search.cancelled".into());
            }
            if Instant::now() >= deadline {
                return Err("session.search.deadline".into());
            }
            let read = match reader.read(&mut buffer[have..]) {
                Ok(read) => read,
                Err(_) => return Ok(true),
            };
            // only what was just read, and the bytes before it a match may span
            let from = have.saturating_sub(keep);
            have += read;
            if contains_folded(&buffer[from..have], needle) {
                return Ok(true);
            }
            if read == 0 {
                break;
            }
            if have >= buffer.len() {
                buffer.copy_within(have - keep..have, 0);
                have = keep;
            }
        }
    }
    Ok(false)
}

/// `needle` (lowered) in `bytes`, ASCII letters compared without case: a
/// scan for its first byte in either case, then a comparison there.
fn contains_folded(bytes: &[u8], needle: &[u8]) -> bool {
    let Some(&first) = needle.first() else {
        return false;
    };
    let upper = first.to_ascii_uppercase();
    let mut rest = bytes;
    while rest.len() >= needle.len() {
        let Some(at) = rest[..=rest.len() - needle.len()]
            .iter()
            .position(|&b| b == first || b == upper)
        else {
            return false;
        };
        if rest[at..at + needle.len()].eq_ignore_ascii_case(needle) {
            return true;
        }
        rest = &rest[at + 1..];
    }
    false
}

fn snippet(text: &str, query: &str) -> String {
    // Scrub before clipping: a credential crossing the excerpt boundary must
    // never become an unrecognized short prefix in a progress event.
    let scrubbed = crate::redact::scrub(text);
    let text = scrubbed.as_ref();
    // Locate in a folded character stream, retaining Unicode-safe original
    // character positions even where lowercasing expands one character.
    let offset = text.to_lowercase().find(query).unwrap_or(0);
    let mut bytes = 0;
    let matched = text
        .chars()
        .position(|character| {
            let previous = bytes;
            bytes += character.to_lowercase().map(char::len_utf8).sum::<usize>();
            offset >= previous && offset < bytes
        })
        .unwrap_or(0);
    let start = matched.saturating_sub(50);
    let mut tail = text.chars().skip(start);
    let excerpt: String = tail.by_ref().take(160).collect();
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        excerpt,
        if tail.next().is_some() { "…" } else { "" }
    )
}
