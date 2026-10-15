//! Machine-local, body-free analytics cache. Native files are read on refresh;
//! changing a query or opening details reads the cache in one batch only.
#[cfg(test)]
pub(crate) use super::session_analytics_discovery::discover_sources;
pub(crate) use super::session_analytics_discovery::{Discovery, DiscoveryFailure};
use super::{
    session_analytics::{AnalyticsInput, SessionDescriptor, SourceReason},
    session_analytics_core::{MessageEvent, Project, Role, SessionIdentity, UsageFact},
    session_analytics_messages::{TranscriptFacts, TranscriptScan},
    usage_query::SourceHealth,
    usage_source,
};
use crate::{
    database::Database,
    error::AppError,
    session_manager::source_cache::{self, Part, SourceStamp},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
const VERSION: u8 = 1;
const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_SOURCES: usize = 20_000;

#[derive(Clone)]
pub(crate) struct SourceSpec {
    pub provider: String,
    pub path: PathBuf,
    pub subagent: bool,
    pub project_file: Option<PathBuf>,
    pub title_file: Option<PathBuf>,
}
#[derive(Default, Debug)]
pub(crate) struct RefreshReport {
    pub parsed: usize,
    pub unchanged: usize,
    pub failed: usize,
    pub provider_states: BTreeMap<String, RefreshState>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum RefreshState {
    Ok,
    Partial(&'static str),
    Failed(&'static str),
}
impl RefreshState {
    fn severity(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Partial(_) => 1,
            Self::Failed(_) => 2,
        }
    }
}
impl RefreshReport {
    fn mark(&mut self, provider: &str, state: RefreshState) {
        let slot = self
            .provider_states
            .entry(provider.to_owned())
            .or_insert(RefreshState::Ok);
        if state.severity() >= slot.severity() {
            *slot = state;
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    version: u8,
    stamp: Option<Value>,
    sessions: Vec<SessionDescriptor>,
    facts: Vec<UsageFact>,
    messages: Vec<MessageEvent>,
    reasons: Vec<String>,
    available: bool,
    status: String,
    priority: u8,
    attempted_at_ms: i64,
    succeeded_at_ms: Option<i64>,
}
struct Row {
    provider: String,
    token: Option<String>,
    entry: Result<Option<Entry>, ()>,
    raw: String,
}
fn invalid(code: &str) -> AppError {
    AppError::Database(format!("usage.analytics.{code}"))
}
pub(crate) fn key(provider: &str, id: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(provider, id)).expect("string tuple"))
    )
}
/// Project filters must remain bounded even for a long native path. The exact
/// path is retained separately for display; no basename or whitespace folding
/// can make two distinct projects share an identity.
pub(crate) fn project_key(path: &str) -> String {
    key("project-v1", path)
}
/// Analytics observations tolerate opaque stamp extensions. This comparison
/// must never be used for ledger acknowledgements or negative identity proof.
fn same_known_revision(saved: &Value, current: &Value) -> bool {
    match (saved, current) {
        (Value::Object(saved), Value::Object(current)) => current.iter().all(|(key, value)| {
            saved
                .get(key)
                .is_some_and(|old| same_known_revision(old, value))
        }),
        (Value::Array(saved), Value::Array(current)) => {
            saved.len() == current.len()
                && saved
                    .iter()
                    .zip(current)
                    .all(|(a, b)| same_known_revision(a, b))
        }
        _ => saved == current,
    }
}
fn source_id(provider: &str) -> &str {
    match provider {
        "claude" => "session_log",
        "codex" => "codex_session",
        "gemini" => "gemini_session",
        "opencode" => "opencode_session",
        _ => "unsupported",
    }
}
/// Exactly one cache-table SELECT for a query/refresh. Changed sources alone
/// perform CAS writes; warm aggregation never point-reads each session.
fn rows_on(conn: &rusqlite::Connection) -> Result<BTreeMap<String, Row>, AppError> {
    let mut statement=conn.prepare("SELECT source_path,provider_id,stamp,parser_version,meta_json FROM session_meta_cache WHERE provider_id IN ('claude','codex','gemini','opencode') ORDER BY source_path")?;
    let mut result = BTreeMap::new();
    let mut bytes = 0usize;
    let rows = statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    for row in rows {
        let (path, provider, stamp, version, raw) = row?;
        let raw = raw.unwrap_or_default();
        bytes = bytes
            .checked_add(raw.len())
            .ok_or_else(|| invalid("cacheBudgetExceeded"))?;
        if bytes > MAX_CACHE_BYTES || result.len() >= MAX_SOURCES {
            return Err(invalid("cacheBudgetExceeded"));
        }
        let decoded = source_cache::decode_row(
            (Some(provider.clone()), stamp, version, Some(raw.clone())),
            &provider,
        );
        let (token, entry) = match decoded {
            Ok(parts) => {
                let token = parts.token(Part::Analytics);
                let entry = parts
                    .value(Part::Analytics)
                    .map(|value| {
                        let entry: Entry = serde_json::from_value(value.clone()).map_err(|_| ())?;
                        if entry.version != VERSION {
                            return Err(());
                        }
                        Ok(entry)
                    })
                    .transpose();
                (token, entry)
            }
            Err(_) => (None, Err(())),
        };
        result.insert(
            path,
            Row {
                provider,
                token,
                entry,
                raw,
            },
        );
    }
    Ok(result)
}
pub(crate) fn source_revision(spec: &SourceSpec) -> Result<Value, AppError> {
    fn dependency(path: Option<&Path>) -> Result<Option<Value>, AppError> {
        match path {
            Some(path) => match std::fs::symlink_metadata(path) {
                Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                    Ok(Some(json!(SourceStamp::at(path)?)))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                _ => Err(invalid("metadataDependencyUnavailable")),
            },
            None => Ok(None),
        }
    }
    let project = dependency(spec.project_file.as_deref())?;
    let title = dependency(spec.title_file.as_deref())?;
    let parts = if spec.provider == "opencode" && spec.path.is_dir() {
        Some(usage_source::Snapshot::tree(&spec.path.join("part"))?.stamp)
    } else {
        None
    };
    Ok(
        json!({"native":usage_source::normalized_stamp(&spec.provider,&spec.path)?,"project":project,"title":title,"parts":parts,"subagent":spec.subagent}),
    )
}
fn empty_entry(now: i64) -> Entry {
    Entry {
        version: VERSION,
        stamp: None,
        sessions: vec![],
        facts: vec![],
        messages: vec![],
        reasons: vec![],
        available: true,
        status: "failed".into(),
        priority: 0,
        attempted_at_ms: now,
        succeeded_at_ms: None,
    }
}
fn save(
    db: &Database,
    path: &str,
    provider: &str,
    token: &Option<String>,
    entry: &Entry,
) -> Result<(), AppError> {
    let mut conn = crate::database::lock_conn!(db.conn);
    let tx = conn.transaction()?;
    source_cache::merge_on(
        &tx,
        path,
        provider,
        None,
        Part::Analytics,
        token,
        Some(serde_json::to_value(entry).map_err(|_| invalid("invalidCacheValue"))?),
    )?;
    tx.commit()?;
    Ok(())
}
pub(crate) fn refresh_discovered(
    db: &Database,
    discovery: &Discovery,
) -> Result<RefreshReport, AppError> {
    refresh_sources_inner(db, &discovery.specs, &discovery.failures)
}

#[cfg(test)]
pub(crate) fn refresh_sources(
    db: &Database,
    specs: &[SourceSpec],
) -> Result<RefreshReport, AppError> {
    refresh_sources_inner(db, specs, &[])
}
fn refresh_sources_inner(
    db: &Database,
    specs: &[SourceSpec],
    failures: &[DiscoveryFailure],
) -> Result<RefreshReport, AppError> {
    if specs.len() > MAX_SOURCES {
        return Err(invalid("sourceBudgetExceeded"));
    }
    let rows = {
        let conn = crate::database::lock_conn!(db.conn);
        rows_on(&conn)?
    };
    let mut report = RefreshReport::default();
    let failed_providers: BTreeSet<_> = failures.iter().map(|f| f.provider.as_str()).collect();
    for failure in failures {
        report.failed += 1;
        report.mark(&failure.provider, RefreshState::Failed(failure.code));
    }
    let mut found = BTreeSet::new();
    for spec in specs {
        if failed_providers.contains(spec.provider.as_str()) {
            continue;
        }
        let Some(path) = spec.path.to_str() else {
            report.failed += 1;
            report.mark(
                &spec.provider,
                RefreshState::Failed("sourcePathUnavailable"),
            );
            continue;
        };
        if !found.insert(path.to_owned()) {
            return Err(invalid("duplicatePhysicalSource"));
        }
        let old = rows.get(path);
        if old.is_some_and(|r| r.provider != spec.provider || r.entry.is_err()) {
            report.failed += 1;
            report.mark(
                &spec.provider,
                RefreshState::Failed("analyticsCacheUnavailable"),
            );
            continue;
        }
        let previous = old
            .and_then(|r| r.entry.as_ref().ok())
            .and_then(Option::as_ref);
        let token = old.and_then(|r| r.token.clone());
        let now = chrono::Utc::now().timestamp_millis();
        let next_stamp = source_revision(spec);
        if let (Ok(next), Some(old)) = (&next_stamp, previous) {
            if old
                .stamp
                .as_ref()
                .is_some_and(|saved| same_known_revision(saved, next))
                && old.available
                && old.status != "failed"
            {
                report.unchanged += 1;
                report.mark(
                    &spec.provider,
                    if old.status == "ok" {
                        RefreshState::Ok
                    } else {
                        RefreshState::Partial("partialSource")
                    },
                );
                continue;
            }
        }
        let parsed = next_stamp.and_then(|before| {
            let mut entry = parse(spec, now)?;
            if source_revision(spec)? != before {
                return Err(invalid("sourceChangedRetry"));
            }
            entry.stamp = Some(before);
            Ok(entry)
        });
        let entry = match parsed {
            Ok(entry) => {
                report.parsed += 1;
                entry
            }
            Err(_) => {
                report.failed += 1;
                let mut entry = previous.cloned().unwrap_or_else(|| empty_entry(now));
                entry.status = "failed".into();
                entry.attempted_at_ms = now;
                entry.reasons.retain(|r| r != "sourceMissing");
                if !entry.reasons.iter().any(|r| r == "sourceReadFailed") {
                    entry.reasons.push("sourceReadFailed".into());
                }
                entry
            }
        };
        report.mark(
            &spec.provider,
            match entry.status.as_str() {
                "ok" => RefreshState::Ok,
                "failed" => RefreshState::Failed("sourceReadFailed"),
                _ => RefreshState::Partial("partialSource"),
            },
        );
        if save(db, path, &spec.provider, &token, &entry).is_err() {
            report.failed += 1;
            report.mark(
                &spec.provider,
                RefreshState::Failed("analyticsCacheWriteFailed"),
            );
        }
    }
    for (path, row) in rows {
        if found.contains(&path) {
            continue;
        }
        let Ok(Some(mut entry)) = row.entry else {
            if row.entry.is_err() {
                report.mark(
                    &row.provider,
                    RefreshState::Failed("analyticsCacheUnavailable"),
                );
            }
            continue;
        };
        if failed_providers.contains(row.provider.as_str()) {
            // Failed enumeration says nothing about deletion. Keep the last
            // successful facts and the manager reference, but expose staleness.
            entry.status = "failed".into();
            entry.attempted_at_ms = chrono::Utc::now().timestamp_millis();
            entry.reasons.retain(|r| r != "sourceDiscoveryFailed");
            entry.reasons.push("sourceDiscoveryFailed".into());
            if save(db, &path, &row.provider, &row.token, &entry).is_err() {
                report.failed += 1;
                report.mark(
                    &row.provider,
                    RefreshState::Failed("analyticsCacheWriteFailed"),
                );
            }
            continue;
        }
        if !entry.available {
            report.mark(&row.provider, RefreshState::Partial("sourceMissing"));
            continue;
        }
        entry.available = false;
        entry.status = "partial".into();
        report.mark(&row.provider, RefreshState::Partial("sourceMissing"));
        entry.reasons.push("sourceMissing".into());
        for session in &mut entry.sessions {
            session.source_available = false;
            session.manager_key = None;
        }
        if save(db, &path, &row.provider, &row.token, &entry).is_err() {
            report.failed += 1;
            report.mark(
                &row.provider,
                RefreshState::Failed("analyticsCacheWriteFailed"),
            );
        }
    }
    Ok(report)
}
pub(crate) struct ParsedSource {
    pub sessions: Vec<SessionDescriptor>,
    pub facts: Vec<UsageFact>,
    pub messages: Vec<MessageEvent>,
    pub reasons: Vec<String>,
}
fn parse(spec: &SourceSpec, now: i64) -> Result<Entry, AppError> {
    let mut entry = empty_entry(now);
    if spec.provider == "opencode" {
        let parsed = super::session_analytics_opencode::read(&spec.path)?;
        entry.sessions = parsed.sessions;
        entry.facts = parsed.facts;
        entry.messages = parsed.messages;
        entry.reasons = parsed.reasons;
        entry.status = if entry.reasons.is_empty() {
            "ok"
        } else {
            "partial"
        }
        .into();
        entry.priority = if spec.path.is_file() { 3 } else { 2 };
        entry.succeeded_at_ms = Some(now);
        return Ok(entry);
    }
    entry.priority = if spec.subagent { 1 } else { 2 };
    let mut candidates = crate::session_manager::title::TitleCandidates::default();
    let transcript = match spec.provider.as_str() {
        "claude" | "codex" => {
            let mut claude = crate::session_manager::providers::claude::ClaudeScan::default();
            let mut codex = crate::session_manager::providers::codex::CodexScan::default();
            let mut scan = TranscriptScan::new(
                &spec.provider,
                &key(&spec.provider, &spec.path.to_string_lossy()),
                spec.subagent,
            )?;
            let text = usage_source::read_text(&spec.path)?;
            let mut ambiguous_codex_identity = false;
            for (i, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let value: Value =
                    serde_json::from_str(line).map_err(|_| invalid("sourceReadFailed"))?;
                // Native fork/replay can retain both parent and child headers.
                // Neither header licenses attributing the whole physical file.
                // Continue validating JSON, but publish no inferred metrics.
                if ambiguous_codex_identity {
                    continue;
                }
                match scan.line(i as i64 + 1, &value) {
                    Err(AppError::InvalidInput(code))
                        if spec.provider == "codex"
                            && code == "usage.analytics.conflictingSessionIdentity" =>
                    {
                        ambiguous_codex_identity = true;
                        continue;
                    }
                    result => result?,
                }
                if spec.provider == "claude" {
                    claude.line(&value);
                } else {
                    codex.line(&value);
                }
            }
            if ambiguous_codex_identity {
                entry.status = "partial".into();
                entry.reasons.push("sessionIdentityUnavailable".into());
                entry.succeeded_at_ms = Some(now);
                return Ok(entry);
            }
            let transcript = scan.finish();
            candidates = if spec.provider == "claude" {
                claude.candidates
            } else {
                codex.candidates
            };
            if let (Some(path), Some(id)) = (&spec.title_file, &transcript.session_id) {
                candidates.agent = crate::session_manager::providers::codex::thread_names(path)
                    .get(id)
                    .cloned();
            }
            transcript
        }
        "gemini" => {
            let conversation =
                crate::session_manager::providers::gemini::read_conversation(&spec.path)
                    .map_err(|_| invalid("sourceReadFailed"))?;
            let mut transcript = TranscriptFacts {
                is_main: !spec.subagent,
                ..Default::default()
            };
            match conversation.get("kind").and_then(Value::as_str) {
                Some("main") => {}
                Some("subagent") => {
                    // Native 0.64 JSONL records the role, but not a verifiable
                    // parent session ID. Keep spending without main activity.
                    transcript.is_main = false;
                    transcript.reasons.insert("sessionParentUnavailable".into());
                }
                None if spec.path.extension().is_some_and(|ext| ext == "json") => {}
                _ => {
                    transcript.is_main = false;
                    transcript.reasons.insert("sessionRoleNotRecorded".into());
                }
            }
            transcript.session_id = conversation["sessionId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            if let Some(project) = &spec.project_file {
                match std::fs::read_to_string(project) {
                    Ok(path) => {
                        transcript.project_parsed = true;
                        transcript.project_dir = (!path.trim().is_empty()).then_some(path);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        transcript.reasons.insert("projectNotRecorded".into());
                    }
                    Err(_) => return Err(invalid("projectDependencyUnavailable")),
                }
            } else {
                transcript.reasons.insert("projectNotRecorded".into());
            }
            for (i, message) in conversation["messages"]
                .as_array()
                .ok_or_else(|| invalid("messagesUnavailable"))?
                .iter()
                .enumerate()
            {
                let role = match message["type"].as_str() {
                    Some("user") => "user",
                    Some("gemini") => "assistant",
                    _ => continue,
                };
                let content = crate::session_manager::providers::extract_text(&message["content"]);
                if (role == "user"
                    && crate::session_manager::title::clean_prompt(&content).is_none())
                    || content.trim().is_empty()
                {
                    continue;
                }
                if role == "user" && candidates.prompt.is_none() {
                    candidates = candidates.with_first_user_text(Some(&content));
                }
                let id = message["id"].as_str().filter(|s| !s.is_empty());
                if id.is_none() {
                    transcript
                        .reasons
                        .insert("messageNativeIdentityUnavailable".into());
                }
                let at_ms = message
                    .get("timestamp")
                    .and_then(crate::session_manager::providers::parse_timestamp_to_ms);
                if at_ms.is_none() {
                    transcript.reasons.insert("messageTimeUnavailable".into());
                }
                let identity = id
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("local:{i}"));
                transcript.messages.insert(
                    identity.clone(),
                    super::session_analytics_messages::MessageFact {
                        identity,
                        at_ms,
                        role: role.into(),
                        native_identity: id.is_some(),
                    },
                );
            }
            transcript
        }
        _ => return Err(invalid("unsupportedTranscriptReader")),
    };
    let normalized = usage_source::read_normalized(&spec.provider, &spec.path)?;
    entry.reasons.extend(normalized.reasons);
    let native_id = transcript.session_id.clone();
    if let Some(native) = &native_id {
        let session_key = key(&spec.provider, native);
        let project = if !transcript.project_parsed {
            Project::Unparsed
        } else {
            transcript
                .project_dir
                .as_deref()
                .map(|path| Project::Known(project_key(path)))
                .unwrap_or(Project::Unknown)
        };
        let source_path = spec.path.to_string_lossy().into_owned();
        let meta = crate::session_manager::SessionMeta {
            provider_id: spec.provider.clone(),
            session_id: native.clone(),
            source_path: Some(source_path),
            ..Default::default()
        };
        let project_label = transcript.project_dir.as_ref().map(|p| {
            Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone())
        });
        entry.sessions.push(SessionDescriptor {
            identity: SessionIdentity {
                key: session_key.clone(),
                tool_id: spec.provider.clone(),
                project,
                verified_root_key: transcript
                    .parent_session_id
                    .as_deref()
                    .filter(|p| *p != native)
                    .map(|id| key(&spec.provider, id)),
            },
            native_session_id: native.clone(),
            title: crate::session_manager::title::pick_title(
                &candidates.with_directory(transcript.project_dir.as_deref()),
            )
            .map(|(title, _)| crate::redact::scrub(&title).into_owned())
            .unwrap_or_else(|| native.clone()),
            project_label,
            project_path: transcript.project_dir,
            manager_key: Some(crate::session_manager::managed::key(&meta)),
            source_available: true,
            messages_supported: true,
            message_reasons: transcript.reasons.iter().cloned().collect(),
        });
        for message in transcript.messages.into_values() {
            if let Some(at_ms) = message.at_ms {
                entry.messages.push(MessageEvent {
                    session_key: session_key.clone(),
                    native_id: message.identity,
                    at_ms,
                    role: if message.role == "user" {
                        Role::User
                    } else {
                        Role::Assistant
                    },
                    main: transcript.is_main,
                });
            }
        }
    } else {
        entry.reasons.push("sessionIdentityUnavailable".into());
    }
    for fact in normalized.facts {
        let Some(at_ms) = fact.at_ms else {
            continue;
        };
        let identity = fact.native_key.as_deref().unwrap_or(&fact.request_id);
        entry.facts.push(UsageFact {
            event_key: key(&spec.provider, identity),
            session_key: fact.session_id.as_deref().map(|id| key(&spec.provider, id)),
            tool_id: spec.provider.clone(),
            source_id: source_id(&spec.provider).into(),
            model: fact.model,
            at_ms,
            buckets: [
                fact.usage.input_tokens,
                fact.usage.output_tokens,
                fact.usage.cache_read_tokens,
                fact.usage.cache_creation_tokens,
            ]
            .map(u64::from),
            complete: true,
        });
    }
    entry.reasons.extend(transcript.reasons);
    entry.reasons.sort();
    entry.reasons.dedup();
    entry.status = if entry.reasons.is_empty() {
        "ok"
    } else {
        "partial"
    }
    .into();
    entry.succeeded_at_ms = Some(now);
    Ok(entry)
}

