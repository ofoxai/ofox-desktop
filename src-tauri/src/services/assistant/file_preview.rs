//! Version-bound file workspace. Opening a file never starts an assistant run.
use super::{
    attachments,
    host::{blocking, AssistantHost, Identity, IDENTITY_CHANGED},
    store::AssistantStore,
    types::{validate_id, Attachment},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

const ERROR: &str = "assistant_preview_unavailable";
const VERSION_CHANGED: &str = "assistant_preview_version_changed";
const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
const MAX_BYTES: u64 = 25 * 1024 * 1024;
const MAX_XML: u64 = 4 * 1024 * 1024;
// A resource limit, not a fabricated document length. The manifest still
// reports the complete source count and the viewer explains the remainder.
const MAX_RENDER_PAGE: u32 = 1000;
const HTML_PORT: u16 = 43763;
const HTML_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; sandbox allow-scripts";
static PAGE_TURN: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
pub(in crate::services::assistant) mod opc;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileRequest {
    pub session_id: String,
    pub expected_owner: String,
    pub expected_generation: u64,
    pub attachment_id: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionRequest {
    #[serde(flatten)]
    pub file: FileRequest,
    pub version: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageRequest {
    #[serde(flatten)]
    pub file: FileRequest,
    pub version: String,
    pub page: u32,
    pub scale: u8,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeckInfo {
    pub(super) presentation_path: String,
    pub(super) total_pages: u32,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) slide_paths: Vec<String>,
}

const PNS: &[u8] = b"http://schemas.openxmlformats.org/presentationml/2006/main";
const RNS: &[u8] = b"http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKGNS: &[u8] = b"http://schemas.openxmlformats.org/package/2006/relationships";
type XmlName = (Vec<u8>, Vec<u8>);
pub(super) struct XmlTag {
    name: XmlName,
    parent: Option<XmlName>,
    depth: usize,
    attrs: HashMap<XmlName, String>,
}
impl XmlTag {
    pub(super) fn is(&self, namespace: &[u8], name: &[u8]) -> bool {
        self.name.0 == namespace && self.name.1 == name
    }
    fn child_of(&self, namespace: &[u8], name: &[u8]) -> bool {
        self.parent
            .as_ref()
            .is_some_and(|p| p.0 == namespace && p.1 == name)
    }
    pub(super) fn attr(&self, name: &[u8]) -> Option<&String> {
        self.nsattr(b"", name)
    }
    fn nsattr(&self, namespace: &[u8], name: &[u8]) -> Option<&String> {
        self.attrs.get(&(namespace.to_vec(), name.to_vec()))
    }
}
fn namespace(result: ResolveResult<'_>) -> Result<Vec<u8>, String> {
    match result {
        ResolveResult::Bound(ns) => Ok(ns.as_ref().to_vec()),
        ResolveResult::Unbound => Ok(vec![]),
        ResolveResult::Unknown(_) => Err(ERROR.into()),
    }
}
pub(super) fn xml_tags(bytes: &[u8]) -> Result<Vec<XmlTag>, String> {
    let mut reader = NsReader::from_reader(bytes);
    let mut result = vec![];
    let mut parents: Vec<XmlName> = vec![];
    let mut roots = 0usize;
    loop {
        let (resolved, event) = reader.read_resolved_event().map_err(|_| ERROR)?;
        let ns = namespace(resolved)?;
        let open = matches!(event, Event::Start(_));
        match event {
            Event::Start(tag) | Event::Empty(tag) => {
                if parents.len() >= 128 || result.len() >= 100_000 {
                    return Err(ERROR.into());
                }
                if parents.is_empty() {
                    roots += 1;
                    if roots > 1 {
                        return Err(ERROR.into());
                    }
                }
                let mut attrs = HashMap::new();
                for attr in tag.attributes() {
                    let attr = attr.map_err(|_| ERROR)?;
                    let (resolved, local) = reader.resolve_attribute(attr.key);
                    let key = (namespace(resolved)?, local.as_ref().to_vec());
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|_| ERROR)?
                        .into_owned();
                    if attrs.insert(key, value).is_some() {
                        return Err(ERROR.into());
                    }
                }
                let name = (ns, tag.local_name().as_ref().to_vec());
                result.push(XmlTag {
                    name: name.clone(),
                    parent: parents.last().cloned(),
                    depth: parents.len(),
                    attrs,
                });
                if open {
                    parents.push(name);
                }
            }
            Event::End(_) => {
                parents.pop().ok_or(ERROR)?;
            }
            Event::DocType(_) => return Err(ERROR.into()),
            Event::Eof => {
                if !parents.is_empty() {
                    return Err(ERROR.into());
                }
                break;
            }
            _ => (),
        }
    }
    Ok(result)
}
pub(super) fn zip_member(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, String> {
    let mut file = zip.by_name(name).map_err(|_| ERROR)?;
    if file.size() > MAX_XML {
        return Err(ERROR.into());
    }
    let mut bytes = vec![];
    file.by_ref()
        .take(MAX_XML + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 != file.size() {
        return Err(ERROR.into());
    }
    Ok(bytes)
}
pub(super) fn deck_info(bytes: &[u8]) -> Result<DeckInfo, String> {
    let mut zip = crate::zip_security::open(Cursor::new(bytes), 40_000).map_err(|_| ERROR)?;
    crate::zip_security::validate_headers(&mut zip).map_err(|_| ERROR)?;
    let mut names = HashSet::new();
    let mut expanded = 0u64;
    for n in 0..zip.len() {
        let file = zip.by_index(n).map_err(|_| ERROR)?;
        let name = file.name().map_err(|_| ERROR)?.into_owned();
        if !names.insert(name.clone())
            || name.contains('\\')
            || name.contains(':')
            || !Path::new(&name)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(ERROR.into());
        }
        expanded = expanded.checked_add(file.size()).ok_or(ERROR)?;
        if file.size() > 32 * 1024 * 1024 || expanded > 256 * 1024 * 1024 {
            return Err(ERROR.into());
        }
    }
    let active = opc::resolve_presentation(&mut zip)?;
    let presentation = xml_tags(&zip_member(&mut zip, &active.presentation_path)?)?;
    let rels = opc::presentation_relationships(&mut zip, &active)?;
    let mut relationships = HashMap::new();
    if !presentation
        .first()
        .is_some_and(|tag| tag.is(PNS, b"presentation"))
    {
        return Err(ERROR.into());
    }
    for relation in rels {
        if relation.kind
            == "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide"
        {
            if relation.external || !names.contains(&relation.target) {
                return Err(ERROR.into());
            }
            relationships.insert(relation.id, relation.target);
        }
    }
    let (mut width, mut height) = (1280, 720);
    let mut slide_paths = vec![];
    for tag in presentation {
        if tag.is(PNS, b"sldId") && tag.child_of(PNS, b"sldIdLst") && tag.depth == 2 {
            let relationship = tag.nsattr(RNS, b"id").ok_or(ERROR)?;
            slide_paths.push(relationships.get(relationship).ok_or(ERROR)?.clone());
        } else if tag.is(PNS, b"sldSz") && tag.child_of(PNS, b"presentation") && tag.depth == 1 {
            let dimension = |key: &[u8]| {
                tag.attr(key)
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|n| *n > 0 && *n <= 100_000_000)
                    .map(|n| ((n + 4762) / 9525) as u32)
                    .ok_or(ERROR.to_string())
            };
            width = dimension(b"cx")?;
            height = dimension(b"cy")?;
        }
    }
    if slide_paths.len() > 10_000 {
        return Err(ERROR.into());
    }
    Ok(DeckInfo {
        presentation_path: active.presentation_path,
        total_pages: slide_paths.len() as u32,
        width,
        height,
        slide_paths,
    })
}

fn related_part(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    source: &str,
    rid: &str,
    kind: &str,
) -> Result<String, String> {
    let relations = opc::relationships_for(zip, source)?;
    let relation = relations
        .into_iter()
        .find(|relation| relation.id == rid)
        .ok_or(ERROR)?;
    if relation.external || relation.kind != kind {
        return Err(ERROR.into());
    }
    Ok(relation.target)
}
/// Structural evidence, not WPS/PowerPoint certification: required native
/// equations and charts must exist on the correct actual page. A raster of
/// either cannot pass simply because its design plan called it editable.
pub(super) fn validate_native_pages(
    path: &Path,
    slides: &[Value],
    target_pages: Option<&[u32]>,
) -> Result<(), String> {
    let (file, size) = attachments::open_written(path, MAX_BYTES)?;
    let mut bytes = vec![];
    file.take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 != size {
        return Err(ERROR.into());
    }
    let info = deck_info(&bytes)?;
    let targets: Vec<u32> = target_pages
        .map(|pages| pages.to_vec())
        .unwrap_or_else(|| (1..=info.total_pages).collect());
    if targets.len() != slides.len()
        || targets
            .iter()
            .any(|page| *page == 0 || *page > info.total_pages)
    {
        return Err("assistant_presentation_plan_invalid".into());
    }
    let mut zip =
        crate::zip_security::open(Cursor::new(bytes.as_slice()), 40_000).map_err(|_| ERROR)?;
    for (index, plan) in slides.iter().enumerate() {
        let source = &info.slide_paths[(targets[index] - 1) as usize];
        let tags = xml_tags(&zip_member(&mut zip, source)?)?;
        let kind = plan["visual"]["kind"]
            .as_str()
            .ok_or("assistant_presentation_plan_invalid")?;
        if plan["editing"]["text"] == true
            && !tags.iter().any(|tag| {
                tag.is(
                    b"http://schemas.openxmlformats.org/drawingml/2006/main",
                    b"t",
                )
            })
        {
            return Err("assistant_presentation_plan_invalid: native page text is missing".into());
        }
        match kind {
            "native-math"
                if !tags.iter().any(|tag| {
                    tag.is(
                        b"http://schemas.openxmlformats.org/officeDocument/2006/math",
                        b"oMath",
                    )
                }) =>
            {
                return Err("assistant_presentation_plan_invalid: native equation is missing from its planned page".into());
            }
            "native-chart" => {
                const CHART: &[u8] = b"http://schemas.openxmlformats.org/drawingml/2006/chart";
                let chart=tags.iter().find(|tag|tag.is(CHART,b"chart")).ok_or("assistant_presentation_plan_invalid: native chart is missing from its planned page")?;
                let rid = chart.nsattr(RNS, b"id").ok_or(ERROR)?;
                let chart = related_part(
                    &mut zip,
                    source,
                    rid,
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart",
                )?;
                let chart_tags = xml_tags(&zip_member(&mut zip, &chart)?)?;
                let data = chart_tags
                    .iter()
                    .find(|tag| tag.is(CHART, b"externalData"))
                    .ok_or(
                        "assistant_presentation_plan_invalid: editable chart workbook is missing",
                    )?;
                let rid = data.nsattr(RNS, b"id").ok_or(ERROR)?;
                let workbook = related_part(
                    &mut zip,
                    &chart,
                    rid,
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/package",
                )?;
                let data = zip_member(&mut zip, &workbook)?;
                let mut workbook = crate::zip_security::open(Cursor::new(data.as_slice()), 4096)
                    .map_err(|_| ERROR)?;
                crate::zip_security::validate_headers(&mut workbook).map_err(|_| ERROR)?;
                zip_member(&mut workbook, "xl/workbook.xml")?;
            }
            "native-diagram"
                if tags
                    .iter()
                    .filter(|tag| tag.is(PNS, b"sp") || tag.is(PNS, b"cxnSp"))
                    .count()
                    < 2 =>
            {
                return Err(
                    "assistant_presentation_plan_invalid: editable diagram objects are missing"
                        .into(),
                );
            }
            _ => (),
        }
    }
    Ok(())
}

/// Caller has already checked owner/generation. Resolves only store IDs;
/// a filename or renderer path never authorizes a local filesystem read.
pub(super) fn validate_references(
    store: &AssistantStore,
    sid: &str,
    references: &[super::types::FileReference],
) -> Result<(), String> {
    for reference in references {
        let request = FileRequest {
            session_id: sid.into(),
            expected_owner: String::new(),
            expected_generation: 0,
            attachment_id: reference.attachment_id.clone(),
        };
        let (file, bytes, version) = read_file(store, &request)?;
        if version != reference.version {
            return Err(VERSION_CHANGED.into());
        }
        if let Some(page) = reference.page {
            let total = if file.mime_type == PPTX {
                deck_info(&bytes)?.total_pages
            } else {
                1
            };
            if page == 0 || page > total {
                return Err("assistant_preview_page_invalid".into());
            }
        }
    }
    Ok(())
}

fn html_missing_resources(html: &str) -> Vec<String> {
    // This is diagnostic discovery only. The HTTP response CSP + opaque
    // iframe sandbox enforce access even for dynamically constructed URLs.
    static ATTR: OnceLock<regex::Regex> = OnceLock::new();
    static CSS: OnceLock<regex::Regex> = OnceLock::new();
    let attr = ATTR.get_or_init(|| {
        regex::Regex::new(r#"(?is)\b(?:src|href|poster)\s*=\s*["']([^"']+)["']"#).unwrap()
    });
    let css = CSS.get_or_init(|| {
        regex::Regex::new(r#"(?is)(?:url\(\s*["']?|@import\s+["'])([^\s"'()<>;]+)"#).unwrap()
    });
    let mut missing = BTreeSet::new();
    for expression in [attr, css] {
        for capture in expression.captures_iter(html) {
            let value = capture[1].trim();
            if !value.starts_with('#') && !value.starts_with("data:") && !value.is_empty() {
                missing.insert(value.chars().take(500).collect::<String>());
                if missing.len() >= 100 {
                    break;
                }
            }
        }
    }
    missing.into_iter().collect()
}

pub(super) fn read_file(
    store: &AssistantStore,
    request: &FileRequest,
) -> Result<(Attachment, Vec<u8>, String), String> {
    validate_id(&request.session_id)?;
    validate_id(&request.attachment_id)?;
    let record = store.load_record(&request.session_id)?;
    if record.session.deleted_at.is_some() {
        return Err("assistant_session_not_found".into());
    }
    let file = record
        .attachments
        .into_iter()
        .find(|f| {
            f.id == request.attachment_id
                && f.status != "missing"
                && matches!(f.mime_type.as_str(), PPTX | "text/html")
        })
        .ok_or("assistant_attachment_not_found")?;
    let root = store.attachment_root(&request.session_id)?;
    if !std::fs::symlink_metadata(&root).is_ok_and(|m| m.is_dir()) {
        return Err(ERROR.into());
    }
    let (handle, size) = attachments::open_written(&root.join(&file.id), MAX_BYTES)?;
    if size != file.size_bytes || size == 0 {
        return Err(ERROR.into());
    }
    let mut bytes = vec![];
    handle
        .take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 != size {
        return Err(ERROR.into());
    }
    let version = format!("{:x}", Sha256::digest(&bytes));
    Ok((file, bytes, version))
}
impl AssistantHost {
    async fn preview_identity(&self, request: &FileRequest) -> Result<Identity, String> {
        let identity = self.identity().await?;
        if identity.owner != request.expected_owner
            || self.generation() != request.expected_generation
        {
            return Err(IDENTITY_CHANGED.into());
        }
        Ok(identity)
    }
    async fn preview_read(
        self: &Arc<Self>,
        request: &FileRequest,
    ) -> Result<(Identity, Attachment, Vec<u8>, String), String> {
        let identity = self.preview_identity(request).await?;
        let store = self.store(&identity.owner)?;
        let r = request.clone();
        let (file, bytes, version) = blocking(move || read_file(&store, &r)).await?;
        self.ensure_current(&identity, request.expected_generation)
            .await?;
        Ok((identity, file, bytes, version))
    }
    pub async fn file_preview(self: &Arc<Self>, request: FileRequest) -> Result<Value, String> {
        let (identity, file, bytes, version) = self.preview_read(&request).await?;
        let (kind, total, width, height, warnings) = if file.mime_type == PPTX {
            let info = blocking(move || deck_info(&bytes)).await?;
            let warnings: Vec<String> = vec![];
            ("pptx", info.total_pages, info.width, info.height, warnings)
        } else {
            let html = std::str::from_utf8(&bytes).map_err(|_| ERROR)?;
            ("html", 1, 1280, 720, html_missing_resources(html))
        };
        self.ensure_current(&identity, request.expected_generation)
            .await?;
        Ok(
            json!({"attachmentId":file.id,"name":file.name,"version":version,"kind":kind,"totalPages":total,"width":width,"height":height,"readyPages":[],"limits":{"maxPagesPerRequest":1,"maxPage":MAX_RENDER_PAGE},"warnings":warnings}),
        )
    }
    pub async fn file_preview_page(
        self: &Arc<Self>,
        request: PageRequest,
    ) -> Result<Value, String> {
        if !(1..=MAX_RENDER_PAGE).contains(&request.page) || !matches!(request.scale, 1 | 2) {
            return Err("assistant_preview_limit".into());
        }
        let _turn = PAGE_TURN.acquire().await.map_err(|_| ERROR)?;
        let (identity, file, bytes, version) = self.preview_read(&request.file).await?;
        if version != request.version {
            return Err(VERSION_CHANGED.into());
        }
        if file.mime_type != PPTX {
            return Err(ERROR.into());
        }
        let info = deck_info(&bytes)?;
        if request.page > info.total_pages {
            return Err("assistant_preview_page_invalid".into());
        }
        let temp = tempfile::Builder::new()
            .prefix("ofox-file-preview-")
            .tempdir()
            .map_err(|_| ERROR)?;
        let folder = temp.path().canonicalize().map_err(|_| ERROR)?;
        let source = folder.join("source.pptx");
        crate::config::atomic_write_with(
            &source,
            &bytes,
            crate::config::AtomicOpts { mode: Some(0o600) },
        )
        .map_err(|_| ERROR)?;
        let target = folder.join("render.pptx");
        let (source_copy, target_copy) = (source.clone(), target.clone());
        let slide_path = info.slide_paths[(request.page - 1) as usize].clone();
        let math = blocking(move || {
            super::documents::math_preview::make_page_copy(&source_copy, &target_copy, &slide_path)
        })
        .await?;
        let render_file = if math.uses_fallback {
            "render.pptx"
        } else {
            "source.pptx"
        };
        let runtime = self.env.runtime_dir()?.canonicalize().map_err(|_| ERROR)?;
        let office = self
            .engines()
            .officecli
            .ok_or(ERROR)?
            .canonicalize()
            .map_err(|_| ERROR)?;
        let home = folder.join("home");
        crate::config::create_private_dir_all(&home).map_err(|_| ERROR)?;
        let at = request.page.to_string();
        let args = ["view", render_file, "svg", "--start", &at, "--end", &at];
        let output = if let Some(sandbox) = self.env.sandbox_exec() {
            let doc = format!("DOC={}", folder.display());
            let engine = format!("ENGINE={}", runtime.display());
            let exe = format!("EXE={}", office.display());
            let executable = office.to_string_lossy();
            let wrapped = [
                &[
                    "-D",
                    &doc,
                    "-D",
                    &engine,
                    "-D",
                    &exe,
                    "-p",
                    super::documents::create::PYTHON_SANDBOX,
                    &executable,
                ][..],
                &args,
            ]
            .concat();
            super::documents::runner::run_engine_limited(
                &sandbox,
                &wrapped,
                &folder,
                &home,
                Duration::from_secs(60),
                super::documents::runner::SVG_OUTPUT_LIMIT,
            )
            .await?
        } else {
            // New external-file preview fails closed without filesystem
            // isolation. Existing generator behavior is unchanged.
            return Err("assistant_preview_sandbox_unavailable".into());
        };
        if output.status != Some(0)
            || output.stdout.len() + 4 >= super::documents::runner::SVG_OUTPUT_LIMIT
        {
            return Err(ERROR.into());
        }
        let input = json!({"page":request.page,"scale":request.scale,"svg":output.stdout});
        crate::config::atomic_write_with(
            &folder.join("file-preview-input.json"),
            &serde_json::to_vec(&input).map_err(|_| ERROR)?,
            crate::config::AtomicOpts { mode: Some(0o600) },
        )
        .map_err(|_| ERROR)?;
        let loaded = super::host::runtime_at(&runtime)?;
        let helper = loaded
            .entry
            .parent()
            .ok_or(ERROR)?
            .join("file-preview-page.mjs");
        let sandbox = self
            .env
            .sandbox_exec()
            .ok_or("assistant_preview_sandbox_unavailable")?;
        let node = loaded.node.canonicalize().map_err(|_| ERROR)?;
        let doc = format!("DOC={}", folder.display());
        let engine = format!("ENGINE={}", runtime.display());
        let exe = format!("EXE={}", node.display());
        let node_text = node.to_string_lossy();
        let helper_text = helper.to_string_lossy();
        let output = super::documents::runner::run_engine_limited(
            &sandbox,
            &[
                "-D",
                &doc,
                "-D",
                &engine,
                "-D",
                &exe,
                "-p",
                super::documents::create::PYTHON_SANDBOX,
                &node_text,
                &helper_text,
            ],
            &folder,
            &home,
            Duration::from_secs(60),
            8192,
        )
        .await?;
        if output.status != Some(0) {
            return Err(ERROR.into());
        }
        let metadata: Value = serde_json::from_str(&output.stdout).map_err(|_| ERROR)?;
        let (handle, size) =
            attachments::open_written(&folder.join("file-preview.jpg"), 2 * 1024 * 1024)?;
        let mut raster = vec![];
        handle
            .take(size + 1)
            .read_to_end(&mut raster)
            .map_err(|_| ERROR)?;
        if raster.len() as u64 != size
            || !raster.starts_with(&[255, 216])
            || !raster.ends_with(&[255, 217])
        {
            return Err(ERROR.into());
        }
        // Re-read the authoritative attachment after both child processes.
        let (_, _, _, current) = self.preview_read(&request.file).await?;
        if current != version {
            return Err(VERSION_CHANGED.into());
        }
        self.ensure_current(&identity, request.file.expected_generation)
            .await?;
        Ok(
            json!({"attachmentId":file.id,"version":version,"page":request.page,"mimeType":"image/jpeg","dataBase64":STANDARD.encode(raster),"width":metadata["width"],"height":metadata["height"],"mathFallback":math.uses_fallback,"mathUnverified":math.unverified}),
        )
    }
    pub async fn file_preview_html(
        self: &Arc<Self>,
        request: VersionRequest,
    ) -> Result<Value, String> {
        let (identity, file, bytes, version) = self.preview_read(&request.file).await?;
        if version != request.version {
            return Err(VERSION_CHANGED.into());
        }
        if file.mime_type != "text/html" || bytes.len() > 4 * 1024 * 1024 {
            return Err("assistant_preview_limit".into());
        }
        let html = String::from_utf8(bytes).map_err(|_| ERROR)?;
        let missing = html_missing_resources(&html);
        let server = html_server()?;
        let token = {
            let mut leases = server.lock().map_err(|_| ERROR)?;
            leases.retain(|_, lease| {
                let Some(host) = lease.host.upgrade() else {
                    return false;
                };
                lease.created.elapsed() < Duration::from_secs(300)
                    && host.generation() == lease.request.expected_generation
                    && (!Arc::ptr_eq(&host, self) || lease.identity == identity)
            });
            let existing = leases.iter_mut().find(|(_, lease)| {
                lease.host.ptr_eq(&Arc::downgrade(self))
                    && lease.identity == identity
                    && lease.request.session_id == request.file.session_id
                    && lease.request.attachment_id == file.id
                    && lease.version == version
            });
            if let Some((token, lease)) = existing {
                lease.created = Instant::now();
                token.clone()
            } else {
                if leases.len() >= 8 {
                    return Err("assistant_preview_limit".into());
                }
                let token = super::types::id().replace('-', "");
                let html = html_reporter(&html, &token)?;
                leases.insert(
                    token.clone(),
                    HtmlLease {
                        host: Arc::downgrade(self),
                        identity: identity.clone(),
                        request: request.file.clone(),
                        version: version.clone(),
                        html,
                        created: Instant::now(),
                    },
                );
                token
            }
        };
        self.ensure_current(&identity, request.file.expected_generation)
            .await?;
        Ok(
            json!({"attachmentId":file.id,"version":version,"url":format!("http://127.0.0.1:{HTML_PORT}/{token}/index.html"),"missingResources":missing}),
        )
    }
}

struct HtmlLease {
    host: Weak<AssistantHost>,
    identity: Identity,
    request: FileRequest,
    version: String,
    html: String,
    created: Instant,
}
type HtmlLeases = Arc<Mutex<HashMap<String, HtmlLease>>>;
fn html_reporter(html: &str, token: &str) -> Result<String, String> {
    let token_json = serde_json::to_string(token).map_err(|_| ERROR)?;
    let reporter = format!(
        r#"<script>(()=>{{let n=0;const report=m=>{{if(n++<5)parent.postMessage({{type:'ofox-html-preview-error',token:{token_json},message:String(m).slice(0,500)}},'*')}};addEventListener('error',e=>report(e.message||'Resource failed to load'),true);addEventListener('unhandledrejection',()=>report('Page interaction failed'));addEventListener('securitypolicyviolation',e=>report('Resource blocked: '+e.violatedDirective));}})();</script>"#
    );
    let head = regex::Regex::new(r"(?i)<head(?:\s[^>]*)?>").map_err(|_| ERROR)?;
    Ok(match head.find(html) {
        Some(found) => format!(
            "{}{}{}",
            &html[..found.end()],
            reporter,
            &html[found.end()..]
        ),
        None => format!("{reporter}{html}"),
    })
}
fn html_server() -> Result<HtmlLeases, String> {
    static SERVER: OnceLock<Mutex<Option<HtmlLeases>>> = OnceLock::new();
    html_server_at(SERVER.get_or_init(|| Mutex::new(None)), HTML_PORT)
}
// Failed initialization is not cached: a temporarily occupied port can be
// retried after it is released. A mutex serializes bind and publish.
fn html_server_at(slot: &Mutex<Option<HtmlLeases>>, port: u16) -> Result<HtmlLeases, String> {
    let mut current = slot.lock().map_err(|_| ERROR)?;
    if let Some(server) = current.as_ref() {
        return Ok(server.clone());
    }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .map_err(|_| "assistant_preview_port_unavailable")?;
    let leases: HtmlLeases = Arc::new(Mutex::new(HashMap::new()));
    let state = leases.clone();
    let handle = tokio::runtime::Handle::current();
    std::thread::Builder::new()
        .name("ofox-isolated-html".into())
        .spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let response = html_request(&mut stream, &state, &handle);
                let _ = stream.write_all(response.as_bytes());
            }
        })
        .map_err(|_| ERROR)?;
    *current = Some(leases.clone());
    Ok(leases)
}
fn html_request(
    stream: &mut TcpStream,
    state: &HtmlLeases,
    runtime: &tokio::runtime::Handle,
) -> String {
    let mut raw = vec![];
    let mut buffer = [0u8; 1024];
    while raw.len() < 8192 {
        let Ok(n) = stream.read(&mut buffer) else {
            return http_html(400, "");
        };
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..n]);
        if raw.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let Ok(text) = std::str::from_utf8(&raw) else {
        return http_html(400, "");
    };
    let mut lines = text.split("\r\n");
    let Some(line) = lines.next() else {
        return http_html(400, "");
    };
    let words: Vec<_> = line.split(' ').collect();
    if words.len() != 3 || words[0] != "GET" || words[2] != "HTTP/1.1" {
        return http_html(405, "");
    }
    let expected_host = format!("127.0.0.1:{HTML_PORT}");
    let hosts: Vec<_> = lines
        .clone()
        .filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| key.eq_ignore_ascii_case("host"))
        .collect();
    if hosts.len() != 1 || hosts[0].1.trim() != expected_host {
        return http_html(403, "");
    }
    let parts: Vec<_> = words[1].split('/').collect();
    if parts.len() != 3
        || !parts[0].is_empty()
        || parts[2] != "index.html"
        || parts[1].len() != 32
        || !parts[1].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return http_html(404, "");
    }
    let Ok(leases) = state.lock() else {
        return http_html(503, "");
    };
    let Some(lease) = leases
        .get(parts[1])
        .filter(|l| l.created.elapsed() < Duration::from_secs(300))
    else {
        return http_html(404, "");
    };
    let Some(host) = lease.host.upgrade() else {
        return http_html(403, "");
    };
    if runtime
        .block_on(host.ensure_current(&lease.identity, lease.request.expected_generation))
        .is_err()
    {
        return http_html(403, "");
    }
    let Ok(store) = host.store(&lease.identity.owner) else {
        return http_html(403, "");
    };
    if !read_file(&store, &lease.request).is_ok_and(|(_, _, v)| v == lease.version) {
        return http_html(404, "");
    }
    if runtime
        .block_on(host.ensure_current(&lease.identity, lease.request.expected_generation))
        .is_err()
    {
        return http_html(403, "");
    }
    http_html(200, &lease.html)
}
fn http_html(status: u16, body: &str) -> String {
    let failure;
    let body = if status == 200 {
        body
    } else {
        failure = format!(
            r#"<!doctype html><meta charset="utf-8"><main role="alert">assistant_preview_unavailable (HTTP {status})</main><script>parent.postMessage({{type:'ofox-html-preview-error',token:location.pathname.split('/')[1],message:'assistant_preview_unavailable (HTTP {status})'}},'*')</script>"#
        );
        &failure
    };
    format!("HTTP/1.1 {status} Preview\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: {HTML_CSP}\r\nPermissions-Policy: camera=(), microphone=(), geolocation=(), clipboard-read=(), clipboard-write=()\r\nConnection: close\r\n\r\n{body}",body.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn deck(pages: u32) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        let mut presentation = String::from("<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><p:sldIdLst>");
        let mut rels = String::from("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
        let mut content_types = String::from("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>");
        // IDs and physical filenames deliberately run backwards: page order
        // belongs to presentation.xml, never a directory listing.
        for n in 1..=pages {
            let physical = pages + 1 - n;
            content_types.push_str(&format!("<Override PartName=\"/ppt/slides/slide{physical}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"));
            presentation.push_str(&format!("<p:sldId id=\"{}\" r:id=\"r{n}\"/>", 256 + n));
            rels.push_str(&format!("<Relationship Id=\"r{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{physical}.xml\"/>"));
            zip.start_file(format!("ppt/slides/slide{physical}.xml"), opts)
                .unwrap();
            zip.write_all(b"<p:sld/>").unwrap();
        }
        presentation
            .push_str("</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/></p:presentation>");
        rels.push_str("</Relationships>");
        content_types.push_str("</Types>");
        for (name, text) in [
            ("ppt/presentation.xml", presentation),
            ("ppt/_rels/presentation.xml.rels", rels),
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"main\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>".into()),
        ] {
            zip.start_file(name, opts).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn rewrite(bytes: Vec<u8>, member: &str, change: impl Fn(String) -> String) -> Vec<u8> {
        let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for n in 0..input.len() {
            let mut file = input.by_index(n).unwrap();
            let name = file.name().unwrap().into_owned();
            let mut data = vec![];
            file.read_to_end(&mut data).unwrap();
            if name == member {
                data = change(String::from_utf8(data).unwrap()).into_bytes();
            }
            output
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            output.write_all(&data).unwrap();
        }
        output.finish().unwrap().into_inner()
    }
    #[test]
    fn only_the_correct_namespace_and_direct_slide_list_determine_page_order() {
        let changed = rewrite(deck(2), "ppt/presentation.xml", |xml| {
            xml.replace(
                "http://schemas.openxmlformats.org/presentationml/2006/main",
                "urn:forged",
            )
        });
        assert!(deck_info(&changed).is_err());
        let aliases = rewrite(deck(2), "ppt/presentation.xml", |xml| {
            xml.replace("xmlns:r=", "xmlns:relationship=")
                .replace(" r:id=", " relationship:id=")
        });
        assert_eq!(deck_info(&aliases).unwrap().total_pages, 2);
        let nested = rewrite(deck(2), "ppt/presentation.xml", |xml| {
            xml.replace("</p:presentation>","<p:extLst><p:ext><p:sldIdLst><p:sldId id=\"999\" r:id=\"r1\"/></p:sldIdLst></p:ext></p:extLst></p:presentation>")
        });
        assert_eq!(deck_info(&nested).unwrap().total_pages, 2);
    }
    #[test]
    fn actual_page_and_html_commands_accept_the_flat_renderer_contract() {
        let value = json!({"sessionId":super::super::types::id(),"attachmentId":super::super::types::id(),"expectedOwner":"ab".repeat(32),"expectedGeneration":1,"version":"a".repeat(64),"page":70,"scale":2});
        let request: PageRequest = serde_json::from_value(value).unwrap();
        assert_eq!((request.page, request.scale), (70, 2));
        let value = json!({"sessionId":request.file.session_id,"attachmentId":request.file.attachment_id,"expectedOwner":request.file.expected_owner,"expectedGeneration":1,"version":request.version});
        let _: VersionRequest = serde_json::from_value(value).unwrap();
    }
    fn stored_deck() -> (tempfile::TempDir, AssistantStore, FileRequest) {
        let temp = tempfile::tempdir().unwrap();
        let owner = "ab".repeat(32);
        let store = AssistantStore::for_owner(temp.path(), &owner).unwrap();
        let sid = store.create("test/chat", false).unwrap().session.id;
        let aid = super::super::types::id();
        let bytes = deck(70);
        let file:Attachment=serde_json::from_value(json!({"id":aid,"name":"70-pages.pptx","mimeType":PPTX,"sizeBytes":bytes.len(),"status":"ready"})).unwrap();
        let root = store.attachment_dir(&sid).unwrap();
        std::fs::write(root.join(&aid), bytes).unwrap();
        store
            .mutate(&sid, |record| {
                record.attachments.push(file);
                Ok(())
            })
            .unwrap();
        (
            temp,
            store,
            FileRequest {
                session_id: sid,
                attachment_id: aid,
                expected_owner: owner,
                expected_generation: 0,
            },
        )
    }
    #[test]
    fn uploaded_deck_references_bind_full_hash_and_real_page_before_model_admission() {
        let (_temp, store, request) = stored_deck();
        let (_, _, version) = read_file(&store, &request).unwrap();
        let reference = super::super::types::FileReference {
            attachment_id: request.attachment_id.clone(),
            version: version.clone(),
            page: Some(70),
        };
        assert!(validate_references(
            &store,
            &request.session_id,
            std::slice::from_ref(&reference)
        )
        .is_ok());
        let invalid = super::super::types::FileReference {
            page: Some(71),
            ..reference.clone()
        };
        assert_eq!(
            validate_references(&store, &request.session_id, &[invalid]).unwrap_err(),
            "assistant_preview_page_invalid"
        );
        let stale = super::super::types::FileReference {
            version: "0".repeat(64),
            ..reference
        };
        assert_eq!(
            validate_references(&store, &request.session_id, &[stale]).unwrap_err(),
            VERSION_CHANGED
        );
        store
            .mutate(&request.session_id, |record| {
                record.session.deleted_at = Some(1);
                Ok(())
            })
            .unwrap();
        assert!(read_file(&store, &request).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn preview_never_follows_an_attachment_symlink() {
        let (_temp, store, request) = stored_deck();
        let root = store.attachment_root(&request.session_id).unwrap();
        let original = root.join(&request.attachment_id);
        let outside = _temp.path().join("outside.pptx");
        std::fs::write(&outside, deck(70)).unwrap();
        std::fs::remove_file(&original).unwrap();
        std::os::unix::fs::symlink(&outside, &original).unwrap();
        assert!(read_file(&store, &request).is_err());
        assert!(outside.is_file());
    }
    #[test]
    fn html_failures_are_visible_and_share_no_app_authority() {
        let response = http_html(403, "");
        assert!(response.contains("sandbox allow-scripts"));
        assert!(!response.contains("allow-same-origin"));
        assert!(!response.contains("Access-Control-Allow-Origin"));
        assert!(response.contains("connect-src 'none'"));
        assert!(response.contains("role=\"alert\""));
        assert!(response.contains("ofox-html-preview-error"));
    }

    #[test]
    fn long_deck_reports_true_order_and_all_seventy_pages() {
        let found = deck_info(&deck(70)).expect("a real 70-page PPTX is supported");
        assert_eq!(found.total_pages, 70);
        assert_eq!(found.slide_paths.first().unwrap(), "ppt/slides/slide70.xml");
        assert_eq!(found.slide_paths.last().unwrap(), "ppt/slides/slide1.xml");
        assert_eq!((found.width, found.height), (1280, 720));
    }

    #[test]
    fn html_external_dependencies_are_reported_instead_of_blank_success() {
        let source = r##"<!doctype html><link href='css/theme.css' rel='stylesheet'><script src="https://cdn.example.test/chart.js"></script><style>@import url('fonts.css'); .x{background:url(images/a.png)}</style><img src="data:image/png;base64,AA=="><a href="#slide2">Next</a>"##;
        assert_eq!(
            html_missing_resources(source),
            vec![
                "css/theme.css",
                "fonts.css",
                "https://cdn.example.test/chart.js",
                "images/a.png"
            ]
        );
    }
}

#[cfg(test)]
#[path = "file_preview/e2e.rs"]
mod e2e;
