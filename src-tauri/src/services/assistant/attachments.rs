//! Files the user imports into a conversation, picked or dropped (paths on
//! disk) or pasted (bytes): which formats can be imported, the limits, and
//! the all-or-nothing copy into the session's private attachments folder
//! (see [`AssistantStore::attachment_dir`]).
use super::documents::runner::SVG_OUTPUT_LIMIT;
use super::{
    store::{private_subdir, AssistantStore, SessionRecord, PAGES_SUFFIX},
    types::*,
};
use crate::config::FileTxn;
use std::borrow::Cow;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

/// The formats the runtime reads (assistant-runtime README, "Attachments"):
/// images, PDF, UTF-8 text and text-only OOXML Office documents.
pub const EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "pdf", "txt", "md", "csv", "json", "log", "yaml", "yml",
    "toml", "xml", "html", "js", "ts", "py", "rs", "go", "docx", "xlsx", "pptx",
];
/// The name of the dialog's file filter: format names, not a sentence.
pub const FILTER_NAME: &str = "PNG, JPG, WebP, GIF, PDF, DOCX, XLSX, PPTX, TXT";

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
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
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
        // Only generation tools use these, as video references.
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        _ => return None,
    })
}

/// Where a file to import comes from.
pub enum Source {
    /// A file the user picked or dropped, read from disk.
    Path(PathBuf),
    /// A file pasted from the clipboard. The name is only a hint: what the
    /// file is comes from its content (see [`classify`]).
    Bytes { name: String, bytes: Vec<u8> },
}

/// [`import_sources`] for files on disk.
pub fn import(
    store: &AssistantStore,
    sid: &str,
    sources: &[PathBuf],
    precondition: impl FnOnce() -> Result<(), String>,
) -> Result<Vec<Attachment>, String> {
    let sources: Vec<_> = sources.iter().cloned().map(Source::Path).collect();
    import_sources(store, sid, &sources, precondition)
}

/// Copy `sources` into the session's attachments folder and add them to its
/// record, all or nothing. `precondition` runs under the session's write lock
/// before the record changes (the host re-checks its authority there) and
/// must not call back into the store. No sources: nothing happens.
///
/// Blocking file IO: async callers run it in `spawn_blocking`.
pub fn import_sources(
    store: &AssistantStore,
    sid: &str,
    sources: &[Source],
    precondition: impl FnOnce() -> Result<(), String>,
) -> Result<Vec<Attachment>, String> {
    if sources.is_empty() {
        return Ok(vec![]);
    }
    store.with_attachment_lock(sid, || {
        // Best effort, as after an update: a leftover copy is in no record,
        // so one that cannot be removed must not block new files.
        if let Err(error) = store.cleanup_orphans_locked(sid) {
            log::warn!("[Assistant] could not remove unused attachment copies: {error}");
        }
        stage(store, sid, sources)?.commit(store, sid, precondition)
    })
}

/// Files copied into the attachments folder but not yet in the record. Until
/// [`Staged::commit`] saves them, the copies are removed on any failure.
struct Staged {
    items: Vec<Attachment>,
    txn: FileTxn,
}

/// A source that passed the format and size checks.
struct Planned<'a> {
    source: &'a Source,
    name: String,
    mime: &'static str,
    size: u64,
}

impl<'a> Planned<'a> {
    fn new(source: &'a Source) -> Result<Self, String> {
        match source {
            Source::Path(path) => Ok(Self {
                source,
                // As the file system reports them, before anything is copied.
                mime: source_type(path)?,
                size: source_size(path)?,
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "attachment".into()),
            }),
            Source::Bytes { name, bytes } => {
                let size = bytes.len() as u64;
                if size > MAX_FILE_BYTES {
                    return Err(TOO_LARGE.into());
                }
                let (name, mime) = classify(name, bytes)?;
                Ok(Self {
                    source,
                    name,
                    mime,
                    size,
                })
            }
        }
    }
}

/// Check every source against the formats and limits, then copy them.
fn stage(store: &AssistantStore, sid: &str, sources: &[Source]) -> Result<Staged, String> {
    if sources.len() > MAX_PER_PICK {
        return Err(TOO_MANY.into());
    }
    let record = store.load_record(sid)?;
    let mut planned = Vec::with_capacity(sources.len());
    let mut bytes = 0u64;
    for source in sources {
        let item = Planned::new(source)?;
        bytes = bytes.saturating_add(item.size);
        planned.push(item);
    }
    check_room(&record, sources.len(), bytes)?;
    let dir = store.attachment_dir(sid)?;
    let mut staged = Staged {
        items: Vec::with_capacity(planned.len()),
        txn: FileTxn::new(),
    };
    for item in &planned {
        if let Err(error) = staged.copy(&dir, item) {
            staged.rollback();
            return Err(error);
        }
    }
    Ok(staged)
}

