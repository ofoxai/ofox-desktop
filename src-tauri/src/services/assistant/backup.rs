//! Versioned account-scoped archives. Configuration SQL is never imported here.
use super::{local_data, policy::PermissionMode, store, types::*};
use crate::{config, database::Database, error::AppError};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = 512 * 1024 * 1024;
const MAX_MANIFEST: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const SDK_VERSION: &str = "1.4.0";
fn invalid() -> String {
    "assistant_backup_invalid".into()
}
fn io_error(_: impl std::fmt::Display) -> String {
    "assistant_backup_io_failed".into()
}
fn owner_valid(owner: &str) -> bool {
    owner.len() == 64
        && owner
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileInfo {
    path: String,
    size: u64,
    sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    format_version: u32,
    runtime_protocol: u32,
    sdk_version: String,
    owner: String,
    created_at: i64,
    records: BTreeMap<String, String>,
    files: Vec<FileInfo>,
}
pub struct Validated {
    manifest: Manifest,
    stage: tempfile::TempDir,
}
#[derive(Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub session_count: usize,
    pub file_count: usize,
    pub new_records: usize,
    pub skipped_records: usize,
    pub conflicts: Vec<String>,
    pub imported_records: usize,
}
struct Plan {
    rows: BTreeMap<String, String>,
    sessions: Vec<String>,
    report: Report,
}

fn row_kind<'a>(key: &'a str, owner: &str) -> Result<(&'static str, &'a str), String> {
    for family in ["assistant_v1", "assistant_memory_v1", "assistant_model_v1"] {
        if let Some(id) = key.strip_prefix(&format!("{family}:{owner}:")) {
            if family == "assistant_v1" {
                validate_id(id)?;
            } else if !matches!(id, "General" | "Configuration") {
                return Err(invalid());
            }
            return Ok((family, id));
        }
    }
    Err(invalid())
}
fn clean_detail(raw: &str, sid: &str) -> Result<Detail, String> {
    let mut detail: Detail = serde_json::from_str(raw).map_err(|_| invalid())?;
    if detail.session.id != sid || !detail.usage.is_object() {
        return Err(invalid());
    }
    if detail.session.active_run.is_some()
        || detail.messages.iter().any(|m| m.status == "streaming")
        || detail.operations.iter().any(|op| op["status"] == "running")
    {
        detail.usage["usageCoverage"] = Value::String("incomplete".into());
        detail.usage["estimatedCostUsd"] = Value::Null;
    }
    detail.session.permission_mode = PermissionMode::Ask;
    detail.session.active_run = None;
    detail.pending_approval = None;
    detail.memories.clear(); // The authoritative memory collection is a separate row.
    for message in &mut detail.messages {
        validate_id(&message.id)?;
        if message.status == "streaming" {
            message.status = "stopped".into();
        }
        for attachment in &mut message.attachments {
            attachment.preview_url = None;
        }
    }
    for attachment in &mut detail.attachments {
        attachment.preview_url = None;
    }
    for operation in &mut detail.operations {
        if operation["status"] == "running" {
            operation["status"] = Value::String("interrupted".into());
        }
    }
    Ok(detail)
}
fn canonical(key: &str, raw: &str, owner: &str) -> Result<String, String> {
    let (kind, id) = row_kind(key, owner)?;
    if kind == "assistant_model_v1" {
        if raw.len() > 512 {
            return Err(invalid());
        }
        return Ok(raw.into());
    }
    let mut value = if kind == "assistant_v1" {
        serde_json::to_value(clean_detail(raw, id)?).map_err(|_| invalid())?
    } else {
        let items: Vec<Memory> = serde_json::from_str(raw).map_err(|_| invalid())?;
        let mut ids = BTreeSet::new();
        for item in &items {
            validate_id(&item.id)?;
            if !ids.insert(&item.id) || item.content.len() > 16_000 {
                return Err(invalid());
            }
        }
        serde_json::to_value(items).map_err(|_| invalid())?
    };
    store::scrub_value(&mut value, &[]);
    serde_json::to_string(&value).map_err(|_| invalid())
}
fn rows(conn: &Connection, owner: &str) -> Result<BTreeMap<String, String>, String> {
    local_data::ensure_passive(conn)?;
    let mut stmt = conn
        .prepare("SELECT key,value FROM settings WHERE substr(key,1,10)='assistant_' ORDER BY key")
        .map_err(io_error)?;
    let values = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(io_error)?;
    let mut result = BTreeMap::new();
    for pair in values {
        let (key, raw) = pair.map_err(io_error)?;
        if row_kind(&key, owner).is_ok() {
            result.insert(key.clone(), canonical(&key, &raw, owner)?);
        }
    }
    Ok(result)
}
fn attachments(detail: &Detail) -> Result<BTreeMap<String, u64>, String> {
    let mut result = BTreeMap::new();
    for id in &detail.session.draft_attachment_ids {
        validate_id(id)?;
        if !detail
            .attachments
            .iter()
            .any(|attachment| &attachment.id == id)
        {
            return Err(invalid());
        }
    }
    for item in detail
        .attachments
        .iter()
        .chain(detail.messages.iter().flat_map(|m| &m.attachments))
    {
        validate_id(&item.id)?;
        if item.size_bytes > 25 * 1024 * 1024
            || result
                .insert(item.id.clone(), item.size_bytes)
                .is_some_and(|size| size != item.size_bytes)
        {
            return Err(invalid());
        }
    }
    Ok(result)
}
fn file_path(name: &str) -> Result<(&str, Option<&str>), String> {
    let parts: Vec<_> = name.split('/').collect();
    match parts.as_slice() {
        ["sessions", sid, "session.jsonl"] => {
            validate_id(sid)?;
            Ok((sid, None))
        }
        ["sessions", sid, "attachments", aid] => {
            validate_id(sid)?;
            validate_id(aid)?;
            Ok((sid, Some(aid)))
        }
        _ => Err(invalid()),
    }
}
fn plain(path: &Path, directory: bool) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(io_error)?;
    if m.file_type().is_symlink() || if directory { !m.is_dir() } else { !m.is_file() } {
        return Err(invalid());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(invalid());
        }
    }
    Ok(())
}
fn entry_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_error(e)),
    }
}
fn private_directory(path: &Path) -> Result<(), String> {
    if entry_exists(path)? {
        plain(path, true)?;
    }
    config::create_private_dir_all(path).map_err(io_error)?;
    plain(path, true)
}
fn read_staged(root: &Path, name: &str) -> Result<Vec<u8>, String> {
    let (sid, aid) = file_path(name)?;
    plain(root, true)?;
    let base = root.join("sessions");
    read_limited(&source_file(&base, sid, aid)?, MAX_FILE)
}