fn health_severity(status: &str) -> u8 {
    match status {
        "unsupported" => 4,
        "failed" => 3,
        "partial" => 2,
        "unknown" => 1,
        _ => 0,
    }
}

/// A corrupt native parent component must not make an unrelated source query
/// fail. Clear unverifiable links with a scoped reason rather than guessing a
/// root. Memoizing each functional-graph chain keeps this linear in its edges.
fn isolate_invalid_roots(
    sessions: &mut BTreeMap<String, SessionDescriptor>,
    reasons: &mut Vec<SourceReason>,
) {
    let parents: BTreeMap<_, _> = sessions
        .iter()
        .map(|(key, s)| {
            (
                key.clone(),
                (
                    s.identity.tool_id.clone(),
                    s.identity.verified_root_key.clone(),
                ),
            )
        })
        .collect();
    let mut resolved: BTreeMap<String, bool> = BTreeMap::new();
    for start in parents.keys() {
        if resolved.contains_key(start) {
            continue;
        }
        let mut current = start.clone();
        let mut path = vec![];
        let mut visiting = BTreeSet::new();
        let valid = loop {
            if let Some(valid) = resolved.get(&current) {
                break *valid;
            }
            if !visiting.insert(current.clone()) {
                break false;
            }
            path.push(current.clone());
            let Some((tool, parent)) = parents.get(&current) else {
                break false;
            };
            let Some(parent) = parent else {
                break true;
            };
            if parents.get(parent).is_none_or(|(other, _)| other != tool) {
                break false;
            }
            current = parent.clone();
        };
        for key in path {
            resolved.insert(key, valid);
        }
    }
    for (key, valid) in resolved {
        if valid {
            continue;
        }
        let session = sessions.get_mut(&key).expect("known native graph key");
        session.identity.verified_root_key = None;
        session.messages_supported = false;
        session.message_reasons.push("invalidSessionRoot".into());
        reasons.push(SourceReason {
            tool_id: session.identity.tool_id.clone(),
            source_id: source_id(&session.identity.tool_id).into(),
            reason: "invalidSessionRoot".into(),
        });
    }
}

