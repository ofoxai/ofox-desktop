//! Manifest-based reversible session management, adapted from Magpie
//! internal/sessions/manage.go and its Claude/Codex/dsh fixtures.
//! Copyright (c) 2026 yetone, MIT; see MAGPIE_LICENSE.
//! Shared indexes and databases are deliberately excluded from every bundle.
use super::{managed, providers, SessionMeta};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

static LOCK: Mutex<()> = Mutex::new(());
const ACTIVE_WINDOW: Duration = Duration::from_secs(60);

#[cfg(test)]
thread_local! { static FINGERPRINT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
pub(crate) fn fingerprint_calls() -> usize {
    FINGERPRINT_CALLS.with(std::cell::Cell::get)
}

pub(crate) struct Bundle {
    pub paths: Vec<PathBuf>,
    pub roots: Vec<PathBuf>,
    pub member_ids: Vec<String>,
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
}

pub(crate) fn plan(meta: &SessionMeta) -> Result<Bundle, String> {
    let roots = managed::roots(&meta.provider_id)?;
    plan_in(meta, roots)
}

pub(crate) fn plan_from_listing(
    meta: &SessionMeta,
    listing: &[SessionMeta],
) -> Result<Bundle, String> {
    plan_with_listing(meta, managed::roots(&meta.provider_id)?, Some(listing))
}

pub(crate) fn plan_in(meta: &SessionMeta, roots: Vec<PathBuf>) -> Result<Bundle, String> {
    plan_with_listing(meta, roots, None)
}

fn plan_with_listing(
    meta: &SessionMeta,
    roots: Vec<PathBuf>,
    listing: Option<&[SessionMeta]>,
) -> Result<Bundle, String> {
    if providers::codex::is_archived(meta) {
        return Err("Native archived sessions are read-only in the session center".into());
    }
    if !matches!(
        meta.provider_id.as_str(),
        "claude" | "codex" | "gemini" | "pi" | "dsh"
    ) {
        return Err("Shared database/index provider is read-only in the session center".into());
    }
    let source = PathBuf::from(
        meta.source_path
            .as_deref()
            .ok_or("Missing session source")?,
    );
    if source.to_string_lossy().starts_with("sqlite:") {
        return Err("Shared SQLite sessions cannot enter the file trash".into());
    }
    managed::validate_path(&source, &roots)?;
    if meta.provider_id != "dsh" && !safe_id(&meta.session_id) {
        return Err("Session ID is unsafe for bundle planning".into());
    }
    let mut paths = Vec::new();
    let mut member_ids = vec![meta.session_id.clone()];
    match meta.provider_id.as_str() {
        "claude" => {
            // Only a top-level project rollout owns this sibling and auxiliary data.
            if source.file_stem().and_then(|s| s.to_str()) != Some(&meta.session_id)
                || source.parent().and_then(Path::parent)
                    != Some(roots[0].join("projects").as_path())
            {
                return Err("Claude session is not a top-level project rollout".into());
            }
            paths.push(source.clone());
            add_existing(&mut paths, source.with_extension(""))?;
            for name in ["file-history", "session-env"] {
                add_existing(&mut paths, roots[0].join(name).join(&meta.session_id))?;
            }
            let todos = roots[0].join("todos");
            if let Ok(entries) = fs::read_dir(&todos) {
                for entry in entries {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with(&format!("{}-", meta.session_id)) && name.ends_with(".json")
                    {
                        paths.push(entry.path());
                    }
                }
            }
        }
        "codex" => {
            // Only the thread's own rollouts matter: compare ids before any
            // costlier check, or a listing plans in time squared to its size.
            let records: Vec<SessionMeta> = listing
                .map(|records| {
                    records
                        .iter()
                        .filter(|s| s.provider_id == "codex" && s.session_id == meta.session_id)
                        .cloned()
                        .collect()
                })
                .unwrap_or_else(|| {
                    roots
                        .iter()
                        .flat_map(|root| providers::codex::scan_in(root))
                        .filter(|s| s.session_id == meta.session_id)
                        .collect()
                });
            if records.iter().any(|record| {
                record.session_id == meta.session_id && providers::codex::is_archived(record)
            }) {
                return Err(
                    "A native archived copy shares this session; file trash is unavailable".into(),
                );
            }
            for record in records {
                if record.session_id == meta.session_id {
                    if let Some(source) = record.source_path {
                        let path = PathBuf::from(source);
                        paths.push(path.clone());
                        let path = if path.extension().is_some_and(|e| e == "zst" || e == "zstd") {
                            path.with_extension("")
                        } else {
                            path
                        };
                        add_existing(&mut paths, path.clone())?;
                        for extension in ["jsonl.zst", "jsonl.zstd"] {
                            add_existing(&mut paths, path.with_extension(extension))?;
                        }
                    }
                }
            }
            for path in &paths {
                if providers::codex::parse_session(path)
                    .is_none_or(|record| record.session_id != meta.session_id)
                {
                    return Err(
                        "Codex rollout mirror has a different or unreadable session ID".into(),
                    );
                }
            }
        }
        "gemini" => {
            let (bundle_paths, ids) = gemini_bundle(meta, &roots[0], listing)?;
            paths.extend(bundle_paths);
            member_ids = ids;
        }
        "dsh" => {
            let home = &roots[0];
            let group = providers::dsh::grouped_files(home, &meta.session_id);
            if group.is_empty() {
                return Err("dsh session group is no longer available".into());
            }
            for member in group {
                if !member_ids.contains(&member.session_id) {
                    member_ids.push(member.session_id.clone());
                }
                paths.push(providers::dsh::dedicated_dir(home, &member)?);
                if safe_id(&member.session_id) {
                    add_existing(
                        &mut paths,
                        home.join("storages/session_projcache/sessions")
                            .join(format!("{}.json", member.session_id)),
                    )?;
                }
            }
        }
        _ => paths.push(source),
    }
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err("Session bundle is empty".into());
    }
    // Eliminate nested entries so a directory and its children are moved once.
    let all = paths.clone();
    paths.retain(|path| {
        !all.iter()
            .any(|other| other != path && path.starts_with(other))
    });
    for path in &paths {
        managed::validate_path(path, &roots)?;
    }
    Ok(Bundle {
        paths,
        roots,
        member_ids,
    })
}

