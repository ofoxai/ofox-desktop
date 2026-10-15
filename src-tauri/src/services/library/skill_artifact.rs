//! Immutable, bounded skill artifacts. Importing never projects into agent roots.
use crate::{config::get_app_config_dir, error::AppError};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const PACKAGE_MAX_BYTES: u64 = 512 * 1024 * 1024;
pub const PACKAGE_MAX_ENTRIES: usize = 100_000;
pub const PACKAGE_MAX_OBJECTS: usize = 1000;
const MAX_ENTRIES: usize = 10_000;
const MAX_DEPTH: usize = 32;
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}

pub fn safe_relative(raw: &str) -> Result<PathBuf, AppError> {
    safe_relative_limits(raw, 512, MAX_DEPTH)
}
fn safe_relative_limits(
    raw: &str,
    max_length: usize,
    max_depth: usize,
) -> Result<PathBuf, AppError> {
    if raw.len() > max_length
        || raw.is_empty()
        || raw.contains(['\\', ':', '\0'])
        || raw.starts_with('/')
    {
        return Err(error("invalidSkillPath"));
    }
    let path = PathBuf::from(raw);
    let mut depth = 0;
    for part in path.components() {
        let Component::Normal(name) = part else {
            return Err(error("invalidSkillPath"));
        };
        let name = name.to_str().ok_or_else(|| error("invalidSkillPath"))?;
        let stem = name
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if name.len() > 128
            || name.ends_with(['.', ' '])
            || name
                .chars()
                .any(|c| c.is_control() || "<>\"|?*".contains(c))
            || [
                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9",
            ]
            .contains(&stem.as_str())
        {
            return Err(error("invalidSkillPath"));
        }
        depth += 1;
    }
    if depth == 0
        || depth > max_depth
        || raw
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(error("invalidSkillPath"));
    }
    Ok(path)
}
pub fn safe_leaf(raw: &str) -> Result<(), AppError> {
    if safe_relative(raw)?.components().count() != 1 || raw.starts_with('.') {
        return Err(error("invalidSkillPath"));
    }
    Ok(())
}
pub(crate) struct Entry {
    pub path: String,
    pub bytes: Option<Vec<u8>>,
    pub executable: bool,
}
struct Tree {
    entries: Vec<Entry>,
    hash: String,
}
pub type ExecutionMap = BTreeMap<String, bool>;
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    execution: ExecutionMap,
}
fn actual_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}
fn normalized_mode(path: &Path, executable: bool, directory: bool) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        crate::config::set_file_mode(
            path,
            if executable || directory {
                0o755
            } else {
                0o644
            },
        )
        .map_err(|_| error("skillWriteFailed"))?;
    }
    #[cfg(windows)]
    {
        let mut permissions = fs::metadata(path)
            .map_err(|_| error("skillWriteFailed"))?
            .permissions();
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions).map_err(|_| error("skillWriteFailed"))?;
        let _ = (executable, directory);
    }
    Ok(())
}
fn mode_matches(metadata: &fs::Metadata, executable: bool, directory: bool) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7777
            == if executable || directory {
                0o755
            } else {
                0o644
            }
    }
    #[cfg(not(unix))]
    {
        let _ = (executable, directory);
        !metadata.permissions().readonly()
    }
}
fn read_tree(root: &Path) -> Result<Tree, AppError> {
    let execution = known_execution(root)?;
    read_tree_flags(root, execution.as_ref(), execution.is_some())
}
fn read_tree_flags(
    root: &Path,
    flags: Option<&ExecutionMap>,
    enforce_mode: bool,
) -> Result<Tree, AppError> {
    if fs::symlink_metadata(root)
        .map_err(|_| error("skillUnreadable"))?
        .file_type()
        .is_symlink()
    {
        return Err(error("skillLinkUnsupported"));
    }
    let mut entries = Vec::new();
    let mut total = 0u64;
    let mut names = BTreeMap::new();
    fn walk(
        root: &Path,
        current: &Path,
        entries: &mut Vec<Entry>,
        total: &mut u64,
        names: &mut BTreeMap<String, String>,
        flags: Option<&ExecutionMap>,
        enforce_mode: bool,
    ) -> Result<(), AppError> {
        for entry in fs::read_dir(current).map_err(|_| error("skillUnreadable"))? {
            let entry = entry.map_err(|_| error("skillUnreadable"))?;
            let path = entry.path();
            let components = path
                .strip_prefix(root)
                .map_err(|_| error("invalidSkillPath"))?;
            let relative = components
                .components()
                .map(|component| {
                    let value = component
                        .as_os_str()
                        .to_str()
                        .ok_or_else(|| error("invalidSkillPath"))?;
                    safe_relative(value)?;
                    Ok(value)
                })
                .collect::<Result<Vec<_>, AppError>>()?
                .join("/");
            safe_relative(&relative)?;
            if names
                .insert(relative.to_lowercase(), relative.clone())
                .is_some()
            {
                return Err(error("skillPathCollision"));
            }
            if entries.len() >= MAX_ENTRIES {
                return Err(error("skillTooLarge"));
            }
            let metadata = fs::symlink_metadata(&path).map_err(|_| error("skillUnreadable"))?;
            if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
                return Err(error("skillLinkUnsupported"));
            }
            let bytes = if metadata.is_file() {
                if metadata.len() > MAX_BYTES.saturating_sub(*total) {
                    return Err(error("skillTooLarge"));
                }
                let mut bytes = Vec::new();
                fs::File::open(&path)
                    .map_err(|_| error("skillUnreadable"))?
                    .take(MAX_BYTES.saturating_sub(*total) + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| error("skillUnreadable"))?;
                *total += bytes.len() as u64;
                if *total > MAX_BYTES {
                    return Err(error("skillTooLarge"));
                }
                Some(bytes)
            } else {
                None
            };
            let executable = if metadata.is_file() {
                match flags {
                    Some(flags) => *flags
                        .get(&relative)
                        .ok_or_else(|| error("skillManifestInvalid"))?,
                    None => actual_executable(&metadata),
                }
            } else {
                false
            };
            if enforce_mode && !mode_matches(&metadata, executable, metadata.is_dir()) {
                return Err(error("skillPermissionsChanged"));
            }
            entries.push(Entry {
                path: relative,
                bytes,
                executable,
            });
            if metadata.is_dir() {
                walk(root, &path, entries, total, names, flags, enforce_mode)?;
            }
        }
        Ok(())
    }
    if enforce_mode
        && !mode_matches(
            &fs::metadata(root).map_err(|_| error("skillUnreadable"))?,
            false,
            true,
        )
    {
        return Err(error("skillPermissionsChanged"));
    }
    walk(
        root,
        root,
        &mut entries,
        &mut total,
        &mut names,
        flags,
        enforce_mode,
    )?;
    if flags.is_some_and(|flags| {
        flags.len() != entries.iter().filter(|entry| entry.bytes.is_some()).count()
    }) {
        return Err(error("skillManifestInvalid"));
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let document = entries
        .iter()
        .find(|entry| entry.path == "SKILL.md")
        .and_then(|entry| entry.bytes.as_deref())
        .ok_or_else(|| error("skillDocumentMissing"))?;
    if document.len() > 1024 * 1024 || std::str::from_utf8(document).is_err() {
        return Err(error("skillDocumentInvalid"));
    }
    let mut hash = Sha256::new();
    for entry in &entries {
        hash.update((entry.path.len() as u64).to_le_bytes());
        hash.update(entry.path.as_bytes());
        match &entry.bytes {
            Some(bytes) => {
                hash.update([1, u8::from(entry.executable)]);
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
            None => hash.update([0]),
        }
    }
    Ok(Tree {
        entries,
        hash: format!("{:x}", hash.finalize()),
    })
}
pub fn tree_hash(path: &Path) -> Result<String, AppError> {
    Ok(read_tree(path)?.hash)
}
pub fn source_hash(path: &Path, execution: &ExecutionMap) -> Result<String, AppError> {
    Ok(read_tree_flags(path, Some(execution), false)?.hash)
}
fn write_tree(tree: &Tree, dest: &Path) -> Result<(), AppError> {
    fs::create_dir_all(dest).map_err(|_| error("skillWriteFailed"))?;
    normalized_mode(dest, false, true)?;
    for entry in &tree.entries {
        let path = dest.join(&entry.path);
        if let Some(bytes) = &entry.bytes {
            fs::create_dir_all(path.parent().ok_or_else(|| error("invalidSkillPath"))?)
                .map_err(|_| error("skillWriteFailed"))?;
            crate::config::atomic_write(&path, bytes).map_err(|_| error("skillWriteFailed"))?;
            normalized_mode(&path, entry.executable, false)?;
        } else {
            fs::create_dir_all(&path).map_err(|_| error("skillWriteFailed"))?;
            normalized_mode(&path, false, true)?;
        }
    }
    Ok(())
}
pub fn copy_verified(source: &Path, dest: &Path) -> Result<String, AppError> {
    let tree = read_tree(source)?;
    write_tree(&tree, dest)?;
    if read_tree_flags(dest, Some(&execution(&tree)), true)?.hash != tree.hash {
        return Err(error("skillChanged"));
    }
    Ok(tree.hash)
}
pub fn root() -> PathBuf {
    get_app_config_dir().join("library").join("skill-artifacts")
}
fn valid_hash(hash: &str) -> Result<(), AppError> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(error("stateUnreadable"));
    }
    Ok(())
}
pub fn path(hash: &str) -> Result<PathBuf, AppError> {
    valid_hash(hash)?;
    Ok(root().join(hash).join("tree"))
}
fn execution(tree: &Tree) -> ExecutionMap {
    tree.entries
        .iter()
        .filter(|entry| entry.bytes.is_some())
        .map(|entry| (entry.path.clone(), entry.executable))
        .collect()
}
pub fn validate_root() -> Result<(), AppError> {
    validate_private_location(&root())
}
pub fn validate_private_location(path: &Path) -> Result<(), AppError> {
    let physical = super::coordinator::physical(path)?;
    for roots in super::skill_resource::reader_paths()?.values() {
        for native in roots {
            let native = super::coordinator::physical(native)?;
            if physical.starts_with(&native) || native.starts_with(&physical) {
                return Err(error("skillArtifactScopeConflict"));
            }
        }
    }
    Ok(())
}
pub fn manifest(hash: &str) -> Result<ExecutionMap, AppError> {
    valid_hash(hash)?;
    validate_root()?;
    let container = root().join(hash);
    if fs::symlink_metadata(&container)
        .map_err(|_| error("skillArtifactMissing"))?
        .file_type()
        .is_symlink()
    {
        return Err(error("skillArtifactChanged"));
    }
    let path = container.join("manifest.json");
    let metadata = fs::symlink_metadata(&path).map_err(|_| error("skillManifestInvalid"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 8 * 1024 * 1024
    {
        return Err(error("skillManifestInvalid"));
    }
    parse_manifest_bytes(&fs::read(path).map_err(|_| error("skillManifestInvalid"))?)
}
pub fn parse_manifest_bytes(raw: &[u8]) -> Result<ExecutionMap, AppError> {
    if raw.len() > 8 * 1024 * 1024 {
        return Err(error("skillManifestInvalid"));
    }
    let data: Manifest = serde_json::from_slice(raw).map_err(|_| error("skillManifestInvalid"))?;
    if data.schema_version != 1 || data.execution.len() > MAX_ENTRIES {
        return Err(error("skillManifestInvalid"));
    }
    for name in data.execution.keys() {
        safe_relative(name)?;
    }
    Ok(data.execution)
}
pub fn archive_entries(hash: &str) -> Result<(Vec<u8>, Vec<Entry>), AppError> {
    let tree = read_tree(&path(hash)?)?;
    if tree.hash != hash {
        return Err(error("skillArtifactChanged"));
    }
    let bytes = serde_json::to_vec(&Manifest {
        schema_version: 1,
        execution: execution(&tree),
    })
    .map_err(|_| error("skillManifestInvalid"))?;
    Ok((bytes, tree.entries))
}
pub fn verify_remote_artifact(
    hash: &str,
    tree: &Path,
    execution: &ExecutionMap,
) -> Result<(), AppError> {
    valid_hash(hash)?;
    if read_tree_flags(tree, Some(execution), false)?.hash != hash {
        return Err(error("skillArtifactChanged"));
    }
    Ok(())
}
fn known_execution(source: &Path) -> Result<Option<ExecutionMap>, AppError> {
    let roots = [root(), root().canonicalize().unwrap_or_else(|_| root())];
    for root in roots {
        if let Ok(relative) = source.strip_prefix(&root) {
            let parts: Vec<_> = relative.components().collect();
            if parts.len() == 2 && parts[1].as_os_str() == "tree" {
                let hash = parts[0]
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| error("stateUnreadable"))?;
                return Ok(Some(manifest(hash)?));
            }
        }
    }
    Ok(None)
}
pub fn hash_with_execution(source: &Path, execution: &ExecutionMap) -> Result<String, AppError> {
    Ok(read_tree_flags(source, Some(execution), true)?.hash)
}
pub fn publish(source: &Path) -> Result<String, AppError> {
    publish_with_execution(source, None)
}
pub(crate) fn source_entries_checked(
    source: &Path,
    flags: &ExecutionMap,
    expected: &str,
) -> Result<Vec<Entry>, AppError> {
    let tree = read_tree_flags(source, Some(flags), false)?;
    if tree.hash != expected {
        return Err(error("skillArtifactChanged"));
    }
    Ok(tree.entries)
}
pub(crate) fn publish_checked(
    source: &Path,
    flags: Option<&ExecutionMap>,
    expected: &str,
) -> Result<String, AppError> {
    publish_inner(source, flags, Some(expected))
}
pub fn publish_with_execution(
    source: &Path,
    flags: Option<&ExecutionMap>,
) -> Result<String, AppError> {
    publish_inner(source, flags, None)
}
fn publish_inner(
    source: &Path,
    flags: Option<&ExecutionMap>,
    expected: Option<&str>,
) -> Result<String, AppError> {
    let tree = if let Some(flags) = flags {
        read_tree_flags(source, Some(flags), false)?
    } else {
        read_tree(source)?
    };
    if expected.is_some_and(|expected| tree.hash != expected) {
        return Err(error("skillArtifactChanged"));
    }
    let _io = crate::config_io::lock();
    validate_root()?;
    let root = root();
    crate::config::create_private_dir_all(&root).map_err(|_| error("skillWriteFailed"))?;
    let dest = root.join(&tree.hash);
    if fs::symlink_metadata(&dest).is_ok() {
        if tree_hash(&path(&tree.hash)?)? != tree.hash {
            return Err(error("skillArtifactChanged"));
        }
        return Ok(tree.hash);
    }
    let staged = tempfile::Builder::new()
        .prefix(".import-")
        .tempdir_in(&root)
        .map_err(|_| error("skillWriteFailed"))?;
    let package = staged.path().join("artifact");
    crate::config::create_private_dir_all(&package).map_err(|_| error("skillWriteFailed"))?;
    let staged_tree = package.join("tree");
    write_tree(&tree, &staged_tree)?;
    let execution = execution(&tree);
    if read_tree_flags(&staged_tree, Some(&execution), true)?.hash != tree.hash {
        return Err(error("skillChanged"));
    }
    let raw = serde_json::to_vec(&Manifest {
        schema_version: 1,
        execution,
    })
    .map_err(|_| error("skillManifestInvalid"))?;
    crate::config::atomic_write(&package.join("manifest.json"), &raw)
        .map_err(|_| error("skillWriteFailed"))?;
    fs::rename(package, dest).map_err(|_| error("skillWriteFailed"))?;
    Ok(tree.hash)
}
pub struct ExtractedDir {
    tmp: tempfile::TempDir,
    execution: ExecutionMap,
}
impl ExtractedDir {
    pub fn path(&self) -> &Path {
        self.tmp.path()
    }
    pub fn execution_for(&self, source: &Path) -> Result<ExecutionMap, AppError> {
        let prefix = source
            .strip_prefix(self.path())
            .map_err(|_| error("invalidSkillPath"))?;
        Ok(self
            .execution
            .iter()
            .filter_map(|(name, executable)| {
                Path::new(name)
                    .strip_prefix(prefix)
                    .ok()
                    .and_then(|path| path.to_str())
                    .map(|path| (path.replace('\\', "/"), *executable))
            })
            .collect())
    }
}

/// Reject the whole archive on traversal, links, collisions or size limits.
pub fn extract_zip(raw: &[u8], strip_root: bool) -> Result<ExtractedDir, AppError> {
    extract_zip_limits(raw, strip_root, MAX_BYTES, MAX_ENTRIES, 512, MAX_DEPTH)
}
pub fn extract_artifact_package(raw: &[u8]) -> Result<ExtractedDir, AppError> {
    extract_zip_limits(
        raw,
        false,
        PACKAGE_MAX_BYTES,
        PACKAGE_MAX_ENTRIES,
        768,
        MAX_DEPTH + 3,
    )
}
fn extract_zip_limits(
    raw: &[u8],
    strip_root: bool,
    max_bytes: u64,
    max_entries: usize,
    max_length: usize,
    max_depth: usize,
) -> Result<ExtractedDir, AppError> {
    if raw.len() as u64 > max_bytes {
        return Err(error("skillTooLarge"));
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(raw))
        .map_err(|_| error("skillArchiveInvalid"))?;
    if archive.len() > max_entries {
        return Err(error("skillTooLarge"));
    }
    let tmp = tempfile::tempdir().map_err(|_| error("skillWriteFailed"))?;
    let mut seen = BTreeMap::new();
    let mut explicit = std::collections::BTreeSet::new();
    let mut total = 0u64;
    let mut root = None;
    let mut execution = ExecutionMap::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| error("skillArchiveInvalid"))?;
        let original = std::str::from_utf8(entry.name_raw())
            .map_err(|_| error("invalidSkillPath"))?
            .trim_end_matches('/');
        let original_path = safe_relative_limits(original, max_length, max_depth)?;
        if entry.is_symlink()
            || entry
                .unix_mode()
                .is_some_and(|mode| ![0, 0o100000, 0o040000].contains(&(mode & 0o170000)))
        {
            return Err(error("skillLinkUnsupported"));
        }
        let relative = if strip_root {
            let first = original_path
                .components()
                .next()
                .ok_or_else(|| error("invalidSkillPath"))?
                .as_os_str()
                .to_owned();
            if root.as_ref().is_some_and(|root| root != &first) {
                return Err(error("skillArchiveInvalid"));
            }
            root = Some(first);
            original_path.components().skip(1).collect::<PathBuf>()
        } else {
            original_path
        };
        if relative.as_os_str().is_empty() {
            if entry.is_dir() {
                continue;
            }
            return Err(error("skillArchiveInvalid"));
        }
        let name = relative
            .to_str()
            .ok_or_else(|| error("invalidSkillPath"))?
            .replace('\\', "/");
        if !explicit.insert(name.to_lowercase()) {
            return Err(error("skillPathCollision"));
        }
        let mut prefix = PathBuf::new();
        for part in relative.components() {
            prefix.push(part.as_os_str());
            let text = prefix
                .to_str()
                .ok_or_else(|| error("invalidSkillPath"))?
                .replace('\\', "/");
            if seen
                .get(&text.to_lowercase())
                .is_some_and(|previous| previous != &text)
            {
                return Err(error("skillPathCollision"));
            }
            seen.insert(text.to_lowercase(), text);
        }
        let dest = tmp.path().join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(dest).map_err(|_| error("skillArchiveInvalid"))?;
            continue;
        }
        if entry.size() > max_bytes.saturating_sub(total) {
            return Err(error("skillTooLarge"));
        }
        let mut bytes = Vec::new();
        (&mut entry)
            .take(max_bytes.saturating_sub(total) + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error("skillArchiveInvalid"))?;
        total += bytes.len() as u64;
        if total > max_bytes {
            return Err(error("skillTooLarge"));
        }
        fs::create_dir_all(dest.parent().ok_or_else(|| error("invalidSkillPath"))?)
            .map_err(|_| error("skillArchiveInvalid"))?;
        crate::config::atomic_write(&dest, &bytes).map_err(|_| error("skillArchiveInvalid"))?;
        let executable = entry.unix_mode().is_some_and(|mode| mode & 0o111 != 0);
        execution.insert(name, executable);
        normalized_mode(&dest, executable, false)?;
    }
    Ok(ExtractedDir { tmp, execution })
}
