//! The assistant host: who may start a run, the one runtime process each run
//! owns, and the projection of the runtime's native history into
//! `session.json`. The application side (sign-in, the assistant key, the model
//! catalog, where files live, where events go) comes in through
//! [`Environment`], so the host runs the same against the app and in tests.
use super::{
    catalog, process,
    projection::{
        add_usage, is_dangling, merge_native, record_attachment_outcome, save_projection,
        settle_interrupted, settle_run, AttachmentOutcome, ProjectionBuffer,
    },
    store::AssistantStore,
    types::*,
    web,
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
/// A web search may or may not have run (and been billed); the runtime ended
/// the run rather than repeat it.
const SEARCH_OUTCOME_UNKNOWN: &str = "assistant_search_outcome_unknown";
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
    /// The usable rows of one of `apex`'s model catalogs, every kind.
    fn models<'a>(
        &'a self,
        apex: &'a str,
        source: catalog::Source,
    ) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>>;
    /// The LLM gateway base of `apex`.
    fn gateway_base(&self, apex: &str) -> Result<String, String>;
    /// The folder that holds `assistant/`.
    fn config_dir(&self) -> PathBuf;
    /// The prepared runtime: `node/`, `runtime/src/main.mjs`, `manifest.json`.
    fn runtime_dir(&self) -> Result<PathBuf, String>;
    /// Publish an `assistant:event`.
    fn emit(&self, event: Event);
    /// The proxy for model requests, as the app's other requests take it;
    /// `None` connects directly. Blocking: it may ask the OS.
    fn proxy(&self) -> Option<RunProxy> {
        None
    }
    /// The assistant key stored for `identity`'s account, read without
    /// minting or fetching one, so a backup or export can leave it out.
    /// `None` when there is none or it cannot be read.
    fn stored_key<'a>(&'a self, _identity: &'a Identity) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }
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
    /// Each catalog source of each apex, as last read.
    models: Mutex<HashMap<(String, catalog::Source), CachedSource>>,
    /// Every assistant key this process handed to a run: a draft or a title
    /// loses exactly these.
    keys: Mutex<Vec<String>>,
}

/// One catalog source's rows and when they were read.
type CachedSource = (std::time::Instant, Vec<catalog::Model>);

fn session_key(owner: &str, sid: &str) -> String {
    format!("{owner}:{sid}")
}

/// Run store IO (blocking file IO under blocking locks) or a keychain read
/// off the async workers.
pub async fn blocking<T: Send + 'static>(
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

/// The codes that stop a turn before its request (assistant-runtime README,
/// "Outcome events": every outcome whose run does not continue).
const BLOCKING_INPUT_CODES: [&str; 5] = [
    "assistant_attachment_unreadable",
    "assistant_attachment_not_found",
    "assistant_attachment_unsupported",
    "assistant_attachment_too_large",
    "assistant_vision_model_required",
];

/// The code at the head of `detail`, the runtime's `<code>: <message>`, when
/// it is one of `codes`.
fn leading_code<'a>(detail: &'a str, codes: &[&str]) -> Option<&'a str> {
    let end = detail
        .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
        .unwrap_or(detail.len());
    let (code, rest) = detail.split_at(end);
    ((rest.is_empty() || rest.starts_with(':')) && codes.contains(&code)).then_some(code)
}

/// The code a turn's attachments stopped it with before its request.
fn blocked_input_code(detail: &str) -> Option<&str> {
    leading_code(detail, &BLOCKING_INPUT_CODES)
}

