//! Host-level preview checks with a task-owned store and a keyless environment.
//! No browser, real account, provider, or installed application is involved.
use super::{
    html_server_at, AssistantHost, FileRequest, HtmlLeases, Identity, PageRequest, VersionRequest,
    HTML_PORT, IDENTITY_CHANGED, PPTX, VERSION_CHANGED,
};
use crate::services::assistant::{
    catalog,
    host::{Admit, BoxFuture, Environment},
    store::AssistantStore,
    types::{id, Attachment, Event as AssistantEvent},
};
#[cfg(target_os = "macos")]
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use std::collections::HashSet;
use std::{
    io::{Cursor, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

struct PreviewEnvironment {
    config: PathBuf,
    runtime: Option<PathBuf>,
    current: Mutex<Identity>,
    key_calls: AtomicUsize,
    model_calls: AtomicUsize,
    provider_calls: AtomicUsize,
    gateway_calls: AtomicUsize,
    events: AtomicUsize,
}

impl Environment for PreviewEnvironment {
    fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
        Box::pin(async { Ok(self.current.lock().unwrap().clone()) })
    }
    fn api_key<'a>(
        &'a self,
        _: &'a Identity,
        _: Option<&'a str>,
        _: &'a Admit<'a>,
    ) -> BoxFuture<'a, Result<String, String>> {
        self.key_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err("fixture_api_key_forbidden".into()) })
    }
    fn key_ready<'a>(&'a self, _: &'a Identity) -> BoxFuture<'a, bool> {
        self.key_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { false })
    }
    fn stored_key<'a>(&'a self, _: &'a Identity) -> BoxFuture<'a, Option<String>> {
        self.key_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { None })
    }
    fn models<'a>(
        &'a self,
        _: &'a str,
        _: catalog::Source,
    ) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>> {
        self.model_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err("fixture_models_forbidden".into()) })
    }
    fn provider_prices<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        self.provider_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err("fixture_provider_forbidden".into()) })
    }
    fn gateway_base(&self, _: &str) -> Result<String, String> {
        self.gateway_calls.fetch_add(1, Ordering::SeqCst);
        Err("fixture_gateway_forbidden".into())
    }
    fn config_dir(&self) -> PathBuf {
        self.config.clone()
    }
    fn runtime_dir(&self) -> Result<PathBuf, String> {
        self.runtime
            .clone()
            .ok_or_else(|| "fixture_no_runtime".into())
    }
    fn emit(&self, _: AssistantEvent) {
        self.events.fetch_add(1, Ordering::SeqCst);
    }
}

