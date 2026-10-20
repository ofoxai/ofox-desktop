use super::{
    policy,
    protocol::{self, Rpc},
    Failure,
};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client, Response,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, future::Future, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::watch,
    time::Instant,
};
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_MESSAGE: usize = 256 * 1024;

pub(super) enum Plan {
    Stdio {
        program: PathBuf,
        args: Vec<String>,
        cwd: PathBuf,
        env: BTreeMap<String, String>,
    },
    Http {
        url: url::Url,
        headers: HeaderMap,
        sse: bool,
    },
}
fn expand(raw: &str, secrets: &mut Vec<String>) -> Result<String, Failure> {
    let mut rest = raw;
    let mut out = String::new();
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or(Failure::Blocked("referenceInvalid"))?;
        let key = &after[..end];
        if key.is_empty()
            || !key.bytes().enumerate().all(|(index, b)| {
                b == b'_' || b.is_ascii_alphabetic() || index > 0 && b.is_ascii_digit()
            })
        {
            return Err(Failure::Blocked("referenceInvalid"));
        }
        let value = std::env::var(key).map_err(|_| Failure::Blocked("credentialMissing"))?;
        if out.len() + value.len() > 65536 {
            return Err(Failure::Limit);
        }
        super::collect_strings(&Value::String(value.clone()), secrets)?;
        out.push_str(&value);
        rest = &after[end + 1..];
        if out.len() > 65536 {
            return Err(Failure::Limit);
        }
    }
    out.push_str(rest);
    if out.len() > 65536 || out.contains('\0') {
        return Err(Failure::Limit);
    }
    Ok(out)
}
impl Plan {
    pub(super) fn new(spec: &Value, secrets: &mut Vec<String>) -> Result<Self, Failure> {
        let obj = spec.as_object().ok_or(Failure::Invalid)?;
        if spec
            .get("args")
            .and_then(Value::as_array)
            .is_some_and(|args| args.len() > 128)
            || ["env", "headers"].iter().any(|key| {
                spec.get(*key)
                    .and_then(Value::as_object)
                    .is_some_and(|map| map.len() > 64)
            })
        {
            return Err(Failure::Limit);
        }
        if spec.get("type").is_some_and(|kind| !kind.is_string()) {
            return Err(Failure::Invalid);
        }
        let mut expanded_bytes = 0usize;
        let mut expand_value = |raw: &str| {
            let text = expand(raw, secrets)?;
            expanded_bytes = expanded_bytes
                .checked_add(text.len())
                .ok_or(Failure::Limit)?;
            if expanded_bytes > MAX_MESSAGE {
                return Err(Failure::Limit);
            }
            Ok(text)
        };

        let kind = spec
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(if obj.contains_key("url") {
                "http"
            } else {
                "stdio"
            });
        if obj.keys().any(|key| {
            !["type", "command", "args", "cwd", "env", "url", "headers"].contains(&key.as_str())
        }) {
            return Err(Failure::Blocked("fieldUnsupported"));
        }
        match kind {
            "stdio" => {
                if obj.contains_key("url") || obj.contains_key("headers") {
                    return Err(Failure::Invalid);
                }
                let mut env = BTreeMap::new();
                for key in [
                    "PATH",
                    "HOME",
                    "USERPROFILE",
                    "SystemRoot",
                    "WINDIR",
                    "TEMP",
                    "TMP",
                    "LANG",
                    "LC_ALL",
                ] {
                    if let Ok(value) = std::env::var(key) {
                        if value.len() > 65536 {
                            return Err(Failure::Limit);
                        }
                        env.insert(key.to_owned(), value);
                    }
                }
                env.entry("HOME".into()).or_insert_with(|| {
                    crate::config::get_home_dir().to_string_lossy().into_owned()
                });
                if let Some(values) = spec.get("env") {
                    for (key, value) in values.as_object().ok_or(Failure::Invalid)? {
                        if key.is_empty() || key.contains(['=', '\0']) {
                            return Err(Failure::Invalid);
                        }
                        env.insert(
                            key.clone(),
                            expand_value(value.as_str().ok_or(Failure::Invalid)?)?,
                        );
                    }
                }
                let cwd = spec
                    .get("cwd")
                    .map(|value| {
                        expand_value(value.as_str().ok_or(Failure::Invalid)?).map(PathBuf::from)
                    })
                    .transpose()?
                    .unwrap_or_else(crate::config::get_home_dir);
                if !cwd.is_absolute() || !cwd.is_dir() {
                    return Err(Failure::Blocked("cwdMissing"));
                }
                let command = expand_value(
                    spec["command"]
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .ok_or(Failure::Invalid)?,
                )?;
                let args = spec
                    .get("args")
                    .map(|args| {
                        args.as_array()
                            .ok_or(Failure::Invalid)?
                            .iter()
                            .map(|value| expand_value(value.as_str().ok_or(Failure::Invalid)?))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                if args.len() > 128 {
                    return Err(Failure::Limit);
                }
                let program = policy::installed_program(
                    &command,
                    &args,
                    env.get("PATH").map(String::as_str).unwrap_or_default(),
                    &cwd,
                )
                .map_err(Failure::Blocked)?;
                Ok(Self::Stdio {
                    program,
                    args,
                    cwd,
                    env,
                })
            }
            "http" | "sse" => {
                if ["command", "args", "cwd", "env"]
                    .iter()
                    .any(|key| obj.contains_key(*key))
                {
                    return Err(Failure::Invalid);
                }
                let raw = expand_value(spec["url"].as_str().ok_or(Failure::Invalid)?)?;
                let url = url::Url::parse(&raw).map_err(|_| Failure::Invalid)?;
                if !["http", "https"].contains(&url.scheme())
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err(Failure::Blocked("urlCredentials"));
                }
                let mut headers = HeaderMap::new();
                if let Some(values) = spec.get("headers") {
                    for (key, value) in values.as_object().ok_or(Failure::Invalid)? {
                        let key =
                            HeaderName::from_bytes(key.as_bytes()).map_err(|_| Failure::Invalid)?;
                        if [
                            "host",
                            "content-length",
                            "content-type",
                            "accept",
                            "connection",
                            "transfer-encoding",
                            "mcp-protocol-version",
                            "mcp-session-id",
                            "mcp-method",
                        ]
                        .contains(&key.as_str())
                            || key.as_str().starts_with("proxy-")
                        {
                            return Err(Failure::Blocked("headerUnsupported"));
                        }
                        let value = expand_value(value.as_str().ok_or(Failure::Invalid)?)?;
                        headers.insert(
                            key,
                            HeaderValue::from_str(&value).map_err(|_| Failure::Invalid)?,
                        );
                    }
                }
                Ok(Self::Http {
                    url,
                    headers,
                    sse: kind == "sse",
                })
            }
            _ => Err(Failure::Blocked("transportUnsupported")),
        }
    }
}
struct Control<'a> {
    cancel: watch::Receiver<bool>,
    end: Instant,
    stage: &'a (dyn Fn(&str) + Send + Sync),
}
async fn bounded<T>(
    cancel: &mut watch::Receiver<bool>,
    end: Instant,
    future: impl Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    if *cancel.borrow() {
        return Err(Failure::Cancelled);
    }
    tokio::select! {biased;
        _=cancel.changed()=>Err(Failure::Cancelled),
        result=tokio::time::timeout_at(end,future)=>result.map_err(|_|Failure::Timeout)?,
    }
}
fn reply(value: Value, id: &Value) -> Result<Value, Failure> {
    if value["jsonrpc"] != "2.0"
        || &value["id"] != id
        || value.get("method").is_some()
        || value.get("result").is_some() == value.get("error").is_some()
    {
        return Err(Failure::Invalid);
    }
    if let Some(error) = value.get("error") {
        return Err(Failure::Rpc(
            error["code"].as_i64().ok_or(Failure::Invalid)?,
        ));
    }
    value
        .get("result")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or(Failure::Invalid)
}

