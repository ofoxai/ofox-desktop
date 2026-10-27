//! The assistant host: who may start a run, the one runtime process each run
//! owns, and the projection of the runtime's native history into
//! `session.json`. The application side (sign-in, the assistant key, the model
//! catalog, where files live, where events go) comes in through
//! [`Environment`], so the host runs the same against the app and in tests.
use super::{
    catalog, process,
    projection::{
        add_usage, is_dangling, merge_native, save_projection, settle_interrupted, settle_run,
        ProjectionBuffer,
    },
    store::AssistantStore,
    types::*,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncWrite};
use tokio::sync::watch;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// Whether a run may still get a key; the error is the code to report.
pub type Admit<'a> = dyn Fn() -> Result<(), String> + Send + Sync + 'a;

pub(super) const STATE: &str = "assistant_state_unavailable";
pub const SIGN_IN_REQUIRED: &str = "assistant_sign_in_required";
pub(super) const IDENTITY_CHANGED: &str = "assistant_identity_changed";
const CANCELLED: &str = "assistant_cancelled";
pub const KEY_REJECTED: &str = "assistant_key_rejected";
const INSUFFICIENT_BALANCE: &str = "assistant_insufficient_balance";
const RATE_LIMITED: &str = "assistant_rate_limited";
pub(super) const MODEL_UNAVAILABLE: &str = "assistant_model_unavailable";
/// Internal: the first attempt's key was refused before anything was shown
/// or billed, so the run may retry with a new key. Never published.
const RETRY_WITH_NEW_KEY: &str = "assistant_retry_with_new_key";
const NETWORK_ERROR: &str = "assistant_network_error";
const MODEL_REQUEST_FAILED: &str = "assistant_model_request_failed";
const STDIO: &str = "assistant_runtime_stdio";
pub(super) const PROTOCOL: &str = "assistant_runtime_protocol";
const RUNTIME_UNAVAILABLE: &str = "assistant_runtime_unavailable";
const RUN_DEADLINE: Duration = Duration::from_secs(900);
/// How long a fetched model catalog is trusted.
const MODELS_TTL: Duration = Duration::from_secs(600);
const FRAME_LIMIT: usize = 8 * 1024 * 1024;

/// The signed-in account (its [`super::store::owner_key`]) and the region a
/// run is bound to. A run keeps the identity it started with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub owner: String,
    pub apex: String,
}

/// What the host needs from the application.
pub trait Environment: Send + Sync + 'static {
    /// The signed-in account and the current region; [`SIGN_IN_REQUIRED`]
    /// when nobody is signed in.
    fn identity(&self) -> BoxFuture<'_, Result<Identity, String>>;
    /// The assistant's own API key for `identity`'s account. `rejected` is a
    /// key the gateway has just refused: replace it, unless another run
    /// already did ([`KEY_REJECTED`] when it is too new to replace again).
    /// `admit` is asked again once the key lock is held, so a logout that
    /// revoked keys meanwhile gets no new one.
    fn api_key<'a>(
        &'a self,
        identity: &'a Identity,
        rejected: Option<&'a str>,
        admit: &'a Admit<'a>,
    ) -> BoxFuture<'a, Result<String, String>>;
    /// Whether a key for `identity` is ready, without minting one.
    fn key_ready<'a>(&'a self, identity: &'a Identity) -> BoxFuture<'a, bool>;
    /// The chat models offered in `apex`.
    fn models<'a>(&'a self, apex: &'a str) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>>;
    /// The LLM gateway base of `apex`.
    fn gateway_base(&self, apex: &str) -> Result<String, String>;
    /// The folder that holds `assistant/`.
    fn config_dir(&self) -> PathBuf;
    /// The prepared runtime: `node/`, `runtime/src/main.mjs`, `manifest.json`.
    fn runtime_dir(&self) -> Result<PathBuf, String>;
    /// Publish an `assistant:event`.
    fn emit(&self, event: Event);
}

pub(super) struct Runtime {
    pub node: PathBuf,
    pub entry: PathBuf,
    pub version: Option<String>,
}

pub struct RunControl {
    pub identity: Identity,
    pub session_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub message_id: String,
    pub cancel: watch::Sender<bool>,
    pub deadline: tokio::time::Instant,
    /// Every API key this run has used (a 401 retry adds the new one). Each is
    /// scrubbed from everything the run shows or saves.
    secret: Mutex<Vec<String>>,
}

impl RunControl {
    pub fn new(identity: Identity, request: &SendRequest, secret: String) -> Self {
        Self {
            identity,
            session_id: request.session_id.clone(),
            run_id: request.run_id.clone(),
            operation_id: request.operation_id.clone(),
            message_id: id(),
            cancel: watch::channel(false).0,
            deadline: tokio::time::Instant::now() + RUN_DEADLINE,
            secret: Mutex::new(vec![secret]),
        }
    }

    pub(super) fn secrets(&self) -> Vec<String> {
        self.secret
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn add_secret(&self, key: &str) {
        self.secret
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
    }

    fn cancelled(&self) -> bool {
        *self.cancel.borrow()
    }
}

/// Whether new runs may start. A logout closes admission until the next
/// sign-in; shutdown closes it for good.
#[derive(Default)]
struct Admission {
    logouts: HashSet<u64>,
    needs_sign_in: bool,
    shutting_down: bool,
}

pub struct AssistantHost {
    pub(super) env: Arc<dyn Environment>,
    /// Bumped by every revocation (logout, shutdown, identity or region
    /// change). A request that saw an older value must not write.
    generation: AtomicU64,
    admission: Mutex<Admission>,
    tasks: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    native_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    recovered: Mutex<HashSet<String>>,
    /// The live run of each session, by session id.
    runs: Mutex<HashMap<String, Arc<RunControl>>>,
    models: Mutex<HashMap<String, (std::time::Instant, Vec<catalog::Model>)>>,
    /// Every assistant key this process handed to a run: a draft or a title
    /// loses exactly these.
    keys: Mutex<Vec<String>>,
}

fn session_key(owner: &str, sid: &str) -> String {
    format!("{owner}:{sid}")
}

/// Run store IO (blocking file IO under blocking locks) off the async workers.
pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| STATE.to_string())?
}

/// `text` without any of `secrets`, the exact keys runs were given.
/// Conversation content (prompts, replies, drafts, titles, events) is
/// otherwise kept byte for byte: code, config samples and "password:"
/// prompts are the user's own words.
pub fn scrub_secrets(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_owned();
    for secret in secrets.iter().filter(|s| s.len() >= 8) {
        if text.contains(secret.as_str()) {
            text = text.replace(secret.as_str(), crate::redact::REDACTED);
        }
    }
    text
}

/// [`scrub_secrets`] on every string of `value`.
pub fn scrub_secrets_in(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => *text = scrub_secrets(text, secrets),
        Value::Array(items) => {
            for item in items {
                scrub_secrets_in(item, secrets);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                scrub_secrets_in(item, secrets);
            }
        }
        _ => {}
    }
}

/// Provider or runtime error text, a diagnostic: the keys, then anything that
/// looks like a credential. Never conversation content.
pub fn scrub_detail(text: &str, secrets: &[String]) -> String {
    crate::redact::scrub(&scrub_secrets(text, secrets)).into_owned()
}

/// The runtime may talk to a plain `http://localhost:<port>` gateway (the
/// `OFOX_USE_LOCAL` dev gateway, a test's mock) only in debug builds. A
/// release build always sends `allowLocalGateway: false`.
pub(super) fn local_gateway_allowed(base_url: &str) -> bool {
    cfg!(debug_assertions) && is_local_gateway(base_url)
}

fn is_local_gateway(base_url: &str) -> bool {
    let Some(port) = base_url.strip_prefix("http://localhost:") else {
        return false;
    };
    !port.is_empty()
        && port.len() <= 5
        && !port.starts_with('0')
        && port.bytes().all(|b| b.is_ascii_digit())
        && port.parse::<u32>().is_ok_and(|p| p <= 65_535)
}

/// The HTTP status at the head of the runtime's error text, in the forms the
/// Pi SDK gives for each protocol: `401: …` / `401 status code (no body)`
/// (Chat Completions), `401 {…}` (Anthropic Messages), `… API error (401): …`
/// (Responses). Used when the runtime gave no `errorStatus`.
fn leading_status(message: &str) -> Option<u16> {
    let message = message.trim_start();
    let digits: String = message.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() == 3 {
        return digits.parse().ok();
    }
    let (head, _) = message.split_once(':')?;
    let code = head.strip_suffix(')')?.rsplit_once("API error (")?.1;
    (code.len() == 3).then(|| code.parse().ok()).flatten()
}