struct Fixture {
    host: Arc<AssistantHost>,
    env: Arc<PreviewEnvironment>,
    store: AssistantStore,
    sid: String,
    _config: tempfile::TempDir,
}
impl Fixture {
    fn new(runtime: Option<PathBuf>) -> Self {
        let task = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".real-eval/file-preview-e2e");
        std::fs::create_dir_all(&task).unwrap();
        let config = tempfile::Builder::new()
            .prefix("keyless-")
            .tempdir_in(task)
            .unwrap();
        let identity = Identity {
            owner: "cd".repeat(32),
            apex: "ofox.ai".into(),
        };
        let store = AssistantStore::for_owner(config.path(), &identity.owner).unwrap();
        let sid = store
            .create("fixture/no-model-request", false)
            .unwrap()
            .session
            .id;
        let env = Arc::new(PreviewEnvironment {
            config: config.path().into(),
            runtime,
            current: Mutex::new(identity),
            key_calls: AtomicUsize::new(0),
            model_calls: AtomicUsize::new(0),
            provider_calls: AtomicUsize::new(0),
            gateway_calls: AtomicUsize::new(0),
            events: AtomicUsize::new(0),
        });
        Self {
            host: AssistantHost::new(env.clone()),
            env,
            store,
            sid,
            _config: config,
        }
    }
    fn add(&self, mime: &str, name: &str, bytes: &[u8]) -> FileRequest {
        let aid = id();
        let folder = self.store.attachment_dir(&self.sid).unwrap();
        std::fs::write(folder.join(&aid), bytes).unwrap();
        self.store
            .mutate(&self.sid, |record| {
                record.attachments.push(Attachment {
                    origin: None,
                    source_message_id: None,
                    id: aid.clone(),
                    name: name.into(),
                    mime_type: mime.into(),
                    size_bytes: bytes.len() as u64,
                    status: "ready".into(),
                    reason: None,
                    preview_url: None,
                    details: None,
                });
                Ok(())
            })
            .unwrap();
        FileRequest {
            session_id: self.sid.clone(),
            expected_owner: self.env.current.lock().unwrap().owner.clone(),
            expected_generation: self.host.generation(),
            attachment_id: aid,
        }
    }
    fn replace(&self, request: &FileRequest, bytes: &[u8]) {
        std::fs::write(
            self.store
                .attachment_root(&self.sid)
                .unwrap()
                .join(&request.attachment_id),
            bytes,
        )
        .unwrap();
        self.store
            .mutate(&self.sid, |record| {
                record
                    .attachments
                    .iter_mut()
                    .find(|file| file.id == request.attachment_id)
                    .unwrap()
                    .size_bytes = bytes.len() as u64;
                Ok(())
            })
            .unwrap();
    }
    fn assert_keyless(&self) {
        assert_eq!(
            self.env.key_calls.load(Ordering::SeqCst),
            0,
            "preview must not look up or mint keys"
        );
        assert_eq!(self.env.model_calls.load(Ordering::SeqCst), 0);
        assert_eq!(self.env.provider_calls.load(Ordering::SeqCst), 0);
        assert_eq!(self.env.gateway_calls.load(Ordering::SeqCst), 0);
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn minimal_deck(count: u32, label: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut presentation = String::from("<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><p:sldIdLst>");
    let mut relationships = String::from(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    let mut content_types = String::from("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>");
    for number in 1..=count {
        content_types.push_str(&format!("<Override PartName=\"/ppt/slides/slide{number}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"));
        presentation.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"r{number}\"/>",
            255 + number
        ));
        relationships.push_str(&format!("<Relationship Id=\"r{number}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{number}.xml\"/>"));
        zip.start_file(format!("ppt/slides/slide{number}.xml"), options)
            .unwrap();
        zip.write_all(format!("<p:sld xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld name=\"{label} {number}\"/></p:sld>").as_bytes()).unwrap();
    }
    presentation
        .push_str("</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/></p:presentation>");
    relationships.push_str("</Relationships>");
    content_types.push_str("</Types>");
    for (name, value) in [
        ("ppt/presentation.xml", presentation),
        ("ppt/_rels/presentation.xml.rels", relationships),
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"document\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>".to_owned()),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[tokio::test]
async fn preview_e2e_actual_manifest_is_versioned_seventy_pages_and_keyless() {
    let fixture = Fixture::new(None);
    let bytes = minimal_deck(70, "first");
    let request = fixture.add(PPTX, "synthetic-seventy.pptx", &bytes);
    let manifest = fixture.host.file_preview(request.clone()).await.unwrap();
    assert_eq!(manifest["attachmentId"], request.attachment_id);
    assert_eq!(manifest["version"], digest(&bytes));
    assert_eq!(manifest["totalPages"], 70);
    assert_eq!(
        manifest["readyPages"],
        json!([]),
        "a manifest is not fabricated rendered-page completion"
    );
    assert_eq!(manifest["limits"]["maxPage"], 1000);
    assert_eq!(
        (manifest["width"].as_u64(), manifest["height"].as_u64()),
        (Some(1280), Some(720))
    );
    assert!(!manifest
        .to_string()
        .contains(fixture.env.config.to_str().unwrap()));
    assert_eq!(fixture.env.events.load(Ordering::SeqCst), 0);
    fixture.assert_keyless();
}

#[tokio::test]
async fn preview_e2e_request_authority_missing_and_path_boundaries() {
    let fixture = Fixture::new(None);
    let request = fixture.add(PPTX, "synthetic.pptx", &minimal_deck(2, "first"));
    let wire = json!({"sessionId":request.session_id,"expectedOwner":request.expected_owner,"expectedGeneration":request.expected_generation,"attachmentId":request.attachment_id,"version":"0".repeat(64),"page":1,"scale":1});
    let decoded: PageRequest = serde_json::from_value(wire.clone())
        .expect("the actual flattened camelCase IPC page request must deserialize");
    assert_eq!(decoded.file.session_id, request.session_id);
    let mut unexpected = wire;
    unexpected["filePath"] = json!("/never/read/a/caller/path");
    assert!(
        serde_json::from_value::<PageRequest>(unexpected).is_err(),
        "caller paths and unknown request fields are refused"
    );
    let mut invalid = request.clone();
    invalid.attachment_id = "../secret".into();
    assert_eq!(
        fixture.host.file_preview(invalid).await.unwrap_err(),
        "assistant_invalid_identifier"
    );
    let mut invalid = request.clone();
    invalid.attachment_id = id();
    assert_eq!(
        fixture.host.file_preview(invalid).await.unwrap_err(),
        "assistant_attachment_not_found"
    );
    let mut invalid = request.clone();
    invalid.session_id = id();
    assert_eq!(
        fixture.host.file_preview(invalid).await.unwrap_err(),
        "assistant_session_not_found"
    );
    let mut invalid = request.clone();
    invalid.expected_owner = "ef".repeat(32);
    assert_eq!(
        fixture.host.file_preview(invalid).await.unwrap_err(),
        IDENTITY_CHANGED
    );
    let mut invalid = request.clone();
    invalid.expected_generation += 1;
    assert_eq!(
        fixture.host.file_preview(invalid).await.unwrap_err(),
        IDENTITY_CHANGED
    );
    let unsupported = fixture.add("text/plain", "unsupported.txt", b"synthetic only");
    assert_eq!(
        fixture.host.file_preview(unsupported).await.unwrap_err(),
        "assistant_attachment_not_found"
    );
    std::fs::remove_file(
        fixture
            .store
            .attachment_root(&fixture.sid)
            .unwrap()
            .join(&request.attachment_id),
    )
    .unwrap();
    assert_eq!(
        fixture.host.file_preview(request).await.unwrap_err(),
        "assistant_attachment_unreadable"
    );
    fixture.assert_keyless();
}

#[tokio::test]
async fn preview_e2e_same_size_replacement_invalidates_the_old_page_version_before_runtime() {
    let fixture = Fixture::new(None);
    let first = minimal_deck(3, "first");
    let other = minimal_deck(3, "other");
    assert_eq!(first.len(), other.len());
    let request = fixture.add(PPTX, "synthetic.pptx", &first);
    let manifest = fixture.host.file_preview(request.clone()).await.unwrap();
    let old = manifest["version"].as_str().unwrap().to_owned();
    fixture.replace(&request, &other);
    assert_eq!(
        fixture
            .host
            .file_preview_page(PageRequest {
                file: request.clone(),
                version: old,
                page: 1,
                scale: 1
            })
            .await
            .unwrap_err(),
        VERSION_CHANGED
    );
    let current = fixture.host.file_preview(request).await.unwrap();
    assert_eq!(current["version"], digest(&other));
    fixture.assert_keyless();
}

#[tokio::test]
async fn preview_e2e_account_switch_does_not_read_another_accounts_session() {
    let fixture = Fixture::new(None);
    let request = fixture.add(PPTX, "synthetic.pptx", &minimal_deck(1, "first"));
    fixture.env.current.lock().unwrap().owner = "ef".repeat(32);
    assert_eq!(
        fixture
            .host
            .file_preview(request.clone())
            .await
            .unwrap_err(),
        IDENTITY_CHANGED
    );
    let mut other_account = request;
    other_account.expected_owner = "ef".repeat(32);
    assert_eq!(
        fixture.host.file_preview(other_account).await.unwrap_err(),
        "assistant_session_not_found"
    );
    fixture.assert_keyless();
}

fn html_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    })
}

