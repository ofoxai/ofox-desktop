//! The assistant host and its commands end to end: the real prepared runtime
//! (Node + Pi SDK) against a local mock gateway that speaks the OpenAI APIs
//! and, under `/anthropic/`, Anthropic Messages, with the conversation files
//! in a temp dir.
use cc_switch_lib::assistant::{
    catalog::{parse_models, Model, Source},
    host::{Admit, AssistantHost, BoxFuture, Environment, Identity},
    types::{
        self, CreateRequest, DeleteRequest, Detail, Event, ImportDropRequest, Message, RunProxy,
        SendRequest, SwitchVersionRequest,
    },
};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const KEY: &str = "synthetic-assistant-key-Q7v2Lm9X";
const NEW_KEY: &str = "synthetic-assistant-key-replacement-T4n8";
const MODEL: &str = "test/chat-model";

/// The prepared runtime, or `None` (with a message) when it is missing on a
/// developer machine. In CI a missing runtime is a failure.
fn runtime_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime/resources");
    let node = dir
        .join("node")
        .join(if cfg!(windows) { "node.exe" } else { "node" });
    if node.is_file() && dir.join("runtime/src/main.mjs").is_file() {
        return Some(dir);
    }
    assert!(
        std::env::var_os("CI").is_none(),
        "the assistant runtime is not prepared at {}; CI must run `pnpm assistant:prepare` before cargo test",
        dir.display()
    );
    eprintln!(
        "skipping: the assistant runtime is not prepared at {} (run `pnpm assistant:prepare`)",
        dir.display()
    );
    None
}

// ── mock gateway ─────────────────────────────────────────────────────────

#[derive(Clone)]
enum Reply {
    /// A complete streamed answer.
    Stream(String),
    /// Streamed reasoning (taking a while), then this answer.
    Thinking(String, String),
    /// Starts streaming, then holds the connection open until the client goes.
    Hang(String),
    /// An HTTP error with this body.
    Error(u16, Value),
    /// Reads the request, then closes the connection without an answer.
    Drop,
    /// A streamed turn that calls these tools (name, arguments).
    ToolCalls(Vec<(&'static str, Value)>),
    /// A JSON answer, as the Responses API gives one to the search server.
    Json(u16, Value),
    /// This reply, once the gate opens.
    Wait(Arc<AtomicBool>, Box<Reply>),
    /// A 200 with these bytes, as a speech request is answered.
    Bytes(&'static str, Vec<u8>),
}

/// Tool-call ids are unique, as a provider's are.
static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn invalid_key() -> Value {
    json!({"error":{"message":"Invalid API key","type":"invalid_request_error","code":"invalid_api_key"}})
}

#[derive(Clone, Debug)]
struct Request {
    path: String,
    authorization: String,
    headers: HashMap<String, String>,
    body: Value,
}

impl Request {
    fn text(&self) -> String {
        self.body["messages"].to_string()
    }
}

/// What the gateway answers and what it was asked. Title requests (at most
/// 32 output tokens) have a script of their own; unscripted ones are refused
/// with a 400, which costs nothing and changes no conversation.
#[derive(Clone, Default)]
struct Script {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Request>>>,
    titles: Arc<Mutex<VecDeque<Reply>>>,
    title_requests: Arc<Mutex<Vec<Request>>>,
}

struct Gateway {
    port: u16,
    script: Script,
}

impl Gateway {
    fn start() -> Self {
        let v4 = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = v4.local_addr().unwrap().port();
        let script = Script::default();
        let mut listeners = vec![v4];
        // `localhost` may resolve to ::1 first.
        if let Ok(v6) = TcpListener::bind(("::1", port)) {
            listeners.push(v6);
        }
        for listener in listeners {
            let script = script.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let script = script.clone();
                    std::thread::spawn(move || serve(stream, &script));
                }
            });
        }
        Self { port, script }
    }

    fn base_url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    fn reply(&self, reply: Reply) {
        self.script.replies.lock().unwrap().push_back(reply);
    }

    /// The answer to the next title request.
    fn title(&self, reply: Reply) {
        self.script.titles.lock().unwrap().push_back(reply);
    }

    /// Every request but titles.
    fn requests(&self) -> Vec<Request> {
        self.script.requests.lock().unwrap().clone()
    }

    fn title_requests(&self) -> Vec<Request> {
        self.script.title_requests.lock().unwrap().clone()
    }
}

/// A request for a generated title: it asks for at most 32 output tokens.
fn is_title_request(body: &Value) -> bool {
    body["max_tokens"] == 32 || body["max_output_tokens"] == 32
}

fn serve(stream: TcpStream, script: &Script) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut headers = HashMap::new();
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let body = if let Some(length) = headers.get("content-length") {
        let mut body = vec![0; length.parse().unwrap()];
        reader.read_exact(&mut body).unwrap();
        body
    } else if headers
        .get("transfer-encoding")
        .is_some_and(|v| v == "chunked")
    {
        read_chunked(&mut reader)
    } else {
        Vec::new()
    };
    let request = Request {
        path: path.clone(),
        authorization: headers.get("authorization").cloned().unwrap_or_default(),
        headers,
        body: if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    };
    let mut reply = if is_title_request(&request.body) {
        script.title_requests.lock().unwrap().push(request);
        let scripted = script.titles.lock().unwrap().pop_front();
        scripted.unwrap_or_else(|| Reply::Error(400, json!({"error": {"message": "unscripted"}})))
    } else {
        script.requests.lock().unwrap().push(request);
        script
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("the gateway got a request the test did not script")
    };
    while let Reply::Wait(gate, inner) = reply {
        while !gate.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        reply = *inner;
    }
    let mut out = stream;
    // Pi's Anthropic client asks for `/anthropic/v1/messages?beta=true`.
    let messages_api = path.split('?').next() == Some("/anthropic/v1/messages");
    match reply {
        Reply::Stream(text) if messages_api => anthropic_stream(&mut out, None, &text),
        Reply::Thinking(thinking, text) if messages_api => {
            anthropic_stream(&mut out, Some(&thinking), &text)
        }
        Reply::Thinking(thinking, text) => {
            start_stream(&mut out);
            for part in split_in_three(&thinking) {
                let chunk = json!({"id":"mock","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":part},"finish_reason":null}]});
                let _ = out.write_all(format!("data: {chunk}\n\n").as_bytes());
                let _ = out.flush();
                std::thread::sleep(THINKING_PAUSE);
            }
            for part in split_in_three(&text) {
                write_delta(&mut out, &part);
            }
            let _ = out.write_all(
                format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}})
                )
                .as_bytes(),
            );
        }
        Reply::Stream(text) => {
            start_stream(&mut out);
            for part in split_in_three(&text) {
                write_delta(&mut out, &part);
                std::thread::sleep(Duration::from_millis(15));
            }
            let _ = out.write_all(
                format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}})
                )
                .as_bytes(),
            );
        }
        Reply::Hang(text) => {
            start_stream(&mut out);
            write_delta(&mut out, &text);
            // Until the runtime aborts the request (or is killed).
            out.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
            let _ = reader.read(&mut [0; 1]);
        }
        Reply::Error(code, body) => {
            let body = body.to_string();
            let _ = out.write_all(
                format!(
                    "HTTP/1.1 {code} Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
        Reply::Drop => {
            let _ = out.shutdown(std::net::Shutdown::Both);
        }
        Reply::ToolCalls(calls) => {
            start_stream(&mut out);
            let calls: Vec<Value> = calls
                .iter()
                .enumerate()
                .map(|(index, (name, args))| {
                    let id = CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    json!({"index": index, "id": format!("call-{id}"), "type": "function",
                           "function": {"name": name, "arguments": args.to_string()}})
                })
                .collect();
            let chunk = json!({"id":"mock","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":calls},"finish_reason":null}]});
            let _ = out.write_all(
                format!(
                    "data: {chunk}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}})
                )
                .as_bytes(),
            );
        }
        Reply::Wait(..) => unreachable!("opened above"),
        Reply::Bytes(mime, bytes) => {
            let _ = out.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: {mime}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    bytes.len()
                )
                .as_bytes(),
            );
            let _ = out.write_all(&bytes);
        }
        Reply::Json(code, body) => {
            let body = body.to_string();
            let _ = out.write_all(
                format!(
                    "HTTP/1.1 {code} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    }
    let _ = out.flush();
}

fn read_chunked(reader: &mut impl BufRead) -> Vec<u8> {
    let mut body = vec![];
    loop {
        let mut size = String::new();
        reader.read_line(&mut size).unwrap();
        let size = usize::from_str_radix(size.trim(), 16).unwrap();
        let mut chunk = vec![0; size + 2];
        reader.read_exact(&mut chunk).unwrap();
        if size == 0 {
            return body;
        }
        body.extend_from_slice(&chunk[..size]);
    }
}

fn start_stream(out: &mut TcpStream) {
    out.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n",
    )
    .unwrap();
}

fn write_delta(out: &mut TcpStream, text: &str) {
    let chunk = json!({"id":"mock","choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":null}]});
    let _ = out.write_all(format!("data: {chunk}\n\n").as_bytes());
    let _ = out.flush();
}

/// Between two parts of streamed reasoning, so it takes measurable time.
const THINKING_PAUSE: Duration = Duration::from_millis(80);

fn write_event(out: &mut TcpStream, event: Value) {
    let kind = event["type"].as_str().unwrap_or_default();
    let _ = out.write_all(format!("event: {kind}\ndata: {event}\n\n").as_bytes());
    let _ = out.flush();
}

/// A Messages API answer as `/anthropic/v1/messages` streams it: reasoning in
/// a thinking block when given, then the text.
fn anthropic_stream(out: &mut TcpStream, thinking: Option<&str>, text: &str) {
    start_stream(out);
    write_event(
        out,
        json!({"type":"message_start","message":{"id":"msg_mock","type":"message","role":"assistant","model":"mock","content":[],"stop_reason":null,"usage":{"input_tokens":10,"output_tokens":0}}}),
    );
    let mut index = 0;
    if let Some(thinking) = thinking {
        write_event(
            out,
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        );
        for part in split_in_three(thinking) {
            write_event(
                out,
                json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":part}}),
            );
            std::thread::sleep(THINKING_PAUSE);
        }
        write_event(
            out,
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"mock-signature"}}),
        );
        write_event(out, json!({"type":"content_block_stop","index":0}));
        index = 1;
    }
    write_event(
        out,
        json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":""}}),
    );
    for part in split_in_three(text) {
        write_event(
            out,
            json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":part}}),
        );
        std::thread::sleep(Duration::from_millis(15));
    }
    write_event(out, json!({"type":"content_block_stop","index":index}));
    write_event(
        out,
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":5}}),
    );
    write_event(out, json!({"type":"message_stop"}));
}

fn split_in_three(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let third = chars.len().div_ceil(3).max(1);
    chars.chunks(third).map(|c| c.iter().collect()).collect()
}

// ── environment ──────────────────────────────────────────────────────────

struct TestEnv {
    local_paths: Mutex<Option<cc_switch_lib::assistant::tools::context::LocalPaths>>,
    skill_db: Mutex<Option<Arc<cc_switch_lib::Database>>>,
    config: PathBuf,
    runtime: PathBuf,
    state: Mutex<EnvState>,
    events: Mutex<Vec<Event>>,
    /// When set, a key fetch waits here first, as on a busy key lock.
    key_gate: Mutex<Option<Arc<tokio::sync::Notify>>>,
    key_waiting: std::sync::atomic::AtomicBool,
    /// The proxy model requests should take.
    proxy: Mutex<Option<RunProxy>>,
    /// The models the catalog offers.
    catalog: Mutex<Vec<Model>>,
    /// The rows `/anthropic/v1/models` gives.
    anthropic: Mutex<Vec<Model>>,
    video_price: Mutex<String>,
}

struct EnvState {
    identity: Result<Identity, String>,
    key: String,
    replacement: Option<String>,
    rejected: Vec<String>,
    key_minted: bool,
    /// Refuse to replace a refused key, as for one minted moments ago.
    refuse_refresh: bool,
    gateways: HashMap<String, String>,
}

fn owner() -> String {
    "ab".repeat(32)
}

impl TestEnv {
    fn new(config: &Path, runtime: &Path, gateways: &[(&str, &Gateway)]) -> Arc<Self> {
        Arc::new(Self {
            local_paths: Mutex::new(None),
            skill_db: Mutex::new(None),
            config: config.to_path_buf(),
            runtime: runtime.to_path_buf(),
            state: Mutex::new(EnvState {
                identity: Ok(Identity {
                    owner: owner(),
                    apex: gateways[0].0.into(),
                }),
                key: KEY.into(),
                replacement: Some(NEW_KEY.into()),
                rejected: vec![],
                key_minted: false,
                refuse_refresh: false,
                gateways: gateways
                    .iter()
                    .map(|(apex, gateway)| (apex.to_string(), gateway.base_url()))
                    .collect(),
            }),
            events: Mutex::new(vec![]),
            key_gate: Mutex::new(None),
            key_waiting: std::sync::atomic::AtomicBool::new(false),
            proxy: Mutex::new(None),
            catalog: Mutex::new(vec![test_model()]),
            anthropic: Mutex::default(),
            video_price: Mutex::new("0.03".into()),
        })
    }

    fn set_identity(&self, identity: Result<Identity, String>) {
        self.state.lock().unwrap().identity = identity;
    }

    fn signed_in(&self, apex: &str) {
        self.set_identity(Ok(Identity {
            owner: owner(),
            apex: apex.into(),
        }));
    }

    /// Send `apex`'s model requests to `base_url`.
    fn route(&self, apex: &str, base_url: String) {
        self.state
            .lock()
            .unwrap()
            .gateways
            .insert(apex.into(), base_url);
    }

    fn events(&self, run_id: &str) -> Vec<Event> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.run_id == run_id)
            .cloned()
            .collect()
    }
}

impl Environment for TestEnv {
    fn local_paths(
        &self,
    ) -> BoxFuture<'_, Result<cc_switch_lib::assistant::tools::context::LocalPaths, String>> {
        Box::pin(async {
            self.local_paths
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| "assistant_local_unavailable".into())
        })
    }

    fn skill_database(&self) -> Result<Arc<cc_switch_lib::Database>, String> {
        self.skill_db
            .lock()
            .unwrap()
            .clone()
            .ok_or("skill_service_unavailable".into())
    }
    fn skill_snapshot(
        &self,
    ) -> Result<cc_switch_lib::assistant_target::AssistantSkillSnapshot, String> {
        match self.skill_db.lock().unwrap().as_ref() {
            Some(db) => cc_switch_lib::assistant::skills::snapshot(db),
            None => Ok(Default::default()),
        }
    }
    fn skill_read(
        &self,
        proof: &cc_switch_lib::assistant_target::AssistantSkillProof,
        access: cc_switch_lib::assistant_target::SkillAccess,
        read: cc_switch_lib::assistant::skills::SkillRead,
    ) -> Result<Value, String> {
        let db = self.skill_db.lock().unwrap().clone().ok_or("no skill db")?;
        cc_switch_lib::assistant::skills::read_verified(&db, proof, access, read)
    }

    fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
        Box::pin(async move { self.state.lock().unwrap().identity.clone() })
    }

    fn api_key<'a>(
        &'a self,
        identity: &'a Identity,
        rejected: Option<&'a str>,
        admit: &'a Admit<'a>,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let gate = self.key_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                self.key_waiting
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                gate.notified().await;
            }
            // What the key lock asks before it hands out or mints a key.
            admit()?;
            let mut state = self.state.lock().unwrap();
            if state.identity.as_ref() != Ok(identity) {
                return Err("assistant_sign_in_required".into());
            }
            if let Some(rejected) = rejected {
                state.rejected.push(rejected.into());
                if state.refuse_refresh {
                    return Err("assistant_key_rejected".into());
                }
                if state.key == rejected {
                    if let Some(next) = state.replacement.take() {
                        state.key = next;
                    }
                }
            }
            state.key_minted = true;
            Ok(state.key.clone())
        })
    }

    fn key_ready<'a>(&'a self, _identity: &'a Identity) -> BoxFuture<'a, bool> {
        Box::pin(async move { self.state.lock().unwrap().key_minted })
    }

    fn models<'a>(
        &'a self,
        _apex: &'a str,
        source: Source,
    ) -> BoxFuture<'a, Result<Vec<Model>, String>> {
        Box::pin(async move {
            Ok(match source {
                Source::V1 => self.catalog.lock().unwrap().clone(),
                Source::Anthropic => self.anthropic.lock().unwrap().clone(),
            })
        })
    }

    fn gateway_base(&self, apex: &str) -> Result<String, String> {
        self.state
            .lock()
            .unwrap()
            .gateways
            .get(apex)
            .cloned()
            .ok_or_else(|| "assistant_unknown_apex".into())
    }

    fn provider_prices<'a>(
        &'a self,
        _apex: &'a str,
        _model: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async {
            Ok(
                json!({"provider_cards":[{"pricing":{"video_pricing":{"unit":"per_second","tiers":[{"resolution":"480p","input_type":"t2v","price":self.video_price.lock().unwrap().clone()}]}}}]}),
            )
        })
    }
    fn config_dir(&self) -> PathBuf {
        self.config.clone()
    }

    fn runtime_dir(&self) -> Result<PathBuf, String> {
        Ok(self.runtime.clone())
    }

    fn emit(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }

    fn proxy(&self) -> Option<RunProxy> {
        self.proxy.lock().unwrap().clone()
    }
}

fn test_model() -> Model {
    serde_json::from_value(json!({
        "id": MODEL,
        "name": "Test chat model",
        "contextWindow": 128000,
        "maxOutputTokens": 4096,
        "supportsImages": false,
        "inputModalities": ["text"],
        "supportedEndpoints": ["/v1/chat/completions"],
        "inputPricePerMillion": "2",
        "outputPricePerMillion": "10",
        "pricing": {"prompt": "0.000002", "completion": "0.00001"},
        "cost": {"input": 2.0, "output": 10.0, "cacheRead": null, "cacheWrite": null},
        "pricingKnown": true,
    }))
    .unwrap()
}

// ── helpers ──────────────────────────────────────────────────────────────

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn request(sid: &str, text: &str, extra: Value) -> SendRequest {
    let mut value = json!({
        "sessionId": sid,
        "runId": types::id(),
        "operationId": types::id(),
        "text": text,
    });
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(value).unwrap()
}

async fn create(host: &AssistantHost) -> String {
    let request: CreateRequest = serde_json::from_value(json!({})).unwrap();
    host.create_session(request).await.unwrap().id
}

async fn wait_for(env: &TestEnv, run_id: &str, kind: &str) -> Event {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(event) = env.events(run_id).into_iter().find(|e| e.r#type == kind) {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "no {kind} event for run {run_id}; got {:?}",
            env.events(run_id)
                .iter()
                .map(|e| e.r#type.clone())
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Send and wait for the end of the run; returns the run id.
async fn turn(host: &Arc<AssistantHost>, env: &TestEnv, request: SendRequest) -> (String, Event) {
    let run = host.send(request).await.unwrap();
    let end = wait_for(env, &run.run_id, "run_end").await;
    (run.run_id, end)
}

fn active(detail: &Detail) -> Vec<(String, String)> {
    detail
        .messages
        .iter()
        .filter(|m| !m.superseded)
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect()
}

fn superseded(detail: &Detail) -> Vec<&Message> {
    detail.messages.iter().filter(|m| m.superseded).collect()
}

fn session_dir(config: &Path, sid: &str) -> PathBuf {
    config.join("assistant").join(owner()).join(sid)
}

fn saved(config: &Path, sid: &str) -> Value {
    serde_json::from_slice(&std::fs::read(session_dir(config, sid).join("session.json")).unwrap())
        .unwrap()
}

/// No file under `root` holds any of `secrets`.
fn assert_no_secret_on_disk(root: &Path, secrets: &[&str]) {
    let mut files = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files += 1;
                let bytes = std::fs::read(&path).unwrap();
                for secret in secrets {
                    assert!(
                        !bytes
                            .windows(secret.len())
                            .any(|window| window == secret.as_bytes()),
                        "{secret} was written to {}",
                        path.display()
                    );
                }
            }
        }
    }
    assert!(files > 0, "nothing was written under {}", root.display());
}

fn assert_no_secret_in_events(env: &TestEnv, secrets: &[&str]) {
    let events = serde_json::to_string(&*env.events.lock().unwrap()).unwrap();
    for secret in secrets {
        assert!(!events.contains(secret), "{secret} reached an event");
    }
}

// ── tests ────────────────────────────────────────────────────────────────

#[test]
fn a_multi_turn_conversation_streams_and_is_saved_in_both_histories() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(boot["owner"], owner());
        assert_eq!(boot["runtime"]["available"], true);
        assert_eq!(boot["runtime"]["version"], "1.5.0");
        assert_eq!(boot["defaultModelId"], MODEL);
        assert_eq!(boot["models"][0]["id"], MODEL);
        assert_eq!(boot["key"]["status"], "missing", "bootstrap mints no key");

        let sid = create(&host).await;
        gateway.reply(Reply::Stream(format!(
            "First answer, and the key {KEY} is never shown"
        )));
        let (first, end) = turn(&host, &env, request(&sid, "Hello there", json!({}))).await;
        assert_eq!(end.data, json!({"stopped": false, "status": "completed"}));

        let events = env.events(&first);
        let kinds: Vec<_> = events.iter().map(|e| e.r#type.as_str()).collect();
        assert_eq!(kinds.first(), Some(&"run_start"));
        assert_eq!(kinds.last(), Some(&"run_end"));
        let message_id = events[0].data["messageId"].as_str().unwrap().to_owned();
        let deltas: String = events
            .iter()
            .filter(|e| e.r#type == "text_delta")
            .inspect(|e| assert_eq!(e.data["messageId"], message_id))
            .map(|e| e.data["delta"].as_str().unwrap().to_owned())
            .collect();
        assert!(deltas.starts_with("First answer"), "{deltas}");
        let usage = events.iter().find(|e| e.r#type == "usage").unwrap();
        assert_eq!(usage.data["usage"]["inputTokens"], 10);
        // The saved message goes out before run_end.
        let message = events.iter().position(|e| e.r#type == "message").unwrap();
        let session = events.iter().position(|e| e.r#type == "session").unwrap();
        assert!(message < session && session < events.len() - 1, "{kinds:?}");
        assert_eq!(events[message].data["message"]["id"], message_id);
        assert_eq!(events[message].data["message"]["status"], "completed");
        assert!(events[session].data["session"]["activeRun"].is_null());

        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.session.title, "Hello there");
        assert!(detail.session.active_run.is_none());
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(detail.messages[0].content, "Hello there");
        assert_eq!(detail.messages[1].id, message_id);
        assert!(detail.messages[1].content.starts_with("First answer"));
        assert!(detail.messages.iter().all(|m| m.status == "completed"));
        assert!(detail.messages.iter().all(|m| m.native_entry_id.is_some()));
        assert_eq!(detail.usage["modelRequests"], 1);
        let native =
            std::fs::read_to_string(session_dir(config.path(), &sid).join("session.jsonl"))
                .unwrap();
        assert!(native.contains("Hello there") && native.contains("First answer"));
        assert_eq!(
            saved(config.path(), &sid)["messages"][1]["content"],
            json!(detail.messages[1].content)
        );
        assert_eq!(host.bootstrap().await.unwrap()["key"]["status"], "ready");

        gateway.reply(Reply::Stream("Second answer".into()));
        let (_, end) = turn(&host, &env, request(&sid, "And then?", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let requests = gateway.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests
            .iter()
            .all(|r| r.authorization == format!("Bearer {KEY}")));
        let context = requests[1].text();
        assert!(context.contains("Hello there") && context.contains("First answer"));
        assert!(context.contains("And then?"));
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(
            active(&detail)
                .iter()
                .map(|(role, _)| role.as_str())
                .collect::<Vec<_>>(),
            ["user", "assistant", "user", "assistant"]
        );
        assert_eq!(detail.messages[3].content, "Second answer");

        let list = host.list_sessions(false, "").await.unwrap();
        assert_eq!(list.sessions.len(), 1);
        assert!(list.unreadable.is_empty());
        assert!(list.matches.is_empty());
        // The first message named the conversation; nobody renamed it.
        assert_eq!(list.sessions[0].title, "Hello there");
        assert_eq!(
            list.sessions[0].title_source,
            Some(types::TitleSource::Auto)
        );
        let found = host.list_sessions(false, "then").await.unwrap();
        assert_eq!(found.sessions.len(), 1);
        // The search jumps to the first message that says it.
        assert_eq!(found.matches.get(&sid), Some(&detail.messages[2].id));
        assert!(host
            .list_sessions(false, "zzz")
            .await
            .unwrap()
            .sessions
            .is_empty());
        let damaged = types::id();
        std::fs::create_dir_all(session_dir(config.path(), &damaged)).unwrap();
        std::fs::write(
            session_dir(config.path(), &damaged).join("session.json"),
            "{",
        )
        .unwrap();
        let list = host.list_sessions(false, "").await.unwrap();
        assert_eq!(list.unreadable.len(), 1);
        assert_eq!(list.unreadable[0].id, damaged);
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

#[test]
fn ordinary_queue_sends_keep_distinct_runs_context_and_usage_on_both_apexes() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    for apex in ["ofox.ai", "ofox.io"] {
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &runtime_dir, &[(apex, &gateway)]);
        let host = AssistantHost::new(env.clone());
        runtime().block_on(async {
            host.set_preferences(
                serde_json::from_value(json!({
                    "instructions": "", "autoTitle": false, "notifyOnFinish": false,
                    "expectedOwner": owner(), "expectedGeneration": host.generation(),
                }))
                .unwrap(),
            )
            .await
            .unwrap();
            let sid = create(&host).await;
            let boot = host.bootstrap().await.unwrap();
            let mut refs = Vec::new();
            for index in 1..=3 {
                let question = format!("Queued question {index}");
                let answer = format!("Ordinary answer {index}");
                let request = request(
                    &sid,
                    &question,
                    json!({
                        "expectedOwner": boot["owner"],
                        "expectedGeneration": boot["authorityGeneration"],
                    }),
                );
                refs.push((request.run_id.clone(), request.operation_id.clone()));
                gateway.reply(Reply::Stream(answer));
                let (_, end) = turn(&host, &env, request).await;
                assert_eq!(end.data["status"], "completed");
            }
            let requests = gateway.requests();
            assert_eq!(requests.len(), 3);
            for (index, request) in requests.iter().enumerate() {
                let text = request.text();
                for previous in 1..=index {
                    assert!(text.contains(&format!("Queued question {previous}")));
                    assert!(text.contains(&format!("Ordinary answer {previous}")));
                }
            }
            let detail = host.get_session(&sid).await.unwrap();
            assert_eq!(detail.messages.len(), 6);
            assert_eq!(detail.usage["modelRequests"], 3);
            let record = saved(config.path(), &sid);
            let operations = record["operations"].as_array().unwrap();
            assert_eq!(operations.len(), 3);
            for (index, (run_id, operation_id)) in refs.iter().enumerate() {
                assert_eq!(operations[index]["runId"], *run_id);
                assert_eq!(operations[index]["id"], *operation_id);
                assert_eq!(detail.messages[index * 2].run_id.as_ref(), Some(run_id));
                assert_eq!(detail.messages[index * 2 + 1].run_id.as_ref(), Some(run_id));
                assert_eq!(
                    detail.messages[index * 2 + 1].usage.as_ref().unwrap()["modelRequests"],
                    1
                );
                assert!(refs[..index]
                    .iter()
                    .all(|old| old.0 != *run_id && old.1 != *operation_id));
            }
        });
        assert_no_secret_on_disk(config.path(), &[KEY]);
        assert_no_secret_in_events(&env, &[KEY]);
    }
}

#[test]
fn delayed_queue_send_is_refused_after_region_or_account_switch_before_minting_a_key() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    for (from, to) in [("ofox.ai", "ofox.io"), ("ofox.io", "ofox.ai")] {
        let config = tempfile::tempdir().unwrap();
        let original = Gateway::start();
        let selected = Gateway::start();
        let env = TestEnv::new(
            config.path(),
            &runtime_dir,
            &[(from, &original), (to, &selected)],
        );
        let host = AssistantHost::new(env.clone());
        runtime().block_on(async {
            let sid = create(&host).await;
            let queued = request(
                &sid,
                "Do not forward old queued text",
                json!({
                    "expectedOwner": owner(), "expectedGeneration": host.generation(),
                }),
            );
            env.signed_in(to);
            host.cancel_all();
            assert_eq!(
                host.send(queued).await.unwrap_err(),
                "assistant_identity_changed"
            );
            let queued = request(
                &sid,
                "Not this account's text",
                json!({
                    "expectedOwner": owner(), "expectedGeneration": host.generation(),
                }),
            );
            env.set_identity(Ok(Identity {
                owner: "cd".repeat(32),
                apex: to.into(),
            }));
            assert_eq!(
                host.send(queued).await.unwrap_err(),
                "assistant_identity_changed"
            );
            assert!(!env.state.lock().unwrap().key_minted);
            assert!(original.requests().is_empty());
            assert!(selected.requests().is_empty());
            assert!(saved(config.path(), &sid)["operations"]
                .as_array()
                .unwrap()
                .is_empty());
            assert!(saved(config.path(), &sid)["messages"]
                .as_array()
                .unwrap()
                .is_empty());
        });
    }
}

#[test]
fn live_steering_uses_the_existing_sidecar_and_keeps_a_fenced_ledger() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(root.path(), &bundle, &[("ofox.io", &gateway)]);
        let host = AssistantHost::new(env.clone());
        let sid = create(&host).await;
        let gate = Arc::new(AtomicBool::new(false));
        gateway.reply(Reply::Wait(gate.clone(), Box::new(Reply::Stream("First answer".into()))));
        gateway.reply(Reply::Stream("Steered answer".into()));
        let run = host.send(request(&sid,"First question",json!({}))).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while gateway.requests().is_empty() { assert!(Instant::now()<deadline); tokio::time::sleep(Duration::from_millis(10)).await; }
        let input = json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"id":types::id(),"text":format!("Change course {KEY}"),"expectedOwner":owner(),"expectedGeneration":host.generation()});
        let mut stale = input.clone(); stale["expectedGeneration"]=json!(host.generation()+1);
        assert_eq!(host.steer(serde_json::from_value(stale).unwrap()).await.unwrap_err(),"assistant_identity_changed");
        host.steer(serde_json::from_value(input.clone()).unwrap()).await.unwrap();
        host.steer(serde_json::from_value(input).unwrap()).await.unwrap();
        gate.store(true,Ordering::SeqCst);
        let end = wait_for(&env,&run.run_id,"run_end").await;
        assert_eq!(end.data["status"],"completed");
        assert_eq!(gateway.requests().len(),2);
        assert!(gateway.requests()[1].text().contains("Change course"));
        let detail=host.get_session(&sid).await.unwrap();
        assert_eq!(detail.messages.iter().filter(|m|m.role=="user").count(),2);
        assert_eq!(detail.messages.iter().filter(|m|m.role=="assistant").count(),2);
        assert!(detail.messages.iter().all(|m|m.status=="completed"));
        assert_eq!(
            detail
                .messages
                .iter()
                .map(|m| m.role.as_str())
                .collect::<Vec<_>>(),
            vec!["user", "assistant", "user", "assistant"]
        );
        let replies: Vec<_> = detail
            .messages
            .iter()
            .filter(|m| m.role == "assistant")
            .collect();
        assert_ne!(replies[0].run_id, replies[1].run_id);
        assert!(replies
            .iter()
            .all(|m| m.usage.as_ref().unwrap()["modelRequests"] == 1));
        assert_eq!(detail.usage["modelRequests"],2);
        let record=saved(root.path(),&sid);
        let ops=record["operations"].as_array().unwrap();
        assert_eq!(ops.len(),2);
        assert!(ops.iter().all(|op|op["status"]=="completed"));
        assert!(env.events(&run.run_id).iter().any(|event|event.r#type=="steer_applied"));
        assert_no_secret_on_disk(root.path(),&[KEY]);
        assert_no_secret_in_events(&env,&[KEY]);
        host.shutdown().await;
    });
}

