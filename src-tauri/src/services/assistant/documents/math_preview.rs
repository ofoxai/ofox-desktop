use quick_xml::{
    events::Event,
    name::{PrefixDeclaration, ResolveResult},
    NsReader, Reader, Writer,
};
use std::{
    collections::HashSet,
    io::{Read, Write},
    path::{Component, Path},
};

#[derive(Debug, Default)]
pub(crate) struct MathPreview {
    pub uses_fallback: bool,
    pub unverified: bool,
}

const MC: &[u8] = b"http://schemas.openxmlformats.org/markup-compatibility/2006";
const MATH: &[u8] = b"http://schemas.openxmlformats.org/officeDocument/2006/math";
const A14: &[u8] = b"http://schemas.microsoft.com/office/drawing/2010/main";
const PRESENTATION: &[u8] = b"http://schemas.openxmlformats.org/presentationml/2006/main";
const DRAWING: &[u8] = b"http://schemas.openxmlformats.org/drawingml/2006/main";
const RELATIONSHIPS: &[u8] = b"http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const ERROR: &str = "assistant_preview_unavailable";
struct Alternate {
    start: usize,
    depth: usize,
    math: bool,
    choice: Option<usize>,
    fallback: Option<FallbackFragment>,
    picture: bool,
}
struct FallbackFragment {
    start: usize,
    end: usize,
    bindings: Vec<(String, String)>,
    active: bool,
    image_fill_depth: Option<usize>,
}

fn scoped_fragment(fragment: &[u8], bindings: &[(String, String)]) -> Result<Vec<u8>, String> {
    if bindings.len() > 64
        || bindings
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
            > 16 * 1024
    {
        return Err(ERROR.into());
    }
    let mut reader = Reader::from_reader(fragment);
    let mut writer = Writer::new(vec![]);
    let mut depth = 0;
    loop {
        let event = reader.read_event().map_err(|_| ERROR)?.into_owned();
        match event {
            Event::Start(mut tag) => {
                if depth == 0 {
                    add_bindings(&mut tag, bindings)?;
                }
                depth += 1;
                writer.write_event(Event::Start(tag)).map_err(|_| ERROR)?;
            }
            Event::Empty(mut tag) => {
                if depth == 0 {
                    add_bindings(&mut tag, bindings)?;
                }
                writer.write_event(Event::Empty(tag)).map_err(|_| ERROR)?;
            }
            Event::End(tag) => {
                depth -= 1;
                writer.write_event(Event::End(tag)).map_err(|_| ERROR)?;
            }
            Event::DocType(_) => return Err(ERROR.into()),
            Event::Eof => break,
            event => writer.write_event(event).map_err(|_| ERROR)?,
        }
        if writer.get_ref().len() > 4 * 1024 * 1024 {
            return Err(ERROR.into());
        }
    }
    Ok(writer.into_inner())
}
fn add_bindings(
    tag: &mut quick_xml::events::BytesStart<'_>,
    bindings: &[(String, String)],
) -> Result<(), String> {
    let existing: HashSet<Vec<u8>> = tag
        .attributes()
        .map(|a| a.map(|a| a.key.as_ref().to_vec()).map_err(|_| ERROR))
        .collect::<Result<_, _>>()?;
    for (name, value) in bindings {
        if !existing.contains(name.as_bytes()) {
            tag.push_attribute((name.as_str(), quick_xml::escape::escape(value).as_ref()));
        }
    }
    Ok(())
}

