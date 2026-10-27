//! Files the user imports into a conversation: which formats can be picked,
//! the limits, and the all-or-nothing copy into the session's private
//! attachments folder (see [`AssistantStore::attachment_dir`]).
use super::{
    store::{AssistantStore, SessionRecord},
    types::*,
};
use crate::config::FileTxn;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

/// The formats the runtime reads (assistant-runtime README, "Attachments"):
/// images, PDF and UTF-8 text. Office documents are not among them.
pub const EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "pdf", "txt", "md", "csv", "json", "log", "yaml", "yml",
    "toml", "xml", "html", "js", "ts", "py", "rs", "go",
];
/// The name of the dialog's file filter: format names, not a sentence.
pub const FILTER_NAME: &str = "Images, PDF, Text";

pub const MAX_PER_PICK: usize = 10;
pub const MAX_PER_SESSION: usize = 200;
pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_SESSION_BYTES: u64 = 500 * 1024 * 1024;

/// More than [`MAX_PER_PICK`] files at once, or more than [`MAX_PER_SESSION`]
/// in the conversation.
pub const TOO_MANY: &str = "assistant_attachment_too_many";
/// A file over [`MAX_FILE_BYTES`] (the runtime uses the same code).
pub const TOO_LARGE: &str = "assistant_attachment_too_large";
/// The conversation's files would exceed [`MAX_SESSION_BYTES`] together.
pub const TOTAL_TOO_LARGE: &str = "assistant_attachment_total_too_large";
/// Not one of [`EXTENSIONS`] (the runtime uses the same code).
pub const UNSUPPORTED: &str = "assistant_attachment_unsupported";
/// Missing, not a regular file, or could not be read.
pub const UNREADABLE: &str = "assistant_attachment_unreadable";
/// The copy into the conversation's folder failed.
pub const WRITE_FAILED: &str = "assistant_attachment_write_failed";
/// The file dialog itself failed.
pub const DIALOG_FAILED: &str = "assistant_attachment_dialog_failed";

/// The MIME type of a file with `extension` (lowercase), if it can be imported.
pub fn mime_type(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "xml" => "application/xml",
        "html" => "text/html",
        "js" => "text/javascript",
        "ts" => "text/x-typescript",
        "py" => "text/x-python",
        "rs" => "text/x-rust",
        "go" => "text/x-go",
        _ => return None,
    })
}

/// Copy `sources` into the session's attachments folder and add them to its
/// record, all or nothing. `precondition` runs under the session's write lock
/// before the record changes (the host re-checks its authority there) and
/// must not call back into the store. No sources: nothing happens.
///
/// Blocking file IO: async callers run it in `spawn_blocking`.
pub fn import(
    store: &AssistantStore,
    sid: &str,
    sources: &[PathBuf],
    precondition: impl FnOnce() -> Result<(), String>,
) -> Result<Vec<Attachment>, String> {
    if sources.is_empty() {
        return Ok(vec![]);
    }
    stage(store, sid, sources)?.commit(store, sid, precondition)
}

/// Files copied into the attachments folder but not yet in the record. Until
/// [`Staged::commit`] saves them, the copies are removed on any failure.
struct Staged {
    items: Vec<Attachment>,
    txn: FileTxn,
}

/// Check every source against the formats and limits, then copy them.
fn stage(store: &AssistantStore, sid: &str, sources: &[PathBuf]) -> Result<Staged, String> {
    if sources.len() > MAX_PER_PICK {
        return Err(TOO_MANY.into());
    }
    let record = store.load_record(sid)?;
    // As the file system reports them, before anything is copied.
    let mut planned = Vec::with_capacity(sources.len());
    let mut bytes = 0u64;
    for source in sources {
        let mime = source_type(source)?;
        bytes = bytes.saturating_add(source_size(source)?);
        planned.push((source, mime));
    }
    check_room(&record, sources.len(), bytes)?;
    let dir = store.attachment_dir(sid)?;
    let mut staged = Staged {
        items: Vec::with_capacity(planned.len()),
        txn: FileTxn::new(),
    };
    for (source, mime) in planned {
        if let Err(error) = staged.copy(&dir, source, mime) {
            staged.rollback();
            return Err(error);
        }
    }
    Ok(staged)
}