#[test]
fn stopping_live_steering_keeps_the_unsent_input_without_replaying_it() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        let root=tempfile::tempdir().unwrap(); let gateway=Gateway::start();
        let env=TestEnv::new(root.path(),&bundle,&[("ofox.io",&gateway)]); let host=AssistantHost::new(env.clone());
        let sid=create(&host).await;
        gateway.reply(Reply::Hang("Partial response ".repeat(20)));
        let run=host.send(request(&sid,"First",json!({}))).await.unwrap();
        wait_for(&env,&run.run_id,"text_delta").await;
        let id=types::id();
        host.steer(serde_json::from_value(json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"id":id,"text":"Unsent interruption","expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap()).await.unwrap();
        host.stop(&sid,&run.run_id,&run.operation_id).await.unwrap();
        wait_for(&env,&run.run_id,"run_end").await;
        let detail=host.get_session(&sid).await.unwrap();
        let user=detail.messages.iter().find(|m|m.id==id).unwrap();
        assert_eq!(user.content,"Unsent interruption"); assert_eq!(user.status,"error");
        assert_eq!(user.error_code.as_deref(),Some("assistant_cancelled"));
        assert_eq!(gateway.requests().len(),1);
        let saved=saved(root.path(),&sid);
        assert!(saved["operations"].as_array().unwrap().iter().all(|op|op["status"]=="cancelled"));
        host.shutdown().await;
    });
}

/// Everyday code and configuration that looks like credentials.
const LOOKALIKES: &str = concat!(
    "OpenAI(api_key=os.environ[\"OPENAI_API_KEY\"])\n",
    "password = input(\"Password: \")\n",
    "interface User { password: string; secret: boolean }\n",
    "Authorization: Bearer YOUR_TOKEN\n",
    "max_tokens: 4096\n",
    "sk-proj-EXAMPLEexample0123456789",
);

/// The text of a message's content: a string or text parts.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        parts => parts
            .as_array()
            .into_iter()
            .flatten()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect(),
    }
}

/// The user and assistant texts of a native JSONL history.
fn native_texts(native: &str) -> Vec<String> {
    native
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| {
            entry["type"] == "message"
                && (entry["message"]["role"] == "user" || entry["message"]["role"] == "assistant")
        })
        .map(|entry| text_of(&entry["message"]["content"]))
        .collect()
}

#[test]
fn conversation_text_passes_byte_for_byte_and_only_the_key_is_scrubbed() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Stream(format!("{LOOKALIKES}\necho {KEY}")));
        let question = format!("{LOOKALIKES}\nmy key {KEY}");
        let (run, end) = turn(&host, &env, request(&sid, &question, json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let asked =
            |text: &str| text.starts_with(&format!("{LOOKALIKES}\nmy key ")) && !text.contains(KEY);
        let answered =
            |text: &str| text.starts_with(&format!("{LOOKALIKES}\necho ")) && !text.contains(KEY);

        let sent = text_of(
            &gateway.requests()[0].body["messages"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["content"],
        );
        assert!(asked(&sent), "the model got {sent:?}");

        let events = env.events(&run);
        let streamed: String = events
            .iter()
            .filter(|e| e.r#type == "text_delta")
            .map(|e| e.data["delta"].as_str().unwrap())
            .collect();
        assert!(answered(&streamed), "streamed {streamed:?}");
        let message = events.iter().find(|e| e.r#type == "message").unwrap();
        assert!(answered(
            message.data["message"]["content"].as_str().unwrap()
        ));
        let session = events.iter().find(|e| e.r#type == "session").unwrap();
        assert_eq!(
            session.data["session"]["title"],
            LOOKALIKES.chars().take(48).collect::<String>()
        );

        let saved = saved(config.path(), &sid);
        assert!(asked(saved["messages"][0]["content"].as_str().unwrap()));
        assert!(answered(saved["messages"][1]["content"].as_str().unwrap()));
        let native =
            std::fs::read_to_string(session_dir(config.path(), &sid).join("session.jsonl"))
                .unwrap();
        let texts = native_texts(&native);
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert!(asked(&texts[0]) && answered(&texts[1]), "{texts:?}");

        // A draft and a title are the user's own words as well.
        let update: types::UpdateRequest = serde_json::from_value(json!({
            "sessionId": sid,
            "draft": format!("{LOOKALIKES} {KEY}"),
            "title": "max_tokens: 4096 password: hunter2",
        }))
        .unwrap();
        host.update_session(update).await.unwrap();
        let detail = host.get_session(&sid).await.unwrap();
        assert!(
            detail.session.draft.starts_with(&format!("{LOOKALIKES} "))
                && !detail.session.draft.contains(KEY),
            "{:?}",
            detail.session.draft
        );
        assert_eq!(detail.session.title, "max_tokens: 4096 password: hunter2");
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

/// The part of a request's system prompt the runtime writes (Pi adds the
/// working folder after it).
fn system_preamble(request: &Request) -> String {
    let prompt = request.body["messages"][0]["content"].as_str().unwrap();
    prompt
        .split("\n<cwd>")
        .next()
        .unwrap()
        .trim_end()
        .to_owned()
}

#[test]
fn saved_settings_shape_the_next_message_and_can_turn_titles_off() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        // The defaults: no instructions, and the first answer asks for a title.
        let sid = create(&host).await;
        gateway.reply(Reply::Stream("Plain answer.".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Hello", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let plain = system_preamble(&gateway.requests()[0]);
        assert!(!plain.contains("standing instructions"), "{plain}");
        title_requests(&gateway, 1).await;

        let settings: types::SetPreferencesRequest = serde_json::from_value(json!({
            "instructions": "Always answer in haiku.",
            "autoTitle": false,
            "notifyOnFinish": true,
        }))
        .unwrap();
        host.set_preferences(settings).await.unwrap();
        // From the next message on, in this conversation and a new one.
        gateway.reply(Reply::Stream("Five, seven, five.".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Again", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let other = create(&host).await;
        gateway.reply(Reply::Stream("Autumn moonlight.".into()));
        let (_, end) = turn(&host, &env, request(&other, "Hi", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        for sent in &gateway.requests()[1..] {
            let instructed = system_preamble(sent);
            assert!(
                instructed.starts_with(&format!("{plain}\n")),
                "{instructed}"
            );
            assert!(
                instructed.ends_with("\nAlways answer in haiku."),
                "{instructed}"
            );
        }
        // Titles are off: the new conversation's first answer asks for none.
        no_more_title_requests(&gateway, 1).await;
        assert_eq!(host.get_session(&other).await.unwrap().session.title, "Hi");
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn stopping_mid_stream_keeps_the_partial_reply_and_the_stopped_operation() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Hang("Partial answer ".repeat(20)));
        let send = request(&sid, "Tell me a long story", json!({}));
        let run = host.send(send.clone()).await.unwrap();
        wait_for(&env, &run.run_id, "text_delta").await;
        assert_eq!(
            host.get_session(&sid).await.unwrap().session.active_run,
            Some(run.clone())
        );
        host.stop(&sid, &run.run_id, &run.operation_id)
            .await
            .unwrap();
        let end = wait_for(&env, &run.run_id, "run_end").await;
        assert_eq!(end.data, json!({"stopped": true, "status": "stopped"}));
        assert!(env.events(&run.run_id).iter().all(|e| e.r#type != "error"));

        let detail = host.get_session(&sid).await.unwrap();
        assert!(detail.session.active_run.is_none());
        let reply = &detail.messages[1];
        assert_eq!(reply.status, "stopped");
        assert!(reply.content.starts_with("Partial answer"), "{reply:?}");
        assert_eq!(detail.usage["usageCoverage"], "incomplete");
        assert_eq!(
            saved(config.path(), &sid)["operations"][0]["status"],
            "cancelled"
        );
        // The same operation never starts twice.
        assert_eq!(host.send(send).await.unwrap_err(), "assistant_cancelled");

        // A stop that arrives before its send wins.
        let early = request(&sid, "Never sent", json!({}));
        host.stop(&sid, &early.run_id, &early.operation_id)
            .await
            .unwrap();
        assert_eq!(host.send(early).await.unwrap_err(), "assistant_cancelled");
        assert_eq!(gateway.requests().len(), 1);

        gateway.reply(Reply::Stream("Short story".into()));
        let (_, end) = turn(&host, &env, request(&sid, "A short one then", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn regenerate_and_edit_branch_and_keep_the_superseded_versions() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Stream("Answer one".into()));
        turn(&host, &env, request(&sid, "Question A", json!({}))).await;
        let first = host.get_session(&sid).await.unwrap();
        let answer_one = first.messages[1].id.clone();

        gateway.reply(Reply::Stream("Answer two".into()));
        let (_, end) = turn(
            &host,
            &env,
            request(&sid, "", json!({"regenerateMessageId": answer_one})),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        let regenerated = host.get_session(&sid).await.unwrap();
        assert_eq!(
            active(&regenerated),
            [
                ("user".into(), "Question A".into()),
                ("assistant".into(), "Answer two".into())
            ]
        );
        let old: Vec<_> = superseded(&regenerated)
            .iter()
            .map(|m| m.content.clone())
            .collect();
        assert_eq!(old, ["Question A", "Answer one"]);
        let answer_two = regenerated
            .messages
            .iter()
            .find(|m| m.content == "Answer two")
            .unwrap();
        assert_eq!(answer_two.revision_of.as_deref(), Some(answer_one.as_str()));
        let resent = gateway.requests()[1].text();
        assert!(resent.contains("Question A") && !resent.contains("Answer one"));

        let question = regenerated
            .messages
            .iter()
            .find(|m| m.role == "user" && !m.superseded)
            .unwrap()
            .id
            .clone();
        gateway.reply(Reply::Stream("Answer three".into()));
        turn(
            &host,
            &env,
            request(&sid, "Question B", json!({"editMessageId": question})),
        )
        .await;
        let edited = host.get_session(&sid).await.unwrap();
        assert_eq!(
            active(&edited),
            [
                ("user".into(), "Question B".into()),
                ("assistant".into(), "Answer three".into())
            ]
        );
        assert_eq!(superseded(&edited).len(), 4);
        let edit = gateway.requests()[2].text();
        assert!(edit.contains("Question B"));
        assert!(!edit.contains("Question A") && !edit.contains("Answer two"));

        // A turn that never reached the runtime cannot be branched from.
        let unknown = request(&sid, "x", json!({"editMessageId": types::id()}));
        assert_eq!(
            host.send(unknown).await.unwrap_err(),
            "assistant_branch_unavailable"
        );
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn a_restarted_host_settles_the_interrupted_run_and_continues_with_its_context() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);

    // The first app process: one finished turn, then a crash mid-stream.
    let crashed = runtime();
    let host = AssistantHost::new(env.clone());
    let (sid, interrupted) = crashed.block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Stream("Noted: apricot".into()));
        turn(
            &host,
            &env,
            request(&sid, "Remember the word apricot", json!({})),
        )
        .await;
        gateway.reply(Reply::Hang("Thinking about it ".repeat(20)));
        let run = host
            .send(request(&sid, "Second question", json!({})))
            .await
            .unwrap();
        wait_for(&env, &run.run_id, "text_delta").await;
        (sid, run)
    });
    // Dropping the runtime drops the run mid-stream, as a crash would; the
    // owned runtime process goes with it.
    crashed.shutdown_timeout(Duration::from_secs(2));
    drop(host);
    let left = saved(config.path(), &sid);
    assert_eq!(left["session"]["activeRun"]["runId"], interrupted.run_id);
    assert_eq!(left["operations"][1]["status"], "running");

    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        host.recover_all().await;
        let saved_now = saved(config.path(), &sid);
        assert!(saved_now["session"]["activeRun"].is_null());
        assert_eq!(saved_now["operations"][1]["status"], "interrupted");
        let detail = host.get_session(&sid).await.unwrap();
        assert!(detail.session.active_run.is_none());
        assert_eq!(detail.messages.len(), 4);
        assert_eq!(detail.messages[2].content, "Second question");
        assert!(detail.messages[2].native_entry_id.is_some());
        assert_eq!(detail.messages[3].status, "stopped");
        assert_eq!(detail.usage["usageCoverage"], "incomplete");

        gateway.reply(Reply::Stream("You asked me to remember apricot".into()));
        let (_, end) = turn(
            &host,
            &env,
            request(&sid, "Which word did I give you?", json!({})),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        let context = gateway.requests().last().unwrap().text();
        assert!(context.contains("Remember the word apricot"));
        assert!(context.contains("Noted: apricot"));
        assert!(context.contains("Second question"));
        assert!(context.contains("Which word did I give you?"));
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn logging_out_cancels_the_run_and_blocks_sends_until_the_next_sign_in() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Hang("Partial answer ".repeat(20)));
        let run = host
            .send(request(&sid, "Long question", json!({})))
            .await
            .unwrap();
        wait_for(&env, &run.run_id, "text_delta").await;

        let receipt = host.begin_logout();
        let end = wait_for(&env, &run.run_id, "run_end").await;
        assert_eq!(end.data["stopped"], true);
        let during = request(&sid, "While signing out", json!({}));
        assert_eq!(
            host.send(during).await.unwrap_err(),
            "assistant_signing_out"
        );

        host.finish_logout(receipt);
        env.set_identity(Err("assistant_sign_in_required".into()));
        assert_eq!(
            host.bootstrap().await.unwrap_err(),
            "assistant_sign_in_required"
        );
        let signed_out = request(&sid, "Signed out", json!({}));
        assert_eq!(
            host.send(signed_out).await.unwrap_err(),
            "assistant_sign_in_required"
        );
        // Credentials back alone do not reopen admission; the sign-in does.
        env.signed_in("ofox.ai");
        let early = request(&sid, "Before sign-in completed", json!({}));
        assert_eq!(
            host.send(early).await.unwrap_err(),
            "assistant_sign_in_required"
        );

        host.sign_in_completed();
        gateway.reply(Reply::Stream("Welcome back".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Hello again", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        assert_eq!(gateway.requests().len(), 2);
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn a_run_keeps_the_region_it_started_in() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let global = Gateway::start();
    let china = Gateway::start();
    let env = TestEnv::new(
        config.path(),
        &runtime_dir,
        &[("ofox.ai", &global), ("ofox.io", &china)],
    );
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        global.reply(Reply::Hang("Partial answer ".repeat(20)));
        let run = host
            .send(request(&sid, "Where am I?", json!({})))
            .await
            .unwrap();
        wait_for(&env, &run.run_id, "text_delta").await;
        // The region changes under the run: it ends rather than move hosts.
        env.signed_in("ofox.io");
        wait_for(&env, &run.run_id, "run_end").await;
        let error = wait_for(&env, &run.run_id, "error").await;
        assert_eq!(error.data["code"], "assistant_identity_changed");
        assert_eq!(global.requests().len(), 1);
        assert!(china.requests().is_empty());

        // The next run starts in the new region, and region-change
        // cancellation stops it without closing admission.
        china.reply(Reply::Hang("Partial answer ".repeat(20)));
        let run = host
            .send(request(&sid, "And now?", json!({})))
            .await
            .unwrap();
        wait_for(&env, &run.run_id, "text_delta").await;
        assert_eq!(china.requests().len(), 1);
        host.cancel_all();
        let end = wait_for(&env, &run.run_id, "run_end").await;
        assert_eq!(end.data["stopped"], true);
        china.reply(Reply::Stream("Still here".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Still there?", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        assert_eq!(global.requests().len(), 1);
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn a_refused_key_is_replaced_once_and_the_turn_is_kept_once() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Error(401, invalid_key()));
        // The retry's reply echoes both keys of the run: neither may show.
        gateway.reply(Reply::Stream(format!(
            "Answer with the new key {NEW_KEY} and the old {KEY}"
        )));
        let (run, end) = turn(&host, &env, request(&sid, "Unique question", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let requests = gateway.requests();
        assert_eq!(requests[0].authorization, format!("Bearer {KEY}"));
        assert_eq!(requests[1].authorization, format!("Bearer {NEW_KEY}"));
        assert_eq!(env.state.lock().unwrap().rejected, [KEY]);
        let streamed: String = env
            .events(&run)
            .iter()
            .filter(|e| e.r#type == "text_delta")
            .map(|e| e.data["delta"].as_str().unwrap().to_owned())
            .collect();
        assert!(
            streamed.starts_with("Answer with the new key "),
            "{streamed}"
        );
        let detail = host.get_session(&sid).await.unwrap();
        let shown = active(&detail);
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[0], ("user".into(), "Unique question".into()));
        assert!(
            shown[1].1.starts_with("Answer with the new key ")
                && shown[1].1.contains(" and the old "),
            "{shown:?}"
        );
        assert!(superseded(&detail).is_empty());
        let native =
            std::fs::read_to_string(session_dir(config.path(), &sid).join("session.jsonl"))
                .unwrap();
        assert_eq!(native.matches("Unique question").count(), 1);

        // A key refused again after the retry ends the run: one retry only.
        gateway.reply(Reply::Error(401, invalid_key()));
        gateway.reply(Reply::Error(401, invalid_key()));
        let (run, end) = turn(&host, &env, request(&sid, "Again", json!({}))).await;
        assert_eq!(end.data["status"], "error");
        let error = wait_for(&env, &run, "error").await;
        assert_eq!(error.data["code"], "assistant_key_rejected");
        assert_eq!(gateway.requests().len(), 4);
    });
    assert_no_secret_on_disk(config.path(), &[KEY, NEW_KEY]);
    assert_no_secret_in_events(&env, &[KEY, NEW_KEY]);
}

#[test]
fn provider_failures_have_their_own_codes_and_unbilled_ones_keep_the_cost_known() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Stream("A priced answer".into()));
        turn(&host, &env, request(&sid, "First", json!({}))).await;
        let priced = host.get_session(&sid).await.unwrap().usage;
        assert!(priced["estimatedCostUsd"].is_string(), "{priced}");

        let assert_failed = |code: &'static str| {
            let (host, env) = (host.clone(), env.clone());
            async move {
                let (run, end) = turn(&host, &env, request(&sid_of(&host).await, code, json!({}))).await;
                assert_eq!(end.data["status"], "error", "{code}");
                let error = wait_for(&env, &run, "error").await;
                assert_eq!(error.data["code"], code);
                let detail = host.get_session(&sid_of(&host).await).await.unwrap();
                assert_eq!(detail.messages.last().unwrap().status, "error", "{code}");
                let saved = serde_json::to_value(detail.messages.last().unwrap()).unwrap();
                assert_eq!(saved["errorCode"], code, "the reason must survive reload");
                let published = wait_for(&env, &run, "message").await;
                assert_eq!(published.data["message"]["errorCode"], code);
                let restarted = AssistantHost::new(env.clone());
                let reloaded = restarted.get_session(&detail.session.id).await.unwrap();
                let restored = serde_json::to_value(reloaded.messages.last().unwrap()).unwrap();
                assert_eq!(restored["errorCode"], code, "a fresh host keeps the reason");
                detail.usage
            }
        };

        // Refused with a 4xx: nothing was generated, the cost stays known.
        for (reply, code) in [
            (
                Reply::Error(
                    402,
                    json!({"error":{"message":"Insufficient credits. Current balance: $0. Please top up to continue.","type":"insufficient_credits","code":402}}),
                ),
                "assistant_insufficient_balance",
            ),
            (
                Reply::Error(
                    429,
                    json!({"error":{"message":"Rate limit reached","type":"rate_limit_error"}}),
                ),
                "assistant_rate_limited",
            ),
            (
                Reply::Error(
                    404,
                    json!({"error":{"message":"The model does not exist","type":"invalid_request_error"}}),
                ),
                "assistant_model_unavailable",
            ),
        ] {
            if code == "assistant_rate_limited" {
                gateway.reply(reply.clone());
                gateway.reply(reply.clone());
            }
            gateway.reply(reply);
            let usage = assert_failed(code).await;
            assert_eq!(usage["estimatedCostUsd"], priced["estimatedCostUsd"], "{code}");
            assert_ne!(usage["usageCoverage"], "incomplete", "{code}");
        }

        // Never sent (connection refused): also a known zero.
        env.route("ofox.io", format!("http://localhost:{}", closed_port()));
        env.signed_in("ofox.io");
        let usage = assert_failed("assistant_network_error").await;
        assert_eq!(usage["estimatedCostUsd"], priced["estimatedCostUsd"]);
        assert_ne!(usage["usageCoverage"], "incomplete");
        env.signed_in("ofox.ai");

        // Sent, then the connection closed without an answer: maybe billed.
        gateway.reply(Reply::Drop);
        let usage = assert_failed("assistant_network_error").await;
        assert!(usage["estimatedCostUsd"].is_null(), "{usage}");
        assert_eq!(usage["usageCoverage"], "incomplete");

        // A server error may have followed upstream billing.
        gateway.reply(Reply::Error(
            500,
            json!({"error":{"message":"upstream failed"}}),
        ));
        let usage = assert_failed("assistant_model_request_failed").await;
        assert!(usage["estimatedCostUsd"].is_null(), "{usage}");

        gateway.reply(Reply::Stream("Back again".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Still there?", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

/// An HTTP CONNECT proxy that records each tunnel's target and credentials,
/// then tunnels to it.
struct Proxy {
    port: u16,
    tunnels: Arc<Mutex<Vec<(String, String)>>>,
}

impl Proxy {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let tunnels = Arc::new(Mutex::new(Vec::new()));
        let recorded = tunnels.clone();
        std::thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let recorded = recorded.clone();
                std::thread::spawn(move || tunnel(client, &recorded));
            }
        });
        Self { port, tunnels }
    }

    fn tunnels(&self) -> Vec<(String, String)> {
        self.tunnels.lock().unwrap().clone()
    }
}

fn tunnel(client: TcpStream, tunnels: &Mutex<Vec<(String, String)>>) {
    let mut reader = BufReader::new(client.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let target = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let mut authorization = String::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header.trim_end().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("proxy-authorization") {
                authorization = value.trim().to_owned();
            }
        }
    }
    tunnels
        .lock()
        .unwrap()
        .push((target.clone(), authorization));
    let mut client = client;
    let Ok(upstream) = TcpStream::connect(&target) else {
        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
        return;
    };
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .unwrap();
    let (mut up_read, mut up_write) = (upstream.try_clone().unwrap(), upstream);
    let mut client_write = client.try_clone().unwrap();
    let forward = std::thread::spawn(move || {
        let _ = std::io::copy(&mut reader, &mut up_write);
        let _ = up_write.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut up_read, &mut client_write);
    let _ = client_write.shutdown(std::net::Shutdown::Both);
    let _ = forward.join();
}

#[test]
fn model_requests_take_the_application_proxy() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let proxy = Proxy::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    *env.proxy.lock().unwrap() = Some(RunProxy {
        url: format!("http://ofox:proxy-secret-9@127.0.0.1:{}", proxy.port),
        no_proxy: String::new(),
    });
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Stream("Answer through the proxy".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Behind a proxy", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let tunnels = proxy.tunnels();
        assert_eq!(tunnels.len(), 1, "{tunnels:?}");
        assert_eq!(tunnels[0].0, format!("localhost:{}", gateway.port));
        assert!(tunnels[0].1.starts_with("Basic "), "{tunnels:?}");
        assert_eq!(gateway.requests().len(), 1);
    });
    assert_no_secret_on_disk(config.path(), &[KEY, "proxy-secret-9"]);
    assert_no_secret_in_events(&env, &[KEY, "proxy-secret-9"]);
}

/// A local port nothing listens on.
fn closed_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The one session of a test.
async fn sid_of(host: &AssistantHost) -> String {
    host.list_sessions(false, "").await.unwrap().sessions[0]
        .id
        .clone()
}

#[test]
fn a_refused_key_that_cannot_be_replaced_keeps_the_turn_in_both_histories() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    env.state.lock().unwrap().refuse_refresh = true;
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Error(401, invalid_key()));
        let (run, end) = turn(&host, &env, request(&sid, "Kept question", json!({}))).await;
        assert_eq!(end.data["status"], "error");
        let error = wait_for(&env, &run, "error").await;
        assert_eq!(error.data["code"], "assistant_key_rejected");
        assert_eq!(gateway.requests().len(), 1, "no second attempt");
        assert_eq!(env.state.lock().unwrap().rejected, [KEY]);

        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(detail.messages[0].content, "Kept question");
        assert!(detail.messages.iter().all(|m| m.native_entry_id.is_some()));
        assert_eq!(detail.messages[1].status, "error");
        let native =
            std::fs::read_to_string(session_dir(config.path(), &sid).join("session.jsonl"))
                .unwrap();
        assert_eq!(native.matches("Kept question").count(), 1);
        // The refused request cost nothing.
        assert_ne!(detail.usage["usageCoverage"], "incomplete");
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn a_logout_while_the_first_send_waits_for_its_key_mints_nothing() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let gate = Arc::new(tokio::sync::Notify::new());
        *env.key_gate.lock().unwrap() = Some(gate.clone());
        let send = tokio::spawn({
            let host = host.clone();
            let request = request(&sid, "First message", json!({}));
            async move { host.send(request).await }
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while !env.key_waiting.load(std::sync::atomic::Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the send never asked for a key");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Logout closes admission and revokes keys while the send waits.
        let receipt = host.begin_logout();
        gate.notify_one();
        assert_eq!(send.await.unwrap().unwrap_err(), "assistant_signing_out");
        host.finish_logout(receipt);
        assert!(
            !env.state.lock().unwrap().key_minted,
            "no key outlives the logout"
        );
        let detail = host.get_session(&sid).await.unwrap();
        assert!(detail.messages.is_empty());
        assert!(gateway.requests().is_empty());
    });
}

// ── attachments ──────────────────────────────────────────────────────────

/// A host whose runtime is never started: picking files needs none.
fn picking_host(config: &Path) -> (Gateway, Arc<TestEnv>, Arc<AssistantHost>) {
    let gateway = Gateway::start();
    let runtime = config.join("no-runtime");
    let env = TestEnv::new(config, &runtime, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    (gateway, env, host)
}

/// `name` with `bytes` in a folder the user picks from.
fn picked(config: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let dir = config.join("picked");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// A dialog that must not open.
fn no_dialog() -> Result<Vec<PathBuf>, String> {
    panic!("the dialog opened")
}

#[test]
fn picked_files_are_copied_into_the_conversation() {
    let config = tempfile::tempdir().unwrap();
    let (_gateway, _env, host) = picking_host(config.path());
    runtime().block_on(async {
        let sid = create(&host).await;
        let sources = vec![
            picked(config.path(), "notes.md", b"# Notes"),
            picked(config.path(), "chart.png", b"\x89PNG"),
        ];
        let imported = host
            .pick_attachments(&sid, move || Ok(sources))
            .await
            .unwrap();
        let names: Vec<_> = imported
            .iter()
            .map(|a| (a.name.as_str(), a.mime_type.as_str(), a.status.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("notes.md", "text/markdown", "ready"),
                ("chart.png", "image/png", "ready")
            ]
        );
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(
            detail
                .attachments
                .iter()
                .map(|a| a.id.clone())
                .collect::<Vec<_>>(),
            imported.iter().map(|a| a.id.clone()).collect::<Vec<_>>()
        );
        let copy = session_dir(config.path(), &sid)
            .join("attachments")
            .join(&imported[0].id);
        assert_eq!(std::fs::read(copy).unwrap(), b"# Notes");

        // A cancelled dialog imports nothing.
        assert!(host
            .pick_attachments(&sid, || Ok(vec![]))
            .await
            .unwrap()
            .is_empty());
        assert_eq!(host.get_session(&sid).await.unwrap().attachments.len(), 2);
    });
}

#[test]
fn a_pick_lands_only_for_the_account_and_authority_that_opened_the_dialog() {
    let config = tempfile::tempdir().unwrap();
    let (_gateway, env, host) = picking_host(config.path());
    runtime().block_on(async {
        let sid = create(&host).await;
        let attachments = session_dir(config.path(), &sid).join("attachments");
        let copies = || std::fs::read_dir(&attachments).map_or(0, |dir| dir.count());

        // The region changed while the dialog was open.
        let source = picked(config.path(), "a.txt", b"x");
        let pick = {
            let (env, source) = (env.clone(), source.clone());
            move || {
                env.signed_in("ofox.io");
                Ok(vec![source])
            }
        };
        assert_eq!(
            host.pick_attachments(&sid, pick).await.unwrap_err(),
            "assistant_identity_changed"
        );
        env.signed_in("ofox.ai");

        // A logout or account switch revoked the authority meanwhile.
        let pick = {
            let (host, source) = (host.clone(), source.clone());
            move || {
                host.cancel_all();
                Ok(vec![source])
            }
        };
        assert_eq!(
            host.pick_attachments(&sid, pick).await.unwrap_err(),
            "assistant_identity_changed"
        );
        assert_eq!(copies(), 0);
        assert!(host.get_session(&sid).await.unwrap().attachments.is_empty());

        // Signed out, a trashed or an unknown session: no dialog at all.
        assert_eq!(
            host.pick_attachments(&types::id(), no_dialog)
                .await
                .unwrap_err(),
            "assistant_session_not_found"
        );
        assert_eq!(
            host.pick_attachments("../x", no_dialog).await.unwrap_err(),
            "assistant_invalid_identifier"
        );
        let trash: types::UpdateRequest =
            serde_json::from_value(json!({"sessionId": sid, "deleted": true})).unwrap();
        host.update_session(trash).await.unwrap();
        assert_eq!(
            host.pick_attachments(&sid, no_dialog).await.unwrap_err(),
            "assistant_session_deleted"
        );
        env.set_identity(Err("assistant_sign_in_required".into()));
        assert_eq!(
            host.pick_attachments(&sid, no_dialog).await.unwrap_err(),
            "assistant_sign_in_required"
        );
    });
}

// ── dropped and pasted files ─────────────────────────────────────────────

/// The authority the page shows for `sid`.
fn page_authority(host: &AssistantHost, sid: &str) -> DeleteRequest {
    serde_json::from_value(json!({
        "sessionId": sid, "expectedOwner": owner(),
        "expectedGeneration": host.generation(),
    }))
    .unwrap()
}

fn drop_request(host: &AssistantHost, sid: &str, token: &str) -> ImportDropRequest {
    serde_json::from_value(json!({
        "sessionId": sid, "token": token, "expectedOwner": owner(),
        "expectedGeneration": host.generation(),
    }))
    .unwrap()
}

/// Drop `paths` on the window; the token the renderer is given.
async fn dropped(host: &AssistantHost, paths: Vec<PathBuf>) -> String {
    host.accept_drop(paths).await.unwrap().unwrap().0
}

fn saved_names(detail: &Detail) -> Vec<(&str, &str)> {
    detail
        .attachments
        .iter()
        .map(|a| (a.name.as_str(), a.mime_type.as_str()))
        .collect()
}

#[test]
fn dropped_files_are_imported_once_with_the_picked_files_checks() {
    let config = tempfile::tempdir().unwrap();
    let (_gateway, _env, host) = picking_host(config.path());
    runtime().block_on(async {
        let sid = create(&host).await;
        let paths = vec![
            picked(config.path(), "notes.md", b"# Notes"),
            picked(config.path(), "chart.png", b"\x89PNG"),
        ];
        let (token, count) = host.accept_drop(paths).await.unwrap().unwrap();
        assert_eq!(count, 2);

        let imported = host
            .import_dropped(drop_request(&host, &sid, &token))
            .await
            .unwrap();
        assert_eq!(
            imported
                .iter()
                .map(|a| (a.name.as_str(), a.mime_type.as_str(), a.status.as_str()))
                .collect::<Vec<_>>(),
            [
                ("notes.md", "text/markdown", "ready"),
                ("chart.png", "image/png", "ready")
            ]
        );
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.attachments.len(), 2);
        let copy = session_dir(config.path(), &sid)
            .join("attachments")
            .join(&imported[0].id);
        assert_eq!(std::fs::read(copy).unwrap(), b"# Notes");

        // The token is spent.
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap_err(),
            "assistant_drop_unavailable"
        );
        assert_eq!(host.get_session(&sid).await.unwrap().attachments.len(), 2);
    });
}

#[test]
fn a_dropped_directory_or_unsupported_file_is_refused_like_a_picked_one() {
    let config = tempfile::tempdir().unwrap();
    let (_gateway, _env, host) = picking_host(config.path());
    runtime().block_on(async {
        let sid = create(&host).await;
        let fine = picked(config.path(), "fine.txt", b"x");
        let folder = config.path().join("picked").join("folder.txt");
        std::fs::create_dir(&folder).unwrap();
        for (bad, code) in [
            (folder, "assistant_attachment_unreadable"),
            (
                picked(config.path(), "tool.exe", b"MZ"),
                "assistant_attachment_unsupported",
            ),
            (
                config.path().join("picked").join("missing.txt"),
                "assistant_attachment_unreadable",
            ),
        ] {
            let token = dropped(&host, vec![fine.clone(), bad.clone()]).await;
            assert_eq!(
                host.import_dropped(drop_request(&host, &sid, &token))
                    .await
                    .unwrap_err(),
                code,
                "{bad:?}"
            );
        }
        // More than ten at once is refused as a whole, as in the dialog.
        let eleven: Vec<_> = (0..11)
            .map(|i| picked(config.path(), &format!("{i}.txt"), b"x"))
            .collect();
        let token = dropped(&host, eleven).await;
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap_err(),
            "assistant_attachment_too_many"
        );
        assert!(host.get_session(&sid).await.unwrap().attachments.is_empty());
    });
}

#[test]
fn only_a_token_the_window_event_issued_imports_a_drop() {
    let config = tempfile::tempdir().unwrap();
    let (_gateway, _env, host) = picking_host(config.path());
    runtime().block_on(async {
        let sid = create(&host).await;
        let secret = picked(config.path(), "secret.txt", b"not for the assistant");
        let token = dropped(&host, vec![secret.clone()]).await;
        let random = types::id();
        // A path, or anything else the renderer makes up, is not a token.
        for guess in [
            secret.to_str().unwrap(),
            "/etc/hosts",
            "../../x",
            "",
            random.as_str(),
        ] {
            assert_eq!(
                host.import_dropped(drop_request(&host, &sid, guess))
                    .await
                    .unwrap_err(),
                "assistant_drop_unavailable",
                "{guess:?}"
            );
        }
        // Guesses do not use up the drop that waits.
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap()
                .len(),
            1
        );
    });
}

#[test]
fn only_the_newest_drop_can_be_imported() {
    let config = tempfile::tempdir().unwrap();
    let (_gateway, _env, host) = picking_host(config.path());
    runtime().block_on(async {
        let sid = create(&host).await;
        let old = dropped(&host, vec![picked(config.path(), "old.txt", b"1")]).await;
        let new = dropped(&host, vec![picked(config.path(), "new.txt", b"2")]).await;
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &old))
                .await
                .unwrap_err(),
            "assistant_drop_unavailable"
        );
        let imported = host
            .import_dropped(drop_request(&host, &sid, &new))
            .await
            .unwrap();
        assert_eq!(imported[0].name, "new.txt");
        assert_eq!(host.get_session(&sid).await.unwrap().attachments.len(), 1);

        // Nothing dropped, nothing to hold.
        assert!(host.accept_drop(vec![]).await.unwrap().is_none());
    });
}

#[test]
fn a_drop_belongs_to_the_account_and_authority_it_was_dropped_under() {
    let config = tempfile::tempdir().unwrap();
    let (_gateway, env, host) = picking_host(config.path());
    runtime().block_on(async {
        let sid = create(&host).await;
        let file = picked(config.path(), "a.txt", b"x");

        // A page that shows another authority is refused, and the drop that
        // waits is not used up by it.
        let token = dropped(&host, vec![file.clone()]).await;
        let mut stale = drop_request(&host, &sid, &token);
        stale.expected_generation += 1;
        assert_eq!(
            host.import_dropped(stale).await.unwrap_err(),
            "assistant_identity_changed"
        );
        let mut other = drop_request(&host, &sid, &token);
        other.expected_owner = "cd".repeat(32);
        assert_eq!(
            host.import_dropped(other).await.unwrap_err(),
            "assistant_identity_changed"
        );
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap()
                .len(),
            1
        );

        // A logout or account switch revokes what was dropped before it.
        let token = dropped(&host, vec![file.clone()]).await;
        host.cancel_all();
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap_err(),
            "assistant_drop_unavailable"
        );

        // Another account is never given what the first one dropped, even
        // when it asks with its own authority.
        let token = dropped(&host, vec![file.clone()]).await;
        let other_account = || Identity {
            owner: "cd".repeat(32),
            apex: "ofox.ai".into(),
        };
        env.set_identity(Ok(other_account()));
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap_err(),
            "assistant_identity_changed"
        );
        let theirs_sid = create(&host).await;
        let mut theirs = drop_request(&host, &theirs_sid, &token);
        theirs.expected_owner = "cd".repeat(32);
        assert_eq!(
            host.import_dropped(theirs).await.unwrap_err(),
            "assistant_identity_changed"
        );
        env.signed_in("ofox.ai");
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap_err(),
            "assistant_drop_unavailable",
            "a drop another account asked for is gone"
        );
        assert_eq!(host.get_session(&sid).await.unwrap().attachments.len(), 1);

        // Signed out, nothing can be dropped.
        env.set_identity(Err("assistant_sign_in_required".into()));
        assert_eq!(
            host.accept_drop(vec![file]).await.unwrap_err(),
            "assistant_sign_in_required"
        );
    });
}