fn source_file(root: &Path, sid: &str, aid: Option<&str>) -> Result<PathBuf, String> {
    plain(root, true)?;
    let session = root.join(sid);
    plain(&session, true)?;
    let file = if let Some(aid) = aid {
        plain(&session.join("attachments"), true)?;
        session.join("attachments").join(aid)
    } else {
        session.join("session.jsonl")
    };
    plain(&file, false)?;
    Ok(file)
}
fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    plain(path, false)?;
    let file = fs::File::open(path).map_err(io_error)?;
    if file.metadata().map_err(io_error)?.len() > max {
        return Err("assistant_backup_too_large".into());
    }
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > max {
        return Err("assistant_backup_too_large".into());
    }
    Ok(bytes)
}
fn native_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let mut value: Value = serde_json::from_str(line).map_err(|_| invalid())?;
        if !value.is_object() {
            return Err(invalid());
        }
        let id = value["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 256)
            .ok_or_else(invalid)?
            .to_string();
        let at = value["timestamp"].as_str().ok_or_else(invalid)?;
        chrono::DateTime::parse_from_rfc3339(at).map_err(|_| invalid())?;
        if index == 0 {
            if value["type"] != "session" || value["version"] != 3 || !value["cwd"].is_string() {
                return Err(invalid());
            }
            value["cwd"] = Value::String(String::new());
            // A source-machine parent path is metadata, never a restore target.
            value.as_object_mut().unwrap().remove("parentSession");
        } else {
            // Parents precede their children in the append-only SDK tree. This
            // rejects duplicate IDs, missing parents, self loops and all cycles.
            if !seen.insert(id.clone()) {
                return Err(invalid());
            }
            if value["parentId"].is_null() {
                if index != 1 {
                    return Err(invalid());
                }
            } else if value["parentId"]
                .as_str()
                .is_none_or(|parent| parent == id || !seen.contains(parent))
            {
                return Err(invalid());
            }
            let kind = value["type"].as_str().ok_or_else(invalid)?;
            match kind {
                "message" => {
                    let m = &value["message"];
                    if !m.is_object()
                        || !matches!(
                            m["role"].as_str(),
                            Some(
                                "user"
                                    | "assistant"
                                    | "toolResult"
                                    | "custom"
                                    | "bashExecution"
                                    | "system"
                            )
                        )
                        || !(m["content"].is_array()
                            || m["content"].is_string()
                            || m["role"] == "bashExecution")
                    {
                        return Err(invalid());
                    }
                }
                "compaction" | "branch_summary" => {
                    if !value["summary"].is_string() {
                        return Err(invalid());
                    }
                    let key = if kind == "compaction" {
                        "firstKeptEntryId"
                    } else {
                        "fromId"
                    };
                    if value[key].as_str().is_none_or(|id| !seen.contains(id)) {
                        return Err(invalid());
                    }
                }
                "label" => {
                    if value["targetId"]
                        .as_str()
                        .is_none_or(|id| !seen.contains(id))
                    {
                        return Err(invalid());
                    }
                }
                "custom" | "custom_message" => {
                    if !value["customType"].is_string() {
                        return Err(invalid());
                    }
                }
                "model_change" => {
                    if !value["provider"].is_string() || !value["modelId"].is_string() {
                        return Err(invalid());
                    }
                }
                "thinking_level_change" => {
                    if !value["thinkingLevel"].is_string() {
                        return Err(invalid());
                    }
                }
                "usage" => {
                    if !value["usage"].is_object() {
                        return Err(invalid());
                    }
                }
                "session_info" | "context_edit" => {}
                _ => return Err(invalid()),
            }
        }
        store::scrub_value(&mut value, &[]);
        serde_json::to_writer(&mut out, &value).map_err(|_| invalid())?;
        out.push(b'\n');
    }
    if out.is_empty() {
        return Err(invalid());
    }
    Ok(out)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn check_manifest(m: &Manifest, owner: &str) -> Result<(), String> {
    if m.owner != owner {
        return Err("assistant_backup_identity_mismatch".into());
    }
    if !owner_valid(owner)
        || m.format_version != 1
        || m.runtime_protocol != 1
        || m.sdk_version != SDK_VERSION
    {
        return Err("assistant_backup_version_unsupported".into());
    }
    if m.files.len() + 1 > MAX_ENTRIES || m.records.len() > MAX_ENTRIES {
        return Err("assistant_backup_too_large".into());
    }
    let mut expected = BTreeMap::new();
    let mut optional = BTreeSet::new();
    for (key, raw) in &m.records {
        canonical(key, raw, owner)?;
        let (kind, sid) = row_kind(key, owner)?;
        if kind == "assistant_v1" {
            let detail = clean_detail(raw, sid)?;
            let native = format!("sessions/{sid}/session.jsonl");
            if detail.messages.is_empty() {
                optional.insert(native);
            } else {
                expected.insert(native, None);
            }
            for (aid, size) in attachments(&detail)? {
                expected.insert(format!("sessions/{sid}/attachments/{aid}"), Some(size));
            }
        }
    }
    let mut total = 0u64;
    let mut seen = BTreeSet::new();
    for f in &m.files {
        file_path(&f.path)?;
        if !seen.insert(&f.path)
            || f.size > MAX_FILE
            || f.sha256.len() != 64
            || !f.sha256.bytes().all(|v| v.is_ascii_hexdigit())
        {
            return Err(invalid());
        }
        if let Some(size) = expected.remove(&f.path) {
            if size.is_some_and(|v| v != f.size) {
                return Err(invalid());
            }
        } else if !optional.remove(&f.path) {
            return Err(invalid());
        }
        total = total.checked_add(f.size).ok_or_else(invalid)?;
        if total > MAX_TOTAL {
            return Err("assistant_backup_too_large".into());
        }
    }
    if !expected.is_empty() {
        return Err("assistant_backup_file_missing".into());
    }
    Ok(())
}

