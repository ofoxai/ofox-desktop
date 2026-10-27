use super::{catalog, policy::*, process, store, types::*};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{Emitter, Manager};
use tokio::sync::{oneshot, watch};

#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    pub owner: String,
    pub apex: String,
}

pub async fn identity(app: &tauri::AppHandle) -> Result<Identity, String> {
    let auth = app.state::<crate::commands::ofox_auth::OfoxAuthState>();
    let manager = auth.0.read().await;
    let status = manager.get_auth_status().await;
    if status.state != crate::ofox_auth::OfoxAuthState::Active {
        return Err("assistant_sign_in_required".into());
    }
    let user = status
        .user
        .ok_or("assistant_identity_unavailable")?
        .key_owner();
    Ok(Identity {
        owner: store::namespace(
            user.user_id.as_deref().unwrap_or(""),
            user.org_id.as_deref().unwrap_or(""),
        )?,
        apex: crate::ofox_apex::current_apex().into(),
    })
}

pub async fn token(app: &tauri::AppHandle) -> Result<String, String> {
    let auth = app.state::<crate::commands::ofox_auth::OfoxAuthState>();
    let manager = auth.0.read().await;
    if !manager.has_scope("llm.invoke") {
        return Err("assistant_reauthorization_required".into());
    }
    manager
        .get_valid_access_token()
        .await
        .map_err(|_| "assistant_sign_in_required".into())
}

#[derive(Default)]
pub struct Grant {
    pub mode: PermissionMode,
    pub epoch: u64,
    pub approved: HashSet<String>,
}

pub struct PendingApproval {
    pub sender: oneshot::Sender<bool>,
    pub details: Value,
    pub signature: String,
}

pub struct RunControl {
    pub identity: Identity,
    pub session_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub message_id: String,
    pub cancel: watch::Sender<bool>,
    pub epoch: u64,
    pub deadline: tokio::time::Instant,
    pub outcome_unknown: AtomicBool,
    pub approvals: Mutex<HashMap<String, PendingApproval>>,
    pub secret: String,
    pub search_capabilities: Value,
}

#[derive(Default)]
struct RunAdmission {
    logouts: HashSet<u64>,
    needs_authorization: bool,
    shutting_down: bool,
}

#[derive(Default)]
pub struct AssistantHost {
    pub generation: AtomicU64,
    admission: Mutex<RunAdmission>,
    pub data_gate: tokio::sync::RwLock<()>,
    pub restore_previews: Mutex<HashMap<String, super::backup::RestoreReceipt>>,
    pub tasks: Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>,
    native_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    recovered: Mutex<HashSet<String>>,
    pub grants: Mutex<HashMap<String, Grant>>,
    pub runs: Mutex<HashMap<String, Arc<RunControl>>>,
    pub models: Mutex<HashMap<String, Vec<catalog::Model>>>,
}

fn grant_key(owner: &str, sid: &str) -> String {
    format!("{owner}:{sid}")
}
pub fn data_root(owner: &str, sid: &str) -> Result<PathBuf, String> {
    validate_id(sid)?;
    if owner.len() != 64 || !owner.bytes().all(|v| v.is_ascii_hexdigit()) {
        return Err("assistant_invalid_storage_path".into());
    }
    let mut root = crate::config::get_app_config_dir();
    for name in ["assistant", owner, sid] {
        root = private_subdir(&root, name)?;
    }
    Ok(root)
}
pub fn private_subdir(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let path = parent.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("assistant_invalid_storage_path".into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("assistant_storage_unavailable".into()),
    }
    crate::config::create_private_dir_all(&path).map_err(|_| "assistant_storage_unavailable")?;
    Ok(path)
}

#[cfg(target_os = "macos")]
fn compatible_macos() -> bool {
    std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|v| String::from_utf8(v.stdout).ok())
        .is_some_and(|v| {
            let mut parts = v.trim().split('.').filter_map(|p| p.parse::<u32>().ok());
            let major = parts.next().unwrap_or(0);
            let minor = parts.next().unwrap_or(0);
            major > 13 || (major == 13 && minor >= 5)
        })
}

pub fn runtime_paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf), String> {
    #[cfg(target_os = "macos")]
    if !compatible_macos() {
        return Err("assistant_requires_macos_13_5".into());
    }
    let base = if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime/resources")
    } else {
        app.path()
            .resource_dir()
            .map_err(|_| "assistant_runtime_unavailable")?
            .join("assistant-runtime")
    };
    let node = base.join(if cfg!(windows) {
        "node/node.exe"
    } else {
        "node/node"
    });
    let entry = base.join("runtime/src/main.mjs");
    if !node.is_file() || !entry.is_file() {
        return Err("assistant_runtime_unavailable".into());
    }
    Ok((node, entry))
}