#[test]
fn preview_e2e_html_server_recovers_after_a_busy_port_without_poisoning_its_slot() {
    html_runtime().block_on(async {
        let occupied = loop {
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
            if listener.local_addr().unwrap().port() != HTML_PORT {
                break listener;
            }
        };
        let port = occupied.local_addr().unwrap().port();
        let slot: Mutex<Option<HtmlLeases>> = Mutex::new(None);
        assert_eq!(
            html_server_at(&slot, port)
                .err()
                .expect("busy listener must fail closed"),
            "assistant_preview_port_unavailable"
        );
        assert!(slot.lock().unwrap().is_none());
        drop(occupied);
        let recovered = html_server_at(&slot, port).expect("released port must be retryable");
        assert!(slot.lock().unwrap().is_some());
        assert!(Arc::ptr_eq(
            &recovered,
            &html_server_at(&slot, port).unwrap()
        ));
    });
}

#[test]
fn preview_e2e_nine_html_reopens_keep_http_authority_version_and_keyless_boundaries() {
    html_runtime().block_on(async {
        let fixture = Fixture::new(None);
        let bytes = b"<!doctype html><html><head><title>Synthetic</title></head><body><h1>Fixture alpha</h1><script>window.fixtureOnly=1</script><img src='missing.png'></body></html>";
        let file = fixture.add("text/html", "synthetic.html", bytes);
        let manifest = fixture.host.file_preview(file.clone()).await.unwrap();
        assert_eq!(manifest["kind"], "html");
        assert_eq!(manifest["warnings"], json!(["missing.png"]));
        let request = VersionRequest { file: file.clone(), version: digest(bytes) };
        let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(3)).build().unwrap();
        let mut last_url = String::new();
        for _ in 0..9 {
            let result = fixture.host.file_preview_html(request.clone()).await.expect("reopening one source must not exhaust the eight-lease limit");
            assert_eq!(result["version"], request.version);
            assert_eq!(result["attachmentId"], file.attachment_id);
            last_url = result["url"].as_str().unwrap().to_owned();
            assert!(last_url.starts_with(&format!("http://127.0.0.1:{HTML_PORT}/")));
            let response = client.get(&last_url).send().await.unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            let policy = response.headers()["Content-Security-Policy"].to_str().unwrap();
            assert!(policy.contains("connect-src 'none'") && policy.contains("sandbox allow-scripts"));
            assert_eq!(response.headers()["Cache-Control"], "no-store");
            assert!(response.text().await.unwrap().contains("Fixture alpha"));
        }
        let other = fixture.add("text/html", "other.html", b"<!doctype html><p>Other synthetic document</p>");
        fixture.host.file_preview_html(VersionRequest { file: other, version: digest(b"<!doctype html><p>Other synthetic document</p>") }).await.unwrap();
        assert_eq!(client.get(&last_url).send().await.unwrap().status(), reqwest::StatusCode::OK, "opening another file must not revoke this file's current lease");
        fixture.replace(&file, std::str::from_utf8(bytes).unwrap().replace("alpha", "omega").as_bytes());
        assert_eq!(fixture.host.file_preview_html(request.clone()).await.unwrap_err(), VERSION_CHANGED);
        assert_eq!(client.get(&last_url).send().await.unwrap().status(), reqwest::StatusCode::NOT_FOUND);
        let updated = fixture.host.file_preview(file.clone()).await.unwrap();
        let current = fixture.host.file_preview_html(VersionRequest { file, version: updated["version"].as_str().unwrap().into() }).await.unwrap();
        let current_url = current["url"].as_str().unwrap();
        fixture.env.current.lock().unwrap().owner = "ef".repeat(32);
        fixture.host.cancel_all();
        assert_eq!(client.get(current_url).send().await.unwrap().status(), reqwest::StatusCode::FORBIDDEN);
        fixture.env.current.lock().unwrap().owner = "cd".repeat(32);
        assert_eq!(client.get(current_url).send().await.unwrap().status(), reqwest::StatusCode::FORBIDDEN, "returning to an account must not revive its old generation lease");
        fixture.assert_keyless();
    });
}

