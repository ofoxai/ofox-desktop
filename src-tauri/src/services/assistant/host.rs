//! The assistant host: who may start a run, the one runtime process each run
//! owns, and the projection of the runtime's native history into
//! `session.json`. The application side (sign-in, the assistant key, the model
//! catalog, where files live, where events go) comes in through
//! [`Environment`], so the host runs the same against the app and in tests.
mod catalog_refresh;
pub mod local;
use super::{
    catalog,
    drops::DropSlot,
    process,
    projection::{
        add_usage, is_dangling, merge_native, record_attachment_outcome, save_projection,
        settle_interrupted, settle_run, AttachmentOutcome, ProjectionBuffer,
    },
    store::AssistantStore,
    title,
    types::*,
    web,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot, watch};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// Whether a run may still get a key; the error is the code to report.
pub type Admit<'a> = dyn Fn() -> Result<(), String> + Send + Sync + 'a;

pub(super) const STATE: &str = "assistant_state_unavailable";
pub const SIGN_IN_REQUIRED: &str = "assistant_sign_in_required";
pub(super) const IDENTITY_CHANGED: &str = "assistant_identity_changed";
pub(super) const CANCELLED: &str = "assistant_cancelled";
const SESSION_BUSY: &str = "assistant_session_busy";
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
pub(super) const STDIO: &str = "assistant_runtime_stdio";
pub(super) const PROTOCOL: &str = "assistant_runtime_protocol";
const RUNTIME_UNAVAILABLE: &str = "assistant_runtime_unavailable";
const RUN_DEADLINE: Duration = Duration::from_secs(900);
/// How long a fetched model catalog is trusted.
const MODELS_TTL: Duration = Duration::from_secs(600);
/// How long a send does not ask again a catalog source whose read failed: a
/// hanging host would otherwise hold up every send. A lookup that misses asks
/// at once, and so do the page's reads ([`AssistantHost::page_models`]).
const FAILED_SOURCE_RETRY: Duration = Duration::from_secs(30);
/// How long a page read waits for a source that failed before while its last
/// copy can stand in: the page opens on the copy rather than wait out a
/// hanging host's whole fetch timeout.
const PAGE_RETRY_WITH_COPY: Duration = Duration::from_secs(4);
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
    fn local_mutate<'a>(
        &'a self,
        _name: &'a str,
        _tool: crate::app_config::BindableTool,
        _model: Option<&'a str>,
        _operation: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Err("assistant_local_unavailable".into()) })
    }
    fn local_restore(&self, journal: &mut super::tools::journal::Journal) -> Result<(), String> {
        journal.restore_files()
    }
    fn local_journal_key(&self) -> BoxFuture<'_, Result<zeroize::Zeroizing<[u8; 32]>, String>> {
        Box::pin(async { Err("assistant_key_storage_unavailable".into()) })
    }

    fn local_paths(&self) -> BoxFuture<'_, Result<super::tools::context::LocalPaths, String>> {
        Box::pin(async { Err("assistant_local_unavailable".into()) })
    }
    fn local_binding_status(
        &self,
        _tool: crate::app_config::BindableTool,
    ) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(async { Ok(json!({"status":"unknown"})) })
    }
    /// Whether the tool's Ofox key is already stored locally. Rebinding without
    /// one issues a new account key, which no journal restore can undo.
    fn local_key_cached(&self, _tool: crate::app_config::BindableTool) -> bool {
        false
    }

    /// Blocking proxy-aware audio transport; tests inject a direct local client.
    fn audio_client(&self) -> Result<reqwest::Client, String> {
        crate::proxy::http_client::assistant_media_client()
    }
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
    /// Keyless, model-specific provider pricing for a media quote.
    fn provider_prices<'a>(
        &'a self,
        apex: &'a str,
        model: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(super::video_http::provider_prices(apex, model))
    }
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
    local_calls: local::Calls,
    pub(super) local_enabled: bool,
    pub identity: Identity,
    pub session_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub message_id: String,
    pub(super) compact_only: bool,
    pub(super) audio_only: bool,
    compacting: AtomicBool,
    pub(super) live_reply: Mutex<Option<String>>,
    pub(super) live_runs: Mutex<HashSet<String>>,
    live_input: Mutex<Option<mpsc::Sender<super::live::LiveCommand>>>,
    pub cancel: watch::Sender<bool>,
    pub deadline: tokio::time::Instant,
    /// Every API key this run has used (a 401 retry adds the new one). Each is
    /// scrubbed from everything the run shows or saves.
    secret: Mutex<Vec<String>>,
}

impl RunControl {
    pub fn new(identity: Identity, request: &SendRequest, secret: String) -> Self {
        Self {
            local_calls: local::Calls::default(),
            local_enabled: false,
            identity,
            session_id: request.session_id.clone(),
            run_id: request.run_id.clone(),
            operation_id: request.operation_id.clone(),
            message_id: id(),
            compact_only: false,
            audio_only: false,
            compacting: AtomicBool::new(false),
            live_reply: Mutex::new(None),
            live_runs: Mutex::new(HashSet::new()),
            live_input: Mutex::new(None),
            cancel: watch::channel(false).0,
            deadline: tokio::time::Instant::now() + RUN_DEADLINE,
            secret: Mutex::new(vec![secret]),
        }
    }

    pub(super) fn reply_id(&self) -> String {
        self.live_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| self.message_id.clone())
    }

    pub(super) fn owns_native_run(&self, run_id: &str) -> bool {
        run_id == self.run_id
            || self
                .live_runs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(run_id)
    }

    pub(super) fn secrets(&self) -> Vec<String> {
        self.secret
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn add_secret(&self, key: &str) {
        self.secret
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
    }

    pub(super) fn cancelled(&self) -> bool {
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
    /// Sessions whose native history a host operation other than a run is
    /// changing (a version switch), by session id. See [`Navigation`].
    navigating: Arc<Mutex<HashSet<String>>>,
    /// Each catalog source of each apex, as last read.
    models: Mutex<HashMap<(String, catalog::Source), CachedSource>>,
    pub(super) catalog_revision: Arc<AtomicU64>,
    catalog_pending: Arc<Mutex<HashSet<String>>>,
    catalog_locks: CatalogLocks,
    /// The files last dropped on the window, waiting for the renderer's
    /// token (see [`super::drops`]).
    pub(super) drops: Mutex<DropSlot>,
    pub(super) audio_inputs: Mutex<super::audio_input::Slot>,
    pub(super) audio_importing: tokio::sync::Mutex<()>,
    pub(super) video_quotes: Mutex<HashMap<String, super::video_host::StoredQuote>>,
    pub(super) video_admission: tokio::sync::Mutex<()>,
    pub(super) video_polling: Arc<Mutex<HashSet<String>>>,
    pub(super) video_revocation: watch::Sender<u64>,
    /// Every assistant key this process handed to a run: a draft or a title
    /// loses exactly these.
    keys: Mutex<Vec<String>>,
    local_states: Mutex<HashMap<String, local::LocalState>>,
    local_approvals: super::tools::approval::Broker,
    /// Background work at it (see [`Self::track`]), such as a title.
    background: Arc<AtomicU64>,
    /// When shutdown aborts what is still at work, once it began.
    shutdown_deadline: Mutex<Option<tokio::time::Instant>>,
}

/// How a catalog read uses what each source last returned.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CatalogRead {
    /// A send: rows younger than [`MODELS_TTL`] stand, and a source that
    /// failed is not asked again within [`FAILED_SOURCE_RETRY`].
    Send,
    /// The page: as a send, except that a failed source is asked again at
    /// once (see [`PAGE_RETRY_WITH_COPY`]).
    Page,
    /// A lookup that missed: every source is asked.
    Fresh,
}

/// What the host knows of one catalog source.
#[derive(Clone, Default)]
struct CachedSource {
    /// The rows last read, and when.
    rows: Option<(std::time::Instant, Vec<catalog::Model>)>,
    /// When a read last failed since then, and its error.
    failed: Option<(std::time::Instant, String)>,
}

type CatalogLocks = Mutex<HashMap<(String, catalog::Source), Arc<tokio::sync::Mutex<()>>>>;
pub(super) struct CatalogSnapshot {
    pub catalog: catalog::Catalog,
    pub error: Option<String>,
    pub revision: u64,
    pub refreshing: bool,
    pub failed: Vec<catalog::Source>,
}
impl CachedSource {
    fn reuse(&self, read: CatalogRead) -> Option<Result<Vec<catalog::Model>, String>> {
        if read == CatalogRead::Fresh {
            return None;
        }
        if let Some((failed, error)) = &self.failed {
            return if read == CatalogRead::Send && failed.elapsed() < FAILED_SOURCE_RETRY {
                Some(
                    self.rows
                        .as_ref()
                        .map(|(_, rows)| rows.clone())
                        .ok_or_else(|| error.clone()),
                )
            } else {
                None
            };
        }
        self.rows
            .as_ref()
            .filter(|(at, _)| at.elapsed() < MODELS_TTL)
            .map(|(_, rows)| Ok(rows.clone()))
    }
}

/// A session held by [`AssistantHost::begin_navigation`] until this drops.
pub(super) struct Navigation {
    sessions: Arc<Mutex<HashSet<String>>>,
    sid: String,
}

impl Drop for Navigation {
    fn drop(&mut self) {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.sid);
    }
}

pub(super) fn session_key(owner: &str, sid: &str) -> String {
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
    if let Some(code) = leading_code(
        &detail,
        &[
            "assistant_repeat_limit",
            "assistant_duration_limit",
            "assistant_cost_limit",
            "assistant_cost_unavailable",
            "assistant_verification_required",
            "assistant_tool_limit",
            "assistant_command_outcome_unknown",
            "assistant_mutation_outcome_unknown",
        ],
    ) {
        return Failure {
            code: code.into(),
            detail: None,
            unbilled: !progressed && result["errorKind"] == "not_sent",
            native: None,
        };
    }
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

pub(super) fn runtime_command(runtime: &Runtime) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(&runtime.node);
    command.arg("--max-old-space-size=512").arg(&runtime.entry);
    process::clean_environment(&mut command);
    command
}

/// How a run ended when it did not fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Finish {
    Completed,
    Stopped,
}

/// A failed run: a stable `assistant_*` code, and for provider failures the
/// scrubbed provider text.
#[derive(Debug)]
pub(super) struct Failure {
    pub(super) code: String,
    pub(super) detail: Option<String>,
    /// The request cost nothing: the session's cost stays known.
    pub(super) unbilled: bool,
    /// For [`RETRY_WITH_NEW_KEY`]: the refused attempt's native entries, kept
    /// if the retry does not start.
    pub(super) native: Option<Value>,
}