#[cfg(test)]
pub fn create(db: &Database, owner: &str, root: &Path, target: &Path) -> Result<(), String> {
    create_guarded(db, owner, root, target, || Ok(()))
}
pub fn create_guarded(
    db: &Database,
    owner: &str,
    root: &Path,
    target: &Path,
    guard: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    if !owner_valid(owner) {
        return Err(invalid());
    }
    let records = rows(&*db.conn.lock().map_err(io_error)?, owner)?;
    let mut m = Manifest {
        format_version: 1,
        runtime_protocol: 1,
        sdk_version: SDK_VERSION.into(),
        owner: owner.into(),
        created_at: now(),
        records,
        files: vec![],
    };
    let stage = tempfile::tempdir().map_err(io_error)?;
    let mut total = 0u64;
    for (key, raw) in &m.records {
        guard()?;
        let (kind, sid) = row_kind(key, owner)?;
        if kind != "assistant_v1" {
            continue;
        }
        let detail = clean_detail(raw, sid)?;
        let mut paths: Vec<Option<String>> = attachments(&detail)?.into_keys().map(Some).collect();
        let native = root.join(sid).join("session.jsonl");
        if native.try_exists().map_err(io_error)? {
            paths.push(None);
        } else if !detail.messages.is_empty() {
            return Err("assistant_backup_file_missing".into());
        }
        for aid in paths {
            let source = source_file(root, sid, aid.as_deref())?;
            let bytes = read_limited(&source, MAX_FILE)?;
            let bytes = if aid.is_none() {
                native_bytes(&bytes)?
            } else {
                bytes
            };
            let path = if let Some(aid) = aid {
                format!("sessions/{sid}/attachments/{aid}")
            } else {
                format!("sessions/{sid}/session.jsonl")
            };
            total = total.checked_add(bytes.len() as u64).ok_or_else(invalid)?;
            if total > MAX_TOTAL || m.files.len() + 2 > MAX_ENTRIES {
                return Err("assistant_backup_too_large".into());
            }
            let destination = stage.path().join(&path);
            config::atomic_write(&destination, &bytes).map_err(io_error)?;
            m.files.push(FileInfo {
                path,
                size: bytes.len() as u64,
                sha256: digest(&bytes),
            });
        }
    }
    check_manifest(&m, owner)?;
    let manifest = serde_json::to_vec(&m).map_err(|_| invalid())?;
    if manifest.len() as u64 > MAX_MANIFEST {
        return Err("assistant_backup_too_large".into());
    }
    config::atomic_write_stream(target, config::AtomicOpts::default(), |file| {
        let result = (|| -> Result<(), String> {
            let mut zip = zip::ZipWriter::new(&mut *file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o600);
            zip.start_file("manifest.json", opts).map_err(io_error)?;
            zip.write_all(&manifest).map_err(io_error)?;
            for f in &m.files {
                zip.start_file(&f.path, opts).map_err(io_error)?;
                std::io::copy(
                    &mut fs::File::open(stage.path().join(&f.path)).map_err(io_error)?,
                    &mut zip,
                )
                .map_err(io_error)?;
            }
            zip.finish().map_err(io_error)?;
            guard()?;
            if file.metadata().map_err(io_error)?.len() > MAX_TOTAL {
                return Err("assistant_backup_too_large".into());
            }
            Ok(())
        })();
        result.map_err(AppError::Config)
    })
    .map_err(|e| match e {
        AppError::Config(v) => v,
        _ => io_error(e),
    })
}