/// Ofox's own error type for an empty balance, whatever the status.
fn is_ofox_balance_error(message: &str) -> bool {
    message
        .to_ascii_lowercase()
        .contains("insufficient_credits")
}

/// Generic balance wording; only trusted on a 403, never on a 429 or 5xx
/// where it can be an upstream provider's quota.
fn mentions_balance(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "insufficient credits",
        "insufficient_balance",
        "insufficient balance",
        "insufficient_quota",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn is_network_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "fetch failed",
        "connection error",
        "timed out",
        "econnrefused",
        "econnreset",
        "etimedout",
        "enotfound",
        "eai_again",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// A failed model request as a stable code. The runtime reports the HTTP
/// status (`errorStatus`) or that no answer came (`errorKind`: `not_sent` when
/// the request never left the machine, `network` when it was sent); older or
/// unforeseen forms fall back to the text, for the code only. The cost is a
/// known zero only for a request refused with a 4xx or never sent, and only
/// when the attempt had not already shown text or counted tokens.
fn provider_failure(result: &Value, detail: String, progressed: bool) -> Failure {
    let status = result["errorStatus"]
        .as_u64()
        .and_then(|s| u16::try_from(s).ok())
        .or_else(|| leading_status(&detail));
    let kind = &result["errorKind"];
    let not_sent = kind == "not_sent";
    let network = kind == "network"
        || (not_sent && result["errorCode"].is_string())
        || (status.is_none() && kind.is_null() && is_network_error(&detail));
    let code = match status {
        Some(401) => KEY_REJECTED,
        Some(402) => INSUFFICIENT_BALANCE,
        Some(404) => MODEL_UNAVAILABLE,
        Some(429) => RATE_LIMITED,
        Some(403) if mentions_balance(&detail) => INSUFFICIENT_BALANCE,
        _ if is_ofox_balance_error(&detail) => INSUFFICIENT_BALANCE,
        _ if network => NETWORK_ERROR,
        _ => MODEL_REQUEST_FAILED,
    };
    Failure {
        code: code.into(),
        detail: Some(detail),
        unbilled: !progressed && (matches!(status, Some(400..=499)) || not_sent),
        native: None,
    }
}

pub(super) fn runtime_at(base: &Path) -> Result<Runtime, String> {
    let node = base
        .join("node")
        .join(if cfg!(windows) { "node.exe" } else { "node" });
    let entry = base.join("runtime").join("src").join("main.mjs");
    if !node.is_file() || !entry.is_file() {
        return Err(RUNTIME_UNAVAILABLE.into());
    }
    let version = std::fs::read(base.join("manifest.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|manifest| manifest["sdkVersion"].as_str().map(str::to_owned));
    Ok(Runtime {
        node,
        entry,
        version,
    })
}

fn runtime_command(runtime: &Runtime) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(&runtime.node);
    command.arg("--max-old-space-size=512").arg(&runtime.entry);
    process::clean_environment(&mut command);
    command
}

/// How a run ended when it did not fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Finish {
    Completed,
    Stopped,
}

/// A failed run: a stable `assistant_*` code, and for provider failures the
/// scrubbed provider text.
#[derive(Debug)]
struct Failure {
    code: String,
    detail: Option<String>,
    /// The request cost nothing: the session's cost stays known.
    unbilled: bool,
    /// For [`RETRY_WITH_NEW_KEY`]: the refused attempt's native entries, kept
    /// if the retry does not start.
    native: Option<Value>,
}

impl Failure {
    /// A failure before anything was sent: it cost nothing.
    fn unsent(code: String) -> Self {
        Self {
            unbilled: true,
            ..code.into()
        }
    }
}

impl From<String> for Failure {
    fn from(code: String) -> Self {
        Self {
            code,
            detail: None,
            unbilled: false,
            native: None,
        }
    }
}

impl From<&str> for Failure {
    fn from(code: &str) -> Self {
        code.to_string().into()
    }
}

/// What one runtime attempt has done so far.
#[derive(Default)]
struct Attempt {
    projection: ProjectionBuffer,
    /// Text reached the user or a request counted tokens: a refused key is
    /// then not retried, so nothing shown or billed is replayed.
    progressed: bool,
    /// The runtime answered the run request; there is nothing left to stop.
    answered: bool,
    /// The run request reached the runtime: before that nothing was billed.
    sent: bool,
}

/// Whether a runtime usage report counts any tokens.
fn counts_tokens(data: &Value) -> bool {
    let usage = data.get("usage").unwrap_or(data);
    ["input", "output", "cacheRead", "cacheWrite", "totalTokens"]
        .iter()
        .any(|key| usage[*key].as_u64().unwrap_or(0) > 0)
}

impl AssistantHost {
    pub fn new(env: Arc<dyn Environment>) -> Arc<Self> {
        Arc::new(Self {
            env,
            generation: AtomicU64::new(0),
            admission: Mutex::default(),
            tasks: Mutex::default(),
            native_locks: Mutex::default(),
            recovered: Mutex::default(),
            runs: Mutex::default(),
            models: Mutex::default(),
            keys: Mutex::default(),
        })
    }

    /// Remember a key handed to a run, so drafts and titles lose it too.
    pub(super) fn remember_key(&self, key: &str) {
        let mut keys = self.keys.lock().unwrap_or_else(PoisonError::into_inner);
        if !keys.iter().any(|known| known == key) {
            keys.push(key.to_owned());
        }
    }

    /// Every key this process handed to a run.
    pub(super) fn known_keys(&self) -> Vec<String> {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub(super) fn store(&self, owner: &str) -> Result<AssistantStore, String> {
        AssistantStore::for_owner(&self.env.config_dir(), owner)
    }

    pub(super) async fn identity(&self) -> Result<Identity, String> {
        self.env.identity().await
    }

    /// Fails unless the account, region and authority generation a request
    /// started with are still current.
    pub(super) async fn ensure_current(
        &self,
        identity: &Identity,
        generation: u64,
    ) -> Result<(), String> {
        if self.generation() != generation || self.identity().await? != *identity {
            return Err(IDENTITY_CHANGED.into());
        }
        Ok(())
    }

    /// The chat models of `apex`, fetched again after [`MODELS_TTL`]. When
    /// that fetch fails, the last catalog is better than none.
    pub(super) async fn models(&self, apex: &str) -> Result<Vec<catalog::Model>, String> {
        self.catalog(apex, false).await
    }

    async fn catalog(&self, apex: &str, refresh: bool) -> Result<Vec<catalog::Model>, String> {
        let cached = self.models.lock().map_err(|_| STATE)?.get(apex).cloned();
        if let Some((fetched, models)) = &cached {
            if !refresh && fetched.elapsed() < MODELS_TTL {
                return Ok(models.clone());
            }
        }
        match self.env.models(apex).await {
            Ok(models) => {
                self.models
                    .lock()
                    .map_err(|_| STATE)?
                    .insert(apex.to_owned(), (std::time::Instant::now(), models.clone()));
                Ok(models)
            }
            Err(error) => cached.map(|(_, models)| models).ok_or(error),
        }
    }

    /// One model of `apex`'s catalog; a model the cached catalog lacks is
    /// looked up once more in a fresh one.
    pub(super) async fn model(&self, apex: &str, id: &str) -> Result<catalog::Model, String> {
        let find = |models: Vec<catalog::Model>| models.into_iter().find(|m| m.id == id);
        if let Some(model) = find(self.models(apex).await?) {
            return Ok(model);
        }
        find(self.catalog(apex, true).await?).ok_or_else(|| MODEL_UNAVAILABLE.into())
    }

    fn forget_models(&self, apex: &str) {
        if let Ok(mut models) = self.models.lock() {
            models.remove(apex);
        }
    }

    /// Admission for a key fetch: open, and no revocation since `generation`.
    pub(super) fn admit(&self, generation: u64) -> Result<(), String> {
        self.ensure_accepting_runs()?;
        if self.generation() != generation {
            return Err(IDENTITY_CHANGED.into());
        }
        Ok(())
    }

    pub(super) fn runtime(&self) -> Result<Runtime, String> {
        runtime_at(&self.env.runtime_dir()?)
    }

    fn native_lock(&self, key: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
        Ok(self
            .native_locks
            .lock()
            .map_err(|_| STATE)?
            .entry(key.into())
            .or_default()
            .clone())
    }

    // ── recovery ─────────────────────────────────────────────────────────

    fn history_is_live(&self, owner: &str, sid: &str) -> Result<bool, String> {
        if self
            .recovered
            .lock()
            .map_err(|_| STATE)?
            .contains(&session_key(owner, sid))
        {
            return Ok(true);
        }
        Ok(self
            .runs
            .lock()
            .map_err(|_| STATE)?
            .get(sid)
            .is_some_and(|r| r.identity.owner == owner))
    }

    /// The session's native lock, unless its history is live (a run owns it)
    /// or already recovered. Never waits for a running turn.
    async fn acquire_recovery_lock(
        &self,
        owner: &str,
        sid: &str,
    ) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>, String> {
        if self.history_is_live(owner, sid)? {
            return Ok(None);
        }
        let guard = self
            .native_lock(&session_key(owner, sid))?
            .lock_owned()
            .await;
        if self.history_is_live(owner, sid)? {
            return Ok(None);
        }
        Ok(Some(guard))
    }

    /// Settle a run the previous app process left behind: take the reply from
    /// the runtime's native history, mark the run interrupted and its usage
    /// incomplete. Once per session and process; a no-op while a run is live.
    pub async fn recover(&self, identity: &Identity, sid: &str) -> Result<(), String> {
        validate_id(sid)?;
        let key = session_key(&identity.owner, sid);
        let Some(_guard) = self.acquire_recovery_lock(&identity.owner, sid).await? else {
            return Ok(());
        };
        let (record, native) = blocking({
            let store = self.store(&identity.owner)?;
            let sid = sid.to_owned();
            move || Ok((store.load_record(&sid)?, store.native_history_path(&sid)?))
        })
        .await?;
        if is_dangling(&record) {
            let store = self.store(&identity.owner)?;
            let entries = match self.runtime() {
                Ok(runtime) if native.is_file() => Some(load_native(&runtime, &native).await?),
                _ => None,
            };
            if self.identity().await? != *identity {
                return Err(IDENTITY_CHANGED.into());
            }
            let sid = sid.to_owned();
            blocking(move || {
                store.mutate(&sid, |record| settle_interrupted(record, entries.as_ref()))
            })
            .await?;
        }
        self.recovered.lock().map_err(|_| STATE)?.insert(key);
        Ok(())
    }

    /// At startup: recover every session of the signed-in account that a
    /// previous process left running.
    pub async fn recover_all(&self) {
        let Ok(identity) = self.identity().await else {
            return;
        };
        let listed = match self.store(&identity.owner) {
            Ok(store) => {
                blocking(move || {
                    let mut sessions = store.list(false, "")?.sessions;
                    sessions.extend(store.list(true, "")?.sessions);
                    Ok(sessions)
                })
                .await
            }
            Err(error) => Err(error),
        };
        let sessions = match listed {
            Ok(sessions) => sessions,
            Err(error) => {
                log::warn!("[Assistant] startup recovery could not list sessions: {error}");
                return;
            }
        };
        for session in sessions.iter().filter(|s| s.active_run.is_some()) {
            if let Err(error) = self.recover(&identity, &session.id).await {
                log::warn!(
                    "[Assistant] could not recover session {}: {error}",
                    session.id
                );
            }
        }
    }

    // ── runs ─────────────────────────────────────────────────────────────

    /// The live run is the only truth for `activeRun`; the saved value can be
    /// left over from a process that died.
    pub(super) fn decorate(&self, owner: &str, session: &mut Session) {
        session.active_run = self.runs.lock().ok().and_then(|runs| {
            runs.get(&session.id)
                .filter(|r| r.identity.owner == owner)
                .map(|r| RunRef {
                    run_id: r.run_id.clone(),
                    operation_id: r.operation_id.clone(),
                })
        });
    }

    pub(super) fn cancel_session(&self, sid: &str) {
        if let Some(run) = self
            .runs
            .lock()
            .ok()
            .and_then(|runs| runs.get(sid).cloned())
        {
            run.cancel.send_replace(true);
        }
    }

    /// Stop a run. A stop that arrives before its run registered is kept as a
    /// cancelled operation, so the late send is refused. Blocking file IO.
    pub(super) fn request_stop(
        &self,
        store: &AssistantStore,
        identity: &Identity,
        sid: &str,
        rid: &str,
        op: &str,
    ) -> Result<(), String> {
        // Held through the receipt: registration takes the same lock while it
        // saves its operation and publishes its RunControl.
        let runs = self.runs.lock().map_err(|_| STATE)?;
        if let Some(run) = runs.get(sid) {
            if run.identity != *identity || run.run_id != rid || run.operation_id != op {
                return Err("assistant_stale_operation".into());
            }
            run.cancel.send_replace(true);
            return Ok(());
        }
        store.mutate(sid, |record| {
            if !record.operations.iter().any(|o| o.id == op) {
                record.operations.push(Operation {
                    id: op.into(),
                    run_id: rid.into(),
                    status: OperationStatus::Cancelled,
                });
            }
            Ok(())
        })
    }

    /// Admit a run: `publish` saves its messages and operation under the same
    /// lock as revocation, so a run either registers before a logout or
    /// shutdown (and is cancelled by it) or never starts. Blocking file IO.
    pub(super) fn register_run(
        &self,
        run: Arc<RunControl>,
        generation: u64,
        publish: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut runs = self.runs.lock().map_err(|_| STATE)?;
        self.ensure_accepting_runs()?;
        if self.generation() != generation {
            return Err(IDENTITY_CHANGED.into());
        }
        if runs.contains_key(&run.session_id) {
            return Err("assistant_session_busy".into());
        }
        publish()?;
        runs.insert(run.session_id.clone(), run);
        Ok(())
    }

    pub fn ensure_accepting_runs(&self) -> Result<(), String> {
        let admission = self.admission.lock().map_err(|_| STATE)?;
        if admission.shutting_down {
            Err("assistant_shutting_down".into())
        } else if !admission.logouts.is_empty() {
            Err("assistant_signing_out".into())
        } else if admission.needs_sign_in {
            Err(SIGN_IN_REQUIRED.into())
        } else {
            Ok(())
        }
    }

    fn validate_run(&self, run: &RunControl) -> Result<(), String> {
        if run.cancelled() {
            return Err(CANCELLED.into());
        }
        self.ensure_accepting_runs()
    }

    /// Before logout revokes keys: cancel every run and refuse new ones until
    /// the next sign-in. Returns the receipt for [`Self::finish_logout`].
    pub fn begin_logout(&self) -> u64 {
        self.revoke_runs(|admission, receipt| {
            admission.needs_sign_in = true;
            admission.logouts.insert(receipt);
        })
    }

    /// Logout is done. Admission stays closed until [`Self::sign_in_completed`].
    pub fn finish_logout(&self, receipt: u64) {
        self.admission
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .logouts
            .remove(&receipt);
    }

    /// The regular sign-in finished: reopen admission, unless a logout is
    /// still revoking keys or the app is shutting down.
    pub fn sign_in_completed(&self) {
        self.cancel_all();
        let _registration = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !admission.shutting_down && admission.logouts.is_empty() {
            admission.needs_sign_in = false;
        }
    }

    pub fn begin_shutdown(&self) {
        self.revoke_runs(|admission, _| admission.shutting_down = true);
    }

    /// A native terminal callback may bypass ExitRequested entirely: finish
    /// the cleanup on that synchronous callback, before the event loop ends.
    pub fn handle_terminal_exit(&self, event: &tauri::RunEvent) -> bool {
        if !matches!(event, tauri::RunEvent::Exit) {
            return false;
        }
        tauri::async_runtime::block_on(self.shutdown());
        true
    }

    /// Cancel every run, give each a moment to save, then abort what is left;
    /// aborting drops the owned process, which kills its process group or Job.
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
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    /// Cancel every run but keep admission open (the account, its sign-in
    /// state or the region changed).
    pub fn cancel_all(&self) {
        self.revoke_runs(|_, _| {});
    }

    fn revoke_runs(&self, transition: impl FnOnce(&mut Admission, u64)) -> u64 {
        // Registration and revocation linearize under the runs lock.
        let runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        let generation = self
            .generation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        transition(
            &mut self
                .admission
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            generation,
        );
        for run in runs.values() {
            run.cancel.send_replace(true);
        }
        generation
    }

    /// Start a registered run in the background.
    pub(super) fn spawn_run(self: &Arc<Self>, control: Arc<RunControl>, params: Value) {
        // Inserted under the lock the task's own removal takes, so a run that
        // ends at once cannot leave its handle behind.
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        let run_id = control.run_id.clone();
        let task = tokio::spawn(self.clone().run(control, params));
        tasks.insert(run_id, task);
    }

    fn emit(&self, run: &RunControl, kind: &str, mut data: Value) {
        scrub_secrets_in(&mut data, &run.secrets());
        self.env.emit(Event {
            session_id: run.session_id.clone(),
            run_id: run.run_id.clone(),
            operation_id: run.operation_id.clone(),
            r#type: kind.into(),
            data,
        });
    }

    pub(super) fn emit_start(&self, run: &RunControl) {
        self.emit(run, "run_start", json!({"messageId": run.message_id}));
    }

    async fn run(self: Arc<Self>, control: Arc<RunControl>, params: Value) {
        let key = session_key(&control.identity.owner, &control.session_id);
        let outcome = match self.native_lock(&key) {
            Ok(lock) => {
                let _guard = lock.lock_owned().await;
                self.run_with_retry(&control, params).await
            }
            Err(error) => Err(error.into()),
        };
        self.finish(&control, outcome).await;
    }

    /// One run, retried once with a new key when the gateway refused the key.
    /// The refused attempt is rolled out of the native history first, so the
    /// conversation holds the turn once.
    async fn run_with_retry(
        &self,
        control: &Arc<RunControl>,
        mut params: Value,
    ) -> Result<Finish, Failure> {
        let native = PathBuf::from(params["sessionFile"].as_str().ok_or(PROTOCOL)?);
        let before = blocking({
            let native = native.clone();
            move || read_optional(&native)
        })
        .await?;
        match self.run_once(control, &params, true).await {
            Err(failure) if failure.code == RETRY_WITH_NEW_KEY => {
                // The refused attempt cost nothing. It is rolled back only once
                // a new key is in hand; without one it stays in both histories.
                let refused = failure.native.unwrap_or(Value::Null);
                let keep_refused = |code: String| async {
                    self.apply_native(control, &refused)
                        .await
                        .map_err(Failure::unsent)?;
                    Err::<Finish, _>(Failure {
                        detail: failure.detail.clone(),
                        ..Failure::unsent(code)
                    })
                };
                let key = match self.key_for_retry(control, &params).await {
                    Ok(key) => key,
                    Err(code) => return keep_refused(code).await,
                };
                if let Err(code) = blocking(move || restore(&native, before)).await {
                    return keep_refused(code).await;
                }
                self.remember_key(&key);
                // The retry's runtime replaces the refused key as well.
                params["knownSecrets"] = json!(control.secrets());
                control.add_secret(&key);
                params["apiKey"] = json!(key);
                self.run_once(control, &params, false).await
            }
            outcome => outcome,
        }
    }

    /// A new key for a run whose key the gateway refused, asked for only
    /// while the run may still go on.
    async fn key_for_retry(
        &self,
        control: &Arc<RunControl>,
        params: &Value,
    ) -> Result<String, String> {
        self.validate_run(control)?;
        if self.identity().await? != control.identity {
            return Err(IDENTITY_CHANGED.into());
        }
        let rejected = params["apiKey"].as_str().unwrap_or_default();
        let admit = || self.validate_run(control);
        self.env
            .api_key(&control.identity, Some(rejected), &admit)
            .await
    }

    async fn run_once(
        &self,
        control: &Arc<RunControl>,
        params: &Value,
        may_retry: bool,
    ) -> Result<Finish, Failure> {
        use tokio::io::BufReader;
        // Until the run request reaches the runtime, nothing is billed.
        let stdio = || Failure::unsent(STDIO.into());
        self.validate_run(control).map_err(Failure::unsent)?;
        let runtime = self.runtime().map_err(Failure::unsent)?;
        let mut child =
            process::Child::spawn(runtime_command(&runtime)).map_err(Failure::unsent)?;
        let mut input = child.take_stdin().ok_or_else(stdio)?;
        let output = child.take_stdout().ok_or_else(stdio)?;
        let stderr = child.take_stderr().ok_or_else(stdio)?;
        // Drain, never publish: dependencies may print request headers.
        let drain = tokio::spawn(async move {
            let _ = process::bounded_output(stderr, 64 * 1024).await;
        });
        let mut output = BufReader::new(output);
        let mut buffer = Vec::new();
        let mut attempt = Attempt::default();
        let request_id = id();
        let result = self
            .converse(
                control,
                params,
                may_retry,
                &request_id,
                (&mut input, &mut output, &mut buffer),
                &mut attempt,
            )
            .await
            .map_err(|failure| Failure {
                unbilled: failure.unbilled || !attempt.sent,
                ..failure
            });
        let flushed = self.flush(control, &mut attempt.projection).await;
        let result = match (result, flushed) {
            (Ok(_), Err(error)) => Err(error.into()),
            (result, _) => result,
        };
        if result.is_err()
            && !attempt.answered
            && (control.cancelled() || tokio::time::Instant::now() >= control.deadline)
        {
            // Let the runtime save the interrupted turn before the process goes.
            self.stop_runtime(control, &request_id, &mut input, &mut output, &mut buffer)
                .await;
        }
        child.stop().await;
        drain.abort();
        result
    }

    async fn converse<I, O>(
        &self,
        control: &Arc<RunControl>,
        params: &Value,
        may_retry: bool,
        request_id: &str,
        (input, output, buffer): (&mut I, &mut O, &mut Vec<u8>),
        attempt: &mut Attempt,
    ) -> Result<Finish, Failure>
    where
        I: AsyncWrite + Unpin,
        O: AsyncBufRead + Unpin,
    {
        self.validate_run(control)?;
        if self.identity().await? != control.identity {
            return Err(IDENTITY_CHANGED.into());
        }
        let line = frame(&json!({"jsonrpc":"2.0","id":request_id,"method":"run","params":params}))?;
        let mut cancel = control.cancel.subscribe();
        self.validate_run(control)?;
        tokio::select! {
            biased;
            _ = cancel.changed() => return Err(CANCELLED.into()),
            result = write_line(input, &line) => result?,
        }
        attempt.sent = true;
        let mut ticker = tokio::time::interval(Duration::from_millis(250));
        loop {
            let length = tokio::select! {
                _ = cancel.changed() => return Err(CANCELLED.into()),
                _ = ticker.tick() => {
                    if attempt.projection.due() {
                        self.flush(control, &mut attempt.projection).await?;
                    }
                    if self.identity().await? != control.identity {
                        return Err(IDENTITY_CHANGED.into());
                    }
                    self.validate_run(control)?;
                    if tokio::time::Instant::now() > control.deadline {
                        return Err("assistant_run_timeout".into());
                    }
                    continue;
                }
                result = read_frame(output, buffer) => result?,
            };
            if length == 0 {
                return Err("assistant_runtime_exited".into());
            }
            let value: Value = serde_json::from_slice(buffer).map_err(|_| PROTOCOL)?;
            buffer.clear();
            if value["method"] == "event" {
                self.on_event(control, &value["params"], attempt).await?;
            } else if value["method"] == "tool" {
                ensure_same_run(control, &value["params"])?;
                // Chat registers no tools; refuse at once so the runtime never waits.
                let reply = frame(
                    &json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32000,"message":"assistant_unknown_tool"}}),
                )?;
                write_line(input, &reply).await?;
            } else if value["id"] == request_id {
                attempt.answered = true;
                return self.on_result(control, &value, may_retry, attempt).await;
            }
        }
    }

    async fn on_event(
        &self,
        control: &Arc<RunControl>,
        params: &Value,
        attempt: &mut Attempt,
    ) -> Result<(), Failure> {
        ensure_same_run(control, params)?;
        let kind = params["type"].as_str().ok_or(PROTOCOL)?;
        let mut data = params["data"].clone();
        if !data.is_object() {
            return Err(PROTOCOL.into());
        }
        match kind {
            "text_delta" | "thinking_delta" => {
                attempt.progressed = true;
                attempt.projection.push(control, kind, &mut data);
                if attempt.projection.due() {
                    self.flush(control, &mut attempt.projection).await?;
                }
                self.emit(control, kind, data);
            }
            "usage" => {
                attempt.progressed |= counts_tokens(&data);
                let usage = self.save_usage(control, data).await?;
                self.emit(control, "usage", json!({ "usage": usage }));
            }
            "compaction_start" | "compaction_end" => self.emit(control, kind, data),
            // run_start, message_end, error and run_end: the host publishes its
            // own once the turn is saved.
            _ => {}
        }
        Ok(())
    }

    async fn on_result(
        &self,
        control: &Arc<RunControl>,
        value: &Value,
        may_retry: bool,
        attempt: &mut Attempt,
    ) -> Result<Finish, Failure> {
        let secrets = control.secrets();
        if let Some(error) = value.get("error") {
            return Err(Failure {
                detail: error["message"].as_str().map(|m| scrub_detail(m, &secrets)),
                ..Failure::from("assistant_runtime_failed")
            });
        }
        self.flush(control, &mut attempt.projection).await?;
        let result = &value["result"];
        match result["status"].as_str() {
            Some("completed") | Some("stopped") => {
                self.apply_native(control, &result["entries"]).await?;
                Ok(if result["status"] == "stopped" {
                    Finish::Stopped
                } else {
                    Finish::Completed
                })
            }
            _ => {
                let detail = scrub_detail(
                    result["errorMessage"].as_str().unwrap_or("Request failed"),
                    &secrets,
                );
                let failure = provider_failure(result, detail, attempt.progressed);
                if failure.code == KEY_REJECTED && may_retry && !attempt.progressed {
                    // Nothing saved yet: the caller retries with a new key, or
                    // keeps these entries if it cannot.
                    return Err(Failure {
                        code: RETRY_WITH_NEW_KEY.into(),
                        native: Some(result["entries"].clone()),
                        ..failure
                    });
                }
                self.apply_native(control, &result["entries"]).await?;
                Err(failure)
            }
        }
    }

    /// After a cancel or the deadline: ask the runtime to stop and save its
    /// interrupted turn, within two seconds. No host tool runs meanwhile.
    async fn stop_runtime<I, O>(
        &self,
        control: &Arc<RunControl>,
        request_id: &str,
        input: &mut I,
        output: &mut O,
        buffer: &mut Vec<u8>,
    ) where
        I: AsyncWrite + Unpin,
        O: AsyncBufRead + Unpin,
    {
        let stop = async {
            let line = frame(
                &json!({"jsonrpc":"2.0","id":"shutdown","method":"stop","params":{"sessionId":control.session_id,"runId":control.run_id,"operationId":control.operation_id}}),
            )?;
            write_line(input, &line).await?;
            buffer.clear();
            loop {
                if read_frame(output, buffer).await? == 0 {
                    return Ok(());
                }
                let value: Value = serde_json::from_slice(buffer).map_err(|_| PROTOCOL)?;
                buffer.clear();
                if value["id"] == request_id {
                    if value.get("error").is_none() {
                        self.apply_native(control, &value["result"]["entries"])
                            .await?;
                    }
                    return Ok(());
                }
                if value["method"] == "tool" {
                    let reply = frame(
                        &json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":"unknown_outcome","message":CANCELLED,"data":{"outcomeUnknown":true}}}),
                    )?;
                    write_line(input, &reply).await?;
                }
            }
        };
        let saved: Result<Result<(), String>, _> =
            tokio::time::timeout(Duration::from_secs(2), stop).await;
        if let Ok(Err(error)) = saved {
            log::warn!("[Assistant] could not save the interrupted turn: {error}");
        }
    }

    async fn flush(
        &self,
        control: &RunControl,
        projection: &mut ProjectionBuffer,
    ) -> Result<(), String> {
        let Some(pending) = projection.take() else {
            return Ok(());
        };
        let store = self.store(&control.identity.owner)?;
        let (sid, mid, secrets) = (
            control.session_id.clone(),
            control.message_id.clone(),
            control.secrets(),
        );
        blocking(move || save_projection(&store, &sid, &mid, &pending, &secrets)).await
    }

    async fn save_usage(&self, control: &RunControl, mut data: Value) -> Result<Value, String> {
        scrub_secrets_in(&mut data, &control.secrets());
        let store = self.store(&control.identity.owner)?;
        let sid = control.session_id.clone();
        blocking(move || {
            store.mutate(&sid, |record| {
                add_usage(&mut record.usage, data.get("usage").unwrap_or(&data));
                record.session.updated_at = now();
                Ok(record.usage.clone())
            })
        })
        .await
    }

    async fn apply_native(&self, control: &RunControl, entries: &Value) -> Result<(), String> {
        let store = self.store(&control.identity.owner)?;
        let (sid, entries, secrets) = (
            control.session_id.clone(),
            entries.clone(),
            control.secrets(),
        );
        blocking(move || store.mutate(&sid, |record| merge_native(record, &entries, &secrets)))
            .await
    }

    /// Save how the run ended, release the session, then publish: the saved
    /// message and session, an error when it failed, and `run_end` last.
    async fn finish(&self, control: &Arc<RunControl>, outcome: Result<Finish, Failure>) {
        let stopped = match &outcome {
            Ok(finish) => *finish == Finish::Stopped,
            Err(_) => control.cancelled(),
        };
        let completed = matches!(outcome, Ok(Finish::Completed));
        // A failure that cost nothing leaves the session's cost known.
        let unbilled = matches!(&outcome, Err(failure) if failure.unbilled && !stopped);
        if let Err(failure) = &outcome {
            if failure.code == MODEL_UNAVAILABLE {
                self.forget_models(&control.identity.apex);
            }
        }
        if outcome.is_err() {
            if let Ok(mut recovered) = self.recovered.lock() {
                recovered.remove(&session_key(&control.identity.owner, &control.session_id));
            }
        }
        let saved = match self.store(&control.identity.owner) {
            Ok(store) => {
                let control = control.clone();
                let sid = control.session_id.clone();
                blocking(move || {
                    store.mutate(&sid, |record| {
                        settle_run(record, &control, completed, stopped, unbilled)
                    })
                })
                .await
            }
            Err(error) => Err(error),
        };
        if let Ok(mut runs) = self.runs.lock() {
            if runs
                .get(&control.session_id)
                .is_some_and(|r| r.run_id == control.run_id)
            {
                runs.remove(&control.session_id);
            }
        }
        match saved {
            Ok((message, session)) => {
                if let Some(message) = message {
                    self.emit(control, "message", json!({ "message": message }));
                }
                self.emit(control, "session", json!({ "session": session }));
            }
            Err(error) => log::warn!("[Assistant] could not save the end of a run: {error}"),
        }
        let status = if completed {
            "completed"
        } else if stopped {
            "stopped"
        } else {
            "error"
        };
        if let Err(failure) = &outcome {
            if !stopped {
                let mut data = json!({ "code": failure.code });
                if let Some(detail) = &failure.detail {
                    data["message"] = json!(detail);
                }
                self.emit(control, "error", data);
            }
        }
        self.emit(
            control,
            "run_end",
            json!({ "stopped": stopped, "status": status }),
        );
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.remove(&control.run_id);
        }
    }
}

