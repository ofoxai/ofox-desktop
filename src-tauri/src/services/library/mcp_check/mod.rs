//! Explicit, bounded MCP inspection. No native configuration or resource writes.
mod policy;
mod protocol;
mod transport;

use super::mcp_resource;
use crate::{error::AppError, Database};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[derive(Debug, PartialEq)]
enum Failure {
    Cancelled,
    Timeout,
    DiscoveryTimeout,
    LegacyRequired,
    Limit,
    Invalid,
    Rpc(i64),
    UnsupportedVersion,
    Network,
    Http(u16),
    Blocked(&'static str),
}
impl Failure {
    fn code(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Timeout | Self::DiscoveryTimeout => "timeout",
            Self::Limit => "limit",
            Self::Invalid | Self::Rpc(_) | Self::LegacyRequired => "protocol",
            Self::UnsupportedVersion => "version",
            Self::Network => "network",
            Self::Http(401) => "authRequired",
            Self::Http(403) => "forbidden",
            Self::Http(300..=399) => "redirect",
            Self::Http(_) => "http",
            Self::Blocked(code) => code,
        }
    }
    fn state(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Timeout | Self::DiscoveryTimeout => "timeout",
            Self::Blocked(_) => "blocked",
            Self::Http(401) => "authRequired",
            Self::Http(403) => "forbidden",
            _ => "error",
        }
    }
}
struct Job {
    resource_id: String,
    revision: u64,
    epoch: u64,
    hash: String,
    spec: Value,
    created: Instant,
    running: AtomicBool,
    cancel: watch::Sender<bool>,
    gate: Mutex<()>,
    finished: AtomicBool,
}
fn jobs() -> &'static Mutex<BTreeMap<String, Arc<Job>>> {
    static JOBS: OnceLock<Mutex<BTreeMap<String, Arc<Job>>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(BTreeMap::new()))
}
fn error(code: &str) -> AppError {
    AppError::Config(format!("library.{code}"))
}
fn frozen(db: &Database, id: &str, revision: u64, epoch: u64) -> Result<(String, Value), AppError> {
    let _io = crate::config_io::lock();
    let snapshot = db.library_snapshot()?;
    if snapshot.machine.epoch != epoch {
        return Err(error("epochChanged"));
    }
    let meta = snapshot
        .definitions
        .mcp
        .get(id)
        .filter(|meta| meta.revision == revision)
        .ok_or_else(|| error("revisionChanged"))?;
    let server = db
        .get_all_mcp_servers()?
        .shift_remove(id)
        .ok_or_else(|| error("resourceMissing"))?;
    if mcp_resource::spec_hash(&server.server)? != meta.hash {
        return Err(error("revisionChanged"));
    }
    Ok((meta.hash.clone(), server.server))
}
pub fn prepare(db: &Database, id: &str, revision: u64, epoch: u64) -> Result<Value, AppError> {
    let _io = crate::config_io::lock();
    let (hash, spec) = frozen(db, id, revision, epoch)?;
    let mut jobs = jobs().lock().map_err(|_| error("mcpCheckBusy"))?;
    jobs.retain(|_, job| {
        job.running.load(Ordering::Acquire) || job.created.elapsed() < Duration::from_secs(60)
    });
    if jobs.len() >= 4 || jobs.values().any(|job| job.resource_id == id) {
        return Err(error("mcpCheckBusy"));
    }
    let operation_id = uuid::Uuid::new_v4().to_string();
    let (cancel, _) = watch::channel(false);
    jobs.insert(
        operation_id.clone(),
        Arc::new(Job {
            resource_id: id.into(),
            revision,
            epoch,
            hash,
            spec,
            created: Instant::now(),
            running: AtomicBool::new(false),
            cancel,
            gate: Mutex::new(()),
            finished: AtomicBool::new(false),
        }),
    );
    Ok(json!({"operationId":operation_id}))
}
pub fn cancel(id: &str) -> Result<bool, AppError> {
    let mut jobs = jobs().lock().map_err(|_| error("mcpCheckBusy"))?;
    let Some(job) = jobs.get(id).cloned() else {
        return Ok(false);
    };
    let _gate = job.gate.lock().map_err(|_| error("mcpCheckBusy"))?;
    if job.finished.load(Ordering::Acquire) {
        return Ok(false);
    }
    job.cancel.send_replace(true);
    if !job.running.load(Ordering::Acquire) {
        jobs.remove(id);
    }
    Ok(true)
}
struct Release(String);
impl Drop for Release {
    fn drop(&mut self) {
        if let Ok(mut jobs) = jobs().lock() {
            jobs.remove(&self.0);
        }
    }
}