/// A failed model request as a stable code. The runtime reports the HTTP
/// status (`errorStatus`) or that no answer came (`errorKind`: `not_sent` when
/// the request never left the machine, `network` when it was sent); older or
/// unforeseen forms fall back to the text, for the code only. A turn its
/// attachments blocked (never sent) keeps their code. The cost is a known
/// zero only for a request refused with a 4xx or never sent, and only when
/// the attempt had not already shown text or counted tokens. A run a web
/// search with an unknown outcome ended (the runtime's
/// `search_outcome_unknown: …`) may have been billed whatever else it says.
fn provider_failure(result: &Value, detail: String, progressed: bool) -> Failure {
    if leading_code(&detail, &["search_outcome_unknown"]).is_some() {
        return Failure {
            code: SEARCH_OUTCOME_UNKNOWN.into(),
            detail: Some(detail),
            unbilled: false,
            native: None,
        };
    }
    let status = result["errorStatus"]
        .as_u64()
        .and_then(|s| u16::try_from(s).ok())
        .or_else(|| leading_status(&detail));
    let kind = &result["errorKind"];
    let not_sent = kind == "not_sent";
    let network = kind == "network"
        || (not_sent && result["errorCode"].is_string())
        || (status.is_none() && kind.is_null() && is_network_error(&detail));
    let blocked = not_sent
        .then(|| blocked_input_code(&detail))
        .flatten()
        .map(str::to_owned);
    let code = blocked.unwrap_or_else(|| {
        match status {
            Some(401) => KEY_REJECTED,
            Some(402) => INSUFFICIENT_BALANCE,
            Some(404) => MODEL_UNAVAILABLE,
            Some(429) => RATE_LIMITED,
            Some(403) if mentions_balance(&detail) => INSUFFICIENT_BALANCE,
            _ if is_ofox_balance_error(&detail) => INSUFFICIENT_BALANCE,
            _ if network => NETWORK_ERROR,
            _ => MODEL_REQUEST_FAILED,
        }
        .to_owned()
    });
    Failure {
        code,
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

    /// Whether a run of `owner`'s conversations is live in this process.
    pub(super) fn has_live_run(&self, owner: &str) -> bool {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .any(|run| run.identity.owner == owner)
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

    /// `apex`'s model catalog, both sources merged. Each source is fetched
    /// again after [`MODELS_TTL`]; when that fetch fails, its last copy is
    /// better than none, and a source with no copy is named in `partial`.
    pub(super) async fn models(&self, apex: &str) -> Result<catalog::Catalog, String> {
        self.catalog(apex, false).await
    }

    async fn catalog(&self, apex: &str, refresh: bool) -> Result<catalog::Catalog, String> {
        let (v1, anthropic) = tokio::join!(
            self.catalog_source(apex, catalog::Source::V1, refresh),
            self.catalog_source(apex, catalog::Source::Anthropic, refresh),
        );
        catalog::merge(v1, anthropic)
    }

    async fn catalog_source(
        &self,
        apex: &str,
        source: catalog::Source,
        refresh: bool,
    ) -> Result<Vec<catalog::Model>, String> {
        let key = (apex.to_owned(), source);
        let cached = self.models.lock().map_err(|_| STATE)?.get(&key).cloned();
        if let Some((fetched, models)) = &cached {
            if !refresh && fetched.elapsed() < MODELS_TTL {
                return Ok(models.clone());
            }
        }
        match self.env.models(apex, source).await {
            Ok(models) => {
                self.models
                    .lock()
                    .map_err(|_| STATE)?
                    .insert(key, (std::time::Instant::now(), models.clone()));
                Ok(models)
            }
            Err(error) => cached.map(|(_, models)| models).ok_or(error),
        }
    }

    /// One conversation model of `apex`'s catalog; a model the cached catalog
    /// lacks is looked up once more in a fresh one.
    pub(super) async fn model(&self, apex: &str, id: &str) -> Result<catalog::Model, String> {
        let find = |merged: catalog::Catalog| {
            merged
                .conversation_models()
                .into_iter()
                .find(|m| m.id == id)
        };
        if let Some(model) = find(self.models(apex).await?) {
            return Ok(model);
        }
        find(self.catalog(apex, true).await?).ok_or_else(|| MODEL_UNAVAILABLE.into())
    }

    fn forget_models(&self, apex: &str) {
        if let Ok(mut models) = self.models.lock() {
            models.retain(|(cached, _), _| cached != apex);
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
                    if let Err(error) = store.cleanup_restore_staging() {
                        log::warn!("[Assistant] restore staging cleanup failed: {error}");
                    }
                    if let Err(error) = store.cleanup_deleted() {
                        log::warn!("[Assistant] permanent deletion cleanup failed: {error}");
                    }
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
        // Imports share this file lock, so their unpublished copies cannot be
        // mistaken for crash leftovers even if startup overlaps the UI.
        for session in sessions {
            let Ok(store) = self.store(&identity.owner) else {
                continue;
            };
            let sid = session.id;
            if let Err(error) = blocking(move || store.cleanup_orphan_attachments(&sid)).await {
                log::warn!("[Assistant] orphan attachment cleanup failed: {error}");
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
                    search_price: None,
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
    /// still revoking keys or the app is shutting down. The authority stays
    /// as it is: the `ofox-auth-restored` event of this sign-in already
    /// cancelled runs, and the renderer may have read the authority since.
    pub fn sign_in_completed(&self) {
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

    /// `web_search`: whether the run was given web search (the session's
    /// switch may be on while the catalog cannot price it).
    pub(super) fn emit_start(&self, run: &RunControl, web_search: bool) {
        self.emit(
            run,
            "run_start",
            json!({"messageId": run.message_id, "webSearch": web_search}),
        );
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
            "tool_start" => {
                // The model's tool call has been answered with tokens, and the
                // tool may cost something: never repeat this attempt.
                attempt.progressed = true;
                // As for text: the query is the model's, the keys are not.
                scrub_secrets_in(&mut data, &control.secrets());
                let mut shown = web::tool_start_event(&data)?;
                shown["messageId"] = json!(control.message_id);
                self.emit(control, kind, shown);
            }
            "tool_end" => {
                attempt.progressed = true;
                scrub_secrets_in(&mut data, &control.secrets());
                let mut shown = web::tool_end_event(&data)?;
                shown["messageId"] = json!(control.message_id);
                if data["name"] == "web_search" && data["success"] == true {
                    let found = web::search_sources(&control.run_id, &data["details"]);
                    if let Some(sources) = self.save_sources(control, found).await? {
                        self.emit(
                            control,
                            "sources",
                            json!({"messageId": control.message_id, "sources": sources}),
                        );
                    }
                }
                self.emit(control, kind, shown);
            }
            "attachment" => {
                // The whole outcome loses the run's keys before it is read.
                scrub_secrets_in(&mut data, &control.secrets());
                let outcome = AttachmentOutcome::parse(&data)?;
                if self.save_attachment_outcome(control, &outcome).await? {
                    self.emit(control, "attachment", outcome.to_event());
                } else {
                    log::warn!("[Assistant] ignored an outcome for a file this turn did not send");
                }
            }
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

    /// Add a search's sources to the run's reply, without the run's keys.
    /// Returns all of the reply's sources when any was new, else `None`.
    async fn save_sources(
        &self,
        control: &RunControl,
        mut found: Vec<Value>,
    ) -> Result<Option<Vec<Value>>, String> {
        if found.is_empty() {
            return Ok(None);
        }
        let secrets = control.secrets();
        for source in &mut found {
            scrub_secrets_in(source, &secrets);
        }
        let store = self.store(&control.identity.owner)?;
        let (sid, mid) = (control.session_id.clone(), control.message_id.clone());
        blocking(move || {
            store.mutate(&sid, |record| {
                let message = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == mid)
                    .ok_or("assistant_message_not_found")?;
                if !web::add_sources(message, found) {
                    return Ok(None);
                }
                let sources = message.sources.clone();
                record.session.updated_at = now();
                Ok(Some(sources))
            })
        })
        .await
    }

    /// Saved before it is published, so a reload shows what the user saw.
    /// `false`: the file is not one this turn sent, and nothing was saved.
    async fn save_attachment_outcome(
        &self,
        control: &RunControl,
        outcome: &AttachmentOutcome,
    ) -> Result<bool, String> {
        let store = self.store(&control.identity.owner)?;
        let (sid, run_id, outcome) = (
            control.session_id.clone(),
            control.run_id.clone(),
            outcome.clone(),
        );
        blocking(move || {
            store.mutate(&sid, |record| {
                Ok(record_attachment_outcome(record, &run_id, &outcome))
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
        let error_code = outcome.as_ref().err().map(|failure| failure.code.clone());
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
                        settle_run(
                            record,
                            &control,
                            completed,
                            stopped,
                            unbilled,
                            error_code.as_deref(),
                        )
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
    /// `/v1` catalog whose fetches are counted, an empty Anthropic one; no key
    /// or runtime.
    struct FixedEnvironment {
        config: PathBuf,
        catalog: Mutex<Vec<String>>,
        /// `/v1` fetches.
        fetches: AtomicU64,
        anthropic: Mutex<Vec<String>>,
        anthropic_fetches: AtomicU64,
        /// Sources whose fetch fails.
        down: Mutex<HashSet<catalog::Source>>,
        /// Rows of other kinds, after the `/v1` chat models.
        media: Mutex<Vec<catalog::Model>>,
        runtime: Mutex<Option<PathBuf>>,
        events: Mutex<Vec<Event>>,
        stored_key: Mutex<Option<String>>,
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
        fn models<'a>(
            &'a self,
            _: &'a str,
            source: catalog::Source,
        ) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>> {
            Box::pin(async move {
                let (fetches, ids) = match source {
                    catalog::Source::V1 => (&self.fetches, &self.catalog),
                    catalog::Source::Anthropic => (&self.anthropic_fetches, &self.anthropic),
                };
                fetches.fetch_add(1, Ordering::SeqCst);
                if self.down.lock().unwrap().contains(&source) {
                    return Err("assistant_catalog_unavailable".to_string());
                }
                let mut models: Vec<_> = ids.lock().unwrap().iter().map(|id| model(id)).collect();
                if source == catalog::Source::V1 {
                    models.extend(self.media.lock().unwrap().iter().cloned());
                }
                Ok(models)
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
        fn emit(&self, event: Event) {
            self.events.lock().unwrap().push(event);
        }
        fn stored_key<'a>(&'a self, _: &'a Identity) -> BoxFuture<'a, Option<String>> {
            Box::pin(async { self.stored_key.lock().unwrap().clone() })
        }
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
            anthropic: Mutex::default(),
            anthropic_fetches: AtomicU64::new(0),
            down: Mutex::default(),
            media: Mutex::default(),
            runtime: Mutex::new(None),
            events: Mutex::default(),
            stored_key: Mutex::default(),
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
                            search_price: None,
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

    #[tokio::test]
    async fn a_read_between_the_restored_event_and_the_end_of_sign_in_stays_current() {
        let (_config, host) = fixture();
        let sid = host
            .store(&identity().owner)
            .unwrap()
            .create("test/model", false)
            .unwrap()
            .session
            .id;
        let receipt = host.begin_logout();
        host.finish_logout(receipt);
        // The device flow emits `ofox-auth-restored` (its listener cancels
        // runs) and the renderer reads the authority before
        // `ofox_poll_for_token` returns and completes the sign-in.
        host.cancel_all();
        let boot = host.bootstrap().await.unwrap();
        host.sign_in_completed();
        let update: UpdateRequest = serde_json::from_value(json!({
            "sessionId": sid,
            "draft": "kept",
            "expectedOwner": boot["owner"],
            "expectedGeneration": boot["authorityGeneration"],
        }))
        .unwrap();
        assert_eq!(host.update_session(update).await.unwrap().draft, "kept");
        assert!(host.ensure_accepting_runs().is_ok());
    }

    #[tokio::test]
    async fn a_message_or_an_edit_over_the_limit_is_too_large() {
        let (_config, host) = fixture();
        for branch in [json!({}), json!({"editMessageId": id()})] {
            let mut value = json!({
                "sessionId": id(), "runId": id(), "operationId": id(),
                "text": "x".repeat(128_001),
            });
            value
                .as_object_mut()
                .unwrap()
                .extend(branch.as_object().unwrap().clone());
            let request: SendRequest = serde_json::from_value(value).unwrap();
            assert_eq!(
                host.send(request).await.unwrap_err(),
                "assistant_message_too_large"
            );
        }
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
                search_price: None,
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
        // A turn its attachments blocked before its request keeps their code.
        for (text, expected) in [
            (
                "assistant_vision_model_required: This model cannot read images; choose a vision model",
                "assistant_vision_model_required",
            ),
            ("assistant_vision_model_required", "assistant_vision_model_required"),
            (
                "assistant_attachment_unreadable: The attachment could not be read",
                "assistant_attachment_unreadable",
            ),
            (
                "assistant_attachment_too_large: A message holds at most 10 images",
                "assistant_attachment_too_large",
            ),
            (
                "assistant_attachment_not_found: Unknown attachment",
                "assistant_attachment_not_found",
            ),
            (
                "assistant_attachment_unsupported: Not a readable format",
                "assistant_attachment_unsupported",
            ),
            // Not a blocking attachment code: the generic failure.
            ("assistant_attachment_: x", MODEL_REQUEST_FAILED),
            ("assistant_attachment_unreadable2x: x", MODEL_REQUEST_FAILED),
            ("assistant_attachment_partial: x", MODEL_REQUEST_FAILED),
            ("assistant_attachment_write_failed: x", MODEL_REQUEST_FAILED),
            ("assistant_attachment_unreadable x", MODEL_REQUEST_FAILED),
            ("assistant_vision_model_required_soon: x", MODEL_REQUEST_FAILED),
            ("A message supports at most 10 images", MODEL_REQUEST_FAILED),
        ] {
            assert_eq!(
                failure(json!({"errorKind": "not_sent"}), text),
                code(expected, true),
                "{text}"
            );
        }
        // Only a turn that was never sent: other text cannot claim the code.
        assert_eq!(
            failure(json!({}), "assistant_vision_model_required: x"),
            code(MODEL_REQUEST_FAILED, false)
        );
        // A search that may have run ended the run: never free, whatever
        // else the last request reported, and never a network error.
        for result in [
            json!({}),
            json!({"errorKind": "not_sent"}),
            json!({"errorStatus": 400}),
        ] {
            assert_eq!(
                failure(
                    result.clone(),
                    "search_outcome_unknown: The web search may or may not have run and been billed: the request timed out. Do not repeat it automatically."
                ),
                code(SEARCH_OUTCOME_UNKNOWN, false),
                "{result}"
            );
        }
        assert_eq!(
            failure(json!({}), "search_outcome_unknownx: x").0,
            MODEL_REQUEST_FAILED
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

    /// An outcome is saved on the file and on its turn, without the run's
    /// key, before the renderer hears of it.
    #[tokio::test]
    async fn an_attachment_outcome_is_saved_then_published() {
        let (_config, env, host) = fixture_with_env();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let run = control(&sid);
        let file = id();
        let attachment = json!({
            "id": file, "name": "a.txt", "mimeType": "text/plain", "sizeBytes": 2,
            "status": "ready",
        });
        store
            .mutate(&sid, |record| {
                record
                    .attachments
                    .push(serde_json::from_value(attachment.clone()).unwrap());
                record.messages.push(
                    serde_json::from_value(json!({
                        "id": id(), "role": "user", "content": "Read it", "createdAt": 1,
                        "status": "completed", "runId": run.run_id, "attachments": [attachment],
                    }))
                    .unwrap(),
                );
                Ok(())
            })
            .unwrap();
        let event = |data: Value| {
            json!({
                "sessionId": sid, "runId": run.run_id, "operationId": run.operation_id,
                "type": "attachment", "data": data,
            })
        };
        let data = json!({
            "id": file, "status": "unreadable", "code": "assistant_attachment_unreadable",
            "details": {"format": "text", "reason": "invalid_utf8", "seen": "test-opaque-run-key"},
        });
        host.on_event(&run, &event(data), &mut Attempt::default())
            .await
            .unwrap();

        let detail = store.load_detail(&sid).unwrap();
        for saved in [&detail.attachments[0], &detail.messages[0].attachments[0]] {
            assert_eq!(saved.status, "unreadable");
            assert_eq!(
                saved.reason.as_deref(),
                Some("assistant_attachment_unreadable")
            );
            let details = saved.details.as_ref().unwrap();
            assert_eq!(details["reason"], "invalid_utf8");
            assert_eq!(details["seen"], crate::redact::REDACTED);
        }
        let published = env.events.lock().unwrap().pop().unwrap();
        assert_eq!(published.r#type, "attachment");
        assert_eq!(published.run_id, run.run_id);
        assert_eq!(
            published.data,
            json!({
                "id": file, "status": "unreadable", "code": "assistant_attachment_unreadable",
                "details": {"format": "text", "reason": "invalid_utf8", "seen": crate::redact::REDACTED},
            })
        );

        let malformed = event(json!({"id": file, "status": "lost"}));
        let refused = host
            .on_event(&run, &malformed, &mut Attempt::default())
            .await
            .unwrap_err();
        assert_eq!(refused.code, PROTOCOL);
    }

    /// The whole outcome loses the run's key before it is read, and only a
    /// file this turn sent takes it.
    #[tokio::test]
    async fn an_attachment_outcome_is_scrubbed_whole_and_kept_only_for_this_turns_files() {
        let (_config, env, host) = fixture_with_env();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        // A key that would pass for an outcome code.
        const KEY: &str = "assistant_run_key_0123456789";
        let request: SendRequest = serde_json::from_value(json!({
            "sessionId": sid, "runId": id(), "operationId": id(), "text": "hi",
        }))
        .unwrap();
        let run = Arc::new(RunControl::new(identity(), &request, KEY.into()));
        let (sent, other) = (id(), id());
        let attachment = |file: &str| {
            json!({
                "id": file, "name": "a.txt", "mimeType": "text/plain", "sizeBytes": 2,
                "status": "ready",
            })
        };
        store
            .mutate(&sid, |record| {
                for file in [&sent, &other] {
                    record
                        .attachments
                        .push(serde_json::from_value(attachment(file)).unwrap());
                }
                record.messages.push(
                    serde_json::from_value(json!({
                        "id": id(), "role": "user", "content": "Read it", "createdAt": 1,
                        "status": "completed", "runId": run.run_id,
                        "attachments": [attachment(&sent)],
                    }))
                    .unwrap(),
                );
                Ok(())
            })
            .unwrap();
        let event = |data: Value| {
            json!({
                "sessionId": sid, "runId": run.run_id, "operationId": run.operation_id,
                "type": "attachment", "data": data,
            })
        };
        let before = std::fs::read(
            store
                .native_history_path(&sid)
                .unwrap()
                .with_file_name("session.json"),
        )
        .unwrap();

        let keyed = event(json!({"id": sent, "status": "partial", "code": KEY, "details": null}));
        let refused = host
            .on_event(&run, &keyed, &mut Attempt::default())
            .await
            .unwrap_err();
        assert_eq!(refused.code, PROTOCOL);

        // Another turn's file: neither saved nor published.
        let foreign = event(json!({
            "id": other, "status": "unreadable", "code": "assistant_attachment_unreadable",
            "details": null,
        }));
        host.on_event(&run, &foreign, &mut Attempt::default())
            .await
            .unwrap();
        let after = std::fs::read(
            store
                .native_history_path(&sid)
                .unwrap()
                .with_file_name("session.json"),
        )
        .unwrap();
        assert_eq!(before, after, "nothing was saved");
        assert!(
            env.events.lock().unwrap().is_empty(),
            "nothing was published"
        );
    }

    /// Tool calls are shown as progress; a search's sources are saved on the
    /// run's reply before they are published.
    #[tokio::test]
    async fn tool_progress_is_published_and_search_sources_are_kept_on_the_reply() {
        let (_config, env, host) = fixture_with_env();
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
        let event = |kind: &str, data: Value| {
            json!({
                "sessionId": sid, "runId": run.run_id, "operationId": run.operation_id,
                "type": kind, "data": data,
            })
        };
        let found = json!({
            "usage": {"searchCalls": 1, "inputTokens": 10, "outputTokens": 5, "model": "openai/gpt-5-nano"},
            "sources": [
                {"title": "Pi", "url": "https://pi.dev/", "snippet": "test-opaque-run-key"},
                {"title": "Plain", "url": "http://plain.example.com/"},
            ],
        });
        let mut attempt = Attempt::default();
        for (kind, data) in [
            (
                "tool_start",
                json!({"name": "web_search", "toolCallId": "call-1", "args": {"query": "Pi"}}),
            ),
            (
                "tool_end",
                json!({"name": "web_search", "toolCallId": "call-1", "success": true, "details": found}),
            ),
            // The same query again: the same pages, nothing new to keep.
            (
                "tool_end",
                json!({"name": "web_search", "toolCallId": "call-2", "success": true, "details": found}),
            ),
            (
                "tool_end",
                json!({"name": "read_attachment", "toolCallId": "call-3", "success": false,
                       "message": "assistant_attachment_not_found: Unknown attachment"}),
            ),
        ] {
            host.on_event(&run, &event(kind, data), &mut attempt)
                .await
                .unwrap();
        }
        assert!(attempt.progressed, "a tool call may have cost something");

        let saved = store.load_detail(&sid).unwrap().messages[0].sources.clone();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0]["url"], "https://pi.dev/");
        assert_eq!(saved[0]["excerpt"], crate::redact::REDACTED);
        let published: Vec<(String, Value)> = env
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| (e.r#type.clone(), e.data.clone()))
            .collect();
        let kinds: Vec<&str> = published.iter().map(|(kind, _)| kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["tool_start", "sources", "tool_end", "tool_end", "tool_end"]
        );
        assert_eq!(
            published[0].1,
            json!({"messageId": run.message_id, "name": "web_search", "toolCallId": "call-1",
                   "args": {"query": "Pi"}})
        );
        assert_eq!(
            published[1].1,
            json!({"messageId": run.message_id, "sources": saved})
        );
        assert_eq!(published[2].1["success"], true);
        assert_eq!(
            published[4].1,
            json!({"messageId": run.message_id, "name": "read_attachment", "toolCallId": "call-3",
                   "success": false, "code": "assistant_attachment_not_found"})
        );

        let malformed = event("tool_end", json!({"name": "web_search", "success": true}));
        assert_eq!(
            host.on_event(&run, &malformed, &mut attempt)
                .await
                .unwrap_err()
                .code,
            PROTOCOL
        );

        // A query never shows the run's key.
        let keyed = event(
            "tool_start",
            json!({"name": "web_search", "toolCallId": "call-4",
                   "args": {"query": "about test-opaque-run-key"}}),
        );
        host.on_event(&run, &keyed, &mut attempt).await.unwrap();
        let shown = env.events.lock().unwrap().last().unwrap().data.clone();
        assert_eq!(
            shown["args"]["query"],
            format!("about {}", crate::redact::REDACTED)
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
        age_catalogs(&host);
        host.models("ofox.ai").await.unwrap();
        assert_eq!(env.fetches.load(Ordering::SeqCst), 4);

        // A run that found its model gone drops the catalog.
        host.forget_models("ofox.ai");
        host.models("ofox.ai").await.unwrap();
        assert_eq!(env.fetches.load(Ordering::SeqCst), 5);
        // Each lookup asked both sources.
        assert_eq!(env.anthropic_fetches.load(Ordering::SeqCst), 5);
    }

    /// Make every cached catalog source older than [`MODELS_TTL`].
    fn age_catalogs(host: &AssistantHost) {
        for (fetched, _) in host.models.lock().unwrap().values_mut() {
            *fetched -= MODELS_TTL;
        }
    }

    fn catalog_ids(merged: &catalog::Catalog) -> Vec<&str> {
        merged.models.iter().map(|m| m.id.as_str()).collect()
    }

    #[tokio::test]
    async fn a_catalog_source_that_fails_falls_back_to_its_own_last_copy() {
        let (_config, env, host) = fixture_with_env();
        *env.anthropic.lock().unwrap() = vec!["anthropic/claude-test".into()];
        let merged = host.models("ofox.ai").await.unwrap();
        assert_eq!(
            catalog_ids(&merged),
            ["test/model", "anthropic/claude-test"]
        );
        assert!(merged.partial.is_empty());

        // Both copies go stale and the Anthropic catalog goes down: `/v1` is
        // read again, Claude comes from its last copy.
        age_catalogs(&host);
        env.down.lock().unwrap().insert(catalog::Source::Anthropic);
        env.catalog.lock().unwrap().push("new/model".into());
        let merged = host.models("ofox.ai").await.unwrap();
        assert_eq!(
            catalog_ids(&merged),
            ["test/model", "new/model", "anthropic/claude-test"]
        );
        assert!(merged.partial.is_empty());
        assert_eq!(
            host.model("ofox.ai", "anthropic/claude-test")
                .await
                .unwrap()
                .id,
            "anthropic/claude-test"
        );

        // With no copy to fall back to, the source is named and the rest is
        // offered.
        host.forget_models("ofox.ai");
        let merged = host.models("ofox.ai").await.unwrap();
        assert_eq!(catalog_ids(&merged), ["test/model", "new/model"]);
        assert_eq!(merged.partial, [catalog::Source::Anthropic]);
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(boot["partial"], json!(["anthropic"]));
        assert_eq!(boot["models"].as_array().unwrap().len(), 2);
        assert_eq!(boot["defaultModelId"], "test/model");
        assert_eq!(boot["modelUnavailableReason"], Value::Null);

        // `/v1` down instead: Claude alone is offered.
        env.down.lock().unwrap().clear();
        env.down.lock().unwrap().insert(catalog::Source::V1);
        host.forget_models("ofox.ai");
        let merged = host.models("ofox.ai").await.unwrap();
        assert_eq!(catalog_ids(&merged), ["anthropic/claude-test"]);
        assert_eq!(merged.partial, [catalog::Source::V1]);

        // Both down with no copy: the error of before.
        env.down.lock().unwrap().insert(catalog::Source::Anthropic);
        host.forget_models("ofox.ai");
        assert_eq!(
            host.models("ofox.ai").await.unwrap_err(),
            "assistant_catalog_unavailable"
        );
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(
            boot["modelUnavailableReason"],
            "assistant_catalog_unavailable"
        );
        assert_eq!(boot["models"], json!([]));
        assert_eq!(boot["partial"], json!([]));
        assert_eq!(boot["defaultModelId"], Value::Null);
    }

    #[tokio::test]
    async fn media_models_are_kept_but_not_offered_for_conversations_yet() {
        let (_config, env, host) = fixture_with_env();
        let video: catalog::Model = serde_json::from_value(json!({
            "id": "video/model", "name": "Video", "contextWindow": 0, "maxOutputTokens": 0,
            "supportsImages": false, "inputModalities": ["text"],
            "supportedEndpoints": ["/v1/videos"], "inputPricePerMillion": null,
            "outputPricePerMillion": null, "pricing": {}, "cost": {}, "pricingKnown": false,
            "kind": "video",
        }))
        .unwrap();
        env.media.lock().unwrap().push(video);
        let merged = host.models("ofox.ai").await.unwrap();
        assert_eq!(catalog_ids(&merged), ["test/model", "video/model"]);
        assert_eq!(
            host.model("ofox.ai", "video/model").await.unwrap_err(),
            MODEL_UNAVAILABLE
        );
        let create: CreateRequest =
            serde_json::from_value(json!({"modelId": "video/model"})).unwrap();
        assert_eq!(
            host.create_session(create).await.unwrap_err(),
            MODEL_UNAVAILABLE
        );
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(boot["models"].as_array().unwrap().len(), 1);
        assert_eq!(boot["models"][0]["id"], "test/model");
        assert_eq!(boot["models"][0]["kind"], "chat");
        assert_eq!(boot["defaultModelId"], "test/model");
    }

    #[tokio::test]
    async fn aggregate_usage_keeps_trash_and_other_accounts_separate_without_getting_a_key() {
        let (config, env, host) = fixture_with_env();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        store.mutate(&sid, |record| {
            record.session.deleted_at = Some(1);
            record.usage = json!({"modelRequests":1,"inputTokens":40,"searchCount":2,"estimatedCostUsd":"0.123456789123456789"});
            Ok(())
        }).unwrap();
        let other = AssistantStore::for_owner(config.path(), &"b".repeat(64)).unwrap();
        let other_sid = other.create("test/model", false).unwrap().session.id;
        other
            .mutate(&other_sid, |record| {
                record.usage = json!({"inputTokens":999,"estimatedCostUsd":"9"});
                Ok(())
            })
            .unwrap();
        let summary = host.usage_summary().await.unwrap();
        assert_eq!(summary["inputTokens"], 40);
        assert_eq!(summary["searchCount"], 2);
        assert_eq!(summary["estimatedCostUsd"], "0.123456789123456789");
        assert_eq!(env.fetches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn backup_and_restore_dialogs_recheck_authority_and_keep_cancel_quiet() {
        let (_config, host) = fixture();
        let sid = host
            .store(&identity().owner)
            .unwrap()
            .create("test/model", false)
            .unwrap()
            .session
            .id;
        let authority = TransferRequest {
            expected_owner: identity().owner,
            expected_generation: host.generation(),
        };
        assert!(host
            .backup(authority.clone(), || Ok(None))
            .await
            .unwrap()
            .is_none());
        let wrong = TransferRequest {
            expected_owner: "b".repeat(64),
            ..authority.clone()
        };
        assert_eq!(
            host.backup(wrong, || panic!("stale owner must not open picker"))
                .await
                .unwrap_err(),
            IDENTITY_CHANGED
        );
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        let selected = archive.clone();
        let backup = host
            .backup(authority, move || Ok(Some(selected)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((backup.sessions, backup.skipped), (1, 0));
        let (_target_config, target) = fixture();
        let request = TransferRequest {
            expected_owner: identity().owner,
            expected_generation: target.generation(),
        };
        assert!(target
            .restore(request.clone(), || Ok(None))
            .await
            .unwrap()
            .is_none());
        let changed = target.clone();
        let selected = archive.clone();
        assert_eq!(
            target
                .restore(request, move || {
                    changed.generation.fetch_add(1, Ordering::SeqCst);
                    Ok(Some(selected))
                })
                .await
                .unwrap_err(),
            IDENTITY_CHANGED
        );
        assert!(target
            .list_sessions(false, "")
            .await
            .unwrap()
            .sessions
            .is_empty());
        let request = TransferRequest {
            expected_owner: identity().owner,
            expected_generation: target.generation(),
        };
        let report = target
            .restore(request, move || Ok(Some(archive)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((report.imported, report.conflicts), (1, 0));
        assert!(report.warning.is_none());
        assert_eq!(
            target.get_session(&sid).await.unwrap().session.model_id,
            "test/model"
        );
    }

    #[tokio::test]
    async fn backup_and_restore_wait_for_a_live_reply_without_opening_a_dialog() {
        let (_config, host) = fixture();
        let sid = host
            .store(&identity().owner)
            .unwrap()
            .create("test/model", false)
            .unwrap()
            .session
            .id;
        host.register_run(control(&sid), host.generation(), || Ok(()))
            .unwrap();
        let authority = TransferRequest {
            expected_owner: identity().owner,
            expected_generation: host.generation(),
        };
        assert_eq!(
            host.backup(authority.clone(), || panic!("no dialog while a reply runs"))
                .await
                .unwrap_err(),
            "assistant_transfer_busy"
        );
        assert_eq!(
            host.restore(authority, || panic!("no dialog while a reply runs"))
                .await
                .unwrap_err(),
            "assistant_transfer_busy"
        );
    }

    #[tokio::test]
    async fn backups_and_exports_leave_out_the_stored_assistant_key() {
        let (_config, env, host) = fixture_with_env();
        let key = "synthetic-stored-assistant-key-0123456789";
        *env.stored_key.lock().unwrap() = Some(key.into());
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&sid, |record| {
                record.session.draft = format!("pasted {key}");
                Ok(())
            })
            .unwrap();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        let selected = archive.clone();
        let authority = TransferRequest {
            expected_owner: identity().owner,
            expected_generation: host.generation(),
        };
        host.backup(authority, move || Ok(Some(selected)))
            .await
            .unwrap()
            .unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
        for index in 0..zip.len() {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut zip.by_index(index).unwrap(), &mut text).unwrap();
            assert!(!text.contains(key));
        }
        let path = out.path().join("export.md");
        let selected = path.clone();
        host.export_markdown(
            export_request(&host, &sid, &format!("# Chat\n\n{key}")),
            move || Ok(Some(selected)),
        )
        .await
        .unwrap();
        assert!(!std::fs::read_to_string(path).unwrap().contains(key));
    }

    #[tokio::test]
    async fn restore_refuses_an_archive_holding_the_stored_assistant_key() {
        let (_source, source) = fixture();
        let key = "synthetic-stored-assistant-key-0123456789";
        let store = source.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&sid, |record| {
                record.session.draft = format!("pasted {key}");
                Ok(())
            })
            .unwrap();
        // Written without knowing the key, as by a process that never saw it.
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let (_config, env, target) = fixture_with_env();
        *env.stored_key.lock().unwrap() = Some(key.into());
        let request = TransferRequest {
            expected_owner: identity().owner,
            expected_generation: target.generation(),
        };
        assert_eq!(
            target
                .restore(request, move || Ok(Some(archive)))
                .await
                .unwrap_err(),
            "assistant_backup_contains_key"
        );
        assert!(target
            .list_sessions(false, "")
            .await
            .unwrap()
            .sessions
            .is_empty());
    }

    fn export_request(host: &AssistantHost, sid: &str, markdown: &str) -> ExportMarkdownRequest {
        ExportMarkdownRequest {
            authority: DeleteRequest {
                session_id: sid.into(),
                expected_owner: identity().owner,
                expected_generation: host.generation(),
            },
            markdown: markdown.into(),
        }
    }

    #[tokio::test]
    async fn markdown_export_saves_privately_without_the_managed_key_and_can_be_cancelled() {
        let (_config, host) = fixture();
        let sid = host
            .store(&identity().owner)
            .unwrap()
            .create("test/model", false)
            .unwrap()
            .session
            .id;
        let destination = tempfile::tempdir().unwrap();
        let path = destination.path().join("会話.md");
        let selected = path.clone();
        host.remember_key("synthetic-export-key-1234567890");
        let request = export_request(&host, &sid, "# 会話\n\nsynthetic-export-key-1234567890");
        assert!(host
            .export_markdown(request.clone(), move || Ok(Some(selected)))
            .await
            .unwrap());
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# 会話"));
        assert!(!saved.contains("synthetic-export-key-1234567890"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(!host.export_markdown(request, || Ok(None)).await.unwrap());
        assert_eq!(std::fs::read_to_string(path).unwrap(), saved);
    }

    #[tokio::test]
    async fn markdown_export_refuses_stale_authority_and_unsafe_destinations() {
        let (config, host) = fixture();
        let sid = host
            .store(&identity().owner)
            .unwrap()
            .create("test/model", false)
            .unwrap()
            .session
            .id;
        let stale = export_request(&host, &sid, "private");
        let destination = tempfile::tempdir().unwrap();
        let path = destination.path().join("export.md");
        let selected = path.clone();
        let changed = host.clone();
        assert_eq!(
            host.export_markdown(stale.clone(), move || {
                changed.generation.fetch_add(1, Ordering::SeqCst);
                Ok(Some(selected))
            })
            .await
            .unwrap_err(),
            IDENTITY_CHANGED
        );
        assert!(!path.exists());
        assert_eq!(
            host.export_markdown(stale, || panic!("no dialog for stale authority"))
                .await
                .unwrap_err(),
            IDENTITY_CHANGED
        );
        let protected = config.path().join("settings.md");
        std::fs::write(&protected, "keep").unwrap();
        let selected = protected.clone();
        assert_eq!(
            host.export_markdown(export_request(&host, &sid, "replace"), move || Ok(Some(
                selected
            )))
            .await
            .unwrap_err(),
            "assistant_export_destination_invalid"
        );
        assert_eq!(std::fs::read_to_string(protected).unwrap(), "keep");
        #[cfg(unix)]
        {
            let target = destination.path().join("keep.md");
            std::fs::write(&target, "keep").unwrap();
            let linked = destination.path().join("link.md");
            std::os::unix::fs::symlink(&target, &linked).unwrap();
            assert_eq!(
                host.export_markdown(export_request(&host, &sid, "replace"), move || Ok(Some(
                    linked
                )))
                .await
                .unwrap_err(),
                "assistant_export_destination_invalid"
            );
            assert_eq!(std::fs::read_to_string(target).unwrap(), "keep");
        }
    }

    #[tokio::test]
    async fn permanent_deletion_checks_the_confirmed_account_and_generation() {
        let (_config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .update(
                &serde_json::from_value(json!({"sessionId":sid,"deleted":true})).unwrap(),
                || Ok(()),
            )
            .unwrap();
        let request = DeleteRequest {
            session_id: sid.clone(),
            expected_owner: identity().owner,
            expected_generation: host.generation(),
        };
        let wrong_owner = DeleteRequest {
            expected_owner: "b".repeat(64),
            ..request.clone()
        };
        assert_eq!(
            host.delete_session(wrong_owner).await.unwrap_err(),
            IDENTITY_CHANGED
        );
        let stale = DeleteRequest {
            expected_generation: request.expected_generation + 1,
            ..request.clone()
        };
        assert_eq!(
            host.delete_session(stale).await.unwrap_err(),
            IDENTITY_CHANGED
        );
        assert!(store.load_record(&sid).is_ok());
        host.delete_session(request).await.unwrap();
        assert!(store.load_record(&sid).is_err());
        assert!(host
            .list_sessions(true, "")
            .await
            .unwrap()
            .sessions
            .is_empty());
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
