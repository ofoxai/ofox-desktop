use super::Failure;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 50_000;
const PACKAGE: &str = "lib/node_modules/@github/copilot";
const NATIVE: &str = "lib/node_modules/@github/copilot/node_modules/@github/copilot-darwin-arm64";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PackageIdentity {
    pub prefix: PathBuf,
    pub entry: PathBuf,
    pub version: String,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    kind: u8,
    size: u64,
    modified: i128,
    mode: u32,
    device: u64,
    inode: u64,
    changed: i64,
    changed_ns: i64,
}
fn stamp(meta: &fs::Metadata) -> Result<Stamp, Failure> {
    let kind = if meta.is_file() {
        1
    } else if meta.is_dir() {
        0
    } else if meta.file_type().is_symlink() {
        2
    } else {
        return Err(Failure::InvalidPackage);
    };
    let modified = match meta
        .modified()
        .map_err(|_| Failure::Unreadable)?
        .duration_since(UNIX_EPOCH)
    {
        Ok(time) => time.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    #[cfg(unix)]
    let (mode, device, inode, changed, changed_ns) = {
        use std::os::unix::fs::MetadataExt;
        (
            meta.mode(),
            meta.dev(),
            meta.ino(),
            meta.ctime(),
            meta.ctime_nsec(),
        )
    };
    #[cfg(not(unix))]
    let (mode, device, inode, changed, changed_ns) =
        (u32::from(meta.permissions().readonly()), 0, 0, 0, 0);
    Ok(Stamp {
        kind,
        size: meta.len(),
        modified,
        mode,
        device,
        inode,
        changed,
        changed_ns,
    })
}
fn hash_stamp(digest: &mut Sha256, value: &Stamp) {
    digest.update([value.kind]);
    digest.update(value.size.to_le_bytes());
    digest.update(value.modified.to_le_bytes());
    digest.update(value.mode.to_le_bytes());
    digest.update(value.device.to_le_bytes());
    digest.update(value.inode.to_le_bytes());
    digest.update(value.changed.to_le_bytes());
    digest.update(value.changed_ns.to_le_bytes());
}
#[derive(Default)]
struct Snapshot {
    digest: Sha256,
    entries: usize,
    bytes: u64,
    stamps: BTreeMap<PathBuf, Stamp>,
    manifests: BTreeMap<PathBuf, Vec<u8>>,
}
impl Snapshot {
    fn manifest(&self, directory: &str) -> Result<Value, Failure> {
        let bytes = self
            .manifests
            .get(&Path::new(directory).join("package.json"))
            .ok_or(Failure::InvalidPackage)?;
        serde_json::from_slice(bytes).map_err(|_| Failure::InvalidPackage)
    }
    fn verify(&self) -> Result<(), Failure> {
        for (path, expected) in &self.stamps {
            if stamp(&fs::symlink_metadata(path).map_err(|_| Failure::Changed)?)? != *expected {
                return Err(Failure::Changed);
            }
        }
        Ok(())
    }
}

pub(super) fn read_regular(path: &Path, max: u64) -> Result<Vec<u8>, Failure> {
    let before = stamp(&fs::symlink_metadata(path).map_err(|_| Failure::Unreadable)?)?;
    if before.kind != 1 || before.size > max {
        return Err(Failure::InvalidPackage);
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path).map_err(|_| Failure::Changed)?;
    if stamp(&file.metadata().map_err(|_| Failure::Changed)?)? != before {
        return Err(Failure::Changed);
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Unreadable)?;
    if bytes.len() as u64 != before.size
        || stamp(&file.metadata().map_err(|_| Failure::Changed)?)? != before
        || stamp(&fs::symlink_metadata(path).map_err(|_| Failure::Changed)?)? != before
    {
        return Err(Failure::Changed);
    }
    Ok(bytes)
}

pub(super) fn file_identity(path: &Path) -> Result<String, Failure> {
    let before = stamp(&fs::symlink_metadata(path).map_err(|_| Failure::Unreadable)?)?;
    let bytes = read_regular(path, 256 * 1024 * 1024)?;
    if stamp(&fs::symlink_metadata(path).map_err(|_| Failure::Changed)?)? != before {
        return Err(Failure::Changed);
    }
    let mut hash = Sha256::new();
    hash_stamp(&mut hash, &before);
    hash.update(bytes);
    Ok(format!("{:x}", hash.finalize()))
}
pub(super) fn empty_identity(root: &Path) -> Result<String, Failure> {
    let snapshot = snapshot_tree(root)?;
    if snapshot.stamps.values().any(|entry| entry.kind != 0) {
        return Err(Failure::InvalidPackage);
    }
    snapshot.verify()?;
    Ok(format!("{:x}", snapshot.digest.finalize()))
}

fn snapshot_tree(root: &Path) -> Result<Snapshot, Failure> {
    let meta = fs::symlink_metadata(root).map_err(|_| Failure::Unreadable)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(Failure::InvalidPackage);
    }
    let mut snapshot = Snapshot::default();
    let root_stamp = stamp(&meta)?;
    hash_stamp(&mut snapshot.digest, &root_stamp);
    snapshot.stamps.insert(root.to_path_buf(), root_stamp);
    hash_directory(root, root, 0, &mut snapshot)?;
    Ok(snapshot)
}
pub(super) fn tree_identity(root: &Path) -> Result<(String, Option<Value>), Failure> {
    let snapshot = snapshot_tree(root)?;
    let manifest = snapshot
        .manifests
        .get(Path::new("package.json"))
        .map(|bytes| serde_json::from_slice(bytes).map_err(|_| Failure::InvalidPackage))
        .transpose()?;
    snapshot.verify()?;
    Ok((format!("{:x}", snapshot.digest.finalize()), manifest))
}

