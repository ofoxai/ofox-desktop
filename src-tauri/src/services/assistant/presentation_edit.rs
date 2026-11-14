//! Explicit source-deck editing. Snapshots and dependent parts stay private;
//! a revision is a new attachment and the original is never overwritten.
use super::{
    file_preview::{self, FileRequest},
    host::{blocking, AssistantHost, RunControl, STATE},
    store::{AssistantStore, SessionRecord},
    types::FileReference,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
const INVALID: &str = "assistant_presentation_plan_invalid";
#[derive(Clone)]
pub(super) struct SourceDocument {
    pub attachment_id: String,
    pub version: String,
    pub total_pages: u32,
    pub page_numbers: Vec<u32>,
    pub style_id: Option<String>,
    pub bytes: Arc<Vec<u8>>,
}
impl SourceDocument {
    pub fn meta(&self) -> Value {
        json!({"version":self.version,"totalPages":self.total_pages,"pageNumbers":self.page_numbers})
    }
    pub fn revision(&self, id: &str) -> Value {
        json!({"parentAttachmentId":self.attachment_id,"parentVersion":self.version,"pageNumbers":self.page_numbers,"revisionId":id})
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceRequest {
    attachment_id: String,
    version: String,
    page_numbers: Vec<u32>,
}

pub(super) fn style_for_file(record: &SessionRecord, id: &str) -> Option<String> {
    let origin = record
        .attachments
        .iter()
        .find(|file| file.id == id)?
        .source_message_id
        .as_ref()?;
    let reply = record
        .messages
        .iter()
        .find(|message| &message.id == origin)?;
    let known = reply
        .presentation_plan
        .as_ref()
        .and_then(|plan| plan["effectiveStyle"].as_str())
        .map(str::to_owned)
        .or_else(|| {
            record
                .messages
                .iter()
                .find(|message| {
                    message.role == "user"
                        && message.run_id.is_some()
                        && message.run_id == reply.run_id
                })
                .and_then(|message| message.presentation_style_id.clone())
        });
    known.filter(|style| {
        style != "auto" && super::types::validate_presentation_style(Some(style)).is_ok()
    })
}
pub(super) fn source_metadata(
    store: &AssistantStore,
    record: &SessionRecord,
    references: &[FileReference],
) -> Result<Vec<Value>, String> {
    let mut ids = BTreeSet::new();
    let mut result = vec![];
    for reference in references {
        if !ids.insert(reference.attachment_id.clone()) {
            continue;
        }
        let request = FileRequest {
            session_id: record.session.id.clone(),
            attachment_id: reference.attachment_id.clone(),
            expected_owner: String::new(),
            expected_generation: 0,
        };
        let (file, bytes, version) = file_preview::read_file(store, &request)?;
        if version != reference.version {
            return Err("assistant_preview_version_changed".into());
        }
        if file.mime_type != PPTX {
            continue;
        }
        let info = file_preview::deck_info(&bytes)?;
        let refs: Vec<_> = references
            .iter()
            .filter(|item| item.attachment_id == file.id)
            .collect();
        let allowed = if refs.iter().any(|item| item.page.is_none()) {
            None
        } else {
            Some(
                refs.iter()
                    .filter_map(|item| item.page)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>(),
            )
        };
        result.push(json!({"attachmentId":file.id,"version":version,"totalPages":info.total_pages,"allowedPages":allowed,"styleId":style_for_file(record,&reference.attachment_id)}));
    }
    Ok(result)
}
impl AssistantHost {
    pub(super) async fn prepare_presentation_source(
        &self,
        run: &Arc<RunControl>,
        source: Option<&Value>,
    ) -> Result<Option<SourceDocument>, String> {
        let Some(source) = source else {
            return Ok(None);
        };
        let source: SourceRequest = serde_json::from_value(source.clone()).map_err(|_| INVALID)?;
        super::types::validate_id(&source.attachment_id)?;
        if source.page_numbers.is_empty()
            || source.page_numbers.len() > 30
            || source
                .page_numbers
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != source.page_numbers.len()
        {
            return Err(INVALID.into());
        }
        if source.page_numbers.iter().any(|page| {
            !run.file_references.iter().any(|reference| {
                reference.attachment_id == source.attachment_id
                    && reference.version == source.version
                    && (reference.page.is_none() || reference.page == Some(*page))
            })
        }) {
            return Err("assistant_presentation_source_unavailable".into());
        }
        let record = self.load(run).await?;
        let style_id = style_for_file(&record, &source.attachment_id);
        let store = self.store(&run.identity.owner)?;
        let request = FileRequest {
            session_id: run.session_id.clone(),
            attachment_id: source.attachment_id.clone(),
            expected_owner: run.identity.owner.clone(),
            expected_generation: self.generation(),
        };
        let source = blocking(move || {
            let (file, bytes, version) = file_preview::read_file(&store, &request)?;
            if file.mime_type != PPTX || version != source.version {
                return Err("assistant_preview_version_changed".into());
            }
            let info = file_preview::deck_info(&bytes)?;
            if info.total_pages > 1000
                || source
                    .page_numbers
                    .iter()
                    .any(|page| *page == 0 || *page > info.total_pages)
            {
                return Err("assistant_preview_limit".into());
            }
            Ok(SourceDocument {
                attachment_id: file.id,
                version,
                total_pages: info.total_pages,
                page_numbers: source.page_numbers,
                style_id,
                bytes: Arc::new(bytes),
            })
        })
        .await?;
        self.validate_run(run)?;
        if self.identity().await? != run.identity {
            return Err(super::host::IDENTITY_CHANGED.into());
        }
        Ok(Some(source))
    }
    pub(super) fn source_snapshot(run: &RunControl) -> Result<Option<SourceDocument>, String> {
        Ok(run.presentation_source.lock().map_err(|_| STATE)?.clone())
    }
}

// A source part may be a large image. ZIP sizes were bounded by deck_info;
// read only one bounded member at a time and retain fingerprints, not bytes.
fn member(
    zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut file = zip.by_name(name).map_err(|_| INVALID)?;
    if file.size() > 32 * 1024 * 1024 {
        return Err(INVALID.into());
    }
    let size = file.size();
    let mut bytes = vec![];
    file.by_ref()
        .take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| INVALID)?;
    if bytes.len() as u64 != file.size() {
        return Err(INVALID.into());
    }
    Ok(bytes)
}
fn xml_fingerprint(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use quick_xml::{events::Event, name::ResolveResult, NsReader};
    let mut reader = NsReader::from_reader(bytes);
    let mut items = vec![];
    let mut parents: Vec<Vec<u8>> = vec![];
    let namespace = |resolved: ResolveResult<'_>| match resolved {
        ResolveResult::Bound(ns) => Ok(ns.as_ref().to_vec()),
        ResolveResult::Unbound => Ok(vec![]),
        ResolveResult::Unknown(_) => Err(INVALID.to_string()),
    };
    loop {
        let (resolved, event) = reader.read_resolved_event().map_err(|_| INVALID)?;
        let ns = namespace(resolved)?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(tag) | Event::Empty(tag) => {
                if parents.len() >= 128 || items.len() > 200_000 {
                    return Err(INVALID.into());
                }
                let name = tag.local_name().as_ref().to_vec();
                let mut attrs = BTreeMap::new();
                for attr in tag.attributes() {
                    let attr = attr.map_err(|_| INVALID)?;
                    if attr.key.as_ref() == b"xmlns" || attr.key.as_ref().starts_with(b"xmlns:") {
                        // MC attributes such as Requires/ Ignorable refer to
                        // prefixes in values, even when elements use another
                        // prefix. Preserve declarations conservatively.
                        attrs.insert(
                            (
                                b"namespace-declaration".to_vec(),
                                attr.key.as_ref().to_vec(),
                            ),
                            attr.decode_and_unescape_value(reader.decoder())
                                .map_err(|_| INVALID)?
                                .into_owned(),
                        );
                        continue;
                    }
                    let (scope, local) = reader.resolve_attribute(attr.key);
                    attrs.insert(
                        (namespace(scope)?, local.as_ref().to_vec()),
                        attr.decode_and_unescape_value(reader.decoder())
                            .map_err(|_| INVALID)?
                            .into_owned(),
                    );
                }
                items.push(json!([
                    "start",
                    ns,
                    name,
                    attrs.into_iter().collect::<Vec<_>>()
                ]));
                if empty {
                    items.push(json!(["end"]));
                } else {
                    parents.push(name);
                }
            }
            Event::End(_) => {
                parents.pop().ok_or(INVALID)?;
                items.push(json!(["end"]));
            }
            Event::Text(text) => {
                let text = text.decode().map_err(|_| INVALID)?;
                let text = quick_xml::escape::unescape(&text).map_err(|_| INVALID)?;
                if !text.trim().is_empty() || parents.last().is_some_and(|name| name == b"t") {
                    items.push(json!(["text", text]));
                }
            }
            Event::CData(text) => items.push(json!(["text", text.decode().map_err(|_| INVALID)?])),
            Event::DocType(_) => return Err(INVALID.into()),
            Event::Eof => break,
            _ => (),
        }
    }
    serde_json::to_vec(&items).map_err(|_| INVALID.into())
}
fn fingerprint(bytes: &[u8], name: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let content = if name.ends_with(".xml") {
        xml_fingerprint(bytes)?
    } else if name.ends_with(".rels") {
        let mut relationships = BTreeMap::new();
        for tag in file_preview::xml_tags(bytes)? {
            if tag.is(
                b"http://schemas.openxmlformats.org/package/2006/relationships",
                b"Relationship",
            ) {
                relationships.insert(
                    tag.attr(b"Id").ok_or(INVALID)?.clone(),
                    (
                        tag.attr(b"Type").cloned(),
                        tag.attr(b"Target").cloned(),
                        tag.attr(b"TargetMode").cloned(),
                    ),
                );
            }
        }
        serde_json::to_vec(&relationships).map_err(|_| INVALID)?
    } else {
        bytes.to_vec()
    };
    Ok(format!("{:x}", Sha256::digest(content)))
}
pub(super) fn validate_preserved_source(
    source: &SourceDocument,
    output: &std::path::Path,
) -> Result<(), String> {
    use std::io::Read;
    let (file, size) = super::attachments::open_written(output, 25 * 1024 * 1024)?;
    let mut bytes = vec![];
    file.take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| INVALID)?;
    if bytes.len() as u64 != size {
        return Err(INVALID.into());
    }
    let original_info = file_preview::deck_info(&source.bytes)?;
    let output_info = file_preview::deck_info(&bytes)?;
    if original_info.slide_paths != output_info.slide_paths {
        return Err("assistant_presentation_source_changed".into());
    }
    let mut original =
        crate::zip_security::open(std::io::Cursor::new(source.bytes.as_slice()), 40_000)
            .map_err(|_| INVALID)?;
    let mut output = crate::zip_security::open(std::io::Cursor::new(bytes.as_slice()), 40_000)
        .map_err(|_| INVALID)?;
    file_preview::opc::validate_preserved_package(&mut original, &mut output)
        .map_err(|_| "assistant_presentation_source_changed")?;
    let main = original_info.presentation_path.clone();
    let main_rels = file_preview::opc::relationships_path(&main);
    let target_paths: BTreeSet<_> = source
        .page_numbers
        .iter()
        .map(|page| original_info.slide_paths[(*page - 1) as usize].clone())
        .collect();
    let mut queue = vec![main.clone(), main_rels.clone()];
    queue.extend(
        original_info
            .slide_paths
            .iter()
            .enumerate()
            .filter(|(index, _)| !source.page_numbers.contains(&(*index as u32 + 1)))
            .map(|(_, path)| path.clone()),
    );
    let mut seen = BTreeSet::new();
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let old = member(&mut original, &name)?;
        let new = member(&mut output, &name)?;
        if fingerprint(&old, &name)? != fingerprint(&new, &name)? {
            return Err("assistant_presentation_source_changed".into());
        }
        // presentation.xml is checked for canvas, IDs and order, but its
        // relationships include the target slides. Walk only from each
        // unchanged page; that also protects shared masters/charts/images.
        if name == main || name == main_rels || name.ends_with(".rels") {
            continue;
        }
        let rels = file_preview::opc::relationships_path(&name);
        if original.by_name(&rels).is_err() {
            continue;
        }
        let content = member(&mut original, &rels)?;
        for tag in file_preview::xml_tags(&content)? {
            if tag.is(
                b"http://schemas.openxmlformats.org/package/2006/relationships",
                b"Relationship",
            ) && tag
                .attr(b"TargetMode")
                .is_none_or(|value| value != "External")
            {
                let target =
                    file_preview::opc::target_path(&name, tag.attr(b"Target").ok_or(INVALID)?)?;
                // Preserve the navigation relationship itself, while allowing
                // its explicitly selected destination slide to change.
                if tag.attr(b"Type").is_some_and(|kind| kind == "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide")
                    && target_paths.contains(&target) {
                    continue;
                }
                queue.push(target);
            }
        }
        queue.push(rels);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, io::Write};

    const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const MAIN: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";
    const PNS: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
    const ANS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

    // Independently authored OPC-only fixtures: no user documents or media.
    // The non-default main path and explicit navigation distinguish package
    // identity from the conventional ppt/presentation.xml location.
    fn source_parts() -> BTreeMap<String, Vec<u8>> {
        let mut parts = BTreeMap::new();
        let texts = [
            ("_rels/.rels", format!("<Relationships xmlns=\"{PKG}\"><Relationship Id=\"main\" Type=\"{REL}/officeDocument\" Target=\"content/actual.xml\"/></Relationships>")),
            ("[Content_Types].xml", format!("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Override PartName=\"/content/actual.xml\" ContentType=\"{MAIN}\"/><Override PartName=\"/content/pages/first.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/><Override PartName=\"/content/pages/second.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/></Types>")),
            ("content/actual.xml", format!("<p:presentation xmlns:p=\"{PNS}\" xmlns:r=\"{REL}\"><p:sldIdLst><p:sldId id=\"256\" r:id=\"first\"/><p:sldId id=\"257\" r:id=\"second\"/></p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/></p:presentation>")),
            ("content/_rels/actual.xml.rels", format!("<Relationships xmlns=\"{PKG}\"><Relationship Id=\"first\" Type=\"{REL}/slide\" Target=\"pages/first.xml\"/><Relationship Id=\"second\" Type=\"{REL}/slide\" Target=\"/content/pages/second.xml\"/></Relationships>")),
            ("content/pages/first.xml", slide("PROTECTED PAGE")),
            ("content/pages/second.xml", slide("SELECTED PAGE")),
            ("content/pages/_rels/first.xml.rels", format!("<Relationships xmlns=\"{PKG}\"><Relationship Id=\"next\" Type=\"{REL}/slide\" Target=\"../pages/second.xml\"/><Relationship Id=\"shared\" Type=\"{REL}/image\" Target=\"../media/shared.png\"/></Relationships>")),
            ("content/pages/_rels/second.xml.rels", format!("<Relationships xmlns=\"{PKG}\"><Relationship Id=\"shared\" Type=\"{REL}/image\" Target=\"../media/shared.png\"/></Relationships>")),
        ];
        for (name, text) in texts {
            parts.insert(name.into(), text.into_bytes());
        }
        parts.insert(
            "content/media/shared.png".into(),
            b"synthetic-shared-resource".to_vec(),
        );
        parts
    }

    fn slide(text: &str) -> String {
        format!("<p:sld xmlns:p=\"{PNS}\" xmlns:a=\"{ANS}\" xmlns:r=\"{REL}\"><p:cSld><p:spTree><p:sp><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>")
    }

    fn packed(parts: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in parts {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn validate(
        original: &BTreeMap<String, Vec<u8>>,
        output: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), String> {
        let bytes = packed(original);
        let source = SourceDocument {
            attachment_id: "synthetic-source".into(),
            version: format!("{:x}", Sha256::digest(&bytes)),
            total_pages: 2,
            page_numbers: vec![2],
            style_id: None,
            bytes: Arc::new(bytes),
        };
        let dir = tempfile::tempdir().unwrap();
        let output_path = dir.path().join("revision.pptx");
        std::fs::write(&output_path, packed(output)).unwrap();
        validate_preserved_source(&source, &output_path)
    }

    fn changed_target(parts: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
        let mut changed = parts.clone();
        changed.insert(
            "content/pages/second.xml".into(),
            slide("EDITED SELECTED PAGE").into_bytes(),
        );
        changed
    }

    #[test]
    fn presentation_source_nondefault_main_and_navigation_allow_only_target_change() {
        let original = source_parts();
        let info = file_preview::deck_info(&packed(&original)).unwrap();
        assert_eq!(info.presentation_path, "content/actual.xml");
        assert_eq!(
            info.slide_paths,
            ["content/pages/first.xml", "content/pages/second.xml"]
        );
        let mut output = changed_target(&original);
        // The root relationship may spell the same package part absolutely.
        output.insert(
            "_rels/.rels".into(),
            String::from_utf8(original["_rels/.rels"].clone())
                .unwrap()
                .replace("content/actual.xml", "/content/actual.xml")
                .into_bytes(),
        );
        assert!(
            validate(&original, &output).is_ok(),
            "navigation to the authorized second page must not protect its content"
        );
    }

    #[test]
    fn presentation_source_rejects_changes_to_untouched_page_and_shared_resource() {
        let original = source_parts();
        let mut page = changed_target(&original);
        page.insert(
            "content/pages/first.xml".into(),
            slide("UNAUTHORIZED CHANGE").into_bytes(),
        );
        assert_eq!(
            validate(&original, &page).unwrap_err(),
            "assistant_presentation_source_changed"
        );
        let mut shared = changed_target(&original);
        shared.insert(
            "content/media/shared.png".into(),
            b"different-shared-resource".to_vec(),
        );
        assert_eq!(
            validate(&original, &shared).unwrap_err(),
            "assistant_presentation_source_changed"
        );
        let mut navigation = changed_target(&original);
        navigation.insert("content/pages/_rels/first.xml.rels".into(), format!("<Relationships xmlns=\"{PKG}\"><Relationship Id=\"shared\" Type=\"{REL}/image\" Target=\"../media/shared.png\"/></Relationships>").into_bytes());
        assert_eq!(
            validate(&original, &navigation).unwrap_err(),
            "assistant_presentation_source_changed",
            "skipping the target content must still preserve the navigation edge"
        );
    }

    #[test]
    fn presentation_source_rejects_root_redirection_and_existing_content_type_change() {
        let original = source_parts();
        let mut root = changed_target(&original);
        root.insert(
            "content/alternate.xml".into(),
            original["content/actual.xml"].clone(),
        );
        root.insert(
            "content/_rels/alternate.xml.rels".into(),
            original["content/_rels/actual.xml.rels"].clone(),
        );
        root.insert(
            "_rels/.rels".into(),
            String::from_utf8(original["_rels/.rels"].clone())
                .unwrap()
                .replace("actual.xml", "alternate.xml")
                .into_bytes(),
        );
        root.insert("[Content_Types].xml".into(), String::from_utf8(original["[Content_Types].xml"].clone()).unwrap().replace("</Types>", &format!("<Override PartName=\"/content/alternate.xml\" ContentType=\"{MAIN}\"/></Types>")).into_bytes());
        assert_eq!(
            validate(&original, &root).unwrap_err(),
            "assistant_presentation_source_changed"
        );
        let mut types = changed_target(&original);
        types.insert(
            "[Content_Types].xml".into(),
            String::from_utf8(original["[Content_Types].xml"].clone())
                .unwrap()
                .replace("image/png", "image/jpeg")
                .into_bytes(),
        );
        assert_eq!(
            validate(&original, &types).unwrap_err(),
            "assistant_presentation_source_changed"
        );
    }

    #[test]
    fn presentation_source_rejects_namespace_binding_changes_that_select_a_different_mc_branch() {
        let mut original = source_parts();
        let slide = format!("<p:sld xmlns:p=\"{PNS}\" xmlns:a=\"{ANS}\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:a14=\"http://schemas.microsoft.com/office/drawing/2010/main\" xmlns:eq=\"http://schemas.microsoft.com/office/drawing/2010/main\" xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\"><p:cSld><p:spTree><mc:AlternateContent><mc:Choice Requires=\"a14\"><p:sp><a:p><eq:m><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath></eq:m></a:p></p:sp></mc:Choice><mc:Fallback><p:sp><p:txBody><a:p><a:r><a:t>FALLBACK</a:t></a:r></a:p></p:txBody></p:sp></mc:Fallback></mc:AlternateContent></p:spTree></p:cSld></p:sld>");
        original.insert("content/pages/first.xml".into(), slide.clone().into_bytes());
        let mut output = changed_target(&original);
        // The math elements keep the eq binding; only the QName-valued
        // Requires prefix changes meaning, so resolved element names match.
        output.insert(
            "content/pages/first.xml".into(),
            slide
                .replace(
                    "xmlns:a14=\"http://schemas.microsoft.com/office/drawing/2010/main\"",
                    "xmlns:a14=\"urn:unsupported-extension\"",
                )
                .into_bytes(),
        );
        assert_eq!(
            validate(&original, &output).unwrap_err(),
            "assistant_presentation_source_changed"
        );
    }
}