pub fn inspect(path: &Path, owner: &str) -> Result<Validated, String> {
    plain(path, false)?;
    let file = fs::File::open(path).map_err(io_error)?;
    if file.metadata().map_err(io_error)?.len() > MAX_TOTAL {
        return Err("assistant_backup_too_large".into());
    }
    let mut zip = zip::ZipArchive::new(file).map_err(|_| invalid())?;
    if zip.len() > MAX_ENTRIES {
        return Err("assistant_backup_too_large".into());
    }
    let mut names = BTreeSet::new();
    for i in 0..zip.len() {
        let f = zip.by_index(i).map_err(|_| invalid())?;
        if f.is_dir()
            || !names.insert(f.name().to_owned())
            || f.unix_mode()
                .is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000))
        {
            return Err(invalid());
        }
        if f.name() != "manifest.json" {
            file_path(f.name())?;
        }
    }
    let m: Manifest = {
        let mut f = zip.by_name("manifest.json").map_err(|_| invalid())?;
        if f.size() > MAX_MANIFEST {
            return Err("assistant_backup_too_large".into());
        }
        let mut bytes = Vec::new();
        (&mut f)
            .take(MAX_MANIFEST + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        if bytes.len() as u64 > MAX_MANIFEST {
            return Err(invalid());
        }
        serde_json::from_slice(&bytes).map_err(|_| invalid())?
    };
    check_manifest(&m, owner)?;
    if names.len() != m.files.len() + 1 {
        return Err(invalid());
    }
    let stage = tempfile::tempdir().map_err(io_error)?;
    for expected in &m.files {
        let mut file = zip
            .by_name(&expected.path)
            .map_err(|_| "assistant_backup_file_missing")?;
        if file.size() != expected.size {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_FILE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        if bytes.len() as u64 != expected.size || digest(&bytes) != expected.sha256 {
            return Err("assistant_backup_checksum_failed".into());
        }
        if file_path(&expected.path)?.1.is_none() {
            native_bytes(&bytes)?;
        }
        config::atomic_write(&stage.path().join(&expected.path), &bytes).map_err(io_error)?;
    }
    for (key, raw) in &m.records {
        let (kind, sid) = row_kind(key, owner)?;
        if kind != "assistant_v1" {
            continue;
        }
        let detail = clean_detail(raw, sid)?;
        let native = stage.path().join(format!("sessions/{sid}/session.jsonl"));
        let mut roles = BTreeMap::new();
        if native.try_exists().map_err(io_error)? {
            let bytes = read_limited(&native, MAX_FILE)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
            for line in text.lines().skip(1) {
                let entry: Value = serde_json::from_str(line).map_err(|_| invalid())?;
                if entry["type"] == "message" {
                    roles.insert(
                        entry["id"].as_str().ok_or_else(invalid)?.to_string(),
                        entry["message"]["role"]
                            .as_str()
                            .ok_or_else(invalid)?
                            .to_string(),
                    );
                }
            }
        }
        for message in &detail.messages {
            if let Some(id) = &message.native_entry_id {
                if roles.get(id) != Some(&message.role) {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(Validated { manifest: m, stage })
}
fn plan(
    local: &BTreeMap<String, String>,
    owner: &str,
    root: &Path,
    archive: &Validated,
) -> Result<Plan, String> {
    check_manifest(&archive.manifest, owner)?;
    let mut plan = Plan {
        rows: BTreeMap::new(),
        sessions: vec![],
        report: Report {
            session_count: archive
                .manifest
                .records
                .keys()
                .filter(|k| k.starts_with("assistant_v1:"))
                .count(),
            file_count: archive.manifest.files.len(),
            ..Default::default()
        },
    };
    for (key, raw) in &archive.manifest.records {
        let raw = canonical(key, raw, owner)?;
        let (kind, id) = row_kind(key, owner)?;
        if kind == "assistant_memory_v1" {
            let remote: Vec<Memory> = serde_json::from_str(&raw).map_err(|_| invalid())?;
            let mut merged: Vec<Memory> = local
                .get(key)
                .map(|s| serde_json::from_str(s).map_err(|_| invalid()))
                .transpose()?
                .unwrap_or_default();
            let before = merged.len();
            for item in remote {
                if let Some(existing) = merged.iter().find(|m| m.id == item.id) {
                    if serde_json::to_value(existing).unwrap()
                        == serde_json::to_value(&item).unwrap()
                    {
                        plan.report.skipped_records += 1;
                    } else {
                        plan.report.conflicts.push(format!("memory:{}", item.id));
                    }
                } else {
                    merged.push(item);
                    plan.report.new_records += 1;
                }
            }
            if merged.len() != before {
                plan.rows.insert(
                    key.clone(),
                    serde_json::to_string(&merged).map_err(|_| invalid())?,
                );
            }
            continue;
        }
        if let Some(existing) = local.get(key) {
            if existing == &raw
                && (kind != "assistant_v1" || local_files_match(root, id, &archive.manifest.files))
            {
                plan.report.skipped_records += 1;
            } else if existing == &raw {
                plan.report.conflicts.push(format!("files:{id}"));
            } else {
                plan.report.conflicts.push(format!(
                    "{}:{id}",
                    if kind == "assistant_v1" {
                        "session"
                    } else {
                        "model"
                    }
                ));
            }
        } else if kind == "assistant_v1" && root.join(id).try_exists().map_err(io_error)? {
            plan.report.conflicts.push(format!("files:{id}"));
        } else {
            plan.rows.insert(key.clone(), raw);
            plan.report.new_records += 1;
            if kind == "assistant_v1" {
                plan.sessions.push(id.into());
            }
        }
    }
    Ok(plan)
}
pub fn preview(
    db: &Database,
    owner: &str,
    root: &Path,
    archive: &Validated,
) -> Result<Report, String> {
    let local = {
        let conn = db.conn.lock().map_err(io_error)?;
        rows(&conn, owner)?
    };
    Ok(plan(&local, owner, root, archive)?.report)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    owner: String,
    operation_id: String,
    sessions: Vec<String>,
    files: Vec<FileInfo>,
}
fn receipt_key(j: &Journal) -> String {
    format!("assistant_restore_v1:{}:{}", j.owner, j.operation_id)
}
fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        fs::File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(io_error)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn sync_tree(path: &Path) -> Result<(), String> {
    plain(path, true)?;
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if entry.file_type().map_err(io_error)?.is_dir() {
            sync_tree(&entry.path())?;
        } else {
            plain(&entry.path(), false)?;
            fs::File::open(entry.path())
                .and_then(|f| f.sync_all())
                .map_err(io_error)?;
        }
    }
    sync_directory(path)
}
fn install_directory(source: &Path, target: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH},
        };
        let from: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(io_error)?;
    }
    #[cfg(not(windows))]
    fs::rename(source, target).map_err(io_error)?;
    sync_directory(target.parent().ok_or_else(invalid)?)?;
    sync_directory(source.parent().ok_or_else(invalid)?)
}
fn local_files_match(root: &Path, sid: &str, files: &[FileInfo]) -> bool {
    files
        .iter()
        .filter(|f| file_path(&f.path).is_ok_and(|(id, _)| id == sid))
        .all(|f| {
            let Ok((_, aid)) = file_path(&f.path) else {
                return false;
            };
            let Ok(source) = source_file(root, sid, aid) else {
                return false;
            };
            let Ok(bytes) = read_limited(&source, MAX_FILE) else {
                return false;
            };
            let bytes = if aid.is_none() {
                match native_bytes(&bytes) {
                    Ok(bytes) => bytes,
                    Err(_) => return false,
                }
            } else {
                bytes
            };
            bytes.len() as u64 == f.size && digest(&bytes) == f.sha256
        })
}
fn copy_staged_session(work: &Path, sid: &str, j: &Journal, target: &Path) -> Result<(), String> {
    private_directory(target)?;
    let marker = target.join(".restore-receipt");
    if entry_exists(&marker)? && read_limited(&marker, 100)? != j.operation_id.as_bytes() {
        return Err("assistant_backup_recovery_conflict".into());
    }
    config::atomic_write(&marker, j.operation_id.as_bytes()).map_err(io_error)?;
    for f in &j.files {
        let (file_sid, aid) = file_path(&f.path)?;
        if file_sid != sid {
            continue;
        }
        let relative = f
            .path
            .strip_prefix(&format!("sessions/{sid}/"))
            .ok_or_else(invalid)?;
        if aid.is_some() {
            private_directory(&target.join("attachments"))?;
        }
        let destination = target.join(relative);
        if entry_exists(&destination)? {
            let bytes = read_limited(&destination, MAX_FILE)?;
            if bytes.len() as u64 != f.size || digest(&bytes) != f.sha256 {
                return Err("assistant_backup_recovery_conflict".into());
            }
            continue;
        }
        let bytes = read_staged(&work.join("sources"), &f.path)?;
        if digest(&bytes) != f.sha256 || bytes.len() as u64 != f.size {
            return Err("assistant_backup_checksum_failed".into());
        }
        config::atomic_write(&destination, &bytes).map_err(io_error)?;
    }
    let attachments = target.join("attachments");
    if attachments.try_exists().map_err(io_error)? {
        sync_directory(&attachments)?;
    }
    sync_directory(target)
}

fn cleanup(
    db: &Database,
    root: &Path,
    journal_root: &Path,
    j: &Journal,
    committed: bool,
) -> Result<(), String> {
    for sid in &j.sessions {
        validate_id(sid)?;
        let directory = root.join(sid);
        let marker = directory.join(".restore-receipt");
        if !directory.try_exists().map_err(io_error)? {
            continue;
        }
        plain(&directory, true)?;
        if marker.try_exists().map_err(io_error)?
            && read_limited(&marker, 100)? == j.operation_id.as_bytes()
        {
            if committed {
                fs::remove_file(marker).map_err(io_error)?;
            } else {
                install_directory(&directory, &journal_root.join(format!("rollback-{sid}")))?;
            }
        } else if !committed {
            return Err("assistant_backup_recovery_conflict".into());
        }
    }
    sync_directory(root)?;
    if !committed {
        local_data::write(|| db.set_setting(&receipt_key(j), "discarded")).map_err(io_error)?;
    }
    // Cleanup itself has a durable phase: a partially removed source tree must
    // never be interpreted as an incomplete restore on the next launch.
    let tombstone = root.join(format!(".cleanup-{}", j.operation_id));
    install_directory(journal_root, &tombstone)?;
    fs::remove_dir_all(tombstone).map_err(io_error)?;
    sync_directory(root)
}
pub fn recover(db: &Database, root: &Path, owner: &str) -> Result<(), String> {
    if !root.try_exists().map_err(io_error)? {
        return Ok(());
    }
    plain(root, true)?;
    let entries = fs::read_dir(root)
        .map_err(io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_error)?;
    for entry in entries {
        let mut name = entry.file_name().to_string_lossy().into_owned();
        let mut work_path = entry.path();
        if let Some(operation) = name.strip_prefix(".prepare-") {
            validate_id(operation)?;
            plain(&work_path, true)?;
            let receipt = db
                .get_setting(&format!("assistant_restore_v1:{owner}:{operation}"))
                .map_err(io_error)?;
            if !matches!(
                receipt.as_deref(),
                Some("preparing" | "discarded" | "committed")
            ) {
                return Err("assistant_backup_recovery_conflict".into());
            }
            if !work_path
                .join("journal.json")
                .try_exists()
                .map_err(io_error)?
                && receipt.as_deref() != Some("committed")
            {
                fs::remove_dir_all(&work_path).map_err(io_error)?;
                sync_directory(root)?;
                continue;
            }
            let restored = root.join(format!(".restore-{operation}"));
            install_directory(&work_path, &restored)?;
            work_path = restored;
            name = format!(".restore-{operation}");
        }
        if let Some(operation) = name.strip_prefix(".cleanup-") {
            validate_id(operation)?;
            plain(&work_path, true)?;
            let receipt = db
                .get_setting(&format!("assistant_restore_v1:{owner}:{operation}"))
                .map_err(io_error)?;
            if !matches!(receipt.as_deref(), Some("committed" | "discarded")) {
                return Err("assistant_backup_recovery_conflict".into());
            }
            fs::remove_dir_all(&work_path).map_err(io_error)?;
            sync_directory(root)?;
            continue;
        }
        let Some(operation) = name.strip_prefix(".restore-") else {
            continue;
        };
        validate_id(operation)?;
        plain(&work_path, true)?;
        let journal_path = work_path.join("journal.json");
        if !journal_path.try_exists().map_err(io_error)? {
            if fs::read_dir(&work_path).map_err(io_error)?.next().is_none() {
                fs::remove_dir(&work_path).map_err(io_error)?;
                continue;
            }
            return Err("assistant_backup_recovery_conflict".into());
        }
        let j: Journal = serde_json::from_slice(&read_limited(&journal_path, MAX_MANIFEST)?)
            .map_err(|_| invalid())?;
        if j.owner != owner || j.operation_id != operation || !owner_valid(owner) {
            return Err(invalid());
        }
        let committed = db
            .get_setting(&receipt_key(&j))
            .map_err(io_error)?
            .as_deref()
            == Some("committed");
        for f in &j.files {
            let (sid, _) = file_path(&f.path)?;
            if !j.sessions.iter().any(|s| s == sid) {
                return Err(invalid());
            }
        }
        if committed {
            for sid in &j.sessions {
                validate_id(sid)?;
                copy_staged_session(&work_path, sid, &j, &root.join(sid))?;
            }
        } else {
            for sid in &j.sessions {
                if db
                    .get_setting(&format!("assistant_v1:{owner}:{sid}"))
                    .map_err(io_error)?
                    .is_some()
                {
                    return Err("assistant_backup_recovery_conflict".into());
                }
            }
        }
        cleanup(db, root, &work_path, &j, committed)?;
        // Keep the small commit receipt. A resurrected directory entry after a
        // power loss must never be mistaken for an uncommitted restore.
    }
    Ok(())
}
pub fn recover_all(db: &Database) -> Result<(), String> {
    let root = config::get_app_config_dir().join("assistant");
    if !root.try_exists().map_err(io_error)? {
        return Ok(());
    }
    plain(&root, true)?;
    for entry in fs::read_dir(&root).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let owner = entry.file_name().to_string_lossy().into_owned();
        if owner_valid(&owner) {
            recover(db, &entry.path(), &owner)?;
        }
    }
    Ok(())
}
#[cfg(test)]
pub fn apply(
    db: &Database,
    owner: &str,
    root: &Path,
    archive: &Validated,
) -> Result<Report, String> {
    apply_guarded(db, owner, root, archive, || Ok(()))
}
pub fn apply_guarded(
    db: &Database,
    owner: &str,
    root: &Path,
    archive: &Validated,
    guard: impl Fn() -> Result<(), String>,
) -> Result<Report, String> {
    private_directory(root)?;
    recover(db, root, owner)?;
    for f in &archive.manifest.files {
        let bytes = read_staged(archive.stage.path(), &f.path)?;
        if bytes.len() as u64 != f.size || digest(&bytes) != f.sha256 {
            return Err("assistant_backup_checksum_failed".into());
        }
    }
    let baseline = {
        let conn = db.conn.lock().map_err(io_error)?;
        rows(&conn, owner)?
    };
    let mut p = plan(&baseline, owner, root, archive)?;
    if p.rows.is_empty() {
        return Ok(p.report);
    }
    let operation = id();
    let work = root.join(format!(".restore-{operation}"));
    let preparation = root.join(format!(".prepare-{operation}"));
    let j = Journal {
        owner: owner.into(),
        operation_id: operation,
        sessions: p.sessions.clone(),
        files: archive
            .manifest
            .files
            .iter()
            .filter(|f| {
                file_path(&f.path).is_ok_and(|(sid, _)| p.sessions.iter().any(|s| s == sid))
            })
            .cloned()
            .collect(),
    };
    // Journal is durable before any final directory is installed. SQLite receipt
    // and merged rows commit together, distinguishing rollback from crash cleanup.
    {
        let conn = db.conn.lock().map_err(io_error)?;
        if rows(&conn, owner)? != baseline {
            return Err("assistant_backup_local_changed".into());
        }
        local_data::write(|| {
            conn.execute(
                "INSERT INTO settings(key,value) VALUES(?1,'preparing')",
                [receipt_key(&j)],
            )
        })
        .map_err(io_error)?;
    }
    config::create_private_dir_all(&preparation).map_err(io_error)?;
    config::atomic_write(
        &preparation.join("journal.json"),
        &serde_json::to_vec(&j).map_err(|_| invalid())?,
    )
    .map_err(io_error)?;
    let result = (|| -> Result<(), String> {
        for f in &j.files {
            let bytes = read_staged(archive.stage.path(), &f.path)?;
            if digest(&bytes) != f.sha256 {
                return Err("assistant_backup_checksum_failed".into());
            }
            config::atomic_write(&preparation.join("sources").join(&f.path), &bytes)
                .map_err(io_error)?;
        }
        sync_tree(&preparation)?;
        sync_directory(root)?;
        install_directory(&preparation, &work)?;
        // Preserve verified sources until both SQLite and directory cleanup are
        // durable; crash recovery can reconstruct a committed missing directory.
        for sid in &p.sessions {
            let directory = work.join(sid);
            copy_staged_session(&work, sid, &j, &directory)?;
            if root.join(sid).try_exists().map_err(io_error)? {
                return Err("assistant_backup_local_changed".into());
            }
            install_directory(&directory, &root.join(sid))?;
        }
        sync_directory(&work)?;
        sync_directory(root)?;
        guard()?;
        let mut conn = db.conn.lock().map_err(io_error)?;
        if rows(&conn, owner)? != baseline {
            return Err("assistant_backup_local_changed".into());
        }
        guard()?;
        local_data::write(|| {
            let tx = conn.transaction().map_err(io_error)?;
            for (key, value) in &p.rows {
                tx.execute(
                    "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
                    params![key, value],
                )
                .map_err(io_error)?;
            }
            tx.execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,'committed')",
                [receipt_key(&j)],
            )
            .map_err(io_error)?;
            tx.commit().map_err(io_error)
        })
    })();
    if let Err(error) = result {
        // If commit succeeded but its final fsync reported an error, the receipt
        // still wins; never remove files backing committed conversation rows.
        recover(db, root, owner)?;
        return Err(error);
    }
    p.report.imported_records = p.report.new_records;
    recover(db, root, owner)?;
    Ok(p.report)
}