pub(super) fn inspect(prefix: &Path) -> Result<PackageIdentity, Failure> {
    inspect_with(prefix, || {})
}
fn inspect_with(prefix: &Path, after_snapshot: impl FnOnce()) -> Result<PackageIdentity, Failure> {
    let meta = fs::symlink_metadata(prefix).map_err(|_| Failure::Unreadable)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(Failure::InvalidPackage);
    }
    let prefix = fs::canonicalize(prefix).map_err(|_| Failure::Unreadable)?;
    let mut snapshot = Snapshot::default();
    let root_stamp = stamp(&meta)?;
    hash_stamp(&mut snapshot.digest, &root_stamp);
    snapshot.stamps.insert(prefix.clone(), root_stamp);
    hash_directory(&prefix, &prefix, 0, &mut snapshot)?;
    // Parse exactly the bytes that contributed to this snapshot's fingerprint.
    let manifest = snapshot.manifest(PACKAGE)?;
    let version = manifest["version"]
        .as_str()
        .ok_or(Failure::InvalidPackage)?;
    if manifest["name"] != "@github/copilot"
        || semver::Version::parse(version).is_err()
        || manifest["bin"]["copilot"] != "npm-loader.js"
        || manifest
            .get("scripts")
            .is_some_and(|value| !value.is_object() || !value.as_object().unwrap().is_empty())
    {
        return Err(Failure::InvalidPackage);
    }
    let native = snapshot.manifest(NATIVE)?;
    if native["name"] != "@github/copilot-darwin-arm64"
        || native["version"] != version
        || native["bin"]["copilot-darwin-arm64"] != "copilot"
        || !prefix.join(NATIVE).join("copilot").is_file()
    {
        return Err(Failure::InvalidPackage);
    }
    let entry = prefix.join(PACKAGE).join("npm-loader.js");
    let bin = prefix.join("bin/copilot");
    if snapshot
        .stamps
        .get(&bin)
        .is_none_or(|value| value.kind != 2)
        || fs::canonicalize(&bin).ok().as_ref() != Some(&entry)
        || !entry.is_file()
    {
        return Err(Failure::InvalidPackage);
    }
    after_snapshot();
    snapshot.verify()?;
    Ok(PackageIdentity {
        prefix,
        entry,
        version: version.into(),
        hash: format!("{:x}", snapshot.digest.finalize()),
    })
}
fn collect_children(
    iter: impl Iterator<Item = Result<PathBuf, Failure>>,
    remaining: usize,
) -> Result<Vec<PathBuf>, Failure> {
    let mut paths = Vec::new();
    for entry in iter.take(remaining.saturating_add(1)) {
        if paths.len() == remaining {
            return Err(Failure::Limit);
        }
        paths.push(entry?);
    }
    Ok(paths)
}
fn hash_directory(
    root: &Path,
    dir: &Path,
    depth: usize,
    snapshot: &mut Snapshot,
) -> Result<(), Failure> {
    if depth > 32 {
        return Err(Failure::Limit);
    }
    let iter = fs::read_dir(dir)
        .map_err(|_| Failure::Unreadable)?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|_| Failure::Unreadable)
        });
    let mut children = collect_children(iter, MAX_ENTRIES.saturating_sub(snapshot.entries))?;
    // Reserve siblings before descending, so all pending vectors share one budget.
    snapshot.entries += children.len();
    children.sort();
    for path in children {
        let relative = path.strip_prefix(root).map_err(|_| Failure::Changed)?;
        let name = relative
            .to_str()
            .filter(|name| name.len() <= 4096)
            .ok_or(Failure::InvalidPackage)?;
        snapshot.digest.update((name.len() as u64).to_le_bytes());
        snapshot.digest.update(name.as_bytes());
        let before = stamp(&fs::symlink_metadata(&path).map_err(|_| Failure::Unreadable)?)?;
        hash_stamp(&mut snapshot.digest, &before);
        snapshot.stamps.insert(path.clone(), before.clone());
        match before.kind {
            0 => hash_directory(root, &path, depth + 1, snapshot)?,
            2 => {
                let target = fs::canonicalize(&path).map_err(|_| Failure::InvalidPackage)?;
                let target_name = target
                    .strip_prefix(root)
                    .map_err(|_| Failure::InvalidPackage)?
                    .to_str()
                    .ok_or(Failure::InvalidPackage)?;
                if !target.is_file() {
                    return Err(Failure::InvalidPackage);
                }
                snapshot
                    .digest
                    .update((target_name.len() as u64).to_le_bytes());
                snapshot.digest.update(target_name.as_bytes());
            }
            1 => {
                let capture = relative == Path::new("package.json")
                    || relative == Path::new(PACKAGE).join("package.json")
                    || relative == Path::new(NATIVE).join("package.json");
                if before.size > MAX_BYTES.saturating_sub(snapshot.bytes)
                    || capture && before.size > 1024 * 1024
                {
                    return Err(Failure::Limit);
                }
                let mut options = fs::OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
                }
                let mut file = options.open(&path).map_err(|_| Failure::Changed)?;
                if stamp(&file.metadata().map_err(|_| Failure::Changed)?)? != before {
                    return Err(Failure::Changed);
                }
                let mut count = 0u64;
                let mut buffer = [0u8; 65536];
                let mut body = Vec::new();
                loop {
                    let read = file.read(&mut buffer).map_err(|_| Failure::Unreadable)?;
                    if read == 0 {
                        break;
                    }
                    count += read as u64;
                    if count > MAX_BYTES.saturating_sub(snapshot.bytes)
                        || capture && count > 1024 * 1024
                    {
                        return Err(Failure::Limit);
                    }
                    snapshot.digest.update(&buffer[..read]);
                    if capture {
                        body.extend_from_slice(&buffer[..read]);
                    }
                }
                if count != before.size
                    || stamp(&file.metadata().map_err(|_| Failure::Changed)?)? != before
                {
                    return Err(Failure::Changed);
                }
                snapshot.bytes += count;
                if capture {
                    snapshot.manifests.insert(relative.to_owned(), body);
                }
            }
            _ => return Err(Failure::InvalidPackage),
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
pub(super) mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, os::unix::fs::symlink};

    pub(crate) fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("lib/node_modules/@github/copilot");
        fs::create_dir_all(&package).unwrap();
        fs::create_dir(dir.path().join("bin")).unwrap();
        fs::write(
            package.join("package.json"),
            json!({"name":"@github/copilot","version":"1.0.98","bin":{"copilot":"npm-loader.js"}})
                .to_string(),
        )
        .unwrap();
        fs::write(
            package.join("npm-loader.js"),
            "// local fixture, never executed\n",
        )
        .unwrap();
        let native = package.join("node_modules/@github/copilot-darwin-arm64");
        fs::create_dir_all(&native).unwrap();
        fs::write(native.join("package.json"), json!({"name":"@github/copilot-darwin-arm64","version":"1.0.98","bin":{"copilot-darwin-arm64":"copilot"}}).to_string()).unwrap();
        fs::write(native.join("copilot"), "fixture native payload").unwrap();
        symlink(
            "../lib/node_modules/@github/copilot/npm-loader.js",
            dir.path().join("bin/copilot"),
        )
        .unwrap();
        dir
    }

    #[test]
    fn inspects_the_designated_copy_without_a_runtime_in_its_prefix() {
        let dir = fixture();
        let found = inspect(dir.path()).unwrap();
        assert_eq!(found.version, "1.0.98");
        assert_eq!(found.prefix, fs::canonicalize(dir.path()).unwrap());
        assert_eq!(
            found.entry,
            found
                .prefix
                .join("lib/node_modules/@github/copilot/npm-loader.js")
        );
        assert_eq!(found.hash.len(), 64);
        assert!(!dir.path().join("bin/node").exists());
        assert!(!dir.path().join("bin/npm").exists());
    }

    #[test]
    fn the_snapshot_covers_native_payloads_unrelated_files_and_link_identity() {
        let dir = fixture();
        let before = inspect(dir.path()).unwrap();
        fs::write(dir.path().join("lib/node_modules/@github/copilot/node_modules/@github/copilot-darwin-arm64/copilot"), "changed native payload").unwrap();
        assert_ne!(inspect(dir.path()).unwrap().hash, before.hash);
        let before = inspect(dir.path()).unwrap();
        fs::write(
            dir.path().join("personal-note.txt"),
            "preserve unknown edits",
        )
        .unwrap();
        assert_ne!(inspect(dir.path()).unwrap().hash, before.hash);
    }

    #[test]
    fn replacing_a_prefix_with_identical_bytes_invalidates_the_physical_proof() {
        let old = fixture();
        let replacement = fixture();
        let held = tempfile::tempdir().unwrap();
        let before = inspect(old.path()).unwrap();
        fs::rename(old.path(), held.path().join("previous-prefix")).unwrap();
        fs::rename(replacement.path(), old.path()).unwrap();
        let after = inspect(old.path()).unwrap();
        assert_eq!(after.prefix, before.prefix);
        assert_ne!(after.hash, before.hash);
    }

    #[test]
    fn enumeration_stops_at_the_remaining_budget_before_allocating_a_whole_directory() {
        let count = std::cell::Cell::new(0);
        let iter = (0..10).map(|index| {
            count.set(count.get() + 1);
            Ok(PathBuf::from(index.to_string()))
        });
        assert_eq!(collect_children(iter, 3), Err(Failure::Limit));
        assert_eq!(count.get(), 4);
    }

    #[test]
    fn manifest_semantics_and_fingerprint_cannot_come_from_different_revisions() {
        let dir = fixture();
        let manifest = dir
            .path()
            .join("lib/node_modules/@github/copilot/package.json");
        let result = inspect_with(dir.path(), || {
            fs::write(
                &manifest,
                json!({"name":"foreign-copy","version":"1.0.98","bin":{"copilot":"npm-loader.js"}})
                    .to_string(),
            )
            .unwrap();
        });
        assert!(
            result.is_err(),
            "accepted the old identity with a new tree fingerprint"
        );
    }

    #[test]
    fn a_same_length_edit_with_restored_mtime_invalidates_the_snapshot() {
        use std::io::{Seek, Write};
        let dir = fixture();
        let loader = dir
            .path()
            .join("lib/node_modules/@github/copilot/npm-loader.js");
        let metadata = fs::metadata(&loader).unwrap();
        let before = stamp(&metadata).unwrap();
        let result = inspect_with(dir.path(), || {
            let mut file = fs::OpenOptions::new().write(true).open(&loader).unwrap();
            // Linux can reuse one ctime tick for rapid writes. Establish the
            // ctime-guard precondition without changing size or original mtime.
            let mut changed = false;
            for _ in 0..100 {
                file.rewind().unwrap();
                file.write_all(&vec![b'x'; metadata.len() as usize])
                    .unwrap();
                file.set_modified(metadata.modified().unwrap()).unwrap();
                let after = stamp(&fs::metadata(&loader).unwrap()).unwrap();
                assert_eq!(after.size, before.size);
                assert_eq!(after.modified, before.modified);
                if (after.changed, after.changed_ns) != (before.changed, before.changed_ns) {
                    changed = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(changed, "fixture ctime did not advance within one second");
        });
        assert_eq!(result, Err(Failure::Changed));
    }

    #[test]
    fn both_manifests_and_external_manifest_links_refuse_fifos_without_waiting_for_a_writer() {
        for relative in [
            "package.json",
            "node_modules/@github/copilot-darwin-arm64/package.json",
            "external",
        ] {
            let dir = fixture();
            let package = dir.path().join("lib/node_modules/@github/copilot");
            let outside = tempfile::tempdir().unwrap();
            let fifo = if relative == "external" {
                outside.path().join("manifest")
            } else {
                package.join(relative)
            };
            if fifo.exists() {
                fs::remove_file(&fifo).unwrap();
            }
            use std::os::unix::ffi::OsStrExt;
            let raw = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
            if relative == "external" {
                fs::remove_file(package.join("package.json")).unwrap();
                symlink(&fifo, package.join("package.json")).unwrap();
            }
            let root = dir.path().to_owned();
            let (sender, receiver) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || {
                let _ = sender.send(inspect(&root));
            });
            let received = receiver.recv_timeout(std::time::Duration::from_secs(1));
            // Release the previous implementation's blocked reader before asserting RED.
            if received.is_err() {
                fs::write(&fifo, "{}").unwrap();
            }
            thread.join().unwrap();
            assert!(received.is_ok(), "manifest {relative} blocked on a FIFO");
            assert!(received.unwrap().is_err());
        }
    }

    #[test]
    fn rejects_same_name_foreign_packages_and_mismatched_platform_versions() {
        let dir = fixture();
        let manifest = dir
            .path()
            .join("lib/node_modules/@github/copilot/package.json");
        fs::write(
            &manifest,
            json!({"name":"copilot","version":"1.0.98","bin":{"copilot":"npm-loader.js"}})
                .to_string(),
        )
        .unwrap();
        assert_eq!(inspect(dir.path()), Err(Failure::InvalidPackage));
        let dir = fixture();
        let manifest = dir.path().join("lib/node_modules/@github/copilot/node_modules/@github/copilot-darwin-arm64/package.json");
        fs::write(&manifest,json!({"name":"@github/copilot-darwin-arm64","version":"0.0.1","bin":{"copilot-darwin-arm64":"copilot"}}).to_string()).unwrap();
        assert_eq!(inspect(dir.path()), Err(Failure::InvalidPackage));
    }

    #[test]
    fn rejects_foreign_bin_links_and_any_link_outside_the_owned_prefix() {
        let dir = fixture();
        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join("copilot.js"), "foreign program").unwrap();
        fs::remove_file(dir.path().join("bin/copilot")).unwrap();
        symlink(
            other.path().join("copilot.js"),
            dir.path().join("bin/copilot"),
        )
        .unwrap();
        assert!(inspect(dir.path()).is_err());
        let dir = fixture();
        symlink(other.path(), dir.path().join("extra")).unwrap();
        assert!(inspect(dir.path()).is_err());
        let alias = other.path().join("prefix");
        symlink(dir.path(), &alias).unwrap();
        assert!(inspect(&alias).is_err());
    }
}
