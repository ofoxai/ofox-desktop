//! Package-level presentation identity and content-type preservation.
use super::{xml_tags, zip_member, ERROR, PKGNS, RNS};
use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
};

const CTNS: &[u8] = b"http://schemas.openxmlformats.org/package/2006/content-types";
const MAIN_MIME: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";

// Resolve only package paths. No URI is ever opened as a filesystem or network
// resource. Percent encodings are refused rather than guessing which ZIP item
// an encoded separator or dot component identifies.
pub(in crate::services::assistant) fn target_path(
    source: &str,
    target: &str,
) -> Result<String, String> {
    if target.is_empty()
        || target.starts_with("//")
        || target.ends_with('/')
        || target.contains(['\\', ':', '?', '#', '%'])
        || target.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(ERROR.into());
    }
    let mut pieces: Vec<&str> = if target.starts_with('/') {
        vec![]
    } else {
        source
            .rsplit_once('/')
            .map(|(folder, _)| folder.split('/').collect())
            .unwrap_or_default()
    };
    for piece in target.strip_prefix('/').unwrap_or(target).split('/') {
        match piece {
            "" => return Err(ERROR.into()),
            "." => (),
            ".." => {
                pieces.pop().ok_or(ERROR)?;
            }
            name => pieces.push(name),
        }
    }
    if pieces.is_empty() {
        return Err(ERROR.into());
    }
    Ok(pieces.join("/"))
}

fn inventory(zip: &mut zip::ZipArchive<Cursor<&[u8]>>) -> Result<HashSet<String>, String> {
    crate::zip_security::validate_headers(zip).map_err(|_| ERROR)?;
    if zip.len() > 40_000 {
        return Err(ERROR.into());
    }
    let mut raw = HashSet::new();
    let mut folded = HashSet::new();
    let mut names = HashSet::new();
    let mut expanded = 0u64;
    for index in 0..zip.len() {
        let file = zip.by_index(index).map_err(|_| ERROR)?;
        let name = file.name().map_err(|_| ERROR)?.into_owned();
        expanded = expanded.checked_add(file.size()).ok_or(ERROR)?;
        if !raw.insert(name.clone())
            || file.size() > 32 * 1024 * 1024
            || expanded > 256 * 1024 * 1024
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(ERROR.into());
        }
        let part = name.strip_suffix('/').unwrap_or(&name);
        if target_path("", &format!("/{part}"))? != part {
            return Err(ERROR.into());
        }
        if !name.ends_with('/') {
            if !folded.insert(name.to_ascii_lowercase()) {
                return Err(ERROR.into());
            }
            names.insert(name);
        }
    }
    Ok(names)
}

pub(in crate::services::assistant) fn relationships_path(source: &str) -> String {
    if source.is_empty() {
        "_rels/.rels".into()
    } else if let Some((folder, name)) = source.rsplit_once('/') {
        format!("{folder}/_rels/{name}.rels")
    } else {
        format!("_rels/{source}.rels")
    }
}

pub(super) struct Relationship {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) target: String,
    pub(super) external: bool,
}

pub(super) fn relationships_for(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    source: &str,
) -> Result<Vec<Relationship>, String> {
    read_relationships(zip, &relationships_path(source), source)
}

pub(super) fn presentation_relationships(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    part: &PresentationPart,
) -> Result<Vec<Relationship>, String> {
    read_relationships(zip, &part.relationships_path, &part.presentation_path)
}

fn read_relationships(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
    source: &str,
) -> Result<Vec<Relationship>, String> {
    let tags = xml_tags(&zip_member(zip, path)?)?;
    if !tags
        .first()
        .is_some_and(|tag| tag.depth == 0 && tag.is(PKGNS, b"Relationships"))
    {
        return Err(ERROR.into());
    }
    let mut ids = HashSet::new();
    let mut result = vec![];
    for tag in tags.iter().skip(1) {
        if tag.depth != 1
            || !tag.child_of(PKGNS, b"Relationships")
            || !tag.is(PKGNS, b"Relationship")
        {
            return Err(ERROR.into());
        }
        let id = tag.attr(b"Id").filter(|id| !id.is_empty()).ok_or(ERROR)?;
        let kind = tag.attr(b"Type").ok_or(ERROR)?;
        let target = tag.attr(b"Target").filter(|v| !v.is_empty()).ok_or(ERROR)?;
        if !ids.insert(id.clone())
            || url::Url::parse(kind).is_err()
            || target.chars().any(|c| c.is_control())
        {
            return Err(ERROR.into());
        }
        let external = match tag.attr(b"TargetMode").map(String::as_str) {
            None | Some("Internal") => false,
            Some("External") => true,
            _ => return Err(ERROR.into()),
        };
        let target = if external {
            target.clone()
        } else {
            target_path(source, target)?
        };
        result.push(Relationship {
            id: id.clone(),
            kind: kind.clone(),
            target,
            external,
        });
    }
    Ok(result)
}