pub async fn run<F: Fn(&str) + Send + Sync>(
    db: &Database,
    id: &str,
    stage: F,
) -> Result<Value, AppError> {
    let job = {
        let jobs = jobs().lock().map_err(|_| error("mcpCheckBusy"))?;
        let job = jobs
            .get(id)
            .filter(|job| job.created.elapsed() < Duration::from_secs(60))
            .ok_or_else(|| error("mcpCheckExpired"))?
            .clone();
        if job.running.swap(true, Ordering::AcqRel) {
            return Err(error("mcpCheckBusy"));
        }
        job
    };
    let _release = Release(id.into());
    let (hash, _) = frozen(db, &job.resource_id, job.revision, job.epoch)?;
    if hash != job.hash {
        return Err(error("revisionChanged"));
    }
    stage("preflight");
    let mut secrets = Vec::new();
    let plan = collect_strings(&job.spec, &mut secrets)
        .and_then(|()| transport::Plan::new(&job.spec, &mut secrets));
    let matcher = secret_matcher(&secrets);
    let result = match (plan, matcher.as_ref()) {
        (Ok(plan), Ok(_)) => {
            if *job.cancel.borrow() {
                Err(Failure::Cancelled)
            } else {
                stage("connect");
                transport::inspect(plan, job.cancel.subscribe(), &job.gate, &stage).await
            }
        }
        (Err(failure), _) => Err(failure),
        (_, Err(_)) => Err(Failure::Limit),
    };
    stage("cleanup");
    let (hash, _) = frozen(db, &job.resource_id, job.revision, job.epoch)?;
    if hash != job.hash {
        return Err(error("revisionChanged"));
    }
    let result = {
        let _gate = job.gate.lock().map_err(|_| error("mcpCheckBusy"))?;
        job.finished.store(true, Ordering::Release);
        if *job.cancel.borrow() {
            Err(Failure::Cancelled)
        } else {
            result
        }
    };
    let (state, reason, tools) = match result {
        Ok(tools) => ("ok", None, tools),
        Err(failure) => (failure.state(), Some(failure.code()), vec![]),
    };
    let tools=tools.into_iter().map(|tool| json!({"name":scrub(tool["name"].as_str().unwrap_or_default(),matcher.as_ref().unwrap(),256),"description":tool["description"].as_str().map(|text|scrub(text,matcher.as_ref().unwrap(),2048))})).collect::<Vec<_>>();
    Ok(json!({"operationId":id,"state":state,"reason":reason,"tools":tools}))
}
fn add_secret(text: &str, secrets: &mut Vec<String>) -> Result<(), Failure> {
    if text.is_empty() || secrets.iter().any(|secret| secret == text) {
        return Ok(());
    }
    if secrets.len() >= 1024
        || secrets.iter().map(String::len).sum::<usize>() + text.len() > 256 * 1024
    {
        return Err(Failure::Limit);
    }
    secrets.push(text.into());
    Ok(())
}
fn collect_strings(value: &Value, secrets: &mut Vec<String>) -> Result<(), Failure> {
    match value {
        Value::String(text) => {
            add_secret(text, secrets)?;
            for token in text.split(|ch: char| ch.is_whitespace() || "=:'\",;&".contains(ch)) {
                add_secret(token, secrets)?;
            }
            let mut tokens = text.split_whitespace();
            if tokens
                .next()
                .is_some_and(|kind| kind.eq_ignore_ascii_case("basic"))
            {
                use base64::Engine;
                if let Some(encoded) = tokens.next() {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                        if let Ok(decoded) = String::from_utf8(bytes) {
                            for part in decoded.split(':') {
                                add_secret(part, secrets)?;
                            }
                        }
                    }
                }
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_strings(value, secrets)?;
            }
        }
        Value::Array(list) => {
            for value in list {
                collect_strings(value, secrets)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn secret_matcher(secrets: &[String]) -> Result<aho_corasick::AhoCorasick, Failure> {
    aho_corasick::AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::Standard)
        .build(
            secrets
                .iter()
                .map(|secret| {
                    secret
                        .chars()
                        .filter(|ch| !ch.is_control() || *ch == '\n')
                        .collect::<String>()
                })
                .filter(|secret| !secret.is_empty()),
        )
        .map_err(|_| Failure::Limit)
}
fn scrub(text: &str, matcher: &aho_corasick::AhoCorasick, limit: usize) -> String {
    let normalized = text
        .chars()
        .filter(|ch| !ch.is_control() || *ch == '\n')
        .collect::<String>();
    let text = normalized.as_str();
    let mut matches = Vec::new();
    for found in matcher.find_overlapping_iter(text) {
        if matches.len() >= 16384 {
            return "[redacted]".into();
        }
        matches.push((found.start(), found.end()));
    }
    matches.sort_unstable();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (start, end) in matches {
        if let Some(previous) = ranges.last_mut().filter(|previous| start <= previous.1) {
            previous.1 = previous.1.max(end)
        } else {
            ranges.push((start, end))
        }
    }
    let mut clean = String::new();
    let mut last = 0;
    for (start, end) in ranges {
        clean.push_str(&text[last..start]);
        clean.push_str("[redacted]");
        last = end;
    }
    clean.push_str(&text[last..]);
    clean
        .chars()
        .filter(|ch| !ch.is_control() || *ch == '\n')
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn server(spec: Value) -> crate::app_config::McpServer {
        serde_json::from_value(
            json!({"id":"state-fixture","name":"Fixture","server":spec,"apps":{}}),
        )
        .unwrap()
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn the_complete_check_redacts_credentials_and_never_changes_resource_assignments() {
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let endpoint = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST")).respond_with(|request:&Request| {
            let value:Value=serde_json::from_slice(&request.body).unwrap();
            let result=if value["method"]=="server/discover" {json!({"resultType":"complete","supportedVersions":[protocol::MODERN],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"})} else {json!({"resultType":"complete","ttlMs":0,"cacheScope":"private","tools":[{"name":"fixture-tool","description":"echo secret-fixture-value","inputSchema":{}}]})};
            ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":value["id"],"result":result}))
        }).mount(&endpoint).await;
        let source = server(
            json!({"url":endpoint.uri(),"headers":{"Authorization":"Bearer secret-fixture-value"}}),
        );
        mcp_resource::save_draft(&db, source.clone(), None).unwrap();
        let before = db.library_snapshot().unwrap();
        let prepared = prepare(&db, &source.id, 1, before.machine.epoch).unwrap();
        let id = prepared["operationId"].as_str().unwrap();
        let result = run(&db, id, |_| {}).await.unwrap();
        assert_eq!(result["state"], "ok");
        assert_eq!(result["tools"].as_array().unwrap().len(), 1);
        assert!(!result.to_string().contains("secret-fixture-value"));
        assert_eq!(db.library_snapshot().unwrap(), before);
        assert_eq!(
            db.get_all_mcp_servers().unwrap()[&source.id].server,
            source.server
        );
        assert!(!cancel(id).unwrap());
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn prepared_checks_reject_definition_or_epoch_changes_and_final_results_are_rechecked() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let mut source = server(json!({"command":"npx"}));
        mcp_resource::save_draft(&db, source.clone(), None).unwrap();
        let epoch = db.library_snapshot().unwrap().machine.epoch;
        assert!(prepare(&db, &source.id, 2, epoch).is_err());
        assert!(prepare(&db, &source.id, 1, epoch + 1).is_err());
        let prepared = prepare(&db, &source.id, 1, epoch).unwrap();
        let id = prepared["operationId"].as_str().unwrap();
        assert!(prepare(&db, &source.id, 1, epoch).is_err());
        source.name = "New name".into();
        mcp_resource::save_draft(&db, source.clone(), Some(1)).unwrap();
        assert!(run(&db, id, |_| panic!("stale check reached transport"))
            .await
            .unwrap_err()
            .to_string()
            .contains("revisionChanged"));
        let prepared = prepare(&db, &source.id, 2, epoch).unwrap();
        let id = prepared["operationId"].as_str().unwrap();
        let before = db.library_snapshot().unwrap();
        let mut restored = before.clone();
        restored.machine.epoch += 1;
        db.library_commit(&before, &restored).unwrap();
        assert!(run(&db, id, |_| panic!("old epoch reached transport"))
            .await
            .unwrap_err()
            .to_string()
            .contains("epochChanged"));
        let prepared = prepare(&db, &source.id, 2, epoch + 1).unwrap();
        let id = prepared["operationId"].as_str().unwrap();
        let changed = AtomicBool::new(false);
        let result = run(&db, id, |stage| {
            if stage == "cleanup" && !changed.swap(true, Ordering::SeqCst) {
                mcp_resource::save_draft(&db, source.clone(), Some(2)).unwrap();
            }
        })
        .await;
        assert!(result.unwrap_err().to_string().contains("revisionChanged"));
    }
    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn cancellation_at_connect_prevents_even_attempting_to_spawn() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let server:crate::app_config::McpServer=serde_json::from_value(json!({"id":"cancel-at-connect","name":"Fixture","server":{"command":"/usr/bin/true"},"apps":{}})).unwrap();
        mcp_resource::save_draft(&db, server, None).unwrap();
        let epoch = db.library_snapshot().unwrap().machine.epoch;
        let prepared = prepare(&db, "cancel-at-connect", 1, epoch).unwrap();
        let id = prepared["operationId"].as_str().unwrap();
        let before = transport::SPAWNS.load(Ordering::SeqCst);
        let result = run(&db, id, |stage| {
            if stage == "connect" {
                assert!(cancel(id).unwrap());
            }
        })
        .await
        .unwrap();
        assert_eq!(result["state"], "cancelled");
        assert_eq!(transport::SPAWNS.load(Ordering::SeqCst), before);
    }
    #[test]
    fn overlapping_public_and_secret_matches_hide_the_entire_credential() {
        let secrets = vec!["node".into(), "eyJ-private-payload".into()];
        assert!(!scrub(
            "nodeyJ-private-payload",
            &secret_matcher(&secrets).unwrap(),
            2048
        )
        .contains("private"));
    }
    #[test]
    fn removing_control_characters_must_not_reassemble_a_secret_after_redaction() {
        let secrets = vec!["sk-private-payload".into()];
        assert!(!scrub(
            "sk-\u{1}private-payload",
            &secret_matcher(&secrets).unwrap(),
            2048
        )
        .contains("private"));
    }
    #[test]
    fn configured_auth_payloads_do_not_cross_ipc_as_tool_descriptions() {
        let spec = json!({"headers":{"Authorization":"Bearer sk-private-payload"},"args":["--api-key=another-private-key"]});
        let mut secrets = vec![];
        collect_strings(&spec, &mut secrets).unwrap();
        assert!(!scrub(
            "tool echoed sk-private-payload and another-private-key",
            &secret_matcher(&secrets).unwrap(),
            2048
        )
        .contains("private"));
    }
}