#[test]
fn a_drop_is_refused_for_a_trashed_or_unknown_conversation_and_keeps_its_token() {
    let config = tempfile::tempdir().unwrap();
    let (_gateway, _env, host) = picking_host(config.path());
    runtime().block_on(async {
        let sid = create(&host).await;
        let token = dropped(&host, vec![picked(config.path(), "a.txt", b"x")]).await;
        assert_eq!(
            host.import_dropped(drop_request(&host, &types::id(), &token))
                .await
                .unwrap_err(),
            "assistant_session_not_found"
        );
        assert_eq!(
            host.import_dropped(drop_request(&host, "../x", &token))
                .await
                .unwrap_err(),
            "assistant_invalid_identifier"
        );
        let trash: types::UpdateRequest =
            serde_json::from_value(json!({"sessionId": sid, "deleted": true})).unwrap();
        host.update_session(trash).await.unwrap();
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap_err(),
            "assistant_session_deleted"
        );
        let restore: types::UpdateRequest =
            serde_json::from_value(json!({"sessionId": sid, "deleted": false})).unwrap();
        host.update_session(restore).await.unwrap();
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap()
                .len(),
            1
        );
    });
}

#[test]
fn a_drop_request_cannot_carry_a_path() {
    let with_path = json!({
        "sessionId": types::id(), "token": "t", "expectedOwner": owner(),
        "expectedGeneration": 0, "paths": ["/etc/hosts"],
    });
    assert!(serde_json::from_value::<ImportDropRequest>(with_path).is_err());
}

#[test]
fn pasted_files_are_typed_by_their_content_and_never_by_the_renderer() {
    let config = tempfile::tempdir().unwrap();
    let (_gateway, _env, host) = picking_host(config.path());
    runtime().block_on(async {
        let sid = create(&host).await;
        let authority = || page_authority(&host, &sid);
        let png = png();

        let by_content = host
            .import_pasted(authority(), "photo.txt".into(), png.clone())
            .await
            .unwrap();
        assert_eq!(
            (
                by_content[0].name.as_str(),
                by_content[0].mime_type.as_str()
            ),
            ("photo.png", "image/png")
        );
        let unnamed = host
            .import_pasted(authority(), String::new(), png.clone())
            .await
            .unwrap();
        assert!(unnamed[0].name.starts_with("pasted-image-") && unnamed[0].name.ends_with(".png"));
        let text = host
            .import_pasted(authority(), "../../notes.md".into(), b"# Notes".to_vec())
            .await
            .unwrap();
        assert_eq!(
            (text[0].name.as_str(), text[0].mime_type.as_str()),
            ("notes.md", "text/markdown")
        );
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.attachments.len(), 3);
        // The copy is the pasted bytes, in the conversation's own folder.
        let copy = session_dir(config.path(), &sid)
            .join("attachments")
            .join(&by_content[0].id);
        assert_eq!(std::fs::read(copy).unwrap(), png);

        // A name that claims what the bytes are not is refused.
        for (name, code) in [
            ("fake.png", "assistant_attachment_unreadable"),
            ("fake.pdf", "assistant_attachment_unreadable"),
            ("tool.exe", "assistant_attachment_unsupported"),
            ("", "assistant_attachment_unsupported"),
        ] {
            assert_eq!(
                host.import_pasted(authority(), name.into(), b"plain text".to_vec())
                    .await
                    .unwrap_err(),
                code,
                "{name:?}"
            );
        }
        assert_eq!(
            host.get_session(&sid).await.unwrap().attachments.len(),
            3,
            "{:?}",
            saved_names(&detail)
        );
    });
}

#[test]
fn a_pasted_file_obeys_the_size_and_count_limits_and_the_authority() {
    let config = tempfile::tempdir().unwrap();
    let (_gateway, env, host) = picking_host(config.path());
    runtime().block_on(async {
        let sid = create(&host).await;
        let authority = || page_authority(&host, &sid);
        let big = vec![b'x'; 25 * 1024 * 1024 + 1];
        assert_eq!(
            host.import_pasted(authority(), "big.txt".into(), big)
                .await
                .unwrap_err(),
            "assistant_attachment_too_large"
        );

        // Another authority than the page's.
        let mut stale = authority();
        stale.expected_generation += 1;
        assert_eq!(
            host.import_pasted(stale, "a.txt".into(), b"x".to_vec())
                .await
                .unwrap_err(),
            "assistant_identity_changed"
        );
        let mut other = authority();
        other.expected_owner = "cd".repeat(32);
        assert_eq!(
            host.import_pasted(other, "a.txt".into(), b"x".to_vec())
                .await
                .unwrap_err(),
            "assistant_identity_changed"
        );
        // The page of an account that is no longer the signed-in one.
        let page = authority();
        env.set_identity(Ok(Identity {
            owner: "cd".repeat(32),
            apex: "ofox.ai".into(),
        }));
        assert_eq!(
            host.import_pasted(page, "a.txt".into(), b"x".to_vec())
                .await
                .unwrap_err(),
            "assistant_identity_changed"
        );
        env.signed_in("ofox.ai");

        // Unknown, malformed and trashed conversations.
        assert_eq!(
            host.import_pasted(
                page_authority(&host, &types::id()),
                "a.txt".into(),
                b"x".to_vec()
            )
            .await
            .unwrap_err(),
            "assistant_session_not_found"
        );
        assert_eq!(
            host.import_pasted(page_authority(&host, "../x"), "a.txt".into(), b"x".to_vec())
                .await
                .unwrap_err(),
            "assistant_invalid_identifier"
        );
        let trash: types::UpdateRequest =
            serde_json::from_value(json!({"sessionId": sid, "deleted": true})).unwrap();
        host.update_session(trash).await.unwrap();
        assert_eq!(
            host.import_pasted(authority(), "a.txt".into(), b"x".to_vec())
                .await
                .unwrap_err(),
            "assistant_session_deleted"
        );
        assert!(host.get_session(&sid).await.unwrap().attachments.is_empty());
    });
}

#[test]
fn dropped_and_pasted_files_are_refused_while_the_conversation_replies() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let open = gate();
        gateway.reply(Reply::Wait(
            open.clone(),
            Box::new(Reply::Stream("Done".into())),
        ));
        let run = host.send(request(&sid, "Hello", json!({}))).await.unwrap();

        let token = dropped(&host, vec![picked(config.path(), "a.txt", b"x")]).await;
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap_err(),
            "assistant_session_busy"
        );
        assert_eq!(
            host.import_pasted(page_authority(&host, &sid), "b.txt".into(), b"y".to_vec())
                .await
                .unwrap_err(),
            "assistant_session_busy"
        );

        open.store(true, Ordering::SeqCst);
        let end = wait_for(&env, &run.run_id, "run_end").await;
        assert_eq!(end.data["status"], "completed");
        assert!(host.get_session(&sid).await.unwrap().attachments.is_empty());
        // The refused drop still waits: the reply is over.
        assert_eq!(
            host.import_dropped(drop_request(&host, &sid, &token))
                .await
                .unwrap()
                .len(),
            1
        );
    });
}

#[test]
fn a_pasted_pdf_with_the_wrong_name_and_a_dropped_text_file_are_read_by_the_runtime() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let pasted = host
            .import_pasted(
                page_authority(&host, &sid),
                "invoice.txt".into(),
                pdf("Invoice total 1234"),
            )
            .await
            .unwrap();
        assert_eq!(
            (pasted[0].name.as_str(), pasted[0].mime_type.as_str()),
            ("invoice.pdf", "application/pdf")
        );
        let token = dropped(
            &host,
            vec![picked(config.path(), "notes.txt", b"Quarterly: 42 apples")],
        )
        .await;
        let drop = host
            .import_dropped(drop_request(&host, &sid, &token))
            .await
            .unwrap();

        gateway.reply(Reply::Stream("1234 and 42".into()));
        let (run, end) = turn(
            &host,
            &env,
            request(
                &sid,
                "Read both",
                json!({"attachmentIds": [pasted[0].id, drop[0].id]}),
            ),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        let sent = gateway.requests()[0].text();
        assert!(sent.contains("Invoice total 1234"), "{sent}");
        assert!(sent.contains("Quarterly: 42 apples"), "{sent}");
        let outcome = outcomes(&env, &run);
        assert_eq!(
            outcome
                .iter()
                .map(|o| (
                    o["status"].as_str().unwrap(),
                    o["details"]["format"].clone()
                ))
                .collect::<Vec<_>>(),
            [("ready", json!("pdf")), ("ready", json!("text"))]
        );
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

/// A 1x1 PNG.
fn png() -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=")
        .unwrap()
}

/// A real one-page PDF showing `text` in Helvetica, with a valid xref (as
/// the runtime's own `pdfPages` fixture builds it).
fn pdf(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 20 150 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = vec![];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out += &format!("{} 0 obj\n{object}\nendobj\n", index + 1);
    }
    let xref = out.len();
    out += &format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        out += &format!("{offset:010} 00000 n \n");
    }
    out += &format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    );
    out.into_bytes()
}

/// Pick one file into the conversation; returns its attachment id.
async fn attach(
    host: &Arc<AssistantHost>,
    config: &Path,
    sid: &str,
    name: &str,
    bytes: &[u8],
) -> String {
    let source = picked(config, name, bytes);
    let imported = host
        .pick_attachments(sid, move || Ok(vec![source]))
        .await
        .unwrap();
    imported[0].id.clone()
}

/// The `attachment` events of a run.
fn outcomes(env: &TestEnv, run: &str) -> Vec<Value> {
    env.events(run)
        .into_iter()
        .filter(|e| e.r#type == "attachment")
        .map(|e| e.data)
        .collect()
}

fn user_message<'a>(detail: &'a Detail, run: &str) -> &'a Message {
    detail
        .messages
        .iter()
        .find(|m| m.role == "user" && m.run_id.as_deref() == Some(run))
        .unwrap()
}

#[test]
fn editing_without_attachments_removes_their_text_from_the_model_and_keeps_the_original_and_draft()
{
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    for apex in ["ofox.ai", "ofox.io"] {
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &runtime_dir, &[(apex, &gateway)]);
        env.signed_in(apex);
        let host = AssistantHost::new(env.clone());
        runtime().block_on(async {
            let sid = create(&host).await;
            let file = attach(&host, config.path(), &sid, "notes.txt", b"Attachment-only sentinel 987654").await;
            gateway.reply(Reply::Stream("Read the file".into()));
            let (first_run, _) = turn(&host, &env, request(&sid, "Read it", json!({"attachmentIds": [file]}))).await;
            let first = host.get_session(&sid).await.unwrap();
            let original = user_message(&first, &first_run);
            host.update_session(serde_json::from_value(json!({"sessionId": sid, "draft": "Unsent draft", "draftAttachmentIds": [file]})).unwrap()).await.unwrap();
            gateway.reply(Reply::Stream("Text-only answer".into()));
            let (edit_run, end) = turn(&host, &env, request(&sid, "Text only now", json!({
                "editMessageId": original.id, "attachmentIds": [], "replaceAttachments": true,
            }))).await;
            assert_eq!(end.data["status"], "completed");
            let sent = gateway.requests().last().unwrap().text();
            assert!(sent.contains("Text only now"));
            assert!(!sent.contains("Attachment-only sentinel"), "{sent}");
            let edited = host.get_session(&sid).await.unwrap();
            assert!(user_message(&edited, &edit_run).attachments.is_empty());
            let old = user_message(&edited, &first_run);
            assert!(old.superseded);
            assert_eq!(old.attachments[0].id, file);
            assert_eq!(edited.session.draft, "Unsent draft");
            assert_eq!(edited.session.draft_attachment_ids, [file]);
        });
        assert_no_secret_on_disk(config.path(), &[KEY]);
    }
}

#[test]
fn chinese_office_files_reach_the_model_across_region_switch_and_restart() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    for start_apex in ["ofox.ai", "ofox.io"] {
        let config = tempfile::tempdir().unwrap();
        let first = Gateway::start();
        let second = Gateway::start();
        let next_apex = if start_apex == "ofox.ai" {
            "ofox.io"
        } else {
            "ofox.ai"
        };
        let env = TestEnv::new(
            config.path(),
            &runtime_dir,
            &[(start_apex, &first), (next_apex, &second)],
        );
        env.signed_in(start_apex);
        runtime().block_on(async {
            let host = AssistantHost::new(env.clone());
            let sid = create(&host).await;
            let mut ids = vec![];
            for (name, bytes) in [
                (
                    "中文.docx",
                    include_bytes!("../../assistant-runtime/test/fixtures/office/chinese.docx")
                        .as_slice(),
                ),
                (
                    "中文.xlsx",
                    include_bytes!("../../assistant-runtime/test/fixtures/office/chinese.xlsx")
                        .as_slice(),
                ),
                (
                    "中文.pptx",
                    include_bytes!("../../assistant-runtime/test/fixtures/office/chinese.pptx")
                        .as_slice(),
                ),
            ] {
                ids.push(attach(&host, config.path(), &sid, name, bytes).await);
            }
            first.reply(Reply::Stream("Read Office text".into()));
            let (_, end) = turn(
                &host,
                &env,
                request(&sid, "总结三个附件", json!({"attachmentIds": ids})),
            )
            .await;
            assert_eq!(end.data["status"], "completed", "{end:?}");
            let check = |text: String| {
                for marker in [
                    "星河项目季度报告",
                    "中文收入",
                    "12345.67",
                    "中文第一页",
                    "中文第二页",
                    "讲者备注",
                    "深圳",
                ] {
                    assert!(text.contains(marker), "missing {marker}: {text}");
                }
                assert!(text.contains("Embedded images"));
            };
            check(first.requests()[0].text());
            assert!(second.requests().is_empty());
            drop(host);
            env.signed_in(next_apex);
            let restarted = AssistantHost::new(env.clone());
            let detail = restarted.get_session(&sid).await.unwrap();
            assert_eq!(detail.attachments.len(), 3);
            for file in &detail.attachments {
                assert_eq!(file.status, "partial");
                assert_eq!(file.details.as_ref().unwrap()["textOnly"], true);
            }
            second.reply(Reply::Stream("Continue with the files".into()));
            let (_, end) = turn(&restarted, &env, request(&sid, "继续总结", json!({}))).await;
            assert_eq!(end.data["status"], "completed");
            check(second.requests()[0].text());
            assert_eq!(first.requests().len(), 1);
        });
        assert_no_secret_on_disk(config.path(), &[KEY]);
    }
}