struct ContentTypes {
    defaults: HashMap<String, String>,
    overrides: HashMap<String, String>,
}
impl ContentTypes {
    fn effective(&self, part: &str) -> Option<&str> {
        self.overrides
            .get(part)
            .or_else(|| {
                part.rsplit_once('.')
                    .and_then(|(_, extension)| self.defaults.get(&extension.to_ascii_lowercase()))
            })
            .map(String::as_str)
    }
}
fn mime(value: &str) -> Result<String, String> {
    let Some((kind, subtype)) = value.split_once('/') else {
        return Err(ERROR.into());
    };
    let token = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"!#$%&'+-.^_`|~".contains(&c))
    };
    if !token(kind) || !token(subtype) {
        return Err(ERROR.into());
    }
    Ok(value.to_ascii_lowercase())
}
fn content_types(zip: &mut zip::ZipArchive<Cursor<&[u8]>>) -> Result<ContentTypes, String> {
    let tags = xml_tags(&zip_member(zip, "[Content_Types].xml")?)?;
    if !tags
        .first()
        .is_some_and(|tag| tag.depth == 0 && tag.is(CTNS, b"Types"))
    {
        return Err(ERROR.into());
    }
    let mut result = ContentTypes {
        defaults: HashMap::new(),
        overrides: HashMap::new(),
    };
    let mut folded = HashSet::new();
    for tag in tags.iter().skip(1) {
        if tag.depth != 1 || !tag.child_of(CTNS, b"Types") {
            return Err(ERROR.into());
        }
        let content_type = mime(tag.attr(b"ContentType").ok_or(ERROR)?)?;
        if tag.is(CTNS, b"Default") {
            let extension = tag.attr(b"Extension").ok_or(ERROR)?;
            if extension.is_empty()
                || !extension.bytes().all(|c| c.is_ascii_alphanumeric())
                || result
                    .defaults
                    .insert(extension.to_ascii_lowercase(), content_type)
                    .is_some()
            {
                return Err(ERROR.into());
            }
        } else if tag.is(CTNS, b"Override") {
            let part = tag.attr(b"PartName").ok_or(ERROR)?;
            if !part.starts_with('/') {
                return Err(ERROR.into());
            }
            let path = target_path("", part)?;
            if format!("/{path}") != *part
                || !folded.insert(path.to_ascii_lowercase())
                || result.overrides.insert(path, content_type).is_some()
            {
                return Err(ERROR.into());
            }
        } else {
            return Err(ERROR.into());
        }
    }
    Ok(result)
}

pub(in crate::services::assistant) struct PresentationPart {
    pub(in crate::services::assistant) presentation_path: String,
    pub(in crate::services::assistant) relationships_path: String,
}

pub(in crate::services::assistant) fn resolve_presentation(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
) -> Result<PresentationPart, String> {
    let names = inventory(zip)?;
    let relationships = relationships_for(zip, "")?;
    let kind = format!(
        "{}/officeDocument",
        std::str::from_utf8(RNS).map_err(|_| ERROR)?
    );
    let mut main = relationships
        .iter()
        .filter(|relation| relation.kind == kind);
    let relationship = main.next().ok_or(ERROR)?;
    if main.next().is_some() || relationship.external || !names.contains(&relationship.target) {
        return Err(ERROR.into());
    }
    let path = relationship.target.clone();
    let types = content_types(zip)?;
    let rels = relationships_path(&path);
    if types.effective(&path) != Some(MAIN_MIME) || !names.contains(&rels) {
        return Err(ERROR.into());
    }
    Ok(PresentationPart {
        presentation_path: path,
        relationships_path: rels,
    })
}

fn relationship_source(path: &str) -> Result<String, String> {
    if path == "_rels/.rels" {
        return Ok(String::new());
    }
    let (folder, name) = path
        .rsplit_once("/_rels/")
        .or_else(|| path.strip_prefix("_rels/").map(|name| ("", name)))
        .ok_or(ERROR)?;
    let name = name
        .strip_suffix(".rels")
        .filter(|v| !v.is_empty())
        .ok_or(ERROR)?;
    if name.contains('/') {
        return Err(ERROR.into());
    }
    Ok(if folder.is_empty() {
        name.into()
    } else {
        format!("{folder}/{name}")
    })
}

