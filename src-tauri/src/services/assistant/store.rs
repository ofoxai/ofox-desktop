//! File store for assistant conversations, one folder per signed-in owner.
//!
//! This module is the only place that knows the layout:
//!
//! ```text
//! <app_config_dir>/assistant/<owner>/prefs.json                 {"version":1,"modelId":…}
//! <app_config_dir>/assistant/<owner>/<session_id>/session.json   SessionRecord + "version"
//! <app_config_dir>/assistant/<owner>/<session_id>/session.jsonl  native runtime history
//! <app_config_dir>/assistant/<owner>/<session_id>/attachments/<attachment_id>  imported files
//! ```
//!
//! Every write replaces the whole file atomically (temp file + rename), and
//! writers to one file are serialized in this process. A damaged file is an
//! error for that session; it is never read as, or overwritten by, an empty one.
mod backup;
mod lifecycle;
pub use backup::{BackupReport, RestoreReport, TRANSFER_BUSY};

use super::types::*;
use crate::settings::KeyOwner;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

const FORMAT_VERSION: u32 = 1;
const ASSISTANT_DIR: &str = "assistant";
const SESSION_FILE: &str = "session.json";
const NATIVE_HISTORY_FILE: &str = "session.jsonl";
const PREFS_FILE: &str = "prefs.json";
const ATTACHMENTS_DIR: &str = "attachments";

const MAX_TITLE_BYTES: usize = 512;
const MAX_DRAFT_BYTES: usize = 128_000;
const MAX_DRAFT_ATTACHMENTS: usize = 200;

const NOT_FOUND: &str = "assistant_session_not_found";
const INVALID_SESSION: &str = "assistant_invalid_session";
const INVALID_PREFERENCES: &str = "assistant_invalid_preferences";
const INVALID_PATH: &str = "assistant_invalid_storage_path";
const UNAVAILABLE: &str = "assistant_storage_unavailable";

/// The storage key of a signed-in account: a hash of user and organization, so
/// two organizations of one user never share conversations.
pub fn owner_key(owner: &KeyOwner) -> Result<String, String> {
    let user = owner.user_id.as_deref().unwrap_or("");
    let org = owner.org_id.as_deref().unwrap_or("");
    if user.is_empty() || org.is_empty() {
        return Err("assistant_identity_unavailable".into());
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(format!("{user}\0{org}").as_bytes())
    ))
}

/// `<config_dir>/assistant/<owner>`. Only an [`owner_key`] (64 lowercase hex
/// digits) is accepted, so the owner can never name a path outside the
/// assistant folder, nor a second folder on a case-insensitive disk.
pub fn owner_dir(config_dir: &Path, owner: &str) -> Result<PathBuf, String> {
    let lower_hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
    if owner.len() != 64 || !owner.bytes().all(lower_hex) {
        return Err(INVALID_PATH.into());
    }
    Ok(config_dir.join(ASSISTANT_DIR).join(owner))
}

/// Everything `session.json` holds. `operations` is the host's run ledger and
/// is left out of [`Detail`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub session: Session,
    pub messages: Vec<Message>,
    pub attachments: Vec<Attachment>,
    pub usage: Value,
    pub operations: Vec<Operation>,
}

impl SessionRecord {
    pub fn into_detail(self) -> Detail {
        Detail {
            session: self.session,
            messages: self.messages,
            attachments: self.attachments,
            usage: self.usage,
        }
    }
}

#[derive(Serialize)]
struct SessionFileOut<'a> {
    version: u32,
    #[serde(flatten)]
    record: &'a SessionRecord,
}