#[test]
fn a_text_attachment_reaches_the_model_and_its_outcome_is_kept() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let file = attach(
            &host,
            config.path(),
            &sid,
            "notes.txt",
            b"Quarterly numbers: 42 apples",
        )
        .await;
        gateway.reply(Reply::Stream("You have 42 apples".into()));
        let (run, end) = turn(
            &host,
            &env,
            request(&sid, "Summarize the file", json!({"attachmentIds": [file]})),
        )
        .await;
        assert_eq!(end.data["status"], "completed");

        let sent = gateway.requests()[0].text();
        assert!(sent.contains("Summarize the file"), "{sent}");
        assert!(
            sent.contains("[User attachment: \\\"notes.txt\\\""),
            "{sent}"
        );
        assert!(sent.contains("Quarterly numbers: 42 apples"), "{sent}");

        let events = env.events(&run);
        let outcome = outcomes(&env, &run);
        assert_eq!(outcome.len(), 1, "{outcome:?}");
        assert_eq!(outcome[0]["id"], file);
        assert_eq!(outcome[0]["status"], "ready");
        assert!(outcome[0]["code"].is_null());
        assert_eq!(outcome[0]["details"]["format"], "text");
        let read = events
            .iter()
            .position(|e| e.r#type == "attachment")
            .unwrap();
        let streamed = events
            .iter()
            .position(|e| e.r#type == "text_delta")
            .unwrap();
        assert!(read < streamed, "the outcome comes before the answer");

        // A reload shows the outcome on the conversation and on the turn.
        let reloaded = AssistantHost::new(env.clone());
        let detail = reloaded.get_session(&sid).await.unwrap();
        let user = user_message(&detail, &run);
        assert_eq!(user.content, "Summarize the file", "the user's own words");
        assert_eq!(user.attachments.len(), 1);
        assert_eq!(user.attachments[0].id, file);
        assert_eq!(user.attachments[0].status, "ready");
        assert_eq!(
            user.attachments[0].details.as_ref().unwrap()["format"],
            "text"
        );
        assert_eq!(detail.attachments[0].status, "ready");
        assert_eq!(
            detail.attachments[0].details.as_ref().unwrap()["characters"],
            28
        );
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

#[test]
fn a_pdf_attachment_is_read_and_its_outcome_persists() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let file = attach(
            &host,
            config.path(),
            &sid,
            "invoice.pdf",
            &pdf("Invoice total 1234"),
        )
        .await;
        gateway.reply(Reply::Stream("The total is 1234".into()));
        let (run, end) = turn(
            &host,
            &env,
            request(&sid, "What is the total?", json!({"attachmentIds": [file]})),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        assert!(gateway.requests()[0].text().contains("Invoice total 1234"));

        let outcome = outcomes(&env, &run);
        assert_eq!(outcome.len(), 1, "{outcome:?}");
        assert_eq!(outcome[0]["status"], "ready");
        assert_eq!(outcome[0]["details"]["format"], "pdf");
        assert_eq!(outcome[0]["details"]["pageCount"], 1);

        let detail = AssistantHost::new(env.clone())
            .get_session(&sid)
            .await
            .unwrap();
        let saved = &detail.attachments[0];
        assert_eq!(
            (saved.name.as_str(), saved.mime_type.as_str()),
            ("invoice.pdf", "application/pdf")
        );
        assert_eq!(saved.details.as_ref().unwrap()["pageCount"], 1);
        assert_eq!(saved.details.as_ref().unwrap()["textCoverage"], "complete");
        let user = user_message(&detail, &run);
        assert_eq!(
            user.attachments[0].details.as_ref().unwrap()["format"],
            "pdf"
        );
        assert!(session_dir(config.path(), &sid)
            .join("attachments")
            .join(&file)
            .is_file());
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn an_image_for_a_text_only_model_is_blocked_before_any_request_and_costs_nothing() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Stream("A priced answer".into()));
        turn(&host, &env, request(&sid, "First", json!({}))).await;
        let priced = host.get_session(&sid).await.unwrap().usage;
        assert!(priced["estimatedCostUsd"].is_string(), "{priced}");

        let image = attach(&host, config.path(), &sid, "photo.png", &png()).await;
        let (run, end) = turn(
            &host,
            &env,
            request(
                &sid,
                "What is in the picture?",
                json!({"attachmentIds": [image]}),
            ),
        )
        .await;
        assert_eq!(end.data["status"], "error");
        let error = wait_for(&env, &run, "error").await;
        assert_eq!(error.data["code"], "assistant_vision_model_required");
        assert_eq!(gateway.requests().len(), 1, "no model request was made");

        let outcome = outcomes(&env, &run);
        assert_eq!(outcome.len(), 1, "{outcome:?}");
        assert_eq!(outcome[0]["status"], "model_unsupported");
        assert_eq!(outcome[0]["code"], "assistant_vision_model_required");
        assert_eq!(outcome[0]["details"]["mimeType"], "image/png");

        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.attachments[0].status, "model_unsupported");
        assert_eq!(
            detail.attachments[0].reason.as_deref(),
            Some("assistant_vision_model_required")
        );
        let user = user_message(&detail, &run);
        assert_eq!(user.content, "What is in the picture?");
        assert_eq!(user.attachments[0].status, "model_unsupported");
        assert_eq!(detail.messages.last().unwrap().status, "error");
        // A blocked turn was never sent: the cost stays known.
        assert_eq!(detail.usage["estimatedCostUsd"], priced["estimatedCostUsd"]);
        assert_ne!(detail.usage["usageCoverage"], "incomplete");

        // Retrying it reads its picture again, and is blocked again for free.
        let blocked = detail.messages.last().unwrap().id.clone();
        let (retry, end) = turn(
            &host,
            &env,
            request(&sid, "", json!({"regenerateMessageId": blocked})),
        )
        .await;
        assert_eq!(end.data["status"], "error");
        assert_eq!(
            wait_for(&env, &retry, "error").await.data["code"],
            "assistant_vision_model_required"
        );
        assert_eq!(outcomes(&env, &retry).len(), 1);
        assert_eq!(gateway.requests().len(), 1);
        let detail = host.get_session(&sid).await.unwrap();
        let user = user_message(&detail, &retry);
        assert_eq!(user.content, "What is in the picture?");
        assert_eq!(user.attachments[0].status, "model_unsupported");
        assert_eq!(detail.usage["estimatedCostUsd"], priced["estimatedCostUsd"]);

        gateway.reply(Reply::Stream("Still here".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Without the picture", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

#[test]
fn regenerate_and_edit_send_the_turns_files_again() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    const MARKER: &str = "Marker-7Q2 inventory: 5 pears";
    runtime().block_on(async {
        let sid = create(&host).await;
        let file = attach(&host, config.path(), &sid, "stock.txt", MARKER.as_bytes()).await;
        gateway.reply(Reply::Stream("Answer one".into()));
        turn(
            &host,
            &env,
            request(&sid, "Count the fruit", json!({"attachmentIds": [file]})),
        )
        .await;
        let answer = host.get_session(&sid).await.unwrap().messages[1].id.clone();

        gateway.reply(Reply::Stream("Answer two".into()));
        let (regenerated, end) = turn(
            &host,
            &env,
            request(&sid, "", json!({"regenerateMessageId": answer})),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        let resent = gateway.requests()[1].text();
        assert_eq!(resent.matches(MARKER).count(), 1, "{resent}");
        assert!(resent.contains("Count the fruit") && !resent.contains("Answer one"));
        assert_eq!(
            outcomes(&env, &regenerated).len(),
            1,
            "the file was read again"
        );
        let detail = host.get_session(&sid).await.unwrap();
        let user = user_message(&detail, &regenerated);
        assert_eq!(user.content, "Count the fruit");
        assert_eq!(user.attachments[0].id, file);

        // An edit without a selection of its own keeps the turn's files.
        gateway.reply(Reply::Stream("Answer three".into()));
        let (edited, end) = turn(
            &host,
            &env,
            request(
                &sid,
                "Count only the pears",
                json!({"editMessageId": user.id}),
            ),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        let sent = gateway.requests()[2].text();
        assert_eq!(sent.matches(MARKER).count(), 1, "{sent}");
        assert!(sent.contains("Count only the pears") && !sent.contains("Count the fruit"));
        assert_eq!(outcomes(&env, &edited).len(), 1);
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(user_message(&detail, &edited).attachments[0].id, file);
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

// ── web search ───────────────────────────────────────────────────────────

const SEARCH_MODEL: &str = "openai/gpt-5-nano";

/// The search model's catalog row, at the live prices.
fn search_model() -> Model {
    serde_json::from_value(json!({
        "id": SEARCH_MODEL,
        "name": "GPT-5 Nano",
        "contextWindow": 128000,
        "maxOutputTokens": 16384,
        "supportsImages": false,
        "inputModalities": ["text"],
        "supportedEndpoints": ["/v1/chat/completions", "/v1/responses"],
        "inputPricePerMillion": "0.05",
        "outputPricePerMillion": "0.4",
        "pricing": {"prompt": "0.00000005", "completion": "0.0000004", "web_search": "0.014"},
        "cost": {"input": 0.05, "output": 0.4, "cacheRead": null, "cacheWrite": null},
        "pricingKnown": true,
    }))
    .unwrap()
}

/// A host whose catalog prices web search, and a conversation with search on.
async fn searching(host: &Arc<AssistantHost>, env: &TestEnv) -> String {
    env.catalog.lock().unwrap().push(search_model());
    let sid = create(host).await;
    let update = serde_json::from_value(json!({"sessionId": sid, "webSearch": true})).unwrap();
    assert!(host.update_session(update).await.unwrap().web_search);
    sid
}

/// One Responses API answer with one billed search: a cited page, then the
/// search's sources (one of them plain http).
fn search_answer(sources: &[&str]) -> Value {
    json!({
        "id": "resp_mock", "object": "response", "status": "completed", "model": SEARCH_MODEL,
        "output": [
            {"type": "web_search_call", "id": "ws_mock", "status": "completed",
             "action": {"type": "search", "query": "pi sdk",
                        "sources": sources.iter().map(|url| json!({"type": "url", "url": url})).collect::<Vec<_>>()}},
            {"type": "message", "id": "msg_mock", "role": "assistant", "status": "completed",
             "content": [{"type": "output_text", "text": "Pi builds agents [Pi SDK](https://pi.dev/sdk).",
                          "annotations": [{"type": "url_citation", "url": "https://pi.dev/sdk#intro",
                                           "title": "Pi SDK", "start_index": 17, "end_index": 46}]}]}
        ],
        "usage": {"input_tokens": 1000, "output_tokens": 500, "total_tokens": 1500},
    })
}

fn tool_names(request: &Request) -> Vec<String> {
    request.body["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
        .collect()
}

fn of_type(env: &TestEnv, run: &str, kind: &str) -> Vec<Value> {
    env.events(run)
        .into_iter()
        .filter(|e| e.r#type == kind)
        .map(|e| e.data)
        .collect()
}

/// The text of the tool results the model was sent.
fn tool_results(request: &Request) -> Vec<String> {
    request.body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["role"] == "tool")
        .map(|m| text_of(&m["content"]))
        .collect()
}

fn decimal(value: &Value) -> f64 {
    value.as_str().unwrap().parse().unwrap()
}

#[test]
fn search_is_offered_only_when_priced_and_only_runs_with_it_get_the_tools() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        // The catalog has no search model: no search, even with the switch on.
        let boot = host.bootstrap().await.unwrap();
        assert_eq!(
            boot["search"],
            json!({"available": false, "unitPriceUsd": null, "model": SEARCH_MODEL, "provider": "ofox"})
        );
        let sid = create(&host).await;
        let on = serde_json::from_value(json!({"sessionId": sid, "webSearch": true})).unwrap();
        host.update_session(on).await.unwrap();
        gateway.reply(Reply::Stream("No search here".into()));
        let (run, end) = turn(&host, &env, request(&sid, "What is new?", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        assert_eq!(
            tool_names(&gateway.requests()[0]),
            ["use_skill", "find_skills", "install_skill", "save_skill"]
        );
        // The run says it could not search, although the switch is on.
        assert_eq!(of_type(&env, &run, "run_start")[0]["webSearch"], false);

        // Priced: offered, and a conversation with search off still gets no web tools.
        env.catalog.lock().unwrap().push(search_model());
        let boot = AssistantHost::new(env.clone()).bootstrap().await.unwrap();
        assert_eq!(
            boot["search"],
            json!({"available": true, "unitPriceUsd": "0.014", "model": SEARCH_MODEL, "provider": "ofox"})
        );
        let host = AssistantHost::new(env.clone());
        let request_off: types::CreateRequest =
            serde_json::from_value(json!({"webSearch": false})).unwrap();
        let off = host.create_session(request_off).await.unwrap();
        assert!(!off.web_search);
        gateway.reply(Reply::Stream("Still no search".into()));
        let (run, _) = turn(&host, &env, request(&off.id, "What is new?", json!({}))).await;
        assert_eq!(
            tool_names(&gateway.requests()[1]),
            ["use_skill", "find_skills", "install_skill", "save_skill"]
        );
        assert_eq!(of_type(&env, &run, "run_start")[0]["webSearch"], false);

        // The conversation with search on now gets both tools.
        gateway.reply(Reply::Stream("Could search".into()));
        let (run, _) = turn(&host, &env, request(&sid, "And now?", json!({}))).await;
        assert_eq!(
            tool_names(&gateway.requests()[2]),
            [
                "use_skill",
                "find_skills",
                "install_skill",
                "save_skill",
                "web_search",
                "web_fetch"
            ]
        );
        assert_eq!(of_type(&env, &run, "run_start")[0]["webSearch"], true);
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.usage["searchCount"], 0);
        assert_eq!(detail.usage["searchCostUsd"], "0");
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn exa_search_is_offered_without_a_price_and_the_choice_is_kept_per_account() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let provider = |value: &str, generation: u64| -> types::SetSearchProviderRequest {
            serde_json::from_value(json!({
                "provider": value, "expectedOwner": owner(), "expectedGeneration": generation,
            }))
            .unwrap()
        };
        host.set_preferences(
            serde_json::from_value(json!({
                "instructions": "Answer briefly.", "autoTitle": false, "notifyOnFinish": true,
                "expectedOwner": owner(), "expectedGeneration": host.generation(),
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        // A stale page cannot change it.
        assert_eq!(
            host.set_search_provider(provider("exa", host.generation() + 1))
                .await
                .unwrap_err(),
            "assistant_identity_changed"
        );
        assert_eq!(host.bootstrap().await.unwrap()["search"]["provider"], "ofox");
        assert_eq!(
            host.set_search_provider(provider("exa", host.generation()))
                .await
                .unwrap(),
            types::SearchProvider::Exa
        );
        // Kept beside the settings, which stay as they were.
        let prefs_file = config.path().join("assistant").join(owner()).join("prefs.json");
        let prefs: Value = serde_json::from_slice(&std::fs::read(&prefs_file).unwrap()).unwrap();
        assert_eq!(prefs["searchProvider"], "exa");
        assert_eq!(prefs["instructions"], "Answer briefly.");
        assert_eq!(host.get_preferences().await.unwrap().instructions, "Answer briefly.");
        let boot = AssistantHost::new(env.clone()).bootstrap().await.unwrap();
        // The catalog still has no search model: Ofox search is not offered.
        assert_eq!(
            boot["search"],
            json!({"available": false, "unitPriceUsd": null, "model": SEARCH_MODEL, "provider": "exa"})
        );

        // Exa needs no price: a conversation with search on gets both tools,
        // and the model is told the searches are free.
        let sid = create(&host).await;
        let on = serde_json::from_value(json!({"sessionId": sid, "webSearch": true})).unwrap();
        host.update_session(on).await.unwrap();
        gateway.reply(Reply::Stream("No search needed".into()));
        let (run, end) = turn(&host, &env, request(&sid, "What is new?", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let requests = gateway.requests();
        assert_eq!(
            tool_names(&requests[0]),
            [
                "use_skill",
                "find_skills",
                "install_skill",
                "save_skill",
                "web_search",
                "web_fetch"
            ]
        );
        let prompt = requests[0].body["messages"][0]["content"].as_str().unwrap();
        assert!(prompt.contains("web_search searches the web for free through Exa"));
        assert!(!prompt.contains("paid search"));
        let start = &of_type(&env, &run, "run_start")[0];
        assert_eq!(
            (&start["webSearch"], &start["searchProvider"]),
            (&json!(true), &json!("exa"))
        );
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.usage["searchCostUsd"], "0");

        // A value this version does not know is the default, not damage.
        let mut prefs = prefs;
        prefs["searchProvider"] = json!("bing");
        std::fs::write(&prefs_file, serde_json::to_vec(&prefs).unwrap()).unwrap();
        let host = AssistantHost::new(env.clone());
        assert_eq!(host.bootstrap().await.unwrap()["search"]["provider"], "ofox");
        assert_eq!(host.get_preferences().await.unwrap().instructions, "Answer briefly.");
        gateway.reply(Reply::Stream("Without search".into()));
        let (run, _) = turn(&host, &env, request(&sid, "And now?", json!({}))).await;
        assert_eq!(of_type(&env, &run, "run_start")[0]["webSearch"], false);
        assert!(of_type(&env, &run, "run_start")[0].get("searchProvider").is_none());
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn a_search_keeps_its_sources_on_the_reply_and_adds_its_cost() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = searching(&host, &env).await;
        gateway.reply(Reply::ToolCalls(vec![(
            "web_search",
            json!({"query": "pi sdk"}),
        )]));
        gateway.reply(Reply::Json(
            200,
            search_answer(&["https://pi.dev/docs", "http://plain.example.com/"]),
        ));
        gateway.reply(Reply::Stream(
            "Pi builds agents [Pi SDK](https://pi.dev/sdk).".into(),
        ));
        let (run, end) = turn(&host, &env, request(&sid, "What is the Pi SDK?", json!({}))).await;
        assert_eq!(end.data["status"], "completed");

        // The model, the search server, the model again.
        let requests = gateway.requests();
        let paths: Vec<_> = requests.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/v1/chat/completions",
                "/v1/responses",
                "/v1/chat/completions"
            ]
        );
        assert!(requests
            .iter()
            .all(|r| r.authorization == format!("Bearer {KEY}")));
        assert_eq!(requests[1].body["model"], SEARCH_MODEL);
        assert!(requests[1].body["input"]
            .as_str()
            .unwrap()
            .ends_with("pi sdk"));
        assert!(tool_results(&requests[2])[0].contains("Pi builds agents"));

        // Progress, then the sources before the search's end.
        let message_id = env.events(&run)[0].data["messageId"].clone();
        let kinds: Vec<String> = env
            .events(&run)
            .into_iter()
            .map(|e| e.r#type)
            .filter(|kind| ["tool_start", "sources", "tool_end"].contains(&kind.as_str()))
            .collect();
        assert_eq!(kinds, ["tool_start", "sources", "tool_end"]);
        assert_eq!(
            of_type(&env, &run, "tool_start")[0],
            json!({"messageId": message_id, "name": "web_search",
                   "toolCallId": of_type(&env, &run, "tool_start")[0]["toolCallId"],
                   "args": {"query": "pi sdk"}})
        );
        let tool_end = &of_type(&env, &run, "tool_end")[0];
        assert_eq!(
            (&tool_end["success"], &tool_end["code"]),
            (&json!(true), &Value::Null)
        );
        let published = &of_type(&env, &run, "sources")[0];
        assert_eq!(published["messageId"], message_id);
        let pages: Vec<&str> = published["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["url"].as_str().unwrap())
            .collect();
        assert_eq!(pages, ["https://pi.dev/sdk", "https://pi.dev/docs"]);

        // A reload shows them on the reply, and the search in the cost.
        let detail = AssistantHost::new(env.clone())
            .get_session(&sid)
            .await
            .unwrap();
        let reply = detail
            .messages
            .iter()
            .find(|m| m.id == message_id.as_str().unwrap())
            .unwrap();
        assert_eq!(
            reply.content,
            "Pi builds agents [Pi SDK](https://pi.dev/sdk)."
        );
        // The step that only called the tool is not a message of its own.
        assert_eq!(
            active(&detail),
            [
                ("user".to_string(), "What is the Pi SDK?".to_string()),
                ("assistant".to_string(), reply.content.clone()),
            ]
        );
        assert_eq!(json!(reply.sources), published["sources"]);
        assert_eq!(reply.sources[0]["title"], "Pi SDK");
        assert_eq!(reply.sources[0]["excerpt"], "Pi builds agents");
        assert!(reply.sources[0]["id"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{run}-")));
        assert_eq!(detail.usage["searchCount"], 1);
        // 0.014 + 1000 × 0.00000005 + 500 × 0.0000004
        assert_eq!(detail.usage["searchCostUsd"], "0.01425");
        assert_eq!(detail.usage["modelRequests"], 2);
        // Two model requests of 10 in, 5 out at 2 and 10 per million.
        let models = decimal(&detail.usage["estimatedCostUsd"]) - 0.01425;
        assert!((models - 0.00014).abs() < 1e-12, "{}", detail.usage);
        assert_ne!(detail.usage["usageCoverage"], "incomplete");
        let native =
            std::fs::read_to_string(session_dir(config.path(), &sid).join("session.jsonl"))
                .unwrap();
        assert!(native.contains("\"searchCalls\":1"));
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

#[test]
fn a_search_with_an_unknown_outcome_ends_the_run_and_leaves_the_cost_unknown() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = searching(&host, &env).await;
        gateway.reply(Reply::Stream("A priced answer".into()));
        turn(&host, &env, request(&sid, "First", json!({}))).await;
        assert!(host.get_session(&sid).await.unwrap().usage["estimatedCostUsd"].is_string());

        gateway.reply(Reply::ToolCalls(vec![(
            "web_search",
            json!({"query": "breaking news"}),
        )]));
        // The gateway failed after the search may have run.
        gateway.reply(Reply::Error(
            502,
            json!({"error": {"message": "upstream failed"}}),
        ));
        let (run, end) = turn(&host, &env, request(&sid, "Any news?", json!({}))).await;
        assert_eq!(end.data["status"], "error");
        let error = wait_for(&env, &run, "error").await;
        assert_eq!(error.data["code"], "assistant_search_outcome_unknown");
        assert_eq!(gateway.requests().len(), 3, "nothing after the search");
        let tool_end = &of_type(&env, &run, "tool_end")[0];
        assert_eq!(tool_end["success"], false);
        assert_eq!(tool_end["code"], "search_outcome_unknown");
        assert!(of_type(&env, &run, "sources").is_empty());

        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.usage["usageCoverage"], "incomplete");
        assert!(detail.usage["estimatedCostUsd"].is_null());
        assert!(detail.usage["searchCostUsd"].is_null());

        // A later answer cannot make it known again.
        gateway.reply(Reply::Stream("Later".into()));
        turn(&host, &env, request(&sid, "Later", json!({}))).await;
        let detail = host.get_session(&sid).await.unwrap();
        assert!(detail.usage["estimatedCostUsd"].is_null());
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

#[test]
fn a_run_may_read_the_pages_of_the_conversation_and_no_others() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    // Pages on port 8443 pass the host's checks but the search server
    // refuses them itself, so no page is ever requested; what the model reads
    // tells which side refused.
    const LINKED: &str = "https://linked.example.com:8443/doc";
    const SOURCE: &str = "https://source.example.com:8443/x";
    const OTHER: &str = "https://other.example.com:8443/";
    const SERVER_REFUSED: &str = "Web URL port must be 443";
    const NOT_ALLOWED: &str = "Only pages the user linked";
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = searching(&host, &env).await;
        // The first turn links a page and searches; its source is kept.
        gateway.reply(Reply::ToolCalls(vec![(
            "web_search",
            json!({"query": "x"}),
        )]));
        gateway.reply(Reply::Json(200, search_answer(&[SOURCE])));
        gateway.reply(Reply::Stream("Found it".into()));
        let prompt = format!("Read [this]({LINKED}), please.");
        let (_, end) = turn(&host, &env, request(&sid, &prompt, json!({}))).await;
        assert_eq!(end.data["status"], "completed");

        // A later turn may read the linked page and the source, not others.
        gateway.reply(Reply::ToolCalls(vec![
            ("web_fetch", json!({"url": LINKED})),
            ("web_fetch", json!({"url": SOURCE})),
            ("web_fetch", json!({"url": OTHER})),
        ]));
        gateway.reply(Reply::Stream("Read them".into()));
        let (run, end) = turn(&host, &env, request(&sid, "Read both again", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let results = tool_results(gateway.requests().last().unwrap());
        let last_three = &results[results.len() - 3..];
        assert!(last_three[0].contains(SERVER_REFUSED), "{last_three:?}");
        assert!(last_three[1].contains(SERVER_REFUSED), "{last_three:?}");
        assert!(last_three[2].contains(NOT_ALLOWED), "{last_three:?}");
        let ends = of_type(&env, &run, "tool_end");
        assert!(ends
            .iter()
            .all(|e| e["name"] == "web_fetch" && e["code"] == "fetch_not_allowed"));
        let starts = of_type(&env, &run, "tool_start");
        assert_eq!(starts[0]["args"], json!({"url": LINKED}));
        // No page and no second search went out.
        assert_eq!(
            gateway
                .requests()
                .iter()
                .filter(|r| r.path == "/v1/responses")
                .count(),
            1
        );
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

#[test]
fn stopping_a_run_during_its_search_leaves_the_cost_unknown() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = searching(&host, &env).await;
        gateway.reply(Reply::ToolCalls(vec![(
            "web_search",
            json!({"query": "slow news"}),
        )]));
        // The search is sent and never answered.
        gateway.reply(Reply::Hang(String::new()));
        let run = host
            .send(request(&sid, "Any slow news?", json!({})))
            .await
            .unwrap();
        wait_for(&env, &run.run_id, "tool_start").await;
        let deadline = Instant::now() + Duration::from_secs(20);
        while !gateway.requests().iter().any(|r| r.path == "/v1/responses") {
            assert!(Instant::now() < deadline, "the search never went out");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        host.stop(&sid, &run.run_id, &run.operation_id)
            .await
            .unwrap();
        let end = wait_for(&env, &run.run_id, "run_end").await;
        assert_eq!(end.data["status"], "stopped");
        assert!(of_type(&env, &run.run_id, "error").is_empty());

        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.usage["usageCoverage"], "incomplete");
        assert!(detail.usage["estimatedCostUsd"].is_null());
        assert!(detail.usage["searchCostUsd"].is_null());
        assert_eq!(gateway.requests().len(), 2, "no request after the stop");
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

#[test]
fn a_search_then_a_stop_or_a_billed_failure_leaves_both_costs_unknown() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let assert_unknown = |usage: &Value| {
            assert_eq!(usage["usageCoverage"], "incomplete", "{usage}");
            assert!(usage["estimatedCostUsd"].is_null(), "{usage}");
            assert!(usage["searchCostUsd"].is_null(), "{usage}");
        };
        // The search succeeds, then the user stops while the answer streams.
        let stopped = searching(&host, &env).await;
        gateway.reply(Reply::ToolCalls(vec![(
            "web_search",
            json!({"query": "pi"}),
        )]));
        gateway.reply(Reply::Json(200, search_answer(&["https://pi.dev/docs"])));
        gateway.reply(Reply::Hang("Partial answer ".repeat(20)));
        let run = host
            .send(request(&stopped, "Tell me about Pi", json!({})))
            .await
            .unwrap();
        wait_for(&env, &run.run_id, "text_delta").await;
        assert_eq!(of_type(&env, &run.run_id, "sources").len(), 1);
        host.stop(&stopped, &run.run_id, &run.operation_id)
            .await
            .unwrap();
        let end = wait_for(&env, &run.run_id, "run_end").await;
        assert_eq!(end.data["status"], "stopped");
        let detail = host.get_session(&stopped).await.unwrap();
        assert_unknown(&detail.usage);
        assert_eq!(detail.usage["searchCount"], 1);

        // The search succeeds, then the answer fails after it may have been billed.
        let failed = create(&host).await;
        let on = serde_json::from_value(json!({"sessionId": failed, "webSearch": true})).unwrap();
        host.update_session(on).await.unwrap();
        gateway.reply(Reply::ToolCalls(vec![(
            "web_search",
            json!({"query": "pi"}),
        )]));
        gateway.reply(Reply::Json(200, search_answer(&["https://pi.dev/docs"])));
        gateway.reply(Reply::Error(
            500,
            json!({"error": {"message": "upstream failed"}}),
        ));
        let (run, end) = turn(&host, &env, request(&failed, "Tell me again", json!({}))).await;
        assert_eq!(end.data["status"], "error");
        assert_eq!(
            wait_for(&env, &run, "error").await.data["code"],
            "assistant_model_request_failed"
        );
        let detail = host.get_session(&failed).await.unwrap();
        assert_unknown(&detail.usage);
        assert_eq!(detail.usage["searchCount"], 1);

        // A priced search, then a later answer cut off by a region change
        // before any usage was counted: nothing re-projects the history.
        let cut = create(&host).await;
        let on = serde_json::from_value(json!({"sessionId": cut, "webSearch": true})).unwrap();
        host.update_session(on).await.unwrap();
        gateway.reply(Reply::ToolCalls(vec![(
            "web_search",
            json!({"query": "pi"}),
        )]));
        gateway.reply(Reply::Json(200, search_answer(&["https://pi.dev/docs"])));
        gateway.reply(Reply::Stream("Pi builds agents.".into()));
        turn(&host, &env, request(&cut, "Tell me about Pi", json!({}))).await;
        let priced = host.get_session(&cut).await.unwrap().usage;
        assert_eq!(priced["searchCostUsd"], "0.01425", "{priced}");
        gateway.reply(Reply::Hang("Partial answer ".repeat(20)));
        let run = host
            .send(request(&cut, "And more?", json!({})))
            .await
            .unwrap();
        wait_for(&env, &run.run_id, "text_delta").await;
        env.signed_in("ofox.io");
        wait_for(&env, &run.run_id, "run_end").await;
        assert_eq!(
            wait_for(&env, &run.run_id, "error").await.data["code"],
            "assistant_identity_changed"
        );
        env.signed_in("ofox.ai");
        let detail = host.get_session(&cut).await.unwrap();
        assert_unknown(&detail.usage);
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

#[test]
fn restored_backup_continues_native_history_with_saved_search_prices_on_both_apexes() {
    use cc_switch_lib::assistant::store::AssistantStore;
    let Some(prepared) = runtime_dir() else {
        return;
    };
    for apex in ["ofox.ai", "ofox.io"] {
        let source = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(source.path(), &prepared, &[(apex, &gateway)]);
        let host = AssistantHost::new(env.clone());
        runtime().block_on(async {
            let sid = searching(&host, &env).await;
            let aid = attach(
                &host,
                source.path(),
                &sid,
                "中文材料.txt",
                "备份后还要读取的附件".as_bytes(),
            )
            .await;
            gateway.reply(Reply::ToolCalls(vec![(
                "web_search",
                json!({"query":"backup fixture"}),
            )]));
            gateway.reply(Reply::Json(
                200,
                search_answer(&["https://example.com/reference"]),
            ));
            gateway.reply(Reply::Stream("Original answer with a source.".into()));
            let (_, end) = turn(
                &host,
                &env,
                request(
                    &sid,
                    "Read this attachment and search.",
                    json!({"attachmentIds":[aid]}),
                ),
            )
            .await;
            assert_eq!(end.data["status"], "completed");
            let before = host.get_session(&sid).await.unwrap();
            let store = AssistantStore::for_owner(source.path(), &owner()).unwrap();
            let operations = store.load_record(&sid).unwrap().operations;
            assert_eq!(operations.len(), 1);
            assert!(operations[0].search_price.is_some());
            let archive_dir = tempfile::tempdir().unwrap();
            let archive = archive_dir.path().join("完整备份.zip");
            store.backup(&archive, &[KEY.into()], || Ok(())).unwrap();
            host.shutdown().await;
            drop(host);
            drop(env);
            drop(store);
            // The original absolute cwd and attachments no longer exist.
            source.close().unwrap();
            let target = tempfile::tempdir().unwrap();
            let restored = AssistantStore::for_owner(target.path(), &owner()).unwrap();
            assert_eq!(
                restored
                    .restore(&archive, &[KEY.into()], || Ok(()))
                    .unwrap()
                    .imported,
                1
            );
            let fresh_env = TestEnv::new(target.path(), &prepared, &[(apex, &gateway)]);
            let mut price = search_model();
            price.pricing["web_search"] = json!("0.999");
            fresh_env.catalog.lock().unwrap().push(price);
            let fresh = AssistantHost::new(fresh_env.clone());
            fresh.recover_all().await;
            let reloaded = fresh.get_session(&sid).await.unwrap();
            assert_eq!(
                reloaded.usage, before.usage,
                "restore must keep historical prices"
            );
            assert_eq!(restored.load_record(&sid).unwrap().operations, operations);
            assert_eq!(reloaded.messages.len(), before.messages.len());
            gateway.reply(Reply::Stream("Continued after restore.".into()));
            let (_, end) = turn(
                &fresh,
                &fresh_env,
                request(
                    &sid,
                    "Continue and reread the attachment.",
                    json!({"attachmentIds":[aid]}),
                ),
            )
            .await;
            assert_eq!(end.data["status"], "completed");
            let last = gateway.requests().last().unwrap().text();
            assert!(last.contains("Original answer with a source."));
            assert!(last.contains("备份后还要读取的附件"));
            let continued = fresh.get_session(&sid).await.unwrap();
            assert_eq!(
                continued.usage["searchCostUsd"],
                before.usage["searchCostUsd"]
            );
            assert!(continued
                .messages
                .iter()
                .any(|message| message.content == "Continued after restore."));
            assert_no_secret_on_disk(target.path(), &[KEY, NEW_KEY]);
            fresh.shutdown().await;
        });
    }
}

// ── thinking levels and Claude ───────────────────────────────────────────

/// Claude Haiku as `/anthropic/v1/models` lists it (the runtime's fixture),
/// with the thinking levels the host's table gives it.
fn claude_model() -> Model {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../assistant-runtime/test/fixtures/catalog-claude-model.json"
    ))
    .unwrap();
    parse_models(fixture["catalog"].clone(), Source::Anthropic).remove(0)
}

/// The reply a run wrote (its `run_start` names it).
fn end_message(env: &TestEnv, run: &str) -> Value {
    of_type(env, run, "run_start")[0]["messageId"].clone()
}

/// Reasoning long enough that the runtime streams it before it ends.
fn reasoning_text() -> String {
    "Weighing the options one by one before answering. ".repeat(6)
}

async fn set_level(host: &Arc<AssistantHost>, sid: &str, level: &str) -> Option<String> {
    let update = serde_json::from_value(json!({"sessionId": sid, "thinkingLevel": level})).unwrap();
    host.update_session(update).await.unwrap().thinking_level
}

/// What the model was sent about thinking, whatever the protocol.
fn thinking_fields(request: &Request) -> Vec<&str> {
    ["thinking", "output_config", "reasoning", "reasoning_effort"]
        .into_iter()
        .filter(|field| request.body.get(*field).is_some())
        .collect()
}

#[test]
fn claude_runs_with_messages_and_thinks_only_at_a_chosen_level() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let claude = claude_model();
    assert_eq!(
        claude.thinking_levels,
        ["off", "low", "medium", "high", "xhigh", "max"]
    );
    env.anthropic.lock().unwrap().push(claude.clone());
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let boot = host.bootstrap().await.unwrap();
        let offered = boot["models"].as_array().unwrap();
        let listed = offered.iter().find(|m| m["id"] == claude.id).unwrap();
        assert_eq!(listed["thinkingLevels"], json!(claude.thinking_levels));
        let create: CreateRequest = serde_json::from_value(json!({"modelId": claude.id})).unwrap();
        let sid = host.create_session(create).await.unwrap().id;

        // The default: Messages, the key as a Bearer token to the gateway
        // only, and no thinking field.
        gateway.reply(Reply::Stream("Claude answers".into()));
        let (run, end) = turn(&host, &env, request(&sid, "Hello Claude", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let sent = &gateway.requests()[0];
        assert_eq!(sent.path, "/anthropic/v1/messages?beta=true");
        assert_eq!(sent.authorization, format!("Bearer {KEY}"));
        assert!(!sent.headers.contains_key("x-api-key"));
        assert_eq!(sent.body["model"], claude.id);
        assert!(thinking_fields(sent).is_empty(), "{:?}", sent.body);
        let start = &of_type(&env, &run, "run_start")[0];
        assert_eq!(start["thinkingLevel"], Value::Null);
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.messages[1].content, "Claude answers");
        assert!(detail.messages[1].reasoning_ms.is_none());
        assert_eq!(detail.usage["inputTokens"], 10);

        // At a level: adaptive thinking at that effort; the reasoning
        // streams, is kept, and its time is counted.
        assert_eq!(
            set_level(&host, &sid, "high").await.as_deref(),
            Some("high")
        );
        gateway.reply(Reply::Thinking(reasoning_text(), "Thought it over".into()));
        let (run, end) = turn(&host, &env, request(&sid, "Think first", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let sent = &gateway.requests()[1];
        assert_eq!(sent.body["thinking"]["type"], "adaptive");
        assert_eq!(sent.body["output_config"], json!({"effort": "high"}));
        assert!(sent.body.get("temperature").is_none());
        assert!(
            sent.text().contains("Claude answers"),
            "the context goes too"
        );
        assert_eq!(of_type(&env, &run, "run_start")[0]["thinkingLevel"], "high");
        // The start is published before the reasoning text.
        let kinds: Vec<String> = env.events(&run).into_iter().map(|e| e.r#type).collect();
        let started = kinds
            .iter()
            .position(|kind| kind == "thinking_start")
            .unwrap();
        assert!(
            started
                < kinds
                    .iter()
                    .position(|kind| kind == "thinking_delta")
                    .unwrap()
        );
        assert_eq!(
            of_type(&env, &run, "thinking_start"),
            [json!({"messageId": end_message(&env, &run)})]
        );
        assert_reasoning_ends_before_the_text(&kinds);
        assert_eq!(
            of_type(&env, &run, "thinking_end"),
            [json!({"messageId": end_message(&env, &run)})]
        );
        let streamed: String = of_type(&env, &run, "thinking_delta")
            .iter()
            .map(|data| data["delta"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(streamed, reasoning_text());
        let detail = host.get_session(&sid).await.unwrap();
        let reply = detail.messages.last().unwrap();
        assert_eq!(reply.content, "Thought it over");
        assert_eq!(reply.reasoning.as_deref(), Some(reasoning_text().as_str()));
        let spent = reply.reasoning_ms.expect("the reasoning was timed");
        assert!(spent >= 100, "{spent} ms");
        assert_eq!(
            saved(config.path(), &sid)["messages"][3]["reasoningMs"],
            spent
        );

        // "off" turns Claude's thinking off; "default" sends nothing again.
        set_level(&host, &sid, "off").await;
        gateway.reply(Reply::Stream("No thinking".into()));
        turn(&host, &env, request(&sid, "Quick one", json!({}))).await;
        assert_eq!(
            gateway.requests()[2].body["thinking"],
            json!({"type": "disabled"})
        );
        assert!(set_level(&host, &sid, "default").await.is_none());
        gateway.reply(Reply::Stream("Default again".into()));
        let (run, _) = turn(&host, &env, request(&sid, "And now?", json!({}))).await;
        assert!(thinking_fields(&gateway.requests()[3]).is_empty());
        assert_eq!(
            of_type(&env, &run, "run_start")[0]["thinkingLevel"],
            Value::Null
        );
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

/// One `thinking_end` among a run's event kinds: after its last reasoning,
/// before the reply's first words.
fn assert_reasoning_ends_before_the_text(kinds: &[String]) {
    let ends: Vec<usize> = (0..kinds.len())
        .filter(|at| kinds[*at] == "thinking_end")
        .collect();
    assert_eq!(ends.len(), 1, "{kinds:?}");
    let last_reasoning = kinds.iter().rposition(|kind| kind == "thinking_delta");
    let first_text = kinds.iter().position(|kind| kind == "text_delta");
    assert!(last_reasoning.unwrap() < ends[0], "{kinds:?}");
    assert!(ends[0] < first_text.unwrap(), "{kinds:?}");
}

/// The test model, reasoning with these levels and no Pi data.
fn reasoning_model(levels: &[&str]) -> Model {
    let mut model = test_model();
    model.reasoning = true;
    model.thinking_levels = levels.iter().map(|level| level.to_string()).collect();
    model
}

#[test]
fn a_chat_completions_level_is_sent_only_while_chosen_and_offered() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    *env.catalog.lock().unwrap() = vec![reasoning_model(&["low", "medium", "high"])];
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Stream("Plain".into()));
        turn(&host, &env, request(&sid, "One", json!({}))).await;
        assert!(thinking_fields(&gateway.requests()[0]).is_empty());

        assert_eq!(
            set_level(&host, &sid, "medium").await.as_deref(),
            Some("medium")
        );
        gateway.reply(Reply::Thinking(reasoning_text(), "Considered".into()));
        let (run, end) = turn(&host, &env, request(&sid, "Two", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        assert_eq!(gateway.requests()[1].body["reasoning_effort"], "medium");
        assert_eq!(
            of_type(&env, &run, "run_start")[0]["thinkingLevel"],
            "medium"
        );
        // Chat Completions ends its blocks only with the stream: the
        // reasoning ends where the text begins.
        let kinds: Vec<String> = env.events(&run).into_iter().map(|e| e.r#type).collect();
        assert_reasoning_ends_before_the_text(&kinds);
        let detail = host.get_session(&sid).await.unwrap();
        let reply = detail.messages.last().unwrap();
        assert_eq!(reply.reasoning.as_deref(), Some(reasoning_text().as_str()));
        assert!(reply.reasoning_ms.is_some_and(|ms| ms >= 100));

        // The catalog no longer offers the level: the run goes at the default.
        *env.catalog.lock().unwrap() = vec![reasoning_model(&["high"])];
        let host = AssistantHost::new(env.clone());
        gateway.reply(Reply::Stream("Default".into()));
        let (run, _) = turn(&host, &env, request(&sid, "Three", json!({}))).await;
        assert!(thinking_fields(&gateway.requests()[2]).is_empty());
        assert_eq!(
            of_type(&env, &run, "run_start")[0]["thinkingLevel"],
            Value::Null
        );
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.session.thinking_level.as_deref(), Some("medium"));
    });
}

#[test]
fn a_model_without_tools_runs_without_search_or_any_tool() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let mut toolless = test_model();
    toolless.tools = false;
    *env.catalog.lock().unwrap() = vec![toolless];
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        // Search is priced and on, and the conversation has a file.
        let sid = searching(&host, &env).await;
        let file = attach(&host, config.path(), &sid, "notes.txt", b"kept for later").await;
        gateway.reply(Reply::Stream("First".into()));
        turn(
            &host,
            &env,
            request(&sid, "Read it", json!({"attachmentIds": [file]})),
        )
        .await;
        gateway.reply(Reply::Stream("Second".into()));
        let (run, end) = turn(&host, &env, request(&sid, "Search the web", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        assert_eq!(of_type(&env, &run, "run_start")[0]["webSearch"], false);
        for sent in gateway.requests() {
            assert!(sent.body.get("tools").is_none(), "{:?}", sent.body["tools"]);
        }
        let detail = host.get_session(&sid).await.unwrap();
        assert!(
            detail.session.web_search,
            "the switch stays as the user set it"
        );
        assert_eq!(detail.usage["searchCount"], 0);
    });
}

// ── generated titles ─────────────────────────────────────────────────────

fn gate() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// The title requests the gateway got, once there are `count` of them.
async fn title_requests(gateway: &Gateway, count: usize) -> Vec<Request> {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let asked = gateway.title_requests();
        if asked.len() >= count {
            return asked;
        }
        assert!(
            Instant::now() < deadline,
            "{} title requests, wanted {count}",
            asked.len()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// No further title request arrives for a while (a title is asked for
/// within a second or two of the turn's end).
async fn no_more_title_requests(gateway: &Gateway, count: usize) {
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(gateway.title_requests().len(), count);
}

/// The saved conversation, once `ready` holds for it.
async fn saved_once(config: &Path, sid: &str, ready: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let record = saved(config, sid);
        if ready(&record) {
            return record;
        }
        assert!(
            Instant::now() < deadline,
            "never ready: {}",
            record["usage"]
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The saved conversation, once it counts `count` model requests.
async fn counting(config: &Path, sid: &str, count: u64) -> Value {
    saved_once(config, sid, |record| {
        record["usage"]["modelRequests"] == count
    })
    .await
}

/// The one `session` event a title published for `sid`, once it is out.
async fn title_event(env: &TestEnv, sid: &str, runs: &[&str]) -> Value {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let published = title_events(env, sid, runs);
        if let [event] = published.as_slice() {
            return event.clone();
        }
        assert!(published.len() < 2, "{published:?}");
        assert!(Instant::now() < deadline, "the title published nothing");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The `session` events of `sid` that no run published: a title's.
fn title_events(env: &TestEnv, sid: &str, runs: &[&str]) -> Vec<Value> {
    env.events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.session_id == sid && e.r#type == "session")
        .filter(|e| !runs.contains(&e.run_id.as_str()))
        .map(|e| e.data.clone())
        .collect()
}

#[test]
fn the_first_answer_names_the_conversation_once_without_holding_up_a_run() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let open = gate();
        gateway.title(Reply::Wait(
            open.clone(),
            Box::new(Reply::Stream("\"Kyoto Trip Plan.\"".into())),
        ));
        gateway.reply(Reply::Stream("Day one: Fushimi Inari.".into()));
        // The turn ends while its title is still held at the gateway.
        let (first, end) = turn(
            &host,
            &env,
            request(&sid, "Plan a trip to Kyoto", json!({})),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        // A turn sent at once is not named again; it waits for the title,
        // which is being written into the native history.
        gateway.reply(Reply::Stream("Day two: Arashiyama.".into()));
        let second = host
            .send(request(&sid, "And day two?", json!({})))
            .await
            .unwrap();
        let asked = title_requests(&gateway, 1).await;
        // The second turn is held back until the title is written.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(gateway.requests().len(), 1, "the next turn waits");
        assert!(of_type(&env, &second.run_id, "text_delta").is_empty());
        assert_eq!(asked[0].authorization, format!("Bearer {KEY}"));
        assert!(!asked[0].body.to_string().contains(KEY));
        // No priced search model in this catalog: the conversation's model.
        assert_eq!(asked[0].body["model"], MODEL);
        assert!(asked[0].body.get("reasoning_effort").is_none());
        let instruction = asked[0].body["messages"][0]["content"].as_str().unwrap();
        assert!(instruction.contains("title only"), "{instruction}");
        let question = asked[0].text();
        assert!(question.contains("Plan a trip to Kyoto"), "{question}");
        assert!(question.contains("Day one: Fushimi Inari."), "{question}");
        open.store(true, Ordering::SeqCst);
        let end = wait_for(&env, &second.run_id, "run_end").await;
        assert_eq!(end.data["status"], "completed");

        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.session.title, "Kyoto Trip Plan");
        assert_eq!(detail.session.title_source, Some(types::TitleSource::Auto));
        // The title is part of the conversation's cost: three requests of
        // 10 + 5 tokens at $2 / $10 per million.
        assert_eq!(detail.usage["modelRequests"], 3);
        assert_eq!(detail.usage["inputTokens"], 30);
        assert!((decimal(&detail.usage["estimatedCostUsd"]) - 0.00021).abs() < 1e-12);
        let native =
            std::fs::read_to_string(session_dir(config.path(), &sid).join("session.jsonl"))
                .unwrap();
        assert!(native.contains("\"type\":\"title\""));
        // Published as the conversation's session, apart from both turns.
        let named = title_events(&env, &sid, &[&first, &second.run_id]);
        assert_eq!(named.len(), 1);
        assert_eq!(named[0]["session"]["title"], "Kyoto Trip Plan");
        // With the conversation's cost as the title left it.
        assert_eq!(named[0]["usage"]["modelRequests"], 2);
        // The next turn's context is the conversation, not its title.
        let requests = gateway.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].text().contains("Fushimi Inari"));
        assert!(!requests[1].text().contains("Kyoto Trip Plan"));

        // Regenerating the first answer does not name the conversation again.
        let reply = end_message(&env, &first);
        gateway.reply(Reply::Stream("Day one: Kiyomizu-dera.".into()));
        let (_, end) = turn(
            &host,
            &env,
            request(&sid, "", json!({"regenerateMessageId": reply})),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        no_more_title_requests(&gateway, 1).await;
        assert_eq!(
            host.get_session(&sid).await.unwrap().session.title,
            "Kyoto Trip Plan"
        );
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

#[test]
fn a_rename_or_trash_while_the_title_is_written_wins_and_the_title_is_still_paid_for() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        for (change, title, source) in [
            (
                json!({"title": "My own name"}),
                "My own name",
                types::TitleSource::User,
            ),
            (
                json!({"deleted": true}),
                "Plan a trip to Kyoto",
                types::TitleSource::Auto,
            ),
        ] {
            let sid = create(&host).await;
            let open = gate();
            gateway.title(Reply::Wait(
                open.clone(),
                Box::new(Reply::Stream("Kyoto Trip Plan".into())),
            ));
            gateway.reply(Reply::Stream("Day one.".into()));
            let asked = gateway.title_requests().len();
            let (run, _) = turn(
                &host,
                &env,
                request(&sid, "Plan a trip to Kyoto", json!({})),
            )
            .await;
            title_requests(&gateway, asked + 1).await;
            let mut update = json!({"sessionId": sid});
            update
                .as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            host.update_session(serde_json::from_value(update).unwrap())
                .await
                .unwrap();
            open.store(true, Ordering::SeqCst);
            // The request was answered: its cost is counted all the same.
            let record = counting(config.path(), &sid, 2).await;
            assert_eq!(record["session"]["title"], title, "{change}");
            assert_eq!(record["session"]["titleSource"], json!(source), "{change}");
            // Published all the same: the open conversation's cost changed.
            let published = title_event(&env, &sid, &[&run]).await;
            assert_eq!(published["session"]["title"], title, "{change}");
            assert_eq!(published["usage"]["modelRequests"], 2, "{change}");
            assert_eq!(published["usage"], record["usage"], "{change}");
        }
    });
}

#[test]
fn a_turn_that_got_no_answer_is_not_named_and_the_first_answer_is() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Error(
            402,
            json!({"error": {"message": "Insufficient credits"}}),
        ));
        let (_, end) = turn(
            &host,
            &env,
            request(&sid, "Plan a trip to Kyoto", json!({})),
        )
        .await;
        assert_eq!(end.data["status"], "error");
        no_more_title_requests(&gateway, 0).await;

        gateway.title(Reply::Stream("Osaka Food Tour".into()));
        gateway.reply(Reply::Stream("Start in Dotonbori.".into()));
        let (_, end) = turn(
            &host,
            &env,
            request(&sid, "Then plan Osaka instead", json!({})),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        let asked = title_requests(&gateway, 1).await;
        assert!(asked[0].text().contains("Then plan Osaka instead"));
        let record = counting(config.path(), &sid, 3).await;
        assert_eq!(record["session"]["title"], "Osaka Food Tour");
        no_more_title_requests(&gateway, 1).await;
    });
}

#[test]
fn a_priced_search_model_writes_titles_with_the_least_reasoning() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let mut nano = search_model();
    nano.reasoning = true;
    nano.thinking_levels = ["minimal", "low", "medium", "high"]
        .map(String::from)
        .to_vec();
    nano.thinking_level_map = Some(
        serde_json::from_value(
            json!({"high": "high", "low": "low", "max": null, "medium": "medium",
                                      "minimal": "minimal", "off": null, "xhigh": null}),
        )
        .unwrap(),
    );
    env.catalog.lock().unwrap().push(nano);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.title(Reply::Stream("Kyoto".into()));
        gateway.reply(Reply::Stream("Day one.".into()));
        turn(
            &host,
            &env,
            request(&sid, "Plan a trip to Kyoto", json!({})),
        )
        .await;
        let asked = title_requests(&gateway, 1).await;
        assert_eq!(asked[0].body["model"], SEARCH_MODEL);
        assert_eq!(asked[0].body["reasoning_effort"], "minimal");
        assert_eq!(asked[0].body["max_tokens"], 32);
        // The conversation keeps its own model for its turns.
        assert_eq!(gateway.requests()[0].body["model"], MODEL);
        let record = counting(config.path(), &sid, 2).await;
        assert_eq!(record["session"]["title"], "Kyoto");
    });
}

#[test]
fn a_logout_stops_a_title_on_its_way_and_its_cost_stays_unknown() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let open = gate();
        gateway.title(Reply::Wait(
            open.clone(),
            Box::new(Reply::Stream("Kyoto Trip Plan".into())),
        ));
        gateway.reply(Reply::Stream("Day one.".into()));
        let (run, _) = turn(
            &host,
            &env,
            request(&sid, "Plan a trip to Kyoto", json!({})),
        )
        .await;
        title_requests(&gateway, 1).await;
        let receipt = host.begin_logout();
        // No answer within the grace: the title is stopped. The request was
        // sent and may have been billed: the conversation's cost becomes
        // unknown, and it keeps its name.
        let record = saved_once(config.path(), &sid, |record| {
            record["usage"]["usageCoverage"] == "incomplete"
        })
        .await;
        assert_eq!(record["session"]["title"], "Plan a trip to Kyoto");
        assert!(record["usage"]["estimatedCostUsd"].is_null());
        assert_eq!(record["usage"]["modelRequests"], 1, "no usage was reported");
        let native =
            std::fs::read_to_string(session_dir(config.path(), &sid).join("session.jsonl"))
                .unwrap();
        assert!(native.contains("\"type\":\"title\""));
        let published = title_event(&env, &sid, &[&run]).await;
        assert_eq!(published["usage"]["usageCoverage"], "incomplete");
        assert_eq!(published["session"]["title"], "Plan a trip to Kyoto");
        open.store(true, Ordering::SeqCst);
        host.finish_logout(receipt);
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn quitting_while_a_title_is_on_its_way_waits_for_its_cost() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let open = gate();
        gateway.title(Reply::Wait(
            open.clone(),
            Box::new(Reply::Stream("Kyoto Trip Plan".into())),
        ));
        gateway.reply(Reply::Stream("Day one.".into()));
        turn(
            &host,
            &env,
            request(&sid, "Plan a trip to Kyoto", json!({})),
        )
        .await;
        title_requests(&gateway, 1).await;
        host.shutdown().await;
        // Shutdown is over: what the title may have cost is already saved.
        let record = saved(config.path(), &sid);
        assert_eq!(record["usage"]["usageCoverage"], "incomplete");
        assert_eq!(record["session"]["title"], "Plan a trip to Kyoto");
        open.store(true, Ordering::SeqCst);
    });
}

#[test]
fn a_title_that_answers_soon_after_a_logout_keeps_its_known_cost() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let open = gate();
        gateway.title(Reply::Wait(
            open.clone(),
            Box::new(Reply::Stream("Kyoto Trip Plan".into())),
        ));
        gateway.reply(Reply::Stream("Day one.".into()));
        let (run, _) = turn(
            &host,
            &env,
            request(&sid, "Plan a trip to Kyoto", json!({})),
        )
        .await;
        title_requests(&gateway, 1).await;
        let receipt = host.begin_logout();
        // The request already on its way answers within the grace.
        tokio::time::sleep(Duration::from_millis(600)).await;
        open.store(true, Ordering::SeqCst);
        let published = title_event(&env, &sid, &[&run]).await;
        assert_eq!(published["usage"]["modelRequests"], 2);
        assert_ne!(published["usage"]["usageCoverage"], "incomplete");
        assert!((decimal(&published["usage"]["estimatedCostUsd"]) - 0.00014).abs() < 1e-12);
        // Not saved under an authority that has gone.
        assert_eq!(published["session"]["title"], "Plan a trip to Kyoto");
        host.finish_logout(receipt);
    });
}

#[test]
fn quitting_lets_a_title_answer_until_shortly_before_the_end_of_shutdown() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let open = gate();
        gateway.title(Reply::Wait(
            open.clone(),
            Box::new(Reply::Stream("Kyoto Trip Plan".into())),
        ));
        gateway.reply(Reply::Stream("Day one.".into()));
        turn(
            &host,
            &env,
            request(&sid, "Plan a trip to Kyoto", json!({})),
        )
        .await;
        title_requests(&gateway, 1).await;
        let answer_soon = async {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            open.store(true, Ordering::SeqCst);
        };
        tokio::join!(host.shutdown(), answer_soon);
        let record = saved(config.path(), &sid);
        assert_eq!(record["usage"]["modelRequests"], 2);
        assert_ne!(record["usage"]["usageCoverage"], "incomplete");
        assert!((decimal(&record["usage"]["estimatedCostUsd"]) - 0.00014).abs() < 1e-12);
    });
}

// ── versions, retries and per-message meta ───────────────────────────────

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(role, content)| (role.to_string(), content.to_string()))
        .collect()
}

fn switch(host: &AssistantHost, sid: &str, message: &str) -> SwitchVersionRequest {
    serde_json::from_value(json!({
        "sessionId": sid, "expectedOwner": owner(),
        "expectedGeneration": host.generation(), "messageId": message,
    }))
    .unwrap()
}

fn native_of(config: &Path, sid: &str) -> Vec<u8> {
    std::fs::read(session_dir(config, sid).join("session.jsonl")).unwrap()
}

fn find<'a>(detail: &'a Detail, content: &str) -> &'a Message {
    detail
        .messages
        .iter()
        .find(|m| m.content == content)
        .unwrap()
}