impl AssistantHost {
    fn native_lock(&self, key: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
        Ok(self
            .native_locks
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .entry(key.into())
            .or_default()
            .clone())
    }
    fn history_is_live(&self, owner: &str, sid: &str) -> Result<bool, String> {
        if self
            .recovered
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .contains(&grant_key(owner, sid))
        {
            return Ok(true);
        }
        Ok(self
            .runs
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .get(sid)
            .is_some_and(|r| r.identity.owner == owner))
    }
    async fn acquire_recovery_lock(
        &self,
        owner: &str,
        sid: &str,
    ) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>, String> {
        if self.history_is_live(owner, sid)? {
            return Ok(None);
        }
        let guard = self.native_lock(&grant_key(owner, sid))?.lock_owned().await;
        if self.history_is_live(owner, sid)? {
            return Ok(None);
        }
        Ok(Some(guard))
    }
    pub async fn recover(
        &self,
        app: &tauri::AppHandle,
        identity: &Identity,
        sid: &str,
    ) -> Result<(), String> {
        let key = grant_key(&identity.owner, sid);
        let Some(_guard) = self.acquire_recovery_lock(&identity.owner, sid).await? else {
            return Ok(());
        };
        if self
            .recovered
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .contains(&key)
        {
            return Ok(());
        }
        if self
            .runs
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .contains_key(sid)
        {
            return Ok(());
        }
        let file = data_root(&identity.owner, sid)?.join("session.jsonl");
        ensure_regular(&file)?;
        if file.exists() {
            let result = load_native(app, &file).await?;
            if &super::host::identity(app).await? != identity {
                return Err("assistant_identity_changed".into());
            }
            let state = app.state::<crate::store::AppState>();
            store::mutate(&state.db, &identity.owner, sid, |detail| {
                let interrupted = detail.session.active_run.is_some();
                if interrupted {
                    detail.usage["usageCoverage"] = json!("incomplete");
                }
                merge_native(detail, &result["entries"], "")?;
                detail.session.active_run = None;
                for op in &mut detail.operations {
                    if op["status"] == "running" {
                        op["status"] = json!("interrupted");
                    }
                }
                Ok(())
            })?;
        }
        self.recovered
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .insert(key);
        Ok(())
    }
    pub fn decorate(&self, owner: &str, session: &mut Session) {
        session.permission_mode = self
            .grants
            .lock()
            .ok()
            .and_then(|g| g.get(&grant_key(owner, &session.id)).map(|g| g.mode))
            .unwrap_or_default();
        session.active_run = self.runs.lock().ok().and_then(|runs| {
            runs.get(&session.id)
                .filter(|r| r.identity.owner == owner)
                .map(|r| RunRef {
                    run_id: r.run_id.clone(),
                    operation_id: r.operation_id.clone(),
                })
        });
    }
    pub fn reset_recovery(&self) {
        if let Ok(mut recovered) = self.recovered.lock() {
            recovered.clear();
        }
    }
    pub fn cancel_session(&self, sid: &str) {
        if let Some(run) = self
            .runs
            .lock()
            .ok()
            .and_then(|runs| runs.get(sid).cloned())
        {
            let _ = run.cancel.send_replace(true);
            if let Ok(mut pending) = run.approvals.lock() {
                pending.clear();
            }
        }
    }
    pub fn request_stop(
        &self,
        db: &crate::database::Database,
        identity: &Identity,
        sid: &str,
        rid: &str,
        op: &str,
    ) -> Result<(), String> {
        // Keep this lock through the cancellation receipt. Registration takes the
        // same lock while it commits its operation row and publishes RunControl.
        let runs = self
            .runs
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        if let Some(run) = runs.get(sid) {
            if &run.identity != identity || run.run_id != rid || run.operation_id != op {
                return Err("assistant_stale_operation".into());
            }
            let _ = run.cancel.send_replace(true);
            run.approvals
                .lock()
                .map_err(|_| "assistant_state_unavailable")?
                .clear();
        } else {
            store::mutate(db, &identity.owner, sid, |detail| {
                if !detail
                    .operations
                    .iter()
                    .any(|operation| operation["id"] == op)
                {
                    detail.operations.push(json!({"id":op,"runId":rid,"title":"assistant_request","status":"cancelled","requestStarted":false}));
                }
                Ok(())
            })?;
        }
        Ok(())
    }
    pub fn register_run(
        &self,
        run: Arc<RunControl>,
        generation: u64,
        publish: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        self.ensure_accepting_runs()?;
        if self.generation.load(Ordering::SeqCst) != generation {
            return Err("assistant_identity_changed".into());
        }
        if runs.contains_key(&run.session_id) {
            return Err("assistant_session_busy".into());
        }
        publish()?;
        runs.insert(run.session_id.clone(), run);
        Ok(())
    }
    pub fn ensure_accepting_runs(&self) -> Result<(), String> {
        let admission = self
            .admission
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        if admission.shutting_down {
            Err("assistant_shutting_down".into())
        } else if !admission.logouts.is_empty() {
            Err("assistant_signing_out".into())
        } else if admission.needs_authorization {
            Err("assistant_sign_in_required".into())
        } else {
            Ok(())
        }
    }
    pub fn begin_logout(&self) -> u64 {
        self.revoke_runs(|admission, receipt| {
            admission.needs_authorization = true;
            admission.logouts.insert(receipt);
        })
    }
    pub fn finish_logout(&self, receipt: u64) {
        self.admission
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .logouts
            .remove(&receipt);
    }
    pub fn authorization_succeeded(&self) {
        self.cancel_all();
        let _registration = self.runs.lock().unwrap_or_else(|error| error.into_inner());
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !admission.shutting_down && admission.logouts.is_empty() {
            admission.needs_authorization = false;
        }
    }
    pub fn begin_shutdown(&self) {
        self.revoke_runs(|admission, _| admission.shutting_down = true);
    }
    /// A native terminal callback may bypass ExitRequested entirely. Finish
    /// cleanup on that synchronous callback before the event loop disappears.
    pub fn handle_terminal_exit(&self, event: &tauri::RunEvent) -> bool {
        if !matches!(event, tauri::RunEvent::Exit) {
            return false;
        }
        tauri::async_runtime::block_on(self.shutdown());
        true
    }
    pub async fn shutdown(&self) {
        self.begin_shutdown();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        while self.runs.lock().is_ok_and(|runs| !runs.is_empty())
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if let Ok(mut tasks) = self.tasks.lock() {
            for (_, task) in tasks.drain() {
                task.abort();
            }
        }
        // Aborting drops the managed Child, which kills its group / Job Object.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    pub fn cancel_all(&self) {
        self.revoke_runs(|_, _| {});
    }
    fn revoke_runs(&self, transition: impl FnOnce(&mut RunAdmission, u64)) -> u64 {
        // Registration and lifecycle revocation linearize under the same lock.
        // Ordinary identity cancellation preserves admission; explicit logout
        // and shutdown close it before any awaited credential/exit cleanup.
        let runs = self.runs.lock().unwrap_or_else(|error| error.into_inner());
        let generation = self
            .generation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        {
            let mut admission = self
                .admission
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            transition(&mut admission, generation);
        }
        for run in runs.values() {
            let _ = run.cancel.send_replace(true);
            if let Ok(mut pending) = run.approvals.lock() {
                pending.clear();
            }
        }
        drop(runs);
        if let Ok(mut previews) = self.restore_previews.lock() {
            previews.clear();
        }
        if let Ok(mut grants) = self.grants.lock() {
            grants.clear();
        }
        generation
    }
    pub fn change_permission(
        &self,
        owner: &str,
        sid: &str,
        mode: PermissionMode,
    ) -> Result<(), String> {
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        let grant = grants.entry(grant_key(owner, sid)).or_default();
        if grant.mode != mode {
            grant.mode = mode;
            grant.epoch += 1;
            grant.approved.clear();
            drop(grants);
            self.cancel_session(sid);
        }
        Ok(())
    }
    pub fn current_epoch(&self, owner: &str, sid: &str) -> Result<u64, String> {
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        Ok(grants.entry(grant_key(owner, sid)).or_default().epoch)
    }
    pub fn validate_run(&self, run: &RunControl) -> Result<(), String> {
        if *run.cancel.borrow() {
            return Err("assistant_cancelled".into());
        }
        self.ensure_accepting_runs()?;
        if self.current_epoch(&run.identity.owner, &run.session_id)? != run.epoch {
            return Err("assistant_permission_changed".into());
        }
        Ok(())
    }
    pub async fn authorize(
        &self,
        app: &tauri::AppHandle,
        run: &Arc<RunControl>,
        configuration: bool,
        name: &str,
        args: &Value,
        class: OperationClass,
    ) -> Result<(), String> {
        self.validate_run(run)?;
        if identity(app).await? != run.identity {
            return Err("assistant_identity_changed".into());
        }
        let signature = format!(
            "{name}:{}",
            serde_json::to_string(args).map_err(|_| "assistant_invalid_tool")?
        );
        let decision = {
            let grants = self
                .grants
                .lock()
                .map_err(|_| "assistant_state_unavailable")?;
            let grant = grants
                .get(&grant_key(&run.identity.owner, &run.session_id))
                .ok_or("assistant_permission_changed")?;
            let decision = decide(configuration, grant.mode, class);
            if decision == Decision::Confirm && grant.approved.contains(&signature) {
                Decision::Allow
            } else {
                decision
            }
        };
        match decision {
            Decision::Allow => Ok(()),
            Decision::Deny => Err("assistant_operation_denied".into()),
            Decision::Confirm => {
                let approval_id = id();
                let (tx, rx) = oneshot::channel();

                let mut details = args.clone();
                store::scrub_value(&mut details, &[&run.secret]);
                let approval = json!({"id":approval_id,"title":name,"description":name,"details":serde_json::to_string_pretty(&details).unwrap_or_default()});
                run.approvals
                    .lock()
                    .map_err(|_| "assistant_state_unavailable")?
                    .insert(
                        approval_id.clone(),
                        PendingApproval {
                            sender: tx,
                            details: approval.clone(),
                            signature,
                        },
                    );
                emit(app, run, "approval", json!({"approval":approval}));
                let mut cancel = run.cancel.subscribe();
                let approved = tokio::select! { result=rx => result.unwrap_or(false), _=cancel.changed()=>false, _=tokio::time::sleep_until(run.deadline)=>false };
                if let Ok(mut pending) = run.approvals.lock() {
                    pending.remove(&approval_id);
                }
                if !approved {
                    return Err("assistant_operation_denied".into());
                }
                self.validate_run(run)?;
                if identity(app).await? != run.identity {
                    return Err("assistant_identity_changed".into());
                }
                Ok(())
            }
        }
    }
}

