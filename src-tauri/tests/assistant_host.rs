//! The assistant host and its commands end to end: the real prepared runtime
//! (Node + Pi SDK) against a local mock gateway that speaks the OpenAI APIs
//! and, under `/anthropic/`, Anthropic Messages, with the conversation files
//! in a temp dir.
use cc_switch_lib::assistant::{
    catalog::{parse_models, Model, Source},
    host::{Admit, AssistantHost, BoxFuture, Environment, Identity},
    types::{self, CreateRequest, Detail, Event, Message, RunProxy, SendRequest},
};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
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

struct Gateway {
    port: u16,
    script: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Gateway {
    fn start() -> Self {
        let v4 = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = v4.local_addr().unwrap().port();
        let script = Arc::new(Mutex::new(VecDeque::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut listeners = vec![v4];
        // `localhost` may resolve to ::1 first.
        if let Ok(v6) = TcpListener::bind(("::1", port)) {
            listeners.push(v6);
        }
        for listener in listeners {
            let (script, requests) = (script.clone(), requests.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (script, requests) = (script.clone(), requests.clone());
                    std::thread::spawn(move || serve(stream, &script, &requests));
                }
            });
        }
        Self {
            port,
            script,
            requests,
        }
    }

    fn base_url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    fn reply(&self, reply: Reply) {
        self.script.lock().unwrap().push_back(reply);
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn serve(stream: TcpStream, script: &Mutex<VecDeque<Reply>>, requests: &Mutex<Vec<Request>>) {
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
    } else {
        read_chunked(&mut reader)
    };
    requests.lock().unwrap().push(Request {
        path: path.clone(),
        authorization: headers.get("authorization").cloned().unwrap_or_default(),
        headers,
        body: serde_json::from_slice(&body).unwrap(),
    });
    let reply = script
        .lock()
        .unwrap()
        .pop_front()
        .expect("the gateway got a request the test did not script");
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
        assert_eq!(boot["runtime"]["version"], "1.4.0");
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
        assert_eq!(
            host.list_sessions(false, "then")
                .await
                .unwrap()
                .sessions
                .len(),
            1
        );
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
            json!({"available": false, "unitPriceUsd": null, "model": SEARCH_MODEL})
        );
        let sid = create(&host).await;
        let on = serde_json::from_value(json!({"sessionId": sid, "webSearch": true})).unwrap();
        host.update_session(on).await.unwrap();
        gateway.reply(Reply::Stream("No search here".into()));
        let (run, end) = turn(&host, &env, request(&sid, "What is new?", json!({}))).await;
        assert_eq!(end.data["status"], "completed");
        assert!(tool_names(&gateway.requests()[0]).is_empty());
        // The run says it could not search, although the switch is on.
        assert_eq!(of_type(&env, &run, "run_start")[0]["webSearch"], false);

        // Priced: offered, and a conversation with search off still gets no tools.
        env.catalog.lock().unwrap().push(search_model());
        let boot = AssistantHost::new(env.clone()).bootstrap().await.unwrap();
        assert_eq!(
            boot["search"],
            json!({"available": true, "unitPriceUsd": "0.014", "model": SEARCH_MODEL})
        );
        let host = AssistantHost::new(env.clone());
        let request_off: types::CreateRequest =
            serde_json::from_value(json!({"webSearch": false})).unwrap();
        let off = host.create_session(request_off).await.unwrap();
        assert!(!off.web_search);
        gateway.reply(Reply::Stream("Still no search".into()));
        let (run, _) = turn(&host, &env, request(&off.id, "What is new?", json!({}))).await;
        assert!(tool_names(&gateway.requests()[1]).is_empty());
        assert_eq!(of_type(&env, &run, "run_start")[0]["webSearch"], false);

        // The conversation with search on now gets both tools.
        gateway.reply(Reply::Stream("Could search".into()));
        let (run, _) = turn(&host, &env, request(&sid, "And now?", json!({}))).await;
        assert_eq!(
            tool_names(&gateway.requests()[2]),
            ["web_search", "web_fetch"]
        );
        assert_eq!(of_type(&env, &run, "run_start")[0]["webSearch"], true);
        let detail = host.get_session(&sid).await.unwrap();
        assert_eq!(detail.usage["searchCount"], 0);
        assert_eq!(detail.usage["searchCostUsd"], "0");
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