const VERSION_A: [(&str, &str); 4] = [
    ("user", "Question A"),
    ("assistant", "Answer A"),
    ("user", "Follow-up"),
    ("assistant", "Answer A2"),
];
const VERSION_B: [(&str, &str); 2] = [("user", "Question B"), ("assistant", "Answer B")];

/// Question A, its follow-up, then Question A edited into Question B.
async fn two_versions(host: &Arc<AssistantHost>, env: &TestEnv, gateway: &Gateway) -> String {
    let sid = create(host).await;
    gateway.reply(Reply::Stream("Answer A".into()));
    turn(host, env, request(&sid, "Question A", json!({}))).await;
    gateway.reply(Reply::Stream("Answer A2".into()));
    turn(host, env, request(&sid, "Follow-up", json!({}))).await;
    let question = host.get_session(&sid).await.unwrap().messages[0].id.clone();
    gateway.reply(Reply::Stream("Answer B".into()));
    turn(
        host,
        env,
        request(&sid, "Question B", json!({ "editMessageId": question })),
    )
    .await;
    sid
}

#[test]
fn switching_versions_shows_the_chosen_turn_and_keeps_it_after_a_restart() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    let (sid, question_a, question_b) = runtime().block_on(async {
        let sid = two_versions(&host, &env, &gateway).await;
        let edited = host.get_session(&sid).await.unwrap();
        assert_eq!(active(&edited), pairs(&VERSION_B));
        // Both versions start the conversation; the follow-up follows answer A.
        let versions: Vec<&str> = edited
            .messages
            .iter()
            .filter(|m| m.role == "user" && m.branch_from == Some(None))
            .map(|m| m.content.as_str())
            .collect();
        assert_eq!(versions, ["Question A", "Question B"]);
        assert_eq!(
            find(&edited, "Follow-up").branch_from,
            Some(find(&edited, "Answer A").native_entry_id.clone())
        );
        let question_a = find(&edited, "Question A").id.clone();
        let question_b = find(&edited, "Question B").id.clone();
        let titles = gateway.title_requests().len();

        let shown = host
            .switch_version(switch(&host, &sid, &question_a))
            .await
            .unwrap();
        assert_eq!(active(&shown), pairs(&VERSION_A));
        assert!(find(&shown, "Answer B").superseded);
        assert_eq!(
            active(&host.get_session(&sid).await.unwrap()),
            pairs(&VERSION_A)
        );
        // Not a new message: the conversation keeps its place in the list.
        assert_eq!(shown.session.updated_at, edited.session.updated_at);
        // The version shown already: nothing is written.
        let native = native_of(config.path(), &sid);
        let again = host
            .switch_version(switch(&host, &sid, &question_a))
            .await
            .unwrap();
        assert_eq!(active(&again), pairs(&VERSION_A));
        // Refused: another authority, or no user turn of native history.
        let mut stale = switch(&host, &sid, &question_b);
        stale.authority.expected_generation += 1;
        assert_eq!(
            host.switch_version(stale).await.unwrap_err(),
            "assistant_identity_changed"
        );
        let reply = find(&shown, "Answer B").id.clone();
        for message in [types::id(), reply] {
            assert_eq!(
                host.switch_version(switch(&host, &sid, &message))
                    .await
                    .unwrap_err(),
                "assistant_version_unavailable"
            );
        }
        assert_eq!(native_of(config.path(), &sid), native);
        // A switch never names the conversation.
        no_more_title_requests(&gateway, titles).await;
        (sid, question_a, question_b)
    });
    drop(host);

    // A restarted app shows the same version, and the next turn continues it.
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        assert_eq!(
            active(&host.get_session(&sid).await.unwrap()),
            pairs(&VERSION_A)
        );
        gateway.reply(Reply::Stream("Answer C".into()));
        turn(&host, &env, request(&sid, "Question C", json!({}))).await;
        let context = gateway.requests().last().unwrap().text();
        assert!(context.contains("Question A") && context.contains("Answer A2"));
        assert!(!context.contains("Question B") && !context.contains("Answer B"));
        let back = host
            .switch_version(switch(&host, &sid, &question_b))
            .await
            .unwrap();
        assert_eq!(active(&back), pairs(&VERSION_B));
        let forth = host
            .switch_version(switch(&host, &sid, &question_a))
            .await
            .unwrap();
        let mut continued = pairs(&VERSION_A);
        continued.extend(pairs(&[("user", "Question C"), ("assistant", "Answer C")]));
        assert_eq!(active(&forth), continued);
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
}

#[test]
fn opening_an_older_conversation_restores_version_metadata_without_another_paid_turn() {
    use cc_switch_lib::assistant::store::AssistantStore;
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = two_versions(&host, &env, &gateway).await;
        let store = AssistantStore::for_owner(config.path(), &owner()).unwrap();
        store
            .mutate(&sid, |record| {
                for message in &mut record.messages {
                    message.branch_from = None;
                }
                Ok(())
            })
            .unwrap();
        let requests = gateway.requests().len();
        let reopened = AssistantHost::new(env.clone());
        let detail = reopened.get_session(&sid).await.unwrap();
        let a = find(&detail, "Question A");
        let b = find(&detail, "Question B");
        assert_eq!(a.branch_from, Some(None));
        assert_eq!(b.branch_from, Some(None));
        assert!(a.superseded && !b.superseded);
        assert_eq!(gateway.requests().len(), requests);
    });
}

#[test]
fn a_version_switch_is_refused_while_the_conversation_replies() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = two_versions(&host, &env, &gateway).await;
        let question_a = find(&host.get_session(&sid).await.unwrap(), "Question A")
            .id
            .clone();
        let open = gate();
        gateway.reply(Reply::Wait(
            open.clone(),
            Box::new(Reply::Stream("Answer C".into())),
        ));
        let run = host
            .send(request(&sid, "Question C", json!({})))
            .await
            .unwrap();
        let native = native_of(config.path(), &sid);
        assert_eq!(
            host.switch_version(switch(&host, &sid, &question_a))
                .await
                .unwrap_err(),
            "assistant_session_busy"
        );
        open.store(true, Ordering::SeqCst);
        let end = wait_for(&env, &run.run_id, "run_end").await;
        assert_eq!(end.data["status"], "completed");
        assert!(native_of(config.path(), &sid).starts_with(&native));
        // Once the reply has ended, the switch goes ahead.
        let shown = host
            .switch_version(switch(&host, &sid, &question_a))
            .await
            .unwrap();
        assert_eq!(active(&shown), pairs(&VERSION_A));
    });
}

#[test]
fn retrying_a_turn_native_history_never_got_hides_it_and_asks_once() {
    use cc_switch_lib::assistant::store::AssistantStore;
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Stream("Answer A".into()));
        turn(&host, &env, request(&sid, "Question A", json!({}))).await;
        // A turn whose runtime never started, as the host saves one.
        let (run, failed) = (types::id(), types::id());
        let store = AssistantStore::for_owner(config.path(), &owner()).unwrap();
        store
            .mutate(&sid, |record| {
                let at = types::now();
                record.messages.extend([
                    serde_json::from_value(json!({
                        "id": types::id(), "role": "user", "content": "Question B",
                        "createdAt": at, "reasoning": null, "status": "completed",
                        "revisionOf": null, "nativeEntryId": null, "runId": run,
                    }))
                    .unwrap(),
                    serde_json::from_value(json!({
                        "id": failed, "role": "assistant", "content": "", "createdAt": at,
                        "reasoning": null, "status": "error",
                        "errorCode": "assistant_runtime_exited", "revisionOf": null,
                        "nativeEntryId": null, "runId": run, "modelId": MODEL, "completedAt": at,
                    }))
                    .unwrap(),
                ]);
                record.operations.push(
                    serde_json::from_value(json!({
                        "id": types::id(), "runId": run, "status": "failed",
                    }))
                    .unwrap(),
                );
                Ok(())
            })
            .unwrap();

        gateway.reply(Reply::Stream("Answer B".into()));
        let retry = request(&sid, "Question B", json!({ "retryMessageId": failed }));
        let (_, end) = turn(&host, &env, retry).await;
        assert_eq!(end.data["status"], "completed");
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(
            active(&detail),
            pairs(&[
                ("user", "Question A"),
                ("assistant", "Answer A"),
                ("user", "Question B"),
                ("assistant", "Answer B"),
            ])
        );
        let hidden: Vec<&str> = superseded(&detail).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(hidden.len(), 2);
        assert!(hidden.contains(&failed.as_str()));
        // The model was asked the question once.
        let context = gateway.requests().last().unwrap().text();
        assert_eq!(context.matches("Question B").count(), 1, "{context}");
        // A retried turn cannot be retried again.
        let again = request(&sid, "Question B", json!({ "retryMessageId": failed }));
        assert_eq!(
            host.send(again).await.unwrap_err(),
            "assistant_branch_unavailable"
        );
    });
}

#[test]
fn each_reply_names_its_model_and_with_the_title_the_replies_add_up_to_the_cost() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.title(Reply::Stream("Two Questions".into()));
        gateway.reply(Reply::Stream("Answer one".into()));
        let (first, _) = turn(&host, &env, request(&sid, "Question one", json!({}))).await;
        title_event(&env, &sid, &[&first]).await;
        gateway.reply(Reply::Stream("Answer two".into()));
        let (second, _) = turn(&host, &env, request(&sid, "Question two", json!({}))).await;
        let detail = host.get_session(&sid).await.unwrap();
        let replies: Vec<&Message> = detail
            .messages
            .iter()
            .filter(|m| m.role == "assistant")
            .collect();
        assert_eq!(replies.len(), 2);
        // Every request is 10 + 5 tokens at $2 / $10 per million.
        for reply in &replies {
            assert_eq!(reply.model_id.as_deref(), Some(MODEL));
            assert!(reply.completed_at.is_some_and(|at| at >= reply.created_at));
            let usage = reply.usage.as_ref().unwrap();
            assert_eq!(usage["modelRequests"], 1, "{usage}");
            assert_eq!(usage["inputTokens"], 10);
            assert_eq!(usage["outputTokens"], 5);
            assert!((decimal(&usage["estimatedCostUsd"]) - 0.00007).abs() < 1e-12);
        }
        for message in detail.messages.iter().filter(|m| m.role == "user") {
            assert!(message.usage.is_none() && message.model_id.is_none());
        }
        // The title is the conversation's third request, on no reply.
        assert_eq!(detail.usage["modelRequests"], 3);
        assert_eq!(detail.usage["inputTokens"], 30);
        assert!((decimal(&detail.usage["estimatedCostUsd"]) - 0.00021).abs() < 1e-12);
        // The reply event carries the meta too.
        let published = wait_for(&env, &second, "message").await;
        assert_eq!(published.data["message"]["modelId"], MODEL);
        assert!(published.data["message"]["completedAt"].is_i64());
        assert_eq!(published.data["message"]["usage"]["inputTokens"], 10);
    });
}

#[test]
fn retrying_a_turn_the_runtime_kept_but_the_host_did_not_save_asks_from_before_it() {
    use cc_switch_lib::assistant::store::AssistantStore;
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Stream("Answer A".into()));
        turn(&host, &env, request(&sid, "Question A", json!({}))).await;
        // The runtime keeps the question and the failed reply.
        gateway.reply(Reply::Error(
            500,
            json!({"error": {"message": "overloaded", "type": "server_error"}}),
        ));
        let (run, end) = turn(&host, &env, request(&sid, "Question B", json!({}))).await;
        assert_eq!(end.data["status"], "error");
        // As a host that failed before saving it leaves the conversation (its
        // runtime died, or the host itself failed): none of it is known.
        let store = AssistantStore::for_owner(config.path(), &owner()).unwrap();
        let failed = store
            .mutate(&sid, |record| {
                let mut failed = None;
                for message in record
                    .messages
                    .iter_mut()
                    .filter(|m| m.run_id.as_deref() == Some(run.as_str()))
                {
                    message.native_entry_id = None;
                    message.branch_from = None;
                    if message.role == "assistant" {
                        failed = Some(message.id.clone());
                    }
                }
                Ok(failed.unwrap())
            })
            .unwrap();

        gateway.reply(Reply::Stream("Answer B".into()));
        let retry = request(&sid, "Question B", json!({ "retryMessageId": failed }));
        let (_, end) = turn(&host, &env, retry).await;
        assert_eq!(end.data["status"], "completed");
        // Asked again from before the kept question: the model saw it once.
        let context = gateway.requests().last().unwrap().text();
        assert_eq!(context.matches("Question B").count(), 1, "{context}");
        gateway.reply(Reply::Stream("Answer C".into()));
        turn(&host, &env, request(&sid, "Question C", json!({}))).await;
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(
            active(&detail),
            pairs(&[
                ("user", "Question A"),
                ("assistant", "Answer A"),
                ("user", "Question B"),
                ("assistant", "Answer B"),
                ("user", "Question C"),
                ("assistant", "Answer C"),
            ])
        );
        let questions: Vec<&Message> = detail
            .messages
            .iter()
            .filter(|m| m.role == "user" && m.content == "Question B")
            .collect();
        assert_eq!(questions.len(), 2);
        // The failed question is a version of the turn, hidden for good.
        let (kept, asked) = (questions[0], questions[1]);
        assert_eq!(kept.run_id.as_deref(), Some(run.as_str()));
        assert!(kept.superseded && !asked.superseded);
        assert!(kept.native_entry_id.is_some());
        assert_eq!(kept.branch_from, asked.branch_from);
        assert_eq!(
            asked.branch_from,
            Some(find(&detail, "Answer A").native_entry_id.clone())
        );
    });
}

#[test]
fn a_turn_that_never_reached_the_runtime_shows_with_the_version_it_was_sent_on() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        let open = gate();
        gateway.title(Reply::Wait(
            open.clone(),
            Box::new(Reply::Stream("Two Versions".into())),
        ));
        gateway.reply(Reply::Stream("Answer A".into()));
        turn(&host, &env, request(&sid, "Question A", json!({}))).await;
        // The title holds the native history, so the next run waits for it
        // and is stopped before it reaches the runtime.
        title_requests(&gateway, 1).await;
        let run = host
            .send(request(&sid, "Follow-up", json!({})))
            .await
            .unwrap();
        host.stop(&sid, &run.run_id, &run.operation_id)
            .await
            .unwrap();
        open.store(true, Ordering::SeqCst);
        let end = wait_for(&env, &run.run_id, "run_end").await;
        assert_eq!(end.data["status"], "stopped");
        let on_a = host.get_session(&sid).await.unwrap();
        let local: Vec<&Message> = on_a
            .messages
            .iter()
            .filter(|m| m.run_id.as_deref() == Some(run.run_id.as_str()))
            .collect();
        assert_eq!(local.len(), 2);
        assert!(local
            .iter()
            .all(|m| m.native_entry_id.is_none() && !m.superseded));
        let question_a = find(&on_a, "Question A").id.clone();
        let with_local = pairs(&[
            ("user", "Question A"),
            ("assistant", "Answer A"),
            ("user", "Follow-up"),
            ("assistant", ""),
        ]);
        assert_eq!(active(&on_a), with_local);

        gateway.reply(Reply::Stream("Answer B".into()));
        turn(
            &host,
            &env,
            request(&sid, "Question B", json!({ "editMessageId": question_a })),
        )
        .await;
        let on_b = host.get_session(&sid).await.unwrap();
        assert_eq!(active(&on_b), pairs(&VERSION_B));
        let question_b = find(&on_b, "Question B").id.clone();
        let back = host
            .switch_version(switch(&host, &sid, &question_a))
            .await
            .unwrap();
        assert_eq!(active(&back), with_local);
        let forth = host
            .switch_version(switch(&host, &sid, &question_b))
            .await
            .unwrap();
        assert_eq!(active(&forth), pairs(&VERSION_B));
    });
}

fn image_model() -> Model {
    let mut model = serde_json::to_value(test_model()).unwrap();
    model["id"] = json!("test/image");
    model["kind"] = json!("image");
    model["tools"] = json!(false);
    model["image"] = json!({"generations":true,"edits":true,"n":true,"sizes":["1024x1024"],"qualities":["low"],"perImagePrice":"0.02"});
    model["supportedEndpoints"] = json!(["/v1/images/generations", "/v1/images/edits"]);
    serde_json::from_value(model).unwrap()
}

#[test]
fn image_media_survives_restart_and_followup_on_both_apexes() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        for apex in ["ofox.ai", "ofox.io"] {
            let config = tempfile::tempdir().unwrap();
            let gateway = Gateway::start();
            let env = TestEnv::new(config.path(), &bundle, &[(apex, &gateway)]);
            env.catalog.lock().unwrap().push(image_model());
            let host = AssistantHost::new(env.clone());
            let sid = host.create_session(serde_json::from_value(json!({"modelId":"test/image","modelPicked":true})).unwrap()).await.unwrap().id;
            gateway.reply(Reply::Json(200,json!({"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="}]})));
            let (_, end) = turn(&host, &env, request(&sid,"Blue square",json!({"media":{"size":"1024x1024","quality":"low"}}))).await;
            assert_eq!(end.data["status"], "completed", "{:?}",end.data);
            assert_eq!(gateway.requests().len(),1);
            assert_eq!(gateway.requests()[0].path,"/v1/images/generations");
            let detail = host.get_session(&sid).await.unwrap();
            let media = detail.messages.iter().find(|m|m.role=="media").expect("media bubble");
            let projected = serde_json::to_value(media).unwrap();
            assert_eq!(projected["media"]["cost"], "0.02");
            assert!(media.native_entry_id.is_some());
            assert_eq!(detail.usage["mediaCostUsd"],"0.02");
            assert_eq!(detail.usage["estimatedCostUsd"],"0.02");
            assert_eq!(detail.usage["requests"][0]["kind"], "image");
            assert_eq!(detail.usage["requests"][0]["apex"], apex);
            assert_eq!(detail.usage["requests"][0]["httpStatus"], 200);
            assert_eq!(detail.usage["requests"][0]["priceSnapshot"]["perImagePrice"], "0.02");
            assert!(detail.usage["requests"][0]["billedCostUsd"].is_null());
            assert_eq!(detail.attachments.len(),1);
            assert!(detail.session.draft_attachment_ids.is_empty());
            assert_eq!(serde_json::to_value(&detail.attachments[0]).unwrap()["origin"],"generated");
            host.shutdown().await;
            // A crash after storing images but before the custom native entry
            // must repair local history without another provider request.
            let folder = session_dir(config.path(), &sid);
            let native = std::fs::read_to_string(folder.join("session.jsonl")).unwrap();
            let kept = native.lines().filter(|line|serde_json::from_str::<Value>(line).unwrap()["customType"]!="ofox-desktop-media").collect::<Vec<_>>().join("\n")+"\n";
            std::fs::write(folder.join("session.jsonl"),kept).unwrap();
            let mut record = saved(config.path(), &sid);
            let media_row=record["messages"].as_array_mut().unwrap().iter_mut().find(|m|m["role"]=="media").unwrap();
            media_row["nativeEntryId"]=Value::Null;
            std::fs::write(folder.join("session.json"),serde_json::to_vec(&record).unwrap()).unwrap();
            let host = AssistantHost::new(env.clone());
            let restored = host.get_session(&sid).await.unwrap();
            assert!(restored.messages.iter().find(|m|m.role=="media").unwrap().native_entry_id.is_some(),"repair media history locally");
            assert_eq!(gateway.requests().len(),1);
            assert_eq!(restored.usage["mediaCostUsd"],"0.02");
            host.update_session(serde_json::from_value(json!({"sessionId":sid,"modelId":MODEL})).unwrap()).await.unwrap();
            gateway.reply(Reply::Stream("I remember the generated blue square.".into()));
            let (_, end) = turn(&host,&env,request(&sid,"What did I generate?",json!({}))).await;
            assert_eq!(end.data["status"],"completed");
            assert!(gateway.requests()[1].text().contains("Blue square"));
            assert_eq!(host.get_session(&sid).await.unwrap().usage["mediaCostUsd"],"0.02");
            assert_no_secret_on_disk(config.path(), &[KEY,NEW_KEY]);
            host.shutdown().await;
        }
    });
}