impl Staged {
    fn copy(&mut self, dir: &Path, planned: &Planned) -> Result<(), String> {
        let bytes: Cow<[u8]> = match planned.source {
            Source::Path(path) => Cow::Owned(read_source(path)?),
            Source::Bytes { bytes, .. } => Cow::Borrowed(bytes),
        };
        let item = Attachment {
            origin: None,
            source_message_id: None,
            id: id(),
            name: planned.name.clone(),
            mime_type: planned.mime.into(),
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
            record
                .session
                .draft_attachment_ids
                .extend(self.items.iter().map(|item| item.id.clone()));
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
    if record.session.active_run.is_some() {
        return Err("assistant_session_busy".into());
    }
    check_capacity(record, count, bytes)
}

fn check_capacity(record: &SessionRecord, count: usize, bytes: u64) -> Result<(), String> {
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

/// Reserve conservatively before a billed generation. A live run prevents
/// imports, so its capacity remains available until its results are saved.
pub(super) fn image_room(record: &SessionRecord, count: usize) -> Result<(), String> {
    check_room(
        record,
        count,
        (count * super::media::MAX_IMAGE_BYTES) as u64,
    )
}

/// Room for a generation tool's `count` outputs of at most `bytes` in all.
/// It runs inside its chat run, so the live run does not refuse it.
pub(super) fn tool_room(record: &SessionRecord, count: usize, bytes: u64) -> Result<(), String> {
    if record.session.deleted_at.is_some() {
        return Err("assistant_session_deleted".into());
    }
    check_capacity(record, count, bytes)
}

/// Conservatively reserve one bounded speech output before a paid call.
pub(super) fn audio_room(record: &SessionRecord) -> Result<(), String> {
    check_room(record, 1, super::speech::MAX_OUTPUT_BYTES as u64)
}

/// Reserve the bounded original before crossing the paid boundary.
pub(super) fn original_audio_room(record: &SessionRecord) -> Result<(), String> {
    check_room(record, 1, super::audio_input::MAX_BYTES as u64)
}
/// Retention is explicit and belongs to a submitted ASR ledger, never chat turns.
pub(super) fn retained_audio<T>(
    store: &AssistantStore,
    sid: &str,
    media: &mut MediaRecord,
    input: &super::audio_input::Input,
    commit: impl FnOnce(&mut SessionRecord, &MediaRecord) -> Result<T, String>,
) -> Result<T, String> {
    if media.kind != "transcription"
        || media.submitted_at.is_none()
        || input.bytes.is_empty()
        || input.bytes.len() > super::audio_input::MAX_BYTES
        || !matches!(input.info.mime_type.as_str(), "audio/wav" | "audio/mpeg")
    {
        return Err(UNSUPPORTED.into());
    }
    store.with_attachment_lock(sid, || {
        let item = Attachment {
            id: id(),
            name: media
                .params
                .get("inputName")
                .and_then(serde_json::Value::as_str)
                .map(clean_name)
                .unwrap_or_else(|| "audio".into()),
            mime_type: input.info.mime_type.clone(),
            size_bytes: input.bytes.len() as u64,
            status: "ready".into(),
            reason: None,
            preview_url: None,
            details: None,
            origin: Some("audio-input".into()),
            source_message_id: None,
        };
        let target = store.attachment_dir(sid)?.join(&item.id);
        if fs::symlink_metadata(&target).is_ok() {
            return Err(WRITE_FAILED.into());
        }
        check_capacity(&store.load_record(sid)?, 1, item.size_bytes)?;
        let mut txn = FileTxn::new();
        let result = (|| {
            txn.write_with_mode(&target, &input.bytes, 0o600)
                .map_err(|_| WRITE_FAILED)?;
            media.input_ids = vec![item.id.clone()];
            store.mutate(sid, |record| {
                check_capacity(record, 1, item.size_bytes)?;
                let value = commit(record, media)?;
                record.attachments.push(item);
                Ok(value)
            })
        })();
        if result.is_err() {
            let _ = txn.rollback();
            media.input_ids.clear();
        }
        result
    })
}

/// Publish an independent audio file and its ledger references atomically.
/// `source`: the reply a generation tool made it for.
pub(super) fn generated_audio<T>(
    store: &AssistantStore,
    sid: &str,
    source: Option<&str>,
    media: &mut MediaRecord,
    bytes: &[u8],
    info: &super::speech::OutputInfo,
    commit: impl FnOnce(&mut SessionRecord, &MediaRecord, &Attachment) -> Result<T, String>,
) -> Result<T, String> {
    if bytes.len() > super::speech::MAX_OUTPUT_BYTES
        || info.size_bytes != bytes.len() as u64
        || media.kind != "speech"
    {
        return Err(UNSUPPORTED.into());
    }
    let extension = match info.mime_type.as_str() {
        "audio/wav" => "wav",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "opus",
        _ => return Err(UNSUPPORTED.into()),
    };
    store.with_attachment_lock(sid, || {
        let item = Attachment {
            id: id(),
            name: format!("speech.{extension}"),
            mime_type: info.mime_type.clone(),
            size_bytes: info.size_bytes,
            status: "ready".into(),
            reason: None,
            preview_url: None,
            details: None,
            origin: Some("generated".into()),
            source_message_id: source.map(str::to_owned),
        };
        let target = store.attachment_dir(sid)?.join(&item.id);
        if fs::symlink_metadata(&target).is_ok() {
            return Err(WRITE_FAILED.into());
        }
        check_capacity(&store.load_record(sid)?, 1, item.size_bytes)?;
        let mut txn = FileTxn::new();
        let result = (|| {
            txn.write_with_mode(&target, bytes, 0o600)
                .map_err(|_| WRITE_FAILED)?;
            media.outputs = vec![item.id.clone()];
            store.mutate(sid, |record| {
                check_capacity(record, 1, item.size_bytes)?;
                let value = commit(record, media, &item)?;
                record.attachments.push(item);
                Ok(value)
            })
        })();
        if result.is_err() {
            let _ = txn.rollback();
            media.outputs.clear();
        }
        result
    })
}

/// Commit generated bytes and their message/ledger references together.
/// Roll back all copied files if authority changed or saving the record fails.
/// `artifact`: the tool call the images are a generation of; `None` for a
/// direct media turn, whose message they replace.
pub(super) fn generated(
    store: &AssistantStore,
    sid: &str,
    mid: &str,
    artifact: Option<&str>,
    sources: &[Source],
    mut media: MediaRecord,
    precondition: impl FnOnce(&SessionRecord) -> Result<(), String>,
) -> Result<Vec<Attachment>, String> {
    if sources.is_empty() || sources.len() > MAX_PER_PICK {
        return Err(TOO_MANY.into());
    }
    store.with_attachment_lock(sid, || {
        let mut staged = Staged {
            items: vec![],
            txn: FileTxn::new(),
        };
        let dir = store.attachment_dir(sid)?;
        let copied = (|| {
            for source in sources {
                let Source::Bytes { name, bytes } = source else {
                    return Err(UNREADABLE.into());
                };
                if bytes.len() > super::media::MAX_IMAGE_BYTES {
                    return Err(TOO_LARGE.into());
                }
                let ext = super::media::image_type(bytes).ok_or(UNSUPPORTED)?;
                staged.copy(
                    &dir,
                    &Planned {
                        source,
                        name: name.clone(),
                        mime: mime_type(ext).ok_or(UNSUPPORTED)?,
                        size: bytes.len() as u64,
                    },
                )?;
                let item = staged.items.last_mut().expect("copied");
                item.origin = Some("generated".into());
                item.source_message_id = Some(mid.into());
            }
            media.outputs = staged.items.iter().map(|a| a.id.clone()).collect();
            store.mutate(sid, |record| {
                precondition(record)?;
                check_capacity(
                    record,
                    staged.items.len(),
                    staged.items.iter().map(|a| a.size_bytes).sum(),
                )?;
                let message = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == mid)
                    .ok_or("assistant_message_not_found")?;
                if artifact.is_some() {
                    message.attachments.extend(staged.items.iter().cloned());
                    message.put_generation(media.clone());
                    record.attachments.extend(staged.items.iter().cloned());
                    return Ok(());
                }
                message.attachments = staged.items.clone();
                message.media = Some(media.clone());
                message.usage = Some(
                    serde_json::json!({"mediaCostUsd":media.cost,"estimatedCostUsd":media.cost}),
                );
                let run = message.run_id.clone();
                for op in record
                    .operations
                    .iter_mut()
                    .filter(|op| Some(&op.run_id) == run.as_ref())
                {
                    op.media = Some(media.clone());
                }
                record.attachments.extend(staged.items.iter().cloned());
                Ok(())
            })
        })();
        match copied {
            Ok(()) => Ok(staged.items),
            Err(error) => {
                staged.rollback();
                Err(error)
            }
        }
    })
}

/// Publish a downloaded video and its references together, without buffering
/// the movie in memory. A failed record write rolls the private copy back.
pub(super) fn generated_video(
    store: &AssistantStore,
    sid: &str,
    mid: &str,
    artifact: Option<&str>,
    file: &mut std::fs::File,
    mut media: MediaRecord,
    precondition: impl FnOnce(&SessionRecord) -> Result<(), String>,
) -> Result<Attachment, String> {
    use std::io::{Seek, SeekFrom};
    let size = file.metadata().map_err(|_| UNREADABLE)?.len();
    if size > super::video::MAX_VIDEO_BYTES {
        return Err(TOO_LARGE.into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| UNREADABLE)?;
    let mut head = [0_u8; 12];
    file.read_exact(&mut head).map_err(|_| UNREADABLE)?;
    if &head[4..8] != b"ftyp" {
        return Err(UNSUPPORTED.into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| UNREADABLE)?;
    store.with_attachment_lock(sid, || {
        let mut txn = FileTxn::new();
        let item = Attachment {
            id: id(),
            name: "generated.mp4".into(),
            mime_type: "video/mp4".into(),
            size_bytes: size,
            status: "ready".into(),
            reason: None,
            preview_url: None,
            details: None,
            origin: Some("generated".into()),
            source_message_id: Some(mid.into()),
        };
        let dir = store.attachment_dir(sid)?;
        let target = dir.join(&item.id);
        if fs::symlink_metadata(&target).is_ok() {
            return Err(WRITE_FAILED.into());
        }
        // Check quota before copying and again under the record lock. Other
        // imports cannot publish in between because this attachment lock is held.
        check_capacity(&store.load_record(sid)?, 1, size)?;
        media.outputs = vec![item.id.clone()];
        media.status = "completed".into();
        let copied = (|| {
            txn.copy_with_mode(&target, &mut file.take(size + 1), 0o600)
                .map_err(|_| WRITE_FAILED)?;
            if fs::metadata(&target).map_err(|_| WRITE_FAILED)?.len() != size {
                return Err(UNREADABLE.into());
            }
            store.mutate(sid, |record| {
                precondition(record)?;
                check_capacity(record, 1, size)?;
                let message = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == mid)
                    .ok_or("assistant_message_not_found")?;
                if artifact.is_some() {
                    message.attachments.push(item.clone());
                    message.put_generation(media.clone());
                    record.attachments.push(item.clone());
                    return Ok(());
                }
                message.attachments = vec![item.clone()];
                message.media = Some(media.clone());
                message.status = "completed".into();
                message.error_code = None;
                message.completed_at = Some(now());
                message.usage = Some(
                    serde_json::json!({"mediaCostUsd":media.cost,"estimatedCostUsd":media.cost}),
                );
                let run = message.run_id.clone();
                for op in record
                    .operations
                    .iter_mut()
                    .filter(|op| Some(&op.run_id) == run.as_ref())
                {
                    op.media = Some(media.clone());
                    op.status = OperationStatus::Completed;
                }
                record.attachments.push(item.clone());
                Ok(())
            })
        })();
        match copied {
            Ok(()) => Ok(item),
            Err(error) => {
                let _ = txn.rollback();
                Err(error)
            }
        }
    })
}

/// Publish a document the host made (see `documents::create`) with its
/// generation record, as generated attachment `id` of reply `mid` (the id of
/// the folder it was made in). Its type is its name's extension, and the
/// bytes must be what that says: a PDF starts with `%PDF`, an Office file is
/// a zip; text is taken as it is. `media.params` (format, counts, issues)
/// also become the attachment's details. A failed record write rolls the
/// copy back.
#[allow(clippy::too_many_arguments)]
pub(super) fn generated_file(
    store: &AssistantStore,
    sid: &str,
    mid: &str,
    id: &str,
    file: &mut std::fs::File,
    name: &str,
    mut media: MediaRecord,
    precondition: impl FnOnce(&SessionRecord) -> Result<(), String>,
) -> Result<Attachment, String> {
    use std::io::{Seek, SeekFrom};
    validate_id(id)?;
    let mime = name
        .rsplit_once('.')
        .and_then(|(_, extension)| mime_type(&extension.to_ascii_lowercase()))
        .ok_or(UNSUPPORTED)?;
    let size = file.metadata().map_err(|_| UNREADABLE)?.len();
    if size > MAX_FILE_BYTES {
        return Err(TOO_LARGE.into());
    }
    let magic: &[u8] = match mime {
        "application/pdf" => b"%PDF",
        m if m.starts_with("application/vnd.openxmlformats-officedocument.") => b"PK\x03\x04",
        _ => b"",
    };
    file.seek(SeekFrom::Start(0)).map_err(|_| UNREADABLE)?;
    let mut head = vec![0_u8; magic.len()];
    if file.read_exact(&mut head).is_err() || head != magic {
        return Err(UNSUPPORTED.into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| UNREADABLE)?;
    let mut details = media.params.clone();
    if let Some(details) = details.as_object_mut() {
        details.remove("name");
    }
    store.with_attachment_lock(sid, || {
        let mut txn = FileTxn::new();
        let item = Attachment {
            id: id.into(),
            name: name.into(),
            mime_type: mime.into(),
            size_bytes: size,
            status: "ready".into(),
            reason: None,
            preview_url: None,
            details: Some(details),
            origin: Some("generated".into()),
            source_message_id: Some(mid.into()),
        };
        let target = store.attachment_dir(sid)?.join(&item.id);
        if fs::symlink_metadata(&target).is_ok() {
            return Err(WRITE_FAILED.into());
        }
        check_capacity(&store.load_record(sid)?, 1, size)?;
        media.outputs = vec![item.id.clone()];
        let copied = (|| {
            txn.copy_with_mode(&target, &mut file.take(size + 1), 0o600)
                .map_err(|_| WRITE_FAILED)?;
            if fs::metadata(&target).map_err(|_| WRITE_FAILED)?.len() != size {
                return Err(UNREADABLE.into());
            }
            store.mutate(sid, |record| {
                precondition(record)?;
                check_capacity(record, 1, size)?;
                let message = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == mid)
                    .ok_or("assistant_message_not_found")?;
                message.attachments.push(item.clone());
                message.put_generation(media.clone());
                record.attachments.push(item.clone());
                Ok(())
            })
        })();
        match copied {
            Ok(()) => Ok(item),
            Err(error) => {
                let _ = txn.rollback();
                Err(error)
            }
        }
    })
}

/// Keep a generated deck's slide pictures (each `<n>.svg`, see
/// `documents::create`) beside its attachment `id`: private copies in the
/// private folder `<id>.pages`, all or none; a page that is not a regular
/// file is refused (see [`open_page`]). How many were kept. The folder goes
/// with the attachment (see [`AssistantStore::cleanup_orphan_attachments`]).
pub(super) fn keep_pages(
    store: &AssistantStore,
    sid: &str,
    id: &str,
    pages: &[PathBuf],
) -> Result<usize, String> {
    validate_id(id)?;
    store.with_attachment_lock(sid, || {
        let dir = private_subdir(&store.attachment_dir(sid)?, &format!("{id}{PAGES_SUFFIX}"))?;
        let mut txn = FileTxn::new();
        let copied = pages.iter().try_for_each(|page| {
            let name = page.file_name().ok_or(UNREADABLE)?;
            let (source, size) = open_page(page)?;
            txn.copy_with_mode(&dir.join(name), &mut source.take(size), 0o600)
                .map_err(|_| WRITE_FAILED.to_string())
        });
        match copied {
            Ok(()) => Ok(pages.len()),
            Err(error) => {
                let _ = txn.rollback();
                let _ = fs::remove_dir(&dir);
                Err(error)
            }
        }
    })
}

/// Record `media`, the generation record of generated attachment `id` of
/// reply `mid`, again with `svgPages` as its params now say (the slide
/// pictures [`keep_pages`] kept), and that count in the attachment's
/// details.
pub(super) fn record_kept_pages(
    store: &AssistantStore,
    sid: &str,
    mid: &str,
    id: &str,
    media: &MediaRecord,
) -> Result<(), String> {
    let count = media.params["svgPages"].clone();
    store.mutate(sid, |record| {
        let counted = |item: &mut Attachment| {
            if item.id == id {
                if let Some(details) = item
                    .details
                    .as_mut()
                    .and_then(serde_json::Value::as_object_mut)
                {
                    details.insert("svgPages".into(), count.clone());
                }
            }
        };
        record.attachments.iter_mut().for_each(counted);
        let message = record
            .messages
            .iter_mut()
            .find(|m| m.id == mid)
            .ok_or("assistant_message_not_found")?;
        message.attachments.iter_mut().for_each(counted);
        message.put_generation(media.clone());
        Ok(())
    })
}

/// A slide picture the host wrote in a document's folder, opened for
/// reading, with its size: a regular file within what the runner keeps of
/// a slide's SVG ([`SVG_OUTPUT_LIMIT`]), in a folder that is not a link (see
/// [`open_written`]).
pub(super) fn open_page(page: &Path) -> Result<(fs::File, u64), String> {
    open_written(page, SVG_OUTPUT_LIMIT as u64)
}

/// A file an engine could write, opened for reading, with its size: a
/// regular file of at most `max` bytes, in a folder that is not a link. A
/// link is never followed (on Unix it cannot be swapped in after the check
/// either) and the file is opened without blocking: a link, pipe or device
/// left in its place is refused instead of read or waited on.
pub(super) fn open_written(page: &Path, max: u64) -> Result<(fs::File, u64), String> {
    let real = |path: &Path, dir: bool| {
        fs::symlink_metadata(path).is_ok_and(
            |meta| {
                if dir {
                    meta.is_dir()
                } else {
                    meta.is_file()
                }
            },
        )
    };
    if !page.parent().is_some_and(|folder| real(folder, true)) || !real(page, false) {
        return Err(UNREADABLE.into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(page).map_err(|_| UNREADABLE)?;
    let meta = file.metadata().map_err(|_| UNREADABLE)?;
    if !meta.is_file() || meta.len() > max {
        return Err(UNREADABLE.into());
    }
    Ok((file, meta.len()))
}

/// The MIME type of `source`, by its extension.
fn source_type(source: &Path) -> Result<&'static str, String> {
    source
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(|extension| mime_type(&extension.to_ascii_lowercase()))
        .ok_or_else(|| UNSUPPORTED.into())
}

/// The formats whose bytes must say what they are: a name alone is not
/// enough for these.
const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "pdf", "docx", "xlsx", "pptx", "mp3", "wav", "mp4",
];

/// The longest stem a pasted file keeps in its name, in characters.
const MAX_PASTED_STEM: usize = 100;

/// A pasted file's name and MIME type. The content decides first: what its
/// first bytes say it is (an image or PDF) wins over its name, and a name
/// that is wrong gets the right extension. A zip is only a format the
/// runtime reads when its name says which Office file it is. Content with no
/// signature (text) is typed by its name; a name that claims a binary format
/// the bytes are not is unreadable, and one with no readable extension is
/// unsupported. A file without a usable name gets one made from its type.
fn classify(name: &str, bytes: &[u8]) -> Result<(String, &'static str), String> {
    let clean = clean_name(name);
    let (stem, name_extension) = clean.rsplit_once('.').unwrap_or((&clean, ""));
    let name_type = name_extension.to_ascii_lowercase();
    let extension = match sniff(bytes) {
        Some("zip") if matches!(name_type.as_str(), "docx" | "xlsx" | "pptx") => name_type.clone(),
        Some("zip") => return Err(UNSUPPORTED.into()),
        Some(kind) => kind.to_owned(),
        None if mime_type(&name_type).is_none() => return Err(UNSUPPORTED.into()),
        None if BINARY_EXTENSIONS.contains(&name_type.as_str()) => return Err(UNREADABLE.into()),
        None => name_type.clone(),
    };
    let mime = mime_type(&extension).ok_or(UNSUPPORTED)?;
    let same_type = name_type == extension || (name_type == "jpeg" && extension == "jpg");
    // An extension of the right type stays as the user wrote it.
    let shown_extension = if same_type {
        name_extension
    } else {
        &extension
    };
    let name = if stem.is_empty() {
        let kind = if mime.starts_with("image/") {
            "image"
        } else {
            "file"
        };
        format!(
            "pasted-{kind}-{}.{shown_extension}",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        )
    } else {
        let stem: String = stem.chars().take(MAX_PASTED_STEM).collect();
        format!("{stem}.{shown_extension}")
    };
    Ok((name, mime))
}

/// The last part of a name the renderer sent, without path separators or
/// control characters: it is shown, never used as a path.
pub(super) fn clean_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let printable: String = last.chars().filter(|c| !c.is_control()).collect();
    printable.trim().trim_end_matches('.').trim_end().to_owned()
}

/// What the first bytes of a file say it is: a canonical extension, or "zip".
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"%PDF-") {
        Some("pdf")
    } else if bytes.starts_with(b"PK\x03\x04") {
        Some("zip")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Some("wav")
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        Some("mp4")
    } else if bytes.starts_with(b"ID3")
        || bytes
            .get(..2)
            .is_some_and(|b| b[0] == 0xff && b[1] & 0xe0 == 0xe0)
    {
        Some("mp3")
    } else {
        None
    }
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
pub(super) fn read_source(source: &Path) -> Result<Vec<u8>, String> {
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
                record.session.draft_attachment_ids =
                    attachments.iter().map(|a| a.id.clone()).collect();
                record.attachments = attachments;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn every_readable_format_has_its_type_and_legacy_office_is_unsupported() {
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
        for extension in ["doc", "xls", "ppt", "exe", "zip", "", "PNG"] {
            assert_eq!(mime_type(extension), None, "{extension}");
        }
    }

    #[test]
    fn imported_copies_and_their_draft_references_are_committed_together() {
        let (config, store, sid) = fixture();
        let source = file(config.path(), "new.txt", b"unsent");
        let imported = import(&store, &sid, &[source], ok).unwrap();
        let saved = store.load_record(&sid).unwrap();
        assert_eq!(saved.session.draft_attachment_ids, [imported[0].id.clone()]);
        store.cleanup_orphan_attachments(&sid).unwrap();
        assert!(store
            .attachment_root(&sid)
            .unwrap()
            .join(&imported[0].id)
            .is_file());
    }

    #[test]
    fn office_files_can_be_imported_with_official_mime_types() {
        let (config, store, sid) = fixture();
        for (extension, suffix) in [
            ("docx", "wordprocessingml.document"),
            ("xlsx", "spreadsheetml.sheet"),
            ("pptx", "presentationml.presentation"),
        ] {
            let source = file(config.path(), &format!("中文.{extension}"), b"PK");
            let imported = import(&store, &sid, &[source], ok).unwrap();
            assert_eq!(
                imported[0].mime_type,
                format!("application/vnd.openxmlformats-officedocument.{suffix}")
            );
            assert!(EXTENSIONS.contains(&extension));
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
            (file(config.path(), "old.doc", b"binary"), UNSUPPORTED),
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
        let staged = stage(&store, &sid, &[Source::Path(source)]).unwrap();
        assert_eq!(copies(&store, &sid).len(), 1);
        seed(&store, &sid, (0..200).map(|_| existing(1)).collect());
        assert_eq!(staged.commit(&store, &sid, ok).unwrap_err(), TOO_MANY);
        assert!(copies(&store, &sid).is_empty());
        assert_eq!(store.load_record(&sid).unwrap().attachments.len(), 200);

        seed(&store, &sid, vec![]);
        let source = file(config.path(), "b.txt", b"xy");
        let staged = stage(&store, &sid, &[Source::Path(source)]).unwrap();
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

    /// A copy no record lists that cannot be removed (here: flagged
    /// immutable) is logged and left; it must not block new files.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_leftover_copy_that_cannot_be_removed_does_not_block_an_import() {
        let (config, store, sid) = fixture();
        let leftover = store.attachment_dir(&sid).unwrap().join(id());
        fs::write(&leftover, b"left by a crash").unwrap();
        let name = std::ffi::CString::new(leftover.to_str().unwrap()).unwrap();
        assert_eq!(
            unsafe { libc::chflags(name.as_ptr(), libc::UF_IMMUTABLE) },
            0
        );
        let source = file(config.path(), "new.txt", b"new");
        let imported = import(&store, &sid, &[source], ok);
        unsafe { libc::chflags(name.as_ptr(), 0) };
        assert_eq!(imported.unwrap().len(), 1);
        assert!(leftover.exists());
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

    // ── bytes pasted into the window ─────────────────────────────────────

    fn pasted(name: &str, bytes: &[u8]) -> Source {
        Source::Bytes {
            name: name.into(),
            bytes: bytes.to_vec(),
        }
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 0x10];
    const GIF: &[u8] = b"GIF89a\x01\0\x01\0";
    const WEBP: &[u8] = b"RIFF\x24\0\0\0WEBPVP8 ";
    const PDF: &[u8] = b"%PDF-1.7\n";
    const ZIP: &[u8] = b"PK\x03\x04\x14\0";

    fn import_one(store: &AssistantStore, sid: &str, source: Source) -> Result<Attachment, String> {
        import_sources(store, sid, &[source], ok).map(|mut files| files.remove(0))
    }

    #[test]
    fn pasted_bytes_become_a_private_copy_with_their_name_and_type() {
        let (_config, store, sid) = fixture();
        let imported = import_one(&store, &sid, pasted("notes.md", b"# Notes")).unwrap();
        assert_eq!(
            (
                imported.name.as_str(),
                imported.mime_type.as_str(),
                imported.size_bytes,
                imported.status.as_str()
            ),
            ("notes.md", "text/markdown", 7, "ready")
        );
        validate_id(&imported.id).unwrap();
        let copy = store.attachment_root(&sid).unwrap().join(&imported.id);
        assert_eq!(fs::read(&copy).unwrap(), b"# Notes");
        #[cfg(unix)]
        assert_eq!(crate::config::file_mode(&copy), Some(0o600));
        assert_eq!(
            store
                .load_record(&sid)
                .unwrap()
                .session
                .draft_attachment_ids,
            [imported.id]
        );
    }

    #[test]
    fn what_the_bytes_are_beats_what_the_name_says() {
        let (_config, store, sid) = fixture();
        for (name, bytes, want_name, want_mime) in [
            ("photo.txt", PNG, "photo.png", "image/png"),
            ("shot.png", JPEG, "shot.jpg", "image/jpeg"),
            ("anim", GIF, "anim.gif", "image/gif"),
            ("pic.PDF", WEBP, "pic.webp", "image/webp"),
            ("report.TXT", PDF, "report.pdf", "application/pdf"),
            // The extension of the same type is left as the user wrote it.
            ("photo.JPEG", JPEG, "photo.JPEG", "image/jpeg"),
            ("scan.png", PNG, "scan.png", "image/png"),
        ] {
            let imported = import_one(&store, &sid, pasted(name, bytes)).unwrap();
            assert_eq!(
                (imported.name.as_str(), imported.mime_type.as_str()),
                (want_name, want_mime),
                "{name}"
            );
        }
    }

    #[test]
    fn office_files_are_told_apart_by_name_because_all_of_them_are_zip() {
        let (_config, store, sid) = fixture();
        for (name, suffix) in [
            ("a.docx", "wordprocessingml.document"),
            ("b.XLSX", "spreadsheetml.sheet"),
            ("c.pptx", "presentationml.presentation"),
        ] {
            let imported = import_one(&store, &sid, pasted(name, ZIP)).unwrap();
            assert_eq!(
                imported.mime_type,
                format!("application/vnd.openxmlformats-officedocument.{suffix}")
            );
        }
        // A zip that is not one of those is not a format the runtime reads.
        for name in ["a.txt", "a.zip", "a"] {
            assert_eq!(
                import_one(&store, &sid, pasted(name, ZIP)).unwrap_err(),
                UNSUPPORTED,
                "{name}"
            );
        }
    }

    #[test]
    fn a_name_that_claims_a_binary_format_the_bytes_are_not_is_unreadable() {
        let (_config, store, sid) = fixture();
        for name in ["fake.png", "fake.jpg", "fake.pdf", "fake.docx", "fake.gif"] {
            assert_eq!(
                import_one(&store, &sid, pasted(name, b"just some text")).unwrap_err(),
                UNREADABLE,
                "{name}"
            );
        }
        // PNG bytes cut short are no PNG.
        assert_eq!(
            import_one(&store, &sid, pasted("a.png", &PNG[..7])).unwrap_err(),
            UNREADABLE
        );
        assert!(copies(&store, &sid).is_empty());
    }

    #[test]
    fn pasted_bytes_of_no_known_format_are_unsupported() {
        let (_config, store, sid) = fixture();
        for name in ["", "README", "tool.exe", "legacy.doc", "x.svg", ".hidden"] {
            assert_eq!(
                import_one(&store, &sid, pasted(name, b"MZ\x90\0")).unwrap_err(),
                UNSUPPORTED,
                "{name:?}"
            );
        }
        assert!(copies(&store, &sid).is_empty());
    }

    #[test]
    fn a_pasted_file_without_a_name_gets_one_from_its_type() {
        let (_config, store, sid) = fixture();
        for (name, bytes, prefix, extension) in [
            ("", PNG, "pasted-image-", ".png"),
            ("   ", JPEG, "pasted-image-", ".jpg"),
            (".png", PNG, "pasted-image-", ".png"),
            ("dir/", GIF, "pasted-image-", ".gif"),
            ("", PDF, "pasted-file-", ".pdf"),
            (".txt", b"hello", "pasted-file-", ".txt"),
        ] {
            let imported = import_one(&store, &sid, pasted(name, bytes)).unwrap();
            assert!(
                imported.name.starts_with(prefix) && imported.name.ends_with(extension),
                "{name:?} -> {}",
                imported.name
            );
            // prefix + YYYYMMDD-HHMMSS + extension
            assert_eq!(
                imported.name.len(),
                prefix.len() + "20260101-120000".len() + extension.len(),
                "{}",
                imported.name
            );
        }
    }

    #[test]
    fn a_pasted_name_is_only_a_file_name() {
        let (_config, store, sid) = fixture();
        for (name, want) in [
            ("../../etc/passwd.txt", "passwd.txt"),
            ("C:\\Users\\me\\notes.md", "notes.md"),
            ("a/b\\c.txt", "c.txt"),
            ("tab\there\u{7}.txt", "tabhere.txt"),
            ("中文 报告.csv", "中文 报告.csv"),
        ] {
            let imported = import_one(&store, &sid, pasted(name, b"x")).unwrap();
            assert_eq!(imported.name, want, "{name:?}");
        }
        let long = format!("{}.txt", "a".repeat(500));
        let imported = import_one(&store, &sid, pasted(&long, b"x")).unwrap();
        assert!(imported.name.ends_with(".txt"));
        assert!(imported.name.chars().count() <= 104, "{}", imported.name);
    }

    #[test]
    fn pasted_files_obey_the_same_limits_as_picked_ones() {
        let (config, store, sid) = fixture();
        let big = vec![b'x'; MAX_FILE_BYTES as usize + 1];
        assert_eq!(
            import_one(&store, &sid, pasted("big.txt", &big)).unwrap_err(),
            TOO_LARGE
        );
        // Whatever the name or content, a body that is too large is too large.
        assert_eq!(
            import_one(&store, &sid, pasted("big.exe", &big)).unwrap_err(),
            TOO_LARGE
        );
        assert!(copies(&store, &sid).is_empty());
        let limit = vec![b'x'; MAX_FILE_BYTES as usize];
        assert_eq!(
            import_one(&store, &sid, pasted("limit.txt", &limit))
                .unwrap()
                .size_bytes,
            MAX_FILE_BYTES
        );

        let (_config, store, sid) = fixture();
        let eleven: Vec<_> = (0..11).map(|i| pasted(&format!("{i}.txt"), b"x")).collect();
        assert_eq!(
            import_sources(&store, &sid, &eleven, ok).unwrap_err(),
            TOO_MANY
        );
        seed(&store, &sid, (0..199).map(|_| existing(1)).collect());
        assert_eq!(
            import_sources(&store, &sid, &eleven[..2], ok).unwrap_err(),
            TOO_MANY
        );
        assert_eq!(
            import_sources(&store, &sid, &eleven[..1], ok)
                .unwrap()
                .len(),
            1
        );

        seed(&store, &sid, vec![existing(MAX_SESSION_BYTES - 2)]);
        assert_eq!(
            import_one(&store, &sid, pasted("three.txt", b"abc")).unwrap_err(),
            TOTAL_TOO_LARGE
        );
        assert_eq!(
            import_one(&store, &sid, pasted("two.txt", b"ab"))
                .unwrap()
                .size_bytes,
            2
        );
        drop(config);
    }

    #[test]
    fn picked_paths_and_pasted_bytes_are_staged_all_or_nothing_together() {
        let (config, store, sid) = fixture();
        let fine = file(config.path(), "fine.txt", b"x");
        let both = [Source::Path(fine), pasted("ok.txt", b"y")];
        let imported = import_sources(&store, &sid, &both, ok).unwrap();
        assert_eq!(
            imported.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["fine.txt", "ok.txt"]
        );

        let (config, store, sid) = fixture();
        let fine = file(config.path(), "fine.txt", b"x");
        let refused = [Source::Path(fine), pasted("bad.exe", b"MZ")];
        assert_eq!(
            import_sources(&store, &sid, &refused, ok).unwrap_err(),
            UNSUPPORTED
        );
        assert!(copies(&store, &sid).is_empty());
        assert!(store.load_record(&sid).unwrap().attachments.is_empty());
    }

    #[test]
    fn a_refused_commit_removes_pasted_copies_too() {
        let (_config, store, sid) = fixture();
        let refused = || Err::<(), _>("assistant_identity_changed".to_string());
        assert_eq!(
            import_sources(&store, &sid, &[pasted("a.txt", b"x")], refused).unwrap_err(),
            "assistant_identity_changed"
        );
        assert!(copies(&store, &sid).is_empty());
        assert!(store.load_record(&sid).unwrap().attachments.is_empty());
    }

    #[test]
    fn a_trashed_conversation_takes_no_pasted_files() {
        let (_config, store, sid) = fixture();
        store
            .mutate(&sid, |record| {
                record.session.deleted_at = Some(now());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            import_one(&store, &sid, pasted("a.txt", b"x")).unwrap_err(),
            "assistant_session_deleted"
        );
        assert!(copies(&store, &sid).is_empty());
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
            inherited_usage: vec![],
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
    #[test]
    fn generated_video_is_streamed_private_atomic_and_quota_checked() {
        use std::io::{Seek, SeekFrom, Write};
        let (_config, store, sid) = fixture();
        let mid = id();
        let run = id();
        let media = MediaRecord {
            kind: "video".into(),
            status: "completed".into(),
            cost: Some("0.12".into()),
            job_id: Some(id()),
            ..Default::default()
        };
        store.mutate(&sid, |record| {
            record.messages.push(serde_json::from_value(json!({"id":mid,"role":"media","content":"square","createdAt":1,"status":"downloading","runId":run,"media":media})).unwrap());
            record.operations.push(Operation {id:id(),run_id:run,status:OperationStatus::PendingMedia,media:Some(media.clone()),search_price:None}); Ok(())
        }).unwrap();
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"\0\0\0\x18ftypisomfixture").unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let attachment =
            generated_video(&store, &sid, &mid, None, &mut file, media.clone(), |_| {
                Ok(())
            })
            .unwrap();
        assert_eq!(attachment.size_bytes, MAX_FILE_BYTES + 1);
        assert_eq!(attachment.mime_type, "video/mp4");
        assert_eq!(attachment.origin.as_deref(), Some("generated"));
        assert_eq!(attachment.source_message_id.as_deref(), Some(mid.as_str()));
        let path = store.attachment_root(&sid).unwrap().join(&attachment.id);
        #[cfg(unix)]
        assert_eq!(crate::config::file_mode(&path), Some(0o600));
        store.cleanup_orphan_attachments(&sid).unwrap();
        assert!(path.exists());
        let record = store.load_record(&sid).unwrap();
        assert_eq!(
            record.messages[0]
                .media
                .as_ref()
                .unwrap()
                .outputs
                .as_slice(),
            std::slice::from_ref(&attachment.id)
        );
        assert_eq!(record.messages[0].status, "completed");
        assert!(matches!(
            record.operations[0].status,
            OperationStatus::Completed
        ));
        let before = copies(&store, &sid);
        file.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(
            generated_video(&store, &sid, &mid, None, &mut file, media.clone(), |_| Err(
                "assistant_identity_changed".into()
            ))
            .unwrap_err(),
            "assistant_identity_changed"
        );
        assert_eq!(copies(&store, &sid), before);
        store
            .mutate(&sid, |record| {
                record
                    .attachments
                    .push(existing(MAX_SESSION_BYTES - attachment.size_bytes));
                Ok(())
            })
            .unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(
            generated_video(&store, &sid, &mid, None, &mut file, media.clone(), |_| Ok(
                ()
            ))
            .unwrap_err(),
            TOTAL_TOO_LARGE
        );
        assert_eq!(copies(&store, &sid), before);
        file.set_len(super::super::video::MAX_VIDEO_BYTES + 1)
            .unwrap();
        assert_eq!(
            generated_video(&store, &sid, &mid, None, &mut file, media, |_| Ok(())).unwrap_err(),
            TOO_LARGE
        );
    }
    #[test]
    fn generated_file_checks_the_bytes_and_keeps_the_record_and_details() {
        use std::io::Write;
        const PPTX: &str =
            "application/vnd.openxmlformats-officedocument.presentationml.presentation";
        let (_config, store, sid) = fixture();
        let mid = id();
        let call = id();
        store
            .mutate(&sid, |record| {
                record.messages.push(serde_json::from_value(json!({"id":mid,"role":"assistant","content":"","createdAt":1,"status":"streaming"})).unwrap());
                Ok(())
            })
            .unwrap();
        let media = MediaRecord {
            tool_call_id: Some(call.clone()),
            kind: "document".into(),
            params: json!({"name": "deck.pptx", "format": "pptx", "slides": 3, "issues": []}),
            status: "completed".into(),
            cost: Some("0".into()),
            ..Default::default()
        };
        let with = |bytes: &[u8]| {
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(bytes).unwrap();
            file
        };
        // A file that is not what its type says is refused, and so is one
        // over the size limit.
        for (bytes, name) in [
            (&b"not a zip"[..], "x.pptx"),
            (b"PK\x03", "x.pptx"),
            (b"PK\x03\x04", "x.pdf"),
            (b"PK\x03\x04", "x"),
            (b"PK\x03\x04", "x.exe"),
        ] {
            assert_eq!(
                generated_file(
                    &store,
                    &sid,
                    &mid,
                    &id(),
                    &mut with(bytes),
                    name,
                    media.clone(),
                    |_| Ok(())
                )
                .unwrap_err(),
                UNSUPPORTED
            );
        }
        let mut large = with(b"%PDF-1.7");
        large.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert_eq!(
            generated_file(
                &store,
                &sid,
                &mid,
                &id(),
                &mut large,
                "x.pdf",
                media.clone(),
                |_| Ok(())
            )
            .unwrap_err(),
            TOO_LARGE
        );
        // Refused under the lock: nothing is kept.
        assert_eq!(
            generated_file(
                &store,
                &sid,
                &mid,
                &id(),
                &mut with(b"PK\x03\x04 deck"),
                "deck.pptx",
                media.clone(),
                |_| { Err("assistant_identity_changed".into()) }
            )
            .unwrap_err(),
            "assistant_identity_changed"
        );
        assert!(copies(&store, &sid).is_empty());
        // The id names a file: only one `id()` makes.
        for odd in ["../x", "NOT-AN-ID"] {
            assert_eq!(
                generated_file(
                    &store,
                    &sid,
                    &mid,
                    odd,
                    &mut with(b"PK\x03\x04 deck"),
                    "deck.pptx",
                    media.clone(),
                    |_| Ok(())
                )
                .unwrap_err(),
                "assistant_invalid_identifier"
            );
        }

        let chosen = id();
        let attachment = generated_file(
            &store,
            &sid,
            &mid,
            &chosen,
            &mut with(b"PK\x03\x04 deck"),
            "deck.pptx",
            media.clone(),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(attachment.id, chosen);
        assert_eq!(attachment.name, "deck.pptx");
        assert_eq!(attachment.mime_type, PPTX);
        assert_eq!(attachment.size_bytes, 9);
        assert_eq!(attachment.origin.as_deref(), Some("generated"));
        assert_eq!(attachment.source_message_id.as_deref(), Some(mid.as_str()));
        assert_eq!(
            attachment.details,
            Some(json!({"format": "pptx", "slides": 3, "issues": []}))
        );
        let path = store.attachment_root(&sid).unwrap().join(&attachment.id);
        assert_eq!(fs::read(&path).unwrap(), b"PK\x03\x04 deck");
        #[cfg(unix)]
        assert_eq!(crate::config::file_mode(&path), Some(0o600));
        let record = store.load_record(&sid).unwrap();
        let ids = |items: &[Attachment]| items.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&record.attachments), vec![attachment.id.clone()]);
        let reply = &record.messages[0];
        assert_eq!(ids(&reply.attachments), vec![attachment.id.clone()]);
        let saved = reply.media_for(Some(&call)).unwrap();
        assert_eq!(saved.outputs, vec![attachment.id.clone()]);
        assert_eq!(saved.kind, "document");
        // Text has no signature to check.
        let notes = generated_file(
            &store,
            &sid,
            &mid,
            &id(),
            &mut with(b"# Notes"),
            "notes.md",
            MediaRecord {
                tool_call_id: Some(id()),
                ..media
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(notes.mime_type, "text/markdown");
    }
    #[test]
    fn pages_are_kept_beside_the_attachment_and_removed_with_it() {
        use std::io::Write;
        let (_config, store, sid) = fixture();
        let mid = id();
        store
            .mutate(&sid, |record| {
                record.messages.push(serde_json::from_value(json!({"id":mid,"role":"assistant","content":"","createdAt":1,"status":"completed"})).unwrap());
                Ok(())
            })
            .unwrap();
        let media = MediaRecord {
            tool_call_id: Some(id()),
            kind: "document".into(),
            params: json!({"name": "deck.pptx", "format": "pptx", "slides": 3, "svgPages": 2}),
            status: "completed".into(),
            ..Default::default()
        };
        let mut deck = tempfile::tempfile().unwrap();
        deck.write_all(b"PK\x03\x04 deck").unwrap();
        let attachment = generated_file(
            &store,
            &sid,
            &mid,
            &id(),
            &mut deck,
            "deck.pptx",
            media,
            |_| Ok(()),
        )
        .unwrap();
        let made = tempfile::tempdir().unwrap();
        let pages = [
            file(made.path(), "1.svg", b"<svg>1</svg>"),
            file(made.path(), "3.svg", b"<svg>3</svg>"),
        ];
        assert_eq!(keep_pages(&store, &sid, &attachment.id, &pages).unwrap(), 2);
        let kept = store
            .attachment_root(&sid)
            .unwrap()
            .join(format!("{}.pages", attachment.id));
        assert_eq!(fs::read(kept.join("1.svg")).unwrap(), b"<svg>1</svg>");
        assert_eq!(fs::read(kept.join("3.svg")).unwrap(), b"<svg>3</svg>");
        #[cfg(unix)]
        {
            assert_eq!(crate::config::file_mode(&kept), Some(0o700));
            assert_eq!(crate::config::file_mode(&kept.join("1.svg")), Some(0o600));
        }
        // A page that is not a regular file (a link, a pipe) is refused, and
        // nothing is kept.
        #[cfg(unix)]
        for odd in ["link.svg", "pipe.svg"] {
            let path = made.path().join(odd);
            if odd == "link.svg" {
                std::os::unix::fs::symlink(&pages[0], &path).unwrap();
            } else {
                assert!(std::process::Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success());
            }
            let other = id();
            assert_eq!(
                keep_pages(&store, &sid, &other, &[pages[1].clone(), path]).unwrap_err(),
                UNREADABLE
            );
            let refused = store
                .attachment_root(&sid)
                .unwrap()
                .join(format!("{other}.pages"));
            assert!(!refused.exists(), "{odd}");
        }
        // Kept while its attachment is.
        store.cleanup_orphan_attachments(&sid).unwrap();
        assert!(kept.join("1.svg").is_file());
        // The reply goes, its attachment with it, and the pages too.
        store
            .mutate(&sid, |record| {
                record.messages.clear();
                Ok(())
            })
            .unwrap();
        store.cleanup_orphan_attachments(&sid).unwrap();
        assert!(copies(&store, &sid).is_empty());
        assert!(!kept.exists());
        // An unknown folder is still left alone.
        let unknown = store.attachment_root(&sid).unwrap().join("notes.pages");
        fs::create_dir(&unknown).unwrap();
        store.cleanup_orphan_attachments(&sid).unwrap();
        assert!(unknown.is_dir());
    }

    #[test]
    fn speech_publication_rolls_back_failed_metadata_and_retains_committed_ledger_files() {
        let (_root, store, sid) = fixture();
        let bytes = super::super::speech::tests::wav(1);
        let info = super::super::speech::inspect("wav", &bytes).unwrap();
        let mut media:MediaRecord=serde_json::from_value(json!({"kind":"speech","model":"google/gemini-3.8-flash-tts","params":{},"status":"completed","audio":{"pricing":{"unit":"tokens"},"text":"Hello","output":info}})).unwrap();
        let result: Result<(), String> =
            generated_audio(&store, &sid, None, &mut media, &bytes, &info, |_, _, _| {
                Err("assistant_identity_changed".into())
            });
        assert_eq!(result.unwrap_err(), "assistant_identity_changed");
        assert!(copies(&store, &sid).is_empty());
        assert!(store.load_record(&sid).unwrap().attachments.is_empty());
        assert!(media.outputs.is_empty());
        let oid = id();
        let rid = id();
        generated_audio(
            &store,
            &sid,
            None,
            &mut media,
            &bytes,
            &info,
            |record, media, _| {
                record.operations.push(Operation {
                    id: oid.clone(),
                    run_id: rid.clone(),
                    status: OperationStatus::Completed,
                    search_price: None,
                    media: Some(media.clone()),
                });
                Ok(())
            },
        )
        .unwrap();
        let target = store.attachment_root(&sid).unwrap().join(&media.outputs[0]);
        assert_eq!(fs::read(&target).unwrap(), bytes);
        store.cleanup_orphan_attachments(&sid).unwrap();
        assert!(target.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("audio.zip");
        assert_eq!(store.backup(&archive, &[], || Ok(())).unwrap().sessions, 1);
        let fresh = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(fresh.path(), &"ef".repeat(32)).unwrap();
        assert_eq!(
            restored.restore(&archive, &[], || Ok(())).unwrap().imported,
            1
        );
        let loaded = restored.load_record(&sid).unwrap();
        assert!(loaded.operations[0].media.as_ref().unwrap().cost.is_none());
        assert_eq!(
            fs::read(
                restored
                    .attachment_root(&sid)
                    .unwrap()
                    .join(&media.outputs[0])
            )
            .unwrap(),
            bytes
        );
    }
    #[test]
    fn original_audio_publication_rolls_back_and_round_trips_with_speech_and_usage() {
        let (_root, store, sid) = fixture();
        let input = super::super::audio_input::Input::from_bytes(
            "original.wav",
            super::super::speech::tests::wav(1),
        )
        .unwrap();
        let mut asr:MediaRecord=serde_json::from_value(json!({"kind":"transcription","model":"openai/gpt-transcribe","apex":"ofox.io","submittedAt":1,"params":{"inputName":"original.wav","keepOriginal":true},"status":"completed","cost":"0.000075","audio":{"input":input.info,"pricing":{"unit":"seconds","inputAudioPerSecond":"0.000075"},"text":"Saved transcript","usage":{"type":"duration","seconds":1},"requestId":"req_original"}})).unwrap();
        let failed: Result<(), String> = retained_audio(&store, &sid, &mut asr, &input, |_, _| {
            Err("assistant_identity_changed".into())
        });
        assert_eq!(failed.unwrap_err(), "assistant_identity_changed");
        assert!(asr.input_ids.is_empty());
        assert!(copies(&store, &sid).is_empty());
        retained_audio(&store, &sid, &mut asr, &input, |record, media| {
            record.operations.push(Operation {
                id: id(),
                run_id: id(),
                status: OperationStatus::Completed,
                search_price: None,
                media: Some(media.clone()),
            });
            Ok(())
        })
        .unwrap();
        let output = super::super::speech::tests::wav(2);
        let info = super::super::speech::inspect("wav", &output).unwrap();
        let mut speech:MediaRecord=serde_json::from_value(json!({"kind":"speech","model":"google/gemini-3.8-flash-tts","apex":"ofox.io","params":{"voice":"Kore","responseFormat":"wav"},"status":"completed","audio":{"pricing":{"unit":"tokens"},"text":"Saved speech","output":info}})).unwrap();
        generated_audio(
            &store,
            &sid,
            None,
            &mut speech,
            &output,
            &info,
            |record, media, _| {
                record.operations.push(Operation {
                    id: id(),
                    run_id: id(),
                    status: OperationStatus::Completed,
                    search_price: None,
                    media: Some(media.clone()),
                });
                super::super::projection::recount_usage(record, &[]);
                Ok(())
            },
        )
        .unwrap();
        store.cleanup_orphan_attachments(&sid).unwrap();
        assert_eq!(copies(&store, &sid).len(), 2);
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("audio-original.zip");
        assert_eq!(store.backup(&archive, &[], || Ok(())).unwrap().sessions, 1);
        let fresh = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(fresh.path(), &"ef".repeat(32)).unwrap();
        assert_eq!(
            restored.restore(&archive, &[], || Ok(())).unwrap().imported,
            1
        );
        let loaded = restored.load_record(&sid).unwrap();
        assert!(loaded.messages.is_empty());
        assert_eq!(loaded.operations.len(), 2);
        assert_eq!(loaded.usage["modelRequests"], 2);
        assert!(loaded.usage["estimatedCostUsd"].is_null());
        assert_eq!(loaded.operations[0].media.as_ref().unwrap(), &asr);
        assert_eq!(loaded.operations[1].media.as_ref().unwrap(), &speech);
        let root = restored.attachment_root(&sid).unwrap();
        assert_eq!(fs::read(root.join(&asr.input_ids[0])).unwrap(), input.bytes);
        assert_eq!(fs::read(root.join(&speech.outputs[0])).unwrap(), output);
        assert_eq!(
            restored.restore(&archive, &[], || Ok(())).unwrap().skipped,
            1
        );
    }
}