fn gemini_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        && ![
            "chats",
            "logs",
            "tool-outputs",
            "checkpoints",
            "plans",
            "tracker",
            "memory",
            "skills",
            "bin",
        ]
        .contains(&id.to_ascii_lowercase().as_str())
}

// Dedicated artifact paths verified against Gemini CLI's sessionOperations.ts
// and storage.ts; project registry, named checkpoints, and shared plans remain.
fn gemini_bundle(
    meta: &SessionMeta,
    root: &Path,
    listing: Option<&[SessionMeta]>,
) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let source = PathBuf::from(meta.source_path.as_deref().ok_or("Missing Gemini source")?);
    let chats = source.parent().ok_or("Missing Gemini chats directory")?;
    let project = chats.parent().ok_or("Missing Gemini project directory")?;
    if chats.file_name().and_then(|n| n.to_str()) != Some("chats")
        || project.parent() != Some(root)
        || !gemini_id(&meta.session_id)
    {
        return Err("Gemini source is not a dedicated project session".into());
    }
    let main_records: Vec<(String, PathBuf)> = if let Some(listing) = listing {
        listing
            .iter()
            .filter(|record| record.provider_id == "gemini")
            .filter_map(|record| {
                let path = PathBuf::from(record.source_path.as_deref()?);
                (path.parent() == Some(chats)).then(|| (record.session_id.clone(), path))
            })
            .collect()
    } else {
        fs::read_dir(chats)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| Some((providers::gemini::session_identity(&e.path())?, e.path())))
            .collect()
    };
    let mut paths = main_records
        .iter()
        .filter(|(id, _)| id == &meta.session_id)
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
    if !paths.contains(&source) {
        return Err("Gemini session identity changed".into());
    }
    let mut ids = vec![meta.session_id.clone()];
    let mut index = 0;
    while index < ids.len() {
        if ids.len() > 10_000 {
            return Err("Gemini subagent graph is too large".into());
        }
        let id = ids[index].clone();
        index += 1;
        add_existing(
            &mut paths,
            project.join("logs").join(format!("session-{id}.jsonl")),
        )?;
        add_existing(
            &mut paths,
            project.join("tool-outputs").join(format!("session-{id}")),
        )?;
        add_existing(&mut paths, project.join(&id))?;
        let children = chats.join(&id);
        match fs::symlink_metadata(&children) {
            Ok(_) => {
                managed::validate_path(&children, &[root.to_path_buf()])?;
                paths.push(children.clone());
                for child in fs::read_dir(&children).map_err(|e| e.to_string())? {
                    let child = child.map_err(|e| e.to_string())?;
                    if !child.file_type().map_err(|e| e.to_string())?.is_file()
                        || !child
                            .path()
                            .extension()
                            .is_some_and(|e| e == "json" || e == "jsonl")
                    {
                        continue;
                    }
                    let child_id = child
                        .path()
                        .file_stem()
                        .and_then(|id| id.to_str())
                        .ok_or("Invalid Gemini subagent filename")?
                        .to_string();
                    if !gemini_id(&child_id) || main_records.iter().any(|(id, _)| id == &child_id) {
                        return Err("Gemini subagent has ambiguous ownership".into());
                    }
                    if let Some(id) = providers::gemini::session_identity(&child.path()) {
                        if id != child_id {
                            return Err("Gemini subagent filename and ID differ".into());
                        }
                    } else {
                        return Err("Gemini subagent metadata cannot be validated".into());
                    }
                    if !ids.contains(&child_id) {
                        ids.push(child_id);
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok((paths, ids))
}

fn add_existing(paths: &mut Vec<PathBuf>, path: PathBuf) -> Result<(), String> {
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            paths.push(path);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn walk(
    path: &Path,
    visit: &mut impl FnMut(&Path, &fs::Metadata) -> Result<(), String>,
) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("Cannot inspect {}: {e}", path.display()))?;
    visit(path, &meta)?;
    if meta.is_dir() {
        let mut entries = fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            walk(&entry.path(), visit)?;
        }
    }
    Ok(())
}

fn hash_path(path: &Path) -> Result<String, String> {
    let mut hash = Sha256::new();
    walk(path, &mut |p, meta| {
        hash.update(
            p.strip_prefix(path)
                .unwrap_or(p)
                .to_string_lossy()
                .as_bytes(),
        );
        hash.update([
            u8::from(meta.is_dir()),
            u8::from(meta.file_type().is_symlink()),
        ]);
        if meta.file_type().is_symlink() {
            hash.update(
                fs::read_link(p)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .as_bytes(),
            );
        } else if meta.is_file() {
            let mut file = fs::File::open(p).map_err(|e| e.to_string())?;
            let mut buf = [0; 64 * 1024];
            loop {
                let count = file.read(&mut buf).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                hash.update(&buf[..count]);
            }
        }
        Ok(())
    })?;
    Ok(format!("{:x}", hash.finalize()))
}

/// A bundle's listing revision, and its size and regular files, from one walk.
pub(crate) struct Fingerprint {
    pub hash: String,
    pub size: u64,
    pub files: Vec<(PathBuf, u64)>,
}

pub(crate) fn fingerprint(paths: &[PathBuf]) -> Result<Fingerprint, String> {
    #[cfg(test)]
    FINGERPRINT_CALLS.with(|count| count.set(count.get() + 1));
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut files = Vec::new();
    for path in paths {
        hash.update(path.to_string_lossy().as_bytes());
        // Listing revisions use recursive stat fingerprints, avoiding a body scan
        // of every conversation. Manifest payloads additionally use SHA-256 bytes.
        walk(path, &mut |p, meta| {
            hash.update(
                p.strip_prefix(path)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .as_bytes(),
            );
            hash.update(meta.len().to_le_bytes());
            hash.update(
                meta.modified()
                    .map_err(|e| e.to_string())?
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_le_bytes(),
            );
            hash.update([
                u8::from(meta.is_dir()),
                u8::from(meta.file_type().is_symlink()),
            ]);
            if meta.is_file() {
                size += meta.len();
                files.push((p.to_path_buf(), meta.len()));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                hash.update(meta.ino().to_le_bytes());
                hash.update(meta.ctime_nsec().to_le_bytes());
            }
            if meta.file_type().is_symlink() {
                hash.update(
                    fs::read_link(p)
                        .map_err(|e| e.to_string())?
                        .to_string_lossy()
                        .as_bytes(),
                );
            }
            Ok(())
        })?;
    }
    Ok(Fingerprint {
        hash: format!("{:x}", hash.finalize()),
        size,
        files,
    })
}

fn reject_active(paths: &[PathBuf], now: SystemTime) -> Result<(), String> {
    for path in paths {
        walk(path, &mut |_, meta| {
            let modified = meta.modified().map_err(|e| e.to_string())?;
            if now.duration_since(modified).unwrap_or_default() < ACTIVE_WINDOW {
                return Err(
                    "Session files were modified within the last minute; stop the CLI and retry"
                        .into(),
                );
            }
            Ok(())
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    original: PathBuf,
    name: String,
    fingerprint: String,
    moved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: u32,
    key: String,
    session: SessionMeta,
    roots: Vec<PathBuf>,
    member_ids: Vec<String>,
    deleted_at: i64,
    #[serde(default)]
    size_bytes: Option<u64>,
    state: String,
    items: Vec<Item>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    pub key: String,
    pub provider_id: String,
    pub session_id: String,
    pub title: Option<String>,
    pub project_dir: Option<String>,
    pub deleted_at: i64,
    pub state: String,
    pub error: Option<String>,
    pub item_count: usize,
    pub size_bytes: Option<u64>,
}

impl Manifest {
    fn entry(&self) -> TrashEntry {
        TrashEntry {
            key: self.key.clone(),
            provider_id: self.session.provider_id.clone(),
            session_id: self.session.session_id.clone(),
            title: self.session.title.clone(),
            project_dir: self.session.project_dir.clone(),
            deleted_at: self.deleted_at,
            state: self.state.clone(),
            error: self.error.clone(),
            item_count: self.items.len(),
            size_bytes: self.size_bytes,
        }
    }
}

fn trash_root() -> PathBuf {
    crate::config::get_app_config_dir().join("session-trash")
}

fn ensure_directory(path: &Path) -> Result<(), String> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err("Session trash directory is not a regular directory".into());
        }
    }
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save(dir: &Path, manifest: &Manifest) -> Result<(), String> {
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, manifest).map_err(|e| e.to_string())?;
    file.flush()
        .and_then(|()| file.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    file.persist(dir.join("manifest.json"))
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        fs::File::open(dir)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn occupied(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

fn validate_target(path: &Path, roots: &[PathBuf]) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Invalid original session path".into());
    }
    let root = roots
        .iter()
        .find(|r| path.starts_with(r) && path != r.as_path())
        .ok_or("Restore destination is outside its provider root")?;
    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("Provider root changed: {e}"))?;
    let mut parent = path.parent().ok_or("Missing original parent")?;
    loop {
        match fs::symlink_metadata(parent) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err("Restore destination has a symbolic link ancestor".into());
                }
                if !parent
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(&canonical_root)
                {
                    return Err("Restore destination escaped its root".into());
                }
                // Continue to the configured root to reject links higher in the path.
                if parent == root {
                    break;
                }
                parent = parent.parent().ok_or("Missing restore root")?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                parent = parent.parent().ok_or("Missing restore root")?
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn read_manifest(root: &Path, key: &str) -> Result<(PathBuf, Manifest), String> {
    read_manifest_with_roots(root, key, None)
}

fn read_manifest_with_roots(
    root: &Path,
    key: &str,
    roots_override: Option<&[PathBuf]>,
) -> Result<(PathBuf, Manifest), String> {
    if uuid::Uuid::parse_str(key).is_err() {
        return Err("Invalid trash reference".into());
    }
    let dir = root.join(key);
    let meta = fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("Invalid trash directory".into());
    }
    let manifest_path = dir.join("manifest.json");
    let stat = fs::symlink_metadata(&manifest_path).map_err(|e| e.to_string())?;
    if !stat.is_file() || stat.file_type().is_symlink() || stat.len() > 1024 * 1024 {
        return Err("Invalid trash manifest".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if manifest.version != 1 || manifest.key != key || manifest.items.is_empty() {
        return Err("Unsupported or invalid trash manifest".into());
    }
    let current_roots = match roots_override {
        Some(roots) => roots.to_vec(),
        None => managed::roots(&manifest.session.provider_id)?,
    };
    if current_roots != manifest.roots {
        return Err("Provider directory changed since this session entered trash".into());
    }
    for (index, item) in manifest.items.iter().enumerate() {
        if item.name != format!("{index:04}") {
            return Err("Invalid trash item name".into());
        }
        validate_target(&item.original, &current_roots)?;
        validate_item_scope(&manifest, item)?;
    }
    let files = dir.join("files");
    if fs::symlink_metadata(&files).is_err()
        || fs::symlink_metadata(&files).is_ok_and(|m| !m.is_dir() || m.file_type().is_symlink())
    {
        return Err("Invalid trash files directory".into());
    }
    Ok((dir, manifest))
}

fn validate_item_scope(manifest: &Manifest, item: &Item) -> Result<(), String> {
    let provider = manifest.session.provider_id.as_str();
    let id = manifest.session.session_id.as_str();
    let primary = manifest
        .session
        .source_path
        .as_deref()
        .map(Path::new)
        .ok_or("Missing original session source")?;
    let root = manifest
        .roots
        .iter()
        .find(|r| item.original.starts_with(r))
        .ok_or("Invalid item root")?;
    let parts: Vec<_> = item
        .original
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let valid = match provider {
        "claude" if safe_id(id) => match parts.as_slice() {
            [kind, _, name] if kind == "projects" => name == id || name == &format!("{id}.jsonl"),
            [kind, name] if kind == "file-history" || kind == "session-env" => name == id,
            [kind, name] if kind == "todos" => {
                name.starts_with(&format!("{id}-")) && name.ends_with(".json")
            }
            _ => false,
        },
        "codex" => item.original.file_name().is_some_and(|name| {
            let name = name.to_string_lossy();
            name != "session_index.jsonl"
                && (name.ends_with(".jsonl")
                    || name.ends_with(".jsonl.zst")
                    || name.ends_with(".jsonl.zstd"))
        }),
        "gemini" => match parts.as_slice() {
            [_, chats, filename] if chats == "chats" => {
                item.original == primary
                    || manifest.member_ids.contains(filename)
                    || (filename.starts_with("session-")
                        && (filename.ends_with(".json") || filename.ends_with(".jsonl")))
            }
            [_, logs, name] if logs == "logs" => manifest
                .member_ids
                .iter()
                .any(|id| gemini_id(id) && name == &format!("session-{id}.jsonl")),
            [_, outputs, name] if outputs == "tool-outputs" => manifest
                .member_ids
                .iter()
                .any(|id| gemini_id(id) && name == &format!("session-{id}")),
            [_, directory] => gemini_id(directory) && manifest.member_ids.contains(directory),
            _ => false,
        },
        "pi" => {
            item.original == primary
                && manifest.items.len() == 1
                && primary.extension().is_some_and(|e| e == "jsonl")
        }
        "dsh" => match parts.as_slice() {
            [kind, _, segment] if kind == "sessions" => {
                !segment.is_empty() && segment != "." && segment != ".."
            }
            [storages, cache, sessions, name] => {
                storages == "storages"
                    && cache == "session_projcache"
                    && sessions == "sessions"
                    && name.strip_suffix(".json").is_some_and(safe_id)
            }
            _ => false,
        },
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("Manifest item is outside the provider's dedicated session bundle layout".into())
    }
}

fn remove_path(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn copy_path(from: &Path, to: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        let target = fs::read_link(from)?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, to)?;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileTypeExt;
            use std::os::windows::fs::{symlink_dir, symlink_file};
            if meta.file_type().is_symlink_dir() {
                symlink_dir(target, to)?;
            } else {
                symlink_file(target, to)?;
            }
        }
    } else if meta.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_path(&entry.path(), &to.join(entry.file_name()))?;
        }
        fs::set_permissions(to, meta.permissions())?;
    } else if meta.is_file() {
        let mut source = fs::File::open(from)?;
        let mut target = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to)?;
        io::copy(&mut source, &mut target)?;
        target.set_permissions(meta.permissions())?;
        target.sync_all()?;
        target.set_times(
            fs::FileTimes::new()
                .set_modified(meta.modified()?)
                .set_accessed(meta.accessed()?),
        )?;
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unsupported session node",
        ));
    }
    Ok(())
}