#[test]
fn image_requests_retry_only_a_refused_key_and_never_replay_unknown_outcomes() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        for (replies, expected, count) in [
            (vec![Reply::Error(401,invalid_key()), Reply::Json(200,json!({"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="}]}))],"completed",2),
            (vec![Reply::Error(401,invalid_key()),Reply::Error(401,invalid_key())],"assistant_key_rejected",2),
            (vec![Reply::Drop],"assistant_media_outcome_unknown",1),
        ] {
            let config=tempfile::tempdir().unwrap(); let gateway=Gateway::start();
            let env=TestEnv::new(config.path(),&bundle,&[("ofox.ai",&gateway)]);
            env.catalog.lock().unwrap().push(image_model());
            let host=AssistantHost::new(env.clone());
            let sid=host.create_session(serde_json::from_value(json!({"modelId":"test/image"})).unwrap()).await.unwrap().id;
            for reply in replies {gateway.reply(reply);}
            let req=request(&sid,"Blue square",json!({"media":{}}));
            let repeat=req.clone();
            let (_,end)=turn(&host,&env,req).await;
            let detail=host.get_session(&sid).await.unwrap();
            if expected=="completed" {assert_eq!(end.data["status"],"completed");}
            else {assert_eq!(detail.messages.last().unwrap().error_code.as_deref(),Some(expected));}
            assert_eq!(gateway.requests().len(),count);
            let attempts = detail.usage["requests"].as_array().expect("attempt receipts");
            assert_eq!(attempts.len(), count);
            assert_eq!(attempts[0]["httpStatus"], if count == 2 {json!(401)} else {Value::Null});
            assert!(attempts.iter().all(|r|r["billedCostUsd"].is_null()));
            assert!(host.send(repeat).await.is_err());
            assert_eq!(gateway.requests().len(),count,"operation identity prevents re-submit");
            if count==2 {
                let calls=gateway.requests();
                assert_eq!(calls[0].authorization,format!("Bearer {KEY}"));
                assert_eq!(calls[1].authorization,format!("Bearer {NEW_KEY}"));
                assert_eq!(env.state.lock().unwrap().rejected.len(),1);
            }
            assert_no_secret_on_disk(config.path(),&[KEY,NEW_KEY]);
            host.shutdown().await;
        }
    });
}

#[test]
fn media_save_checks_file_and_owner_before_dialog_and_rechecks_after_it() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        use cc_switch_lib::assistant::{store::AssistantStore,media_protocol::SaveMediaRequest};
        let config=tempfile::tempdir().unwrap();let output=tempfile::tempdir().unwrap();let gateway=Gateway::start();
        let env=TestEnv::new(config.path(),&bundle,&[("ofox.ai",&gateway)]);let host=AssistantHost::new(env.clone());
        let sid=host.create_session(serde_json::from_value(json!({})).unwrap()).await.unwrap().id;
        let request=|aid:String|->SaveMediaRequest {serde_json::from_value(json!({"sessionId":sid,"attachmentId":aid,"expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap()};
        let opened=Arc::new(AtomicBool::new(false));let marker=opened.clone();
        assert!(host.save_media(request(types::id()),move |_name|{marker.store(true,Ordering::SeqCst);Ok(None)}).await.is_err());
        assert!(!opened.load(Ordering::SeqCst),"unavailable file must not open a save dialog");
        let aid=types::id();let bytes=b"\x89PNG\r\n\x1a\nfixture";
        let store=AssistantStore::for_owner(config.path(),&owner()).unwrap();
        store.mutate(&sid,|record| {record.attachments.push(serde_json::from_value(json!({"id":aid,"name":"result.png","mimeType":"image/png","sizeBytes":bytes.len(),"status":"ready","origin":"generated"})).unwrap());Ok(())}).unwrap();
        std::fs::write(store.attachment_dir(&sid).unwrap().join(&aid),bytes).unwrap();
        let dest=output.path().join("saved.png");let picked=dest.clone();
        assert_eq!(host.save_media(request(aid.clone()),move |_name|Ok(Some(picked))).await.unwrap(),Some(dest.clone()));assert_eq!(std::fs::read(&dest).unwrap(),bytes);
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777, 0o600); }
        let private=output.path().join("must-not-exist.png");let picked=private.clone();let auth=env.clone();
        assert!(host.save_media(request(aid),move |_name|{auth.set_identity(Err("assistant_sign_in_required".into()));Ok(Some(picked))}).await.is_err());
        assert!(!private.exists());assert!(gateway.requests().is_empty());host.shutdown().await;
    });
}

fn video_model() -> Model {
    let mut model = serde_json::to_value(test_model()).unwrap();
    model["id"] = json!("test/video");
    model["kind"] = json!("video");
    model["tools"] = json!(false);
    model["video"] = json!({"modes":["t2v","i2v"],"resolutions":["480p"],"defaultResolution":"480p","minDuration":4,"maxDuration":15,"supportsAudio":false,"aspectRatios":["16:9"],"fromPricePerSecond":"0.02"});
    model["supportedEndpoints"] = json!(["/v1/videos"]);
    serde_json::from_value(model).unwrap()
}
#[test]
fn video_quotes_are_keyless_and_bound_to_current_authority_on_both_apexes() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        for apex in ["ofox.ai","ofox.io"] {
            let config=tempfile::tempdir().unwrap();let gateway=Gateway::start();
            let env=TestEnv::new(config.path(),&bundle,&[(apex,&gateway)]);
            env.catalog.lock().unwrap().push(video_model());let host=AssistantHost::new(env.clone());
            let sid=host.create_session(serde_json::from_value(json!({"modelId":"test/video"})).unwrap()).await.unwrap().id;
            let request=json!({"sessionId":sid,"expectedOwner":owner(),"expectedGeneration":host.generation(),"params":{"duration":4,"resolution":"480p"}});
            let before=types::now();
            let quote=host.quote_media(serde_json::from_value(request.clone()).unwrap()).await.unwrap();
            assert_eq!(quote.estimated_cost_usd.as_deref(),Some("0.12"));
            assert!(quote.expires_at>=before+600_000);
            assert!(!env.state.lock().unwrap().key_minted);
            assert!(gateway.requests().is_empty());
            let mut wrong=request.clone();wrong["expectedOwner"]=json!("cd".repeat(32));
            assert_eq!(host.quote_media(serde_json::from_value(wrong).unwrap()).await.unwrap_err(),"assistant_identity_changed");
            host.cancel_all();
            assert_eq!(host.quote_media(serde_json::from_value(request).unwrap()).await.unwrap_err(),"assistant_identity_changed");
            host.shutdown().await;
        }
    });
}

async fn video_quote_for(host: &Arc<AssistantHost>, sid: &str) -> String {
    host.quote_media(serde_json::from_value(json!({"sessionId":sid,"expectedOwner":owner(),"expectedGeneration":host.generation(),"params":{"duration":4,"resolution":"480p"}})).unwrap()).await.unwrap().id
}
#[test]
fn video_submit_persists_before_post_and_releases_chat_only_after_job_is_saved() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        for apex in ["ofox.ai", "ofox.io"] {
            let config = tempfile::tempdir().unwrap();
            let gateway = Gateway::start();
            let env = TestEnv::new(config.path(), &bundle, &[(apex, &gateway)]);
            env.catalog.lock().unwrap().push(video_model());
            let host = AssistantHost::new(env.clone());
            let sid = host
                .create_session(serde_json::from_value(json!({"modelId":"test/video"})).unwrap())
                .await
                .unwrap()
                .id;
            let params = json!({"duration":4,"resolution":"480p"});
            assert_eq!(
                host.send(request(&sid, "Moving square", json!({"video":params})))
                    .await
                    .unwrap_err(),
                "assistant_media_quote_expired"
            );
            assert!(gateway.requests().is_empty());
            let quote = video_quote_for(&host, &sid).await;
            let job = types::id();
            let gate = Arc::new(AtomicBool::new(false));
            gateway.reply(Reply::Wait(
                gate.clone(),
                Box::new(Reply::Json(202, json!({"id":job,"status":"pending"}))),
            ));
            gateway.reply(Reply::Json(200, json!({"id":job,"status":"queued"})));
            let req = request(
                &sid,
                "Moving square",
                json!({"video":params,"quoteId":quote}),
            );
            let repeat = req.clone();
            let run = host.send(req).await.unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while gateway.requests().is_empty() {
                assert!(Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let disk = saved(config.path(), &sid);
            assert_eq!(disk["operations"][0]["media"]["status"], "submitting");
            assert_eq!(disk["messages"][1]["media"]["status"], "submitting");
            gate.store(true, Ordering::SeqCst);
            while host
                .get_session(&sid)
                .await
                .unwrap()
                .session
                .active_run
                .is_some()
            {
                assert!(Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let disk = saved(config.path(), &sid);
            assert_eq!(disk["operations"][0]["status"], "pendingMedia");
            assert_eq!(disk["messages"][1]["media"]["jobId"], job);
            assert_eq!(disk["messages"][1]["media"]["apex"], apex);
            assert_eq!(
                disk["messages"][1]["media"]["requestAttempts"][0]["httpStatus"],
                202
            );
            assert!(disk["messages"][1]["media"]["priceSnapshot"]["quotedTotalUsd"].is_string());
            assert!(disk["messages"][1]["nativeEntryId"].is_string());
            assert!(host.send(repeat).await.is_err());
            assert_eq!(
                gateway
                    .requests()
                    .iter()
                    .filter(|r| r.path == "/v1/videos")
                    .count(),
                1
            );
            assert!(env
                .events(&run.run_id)
                .iter()
                .any(|e| e.r#type == "run_end" && e.data["status"] == "queued"));
            host.shutdown().await;
            gateway.reply(Reply::Json(200, json!({"id":job,"status":"queued"})));
            let before = gateway.requests().len();
            let fresh = AssistantHost::new(env.clone());
            fresh.recover_all().await;
            let deadline = Instant::now() + Duration::from_secs(10);
            while gateway.requests().len() == before {
                assert!(Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(
                gateway
                    .requests()
                    .iter()
                    .filter(|r| r.path == "/v1/videos")
                    .count(),
                1,
                "recovery only polls a saved job"
            );
            assert_eq!(
                fresh
                    .get_session(&sid)
                    .await
                    .unwrap()
                    .messages
                    .iter()
                    .filter(|m| m.role == "user")
                    .count(),
                1
            );
            fresh.shutdown().await;
            assert_no_secret_on_disk(config.path(), &[KEY, NEW_KEY]);
        }
    });
}
#[test]
fn video_ambiguous_submission_is_never_replayed_after_restart() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
        env.catalog.lock().unwrap().push(video_model());
        let host = AssistantHost::new(env.clone());
        let sid = host
            .create_session(serde_json::from_value(json!({"modelId":"test/video"})).unwrap())
            .await
            .unwrap()
            .id;
        let quote = video_quote_for(&host, &sid).await;
        gateway.reply(Reply::Drop);
        let req = request(
            &sid,
            "Moving square",
            json!({"video":{"duration":4,"resolution":"480p"},"quoteId":quote}),
        );
        let repeat = req.clone();
        let (_, end) = turn(&host, &env, req).await;
        assert_eq!(end.data["status"], "error");
        assert_eq!(
            saved(config.path(), &sid)["messages"][1]["media"]["status"],
            "unknown"
        );
        host.shutdown().await;
        let fresh = AssistantHost::new(env.clone());
        fresh.recover_all().await;
        assert!(fresh.send(repeat).await.is_err());
        assert_eq!(gateway.requests().len(), 1);
        fresh.shutdown().await;
        assert_no_secret_on_disk(config.path(), &[KEY, NEW_KEY]);
    });
}

#[test]
fn video_submit_rechecks_price_and_binds_quote_to_its_conversation_and_parameters() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
        env.catalog.lock().unwrap().push(video_model());
        let host = AssistantHost::new(env.clone());
        let one = host
            .create_session(serde_json::from_value(json!({"modelId":"test/video"})).unwrap())
            .await
            .unwrap()
            .id;
        let two = host
            .create_session(serde_json::from_value(json!({"modelId":"test/video"})).unwrap())
            .await
            .unwrap()
            .id;
        let quote = video_quote_for(&host, &one).await;
        for (sid, duration) in [(&two, 4), (&one, 5)] {
            assert_eq!(
                host.send(request(
                    sid,
                    "Square",
                    json!({"video":{"duration":duration,"resolution":"480p"},"quoteId":quote})
                ))
                .await
                .unwrap_err(),
                "assistant_media_quote_expired"
            );
        }
        *env.video_price.lock().unwrap() = "0.04".into();
        assert_eq!(
            host.send(request(
                &one,
                "Square",
                json!({"video":{"duration":4,"resolution":"480p"},"quoteId":quote})
            ))
            .await
            .unwrap_err(),
            "assistant_media_quote_expired"
        );
        assert!(gateway.requests().is_empty());
        assert!(!env.state.lock().unwrap().key_minted);
        host.shutdown().await;
    });
}
#[test]
fn video_limits_allow_one_job_per_conversation_and_three_per_account() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        use wiremock::{
            matchers::{method, path, path_regex},
            Mock, MockServer, ResponseTemplate,
        };
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos"))
            .respond_with(|_: &wiremock::Request| {
                ResponseTemplate::new(202)
                    .set_body_json(json!({"id":types::id(),"status":"pending"}))
            })
            .expect(4)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("^/v1/videos/"))
            .respond_with(|req: &wiremock::Request| {
                ResponseTemplate::new(200).set_body_json(
                    json!({"id":req.url.path().rsplit('/').next(),"status":"queued"}),
                )
            })
            .mount(&server)
            .await;
        let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
        env.route("ofox.ai", server.uri());
        env.catalog.lock().unwrap().push(video_model());
        let host = AssistantHost::new(env.clone());
        let mut sessions = vec![];
        for i in 0..4 {
            let sid = host
                .create_session(serde_json::from_value(json!({"modelId":"test/video"})).unwrap())
                .await
                .unwrap()
                .id;
            let quote = video_quote_for(&host, &sid).await;
            let req = request(
                &sid,
                "Square",
                json!({"video":{"duration":4,"resolution":"480p"},"quoteId":quote}),
            );
            if i < 3 {
                let (_, end) = turn(&host, &env, req).await;
                assert_eq!(end.data["status"], "queued");
            } else {
                assert_eq!(
                    host.send(req).await.unwrap_err(),
                    "assistant_media_video_limit"
                );
            }
            sessions.push(sid);
        }
        let quote = video_quote_for(&host, &sessions[0]).await;
        assert_eq!(
            host.send(request(
                &sessions[0],
                "Another",
                json!({"video":{"duration":4,"resolution":"480p"},"quoteId":quote})
            ))
            .await
            .unwrap_err(),
            "assistant_media_video_limit"
        );
        // The provider has finished this job; a local download failure must not
        // reserve a generation slot forever, even while it remains recoverable.
        let store =
            cc_switch_lib::assistant::store::AssistantStore::for_owner(config.path(), &owner())
                .unwrap();
        store
            .mutate(&sessions[0], |record| {
                for message in &mut record.messages {
                    if let Some(media) = message.media.as_mut() {
                        media.status = "download_failed".into();
                        message.status = "download_failed".into();
                    }
                }
                Ok(())
            })
            .unwrap();
        let quote = video_quote_for(&host, &sessions[0]).await;
        let (_, end) = turn(
            &host,
            &env,
            request(
                &sessions[0],
                "After completed remote job",
                json!({"video":{"duration":4,"resolution":"480p"},"quoteId":quote}),
            ),
        )
        .await;
        assert_eq!(end.data["status"], "queued");
        host.shutdown().await;
    });
}

#[test]
fn video_cancel_is_remote_and_unsupported_cancellation_keeps_tracking() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        use wiremock::{matchers::{method,path},Mock,MockServer,ResponseTemplate};
        for supported in [false,true] {
            let root=tempfile::tempdir().unwrap();let gateway=Gateway::start();let server=MockServer::start().await;let job=types::id();
            Mock::given(method("POST")).and(path("/v1/videos")).respond_with(ResponseTemplate::new(202).set_body_json(json!({"id":job,"status":"pending"}))).expect(1).mount(&server).await;
            Mock::given(method("GET")).and(path(format!("/v1/videos/{job}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":job,"status":"queued"}))).mount(&server).await;
            let reply=if supported {ResponseTemplate::new(204)} else {ResponseTemplate::new(400).set_body_json(json!({"error":{"code":"cancel_not_supported"}}))};
            Mock::given(method("DELETE")).and(path(format!("/v1/videos/{job}"))).respond_with(reply).expect(1).mount(&server).await;
            let env=TestEnv::new(root.path(),&bundle,&[("ofox.ai",&gateway)]);env.route("ofox.ai",server.uri());env.catalog.lock().unwrap().push(video_model());
            let host=AssistantHost::new(env.clone());let sid=host.create_session(serde_json::from_value(json!({"modelId":"test/video"})).unwrap()).await.unwrap().id;
            let quote=video_quote_for(&host,&sid).await;
            turn(&host,&env,request(&sid,"square",json!({"video":{"duration":4,"resolution":"480p"},"quoteId":quote}))).await;
            let detail=host.get_session(&sid).await.unwrap();let mid=&detail.messages.iter().find(|m|m.role=="media").unwrap().id;
            let action=json!({"sessionId":sid,"messageId":mid,"expectedOwner":owner(),"expectedGeneration":host.generation()});
            let mut stale=action.clone();stale["expectedGeneration"]=json!(99999);
            assert_eq!(host.cancel_video(serde_json::from_value(stale).unwrap()).await.unwrap_err(),"assistant_identity_changed");
            let result=host.cancel_video(serde_json::from_value(action).unwrap()).await;
            if supported {result.unwrap();} else {assert_eq!(result.unwrap_err(),"assistant_media_cancel_not_supported");}
            let record=saved(root.path(),&sid);let media=record["messages"].as_array().unwrap().iter().find(|m|m["role"]=="media").unwrap();
            assert_eq!(media["media"]["status"],if supported {"cancelled"} else {"queued"});
            assert_eq!(media["media"]["jobId"],job);
            if supported {
                let events = env.events.lock().unwrap();
                let progress = events.iter().rev().find(|event| event.r#type == "media_progress").unwrap();
                assert_eq!(progress.data["owner"], owner());
                assert_eq!(progress.data["generation"], host.generation());
                assert_eq!(progress.data["apex"], "ofox.ai");
                assert_eq!(progress.data["messageId"], *mid);
            }
            host.shutdown().await;
        }
    });
}
#[test]
fn video_failed_download_preserves_job_cost_and_resumes_without_posting() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        use wiremock::{matchers::{method,path},Mock,MockServer,ResponseTemplate};
        let root=tempfile::tempdir().unwrap();let gateway=Gateway::start();let server=MockServer::start().await;let job=types::id();
        Mock::given(method("POST")).and(path("/v1/videos")).respond_with(ResponseTemplate::new(202).set_body_json(json!({"id":job,"status":"pending"}))).expect(1).mount(&server).await;
        Mock::given(method("GET")).and(path(format!("/v1/videos/{job}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":job,"status":"completed","usage":{"video_cost":"0.123"},"mirror_urls":["http://localhost/private"]}))).mount(&server).await;
        let env=TestEnv::new(root.path(),&bundle,&[("ofox.ai",&gateway)]);env.route("ofox.ai",server.uri());env.catalog.lock().unwrap().push(video_model());let host=AssistantHost::new(env.clone());
        let sid=host.create_session(serde_json::from_value(json!({"modelId":"test/video"})).unwrap()).await.unwrap().id;
        let quote=video_quote_for(&host,&sid).await;turn(&host,&env,request(&sid,"square",json!({"video":{"duration":4,"resolution":"480p"},"quoteId":quote}))).await;
        let deadline=Instant::now()+Duration::from_secs(10);
        let mid=loop {
            let record=saved(root.path(),&sid);let media=record["messages"].as_array().unwrap().iter().find(|m|m["role"]=="media").unwrap();
            if media["media"]["status"]=="download_failed" {
                assert_eq!(media["media"]["cost"],"0.123");assert_eq!(media["media"]["jobId"],job);
                assert_eq!(record["usage"]["mediaCostUsd"],"0.123");break media["id"].as_str().unwrap().to_owned();
            }
            assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let before=server.received_requests().await.unwrap().len();
        host.resume_video(serde_json::from_value(json!({"sessionId":sid,"messageId":mid,"expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap()).await.unwrap();
        while server.received_requests().await.unwrap().len()<=before {assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;}
        host.shutdown().await;
    });
}

#[test]
fn video_resume_tracks_only_the_current_owner_and_apex_and_pauses_after_a_day() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        use cc_switch_lib::assistant::store::AssistantStore;
        use wiremock::{matchers::{method,path_regex},Mock,MockServer,ResponseTemplate};
        let root=tempfile::tempdir().unwrap();let gateway=Gateway::start();let ai=MockServer::start().await;let io=MockServer::start().await;
        for server in [&ai,&io] {Mock::given(method("GET")).and(path_regex("^/v1/videos/")).respond_with(|req:&wiremock::Request|ResponseTemplate::new(200).set_body_json(json!({"id":req.url.path().rsplit('/').next(),"status":"queued"}))).mount(server).await;}
        let env=TestEnv::new(root.path(),&bundle,&[("ofox.ai",&gateway)]);env.route("ofox.ai",ai.uri());env.route("ofox.io",io.uri());env.catalog.lock().unwrap().push(video_model());
        let host=AssistantHost::new(env.clone());let store=AssistantStore::for_owner(root.path(),&owner()).unwrap();let mut expired=None;
        for (apex,started) in [("ofox.ai",types::now()),("ofox.io",types::now()),("ofox.ai",types::now()-86_400_001)] {
            let sid=host.create_session(serde_json::from_value(json!({"modelId":"test/video"})).unwrap()).await.unwrap().id;
            let run=types::id();let mid=types::id();let media=json!({"kind":"video","model":"test/video","params":{},"status":"queued","jobId":types::id(),"apex":apex,"submittedAt":started,"pollStartedAt":started});
            store.mutate(&sid,|record|{record.messages.push(serde_json::from_value(json!({"id":mid,"role":"media","content":"square","createdAt":1,"status":"queued","runId":run,"media":media})).unwrap());record.operations.push(serde_json::from_value(json!({"id":types::id(),"runId":run,"status":"pendingMedia","media":media})).unwrap());Ok(())}).unwrap();
            if started<types::now()-86_400_000 {expired=Some(sid);}
        }
        host.recover_all().await;
        let deadline=Instant::now()+Duration::from_secs(5);
        while ai.received_requests().await.unwrap().is_empty(){assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;}
        assert!(io.received_requests().await.unwrap().is_empty());
        while saved(root.path(),expired.as_ref().unwrap())["messages"][0]["media"]["status"]!="query_expired"{assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;}
        assert_eq!(ai.received_requests().await.unwrap().len(),1);
        // A persisted timeout must also update an already-open renderer.
        let expiry_event_deadline = Instant::now() + Duration::from_millis(500);
        loop {
            if env.events.lock().unwrap().iter().any(|event| event.r#type == "media_progress" && event.session_id == *expired.as_ref().unwrap() && event.data["status"] == "query_expired") { break; }
            assert!(Instant::now() < expiry_event_deadline, "query expiry did not reach the renderer");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let receipt=host.begin_logout();env.set_identity(Err("assistant_sign_in_required".into()));host.finish_logout(receipt);
        env.set_identity(Ok(Identity{owner:owner(),apex:"ofox.io".into()}));host.sign_in_completed();
        while io.received_requests().await.unwrap().is_empty(){assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;}
        assert_eq!(ai.received_requests().await.unwrap().len(),1);
        env.set_identity(Ok(Identity{owner:owner(),apex:"ofox.ai".into()}));host.cancel_all();
        while ai.received_requests().await.unwrap().len()<2{assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;}
        assert_eq!(io.received_requests().await.unwrap().len(),1);
        host.shutdown().await;
    });
}

#[test]
fn video_fields_cannot_be_silently_ignored_by_an_image_run() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        gateway.reply(Reply::Error(
            400,
            json!({"error":{"code":"invalid_request"}}),
        ));
        let env = TestEnv::new(root.path(), &bundle, &[("ofox.ai", &gateway)]);
        env.catalog.lock().unwrap().push(image_model());
        let host = AssistantHost::new(env);
        let sid = host
            .create_session(serde_json::from_value(json!({"modelId":"test/image"})).unwrap())
            .await
            .unwrap()
            .id;
        let result = host
            .send(request(
                &sid,
                "square",
                json!({"video":{"duration":4,"resolution":"480p"},"quoteId":types::id()}),
            ))
            .await;
        host.shutdown().await;
        assert_eq!(result.unwrap_err(), "assistant_media_invalid_params");
        assert!(gateway.requests().is_empty());
    });
}

#[test]
fn forking_a_native_reply_keeps_only_its_context_and_does_not_double_charge() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    for apex in ["ofox.ai", "ofox.io"] {
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &runtime_dir, &[(apex, &gateway)]);
        env.signed_in(apex);
        let host = AssistantHost::new(env.clone());
        runtime().block_on(async {
            let sid = create(&host).await;
            gateway.reply(Reply::Stream("First answer".into()));
            turn(&host, &env, request(&sid, "First question", json!({}))).await;
            gateway.reply(Reply::Stream("Later answer".into()));
            turn(&host, &env, request(&sid, "Later question", json!({}))).await;
            let source = host.get_session(&sid).await.unwrap();
            let bytes = native_of(config.path(), &sid);
            let total = host.usage_summary().await.unwrap();
            let req = json!({"sessionId":sid,"messageId":find(&source,"First answer").id,
                "expectedOwner":owner(),"expectedGeneration":host.generation(),"title":"Original (fork)"});
            let mut stale = req.clone(); stale["expectedGeneration"] = json!(host.generation()+1);
            assert_eq!(host.fork_session(serde_json::from_value(stale).unwrap()).await.unwrap_err(), "assistant_identity_changed");
            let fork = host.fork_session(serde_json::from_value(req).unwrap()).await.unwrap();
            assert_ne!(fork.session.id, sid);
            assert_eq!(active(&fork), pairs(&[("user","First question"),("assistant","First answer")]));
            assert_eq!(native_of(config.path(), &sid), bytes);
            assert_eq!(host.usage_summary().await.unwrap()["estimatedCostUsd"], total["estimatedCostUsd"]);
            gateway.reply(Reply::Stream("Fork answer".into()));
            turn(&host, &env, request(&fork.session.id, "Fork question", json!({}))).await;
            let continued = host.get_session(&fork.session.id).await.unwrap();
            let expected = decimal(&total["estimatedCostUsd"]) + decimal(&continued.usage["estimatedCostUsd"]) - decimal(&fork.usage["estimatedCostUsd"]);
            assert!((decimal(&host.usage_summary().await.unwrap()["estimatedCostUsd"]) - expected).abs() < 1e-12);
            let sent = gateway.requests().last().unwrap().text();
            assert!(sent.contains("First question"));
            assert!(!sent.contains("Later question"));
            assert_eq!(native_of(config.path(), &sid), bytes);
            gateway.reply(Reply::Hang("Still working".into()));
            let run = host.send(request(&sid,"One more turn",json!({}))).await.unwrap();
            let busy = json!({"sessionId":sid,"messageId":find(&source,"First answer").id,
                "expectedOwner":owner(),"expectedGeneration":host.generation(),"title":"busy fork"});
            assert_eq!(host.fork_session(serde_json::from_value(busy).unwrap()).await.unwrap_err(), "assistant_session_busy");
            host.stop(&sid,&run.run_id,&run.operation_id).await.unwrap();
            wait_for(&env,&run.run_id,"run_end").await;
            host.shutdown().await;
        });
    }
}