fn server_reply(request: &Value) -> Option<Value> {
    let id = request.get("id")?;
    request.get("method")?;
    Some(if request["method"] == "ping" {
        json!({"jsonrpc":"2.0","id":id,"result":{}})
    } else {
        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Client capability unavailable"}})
    })
}

#[cfg(test)]
pub(super) static SPAWNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct ManagedChild {
    #[cfg(unix)]
    child: tokio::process::Child,
    #[cfg(unix)]
    group: u32,
    #[cfg(windows)]
    child: Box<dyn process_wrap::tokio::ChildWrapper>,
    stopped: bool,
    stderr: Option<tokio::task::JoinHandle<()>>,
}
impl ManagedChild {
    fn spawn(mut command: tokio::process::Command) -> Result<Self, Failure> {
        #[cfg(test)]
        SPAWNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
            let mut child = command
                .spawn()
                .map_err(|_| Failure::Blocked("spawnFailed"))?;
            let Some(group) = child.id() else {
                let _ = child.start_kill();
                return Err(Failure::Blocked("spawnFailed"));
            };
            Ok(Self {
                child,
                group,
                stopped: false,
                stderr: None,
            })
        }
        #[cfg(windows)]
        {
            use process_wrap::tokio::{CommandWrap, CreationFlags, JobObject, KillOnDrop};
            use windows::Win32::System::Threading::CREATE_NO_WINDOW;
            let mut command = CommandWrap::from(command);
            command
                .wrap(KillOnDrop)
                .wrap(CreationFlags(CREATE_NO_WINDOW))
                .wrap(JobObject);
            Ok(Self {
                child: command
                    .spawn()
                    .map_err(|_| Failure::Blocked("spawnFailed"))?,
                stopped: false,
                stderr: None,
            })
        }
    }
    fn kill(&mut self) {
        if self.stopped {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.group as i32), libc::SIGKILL);
        }
        let _ = self.child.start_kill();
        self.stopped = true;
    }
    async fn stop(&mut self) {
        self.kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        if let Some(task) = self.stderr.take() {
            task.abort();
        }
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.kill();
        if let Some(task) = self.stderr.take() {
            task.abort();
        }
    }
}
struct StdioRpc<'a> {
    input: tokio::process::ChildStdin,
    output: BufReader<tokio::process::ChildStdout>,
    control: Control<'a>,
    seq: u32,
    total: usize,
    buffer: Vec<u8>,
    retired: Vec<Value>,
}
impl StdioRpc<'_> {
    async fn write(&mut self, value: &Value, end: Instant) -> Result<(), Failure> {
        let mut bytes = serde_json::to_vec(value).map_err(|_| Failure::Invalid)?;
        bytes.push(b'\n');
        bounded(&mut self.control.cancel, end, async {
            self.input
                .write_all(&bytes)
                .await
                .map_err(|_| Failure::Network)?;
            self.input.flush().await.map_err(|_| Failure::Network)
        })
        .await
    }
    async fn read(&mut self, end: Instant) -> Result<Value, Failure> {
        bounded(&mut self.control.cancel, end, async {
            loop {
                let available = self.output.fill_buf().await.map_err(|_| Failure::Network)?;
                if available.is_empty() {
                    return Err(Failure::Network);
                }
                let length = available
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map(|end| end + 1)
                    .unwrap_or(available.len());
                if self.buffer.len() + length > MAX_MESSAGE || self.total + length > MAX_BYTES {
                    return Err(Failure::Limit);
                }
                self.buffer.extend_from_slice(&available[..length]);
                self.total += length;
                self.output.consume(length);
                if self.buffer.last() == Some(&b'\n') {
                    return serde_json::from_slice(&std::mem::take(&mut self.buffer))
                        .map_err(|_| Failure::Invalid);
                }
            }
        })
        .await
    }
}
impl Rpc for StdioRpc<'_> {
    async fn call(
        &mut self,
        method: &str,
        params: Value,
        _version: &str,
    ) -> Result<Value, Failure> {
        (self.control.stage)(if method == "tools/list" {
            "list"
        } else {
            "negotiate"
        });
        self.seq += 1;
        let id = json!(format!("ofox-{}", self.seq));
        let discovery = method == "server/discover";
        let end = if discovery {
            self.control
                .end
                .min(Instant::now() + Duration::from_secs(3))
        } else {
            self.control.end
        };
        let result = async {
            self.write(
                &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
                end,
            )
            .await?;
            for _ in 0..100 {
                let value = self.read(end).await?;
                if value.get("method").is_some() {
                    if value["jsonrpc"] != "2.0" {
                        return Err(Failure::Invalid);
                    }
                    if value.get("id").is_some() {
                        let response = server_reply(&value).ok_or(Failure::Invalid)?;
                        self.write(&response, end).await?;
                    }
                    continue;
                }
                if value.get("id").is_some_and(|id| self.retired.contains(id)) {
                    continue;
                }
                return reply(value, &id);
            }
            Err(Failure::Limit)
        }
        .await;
        if discovery && result == Err(Failure::Timeout) {
            self.retired.push(id);
            Err(Failure::DiscoveryTimeout)
        } else {
            result
        }
    }
    async fn notify(&mut self, method: &str, params: Value, _version: &str) -> Result<(), Failure> {
        self.write(
            &json!({"jsonrpc":"2.0","method":method,"params":params}),
            self.control.end,
        )
        .await
    }
}