pub struct RestoreReceipt {
    pub owner: String,
    pub apex: String,
    pub expires: std::time::Instant,
    pub generation: u64,
    pub report: Report,
    pub archive: Validated,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::assistant::store;
    use serde_json::json;
    fn empty_fixture(tmp: &Path) -> (Database, String, String, PathBuf) {
        let db = Database::memory().unwrap();
        let owner = store::namespace("test-user", "test-org").unwrap();
        let d = store::create(
            &db,
            &owner,
            AssistantKind::Configuration,
            "test-model",
            false,
        )
        .unwrap();
        let archive = tmp.join("backup.zip");
        create(&db, &owner, &tmp.join("source"), &archive).unwrap();
        (db, owner, d.session.id, archive)
    }
    fn crafted(path: &Path, m: &Manifest, extra: Option<(&str, &[u8])>) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(m).unwrap()).unwrap();
        if let Some((name, bytes)) = extra {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    #[test]
    fn preparation_and_interrupted_rollback_cleanup_are_recoverable() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let owner = store::namespace("test-user", "test-org").unwrap();
        let root = tmp.path();
        let op = id();
        db.set_setting(&format!("assistant_restore_v1:{owner}:{op}"), "preparing")
            .unwrap();
        let preparation = root.join(format!(".prepare-{op}"));
        config::atomic_write(
            &preparation.join("unfinished.tmp"),
            b"partial journal write",
        )
        .unwrap();
        recover(&db, root, &owner).unwrap();
        assert!(!preparation.exists());
        let sid = id();
        let work = root.join(format!(".restore-{op}"));
        let j = Journal {
            owner: owner.clone(),
            operation_id: op.clone(),
            sessions: vec![sid.clone()],
            files: vec![],
        };
        config::atomic_write(&work.join("journal.json"), &serde_json::to_vec(&j).unwrap()).unwrap();
        config::atomic_write(
            &work.join(format!("rollback-{sid}")).join("remaining-file"),
            b"partial recursive cleanup",
        )
        .unwrap();
        recover(&db, root, &owner).unwrap();
        assert!(!work.exists());
        assert!(!root.join(sid).exists());
    }
    #[cfg(unix)]
    #[test]
    fn recovery_rejects_symlinked_destination_directories_and_dangling_files() {
        use std::os::unix::fs::symlink;
        for variant in ["session", "attachments", "file"] {
            let tmp = tempfile::tempdir().unwrap();
            let outside = tmp.path().join("outside");
            fs::create_dir(&outside).unwrap();
            let db = Database::memory().unwrap();
            let owner = store::namespace("test-user", "org").unwrap();
            let sid = id();
            let aid = id();
            let op = id();
            let root = tmp.path().join("root");
            let work = root.join(format!(".restore-{op}"));
            let bytes = b"never write outside";
            let file = FileInfo {
                path: format!("sessions/{sid}/attachments/{aid}"),
                size: bytes.len() as u64,
                sha256: digest(bytes),
            };
            let j = Journal {
                owner: owner.clone(),
                operation_id: op,
                sessions: vec![sid.clone()],
                files: vec![file.clone()],
            };
            config::atomic_write(&work.join("sources").join(&file.path), bytes).unwrap();
            config::atomic_write(&work.join("journal.json"), &serde_json::to_vec(&j).unwrap())
                .unwrap();
            db.set_setting(&receipt_key(&j), "committed").unwrap();
            match variant {
                "session" => symlink(&outside, root.join(&sid)).unwrap(),
                "attachments" => {
                    fs::create_dir(root.join(&sid)).unwrap();
                    symlink(&outside, root.join(&sid).join("attachments")).unwrap();
                }
                _ => {
                    fs::create_dir_all(root.join(&sid).join("attachments")).unwrap();
                    symlink(
                        outside.join("new-secret"),
                        root.join(&sid).join("attachments").join(&aid),
                    )
                    .unwrap();
                }
            }
            assert!(recover(&db, &root, &owner).is_err(), "{variant}");
            assert!(
                fs::read_dir(&outside).unwrap().next().is_none(),
                "{variant} escaped private storage"
            );
        }
    }

    #[test]
    fn journal_budget_covers_the_largest_legal_file_index() {
        let sid = id();
        let owner = store::namespace("user", "org").unwrap();
        let j = Journal {
            owner,
            operation_id: id(),
            sessions: vec![sid.clone()],
            files: (0..MAX_ENTRIES - 1)
                .map(|_| FileInfo {
                    path: format!("sessions/{sid}/attachments/{}", id()),
                    size: 1,
                    sha256: "a".repeat(64),
                })
                .collect(),
        };
        assert!((serde_json::to_vec(&j).unwrap().len() as u64) < MAX_MANIFEST);
    }

    #[test]
    fn malformed_graph_paths_checksums_versions_and_missing_files_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, owner, sid, path) = empty_fixture(tmp.path());
        let valid = inspect(&path, &owner).unwrap();
        let mut m = valid.manifest.clone();
        crafted(&path, &m, Some(("../escape", b"bad")));
        assert!(inspect(&path, &owner).is_err());
        m.format_version = 2;
        crafted(&path, &m, None);
        assert_eq!(
            inspect(&path, &owner).err().unwrap(),
            "assistant_backup_version_unsupported"
        );
        m.format_version = 1;
        for bytes in [b"{\"type\":\"session\"}\n".as_slice(), b"{\"type\":\"session\",\"version\":3,\"id\":\"header\",\"timestamp\":\"2026-10-27T00:00:00Z\",\"cwd\":\"\"}\n{\"type\":\"message\",\"id\":\"cycle\",\"parentId\":\"cycle\",\"timestamp\":\"2026-10-27T00:00:00Z\",\"message\":{\"role\":\"user\",\"content\":\"bad\"}}\n".as_slice()] {
            let name=format!("sessions/{sid}/session.jsonl");
            m.files=vec![FileInfo{path:name.clone(),size:bytes.len() as u64,sha256:digest(bytes)}];
            crafted(&path,&m,Some((&name,bytes)));assert!(inspect(&path,&owner).is_err());
        }
        let name = format!("sessions/{sid}/session.jsonl");
        let bytes=serde_json::to_vec(&json!({"type":"session","version":3,"id":"header","timestamp":"2026-10-27T00:00:00Z","cwd":""})).unwrap();
        m.files = vec![FileInfo {
            path: name.clone(),
            size: bytes.len() as u64,
            sha256: digest(b"different"),
        }];
        crafted(&path, &m, Some((&name, &bytes)));
        assert!(inspect(&path, &owner).is_err());
        crafted(&path, &m, None);
        assert!(inspect(&path, &owner).is_err());
    }
    #[test]
    fn sdk_generated_history_survives_archive_and_only_the_final_commit_locks_configuration() {
        let tmp = tempfile::tempdir().unwrap();
        let (db, owner, sid, path) = empty_fixture(tmp.path());
        let native =
            include_str!("../../../../assistant-runtime/test/fixtures/portable-session.jsonl");
        let root = tmp.path().join("source");
        config::atomic_write(&root.join(&sid).join("session.jsonl"), native.as_bytes()).unwrap();
        store::mutate(&db,&owner,&sid,|d|{
            for line in native.lines().skip(1) { let e:Value=serde_json::from_str(line).unwrap();if e["type"]!="message" {continue;}
                let role=e["message"]["role"].as_str().unwrap();let content=if role=="user" {e["message"]["content"].as_str().unwrap()} else {e["message"]["content"][0]["text"].as_str().unwrap()};
                d.messages.push(serde_json::from_value(json!({"id":id(),"role":role,"content":content,"createdAt":1,"status":"completed","nativeEntryId":e["id"],"runId":"a48cac04-e861-4e76-92e8-6a73896a095c"})).unwrap());
            }Ok(())
        }).unwrap();
        create(&db, &owner, &root, &path).unwrap();
        let archive = inspect(&path, &owner).unwrap();
        let target = Database::memory().unwrap();
        let destination = tmp.path().join("target");
        let count = std::cell::Cell::new(0);
        apply_guarded(&target, &owner, &destination, &archive, || {
            if count.get() == 0 {
                assert!(
                    target.conn.try_lock().is_ok(),
                    "file preparation must not monopolize the configuration database"
                );
                target
                    .set_setting("ordinary-config", "still-responsive")
                    .unwrap();
            }
            count.set(count.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(store::get(&target, &owner, &sid).unwrap().messages.len(), 2);
        assert_eq!(
            target.get_setting("ordinary-config").unwrap().as_deref(),
            Some("still-responsive")
        );
        assert_eq!(
            native_bytes(&fs::read(destination.join(&sid).join("session.jsonl")).unwrap()).unwrap(),
            native_bytes(native.as_bytes()).unwrap()
        );
    }

    #[test]
    fn memory_merges_by_id_and_backup_does_not_carry_transient_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let (source, owner, sid, path) = empty_fixture(tmp.path());
        let one = store::save_memory(
            &source,
            &owner,
            AssistantKind::Configuration,
            None,
            "remote one",
        )
        .unwrap();
        let two = store::save_memory(
            &source,
            &owner,
            AssistantKind::Configuration,
            None,
            "remote two",
        )
        .unwrap();
        store::remember_model(&source, &owner, AssistantKind::Configuration, "remembered").unwrap();
        store::mutate(&source, &owner, &sid, |d| {
            d.session.permission_mode = PermissionMode::FullAccess;
            d.session.active_run = Some(RunRef {
                run_id: id(),
                operation_id: id(),
            });
            d.pending_approval = Some(json!({"secret":"not-portable"}));
            Ok(())
        })
        .unwrap();
        create(&source, &owner, &tmp.path().join("source"), &path).unwrap();
        let valid = inspect(&path, &owner).unwrap();
        assert!(!valid
            .manifest
            .records
            .values()
            .any(|s| s.contains("not-portable")));
        let target = Database::memory().unwrap();
        store::save_memory(
            &target,
            &owner,
            AssistantKind::Configuration,
            Some(&one.id),
            "local edit",
        )
        .unwrap();
        let root = tmp.path().join("target");
        let report = apply(&target, &owner, &root, &valid).unwrap();
        assert_eq!(report.conflicts.len(), 1);
        let memories = store::memories(&target, &owner, AssistantKind::Configuration).unwrap();
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[0].content, "local edit");
        assert_eq!(memories[1].id, two.id);
        let detail = store::get(&target, &owner, &sid).unwrap();
        assert_eq!(detail.session.permission_mode, PermissionMode::Ask);
        assert!(detail.pending_approval.is_none());
        assert!(detail.session.active_run.is_none());
        assert_eq!(
            store::usage(&target, &owner).unwrap()["usageCoverage"],
            "incomplete"
        );
        assert!(store::usage(&target, &owner).unwrap()["estimatedCostUsd"].is_null());
        assert_eq!(
            store::remembered_model(&target, &owner, AssistantKind::Configuration)
                .unwrap()
                .as_deref(),
            Some("remembered")
        );
    }
    #[test]
    fn cancelled_commit_rolls_back_new_files_and_keeps_existing_export() {
        let tmp = tempfile::tempdir().unwrap();
        let (db, owner, sid, path) = empty_fixture(tmp.path());
        let valid = inspect(&path, &owner).unwrap();
        let target = Database::memory().unwrap();
        let root = tmp.path().join("target");
        assert!(apply_guarded(&target, &owner, &root, &valid, || Err(
            "assistant_identity_changed".into()
        ))
        .is_err());
        assert!(!root.join(&sid).exists());
        assert!(store::get(&target, &owner, &sid).is_err());
        fs::write(&path, b"keep old archive").unwrap();
        assert!(
            create_guarded(&db, &owner, &tmp.path().join("source"), &path, || Err(
                "assistant_identity_changed".into()
            ))
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"keep old archive");
    }
    #[test]
    fn crash_recovery_finishes_committed_files_or_rolls_back_owned_directories() {
        for committed in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let db = Database::memory().unwrap();
            let owner = store::namespace("test-user", "test-org").unwrap();
            let sid = id();
            let op = id();
            let root = tmp.path().join("target");
            let work = root.join(format!(".restore-{op}"));
            let bytes = b"portable payload";
            let file = FileInfo {
                path: format!("sessions/{sid}/attachments/{}", id()),
                size: bytes.len() as u64,
                sha256: digest(bytes),
            };
            let j = Journal {
                owner: owner.clone(),
                operation_id: op,
                sessions: vec![sid.clone()],
                files: vec![file.clone()],
            };
            config::atomic_write(&work.join("sources").join(&file.path), bytes).unwrap();
            config::atomic_write(&work.join("journal.json"), &serde_json::to_vec(&j).unwrap())
                .unwrap();
            if committed {
                db.set_setting(&receipt_key(&j), "committed").unwrap();
            } else {
                config::atomic_write(
                    &root.join(&sid).join(".restore-receipt"),
                    j.operation_id.as_bytes(),
                )
                .unwrap();
            }
            recover(&db, &root, &owner).unwrap();
            assert!(!work.exists());
            assert_eq!(root.join(&sid).exists(), committed);
            if committed {
                assert!(root.join(&sid).join("attachments").exists());
                // Simulate interruption after renaming cleanup but before recursive removal.
                let tombstone = root.join(format!(".cleanup-{}", j.operation_id));
                config::atomic_write(&tombstone.join("leftover"), b"partial cleanup").unwrap();
                recover(&db, &root, &owner).unwrap();
                assert!(!tombstone.exists());
                assert!(root.join(&sid).exists());
            }
        }
    }

    #[test]
    fn complete_archive_contains_native_history_and_attachment_copies_without_grants() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("source");
        let db = Database::memory().unwrap();
        let owner = store::namespace("test-user", "test-org").unwrap();
        let session =
            store::create(&db, &owner, AssistantKind::General, "test-model", true).unwrap();
        let sid = session.session.id;
        let aid = id();
        let session_root = root.join(&sid);
        std::fs::create_dir_all(session_root.join("attachments")).unwrap();
        std::fs::write(
            session_root.join("attachments").join(&aid),
            b"portable attachment",
        )
        .unwrap();
        std::fs::write(session_root.join("session.jsonl"), format!("{}\n", json!({"type":"session","version":3,"id":"native-session","timestamp":"2026-10-27T00:00:00Z","cwd":"/old/machine"}))).unwrap();
        store::mutate(&db, &owner, &sid, |d| {
            d.session.draft = "preserved draft".into();
            d.session.draft_attachment_ids = vec![aid.clone()];
            d.attachments.push(Attachment {
                id: aid.clone(),
                name: "sample.txt".into(),
                mime_type: "text/plain".into(),
                size_bytes: 19,
                status: "ready".into(),
                reason: None,
                preview_url: None,
            });
            Ok(())
        })
        .unwrap();
        let archive = tmp.path().join("complete.ofox-assistant");
        create(&db, &owner, &root, &archive).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
        assert!(zip
            .by_name(&format!("sessions/{sid}/session.jsonl"))
            .is_ok());
        assert!(zip
            .by_name(&format!("sessions/{sid}/attachments/{aid}"))
            .is_ok());
        assert!(zip.by_name("runtime/node").is_err());
        drop(zip);
        let valid = inspect(&archive, &owner).unwrap();
        let restored = Database::memory().unwrap();
        let destination = tmp.path().join("destination");
        let report = apply(&restored, &owner, &destination, &valid).unwrap();
        assert_eq!(report.imported_records, 1);
        let d = store::get(&restored, &owner, &sid).unwrap();
        assert_eq!(d.session.draft, "preserved draft");
        assert_eq!(d.session.draft_attachment_ids, vec![aid.clone()]);
        assert_eq!(d.session.permission_mode, PermissionMode::Ask);
        assert_eq!(
            std::fs::read(destination.join(&sid).join("attachments").join(&aid)).unwrap(),
            b"portable attachment"
        );
        assert_eq!(
            apply(&restored, &owner, &destination, &valid)
                .unwrap()
                .imported_records,
            0
        );
        let local_attachment = destination.join(&sid).join("attachments").join(&aid);
        fs::remove_file(&local_attachment).unwrap();
        assert_eq!(
            preview(&restored, &owner, &destination, &valid)
                .unwrap()
                .conflicts
                .len(),
            1
        );
        assert_eq!(
            apply(&restored, &owner, &destination, &valid)
                .unwrap()
                .conflicts
                .len(),
            1
        );
        assert!(!local_attachment.exists());
        config::atomic_write(&local_attachment, b"portable attachment").unwrap();
        store::mutate(&restored, &owner, &sid, |d| {
            d.session.draft = "local edit".into();
            Ok(())
        })
        .unwrap();
        let report = apply(&restored, &owner, &destination, &valid).unwrap();
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(
            store::get(&restored, &owner, &sid).unwrap().session.draft,
            "local edit"
        );
        assert_eq!(store::usage(&restored, &owner).unwrap()["sessionCount"], 1);
        assert_eq!(
            inspect(&archive, &store::namespace("another", "org").unwrap())
                .err()
                .unwrap(),
            "assistant_backup_identity_mismatch"
        );
        let f = &valid.manifest.files[0];
        std::fs::write(valid.stage.path().join(&f.path), b"corrupt staged file").unwrap();
        let empty = Database::memory().unwrap();
        let missing = tmp.path().join("must-stay-empty");
        assert!(apply(&empty, &owner, &missing, &valid).is_err());
        assert!(
            store::list(&empty, &owner, AssistantKind::General, false, "")
                .unwrap()
                .is_empty()
        );
    }
}
