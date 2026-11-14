//! Derived, bounded raster previews; active document bytes are never served.
use crate::services::assistant::{attachments, host::runtime_at, types::Attachment};
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

pub(crate) const MAX_JPEG: u64 = 200 * 1024;
/// Maximum images in one raster batch or persisted attachment preview.
pub(crate) const MAX_PAGES: u32 = 60;
/// Actual one-based source page; independent of the bounded batch length.
pub(crate) const MAX_PAGE_NUMBER: u32 = 1000;

pub(crate) fn pages(file: &Attachment) -> Vec<u32> {
    if file.origin.as_deref() != Some("generated")
        || file.status == "missing"
        || file.mime_type
            != "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    {
        return vec![];
    }
    let Some(value) = file.details.as_ref().and_then(|v| v.get("documentPreview")) else {
        return vec![];
    };
    if value["mimeType"] != "image/jpeg" {
        return vec![];
    }
    let Some(raw) = value["pages"]
        .as_array()
        .filter(|p| !p.is_empty() && p.len() <= MAX_PAGES as usize)
    else {
        return vec![];
    };
    let mut result = vec![];
    for item in raw {
        let Some(page) = item
            .as_u64()
            .filter(|p| (1..=MAX_PAGE_NUMBER as u64).contains(p))
        else {
            return vec![];
        };
        if result.contains(&(page as u32)) {
            return vec![];
        }
        result.push(page as u32);
    }
    result.sort_unstable();
    result
}

/// Check SOF dimensions as well as JPEG framing before bytes enter a WebView.
pub(crate) fn valid_jpeg(bytes: &[u8]) -> bool {
    if bytes.len() < 4
        || bytes.len() as u64 > MAX_JPEG
        || !bytes.starts_with(&[255, 216])
        || !bytes.ends_with(&[255, 217])
    {
        return false;
    }
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 255 {
            return false;
        }
        let marker = bytes[offset + 1];
        offset += 2;
        if marker == 255 {
            offset -= 1;
            continue;
        }
        if marker == 218 || marker == 217 {
            return false;
        }
        let length = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        if length < 2 || offset + length > bytes.len() {
            return false;
        }
        if matches!(marker, 192..=195 | 197..=199 | 201..=203 | 205..=207) {
            if length < 8 {
                return false;
            }
            let h = u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]);
            let w = u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]);
            return w > 0 && h > 0 && w <= 768 && h <= 768;
        }
        offset += length;
    }
    false
}

pub(crate) fn read_jpeg(path: &Path) -> Result<Vec<u8>, String> {
    let (file, size) = attachments::open_written(path, MAX_JPEG)?;
    let mut bytes = vec![];
    file.take(MAX_JPEG + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "assistant_attachment_read_failed")?;
    if bytes.len() as u64 != size || !valid_jpeg(&bytes) {
        return Err("assistant_attachment_read_failed".into());
    }
    Ok(bytes)
}