fn sse_frame(buffer: &mut Vec<u8>) -> Result<Option<(String, String)>, Failure> {
    let mut index = 0;
    let mut line_start = 0;
    let mut boundary = None;
    while index < buffer.len() {
        if index > MAX_MESSAGE {
            return Err(Failure::Limit);
        }
        if buffer[index] == b'\r' || buffer[index] == b'\n' {
            let length = if buffer[index] == b'\r' && buffer.get(index + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            if index == line_start {
                boundary = Some((line_start, index + length));
                break;
            }
            index += length;
            line_start = index;
        } else {
            index += 1;
        }
    }
    let Some((end, consumed)) = boundary else {
        return if buffer.len() > MAX_MESSAGE {
            Err(Failure::Limit)
        } else {
            Ok(None)
        };
    };
    let text = std::str::from_utf8(&buffer[..end])
        .map_err(|_| Failure::Invalid)?
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut event = "message".to_owned();
    let mut data = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event = value.strip_prefix(' ').unwrap_or(value).to_owned();
        }
        if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    let result = (event, data.join("\n"));
    buffer.drain(..consumed);
    Ok(Some(result))
}
fn append_sse_chunk(buffer: &mut Vec<u8>, total: &mut usize, chunk: &[u8]) -> Result<(), Failure> {
    *total = total.checked_add(chunk.len()).ok_or(Failure::Limit)?;
    if *total > MAX_BYTES {
        return Err(Failure::Limit);
    }
    buffer.extend_from_slice(chunk);
    Ok(())
}