#[derive(Deserialize)]
struct SessionFileIn {
    version: u32,
    #[serde(flatten)]
    record: SessionRecord,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Prefs {
    version: u32,
    model_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionList {
    pub sessions: Vec<Session>,
    /// Session folders whose `session.json` could not be read, whatever the
    /// filter: their title, content and trash state are unknown.
    pub unreadable: Vec<UnreadableSession>,
    /// Readable conversations in the trash, whatever the filter and search.
    pub trash_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadableSession {
    pub id: String,
    pub error: String,
}

/// One owner's conversations. Cheap to build; holds no open files. Every
/// method does synchronous file IO, and the writers wait on a per-file lock, so
/// async code calls them inside `tauri::async_runtime::spawn_blocking`.
pub struct AssistantStore {
    config_dir: PathBuf,
    owner: String,
    root: PathBuf,
}

impl AssistantStore {
    pub fn for_owner(config_dir: &Path, owner: &str) -> Result<Self, String> {
        Ok(Self {
            root: owner_dir(config_dir, owner)?,
            config_dir: config_dir.to_path_buf(),
            owner: owner.to_string(),
        })
    }

    /// Blocking file IO: async callers run it in `spawn_blocking`.
    pub fn create(&self, model_id: &str, web_search: bool) -> Result<Detail, String> {
        let at = now();
        let record = SessionRecord {
            session: Session {
                id: id(),
                title: String::new(),
                model_id: model_id.into(),
                web_search,
                created_at: at,
                updated_at: at,
                deleted_at: None,
                active_run: None,
                draft: String::new(),
                draft_attachment_ids: vec![],
                cleanup_warning: None,
            },
            messages: vec![],
            attachments: vec![],
            usage: json!({}),
            operations: vec![],
        };
        let directory = private_subdir(&self.ensure_root()?, &record.session.id)?;
        let path = directory.join(SESSION_FILE);
        let lock = file_lock(&path);
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        write_record(&path, &record)?;
        Ok(record.into_detail())
    }

    /// Sessions in the trash (`deleted`) or out of it, most recently updated
    /// first (ties by id). `search` matches the title or any message, ignoring
    /// case. The trash count covers the whole trash either way.
    ///
    /// Blocking file IO (reads every session): async callers run it in
    /// `spawn_blocking`.
    pub fn list(&self, deleted: bool, search: &str) -> Result<SessionList, String> {
        let mut list = SessionList::default();
        if !real_dir(&self.root)? {
            return Ok(list);
        }
        let search = search.to_lowercase();
        for sid in self.session_ids()? {
            match read_listed(&self.root, &sid) {
                Ok(Some(record)) => {
                    let trashed = record.session.deleted_at.is_some();
                    if trashed {
                        list.trash_count += 1;
                    }
                    if trashed == deleted && matches_search(&record, &search) {
                        list.sessions.push(record.session);
                    }
                }
                Ok(None) => {}
                Err(error) => list.unreadable.push(UnreadableSession { id: sid, error }),
            }
        }
        list.sessions.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(list)
    }

    pub fn load_detail(&self, sid: &str) -> Result<Detail, String> {
        self.load_record(sid).map(SessionRecord::into_detail)
    }

    /// The whole record, run ledger included, for the host.
    pub fn load_record(&self, sid: &str) -> Result<SessionRecord, String> {
        validate_id(sid)?;
        read_record(&self.root, sid)
    }

    /// Read, change and save one session while holding its write lock. Nothing
    /// is saved when `change` fails. `change` must not call back into the store.
    ///
    /// Blocking file IO under a blocking lock: async callers run it in
    /// `spawn_blocking`, never directly on a runtime worker.
    pub fn mutate<T>(
        &self,
        sid: &str,
        change: impl FnOnce(&mut SessionRecord) -> Result<T, String>,
    ) -> Result<T, String> {
        validate_id(sid)?;
        let path = self.root.join(sid).join(SESSION_FILE);
        let lock = file_lock(&path);
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut record = read_record(&self.root, sid)?;
        let result = change(&mut record)?;
        // Never save what the next read would refuse.
        check_record(&record, sid)?;
        write_record(&path, &record)?;
        Ok(result)
    }

    /// Apply the fields an [`UpdateRequest`] carries. `precondition` runs under
    /// the session's write lock before anything changes; the host
    /// re-checks its authority generation there, so an update raced by a logout
    /// or account switch fails instead of landing. `precondition` runs under
    /// the lock and must not call back into the store. The request's owner and
    /// generation fields are the host's to compare, not the store's.
    ///
    /// Blocking file IO: async callers run it in `spawn_blocking`.
    pub fn update(
        &self,
        request: &UpdateRequest,
        precondition: impl FnOnce() -> Result<(), String>,
    ) -> Result<Session, String> {
        self.with_attachment_lock(&request.session_id, || {
            let (mut session, shrank) = self.mutate(&request.session_id, |record| {
                precondition()?;
                let previous = record.session.draft_attachment_ids.clone();
                apply_update(record, request)?;
                let shrank = previous
                    .iter()
                    .any(|id| !record.session.draft_attachment_ids.contains(id));
                let removed: HashSet<_> = previous
                    .into_iter()
                    .filter(|id| !record.session.draft_attachment_ids.contains(id))
                    .filter(|id| {
                        !record
                            .messages
                            .iter()
                            .any(|m| m.attachments.iter().any(|a| &a.id == id))
                    })
                    .collect();
                if !removed.is_empty() {
                    // A live reader can access any file of its conversation.
                    if record.session.active_run.is_some() {
                        return Err("assistant_session_busy".into());
                    }
                    record
                        .attachments
                        .retain(|item| !removed.contains(&item.id));
                }
                Ok((record.session.clone(), shrank))
            })?;
            // Persist removed references first. A crash can then leave only
            // an unreferenced copy, never a saved draft pointing at a lost file.
            // Only a draft that dropped files can have left copies to remove.
            if shrank && self.cleanup_orphans_locked(&request.session_id).is_err() {
                // The update is durable. Return it so host side effects and
                // renderer state still follow the committed settings.
                session.cleanup_warning = Some("assistant_attachment_cleanup_failed".into());
            }
            Ok(session)
        })
    }

    /// Serializes file imports and cleanup before taking the session record
    /// lock. An in-flight import must not look like a crash orphan to cleanup.
    pub(super) fn with_attachment_lock<T>(
        &self,
        sid: &str,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        validate_id(sid)?;
        let lock = file_lock(&self.root.join(sid).join(ATTACHMENTS_DIR));
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        action()
    }

    /// Remove copies a crashed import never published. Unknown filenames and
    /// directories are left alone; symlinks are removed, never followed.
    pub fn cleanup_orphan_attachments(&self, sid: &str) -> Result<usize, String> {
        self.with_attachment_lock(sid, || self.cleanup_orphans_locked(sid))
    }

    /// Requires the attachment IO lock. Readers publish their active run under
    /// the record lock before opening files, so cleanup never races a reader.
    pub(super) fn cleanup_orphans_locked(&self, sid: &str) -> Result<usize, String> {
        let lock = file_lock(&self.root.join(sid).join(SESSION_FILE));
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut record = read_record(&self.root, sid)?;
        if record.session.active_run.is_some() {
            return Ok(0);
        }
        let known: HashSet<String> = record
            .session
            .draft_attachment_ids
            .iter()
            .cloned()
            .chain(
                record
                    .messages
                    .iter()
                    .flat_map(|m| &m.attachments)
                    .map(|a| a.id.clone()),
            )
            .collect();
        let before = record.attachments.len();
        record.attachments.retain(|a| known.contains(&a.id));
        if record.attachments.len() != before {
            // Older versions could publish import metadata before saving the
            // draft. Reclaim those unused entries before touching their bytes.
            check_record(&record, sid)?;
            write_record(&self.root.join(sid).join(SESSION_FILE), &record)?;
        }
        let dir = self.attachment_root(sid)?;
        if !real_dir(&dir)? {
            return Ok(0);
        }
        let mut removed = 0;
        let mut failed = false;
        for entry in fs::read_dir(dir).map_err(|_| "assistant_attachment_cleanup_failed")? {
            let entry = entry.map_err(|_| "assistant_attachment_cleanup_failed")?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if validate_id(name).is_err() || known.contains(name) {
                continue;
            }
            let kind = entry
                .file_type()
                .map_err(|_| "assistant_attachment_cleanup_failed")?;
            if kind.is_file() || kind.is_symlink() {
                // One copy that cannot go must not keep the others.
                match fs::remove_file(entry.path()) {
                    Ok(()) => removed += 1,
                    Err(error) => {
                        log::warn!(
                            "[Assistant] could not remove an unused attachment copy: {error}"
                        );
                        failed = true;
                    }
                }
            }
        }
        if failed {
            return Err("assistant_attachment_cleanup_failed".into());
        }
        Ok(removed)
    }

    /// Blocking file IO: async callers run it in `spawn_blocking`.
    pub fn remember_model(&self, model_id: &str) -> Result<(), String> {
        let path = self.ensure_root()?.join(PREFS_FILE);
        let lock = file_lock(&path);
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        ensure_regular_or_absent(&path)?;
        let prefs = Prefs {
            version: FORMAT_VERSION,
            model_id: Some(model_id.into()),
        };
        write_bytes(&path, &serde_json::to_vec(&prefs).map_err(|_| UNAVAILABLE)?)
    }

    pub fn remembered_model(&self) -> Result<Option<String>, String> {
        if !real_dir(&self.root)? {
            return Ok(None);
        }
        let Some(bytes) = read_regular(&self.root.join(PREFS_FILE))? else {
            return Ok(None);
        };
        let prefs: Prefs = serde_json::from_slice(&bytes).map_err(|_| INVALID_PREFERENCES)?;
        if prefs.version != FORMAT_VERSION {
            return Err(INVALID_PREFERENCES.into());
        }
        Ok(prefs.model_id)
    }

    /// Token and cost totals over every session of this owner, trash included.
    /// A cost is only given when every session's usage is known and complete:
    /// an interrupted run, an unpriced request or an unreadable session turns
    /// the estimate into `null` instead of a low number.
    pub fn usage_summary(&self) -> Result<Value, String> {
        let mut totals = UsageTotals::new();
        if real_dir(&self.root)? {
            for sid in self.session_ids()? {
                match read_listed(&self.root, &sid) {
                    Ok(Some(record)) => totals.add(&record),
                    Ok(None) => {}
                    Err(_) => totals.priced = false,
                }
            }
        }
        Ok(totals.into_json())
    }

    /// Where the runtime keeps this session's native history: next to
    /// `session.json`. The runtime derives its own `<sid>/workspace` and
    /// `<sid>/runtime-settings` folders from this path.
    pub fn native_history_path(&self, sid: &str) -> Result<PathBuf, String> {
        validate_id(sid)?;
        if !real_dir(&self.root)? || !real_dir(&self.root.join(sid))? {
            return Err(NOT_FOUND.into());
        }
        let path = self.root.join(sid).join(NATIVE_HISTORY_FILE);
        ensure_regular_or_absent(&path)?;
        Ok(path)
    }

    /// `<sid>/attachments`, where imported files are kept as `<attachment id>`:
    /// the runtime's `attachmentRoot`. Only names the folder.
    pub fn attachment_root(&self, sid: &str) -> Result<PathBuf, String> {
        validate_id(sid)?;
        Ok(self.root.join(sid).join(ATTACHMENTS_DIR))
    }

    /// [`Self::attachment_root`] of a session that exists, created private
    /// (0o700) if needed; a link anywhere on the way is refused.
    ///
    /// Blocking file IO: async callers run it in `spawn_blocking`.
    pub fn attachment_dir(&self, sid: &str) -> Result<PathBuf, String> {
        validate_id(sid)?;
        if !real_dir(&self.root)? || !real_dir(&self.root.join(sid))? {
            return Err(NOT_FOUND.into());
        }
        private_subdir(&self.root.join(sid), ATTACHMENTS_DIR)
    }

    fn ensure_root(&self) -> Result<PathBuf, String> {
        private_subdir(
            &private_subdir(&self.config_dir, ASSISTANT_DIR)?,
            &self.owner,
        )
    }

    /// Names under the owner folder that are session ids; `prefs.json` and
    /// anything foreign are skipped.
    fn session_ids(&self) -> Result<Vec<String>, String> {
        let mut ids = vec![];
        for entry in fs::read_dir(&self.root).map_err(|_| UNAVAILABLE)? {
            let entry = entry.map_err(|_| UNAVAILABLE)?;
            if let Some(name) = entry.file_name().to_str() {
                if validate_id(name).is_ok() {
                    ids.push(name.to_string());
                }
            }
        }
        Ok(ids)
    }
}

/// Refuse an operation id the session already knows: a resubmission must not
/// start a second request, and a stop that arrived first must win.
pub fn ensure_new_operation(record: &SessionRecord, operation_id: &str) -> Result<(), String> {
    match record.operations.iter().find(|op| op.id == operation_id) {
        None => Ok(()),
        Some(op) if op.status == OperationStatus::Cancelled => Err("assistant_cancelled".into()),
        Some(_) => Err("assistant_operation_already_submitted".into()),
    }
}

/// A new user turn takes the composer draft with it; editing or regenerating an
/// earlier turn leaves the unsent draft alone.
pub fn consume_draft(session: &mut Session, request: &SendRequest) {
    if request.edit_message_id.is_none() && request.regenerate_message_id.is_none() {
        session.draft.clear();
        session.draft_attachment_ids.clear();
    }
}

fn apply_update(record: &mut SessionRecord, request: &UpdateRequest) -> Result<(), String> {
    if let Some(title) = &request.title {
        if title.len() > MAX_TITLE_BYTES {
            return Err("assistant_title_too_long".into());
        }
        record.session.title = title.clone();
    }
    if let Some(model) = &request.model_id {
        record.session.model_id = model.clone();
    }
    if let Some(web) = request.web_search {
        record.session.web_search = web;
    }
    if let Some(deleted) = request.deleted {
        record.session.deleted_at = deleted.then(now);
    }
    if let Some(draft) = &request.draft {
        if draft.len() > MAX_DRAFT_BYTES {
            return Err("assistant_message_too_large".into());
        }
        record.session.draft = draft.clone();
    }
    if let Some(ids) = &request.draft_attachment_ids {
        let selectable = |id: &String| {
            record
                .attachments
                .iter()
                .any(|a| &a.id == id && a.selectable())
        };
        if ids.len() > MAX_DRAFT_ATTACHMENTS || !ids.iter().all(selectable) {
            return Err("assistant_attachment_not_found".into());
        }
        record.session.draft_attachment_ids = ids.clone();
    }
    record.session.updated_at = now();
    Ok(())
}

fn matches_search(record: &SessionRecord, search: &str) -> bool {
    search.is_empty()
        || record.session.title.to_lowercase().contains(search)
        || record
            .messages
            .iter()
            .any(|m| m.content.to_lowercase().contains(search))
}

struct UsageTotals {
    sessions: u64,
    runs: u64,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    searches: u64,
    estimate: rust_decimal::Decimal,
    search_estimate: rust_decimal::Decimal,
    /// Whether every session's cost is known; see [`AssistantStore::usage_summary`].
    priced: bool,
}

impl UsageTotals {
    fn new() -> Self {
        Self {
            sessions: 0,
            runs: 0,
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            searches: 0,
            estimate: rust_decimal::Decimal::ZERO,
            search_estimate: rust_decimal::Decimal::ZERO,
            priced: true,
        }
    }

    fn add(&mut self, record: &SessionRecord) {
        let usage = &record.usage;
        self.sessions += 1;
        let interrupted = record.session.active_run.is_some()
            || record.messages.iter().any(|m| m.status == "streaming")
            || record
                .operations
                .iter()
                .any(|op| op.status == OperationStatus::Running);
        if interrupted || usage["usageCoverage"] == "incomplete" {
            self.priced = false;
        }
        let requests = usage["modelRequests"].as_u64().unwrap_or(0);
        self.runs += requests;
        self.input += usage["inputTokens"].as_u64().unwrap_or(0);
        self.output += usage["outputTokens"].as_u64().unwrap_or(0);
        self.cache_read += usage["cacheReadTokens"].as_u64().unwrap_or(0);
        self.cache_write += usage["cacheWriteTokens"].as_u64().unwrap_or(0);
        self.searches += usage["searchCount"].as_u64().unwrap_or(0);
        match decimal(&usage["estimatedCostUsd"]) {
            Some(value) => self.estimate += value,
            None if requests > 0 => self.priced = false,
            None => {}
        }
        // Already part of the session's estimate; kept apart for display.
        if let Some(value) = decimal(&usage["searchCostUsd"]) {
            self.search_estimate += value;
        }
    }

    fn into_json(self) -> Value {
        let priced = self.priced;
        let price = |value: rust_decimal::Decimal| priced.then(|| value.normalize().to_string());
        json!({
            "sessionCount": self.sessions,
            "runCount": self.runs,
            "inputTokens": self.input,
            "outputTokens": self.output,
            "cacheReadTokens": self.cache_read,
            "cacheWriteTokens": self.cache_write,
            "estimatedCostUsd": price(self.estimate),
            "billedCostUsd": null,
            "searchCount": self.searches,
            "usageCoverage": if priced { "complete" } else { "incomplete" },
            "searchCostUsd": price(self.search_estimate),
        })
    }
}

/// A decimal cost saved as a string or a JSON number.
fn decimal(value: &Value) -> Option<rust_decimal::Decimal> {
    match value {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.to_string().parse().ok(),
        _ => None,
    }
}

fn read_record(root: &Path, sid: &str) -> Result<SessionRecord, String> {
    if !real_dir(root)? || !real_dir(&root.join(sid))? {
        return Err(NOT_FOUND.into());
    }
    let bytes = read_regular(&root.join(sid).join(SESSION_FILE))?.ok_or(NOT_FOUND)?;
    decode(&bytes, sid)
}

/// A session folder found by listing: its record, `None` for a folder without
/// content (a creation that never wrote its file, or one removed meanwhile),
/// or an error. A folder with content but no `session.json` (say, native
/// history alone) is an error, so a conversation is never hidden silently.
fn read_listed(root: &Path, sid: &str) -> Result<Option<SessionRecord>, String> {
    match read_record(root, sid) {
        Ok(record) => return Ok(Some(record)),
        Err(error) if error == NOT_FOUND => {}
        Err(error) => return Err(error),
    }
    // `create` may be between its temp file and the rename: wait for its lock,
    // then look again.
    let lock = file_lock(&root.join(sid).join(SESSION_FILE));
    let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
    match read_record(root, sid) {
        Ok(record) => return Ok(Some(record)),
        Err(error) if error == NOT_FOUND => {}
        Err(error) => return Err(error),
    }
    if !has_content(&root.join(sid))? {
        return Ok(None);
    }
    log::warn!("assistant session {sid} has files but no {SESSION_FILE}");
    Err(INVALID_SESSION.into())
}

/// Whether a session folder holds anything but the temp file of a
/// `session.json` write that never finished (`config::atomic_write_with`
/// names it `.session.json.ofox-tmp-…`).
fn has_content(dir: &Path) -> Result<bool, String> {
    let unfinished = format!(".{SESSION_FILE}.ofox-tmp-");
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|_| UNAVAILABLE)?;
                let name = entry.file_name();
                if !name.to_str().is_some_and(|n| n.starts_with(&unfinished)) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(_) => Err(UNAVAILABLE.into()),
    }
}

/// A saved record for use. A message's `errorCode` that is not an
/// `assistant_*` code (a hand-edited or foreign file) is dropped: only the
/// host's stable codes are ever shown or saved.
fn decode(bytes: &[u8], sid: &str) -> Result<SessionRecord, String> {
    let mut record = parse_record(bytes, sid)?;
    for message in record.messages.iter_mut().filter(|m| unknown_code(m)) {
        log::warn!("assistant session {sid} has a message with an unknown error code");
        message.error_code = None;
    }
    check_record(&record, sid)?;
    Ok(record)
}

/// A saved record exactly as written, for backup and restore: what
/// [`decode`] would repair makes it invalid instead.
fn decode_exact(bytes: &[u8], sid: &str) -> Result<SessionRecord, String> {
    let record = parse_record(bytes, sid)?;
    check_record(&record, sid)?;
    Ok(record)
}

fn parse_record(bytes: &[u8], sid: &str) -> Result<SessionRecord, String> {
    let file: SessionFileIn = serde_json::from_slice(bytes).map_err(|error| {
        // Line and column only: the message can quote conversation text.
        log::warn!(
            "assistant session {sid} is unreadable: {:?} error at line {} column {}",
            error.classify(),
            error.line(),
            error.column()
        );
        INVALID_SESSION
    })?;
    if file.version != FORMAT_VERSION {
        log::warn!(
            "assistant session {sid} has unsupported version {}",
            file.version
        );
        return Err(INVALID_SESSION.into());
    }
    Ok(file.record)
}

/// A record belongs in `<sid>/session.json` only if it names that session,
/// its usage is an object and every saved error is an `assistant_*` code.
fn check_record(record: &SessionRecord, sid: &str) -> Result<(), String> {
    if record.session.id != sid || !record.usage.is_object() {
        log::warn!("assistant session {sid} does not match its folder or has no usage object");
        return Err(INVALID_SESSION.into());
    }
    if record.messages.iter().any(unknown_code) {
        log::warn!("assistant session {sid} has a message with an unknown error code");
        return Err(INVALID_SESSION.into());
    }
    Ok(())
}

fn unknown_code(message: &Message) -> bool {
    message
        .error_code
        .as_deref()
        .is_some_and(|code| !is_assistant_code(code))
}

fn write_record(path: &Path, record: &SessionRecord) -> Result<(), String> {
    ensure_regular_or_absent(path)?;
    write_bytes(path, &encode_record(record)?)
}

/// `record` as `session.json` holds it.
fn encode_record(record: &SessionRecord) -> Result<Vec<u8>, String> {
    let file = SessionFileOut {
        version: FORMAT_VERSION,
        record,
    };
    serde_json::to_vec(&file).map_err(|_| UNAVAILABLE.to_string())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let opts = crate::config::AtomicOpts { mode: Some(0o600) };
    crate::config::atomic_write_with(path, bytes, opts).map_err(|error| {
        log::warn!("assistant store write failed: {error}");
        UNAVAILABLE.to_string()
    })
}

/// The process-wide write lock of one store file. A panic while it was held
/// left the file whole (writes are atomic), so a poisoned lock is still used.
fn file_lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
        LazyLock::new(Default::default);
    LOCKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(path.to_path_buf())
        .or_default()
        .clone()
}