impl Staged {
    fn copy(&mut self, dir: &Path, source: &Path, mime: &str) -> Result<(), String> {
        let bytes = read_source(source)?;
        let item = Attachment {
            id: id(),
            name: source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "attachment".into()),
            mime_type: mime.into(),
            size_bytes: bytes.len() as u64,
            status: "ready".into(),
            reason: None,
            preview_url: None,
            details: None,
        };
        let target = dir.join(&item.id);
        // A new id names nothing yet: whatever is there is not ours to replace.
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            _ => return Err(WRITE_FAILED.into()),
        }
        self.txn
            .write_with_mode(&target, &bytes, 0o600)
            .map_err(|error| {
                log::warn!("[Assistant] could not copy an attachment: {error}");
                WRITE_FAILED
            })?;
        self.items.push(item);
        Ok(())
    }

    /// Save the copies in the record. The limits are checked again under the
    /// session's lock, with what another pick may have added meanwhile.
    fn commit(
        self,
        store: &AssistantStore,
        sid: &str,
        precondition: impl FnOnce() -> Result<(), String>,
    ) -> Result<Vec<Attachment>, String> {
        let bytes = self
            .items
            .iter()
            .fold(0u64, |sum, item| sum.saturating_add(item.size_bytes));
        let saved = store.mutate(sid, |record| {
            precondition()?;
            check_room(record, self.items.len(), bytes)?;
            record.attachments.extend(self.items.iter().cloned());
            Ok(())
        });
        match saved {
            Ok(()) => Ok(self.items),
            Err(error) => {
                self.rollback();
                Err(error)
            }
        }
    }

    /// Remove the copies. A copy left behind is in no record, so never sent.
    fn rollback(self) {
        if let Err(error) = self.txn.rollback() {
            log::warn!("[Assistant] could not remove copied attachments: {error}");
        }
    }
}

/// Whether a session that is not in the trash has room for `count` more
/// files of `bytes` together.
fn check_room(record: &SessionRecord, count: usize, bytes: u64) -> Result<(), String> {
    if record.session.deleted_at.is_some() {
        return Err("assistant_session_deleted".into());
    }
    if record.attachments.len().saturating_add(count) > MAX_PER_SESSION {
        return Err(TOO_MANY.into());
    }
    let held = record
        .attachments
        .iter()
        .fold(0u64, |sum, item| sum.saturating_add(item.size_bytes));
    if held.saturating_add(bytes) > MAX_SESSION_BYTES {
        return Err(TOTAL_TOO_LARGE.into());
    }
    Ok(())
}

/// The MIME type of `source`, by its extension.
fn source_type(source: &Path) -> Result<&'static str, String> {
    source
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(|extension| mime_type(&extension.to_ascii_lowercase()))
        .ok_or_else(|| UNSUPPORTED.into())
}

/// `source` opened for reading, with its size, if it is a regular file within
/// [`MAX_FILE_BYTES`]. A link to one is followed: the user picked it. The
/// checks are made on the open file, so a path swapped meanwhile cannot slip
/// past them; on Unix it is opened without blocking, so a pipe or device put
/// there is refused instead of waiting.
fn open_source(source: &Path) -> Result<(fs::File, u64), String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(source).map_err(|_| UNREADABLE)?;
    let meta = file.metadata().map_err(|_| UNREADABLE)?;
    if !meta.is_file() {
        return Err(UNREADABLE.into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(TOO_LARGE.into());
    }
    Ok((file, meta.len()))
}

/// The size of a regular file, see [`open_source`].
fn source_size(source: &Path) -> Result<u64, String> {
    open_source(source).map(|(_, size)| size)
}