pub(crate) async fn raster(
    runtime: &Path,
    folder: &Path,
    sources: &[PathBuf],
    sandbox: Option<&Path>,
) -> Result<Vec<(u32, Vec<u8>)>, String> {
    let runtime = runtime
        .canonicalize()
        .map_err(|_| "assistant_preview_unavailable")?;
    let loaded = runtime_at(&runtime)?;
    let helper = loaded
        .entry
        .parent()
        .ok_or("assistant_preview_unavailable")?
        .join("document-preview.mjs");
    if !helper.is_file() {
        return Ok(vec![]);
    }
    let temp = tempfile::Builder::new()
        .prefix(".raster-")
        .tempdir_in(folder)
        .map_err(|_| "assistant_preview_unavailable")?;
    let mut input = vec![];
    let mut total = 0;
    for source in sources.iter().take(MAX_PAGES as usize) {
        let page = source
            .file_stem()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|n| (1..=MAX_PAGE_NUMBER).contains(n));
        let Some(page) = page else {
            continue;
        };
        let Ok((file, size)) = attachments::open_page(source) else {
            continue;
        };
        if total + size > 12 * 1024 * 1024 {
            break;
        }
        let mut svg = String::new();
        if file.take(size + 1).read_to_string(&mut svg).is_err() || svg.len() as u64 != size {
            continue;
        }
        total += size;
        input.push(json!({"page":page,"svg":svg}));
    }
    let input = serde_json::to_vec(&input).map_err(|_| "assistant_preview_unavailable")?;
    if input.len() > 16 * 1024 * 1024 {
        return Ok(vec![]);
    }
    crate::config::atomic_write_with(
        &temp.path().join("document-preview-input.json"),
        &input,
        crate::config::AtomicOpts { mode: Some(0o600) },
    )
    .map_err(|_| "assistant_preview_unavailable")?;
    let home = temp.path().join("home");
    crate::config::create_private_dir_all(&home).map_err(|_| "assistant_preview_unavailable")?;
    let node = loaded.node.to_string_lossy();
    let helper = helper.to_string_lossy();
    let output = if let Some(sandbox) = sandbox {
        let doc = format!(
            "DOC={}",
            temp.path()
                .canonicalize()
                .map_err(|_| "assistant_preview_unavailable")?
                .display()
        );
        let engine = format!("ENGINE={}", runtime.display());
        let exe = format!(
            "EXE={}",
            loaded
                .node
                .canonicalize()
                .map_err(|_| "assistant_preview_unavailable")?
                .display()
        );
        super::runner::run_engine_limited(
            sandbox,
            &[
                "-D",
                &doc,
                "-D",
                &engine,
                "-D",
                &exe,
                "-p",
                super::create::PYTHON_SANDBOX,
                &node,
                &helper,
            ],
            temp.path(),
            &home,
            Duration::from_secs(60),
            8192,
        )
        .await?
    } else {
        if cfg!(target_os = "macos") {
            return Ok(vec![]);
        }
        super::runner::run_engine_limited(
            &loaded.node,
            &[&helper],
            temp.path(),
            &home,
            Duration::from_secs(60),
            8192,
        )
        .await?
    };
    if output.status != Some(0) {
        return Ok(vec![]);
    }
    let output: Value =
        serde_json::from_str(&output.stdout).map_err(|_| "assistant_preview_unavailable")?;
    let mut result = vec![];
    if let Some(pages) = output["pages"]
        .as_array()
        .filter(|p| p.len() <= MAX_PAGES as usize)
    {
        for page in pages {
            if let Some(page) = page
                .as_u64()
                .filter(|p| (1..=MAX_PAGE_NUMBER as u64).contains(p))
            {
                if result.iter().any(|(n, _)| *n == page as u32) {
                    continue;
                }
                if let Ok(bytes) =
                    read_jpeg(&temp.path().join(format!("document-preview-{page}.jpg")))
                {
                    result.push((page as u32, bytes));
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(pages: Value) -> Attachment {
        serde_json::from_value(json!({"id":super::super::super::types::id(),"name":"long-deck.pptx","mimeType":"application/vnd.openxmlformats-officedocument.presentationml.presentation","origin":"generated","sizeBytes":5,"status":"ready","details":{"documentPreview":{"mimeType":"image/jpeg","pages":pages}}})).unwrap()
    }

    #[test]
    fn preview_page_number_range_is_independent_of_batch_count() {
        assert_eq!(pages(&attachment(json!([1000, 70]))), [70, 1000]);
        let last_sixty: Vec<u32> = (941..=1000).collect();
        assert_eq!(pages(&attachment(json!(last_sixty))), last_sixty);
        for invalid in [
            json!([0]),
            json!([1001]),
            json!([70, 70]),
            json!([1.5]),
            json!((1..=61).collect::<Vec<u32>>()),
        ] {
            assert!(pages(&attachment(invalid)).is_empty());
        }
    }
}
