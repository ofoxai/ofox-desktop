//! File store for assistant conversations, one folder per signed-in owner.
//!
//! This module is the only place that knows the layout:
//!
//! ```text
//! <app_config_dir>/assistant/<owner>/prefs.json                 {"version":1,"modelId":…,"instructions":…,"autoTitle":…,"notifyOnFinish":…}
//! <app_config_dir>/assistant/<owner>/<session_id>/session.json   SessionRecord + "version"
//! <app_config_dir>/assistant/<owner>/<session_id>/session.jsonl  native runtime history
//! <app_config_dir>/assistant/<owner>/<session_id>/attachments/<attachment_id>  imported files
//! ```
//!
//! Every write replaces the whole file atomically (temp file + rename), and
//! writers to one file are serialized in this process. A damaged file is an
//! error for that session; it is never read as, or overwritten by, an empty one.
mod backup;
mod fork;
mod lifecycle;
pub use backup::{BackupReport, RestoreReport, TRANSFER_BUSY};
pub use fork::InheritedUsage;

use super::thinking;
use super::types::*;
use crate::settings::KeyOwner;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
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
const INSTRUCTIONS_TOO_LONG: &str = "assistant_instructions_too_long";
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inherited_usage: Vec<InheritedUsage>,
}

impl SessionRecord {
    pub fn into_detail(self) -> Detail {
        Detail {
            audio_operations: super::audio::results(&self.operations),
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

/// The fields of `prefs.json` this version knows. A settings field the user
/// never saved is absent (`None`), so a restore can tell it from a choice.
/// Fields are only ever set, never cleared: empty instructions are `""`.
#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Prefs {
    version: u32,
    model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    video_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    speech_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transcription_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_title: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notify_on_finish: Option<bool>,
}

impl Prefs {
    /// A preferences file this version can read: its known fields and the
    /// whole object, unknown fields included. `None` for a damaged file, one
    /// of another version or with instructions over their limit.
    fn parse(bytes: &[u8]) -> Option<(Self, Map<String, Value>)> {
        let object: Map<String, Value> = serde_json::from_slice(bytes).ok()?;
        let prefs: Self = serde_json::from_value(Value::Object(object.clone())).ok()?;
        let fits = prefs
            .instructions
            .as_deref()
            .is_none_or(|text| text.chars().count() <= MAX_INSTRUCTIONS_CHARS);
        (prefs.version == FORMAT_VERSION && fits).then_some((prefs, object))
    }

    /// The file's bytes: `object` (what it held, unknown fields included)
    /// with the known fields replaced by these.
    fn to_bytes(&self, mut object: Map<String, Value>) -> Result<Vec<u8>, String> {
        let Value::Object(known) = serde_json::to_value(self).map_err(|_| UNAVAILABLE)? else {
            return Err(UNAVAILABLE.into());
        };
        object.extend(known);
        serde_json::to_vec(&object).map_err(|_| UNAVAILABLE.into())
    }

    /// Take each field of `other` that this one does not have; true when
    /// one was taken.
    fn fill_from(&mut self, other: &Prefs) -> bool {
        fn fill<T: Clone>(mine: &mut Option<T>, theirs: &Option<T>) -> bool {
            let taken = mine.is_none() && theirs.is_some();
            if taken {
                mine.clone_from(theirs);
            }
            taken
        }
        // `|`, not `||`: every field is filled.
        fill(&mut self.model_id, &other.model_id)
            | fill(&mut self.image_model_id, &other.image_model_id)
            | fill(&mut self.video_model_id, &other.video_model_id)
            | fill(&mut self.speech_model_id, &other.speech_model_id)
            | fill(
                &mut self.transcription_model_id,
                &other.transcription_model_id,
            )
            | fill(&mut self.instructions, &other.instructions)
            | fill(&mut self.auto_title, &other.auto_title)
            | fill(&mut self.notify_on_finish, &other.notify_on_finish)
    }

    fn resolved(&self) -> Preferences {
        let defaults = Preferences::default();
        Preferences {
            instructions: self.instructions.clone().unwrap_or(defaults.instructions),
            auto_title: self.auto_title.unwrap_or(defaults.auto_title),
            notify_on_finish: self.notify_on_finish.unwrap_or(defaults.notify_on_finish),
        }
    }
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
    /// For a search: the first active message of each listed conversation
    /// that matches it (session id to message id). A conversation found by
    /// its title alone, or only through a replaced message, has no entry.
    pub matches: HashMap<String, String>,
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
#[derive(Clone)]
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
                audio_only: None,
                context_usage: None,
                compacting: None,
                id: id(),
                title: String::new(),
                title_source: Some(TitleSource::Auto),
                model_id: model_id.into(),
                web_search,
                thinking_level: None,
                created_at: at,
                updated_at: at,
                deleted_at: None,
                pinned_at: None,
                active_run: None,
                draft: String::new(),
                draft_attachment_ids: vec![],
                cleanup_warning: None,
            },
            messages: vec![],
            attachments: vec![],
            usage: json!({}),
            operations: vec![],
            inherited_usage: vec![],
        };
        let directory = private_subdir(&self.ensure_root()?, &record.session.id)?;
        let path = directory.join(SESSION_FILE);
        let lock = file_lock(&path);
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        write_record(&path, &record)?;
        Ok(record.into_detail())
    }

    /// Sessions in the trash (`deleted`) or out of it. Out of it, pinned ones
    /// come first (the latest pin on top), then the rest; the trash and the
    /// rest go most recently updated first (ties by id). `search` matches the
    /// title or any message, ignoring case, and `matches` names the first
    /// active message that does. The trash count covers the whole trash
    /// either way.
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
                        if let Some(hit) = first_active_hit(&record, &search) {
                            list.matches
                                .insert(record.session.id.clone(), hit.id.clone());
                        }
                        list.sessions.push(record.session);
                    }
                }
                Ok(None) => {}
                Err(error) => list.unreadable.push(UnreadableSession { id: sid, error }),
            }
        }
        list.sessions.sort_by(|a, b| {
            // The trash is not a list of pins: a pin only waits there for an undo.
            let pinned = |s: &Session| if deleted { None } else { s.pinned_at };
            pinned(b)
                .cmp(&pinned(a))
                .then_with(|| b.updated_at.cmp(&a.updated_at))
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
        self.update_checked(request, None, precondition)
    }

    /// [`Self::update`]; with `levels` the session's thinking level is also
    /// checked, under the same lock, against the model the saved session has
    /// once the update applies: a level the update sets must be one that
    /// model offers, and one a model change leaves behind goes back to the
    /// default.
    pub fn update_checked(
        &self,
        request: &UpdateRequest,
        levels: Option<&ThinkingLevels>,
        precondition: impl FnOnce() -> Result<(), String>,
    ) -> Result<Session, String> {
        self.with_attachment_lock(&request.session_id, || {
            let (mut session, shrank) = self.mutate(&request.session_id, |record| {
                precondition()?;
                let previous = record.session.draft_attachment_ids.clone();
                apply_update(record, request)?;
                if let Some(levels) = levels {
                    check_thinking_level(&mut record.session, request, levels)?;
                }
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
        self.update_prefs(|prefs| prefs.model_id = Some(model_id.into()))
    }

    pub fn remember_model_kind(
        &self,
        model_id: &str,
        kind: super::catalog::Kind,
    ) -> Result<(), String> {
        use super::catalog::Kind;
        match kind {
            Kind::Chat => self.remember_model(model_id),
            Kind::Image => self.update_prefs(|prefs| prefs.image_model_id = Some(model_id.into())),
            Kind::Video => self.update_prefs(|prefs| prefs.video_model_id = Some(model_id.into())),
            Kind::Speech => {
                self.update_prefs(|prefs| prefs.speech_model_id = Some(model_id.into()))
            }
            Kind::Transcription => {
                self.update_prefs(|prefs| prefs.transcription_model_id = Some(model_id.into()))
            }
        }
    }

    pub fn remembered_models(&self) -> Result<Value, String> {
        let prefs = self.read_prefs()?.unwrap_or_default();
        Ok(
            json!({"chat":prefs.model_id,"image":prefs.image_model_id,"video":prefs.video_model_id,"speech":prefs.speech_model_id,"transcription":prefs.transcription_model_id}),
        )
    }

    pub fn remembered_model(&self) -> Result<Option<String>, String> {
        Ok(self.read_prefs()?.and_then(|prefs| prefs.model_id))
    }

    /// The assistant settings, the defaults for what was never saved.
    /// A damaged preferences file is an error.
    pub fn preferences(&self) -> Result<Preferences, String> {
        Ok(self.read_prefs()?.unwrap_or_default().resolved())
    }

    /// [`Self::preferences`], or the defaults when the file is damaged: a
    /// damaged file is preserved; background runs use defaults until repaired.
    pub fn preferences_or_default(&self) -> Preferences {
        self.preferences().unwrap_or_else(|error| {
            log::warn!("[Assistant] using the default settings: {error}");
            Preferences::default()
        })
    }

    /// Save every setting; the remembered model and fields this version does
    /// not know stay. A damaged or unsupported file is preserved and reported.
    ///
    /// Blocking file IO: async callers run it in `spawn_blocking`.
    pub fn save_preferences(&self, preferences: &Preferences) -> Result<Preferences, String> {
        if preferences.instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
            return Err(INSTRUCTIONS_TOO_LONG.into());
        }
        self.update_prefs(|prefs| {
            prefs.instructions = Some(preferences.instructions.clone());
            prefs.auto_title = Some(preferences.auto_title);
            prefs.notify_on_finish = Some(preferences.notify_on_finish);
        })?;
        Ok(preferences.clone())
    }

    /// Whether a conversation's first answer gets it a generated title.
    pub fn auto_title_enabled(&self) -> bool {
        self.preferences_or_default().auto_title
    }

    fn read_prefs(&self) -> Result<Option<Prefs>, String> {
        if !real_dir(&self.root)? {
            return Ok(None);
        }
        let Some(bytes) = read_regular(&self.root.join(PREFS_FILE))? else {
            return Ok(None);
        };
        let (prefs, _) = Prefs::parse(&bytes).ok_or(INVALID_PREFERENCES)?;
        Ok(Some(prefs))
    }

    /// Read, change and write `prefs.json` under its lock. Whatever else the
    /// file holds stays; a damaged or unsupported file is never overwritten.
    fn update_prefs(&self, change: impl FnOnce(&mut Prefs)) -> Result<(), String> {
        let path = self.ensure_root()?.join(PREFS_FILE);
        let lock = file_lock(&path);
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        ensure_regular_or_absent(&path)?;
        let (mut prefs, object) = match read_regular(&path)? {
            Some(bytes) => Prefs::parse(&bytes).ok_or(INVALID_PREFERENCES)?,
            None => Default::default(),
        };
        prefs.version = FORMAT_VERSION;
        change(&mut prefs);
        write_bytes(&path, &prefs.to_bytes(object)?)
    }

    /// Token and cost totals over every session of this owner, trash included.
    /// A cost is only given when every session's usage is known and complete:
    /// an interrupted run, an unpriced request or an unreadable session turns
    /// the estimate into `null` instead of a low number.
    pub fn usage_summary(&self) -> Result<Value, String> {
        let mut totals = UsageTotals::new();
        let mut records = vec![];
        if real_dir(&self.root)? {
            for sid in self.session_ids()? {
                match read_listed(&self.root, &sid) {
                    Ok(Some(mut record)) => {
                        // The summary needs billing facts, not every transcript
                        // and inline preview held in memory at the same time.
                        if record.messages.iter().any(|m| m.status == "streaming") {
                            record.usage["usageCoverage"] = json!("incomplete");
                        }
                        record.messages.clear();
                        record.attachments.clear();
                        record.session.draft.clear();
                        record.session.draft_attachment_ids.clear();
                        for op in &mut record.operations {
                            op.media = None;
                            op.search_price = None;
                        }
                        records.push(record);
                    }
                    Ok(None) => {}
                    Err(_) => totals.priced = false,
                }
            }
        }
        let present_sessions: HashSet<&str> =
            records.iter().map(|r| r.session.id.as_str()).collect();
        let owned_runs: HashSet<String> = records
            .iter()
            .flat_map(|r| {
                let inherited: HashSet<&str> = r
                    .inherited_usage
                    .iter()
                    .map(|i| i.run_id.as_str())
                    .collect();
                r.operations
                    .iter()
                    .filter(move |op| !inherited.contains(op.run_id.as_str()))
                    .map(|op| op.run_id.clone())
            })
            .collect();
        let mut seen = HashSet::new();
        for record in &records {
            if record.inherited_usage.is_empty() {
                totals.add(record);
                continue;
            }
            totals.sessions += 1;
            let usage = fork::own_usage(record);
            let own_pending = record.session.active_run.is_some()
                || record.operations.iter().any(|op| {
                    op.status == OperationStatus::Running
                        && !record.inherited_usage.iter().any(|i| i.run_id == op.run_id)
                });
            let has_own_operations = record
                .operations
                .iter()
                .any(|op| !record.inherited_usage.iter().any(|i| i.run_id == op.run_id));
            if own_pending
                || (has_own_operations && usage["usageCoverage"] == "incomplete")
                || usage["modelRequests"].as_u64().unwrap_or(0) > 0
                || usage["searchCount"].as_u64().unwrap_or(0) > 0
            {
                totals.add_values(&usage, own_pending);
            }
            for inherited in &record.inherited_usage {
                if !present_sessions.contains(inherited.source_session_id.as_str())
                    && !owned_runs.contains(&inherited.run_id)
                    && seen.insert((inherited.source_session_id.clone(), inherited.key.clone()))
                {
                    totals.add_values(&inherited.usage, false);
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

/// Each conversation model's thinking levels, by model id, as the host's
/// catalog lists them.
pub type ThinkingLevels = HashMap<String, Vec<String>>;

/// The level `session` has after an update must be one its model offers: a
/// level the update set is refused otherwise, one a model change left behind
/// goes back to the default.
fn check_thinking_level(
    session: &mut Session,
    request: &UpdateRequest,
    levels: &ThinkingLevels,
) -> Result<(), String> {
    let Some(level) = session.thinking_level.as_deref() else {
        return Ok(());
    };
    let offered = levels
        .get(&session.model_id)
        .is_some_and(|offered| offered.iter().any(|known| known == level));
    if offered {
        return Ok(());
    }
    if request.thinking_level.is_some() {
        return Err("assistant_invalid_thinking_level".into());
    }
    session.thinking_level = None;
    Ok(())
}

fn apply_update(record: &mut SessionRecord, request: &UpdateRequest) -> Result<(), String> {
    if let Some(title) = &request.title {
        if title.len() > MAX_TITLE_BYTES {
            return Err("assistant_title_too_long".into());
        }
        record.session.title = title.clone();
        record.session.title_source = Some(TitleSource::User);
    }
    if let Some(model) = &request.model_id {
        if record.session.model_id != *model {
            record.session.context_usage = None;
        }
        record.session.model_id = model.clone();
    }
    if let Some(web) = request.web_search {
        record.session.web_search = web;
    }
    // Checked against the model by `update_checked`.
    if let Some(level) = &request.thinking_level {
        record.session.thinking_level = (level != thinking::DEFAULT).then(|| level.clone());
    }
    if let Some(deleted) = request.deleted {
        record.session.deleted_at = deleted.then(now);
    }
    if let Some(pinned) = request.pinned {
        // Pinning again keeps the first pin's place.
        record.session.pinned_at = pinned.then(|| record.session.pinned_at.unwrap_or_else(now));
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
    // A pin or the composer draft is bookkeeping, not activity: it must not
    // move the conversation in the list.
    let changes_the_conversation = request.title.is_some()
        || request.model_id.is_some()
        || request.web_search.is_some()
        || request.thinking_level.is_some()
        || request.deleted.is_some();
    if changes_the_conversation {
        record.session.updated_at = now();
    }
    Ok(())
}

fn matches_search(record: &SessionRecord, search: &str) -> bool {
    search.is_empty()
        || record.session.title.to_lowercase().contains(search)
        || record.messages.iter().any(|m| says(m, search))
}

/// The first message of the live branch that `search` (already lowercase)
/// matches: what the page scrolls to. A replaced message is not shown.
fn first_active_hit<'a>(record: &'a SessionRecord, search: &str) -> Option<&'a Message> {
    if search.is_empty() {
        return None;
    }
    record
        .messages
        .iter()
        .find(|m| !m.superseded && says(m, search))
}

fn says(message: &Message, search: &str) -> bool {
    message.content.to_lowercase().contains(search)
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
    media_estimate: rust_decimal::Decimal,
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
            media_estimate: rust_decimal::Decimal::ZERO,
            priced: true,
        }
    }

    fn add(&mut self, record: &SessionRecord) {
        self.sessions += 1;
        let interrupted = record.session.active_run.is_some()
            || record.messages.iter().any(|m| m.status == "streaming")
            || record
                .operations
                .iter()
                .any(|op| op.status == OperationStatus::Running);
        self.add_values(&record.usage, interrupted);
    }

    fn add_values(&mut self, usage: &Value, interrupted: bool) {
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
        if let Some(value) = decimal(&usage["mediaCostUsd"]) {
            self.media_estimate += value;
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
            "mediaCostUsd": price(self.media_estimate),
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
    let mut record = file.record;
    derive_title_source(&mut record);
    Ok(record)
}

/// A record saved before titles had a source gets one: its title is the
/// first message's own start (what a send names a conversation) unless the
/// user renamed it. Read again, a record saved with a source keeps it.
fn derive_title_source(record: &mut SessionRecord) {
    if record.session.title_source.is_some() {
        return;
    }
    let automatic = record
        .messages
        .iter()
        .find(|m| m.role == "user")
        .map_or_else(String::new, |first| {
            auto_title(&first.content, &first.attachments)
        });
    record.session.title_source = Some(if record.session.title == automatic {
        TitleSource::Auto
    } else {
        TitleSource::User
    });
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
            expected_owner: None,
            expected_generation: None,
            media: None,
            video: None,
            quote_id: None,
            session_id: sid.into(),
            run_id: id(),
            operation_id: id(),
            text: "New text".into(),
            attachment_ids: vec![],
            replace_attachments: false,
            edit_message_id: edit.map(str::to_owned),
            regenerate_message_id: regenerate.map(str::to_owned),
            retry_message_id: None,
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
            media: None,
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

    /// A conversation with its update time fixed, so the order is explicit.
    fn session_updated_at(store: &AssistantStore, at: i64) -> String {
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&sid, |record| {
                record.session.updated_at = at;
                Ok(())
            })
            .unwrap();
        sid
    }

    fn pin(store: &AssistantStore, sid: &str, pinned: bool) -> Session {
        let mut change = request(sid);
        change.pinned = Some(pinned);
        store.update(&change, no_precondition).unwrap()
    }

    fn listed_ids(store: &AssistantStore, deleted: bool) -> Vec<String> {
        store
            .list(deleted, "")
            .unwrap()
            .sessions
            .into_iter()
            .map(|s| s.id)
            .collect()
    }

    #[test]
    fn pinned_conversations_come_first_the_latest_pin_on_top() {
        let (_config, store) = fixture();
        let oldest = session_updated_at(&store, 10);
        let middle = session_updated_at(&store, 20);
        let newest = session_updated_at(&store, 30);
        assert_eq!(
            listed_ids(&store, false),
            [newest.as_str(), middle.as_str(), oldest.as_str()]
        );

        pin(&store, &oldest, true);
        std::thread::sleep(std::time::Duration::from_millis(5));
        pin(&store, &middle, true);
        // Pinned first (latest pin on top), then the rest by update time.
        assert_eq!(
            listed_ids(&store, false),
            [middle.as_str(), oldest.as_str(), newest.as_str()]
        );

        pin(&store, &middle, false);
        assert_eq!(
            listed_ids(&store, false),
            [oldest.as_str(), newest.as_str(), middle.as_str()]
        );
    }

    #[test]
    fn pinning_twice_keeps_the_first_pin_time_and_unpinning_clears_it() {
        let (_config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let first = pin(&store, &sid, true).pinned_at.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(pin(&store, &sid, true).pinned_at, Some(first));
        assert_eq!(pin(&store, &sid, false).pinned_at, None);
        assert_eq!(store.load_detail(&sid).unwrap().session.pinned_at, None);
    }

    #[test]
    fn a_pin_does_not_reorder_the_trash() {
        let (_config, store) = fixture();
        let pinned_then_trashed = session_updated_at(&store, 10);
        let trashed_later = session_updated_at(&store, 20);
        pin(&store, &pinned_then_trashed, true);
        for sid in [&pinned_then_trashed, &trashed_later] {
            let mut trash = request(sid);
            trash.deleted = Some(true);
            store.update(&trash, no_precondition).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Most recently trashed first, pin or not; the pin survives for an undo.
        assert_eq!(
            listed_ids(&store, true),
            [trashed_later.as_str(), pinned_then_trashed.as_str()]
        );
        let session = &store.load_detail(&pinned_then_trashed).unwrap().session;
        assert!(session.pinned_at.is_some());
    }

    #[test]
    fn pin_and_draft_changes_leave_the_update_time_alone() {
        let (_config, store) = fixture();
        let sid = session_updated_at(&store, 5);
        let attachment = attachment("ready");
        let attachment_id = attachment.id.clone();
        store
            .mutate(&sid, |record| {
                record.attachments.push(attachment);
                Ok(())
            })
            .unwrap();
        let quiet = |change: &dyn Fn(&mut UpdateRequest)| {
            let mut update = request(&sid);
            change(&mut update);
            let session = store.update(&update, no_precondition).unwrap();
            assert_eq!(session.updated_at, 5);
            assert_eq!(store.load_detail(&sid).unwrap().session.updated_at, 5);
        };
        quiet(&|u| u.pinned = Some(true));
        quiet(&|u| u.pinned = Some(false));
        quiet(&|u| u.draft = Some("half a thought".into()));
        quiet(&|u| u.draft_attachment_ids = Some(vec![attachment_id.clone()]));
        quiet(&|u| {
            u.pinned = Some(true);
            u.draft = Some("both at once".into());
        });
        assert_eq!(
            store.load_detail(&sid).unwrap().session.draft,
            "both at once"
        );
    }

    #[test]
    fn every_other_update_still_moves_the_update_time() {
        let (_config, store) = fixture();
        let sid = session_updated_at(&store, 5);
        let loud: [&dyn Fn(&mut UpdateRequest); 6] = [
            &|u| u.title = Some("Renamed".into()),
            &|u| u.model_id = Some("other/model".into()),
            &|u| u.web_search = Some(true),
            &|u| u.thinking_level = Some("high".into()),
            &|u| u.deleted = Some(true),
            &|u| u.deleted = Some(false),
        ];
        for change in loud {
            let mut update = request(&sid);
            change(&mut update);
            assert!(store.update(&update, no_precondition).unwrap().updated_at > 5);
            store
                .mutate(&sid, |record| {
                    record.session.updated_at = 5;
                    Ok(())
                })
                .unwrap();
        }
        // A pin does not hide a change that does move it.
        let mut update = request(&sid);
        update.pinned = Some(true);
        update.title = Some("Pinned and renamed".into());
        assert!(store.update(&update, no_precondition).unwrap().updated_at > 5);
    }

    // ── title source ─────────────────────────────────────────────────────

    /// `session.json` as an earlier version wrote it: no `pinnedAt`, no
    /// `titleSource`; `title` and `messages` as given.
    fn save_as_old_version(
        config: &Path,
        store: &AssistantStore,
        sid: &str,
        title: &str,
        messages: &[Message],
    ) {
        let path = session_file(config, store, sid);
        let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let session = saved["session"].as_object_mut().unwrap();
        session.remove("pinnedAt");
        session.remove("titleSource");
        session.insert("title".into(), json!(title));
        saved["messages"] = serde_json::to_value(messages).unwrap();
        fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    }

    fn title_source_of(store: &AssistantStore, sid: &str) -> Option<TitleSource> {
        store.load_detail(sid).unwrap().session.title_source
    }

    #[test]
    fn a_new_conversation_is_named_by_its_first_message_and_a_rename_is_the_users() {
        let (_config, store) = fixture();
        let created = store.create("test/model", false).unwrap().session;
        assert_eq!(created.title_source, Some(TitleSource::Auto));
        assert_eq!(created.pinned_at, None);

        let mut rename = request(&created.id);
        rename.title = Some("Trip plan".into());
        let renamed = store.update(&rename, no_precondition).unwrap();
        assert_eq!(renamed.title_source, Some(TitleSource::User));
        assert_eq!(
            title_source_of(&store, &created.id),
            Some(TitleSource::User)
        );

        // Only a rename counts: a pin keeps whoever named it.
        let other = store.create("test/model", false).unwrap().session.id;
        assert_eq!(
            pin(&store, &other, true).title_source,
            Some(TitleSource::Auto)
        );
    }

    #[test]
    fn an_old_conversation_titled_by_its_first_message_reads_as_auto() {
        let (config, store) = fixture();
        let long = "Explain the difference between a mutex and a semaphore, with examples";
        assert!(long.chars().count() > 48);
        let cases: [(&str, Vec<Message>, Option<TitleSource>); 4] = [
            // Cut to 48 characters, as the first send does.
            (&long[..48], vec![message(long)], Some(TitleSource::Auto)),
            // Not sent yet.
            ("", vec![], Some(TitleSource::Auto)),
            // Renamed by hand in the old version.
            (
                "Locking notes",
                vec![message(long)],
                Some(TitleSource::User),
            ),
            // Renamed before its first message.
            ("Locking notes", vec![], Some(TitleSource::User)),
        ];
        for (title, messages, expected) in cases {
            let sid = store.create("test/model", false).unwrap().session.id;
            save_as_old_version(config.path(), &store, &sid, title, &messages);
            assert_eq!(title_source_of(&store, &sid), expected, "{title:?}");
            // The title is never changed by deriving its source.
            assert_eq!(store.load_detail(&sid).unwrap().session.title, title);
        }
    }

    #[test]
    fn an_old_conversation_started_with_a_file_alone_reads_as_auto() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut first = message("   ");
        first.attachments = vec![attachment("ready")];
        save_as_old_version(config.path(), &store, &sid, "a.txt", &[first]);
        assert_eq!(title_source_of(&store, &sid), Some(TitleSource::Auto));
    }

    #[test]
    fn an_old_conversation_is_compared_with_its_first_message_not_a_later_one() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut reply = message("an answer");
        reply.role = "assistant".into();
        let messages = [message("first question"), reply, message("second question")];
        save_as_old_version(config.path(), &store, &sid, "second question", &messages);
        assert_eq!(title_source_of(&store, &sid), Some(TitleSource::User));
        save_as_old_version(config.path(), &store, &sid, "first question", &messages);
        assert_eq!(title_source_of(&store, &sid), Some(TitleSource::Auto));
    }

    #[test]
    fn an_old_conversation_round_trips_with_the_new_fields_filled_in() {
        let (config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        save_as_old_version(config.path(), &store, &sid, "Kept", &[message("hello")]);
        let session = store.load_detail(&sid).unwrap().session;
        assert_eq!(session.pinned_at, None);
        assert_eq!(session.title_source, Some(TitleSource::User));

        // The next save writes what the read derived, and it is read back as is.
        pin(&store, &sid, true);
        let saved: Value =
            serde_json::from_slice(&fs::read(session_file(config.path(), &store, &sid)).unwrap())
                .unwrap();
        assert_eq!(saved["session"]["titleSource"], "user");
        assert!(saved["session"]["pinnedAt"].is_i64());
        let session = store.load_detail(&sid).unwrap().session;
        assert_eq!(session.title_source, Some(TitleSource::User));
        assert!(session.pinned_at.is_some());
    }

    // ── search hits ──────────────────────────────────────────────────────

    #[test]
    fn a_search_names_the_first_matching_message_that_is_still_active() {
        let (_config, store) = fixture();
        let talked = store.create("test/model", false).unwrap().session.id;
        let mut old_branch = message("The Kyoto draft I replaced");
        old_branch.superseded = true;
        let first_hit = message("Plan the KYOTO trip");
        let later_hit = message("More about kyoto food");
        let wanted = first_hit.id.clone();
        store
            .mutate(&talked, |record| {
                record.messages.extend([
                    message("Hello"),
                    old_branch,
                    first_hit,
                    message("Anything else?"),
                    later_hit,
                ]);
                Ok(())
            })
            .unwrap();
        let titled = store.create("test/model", false).unwrap().session.id;
        let mut rename = request(&titled);
        rename.title = Some("Kyoto notes".into());
        store.update(&rename, no_precondition).unwrap();
        let only_replaced = store.create("test/model", false).unwrap().session.id;
        let mut replaced = message("kyoto, before the edit");
        replaced.superseded = true;
        store
            .mutate(&only_replaced, |record| {
                record.messages.push(replaced);
                Ok(())
            })
            .unwrap();
        store.create("test/model", false).unwrap();

        let list = store.list(false, "kyoto").unwrap();
        assert_eq!(list.sessions.len(), 3);
        // A title-only or replaced-only match is listed but has nowhere to jump.
        assert_eq!(list.matches, HashMap::from([(talked.clone(), wanted)]));
        let value = serde_json::to_value(&list).unwrap();
        assert!(value["matches"][&talked].is_string());
    }

    #[test]
    fn without_a_search_there_are_no_matches() {
        let (_config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&sid, |record| {
                record.messages.push(message("anything"));
                Ok(())
            })
            .unwrap();
        assert!(store.list(false, "").unwrap().matches.is_empty());
        assert!(store.list(false, "nowhere").unwrap().matches.is_empty());
        let value = serde_json::to_value(store.list(false, "").unwrap()).unwrap();
        assert_eq!(value["matches"], json!({}));
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
                audio_only: None,
                context_usage: None,
                compacting: None,
                id: sid.into(),
                title: String::new(),
                title_source: Some(TitleSource::Auto),
                model_id: "test/model".into(),
                web_search: false,
                thinking_level: None,
                created_at: 1,
                updated_at: 1,
                deleted_at: None,
                pinned_at: None,
                active_run: None,
                draft: String::new(),
                draft_attachment_ids: vec![],
                cleanup_warning: None,
            },
            messages: vec![],
            attachments: vec![],
            usage: json!({}),
            operations: vec![],
            inherited_usage: vec![],
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
    fn a_thinking_level_is_kept_until_default_clears_it() {
        let (_config, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut change = request(&sid);
        change.thinking_level = Some("high".into());
        assert_eq!(
            store
                .update(&change, no_precondition)
                .unwrap()
                .thinking_level
                .as_deref(),
            Some("high")
        );
        let mut other = request(&sid);
        other.title = Some("Renamed".into());
        store.update(&other, no_precondition).unwrap();
        assert_eq!(
            store
                .load_detail(&sid)
                .unwrap()
                .session
                .thinking_level
                .as_deref(),
            Some("high")
        );
        change.thinking_level = Some(thinking::DEFAULT.into());
        assert!(store
            .update(&change, no_precondition)
            .unwrap()
            .thinking_level
            .is_none());
        assert!(store
            .load_detail(&sid)
            .unwrap()
            .session
            .thinking_level
            .is_none());
    }

    #[test]
    fn a_level_is_checked_against_the_model_the_saved_session_has() {
        let (_config, store) = fixture();
        let sid = store.create("test/a", false).unwrap().session.id;
        let levels: ThinkingLevels = [
            (
                "test/a".to_string(),
                vec!["low".to_string(), "high".to_string()],
            ),
            ("test/b".to_string(), vec!["low".to_string()]),
        ]
        .into();
        let change = |value: Value| -> UpdateRequest {
            let mut request = json!({ "sessionId": sid });
            request
                .as_object_mut()
                .unwrap()
                .extend(value.as_object().unwrap().clone());
            serde_json::from_value(request).unwrap()
        };
        let checked =
            |value: Value| store.update_checked(&change(value), Some(&levels), no_precondition);
        let saved = || store.load_record(&sid).unwrap().session;
        const INVALID: &str = "assistant_invalid_thinking_level";

        assert_eq!(
            checked(json!({"thinkingLevel": "high"}))
                .unwrap()
                .thinking_level
                .as_deref(),
            Some("high")
        );
        // Another update switched the model after the host looked: the level
        // is checked against the model saved now, under the same lock.
        store
            .update(
                &change(json!({"modelId": "test/b", "thinkingLevel": "default"})),
                no_precondition,
            )
            .unwrap();
        assert_eq!(
            checked(json!({"thinkingLevel": "high"})).unwrap_err(),
            INVALID
        );
        assert!(saved().thinking_level.is_none());
        // A refused update changes nothing, the model included.
        assert_eq!(
            checked(json!({"modelId": "test/a", "thinkingLevel": "max"})).unwrap_err(),
            INVALID
        );
        assert_eq!(saved().model_id, "test/b");
        // A model and its level together; a model change keeps a level the
        // new model offers and drops one it lacks.
        checked(json!({"modelId": "test/a", "thinkingLevel": "high"})).unwrap();
        let dropped = checked(json!({"modelId": "test/b"})).unwrap();
        assert!(dropped.thinking_level.is_none());
        checked(json!({"thinkingLevel": "low"})).unwrap();
        let kept = checked(json!({"modelId": "test/a"})).unwrap();
        assert_eq!(kept.thinking_level.as_deref(), Some("low"));
        // A model the catalog no longer offers has no level.
        let gone = store.update(&change(json!({"modelId": "test/gone"})), no_precondition);
        assert_eq!(gone.unwrap().thinking_level.as_deref(), Some("low"));
        assert_eq!(
            checked(json!({"thinkingLevel": "low"})).unwrap_err(),
            INVALID
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
            media: None,
            id: running.clone(),
            run_id: id(),
            status: OperationStatus::Running,
            search_price: None,
        });
        record.operations.push(Operation {
            media: None,
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
    fn damaged_prefs_are_reported_and_model_choices_preserve_them() {
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
        let before = fs::read(&path).unwrap();
        assert_eq!(
            store.remember_model("next/model").unwrap_err(),
            INVALID_PREFERENCES
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    // ── assistant settings ───────────────────────────────────────────────

    fn prefs_path(config: &Path, store: &AssistantStore) -> PathBuf {
        config
            .join("assistant")
            .join(&store.owner)
            .join("prefs.json")
    }

    fn settings(instructions: &str, auto_title: bool, notify_on_finish: bool) -> Preferences {
        Preferences {
            instructions: instructions.into(),
            auto_title,
            notify_on_finish,
        }
    }

    #[test]
    fn preferences_start_with_no_instructions_and_both_switches_on() {
        let (_config, store) = fixture();
        let defaults = settings("", true, true);
        assert_eq!(store.preferences().unwrap(), defaults);
        // A file from before the settings existed holds only the model.
        store.remember_model("test/model").unwrap();
        assert_eq!(store.preferences().unwrap(), defaults);
        assert!(store.auto_title_enabled());
    }

    #[test]
    fn saved_preferences_are_read_back_beside_the_remembered_model_and_unknown_fields() {
        let (config, store) = fixture();
        store.remember_model("test/model").unwrap();
        let path = prefs_path(config.path(), &store);
        fs::write(
            &path,
            r#"{"version":1,"modelId":"test/model","laterField":{"kept":true}}"#,
        )
        .unwrap();
        let chosen = settings("Answer in English.", false, false);
        assert_eq!(store.save_preferences(&chosen).unwrap(), chosen);
        assert_eq!(store.preferences().unwrap(), chosen);
        assert_eq!(
            store.remembered_model().unwrap().as_deref(),
            Some("test/model")
        );
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved,
            json!({
                "version": 1, "modelId": "test/model", "laterField": {"kept": true},
                "instructions": "Answer in English.", "autoTitle": false, "notifyOnFinish": false,
            })
        );
    }

    #[test]
    fn remembering_a_model_keeps_the_saved_preferences_and_unknown_fields() {
        let (config, store) = fixture();
        let chosen = settings("Be brief.", false, true);
        store.save_preferences(&chosen).unwrap();
        let path = prefs_path(config.path(), &store);
        let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        saved["laterField"] = json!(7);
        fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        store.remember_model("next/model").unwrap();
        assert_eq!(store.preferences().unwrap(), chosen);
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["modelId"], "next/model");
        assert_eq!(saved["laterField"], 7);
    }

    #[test]
    fn instructions_are_limited_to_4000_characters() {
        let (_config, store) = fixture();
        // Characters, not bytes: 4,000 CJK characters are 12,000 bytes.
        let longest = settings(&"字".repeat(4000), true, true);
        assert_eq!(store.save_preferences(&longest).unwrap(), longest);
        assert_eq!(
            store
                .save_preferences(&settings(&"a".repeat(4001), true, true))
                .unwrap_err(),
            "assistant_instructions_too_long"
        );
        assert_eq!(store.preferences().unwrap(), longest);
    }

    #[test]
    fn auto_title_follows_the_saved_switch_and_a_damaged_file_leaves_it_on() {
        let (config, store) = fixture();
        store.save_preferences(&settings("", false, true)).unwrap();
        assert!(!store.auto_title_enabled());
        store.save_preferences(&settings("", true, true)).unwrap();
        assert!(store.auto_title_enabled());
        fs::write(prefs_path(config.path(), &store), "damaged").unwrap();
        assert!(store.auto_title_enabled());
    }

    #[test]
    fn damaged_preferences_are_reported_and_saving_preserves_them() {
        let (config, store) = fixture();
        let path = prefs_path(config.path(), &store);
        store.remember_model("first/model").unwrap();
        let too_long = format!(r#"{{"version":1,"instructions":"{}"}}"#, "a".repeat(4001));
        for damaged in [
            "damaged",
            r#"{"version":2}"#,
            r#"{"version":1,"autoTitle":"yes"}"#,
            too_long.as_str(),
        ] {
            fs::write(&path, damaged).unwrap();
            assert_eq!(
                store.preferences().unwrap_err(),
                "assistant_invalid_preferences",
                "{damaged}"
            );
            assert_eq!(store.preferences_or_default(), settings("", true, true));
            let chosen = settings("Use metric units.", true, false);
            assert_eq!(
                store.save_preferences(&chosen).unwrap_err(),
                INVALID_PREFERENCES
            );
            assert_eq!(
                store.remember_model("next/model").unwrap_err(),
                INVALID_PREFERENCES
            );
            assert_eq!(fs::read(&path).unwrap(), damaged.as_bytes());
        }
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
                    media: None,
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
    #[test]
    fn changing_a_model_invalidates_the_previous_context_window_estimate() {
        let (_temp, store) = fixture();
        let detail = store.create("model", false).unwrap();
        let sid = detail.session.id;
        store
            .mutate(&sid, |r| {
                r.session.context_usage = Some(ContextUsage {
                    model_id: "model".into(),
                    tokens: Some(15),
                    context_window: 32000,
                });
                Ok(())
            })
            .unwrap();
        let mut patch = request(&sid);
        patch.model_id = Some("other".into());
        assert!(store
            .update(&patch, no_precondition)
            .unwrap()
            .context_usage
            .is_none());
    }
    #[test]
    fn audio_models_are_remembered_independently_without_replacing_chat_or_media() {
        let (_temp, store) = fixture();
        store.remember_model("chat").unwrap();
        store
            .remember_model_kind("image", super::super::catalog::Kind::Image)
            .unwrap();
        store
            .remember_model_kind("video", super::super::catalog::Kind::Video)
            .unwrap();
        store
            .remember_model_kind("speech", super::super::catalog::Kind::Speech)
            .unwrap();
        store
            .remember_model_kind("transcription", super::super::catalog::Kind::Transcription)
            .unwrap();
        let remembered = store.remembered_models().unwrap();
        assert_eq!(remembered["speech"], "speech");
        assert_eq!(remembered["transcription"], "transcription");
        assert_eq!(remembered["chat"], "chat");
        assert_eq!(remembered["image"], "image");
        assert_eq!(remembered["video"], "video");
    }

    #[test]
    fn audio_preferences_restore_only_fills_models_never_saved_on_this_machine() {
        let (mut current, object) =
            Prefs::parse(br#"{"version":1,"modelId":"chat","speechModelId":"mine"}"#).unwrap();
        let (archive, _) = Prefs::parse(
            br#"{"version":1,"speechModelId":"archived","transcriptionModelId":"recognizer"}"#,
        )
        .unwrap();
        assert!(current.fill_from(&archive));
        let merged: Value = serde_json::from_slice(&current.to_bytes(object).unwrap()).unwrap();
        assert_eq!(merged["speechModelId"], "mine");
        assert_eq!(merged["transcriptionModelId"], "recognizer");
        assert_eq!(merged["modelId"], "chat");
    }
    fn transcription_operation(status: &str, running: bool) -> Operation {
        serde_json::from_value(json!({"id":id(),"runId":id(),"status":if running {"running"} else {"completed"},"media":{
            "kind":"transcription","model":"openai/gpt-transcribe","apex":"ofox.io","params":{"language":"zh"},"status":status,"cost":"0.000675",
            "audio":{"pricing":{"unit":"seconds","inputAudioPerSecond":"0.000075"},"text":"下午三点开会。","usage":{"type":"duration","seconds":9},"requestId":"req_test_123"}
        }})).unwrap()
    }
    #[test]
    fn audio_transcription_is_kept_as_a_separate_result_without_a_phantom_chat_message() {
        let (_dir, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&sid, |r| {
                r.session.draft = "未发送的草稿".into();
                r.operations
                    .push(transcription_operation("completed", false));
                Ok(())
            })
            .unwrap();
        let detail = serde_json::to_value(store.load_detail(&sid).unwrap()).unwrap();
        assert_eq!(
            detail["audioOperations"][0]["audio"]["text"],
            "下午三点开会。"
        );
        assert_eq!(
            detail["audioOperations"][0]["audio"]["requestId"],
            "req_test_123"
        );
        assert_eq!(detail["session"]["draft"], "未发送的草稿");
        assert_eq!(detail["session"]["modelId"], "test/model");
        assert_eq!(detail["messages"], json!([]));
        assert_eq!(detail["attachments"], json!([]));
    }
    #[test]
    fn audio_request_cancelled_before_submission_is_not_counted_as_a_paid_attempt() {
        let (_dir, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&sid, |r| {
                r.operations
                    .push(transcription_operation("not_sent", false));
                super::super::projection::recount_usage(r, &[]);
                Ok(())
            })
            .unwrap();
        let usage = store.load_record(&sid).unwrap().usage;
        assert_eq!(usage["modelRequests"], 0);
        assert_ne!(usage["usageCoverage"], "incomplete");
    }
    #[test]
    fn audio_recovery_keeps_a_proven_unsent_attempt_unbilled() {
        let (_dir, store) = fixture();
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&sid, |r| {
                r.operations.push(transcription_operation("not_sent", true));
                super::super::projection::settle_interrupted(r, None)?;
                Ok(())
            })
            .unwrap();
        let record = store.load_record(&sid).unwrap();
        assert_eq!(record.operations[0].status, OperationStatus::Interrupted);
        assert_ne!(record.usage["usageCoverage"], "incomplete");
    }
}
