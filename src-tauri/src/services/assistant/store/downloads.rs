//! Temporary downloads are separate from portable session attachments.
use super::*;

fn owned(name: &str) -> bool {
    let Some(body) = name
        .strip_prefix("video-")
        .and_then(|v| v.strip_suffix(".part"))
    else {
        return false;
    };
    let Some(uuid) = body.get(..36) else {
        return false;
    };
    let Some(random) = body.get(36..).and_then(|v| v.strip_prefix('-')) else {
        return false;
    };
    validate_id(uuid).is_ok()
        && random.len() >= 6
        && random.bytes().all(|v| v.is_ascii_alphanumeric())
}
impl AssistantStore {
    pub(crate) fn video_temp(&self) -> Result<tempfile::NamedTempFile, String> {
        let root = self.ensure_root()?;
        let dir = private_subdir(&root, ".downloads")?;
        self.cleanup_downloads()?;
        let prefix = format!("video-{}-", id());
        let file = tempfile::Builder::new()
            .prefix(&prefix)
            .suffix(".part")
            .tempfile_in(&dir)
            .map_err(|_| UNAVAILABLE)?;
        if !real_dir(&root)? || !real_dir(&dir)? {
            return Err(INVALID_PATH.into());
        }
        Ok(file)
    }
    /// Consumers keep an open file, never reopen this temporary pathname.
    /// Unix unlink leaves that handle usable; Windows busy files are retried
    /// after their owner drops them. No foreign name or directory is removed.
    pub(crate) fn cleanup_downloads(&self) -> Result<usize, String> {
        if !real_dir(&self.config_dir.join(ASSISTANT_DIR))? || !real_dir(&self.root)? {
            return Ok(0);
        }
        let dir = self.root.join(".downloads");
        if !real_dir(&dir)? {
            return Ok(0);
        }
        let mut count = 0;
        for entry in fs::read_dir(&dir).map_err(|_| UNAVAILABLE)? {
            let entry = entry.map_err(|_| UNAVAILABLE)?;
            if !entry.file_name().to_str().is_some_and(owned) {
                continue;
            }
            let kind = entry.file_type().map_err(|_| UNAVAILABLE)?;
            if !kind.is_file() && !kind.is_symlink() {
                continue;
            }
            match fs::remove_file(entry.path()) {
                Ok(()) => count += 1,
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                #[cfg(windows)]
                Err(e) if e.kind() == ErrorKind::PermissionDenied => {}
                Err(_) => return Err(UNAVAILABLE.into()),
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::io::{Read, Seek, SeekFrom, Write};
    fn store() -> (tempfile::TempDir, AssistantStore) {
        let root = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
        (root, store)
    }
    #[test]
    fn download_files_are_private_owned_and_removed_on_drop() {
        let (_root, store) = store();
        let mut file = store.video_temp().unwrap();
        let path = file.path().to_owned();
        file.write_all(b"private fixture").unwrap();
        assert!(path.starts_with(store.root.join(".downloads")));
        #[cfg(unix)]
        assert_eq!(crate::config::file_mode(&path), Some(0o600));
        drop(file);
        assert!(!path.exists());
    }
    #[test]
    fn download_cleanup_removes_owned_orphans_but_preserves_unrelated_files_and_open_handles() {
        let (_root, store) = store();
        let mut open = store.video_temp().unwrap();
        open.write_all(b"open fixture").unwrap();
        let parent = open.path().parent().unwrap().to_owned();
        let orphan = store.video_temp().unwrap().into_temp_path().keep().unwrap();
        let unrelated = parent.join("user-note.txt");
        fs::write(&unrelated, b"keep").unwrap();
        store.cleanup_downloads().unwrap();
        assert!(!orphan.exists());
        assert!(unrelated.exists());
        // Unix may unlink an active path, Windows may retain a busy one. Both
        // keep the owned file handle readable until publication/cancellation.
        open.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        open.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"open fixture");
        drop(open);
        store.cleanup_downloads().unwrap();
        assert_eq!(fs::read_dir(parent).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn download_cleanup_and_creation_refuse_linked_directories_without_touching_targets() {
        let (_root, store) = store();
        store.ensure_root().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let owned = outside.path().join(format!("video-{}.part", id()));
        fs::write(&owned, b"outside").unwrap();
        std::os::unix::fs::symlink(outside.path(), store.root.join(".downloads")).unwrap();
        assert!(store.cleanup_downloads().is_err());
        assert!(store.video_temp().is_err());
        assert_eq!(fs::read(owned).unwrap(), b"outside");
    }
}

#[cfg(test)]
mod backup_tests {
    use super::*;
    #[test]
    fn download_partials_never_enter_a_portable_backup() {
        let root = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
        store.create("test/model", false).unwrap();
        let _active = store.video_temp().unwrap();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("backup.zip");
        assert_eq!(store.backup(&archive, &[], || Ok(())).unwrap().sessions, 1);
        let zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
        assert!(!zip
            .file_names()
            .map(Result::unwrap)
            .any(|name| name.contains(".downloads") || name.ends_with(".part")));
    }
}