#[cfg(test)]
pub(crate) fn read_input(db: &Database) -> Result<AnalyticsInput, AppError> {
    let conn = crate::database::lock_conn!(db.conn);
    read_input_on(&conn)
}

pub(crate) fn read_input_on(conn: &rusqlite::Connection) -> Result<AnalyticsInput, AppError> {
    let rows = rows_on(conn)?;
    let mut input = AnalyticsInput {
        snapshot_version: String::new(),
        sessions: vec![],
        facts: vec![],
        messages: vec![],
        reasons: vec![],
        source_reasons: vec![],
        sources: vec![],
    };
    let mut hash = Sha256::new();
    let mut entries = Vec::new();
    for (path, row) in rows {
        hash.update(path.as_bytes());
        hash.update(row.raw.as_bytes());
        match row.entry {
            Ok(Some(entry)) => entries.push((row.provider, entry)),
            Ok(None) => {}
            Err(()) => input.source_reasons.push(SourceReason {
                tool_id: row.provider.clone(),
                source_id: source_id(&row.provider).into(),
                reason: "analyticsCacheUnavailable".into(),
            }),
        }
    }
    let current_sessions: BTreeSet<_> = entries
        .iter()
        .filter(|(_, e)| e.available && e.status != "failed")
        .flat_map(|(_, e)| e.sessions.iter().map(|s| s.identity.key.clone()))
        .collect();
    let current_facts: BTreeSet<_> = entries
        .iter()
        .filter(|(_, e)| e.available && e.status != "failed")
        .flat_map(|(_, e)| e.facts.iter().map(|f| f.event_key.clone()))
        .collect();
    let current_messages: BTreeSet<_> = entries
        .iter()
        .filter(|(_, e)| e.available && e.status != "failed")
        .flat_map(|(_, e)| {
            e.messages
                .iter()
                .map(|m| (m.session_key.clone(), m.native_id.clone()))
        })
        .collect();
    entries.sort_by_key(|(_, entry)| (entry.available && entry.status != "failed", entry.priority));
    let mut sessions: BTreeMap<String, SessionDescriptor> = BTreeMap::new();
    let mut priorities: BTreeMap<String, u8> = BTreeMap::new();
    let mut metadata_conflicts = BTreeSet::new();
    let mut message_conflicts = BTreeSet::new();
    let mut message_conflict_sessions = BTreeSet::new();
    let mut facts: BTreeMap<String, UsageFact> = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    let mut messages: BTreeMap<(String, String), MessageEvent> = BTreeMap::new();
    let mut health: BTreeMap<String, SourceHealth> = BTreeMap::new();
    for (provider, mut entry) in entries {
        if !entry.available || entry.status == "failed" {
            // Preserve absent history, but a currently observed native identity
            // supersedes an obsolete cached copy of that same identity. Only
            // simultaneous current copies can disagree as native replicas.
            entry
                .sessions
                .retain(|s| !current_sessions.contains(&s.identity.key));
            entry
                .facts
                .retain(|f| !current_facts.contains(&f.event_key));
            entry.messages.retain(|m| {
                !current_messages.contains(&(m.session_key.clone(), m.native_id.clone()))
            });
        }
        let source = source_id(&provider).to_owned();
        for reason in &entry.reasons {
            input.source_reasons.push(SourceReason {
                tool_id: provider.clone(),
                source_id: source.clone(),
                reason: reason.clone(),
            });
        }
        let health = health
            .entry(source.clone())
            .or_insert_with(|| SourceHealth {
                source_id: source,
                tool_id: provider.clone(),
                status: "ok".into(),
                last_attempt_at_ms: None,
                last_success_at_ms: entry.succeeded_at_ms,
                error_code: None,
                last_scan: None,
                retained_detail_range: None,
            });
        if entry.status != "ok" {
            if entry.status == "failed" || health.status != "failed" {
                health.status = if entry.status == "failed" {
                    "failed"
                } else {
                    "partial"
                }
                .into();
            }
            health.error_code = Some(
                if entry.status == "failed" {
                    "sourceReadFailed"
                } else {
                    "partialSource"
                }
                .into(),
            );
        }
        health.last_attempt_at_ms = Some(
            health
                .last_attempt_at_ms
                .unwrap_or(i64::MIN)
                .max(entry.attempted_at_ms),
        );
        health.last_success_at_ms = health
            .last_success_at_ms
            .zip(entry.succeeded_at_ms)
            .map(|(a, b)| a.min(b));
        for mut session in entry.sessions {
            let id = session.identity.key.clone();
            if let Some(prior) = sessions.get(&id) {
                if priorities.get(&id) == Some(&entry.priority)
                    && prior.identity != session.identity
                {
                    metadata_conflicts.insert(id.clone());
                    input.source_reasons.push(SourceReason {
                        tool_id: provider.clone(),
                        source_id: source_id(&provider).into(),
                        reason: "conflictingSessionMetadata".into(),
                    });
                }
            }
            if metadata_conflicts.contains(&id) {
                session.identity.project = Project::Unparsed;
                session.identity.verified_root_key = None;
                session.project_path = None;
                session.project_label = None;
                session.messages_supported = false;
                session
                    .message_reasons
                    .push("conflictingSessionMetadata".into());
            }
            priorities.insert(id.clone(), entry.priority);
            sessions.insert(id, session);
        }
        for fact in entry.facts {
            if let Some(prior) = facts.get(&fact.event_key) {
                if prior != &fact {
                    conflicts.insert(fact.event_key.clone());
                }
            }
            facts.insert(fact.event_key.clone(), fact);
        }
        for message in entry.messages {
            let k = (message.session_key.clone(), message.native_id.clone());
            if message_conflicts.contains(&k) {
                continue;
            }
            if let Some(prior) = messages.get(&k) {
                if prior != &message {
                    message_conflicts.insert(k.clone());
                    message_conflict_sessions.insert(message.session_key.clone());
                    messages.remove(&k);
                    input.source_reasons.push(SourceReason {
                        tool_id: provider.clone(),
                        source_id: source_id(&provider).into(),
                        reason: "conflictingMessageReplica".into(),
                    });
                    continue;
                }
            }
            messages.insert(k, message);
        }
    }
    for id in conflicts {
        if let Some(fact) = facts.remove(&id) {
            input.source_reasons.push(SourceReason {
                tool_id: fact.tool_id,
                source_id: fact.source_id,
                reason: "conflictingUsageReplica".into(),
            });
        }
    }
    let keys: BTreeSet<_> = sessions.keys().cloned().collect();
    for session in sessions.values_mut() {
        if message_conflict_sessions.contains(&session.identity.key) {
            session.messages_supported = false;
            session
                .message_reasons
                .push("conflictingMessageReplica".into());
        }
        if session
            .identity
            .verified_root_key
            .as_ref()
            .is_some_and(|key| !keys.contains(key))
        {
            session.identity.verified_root_key = None;
            session.messages_supported = false;
            session
                .message_reasons
                .push("sessionParentUnavailable".into());
            input.source_reasons.push(SourceReason {
                tool_id: session.identity.tool_id.clone(),
                source_id: source_id(&session.identity.tool_id).into(),
                reason: "sessionParentUnavailable".into(),
            });
        }
    }
    isolate_invalid_roots(&mut sessions, &mut input.source_reasons);
    input.sessions = sessions.into_values().collect();
    input.facts = facts.into_values().collect();
    input.messages = messages.into_values().collect();
    // Read the common health under this query's existing SQLite snapshot.
    // A successful old cache never upgrades an unattempted/failed channel.
    let common = super::usage_health::source_health_on(conn)?;
    for mut source in common
        .into_iter()
        .filter(|s| source_id(&s.tool_id) != "unsupported")
    {
        if let Some(cached) = health.remove(&source.source_id) {
            if health_severity(&cached.status) > health_severity(&source.status) {
                source.status = cached.status;
                source.error_code = cached.error_code;
            }
            source.last_attempt_at_ms = source
                .last_attempt_at_ms
                .into_iter()
                .chain(cached.last_attempt_at_ms)
                .max();
            source.last_success_at_ms = source
                .last_success_at_ms
                .zip(cached.last_success_at_ms)
                .map(|(a, b)| a.min(b));
        }
        input.sources.push(source);
    }
    input.sources.extend(health.into_values());
    if input.sessions.is_empty() && input.source_reasons.is_empty() {
        input.reasons.push("analyticsIndexEmpty".into());
    }
    input.source_reasons.sort_by(|a, b| {
        (&a.tool_id, &a.source_id, &a.reason).cmp(&(&b.tool_id, &b.source_id, &b.reason))
    });
    input.source_reasons.dedup_by(|a, b| {
        a.tool_id == b.tool_id && a.source_id == b.source_id && a.reason == b.reason
    });
    input.snapshot_version = format!("{:x}", hash.finalize());
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::source_cache::{self, Part};
    use serde_json::json;

    fn write(path: &std::path::Path, value: &serde_json::Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }
    fn row(sid: &str, id: &str, tokens: u64) -> serde_json::Value {
        json!({"type":"assistant","sessionId":sid,"cwd":"/project","timestamp":"2026-10-15T00:00:00.123Z",
            "message":{"id":id,"role":"assistant","model":"m","content":[{"type":"text","text":"not retained in cache"}],
                "usage":{"input_tokens":tokens,"output_tokens":2}}})
    }
    fn spec(path: &std::path::Path) -> SourceSpec {
        SourceSpec {
            provider: "claude".into(),
            path: path.to_owned(),
            subagent: false,
            project_file: None,
            title_file: None,
        }
    }
    fn oc_db(path: &Path) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE session(id TEXT PRIMARY KEY,parent_id TEXT,title TEXT,directory TEXT,time_created INTEGER,time_updated INTEGER);CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT); INSERT INTO session VALUES('ses_a',NULL,'A','/one/project',1,1),('ses_b',NULL,'B','/two/project',1,1);").unwrap();
        for (id, sid, input) in [("ma", "ses_a", 3), ("mb", "ses_b", 4)] {
            let data = json!({"id":id,"sessionID":sid,"role":"assistant","modelID":"m","tokens":{"input":input,"output":2},"time":{"created":1792022400123_i64}});
            conn.execute(
                "INSERT INTO message VALUES(?1,?2,1792022400123,1792022400123,?3)",
                rusqlite::params![id, sid, data.to_string()],
            )
            .unwrap();
        }
        conn
    }
    #[test]
    fn session_analytics_index_conflicting_project_replicas_do_not_choose_by_path_order() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        let mut one = row("s", "m", 3);
        one["cwd"] = json!("/a");
        write(&a, &one);
        one["cwd"] = json!("/b");
        write(&b, &one);
        refresh_sources(&db, &[spec(&a), spec(&b)]).unwrap();
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        assert_eq!(input.sessions.len(), 1);
        assert_eq!(input.sessions[0].identity.project, Project::Unparsed);
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "conflictingSessionMetadata"));
    }
    #[test]
    fn session_analytics_index_conflicting_message_times_are_unknown_not_first_replica_wins() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        for (path, time) in [(&a, "2026-10-14T00:00:00Z"), (&b, "2026-10-15T00:00:00Z")] {
            let user = json!({"type":"user","sessionId":"s","cwd":"/project","uuid":"same-user","timestamp":time,"message":{"role":"user","content":"question"}});
            std::fs::write(path, format!("{}\n{}", user, row("s", "m", 3))).unwrap();
        }
        refresh_sources(&db, &[spec(&a), spec(&b)]).unwrap();
        let input = read_input(&db).unwrap();
        assert!(!input
            .messages
            .iter()
            .any(|m| m.native_id == "user:same-user"));
        assert!(!input.sessions[0].messages_supported);
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "conflictingMessageReplica"));
    }
    #[test]
    fn session_analytics_index_opencode_shared_database_keeps_all_sessions_and_wal_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let native = oc_db(&path);
        let db = Database::memory().unwrap();
        let specs = [SourceSpec {
            provider: "opencode".into(),
            path,
            subagent: false,
            project_file: None,
            title_file: None,
        }];
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        let input = read_input(&db).unwrap();
        assert_eq!(input.sessions.len(), 2);
        assert_eq!(input.facts.len(), 2);
        assert!(input
            .sessions
            .iter()
            .any(|s| s.project_path.as_deref() == Some("/two/project")));
        let mut data: Value = serde_json::from_str(
            &native
                .query_row::<String, _, _>("SELECT data FROM message WHERE id='ma'", [], |r| {
                    r.get(0)
                })
                .unwrap(),
        )
        .unwrap();
        data["tokens"]["input"] = json!(20);
        native
            .execute(
                "UPDATE message SET data=?1 WHERE id='ma'",
                [data.to_string()],
            )
            .unwrap();
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        assert_eq!(
            read_input(&db)
                .unwrap()
                .facts
                .iter()
                .map(|f| f.buckets[0])
                .sum::<u64>(),
            24
        );
    }
    #[test]
    fn session_analytics_index_opencode_parent_table_only_change_refreshes_root_relation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let native = oc_db(&path);
        let db = Database::memory().unwrap();
        let specs = [SourceSpec {
            provider: "opencode".into(),
            path,
            subagent: false,
            project_file: None,
            title_file: None,
        }];
        refresh_sources(&db, &specs).unwrap();
        native
            .execute("UPDATE session SET parent_id='ses_a' WHERE id='ses_b'", [])
            .unwrap();
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        let input = read_input(&db).unwrap();
        let parent = input
            .sessions
            .iter()
            .find(|s| s.native_session_id == "ses_a")
            .unwrap();
        let child = input
            .sessions
            .iter()
            .find(|s| s.native_session_id == "ses_b")
            .unwrap();
        assert_eq!(
            child.identity.verified_root_key.as_deref(),
            Some(parent.identity.key.as_str())
        );
    }
    #[test]
    fn session_analytics_index_opencode_storage_metadata_is_a_dependency() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage");
        let db = Database::memory().unwrap();
        let session = path.join("session/project/ses_a.json");
        write(
            &session,
            &json!({"id":"ses_a","directory":"/one/project ","title":"A"}),
        );
        write(
            &path.join("message/ses_a/ma.json"),
            &json!({"id":"ma","sessionID":"ses_a","role":"assistant","modelID":"m","tokens":{"input":3,"output":2},"time":{"created":1792022400123_i64}}),
        );
        let specs = [SourceSpec {
            provider: "opencode".into(),
            path,
            subagent: false,
            project_file: None,
            title_file: None,
        }];
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        assert_eq!(
            read_input(&db).unwrap().sessions[0].project_path.as_deref(),
            Some("/one/project ")
        );
        write(
            &session,
            &json!({"id":"ses_a","directory":"/two/project","title":"B"}),
        );
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        assert_eq!(
            read_input(&db).unwrap().sessions[0].project_path.as_deref(),
            Some("/two/project")
        );
    }
    #[test]
    fn session_analytics_index_six_hundred_sources_reuse_cache_without_parsing_and_query_is_read_only(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let specs: Vec<_> = (0..601)
            .map(|i| {
                let path = dir.path().join(format!("{i}.jsonl"));
                write(&path, &row(&format!("s{i}"), &format!("m{i}"), 3));
                spec(&path)
            })
            .collect();
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 601);
        let second = refresh_sources(&db, &specs).unwrap();
        assert_eq!(second.parsed, 0);
        assert_eq!(second.unchanged, 601);
        let before: Vec<(String, String)> = db
            .conn
            .lock()
            .unwrap()
            .prepare("SELECT source_path,meta_json FROM session_meta_cache ORDER BY source_path")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let input = read_input(&db).unwrap();
        assert_eq!(input.sessions.len(), 601);
        assert_eq!(input.facts.len(), 601);
        assert_eq!(input.facts[0].at_ms, 1_792_022_400_123);
        let after: Vec<(String, String)> = db
            .conn
            .lock()
            .unwrap()
            .prepare("SELECT source_path,meta_json FROM session_meta_cache ORDER BY source_path")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(before, after);
        assert!(before
            .iter()
            .all(|(_, v)| !v.contains("not retained in cache")));
    }
    #[test]
    fn session_analytics_index_changed_source_replaces_contribution_and_metadata_survives() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("a.jsonl");
        write(&path, &row("s", "m", 3));
        let specs = [spec(&path)];
        refresh_sources(&db, &specs).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            source_cache::merge_on(&conn,path.to_str().unwrap(),"claude",Some("s"),Part::Metadata,&None,Some(json!({"parserVersion":crate::session_manager::meta_cache::PARSER_VERSION,"candidates":{"custom":"user title"}}))).unwrap();
        }
        write(&path, &row("s", "m", 999));
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        assert_eq!(input.facts[0].buckets[0], 999);
        let conn = db.conn.lock().unwrap();
        let parts = source_cache::read_on(&conn, path.to_str().unwrap(), "claude").unwrap();
        assert_eq!(
            parts.value(Part::Metadata).unwrap()["candidates"]["custom"],
            "user title"
        );
    }
    #[test]
    fn session_analytics_index_missing_source_retains_known_facts_and_marks_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("a.jsonl");
        write(&path, &row("s", "m", 3));
        refresh_sources(&db, &[spec(&path)]).unwrap();
        std::fs::remove_file(&path).unwrap();
        refresh_sources(&db, &[]).unwrap();
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        assert_eq!(input.sessions.len(), 1);
        assert!(!input.sessions[0].source_available);
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "sourceMissing"));
    }
    #[test]
    fn session_analytics_index_one_corrupt_source_preserves_last_success_and_other_source_updates()
    {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        write(&a, &row("a", "ma", 3));
        write(&b, &row("b", "mb", 4));
        let specs = [spec(&a), spec(&b)];
        refresh_sources(&db, &specs).unwrap();
        std::fs::write(&a, "{broken private payload").unwrap();
        write(&b, &row("b", "mb", 9));
        let result = refresh_sources(&db, &specs).unwrap();
        assert_eq!(result.failed, 1);
        assert_eq!(result.parsed, 1);
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 2);
        assert_eq!(input.facts.iter().map(|f| f.buckets[0]).sum::<u64>(), 12);
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "sourceReadFailed"));
        assert!(!format!("{input:?}").contains("private payload"));
    }
    #[test]
    fn session_analytics_index_future_owned_version_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("a.jsonl");
        write(&path, &row("s", "m", 3));
        {
            let conn = db.conn.lock().unwrap();
            source_cache::merge_on(
                &conn,
                path.to_str().unwrap(),
                "claude",
                Some("s"),
                Part::Analytics,
                &None,
                Some(json!({"version":99,"future":"preserve"})),
            )
            .unwrap();
        }
        let report = refresh_sources(&db, &[spec(&path)]).unwrap();
        assert_eq!(report.failed, 1);
        let conn = db.conn.lock().unwrap();
        let parts = source_cache::read_on(&conn, path.to_str().unwrap(), "claude").unwrap();
        assert_eq!(parts.value(Part::Analytics).unwrap()["version"], 99);
        assert_eq!(parts.value(Part::Analytics).unwrap()["future"], "preserve");
    }
    #[test]
    fn session_analytics_index_gemini_project_dependency_change_preserves_path_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("chats/session-a.json");
        let project = dir.path().join(".project_root");
        write(
            &path,
            &json!({"sessionId":"s","messages":[{"id":"m","type":"gemini","model":"m","timestamp":"2026-10-15T00:00:00Z","content":"reply","tokens":{"input":3,"output":2}}]}),
        );
        std::fs::write(&project, "/one/project").unwrap();
        let specs = [SourceSpec {
            provider: "gemini".into(),
            path,
            subagent: false,
            project_file: Some(project.clone()),
            title_file: None,
        }];
        refresh_sources(&db, &specs).unwrap();
        std::fs::write(&project, "/one/project ").unwrap();
        assert_eq!(refresh_sources(&db, &specs).unwrap().parsed, 1);
        assert_eq!(
            read_input(&db).unwrap().sessions[0].project_path.as_deref(),
            Some("/one/project ")
        );
    }
    #[test]
    fn session_analytics_index_project_ids_are_bounded_exact_paths_not_basenames() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let paths = [
            format!("/one/{}/project ", "long/".repeat(150)),
            "/two/project ".into(),
            "/two/project".into(),
        ];
        let specs: Vec<_> = paths
            .iter()
            .enumerate()
            .map(|(i, project)| {
                let file = dir.path().join(format!("{i}.jsonl"));
                let mut value = row(&format!("s{i}"), &format!("m{i}"), 3);
                value["cwd"] = json!(project);
                write(&file, &value);
                spec(&file)
            })
            .collect();
        refresh_sources(&db, &specs).unwrap();
        let input = read_input(&db).unwrap();
        let mut ids = BTreeSet::new();
        for session in &input.sessions {
            let Project::Known(id) = &session.identity.project else {
                panic!("known native path");
            };
            assert!(
                id.len() <= 512,
                "project filter must satisfy query ID limit"
            );
            assert_ne!(Some(id.as_str()), session.project_path.as_deref());
            assert!(paths
                .iter()
                .any(|p| Some(p.as_str()) == session.project_path.as_deref()));
            ids.insert(id.clone());
        }
        assert_eq!(
            ids.len(),
            3,
            "same basename and significant whitespace remain distinct"
        );
    }
    #[test]
    fn session_analytics_index_titles_follow_native_candidates_for_each_text_provider() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let claude = dir.path().join("claude.jsonl");
        std::fs::write(&claude, [
            json!({"type":"user","sessionId":"c","cwd":"/project","uuid":"u","timestamp":"2026-10-15T00:00:00Z","message":{"role":"user","content":"<context>metadata</context>\n[Image #1]\nActual request"}}),
            json!({"type":"custom-title","customTitle":"Native renamed title"}), row("c","m",3)
        ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let codex = dir.path().join("codex.jsonl");
        std::fs::write(&codex,[
            json!({"type":"session_meta","payload":{"id":"x","cwd":"/project","source":"cli"}}),
            json!({"type":"response_item","timestamp":"2026-10-15T00:00:00Z","payload":{"type":"message","role":"user","id":"u","content":[{"type":"input_text","text":"<div>How should this render?</div>"}]}})
        ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let gemini = dir.path().join("gemini.json");
        write(
            &gemini,
            &json!({"sessionId":"g","messages":[{"id":"u","type":"user","timestamp":"2026-10-15T00:00:00Z","content":"Explain the Gemini renderer"}]}),
        );
        let mut specs = vec![spec(&claude), spec(&codex), spec(&gemini)];
        specs[1].provider = "codex".into();
        specs[2].provider = "gemini".into();
        refresh_sources(&db, &specs).unwrap();
        let input = read_input(&db).unwrap();
        let titles: BTreeMap<_, _> = input
            .sessions
            .iter()
            .map(|s| (s.native_session_id.as_str(), s.title.as_str()))
            .collect();
        assert_eq!(titles.get("c"), Some(&"Native renamed title"));
        assert!(titles.get("x").unwrap().contains("How should this render?"));
        assert_eq!(titles.get("g"), Some(&"Explain the Gemini renderer"));
    }
    #[test]
    fn session_analytics_index_opencode_filename_is_not_native_message_identity() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let storage = dir.path().join("storage");
        write(
            &storage.join("session/p/s.json"),
            &json!({"id":"s","directory":"/p","title":"S"}),
        );
        for name in ["copy-a", "copy-b"] {
            write(
                &storage.join(format!("message/s/{name}.json")),
                &json!({"sessionID":"s","role":"assistant","modelID":"m","tokens":{"input":3,"output":2},"time":{"created":1792022400123_i64},"parts":[{"type":"text","text":"answer"}]}),
            );
        }
        let mut source = spec(&storage);
        source.provider = "opencode".into();
        refresh_sources(&db, &[source]).unwrap();
        let input = read_input(&db).unwrap();
        assert!(
            input.facts.is_empty(),
            "copies without native IDs cannot become independent usage calls"
        );
        assert!(
            input.messages.is_empty(),
            "filenames cannot certify message identity"
        );
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "missingNativeEventIdentity"));
        assert!(!input.sessions[0].messages_supported);
    }

    #[test]
    fn session_analytics_index_codex_name_index_is_a_revision_dependency() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("s.jsonl");
        let names = dir.path().join("session_index.jsonl");
        std::fs::write(&path,[json!({"type":"session_meta","payload":{"id":"x","cwd":"/p","source":"cli"}}),json!({"type":"event_msg","payload":{"type":"user_message","message":"First question"}})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let mut source = spec(&path);
        source.provider = "codex".into();
        source.title_file = Some(names.clone());
        refresh_sources(&db, &[source.clone()]).unwrap();
        let before = source_revision(&source).unwrap();
        std::fs::write(
            &names,
            json!({"id":"x","thread_name":"Native thread name"}).to_string(),
        )
        .unwrap();
        assert_ne!(
            before,
            source_revision(&source).unwrap(),
            "metadata-only update must invalidate analytics cache"
        );
        assert_eq!(refresh_sources(&db, &[source]).unwrap().parsed, 1);
        assert_eq!(
            read_input(&db).unwrap().sessions[0].title,
            "Native thread name"
        );
    }
    #[test]
    fn session_analytics_index_failed_discovery_preserves_last_success_without_claiming_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let claude = dir.path().join("c.jsonl");
        let codex = dir.path().join("x.jsonl");
        write(&claude, &row("c", "m", 3));
        std::fs::write(&codex,[json!({"type":"session_meta","payload":{"id":"x","cwd":"/p","source":"cli"}}),json!({"type":"event_msg","timestamp":"2026-10-15T00:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5,"output_tokens":2}}}})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let mut x = spec(&codex);
        x.provider = "codex".into();
        refresh_sources(&db, &[spec(&claude), x.clone()]).unwrap();
        let discovery = Discovery {
            specs: vec![x],
            failures: vec![DiscoveryFailure {
                provider: "claude".into(),
                code: "sourceDiscoveryFailed",
            }],
        };
        let report = refresh_discovered(&db, &discovery).unwrap();
        assert!(report.failed > 0);
        let input = read_input(&db).unwrap();
        let retained = input
            .sessions
            .iter()
            .find(|s| s.native_session_id == "c")
            .unwrap();
        assert!(
            retained.source_available,
            "an unreadable roster is not evidence of deletion"
        );
        assert!(retained.manager_key.is_some());
        assert!(input
            .facts
            .iter()
            .any(|f| f.tool_id == "claude" && f.buckets[0] == 3));
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.tool_id == "claude" && r.reason == "sourceDiscoveryFailed"));
        assert!(!input
            .source_reasons
            .iter()
            .any(|r| r.tool_id == "claude" && r.reason == "sourceMissing"));
    }
    #[test]
    fn session_analytics_index_cache_success_cannot_hide_common_failed_or_pending_health() {
        use crate::services::usage_health::{self, Channel, Outcome};
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("c.jsonl");
        write(&path, &row("c", "m", 3));
        refresh_sources(&db, &[spec(&path)]).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let ledger =
                usage_health::begin_on(&conn, "claude", "session_log", Channel::Ledger, 1000)
                    .unwrap();
            usage_health::finish_on(&conn, &ledger, Outcome::Ok, 1100).unwrap();
            let analytics =
                usage_health::begin_on(&conn, "claude", "session_log", Channel::Analytics, 1200)
                    .unwrap();
            usage_health::finish_on(
                &conn,
                &analytics,
                Outcome::Failed("usage.sources.readFailed"),
                1300,
            )
            .unwrap();
        }
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        let source = input
            .sources
            .iter()
            .find(|s| s.tool_id == "claude")
            .unwrap();
        assert_eq!(source.status, "failed");
        assert_eq!(source.last_success_at_ms, None);
        {
            let conn = db.conn.lock().unwrap();
            usage_health::begin_on(&conn, "claude", "session_log", Channel::Analytics, 1400)
                .unwrap();
        }
        let pending = read_input(&db).unwrap();
        assert_eq!(
            pending
                .sources
                .iter()
                .find(|s| s.tool_id == "claude")
                .unwrap()
                .status,
            "unknown"
        );
    }
    #[test]
    fn session_analytics_index_corrupt_common_health_remains_unknown_and_keeps_native_cache() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("c.jsonl");
        write(&path, &row("c", "m", 3));
        refresh_sources(&db, &[spec(&path)]).unwrap();
        db.set_setting(
            crate::services::usage_health::SETTING_KEY,
            "not valid health",
        )
        .unwrap();
        let input = read_input(&db).unwrap();
        assert_eq!(input.facts.len(), 1);
        let source = input
            .sources
            .iter()
            .find(|s| s.tool_id == "claude")
            .unwrap();
        assert_eq!(source.status, "unknown");
        assert_eq!(source.last_success_at_ms, None);
        assert_eq!(
            source.error_code.as_deref(),
            Some("usage.sources.invalidHealth")
        );
    }
    #[test]
    fn session_analytics_index_invalid_native_parent_cycle_is_scoped_and_does_not_block_a_healthy_tool(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let native = dir.path().join("opencode.db");
        let oc = oc_db(&native);
        oc.execute_batch(
            "UPDATE session SET parent_id=CASE id WHEN 'ses_a' THEN 'ses_b' ELSE 'ses_a' END",
        )
        .unwrap();
        let codex = dir.path().join("x.jsonl");
        std::fs::write(&codex,[json!({"type":"session_meta","payload":{"id":"x","cwd":"/p","source":"cli"}}),json!({"type":"event_msg","timestamp":"2026-10-15T00:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5,"output_tokens":2}}}})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let mut oc_spec = spec(&native);
        oc_spec.provider = "opencode".into();
        let mut cx_spec = spec(&codex);
        cx_spec.provider = "codex".into();
        refresh_sources(&db, &[oc_spec, cx_spec]).unwrap();
        let query = crate::services::usage_query::UsageQueryV1 {
            version: 1,
            start_ms: Some(1792022400000),
            end_ms: 1792108800000,
            tool_ids: vec!["codex".into()],
            model_ids: vec![],
            project_ids: vec![],
            source_ids: vec![],
        };
        let result = crate::services::session_analytics_api::query(
            &db,
            query.clone(),
            "ofox.ai",
            1792108800000,
            &chrono::Utc,
        )
        .unwrap();
        assert_eq!(result.totals.tokens.total_tokens, 7);
        assert!(!result
            .coverage
            .reasons
            .iter()
            .any(|r| r == "invalidSessionRoot"));
        let mut all = query;
        all.tool_ids.clear();
        let result = crate::services::session_analytics_api::query(
            &db,
            all,
            "ofox.ai",
            1792108800000,
            &chrono::Utc,
        )
        .unwrap();
        assert_eq!(result.coverage.state, "partial");
        assert!(result
            .coverage
            .reasons
            .iter()
            .any(|r| r == "invalidSessionRoot"));
    }
    #[test]
    fn session_analytics_index_unknown_stamp_peer_does_not_reparse_an_unchanged_source() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let path = dir.path().join("c.jsonl");
        write(&path, &row("c", "m", 3));
        refresh_sources(&db, &[spec(&path)]).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let parts = source_cache::read_on(&conn, path.to_str().unwrap(), "claude").unwrap();
            let mut value = parts.value(Part::Analytics).unwrap().clone();
            value["stamp"]["futurePeer"] = json!({"preserve":"opaque"});
            value["stamp"]["native"]["futurePhysicalPeer"] = json!("opaque");
            source_cache::merge_on(
                &conn,
                path.to_str().unwrap(),
                "claude",
                None,
                Part::Analytics,
                &parts.token(Part::Analytics),
                Some(value),
            )
            .unwrap();
        }
        let warm = refresh_sources(&db, &[spec(&path)]).unwrap();
        assert_eq!(warm.parsed, 0);
        assert_eq!(warm.unchanged, 1);
        let conn = db.conn.lock().unwrap();
        let parts = source_cache::read_on(&conn, path.to_str().unwrap(), "claude").unwrap();
        assert_eq!(
            parts.value(Part::Analytics).unwrap()["stamp"]["futurePeer"]["preserve"],
            "opaque"
        );
        assert_eq!(
            parts.value(Part::Analytics).unwrap()["stamp"]["native"]["futurePhysicalPeer"],
            "opaque"
        );
    }
    #[test]
    fn session_analytics_index_moved_revised_source_supersedes_its_missing_cached_replica() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let old = dir.path().join("old.jsonl");
        let new = dir.path().join("new.jsonl");
        let mut before = row("s", "m", 3);
        before["cwd"] = json!("/before");
        write(&old, &before);
        refresh_sources(&db, &[spec(&old)]).unwrap();
        std::fs::rename(&old, &new).unwrap();
        let mut revised = row("s", "m", 9);
        revised["cwd"] = json!("/after");
        revised["timestamp"] = json!("2026-10-15T00:01:00Z");
        write(&new, &revised);
        refresh_sources(&db, &[spec(&new)]).unwrap();
        let input = read_input(&db).unwrap();
        assert_eq!(
            input.facts.len(),
            1,
            "missing old replicas are not current conflicting calls"
        );
        assert_eq!(input.facts[0].buckets[0], 9);
        assert_eq!(input.messages.len(), 1);
        assert_eq!(input.messages[0].at_ms, 1792022460000);
        assert_eq!(input.sessions.len(), 1);
        assert_eq!(input.sessions[0].project_path.as_deref(), Some("/after"));
        assert!(input.sessions[0].source_available);
        assert!(input
            .source_reasons
            .iter()
            .any(|r| r.reason == "sourceMissing"));
        assert!(!input.source_reasons.iter().any(|r| matches!(
            r.reason.as_str(),
            "conflictingUsageReplica" | "conflictingMessageReplica" | "conflictingSessionMetadata"
        )));
    }
}