struct Sse {
    response: Response,
    buffer: Vec<u8>,
    total: usize,
}
impl Sse {
    async fn event(&mut self, control: &mut Control<'_>) -> Result<(String, String), Failure> {
        bounded(&mut control.cancel, control.end, async {
            loop {
                if let Some((event, data)) = sse_frame(&mut self.buffer)? {
                    if !data.is_empty() {
                        return Ok((event, data));
                    }
                    continue;
                }
                let chunk = self
                    .response
                    .chunk()
                    .await
                    .map_err(|_| Failure::Network)?
                    .ok_or(Failure::Network)?;
                append_sse_chunk(&mut self.buffer, &mut self.total, &chunk)?;
            }
        })
        .await
    }
}
struct HttpRpc<'a> {
    client: Client,
    url: url::Url,
    headers: HeaderMap,
    session: Option<HeaderValue>,
    seq: u32,
    control: Control<'a>,
    stream: Option<Sse>,
    total: usize,
}
impl HttpRpc<'_> {
    async fn open_sse(&mut self) -> Result<(), Failure> {
        let response = bounded(&mut self.control.cancel, self.control.end, async {
            self.client
                .get(self.url.clone())
                .headers(self.headers.clone())
                .header("accept", "text/event-stream")
                .send()
                .await
                .map_err(|_| Failure::Network)
        })
        .await?;
        if response.status().as_u16() != 200 {
            return Err(Failure::Http(response.status().as_u16()));
        }
        if !response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|ct| ct.starts_with("text/event-stream"))
        {
            return Err(Failure::Invalid);
        }
        let mut stream = Sse {
            response,
            buffer: vec![],
            total: 0,
        };
        for _ in 0..16 {
            let (event, data) = stream.event(&mut self.control).await?;
            if event == "endpoint" {
                let endpoint = self.url.join(&data).map_err(|_| Failure::Invalid)?;
                if endpoint.origin() != self.url.origin()
                    || !endpoint.username().is_empty()
                    || endpoint.password().is_some()
                    || endpoint.fragment().is_some()
                {
                    return Err(Failure::Blocked("endpointOrigin"));
                }
                self.url = endpoint;
                self.stream = Some(stream);
                return Ok(());
            }
        }
        Err(Failure::Invalid)
    }
    async fn post(
        &mut self,
        value: Value,
        version: &str,
        method: &str,
        id: Option<&Value>,
    ) -> Result<Value, Failure> {
        let mut request = self
            .client
            .post(self.url.clone())
            .headers(self.headers.clone())
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", version);
        if version == protocol::MODERN {
            request = request.header("Mcp-Method", method)
        }
        if version != protocol::MODERN {
            if let Some(session) = &self.session {
                request = request.header("Mcp-Session-Id", session);
            }
        }
        let mut response = bounded(&mut self.control.cancel, self.control.end, async {
            request
                .json(&value)
                .send()
                .await
                .map_err(|_| Failure::Network)
        })
        .await?;
        let status = response.status().as_u16();
        if status != 200 && status != 202 && status != 400 {
            return Err(Failure::Http(status));
        }
        if method == "initialize" {
            if let Some(session) = response.headers().get("Mcp-Session-Id") {
                if session.as_bytes().len() > 1024 {
                    return Err(Failure::Limit);
                }
                self.session = Some(session.clone());
            }
        }
        let Some(id) = id else {
            return if (200..300).contains(&status) {
                Ok(Value::Null)
            } else {
                Err(Failure::Http(status))
            };
        };
        if let Some(mut stream) = self.stream.take() {
            if !(200..300).contains(&status) {
                self.stream = Some(stream);
                return Err(Failure::Http(status));
            }
            let result = self.sse_reply(&mut stream, id, version).await;
            self.stream = Some(stream);
            return result;
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if content_type.starts_with("text/event-stream") {
            let mut stream = Sse {
                response,
                buffer: vec![],
                total: 0,
            };
            let result = self.sse_reply(&mut stream, id, version).await;
            self.total += stream.total;
            if self.total > MAX_BYTES {
                return Err(Failure::Limit);
            }
            if status == 400 && result.is_ok() {
                return Err(Failure::Http(status));
            }
            return result;
        }
        if !content_type.starts_with("application/json") {
            return Err(if status == 400 && method == "server/discover" {
                Failure::LegacyRequired
            } else if status == 400 {
                Failure::Http(status)
            } else {
                Failure::Invalid
            });
        }
        let mut bytes = Vec::new();
        bounded(&mut self.control.cancel, self.control.end, async {
            while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Network)? {
                self.total += chunk.len();
                if bytes.len() + chunk.len() > MAX_MESSAGE || self.total > MAX_BYTES {
                    return Err(Failure::Limit);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(())
        })
        .await?;
        if status == 400 && method == "server/discover" {
            let parsed = serde_json::from_slice::<Value>(&bytes).ok();
            if let Some(code) = parsed
                .as_ref()
                .and_then(|value| value["error"]["code"].as_i64())
                .filter(|code| [-32020, -32021, -32022].contains(code))
            {
                return Err(Failure::Rpc(code));
            }
            return Err(Failure::LegacyRequired);
        }
        let parsed = serde_json::from_slice(&bytes).map_err(|_| Failure::Invalid)?;
        let result = reply(parsed, id);
        if status == 400 && result.is_ok() {
            return Err(Failure::Http(status));
        }

        result
    }
    async fn sse_reply(
        &mut self,
        stream: &mut Sse,
        id: &Value,
        version: &str,
    ) -> Result<Value, Failure> {
        for _ in 0..100 {
            let (event, data) = stream.event(&mut self.control).await?;
            if event != "message" {
                continue;
            }
            let value: Value = serde_json::from_str(&data).map_err(|_| Failure::Invalid)?;
            if value["jsonrpc"] != "2.0" {
                return Err(Failure::Invalid);
            }
            if value.get("method").is_some() {
                if let Some(response) = server_reply(&value) {
                    let mut request = self
                        .client
                        .post(self.url.clone())
                        .headers(self.headers.clone())
                        .header("content-type", "application/json")
                        .header("accept", "application/json, text/event-stream")
                        .header("MCP-Protocol-Version", version);
                    if let Some(session) = &self.session {
                        request = request.header("Mcp-Session-Id", session);
                    }
                    let response = bounded(&mut self.control.cancel, self.control.end, async {
                        request
                            .json(&response)
                            .send()
                            .await
                            .map_err(|_| Failure::Network)
                    })
                    .await?;
                    if !response.status().is_success() {
                        return Err(Failure::Http(response.status().as_u16()));
                    }
                }
                continue;
            }
            return reply(value, id);
        }
        Err(Failure::Limit)
    }
}
impl Rpc for HttpRpc<'_> {
    async fn call(&mut self, method: &str, params: Value, version: &str) -> Result<Value, Failure> {
        (self.control.stage)(if method == "tools/list" {
            "list"
        } else {
            "negotiate"
        });
        self.seq += 1;
        let id = json!(format!("ofox-{}", self.seq));
        self.post(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            version,
            method,
            Some(&id),
        )
        .await
    }
    async fn notify(&mut self, method: &str, params: Value, version: &str) -> Result<(), Failure> {
        self.post(
            json!({"jsonrpc":"2.0","method":method,"params":params}),
            version,
            method,
            None,
        )
        .await
        .map(|_| ())
    }
}
pub(super) async fn inspect(
    plan: Plan,
    cancel: watch::Receiver<bool>,
    gate: &std::sync::Mutex<()>,
    stage: &(dyn Fn(&str) + Send + Sync),
) -> Result<Vec<Value>, Failure> {
    let control = Control {
        cancel,
        end: Instant::now() + Duration::from_secs(120),
        stage,
    };
    match plan {
        Plan::Stdio {
            program,
            args,
            cwd,
            env,
        } => {
            let mut command = tokio::process::Command::new(program);
            command.args(args).current_dir(cwd).env_clear().envs(env);
            let mut child = {
                let _gate = gate.lock().map_err(|_| Failure::Cancelled)?;
                if *control.cancel.borrow() {
                    return Err(Failure::Cancelled);
                }
                ManagedChild::spawn(command)?
            };
            #[cfg(unix)]
            let (input, output, stderr) = (
                child.child.stdin.take(),
                child.child.stdout.take(),
                child.child.stderr.take(),
            );
            #[cfg(windows)]
            let (input, output, stderr) = (
                child.child.stdin().take(),
                child.child.stdout().take(),
                child.child.stderr().take(),
            );
            let result =
                if let (Some(input), Some(output), Some(mut stderr)) = (input, output, stderr) {
                    child.stderr = Some(tokio::spawn(async move {
                        let _ = tokio::time::timeout(Duration::from_secs(120), async {
                            let mut bytes = [0u8; 4096];
                            let mut total = 0;
                            while let Ok(length) = stderr.read(&mut bytes).await {
                                if length == 0 {
                                    break;
                                }
                                total += length;
                                if total > MAX_BYTES {
                                    break;
                                }
                            }
                        })
                        .await;
                    }));
                    let mut rpc = StdioRpc {
                        input,
                        output: BufReader::new(output),
                        control,
                        seq: 0,
                        total: 0,
                        buffer: vec![],
                        retired: vec![],
                    };
                    protocol::inspect(&mut rpc, false).await
                } else {
                    Err(Failure::Network)
                };
            stage("cleanup");
            child.stop().await;
            result
        }
        Plan::Http { url, headers, sse } => {
            let client =
                crate::proxy::http_client::mcp_check_client().map_err(|_| Failure::Network)?;
            let mut rpc = HttpRpc {
                client,
                url,
                headers,
                session: None,
                seq: 0,
                control,
                stream: None,
                total: 0,
            };
            if sse {
                rpc.open_sse().await?;
            }
            protocol::inspect(&mut rpc, sse).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node_fixture(body: &str) -> (tempfile::TempDir, Plan) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fixture.cjs");
        std::fs::write(&script, body).unwrap();
        let plan = Plan::new(
            &json!({"command":"node","args":[script],"cwd":dir.path()}),
            &mut vec![],
        )
        .unwrap();
        (dir, plan)
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn installed_node_fixture_checks_modern_stdio_without_tools_call_or_config_writes() {
        let _sandbox = crate::test_sandbox::sandbox();
        let (dir, plan) = node_fixture(
            r#"
const fs=require('fs');const readline=require('readline');
const out=v=>process.stdout.write(JSON.stringify(v)+'\n');
readline.createInterface({input:process.stdin}).on('line',line=>{
const v=JSON.parse(line);fs.appendFileSync('requests.jsonl',line+'\n');
if(v.method==='server/discover') out({jsonrpc:'2.0',id:v.id,result:{resultType:'complete',supportedVersions:['2026-07-28'],capabilities:{tools:{}},ttlMs:0,cacheScope:'private'}});
if(v.method==='tools/list') {process.stderr.write('fixture diagnostic\n'.repeat(1000));out({jsonrpc:'2.0',id:v.id,result:{resultType:'complete',ttlMs:0,cacheScope:'private',tools:[{name:'local-fixture-tool',inputSchema:{type:'object'}}]}});}
});
"#,
        );
        let (_sender, cancel) = watch::channel(false);
        let gate = std::sync::Mutex::new(());
        let tools = inspect(plan, cancel, &gate, &ignore_stage).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "local-fixture-tool");
        let requests = std::fs::read_to_string(dir.path().join("requests.jsonl")).unwrap();
        let methods = requests
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap()["method"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(methods, ["server/discover", "tools/list"]);
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn late_partial_probe_reply_and_core_ping_do_not_corrupt_legacy_stdio_initialization() {
        let _sandbox = crate::test_sandbox::sandbox();
        let (_dir, plan) = node_fixture(
            r#"
const readline=require('readline');const out=v=>process.stdout.write(JSON.stringify(v)+'\n');let init;
readline.createInterface({input:process.stdin}).on('line',line=>{
const v=JSON.parse(line);
if(v.method==='server/discover') {
 const text=JSON.stringify({jsonrpc:'2.0',id:v.id,error:{code:-32000,message:'Legacy server'}})+'\n';const half=Math.floor(text.length/2);
 setTimeout(()=>process.stdout.write(text.slice(0,half)),2700);setTimeout(()=>process.stdout.write(text.slice(half)),3300);
}
if(v.method==='initialize') {init=v.id;setTimeout(()=>out({jsonrpc:'2.0',id:'fixture-ping',method:'ping'}),700);}
if(v.id==='fixture-ping' && v.result && !v.error) out({jsonrpc:'2.0',id:init,result:{protocolVersion:'2025-06-18',capabilities:{tools:{}},serverInfo:{name:'local legacy fixture',version:'1'}}});
if(v.method==='tools/list') out({jsonrpc:'2.0',id:v.id,result:{tools:[{name:'legacy-fixture-tool',inputSchema:{}}]}});
});
"#,
        );
        let (_sender, cancel) = watch::channel(false);
        let gate = std::sync::Mutex::new(());
        let tools = tokio::time::timeout(
            Duration::from_secs(10),
            inspect(plan, cancel, &gate, &ignore_stage),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(tools[0]["name"], "legacy-fixture-tool");
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn cancelling_a_check_reaps_the_fixture_process_tree() {
        let _sandbox = crate::test_sandbox::sandbox();
        let (dir, plan) = node_fixture(
            r#"
const cp=require('child_process');const readline=require('readline');const out=v=>process.stdout.write(JSON.stringify(v)+'\n');
cp.spawn(process.execPath,['-e',"const fs=require('fs');fs.writeFileSync('descendant-ready','yes');setTimeout(()=>fs.writeFileSync('descendant-survived','yes'),1000);setInterval(()=>{},1000)"],{stdio:'ignore'});
readline.createInterface({input:process.stdin}).on('line',line=>{const v=JSON.parse(line);if(v.method==='server/discover')out({jsonrpc:'2.0',id:v.id,result:{resultType:'complete',supportedVersions:['2026-07-28'],capabilities:{tools:{}},ttlMs:0,cacheScope:'private'}});});
"#,
        );
        let (sender, cancel) = watch::channel(false);
        let gate = std::sync::Mutex::new(());
        let work = inspect(plan, cancel, &gate, &ignore_stage);
        // A loaded CI runner can take seconds to start both node processes.
        let cancel_when_ready = async {
            for _ in 0..500 {
                if dir.path().join("descendant-ready").exists() {
                    sender.send_replace(true);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            panic!("fixture descendant did not start");
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(20), async {
            tokio::join!(work, cancel_when_ready)
        })
        .await
        .unwrap();
        assert_eq!(result, Err(Failure::Cancelled));
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!dir.path().join("descendant-survived").exists());
    }
    fn ignore_stage(_: &str) {}
    fn http_rpc(url: &str) -> (watch::Sender<bool>, HttpRpc<'static>) {
        let (cancel, receiver) = watch::channel(false);
        let client = crate::proxy::http_client::mcp_check_client().unwrap();
        (
            cancel,
            HttpRpc {
                client,
                url: url::Url::parse(url).unwrap(),
                headers: HeaderMap::new(),
                session: None,
                seq: 0,
                control: Control {
                    cancel: receiver,
                    end: Instant::now() + Duration::from_secs(5),
                    stage: &ignore_stage,
                },
                stream: None,
                total: 0,
            },
        )
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn http_legacy_plain_400_probe_can_initialize_without_downgrading_modern_errors() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, Request, ResponseTemplate,
        };
        let _sandbox = crate::test_sandbox::sandbox();
        let server = MockServer::start().await;
        Mock::given(path("/mcp")).and(method("POST")).respond_with(|request:&Request| {
            let value:Value=serde_json::from_slice(&request.body).unwrap();
            if value["method"]=="server/discover" {return ResponseTemplate::new(400).set_body_string("legacy handshake required")}
            if value["method"]=="initialize" {return ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":value["id"],"result":{"protocolVersion":protocol::LEGACY,"capabilities":{},"serverInfo":{"name":"fixture"}}}))}
            ResponseTemplate::new(202)
        }).mount(&server).await;
        let (_sender, mut rpc) = http_rpc(&format!("{}/mcp", server.uri()));
        assert_eq!(
            protocol::inspect(&mut rpc, false).await.unwrap(),
            Vec::<Value>::new()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn http_sse_notifications_and_ping_before_the_matching_reply_are_allowed() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, Request, ResponseTemplate,
        };
        let _sandbox = crate::test_sandbox::sandbox();
        let server = MockServer::start().await;
        Mock::given(path("/mcp")).and(method("POST")).respond_with(|request:&Request| {
            let value:Value=serde_json::from_slice(&request.body).unwrap();
            if value["method"]=="initialize" {return ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":value["id"],"result":{"protocolVersion":protocol::LEGACY,"capabilities":{"tools":{}},"serverInfo":{"name":"fixture"}}}))}
            if value["method"]=="tools/list" {
                let messages=[json!({"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info","data":"fixture"}}),json!({"jsonrpc":"2.0","id":"server-ping","method":"ping"}),json!({"jsonrpc":"2.0","id":value["id"],"result":{"tools":[{"name":"工具","inputSchema":{}}]}})];
                return ResponseTemplate::new(200).set_body_raw(messages.iter().map(|value|format!("event: message\ndata: {value}\n\n")).collect::<String>(),"text/event-stream");
            }
            if value["id"]=="server-ping" && request.headers.get("accept").and_then(|value|value.to_str().ok())!=Some("application/json, text/event-stream") {return ResponseTemplate::new(406)}
            ResponseTemplate::new(202)
        }).mount(&server).await;
        let (_sender, mut rpc) = http_rpc(&format!("{}/mcp", server.uri()));
        assert_eq!(
            protocol::inspect(&mut rpc, true).await.unwrap()[0]["name"],
            "工具"
        );
        let requests = server.received_requests().await.unwrap();
        assert!(requests
            .iter()
            .map(|request| serde_json::from_slice::<Value>(&request.body).unwrap())
            .any(|value| value["id"] == "server-ping" && value["result"] == json!({})));
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn redirects_never_forward_authorization_and_auth_errors_never_fall_back() {
        use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
        let _sandbox = crate::test_sandbox::sandbox();
        let source = MockServer::start().await;
        let destination = MockServer::start().await;
        Mock::given(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("location", format!("{}/credential-sink", destination.uri())),
            )
            .mount(&source)
            .await;
        Mock::given(path("/auth"))
            .respond_with(ResponseTemplate::new(401).set_body_string("private error body"))
            .mount(&source)
            .await;
        let (_sender, mut rpc) = http_rpc(&format!("{}/redirect", source.uri()));
        rpc.headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer private-fixture"),
        );
        assert_eq!(
            protocol::inspect(&mut rpc, false).await,
            Err(Failure::Http(307))
        );
        assert!(destination.received_requests().await.unwrap().is_empty());
        let (_sender, mut rpc) = http_rpc(&format!("{}/auth", source.uri()));
        assert_eq!(
            protocol::inspect(&mut rpc, false).await,
            Err(Failure::Http(401))
        );
        assert_eq!(source.received_requests().await.unwrap().len(), 2);
    }
    #[test]
    fn sse_frame_limits_do_not_reject_coalesced_frames_or_mixed_line_endings() {
        let first = format!("data: {}\n\n", "x".repeat(150000));
        let second = format!("data: {}\n\n", "y".repeat(150000));
        let mut buffer = vec![];
        let mut total = 0;
        append_sse_chunk(
            &mut buffer,
            &mut total,
            format!("{first}{second}").as_bytes(),
        )
        .unwrap();
        assert_eq!(sse_frame(&mut buffer).unwrap().unwrap().1.len(), 150000);
        assert_eq!(sse_frame(&mut buffer).unwrap().unwrap().1.len(), 150000);
        let mut mixed = "data: 甲\r\ndata: 乙\n\r\n".as_bytes().to_vec();
        assert_eq!(
            sse_frame(&mut mixed).unwrap(),
            Some(("message".into(), "甲\n乙".into()))
        );
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn a_400_sse_success_body_is_never_reported_as_a_successful_call() {
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};
        let _sandbox = crate::test_sandbox::sandbox();
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST")).respond_with(|request:&Request| {
            let value:Value=serde_json::from_slice(&request.body).unwrap();
            let reply=json!({"jsonrpc":"2.0","id":value["id"],"result":{"resultType":"complete","ttlMs":0,"cacheScope":"private","supportedVersions":[protocol::MODERN],"capabilities":{}}});
            ResponseTemplate::new(400).set_body_raw(format!("data: {reply}\n\n"),"text/event-stream")
        }).mount(&server).await;
        let (_sender, mut rpc) = http_rpc(&server.uri());
        assert_eq!(
            rpc.call("server/discover", json!({}), protocol::MODERN)
                .await,
            Err(Failure::Http(400))
        );
    }
    #[test]
    fn expansion_limits_are_checked_before_resolving_fields_and_across_the_plan() {
        for spec in [
            json!({"command":"missing","args":vec!["${}";129]}),
            json!({"command":"missing","env":(0..65).map(|i|(format!("VAR_{i}"),json!("${}"))).collect::<serde_json::Map<_,_>>()}),
            json!({"url":"http://127.0.0.1/mcp","headers":(0..65).map(|i|(format!("x-{i}"),json!("${}"))).collect::<serde_json::Map<_,_>>()}),
            json!({"command":"missing","args":vec!["x".repeat(60000);5]}),
        ] {
            assert!(matches!(Plan::new(&spec, &mut vec![]), Err(Failure::Limit)));
        }
    }
    #[test]
    fn sse_accepts_split_utf8_and_multiline_data_without_decoding_an_incomplete_frame() {
        let text = "event: message\r\ndata: 工具\r\ndata: 描述\r\n\r\n";
        let mut buffer = vec![];
        let mut frames = Vec::new();
        for byte in text.bytes() {
            buffer.push(byte);
            if let Some(frame) = sse_frame(&mut buffer).unwrap() {
                if !frame.1.is_empty() {
                    frames.push(frame)
                }
            }
        }
        assert_eq!(frames, vec![("message".into(), "工具\n描述".into())]);
    }
}