pub fn emit(app: &tauri::AppHandle, run: &RunControl, kind: &str, mut data: Value) {
    store::scrub_value(&mut data, &[&run.secret]);
    let _ = app.emit(
        "assistant:event",
        Event {
            session_id: run.session_id.clone(),
            run_id: run.run_id.clone(),
            operation_id: run.operation_id.clone(),
            r#type: kind.into(),
            data,
        },
    );
}

/// UI events are emitted immediately; only the durable projection is coalesced.
struct ProjectionBuffer {
    text: String,
    thinking: String,
    last_flush: std::time::Instant,
}
impl Default for ProjectionBuffer {
    fn default() -> Self {
        Self {
            text: String::new(),
            thinking: String::new(),
            last_flush: std::time::Instant::now(),
        }
    }
}
impl ProjectionBuffer {
    fn push(&mut self, run: &RunControl, kind: &str, data: &mut Value) {
        store::scrub_value(data, &[&run.secret]);
        data["messageId"] = json!(run.message_id);
        if let Some(delta) = data["delta"].as_str() {
            if kind == "text_delta" {
                self.text.push_str(delta);
            } else {
                self.thinking.push_str(delta);
            }
        }
    }
    fn due(&self) -> bool {
        (!self.text.is_empty() || !self.thinking.is_empty())
            && (self.last_flush.elapsed() >= Duration::from_millis(500)
                || self.text.len() + self.thinking.len() >= 64 * 1024)
    }
    fn flush(&mut self, db: &crate::database::Database, run: &RunControl) -> Result<(), String> {
        if self.text.is_empty() && self.thinking.is_empty() {
            return Ok(());
        }
        store::mutate(db, &run.identity.owner, &run.session_id, |detail| {
            let message = detail
                .messages
                .iter_mut()
                .find(|m| m.id == run.message_id)
                .ok_or("assistant_message_not_found")?;
            message.content.push_str(&self.text);
            if !self.thinking.is_empty() {
                message
                    .reasoning
                    .get_or_insert_default()
                    .push_str(&self.thinking);
            }
            message.content = crate::redact::scrub(
                &message
                    .content
                    .replace(&run.secret, crate::redact::REDACTED),
            )
            .into_owned();
            if let Some(reasoning) = &mut message.reasoning {
                *reasoning =
                    crate::redact::scrub(&reasoning.replace(&run.secret, crate::redact::REDACTED))
                        .into_owned();
            }
            detail.session.updated_at = now();
            Ok(())
        })?;
        self.text.clear();
        self.thinking.clear();
        self.last_flush = std::time::Instant::now();
        Ok(())
    }
}

fn event_projection(
    db: &crate::database::Database,
    run: &RunControl,
    kind: &str,
    data: &mut Value,
) -> Result<(), String> {
    store::scrub_value(data, &[&run.secret]);
    store::mutate(db, &run.identity.owner, &run.session_id, |detail| {
        let Some(message) = detail.messages.iter_mut().find(|m| m.id == run.message_id) else {
            return Err("assistant_message_not_found".into());
        };
        match kind {
            "text_delta" => {
                if let Some(delta) = data["delta"].as_str() {
                    message.content.push_str(delta);
                }
                data["messageId"] = json!(run.message_id);
            }
            "thinking_delta" => {
                if let Some(delta) = data["delta"].as_str() {
                    message.reasoning.get_or_insert_default().push_str(delta);
                }
                data["messageId"] = json!(run.message_id);
            }
            "usage" => {
                let usage = data.get("usage").unwrap_or(data);
                if !detail.usage.is_object() {
                    detail.usage = json!({});
                }
                for (target, source) in [
                    ("inputTokens", "input"),
                    ("outputTokens", "output"),
                    ("cacheReadTokens", "cacheRead"),
                    ("cacheWriteTokens", "cacheWrite"),
                ] {
                    detail.usage[target] = json!(detail.usage[target]
                        .as_u64()
                        .unwrap_or(0)
                        .saturating_add(usage[source].as_u64().unwrap_or(0)));
                }
                detail.usage["estimatedCostUsd"] = Value::Null;
                detail.usage["billedCostUsd"] = Value::Null;
            }
            "sources" => {
                if let Some(sources) = data["sources"].as_array() {
                    message.sources = sources.clone();
                }
            }
            _ => {}
        }
        detail.session.updated_at = now();
        Ok(())
    })
}

fn merge_native(detail: &mut Detail, entries: &Value, secret: &str) -> Result<(), String> {
    let entries = entries.as_array().ok_or("assistant_runtime_protocol")?;
    for entry in entries {
        let Some(native_id) = entry["id"].as_str() else {
            continue;
        };
        let Some(role) = entry["role"]
            .as_str()
            .filter(|r| matches!(*r, "user" | "assistant"))
        else {
            continue;
        };
        let run_id = entry["runId"]
            .as_str()
            .ok_or("assistant_native_run_missing")?;
        if !detail
            .messages
            .iter()
            .any(|m| m.run_id.as_deref() == Some(run_id))
        {
            return Err("assistant_native_unknown_run".into());
        }
        let existing = detail
            .messages
            .iter()
            .position(|m| m.native_entry_id.as_deref() == Some(native_id));
        let final_message = role == "user" || entry["final"] != false;
        let target = existing.or_else(|| {
            final_message
                .then(|| {
                    detail.messages.iter().position(|m| {
                        m.run_id.as_deref() == Some(run_id)
                            && m.role == role
                            && m.native_entry_id.is_none()
                    })
                })
                .flatten()
        });
        let clean = |value: &str| {
            let text = crate::redact::scrub(value).into_owned();
            if secret.is_empty() {
                text
            } else {
                text.replace(secret, crate::redact::REDACTED)
            }
        };
        let content = clean(entry["content"].as_str().unwrap_or(""));
        let thinking = entry["thinking"].as_str().map(clean);
        let status = if entry["stopReason"] == "error" {
            "error"
        } else if entry["stopReason"] == "aborted" {
            "stopped"
        } else {
            "completed"
        };
        if let Some(index) = target {
            let m = &mut detail.messages[index];
            m.native_entry_id = Some(native_id.into());
            m.superseded = entry["active"] == false;
            if role == "assistant" {
                m.content = content;
            }
            m.reasoning = thinking;
            m.status = status.into();
            if let Some(timestamp) = entry["timestamp"].as_i64() {
                m.created_at = timestamp;
            }
        } else {
            detail.messages.push(Message {
                id: id(),
                role: role.into(),
                content,
                created_at: entry["timestamp"].as_i64().unwrap_or_else(now),
                reasoning: thinking,
                status: status.into(),
                attachments: vec![],
                sources: vec![],
                superseded: entry["active"] == false,
                revision_of: None,
                native_entry_id: Some(native_id.into()),
                run_id: Some(run_id.into()),
            });
        }
    }
    // A native assistant entry is authoritative even when the final UI event was lost.
    for message in &mut detail.messages {
        if message.status == "streaming" {
            message.status = "stopped".into();
        }
    }
    detail.messages.sort_by_key(|m| m.created_at);
    project_usage(detail, entries);
    Ok(())
}