#[test]
fn manual_compaction_is_an_exclusive_billed_native_run_without_a_fake_turn() {
    let Some(bundle) = runtime_dir() else {
        return;
    };
    for apex in ["ofox.ai", "ofox.io"] {
        let root = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(root.path(), &bundle, &[(apex, &gateway)]);
        env.catalog.lock().unwrap()[0].context_window = 8192;
        env.catalog.lock().unwrap()[0].max_output_tokens = 128;
        let host = AssistantHost::new(env.clone());
        runtime().block_on(async {
            let sid=create(&host).await;
            for text in ["long first context ".repeat(400),"long second context ".repeat(400),"long third context ".repeat(400)] {
                gateway.reply(Reply::Stream("Original answer".into()));
                turn(&host,&env,request(&sid,&text,json!({}))).await;
            }
            host.update_session(serde_json::from_value(json!({"sessionId":sid,"draft":"Preserve this unsent draft"})).unwrap()).await.unwrap();
            let before=host.get_session(&sid).await.unwrap();
            assert!(serde_json::to_value(&before.session).unwrap()["contextUsage"]["tokens"].as_u64().unwrap()>0);
            let compact=json!({"sessionId":sid,"runId":types::id(),"operationId":types::id(),"expectedOwner":owner(),"expectedGeneration":host.generation()});
            let mut stale=compact.clone();stale["expectedGeneration"]=json!(host.generation()+1);
            assert_eq!(host.compact_session(serde_json::from_value(stale).unwrap()).await.unwrap_err(),"assistant_identity_changed");
            let compact_gate=gate();
            gateway.reply(Reply::Wait(compact_gate.clone(), Box::new(Reply::Stream("Requirements and unfinished work summary".into()))));
            let run=host.compact_session(serde_json::from_value(compact.clone()).unwrap()).await.unwrap();
            assert_eq!(host.fork_session(serde_json::from_value(json!({"sessionId":sid,"messageId":before.messages[0].id,"title":"busy","expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap()).await.unwrap_err(),"assistant_session_busy");
            assert_eq!(host.compact_session(serde_json::from_value(compact).unwrap()).await.unwrap_err(),"assistant_session_busy");
            let active=host.get_session(&sid).await.unwrap();
            let compacting=serde_json::to_value(active.session).unwrap()["compacting"]==true;
            compact_gate.store(true,Ordering::SeqCst);
            let end=wait_for(&env,&run.run_id,"run_end").await;
            assert_eq!(end.data["status"],"completed", "errors: {:?}", env.events.lock().unwrap().iter().filter(|e|e.run_id==run.run_id && e.r#type=="error").collect::<Vec<_>>());
            let after=host.get_session(&sid).await.unwrap();
            assert_eq!(after.messages.len(),before.messages.len());
            assert_eq!(after.session.draft,"Preserve this unsent draft");
            assert!(after.usage["estimatedCostUsd"].as_str().is_some(),"the completed compaction must retain a known estimate");
            assert!(after.usage["modelRequests"].as_u64().unwrap()>before.usage["modelRequests"].as_u64().unwrap());
            let context=serde_json::to_value(&after.session).unwrap()["contextUsage"].clone();
            assert_eq!(context["contextWindow"],8192);
            assert!(context["tokens"].is_null());
            assert!(String::from_utf8(native_of(root.path(),&sid)).unwrap().contains("\"type\":\"compaction\""));
            let saved=saved(root.path(),&sid);
            assert_eq!(saved["operations"].as_array().unwrap().last().unwrap()["status"],"completed");
            assert!(env.events.lock().unwrap().iter().any(|e| e.run_id==run.run_id&&e.r#type=="compaction_start"));
            let requests=gateway.requests().len();
            let refused=host.compact_session(serde_json::from_value(json!({"sessionId":sid,"runId":types::id(),"operationId":types::id(),"expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap()).await.unwrap();
            let failed=wait_for(&env,&refused.run_id,"run_end").await;
            assert_eq!(failed.data["status"],"error");
            assert_eq!(wait_for(&env,&refused.run_id,"error").await.data["code"],"assistant_compact_unavailable");
            assert_eq!(gateway.requests().len(),requests);
            assert_eq!(host.get_session(&sid).await.unwrap().usage["estimatedCostUsd"],after.usage["estimatedCostUsd"]);
            host.shutdown().await;
            assert!(compacting,"returning to an active compaction must expose its state");
        });
    }
}

// ── generation tools ─────────────────────────────────────────────────────

const PNG_1X1: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

fn png_bytes() -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(PNG_1X1)
        .unwrap()
}

fn generation_events(env: &TestEnv, run: &str) -> Vec<Value> {
    of_type(env, run, "generation")
}

#[test]
fn a_chat_model_generates_an_image_with_the_conversation_defaults_on_both_apexes() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        for apex in ["ofox.ai", "ofox.io"] {
            let config = tempfile::tempdir().unwrap();
            let gateway = Gateway::start();
            let env = TestEnv::new(config.path(), &bundle, &[(apex, &gateway)]);
            env.catalog.lock().unwrap().push(image_model());
            let host = AssistantHost::new(env.clone());
            let sid = create(&host).await;
            host.update_session(serde_json::from_value(json!({"sessionId":sid,
                "media":{"image":{"model":"test/image","size":"1024x1024","quality":"low"}}})).unwrap())
                .await
                .unwrap();
            gateway.reply(Reply::ToolCalls(vec![("generate_image", json!({"prompt":"a blue square"}))]));
            gateway.reply(Reply::Json(200, json!({"data":[{"b64_json":PNG_1X1}]})));
            gateway.reply(Reply::Stream("Here is your blue square.".into()));
            let (run, end) = turn(&host, &env, request(&sid, "Draw a blue square", json!({}))).await;
            assert_eq!(end.data["status"], "completed", "{:?} {:?} {:?}", of_type(&env, &run, "error"), of_type(&env, &run, "tool_end"), gateway.requests().iter().map(tool_results).collect::<Vec<_>>());
            let calls = gateway.requests();
            assert_eq!(calls.len(), 3);
            assert!(tool_names(&calls[0]).contains(&"generate_image".to_string()));
            assert_eq!(calls[1].path, "/v1/images/generations");
            assert_eq!(calls[1].authorization, format!("Bearer {KEY}"));
            assert_eq!(
                calls[1].body,
                json!({"model":"test/image","prompt":"a blue square","size":"1024x1024","quality":"low"})
            );
            let answer = calls[2].text();
            assert!(answer.contains("attachment id"), "{answer}");
            assert!(!answer.contains(&PNG_1X1[..24]), "the model never gets image bytes");
            let detail = host.get_session(&sid).await.unwrap();
            let reply = detail
                .messages
                .iter()
                .find(|m| m.role == "assistant" && m.content.contains("blue square"))
                .expect("the reply");
            assert_eq!(reply.generations.len(), 1);
            let made = &reply.generations[0];
            assert_eq!(made.status, "completed");
            assert_eq!(made.cost.as_deref(), Some("0.02"));
            assert_eq!(made.prompt.as_deref(), Some("a blue square"));
            assert_eq!(made.apex.as_deref(), Some(apex));
            assert!(made.tool_call_id.is_some());
            assert_eq!(made.outputs.len(), 1);
            assert_eq!(reply.attachments.len(), 1);
            assert_eq!(reply.attachments[0].origin.as_deref(), Some("generated"));
            assert_eq!(reply.attachments[0].source_message_id.as_deref(), Some(reply.id.as_str()));
            assert_eq!(detail.attachments.len(), 1);
            let usage = reply.usage.as_ref().expect("reply usage");
            assert_eq!(usage["mediaCostUsd"], "0.02");
            let lines = usage["requests"].as_array().unwrap();
            assert!(lines.iter().any(|l| l["kind"] == "image" && l["estimatedCostUsd"] == "0.02" && l["toolCallId"] == json!(made.tool_call_id)));
            assert_eq!(detail.usage["mediaCostUsd"], "0.02");
            let events = generation_events(&env, &run);
            assert!(events.iter().any(|e| e["generation"]["status"] == "completed" && e["attachments"].as_array().is_some_and(|a| a.len() == 1)));
            // Regenerate with another prompt from the result card: no chat
            // model, a second generation on the same reply.
            gateway.reply(Reply::Json(200, json!({"data":[{"b64_json":PNG_1X1}]})));
            let started = host
                .regenerate_media(serde_json::from_value(json!({
                    "sessionId": sid, "expectedOwner": owner(), "expectedGeneration": host.generation(),
                    "operationId": types::id(), "messageId": reply.id, "toolCallId": made.tool_call_id,
                    "prompt": "a blue square, larger", "model": "test/image",
                    "params": {"size": "1024x1024", "quality": "low"},
                })).unwrap())
                .await
                .unwrap();
            assert_eq!(started["status"], "started");
            let deadline = Instant::now() + Duration::from_secs(20);
            let again = loop {
                let detail = host.get_session(&sid).await.unwrap();
                let shown = detail.messages.into_iter().find(|m| m.id == reply.id).unwrap();
                if shown.generations.len() == 2 && shown.generations[1].status == "completed" && detail.session.active_run.is_none() {
                    break shown;
                }
                assert!(Instant::now() < deadline, "the regeneration did not finish: {:?}", shown.generations);
                tokio::time::sleep(Duration::from_millis(50)).await;
            };
            let calls = gateway.requests();
            assert_eq!(calls.len(), 4, "one more image request and no chat request");
            assert_eq!(calls[3].path, "/v1/images/generations");
            assert_eq!(calls[3].body["prompt"], "a blue square, larger");
            assert_eq!(again.attachments.len(), 2);
            assert_eq!(again.generations[1].prompt.as_deref(), Some("a blue square, larger"));
            let detail = host.get_session(&sid).await.unwrap();
            assert_eq!(detail.usage["mediaCostUsd"], "0.04");
            assert_eq!(again.usage.as_ref().unwrap()["mediaCostUsd"], "0.04");
            assert_no_secret_on_disk(config.path(), &[KEY, NEW_KEY]);
            host.shutdown().await;
        }
    });
}

#[test]
fn a_model_that_takes_no_tools_gets_no_generation_tools() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
        env.catalog.lock().unwrap().push(image_model());
        let mut plain = serde_json::to_value(test_model()).unwrap();
        plain["id"] = json!("test/no-tools");
        plain["tools"] = json!(false);
        env.catalog
            .lock()
            .unwrap()
            .push(serde_json::from_value(plain).unwrap());
        let host = AssistantHost::new(env.clone());
        let sid = host
            .create_session(serde_json::from_value(json!({"modelId":"test/no-tools"})).unwrap())
            .await
            .unwrap()
            .id;
        gateway.reply(Reply::Stream("Hello.".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Draw a cat", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        assert!(tool_names(&gateway.requests()[0]).is_empty());
        host.shutdown().await;
    });
}

fn vision_model() -> Model {
    let mut model = serde_json::to_value(test_model()).unwrap();
    model["supportsImages"] = json!(true);
    model["inputModalities"] = json!(["text", "image"]);
    serde_json::from_value(model).unwrap()
}

#[test]
fn a_video_asks_first_and_a_declined_one_is_never_submitted() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        use cc_switch_lib::assistant::host::local::ApprovalRequest;
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
        {
            let mut catalog = env.catalog.lock().unwrap();
            catalog.clear();
            catalog.push(vision_model());
            catalog.push(video_model());
        }
        let host = AssistantHost::new(env.clone());
        let sid = create(&host).await;
        let image = host
            .pick_attachments(&sid, {
                let path = picked(config.path(), "start.png", &png_bytes());
                move || Ok(vec![path])
            })
            .await
            .unwrap()
            .remove(0);
        let answer = |choice: &'static str| {
            let host = host.clone();
            let env = env.clone();
            let sid = sid.clone();
            async move {
                let run = host
                    .send(request(&sid, "Animate this", json!({"attachmentIds":[]})))
                    .await
                    .unwrap();
                let approval = wait_for(&env, &run.run_id, "approval").await;
                assert_eq!(approval.data["name"], "generate_video");
                assert_eq!(approval.data["category"], "generation");
                assert_eq!(
                    approval.data["details"]["estimatedCostUsd"], "0.15",
                    "5 s at the 480p rate"
                );
                let respond: ApprovalRequest = serde_json::from_value(json!({
                    "sessionId": sid, "runId": run.run_id, "operationId": run.operation_id,
                    "approvalId": approval.data["approvalId"], "choice": choice,
                    "expectedOwner": owner(), "expectedGeneration": host.generation()}))
                .unwrap();
                host.respond_approval(respond).await.unwrap();
                wait_for(&env, &run.run_id, "run_end").await
            }
        };
        let call = json!({"prompt":"the square spins","duration":5,"material_ids":[image.id]});
        gateway.reply(Reply::ToolCalls(vec![("generate_video", call.clone())]));
        gateway.reply(Reply::Stream("Okay, I will not make it.".into()));
        let end = answer("deny").await;
        assert_eq!(end.data["status"], "completed");
        let calls = gateway.requests();
        assert!(
            calls.iter().all(|r| r.path != "/v1/videos"),
            "declined: nothing submitted"
        );
        assert!(calls[1].text().contains("assistant_media_declined"));

        gateway.reply(Reply::ToolCalls(vec![("generate_video", call)]));
        let job = types::id();
        gateway.reply(Reply::Json(202, json!({"id":job,"status":"pending"})));
        gateway.reply(Reply::Stream("Your video is on its way.".into()));
        let end = answer("once").await;
        assert_eq!(
            end.data["status"],
            "completed",
            "{:?} {:?}",
            env.events(&end.run_id)
                .iter()
                .filter(|e| e.r#type == "error")
                .map(|e| e.data.clone())
                .collect::<Vec<_>>(),
            gateway
                .requests()
                .iter()
                .map(|r| (r.path.clone(), tool_results(r)))
                .collect::<Vec<_>>()
        );
        let calls = gateway.requests();
        let submitted = calls
            .iter()
            .find(|r| r.path == "/v1/videos")
            .expect("submitted");
        assert_eq!(submitted.body["duration"], 5);
        assert_eq!(
            submitted.body["frame_images"][0]["frame_type"],
            "first_frame"
        );
        assert!(submitted.body["frame_images"][0]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(submitted.body.get("input_references").is_none());
        let detail = host.get_session(&sid).await.unwrap();
        let reply = detail
            .messages
            .iter()
            .find(|m| m.content.contains("on its way"))
            .unwrap();
        let video = &reply.generations[0];
        assert_eq!(video.kind, "video");
        assert_eq!(video.job_id.as_deref(), Some(job.as_str()));
        assert!(
            matches!(video.status.as_str(), "queued" | "in_progress"),
            "{}",
            video.status
        );
        assert_eq!(video.input_ids, vec![image.id.clone()]);
        assert_eq!(
            reply.status, "completed",
            "the reply's own status stays its chat run's"
        );
        // A video made again from its card answers with the estimate first,
        // and is never submitted without it.
        let regenerate =
            |confirmed: Value| -> cc_switch_lib::assistant::media_tools::RegenerateRequest {
                serde_json::from_value(json!({
                "sessionId": sid, "expectedOwner": owner(), "expectedGeneration": host.generation(),
                "operationId": types::id(), "messageId": reply.id, "toolCallId": video.tool_call_id,
                "prompt": "the square spins faster", "model": "test/video",
                "params": {"duration": 4}, "confirmedCostUsd": confirmed,
            })).unwrap()
            };
        let quoted = host
            .regenerate_media(regenerate(Value::Null))
            .await
            .unwrap();
        assert_eq!(quoted["needsConfirmation"], true);
        assert_eq!(quoted["estimatedCostUsd"], "0.12");
        assert_eq!(
            gateway
                .requests()
                .iter()
                .filter(|r| r.path == "/v1/videos")
                .count(),
            1,
            "nothing submitted before the estimate is confirmed"
        );
        host.shutdown().await;
    });
}

fn wav(seconds: u32) -> Vec<u8> {
    let size = 8000 * 2 * seconds;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + size).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&8000u32.to_le_bytes());
    b.extend_from_slice(&16000u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&size.to_le_bytes());
    b.resize(44 + size as usize, 0);
    b
}

#[test]
fn a_chat_model_reads_text_aloud_into_a_file_on_its_reply() {
    let Some(bundle) = runtime_dir() else { return };
    runtime().block_on(async {
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &bundle, &[("ofox.io", &gateway)]);
        let speech = parse_models(
            json!({"data":[{"id":"elevenlabs/eleven_v4","supported_endpoints":["/v1/audio/speech"],"pricing":{"input_text_per_character":"0.0002"}}]}),
            Source::V1,
        )
        .remove(0);
        env.catalog.lock().unwrap().push(speech);
        let host = AssistantHost::new(env.clone());
        let sid = create(&host).await;
        host.update_session(serde_json::from_value(json!({"sessionId":sid,
            "media":{"speech":{"model":"elevenlabs/eleven_v4","voice":"21m00Tcm4TlvDq8ikWAM","format":"wav"}}})).unwrap())
            .await
            .unwrap();
        gateway.reply(Reply::ToolCalls(vec![("generate_speech", json!({"text":"Welcome in"}))]));
        gateway.reply(Reply::Bytes("audio/wav", wav(1)));
        gateway.reply(Reply::Stream("Here is the voice-over.".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Read: Welcome in", json!({}))).await;
        assert_eq!(end.data["status"], "completed", "{:?}", gateway.requests().iter().map(tool_results).collect::<Vec<_>>());
        let calls = gateway.requests();
        assert_eq!(calls[1].path, "/v1/audio/speech");
        assert_eq!(calls[1].body["input"], "Welcome in");
        assert_eq!(calls[1].body["voice"], "21m00Tcm4TlvDq8ikWAM");
        assert_eq!(calls[1].body["response_format"], "wav");
        let detail = host.get_session(&sid).await.unwrap();
        let reply = detail.messages.iter().find(|m| m.content.contains("voice-over")).unwrap();
        let spoken = &reply.generations[0];
        assert_eq!(spoken.kind, "speech");
        assert_eq!(spoken.status, "completed");
        assert_eq!(spoken.cost.as_deref(), Some("0.002"), "10 characters at the catalog rate");
        assert_eq!(reply.attachments.len(), 1);
        assert_eq!(reply.attachments[0].mime_type, "audio/wav");
        assert_eq!(reply.attachments[0].source_message_id.as_deref(), Some(reply.id.as_str()));
        assert_eq!(spoken.outputs, vec![reply.attachments[0].id.clone()]);
        host.shutdown().await;
    });
}

#[test]
fn retrying_events_and_actual_ui_language_reach_the_real_host_and_model() {
    let Some(runtime_dir) = runtime_dir() else {
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &runtime_dir, &[("ofox.ai", &gateway)]);
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        for language in ["en", "ja", "zh"] {
            gateway.reply(Reply::Error(
                503,
                json!({"error":{"message":format!("busy {KEY}")}}),
            ));
            gateway.reply(Reply::Stream("One answer".into()));
            let (run, end) = turn(
                &host,
                &env,
                request(&sid, "Hello", json!({"uiLanguage":language})),
            )
            .await;
            assert_eq!(end.data["status"], "completed");
            assert_eq!(of_type(&env, &run, "retrying"), vec![json!({"attempt":1})]);
            assert!(system_preamble(gateway.requests().last().unwrap())
                .contains(&format!("App language: {language}.")));
        }
        assert_eq!(gateway.requests().len(), 6);
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.usage["usageCoverage"], "incomplete");
        assert!(detail.usage["estimatedCostUsd"].is_null());
        assert_eq!(detail.usage["inputTokens"], 30);
        assert_eq!(detail.usage["outputTokens"], 15);
        assert_eq!(detail.usage["modelRequests"], 3);
        let requests = detail.usage["requests"].as_array().unwrap();
        assert_eq!(
            requests.iter().filter(|r| r["status"] == "unknown").count(),
            3
        );
        for reply in detail.messages.iter().filter(|m| m.role == "assistant") {
            let usage = reply.usage.as_ref().unwrap();
            assert_eq!(usage["usageCoverage"], "incomplete");
            assert!(usage["estimatedCostUsd"].is_null());
            assert_eq!(usage["inputTokens"], 10);
            assert_eq!(usage["outputTokens"], 5);
        }
    });
    assert_no_secret_on_disk(config.path(), &[KEY]);
    assert_no_secret_in_events(&env, &[KEY]);
}

#[test]
#[serial_test::serial]
fn requests_pass_assistant_skills_without_broken_ones() {
    let bundle = runtime_dir().expect("run pnpm assistant:prepare before skill integration tests");
    let _sandbox = cc_switch_lib::test_sandbox::sandbox();
    let root = cc_switch_lib::assistant_target::skills_dir();
    for (name, body) in [
        (
            "drawing",
            "---\nname: drawing\ndescription: Draw safely\n---\nskill-body-marker",
        ),
        ("broken", "no frontmatter"),
    ] {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("SKILL.md"), body).unwrap();
    }
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
    *env.skill_db.lock().unwrap() = Some(Arc::new(cc_switch_lib::Database::memory().unwrap()));
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::ToolCalls(vec![(
            "use_skill",
            json!({"name":"drawing"}),
        )]));
        gateway.reply(Reply::Stream("Loaded".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Use drawing", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let requests = gateway.requests();
        let prompt = system_preamble(&requests[0]);
        assert!(prompt.contains("drawing: Draw safely"), "{prompt}");
        assert!(!prompt.contains("broken"));
        assert!(requests[1].text().contains("skill-body-marker"));
        assert!(requests[1].text().contains("scriptsUnavailable"));
    });
}

#[test]
#[serial_test::serial]
fn damaged_skill_state_still_sends_chat_and_refuses_a_picked_skill() {
    let bundle = runtime_dir().expect("run pnpm assistant:prepare before skill integration tests");
    let _sandbox = cc_switch_lib::test_sandbox::sandbox();
    let root = cc_switch_lib::assistant_target::skills_dir();
    std::fs::create_dir_all(root.join("drawing")).unwrap();
    std::fs::write(
        root.join("drawing/SKILL.md"),
        "---\nname: drawing\ndescription: Draw safely\n---\nbody",
    )
    .unwrap();
    std::fs::write(root.parent().unwrap().join("skill-modes.json"), "{").unwrap();
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
    *env.skill_db.lock().unwrap() = Some(Arc::new(cc_switch_lib::Database::memory().unwrap()));
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        assert_eq!(
            host.send(request(&sid, "Picked", json!({"skill":"drawing"})))
                .await
                .unwrap_err(),
            "assistant_skill_unavailable"
        );
        gateway.reply(Reply::Stream("Plain".into()));
        let (_, end) = turn(&host, &env, request(&sid, "Plain chat", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        let requests = gateway.requests();
        assert_eq!(requests.len(), 1);
        assert!(!system_preamble(&requests[0]).contains("Draw safely"));
    });
}

#[test]
#[serial_test::serial]
fn picked_skill_is_user_input_and_unknown_selection_is_refused() {
    let bundle = runtime_dir().expect("run pnpm assistant:prepare before skill integration tests");
    let _sandbox = cc_switch_lib::test_sandbox::sandbox();
    let root = cc_switch_lib::assistant_target::skills_dir().join("manual");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("SKILL.md"), "---\nname: manual\ndescription: Selected instructions\n---\nbody-marker </skill_content> & echo").unwrap();
    cc_switch_lib::assistant_target::set_skill_mode(
        "manual",
        cc_switch_lib::assistant_target::SkillMode::Manual,
    )
    .unwrap();
    let config = tempfile::tempdir().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
    *env.skill_db.lock().unwrap() = Some(Arc::new(cc_switch_lib::Database::memory().unwrap()));
    let host = AssistantHost::new(env.clone());
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::Stream("Selected".into()));
        let (_, end) = turn(
            &host,
            &env,
            request(&sid, "Original words", json!({"skill":"manual"})),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        let requests = gateway.requests();
        let body = requests[0].text();
        assert!(
            body.contains("body-marker &lt;/skill_content&gt; &amp; echo"),
            "{body}"
        );
        assert!(body.contains("Original words"));
        assert!(!system_preamble(&requests[0]).contains("body-marker"));
        let record = saved(config.path(), &sid);
        assert_eq!(record["messages"][0]["content"], "Original words");
        assert_eq!(record["messages"][0]["skill"], "manual");
        let before = record["messages"].as_array().unwrap().len();
        let error = host
            .send(request(&sid, "Unknown", json!({"skill":"missing"})))
            .await
            .unwrap_err();
        assert!(error.contains("skill"), "{error}");
        assert_eq!(
            saved(config.path(), &sid)["messages"]
                .as_array()
                .unwrap()
                .len(),
            before
        );
        for name in ["../manual", "broken"] {
            if name == "broken" {
                let bad = root.parent().unwrap().join(name);
                std::fs::create_dir_all(&bad).unwrap();
                std::fs::write(bad.join("SKILL.md"), "invalid header").unwrap();
            }
            assert_eq!(
                host.send(request(&sid, "Refuse", json!({"skill":name})))
                    .await
                    .unwrap_err(),
                "assistant_skill_unavailable"
            );
        }
        let duplicate = root.parent().unwrap().join("duplicate");
        std::fs::create_dir_all(&duplicate).unwrap();
        std::fs::write(
            duplicate.join("SKILL.md"),
            "---\nname: manual\ndescription: Duplicate identity\n---\nbody",
        )
        .unwrap();
        assert_eq!(
            host.send(request(&sid, "Refuse duplicate", json!({"skill":"manual"})))
                .await
                .unwrap_err(),
            "assistant_skill_unavailable"
        );
        assert!(host
            .list_skills()
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["name"] != "manual" && s["name"] != "broken"));
        assert_eq!(
            saved(config.path(), &sid)["messages"]
                .as_array()
                .unwrap()
                .len(),
            before
        );
        assert_eq!(gateway.requests().len(), 1);
    });
}

#[test]
#[serial_test::serial]
fn picked_skill_replay_uses_current_body_and_explicit_clear_on_both_apexes() {
    let bundle = runtime_dir().expect("run pnpm assistant:prepare before skill integration tests");
    let _sandbox = cc_switch_lib::test_sandbox::sandbox();
    let root = cc_switch_lib::assistant_target::skills_dir().join("replay");
    std::fs::create_dir_all(&root).unwrap();
    cc_switch_lib::assistant_target::set_skill_mode(
        "replay",
        cc_switch_lib::assistant_target::SkillMode::Manual,
    )
    .unwrap();
    for apex in ["ofox.ai", "ofox.io"] {
        let write_body = |body: &str| {
            std::fs::write(
                root.join("SKILL.md"),
                format!("---\nname: replay\ndescription: Replay instructions\n---\n{body}"),
            )
            .unwrap()
        };
        write_body("version-one");
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &bundle, &[(apex, &gateway)]);
        *env.skill_db.lock().unwrap() = Some(Arc::new(cc_switch_lib::Database::memory().unwrap()));
        let host = AssistantHost::new(env.clone());
        runtime().block_on(async {
            let sid = create(&host).await;
            let list = host.list_skills().await.unwrap();
            assert!(list
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"] == "replay" && s["origin"] == "local"));
            gateway.reply(Reply::Stream("First".into()));
            turn(
                &host,
                &env,
                request(&sid, "Only original words", json!({"skill":"replay"})),
            )
            .await;
            let first = host.get_session(&sid).await.unwrap();
            let user = first.messages.iter().find(|m| m.role == "user").unwrap();
            let reply = first
                .messages
                .iter()
                .find(|m| m.role == "assistant")
                .unwrap();
            assert_eq!(user.content, "Only original words");
            assert_eq!(user.skill.as_deref(), Some("replay"));
            write_body("version-two");
            gateway.reply(Reply::Stream("Regenerated".into()));
            turn(
                &host,
                &env,
                request(&sid, "", json!({"regenerateMessageId":reply.id})),
            )
            .await;
            let calls = gateway.requests();
            assert!(calls[1].text().contains("version-two"));
            assert!(!calls[1].text().contains("version-one"));
            let current = host.get_session(&sid).await.unwrap();
            let regenerated_user = current
                .messages
                .iter()
                .find(|m| m.role == "user" && !m.superseded)
                .unwrap();
            assert_eq!(regenerated_user.content, "Only original words");
            assert_eq!(regenerated_user.skill.as_deref(), Some("replay"));
            gateway.reply(Reply::Stream("Cleared".into()));
            turn(
                &host,
                &env,
                request(
                    &sid,
                    "Edited plain words",
                    json!({"editMessageId":regenerated_user.id,"replaceSkill":true}),
                ),
            )
            .await;
            let calls = gateway.requests();
            assert!(!calls[2].text().contains("skill_content"));
            assert!(calls[2].text().contains("Edited plain words"));
            let current = host.get_session(&sid).await.unwrap();
            assert!(current
                .messages
                .iter()
                .find(|m| m.role == "user" && !m.superseded)
                .unwrap()
                .skill
                .is_none());
            write_body("version-three-failed");
            gateway.reply(Reply::Error(400, "synthetic refusal".into()));
            let (failed_run, failed_end) = turn(
                &host,
                &env,
                request(&sid, "Retry original", json!({"skill":"replay"})),
            )
            .await;
            assert_eq!(failed_end.data["status"], "error");
            let failed_detail = host.get_session(&sid).await.unwrap();
            let failed_reply = failed_detail
                .messages
                .iter()
                .find(|m| m.role == "assistant" && m.run_id.as_deref() == Some(&failed_run))
                .unwrap();
            write_body("version-four-retry");
            gateway.reply(Reply::Stream("Recovered".into()));
            turn(
                &host,
                &env,
                request(
                    &sid,
                    "Retry original",
                    json!({"retryMessageId":failed_reply.id}),
                ),
            )
            .await;
            let last = gateway.requests().last().unwrap().text();
            assert!(last.contains("version-four-retry"));
            assert!(!last.contains("version-three-failed"));
            // An old history chip is not authority after the skill disappears.
            std::fs::remove_file(root.join("SKILL.md")).unwrap();
            let error = host
                .send(request(&sid, "", json!({"regenerateMessageId":reply.id})))
                .await
                .unwrap_err();
            assert_eq!(error, "assistant_skill_unavailable");
            assert_eq!(gateway.requests().len(), 5);
        });
    }
}

#[test]
#[serial_test::serial]
fn picked_skill_manual_reference_grant_and_tools_false_guidance() {
    let bundle = runtime_dir().expect("run pnpm assistant:prepare before skill integration tests");
    let _sandbox = cc_switch_lib::test_sandbox::sandbox();
    let root = cc_switch_lib::assistant_target::skills_dir().join("manual-reference");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("SKILL.md"),
        "---\nname: manual-reference\ndescription: Read the reference\n---\nUse reference.txt",
    )
    .unwrap();
    std::fs::write(root.join("reference.txt"), "manual-reference-read-marker").unwrap();
    cc_switch_lib::assistant_target::set_skill_mode(
        "manual-reference",
        cc_switch_lib::assistant_target::SkillMode::Manual,
    )
    .unwrap();
    for tools in [true, false] {
        let config = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &bundle, &[("ofox.ai", &gateway)]);
        *env.skill_db.lock().unwrap() = Some(Arc::new(cc_switch_lib::Database::memory().unwrap()));
        env.catalog.lock().unwrap()[0].tools = tools;
        *env.local_paths.lock().unwrap() =
            Some(cc_switch_lib::assistant::tools::context::LocalPaths {
                home: home.path().into(),
                config_roots: vec![],
                shell_files: vec![],
                skill: std::fs::canonicalize(bundle.join("runtime/skills/local-config/SKILL.md"))
                    .unwrap(),
            });
        let host = AssistantHost::new(env.clone());
        runtime().block_on(async {
            let sid = host
                .create_session(
                    serde_json::from_value(
                        json!({"capabilities":[{"id":"local_config","version":1}]}),
                    )
                    .unwrap(),
                )
                .await
                .unwrap()
                .id;
            if tools {
                gateway.reply(Reply::ToolCalls(vec![(
                    "read",
                    json!({"path":root.join("reference.txt")}),
                )]));
            }
            gateway.reply(Reply::Stream("Read selected skill".into()));
            let (_, end) = turn(
                &host,
                &env,
                request(
                    &sid,
                    "Read its reference",
                    json!({"skill":"manual-reference"}),
                ),
            )
            .await;
            assert_eq!(end.data["status"], "completed");
            let calls = gateway.requests();
            if tools {
                assert!(
                    calls[1].text().contains("manual-reference-read-marker"),
                    "{}",
                    calls[1].text()
                );
            } else {
                assert!(calls[0]
                    .text()
                    .contains("This model cannot execute scripts or tools"));
                assert!(
                    calls[0].body["tools"].is_null()
                        || calls[0].body["tools"].as_array().unwrap().is_empty()
                );
            }
        });
    }
}

#[test]
#[serial_test::serial]
fn skill_tools_real_bridge_save_click_refresh_then_same_turn_use_on_both_apexes() {
    use cc_switch_lib::assistant::host::local::ApprovalRequest;
    let bundle =
        runtime_dir().expect("prepare Node/Pi bundle is required for skill tool bridge tests");
    for apex in ["ofox.ai", "ofox.io"] {
        let _sandbox = cc_switch_lib::test_sandbox::sandbox();
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), &bundle, &[(apex, &gateway)]);
        let db = Arc::new(cc_switch_lib::Database::memory().unwrap());
        *env.skill_db.lock().unwrap() = Some(db.clone());
        let host = AssistantHost::new(env.clone());
        runtime().block_on(async{
            let sid=create(&host).await;
            let prefix="same-turn-bridge-body-marker\n";let body=format!("{prefix}{}","😀".repeat(60000-prefix.chars().count()));let description="😀".repeat(500);
            gateway.reply(Reply::ToolCalls(vec![("save_skill",json!({"name":"bridge-fixture","description":description,"body":body}))]));
            gateway.reply(Reply::ToolCalls(vec![("use_skill",json!({"name":"bridge-fixture"}))]));gateway.reply(Reply::Stream("Saved and used in this turn".into()));
            let run=host.send(request(&sid,"Save the reusable fixture then use it",json!({}))).await.unwrap();let approval=wait_for(&env,&run.run_id,"approval").await;
            assert_eq!(approval.data["name"],"save_skill");assert_eq!(approval.data["allowSession"],false);assert_eq!(approval.data["details"]["description"].as_str().unwrap().chars().count(),500);assert_eq!(approval.data["details"]["body"].as_str().unwrap().chars().count(),60000);
            assert!(db.get_all_installed_skills().unwrap().is_empty());assert!(!cc_switch_lib::assistant_target::skills_dir().join("bridge-fixture").exists());
            let respond:ApprovalRequest=serde_json::from_value(json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"approvalId":approval.data["approvalId"],"choice":"once","expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap();host.respond_approval(respond).await.unwrap();
            let end=wait_for(&env,&run.run_id,"run_end").await;assert_eq!(end.data["status"],"completed","{end:?}");let calls=gateway.requests();assert_eq!(calls.len(),3);
            let tool=calls[1].body["messages"].as_array().unwrap().iter().find(|m|m["role"]=="tool").expect("save result reaches the next model turn");
            let result:Value=serde_json::from_str(tool["content"].as_str().unwrap()).unwrap();assert_eq!(result["saved"],true);assert_eq!(result["usable"],true);assert!(result.get("refresh").is_none(),"host-only refresh must not be model content");
            assert!(calls[2].text().contains(prefix.trim()),"same-turn use must return the saved body");assert!(calls[2].text().contains("scriptsUnavailable"));
            assert_eq!(db.get_all_installed_skills().unwrap().len(),1);assert_eq!(cc_switch_lib::assistant_target::list_assistant_skills(&db,&Default::default()).unwrap()[0].origin,Some(cc_switch_lib::assistant_target::SkillOrigin::AssistantCreated));
            let detail=host.get_session(&sid).await.unwrap();assert_eq!(detail.messages.last().unwrap().content,"Saved and used in this turn");
        });
    }
}