pub(in crate::services::assistant) fn validate_preserved_package(
    original: &mut zip::ZipArchive<Cursor<&[u8]>>,
    output: &mut zip::ZipArchive<Cursor<&[u8]>>,
) -> Result<(), String> {
    let before = resolve_presentation(original)?;
    let after = resolve_presentation(output)?;
    if before.presentation_path != after.presentation_path {
        return Err(ERROR.into());
    }
    let old_names = inventory(original)?;
    let new_names = inventory(output)?;
    let old_types = content_types(original)?;
    let new_types = content_types(output)?;
    for name in new_names
        .iter()
        .filter(|name| name.as_str() != "[Content_Types].xml")
    {
        if old_names.contains(name) {
            if let Some(old) = old_types.effective(name) {
                if new_types.effective(name) != Some(old) {
                    return Err(ERROR.into());
                }
            }
        } else if new_types.effective(name).is_none() {
            return Err(ERROR.into());
        }
        if name.ends_with(".rels") {
            let source = relationship_source(name)?;
            if !source.is_empty() && !new_names.contains(&source) {
                return Err(ERROR.into());
            }
            for relation in relationships_for(output, &source)? {
                if !relation.external
                    && (!new_names.contains(&relation.target)
                        || new_types.effective(&relation.target).is_none())
                {
                    return Err(ERROR.into());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, io::Write};

    const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
    const MAIN: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";
    const CHART: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";

    fn root(target: &str) -> String {
        format!("<Relationships xmlns=\"{PKG}\"><Relationship Id=\"main\" Type=\"{REL}/officeDocument\" Target=\"{target}\"/></Relationships>")
    }
    fn parts() -> BTreeMap<String, String> {
        let mut files = BTreeMap::new();
        files.insert("_rels/.rels".into(), root("ppt/presentation.xml"));
        let mut types = format!("<Types xmlns=\"{CT}\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>");
        for (path, count) in [("ppt/presentation.xml", 3), ("content/actual.xml", 2)] {
            let (folder, name) = path.rsplit_once('/').unwrap();
            let stem = name.trim_end_matches(".xml");
            types.push_str(&format!(
                "<Override PartName=\"/{path}\" ContentType=\"{MAIN}\"/>"
            ));
            let mut presentation = format!("<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:r=\"{REL}\"><p:sldIdLst>");
            let mut rels = format!("<Relationships xmlns=\"{PKG}\">");
            for number in 1..=count {
                let physical = count + 1 - number;
                let slide = format!("{folder}/slides/{stem}-{physical}.xml");
                types.push_str(&format!("<Override PartName=\"/{slide}\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"));
                presentation.push_str(&format!(
                    "<p:sldId id=\"{}\" r:id=\"r{number}\"/>",
                    256 + number
                ));
                rels.push_str(&format!("<Relationship Id=\"r{number}\" Type=\"{REL}/slide\" Target=\"slides/{stem}-{physical}.xml\"/>"));
                files.insert(slide, "<p:sld xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"/>".into());
            }
            presentation.push_str(
                "</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/></p:presentation>",
            );
            rels.push_str("</Relationships>");
            files.insert(path.into(), presentation);
            files.insert(format!("{folder}/_rels/{name}.rels"), rels);
        }
        types.push_str("</Types>");
        files.insert("[Content_Types].xml".into(), types);
        files
    }
    fn packed(files: &BTreeMap<String, String>) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in files {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(text.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
    fn resolve(files: &BTreeMap<String, String>) -> Result<PresentationPart, String> {
        let bytes = packed(files);
        resolve_presentation(&mut zip::ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap())
    }
    fn preserves(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> Result<(), String> {
        let (a, b) = (packed(a), packed(b));
        validate_preserved_package(
            &mut zip::ZipArchive::new(Cursor::new(a.as_slice())).unwrap(),
            &mut zip::ZipArchive::new(Cursor::new(b.as_slice())).unwrap(),
        )
    }

    #[test]
    fn opc_actual_root_main_determines_page_count_order_and_main_path() {
        let mut files = parts();
        files.insert("_rels/.rels".into(), root("content/actual.xml"));
        let info = super::super::deck_info(&packed(&files)).unwrap();
        assert_eq!(
            info.total_pages, 2,
            "the inactive three-page ppt/presentation.xml must not determine page count"
        );
        assert_eq!(info.presentation_path, "content/actual.xml");
        assert_eq!(
            info.slide_paths,
            ["content/slides/actual-2.xml", "content/slides/actual-1.xml"]
        );
    }

    #[test]
    fn opc_absolute_and_relative_main_targets_have_the_same_package_identity() {
        for target in [
            "content/actual.xml",
            "/content/actual.xml",
            "./content/actual.xml",
            "ppt/../content/actual.xml",
        ] {
            let mut files = parts();
            files.insert("_rels/.rels".into(), root(target));
            let part = resolve(&files).unwrap();
            assert_eq!(part.presentation_path, "content/actual.xml");
            assert_eq!(part.relationships_path, "content/_rels/actual.xml.rels");
        }
    }

    #[test]
    fn opc_forged_external_duplicate_and_escaping_main_relationships_are_rejected() {
        let good = root("content/actual.xml");
        let mut cases = vec![
            good.replace(PKG, "urn:forged"),
            good.replace(&format!("{REL}/officeDocument"), "urn:forged/officeDocument"),
            good.replace("Target=", "TargetMode=\"External\" Target="),
            good.replace("</Relationships>", &format!("<Relationship Id=\"other\" Type=\"{REL}/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>")),
            good.replace("content/actual.xml", "../content/actual.xml"),
            good.replace("content/actual.xml", "//evil.invalid/actual.xml"),
            good.replace("content/actual.xml", "content/%2e%2e/actual.xml"),
            good.replace("content/actual.xml", "content/actual.xml#fragment"),
        ];
        for target in [
            "",
            "missing.xml",
            "content/actual.xml?query=1",
            "content\\actual.xml",
            "https://evil.invalid/actual.xml",
            "content%2factual.xml",
            "content%5cactual.xml",
        ] {
            cases.push(root(target));
        }
        for mode in ["external", "Unknown", ""] {
            cases.push(good.replace("Target=", &format!("TargetMode=\"{mode}\" Target=")));
        }
        cases.push(good.replace("</Relationships>", &format!("<Relationship Id=\"main\" Type=\"{REL}/image\" Target=\"ppt/presentation.xml\"/></Relationships>")));
        cases.push(
            good.replace("<Relationship Id=", "<container><Relationship Id=")
                .replace("/></Relationships>", "/></container></Relationships>"),
        );
        for bad in cases {
            let mut files = parts();
            files.insert("_rels/.rels".into(), bad);
            assert!(resolve(&files).is_err());
        }
    }

    #[test]
    fn opc_real_main_content_type_is_required_and_macro_or_foreign_types_are_refused() {
        let good = parts();
        let mut cases = vec![
            good["[Content_Types].xml"].replace(CT, "urn:forged"),
            good["[Content_Types].xml"].replace(
                MAIN,
                "application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml",
            ),
            good["[Content_Types].xml"].replace(
                &format!("<Override PartName=\"/ppt/presentation.xml\" ContentType=\"{MAIN}\"/>"),
                "",
            ),
            good["[Content_Types].xml"].replace(
                "</Types>",
                &format!(
                    "<Override PartName=\"/ppt/presentation.xml\" ContentType=\"{MAIN}\"/></Types>"
                ),
            ),
        ];
        cases.extend([
            good["[Content_Types].xml"].replace(
                "</Types>",
                "<Default Extension=\"XML\" ContentType=\"application/xml\"/></Types>",
            ),
            good["[Content_Types].xml"].replace(
                "</Types>",
                &format!(
                    "<Override PartName=\"/PPT/PRESENTATION.XML\" ContentType=\"{MAIN}\"/></Types>"
                ),
            ),
            good["[Content_Types].xml"]
                .replace("/ppt/presentation.xml", "/ppt/../ppt/presentation.xml"),
        ]);
        for changed in cases {
            let mut files = good.clone();
            files.insert("[Content_Types].xml".into(), changed);
            assert!(resolve(&files).is_err());
        }
    }

    #[test]
    fn opc_preservation_rejects_active_main_redirection_and_existing_chart_mime_swaps() {
        let mut original = parts();
        original.insert("ppt/charts/chart1.xml".into(), "<chart/>".into());
        let types = original.get_mut("[Content_Types].xml").unwrap();
        *types = types.replace(
            "</Types>",
            &format!(
                "<Override PartName=\"/ppt/charts/chart1.xml\" ContentType=\"{CHART}\"/></Types>"
            ),
        );
        let mut redirected = original.clone();
        redirected.insert("_rels/.rels".into(), root("content/actual.xml"));
        assert!(preserves(&original, &redirected).is_err());
        let mut changed = original.clone();
        *changed.get_mut("[Content_Types].xml").unwrap() =
            original["[Content_Types].xml"].replace(CHART, "application/xml");
        assert!(preserves(&original, &changed).is_err());
        *changed.get_mut("[Content_Types].xml").unwrap() = original["[Content_Types].xml"].replace(
            &format!("<Override PartName=\"/ppt/charts/chart1.xml\" ContentType=\"{CHART}\"/>"),
            "",
        );
        assert!(preserves(&original, &changed).is_err());
    }

    #[test]
    fn opc_preservation_allows_semantically_identical_main_new_mappings_and_unreferenced_cleanup() {
        let mut original = parts();
        original.insert("ppt/media/old.xml".into(), "unused".into());
        let mut output = original.clone();
        output.insert("_rels/.rels".into(), root("/ppt/presentation.xml"));
        output.remove("ppt/media/old.xml");
        output.insert("ppt/media/new.png".into(), "synthetic-test-only".into());
        let types = output.get_mut("[Content_Types].xml").unwrap();
        *types = types.replace(
            "</Types>",
            "<Default Extension=\"png\" ContentType=\"image/png\"/></Types>",
        );
        assert!(preserves(&original, &output).is_ok());
    }

    #[test]
    fn opc_still_referenced_deleted_parts_and_changed_existing_default_mime_are_rejected() {
        let mut original = parts();
        original.insert("ppt/media/old.png".into(), "synthetic-test-only".into());
        let types = original.get_mut("[Content_Types].xml").unwrap();
        *types = types.replace(
            "</Types>",
            "<Default Extension=\"png\" ContentType=\"image/png\"/></Types>",
        );
        let rels = original.get_mut("ppt/_rels/presentation.xml.rels").unwrap();
        *rels = rels.replace("</Relationships>", &format!("<Relationship Id=\"image\" Type=\"{REL}/image\" Target=\"media/old.png\"/></Relationships>"));
        let mut output = original.clone();
        output.remove("ppt/media/old.png");
        assert!(preserves(&original, &output).is_err());
        let mut output = original.clone();
        *output.get_mut("[Content_Types].xml").unwrap() =
            original["[Content_Types].xml"].replace("image/png", "image/jpeg");
        assert!(preserves(&original, &output).is_err());
    }

    #[test]
    fn opc_effective_content_types_allow_equivalent_override_default_changes() {
        let mut original = parts();
        original.insert("ppt/media/old.png".into(), "synthetic-test-only".into());
        let types = original.get_mut("[Content_Types].xml").unwrap();
        *types = types.replace("</Types>", "<Default Extension=\"png\" ContentType=\"image/png\"/><Override PartName=\"/ppt/media/old.png\" ContentType=\"image/png\"/></Types>");
        let mut output = original.clone();
        *output.get_mut("[Content_Types].xml").unwrap() = original["[Content_Types].xml"].replace(
            "<Override PartName=\"/ppt/media/old.png\" ContentType=\"image/png\"/>",
            "",
        );
        assert!(
            preserves(&original, &output).is_ok(),
            "an equivalent Default can replace an Override"
        );
        let mut output = original.clone();
        *output.get_mut("[Content_Types].xml").unwrap() = original["[Content_Types].xml"].replace(
            "<Default Extension=\"png\" ContentType=\"image/png\"/>",
            "<Default Extension=\"PNG\" ContentType=\"image/jpeg\"/>",
        );
        assert!(
            preserves(&original, &output).is_ok(),
            "an existing identical Override still protects this part's effective MIME"
        );
    }

    #[test]
    fn opc_new_referenced_part_without_an_effective_content_type_is_rejected() {
        let original = parts();
        let mut output = original.clone();
        output.insert("ppt/media/new.png".into(), "synthetic-test-only".into());
        let rels = output.get_mut("ppt/_rels/presentation.xml.rels").unwrap();
        *rels = rels.replace("</Relationships>", &format!("<Relationship Id=\"new-image\" Type=\"{REL}/image\" Target=\"media/new.png\"/></Relationships>"));
        assert!(preserves(&original, &output).is_err());
    }

    #[test]
    fn opc_actual_main_slide_targets_accept_absolute_and_parent_relative_package_paths() {
        let mut files = parts();
        files.insert("_rels/.rels".into(), root("/content/actual.xml"));
        let rels = files.get_mut("content/_rels/actual.xml.rels").unwrap();
        *rels = rels
            .replace(
                "Target=\"slides/actual-2.xml\"",
                "Target=\"/content/slides/actual-2.xml\"",
            )
            .replace(
                "Target=\"slides/actual-1.xml\"",
                "Target=\"../content/slides/actual-1.xml\"",
            );
        let info = super::super::deck_info(&packed(&files)).unwrap();
        assert_eq!(info.total_pages, 2);
        assert_eq!(
            info.slide_paths,
            ["content/slides/actual-2.xml", "content/slides/actual-1.xml"]
        );
    }
}