impl Failure {
    /// A failure before anything was sent: it cost nothing.
    pub(super) fn unsent(code: String) -> Self {
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
            navigating: Arc::default(),
            recovered: Mutex::default(),
            runs: Mutex::default(),
            models: Mutex::default(),
            catalog_revision: Arc::new(AtomicU64::new(0)),
            catalog_pending: Arc::default(),
            catalog_locks: Mutex::default(),
            drops: Mutex::default(),
            audio_inputs: Mutex::default(),
            audio_importing: tokio::sync::Mutex::new(()),
            video_quotes: Mutex::default(),
            video_admission: tokio::sync::Mutex::new(()),
            video_polling: Arc::default(),
            video_revocation: watch::channel(0).0,
            keys: Mutex::default(),
            local_states: Mutex::default(),
            local_approvals: super::tools::approval::Broker::default(),
            background: Arc::default(),
            shutdown_deadline: Mutex::default(),
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

    /// `apex`'s model catalog, both sources merged, as a send reads it. Each
    /// source is fetched again after [`MODELS_TTL`]; when that fetch fails,
    /// its last copy is better than none, and a source with no copy is named
    /// in `partial`. A failed source is asked again after
    /// [`FAILED_SOURCE_RETRY`]; the page's reads bypass that pause
    /// ([`Self::page_models`]).
    pub(super) async fn models(&self, apex: &str) -> Result<catalog::Catalog, String> {
        self.catalog(apex, CatalogRead::Send).await
    }

    /// [`Self::models`] for a read the user starts (opening the page, Retry,
    /// a refresh): a source that failed is asked again at once, and waited
    /// for only [`PAGE_RETRY_WITH_COPY`] while its last copy can stand in.
    pub(super) async fn page_models(&self, apex: &str) -> Result<catalog::Catalog, String> {
        self.catalog(apex, CatalogRead::Page).await
    }

    async fn catalog(&self, apex: &str, read: CatalogRead) -> Result<catalog::Catalog, String> {
        let (v1, anthropic) = tokio::join!(
            self.catalog_source(apex, catalog::Source::V1, read),
            self.catalog_source(apex, catalog::Source::Anthropic, read),
        );
        catalog::merge(v1, anthropic)
    }

    async fn catalog_source(
        &self,
        apex: &str,
        source: catalog::Source,
        read: CatalogRead,
    ) -> Result<Vec<catalog::Model>, String> {
        let begun = std::time::Instant::now();
        let key = (apex.to_owned(), source);
        let cached = self
            .models
            .lock()
            .map_err(|_| STATE)?
            .get(&key)
            .cloned()
            .unwrap_or_default();
        if let Some(result) = cached.reuse(read) {
            return result;
        }
        let lock = self
            .catalog_locks
            .lock()
            .map_err(|_| STATE)?
            .entry(key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _source = match (read, cached.failed.as_ref(), cached.rows.as_ref()) {
            (CatalogRead::Page, Some(_), Some((_, rows))) => {
                match tokio::time::timeout(PAGE_RETRY_WITH_COPY, lock.lock_owned()).await {
                    Ok(guard) => guard,
                    Err(_) => return Ok(rows.clone()),
                }
            }
            _ => lock.lock_owned().await,
        };
        let cached = self
            .models
            .lock()
            .map_err(|_| STATE)?
            .get(&key)
            .cloned()
            .unwrap_or_default();
        if let Some(result) = cached.reuse(read) {
            return result;
        }
        if read == CatalogRead::Fresh {
            if let Some((at, rows)) = &cached.rows {
                if *at >= begun {
                    return Ok(rows.clone());
                }
            }
        }
        let fetch = self.env.models(apex, source);
        let copy_stands_in = cached.failed.is_some() && cached.rows.is_some();
        let fetched = if read == CatalogRead::Page && copy_stands_in {
            tokio::time::timeout(PAGE_RETRY_WITH_COPY, fetch)
                .await
                .unwrap_or_else(|_| Err("assistant_catalog_unavailable".into()))
        } else {
            fetch.await
        };
        let now = std::time::Instant::now();
        let mut cache = self.models.lock().map_err(|_| STATE)?;
        let entry = cache.entry(key).or_default();
        self.catalog_revision.fetch_add(1, Ordering::SeqCst);
        match fetched {
            Ok(models) => {
                *entry = CachedSource {
                    rows: Some((now, models.clone())),
                    failed: None,
                };
                Ok(models)
            }
            Err(error) => {
                entry.failed = Some((now, error.clone()));
                entry.rows.clone().map(|(_, models)| models).ok_or(error)
            }
        }
    }

    /// One conversation model of `apex`'s catalog; a model the cached catalog
    /// lacks is looked up once more in a fresh one.
    pub(super) async fn model(&self, apex: &str, id: &str) -> Result<catalog::Model, String> {
        if let Some(model) = self.models(apex).await?.conversation_model(id) {
            return Ok(model);
        }
        self.catalog(apex, CatalogRead::Fresh)
            .await?
            .conversation_model(id)
            .ok_or_else(|| MODEL_UNAVAILABLE.into())
    }

    fn forget_models(&self, apex: &str) {
        if let Ok(mut models) = self.models.lock() {
            models.retain(|(cached, _), _| cached != apex);
            self.catalog_revision.fetch_add(1, Ordering::SeqCst);
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

    /// The lock over one conversation's native history (see [`session_key`]):
    /// its runs, its recovery and its titles take turns.
    pub(super) fn native_lock(&self, key: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
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
    pub async fn recover_all(self: &Arc<Self>) {
        let Ok(identity) = self.identity().await else {
            return;
        };
        let listed = match self.store(&identity.owner) {
            Ok(store) => {
                blocking(move || {
                    if let Err(error) = store.cleanup_downloads() {
                        log::warn!("[Assistant] media download cleanup failed: {error}");
                    }
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
        for session in &sessions {
            let Ok(store) = self.store(&identity.owner) else {
                continue;
            };
            let sid = session.id.clone();
            if let Err(error) = blocking(move || store.cleanup_orphan_attachments(&sid)).await {
                log::warn!("[Assistant] orphan attachment cleanup failed: {error}");
            }
        }
        self.resume_video_jobs().await;
    }

    // ── runs ─────────────────────────────────────────────────────────────

    /// The live run is the only truth for `activeRun`; the saved value can be
    /// left over from a process that died.
    pub(super) fn decorate(&self, owner: &str, session: &mut Session) {
        let active = self.runs.lock().ok().and_then(|runs| {
            runs.get(&session.id)
                .filter(|r| r.identity.owner == owner)
                .cloned()
        });
        session.audio_only = active.as_ref().filter(|run| run.audio_only).map(|_| true);
        session.compacting = active
            .as_ref()
            .filter(|run| run.compact_only || run.compacting.load(Ordering::SeqCst))
            .map(|_| true);
        session.active_run = active.map(|run| RunRef {
            run_id: run.run_id.clone(),
            operation_id: run.operation_id.clone(),
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
                    media: None,
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
        if runs.contains_key(&run.session_id)
            || self
                .navigating
                .lock()
                .map_err(|_| STATE)?
                .contains(&run.session_id)
        {
            return Err(SESSION_BUSY.into());
        }
        publish()?;
        runs.insert(run.session_id.clone(), run);
        Ok(())
    }

    /// Hold a session's native history for a host operation that changes it
    /// outside a run, such as a version switch: refused while the session
    /// has a live run or is already held, and while it is held no run of it
    /// registers ([`Self::register_run`]). Released when the [`Navigation`]
    /// drops. Taken once the session's native lock is held, so that waiting
    /// for the lock (a title writing the history) holds no send up.
    pub(super) fn begin_navigation(&self, sid: &str) -> Result<Navigation, String> {
        // The runs lock first, as registration takes them.
        let runs = self.runs.lock().map_err(|_| STATE)?;
        let mut navigating = self.navigating.lock().map_err(|_| STATE)?;
        if runs.contains_key(sid) || !navigating.insert(sid.to_owned()) {
            return Err(SESSION_BUSY.into());
        }
        Ok(Navigation {
            sessions: self.navigating.clone(),
            sid: sid.to_owned(),
        })
    }

    /// Refused while `sid` has a live run.
    pub(super) fn ensure_idle(&self, sid: &str) -> Result<(), String> {
        if self.runs.lock().map_err(|_| STATE)?.contains_key(sid) {
            return Err(SESSION_BUSY.into());
        }
        Ok(())
    }

    /// Bring a conversation's messages up to date with its native history,
    /// under its native lock. A run that failed without saving what the
    /// runtime wrote (its process died, or the host failed before reading
    /// the result) leaves a turn there that the messages hold without native
    /// entries. Refused while the conversation replies.
    pub(super) async fn sync_native(&self, identity: &Identity, sid: &str) -> Result<(), String> {
        self.ensure_idle(sid)?;
        let _history = self
            .native_lock(&session_key(&identity.owner, sid))?
            .lock_owned()
            .await;
        // Admission can change while the native lock is held by another
        // operation. Fence new runs until this projection has been saved.
        let _navigation = self.begin_navigation(sid)?;
        let native = blocking({
            let store = self.store(&identity.owner)?;
            let sid = sid.to_owned();
            move || store.native_history_path(&sid)
        })
        .await?;
        if !native.is_file() {
            return Ok(());
        }
        let entries = load_native(&self.runtime()?, &native).await?;
        if self.identity().await? != *identity {
            return Err(IDENTITY_CHANGED.into());
        }
        let store = self.store(&identity.owner)?;
        let (sid, keys) = (sid.to_owned(), self.known_keys());
        blocking(move || store.mutate(&sid, |record| merge_native(record, &entries, &keys))).await
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

    pub(super) fn validate_run(&self, run: &RunControl) -> Result<(), String> {
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
    pub fn sign_in_completed(self: &Arc<Self>) {
        let allowed = {
            let _registration = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
            let mut admission = self
                .admission
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if !admission.shutting_down && admission.logouts.is_empty() {
                admission.needs_sign_in = false;
                true
            } else {
                false
            }
        };
        if allowed {
            self.schedule_video_resume();
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

    /// Cancel every run, give each (and background work) a moment to save,
    /// then abort what is left; aborting drops the owned process, which kills
    /// its process group or Job.
    pub async fn shutdown(&self) {
        self.begin_shutdown();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        *self
            .shutdown_deadline
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(deadline);
        while (self.runs.lock().is_ok_and(|runs| !runs.is_empty())
            || self.background.load(Ordering::SeqCst) > 0)
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

    /// When a shutdown under way aborts what is still at work.
    pub(super) fn shutdown_deadline(&self) -> Option<tokio::time::Instant> {
        *self
            .shutdown_deadline
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Cancel every run but keep admission open (the account, its sign-in
    /// state or the region changed).
    pub fn cancel_all(self: &Arc<Self>) {
        self.revoke_runs(|_, _| {});
        self.schedule_video_resume();
    }

    pub(super) fn release_run(&self, control: &RunControl) {
        let mut runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        if runs
            .get(&control.session_id)
            .is_some_and(|run| run.run_id == control.run_id)
        {
            runs.remove(&control.session_id);
        }
    }

    fn revoke_runs(&self, transition: impl FnOnce(&mut Admission, u64)) -> u64 {
        // Registration and revocation linearize under the runs lock.
        let runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        let generation = self
            .generation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        self.local_states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        // What was dropped belongs to the authority that just ended.
        self.drops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.audio_inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
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
        self.video_revocation.send_replace(generation);
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

    /// Run `work` in the background like a run's task: shutdown gives it the
    /// time it gives runs to end, then aborts it, which ends any process it
    /// owns. `key` must be unique among tasks.
    pub(super) fn track(
        self: &Arc<Self>,
        key: String,
        work: impl Future<Output = ()> + Send + 'static,
    ) {
        /// Counts the work as over however it ends: done, panicked or aborted.
        struct AtWork(Arc<AtomicU64>);
        impl Drop for AtWork {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        let host = self.clone();
        let done = key.clone();
        self.background.fetch_add(1, Ordering::SeqCst);
        let at_work = AtWork(self.background.clone());
        let task = tokio::spawn(async move {
            let _at_work = at_work;
            work.await;
            if let Ok(mut tasks) = host.tasks.lock() {
                tasks.remove(&done);
            }
        });
        tasks.insert(key, task);
    }

    pub(super) fn emit(&self, run: &RunControl, kind: &str, mut data: Value) {
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
    /// switch may be on while the catalog cannot price it or the model takes
    /// no tools). `thinking_level`: the level the run was sent, `None` for
    /// the default.
    pub(super) fn emit_start(
        &self,
        run: &RunControl,
        web_search: bool,
        thinking_level: Option<&str>,
    ) {
        let mut data = json!({"messageId":run.message_id,"webSearch":web_search,"thinkingLevel":thinking_level});
        if run.compact_only {
            data["compactOnly"] = json!(true);
        }
        self.emit(run, "run_start", data);
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
    pub(super) async fn key_for_retry(
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

    pub async fn steer(&self, request: SteerRequest) -> Result<(), String> {
        self.ensure_accepting_runs()?;
        let identity = self.identity().await?;
        if request.expected_owner != identity.owner
            || request.expected_generation != self.generation()
        {
            return Err(IDENTITY_CHANGED.into());
        }
        for id in [
            &request.session_id,
            &request.run_id,
            &request.operation_id,
            &request.id,
        ] {
            validate_id(id)?;
        }
        if request.text.len() > 128_000 {
            return Err("assistant_message_too_large".into());
        }
        if request.text.trim().is_empty() {
            return Err("assistant_invalid_message".into());
        }
        let run = self
            .runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&request.session_id)
            .cloned()
            .ok_or(SESSION_BUSY)?;
        if run.identity != identity
            || run.run_id != request.run_id
            || run.operation_id != request.operation_id
        {
            return Err(IDENTITY_CHANGED.into());
        }
        self.validate_run(&run)?;
        if request.id == run.run_id || request.id == run.operation_id {
            return Err("assistant_stale_operation".into());
        }
        let sender = run
            .live_input
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or(SESSION_BUSY)?;
        let (reply, receipt) = oneshot::channel();
        sender
            .try_send(super::live::LiveCommand { request, reply })
            .map_err(|_| SESSION_BUSY)?;
        tokio::time::timeout(Duration::from_secs(10), receipt)
            .await
            .map_err(|_| "assistant_runtime_failed")?
            .map_err(|_| SESSION_BUSY)?
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
        let (sender, mut commands) = mpsc::channel(8);
        if !control.compact_only {
            *control
                .live_input
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(sender);
        }
        let result = self
            .converse(
                control,
                params,
                may_retry,
                &request_id,
                (&mut input, &mut output, &mut buffer),
                (&mut attempt, &mut commands),
            )
            .await
            .map_err(|failure| Failure {
                unbilled: failure.unbilled || !attempt.sent,
                ..failure
            });
        *control
            .live_input
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        attempt.projection.end_thinking();
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
        (attempt, commands): (&mut Attempt, &mut mpsc::Receiver<super::live::LiveCommand>),
    ) -> Result<Finish, Failure>
    where
        I: AsyncWrite + Unpin,
        O: AsyncBufRead + Unpin,
    {
        self.validate_run(control)?;
        if self.identity().await? != control.identity {
            return Err(IDENTITY_CHANGED.into());
        }
        let method = if control.compact_only {
            "compact"
        } else {
            "run"
        };
        let line =
            frame(&json!({"jsonrpc":"2.0","id":request_id,"method":method,"params":params}))?;
        let mut cancel = control.cancel.subscribe();
        self.validate_run(control)?;
        tokio::select! {
            biased;
            _ = cancel.changed() => return Err(CANCELLED.into()),
            result = write_line(input, &line) => result?,
        }
        attempt.sent = true;
        let mut ticker = tokio::time::interval(Duration::from_millis(250));
        let mut pending: HashMap<String, super::live::LiveCommand> = HashMap::new();
        loop {
            let length = tokio::select! {
                _ = cancel.changed() => return Err(CANCELLED.into()),
                Some(command) = commands.recv() => {
                    match self.admit_live_input(control,&command.request).await {
                        Ok(false) => {let _=command.reply.send(Ok(()));}
                        Ok(true) => {
                            let rpc_id=format!("steer:{}",command.request.id);
                            let text=scrub_secrets(&command.request.text,&self.known_keys());
                            let line=frame(&json!({"id":rpc_id,"method":"steer","params":{"sessionId":control.session_id,"runId":control.run_id,"operationId":control.operation_id,"id":command.request.id,"text":text,"behavior":"steer"}}))?;
                            self.ensure_current(&control.identity,command.request.expected_generation).await?;
                            self.validate_run(control)?;
                            attempt.progressed=true;
                            write_line(input,&line).await?;
                            pending.insert(rpc_id,command);
                        }
                        Err(error) => {let _=command.reply.send(Err(error));}
                    }
                    continue;
                }
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
            if let Some(command) = value["id"].as_str().and_then(|id| pending.remove(id)) {
                let result = if value["result"]["accepted"] == true {
                    Ok(())
                } else {
                    self.refuse_live_input(control, &command.request.id).await?;
                    Err(SESSION_BUSY.into())
                };
                let _ = command.reply.send(result);
            } else if value["method"] == "event" {
                self.on_event(control, &value["params"], attempt).await?;
            } else if value["method"] == "tool" {
                ensure_same_run(control, &value["params"])?;
                attempt.progressed = true;
                let result = self.dispatch_tool(control, &value["params"]).await;
                let reply = tool_reply(&value["id"], result, &self.known_keys())?;
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
            "steer_applied" => {
                attempt.projection.end_thinking();
                self.flush(control, &mut attempt.projection).await?;
                let data = self.apply_live_input(control, &data).await?;
                self.emit(control, "steer_applied", data);
            }
            "text_delta" | "thinking_delta" => {
                attempt.progressed = true;
                attempt.projection.push(control, kind, &mut data);
                if attempt.projection.due() {
                    self.flush(control, &mut attempt.projection).await?;
                }
                self.emit(control, kind, data);
            }
            // Reasoning began; its text may follow a little later (the runtime
            // holds back a tail while it scrubs keys).
            "thinking_start" => {
                attempt.progressed = true;
                attempt.projection.start_thinking();
                self.emit(control, kind, json!({"messageId": control.reply_id()}));
            }
            // Reasoning stopped. The reply's text may come in a little later:
            // the runtime holds back its start the same way.
            "thinking_end" => {
                attempt.projection.end_thinking();
                self.emit(control, kind, json!({"messageId": control.reply_id()}));
            }
            "usage" => {
                attempt.progressed |= counts_tokens(&data);
                let usage = self.save_usage(control, data).await?;
                self.emit(control, "usage", json!({ "usage": usage }));
            }
            "compaction_start" | "compaction_end" => {
                control
                    .compacting
                    .store(kind == "compaction_start", Ordering::SeqCst);
                self.emit(control, kind, data);
            }
            "task_plan" | "task_status" if control.local_enabled => {
                scrub_secrets_in(&mut data, &control.secrets());
                let shown = super::tools::events::task_event(kind, &data)?;
                self.emit(control, kind, shown);
            }
            "tool_start" => {
                // The model's tool call has been answered with tokens, and the
                // tool may cost something: never repeat this attempt.
                attempt.progressed = true;
                attempt.projection.end_thinking();
                // As for text: the query is the model's, the keys are not.
                scrub_secrets_in(&mut data, &control.secrets());
                let mut shown = super::tools::events::tool_start_event(&data)?;
                shown["messageId"] = json!(control.reply_id());
                self.emit(control, kind, shown);
            }
            "tool_end" => {
                attempt.progressed = true;
                scrub_secrets_in(&mut data, &control.secrets());
                let mut shown = super::tools::events::tool_end_event(&data)?;
                shown["messageId"] = json!(control.reply_id());
                if data["name"] == "web_search" && data["success"] == true {
                    let found = web::search_sources(&control.run_id, &data["details"]);
                    if let Some(sources) = self.save_sources(control, found).await? {
                        self.emit(
                            control,
                            "sources",
                            json!({"messageId": control.reply_id(), "sources": sources}),
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
            // The host publishes its own message once the turn is saved.
            "message_end" => attempt.projection.end_thinking(),
            // run_start, error and run_end: the host publishes its own.
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
        attempt.projection.end_thinking();
        self.flush(control, &mut attempt.projection).await?;
        let result = &value["result"];
        self.keep_local_task(control, result);
        if control.compact_only
            && result["errorCode"] == "assistant_compact_unavailable"
            && result["errorKind"] == "not_sent"
        {
            self.apply_native(control, &result["entries"]).await?;
            return Err(Failure {
                code: "assistant_compact_unavailable".into(),
                detail: None,
                unbilled: true,
                native: None,
            });
        }
        match result["status"].as_str() {
            Some("completed") | Some("stopped") => {
                self.apply_native(control, &result["entries"]).await?;
                self.save_context_usage(control, &result["contextUsage"])
                    .await?;
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
            control.reply_id(),
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
        let (sid, mid) = (control.session_id.clone(), control.reply_id());
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

    pub(super) async fn apply_native(
        &self,
        control: &RunControl,
        entries: &Value,
    ) -> Result<(), String> {
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
    /// The conversation's first answer then has it named in the background.
    pub(super) async fn finish(
        self: &Arc<Self>,
        control: &Arc<RunControl>,
        outcome: Result<Finish, Failure>,
    ) {
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
                    let (settled, first_answer) = store.mutate(&sid, |record| {
                        let settled = settle_run(
                            record,
                            &control,
                            completed,
                            stopped,
                            unbilled,
                            error_code.as_deref(),
                        )?;
                        let first_answer = completed
                            .then(|| {
                                title::first_answer(record, &control.run_id, &control.message_id)
                            })
                            .flatten();
                        Ok((settled, first_answer))
                    })?;
                    Ok((settled, first_answer.filter(|_| store.auto_title_enabled())))
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
        let first_answer = match saved {
            Ok(((message, session), first_answer)) => {
                if let Some(message) = message {
                    self.emit(control, "message", json!({ "message": message }));
                }
                self.emit(control, "session", json!({ "session": session }));
                first_answer
            }
            Err(error) => {
                log::warn!("[Assistant] could not save the end of a run: {error}");
                None
            }
        };
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
        if let Some(input) = first_answer {
            self.spawn_title(control, input);
        }
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

pub(super) fn frame(value: &Value) -> Result<Vec<u8>, String> {
    let mut line = serde_json::to_vec(value).map_err(|_| PROTOCOL)?;
    line.push(b'\n');
    Ok(line)
}

/// The runtime drops input lines over 1 MiB, so a larger tool result would
/// never settle its pending call; it becomes an error the model can act on.
const RUNTIME_LINE_LIMIT: usize = 1024 * 1024;

pub(super) fn tool_reply(
    id: &Value,
    result: Result<Value, String>,
    known_keys: &[String],
) -> Result<Vec<u8>, String> {
    let error = |code: &str| {
        let code = local::tool_failure(code, known_keys);
        frame(&json!({"id":id,"error":{"code":code,"message":code}}))
    };
    match result {
        Ok(result) => {
            let reply = frame(&json!({"id":id,"result":result}))?;
            if reply.len() > RUNTIME_LINE_LIMIT {
                error("assistant_result_too_large")
            } else {
                Ok(reply)
            }
        }
        Err(code) => error(&code),
    }
}

pub(super) async fn write_line<W: AsyncWrite + Unpin>(
    input: &mut W,
    line: &[u8],
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    input.write_all(line).await.map_err(|_| STDIO)?;
    input.flush().await.map_err(|_| STDIO.to_string())
}

/// One newline-terminated frame into `buffer`; `Ok(0)` at end of stream.
pub(super) async fn read_frame<R: AsyncBufRead + Unpin>(
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
pub(super) async fn load_native(runtime: &Runtime, file: &Path) -> Result<Value, String> {
    native_entries(
        runtime,
        "load",
        json!({ "sessionFile": file }),
        (
            "assistant_history_recovery_failed",
            "assistant_history_recovery_timeout",
        ),
    )
    .await
}

/// The native entries of `file` once the runtime's `navigate` showed the
/// version of message `entry` (assistant-runtime README), from a short-lived
/// runtime process. The caller holds the session's native lock.
pub(super) async fn navigate_native(
    runtime: &Runtime,
    file: &Path,
    entry: &str,
) -> Result<Value, String> {
    const FAILED: &str = "assistant_version_switch_failed";
    native_entries(
        runtime,
        "navigate",
        json!({ "sessionFile": file, "entryId": entry }),
        (FAILED, FAILED),
    )
    .await
}

/// The projection's entries from one `method` request to a short-lived
/// runtime process; `(failed, timeout)` are the codes when it refuses, or
/// gives no answer within 15 seconds.
pub(super) async fn native_entries(
    runtime: &Runtime,
    method: &str,
    params: Value,
    (failed, timeout): (&str, &str),
) -> Result<Value, String> {
    use tokio::io::BufReader;
    let mut child = process::Child::spawn(runtime_command(runtime))?;
    let mut input = child.take_stdin().ok_or(STDIO)?;
    let mut output = BufReader::new(child.take_stdout().ok_or(STDIO)?);
    let stderr = child.take_stderr().ok_or(STDIO)?;
    let drain = tokio::spawn(async move {
        let _ = process::bounded_output(stderr, 64 * 1024).await;
    });
    let request = frame(&json!({"jsonrpc":"2.0","id":method,"method":method,"params":params}))?;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        write_line(&mut input, &request).await?;
        let mut bytes = vec![];
        read_frame(&mut output, &mut bytes).await?;
        let response: Value = serde_json::from_slice(&bytes).map_err(|_| PROTOCOL)?;
        if response["id"] != method || response.get("error").is_some() {
            return Err(failed.to_string());
        }
        Ok(response["result"]["entries"].clone())
    })
    .await
    .map_err(|_| timeout.to_string())
    .and_then(|v| v);
    child.stop().await;
    drain.abort();
    result
}

pub(super) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("assistant_storage_unavailable".into()),
    }
}

/// Put the native history back as it was before a refused attempt, or a
/// version switch that was not saved.
pub(super) fn restore(path: &Path, before: Option<Vec<u8>>) -> Result<(), String> {
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

    #[tokio::test]
    async fn local_write_journals_and_restores_only_after_the_run_ends() {
        use super::local::PermissionRequest;
        let (_tmp, env, host) = fixture_with_env();
        let sid = host
            .store(&identity().owner)
            .unwrap()
            .create_with_capabilities(
                "test/model",
                false,
                "ofox",
                &[super::super::capabilities::CapabilityRef {
                    id: "local_config".into(),
                    version: 1,
                }],
            )
            .unwrap()
            .session
            .id;
        host.set_permission(PermissionRequest {
            session_id: sid.clone(),
            mode: super::super::tools::policy::PermissionMode::FullAuto,
            confirmed_full_access: true,
            expected_owner: identity().owner,
            expected_generation: host.generation(),
        })
        .await
        .unwrap();
        let file = env.config.join("config/settings");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "original").unwrap();
        let mut run = control(&sid);
        Arc::get_mut(&mut run).unwrap().local_enabled = true;
        host.register_run(run.clone(), host.generation(), || Ok(()))
            .unwrap();
        let args = json!({"path":file,"content":"repaired","expected":"original"});
        let call = json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"toolCallId":"write-1","name":"edit","args":args});
        let result = host.dispatch_tool(&run, &call).await.unwrap();
        assert_eq!(result["changed"], true);
        let authority = || DeleteRequest {
            session_id: sid.clone(),
            expected_owner: identity().owner,
            expected_generation: host.generation(),
        };
        assert_eq!(
            host.local_changes(authority(), true).await.unwrap_err(),
            SESSION_BUSY
        );
        host.release_run(&run);
        let changes = host.local_changes(authority(), false).await.unwrap();
        assert_eq!(changes.len(), 1);
        std::fs::write(&file, "user changed").unwrap();
        assert_eq!(
            host.local_changes(authority(), true).await.unwrap_err(),
            "assistant_restore_conflict"
        );
        std::fs::write(&file, "repaired").unwrap();
        assert!(host
            .local_changes(authority(), true)
            .await
            .unwrap()
            .iter()
            .all(|c| c.restored));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
    }

    #[tokio::test]
    async fn local_reads_anywhere_run_without_approval_but_stay_scrubbed_and_protected() {
        let (_tmp, env, host) = fixture_with_env();
        let (sid, run) = local_run(&host);
        let folder = env.config.join("project");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("config.json"),
            r#"{"api_key":"sk-test-sensitive-value-123456789"}"#,
        )
        .unwrap();
        let read = |id: &str, path: PathBuf| json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"toolCallId":id,"name":"read","args":{"path":path}});
        let text = host
            .dispatch_tool(&run, &read("read-1", folder.join("config.json")))
            .await
            .unwrap()
            .to_string();
        assert!(!text.contains("sensitive-value"));
        let key = env.config.join(".ssh/id_ed25519");
        std::fs::create_dir_all(key.parent().unwrap()).unwrap();
        std::fs::write(&key, "private").unwrap();
        assert_eq!(
            host.dispatch_tool(&run, &read("read-2", key))
                .await
                .unwrap_err(),
            "assistant_operation_denied"
        );
        assert!(approvals(&env).is_empty());
        host.release_run(&run);
    }

    /// A local-config session with one registered run in the default mode.
    fn local_run(host: &Arc<AssistantHost>) -> (String, Arc<RunControl>) {
        let sid = host
            .store(&identity().owner)
            .unwrap()
            .create_with_capabilities(
                "test/model",
                false,
                "ofox",
                &[super::super::capabilities::CapabilityRef {
                    id: "local_config".into(),
                    version: 1,
                }],
            )
            .unwrap()
            .session
            .id;
        let mut run = control(&sid);
        Arc::get_mut(&mut run).unwrap().local_enabled = true;
        host.register_run(run.clone(), host.generation(), || Ok(()))
            .unwrap();
        (sid, run)
    }

    fn approvals(env: &FixedEnvironment) -> Vec<Value> {
        env.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.r#type == "approval")
            .map(|e| e.data.clone())
            .collect()
    }

    /// Dispatches one call and answers the approval it raises with `choice`.
    async fn dispatch_answering(
        host: &Arc<AssistantHost>,
        env: &Arc<FixedEnvironment>,
        run: &Arc<RunControl>,
        params: Value,
        choice: &str,
    ) -> (Value, Result<Value, String>) {
        let seen = approvals(env).len();
        let task = tokio::spawn({
            let host = host.clone();
            let run = run.clone();
            async move { host.dispatch_tool(&run, &params).await }
        });
        let approval = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(data) = approvals(env).get(seen) {
                    break data.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        host.respond_approval(serde_json::from_value(json!({"sessionId":run.session_id,"runId":run.run_id,"operationId":run.operation_id,"approvalId":approval["approvalId"],"choice":choice,"expectedOwner":identity().owner,"expectedGeneration":host.generation()})).unwrap())
            .await
            .unwrap();
        (approval, task.await.unwrap())
    }

    #[tokio::test]
    async fn local_read_only_commands_from_home_run_without_approval() {
        let (_tmp, env, host) = fixture_with_env();
        let (sid, run) = local_run(&host);
        let (name, command) = if cfg!(windows) {
            ("powershell", "Get-Location")
        } else {
            ("bash", "pwd")
        };
        let params = json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"toolCallId":"pwd-1","name":name,"args":{"command":command,"folder":env.config,"timeout":30}});
        let result = host.dispatch_tool(&run, &params).await.unwrap();
        assert_eq!(result["exitCode"], 0, "{result}");
        assert!(approvals(&env).is_empty());
        host.release_run(&run);
    }

    #[tokio::test]
    async fn local_session_grant_covers_config_changes_and_one_program() {
        let (_tmp, env, host) = fixture_with_env();
        let (sid, run) = local_run(&host);
        let call = |id: &str, name: &str, args: Value| json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"toolCallId":id,"name":name,"args":args});
        let write = |id: &str, file: &str| {
            call(
                id,
                "write",
                json!({"path":env.config.join(file),"content":"new","expected":null}),
            )
        };
        let (approval, first) =
            dispatch_answering(&host, &env, &run, write("w-1", "a.txt"), "session").await;
        assert_eq!(approval["allowSession"], true);
        assert_eq!(first.unwrap()["changed"], true);
        // The grant is the whole configuration-change category.
        host.dispatch_tool(&run, &write("w-2", "b.txt"))
            .await
            .unwrap();
        assert_eq!(approvals(&env).len(), 1);
        let (shell, echo, other) = if cfg!(windows) {
            ("powershell", "Write-Output one", "Get-Date")
        } else {
            ("bash", "echo one", "date")
        };
        let exec = |id: &str, command: &str| {
            call(
                id,
                shell,
                json!({"command":command,"folder":env.config,"timeout":30}),
            )
        };
        let (_, ran) = dispatch_answering(&host, &env, &run, exec("e-1", echo), "session").await;
        assert_eq!(ran.unwrap()["exitCode"], 0);
        host.dispatch_tool(&run, &exec("e-2", &format!("{echo} two")))
            .await
            .unwrap();
        assert_eq!(approvals(&env).len(), 2);
        // Another program asks again.
        let (_, denied) = dispatch_answering(&host, &env, &run, exec("e-3", other), "deny").await;
        assert_eq!(denied.unwrap_err(), "assistant_operation_denied");
        host.release_run(&run);
    }

    #[tokio::test]
    async fn local_rebind_that_may_issue_a_new_key_asks_like_a_command_until_full_auto() {
        use super::local::PermissionRequest;
        let (_tmp, env, host) = fixture_with_env();
        let (sid, run) = local_run(&host);
        let rebind = |id: &str| json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"toolCallId":id,"name":"rebind_tool","args":{"tool":"codex"}});
        let (approval, result) = dispatch_answering(&host, &env, &run, rebind("r-1"), "deny").await;
        assert_eq!(approval["category"], "exec");
        assert_eq!(result.unwrap_err(), "assistant_operation_denied");
        host.release_run(&run);
        host.set_permission(PermissionRequest {
            session_id: sid.clone(),
            mode: super::super::tools::policy::PermissionMode::FullAuto,
            confirmed_full_access: true,
            expected_owner: identity().owner,
            expected_generation: host.generation(),
        })
        .await
        .unwrap();
        let run = control(&sid);
        let mut run = run;
        Arc::get_mut(&mut run).unwrap().local_enabled = true;
        host.register_run(run.clone(), host.generation(), || Ok(()))
            .unwrap();
        let params = json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"toolCallId":"r-2","name":"rebind_tool","args":{"tool":"codex"}});
        // The test environment has no binding writer; no approval was asked.
        assert_eq!(
            host.dispatch_tool(&run, &params).await.unwrap_err(),
            "assistant_local_unavailable"
        );
        assert_eq!(approvals(&env).len(), 1);
        host.release_run(&run);
    }

    #[tokio::test]
    async fn local_pending_verification_is_kept_for_the_next_run_of_the_conversation() {
        let (_tmp, _env, host) = fixture_with_env();
        let (sid, run) = local_run(&host);
        let owner = identity().owner;
        let pending =
            json!({"changed":true,"targets":[{"tool":"codex","folder":"/p","verified":false}]});
        host.keep_local_task(&run, &json!({"localTaskState":pending}));
        assert_eq!(host.local_task_state(&owner, &sid), pending);
        // A run that ends without reporting keeps the debt; one that verified clears it.
        host.keep_local_task(&run, &json!({"status":"error"}));
        assert_eq!(host.local_task_state(&owner, &sid), pending);
        host.keep_local_task(&run, &json!({"localTaskState":null}));
        assert_eq!(host.local_task_state(&owner, &sid), Value::Null);
        host.release_run(&run);
    }

    #[test]
    fn local_tool_replies_over_the_runtime_line_limit_become_an_error() {
        let id = json!("tool-1");
        let small = super::tool_reply(&id, Ok(json!("short")), &[]).unwrap();
        assert!(String::from_utf8(small).unwrap().contains("short"));
        let large = super::tool_reply(&id, Ok(json!("中".repeat(400_000))), &[]).unwrap();
        assert!(large.len() < 1024);
        let text = String::from_utf8(large).unwrap();
        assert!(text.contains("assistant_result_too_large"), "{text}");
        assert!(text.contains("tool-1"));
    }

    #[tokio::test]
    async fn local_tools_approval_dispatch_deduplicates_and_rejects_stale_authority() {
        use super::local::ApprovalRequest;
        let (_tmp, env, host) = fixture_with_env();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut run = control(&sid);
        Arc::get_mut(&mut run).unwrap().local_enabled = true;
        let params = json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,
            "toolCallId":"probe-1","name":"probe_write","args":{"command":"synthetic"}});
        assert_eq!(
            host.dispatch_tool(&run, &params).await.unwrap_err(),
            "assistant_capability_disabled"
        );
        store
            .update(
                &serde_json::from_value(
                    json!({"sessionId":sid,"capabilities":[{"id":"local_config","version":1}]}),
                )
                .unwrap(),
                || Ok(()),
            )
            .unwrap();
        host.register_run(run.clone(), host.generation(), || Ok(()))
            .unwrap();
        let task = tokio::spawn({
            let host = host.clone();
            let run = run.clone();
            let params = params.clone();
            async move { host.dispatch_tool(&run, &params).await }
        });
        let approval = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(event) = env
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|e| e.r#type == "approval")
                    .cloned()
                {
                    break event.data;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let response = json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,
            "approvalId":approval["approvalId"],"choice":"once","expectedOwner":identity().owner,"expectedGeneration":host.generation()});
        let mut stale = response.clone();
        stale["expectedGeneration"] = json!(host.generation() + 1);
        assert_eq!(
            host.respond_approval(serde_json::from_value::<ApprovalRequest>(stale).unwrap())
                .await
                .unwrap_err(),
            IDENTITY_CHANGED
        );
        host.respond_approval(serde_json::from_value(response).unwrap())
            .await
            .unwrap();
        assert_eq!(task.await.unwrap().unwrap(), json!({"approved":true}));
        assert_eq!(
            host.dispatch_tool(&run, &params).await.unwrap(),
            json!({"approved":true})
        );
        let mut conflict = params.clone();
        conflict["args"]["command"] = json!("different");
        assert_eq!(
            host.dispatch_tool(&run, &conflict).await.unwrap_err(),
            "assistant_tool_call_conflict"
        );
        let mut malformed = params.clone();
        malformed["args"]["unexpected"] = json!(true);
        assert_eq!(
            host.dispatch_tool(&run, &malformed).await.unwrap_err(),
            "assistant_invalid_tool"
        );
        run.cancel.send_replace(true);
        assert_eq!(
            host.dispatch_tool(&run, &params).await.unwrap_err(),
            CANCELLED
        );
        host.release_run(&run);
    }

    #[tokio::test]
    async fn local_tools_jsonl_callback_waits_for_approval_and_returns_result() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let (_tmp, env, host) = fixture_with_env();
        let store = host.store(&identity().owner).unwrap();
        let capabilities: Vec<super::super::capabilities::CapabilityRef> =
            serde_json::from_value(json!([{"id":"local_config","version":1}])).unwrap();
        let sid = store
            .create_with_capabilities("test/model", false, "ofox", &capabilities)
            .unwrap()
            .session
            .id;
        let mut run = control(&sid);
        Arc::get_mut(&mut run).unwrap().local_enabled = true;
        host.register_run(run.clone(), host.generation(), || Ok(()))
            .unwrap();
        let (host_pipe, runtime_pipe) = tokio::io::duplex(8192);
        let (output, mut input) = tokio::io::split(host_pipe);
        let peer_run = run.clone();
        let peer = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(runtime_pipe);
            let mut read = BufReader::new(read);
            let mut line = String::new();
            read.read_line(&mut line).await.unwrap();
            let start: Value = serde_json::from_str(&line).unwrap();
            let params = json!({"sessionId":peer_run.session_id,"runId":peer_run.run_id,"operationId":peer_run.operation_id,"toolCallId":"rpc-probe","name":"probe_write","args":{"command":"synthetic"}});
            write
                .write_all(
                    &frame(&json!({"id":"tool-probe","method":"tool","params":params})).unwrap(),
                )
                .await
                .unwrap();
            line.clear();
            read.read_line(&mut line).await.unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(reply["id"], "tool-probe");
            assert_eq!(reply["result"], json!({"approved":true}));
            write
                .write_all(
                    &frame(&json!({"id":start["id"],"error":{"message":"synthetic end"}})).unwrap(),
                )
                .await
                .unwrap();
        });
        let confirmer = tokio::spawn({
            let host = host.clone();
            let run = run.clone();
            async move {
                let approval = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        if let Some(event) = env
                            .events
                            .lock()
                            .unwrap()
                            .iter()
                            .find(|e| e.r#type == "approval")
                            .cloned()
                        {
                            break event.data;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                host.respond_approval(serde_json::from_value(json!({"sessionId":run.session_id,"runId":run.run_id,"operationId":run.operation_id,"approvalId":approval["approvalId"],"choice":"once","expectedOwner":identity().owner,"expectedGeneration":host.generation()})).unwrap()).await.unwrap();
            }
        });
        let mut output = BufReader::new(output);
        let mut buffer = Vec::new();
        let mut attempt = Attempt::default();
        let (_sender, mut commands) = mpsc::channel(1);
        let result = host
            .converse(
                &run,
                &json!({}),
                false,
                "request",
                (&mut input, &mut output, &mut buffer),
                (&mut attempt, &mut commands),
            )
            .await;
        assert_eq!(result.unwrap_err().code, "assistant_runtime_failed");
        assert!(attempt.progressed);
        peer.await.unwrap();
        confirmer.await.unwrap();
        host.release_run(&run);
    }

    /// Signed in as one account in `ofox.ai`, files in a temp dir; a one-model
    /// `/v1` catalog whose fetches are counted, an empty Anthropic one; no key
    /// or runtime.
    struct FixedEnvironment {
        apex: Mutex<String>,
        catalog_apexes: Mutex<Vec<String>>,
        config: PathBuf,
        catalog: Mutex<Vec<String>>,
        /// `/v1` fetches.
        fetches: AtomicU64,
        anthropic: Mutex<Vec<String>>,
        anthropic_fetches: AtomicU64,
        /// Sources whose fetch fails.
        down: Mutex<HashSet<catalog::Source>>,
        /// Sources whose fetch fails only after the HTTP client's timeout.
        hanging: Mutex<HashSet<catalog::Source>>,
        /// Rows of other kinds, after the `/v1` chat models.
        media: Mutex<Vec<catalog::Model>>,
        runtime: Mutex<Option<PathBuf>>,
        events: Mutex<Vec<Event>>,
        stored_key: Mutex<Option<String>>,
    }

    impl Environment for FixedEnvironment {
        fn local_journal_key(&self) -> BoxFuture<'_, Result<zeroize::Zeroizing<[u8; 32]>, String>> {
            Box::pin(async { Ok(zeroize::Zeroizing::new([7; 32])) })
        }

        fn local_paths(
            &self,
        ) -> BoxFuture<'_, Result<super::super::tools::context::LocalPaths, String>> {
            Box::pin(async {
                Ok(super::super::tools::context::LocalPaths {
                    home: self.config.clone(),
                    config_roots: vec![self.config.join("config")],
                    shell_files: vec![self.config.join(".zshrc")],
                    skill: self.config.join("skills/SKILL.md"),
                })
            })
        }
        fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
            Box::pin(async {
                let mut value = identity();
                value.apex = self.apex.lock().unwrap().clone();
                Ok(value)
            })
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
            apex: &'a str,
            source: catalog::Source,
        ) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>> {
            Box::pin(async move {
                self.catalog_apexes.lock().unwrap().push(apex.to_owned());
                let (fetches, ids) = match source {
                    catalog::Source::V1 => (&self.fetches, &self.catalog),
                    catalog::Source::Anthropic => (&self.anthropic_fetches, &self.anthropic),
                };
                fetches.fetch_add(1, Ordering::SeqCst);
                if self.hanging.lock().unwrap().contains(&source) {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    return Err("assistant_catalog_unavailable".to_string());
                }
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
            apex: Mutex::new("ofox.ai".into()),
            catalog_apexes: Mutex::default(),
            config: config.path().to_path_buf(),
            catalog: Mutex::new(vec!["test/model".into()]),
            fetches: AtomicU64::new(0),
            anthropic: Mutex::default(),
            anthropic_fetches: AtomicU64::new(0),
            down: Mutex::default(),
            hanging: Mutex::default(),
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
                            media: None,
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

    #[tokio::test]
    async fn sends_prepared_for_stale_authority_are_refused_before_recovery() {
        let (_config, env, host) = fixture_with_env();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let generation = host.generation();
        for authority in [
            json!({"expectedOwner": "b".repeat(64), "expectedGeneration": generation}),
            json!({"expectedOwner": identity().owner, "expectedGeneration": generation + 1}),
            json!({"expectedOwner": identity().owner}),
            json!({"expectedGeneration": generation}),
        ] {
            for media in [
                json!({}),
                json!({"media": {}}),
                json!({"video": {"duration": 4, "resolution": "480p"}}),
            ] {
                let mut value = json!({
                    "sessionId": sid, "runId": id(), "operationId": id(), "text": "Queued text",
                });
                value
                    .as_object_mut()
                    .unwrap()
                    .extend(authority.as_object().unwrap().clone());
                value
                    .as_object_mut()
                    .unwrap()
                    .extend(media.as_object().unwrap().clone());
                let request: SendRequest = serde_json::from_value(value).unwrap();
                assert_eq!(host.send(request).await.unwrap_err(), IDENTITY_CHANGED);
            }
        }
        assert_eq!(env.fetches.load(Ordering::SeqCst), 0);
        assert!(env.events.lock().unwrap().is_empty());
        let record = store.load_record(&sid).unwrap();
        assert!(record.messages.is_empty());
        assert!(record.operations.is_empty());
    }

    #[tokio::test]
    async fn current_and_legacy_send_authority_reach_normal_admission() {
        let (_config, host) = fixture();
        let sid = host
            .store(&identity().owner)
            .unwrap()
            .create("test/model", false)
            .unwrap()
            .session
            .id;
        for authority in [
            json!({}),
            json!({
                "expectedOwner": identity().owner, "expectedGeneration": host.generation(),
            }),
        ] {
            let mut value = json!({
                "sessionId": sid, "runId": id(), "operationId": id(), "text": "Ordinary text",
            });
            value
                .as_object_mut()
                .unwrap()
                .extend(authority.as_object().unwrap().clone());
            let request: SendRequest = serde_json::from_value(value).unwrap();
            // This fixture deliberately has no runtime. Both forms get past
            // the authority fence, without minting a key or starting a run.
            assert_eq!(host.send(request).await.unwrap_err(), RUNTIME_UNAVAILABLE);
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
    fn a_version_switch_and_a_run_of_the_same_conversation_exclude_each_other() {
        let (_config, host) = fixture();
        let (sid, other) = (id(), id());
        let held = host.begin_navigation(&sid).unwrap();
        assert_eq!(
            host.begin_navigation(&sid).err().unwrap(),
            "assistant_session_busy"
        );
        let run = control(&sid);
        assert_eq!(
            host.register_run(run.clone(), host.generation(), || Ok(()))
                .unwrap_err(),
            "assistant_session_busy"
        );
        // Another conversation is not held.
        drop(host.begin_navigation(&other).unwrap());
        drop(held);
        host.register_run(run, host.generation(), || Ok(()))
            .unwrap();
        // A conversation that replies cannot be held.
        assert_eq!(
            host.begin_navigation(&sid).err().unwrap(),
            "assistant_session_busy"
        );
        assert!(host.begin_navigation(&other).is_ok());
    }

    /// A title writing the native history makes a switch wait, not a send:
    /// a switch holds the conversation only once it has the history.
    #[tokio::test]
    async fn a_switch_waiting_for_the_native_history_does_not_hold_the_conversation() {
        let (_config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        host.recover(&identity(), &sid).await.unwrap();
        let title = host
            .native_lock(&session_key(&identity().owner, &sid))
            .unwrap()
            .lock_owned()
            .await;
        let request: SwitchVersionRequest = serde_json::from_value(json!({
            "sessionId": sid, "messageId": id(), "expectedOwner": identity().owner,
            "expectedGeneration": host.generation(),
        }))
        .unwrap();
        let switching = tokio::spawn({
            let host = host.clone();
            async move { host.switch_version(request).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        host.register_run(control(&sid), host.generation(), || Ok(()))
            .unwrap();
        drop(title);
        assert_eq!(
            switching.await.unwrap().unwrap_err(),
            "assistant_session_busy"
        );
    }

    #[tokio::test]
    async fn a_native_sync_rechecks_run_admission_after_waiting_for_history() {
        let (_config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let held = host
            .native_lock(&session_key(&identity().owner, &sid))
            .unwrap()
            .lock_owned()
            .await;
        let current_identity = identity();
        let syncing = host.sync_native(&current_identity, &sid);
        tokio::pin!(syncing);
        tokio::select! {
            result = &mut syncing => panic!("must wait for history: {result:?}"),
            _ = tokio::task::yield_now() => {}
        }
        host.register_run(control(&sid), host.generation(), || Ok(()))
            .unwrap();
        drop(held);
        assert_eq!(syncing.await.unwrap_err(), SESSION_BUSY);
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
                media: None,
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

    /// A stand-in runtime: `script` is the shell body of its `node`.
    #[cfg(unix)]
    fn stub_runtime(dir: &Path, script: &str) -> Runtime {
        use std::os::unix::fs::PermissionsExt;
        let base = dir.join("runtime-stub");
        let node = base.join("node").join("node");
        std::fs::create_dir_all(node.parent().unwrap()).unwrap();
        std::fs::write(&node, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
        let entry = base.join("runtime").join("src").join("main.mjs");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(&entry, "").unwrap();
        runtime_at(&base).unwrap()
    }

    /// A title still unanswered at its deadline is asked to stop, and the
    /// answer it then gives (with what the request may have cost) is kept.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_title_past_its_deadline_is_stopped_and_still_answers() {
        let (config, host) = fixture();
        // Answers only once it is told to stop.
        let runtime = stub_runtime(
            config.path(),
            r#"read request
read stop
case "$stop" in *'"method":"stop"'*'"runId":"turn"'*) echo '{"id":"title","result":{"title":null,"entries":[]}}' ;; esac
while read line; do :; done"#,
        );
        let params = json!({"sessionId": "conversation", "runId": "turn", "operationId": "op"});
        let times = title::TitleTimes {
            deadline: Duration::from_millis(300),
            answer_grace: Duration::from_secs(30),
            stop_grace: Duration::from_secs(5),
        };
        let answer = host
            .ask_title(&runtime, &params, host.generation(), &times)
            .await
            .unwrap();
        assert_eq!(answer, json!({"title": null, "entries": []}));
    }

    /// Background work that panics still counts as over: shutdown does not
    /// wait for it.
    #[tokio::test]
    async fn background_work_that_panics_does_not_hold_up_shutdown() {
        let (_config, host) = fixture();
        host.track("panics".into(), async { panic!("a title went wrong") });
        let finished = tokio::time::Instant::now() + Duration::from_secs(2);
        while host
            .tasks
            .lock()
            .unwrap()
            .values()
            .any(|task| !task.is_finished())
            && tokio::time::Instant::now() < finished
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let started = std::time::Instant::now();
        host.shutdown().await;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
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

    #[tokio::test]
    async fn the_runtimes_thinking_end_closes_the_reasoning_span_and_is_published() {
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
        let mut attempt = Attempt::default();
        for (kind, data) in [
            ("thinking_start", json!({})),
            ("thinking_delta", json!({"delta": "Hmm"})),
            ("thinking_end", json!({})),
        ] {
            host.on_event(&run, &event(kind, data), &mut attempt)
                .await
                .unwrap();
        }
        // The reply's first words come in later (the runtime holds back the
        // start of the text): that wait is not thinking.
        tokio::time::sleep(Duration::from_millis(300)).await;
        host.on_event(
            &run,
            &event("text_delta", json!({"delta": "Answer"})),
            &mut attempt,
        )
        .await
        .unwrap();
        host.flush(&run, &mut attempt.projection).await.unwrap();
        let spent = store.load_detail(&sid).unwrap().messages[0]
            .reasoning_ms
            .unwrap();
        assert!(spent < 300, "{spent} ms");
        let published: Vec<(String, Value)> = env
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| (e.r#type.clone(), e.data.clone()))
            .collect();
        assert_eq!(
            published
                .iter()
                .map(|(kind, _)| kind.as_str())
                .collect::<Vec<_>>(),
            [
                "thinking_start",
                "thinking_delta",
                "thinking_end",
                "text_delta"
            ]
        );
        assert_eq!(published[2].1, json!({"messageId": run.message_id}));
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

    /// Make every cached catalog source older than [`MODELS_TTL`], and every
    /// failed read older than [`FAILED_SOURCE_RETRY`].
    fn age_catalogs(host: &AssistantHost) {
        for cached in host.models.lock().unwrap().values_mut() {
            if let Some((fetched, _)) = &mut cached.rows {
                *fetched -= MODELS_TTL;
            }
            if let Some((failed, _)) = &mut cached.failed {
                *failed -= FAILED_SOURCE_RETRY;
            }
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
    async fn a_failed_catalog_source_is_not_asked_again_at_once() {
        let (_config, env, host) = fixture_with_env();
        let anthropic_reads = || env.anthropic_fetches.load(Ordering::SeqCst);
        env.down.lock().unwrap().insert(catalog::Source::Anthropic);
        assert_eq!(
            host.models("ofox.ai").await.unwrap().partial,
            [catalog::Source::Anthropic]
        );
        // What one send reads: the model, then the search price.
        host.model("ofox.ai", "test/model").await.unwrap();
        assert_eq!(
            host.models("ofox.ai").await.unwrap().partial,
            [catalog::Source::Anthropic]
        );
        assert_eq!(
            anthropic_reads(),
            1,
            "a failed read is not repeated at once"
        );

        // A lookup that misses asks every source again.
        assert_eq!(
            host.model("ofox.ai", "gone/model").await.unwrap_err(),
            MODEL_UNAVAILABLE
        );
        assert_eq!(anthropic_reads(), 2);

        // Back up, but asked again only after the interval.
        env.down.lock().unwrap().clear();
        *env.anthropic.lock().unwrap() = vec!["anthropic/claude-test".into()];
        host.models("ofox.ai").await.unwrap();
        assert_eq!(anthropic_reads(), 2);
        age_catalogs(&host);
        let merged = host.models("ofox.ai").await.unwrap();
        assert_eq!(anthropic_reads(), 3);
        assert_eq!(
            catalog_ids(&merged),
            ["test/model", "anthropic/claude-test"]
        );
        assert!(merged.partial.is_empty());

        // Down again with a copy: the copy is served, the host not asked
        // again until the interval is over.
        age_catalogs(&host);
        env.down.lock().unwrap().insert(catalog::Source::Anthropic);
        for _ in 0..3 {
            let merged = host.models("ofox.ai").await.unwrap();
            assert_eq!(
                catalog_ids(&merged),
                ["test/model", "anthropic/claude-test"]
            );
        }
        assert_eq!(anthropic_reads(), 4);
    }

    #[tokio::test]
    async fn opening_the_page_asks_a_failed_catalog_source_again_at_once() {
        let (_config, env, host) = fixture_with_env();
        let anthropic_reads = || env.anthropic_fetches.load(Ordering::SeqCst);
        let boot_ids = |boot: &Value| -> Vec<String> {
            boot["models"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["id"].as_str().unwrap().to_owned())
                .collect()
        };
        *env.anthropic.lock().unwrap() = vec!["anthropic/claude-test".into()];
        host.models("ofox.ai").await.unwrap();
        age_catalogs(&host);
        env.down.lock().unwrap().insert(catalog::Source::Anthropic);
        // The read fails: the stale copy stands in.
        host.models("ofox.ai").await.unwrap();
        assert_eq!(anthropic_reads(), 2);

        // Send-time reads within the pause do not ask again.
        host.model("ofox.ai", "test/model").await.unwrap();
        host.models("ofox.ai").await.unwrap();
        assert_eq!(anthropic_reads(), 2);

        // Opening the page (or Retry) does, and keeps the copy when that
        // read fails too.
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(anthropic_reads(), 3);
        assert_eq!(boot_ids(&boot), ["test/model", "anthropic/claude-test"]);
        assert_eq!(boot["partial"], json!([]));
        // That failure pauses send-time reads again.
        host.models("ofox.ai").await.unwrap();
        assert_eq!(anthropic_reads(), 3);

        // Back up: the next page read gets the new rows at once.
        env.down.lock().unwrap().clear();
        env.anthropic
            .lock()
            .unwrap()
            .push("anthropic/claude-new".into());
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(anthropic_reads(), 4);
        assert_eq!(
            boot_ids(&boot),
            [
                "test/model",
                "anthropic/claude-test",
                "anthropic/claude-new"
            ]
        );
        // Fresh rows are not read again.
        host.bootstrap().await.unwrap();
        assert_eq!(anthropic_reads(), 4);

        // With no copy, each page read asks again and reports the gap.
        host.forget_models("ofox.ai");
        env.down.lock().unwrap().insert(catalog::Source::Anthropic);
        for read in [5, 6] {
            let boot = host.bootstrap().await.unwrap();
            assert_eq!(anthropic_reads(), read);
            assert_eq!(boot["partial"], json!(["anthropic"]));
            assert_eq!(boot_ids(&boot), ["test/model"]);
        }
        host.models("ofox.ai").await.unwrap();
        assert_eq!(anthropic_reads(), 6, "send-time reads stay paused");
    }

    /// Claude as `/anthropic/v1/models` lists it: the priciest first.
    const CLAUDE_IN_CATALOG_ORDER: [&str; 3] = [
        "anthropic/claude-fable-5",
        "anthropic/claude-opus-5",
        "anthropic/claude-haiku-5.5",
    ];

    fn create_request(value: Value) -> CreateRequest {
        serde_json::from_value(value).unwrap()
    }

    fn remembered_model(host: &AssistantHost) -> Option<String> {
        host.store(&identity().owner)
            .unwrap()
            .remembered_model()
            .unwrap()
    }

    fn settings_request(value: Value) -> SetPreferencesRequest {
        serde_json::from_value(value).unwrap()
    }

    #[tokio::test]
    async fn the_settings_are_saved_for_the_signed_in_account_without_a_known_key() {
        let (config, _env, host) = fixture_with_env();
        assert_eq!(
            host.get_preferences().await.unwrap(),
            Preferences::default()
        );
        let key = "sk-of-a-key-this-process-handed-out";
        host.remember_key(key);
        let saved = host
            .set_preferences(settings_request(json!({
                "instructions": format!("Sign as {key}"),
                "autoTitle": false,
                "notifyOnFinish": true,
            })))
            .await
            .unwrap();
        let expected = Preferences {
            instructions: format!("Sign as {}", crate::redact::REDACTED),
            auto_title: false,
            notify_on_finish: true,
        };
        assert_eq!(saved, expected);
        assert_eq!(host.get_preferences().await.unwrap(), expected);
        let store = host.store(&identity().owner).unwrap();
        assert_eq!(store.preferences().unwrap(), expected);
        // A damaged file must be surfaced to the settings editor.
        let prefs = config
            .path()
            .join("assistant")
            .join(identity().owner)
            .join("prefs.json");
        std::fs::write(&prefs, "damaged").unwrap();
        assert_eq!(
            host.get_preferences().await.unwrap_err(),
            "assistant_invalid_preferences"
        );
        let too_long = settings_request(json!({
            "instructions": "a".repeat(4001), "autoTitle": true, "notifyOnFinish": true,
        }));
        assert_eq!(
            host.set_preferences(too_long).await.unwrap_err(),
            "assistant_instructions_too_long"
        );
    }

    #[tokio::test]
    async fn settings_saved_for_another_account_or_authority_are_refused() {
        let (_config, host) = fixture();
        let settings = json!({"instructions": "x", "autoTitle": false, "notifyOnFinish": false});
        for (owner, generation) in [
            ("b".repeat(64), host.generation()),
            (identity().owner, host.generation() + 1),
        ] {
            let mut request = settings.clone();
            request["expectedOwner"] = json!(owner);
            request["expectedGeneration"] = json!(generation);
            assert_eq!(
                host.set_preferences(settings_request(request))
                    .await
                    .unwrap_err(),
                IDENTITY_CHANGED
            );
        }
        assert_eq!(
            host.get_preferences().await.unwrap(),
            Preferences::default()
        );
    }

    #[tokio::test]
    async fn with_v1_down_a_new_conversation_starts_on_the_cheapest_claude() {
        let (_config, env, host) = fixture_with_env();
        *env.anthropic.lock().unwrap() = CLAUDE_IN_CATALOG_ORDER.map(str::to_owned).to_vec();
        env.down.lock().unwrap().insert(catalog::Source::V1);
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(boot["partial"], json!(["v1"]));
        assert_eq!(boot["defaultModelId"], "anthropic/claude-haiku-5.5");
        let session = host
            .create_session(create_request(json!({})))
            .await
            .unwrap();
        assert_eq!(session.model_id, "anthropic/claude-haiku-5.5");
    }

    #[tokio::test]
    async fn a_conversation_created_while_models_are_missing_keeps_the_remembered_model() {
        let (_config, env, host) = fixture_with_env();
        *env.anthropic.lock().unwrap() = CLAUDE_IN_CATALOG_ORDER.map(str::to_owned).to_vec();
        host.store(&identity().owner)
            .unwrap()
            .remember_model("test/model")
            .unwrap();
        env.down.lock().unwrap().insert(catalog::Source::V1);
        // What the renderer sends unless the user picked: the default it
        // shows, or nothing.
        for request in [json!({"modelId": "anthropic/claude-haiku-5.5"}), json!({})] {
            let session = host.create_session(create_request(request)).await.unwrap();
            assert_eq!(session.model_id, "anthropic/claude-haiku-5.5");
            assert_eq!(remembered_model(&host).as_deref(), Some("test/model"));
        }
        // Once `/v1` is back, the remembered model is the default again.
        env.down.lock().unwrap().clear();
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(boot["partial"], json!([]));
        assert_eq!(boot["defaultModelId"], "test/model");
    }

    #[tokio::test]
    async fn a_model_the_user_picked_is_remembered_even_while_models_are_missing() {
        let (_config, env, host) = fixture_with_env();
        *env.anthropic.lock().unwrap() = CLAUDE_IN_CATALOG_ORDER.map(str::to_owned).to_vec();
        host.store(&identity().owner)
            .unwrap()
            .remember_model("test/model")
            .unwrap();
        env.down.lock().unwrap().insert(catalog::Source::V1);
        let picked = json!({"modelId": "anthropic/claude-opus-5", "modelPicked": true});
        host.create_session(create_request(picked)).await.unwrap();
        assert_eq!(
            remembered_model(&host).as_deref(),
            Some("anthropic/claude-opus-5")
        );
        // With every source read, the default a conversation starts on is
        // remembered, picked or not, as before. Here the remembered model
        // is gone from the catalog, so the default is the first preferred one.
        env.down.lock().unwrap().clear();
        host.bootstrap().await.unwrap();
        host.store(&identity().owner)
            .unwrap()
            .remember_model("gone/model")
            .unwrap();
        let session = host
            .create_session(create_request(json!({})))
            .await
            .unwrap();
        assert_eq!(session.model_id, "anthropic/claude-haiku-5.5");
        assert_eq!(
            remembered_model(&host).as_deref(),
            Some("anthropic/claude-haiku-5.5")
        );
    }

    #[tokio::test]
    async fn a_stale_default_from_an_earlier_outage_does_not_replace_the_remembered_model() {
        let (_config, env, host) = fixture_with_env();
        *env.anthropic.lock().unwrap() = CLAUDE_IN_CATALOG_ORDER.map(str::to_owned).to_vec();
        host.store(&identity().owner)
            .unwrap()
            .remember_model("test/model")
            .unwrap();
        // The page opened while `/v1` was down and showed the Claude default.
        env.down.lock().unwrap().insert(catalog::Source::V1);
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(boot["defaultModelId"], "anthropic/claude-haiku-5.5");
        // `/v1` is back, but the page still sends what it showed, unpicked.
        env.down.lock().unwrap().clear();
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(boot["partial"], json!([]));
        assert_eq!(boot["defaultModelId"], "test/model");
        let session = host
            .create_session(create_request(
                json!({"modelId": "anthropic/claude-haiku-5.5"}),
            ))
            .await
            .unwrap();
        // The conversation uses what the user was shown ...
        assert_eq!(session.model_id, "anthropic/claude-haiku-5.5");
        // ... and the remembered choice is the user's, not the stand-in.
        assert_eq!(remembered_model(&host).as_deref(), Some("test/model"));

        // A pick is still remembered.
        let picked = json!({"modelId": "anthropic/claude-haiku-5.5", "modelPicked": true});
        host.create_session(create_request(picked)).await.unwrap();
        assert_eq!(
            remembered_model(&host).as_deref(),
            Some("anthropic/claude-haiku-5.5")
        );
    }

    #[tokio::test]
    async fn an_unpicked_model_that_is_the_current_default_is_remembered_with_a_complete_catalog() {
        let (_config, env, host) = fixture_with_env();
        *env.anthropic.lock().unwrap() = CLAUDE_IN_CATALOG_ORDER.map(str::to_owned).to_vec();
        // Nothing remembered yet: the default is the first preferred model.
        assert_eq!(remembered_model(&host), None);
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(boot["partial"], json!([]));
        assert_eq!(boot["defaultModelId"], "anthropic/claude-haiku-5.5");
        // The page sends the default it shows, by name and unpicked.
        let session = host
            .create_session(create_request(
                json!({"modelId": "anthropic/claude-haiku-5.5"}),
            ))
            .await
            .unwrap();
        assert_eq!(session.model_id, "anthropic/claude-haiku-5.5");
        assert_eq!(
            remembered_model(&host).as_deref(),
            Some("anthropic/claude-haiku-5.5")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn opening_the_page_waits_only_briefly_for_a_hanging_source_whose_copy_stands_in() {
        let (_config, env, host) = fixture_with_env();
        let anthropic_reads = || env.anthropic_fetches.load(Ordering::SeqCst);
        *env.anthropic.lock().unwrap() = vec!["anthropic/claude-test".into()];
        host.models("ofox.ai").await.unwrap();
        // The copy goes stale, then the source hangs until the client gives up.
        age_catalogs(&host);
        env.hanging
            .lock()
            .unwrap()
            .insert(catalog::Source::Anthropic);
        host.models("ofox.ai").await.unwrap();
        assert_eq!(anthropic_reads(), 2);

        // Opening the page asks again, but opens on the copy within seconds.
        let opened = tokio::time::Instant::now();
        let boot = host.bootstrap().await.unwrap();
        assert!(
            opened.elapsed() <= PAGE_RETRY_WITH_COPY,
            "{:?}",
            opened.elapsed()
        );
        assert_eq!(anthropic_reads(), 3);
        assert_eq!(boot["models"][1]["id"], "anthropic/claude-test");
        assert_eq!(boot["partial"], json!([]));
        // Giving up counts as a failure: sends do not ask again at once.
        host.models("ofox.ai").await.unwrap();
        assert_eq!(anthropic_reads(), 3);

        // With no copy to show, the page waits for the read to end.
        host.forget_models("ofox.ai");
        let opened = tokio::time::Instant::now();
        let boot = host.bootstrap().await.unwrap();
        assert!(opened.elapsed() >= Duration::from_secs(15));
        assert_eq!(boot["partial"], json!(["anthropic"]));
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
            exclude_videos: false,
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
            exclude_videos: false,
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
            exclude_videos: false,
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
            exclude_videos: false,
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
            exclude_videos: false,
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
            exclude_videos: false,
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

    fn trashed_session(store: &AssistantStore) -> String {
        let sid = store.create("test/model", false).unwrap().session.id;
        store
            .update(
                &serde_json::from_value(json!({"sessionId":sid,"deleted":true})).unwrap(),
                || Ok(()),
            )
            .unwrap();
        sid
    }

    fn set_saved_run(store: &AssistantStore, sid: &str, running: bool) {
        store
            .mutate(sid, |record| {
                record.session.active_run = running.then(|| RunRef {
                    run_id: id(),
                    operation_id: id(),
                });
                Ok(())
            })
            .unwrap();
    }

    fn empty_trash_request(host: &AssistantHost) -> EmptyTrashRequest {
        EmptyTrashRequest {
            expected_owner: identity().owner,
            expected_generation: host.generation(),
        }
    }

    /// The names in the account's folder, sorted: session folders, tombstones.
    fn account_folder(config: &std::path::Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(config.join("assistant").join(identity().owner))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn emptying_the_trash_deletes_every_idle_trashed_conversation_from_disk() {
        let (config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let kept = store.create("test/model", false).unwrap().session.id;
        let (first, second) = (trashed_session(&store), trashed_session(&store));
        std::fs::write(store.native_history_path(&second).unwrap(), b"history").unwrap();
        std::fs::write(store.attachment_dir(&second).unwrap().join(id()), b"copy").unwrap();
        // A run the previous process left behind is settled, as a single
        // permanent deletion settles it, and the conversation goes.
        let left_behind = trashed_session(&store);
        set_saved_run(&store, &left_behind, true);
        // A reply still streaming keeps its conversation.
        let replying = trashed_session(&store);
        set_saved_run(&store, &replying, true);
        host.runs
            .lock()
            .unwrap()
            .insert(replying.clone(), control(&replying));

        let report = host.empty_trash(empty_trash_request(&host)).await.unwrap();
        assert_eq!(
            report,
            EmptyTrashReport {
                deleted: 3,
                failed: 1
            }
        );
        let mut remaining = vec![kept.clone(), replying.clone()];
        remaining.sort();
        assert_eq!(account_folder(config.path()), remaining);
        for gone in [&first, &second, &left_behind] {
            assert_eq!(
                store.load_record(gone).unwrap_err(),
                "assistant_session_not_found"
            );
        }

        // `trashCount` counts what is left, and the live list is untouched.
        let trash = host.list_sessions(true, "").await.unwrap();
        assert_eq!(trash.trash_count, 1);
        assert_eq!(trash.sessions[0].id, replying);
        let live = host.list_sessions(false, "").await.unwrap();
        assert_eq!(live.trash_count, 1);
        assert_eq!(live.sessions.len(), 1);
        assert_eq!(live.sessions[0].id, kept);

        // Once its reply ended, the next emptying takes the rest.
        host.runs.lock().unwrap().remove(&replying);
        set_saved_run(&store, &replying, false);
        let report = host.empty_trash(empty_trash_request(&host)).await.unwrap();
        assert_eq!(
            report,
            EmptyTrashReport {
                deleted: 1,
                failed: 0
            }
        );
        assert_eq!(account_folder(config.path()), [kept]);
        assert_eq!(host.list_sessions(false, "").await.unwrap().trash_count, 0);
        let report = host.empty_trash(empty_trash_request(&host)).await.unwrap();
        assert_eq!(report, EmptyTrashReport::default());
    }

    #[tokio::test]
    async fn emptying_the_trash_names_its_counts_as_the_renderer_reads_them() {
        let (_config, host) = fixture();
        let report = host.empty_trash(empty_trash_request(&host)).await.unwrap();
        assert_eq!(
            serde_json::to_value(report).unwrap(),
            json!({"deleted": 0, "failed": 0})
        );
    }

    #[tokio::test]
    async fn emptying_the_trash_checks_the_confirmed_account_and_generation() {
        let (config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = trashed_session(&store);
        let wrong_owner = EmptyTrashRequest {
            expected_owner: "b".repeat(64),
            ..empty_trash_request(&host)
        };
        let stale = EmptyTrashRequest {
            expected_generation: host.generation() + 1,
            ..empty_trash_request(&host)
        };
        for request in [wrong_owner, stale] {
            assert_eq!(
                host.empty_trash(request).await.unwrap_err(),
                IDENTITY_CHANGED
            );
        }
        assert_eq!(account_folder(config.path()), [sid]);
    }

    #[tokio::test]
    async fn a_conversation_restored_while_the_trash_is_emptied_stays_and_is_no_failure() {
        let (_config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let restored = trashed_session(&store);
        std::thread::sleep(std::time::Duration::from_millis(5));
        // Newer, so it is emptied first.
        let gone = trashed_session(&store);
        // Holding the other one's history lock parks the loop at it.
        let lock = host
            .native_lock(&session_key(&identity().owner, &restored))
            .unwrap();
        let held = lock.lock().await;
        let emptying = tokio::spawn({
            let (host, request) = (host.clone(), empty_trash_request(&host));
            async move { host.empty_trash(request).await }
        });
        // Once `gone` is deleted, the trash was listed and the loop waits at `restored`.
        for _ in 0..500 {
            if store.load_record(&gone).is_err() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(store.load_record(&gone).is_err());
        // The user presses Undo.
        let undo: UpdateRequest =
            serde_json::from_value(json!({"sessionId": restored, "deleted": false})).unwrap();
        store.update(&undo, || Ok(())).unwrap();
        drop(held);

        let report = emptying.await.unwrap().unwrap();
        assert_eq!(
            report,
            EmptyTrashReport {
                deleted: 1,
                failed: 0
            }
        );
        assert!(store
            .load_record(&restored)
            .unwrap()
            .session
            .deleted_at
            .is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_conversation_that_cannot_be_removed_does_not_stop_the_others() {
        let (config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let other = trashed_session(&store);
        std::thread::sleep(std::time::Duration::from_millis(5));
        // The newest trashed one is emptied first, so the loop has to go on
        // past its failure to reach the other.
        let blocked = trashed_session(&store);
        assert_eq!(store.list(true, "").unwrap().sessions[0].id, blocked);
        // A link where the deletion's private tombstone goes is refused.
        let folder = config.path().join("assistant").join(identity().owner);
        let outside = config.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, folder.join(format!(".deleting-{blocked}"))).unwrap();

        let report = host.empty_trash(empty_trash_request(&host)).await.unwrap();
        assert_eq!(
            report,
            EmptyTrashReport {
                deleted: 1,
                failed: 1
            }
        );
        assert!(store.load_record(&blocked).is_ok());
        assert!(store.load_record(&other).is_err());
        assert!(outside.is_dir(), "the link's target is never followed");
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

    /// A chat model that offers these thinking levels.
    fn thinker(id: &str, levels: &[&str]) -> catalog::Model {
        let mut row = model(id);
        row.reasoning = true;
        row.thinking_levels = levels.iter().map(|level| level.to_string()).collect();
        row
    }

    #[tokio::test]
    async fn a_thinking_level_must_be_one_the_model_offers_and_never_stops_the_run() {
        let (_config, env, host) = fixture_with_env();
        // Extra `/v1` rows: two chat models that reason.
        env.media.lock().unwrap().extend([
            thinker("test/thinker", &["off", "low", "high"]),
            thinker("test/low-only", &["low"]),
        ]);
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let update = |change: Value| -> UpdateRequest {
            let mut value = json!({ "sessionId": sid });
            value
                .as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            serde_json::from_value(value).unwrap()
        };
        let saved_level = || store.load_record(&sid).unwrap().session.thinking_level;
        const INVALID: &str = "assistant_invalid_thinking_level";

        // test/model does not reason: only the default.
        let refused = host.update_session(update(json!({"thinkingLevel": "high"})));
        assert_eq!(refused.await.unwrap_err(), INVALID);
        let cleared = host.update_session(update(json!({"thinkingLevel": "default"})));
        assert!(cleared.await.unwrap().thinking_level.is_none());

        // A model and one of its levels in one update.
        let both = json!({"modelId": "test/thinker", "thinkingLevel": "high"});
        let session = host.update_session(update(both)).await.unwrap();
        assert_eq!(session.thinking_level.as_deref(), Some("high"));

        // A level change leaves the reply running.
        let run = control(&sid);
        host.runs.lock().unwrap().insert(sid.clone(), run.clone());
        let session = host
            .update_session(update(json!({"thinkingLevel": "off"})))
            .await
            .unwrap();
        assert_eq!(session.thinking_level.as_deref(), Some("off"));
        assert!(!run.cancelled());
        let unknown = host.update_session(update(json!({"thinkingLevel": "max"})));
        assert_eq!(unknown.await.unwrap_err(), INVALID);
        assert_eq!(saved_level().as_deref(), Some("off"));

        // A new model keeps the level it offers too, else goes back to the default.
        let switched = host.update_session(update(json!({"modelId": "test/low-only"})));
        assert!(switched.await.unwrap().thinking_level.is_none());
        host.update_session(update(json!({"thinkingLevel": "low"})))
            .await
            .unwrap();
        let back = host.update_session(update(json!({"modelId": "test/thinker"})));
        assert_eq!(back.await.unwrap().thinking_level.as_deref(), Some("low"));
        let wrong = json!({"modelId": "test/low-only", "thinkingLevel": "high"});
        assert_eq!(
            host.update_session(update(wrong)).await.unwrap_err(),
            INVALID
        );
        assert_eq!(saved_level().as_deref(), Some("low"));
        assert_eq!(
            store.load_record(&sid).unwrap().session.model_id,
            "test/thinker",
            "a refused update changes nothing"
        );
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
    #[tokio::test]
    async fn audio_bootstrap_keeps_tool_models_out_of_chat_and_never_mints_a_key() {
        let (_config, env, host) = fixture_with_env();
        let audio = catalog::parse_models(
            json!({"data":[{"id":"google/gemini-3.8-flash-tts","supported_endpoints":["/v1/audio/speech"],"pricing":{"prompt":"0.0000005","output_audio":"0.000009"}}]}),
            catalog::Source::V1,
        );
        env.media.lock().unwrap().extend(audio);
        host.store(&identity().owner)
            .unwrap()
            .remember_model_kind("google/gemini-3.8-flash-tts", catalog::Kind::Speech)
            .unwrap();
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(boot["audioModels"].as_array().map(Vec::len), Some(1));
        assert_eq!(boot["audioModels"][0]["audio"]["defaultFormat"], "wav");
        assert_eq!(boot["models"].as_array().unwrap().len(), 1);
        assert_eq!(boot["defaultModelId"], "test/model");
        assert_eq!(
            boot["rememberedModels"]["speech"],
            "google/gemini-3.8-flash-tts"
        );
        assert_eq!(boot["key"]["status"], "missing");
        let create =
            serde_json::from_value(json!({"modelId":"google/gemini-3.8-flash-tts"})).unwrap();
        assert_eq!(
            host.create_session(create).await.unwrap_err(),
            MODEL_UNAVAILABLE
        );
    }
    fn audio_target(host: &AssistantHost, sid: &str) -> DeleteRequest {
        DeleteRequest {
            session_id: sid.into(),
            expected_owner: identity().owner,
            expected_generation: host.generation(),
        }
    }
    #[tokio::test]
    async fn audio_import_is_private_temporary_and_does_not_create_a_chat_turn_or_mint_keys() {
        let (config, env, host) = fixture_with_env();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let before = serde_json::to_value(store.load_record(&sid).unwrap()).unwrap();
        let bytes = include_bytes!("../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3");
        let path = config.path().join("picked.bin");
        std::fs::write(&path, bytes).unwrap();
        let picked = host
            .pick_audio(audio_target(&host, &sid), move || Ok(Some(path)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(picked.name, "picked.mp3");
        let first = host
            .import_audio(
                audio_target(&host, &sid),
                "voice.txt".into(),
                bytes.to_vec(),
            )
            .await
            .unwrap();
        assert_eq!(first.name, "voice.mp3");
        assert_eq!(first.info.mime_type, "audio/mpeg");
        let encoded = serde_json::to_value(&first).unwrap();
        assert!(encoded.get("bytes").is_none());
        assert!(encoded.get("path").is_none());
        assert_eq!(
            before,
            serde_json::to_value(store.load_record(&sid).unwrap()).unwrap()
        );
        assert_eq!(
            std::fs::read_dir(store.attachment_dir(&sid).unwrap())
                .unwrap()
                .count(),
            0
        );
        assert!(env.events.lock().unwrap().is_empty());
        assert_eq!(env.fetches.load(Ordering::SeqCst), 0);
        assert!(host
            .import_audio(
                audio_target(&host, &sid),
                "broken.wav".into(),
                b"RIFF".to_vec()
            )
            .await
            .is_err());
        let second = host
            .import_audio(audio_target(&host, &sid), "next.mp3".into(), bytes.to_vec())
            .await
            .unwrap();
        let discard = |id| super::super::audio_input::DiscardRequest {
            authority: audio_target(&host, &sid),
            input_id: id,
        };
        assert!(!host.discard_audio(discard(first.input_id)).await.unwrap());
        assert!(host
            .discard_audio(discard(second.input_id.clone()))
            .await
            .unwrap());
        assert!(!host.discard_audio(discard(second.input_id)).await.unwrap());
        let pending = host
            .import_audio(
                audio_target(&host, &sid),
                "pending.mp3".into(),
                bytes.to_vec(),
            )
            .await
            .unwrap();
        host.cancel_all();
        assert!(!host.discard_audio(discard(pending.input_id)).await.unwrap());
    }
    #[tokio::test]
    async fn audio_import_checks_authority_before_the_picker_and_after_it_returns() {
        let (config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let mut wrong = audio_target(&host, &sid);
        wrong.expected_generation += 1;
        assert_eq!(
            host.pick_audio(wrong, || panic!("a stale request opens no picker"))
                .await
                .unwrap_err(),
            IDENTITY_CHANGED
        );
        assert!(host
            .pick_audio(audio_target(&host, &sid), || Ok(None))
            .await
            .unwrap()
            .is_none());
        let file = config.path().join("audio.mp3");
        std::fs::write(
            &file,
            include_bytes!("../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3"),
        )
        .unwrap();
        let changed = host.clone();
        let result = host
            .pick_audio(audio_target(&host, &sid), move || {
                changed.cancel_all();
                Ok(Some(file))
            })
            .await;
        assert_eq!(result.unwrap_err(), IDENTITY_CHANGED);
    }

    #[tokio::test]
    async fn audio_import_refuses_concurrent_validation_and_can_discard_after_session_deletion() {
        let (_config, host) = fixture();
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        let bytes = include_bytes!("../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3");
        let ready = host
            .import_audio(
                audio_target(&host, &sid),
                "input.mp3".into(),
                bytes.to_vec(),
            )
            .await
            .unwrap();
        let (opened, wait) = tokio::sync::oneshot::channel();
        let (release, dialog) = std::sync::mpsc::channel();
        let picking = tokio::spawn({
            let host = host.clone();
            let target = audio_target(&host, &sid);
            async move {
                host.pick_audio(target, move || {
                    let _ = opened.send(());
                    let _ = dialog.recv();
                    Ok(None)
                })
                .await
            }
        });
        wait.await.unwrap();
        assert_eq!(
            host.import_audio(
                audio_target(&host, &sid),
                "other.mp3".into(),
                bytes.to_vec()
            )
            .await
            .unwrap_err(),
            "assistant_session_busy"
        );
        release.send(()).unwrap();
        assert!(picking.await.unwrap().unwrap().is_none());
        store
            .mutate(&sid, |r| {
                r.session.deleted_at = Some(1);
                Ok(())
            })
            .unwrap();
        assert!(host
            .discard_audio(super::super::audio_input::DiscardRequest {
                authority: audio_target(&host, &sid),
                input_id: ready.input_id
            })
            .await
            .unwrap());
    }
    #[tokio::test]
    async fn cache_first_page_returns_a_stale_copy_before_a_hanging_source() {
        let (_config, env, host) = fixture_with_env();
        *env.anthropic.lock().unwrap() = vec!["anthropic/claude-test".into()];
        host.models("ofox.ai").await.unwrap();
        age_catalogs(&host);
        env.hanging
            .lock()
            .unwrap()
            .insert(catalog::Source::Anthropic);
        let boot = tokio::time::timeout(Duration::from_secs(1), host.bootstrap_cached(false))
            .await
            .expect("cached page must not wait for a source timeout")
            .unwrap();
        assert_eq!(boot["models"][1]["id"], "anthropic/claude-test");
        assert_eq!(boot["catalogRefreshing"], true);
        let again = tokio::time::timeout(Duration::from_secs(1), host.bootstrap_cached(false))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(again["catalogRefreshing"], true);
        for _ in 0..100 {
            if env.anthropic_fetches.load(Ordering::SeqCst) >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            env.anthropic_fetches.load(Ordering::SeqCst),
            2,
            "reopening must share one refresh"
        );
        let receipt = host.begin_logout();
        host.finish_logout(receipt);
    }
    #[tokio::test]
    async fn cache_first_cold_page_publishes_the_healthy_source_before_the_other_finishes() {
        let (_config, env, host) = fixture_with_env();
        env.hanging
            .lock()
            .unwrap()
            .insert(catalog::Source::Anthropic);
        let boot = tokio::time::timeout(Duration::from_secs(1), host.bootstrap_cached(false))
            .await
            .expect("cold page should open before sources finish")
            .unwrap();
        assert_eq!(boot["catalogRefreshing"], true);
        for _ in 0..100 {
            if env.events.lock().unwrap().iter().any(|e| {
                e.r#type == "catalog_state"
                    && e.data["catalog"]["models"]
                        .as_array()
                        .is_some_and(|m| m.iter().any(|m| m["id"] == "test/model"))
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let events = env.events.lock().unwrap();
        let event = events
            .iter()
            .find(|e| {
                e.r#type == "catalog_state"
                    && e.data["catalog"]["models"]
                        .as_array()
                        .is_some_and(|m| m.iter().any(|m| m["id"] == "test/model"))
            })
            .expect("healthy source must publish while another is pending");
        assert_eq!(event.data["owner"], identity().owner);
        assert_eq!(event.data["generation"], host.generation());
        assert_eq!(event.data["catalog"]["catalogRefreshing"], true);
        drop(events);
        let receipt = host.begin_logout();
        host.finish_logout(receipt);
    }
    async fn wait_catalog_work(host: &AssistantHost) {
        for _ in 0..200 {
            if host.background.load(Ordering::SeqCst) == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("catalog background work did not settle");
    }
    #[tokio::test]
    async fn cache_first_failed_cold_sources_obey_cooldown_and_explicit_retry() {
        let (_config, env, host) = fixture_with_env();
        env.down
            .lock()
            .unwrap()
            .extend([catalog::Source::V1, catalog::Source::Anthropic]);
        host.bootstrap_cached(false).await.unwrap();
        wait_catalog_work(&host).await;
        assert_eq!(env.fetches.load(Ordering::SeqCst), 1);
        assert_eq!(env.anthropic_fetches.load(Ordering::SeqCst), 1);
        let again = host.bootstrap_cached(false).await.unwrap();
        assert_eq!(again["catalogRefreshing"], false);
        assert!(again["modelUnavailableReason"].is_string());
        assert_eq!(env.fetches.load(Ordering::SeqCst), 1);
        host.bootstrap_cached(true).await.unwrap();
        wait_catalog_work(&host).await;
        assert_eq!(env.fetches.load(Ordering::SeqCst), 2);
        assert_eq!(env.anthropic_fetches.load(Ordering::SeqCst), 2);
        env.down.lock().unwrap().clear();
        host.bootstrap_cached(true).await.unwrap();
        wait_catalog_work(&host).await;
        let recovered = host.bootstrap_cached(false).await.unwrap();
        assert_eq!(recovered["models"][0]["id"], "test/model");
        assert_eq!(recovered["catalogRefreshFailed"], json!([]));
    }
    #[tokio::test]
    async fn cache_first_explicit_retry_refetches_a_valid_but_empty_catalog() {
        let (_config, env, host) = fixture_with_env();
        env.catalog.lock().unwrap().clear();
        host.bootstrap_cached(false).await.unwrap();
        wait_catalog_work(&host).await;
        assert_eq!(env.fetches.load(Ordering::SeqCst), 1);
        env.catalog.lock().unwrap().push("recovered/model".into());
        host.bootstrap_cached(true).await.unwrap();
        wait_catalog_work(&host).await;
        assert_eq!(
            host.bootstrap_cached(false).await.unwrap()["models"][0]["id"],
            "recovered/model"
        );
        assert_eq!(env.fetches.load(Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn cache_first_region_change_uses_only_selected_apex_rows_and_events() {
        let (_config, env, host) = fixture_with_env();
        host.models("ofox.ai").await.unwrap();
        age_catalogs(&host);
        env.hanging
            .lock()
            .unwrap()
            .insert(catalog::Source::Anthropic);
        host.bootstrap_cached(false).await.unwrap();
        *env.apex.lock().unwrap() = "ofox.io".into();
        *env.catalog.lock().unwrap() = vec!["io/model".into()];
        env.hanging.lock().unwrap().clear();
        host.cancel_all();
        let generation = host.generation();
        let boot = host.bootstrap_cached(false).await.unwrap();
        assert!(!boot["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "test/model"));
        wait_catalog_work(&host).await;
        let events = env.events.lock().unwrap();
        let current: Vec<_> = events
            .iter()
            .filter(|e| e.r#type == "catalog_state" && e.data["generation"] == generation)
            .collect();
        assert!(current.iter().any(|e| e.data["catalog"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "io/model")));
        assert!(current.iter().all(|e| !e.data["catalog"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "test/model")));
        assert!(env
            .catalog_apexes
            .lock()
            .unwrap()
            .iter()
            .any(|apex| apex == "ofox.io"));
    }
    #[tokio::test]
    async fn cache_first_shutdown_aborts_pending_fetches_and_releases_locks() {
        let (_config, env, host) = fixture_with_env();
        env.hanging
            .lock()
            .unwrap()
            .extend([catalog::Source::V1, catalog::Source::Anthropic]);
        host.bootstrap_cached(false).await.unwrap();
        for _ in 0..100 {
            if env.fetches.load(Ordering::SeqCst) > 0
                && env.anthropic_fetches.load(Ordering::SeqCst) > 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::timeout(Duration::from_secs(1), host.shutdown())
            .await
            .unwrap();
        assert!(host.catalog_pending.lock().unwrap().is_empty());
        assert_eq!(host.background.load(Ordering::SeqCst), 0);
        assert!(host
            .catalog_locks
            .lock()
            .unwrap()
            .values()
            .all(|lock| lock.try_lock().is_ok()));
    }
    #[tokio::test(start_paused = true)]
    async fn cache_first_send_and_refresh_source_reads_share_one_fetch_and_failure_cooldown() {
        let (_config, env, host) = fixture_with_env();
        env.hanging
            .lock()
            .unwrap()
            .insert(catalog::Source::Anthropic);
        let (first, second) = tokio::join!(host.models("ofox.ai"), host.models("ofox.ai"));
        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(env.fetches.load(Ordering::SeqCst), 1);
        assert_eq!(env.anthropic_fetches.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn cache_first_read_only_refresh_can_run_before_paid_admission_reopens() {
        let (_config, env, host) = fixture_with_env();
        let receipt = host.begin_logout();
        host.finish_logout(receipt);
        assert!(host.ensure_accepting_runs().is_err());
        host.bootstrap_cached(false).await.unwrap();
        wait_catalog_work(&host).await;
        assert_eq!(env.fetches.load(Ordering::SeqCst), 1);
        assert_eq!(
            host.bootstrap_cached(false).await.unwrap()["models"][0]["id"],
            "test/model"
        );
    }
}