fn project_usage(detail: &mut Detail, entries: &[Value]) {
    let mut input = 0u64;
    let mut output = 0u64;
    let mut cache_read = 0u64;
    let mut cache_write = 0u64;
    let mut estimate = 0.0;
    let mut priced = detail.usage["usageCoverage"] != "incomplete";
    let mut count = 0u64;
    for entry in entries
        .iter()
        .filter(|e| e["role"] == "assistant" || e["role"] == "usage")
    {
        let usage = &entry["usage"];
        if entry["usageCoverage"] == "incomplete" {
            priced = false;
            detail.usage["usageCoverage"] = json!("incomplete");
        }
        if !usage.is_object() {
            priced = false;
            detail.usage["usageCoverage"] = json!("incomplete");
            continue;
        }
        count += 1;
        input += usage["input"].as_u64().unwrap_or(0);
        output += usage["output"].as_u64().unwrap_or(0);
        cache_read += usage["cacheRead"].as_u64().unwrap_or(0);
        cache_write += usage["cacheWrite"].as_u64().unwrap_or(0);
        if entry["pricingKnown"] == true && entry["usageCoverage"] != "incomplete" {
            estimate += usage["cost"]["total"].as_f64().unwrap_or(0.0);
        } else {
            priced = false;
        }
    }
    if !detail.usage.is_object() {
        detail.usage = json!({});
    }
    detail.usage["inputTokens"] = json!(input);
    detail.usage["outputTokens"] = json!(output);
    detail.usage["cacheReadTokens"] = json!(cache_read);
    detail.usage["cacheWriteTokens"] = json!(cache_write);
    detail.usage["estimatedCostUsd"] = if priced && count > 0 {
        json!(estimate.to_string())
    } else {
        Value::Null
    };
    detail.usage["billedCostUsd"] = Value::Null;
    detail.usage["modelRequests"] = json!(count);
}

fn apply_native(
    db: &crate::database::Database,
    run: &RunControl,
    entries: &Value,
) -> Result<(), String> {
    store::mutate(db, &run.identity.owner, &run.session_id, |detail| {
        merge_native(detail, entries, &run.secret)
    })
}

pub async fn run(app: tauri::AppHandle, control: Arc<RunControl>, params: Value) {
    let host = app.state::<AssistantHost>();
    let native_lock = host.native_lock(&grant_key(&control.identity.owner, &control.session_id));
    let result = match native_lock {
        Ok(lock) => {
            let _guard = lock.lock_owned().await;
            run_inner(&app, &control, params).await
        }
        Err(e) => Err(e),
    };
    if result.is_err() {
        if let Ok(mut recovered) = host.recovered.lock() {
            recovered.remove(&grant_key(&control.identity.owner, &control.session_id));
        }
    }
    let state = app.state::<crate::store::AppState>();
    let host = app.state::<AssistantHost>();
    let cancelled = *control.cancel.borrow();
    let _ = store::mutate(
        &state.db,
        &control.identity.owner,
        &control.session_id,
        |detail| {
            if let Some(message) = detail
                .messages
                .iter_mut()
                .find(|m| m.id == control.message_id)
            {
                message.status = if cancelled {
                    "stopped"
                } else if result.is_ok() && message.status != "error" {
                    "completed"
                } else {
                    "error"
                }
                .into();
            }
            detail.session.active_run = None;
            if result.is_err() || cancelled {
                detail.usage["usageCoverage"] = json!("incomplete");
                detail.usage["estimatedCostUsd"] = Value::Null;
            }
            for op in &mut detail.operations {
                if op["runId"] == control.run_id && op["status"] == "running" {
                    op["status"] = json!(if op["toolName"] == "config_status" {
                        "cancelled"
                    } else {
                        "unknown"
                    });
                }
                if op["id"] == control.operation_id {
                    op["status"] = json!(if cancelled {
                        "cancelled"
                    } else if result.is_ok() {
                        "completed"
                    } else {
                        "failed"
                    });
                }
            }
            Ok(())
        },
    );
    if let Ok(mut runs) = host.runs.lock() {
        if runs
            .get(&control.session_id)
            .is_some_and(|r| r.run_id == control.run_id)
        {
            runs.remove(&control.session_id);
        }
    }
    if let Err(error) = result {
        if !cancelled || error.contains("outcome_unknown") {
            emit(&app, &control, "error", json!({"code":error}));
        }
    }
    emit(&app, &control, "run_end", json!({"stopped":cancelled}));
    if let Ok(mut tasks) = host.tasks.lock() {
        tasks.remove(&control.run_id);
    };
}