fn ensure_same_run(control: &RunControl, params: &Value) -> Result<(), String> {
    if params["sessionId"] != control.session_id
        || params["runId"] != control.run_id
        || params["operationId"] != control.operation_id
    {
        return Err("assistant_runtime_identity_mismatch".into());
    }
    Ok(())
}

fn frame(value: &Value) -> Result<Vec<u8>, String> {
    let mut line = serde_json::to_vec(value).map_err(|_| PROTOCOL)?;
    line.push(b'\n');
    Ok(line)
}

async fn write_line<W: AsyncWrite + Unpin>(input: &mut W, line: &[u8]) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    input.write_all(line).await.map_err(|_| STDIO)?;
    input.flush().await.map_err(|_| STDIO.to_string())
}

/// One newline-terminated frame into `buffer`; `Ok(0)` at end of stream.
async fn read_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> Result<usize, String> {
    use tokio::io::AsyncBufReadExt;
    loop {
        let bytes = reader.fill_buf().await.map_err(|_| STDIO)?;
        if bytes.is_empty() {
            return Ok(0);
        }
        let count = bytes
            .iter()
            .position(|v| *v == b'\n')
            .map_or(bytes.len(), |i| i + 1);
        if buffer.len() + count > FRAME_LIMIT {
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

/// The native entries of `file`, read by a short-lived runtime process.
async fn load_native(runtime: &Runtime, file: &Path) -> Result<Value, String> {
    use tokio::io::BufReader;
    let mut child = process::Child::spawn(runtime_command(runtime))?;
    let mut input = child.take_stdin().ok_or(STDIO)?;
    let mut output = BufReader::new(child.take_stdout().ok_or(STDIO)?);
    let stderr = child.take_stderr().ok_or(STDIO)?;
    let drain = tokio::spawn(async move {
        let _ = process::bounded_output(stderr, 64 * 1024).await;
    });
    let request =
        frame(&json!({"jsonrpc":"2.0","id":"load","method":"load","params":{"sessionFile":file}}))?;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        write_line(&mut input, &request).await?;
        let mut bytes = vec![];
        read_frame(&mut output, &mut bytes).await?;
        let response: Value = serde_json::from_slice(&bytes).map_err(|_| PROTOCOL)?;
        if response["id"] != "load" || response.get("error").is_some() {
            return Err("assistant_history_recovery_failed".to_string());
        }
        Ok(response["result"]["entries"].clone())
    })
    .await
    .map_err(|_| "assistant_history_recovery_timeout".to_string())
    .and_then(|v| v);
    child.stop().await;
    drain.abort();
    result
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("assistant_storage_unavailable".into()),
    }
}