fn cross_device(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::EXDEV)
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(17)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = error;
        false
    }
}

fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    move_with(from, to, rename_no_replace)
}

fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let from = std::ffi::CString::new(from.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid source path"))?;
        let to = std::ffi::CString::new(to.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid destination path"))?;
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            ) as libc::c_int
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Exclusive rename is unavailable on this platform",
        ));
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileW(existing: *const u16, new: *const u16) -> i32;
        }
        let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileW(from.as_ptr(), to.as_ptr()) } != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (from, to);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Exclusive rename unavailable",
        ))
    }
}

fn move_with(
    from: &Path,
    to: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    move_with_ops(from, to, rename, remove_path)
}

fn move_with_ops(
    from: &Path,
    to: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
    remove_source: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), String> {
    if occupied(to)? {
        return Err(format!("Destination is occupied: {}", to.display()));
    }
    fs::create_dir_all(to.parent().ok_or("Missing move parent")?).map_err(|e| e.to_string())?;
    match rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if cross_device(&error) => {
            let before = hash_path(from)?;
            if let Err(error) = copy_path(from, to) {
                let _ = remove_path(to);
                return Err(error.to_string());
            }
            if hash_path(to)? != before || hash_path(from)? != before {
                let _ = remove_path(to);
                return Err("Session changed during cross-device copy".into());
            }
            if let Err(error) = remove_source(from) {
                // Directory removal can fail after deleting some children. Keep
                // the verified complete copy for recovery rather than discarding it.
                return Err(format!(
                    "Source cleanup failed; complete copy retained at {}: {error}",
                    to.display()
                ));
            }
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn compensate(dir: &Path, manifest: &mut Manifest, restoring: bool) -> Result<(), String> {
    for index in (0..manifest.items.len()).rev() {
        let item = &manifest.items[index];
        let trash = dir.join("files").join(&item.name);
        let source_exists = occupied(&item.original)?;
        let trash_exists = occupied(&trash)?;
        match (source_exists, trash_exists) {
            (true, true) | (false, false) => {
                return Err(
                    "Both locations exist or both are missing; manual recovery is required".into(),
                )
            }
            (false, true) if !restoring => {
                if hash_path(&trash)? != item.fingerprint {
                    return Err(
                        "Trashed data changed; compensation requires manual recovery".into(),
                    );
                }
                validate_target(&item.original, &manifest.roots)?;
                move_path(&trash, &item.original)?;
            }
            (true, false) if restoring => {
                if hash_path(&item.original)? != item.fingerprint {
                    return Err(
                        "Restored data changed; compensation requires manual recovery".into(),
                    );
                }
                move_path(&item.original, &trash)?;
            }
            _ => {}
        }
        manifest.items[index].moved = restoring;
        save(dir, manifest)?;
    }
    Ok(())
}

fn recover(root: &Path) -> Result<(), String> {
    recover_with_roots(root, None)
}

fn recover_with_roots(root: &Path, roots_override: Option<&[PathBuf]>) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    ensure_directory(root)?;
    for entry in fs::read_dir(root).map_err(|e| e.to_string())?.flatten() {
        let key = entry.file_name().to_string_lossy().to_string();
        let Ok((dir, mut manifest)) = read_manifest_with_roots(root, &key, roots_override) else {
            continue;
        };
        if manifest.state == "restored" {
            let intact = manifest
                .items
                .iter()
                .all(|item| hash_path(&item.original).is_ok_and(|hash| hash == item.fingerprint));
            if intact {
                fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            } else {
                manifest.state = "recoveryRequired".into();
                manifest.error = Some("Restored data changed before transaction cleanup".into());
                save(&dir, &manifest)?;
            }
            continue;
        }
        if !matches!(manifest.state.as_str(), "preparing" | "restoring") {
            continue;
        }
        let restoring = manifest.state == "restoring";
        match compensate(&dir, &mut manifest, restoring) {
            Ok(()) if !restoring => fs::remove_dir_all(&dir).map_err(|e| e.to_string())?,
            Ok(()) => {
                manifest.state = "trashed".into();
                manifest.error = None;
                save(&dir, &manifest)?;
            }
            Err(error) => {
                manifest.state = "recoveryRequired".into();
                manifest.error = Some(error);
                save(&dir, &manifest)?;
            }
        }
    }
    Ok(())
}

pub fn list() -> Result<Vec<TrashEntry>, String> {
    let _guard = LOCK.lock().map_err(|e| e.to_string())?;
    list_in(&trash_root())
}

fn list_in(root: &Path) -> Result<Vec<TrashEntry>, String> {
    recover(root)?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let key = entry.file_name().to_string_lossy().into_owned();
        if uuid::Uuid::parse_str(&key).is_err() {
            continue;
        }
        entries.push(match read_manifest(root, &key) {
            Ok((_, manifest)) => manifest.entry(),
            Err(error) => TrashEntry {
                key: key.clone(),
                provider_id: "unknown".into(),
                session_id: key,
                title: None,
                project_dir: None,
                deleted_at: 0,
                state: "recoveryRequired".into(),
                error: Some(error),
                item_count: 0,
                size_bytes: None,
            },
        });
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.deleted_at));
    Ok(entries)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashRequest {
    pub session_key: String,
    pub revision: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashOutcome {
    pub session_key: String,
    pub success: bool,
    pub trash_key: Option<String>,
    pub error: Option<String>,
}

pub fn trash_many(requests: &[TrashRequest]) -> Vec<TrashOutcome> {
    requests
        .iter()
        .map(|request| match trash_one(request) {
            Ok(entry) => TrashOutcome {
                session_key: request.session_key.clone(),
                success: true,
                trash_key: Some(entry.key),
                error: None,
            },
            Err(error) => TrashOutcome {
                session_key: request.session_key.clone(),
                success: false,
                trash_key: None,
                error: Some(error),
            },
        })
        .collect()
}

fn trash_one(request: &TrashRequest) -> Result<TrashEntry, String> {
    let _guard = LOCK.lock().map_err(|e| e.to_string())?;
    let root = trash_root();
    ensure_directory(&root)?;
    recover(&root)?;
    let record = managed::resolve(&request.session_key)?;
    let bundle = plan(&record.meta)?;
    let trashed = trash_bundle(
        &root,
        record.meta,
        bundle,
        &request.revision,
        SystemTime::now(),
    );
    managed::forget_listing();
    trashed
}

fn trash_bundle(
    root: &Path,
    session: SessionMeta,
    bundle: Bundle,
    revision: &str,
    now: SystemTime,
) -> Result<TrashEntry, String> {
    reject_active(&bundle.paths, now)?;
    let bundle_fingerprint = fingerprint(&bundle.paths)?;
    if bundle_fingerprint.hash != revision {
        return Err("Session bundle changed; refresh before moving it to trash".into());
    }
    let key = uuid::Uuid::new_v4().to_string();
    let dir = root.join(&key);
    let items = bundle
        .paths
        .iter()
        .enumerate()
        .map(|(index, original)| {
            Ok(Item {
                original: original.clone(),
                name: format!("{index:04}"),
                fingerprint: hash_path(original)?,
                moved: false,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    ensure_directory(&dir)?;
    ensure_directory(&dir.join("files"))?;
    let mut manifest = Manifest {
        version: 1,
        key,
        session,
        roots: bundle.roots,
        member_ids: bundle.member_ids,
        deleted_at: chrono::Utc::now().timestamp_millis(),
        size_bytes: Some(bundle_fingerprint.size),
        state: "preparing".into(),
        items,
        error: None,
    };
    if let Err(error) = save(&dir, &manifest) {
        let _ = fs::remove_dir_all(&dir);
        return Err(error);
    }
    let result = (|| {
        for index in 0..manifest.items.len() {
            let item = &manifest.items[index];
            managed::validate_path(&item.original, &manifest.roots)?;
            reject_active(std::slice::from_ref(&item.original), now)?;
            if hash_path(&item.original)? != item.fingerprint {
                return Err("Session changed while moving its bundle".into());
            }
            move_path(&item.original, &dir.join("files").join(&item.name))?;
            manifest.items[index].moved = true;
            save(&dir, &manifest)?;
        }
        manifest.state = "trashed".into();
        save(&dir, &manifest)
    })();
    if let Err(error) = result {
        match compensate(&dir, &mut manifest, false) {
            Ok(()) => fs::remove_dir_all(&dir).map_err(|e| e.to_string())?,
            Err(recovery) => {
                manifest.state = "recoveryRequired".into();
                manifest.error = Some(format!("{error}; {recovery}"));
                save(&dir, &manifest)?;
                return Err(format!(
                    "{error}; recovery required (trash {})",
                    manifest.key
                ));
            }
        }
        return Err(error);
    }
    Ok(manifest.entry())
}

pub fn restore(key: &str) -> Result<TrashEntry, String> {
    let _guard = LOCK.lock().map_err(|e| e.to_string())?;
    let root = trash_root();
    recover(&root)?;
    let restored = restore_in(&root, key);
    managed::forget_listing();
    restored
}

fn restore_in(root: &Path, key: &str) -> Result<TrashEntry, String> {
    restore_with_roots(root, key, None)
}

fn restore_with_roots(
    root: &Path,
    key: &str,
    roots_override: Option<&[PathBuf]>,
) -> Result<TrashEntry, String> {
    let (dir, mut manifest) = read_manifest_with_roots(root, key, roots_override)?;
    if manifest.state != "trashed" {
        return Err("Trash transaction needs recovery before restore".into());
    }
    for item in &manifest.items {
        if occupied(&item.original)? {
            return Err(format!(
                "Restore destination is occupied: {}",
                item.original.display()
            ));
        }
        if hash_path(&dir.join("files").join(&item.name))? != item.fingerprint {
            return Err("Trashed session fingerprint changed".into());
        }
    }
    manifest.state = "restoring".into();
    save(&dir, &manifest)?;
    let result = (|| {
        for index in 0..manifest.items.len() {
            let item = &manifest.items[index];
            validate_target(&item.original, &manifest.roots)?;
            move_path(&dir.join("files").join(&item.name), &item.original)?;
            manifest.items[index].moved = false;
            save(&dir, &manifest)?;
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = result {
        match compensate(&dir, &mut manifest, true) {
            Ok(()) => {
                manifest.state = "trashed".into();
                manifest.error = None;
                save(&dir, &manifest)?;
            }
            Err(recovery) => {
                manifest.state = "recoveryRequired".into();
                manifest.error = Some(recovery);
                save(&dir, &manifest)?;
            }
        }
        return Err(error);
    }
    let entry = manifest.entry();
    manifest.state = "restored".into();
    save(&dir, &manifest)?;
    fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    Ok(entry)
}

pub fn purge(key: &str) -> Result<(), String> {
    let _guard = LOCK.lock().map_err(|e| e.to_string())?;
    let root = trash_root();
    recover(&root)?;
    let (dir, manifest) = read_manifest(&root, key)?;
    if manifest.state != "trashed" {
        return Err("A transaction requiring recovery cannot be purged".into());
    }
    fs::remove_dir_all(dir).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn meta(provider: &str, id: &str, source: &Path) -> SessionMeta {
        SessionMeta {
            provider_id: provider.into(),
            session_id: id.into(),
            title: Some("fixture".into()),
            summary: None,
            project_dir: None,
            created_at: None,
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into()),
            resume_command: None,
            title_source: None,
            message_count: None,
            parent_session_id: None,
        }
    }
    // Advance comparison time instead of touching process-global env or personal configs.
    fn idle_now() -> SystemTime {
        SystemTime::now() + Duration::from_secs(120)
    }
    #[test]
    fn claude_bundle_and_active_subagent_guard() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path();
        let source = root.join("projects/p/s.jsonl");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "main").unwrap();
        for (name, contents) in [
            ("projects/p/s/subagents/agent.jsonl", "child"),
            ("file-history/s/1", "history"),
            ("session-env/s/env", "env"),
            ("todos/s-1.json", "todo"),
            ("todos/other-1.json", "other"),
        ] {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        let bundle = plan_in(&meta("claude", "s", &source), vec![root.into()]).unwrap();
        assert_eq!(bundle.paths.len(), 5);
        assert!(reject_active(&bundle.paths, SystemTime::now()).is_err());
        assert!(reject_active(&bundle.paths, idle_now()).is_ok());
        let child = root.join("projects/p/s/subagents/agent.jsonl");
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&child)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(SystemTime::now() + Duration::from_secs(600)),
            )
            .unwrap();
        assert!(reject_active(&bundle.paths, idle_now()).is_err());
        assert!(!bundle.paths.contains(&root.join("todos/other-1.json")));
    }
    #[test]
    fn bundle_revision_changes_and_shared_database_rejected() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("s.json");
        fs::write(&path, "before").unwrap();
        let paths = vec![path.clone()];
        let before = fingerprint(&paths).unwrap().hash;
        fs::write(&path, "after").unwrap();
        assert_ne!(before, fingerprint(&paths).unwrap().hash);
        assert!(plan_in(&meta("hermes", "s", &path), vec![home.path().into()]).is_err());
    }
    #[test]
    fn rename_permission_error_never_copies_and_exdev_does() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, "original").unwrap();
        assert!(move_with(&source, &target, |_, _| Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "locked"
        )))
        .is_err());
        assert!(source.exists());
        assert!(!target.exists());
        #[cfg(unix)]
        {
            move_with(&source, &target, |_, _| {
                Err(io::Error::from_raw_os_error(libc::EXDEV))
            })
            .unwrap();
            assert!(!source.exists());
            assert_eq!(fs::read_to_string(target).unwrap(), "original");
        }
    }

    #[test]
    #[cfg(unix)]
    fn cross_device_partial_source_cleanup_keeps_the_complete_copy() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("a"), "a").unwrap();
        fs::write(source.join("b"), "b").unwrap();
        let error = move_with_ops(
            &source,
            &target,
            |_, _| Err(io::Error::from_raw_os_error(libc::EXDEV)),
            |source| {
                fs::remove_file(source.join("a"))?;
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "second node locked",
                ))
            },
        )
        .unwrap_err();
        assert!(error.contains("complete copy retained"));
        assert_eq!(fs::read_to_string(target.join("a")).unwrap(), "a");
        assert_eq!(fs::read_to_string(target.join("b")).unwrap(), "b");
        assert!(source.join("b").exists());
    }
    #[test]
    fn symlinks_preserved_without_following_their_targets() {
        #[cfg(unix)]
        {
            let root = tempfile::tempdir().unwrap();
            let from = root.path().join("from");
            let to = root.path().join("to");
            fs::create_dir(&from).unwrap();
            std::os::unix::fs::symlink("../missing", from.join("link")).unwrap();
            copy_path(&from, &to).unwrap();
            assert_eq!(
                fs::read_link(to.join("link")).unwrap(),
                PathBuf::from("../missing")
            );
        }
    }

    #[test]
    fn claude_roundtrip_occupied_restore_and_fingerprint_guard() {
        let home = tempfile::tempdir().unwrap();
        let trash = tempfile::tempdir().unwrap();
        let source = home.path().join("projects/p/s.jsonl");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "main").unwrap();
        for name in [
            "projects/p/s/subagents/child.jsonl",
            "file-history/s/1",
            "session-env/s/env",
            "todos/s-1.json",
        ] {
            let path = home.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name).unwrap();
        }
        let roots = vec![home.path().into()];
        let session = meta("claude", "s", &source);
        let bundle = plan_in(&session, roots.clone()).unwrap();
        let revision = fingerprint(&bundle.paths).unwrap().hash;
        let entry = trash_bundle(trash.path(), session, bundle, &revision, idle_now()).unwrap();
        assert!(!source.exists());
        fs::write(&source, "occupied").unwrap();
        assert!(restore_with_roots(trash.path(), &entry.key, Some(&roots))
            .unwrap_err()
            .contains("occupied"));
        assert_eq!(fs::read_to_string(&source).unwrap(), "occupied");
        fs::remove_file(&source).unwrap();
        let (dir, manifest) =
            read_manifest_with_roots(trash.path(), &entry.key, Some(&roots)).unwrap();
        let saved = dir.join("files").join(
            &manifest
                .items
                .iter()
                .find(|item| item.original == source)
                .unwrap()
                .name,
        );
        let before = fs::read(&saved).unwrap();
        fs::write(&saved, "tampered").unwrap();
        assert!(restore_with_roots(trash.path(), &entry.key, Some(&roots))
            .unwrap_err()
            .contains("fingerprint"));
        fs::write(&saved, before).unwrap();
        restore_with_roots(trash.path(), &entry.key, Some(&roots)).unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "main");
        assert!(home.path().join("file-history/s/1").exists());
        assert!(!dir.exists());
    }

    #[test]
    fn codex_all_rollout_segments_and_compressed_twins_leave_indexes_untouched() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("sessions");
        fs::create_dir_all(&root).unwrap();
        let header = "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s\"}}\n";
        for name in ["rollout-a.jsonl", "rollout-b.jsonl"] {
            fs::write(root.join(name), header).unwrap();
        }
        fs::write(
            root.join("rollout-a.jsonl.zst"),
            zstd::stream::encode_all(header.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("other.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"other\"}}\n",
        )
        .unwrap();
        fs::write(home.path().join("session_index.jsonl"), "shared").unwrap();
        fs::write(home.path().join("state.db"), "shared").unwrap();
        let session = meta("codex", "s", &root.join("rollout-a.jsonl"));
        let bundle = plan_in(&session, vec![root]).unwrap();
        assert_eq!(bundle.paths.len(), 3);
        assert!(!bundle
            .paths
            .iter()
            .any(|p| p.ends_with("session_index.jsonl") || p.ends_with("state.db")));
    }

    #[test]
    fn dsh_complete_generations_children_locks_and_projection_rows_roundtrip() {
        let home = tempfile::tempdir().unwrap();
        let trash = tempfile::tempdir().unwrap();
        for (id, parent) in [("root", ""), ("child", "root"), ("other", "")] {
            let dir = home.path().join("sessions/project").join(id);
            fs::create_dir_all(&dir).unwrap();
            let header = format!(
                r#"{{"type":"session","id":"{id}","version":4,"createdAt":1000,"parentSession":"{parent}","delegationDepth":{}}}"#,
                if parent.is_empty() { 0 } else { 1 }
            );
            let compressed = zstd::stream::encode_all(format!("{header}\n").as_bytes(), 1).unwrap();
            fs::write(dir.join("session.v4.jsonl.zstd"), compressed).unwrap();
            fs::write(dir.join("session.v3.jsonl"), format!("{header}\n")).unwrap();
            fs::write(dir.join("session.lock"), "").unwrap();
            let cache = home.path().join("storages/session_projcache/sessions");
            fs::create_dir_all(&cache).unwrap();
            fs::write(cache.join(format!("{id}.json")), "{}").unwrap();
        }
        let roots = vec![home.path().into()];
        let session = providers::dsh::scan_in(home.path())
            .into_iter()
            .find(|s| s.session_id == "root")
            .unwrap();
        let bundle = plan_in(&session, roots.clone()).unwrap();
        assert_eq!(bundle.paths.len(), 4);
        let revision = fingerprint(&bundle.paths).unwrap().hash;
        let entry = trash_bundle(trash.path(), session, bundle, &revision, idle_now()).unwrap();
        assert!(home
            .path()
            .join("sessions/project/other/session.lock")
            .exists());
        assert!(!home.path().join("sessions/project/child").exists());
        restore_with_roots(trash.path(), &entry.key, Some(&roots)).unwrap();
        assert!(home
            .path()
            .join("sessions/project/root/session.v3.jsonl")
            .exists());
        assert!(home
            .path()
            .join("sessions/project/child/session.lock")
            .exists());
        assert!(home
            .path()
            .join("storages/session_projcache/sessions/child.json")
            .exists());
    }

    #[test]
    fn trash_listing_keeps_bundle_size_without_reading_live_files() {
        let home = tempfile::tempdir().unwrap();
        let trash = tempfile::tempdir().unwrap();
        let source = home.path().join("s.jsonl");
        fs::write(&source, "original").unwrap();
        let session = meta("pi", "s", &source);
        let bundle = plan_in(&session, vec![home.path().into()]).unwrap();
        let revision = fingerprint(&bundle.paths).unwrap().hash;
        let entry = trash_bundle(trash.path(), session, bundle, &revision, idle_now()).unwrap();
        assert_eq!(serde_json::to_value(entry).unwrap()["sizeBytes"], 8);
    }

    #[test]
    fn crash_between_move_and_manifest_progress_is_compensated() {
        let home = tempfile::tempdir().unwrap();
        let trash = tempfile::tempdir().unwrap();
        let source = home.path().join("s.jsonl");
        fs::write(&source, "original").unwrap();
        let roots = vec![home.path().into()];
        let key = uuid::Uuid::new_v4().to_string();
        let dir = trash.path().join(&key);
        ensure_directory(&dir.join("files")).unwrap();
        let mut manifest = Manifest {
            version: 1,
            key: key.clone(),
            session: meta("pi", "s", &source),
            roots: roots.clone(),
            member_ids: vec!["s".into()],
            deleted_at: 1,
            size_bytes: None,
            state: "preparing".into(),
            items: vec![Item {
                original: source.clone(),
                name: "0000".into(),
                fingerprint: hash_path(&source).unwrap(),
                moved: false,
            }],
            error: None,
        };
        save(&dir, &manifest).unwrap();
        move_path(&source, &dir.join("files/0000")).unwrap();
        recover_with_roots(trash.path(), Some(&roots)).unwrap();
        assert!(source.exists());
        assert!(!dir.exists());
        ensure_directory(&dir.join("files")).unwrap();
        move_path(&source, &dir.join("files/0000")).unwrap();
        manifest.state = "restoring".into();
        manifest.items[0].moved = true;
        save(&dir, &manifest).unwrap();
        move_path(&dir.join("files/0000"), &source).unwrap();
        recover_with_roots(trash.path(), Some(&roots)).unwrap();
        assert!(!source.exists());
        assert_eq!(
            read_manifest_with_roots(trash.path(), &key, Some(&roots))
                .unwrap()
                .1
                .state,
            "trashed"
        );
    }

    #[test]
    fn pi_custom_directory_bundle_and_wrong_dsh_directory_are_safe() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("date_s.jsonl");
        fs::write(&file, "{}").unwrap();
        assert_eq!(
            plan_in(&meta("pi", "s", &file), vec![root.path().into()])
                .unwrap()
                .paths,
            [file]
        );
        let wrong = root.path().join("sessions/project/shared/session.v4.jsonl");
        fs::create_dir_all(wrong.parent().unwrap()).unwrap();
        fs::write(
            &wrong,
            "{\"type\":\"session\",\"id\":\"s\",\"version\":4}\n",
        )
        .unwrap();
        assert!(plan_in(&meta("dsh", "s", &wrong), vec![root.path().into()]).is_err());
    }

    #[test]
    fn gemini_and_pi_roundtrip_and_stale_revision_refuses_before_any_move() {
        for (provider, relative) in [
            ("gemini", "project/chats/session-s.json"),
            ("pi", "date_s.jsonl"),
        ] {
            let home = tempfile::tempdir().unwrap();
            let trash = tempfile::tempdir().unwrap();
            let source = home.path().join(relative);
            fs::create_dir_all(source.parent().unwrap()).unwrap();
            let original = if provider == "gemini" {
                "{\"sessionId\":\"s\",\"messages\":[{\"type\":\"user\",\"content\":\"original\"}]}"
            } else {
                "original"
            };
            let edited = if provider == "gemini" {
                "{\"sessionId\":\"s\",\"messages\":[{\"type\":\"user\",\"content\":\"edited body\"}]}"
            } else {
                "edited body"
            };
            fs::write(&source, original).unwrap();
            let roots = vec![home.path().into()];
            let session = meta(provider, "s", &source);
            let bundle = plan_in(&session, roots.clone()).unwrap();
            let stale = fingerprint(&bundle.paths).unwrap().hash;
            fs::write(&source, edited).unwrap();
            assert!(
                trash_bundle(trash.path(), session.clone(), bundle, &stale, idle_now())
                    .unwrap_err()
                    .contains("changed")
            );
            assert_eq!(fs::read_dir(trash.path()).unwrap().count(), 0);
            let bundle = plan_in(&session, roots.clone()).unwrap();
            let revision = fingerprint(&bundle.paths).unwrap().hash;
            let entry = trash_bundle(trash.path(), session, bundle, &revision, idle_now()).unwrap();
            assert!(!source.exists());
            restore_with_roots(trash.path(), &entry.key, Some(&roots)).unwrap();
            assert_eq!(fs::read_to_string(source).unwrap(), edited);
        }
    }

    #[test]
    fn manifest_cannot_restore_shared_database_or_escape_provider_root() {
        let home = tempfile::tempdir().unwrap();
        let trash = tempfile::tempdir().unwrap();
        let source = home.path().join("session.jsonl");
        fs::write(&source, "original").unwrap();
        let key = uuid::Uuid::new_v4().to_string();
        let dir = trash.path().join(&key);
        ensure_directory(&dir.join("files")).unwrap();
        let mut manifest = Manifest {
            version: 1,
            key: key.clone(),
            session: meta("codex", "s", &source),
            roots: vec![home.path().into()],
            member_ids: vec!["s".into()],
            deleted_at: 1,
            size_bytes: None,
            state: "trashed".into(),
            items: vec![Item {
                original: home.path().join("state.db"),
                name: "0000".into(),
                fingerprint: "unused".into(),
                moved: true,
            }],
            error: None,
        };
        save(&dir, &manifest).unwrap();
        assert!(
            read_manifest_with_roots(trash.path(), &key, Some(&manifest.roots))
                .unwrap_err()
                .contains("bundle layout")
        );
        manifest.items[0].original = home.path().join("../outside.jsonl");
        save(&dir, &manifest).unwrap();
        assert!(read_manifest_with_roots(trash.path(), &key, Some(&manifest.roots)).is_err());
    }

    #[test]
    fn gemini_complete_bundle_includes_migration_children_and_owned_artifacts() {
        let home = tempfile::tempdir().unwrap();
        let trash = tempfile::tempdir().unwrap();
        let project = home.path().join("project");
        let chats = project.join("chats");
        fs::create_dir_all(&chats).unwrap();
        let main = chats.join("session-s.jsonl");
        fs::write(&main, "{\"sessionId\":\"s\",\"projectHash\":\"p\"}\n{\"id\":\"m\",\"type\":\"user\",\"content\":\"hello\"}\n").unwrap();
        fs::write(
            chats.join("session-s.json"),
            "{\"sessionId\":\"s\",\"messages\":[]}",
        )
        .unwrap();
        let children = chats.join("s");
        fs::create_dir(&children).unwrap();
        fs::write(
            children.join("child.jsonl"),
            "{\"sessionId\":\"child\",\"projectHash\":\"p\",\"kind\":\"subagent\"}\n",
        )
        .unwrap();
        for name in [
            "logs/session-s.jsonl",
            "logs/session-child.jsonl",
            "tool-outputs/session-s/a",
            "tool-outputs/session-child/b",
            "s/plans/plan.md",
            "child/tracker/task.json",
            "checkpoints/named.json",
            "plans/shared.md",
            ".project_root",
        ] {
            let file = project.join(name);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, name).unwrap();
        }
        let session = meta("gemini", "s", &main);
        let roots = vec![home.path().into()];
        let bundle = plan_in(&session, roots.clone()).unwrap();
        assert_eq!(bundle.paths.len(), 9);
        assert_eq!(bundle.member_ids, ["s", "child"]);
        assert!(reject_active(&bundle.paths, SystemTime::now()).is_err());
        let revision = fingerprint(&bundle.paths).unwrap().hash;
        let entry = trash_bundle(trash.path(), session, bundle, &revision, idle_now()).unwrap();
        assert!(project.join("checkpoints/named.json").exists());
        assert!(project.join("plans/shared.md").exists());
        assert!(project.join(".project_root").exists());
        assert!(!project.join("tool-outputs/session-child").exists());
        assert!(!children.exists());
        restore_with_roots(trash.path(), &entry.key, Some(&roots)).unwrap();
        assert!(project.join("child/tracker/task.json").exists());
        assert!(main.exists());
        let unsafe_meta = meta("gemini", "Chats", &main);
        assert!(plan_in(&unsafe_meta, roots).is_err());
    }
}