async fn run_inner(
    app: &tauri::AppHandle,
    control: &Arc<RunControl>,
    params: Value,
) -> Result<(), String> {
    use tokio::io::{AsyncWriteExt, BufReader};
    app.state::<AssistantHost>().validate_run(control)?;
    let (node, entry) = runtime_paths(app)?;
    let mut command = tokio::process::Command::new(node);
    command.arg("--max-old-space-size=512").arg(entry);
    process::clean_environment(&mut command);
    let mut child = process::Child::spawn(command)?;
    let mut input = child.take_stdin().ok_or("assistant_runtime_stdio")?;
    let output = child.take_stdout().ok_or("assistant_runtime_stdio")?;
    let stderr = child.take_stderr().ok_or("assistant_runtime_stdio")?;
    // Drain, but never publish stderr: dependencies may include request headers.
    let stderr_task = tokio::spawn(async move {
        let _ = process::bounded_output(stderr, 64 * 1024).await;
    });
    app.state::<AssistantHost>().validate_run(control)?;
    if identity(app).await? != control.identity {
        return Err("assistant_identity_changed".into());
    }
    let request_id = id();
    let mut line = serde_json::to_vec(
        &json!({"jsonrpc":"2.0","id":request_id,"method":"run","params":params}),
    )
    .map_err(|_| "assistant_runtime_protocol")?;
    line.push(b'\n');
    let mut cancel = control.cancel.subscribe();
    app.state::<AssistantHost>().validate_run(control)?;
    tokio::select! {
        biased;
        _=cancel.changed()=>return Err("assistant_cancelled".into()),
        result=input.write_all(&line)=>result.map_err(|_|"assistant_runtime_stdio")?,
    }
    input.flush().await.map_err(|_| "assistant_runtime_stdio")?;
    let mut output = BufReader::new(output);
    let mut buffer = Vec::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    let end = control.deadline;
    let mut projection = ProjectionBuffer::default();
    let result=async {
        loop {
            let length=tokio::select! {
                _=cancel.changed()=>return Err("assistant_cancelled".into()),
                _=ticker.tick()=>{if projection.due() { projection.flush(&app.state::<crate::store::AppState>().db, control)?; }
                    if identity(app).await?!=control.identity {return Err("assistant_identity_changed".into());} app.state::<AssistantHost>().validate_run(control)?; if tokio::time::Instant::now()>end {return Err("assistant_run_timeout".into());} continue;},
                result=read_frame(&mut output,&mut buffer)=>result?,
            };
            if length==0 { return Err("assistant_runtime_exited".into()); }
            if buffer.len()>8*1024*1024 {return Err("assistant_runtime_protocol_limit".into());}
            let value:Value=serde_json::from_slice(&buffer).map_err(|_|"assistant_runtime_protocol")?; buffer.clear();
            if value["method"]=="event" {
                let params=&value["params"];
                if params["sessionId"]!=control.session_id || params["runId"]!=control.run_id || params["operationId"]!=control.operation_id { return Err("assistant_runtime_identity_mismatch".into()); }
                let kind=params["type"].as_str().ok_or("assistant_runtime_protocol")?;
                let mut data=params["data"].clone();
                if !data.is_object() { return Err("assistant_runtime_protocol".into()); }
                let state=app.state::<crate::store::AppState>();
                if matches!(kind, "text_delta" | "thinking_delta") {
                    projection.push(control, kind, &mut data);
                    if projection.due() { projection.flush(&state.db, control)?; }
                } else if matches!(kind, "usage" | "sources") { event_projection(&state.db,control,kind,&mut data)?; }
                if kind != "run_end" && kind != "message_end" { emit(app,control,kind,data); }
            } else if value["method"]=="tool" {
                let params=&value["params"];
                if params["sessionId"]!=control.session_id || params["runId"]!=control.run_id || params["operationId"]!=control.operation_id {return Err("assistant_runtime_identity_mismatch".into());}
                let name=params["name"].as_str().ok_or("assistant_invalid_tool")?;
                app.state::<AssistantHost>().validate_run(control)?;
                let response=tokio::select! {
                    response=super::tools::execute(app,control,name,params["args"].clone(),params["toolCallId"].as_str().ok_or("assistant_invalid_tool_call_id")?)=>response,
                    _=cancel.changed()=>return Err("assistant_cancelled".into()),
                    _=tokio::time::sleep_until(end)=>{let _=control.cancel.send_replace(true);return Err("assistant_operation_outcome_unknown".into());}
                };
                let mut result=match response {Ok(value)=>json!({"jsonrpc":"2.0","result":value}),Err(error)=>{let unknown=error.contains("outcome_unknown");if unknown {control.outcome_unknown.store(true,Ordering::SeqCst);}json!({"jsonrpc":"2.0","error":{"code":if unknown {json!("unknown_outcome")} else {json!(-32000)},"message":error,"data":{"outcomeUnknown":unknown}}})}};
                result["id"]=value["id"].clone(); store::scrub_value(&mut result,&[&control.secret]);
                let mut bytes=serde_json::to_vec(&result).map_err(|_|"assistant_runtime_protocol")?;bytes.push(b'\n');
                input.write_all(&bytes).await.map_err(|_|"assistant_runtime_stdio")?;input.flush().await.map_err(|_|"assistant_runtime_stdio")?;
            } else if value["id"]==request_id {
                if value.get("error").is_some() { return Err(value["error"]["message"].as_str().unwrap_or("assistant_runtime_failed").to_string()); }
                projection.flush(&app.state::<crate::store::AppState>().db, control)?;
                apply_native(&app.state::<crate::store::AppState>().db,control,&value["result"]["entries"])?;
                if value["result"]["status"]=="error" {return Err(format!("assistant_model_request_failed: {}",value["result"]["errorMessage"].as_str().unwrap_or("Request failed")));}
                return Ok(());
            }
        }
    }.await;
    let flushed = projection.flush(&app.state::<crate::store::AppState>().db, control);
    let result = result.and(flushed);
    if result.is_err() && (*control.cancel.borrow() || tokio::time::Instant::now() >= end) {
        // Ask the SDK to persist its interrupted turn before enforcing the process deadline.
        // No further host tool may execute during this bounded shutdown.
        let _=tokio::time::timeout(Duration::from_secs(2),async {
            let mut stop=serde_json::to_vec(&json!({"jsonrpc":"2.0","id":"shutdown","method":"stop","params":{"sessionId":control.session_id,"runId":control.run_id,"operationId":control.operation_id}})).map_err(|_|"assistant_runtime_protocol")?;stop.push(b'\n');
            input.write_all(&stop).await.map_err(|_|"assistant_runtime_stdio")?;input.flush().await.map_err(|_|"assistant_runtime_stdio")?;
            loop {
                if read_frame(&mut output,&mut buffer).await?==0 {break;}
                let value:Value=serde_json::from_slice(&buffer).map_err(|_|"assistant_runtime_protocol")?;buffer.clear();
                if value["id"]==request_id {
                    if value.get("error").is_none() {apply_native(&app.state::<crate::store::AppState>().db,control,&value["result"]["entries"])?;}
                    break;
                }
                if value["method"]=="tool" {
                    let mut response=serde_json::to_vec(&json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":"unknown_outcome","message":"assistant_cancelled","data":{"outcomeUnknown":true}}})).map_err(|_|"assistant_runtime_protocol")?;response.push(b'\n');input.write_all(&response).await.map_err(|_|"assistant_runtime_stdio")?;
                }
            } Ok::<(),String>(())
        }).await;
    }
    child.stop().await;
    stderr_task.abort();
    result
}

async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> Result<usize, String> {
    use tokio::io::AsyncBufReadExt;
    loop {
        let bytes = reader
            .fill_buf()
            .await
            .map_err(|_| "assistant_runtime_stdio")?;
        if bytes.is_empty() {
            return Ok(0);
        }
        let count = bytes
            .iter()
            .position(|v| *v == b'\n')
            .map_or(bytes.len(), |i| i + 1);
        if buffer.len() + count > 8 * 1024 * 1024 {
            return Err("assistant_runtime_protocol_limit".into());
        }
        let finished = bytes[count - 1] == b'\n';
        buffer.extend_from_slice(&bytes[..count]);
        reader.consume(count);
        if finished {
            return Ok(buffer.len());
        }
    }
}

async fn load_native(app: &tauri::AppHandle, file: &Path) -> Result<Value, String> {
    use tokio::io::{AsyncWriteExt, BufReader};
    let (node, entry) = runtime_paths(app)?;
    let mut command = tokio::process::Command::new(node);
    command.arg("--max-old-space-size=512").arg(entry);
    process::clean_environment(&mut command);
    let mut child = process::Child::spawn(command)?;
    let mut input = child.take_stdin().ok_or("assistant_runtime_stdio")?;
    let mut output = BufReader::new(child.take_stdout().ok_or("assistant_runtime_stdio")?);
    let stderr = child.take_stderr().ok_or("assistant_runtime_stdio")?;
    let drain = tokio::spawn(async move {
        let _ = process::bounded_output(stderr, 64 * 1024).await;
    });
    let mut request = serde_json::to_vec(
        &json!({"jsonrpc":"2.0","id":"load","method":"load","params":{"sessionFile":file}}),
    )
    .map_err(|_| "assistant_runtime_protocol")?;
    request.push(b'\n');
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        input
            .write_all(&request)
            .await
            .map_err(|_| "assistant_runtime_stdio")?;
        input.flush().await.map_err(|_| "assistant_runtime_stdio")?;
        let mut bytes = vec![];
        read_frame(&mut output, &mut bytes).await?;
        let response: Value =
            serde_json::from_slice(&bytes).map_err(|_| "assistant_runtime_protocol")?;
        if response["id"] != "load" || response.get("error").is_some() {
            return Err("assistant_history_recovery_failed".into());
        }
        Ok(response["result"].clone())
    })
    .await
    .map_err(|_| "assistant_history_recovery_timeout".to_string())
    .and_then(|v| v);
    child.stop().await;
    drain.abort();
    result
}