/// The bytes of `source`, read no further than one past [`MAX_FILE_BYTES`]:
/// a file that grew past the limit since its size was checked is too large.
fn read_source(source: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    open_source(source)?
        .0
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UNREADABLE)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(TOO_LARGE.into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, AssistantStore, String) {
        let config = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(config.path(), &"ef".repeat(32)).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        (config, store, sid)
    }

    fn file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn ok() -> Result<(), String> {
        Ok(())
    }

    /// The files in the session's attachments folder, by name.
    fn copies(store: &AssistantStore, sid: &str) -> Vec<String> {
        let root = store.attachment_root(sid).unwrap();
        let mut names: Vec<_> = match fs::read_dir(&root) {
            Ok(entries) => entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => vec![],
        };
        names.sort();
        names
    }

    fn existing(size_bytes: u64) -> Attachment {
        serde_json::from_value(json!({
            "id": id(), "name": "old.txt", "mimeType": "text/plain",
            "sizeBytes": size_bytes, "status": "ready",
        }))
        .unwrap()
    }

    fn seed(store: &AssistantStore, sid: &str, attachments: Vec<Attachment>) {
        store
            .mutate(sid, |record| {
                record.attachments = attachments;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn every_format_the_runtime_reads_has_its_type_and_office_files_have_none() {
        for extension in EXTENSIONS {
            assert!(mime_type(extension).is_some(), "{extension}");
        }
        for (extension, mime) in [
            ("png", "image/png"),
            ("jpg", "image/jpeg"),
            ("jpeg", "image/jpeg"),
            ("webp", "image/webp"),
            ("gif", "image/gif"),
            ("pdf", "application/pdf"),
            ("txt", "text/plain"),
            ("md", "text/markdown"),
            ("csv", "text/csv"),
            ("json", "application/json"),
            ("html", "text/html"),
            ("js", "text/javascript"),
        ] {
            assert_eq!(mime_type(extension), Some(mime), "{extension}");
        }
        for extension in ["docx", "xlsx", "pptx", "doc", "exe", "zip", "", "PNG"] {
            assert_eq!(mime_type(extension), None, "{extension}");
        }
    }

    #[test]
    fn imported_files_are_private_copies_with_their_names_and_types() {
        let (config, store, sid) = fixture();
        let picked = config.path().join("picked");
        fs::create_dir(&picked).unwrap();
        let sources = [
            file(&picked, "notes.txt", b"hello"),
            file(&picked, "Scan.PDF", b"%PDF-1.4"),
            file(&picked, "photo.jpeg", &[0xff, 0xd8, 0xff]),
        ];

        let imported = import(&store, &sid, &sources, ok).unwrap();

        let summary: Vec<_> = imported
            .iter()
            .map(|a| (a.name.as_str(), a.mime_type.as_str(), a.size_bytes))
            .collect();
        assert_eq!(
            summary,
            [
                ("notes.txt", "text/plain", 5),
                ("Scan.PDF", "application/pdf", 8),
                ("photo.jpeg", "image/jpeg", 3),
            ]
        );
        for attachment in &imported {
            validate_id(&attachment.id).unwrap();
            assert_eq!(attachment.status, "ready");
            assert!(attachment.reason.is_none() && attachment.details.is_none());
            let copy = store.attachment_root(&sid).unwrap().join(&attachment.id);
            let original = sources
                .iter()
                .find(|s| s.file_name().unwrap() == attachment.name.as_str())
                .unwrap();
            assert_eq!(fs::read(&copy).unwrap(), fs::read(original).unwrap());
            #[cfg(unix)]
            assert_eq!(crate::config::file_mode(&copy), Some(0o600));
        }
        let saved = store.load_record(&sid).unwrap().attachments;
        assert_eq!(
            saved.iter().map(|a| &a.id).collect::<Vec<_>>(),
            imported.iter().map(|a| &a.id).collect::<Vec<_>>()
        );
        // The originals are left as they were.
        assert_eq!(fs::read(&sources[0]).unwrap(), b"hello");
    }

    #[test]
    fn an_empty_pick_changes_nothing() {
        let (_config, store, sid) = fixture();
        assert!(import(&store, &sid, &[], ok).unwrap().is_empty());
        assert!(!store.attachment_root(&sid).unwrap().exists());
    }

    #[test]
    fn at_most_ten_files_a_pick_and_two_hundred_a_conversation() {
        let (config, store, sid) = fixture();
        let eleven: Vec<_> = (0..11)
            .map(|i| file(config.path(), &format!("{i}.txt"), b"x"))
            .collect();
        assert_eq!(import(&store, &sid, &eleven, ok).unwrap_err(), TOO_MANY);
        assert!(copies(&store, &sid).is_empty());

        seed(&store, &sid, (0..199).map(|_| existing(1)).collect());
        assert_eq!(
            import(&store, &sid, &eleven[..2], ok).unwrap_err(),
            TOO_MANY
        );
        assert_eq!(import(&store, &sid, &eleven[..1], ok).unwrap().len(), 1);
        assert_eq!(store.load_record(&sid).unwrap().attachments.len(), 200);
    }

    #[test]
    fn a_file_over_25_mib_is_refused_with_the_rest_of_its_pick() {
        let (config, store, sid) = fixture();
        let small = file(config.path(), "small.txt", b"x");
        let large = config.path().join("large.txt");
        fs::File::create(&large)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert_eq!(
            import(&store, &sid, &[small.clone(), large], ok).unwrap_err(),
            TOO_LARGE
        );
        assert!(copies(&store, &sid).is_empty());
        assert!(store.load_record(&sid).unwrap().attachments.is_empty());

        let limit = config.path().join("limit.txt");
        fs::File::create(&limit)
            .unwrap()
            .set_len(MAX_FILE_BYTES)
            .unwrap();
        assert_eq!(import(&store, &sid, &[limit], ok).unwrap().len(), 1);
    }

    #[test]
    fn a_conversation_holds_at_most_500_mib_of_files() {
        let (config, store, sid) = fixture();
        seed(&store, &sid, vec![existing(MAX_SESSION_BYTES - 2)]);
        let two = file(config.path(), "two.txt", b"ab");
        let three = file(config.path(), "three.txt", b"abc");
        assert_eq!(
            import(&store, &sid, &[three], ok).unwrap_err(),
            TOTAL_TOO_LARGE
        );
        assert!(copies(&store, &sid).is_empty());
        assert_eq!(import(&store, &sid, &[two], ok).unwrap().len(), 1);
    }

    #[test]
    fn only_readable_files_of_the_runtimes_formats_are_imported() {
        let (config, store, sid) = fixture();
        let fine = file(config.path(), "fine.txt", b"x");
        for (source, code) in [
            (file(config.path(), "report.docx", b"PK"), UNSUPPORTED),
            (file(config.path(), "sheet.xlsx", b"PK"), UNSUPPORTED),
            (file(config.path(), "deck.pptx", b"PK"), UNSUPPORTED),
            (file(config.path(), "README", b"x"), UNSUPPORTED),
            (config.path().join("missing.txt"), UNREADABLE),
            (
                {
                    let dir = config.path().join("folder.txt");
                    fs::create_dir(&dir).unwrap();
                    dir
                },
                UNREADABLE,
            ),
        ] {
            assert_eq!(
                import(&store, &sid, &[fine.clone(), source.clone()], ok).unwrap_err(),
                code,
                "{source:?}"
            );
        }
        assert!(copies(&store, &sid).is_empty());
        assert!(store.load_record(&sid).unwrap().attachments.is_empty());
    }

    /// A file swapped for a pipe after its size was checked: reading it is
    /// refused at once instead of waiting for a writer that never comes.
    #[cfg(unix)]
    #[test]
    fn a_source_swapped_for_a_pipe_is_refused_without_waiting() {
        let (config, _store, _sid) = fixture();
        let pipe = config.path().join("notes.txt");
        let name = std::ffi::CString::new(pipe.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(read_source(&pipe));
        });
        let read = result
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("reading a pipe must not wait for a writer");
        assert_eq!(read.unwrap_err(), UNREADABLE);
    }

    /// The size is the open file's, so a file that grew past the limit after
    /// it was picked is too large.
    #[test]
    fn the_size_is_checked_on_the_opened_file() {
        let (config, _store, _sid) = fixture();
        let big = file(config.path(), "big.txt", b"x");
        fs::File::options()
            .write(true)
            .open(&big)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert_eq!(read_source(&big).unwrap_err(), TOO_LARGE);
        assert_eq!(source_size(&big).unwrap_err(), TOO_LARGE);
    }

    /// A file that fails while it is copied takes the files already copied
    /// with it.
    #[cfg(unix)]
    #[test]
    fn a_copy_that_fails_midway_removes_what_it_copied() {
        use std::os::unix::fs::PermissionsExt;
        let (config, store, sid) = fixture();
        let first = file(config.path(), "first.txt", b"one");
        let locked = file(config.path(), "locked.txt", b"two");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::File::open(&locked).is_ok() {
            // Running as root: permissions do not stop the read.
            return;
        }
        assert_eq!(
            import(&store, &sid, &[first, locked], ok).unwrap_err(),
            UNREADABLE
        );
        assert!(copies(&store, &sid).is_empty());
        assert!(store.load_record(&sid).unwrap().attachments.is_empty());
    }

    #[test]
    fn a_refused_commit_removes_the_copies_and_saves_nothing() {
        let (config, store, sid) = fixture();
        let source = file(config.path(), "a.txt", b"x");
        let refused = || Err::<(), _>("assistant_identity_changed".to_string());
        assert_eq!(
            import(&store, &sid, &[source], refused).unwrap_err(),
            "assistant_identity_changed"
        );
        assert!(copies(&store, &sid).is_empty());
        assert!(store.load_record(&sid).unwrap().attachments.is_empty());
    }

    /// The limits are checked again under the session's lock: files another
    /// pick added meanwhile count.
    #[test]
    fn the_limits_hold_against_a_pick_that_landed_meanwhile() {
        let (config, store, sid) = fixture();
        let source = file(config.path(), "a.txt", b"x");
        let staged = stage(&store, &sid, &[source]).unwrap();
        assert_eq!(copies(&store, &sid).len(), 1);
        seed(&store, &sid, (0..200).map(|_| existing(1)).collect());
        assert_eq!(staged.commit(&store, &sid, ok).unwrap_err(), TOO_MANY);
        assert!(copies(&store, &sid).is_empty());
        assert_eq!(store.load_record(&sid).unwrap().attachments.len(), 200);

        seed(&store, &sid, vec![]);
        let source = file(config.path(), "b.txt", b"xy");
        let staged = stage(&store, &sid, &[source]).unwrap();
        seed(&store, &sid, vec![existing(MAX_SESSION_BYTES - 1)]);
        assert_eq!(
            staged.commit(&store, &sid, ok).unwrap_err(),
            TOTAL_TOO_LARGE
        );
        assert!(copies(&store, &sid).is_empty());
    }

    #[test]
    fn a_trashed_or_unknown_session_takes_no_files() {
        let (config, store, sid) = fixture();
        let source = file(config.path(), "a.txt", b"x");
        store
            .mutate(&sid, |record| {
                record.session.deleted_at = Some(now());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            import(&store, &sid, std::slice::from_ref(&source), ok).unwrap_err(),
            "assistant_session_deleted"
        );
        assert!(copies(&store, &sid).is_empty());
        assert_eq!(
            import(&store, &id(), &[source], ok).unwrap_err(),
            "assistant_session_not_found"
        );
    }

    #[cfg(unix)]
    #[test]
    fn files_are_never_copied_through_a_linked_folder() {
        let (config, store, sid) = fixture();
        let elsewhere = config.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, store.attachment_root(&sid).unwrap()).unwrap();
        let source = file(config.path(), "a.txt", b"x");
        assert_eq!(
            import(&store, &sid, &[source], ok).unwrap_err(),
            "assistant_invalid_storage_path"
        );
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    #[test]
    fn the_session_record_check_counts_files_and_bytes() {
        let mut record = SessionRecord {
            session: serde_json::from_value(json!({
                "id": id(), "title": "", "modelId": "m", "createdAt": 1, "updatedAt": 1,
            }))
            .unwrap(),
            messages: vec![],
            attachments: vec![existing(10)],
            usage: json!({}),
            operations: vec![],
        };
        assert!(check_room(&record, MAX_PER_SESSION - 1, MAX_SESSION_BYTES - 10).is_ok());
        assert_eq!(
            check_room(&record, MAX_PER_SESSION, 0).unwrap_err(),
            TOO_MANY
        );
        assert_eq!(
            check_room(&record, 1, MAX_SESSION_BYTES - 9).unwrap_err(),
            TOTAL_TOO_LARGE
        );
        record.session.deleted_at = Some(1);
        assert_eq!(
            check_room(&record, 0, 0).unwrap_err(),
            "assistant_session_deleted"
        );
    }
}