fn select_math_fallbacks(xml: &[u8]) -> Result<(Vec<u8>, MathPreview), String> {
    if xml.len() > 4 * 1024 * 1024 {
        return Err(ERROR.into());
    }
    let mut reader = NsReader::from_reader(xml);
    let mut depth = 0;
    let mut alternates: Vec<Alternate> = vec![];
    let mut changes: Vec<(usize, usize, Vec<u8>)> = vec![];
    let mut report = MathPreview::default();
    loop {
        let before = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|_| ERROR)?.into_owned();
        let after = reader.buffer_position() as usize;
        match event {
            Event::Start(ref tag) | Event::Empty(ref tag) => {
                let empty = matches!(event, Event::Empty(_));
                let at = depth + 1;
                if at > 128 {
                    return Err(ERROR.into());
                }
                let (namespace, local) = reader.resolve_element(tag.name());
                let ns = match namespace {
                    ResolveResult::Bound(ns) => ns.as_ref().to_vec(),
                    ResolveResult::Unbound => vec![],
                    ResolveResult::Unknown(_) => return Err(ERROR.into()),
                };
                let local = local.as_ref();
                if ns == MC && local == b"AlternateContent" && !empty {
                    alternates.push(Alternate {
                        start: before,
                        depth: at,
                        math: false,
                        choice: None,
                        fallback: None,
                        picture: false,
                    });
                }
                let is_math = (ns == MATH && matches!(local, b"oMath" | b"oMathPara"))
                    || (ns == A14 && local == b"m");
                let mut owned_math = false;
                for alt in &mut alternates {
                    if ns == MC && local == b"Choice" && at == alt.depth + 1 && !empty {
                        alt.choice = Some(at);
                    }
                    if is_math && alt.choice.is_some_and(|choice| at > choice) {
                        alt.math = true;
                        owned_math = true;
                    }
                    if ns == MC && local == b"Fallback" && at == alt.depth + 1 && !empty {
                        let bindings = reader
                            .prefixes()
                            .map(|(prefix, namespace)| {
                                let key = match prefix {
                                    PrefixDeclaration::Default => "xmlns".to_string(),
                                    PrefixDeclaration::Named(prefix) => {
                                        format!("xmlns:{}", String::from_utf8_lossy(prefix))
                                    }
                                };
                                (
                                    key,
                                    String::from_utf8_lossy(namespace.as_ref()).into_owned(),
                                )
                            })
                            .collect();
                        alt.fallback = Some(FallbackFragment {
                            start: after,
                            end: after,
                            bindings,
                            active: true,
                            image_fill_depth: None,
                        });
                    }
                    if let Some(fragment) = alt
                        .fallback
                        .as_mut()
                        .filter(|f| f.active && at > alt.depth + 1)
                    {
                        // Preserve legacy picture fallback support, and recognize
                        // the Math-specific p:sp/p:spPr/a:blipFill form by namespace.
                        if ns == PRESENTATION && local == b"pic" {
                            alt.picture = true;
                        }
                        if (ns == DRAWING || ns == PRESENTATION) && local == b"blipFill" && !empty {
                            fragment.image_fill_depth = Some(at);
                        }
                        if ns == DRAWING
                            && local == b"blip"
                            && fragment.image_fill_depth.is_some_and(|fill| at > fill)
                        {
                            for attribute in tag.attributes() {
                                let attribute = attribute.map_err(|_| ERROR)?;
                                let (namespace, name) = reader.resolve_attribute(attribute.key);
                                if matches!(namespace, ResolveResult::Bound(ns) if ns.as_ref() == RELATIONSHIPS)
                                    && name.as_ref() == b"embed"
                                    && !attribute.value.is_empty()
                                {
                                    alt.picture = true;
                                }
                            }
                        }
                    }
                }
                if is_math && !owned_math {
                    report.unverified = true;
                }
                if !empty {
                    depth = at;
                }
            }
            Event::End(tag) => {
                let (namespace, local) = reader.resolve_element(tag.name());
                let mc = matches!(namespace,ResolveResult::Bound(ns) if ns.as_ref()==MC);
                for alt in &mut alternates {
                    if mc && local.as_ref() == b"Fallback" && depth == alt.depth + 1 {
                        if let Some(fragment) = &mut alt.fallback {
                            fragment.end = before;
                            fragment.active = false;
                            fragment.image_fill_depth = None;
                        }
                    }
                    if let Some(fragment) = &mut alt.fallback {
                        if fragment.image_fill_depth == Some(depth) {
                            fragment.image_fill_depth = None;
                        }
                    }
                    if alt.choice == Some(depth) {
                        alt.choice = None;
                    }
                }
                if mc && local.as_ref() == b"AlternateContent" {
                    let alt = alternates.pop().ok_or(ERROR)?;
                    if alt.math {
                        if let Some(fragment) = alt
                            .fallback
                            .filter(|fragment| fragment.end > fragment.start)
                        {
                            changes.push((
                                alt.start,
                                after,
                                scoped_fragment(
                                    &xml[fragment.start..fragment.end],
                                    &fragment.bindings,
                                )?,
                            ));
                            report.uses_fallback = true;
                            if !alt.picture {
                                report.unverified = true;
                            }
                        } else {
                            report.unverified = true;
                        }
                    }
                }
                depth = depth.checked_sub(1).ok_or(ERROR)?;
            }
            Event::DocType(_) => return Err(ERROR.into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 || !alternates.is_empty() {
        return Err(ERROR.into());
    }
    changes.sort_by_key(|(start, end, _)| (*start, std::cmp::Reverse(*end)));
    let mut result = vec![];
    let mut at = 0;
    for (start, end, replacement) in changes {
        if start < at {
            continue;
        }
        result.extend_from_slice(&xml[at..start]);
        result.extend(replacement);
        if result.len() > 4 * 1024 * 1024 {
            return Err(ERROR.into());
        }
        at = end;
    }
    result.extend_from_slice(&xml[at..]);
    if result.len() > 4 * 1024 * 1024 {
        return Err(ERROR.into());
    }
    Ok((result, report))
}

/// Never mutates source. Only bounded, regular ZIP members enter a fresh copy.
/// On-demand preview inspects only the requested actual slide. Other ZIP
/// members are copied compressed, without expanding an entire long deck.
pub(crate) fn make_page_copy(
    source: &Path,
    destination: &Path,
    slide_path: &str,
) -> Result<MathPreview, String> {
    let (mut file, size) =
        crate::services::assistant::attachments::open_written(source, 25 * 1024 * 1024)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 != size {
        return Err(ERROR.into());
    }
    let info = crate::services::assistant::file_preview::deck_info(&bytes)?;
    if !info.slide_paths.iter().any(|path| path == slide_path) {
        return Err(ERROR.into());
    }
    let mut archive = crate::zip_security::open(std::io::Cursor::new(bytes.as_slice()), 40_000)
        .map_err(|_| ERROR)?;
    let xml = crate::services::assistant::file_preview::zip_member(&mut archive, slide_path)?;
    let (changed, report) = select_math_fallbacks(&xml)?;
    if !report.uses_fallback {
        return Ok(report);
    }
    let mut staged =
        tempfile::NamedTempFile::new_in(destination.parent().ok_or(ERROR)?).map_err(|_| ERROR)?;
    {
        let mut writer = zip::ZipWriter::new(staged.as_file_mut());
        for index in 0..archive.len() {
            let file = archive.by_index(index).map_err(|_| ERROR)?;
            if file.name().map_err(|_| ERROR)?.as_ref() == slide_path {
                writer
                    .start_file(
                        slide_path,
                        crate::zip_security::portable_options()
                            .compression_method(zip::CompressionMethod::Deflated)
                            .unix_permissions(0o600),
                    )
                    .map_err(|_| ERROR)?;
                writer.write_all(&changed).map_err(|_| ERROR)?;
            } else {
                writer.raw_copy_file(file).map_err(|_| ERROR)?;
            }
        }
        writer.finish().map_err(|_| ERROR)?;
    }
    staged.persist_noclobber(destination).map_err(|_| ERROR)?;
    Ok(report)
}

pub(crate) fn make_copy(source: &Path, destination: &Path) -> Result<MathPreview, String> {
    let (file, size) =
        crate::services::assistant::attachments::open_written(source, 25 * 1024 * 1024)?;
    if size == 0 {
        return Err(ERROR.into());
    }
    let mut archive = crate::zip_security::open(file, 4096).map_err(|_| ERROR)?;
    crate::zip_security::validate_headers(&mut archive).map_err(|_| ERROR)?;
    if archive.len() > 4096 {
        return Err(ERROR.into());
    }
    let mut members = vec![];
    let mut total = 0u64;
    let mut names = HashSet::new();
    let mut report = MathPreview::default();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|_| ERROR)?;
        let name = file.name().map_err(|_| ERROR)?.into_owned();
        if name.len() > 256
            || name.contains('\\')
            || name.contains(':')
            || !Path::new(&name)
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
            || !names.insert(name.clone())
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(ERROR.into());
        }
        if file.is_dir() {
            continue;
        }
        let size = file.size();
        total = total.checked_add(size).ok_or(ERROR)?;
        if size > 16 * 1024 * 1024 || total > 64 * 1024 * 1024 {
            return Err(ERROR.into());
        }
        let mut bytes = vec![];
        file.by_ref()
            .take(size + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ERROR)?;
        if bytes.len() as u64 != size {
            return Err(ERROR.into());
        }
        if name.starts_with("ppt/slides/slide")
            && name.ends_with(".xml")
            && !name.contains("/_rels/")
        {
            let (changed, found) = select_math_fallbacks(&bytes)?;
            bytes = changed;
            report.uses_fallback |= found.uses_fallback;
            report.unverified |= found.unverified;
        }
        members.push((name, bytes));
    }
    if !report.uses_fallback {
        return Ok(report);
    }
    let parent = destination.parent().ok_or(ERROR)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|_| ERROR)?;
    {
        let mut writer = zip::ZipWriter::new(staged.as_file_mut());
        for (name, bytes) in members {
            writer
                .start_file(
                    name,
                    crate::zip_security::portable_options()
                        .compression_method(zip::CompressionMethod::Deflated)
                        .unix_permissions(0o600),
                )
                .map_err(|_| ERROR)?;
            writer.write_all(&bytes).map_err(|_| ERROR)?;
        }
        writer.finish().map_err(|_| ERROR)?;
    }
    staged.persist_noclobber(destination).map_err(|_| ERROR)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    const SLIDE: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:c="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><p:spTree><c:AlternateContent><c:Choice Requires="a14"><p:sp><a14:m><m:oMath><m:t>native-math</m:t></m:oMath></a14:m></p:sp></c:Choice><c:Fallback><p:pic name="math-fallback"/></c:Fallback></c:AlternateContent><c:AlternateContent><c:Choice Requires="a14"><p:sp name="unrelated-choice"/></c:Choice><c:Fallback><p:sp name="unrelated-fallback"/></c:Fallback></c:AlternateContent></p:spTree></p:sld>"#;
    #[test]
    fn math_preview_changes_only_a_copy_and_selects_only_math_fallbacks() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("original.pptx");
        let target = temp.path().join("preview.pptx");
        let file = std::fs::File::create(&source).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "ppt/slides/slide1.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(SLIDE.as_bytes()).unwrap();
        zip.finish().unwrap();
        let before = std::fs::read(&source).unwrap();
        let result = make_copy(&source, &target).unwrap();
        assert!(result.uses_fallback);
        assert!(!result.unverified);
        assert_eq!(std::fs::read(&source).unwrap(), before);
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&target).unwrap()).unwrap();
        let mut xml = String::new();
        zip.by_name("ppt/slides/slide1.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(!xml.contains("native-math"));
        assert!(xml.contains("math-fallback"));
        assert!(xml.contains("unrelated-choice"));
        assert!(xml.contains("unrelated-fallback"));
    }
    #[test]
    fn math_preview_without_fallback_is_unverified_and_rejects_dtd() {
        let missing = SLIDE.replace(
            r#"<c:Fallback><p:pic name="math-fallback"/></c:Fallback>"#,
            "",
        );
        let (result, report) = select_math_fallbacks(missing.as_bytes()).unwrap();
        assert!(!report.uses_fallback);
        assert!(report.unverified);
        assert_eq!(result, missing.as_bytes());
        assert!(select_math_fallbacks(
            format!("<!DOCTYPE root [<!ENTITY e SYSTEM 'file:///private/test'>]>{SLIDE}")
                .as_bytes()
        )
        .is_err());
    }
    fn normative_slide(fill: &str) -> String {
        SLIDE.replacen("<p:sld ",r#"<p:sld xmlns:d="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:ri="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,1)
            .replacen(r#"<p:pic name="math-fallback"/>"#,&format!(r#"<p:sp name="normative-fallback"><p:spPr>{fill}</p:spPr></p:sp>"#),1)
    }
    #[test]
    fn math_preview_recognizes_normative_shape_image_fill_with_any_prefix() {
        let xml = normative_slide(r#"<d:blipFill><d:blip ri:embed="rId1"/></d:blipFill>"#);
        let (changed, report) = select_math_fallbacks(xml.as_bytes()).unwrap();
        assert!(report.uses_fallback);
        assert!(!report.unverified);
        let changed = String::from_utf8(changed).unwrap();
        assert!(changed.contains("normative-fallback"));
        assert!(!changed.contains("native-math"));
    }
    #[test]
    fn math_preview_image_fill_requires_real_namespaces_embed_and_fallback_scope() {
        let fill = r#"<d:blipFill><d:blip ri:embed="rId1"/></d:blipFill>"#;
        let valid = normative_slide(fill);
        let fake_ns = valid.replace(
            "xmlns:d=\"http://schemas.openxmlformats.org/drawingml/2006/main\"",
            "xmlns:d=\"urn:fake-drawing\"",
        );
        let fake_relationship = valid.replace(
            "xmlns:ri=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"",
            "xmlns:ri=\"urn:fake-relationship\"",
        );
        let text = normative_slide("");
        let in_choice = text.replacen("</a14:m>", &format!("</a14:m>{fill}"), 1);
        let after_fallback = text.replacen("</c:Fallback>", &format!("</c:Fallback>{fill}"), 1);
        let uncontained = normative_slide(r#"<d:blipFill/><d:blip ri:embed="rId1"/>"#);
        for xml in [
            fake_ns,
            fake_relationship,
            in_choice,
            after_fallback,
            uncontained,
        ] {
            let (_, report) = select_math_fallbacks(xml.as_bytes()).unwrap();
            assert!(report.uses_fallback);
            assert!(report.unverified);
        }
    }
    #[test]
    fn math_preview_rebinds_namespaces_and_refuses_zip_traversal() {
        let scoped=SLIDE.replacen(r#"<c:Fallback><p:pic name="math-fallback"/></c:Fallback>"#,r#"<c:Fallback xmlns:q="http://schemas.openxmlformats.org/presentationml/2006/main"><q:pic name="math-fallback"/></c:Fallback>"#,1);
        let (result, report) = select_math_fallbacks(scoped.as_bytes()).unwrap();
        assert!(report.uses_fallback);
        assert!(String::from_utf8(result).unwrap().contains("xmlns:q="));
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("bad.pptx");
        let file = std::fs::File::create(&source).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("../escape.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"no").unwrap();
        zip.finish().unwrap();
        assert!(make_copy(&source, &temp.path().join("copy.pptx")).is_err());
        assert!(!temp.path().join("copy.pptx").exists());
    }
    #[test]
    fn math_preview_refuses_ambiguous_footer_entry_count_and_local_header_mismatch() {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file(
                "ppt/slides/slide1.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(SLIDE.as_bytes()).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let mut footer = bytes.clone();
        footer.extend_from_slice(b"PK\x05\x06");
        let mut count = bytes.clone();
        let end = count.len() - 22;
        count[end + 8..end + 10].copy_from_slice(&4097u16.to_le_bytes());
        count[end + 10..end + 12].copy_from_slice(&4097u16.to_le_bytes());
        let mut mismatch = bytes;
        mismatch[30] = b'z';
        let temp = tempfile::tempdir().unwrap();
        for (i, bytes) in [footer, count, mismatch].into_iter().enumerate() {
            let source = temp.path().join(format!("bad-{i}.pptx"));
            let target = temp.path().join(format!("copy-{i}.pptx"));
            std::fs::write(&source, &bytes).unwrap();
            assert!(make_copy(&source, &target).is_err());
            assert!(!target.exists());
        }
    }
    #[test]
    #[ignore = "explicit local fixture; no network or user-store writes"]
    fn math_preview_real_fixture() {
        let source = std::path::PathBuf::from(std::env::var("OFOX_MATH_PREVIEW_FIXTURE").unwrap());
        let output = std::path::PathBuf::from(std::env::var("OFOX_MATH_PREVIEW_OUTPUT").unwrap());
        let before = std::fs::read(&source).unwrap();
        let result = make_copy(&source, &output).unwrap();
        assert!(result.uses_fallback);
        assert!(!result.unverified);
        assert_eq!(std::fs::read(source).unwrap(), before);
        println!("local native-math fixture: copied with raster fallback; original unchanged");
    }
}