#[cfg(target_os = "macos")]
fn jpeg_dimensions(bytes: &[u8]) -> (u16, u16) {
    assert!(bytes.starts_with(&[255, 216, 255]) && bytes.ends_with(&[255, 217]));
    let mut offset = 2usize;
    while offset + 8 < bytes.len() {
        assert_eq!(bytes[offset], 255);
        offset += 1;
        while bytes[offset] == 255 {
            offset += 1;
        }
        let marker = bytes[offset];
        offset += 1;
        if matches!(marker, 0xc0..=0xc2) {
            return (
                u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]),
                u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]),
            );
        }
        assert!(
            !matches!(marker, 0xda | 0xd9),
            "JPEG must declare its actual dimensions before scan data"
        );
        let length = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        assert!(length >= 2);
        offset += length;
    }
    panic!("JPEG dimensions were not found")
}

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit task-private OFOX_PREVIEW_RUNTIME and OFOX_PREVIEW_CASE; never uses installed/user documents"]
async fn preview_e2e_actual_runtime_renders_pages_one_sixty_one_seventy_and_scale_two_without_keys()
{
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    let private = repository.join(".real-eval");
    let runtime = PathBuf::from(
        std::env::var_os("OFOX_PREVIEW_RUNTIME").expect("provide an explicit task-private runtime"),
    )
    .canonicalize()
    .unwrap();
    let case = PathBuf::from(
        std::env::var_os("OFOX_PREVIEW_CASE")
            .expect("provide an explicit task-owned seventy-page case"),
    )
    .canonicalize()
    .unwrap();
    assert!(
        runtime.starts_with(&private) && case.starts_with(&private),
        "live probes may only read task-owned copies inside this checkout"
    );
    let source = std::fs::read(&case).unwrap();
    let source_hash = digest(&source);
    let fixture = Fixture::new(Some(runtime));
    let request = fixture.add(PPTX, "synthetic-real-seventy.pptx", &source);
    let manifest = fixture.host.file_preview(request.clone()).await.unwrap();
    assert_eq!(manifest["totalPages"], 70);
    assert_eq!(manifest["version"], source_hash);
    let all_pages = std::env::var("OFOX_PREVIEW_ALL_PAGES").as_deref() == Ok("1");
    let evidence = all_pages.then(|| {
        private
            .join("ppt-completion-2026-11-10")
            .join(format!("host-all70-{}", super::super::types::id()))
    });
    if let Some(path) = &evidence {
        std::fs::create_dir(path).unwrap();
    }
    let requested: Vec<(u32, u8)> = if all_pages {
        (1..=70).map(|page| (page, 1)).chain([(70, 2)]).collect()
    } else {
        vec![(1, 1), (61, 1), (70, 1), (70, 2)]
    };
    let mut receipts = vec![];
    let mut images = HashSet::new();
    for (page, scale) in requested {
        let result = fixture
            .host
            .file_preview_page(PageRequest {
                file: request.clone(),
                version: source_hash.clone(),
                page,
                scale,
            })
            .await
            .expect("actual sandboxed page preview, including pages beyond sixty");
        assert_eq!(result["attachmentId"], request.attachment_id);
        assert_eq!(result["version"], source_hash);
        assert_eq!(result["page"], page);
        assert_eq!(result["mimeType"], "image/jpeg");
        let bytes = STANDARD
            .decode(result["dataBase64"].as_str().unwrap())
            .unwrap();
        assert!(!bytes.is_empty() && bytes.len() <= 2 * 1024 * 1024);
        let actual = jpeg_dimensions(&bytes);
        assert_eq!(
            (u64::from(actual.0), u64::from(actual.1)),
            (
                result["width"].as_u64().unwrap(),
                result["height"].as_u64().unwrap()
            )
        );
        assert_eq!(
            actual,
            if scale == 2 {
                (3072, 1728)
            } else {
                (1536, 864)
            }
        );
        if scale == 1 {
            images.insert(digest(&bytes));
        }
        if let Some(path) = &evidence {
            let filename = format!("page-{page:03}-scale-{scale}.jpg");
            std::fs::write(path.join(&filename), &bytes).unwrap();
            receipts.push(json!({"page":page,"scale":scale,"file":filename,"sha256":digest(&bytes),"width":actual.0,"height":actual.1}));
        }
        fixture.assert_keyless();
    }
    assert_eq!(
        images.len(),
        if all_pages { 70 } else { 3 },
        "distinct source pages must not all return the first page"
    );
    assert_eq!(digest(&std::fs::read(&case).unwrap()), source_hash);
    assert_eq!(
        digest(
            &std::fs::read(
                fixture
                    .store
                    .attachment_root(&fixture.sid)
                    .unwrap()
                    .join(&request.attachment_id)
            )
            .unwrap()
        ),
        source_hash
    );
    fixture.assert_keyless();
    assert_eq!(fixture.env.events.load(Ordering::SeqCst), 0);
    if let Some(path) = &evidence {
        std::fs::write(path.join("receipt.json"), serde_json::to_vec_pretty(&json!({"sourceSha256":source_hash,"totalPages":70,"providerCalls":0,"keyCalls":0,"modelCalls":0,"renders":receipts})).unwrap()).unwrap();
        eprintln!("Real Host page evidence: {}", path.display());
    }
}