/// `Ok(true)` for a real directory, `Ok(false)` when absent. A link or a file
/// where a store folder belongs is refused rather than followed.
fn real_dir(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => Err(INVALID_PATH.into()),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(_) => Err(UNAVAILABLE.into()),
    }
}

/// `parent/name` as a private (0o700 on Unix) real directory, created if needed.
fn private_subdir(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let path = parent.join(name);
    if !real_dir(&path)? {
        crate::config::create_private_dir_all(&path).map_err(|_| UNAVAILABLE)?;
        // Re-check: a link planted between the check and the creation is refused.
        if !real_dir(&path)? {
            return Err(UNAVAILABLE.into());
        }
    }
    Ok(path)
}

fn ensure_regular_or_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => Err(INVALID_PATH.into()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(UNAVAILABLE.into()),
    }
}

/// The bytes of a regular file, `None` when absent; links are refused.
fn read_regular(path: &Path) -> Result<Option<Vec<u8>>, String> {
    ensure_regular_or_absent(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(UNAVAILABLE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::KeyOwner;
    use std::fs;

    fn key_owner(user: &str, org: &str) -> KeyOwner {
        KeyOwner {
            user_id: Some(user.into()),
            org_id: Some(org.into()),
        }
    }

    /// A store for `user`/`org` under a private temp config dir.
    fn store_for(config: &Path, user: &str, org: &str) -> AssistantStore {
        let owner = owner_key(&key_owner(user, org)).unwrap();
        AssistantStore::for_owner(config, &owner).unwrap()
    }

    fn fixture() -> (tempfile::TempDir, AssistantStore) {
        let config = tempfile::tempdir().unwrap();
        let store = store_for(config.path(), "user", "org");
        (config, store)
    }

    fn session_file(config: &Path, store: &AssistantStore, sid: &str) -> PathBuf {
        config
            .join("assistant")
            .join(&store.owner)
            .join(sid)
            .join("session.json")
    }

    fn message(content: &str) -> Message {
        serde_json::from_value(json!({
            "id": id(), "role": "user", "content": content, "createdAt": now(),
            "reasoning": null, "status": "completed", "revisionOf": null, "nativeEntryId": null
        }))
        .unwrap()
    }

    fn attachment(status: &str) -> Attachment {
        serde_json::from_value(json!({
            "id": id(), "name": "a.txt", "mimeType": "text/plain", "sizeBytes": 1, "status": status
        }))
        .unwrap()
    }

    fn no_precondition() -> Result<(), String> {
        Ok(())
    }

    fn request(sid: &str) -> UpdateRequest {
        serde_json::from_value(json!({ "sessionId": sid })).unwrap()
    }

    fn send(sid: &str, edit: Option<&str>, regenerate: Option<&str>) -> SendRequest {
        SendRequest {
            session_id: sid.into(),
            run_id: id(),
            operation_id: id(),
            text: "New text".into(),
            attachment_ids: vec![],
            replace_attachments: false,
            edit_message_id: edit.map(str::to_owned),
            regenerate_message_id: regenerate.map(str::to_owned),
        }
    }

    #[test]
    fn removing_draft_attachments_deletes_only_unreferenced_copies_and_frees_quota() {
        let (_config, store) = fixture();
        let sid = store.create("model", false).unwrap().session.id;
        let unused = attachment("ready");
        let kept = attachment("ready");
        let dir = store.attachment_dir(&sid).unwrap();
        for item in [&unused, &kept] {
            fs::write(dir.join(&item.id), b"x").unwrap();
        }
        store
            .mutate(&sid, |record| {
                record.attachments = vec![unused.clone(), kept.clone()];
                record.session.draft_attachment_ids = vec![unused.id.clone(), kept.id.clone()];
                let mut original = message("Previous version");
                original.superseded = true;
                original.attachments = vec![kept.clone()];
                record.messages.push(original);
                Ok(())
            })
            .unwrap();
        let mut remove = request(&sid);
        remove.draft_attachment_ids = Some(vec![]);
        store.update(&remove, no_precondition).unwrap();
        let saved = store.load_record(&sid).unwrap();
        assert_eq!(saved.attachments.len(), 1);
        assert_eq!(saved.attachments[0].id, kept.id);
        assert!(!dir.join(&unused.id).exists());
        assert_eq!(fs::read(dir.join(&kept.id)).unwrap(), b"x");
        assert_eq!(saved.messages[0].attachments[0].id, kept.id);
    }

    #[test]
    fn removing_draft_attachments_cannot_race_a_running_reader() {
        let (_config, store) = fixture();
        let sid = store.create("model", false).unwrap().session.id;
        let file = attachment("ready");
        store
            .mutate(&sid, |record| {
                record.attachments.push(file.clone());
                record.session.draft_attachment_ids.push(file.id.clone());
                record.session.active_run = Some(RunRef {
                    run_id: id(),
                    operation_id: id(),
                });
                Ok(())
            })
            .unwrap();
        let mut remove = request(&sid);
        remove.draft_attachment_ids = Some(vec![]);
        assert_eq!(
            store.update(&remove, no_precondition).unwrap_err(),
            "assistant_session_busy"
        );
        assert_eq!(
            store
                .load_record(&sid)
                .unwrap()
                .session
                .draft_attachment_ids,
            [file.id]
        );
    }

    #[test]
    fn cleanup_reclaims_legacy_unreferenced_metadata_and_crash_orphans() {
        let (_config, store) = fixture();
        let sid = store.create("model", false).unwrap().session.id;
        let lost = attachment("ready");
        let kept = attachment("ready");
        let orphan = id();
        let dir = store.attachment_dir(&sid).unwrap();
        for id in [&lost.id, &kept.id, &orphan] {
            fs::write(dir.join(id), b"x").unwrap();
        }
        fs::write(dir.join("unrelated.txt"), b"keep").unwrap();
        store
            .mutate(&sid, |record| {
                record.attachments = vec![lost.clone(), kept.clone()];
                record.session.draft_attachment_ids = vec![kept.id.clone()];
                Ok(())
            })
            .unwrap();
        assert_eq!(store.cleanup_orphan_attachments(&sid).unwrap(), 2);
        assert_eq!(store.load_record(&sid).unwrap().attachments.len(), 1);
        assert!(dir.join(&kept.id).exists());
        assert!(dir.join("unrelated.txt").exists());
        assert!(!dir.join(&lost.id).exists());
        assert!(!dir.join(orphan).exists());
    }

    #[test]
    fn failed_metadata_save_never_deletes_the_old_draft_file() {
        let (config, store) = fixture();
        let sid = store.create("model", false).unwrap().session.id;
        let item = attachment("ready");
        let copy = store.attachment_dir(&sid).unwrap().join(&item.id);
        fs::write(&copy, b"original").unwrap();
        store
            .mutate(&sid, |record| {
                record.attachments.push(item.clone());
                record.session.draft_attachment_ids.push(item.id.clone());
                Ok(())
            })
            .unwrap();
        let file = session_file(config.path(), &store, &sid);
        let original = fs::read(&file).unwrap();
        let mut remove = request(&sid);
        remove.draft_attachment_ids = Some(vec![]);
        // Fault injected after reading the record, before its atomic save.
        assert!(store
            .update(&remove, || {
                fs::remove_file(&file).unwrap();
                fs::create_dir(&file).unwrap();
                Ok(())
            })
            .is_err());
        assert_eq!(fs::read(copy).unwrap(), b"original");
        fs::remove_dir(&file).unwrap();
        fs::write(&file, original).unwrap();
        assert_eq!(
            store
                .load_record(&sid)
                .unwrap()
                .session
                .draft_attachment_ids,
            [item.id]
        );
    }

    #[test]
    fn a_fresh_store_finishes_cleanup_after_metadata_was_committed_before_a_crash() {
        let (config, store) = fixture();
        let sid = store.create("model", false).unwrap().session.id;
        let copy = store.attachment_dir(&sid).unwrap().join(id());
        // The record has already removed references, but unlink never ran.
        fs::write(&copy, b"left behind after commit").unwrap();
        let restarted = store_for(config.path(), "user", "org");
        assert_eq!(restarted.cleanup_orphan_attachments(&sid).unwrap(), 1);
        assert!(!copy.exists());
        assert_eq!(restarted.cleanup_orphan_attachments(&sid).unwrap(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_unlinks_an_orphan_symlink_without_touching_its_target() {
        let (config, store) = fixture();
        let sid = store.create("model", false).unwrap().session.id;
        let outside = config.path().join("keep.txt");
        fs::write(&outside, b"not an attachment").unwrap();
        let link = store.attachment_dir(&sid).unwrap().join(id());
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert_eq!(store.cleanup_orphan_attachments(&sid).unwrap(), 1);
        assert_eq!(fs::read(outside).unwrap(), b"not an attachment");
        assert!(fs::symlink_metadata(link).is_err());
    }

    #[test]
    fn a_committed_update_returns_success_even_when_orphan_cleanup_fails() {
        let (_config, store) = fixture();
        let sid = store.create("model", false).unwrap().session.id;
        let item = attachment("ready");
        store
            .mutate(&sid, |record| {
                record.attachments.push(item.clone());
                record.session.draft_attachment_ids.push(item.id.clone());
                Ok(())
            })
            .unwrap();
        let dir = store.attachment_dir(&sid).unwrap();
        fs::remove_dir(&dir).unwrap();
        fs::write(&dir, b"blocked attachment directory").unwrap();
        let request: UpdateRequest = serde_json::from_value(json!({
            "sessionId": sid, "webSearch": true, "modelId": "another-model",
            "draftAttachmentIds": [],
        }))
        .unwrap();
        let session = store.update(&request, || Ok(())).unwrap();
        assert!(session.web_search);
        assert_eq!(session.model_id, "another-model");
        assert_eq!(
            serde_json::to_value(session).unwrap()["cleanupWarning"],
            "assistant_attachment_cleanup_failed"
        );
        let saved = store.load_record(&sid).unwrap();
        assert!(saved.session.web_search);
        assert_eq!(saved.session.model_id, "another-model");
        assert!(serde_json::to_value(saved).unwrap()["session"]
            .get("cleanupWarning")
            .is_none());
    }

    #[test]
    fn only_an_update_that_drops_draft_files_cleans_the_attachments_folder() {
        let (_config, store) = fixture();
        let sid = store.create("model", false).unwrap().session.id;
        let item = attachment("ready");
        let dir = store.attachment_dir(&sid).unwrap();
        fs::write(dir.join(&item.id), b"x").unwrap();
        store
            .mutate(&sid, |record| {
                record.attachments.push(item.clone());
                record.session.draft_attachment_ids.push(item.id.clone());
                Ok(())
            })
            .unwrap();
        let orphan = dir.join(id());
        fs::write(&orphan, b"left by a crash").unwrap();
        let mut rename = request(&sid);
        rename.title = Some("Renamed".into());
        store.update(&rename, no_precondition).unwrap();
        assert!(orphan.exists(), "a title change leaves the folder alone");
        let mut remove = request(&sid);
        remove.draft_attachment_ids = Some(vec![]);
        store.update(&remove, no_precondition).unwrap();
        assert!(!orphan.exists());
        assert!(!dir.join(&item.id).exists());
    }

    #[test]
    fn an_error_that_is_not_an_assistant_code_is_dropped_on_load_and_never_saved() {
        let (config, store) = fixture();
        let sid = store.create("model", false).unwrap().session.id;
        let mut failed = message("Failed");
        failed.status = "error".into();
        failed.error_code = Some("assistant_network_error".into());
        store
            .mutate(&sid, |record| {
                record.messages.push(failed.clone());
                Ok(())
            })
            .unwrap();
        // Hand-edited: provider text where only a stable code belongs.
        let file = session_file(config.path(), &store, &sid);
        let text = fs::read_to_string(&file)
            .unwrap()
            .replace("assistant_network_error", "Upstream said: private detail");
        fs::write(&file, text).unwrap();
        let loaded = store.load_record(&sid).unwrap();
        assert!(loaded.messages[0].error_code.is_none());
        assert_eq!(loaded.messages[0].status, "error");
        assert_eq!(
            store
                .mutate(&sid, |record| {
                    record.messages[0].error_code = Some("ECONNRESET".into());
                    Ok(())
                })
                .unwrap_err(),
            INVALID_SESSION
        );
        assert!(fs::read_to_string(&file)
            .unwrap()
            .contains("Upstream said: private detail"));
    }

    #[test]
    fn cleanup_waits_for_an_import_to_publish_its_draft_reference() {
        let (_config, store) = fixture();
        let sid = store.create("model", false).unwrap().session.id;
        let item = attachment("ready");
        let copy = store.attachment_dir(&sid).unwrap().join(&item.id);
        let (copied_tx, copied_rx) = std::sync::mpsc::channel();
        let (commit_tx, commit_rx) = std::sync::mpsc::channel();
        let (cleanup_tx, cleanup_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let store = &store;
            let sid = &sid;
            let copy = &copy;
            scope.spawn(move || {
                store
                    .with_attachment_lock(sid, || {
                        fs::write(copy, b"staged").unwrap();
                        copied_tx.send(()).unwrap();
                        commit_rx.recv().unwrap();
                        store.mutate(sid, |record| {
                            record.attachments.push(item.clone());
                            record.session.draft_attachment_ids.push(item.id.clone());
                            Ok(())
                        })
                    })
                    .unwrap()
            });
            copied_rx.recv().unwrap();
            scope.spawn(move || {
                cleanup_tx
                    .send(store.cleanup_orphan_attachments(sid))
                    .unwrap();
            });
            assert!(cleanup_rx
                .recv_timeout(std::time::Duration::from_millis(30))
                .is_err());
            commit_tx.send(()).unwrap();
        });
        assert_eq!(cleanup_rx.recv().unwrap().unwrap(), 0);
        assert_eq!(fs::read(copy).unwrap(), b"staged");
    }

    // ── owner identity → directory ───────────────────────────────────────

    #[test]
    fn owner_key_is_stable_per_account_and_organization() {
        let a = owner_key(&key_owner("alice", "one")).unwrap();
        assert_eq!(a, owner_key(&key_owner("alice", "one")).unwrap());
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, owner_key(&key_owner("alice", "two")).unwrap());
        assert_ne!(a, owner_key(&key_owner("bob", "one")).unwrap());
        // The separator keeps ("ab", "c") and ("a", "bc") apart.
        assert_ne!(
            owner_key(&key_owner("ab", "c")).unwrap(),
            owner_key(&key_owner("a", "bc")).unwrap()
        );
    }

    #[test]
    fn an_account_without_user_or_organization_has_no_owner_key() {
        for owner in [
            KeyOwner::default(),
            KeyOwner {
                user_id: Some("alice".into()),
                org_id: None,
            },
            key_owner("", "org"),
        ] {
            assert_eq!(
                owner_key(&owner).unwrap_err(),
                "assistant_identity_unavailable"
            );
        }
    }

    #[test]
    fn owner_dir_is_under_the_assistant_folder_and_rejects_foreign_names() {
        let config = Path::new("/config");
        let owner = owner_key(&key_owner("alice", "one")).unwrap();
        assert_eq!(
            owner_dir(config, &owner).unwrap(),
            config.join("assistant").join(&owner)
        );
        for bad in [
            "",
            "..",
            "../../etc",
            &owner[..63],
            &format!("{}x", &owner[..63]),
            &owner.to_uppercase(),
        ] {
            assert_eq!(
                owner_dir(config, bad).unwrap_err(),
                "assistant_invalid_storage_path"
            );
        }
    }

    // ── session files ────────────────────────────────────────────────────

    #[test]
    fn a_created_session_round_trips_through_its_own_file() {
        let (config, store) = fixture();
        let created = store.create("test/model", true).unwrap();
        let sid = created.session.id.clone();
        let loaded = store.load_detail(&sid).unwrap();
        assert_eq!(
            serde_json::to_value(&loaded).unwrap(),
            serde_json::to_value(&created).unwrap()
        );
        assert_eq!(loaded.session.model_id, "test/model");
        assert!(loaded.session.web_search);
        assert!(loaded.messages.is_empty());
        assert_eq!(loaded.usage, json!({}));

        let saved: Value =
            serde_json::from_slice(&fs::read(session_file(config.path(), &store, &sid)).unwrap())
                .unwrap();
        let mut keys: Vec<_> = saved.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "attachments",
                "messages",
                "operations",
                "session",
                "usage",
                "version"
            ]
        );
        assert_eq!(saved["version"], 1);
        assert_eq!(saved["session"]["id"], sid.as_str());
    }

    #[test]
    fn the_run_ledger_is_saved_but_never_part_of_the_detail() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let operation = Operation {
            id: id(),
            run_id: id(),
            status: OperationStatus::Running,
            search_price: None,
        };
        store
            .mutate(&sid, |record| {
                record.operations.push(operation.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            store.load_record(&sid).unwrap().operations,
            std::slice::from_ref(&operation)
        );
        let detail = serde_json::to_string(&store.load_detail(&sid).unwrap()).unwrap();
        assert!(!detail.contains("operations"));
        assert!(!detail.contains(&operation.id));
        let saved = fs::read_to_string(session_file(config.path(), &store, &sid)).unwrap();
        assert!(saved.contains(&operation.id));
    }

    #[test]
    fn a_failed_change_leaves_the_file_untouched() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let path = session_file(config.path(), &store, &sid);
        let before = fs::read(&path).unwrap();
        let result: Result<(), String> = store.mutate(&sid, |record| {
            record.messages.push(message("half-applied"));
            Err("assistant_test_refused".into())
        });
        assert_eq!(result.unwrap_err(), "assistant_test_refused");
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn a_change_that_could_not_be_read_back_is_not_saved() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let path = session_file(config.path(), &store, &sid);
        let before = fs::read(&path).unwrap();
        let moved = store.mutate(&sid, |record| {
            record.session.id = id();
            Ok(())
        });
        assert_eq!(moved.unwrap_err(), "assistant_invalid_session");
        let unusable = store.mutate(&sid, |record| {
            record.usage = Value::Null;
            Ok(())
        });
        assert_eq!(unusable.unwrap_err(), "assistant_invalid_session");
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(store.load_detail(&sid).is_ok());
    }

    #[test]
    fn writes_replace_the_file_atomically_and_leave_no_temp_files() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        for n in 0..5 {
            store
                .mutate(&sid, |record| {
                    record.messages.push(message(&format!("message {n}")));
                    Ok(())
                })
                .unwrap();
        }
        let path = session_file(config.path(), &store, &sid);
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["session.json"]);
        assert_eq!(store.load_detail(&sid).unwrap().messages.len(), 5);
        #[cfg(unix)]
        {
            assert_eq!(crate::config::file_mode(&path), Some(0o600));
            assert_eq!(
                crate::config::file_mode(path.parent().unwrap()),
                Some(0o700)
            );
            assert_eq!(
                crate::config::file_mode(&config.path().join("assistant")),
                Some(0o700)
            );
        }
    }

    #[test]
    fn concurrent_writers_to_one_session_never_lose_an_update() {
        let (_config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let store = std::sync::Arc::new(store);
        let writers: Vec<_> = (0..8)
            .map(|n| {
                let store = store.clone();
                let sid = sid.clone();
                std::thread::spawn(move || {
                    for m in 0..5 {
                        store
                            .mutate(&sid, |record| {
                                record.messages.push(message(&format!("{n}-{m}")));
                                Ok(())
                            })
                            .unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        assert_eq!(store.load_detail(&sid).unwrap().messages.len(), 40);
    }

    #[test]
    fn a_session_id_that_is_not_a_uuid_never_reaches_the_file_system() {
        let (_config, store) = fixture();
        for bad in ["../escape", "", "prefs.json"] {
            assert_eq!(
                store.load_detail(bad).unwrap_err(),
                "assistant_invalid_identifier"
            );
            assert_eq!(
                store.mutate(bad, |_| Ok(())).unwrap_err(),
                "assistant_invalid_identifier"
            );
        }
        assert_eq!(
            store.load_detail(&id()).unwrap_err(),
            "assistant_session_not_found"
        );
    }

    // ── damaged files ────────────────────────────────────────────────────

    #[test]
    fn a_damaged_session_file_is_an_error_and_is_never_rewritten() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let path = session_file(config.path(), &store, &sid);
        for damaged in [
            b"damaged".to_vec(),
            b"{\"version\":1,\"session\":".to_vec(),
            // A file without its messages is not an empty conversation.
            serde_json::to_vec(&json!({"version": 1, "session": store.load_detail(&sid).unwrap().session, "attachments": [], "usage": {}, "operations": []})).unwrap(),
        ] {
            fs::write(&path, &damaged).unwrap();
            assert_eq!(
                store.load_detail(&sid).unwrap_err(),
                "assistant_invalid_session"
            );
            assert!(store
                .mutate(&sid, |record| {
                    record.messages.clear();
                    Ok(())
                })
                .is_err());
            assert!(store.update(&request(&sid), no_precondition).is_err());
            assert_eq!(fs::read(&path).unwrap(), damaged);
        }
    }

    #[test]
    fn a_session_file_from_another_version_or_session_is_refused() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let path = session_file(config.path(), &store, &sid);
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();

        let mut newer = saved.clone();
        newer["version"] = json!(2);
        fs::write(&path, serde_json::to_vec(&newer).unwrap()).unwrap();
        assert_eq!(
            store.load_detail(&sid).unwrap_err(),
            "assistant_invalid_session"
        );

        let mut moved = saved.clone();
        moved["session"]["id"] = json!(id());
        fs::write(&path, serde_json::to_vec(&moved).unwrap()).unwrap();
        assert_eq!(
            store.load_detail(&sid).unwrap_err(),
            "assistant_invalid_session"
        );

        let mut usage = saved;
        usage["usage"] = json!(null);
        fs::write(&path, serde_json::to_vec(&usage).unwrap()).unwrap();
        assert_eq!(
            store.load_detail(&sid).unwrap_err(),
            "assistant_invalid_session"
        );
    }

    #[test]
    fn the_list_reports_a_damaged_session_and_still_shows_the_others() {
        let (config, store) = fixture();
        let good = store.create("test/model", false).unwrap().session.id;
        let bad = store.create("test/model", false).unwrap().session.id;
        fs::write(session_file(config.path(), &store, &bad), "damaged").unwrap();
        let list = store.list(false, "").unwrap();
        assert_eq!(
            list.sessions.iter().map(|s| &s.id).collect::<Vec<_>>(),
            [&good]
        );
        assert_eq!(list.unreadable.len(), 1);
        assert_eq!(list.unreadable[0].id, bad);
        assert_eq!(list.unreadable[0].error, "assistant_invalid_session");
        // Reported in either view: whether it was in the trash is unknown.
        assert_eq!(store.list(true, "anything").unwrap().unreadable.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn linked_session_files_and_folders_are_refused() {
        use std::os::unix::fs::symlink;
        let (config, store) = fixture();
        let outside = config.path().join("outside");
        fs::create_dir_all(&outside).unwrap();

        let sid = store.create("test/model", false).unwrap().session.id;
        let path = session_file(config.path(), &store, &sid);
        fs::rename(&path, outside.join("session.json")).unwrap();
        symlink(outside.join("session.json"), &path).unwrap();
        assert_eq!(
            store.load_detail(&sid).unwrap_err(),
            "assistant_invalid_storage_path"
        );
        assert!(store.mutate(&sid, |_| Ok(())).is_err());

        let linked = id();
        symlink(
            &outside,
            path.parent().unwrap().parent().unwrap().join(&linked),
        )
        .unwrap();
        assert_eq!(
            store.load_detail(&linked).unwrap_err(),
            "assistant_invalid_storage_path"
        );
        let list = store.list(false, "").unwrap();
        let mut unreadable: Vec<_> = list.unreadable.iter().map(|u| u.id.clone()).collect();
        unreadable.sort();
        let mut expected = vec![sid.clone(), linked];
        expected.sort();
        assert_eq!(unreadable, expected);

        let history = store.native_history_path(&sid);
        assert!(history.is_ok(), "the folder itself is still real");
        symlink(outside.join("x"), path.with_file_name("session.jsonl")).unwrap();
        assert_eq!(
            store.native_history_path(&sid).unwrap_err(),
            "assistant_invalid_storage_path"
        );
    }

    // ── owners ───────────────────────────────────────────────────────────

    #[test]
    fn two_owners_never_see_each_others_sessions_or_preferences() {
        let config = tempfile::tempdir().unwrap();
        let one = store_for(config.path(), "alice", "one");
        let two = store_for(config.path(), "alice", "two");
        let sid = one.create("test/model", false).unwrap().session.id;
        one.remember_model("one/model").unwrap();

        assert_eq!(
            two.load_detail(&sid).unwrap_err(),
            "assistant_session_not_found"
        );
        assert!(two.mutate(&sid, |_| Ok(())).is_err());
        assert!(two.update(&request(&sid), no_precondition).is_err());
        assert!(two.native_history_path(&sid).is_err());
        assert!(two.list(false, "").unwrap().sessions.is_empty());
        assert_eq!(two.remembered_model().unwrap(), None);
        assert_eq!(two.usage_summary().unwrap()["sessionCount"], 0);
        assert_eq!(one.list(false, "").unwrap().sessions.len(), 1);
    }

    // ── list ─────────────────────────────────────────────────────────────

    #[test]
    fn the_list_separates_the_trash_and_puts_recent_sessions_first() {
        let (_config, store) = fixture();
        let mut ids = vec![];
        for title in ["old", "middle", "new"] {
            let sid = store.create("test/model", false).unwrap().session.id;
            let mut update = request(&sid);
            update.title = Some(title.into());
            store.update(&update, no_precondition).unwrap();
            ids.push(sid);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut trash = request(&ids[1]);
        trash.deleted = Some(true);
        store.update(&trash, no_precondition).unwrap();

        let titles = |deleted| -> Vec<String> {
            store
                .list(deleted, "")
                .unwrap()
                .sessions
                .into_iter()
                .map(|s| s.title)
                .collect()
        };
        assert_eq!(titles(false), ["new", "old"]);
        assert_eq!(titles(true), ["middle"]);

        let mut restore = request(&ids[1]);
        restore.deleted = Some(false);
        store.update(&restore, no_precondition).unwrap();
        assert_eq!(titles(false), ["middle", "new", "old"]);
    }

    #[test]
    fn search_matches_titles_and_message_text_ignoring_case() {
        let (_config, store) = fixture();
        let titled = store.create("test/model", false).unwrap().session.id;
        let mut update = request(&titled);
        update.title = Some("Trip to Kyoto".into());
        store.update(&update, no_precondition).unwrap();
        let talked = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&talked, |record| {
                record.messages.push(message("What is a Rust lifetime?"));
                Ok(())
            })
            .unwrap();
        store.create("test/model", false).unwrap();

        let found = |search: &str| -> Vec<String> {
            store
                .list(false, search)
                .unwrap()
                .sessions
                .into_iter()
                .map(|s| s.id)
                .collect()
        };
        assert_eq!(found("kyoto"), [titled]);
        assert_eq!(found("RUST LIFE"), [talked]);
        assert!(found("nowhere").is_empty());
        assert_eq!(found("").len(), 3);
    }

    #[test]
    fn the_trash_count_covers_every_trashed_conversation_whatever_the_list_shows() {
        let (_config, store) = fixture();
        assert_eq!(store.list(false, "").unwrap().trash_count, 0);
        for title in ["kept", "first gone", "second gone"] {
            let sid = store.create("test/model", false).unwrap().session.id;
            let mut update = request(&sid);
            update.title = Some(title.into());
            update.deleted = Some(title != "kept");
            store.update(&update, no_precondition).unwrap();
        }

        for (deleted, search) in [(false, ""), (true, ""), (false, "kept"), (true, "nowhere")] {
            assert_eq!(store.list(deleted, search).unwrap().trash_count, 2);
        }
        // The renderer reads it as `trashCount`.
        let listed = serde_json::to_value(store.list(false, "").unwrap()).unwrap();
        assert_eq!(listed["trashCount"], 2);
    }

    #[test]
    fn equal_update_times_are_listed_in_id_order() {
        let (_config, store) = fixture();
        let mut ids = vec![];
        for _ in 0..6 {
            let sid = store.create("test/model", false).unwrap().session.id;
            store
                .mutate(&sid, |record| {
                    record.session.updated_at = 1;
                    Ok(())
                })
                .unwrap();
            ids.push(sid);
        }
        ids.sort();
        let listed: Vec<_> = store
            .list(false, "")
            .unwrap()
            .sessions
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(listed, ids);
    }

    #[test]
    fn a_session_folder_with_history_but_no_session_file_is_reported() {
        let (config, store) = fixture();
        let kept = store.create("test/model", false).unwrap().session.id;
        let root = session_file(config.path(), &store, &kept)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let orphan = id();
        fs::create_dir(root.join(&orphan)).unwrap();
        fs::write(root.join(&orphan).join("session.jsonl"), "{}\n").unwrap();
        // An empty folder is a creation that never wrote its file: nothing to show.
        fs::create_dir(root.join(id())).unwrap();

        let list = store.list(false, "").unwrap();
        assert_eq!(
            list.sessions.iter().map(|s| &s.id).collect::<Vec<_>>(),
            [&kept]
        );
        assert_eq!(list.unreadable.len(), 1);
        assert_eq!(list.unreadable[0].id, orphan);
        assert_eq!(list.unreadable[0].error, "assistant_invalid_session");
        assert_eq!(
            store.usage_summary().unwrap()["usageCoverage"],
            "incomplete"
        );
    }

    /// What `config::atomic_write_with` leaves next to `session.json` while it
    /// writes, or after a crash mid-write.
    fn unfinished_write(dir: &Path) -> PathBuf {
        dir.join(format!(".session.json.ofox-tmp-{}-1", std::process::id()))
    }

    fn record_for(sid: &str) -> SessionRecord {
        SessionRecord {
            session: Session {
                id: sid.into(),
                title: String::new(),
                model_id: "test/model".into(),
                web_search: false,
                created_at: 1,
                updated_at: 1,
                deleted_at: None,
                active_run: None,
                draft: String::new(),
                draft_attachment_ids: vec![],
                cleanup_warning: None,
            },
            messages: vec![],
            attachments: vec![],
            usage: json!({}),
            operations: vec![],
        }
    }

    #[test]
    fn a_folder_holding_only_an_unfinished_write_is_not_reported() {
        let (config, store) = fixture();
        let kept = store.create("test/model", false).unwrap().session.id;
        let root = session_file(config.path(), &store, &kept)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let interrupted = root.join(id());
        fs::create_dir(&interrupted).unwrap();
        fs::write(unfinished_write(&interrupted), "{").unwrap();

        let list = store.list(false, "").unwrap();
        assert_eq!(
            list.sessions.iter().map(|s| &s.id).collect::<Vec<_>>(),
            [&kept]
        );
        assert!(list.unreadable.is_empty(), "{:?}", list.unreadable);
    }

    /// `create` holds the file lock from its temp file to the rename. A listing
    /// that finds the folder in between waits for it instead of calling the
    /// new conversation unreadable.
    #[test]
    fn a_listing_during_a_creation_waits_for_its_session_file() {
        let (config, store) = fixture();
        let kept = store.create("test/model", false).unwrap().session.id;
        let root = session_file(config.path(), &store, &kept)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let sid = id();
        let path = root.join(&sid).join(SESSION_FILE);
        let lock = file_lock(&path);
        let guard = lock.lock().unwrap();
        fs::create_dir(root.join(&sid)).unwrap();
        fs::write(unfinished_write(&root.join(&sid)), "{").unwrap();

        let listing = std::thread::spawn({
            let config = config.path().to_path_buf();
            move || store_for(&config, "user", "org").list(false, "").unwrap()
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        write_record(&path, &record_for(&sid)).unwrap();
        fs::remove_file(unfinished_write(&root.join(&sid))).unwrap();
        drop(guard);

        let list = listing.join().unwrap();
        assert!(list.unreadable.is_empty(), "{:?}", list.unreadable);
        let mut ids: Vec<_> = list.sessions.iter().map(|s| s.id.clone()).collect();
        ids.sort();
        let mut expected = vec![kept, sid];
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn a_missing_store_lists_nothing_and_creates_nothing() {
        let (config, store) = fixture();
        let list = store.list(false, "").unwrap();
        assert!(list.sessions.is_empty() && list.unreadable.is_empty());
        assert_eq!(store.remembered_model().unwrap(), None);
        assert!(!config.path().join("assistant").exists());
    }

    // ── update ───────────────────────────────────────────────────────────

    #[test]
    fn an_update_whose_precondition_fails_changes_nothing() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let path = session_file(config.path(), &store, &sid);
        let before = fs::read(&path).unwrap();
        let mut change = request(&sid);
        change.title = Some("Renamed".into());
        change.deleted = Some(true);
        let mut checked = false;
        let result = store.update(&change, || {
            checked = true;
            Err("assistant_identity_changed".into())
        });
        assert_eq!(result.unwrap_err(), "assistant_identity_changed");
        assert!(checked);
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn update_applies_only_the_fields_it_carries() {
        let (_config, store) = fixture();
        let created = store.create("test/model", false).unwrap().session;
        let mut change = request(&created.id);
        change.title = Some("Renamed".into());
        change.model_id = Some("other/model".into());
        change.web_search = Some(true);
        change.draft = Some("unsent".into());
        let session = store.update(&change, no_precondition).unwrap();
        assert_eq!(session.title, "Renamed");
        assert_eq!(session.model_id, "other/model");
        assert!(session.web_search);
        assert_eq!(session.draft, "unsent");
        assert!(session.deleted_at.is_none());
        assert!(session.updated_at >= created.updated_at);

        let untouched = store
            .update(&request(&created.id), no_precondition)
            .unwrap();
        assert_eq!(untouched.title, "Renamed");
        assert_eq!(untouched.model_id, "other/model");
        assert_eq!(
            store.load_detail(&created.id).unwrap().session.draft,
            "unsent"
        );
    }

    #[test]
    fn update_keeps_title_and_draft_byte_for_byte() {
        let (_config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut change = request(&sid);
        change.title = Some("max_tokens: 4096 password: hunter2".into());
        change.draft = Some("OpenAI(api_key=os.environ[\"OPENAI_API_KEY\"])".into());
        let session = store.update(&change, no_precondition).unwrap();
        assert_eq!(session.title, "max_tokens: 4096 password: hunter2");
        assert_eq!(
            store.load_detail(&sid).unwrap().session.draft,
            "OpenAI(api_key=os.environ[\"OPENAI_API_KEY\"])"
        );
    }

    #[test]
    fn update_refuses_oversized_text_and_unknown_draft_attachments() {
        let (_config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let ready = attachment("ready");
        let pending = attachment("processing");
        // Read in part by an earlier run: still a file the user can send.
        let partial = attachment("partial");
        store
            .mutate(&sid, |record| {
                record.attachments = vec![ready.clone(), pending.clone(), partial.clone()];
                Ok(())
            })
            .unwrap();

        let mut long_title = request(&sid);
        long_title.title = Some("t".repeat(513));
        assert_eq!(
            store.update(&long_title, no_precondition).unwrap_err(),
            "assistant_title_too_long"
        );
        let mut long_draft = request(&sid);
        long_draft.draft = Some("d".repeat(128_001));
        assert_eq!(
            store.update(&long_draft, no_precondition).unwrap_err(),
            "assistant_message_too_large"
        );
        for ids in [
            vec![pending.id.clone()],
            vec![id()],
            vec![ready.id.clone(); 201],
        ] {
            let mut drafted = request(&sid);
            drafted.draft_attachment_ids = Some(ids);
            assert_eq!(
                store.update(&drafted, no_precondition).unwrap_err(),
                "assistant_attachment_not_found"
            );
        }
        let mut drafted = request(&sid);
        drafted.draft_attachment_ids = Some(vec![ready.id.clone(), partial.id.clone()]);
        assert_eq!(
            store
                .update(&drafted, no_precondition)
                .unwrap()
                .draft_attachment_ids,
            [ready.id, partial.id]
        );
    }

    // ── run ledger and drafts ────────────────────────────────────────────

    #[test]
    fn an_operation_id_is_accepted_once_and_a_stopped_one_stays_stopped() {
        let (_config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let running = id();
        let stopped = id();
        let mut record = store.load_record(&sid).unwrap();
        assert!(ensure_new_operation(&record, &running).is_ok());
        record.operations.push(Operation {
            id: running.clone(),
            run_id: id(),
            status: OperationStatus::Running,
            search_price: None,
        });
        record.operations.push(Operation {
            id: stopped.clone(),
            run_id: id(),
            status: OperationStatus::Cancelled,
            search_price: None,
        });
        assert_eq!(
            ensure_new_operation(&record, &running).unwrap_err(),
            "assistant_operation_already_submitted"
        );
        assert_eq!(
            ensure_new_operation(&record, &stopped).unwrap_err(),
            "assistant_cancelled"
        );
        assert!(ensure_new_operation(&record, &id()).is_ok());
    }

    #[test]
    fn only_a_new_user_turn_consumes_the_composer_draft() {
        for (edit, regenerate) in [
            (None, None),
            (Some("edit"), None),
            (None, Some("regenerate")),
        ] {
            let (_config, store) = fixture();
            let mut session = store.create("test/model", false).unwrap().session;
            session.draft = "Unsent text".into();
            session.draft_attachment_ids = vec!["unsent-attachment".into()];
            let request = send(&session.id, edit, regenerate);
            consume_draft(&mut session, &request);
            let revision = edit.is_some() || regenerate.is_some();
            assert_eq!(session.draft, if revision { "Unsent text" } else { "" });
            assert_eq!(session.draft_attachment_ids.len(), usize::from(revision));
        }
    }

    #[test]
    fn native_history_lives_next_to_the_session_file() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        assert_eq!(
            store.native_history_path(&sid).unwrap(),
            session_file(config.path(), &store, &sid).with_file_name("session.jsonl")
        );
        assert_eq!(
            store.native_history_path(&id()).unwrap_err(),
            "assistant_session_not_found"
        );
    }

    // ── preferences ──────────────────────────────────────────────────────

    #[test]
    fn attachments_live_in_a_private_folder_of_their_session() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let root = store.attachment_root(&sid).unwrap();
        assert_eq!(
            root,
            session_file(config.path(), &store, &sid).with_file_name("attachments")
        );
        assert!(!root.exists(), "naming the folder creates nothing");
        assert_eq!(store.attachment_dir(&sid).unwrap(), root);
        assert!(root.is_dir());
        #[cfg(unix)]
        assert_eq!(crate::config::file_mode(&root), Some(0o700));

        // Never a folder for a session that does not exist.
        let unknown = id();
        assert_eq!(
            store.attachment_dir(&unknown).unwrap_err(),
            "assistant_session_not_found"
        );
        assert!(!root.parent().unwrap().with_file_name(&unknown).exists());
        assert_eq!(
            store.attachment_root("../x").unwrap_err(),
            "assistant_invalid_identifier"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_attachments_folder_is_refused() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let elsewhere = config.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, store.attachment_root(&sid).unwrap()).unwrap();
        assert_eq!(
            store.attachment_dir(&sid).unwrap_err(),
            "assistant_invalid_storage_path"
        );
    }

    #[test]
    fn the_remembered_model_is_kept_in_prefs_json() {
        let (config, store) = fixture();
        assert_eq!(store.remembered_model().unwrap(), None);
        store.remember_model("first/model").unwrap();
        store.remember_model("second/model").unwrap();
        assert_eq!(
            store.remembered_model().unwrap().as_deref(),
            Some("second/model")
        );
        let path = config
            .path()
            .join("assistant")
            .join(&store.owner)
            .join("prefs.json");
        let saved: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(saved, json!({"version": 1, "modelId": "second/model"}));
    }

    #[test]
    fn damaged_prefs_are_reported_and_the_next_choice_replaces_them() {
        let (config, store) = fixture();
        store.remember_model("first/model").unwrap();
        let path = config
            .path()
            .join("assistant")
            .join(&store.owner)
            .join("prefs.json");
        for damaged in ["damaged", r#"{"version":2,"modelId":"x/y"}"#] {
            fs::write(&path, damaged).unwrap();
            assert_eq!(
                store.remembered_model().unwrap_err(),
                "assistant_invalid_preferences"
            );
        }
        store.remember_model("next/model").unwrap();
        assert_eq!(
            store.remembered_model().unwrap().as_deref(),
            Some("next/model")
        );
    }

    // ── usage ────────────────────────────────────────────────────────────

    #[test]
    fn usage_summary_adds_up_every_session_of_the_owner() {
        let (_config, store) = fixture();
        // A session's cost includes its searches; `searchCostUsd` is that part.
        for (input, cost, searches, search_cost) in [(10, "0.5", 2, "0.028"), (5, "0.25", 0, "0")] {
            let sid = store.create("test/model", false).unwrap().session.id;
            store
                .mutate(&sid, |record| {
                    record.usage = json!({
                        "modelRequests": 1, "inputTokens": input, "outputTokens": 2,
                        "estimatedCostUsd": cost, "searchCount": searches,
                        "searchCostUsd": search_cost,
                    });
                    Ok(())
                })
                .unwrap();
        }
        let summary = store.usage_summary().unwrap();
        assert_eq!(summary["sessionCount"], 2);
        assert_eq!(summary["runCount"], 2);
        assert_eq!(summary["inputTokens"], 15);
        assert_eq!(summary["outputTokens"], 4);
        assert_eq!(summary["estimatedCostUsd"], "0.75");
        assert_eq!(summary["searchCount"], 2);
        assert_eq!(summary["searchCostUsd"], "0.028");
        assert_eq!(summary["usageCoverage"], "complete");
    }

    #[test]
    fn unknown_usage_cannot_become_a_zero_price_in_the_account_summary() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&sid, |record| {
                record.usage = json!({"usageCoverage": "incomplete", "modelRequests": 0});
                Ok(())
            })
            .unwrap();
        assert!(store.usage_summary().unwrap()["estimatedCostUsd"].is_null());

        let (_other, clean) = fixture();
        let sid = clean.create("test/model", false).unwrap().session.id;
        clean
            .mutate(&sid, |record| {
                record.operations.push(Operation {
                    id: id(),
                    run_id: id(),
                    status: OperationStatus::Running,
                    search_price: None,
                });
                Ok(())
            })
            .unwrap();
        assert_eq!(
            clean.usage_summary().unwrap()["usageCoverage"],
            "incomplete"
        );

        let (damaged_config, damaged) = fixture();
        let sid = damaged.create("test/model", false).unwrap().session.id;
        fs::write(
            session_file(damaged_config.path(), &damaged, &sid),
            "damaged",
        )
        .unwrap();
        let summary = damaged.usage_summary().unwrap();
        assert_eq!(summary["usageCoverage"], "incomplete");
        assert!(summary["estimatedCostUsd"].is_null());
        drop(config);
    }
}