/// Put the native history back as it was before a refused attempt.
fn restore(path: &Path, before: Option<Vec<u8>>) -> Result<(), String> {
    let restored = match before {
        Some(bytes) => crate::config::atomic_write_with(
            path,
            &bytes,
            crate::config::AtomicOpts { mode: Some(0o600) },
        )
        .is_ok(),
        None => match std::fs::remove_file(path) {
            Ok(()) => true,
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        },
    };
    if restored {
        Ok(())
    } else {
        Err("assistant_storage_unavailable".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// Signed in as one account in `ofox.ai`, files in a temp dir; a one-model
    /// catalog whose fetches are counted; no key or runtime.
    struct FixedEnvironment {
        config: PathBuf,
        catalog: Mutex<Vec<String>>,
        fetches: AtomicU64,
        runtime: Mutex<Option<PathBuf>>,
    }

    impl Environment for FixedEnvironment {
        fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
            Box::pin(async { Ok(identity()) })
        }
        fn api_key<'a>(
            &'a self,
            _: &'a Identity,
            _: Option<&'a str>,
            _: &'a Admit<'a>,
        ) -> BoxFuture<'a, Result<String, String>> {
            Box::pin(async { Err("assistant_key_unavailable".to_string()) })
        }
        fn key_ready<'a>(&'a self, _: &'a Identity) -> BoxFuture<'a, bool> {
            Box::pin(async { false })
        }
        fn models<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>> {
            Box::pin(async {
                self.fetches.fetch_add(1, Ordering::SeqCst);
                Ok(self
                    .catalog
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|id| model(id))
                    .collect())
            })
        }
        fn gateway_base(&self, _: &str) -> Result<String, String> {
            Err("assistant_unknown_apex".into())
        }
        fn config_dir(&self) -> PathBuf {
            self.config.clone()
        }
        fn runtime_dir(&self) -> Result<PathBuf, String> {
            self.runtime
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| RUNTIME_UNAVAILABLE.into())
        }
        fn emit(&self, _: Event) {}
    }

    fn identity() -> Identity {
        Identity {
            owner: "cd".repeat(32),
            apex: "ofox.ai".into(),
        }
    }

    fn model(id: &str) -> catalog::Model {
        serde_json::from_value(json!({
            "id": id, "name": id, "contextWindow": 32000, "maxOutputTokens": 1024,
            "supportsImages": false, "inputModalities": ["text"],
            "supportedEndpoints": ["/v1/chat/completions"], "inputPricePerMillion": null,
            "outputPricePerMillion": null, "pricing": {}, "cost": {}, "pricingKnown": false,
        }))
        .unwrap()
    }

    fn fixture_with_env() -> (tempfile::TempDir, Arc<FixedEnvironment>, Arc<AssistantHost>) {
        let config = tempfile::tempdir().unwrap();
        let env = Arc::new(FixedEnvironment {
            config: config.path().to_path_buf(),
            catalog: Mutex::new(vec!["test/model".into()]),
            fetches: AtomicU64::new(0),
            runtime: Mutex::new(None),
        });
        let host = AssistantHost::new(env.clone());
        (config, env, host)
    }

    fn fixture() -> (tempfile::TempDir, Arc<AssistantHost>) {
        let (config, _env, host) = fixture_with_env();
        (config, host)
    }

    fn control(sid: &str) -> Arc<RunControl> {
        let request: SendRequest = serde_json::from_value(json!({
            "sessionId": sid, "runId": id(), "operationId": id(), "text": "hi",
        }))
        .unwrap();
        Arc::new(RunControl::new(
            identity(),
            &request,
            "test-opaque-run-key".into(),
        ))
    }

    async fn queued_registration_after_lifecycle_start(shutdown: bool) {
        let (_config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let calls = Arc::new(AtomicU64::new(0));
        let (release, queued) = tokio::sync::oneshot::channel::<()>();
        let worker = tokio::spawn({
            let (host, sid, calls) = (host.clone(), sid.clone(), calls.clone());
            async move {
                queued.await.map_err(|_| "test_queue_closed".to_string())?;
                // The queued IPC enters only after revocation, so it sees the
                // new generation: generation equality alone cannot refuse it.
                let generation = host.generation();
                let run = control(&sid);
                let store = host.store(&identity().owner)?;
                host.register_run(run.clone(), generation, || {
                    store.mutate(&sid, |record| {
                        record.operations.push(Operation {
                            id: run.operation_id.clone(),
                            run_id: run.run_id.clone(),
                            status: OperationStatus::Running,
                        });
                        Ok(())
                    })
                })?;
                host.validate_run(&run)?;
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>(())
            }
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
            "queued work must not reach the provider after revocation begins"
        );
        assert!(result.is_err(), "queued work must fail before registration");
        assert!(host.runs.lock().unwrap().is_empty());
        let record = store.load_record(&sid).unwrap();
        assert!(record.operations.is_empty());
        assert!(record.messages.is_empty());
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
    fn sign_in_reopens_only_a_finished_logout_and_never_shutdown() {
        let (_config, host) = fixture();
        let first = host.begin_logout();
        let second = host.begin_logout();
        host.finish_logout(first);
        host.sign_in_completed();
        assert_eq!(
            host.ensure_accepting_runs().unwrap_err(),
            "assistant_signing_out",
            "another logout is still revoking keys"
        );
        host.finish_logout(second);
        assert_eq!(
            host.ensure_accepting_runs().unwrap_err(),
            SIGN_IN_REQUIRED,
            "finishing a logout is not a sign-in"
        );
        host.sign_in_completed();
        assert!(host.ensure_accepting_runs().is_ok());
        host.begin_shutdown();
        host.sign_in_completed();
        assert_eq!(
            host.ensure_accepting_runs().unwrap_err(),
            "assistant_shutting_down",
            "a late sign-in never reopens shutdown"
        );
    }

    #[test]
    fn ordinary_identity_cancellation_keeps_new_authority_admissible() {
        let (_config, host) = fixture();
        let before = host.generation();
        host.cancel_all();
        assert!(host.ensure_accepting_runs().is_ok());
        assert_eq!(host.generation(), before + 1);
    }

    #[tokio::test]
    async fn live_history_read_never_waits_for_a_running_turns_native_lock() {
        let (_config, host) = fixture();
        let sid = id();
        let owner = identity().owner;
        let _running = host
            .native_lock(&session_key(&owner, &sid))
            .unwrap()
            .lock_owned()
            .await;
        host.runs.lock().unwrap().insert(sid.clone(), control(&sid));
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
    fn an_immediate_stop_is_kept_before_the_runtime_subscribes() {
        let (_config, host) = fixture();
        let sid = id();
        let run = control(&sid);
        host.runs.lock().unwrap().insert(sid.clone(), run.clone());
        host.cancel_session(&sid);
        assert!(
            *run.cancel.subscribe().borrow(),
            "a stop must survive without an attached receiver"
        );
        assert_eq!(host.validate_run(&run).unwrap_err(), CANCELLED);
    }

    #[test]
    fn a_stop_before_the_run_registers_is_kept_once_and_wins() {
        let (_config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let (rid, op) = (id(), id());
        host.request_stop(&store, &identity(), &sid, &rid, &op)
            .unwrap();
        host.request_stop(&store, &identity(), &sid, &rid, &op)
            .unwrap();
        let record = store.load_record(&sid).unwrap();
        assert_eq!(
            record.operations,
            [Operation {
                id: op.clone(),
                run_id: rid.clone(),
                status: OperationStatus::Cancelled,
            }]
        );
        assert!(record.messages.is_empty());
        assert_eq!(
            super::super::store::ensure_new_operation(&record, &op).unwrap_err(),
            CANCELLED
        );
    }

    #[test]
    fn a_stop_for_another_run_of_the_session_is_refused() {
        let (_config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let run = control(&sid);
        host.runs.lock().unwrap().insert(sid.clone(), run.clone());
        assert_eq!(
            host.request_stop(&store, &identity(), &sid, &id(), &run.operation_id)
                .unwrap_err(),
            "assistant_stale_operation"
        );
        let other = Identity {
            apex: "ofox.io".into(),
            ..identity()
        };
        assert!(host
            .request_stop(&store, &other, &sid, &run.run_id, &run.operation_id)
            .is_err());
        assert!(!run.cancelled());
    }

    #[test]
    fn conversation_loses_only_the_run_keys_and_diagnostics_lose_lookalikes_too() {
        let secrets = vec!["opaque-run-key-0123".to_string(), "short".to_string()];
        let lookalikes = concat!(
            "OpenAI(api_key=os.environ[\"OPENAI_API_KEY\"]) ",
            "password = input(\"Password: \") ",
            "Authorization: Bearer YOUR_TOKEN max_tokens: 4096 sk-of-abcdefghijklmnop",
        );
        let mut value = json!({
            "delta": format!("{lookalikes} opaque-run-key-0123"),
            "nested": [{"apiKey": "anything", "password": lookalikes}],
            "word": "short stays",
        });
        scrub_secrets_in(&mut value, &secrets);
        assert_eq!(
            value,
            json!({
                "delta": format!("{lookalikes} {}", crate::redact::REDACTED),
                "nested": [{"apiKey": "anything", "password": lookalikes}],
                // A secret too short to be a key is not used to mangle text.
                "word": "short stays",
            })
        );
        let detail = scrub_detail(&format!("{lookalikes} opaque-run-key-0123"), &secrets);
        for gone in [
            "opaque-run-key-0123",
            "YOUR_TOKEN",
            "abcdefghijklmnop",
            "4096",
        ] {
            assert!(!detail.contains(gone), "{gone} in {detail}");
        }
    }

    #[test]
    fn the_runtime_may_use_a_local_gateway_only_in_debug_builds() {
        assert_eq!(
            local_gateway_allowed("http://localhost:8088"),
            cfg!(debug_assertions)
        );
        for refused in [
            "https://api.ofox.ai",
            "http://localhost",
            "http://localhost:",
            "http://localhost:0",
            "http://localhost:08088",
            "http://localhost:65536",
            "http://localhost:8088/v1",
            "http://localhost.evil.com:8088",
            "http://127.0.0.1:8088",
        ] {
            assert!(!local_gateway_allowed(refused), "{refused}");
        }
    }

    #[test]
    fn provider_failures_get_their_own_codes_and_only_refused_or_unsent_ones_cost_nothing() {
        let failure = |result: Value, text: &str| {
            let failure = provider_failure(&result, text.into(), false);
            (failure.code, failure.unbilled)
        };
        let code = |code: &str, unbilled: bool| (code.to_string(), unbilled);
        // The runtime's structured status: a 4xx was refused, a 5xx may have
        // followed upstream billing.
        for (status, expected) in [
            (401, code(KEY_REJECTED, true)),
            (402, code(INSUFFICIENT_BALANCE, true)),
            (404, code(MODEL_UNAVAILABLE, true)),
            (429, code(RATE_LIMITED, true)),
            (400, code(MODEL_REQUEST_FAILED, true)),
            (500, code(MODEL_REQUEST_FAILED, false)),
            (503, code(MODEL_REQUEST_FAILED, false)),
        ] {
            assert_eq!(
                failure(json!({"errorStatus": status}), "x"),
                expected,
                "{status}"
            );
        }
        // Never left the machine: a known zero. Sent without an answer: unknown.
        assert_eq!(
            failure(
                json!({"errorKind": "not_sent", "errorCode": "ECONNREFUSED"}),
                "Connection error."
            ),
            code(NETWORK_ERROR, true)
        );
        assert_eq!(
            failure(
                json!({"errorKind": "network", "errorCode": "UND_ERR_SOCKET"}),
                "Connection error."
            ),
            code(NETWORK_ERROR, false)
        );
        assert_eq!(
            failure(
                json!({"errorKind": "not_sent"}),
                "assistant_vision_model_required"
            ),
            code(MODEL_REQUEST_FAILED, true)
        );
        // Text alone names the failure but never makes it free.
        assert_eq!(
            failure(json!({}), "fetch failed"),
            code(NETWORK_ERROR, false)
        );
        // Balance: a 402, a 403 that says so, or Ofox's own error type; never
        // an upstream quota behind a 429 or 5xx.
        for (status, text, expected) in [
            (
                403,
                "403: {\"type\":\"insufficient_quota\"}",
                INSUFFICIENT_BALANCE,
            ),
            (
                400,
                "400: {\"type\":\"insufficient_credits\"}",
                INSUFFICIENT_BALANCE,
            ),
            (429, "429: {\"type\":\"insufficient_quota\"}", RATE_LIMITED),
            (
                500,
                "500: {\"type\":\"insufficient_quota\"}",
                MODEL_REQUEST_FAILED,
            ),
            (
                503,
                "503: Insufficient balance upstream",
                MODEL_REQUEST_FAILED,
            ),
        ] {
            assert_eq!(
                failure(json!({"errorStatus": status}), text).0,
                expected,
                "{text}"
            );
        }
        // An attempt that already showed text or counted tokens is never free.
        assert!(!provider_failure(&json!({"errorStatus": 402}), "x".into(), true).unbilled);
        // Without a reported status: the head of the text, per protocol.
        for (text, expected) in [
            ("401: {\"message\":\"Invalid API key\"}", KEY_REJECTED),
            ("401 status code (no body)", KEY_REJECTED),
            (
                "401 {\"error\":{\"message\":\"Invalid API key\"}}",
                KEY_REJECTED,
            ),
            (
                "ofox-desktop API error (401): {\"message\":\"x\"}",
                KEY_REJECTED,
            ),
            (
                "402: {\"message\":\"Insufficient credits\"}",
                INSUFFICIENT_BALANCE,
            ),
            ("429 Too Many Requests", RATE_LIMITED),
            (
                "ofox-desktop API error (404): no such model",
                MODEL_UNAVAILABLE,
            ),
            ("Connection error.", NETWORK_ERROR),
            ("connect ECONNREFUSED 127.0.0.1:443", NETWORK_ERROR),
            ("4010 retries", MODEL_REQUEST_FAILED),
            ("Request failed after 401 attempts", MODEL_REQUEST_FAILED),
            ("ofox-desktop API error (500): boom", MODEL_REQUEST_FAILED),
        ] {
            assert_eq!(failure(json!({}), text).0, expected, "{text}");
        }
    }

    #[tokio::test]
    async fn a_run_that_never_reached_the_runtime_cost_nothing() {
        let (_config, host) = fixture();
        let run = control(&id());
        let failure = host.run_once(&run, &json!({}), true).await.unwrap_err();
        assert_eq!(failure.code, RUNTIME_UNAVAILABLE);
        assert!(failure.unbilled);
    }

    /// The runtime started, but the run request never reached it: nothing
    /// could have been billed.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_refused_before_its_request_reached_the_runtime_cost_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let (config, env, host) = fixture_with_env();
        let base = config.path().join("runtime-stub");
        let node = base.join("node").join("node");
        std::fs::create_dir_all(node.parent().unwrap()).unwrap();
        std::fs::write(&node, "#!/bin/sh\ncat > /dev/null\n").unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
        let entry = base.join("runtime").join("src").join("main.mjs");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(&entry, "").unwrap();
        *env.runtime.lock().unwrap() = Some(base);
        // The run was started in another region than the one now current.
        let request: SendRequest = serde_json::from_value(json!({
            "sessionId": id(), "runId": id(), "operationId": id(), "text": "hi",
        }))
        .unwrap();
        let elsewhere = Identity {
            apex: "ofox.io".into(),
            ..identity()
        };
        let run = Arc::new(RunControl::new(elsewhere, &request, "test-key-0123".into()));
        let failure = host.run_once(&run, &json!({}), true).await.unwrap_err();
        assert_eq!(failure.code, IDENTITY_CHANGED);
        assert!(failure.unbilled);
    }

    /// A refused key after shown text is not retried: the retry would append
    /// a second answer to what the user already saw.
    #[tokio::test]
    async fn a_refused_key_after_shown_text_ends_the_run_without_a_retry() {
        let (_config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let run = control(&sid);
        store
            .mutate(&sid, |record| {
                record.messages.push(
                    serde_json::from_value(json!({
                        "id": run.message_id, "role": "assistant", "content": "",
                        "createdAt": 1, "status": "streaming", "runId": run.run_id,
                    }))
                    .unwrap(),
                );
                Ok(())
            })
            .unwrap();
        let mut attempt = Attempt::default();
        let event = json!({
            "sessionId": sid, "runId": run.run_id, "operationId": run.operation_id,
            "type": "text_delta", "data": {"delta": "Already shown"},
        });
        host.on_event(&run, &event, &mut attempt).await.unwrap();
        let result = json!({"result": {
            "status": "error", "errorStatus": 401, "entries": [],
            "errorMessage": "401: {\"message\":\"Invalid API key\"}",
        }});
        let failure = host
            .on_result(&run, &result, true, &mut attempt)
            .await
            .unwrap_err();
        assert_eq!(failure.code, KEY_REJECTED);
        assert!(!failure.unbilled, "shown text may have been billed");
        assert_eq!(
            store.load_detail(&sid).unwrap().messages[0].content,
            "Already shown"
        );
    }

    #[test]
    fn only_shown_text_or_counted_tokens_make_an_attempt_unrepeatable() {
        let zero = json!({"usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0}});
        assert!(!counts_tokens(&zero));
        assert!(!counts_tokens(&json!({})));
        assert!(counts_tokens(&json!({"usage": {"input": 12, "output": 0}})));
        assert!(counts_tokens(&json!({"totalTokens": 3})));
    }

    #[tokio::test]
    async fn the_model_catalog_is_fetched_again_when_stale_or_missing_a_model() {
        let (_config, env, host) = fixture_with_env();
        host.models("ofox.ai").await.unwrap();
        host.models("ofox.ai").await.unwrap();
        assert_eq!(
            env.fetches.load(Ordering::SeqCst),
            1,
            "fresh catalogs are reused"
        );

        // A model the cached catalog lacks: one fresh look before refusing.
        env.catalog.lock().unwrap().push("new/model".into());
        assert_eq!(
            host.model("ofox.ai", "new/model").await.unwrap().id,
            "new/model"
        );
        assert_eq!(env.fetches.load(Ordering::SeqCst), 2);
        assert_eq!(
            host.model("ofox.ai", "gone/model").await.unwrap_err(),
            MODEL_UNAVAILABLE
        );
        assert_eq!(env.fetches.load(Ordering::SeqCst), 3);

        // After the TTL the catalog is fetched again.
        if let Some((fetched, _)) = host.models.lock().unwrap().get_mut("ofox.ai") {
            *fetched -= MODELS_TTL;
        }
        host.models("ofox.ai").await.unwrap();
        assert_eq!(env.fetches.load(Ordering::SeqCst), 4);

        // A run that found its model gone drops the catalog.
        host.forget_models("ofox.ai");
        host.models("ofox.ai").await.unwrap();
        assert_eq!(env.fetches.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn an_update_that_is_not_saved_leaves_the_run_running() {
        let (config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let run = control(&sid);
        host.runs.lock().unwrap().insert(sid.clone(), run.clone());
        let trash = |sid: &str| -> UpdateRequest {
            serde_json::from_value(json!({"sessionId": sid, "deleted": true})).unwrap()
        };
        let damaged = config
            .path()
            .join("assistant")
            .join(identity().owner)
            .join(&sid)
            .join("session.json");
        let saved = std::fs::read(&damaged).unwrap();
        std::fs::write(&damaged, "{").unwrap();
        assert_eq!(
            host.update_session(trash(&sid)).await.unwrap_err(),
            "assistant_invalid_session"
        );
        assert!(!run.cancelled(), "a refused update stops nothing");

        std::fs::write(&damaged, saved).unwrap();
        host.update_session(trash(&sid)).await.unwrap();
        assert!(run.cancelled(), "trashing a session stops its run");

        // Saved, but the choice could not be remembered: the old model's run
        // still stops before the error is returned.
        let run = control(&sid);
        host.runs.lock().unwrap().insert(sid.clone(), run.clone());
        let prefs = damaged
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("prefs.json");
        std::fs::create_dir_all(&prefs).unwrap();
        let switch: UpdateRequest =
            serde_json::from_value(json!({"sessionId": sid, "modelId": "test/model"})).unwrap();
        assert_eq!(
            host.update_session(switch).await.unwrap_err(),
            "assistant_invalid_storage_path"
        );
        assert!(run.cancelled());
    }

    #[test]
    fn the_runtime_needs_node_and_its_entry_and_reports_its_sdk_version() {
        let base = tempfile::tempdir().unwrap();
        assert_eq!(runtime_at(base.path()).err().unwrap(), RUNTIME_UNAVAILABLE);
        let node = base
            .path()
            .join("node")
            .join(if cfg!(windows) { "node.exe" } else { "node" });
        std::fs::create_dir_all(node.parent().unwrap()).unwrap();
        std::fs::write(&node, "").unwrap();
        assert!(runtime_at(base.path()).is_err());
        let entry = base.path().join("runtime/src/main.mjs");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(&entry, "").unwrap();
        assert_eq!(runtime_at(base.path()).unwrap().version, None);
        std::fs::write(
            base.path().join("manifest.json"),
            r#"{"sdkVersion":"1.4.0"}"#,
        )
        .unwrap();
        assert_eq!(
            runtime_at(base.path()).unwrap().version.as_deref(),
            Some("1.4.0")
        );
    }

    #[test]
    fn a_refused_attempt_is_rolled_out_of_the_native_history() {
        let dir = tempfile::tempdir().unwrap();
        let native = dir.path().join("session.jsonl");
        let before = read_optional(&native).unwrap();
        std::fs::write(&native, "{\"refused\":true}\n").unwrap();
        restore(&native, before).unwrap();
        assert!(!native.exists());

        std::fs::write(&native, "{\"kept\":true}\n").unwrap();
        let before = read_optional(&native).unwrap();
        std::fs::write(&native, "{\"kept\":true}\n{\"refused\":true}\n").unwrap();
        restore(&native, before).unwrap();
        assert_eq!(
            std::fs::read_to_string(&native).unwrap(),
            "{\"kept\":true}\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn terminal_exit_waits_for_the_run_and_ends_its_processes() {
        use std::os::unix::process::CommandExt;

        fn running(pid: u32) -> bool {
            let output = std::process::Command::new("/bin/ps")
                .args(["-p", &pid.to_string(), "-o", "stat="])
                .env_clear()
                .output()
                .unwrap();
            let status = String::from_utf8(output.stdout).unwrap();
            let status = status.trim();
            !status.is_empty() && !status.starts_with('Z')
        }
        fn read_pid(path: &Path) -> Option<u32> {
            std::fs::read_to_string(path).ok()?.trim().parse().ok()
        }

        let (config, host) = fixture();
        let mut sentinel = std::process::Command::new("/bin/sleep")
            .arg("60")
            .process_group(0)
            .spawn()
            .unwrap();
        let (parent_file, child_file) = (
            config.path().join("parent.pid"),
            config.path().join("child.pid"),
        );
        let sid = id();
        let run = control(&sid);
        host.register_run(run.clone(), host.generation(), || Ok(()))
            .unwrap();
        let mut command = tokio::process::Command::new("/bin/sh");
        process::clean_environment(&mut command);
        command
            .args([
                "-c",
                "echo $$ > \"$PARENT\"; /bin/sleep 60 & echo $! > \"$CHILD\"; wait",
            ])
            .env("PARENT", &parent_file)
            .env("CHILD", &child_file);
        let completed = Arc::new(AtomicBool::new(false));
        let worker = tokio::runtime::Runtime::new().unwrap();
        let task = worker.spawn({
            let (host, run, completed) = (host.clone(), run.clone(), completed.clone());
            async move {
                let mut child = process::Child::spawn(command).unwrap();
                let mut cancel = run.cancel.subscribe();
                let _ = cancel.wait_for(|cancelled| *cancelled).await;
                child.stop().await;
                // Ending the process alone does not prove that exit waited for
                // the run's remaining saving and cleanup.
                tokio::time::sleep(Duration::from_millis(250)).await;
                completed.store(true, Ordering::SeqCst);
                host.runs.lock().unwrap().remove(&run.session_id);
            }
        });
        host.tasks.lock().unwrap().insert(run.run_id.clone(), task);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (parent, child) = loop {
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

        let generation = host.generation();
        assert!(!host.handle_terminal_exit(&tauri::RunEvent::Ready));
        assert_eq!(host.generation(), generation);
        assert!(!run.cancelled());
        assert!(host.handle_terminal_exit(&tauri::RunEvent::Exit));
        assert!(completed.load(Ordering::SeqCst));
        assert!(host.runs.lock().unwrap().is_empty());
        assert!(!running(parent) && !running(child));
        assert!(
            sentinel.try_wait().unwrap().is_none(),
            "unrelated process survives"
        );
        assert_eq!(
            host.ensure_accepting_runs().unwrap_err(),
            "assistant_shutting_down"
        );
        let _ = sentinel.kill();
        let _ = sentinel.wait();
    }
}