#[test]
#[serial_test::serial]
fn assistant_mcp_real_bridge_discovery_read_write_approval_and_persisted_steps() {
    use cc_switch_lib::assistant::host::local::{ApprovalRequest, PermissionRequest};
    use std::fs;
    let bundle = runtime_dir().expect("prepared Node/Pi is required for MCP bridge tests");
    let node = bundle
        .join("node")
        .join(if cfg!(windows) { "node.exe" } else { "node" });
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assistant-runtime/test/fixtures/mcp-fixture.mjs");
    for apex in ["ofox.ai", "ofox.io"] {
        for (mode, choice) in [
            ("default", "once"),
            ("autoEdit", "session"),
            ("default", "deny"),
            ("fullAuto", "once"),
        ] {
            let _sandbox = cc_switch_lib::test_sandbox::sandbox();
            let config = tempfile::tempdir().unwrap();
            let gateway = Gateway::start();
            let physical_config = config.path().canonicalize().unwrap();
            let env = TestEnv::new(&physical_config, &bundle, &[(apex, &gateway)]);
            let host = AssistantHost::new(env.clone());
            let record = config.path().join("mcp-calls.jsonl");
            fs::create_dir_all(config.path().join("assistant")).unwrap();
            fs::write(config.path().join("assistant/mcp.json"),json!({"mcpServers":{"fixture":{"command":node,"args":[fixture],"env":{"MCP_RECORD":record,"MCP_CHILD_FILE":config.path().join("child-pid")}}}}).to_string()).unwrap();
            runtime().block_on(async {
            let sid = create(&host).await;
            host.set_permission(serde_json::from_value::<PermissionRequest>(json!({"sessionId":sid,"mode":mode,"confirmedFullAccess":true,"expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap()).await.unwrap();
            gateway.reply(Reply::ToolCalls(vec![("tool_search",json!({"query":"fixture resources"}))]));
            gateway.reply(Reply::ToolCalls(vec![("list_mcp_resources",json!({}))]));
            gateway.reply(Reply::ToolCalls(vec![("read_mcp_resource",json!({"server":"fixture","uri":"fixture://read"}))]));
            gateway.reply(Reply::ToolCalls(vec![("mcp__fixture__read_fixture",json!({}))]));
            gateway.reply(Reply::ToolCalls(vec![("mcp__fixture__write_fixture",json!({}))]));
            if choice=="session" { gateway.reply(Reply::ToolCalls(vec![("mcp__fixture__write_fixture",json!({}))])); }
            gateway.reply(Reply::Stream("MCP turn finished".into()));
            let run = host.send(request(&sid,"Use the fixture",json!({}))).await.unwrap();
            if mode != "fullAuto" {
                let approval = wait_for(&env,&run.run_id,"approval").await;
                assert_eq!(approval.data["name"],"mcp__fixture__write_fixture");
                assert_eq!(approval.data["allowSession"],true);
                assert_eq!(fs::read_to_string(&record).unwrap().lines().count(),1,"read-only ran without a prompt");
                host.respond_approval(serde_json::from_value::<ApprovalRequest>(json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"approvalId":approval.data["approvalId"],"choice":choice,"expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap()).await.unwrap();
            }
            let end = wait_for(&env,&run.run_id,"run_end").await;
            assert_eq!(end.data["status"],"completed","{end:?}");
            let rows=fs::read_to_string(&record).unwrap();
            assert_eq!(rows.lines().count(),if choice=="deny" {1} else if choice=="session" {3} else {2});
            let detail=host.get_session(&sid).await.unwrap();
            let message=detail.messages.last().unwrap();
            assert_eq!(message.content,"MCP turn finished");
            assert_eq!(message.mcp_steps.len(),if choice=="session" {6} else {5});
            assert!(message.mcp_steps.iter().any(|s|s["tool"]=="mcp__fixture__read_fixture" && s["status"]=="completed"));
            assert!(!serde_json::to_string(&message.mcp_steps).unwrap().contains(record.to_str().unwrap()));
            assert_mcp_child_gone(&node, &config.path().join("child-pid"));
            assert_eq!(gateway.requests().len(),if choice=="session" {7} else {6});
            if choice=="session" {
                gateway.reply(Reply::ToolCalls(vec![("tool_search",json!({"query":"fixture"}))]));
                gateway.reply(Reply::ToolCalls(vec![("mcp__fixture__write_fixture",json!({}))]));
                gateway.reply(Reply::Stream("Next reply reused conversation grant".into()));
                let (next_id,next_end)=turn(&host,&env,request(&sid,"Write again in this conversation",json!({}))).await;
                assert_eq!(next_end.data["status"],"completed");
                assert!(!env.events(&next_id).iter().any(|event|event.r#type=="approval"),"conversation grant must survive a new run");
                assert_eq!(fs::read_to_string(&record).unwrap().lines().count(),4);
                assert_mcp_child_gone(&node,&config.path().join("child-pid"));
                for boundary in ["other_session","changed_config"] {
                    let target=if boundary=="other_session" {create(&host).await} else {
                        let file=config.path().join("assistant/mcp.json");
                        let mut changed:Value=serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
                        changed["mcpServers"]["fixture"]["env"]["MCP_REAUTH"]=json!("configuration-changed");
                        fs::write(&file,serde_json::to_vec(&changed).unwrap()).unwrap();
                        sid.clone()
                    };
                    gateway.reply(Reply::ToolCalls(vec![("tool_search",json!({"query":"fixture"}))]));
                    gateway.reply(Reply::ToolCalls(vec![("mcp__fixture__write_fixture",json!({}))]));
                    gateway.reply(Reply::Stream("Boundary required a new approval".into()));
                    let again=host.send(request(&target,boundary,json!({}))).await.unwrap();
                    let card=wait_for(&env,&again.run_id,"approval").await;
                    host.respond_approval(serde_json::from_value::<ApprovalRequest>(json!({"sessionId":target,"runId":again.run_id,"operationId":again.operation_id,"approvalId":card.data["approvalId"],"choice":"deny","expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap()).await.unwrap();
                    assert_eq!(wait_for(&env,&again.run_id,"run_end").await.data["status"],"completed");
                    assert_eq!(fs::read_to_string(&record).unwrap().lines().count(),4,"a different conversation/config must not inherit permission");
                    assert_mcp_child_gone(&node,&config.path().join("child-pid"));
                }
            }
        });
        }
    }
}

#[test]
#[serial_test::serial]
fn assistant_mcp_real_bridge_revalidates_approval_after_config_change_and_cancel() {
    use cc_switch_lib::assistant::host::local::ApprovalRequest;
    use std::fs;
    let bundle = runtime_dir().expect("prepared Node/Pi required for MCP stale/cancel bridge");
    let node = bundle
        .join("node")
        .join(if cfg!(windows) { "node.exe" } else { "node" });
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assistant-runtime/test/fixtures/mcp-fixture.mjs");
    for action in ["stale", "cancel", "unknown_cost", "leader_failure"] {
        let _sandbox = cc_switch_lib::test_sandbox::sandbox();
        let config = tempfile::tempdir().unwrap();
        let physical = config.path().canonicalize().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(&physical, &bundle, &[("ofox.io", &gateway)]);
        if action == "unknown_cost" {
            let mut model = serde_json::to_value(test_model()).unwrap();
            model["pricingKnown"] = json!(false);
            model["cost"] = json!({"input":null,"output":null,"cacheRead":null,"cacheWrite":null});
            *env.catalog.lock().unwrap() = vec![serde_json::from_value(model).unwrap()];
        }
        let host = AssistantHost::new(env.clone());
        let calls = physical.join("calls.jsonl");
        fs::create_dir_all(physical.join("assistant")).unwrap();
        let source = physical.join("assistant/mcp.json");
        fs::write(&source,json!({"mcpServers":{"fixture":{"command":node,"args":[fixture],"env":{"MCP_RECORD":calls,"MCP_CHILD_FILE":physical.join("child-pid"),"MCP_SCENARIO":if action=="leader_failure" {"fail"} else {"normal"}}}}}).to_string()).unwrap();
        runtime().block_on(async {
            let sid=create(&host).await;
            gateway.reply(Reply::ToolCalls(vec![("tool_search",json!({"query":"fixture"}))]));
            gateway.reply(Reply::ToolCalls(vec![("mcp__fixture__write_fixture",json!({}))]));
            gateway.reply(Reply::Stream("No external write completed".into()));
            let run=host.send(request(&sid,"Use fixture",json!({}))).await.unwrap();
            if action!="unknown_cost" && action!="leader_failure" {
                let approval=wait_for(&env,&run.run_id,"approval").await;
                if action=="cancel" { host.stop(&sid,&run.run_id,&run.operation_id).await.unwrap(); }
                else { fs::write(&source,"{\"mcpServers\":{}}").unwrap();host.respond_approval(serde_json::from_value::<ApprovalRequest>(json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"approvalId":approval.data["approvalId"],"choice":"once","expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap()).await.unwrap(); }
            }
            let end=wait_for(&env,&run.run_id,"run_end").await;
            if action=="cancel" {assert_eq!(end.data["status"],"stopped");}
            assert!(!calls.exists(),"{action} must prevent actual external execution");
            if action!="unknown_cost" { assert_mcp_child_gone(&node,&physical.join("child-pid")); }
            if action=="unknown_cost" { assert!(!env.events.lock().unwrap().iter().any(|e|e.run_id==run.run_id && e.r#type=="approval")); }
        });
    }
}

fn assert_mcp_child_gone(node: &Path, record: &Path) {
    let pid = std::fs::read_to_string(record).expect("fixture must really spawn a descendant");
    let pid: u32 = pid.parse().unwrap();
    assert!(pid > 1);
    let status=std::process::Command::new(node).args(["-e","try {process.kill(Number(process.argv[1]),0); process.exit(1)} catch(e) {process.exit(e.code==='ESRCH'?0:2)}",&pid.to_string()]).status().expect("prepared Node process probe");
    assert!(
        status.success(),
        "MCP descendant {pid} survived run teardown"
    );
}

#[test]
#[serial_test::serial]
fn assistant_mcp_missing_environment_isolated_to_one_server_in_real_bridge() {
    use std::fs;
    let bundle = runtime_dir().expect("prepared Node/Pi required for missing-environment bridge");
    let node = bundle
        .join("node")
        .join(if cfg!(windows) { "node.exe" } else { "node" });
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assistant-runtime/test/fixtures/mcp-fixture.mjs");
    let _sandbox = cc_switch_lib::test_sandbox::sandbox();
    let config = tempfile::tempdir().unwrap();
    let physical = config.path().canonicalize().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(&physical, &bundle, &[("ofox.io", &gateway)]);
    let host = AssistantHost::new(env.clone());
    let record = physical.join("calls.jsonl");
    let blocked = physical.join("must-not-start");
    assert!(std::env::var_os("OFOX_TASK7_MISSING_ENV_7A").is_none());
    fs::create_dir_all(physical.join("assistant")).unwrap();
    fs::write(physical.join("assistant/mcp.json"),json!({"mcpServers":{
        "fixture":{"command":node,"args":[fixture],"env":{"MCP_RECORD":record}},
        "missing":{"command":node,"args":[fixture],"env":{"TOKEN":"${OFOX_TASK7_MISSING_ENV_7A}","MCP_PID_FILE":blocked}}
    }}).to_string()).unwrap();
    runtime().block_on(async {
        let sid = create(&host).await;
        gateway.reply(Reply::ToolCalls(vec![(
            "tool_search",
            json!({"query":"fixture"}),
        )]));
        gateway.reply(Reply::ToolCalls(vec![(
            "mcp__fixture__read_fixture",
            json!({}),
        )]));
        gateway.reply(Reply::Stream("Healthy service completed".into()));
        let (_, end) = turn(
            &host,
            &env,
            request(&sid, "Read healthy service", json!({})),
        )
        .await;
        assert_eq!(end.data["status"], "completed");
        assert!(
            record.exists(),
            "one missing environment must not stop the healthy real MCP server"
        );
        assert_eq!(fs::read_to_string(&record).unwrap().lines().count(), 1);
        assert!(
            !blocked.exists(),
            "unresolved credentials must never reach a launched service"
        );
        assert!(!env
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.r#type == "approval"));
        let detail = host.get_session(&sid).await.unwrap();
        let steps = &detail.messages.last().unwrap().mcp_steps;
        assert!(steps
            .iter()
            .any(|s| s["server"] == "missing" && s["status"] == "unavailable"));
        assert!(steps.iter().all(|s| s.get("identity").is_none()));
        for call in gateway.requests() {
            assert!(!call.text().contains("OFOX_TASK7_MISSING_ENV_7A"));
            assert!(!call.text().contains("captured-config"));
        }
    });
}

#[test]
#[serial_test::serial]
fn assistant_mcp_no_local_twenty_tool_budget_includes_existing_skill_tools() {
    use std::fs;
    let bundle = runtime_dir().expect("prepared Node/Pi required for MCP budget regression");
    let node = bundle
        .join("node")
        .join(if cfg!(windows) { "node.exe" } else { "node" });
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assistant-runtime/test/fixtures/mcp-fixture.mjs");
    for local_capability in [false, true] {
        let _sandbox = cc_switch_lib::test_sandbox::sandbox();
        let skill = cc_switch_lib::assistant_target::skills_dir().join("budget-fixture");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"),"---\nname: budget-fixture\ndescription: Budget fixture\n---\nReadable skill instructions").unwrap();
        cc_switch_lib::assistant_target::set_skill_mode(
            "budget-fixture",
            cc_switch_lib::assistant_target::SkillMode::Auto,
        )
        .unwrap();
        let config = tempfile::tempdir().unwrap();
        let physical = config.path().canonicalize().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(&physical, &bundle, &[("ofox.io", &gateway)]);
        *env.skill_db.lock().unwrap() = Some(Arc::new(cc_switch_lib::Database::memory().unwrap()));
        let host = AssistantHost::new(env.clone());
        let calls = physical.join("calls");
        fs::create_dir_all(physical.join("assistant")).unwrap();
        fs::write(physical.join("assistant/mcp.json"),json!({"mcpServers":{"fixture":{"command":node,"args":[fixture],"env":{"MCP_RECORD":calls}}}}).to_string()).unwrap();
        runtime().block_on(async {
            let sid=host.create_session(serde_json::from_value(if local_capability {json!({"capabilities":[{"id":"local_config","version":1}]})}else{json!({})}).unwrap()).await.unwrap().id;
            gateway.reply(Reply::ToolCalls(vec![("tool_search",json!({"query":"fixture"}))]));
            for index in 2..=21 { gateway.reply(Reply::ToolCalls(vec![if index%5==0 {("use_skill",json!({"name":"budget-fixture"}))}else{("mcp__fixture__read_fixture",json!({"index":index}))}])); }
            gateway.reply(Reply::Stream("Budget complete".into()));
            let (run,_)=turn(&host,&env,request(&sid,"Use mixed tools",json!({}))).await;
            let events=env.events(&run);let starts:Vec<_>=events.iter().filter(|event|event.r#type=="tool_start").collect();
            assert_eq!(starts.len(),20,"actual Host no-local params must retain twenty tools, capability={local_capability}");
            assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(),15,"call21 never executes");
            assert_eq!(starts.iter().filter(|event|event.data["name"]=="use_skill").count(),4,"existing tool20 executes");
            assert_eq!(gateway.requests()[20].body["tool_choice"],"none");
            assert!(!gateway.requests()[0].body["tools"].as_array().unwrap().iter().any(|tool|tool["function"]["name"]=="read"));
        });
    }
}

#[test]
#[serial_test::serial]
fn final_mcp_approval_previews_show_distinct_safe_arguments_and_limits() {
    use cc_switch_lib::assistant::host::local::ApprovalRequest;
    use std::fs;
    let bundle = runtime_dir().expect("prepared Node/Pi required for actual MCP approval previews");
    let node = bundle
        .join("node")
        .join(if cfg!(windows) { "node.exe" } else { "node" });
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assistant-runtime/test/fixtures/mcp-fixture.mjs");
    let _sandbox = cc_switch_lib::test_sandbox::sandbox();
    let config = tempfile::tempdir().unwrap();
    let physical = config.path().canonicalize().unwrap();
    let gateway = Gateway::start();
    let env = TestEnv::new(&physical, &bundle, &[("ofox.io", &gateway)]);
    let host = AssistantHost::new(env.clone());
    fs::create_dir_all(physical.join("assistant")).unwrap();
    fs::write(physical.join("assistant/mcp.json"), json!({"mcpServers":{"fixture":{"command":node,"args":[fixture],"env":{"PIN":"418731","SHORT_TOKEN":"xy7","FLAG":"false"}}}}).to_string()).unwrap();
    runtime().block_on(async {
        let sid = create(&host).await;
        let variants = [json!({"target":"draft-inbox","body":"First readable body","nested":{"text":"xy7","pin":418731,"418731":"Readable key"}}), json!({"target":"published-inbox","body":"Second readable body"}),json!({}),json!({"target":"large-inbox","body":"😀".repeat(9000)})];
        let mut displays = vec![];
        for args in variants {
            gateway.reply(Reply::ToolCalls(vec![("tool_search",json!({"query":"fixture"}))]));
            gateway.reply(Reply::ToolCalls(vec![("mcp__fixture__write_fixture",args)]));
            gateway.reply(Reply::Stream("Preview checked".into()));
            let run = host.send(request(&sid,"Use fixture",json!({}))).await.unwrap();
            let approval = wait_for(&env,&run.run_id,"approval").await;
            host.respond_approval(serde_json::from_value::<ApprovalRequest>(json!({"sessionId":sid,"runId":run.run_id,"operationId":run.operation_id,"approvalId":approval.data["approvalId"],"choice":"deny","expectedOwner":owner(),"expectedGeneration":host.generation()})).unwrap()).await.unwrap();
            assert_eq!(wait_for(&env,&run.run_id,"run_end").await.data["status"],"completed");
            displays.push(approval.data.clone());
        }
        let previews:Vec<_> = displays.iter().map(|v|v["argumentsPreview"].as_str().expect("every new MCP approval supplies a safe preview")).collect();
        assert!(previews[0].contains("draft-inbox") && previews[0].contains("First readable body"));
        assert!(previews[1].contains("published-inbox") && previews[1].contains("Second readable body"));
        assert!(!previews[0].contains("418731") && !previews[0].contains("xy7"));
        assert!(previews[0].contains("Readable key") && previews[0].contains("***"));
        assert_eq!(previews[2],"{}");
        assert_eq!(previews[3].chars().count(),8000);
        assert_eq!(displays[3]["argumentsTruncated"],true);
        assert!(displays[..3].iter().all(|v|v["argumentsTruncated"]==false));
        let detail=host.get_session(&sid).await.unwrap();
        assert!(!serde_json::to_string(&detail).unwrap().contains("argumentsPreview"),"previews are ephemeral approval DTOs, not chat metadata");
    });
}

// ── documents ────────────────────────────────────────────────────────────

/// `create_document` end to end: the model reads the bundled skill, then the
/// real engines make the file and the host keeps it as a generated
/// attachment of the reply. `cargo test --test assistant_host documents`.
mod documents {
    use super::*;

    const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
    const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

    /// The prepared runtime with both document engines, or `None` (with a
    /// message) on a developer machine without them. In CI a missing engine
    /// is a failure.
    fn engines_runtime() -> Option<PathBuf> {
        let bundle = runtime_dir()?;
        let engines = cc_switch_lib::assistant::documents::engines::engines_at(&bundle);
        if engines.officecli.is_some() && engines.typst.is_some() {
            return Some(bundle);
        }
        assert!(
            std::env::var_os("CI").is_none(),
            "the document engines are not prepared at {}; CI must run `pnpm assistant:prepare` before cargo test",
            bundle.display()
        );
        eprintln!(
            "skipping: the document engines are not prepared at {} (run `pnpm assistant:prepare`)",
            bundle.display()
        );
        None
    }

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/assistant-documents")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn batch(name: &str) -> Value {
        serde_json::from_str(&fixture(name)).unwrap()
    }

    /// What one turn that read a skill and called `create_document` left.
    struct Outcome {
        config: tempfile::TempDir,
        sid: String,
        generations: Vec<Value>,
        tool_ends: Vec<Value>,
        requests: Vec<Request>,
        detail: Detail,
        /// The `create_document` result as the runtime keeps it in its history.
        result: Value,
    }

    impl Outcome {
        /// The `create_document` result's text as the model was sent it.
        fn sent(&self) -> String {
            let id = &self.result["toolCallId"];
            self.requests[1].body["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|m| m["role"] == "tool" && &m["tool_call_id"] == id)
                .map(|m| text_of(&m["content"]))
                .unwrap_or_else(|| {
                    panic!(
                        "no create_document result was sent: {:?}",
                        self.requests[1].body
                    )
                })
        }

        /// The images the model was sent after the tool calls.
        fn images_sent(&self) -> Vec<String> {
            self.requests[1].body["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m["content"].as_array())
                .flatten()
                .filter(|part| part["type"] == "image_url")
                .filter_map(|part| part["image_url"]["url"].as_str().map(str::to_owned))
                .collect()
        }

        fn stored(&self) -> PathBuf {
            session_dir(self.config.path(), &self.sid).join("attachments")
        }

        /// The one generated document: checked as every format's, and its
        /// generation record and bytes.
        fn made(&self, mime: &str) -> (Value, Vec<u8>) {
            let [event] = &self.generations[..] else {
                panic!("one generation event: {:?}", self.generations);
            };
            let generation = &event["generation"];
            assert_eq!(generation["kind"], "document");
            assert_eq!(generation["status"], "completed");
            assert_eq!(generation["cost"], "0", "made on this computer");
            let [attachment] = &self.detail.attachments[..] else {
                panic!("one attachment: {:?}", self.detail.attachments);
            };
            assert_eq!(attachment.origin.as_deref(), Some("generated"));
            assert_eq!(attachment.mime_type, mime);
            assert_eq!(event["attachments"][0]["id"], attachment.id.as_str());
            assert_eq!(event["attachments"][0]["origin"], "generated");
            assert_eq!(generation["outputs"], json!([attachment.id]));
            let reply = self
                .detail
                .messages
                .iter()
                .find(|m| m.role == "assistant" && !m.generations.is_empty())
                .expect("the reply keeps the document");
            assert_eq!(reply.generations[0].kind, "document");
            assert_eq!(reply.attachments.len(), 1);
            assert_eq!(reply.attachments[0].id, attachment.id);
            assert_ne!(self.result["isError"], true, "{}", self.result);
            let sent = self.sent();
            assert!(
                sent.contains(&format!("as attachment {}", attachment.id)),
                "{sent}"
            );
            let bytes = std::fs::read(self.stored().join(&attachment.id)).unwrap();
            assert_eq!(bytes.len() as u64, attachment.size_bytes);
            (generation.clone(), bytes)
        }

        /// A refused document: the model hears why, and nothing is kept.
        fn refused(&self) -> String {
            assert_eq!(self.result["isError"], true, "{}", self.result);
            let end = self
                .tool_ends
                .iter()
                .find(|e| e["name"] == "create_document")
                .expect("create_document ended");
            assert_eq!(end["success"], false);
            assert!(self.generations.is_empty(), "{:?}", self.generations);
            assert!(self.detail.attachments.is_empty());
            assert!(self
                .detail
                .messages
                .iter()
                .all(|m| m.attachments.is_empty() && m.generations.is_empty()));
            let kept = std::fs::read_dir(self.stored()).map_or(0, |dir| dir.count());
            assert_eq!(kept, 0, "no attachment file is kept");
            self.sent()
        }
    }

    /// One turn: the model reads `skill`, calls `create_document` with
    /// `args`, then answers.
    fn create_document(bundle: &Path, model: Model, skill: &str, args: Value) -> Outcome {
        let config = tempfile::tempdir().unwrap();
        let gateway = Gateway::start();
        let env = TestEnv::new(config.path(), bundle, &[("ofox.ai", &gateway)]);
        *env.catalog.lock().unwrap() = vec![model];
        let host = AssistantHost::new(env.clone());
        let (sid, run, end, detail) = runtime().block_on(async {
            let sid = create(&host).await;
            gateway.reply(Reply::ToolCalls(vec![
                ("use_skill", json!({"name": skill})),
                ("create_document", args),
            ]));
            gateway.reply(Reply::Stream("The document is ready.".into()));
            let (run, end) = turn(&host, &env, request(&sid, "Make the document", json!({}))).await;
            let detail = host.get_session(&sid).await.unwrap();
            host.shutdown().await;
            (sid, run, end, detail)
        });
        let requests = gateway.requests();
        assert_eq!(
            end.data["status"],
            "completed",
            "{:?} {:?}",
            of_type(&env, &run, "error"),
            requests.iter().map(tool_results).collect::<Vec<_>>()
        );
        assert_eq!(requests.len(), 2, "the tool calls, then the answer");
        assert!(tool_names(&requests[0]).contains(&"create_document".to_string()));
        let tool_ends = of_type(&env, &run, "tool_end");
        let skill_end = tool_ends
            .iter()
            .find(|e| e["name"] == "use_skill")
            .expect("use_skill ended");
        assert_eq!(skill_end["success"], true, "{skill_end}");
        // Every office skill opens with the call it teaches.
        assert!(
            tool_results(&requests[1])
                .iter()
                .any(|text| text.contains(skill) && text.contains("create_document({ name: ")),
            "the bundled skill reached the model"
        );
        let native =
            std::fs::read_to_string(session_dir(config.path(), &sid).join("session.jsonl"))
                .unwrap();
        let result = native
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|entry| entry["type"] == "message")
            .map(|entry| entry["message"].clone())
            .find(|m| m["role"] == "toolResult" && m["toolName"] == "create_document")
            .expect("the create_document result is in the history");
        assert_no_secret_in_events(&env, &[KEY, NEW_KEY]);
        Outcome {
            config,
            sid,
            generations: of_type(&env, &run, "generation"),
            tool_ends,
            requests,
            detail,
            result,
        }
    }

    #[test]
    fn creates_a_pptx_from_a_batch() {
        let Some(bundle) = engines_runtime() else {
            return;
        };
        let deck = batch("deck.json");
        let outcome = create_document(
            &bundle,
            test_model(),
            "ofox-office-pptx",
            json!({"name": "deck.pptx", "format": "pptx", "content": deck}),
        );
        let (generation, bytes) = outcome.made(PPTX);
        assert_eq!(generation["params"]["format"], "pptx");
        assert_eq!(generation["params"]["slides"], 8);
        assert_eq!(outcome.result["details"]["file"]["slides"], 8);
        assert!(bytes.starts_with(b"PK"));
        let sent = outcome.sent();
        assert!(sent.contains("Created deck.pptx (pptx, "), "{sent}");
        assert!(
            sent.contains("8 slides") && sent.contains("attachment"),
            "{sent}"
        );
        assert!(
            outcome.images_sent().is_empty(),
            "a text model gets no preview"
        );
        assert_no_secret_on_disk(outcome.config.path(), &[KEY, NEW_KEY]);
    }

    #[test]
    fn creates_a_pdf_and_reports_pages() {
        let Some(bundle) = engines_runtime() else {
            return;
        };
        let outcome = create_document(
            &bundle,
            test_model(),
            "ofox-office-pdf",
            json!({"name": "essay.pdf", "format": "pdf", "content": fixture("essay.typ")}),
        );
        let (generation, bytes) = outcome.made("application/pdf");
        assert!(bytes.starts_with(b"%PDF"));
        let pages = outcome.result["details"]["file"]["pages"]
            .as_u64()
            .expect("the runtime counts the pages");
        assert!(pages >= 1);
        assert_eq!(
            generation["params"]["pages"], pages,
            "host and runtime agree"
        );
        let sent = outcome.sent();
        assert!(sent.contains(&format!(" {pages} page")), "{sent}");
        assert!(
            outcome.images_sent().is_empty(),
            "a text model gets no preview"
        );
        assert_no_secret_on_disk(outcome.config.path(), &[KEY, NEW_KEY]);
    }

    #[test]
    fn creates_a_pdf_and_shows_a_vision_model_its_first_page() {
        let Some(bundle) = engines_runtime() else {
            return;
        };
        let outcome = create_document(
            &bundle,
            vision_model(),
            "ofox-office-pdf",
            json!({"name": "essay.pdf", "format": "pdf", "content": fixture("essay.typ")}),
        );
        let (_, bytes) = outcome.made("application/pdf");
        assert!(bytes.starts_with(b"%PDF"));
        let images = outcome.images_sent();
        assert_eq!(images.len(), 1, "the first page's thumbnail");
        assert!(
            images[0].starts_with("data:image/"),
            "{}",
            &images[0][..40.min(images[0].len())]
        );
        let pixels = outcome.result["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["type"] == "image")
            .count();
        assert_eq!(pixels, 1, "the history keeps the thumbnail with the result");
        assert_no_secret_on_disk(outcome.config.path(), &[KEY, NEW_KEY]);
    }

    #[test]
    fn creates_an_xlsx() {
        let Some(bundle) = engines_runtime() else {
            return;
        };
        let cell = |at: &str, value: &str| json!({"command":"set","path":format!("/Sheet1/{at}"),"props":{"value":value}});
        let table = json!([
            cell("A1", "Item"), cell("B1", "Q1"), cell("C1", "Q2"),
            cell("A2", "Apples"), cell("B2", "120"), cell("C2", "135"),
            cell("A3", "Pears"), cell("B3", "98"), cell("C3", "104"),
            {"command":"set","path":"/Sheet1/A1:C1","props":{"font.bold":"true"}},
        ]);
        let outcome = create_document(
            &bundle,
            test_model(),
            "ofox-office-xlsx",
            json!({"name": "fruit.xlsx", "format": "xlsx", "content": table}),
        );
        let (generation, bytes) = outcome.made(XLSX);
        assert!(bytes.starts_with(b"PK"));
        assert_eq!(generation["params"]["format"], "xlsx");
        assert_eq!(generation["params"]["sheets"], 1);
        assert_eq!(outcome.result["details"]["file"]["sheets"], 1);
        assert_no_secret_on_disk(outcome.config.path(), &[KEY, NEW_KEY]);
    }

    #[test]
    fn rejects_an_invalid_batch_without_a_file() {
        let Some(bundle) = engines_runtime() else {
            return;
        };
        let outcome = create_document(
            &bundle,
            test_model(),
            "ofox-office-pptx",
            json!({"name": "deck.pptx", "format": "pptx", "content": batch("bad-batch.json")}),
        );
        let sent = outcome.refused();
        assert!(sent.contains("assistant_document_failed"), "{sent}");
        assert!(
            sent.contains("item 1") && sent.contains("\"explode\""),
            "{sent}"
        );
        assert_no_secret_on_disk(outcome.config.path(), &[KEY, NEW_KEY]);
    }

    #[test]
    fn rolls_back_a_batch_that_officecli_refuses() {
        let Some(bundle) = engines_runtime() else {
            return;
        };
        // Valid to the host, but the deck has one slide, not nine.
        let rollback = json!([
            {"command":"add","parent":"/","type":"slide","props":{"title":"Plan"}},
            {"command":"set","path":"/slide[9]/shape[1]","props":{"text":"x"}},
        ]);
        let outcome = create_document(
            &bundle,
            test_model(),
            "ofox-office-pptx",
            json!({"name": "deck.pptx", "format": "pptx", "content": rollback}),
        );
        let sent = outcome.refused();
        let report = sent
            .strip_prefix("assistant_document_failed: ")
            .unwrap_or_else(|| panic!("{sent}"));
        let report: Value = serde_json::from_str(report).unwrap_or_else(|e| panic!("{e}: {sent}"));
        let message = report["message"].as_str().unwrap();
        assert!(message.contains("1 of 2 commands failed"), "{sent}");
        assert!(message.contains("rolled back"), "{sent}");
        let [item] = report["items"].as_array().unwrap().as_slice() else {
            panic!("one failed command: {sent}");
        };
        assert_eq!(item["index"], 1, "{sent}");
        let said = item["output"].as_str().unwrap();
        // officecli's own words, e.g. "Slide 9 not found (total: 1)".
        assert!(said.contains("not found"), "{sent}");
        let folder = outcome.config.path().to_string_lossy().into_owned();
        assert!(
            !sent.contains(&folder),
            "the document folder is scrubbed: {sent}"
        );
        assert_no_secret_on_disk(outcome.config.path(), &[KEY, NEW_KEY]);
    }

    #[test]
    fn rejects_a_typst_syntax_error() {
        let Some(bundle) = engines_runtime() else {
            return;
        };
        let body = "= 一、开头\n\n这是一段正文。\n\n#table(\n  columns: 2,\n  [甲], [乙],\n";
        let outcome = create_document(
            &bundle,
            test_model(),
            "ofox-office-pdf",
            json!({"name": "broken.pdf", "format": "pdf", "content": body}),
        );
        let sent = outcome.refused();
        assert!(sent.contains("assistant_document_failed"), "{sent}");
        assert!(
            sent.contains("Typst could not compile") && sent.contains("error"),
            "{sent}"
        );
        assert_no_secret_on_disk(outcome.config.path(), &[KEY, NEW_KEY]);
    }
}