pub fn ensure_regular(path: &Path) -> Result<(), String> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("assistant_invalid_storage_path".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admission_fixture_run(host: &AssistantHost, owner: &str, sid: &str) -> Arc<RunControl> {
        let (cancel, _) = watch::channel(false);
        Arc::new(RunControl {
            identity: Identity {
                owner: owner.into(),
                apex: "ofox.ai".into(),
            },
            session_id: sid.into(),
            run_id: id(),
            operation_id: id(),
            message_id: id(),
            cancel,
            epoch: host.current_epoch(owner, sid).unwrap(),
            deadline: tokio::time::Instant::now() + Duration::from_secs(900),
            outcome_unknown: AtomicBool::new(false),
            approvals: Default::default(),
            secret: "test-admission-token".into(),
            search_capabilities: json!({}),
        })
    }

    async fn queued_registration_after_lifecycle_start(shutdown: bool) {
        let host = Arc::new(AssistantHost::default());
        let db = Arc::new(crate::database::Database::memory().unwrap());
        let owner = store::namespace("test-user", "test-org").unwrap();
        let detail =
            store::create(&db, &owner, AssistantKind::General, "test/model", false).unwrap();
        let sid = detail.session.id;
        let calls = Arc::new(AtomicU64::new(0));
        let (release, queued) = oneshot::channel();
        let worker_host = host.clone();
        let worker_db = db.clone();
        let worker_owner = owner.clone();
        let worker_sid = sid.clone();
        let worker_calls = calls.clone();
        let worker = tokio::spawn(async move {
            queued.await.map_err(|_| "test_queue_closed".to_string())?;
            // The queued IPC only enters send after revocation, so it observes
            // the new generation. Generation equality alone cannot reject it.
            let generation = worker_host.generation.load(Ordering::SeqCst);
            let run = admission_fixture_run(&worker_host, &worker_owner, &worker_sid);
            worker_host.register_run(run.clone(), generation, || {
                store::mutate(&worker_db, &worker_owner, &worker_sid, |detail| {
                    detail
                        .operations
                        .push(json!({"id":run.operation_id,"status":"running"}));
                    Ok(())
                })
            })?;
            worker_host.validate_run(&run)?;
            worker_calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(())
        });
        if shutdown {
            host.begin_shutdown();
        } else {
            let _ = host.begin_logout();
        }
        release.send(()).unwrap();
        let result = worker.await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "queued work must not reach the provider after lifecycle revocation begins"
        );
        assert!(result.is_err(), "queued work must fail before registration");
        assert!(host.runs.lock().unwrap().is_empty());
        let detail = store::get(&db, &owner, &sid).unwrap();
        assert!(detail.operations.is_empty());
        assert!(detail.messages.is_empty());
        assert_eq!(detail.usage, json!({}));
    }

    #[tokio::test]
    async fn queued_send_cannot_enter_after_logout_begins() {
        queued_registration_after_lifecycle_start(false).await;
    }

    #[tokio::test]
    async fn queued_send_cannot_enter_after_shutdown_begins() {
        queued_registration_after_lifecycle_start(true).await;
    }

    #[test]
    fn authorization_reopens_only_completed_logout_and_never_shutdown() {
        let host = AssistantHost::default();
        let first = host.begin_logout();
        let second = host.begin_logout();
        host.finish_logout(first);
        host.authorization_succeeded();
        assert!(
            host.ensure_accepting_runs().is_err(),
            "another logout is still revoking credentials"
        );
        host.finish_logout(second);
        assert!(
            host.ensure_accepting_runs().is_err(),
            "finishing logout is not a new authorization"
        );
        host.authorization_succeeded();
        assert!(host.ensure_accepting_runs().is_ok());
        host.begin_shutdown();
        host.authorization_succeeded();
        assert!(
            host.ensure_accepting_runs().is_err(),
            "a late auth response must never reopen app shutdown"
        );
    }

    #[test]
    fn ordinary_identity_cancellation_keeps_new_authority_admissible() {
        let host = AssistantHost::default();
        host.cancel_all();
        assert!(host.ensure_accepting_runs().is_ok());
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn terminal_exit_waits_for_owned_processes_and_preserves_unrelated_sentinel() {
        use std::os::unix::process::CommandExt;
        use std::process::{Child, Command, Stdio};

        struct Fixture {
            host: Arc<AssistantHost>,
            sentinel: Child,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                self.host.cancel_all();
                let tasks: Vec<_> = self
                    .host
                    .tasks
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .drain()
                    .map(|(_, task)| task)
                    .collect();
                for task in &tasks {
                    task.abort();
                }
                tauri::async_runtime::block_on(async {
                    for task in tasks {
                        let _ = tokio::time::timeout(Duration::from_secs(3), task).await;
                    }
                });
                // The retained Child identifies only this test's sentinel.
                if matches!(self.sentinel.try_wait(), Ok(None)) {
                    let _ = self.sentinel.kill();
                }
                let _ = self.sentinel.wait();
            }
        }
        fn running(pid: u32) -> bool {
            let output = Command::new("/bin/ps")
                .args(["-p", &pid.to_string(), "-o", "stat="])
                .env_clear()
                .output()
                .unwrap();
            let status = String::from_utf8(output.stdout).unwrap();
            if output.status.code() == Some(1) && status.trim().is_empty() {
                return false;
            }
            assert!(output.status.success(), "known-PID ps lookup failed");
            let status = status.trim();
            !status.is_empty() && !status.starts_with('Z') && !status.starts_with('X')
        }

        let sandbox = crate::test_sandbox::sandbox();
        let host = Arc::new(AssistantHost::default());
        let sentinel = Command::new("/bin/sleep")
            .arg("60")
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut fixture = Fixture {
            host: host.clone(),
            sentinel,
        };
        let parent_file = sandbox.home().join("terminal-parent.pid");
        let child_file = sandbox.home().join("terminal-child.pid");
        let owner = store::namespace("terminal-user", "test-org").unwrap();
        let sid = id();
        let control = admission_fixture_run(&host, &owner, &sid);
        let cancelled = control.cancel.subscribe();
        host.register_run(
            control.clone(),
            host.generation.load(Ordering::SeqCst),
            || Ok(()),
        )
        .unwrap();
        let mut command = tokio::process::Command::new("/bin/sh");
        process::clean_environment(&mut command);
        command
            .args([
                "-c",
                "printf '%s\\n' \"$$\" > \"$PARENT_PID_FILE\"; /bin/sleep 60 & printf '%s\\n' \"$!\" > \"$CHILD_PID_FILE\"; wait",
            ])
            .env("PARENT_PID_FILE", &parent_file)
            .env("CHILD_PID_FILE", &child_file)
            .current_dir(sandbox.home());
        let worker_host = host.clone();
        let worker_sid = sid.clone();
        let completed = Arc::new(AtomicBool::new(false));
        let worker_completed = completed.clone();
        let task = tauri::async_runtime::spawn(async move {
            let _ =
                process::run(command, control.cancel.subscribe(), Duration::from_secs(60)).await;
            // Process termination alone does not prove the terminal callback
            // waited for the run's remaining persistence/cleanup work.
            tokio::time::sleep(Duration::from_millis(250)).await;
            worker_completed.store(true, Ordering::SeqCst);
            worker_host.runs.lock().unwrap().remove(&worker_sid);
        });
        host.tasks.lock().unwrap().insert(sid, task);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (parent, child) = loop {
            let read_pid = |path: &Path| {
                std::fs::read_to_string(path)
                    .ok()?
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|pid| *pid > 1)
            };
            if let (Some(parent), Some(child)) = (read_pid(&parent_file), read_pid(&child_file)) {
                break (parent, child);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "fixture did not start"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(running(parent) && running(child));
        let group = unsafe { libc::getpgid(parent as i32) };
        assert_eq!(group, parent as i32);
        assert_eq!(unsafe { libc::getpgid(child as i32) }, group);
        assert_ne!(
            unsafe { libc::getpgid(fixture.sentinel.id() as i32) },
            group
        );

        let generation = host.generation.load(Ordering::SeqCst);
        assert!(!host.handle_terminal_exit(&tauri::RunEvent::Ready));
        assert_eq!(host.generation.load(Ordering::SeqCst), generation);
        assert!(!*cancelled.borrow());
        assert!(host.ensure_accepting_runs().is_ok());
        assert!(running(parent) && running(child));
        assert!(host.handle_terminal_exit(&tauri::RunEvent::Exit));
        assert!(completed.load(Ordering::SeqCst));
        assert!(host.runs.lock().unwrap().is_empty());
        // Observe before Fixture::drop so teardown cannot make a broken
        // terminal handler appear successful.
        assert!(!running(parent) && !running(child));
        assert!(fixture.sentinel.try_wait().unwrap().is_none());
        assert_eq!(
            host.ensure_accepting_runs().unwrap_err(),
            "assistant_shutting_down"
        );
        host.authorization_succeeded();
        assert!(host.ensure_accepting_runs().is_err());
    }

    #[cfg(windows)]
    #[test]
    #[serial_test::serial]
    fn windows_terminal_exit_waits_for_owned_job_and_preserves_unrelated_sentinel() {
        let sandbox = crate::test_sandbox::sandbox();
        let mut fixture = process::windows_fixture::Fixture::new(sandbox.home());
        let host = Arc::new(AssistantHost::default());
        let owner = store::namespace("terminal-windows-user", "test-org").unwrap();
        let sid = id();
        let control = admission_fixture_run(&host, &owner, &sid);
        let cancelled = control.cancel.subscribe();
        let generation = host.generation.load(Ordering::SeqCst);
        host.register_run(control.clone(), generation, || Ok(()))
            .unwrap();
        let command = fixture.command();
        let worker_host = host.clone();
        let worker_sid = sid.clone();
        let completed = Arc::new(AtomicBool::new(false));
        let worker_completed = completed.clone();
        let task = tauri::async_runtime::spawn(async move {
            let result =
                process::run(command, control.cancel.subscribe(), Duration::from_secs(60)).await;
            // Also cover the interval after Job termination but before the
            // run has finished its persistence/cleanup work.
            tokio::time::sleep(Duration::from_millis(250)).await;
            if result
                .as_ref()
                .is_err_and(|error| error == "assistant_cancelled")
            {
                worker_completed.store(true, Ordering::SeqCst);
            }
            worker_host.runs.lock().unwrap().remove(&worker_sid);
        });
        fixture.set_abort(task.inner().abort_handle());
        host.tasks.lock().unwrap().insert(sid, task);
        fixture.observe_owned();
        assert!(!host.handle_terminal_exit(&tauri::RunEvent::Ready));
        assert_eq!(host.generation.load(Ordering::SeqCst), generation);
        assert!(!*cancelled.borrow());
        assert!(host.ensure_accepting_runs().is_ok());
        fixture.assert_owned_alive();

        assert!(host.handle_terminal_exit(&tauri::RunEvent::Exit));
        assert!(completed.load(Ordering::SeqCst));
        assert!(host.runs.lock().unwrap().is_empty());
        // These assertions precede the fixture's failure-cleanup Drop.
        fixture.assert_owned_stopped();
        assert_eq!(
            host.ensure_accepting_runs().unwrap_err(),
            "assistant_shutting_down"
        );
        host.authorization_succeeded();
        assert!(host.ensure_accepting_runs().is_err());
    }

    #[test]
    fn streaming_projection_batches_writes_and_flushes_the_last_text_and_reasoning() {
        let db = crate::database::Database::memory().unwrap();
        let owner = store::namespace("stream-user", "org").unwrap();
        let session = store::create(&db, &owner, AssistantKind::General, "test", false).unwrap();
        let sid = session.session.id;
        let mid = id();
        store::mutate(&db,&owner,&sid,|d|{d.messages.push(serde_json::from_value(json!({"id":mid,"role":"assistant","content":"","createdAt":1,"status":"streaming"})).unwrap());Ok(())}).unwrap();
        let (cancel, _) = watch::channel(false);
        let run = RunControl {
            identity: Identity {
                owner: owner.clone(),
                apex: "ofox.ai".into(),
            },
            session_id: sid.clone(),
            message_id: mid.clone(),
            run_id: id(),
            operation_id: id(),
            cancel,
            epoch: 0,
            deadline: tokio::time::Instant::now() + Duration::from_secs(900),
            outcome_unknown: AtomicBool::new(false),
            approvals: Default::default(),
            secret: "test-opaque-credential".into(),
            search_capabilities: json!({}),
        };
        let changes = || {
            db.conn
                .lock()
                .unwrap()
                .query_row("SELECT total_changes()", [], |row| row.get::<_, u64>(0))
                .unwrap()
        };
        let before = changes();
        let mut buffer = ProjectionBuffer::default();
        for _ in 0..10_000 {
            let mut data = json!({"delta":"x"});
            buffer.push(&run, "text_delta", &mut data);
            assert_eq!(data["messageId"], mid);
        }
        assert_eq!(changes(), before);
        buffer.last_flush -= Duration::from_millis(500);
        assert!(buffer.due());
        buffer.flush(&db, &run).unwrap();
        assert_eq!(changes(), before + 1);
        let mut delta = json!({"delta":"last"});
        buffer.push(&run, "text_delta", &mut delta);
        buffer.push(&run, "thinking_delta", &mut json!({"delta":"reasoning"}));
        buffer.flush(&db, &run).unwrap();
        buffer.flush(&db, &run).unwrap();
        assert_eq!(changes(), before + 2);
        let d = store::get(&db, &owner, &sid).unwrap();
        assert_eq!(d.messages[0].content, format!("{}last", "x".repeat(10_000)));
        assert_eq!(d.messages[0].reasoning.as_deref(), Some("reasoning"));
    }

    #[test]
    fn compaction_usage_is_counted_and_unknown_coverage_stays_unknown() {
        let db = crate::database::Database::memory().unwrap();
        let mut detail = store::create(
            &db,
            &store::namespace("a", "b").unwrap(),
            AssistantKind::General,
            "test/model",
            false,
        )
        .unwrap();
        let entries = vec![
            json!({"role":"assistant","pricingKnown":true,"usage":{"input":10,"output":5,"cost":{"total":0}}}),
            json!({"role":"usage","pricingKnown":true,"usageCoverage":"complete","usage":{"input":20,"output":3,"cost":{"total":0.1}}}),
        ];
        project_usage(&mut detail, &entries);
        assert_eq!(detail.usage["inputTokens"], 30);
        assert_eq!(detail.usage["outputTokens"], 8);
        assert_eq!(detail.usage["estimatedCostUsd"], "0.1");
        project_usage(
            &mut detail,
            &[json!({"role":"usage","usage":null,"usageCoverage":"incomplete"})],
        );
        assert!(detail.usage["estimatedCostUsd"].is_null());
        project_usage(&mut detail, &entries);
        assert!(
            detail.usage["estimatedCostUsd"].is_null(),
            "a later complete response cannot erase an unaccounted interrupted request"
        );
    }

    #[tokio::test]
    async fn live_history_read_never_waits_for_a_running_turns_native_lock() {
        let host = AssistantHost::default();
        let owner = store::namespace("test-user", "org").unwrap();
        let sid = id();
        let (cancel, _) = watch::channel(false);
        let run = Arc::new(RunControl {
            identity: Identity {
                owner: owner.clone(),
                apex: "ofox.ai".into(),
            },
            session_id: sid.clone(),
            run_id: id(),
            operation_id: id(),
            message_id: id(),
            cancel,
            epoch: 0,
            deadline: tokio::time::Instant::now() + Duration::from_secs(900),
            outcome_unknown: AtomicBool::new(false),
            approvals: Default::default(),
            secret: "test-token".into(),
            search_capabilities: json!({}),
        });
        let _running = host
            .native_lock(&grant_key(&owner, &sid))
            .unwrap()
            .lock_owned()
            .await;
        host.runs.lock().unwrap().insert(sid.clone(), run);
        let result = tokio::time::timeout(
            Duration::from_millis(30),
            host.acquire_recovery_lock(&owner, &sid),
        )
        .await;
        assert!(
            result.is_ok(),
            "reading a running conversation must not wait for its native writer"
        );
        assert!(result.unwrap().unwrap().is_none());
    }

    #[test]
    fn immediate_stop_is_retained_before_the_sidecar_subscribes() {
        let host = AssistantHost::default();
        let (cancel, receiver) = watch::channel(false);
        drop(receiver);
        let sid = id();
        let control = Arc::new(RunControl {
            identity: Identity {
                owner: store::namespace("a", "b").unwrap(),
                apex: "ofox.ai".into(),
            },
            session_id: sid.clone(),
            run_id: id(),
            operation_id: id(),
            message_id: id(),
            cancel,
            epoch: 0,
            deadline: tokio::time::Instant::now() + Duration::from_secs(900),
            outcome_unknown: AtomicBool::new(false),
            approvals: Default::default(),
            secret: "fixture-token".into(),
            search_capabilities: json!({}),
        });
        host.runs
            .lock()
            .unwrap()
            .insert(sid.clone(), control.clone());
        host.cancel_session(&sid);
        assert!(
            *control.cancel.subscribe().borrow(),
            "stop must survive without an attached receiver"
        );
        assert!(host.validate_run(&control).is_err());
    }

    #[test]
    fn stop_is_retained_before_the_run_registers() {
        let host = AssistantHost::default();
        let db = crate::database::Database::memory().unwrap();
        let identity = Identity {
            owner: store::namespace("a", "b").unwrap(),
            apex: "ofox.ai".into(),
        };
        let detail = store::create(
            &db,
            &identity.owner,
            AssistantKind::General,
            "test/model",
            false,
        )
        .unwrap();
        let (sid, rid, op) = (detail.session.id, id(), id());
        host.request_stop(&db, &identity, &sid, &rid, &op).unwrap();
        host.request_stop(&db, &identity, &sid, &rid, &op).unwrap();
        let stopped = store::get(&db, &identity.owner, &sid).unwrap();
        assert_eq!(stopped.operations.len(), 1);
        assert_eq!(stopped.operations[0]["status"], "cancelled");
        assert_eq!(stopped.operations[0]["requestStarted"], false);
        assert!(stopped.messages.is_empty());
        assert_eq!(
            store::ensure_new_operation(&stopped, &op).unwrap_err(),
            "assistant_cancelled"
        );
        let other = Identity {
            owner: store::namespace("a", "other-org").unwrap(),
            apex: "ofox.ai".into(),
        };
        assert!(host.request_stop(&db, &other, &sid, &rid, &op).is_err());
    }

    #[test]
    fn native_recovery_binds_each_run_instead_of_claiming_previous_turns() {
        let db = crate::database::Database::memory().unwrap();
        let owner = store::namespace("test-user", "test-org").unwrap();
        let detail =
            store::create(&db, &owner, AssistantKind::General, "test/model", false).unwrap();
        let sid = detail.session.id;
        store::mutate(&db,&owner,&sid,|d|{
            for (mid,role,run) in [("u1","user","run1"),("a1","assistant","run1"),("u2","user","run2"),("a2","assistant","run2")] {
                d.messages.push(serde_json::from_value(json!({"id":mid,"role":role,"content":"partial","createdAt":1,"status":"streaming","runId":run})).unwrap());
            } Ok(())
        }).unwrap();
        let (cancel, _) = watch::channel(false);
        let run = RunControl {
            identity: Identity {
                owner: owner.clone(),
                apex: "ofox.ai".into(),
            },
            session_id: sid.clone(),
            run_id: "run2".into(),
            operation_id: id(),
            message_id: "a2".into(),
            cancel,
            epoch: 0,
            deadline: tokio::time::Instant::now() + Duration::from_secs(900),
            outcome_unknown: AtomicBool::new(false),
            approvals: Default::default(),
            secret: "test-opaque-token".into(),
            search_capabilities: json!({}),
        };
        apply_native(&db,&run,&json!([
            {"id":"native-u1","role":"user","runId":"run1","content":"old question","active":true},
            {"id":"native-a1","role":"assistant","runId":"run1","content":"old answer","active":true},
            {"id":"native-u2","role":"user","runId":"run2","content":"new question","active":true},
            {"id":"native-a2","role":"assistant","runId":"run2","content":"new answer","active":true}
        ])).unwrap();
        let d = store::get(&db, &owner, &sid).unwrap();
        assert_eq!(d.messages.len(), 4);
        assert_eq!(d.messages[0].native_entry_id.as_deref(), Some("native-u1"));
        assert_eq!(d.messages[3].native_entry_id.as_deref(), Some("native-a2"));
        assert_eq!(d.messages[3].content, "new answer");
    }
}
