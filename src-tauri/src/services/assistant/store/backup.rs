//! Portable archives contain conversation records, native history, referenced
//! attachments and preferences. A backup leaves damaged conversations out and
//! counts them. Restore adds the conversations that are missing, each
//! published atomically; existing ones are compared and never overwritten.
mod native;
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom, Write};

const INVALID: &str = "assistant_backup_invalid";
const TOO_LARGE: &str = "assistant_backup_too_large";
const CONFLICT: &str = "assistant_backup_conflict";
const FAILED: &str = "assistant_backup_failed";
const CONTAINS_KEY: &str = "assistant_backup_contains_key";
/// A reply is running for this owner: a backup or restore waits until it ends.
pub const TRANSFER_BUSY: &str = "assistant_transfer_busy";
const MAX_ZIP: u64 = 512 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 20_000;
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;
const MAX_RECORD: u64 = 64 * 1024 * 1024;
const MAX_NATIVE: u64 = 256 * 1024 * 1024;
const MAX_ATTACHMENT: u64 = 50 * 1024 * 1024;
const EXCLUDED_VIDEO: &str = "assistant_attachment_not_in_backup";
const MANIFEST: &str = "manifest.json";

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupReport {
    /// Conversations written to the archive.
    pub sessions: usize,
    /// Damaged conversations left out: an unreadable or invalid record, a
    /// missing native history or attachment file, a file over its limit.
    pub skipped: usize,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    /// Conversations that were missing and are now installed.
    pub imported: usize,
    /// Already present with the same content.
    pub skipped: usize,
    /// Present with other content, or replying: kept as they are.
    pub conflicts: usize,
    /// Some sessions may already have been published. They are valid and kept;
    /// retrying the same archive skips them rather than deleting user work.
    pub warning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Fingerprint {
    size: u64,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    version: u32,
    native_version: u32,
    owner: String,
    sessions: Vec<String>,
    files: BTreeMap<String, Fingerprint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    excluded_videos: Vec<String>,
}

/// One conversation folder as a backup finds it.
enum Found {
    /// No content: a creation that never wrote its record, or one removed
    /// meanwhile. A listing does not show it either.
    Empty,
    /// Left out of the archive and counted.
    Damaged,
    /// Checked: its record's bytes and the member names it consists of.
    Ready {
        record: Vec<u8>,
        names: BTreeSet<String>,
    },
}

/// A conversation of the archive against the local one with its id.
enum Local {
    Missing,
    Same,
    /// Other content, unreadable or replying: never replaced.
    Different,
}

enum Placed {
    Imported,
    Skipped,
    Conflict,
}

fn digest(bytes: &[u8]) -> Fingerprint {
    Fingerprint {
        size: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

/// A strict whitelist makes platform path separators, dot segments, aliases,
/// absolute names and unexpected runtime/credential files all invalid.
fn limit(name: &str) -> Result<u64, String> {
    match name.split('/').collect::<Vec<_>>().as_slice() {
        [MANIFEST] => Ok(MAX_MANIFEST),
        [PREFS_FILE] => Ok(1024 * 1024),
        [sid, SESSION_FILE] if validate_id(sid).is_ok() => Ok(MAX_RECORD),
        [sid, NATIVE_HISTORY_FILE] if validate_id(sid).is_ok() => Ok(MAX_NATIVE),
        [sid, ATTACHMENTS_DIR, aid] if validate_id(sid).is_ok() && validate_id(aid).is_ok() => {
            Ok(super::super::video::MAX_VIDEO_BYTES)
        }
        _ => Err(INVALID.into()),
    }
}

fn read_bounded(reader: impl Read, max: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| INVALID)?;
    if bytes.len() as u64 > max {
        return Err(TOO_LARGE.into());
    }
    Ok(bytes)
}

fn read_file(path: &Path, max: u64) -> Result<Option<Vec<u8>>, String> {
    ensure_regular_or_absent(path)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(FAILED.into()),
    };
    if file.metadata().map_err(|_| FAILED)?.len() > max {
        return Err(TOO_LARGE.into());
    }
    read_bounded(file, max).map(Some)
}

/// A member's bytes, exactly as long as the central directory declares:
/// the `zip` crate inflates a member past a smaller declared size.
fn read_member(
    zip: &mut zip::ZipArchive<fs::File>,
    name: &str,
    max: u64,
) -> Result<Vec<u8>, String> {
    let entry = zip.by_name(name).map_err(|_| INVALID)?;
    let declared = entry.size();
    let bytes = read_bounded(entry, max)?;
    if bytes.len() as u64 != declared {
        return Err(INVALID.into());
    }
    Ok(bytes)
}

fn little_endian(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .rev()
        .fold(0, |value, &byte| value << 8 | u64::from(byte))
}

fn read_at(reader: &mut (impl Read + Seek), at: u64, len: usize) -> Result<Vec<u8>, String> {
    reader.seek(SeekFrom::Start(at)).map_err(|_| INVALID)?;
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes).map_err(|_| INVALID)?;
    Ok(bytes)
}

/// The member count of an archive whose layout the `zip` crate is certain to
/// read as it is checked here, before the crate parses anything.
///
/// The crate searches end records backwards and silently falls back to an
/// earlier one when the last fails its checks or its directory does not
/// parse: a fake record at the end would hide a huge directory before it.
/// So only the last end record is accepted, with no trailing bytes, one disk
/// and archive offset 0, and only when every check the crate makes holds:
/// the ZIP64 locator and record exactly where they point (when the classic
/// record defers to them), a central directory that ends exactly at the end
/// record and holds exactly the declared headers, each without extra fields
/// (nothing the crate could fail to parse), stored or deflated, pointing at
/// a local header before the directory. Our writer produces exactly this.
fn checked_entries(reader: &mut (impl Read + Seek)) -> Result<u64, String> {
    const END: usize = 22;
    const LOCATOR: usize = 20;
    const RECORD64: usize = 56;
    const HEADER: usize = 46;
    const LOCAL: usize = 30;
    /// Room for 20,000 of our member names; ours stay far below it.
    const MAX_DIRECTORY: u64 = MAX_FILES as u64 * 512;
    let len = reader.seek(SeekFrom::End(0)).map_err(|_| INVALID)?;
    let tail_len = len.min((END + LOCATOR + usize::from(u16::MAX)) as u64);
    let tail = read_at(reader, len - tail_len, tail_len as usize)?;
    let at = tail
        .windows(4)
        .rposition(|part| part == b"PK\x05\x06")
        .filter(|&at| at + END <= tail.len())
        .ok_or(INVALID)?;
    let field = |from: usize, size: usize| little_endian(&tail[at + from..at + from + size]);
    let end = len - tail_len + at as u64;
    if at + END + field(20, 2) as usize != tail.len() || field(4, 2) != 0 || field(6, 2) != 0 {
        return Err(INVALID.into());
    }
    let (on_disk, total) = (field(8, 2), field(10, 2));
    let (count, directory, directory_end) = if total == u64::from(u16::MAX)
        || field(16, 4) == u64::from(u32::MAX)
    {
        // The crate then reads only the ZIP64 record.
        let locator = at.checked_sub(LOCATOR).ok_or(INVALID)?;
        let locator_field =
            |from: usize, size: usize| little_endian(&tail[locator + from..locator + from + size]);
        let locator_at = end - LOCATOR as u64;
        let record_at = locator_field(8, 8);
        if tail[locator..locator + 4] != *b"PK\x06\x07"
            || locator_field(4, 4) != 0
            || locator_field(16, 4) > 1
            || record_at
                .checked_add(RECORD64 as u64)
                .is_none_or(|end| end > locator_at)
        {
            return Err(INVALID.into());
        }
        let record = read_at(reader, record_at, RECORD64)?;
        let field = |from: usize, size: usize| little_endian(&record[from..from + size]);
        let count = field(32, 8);
        if record[..4] != *b"PK\x06\x06"
            || field(4, 8) < 44
            || field(4, 8).checked_add(12) != Some(locator_at - record_at)
            || field(14, 2) > field(12, 2)
            || field(16, 4) != 0
            || field(20, 4) != 0
            || field(24, 8) != count
            || on_disk != total
            || (total != u64::from(u16::MAX) && total != count)
        {
            return Err(INVALID.into());
        }
        (count, (field(48, 8), field(40, 8)), record_at)
    } else if on_disk == total {
        (total, (field(16, 4), field(12, 4)), end)
    } else {
        return Err(INVALID.into());
    };
    if count > MAX_FILES as u64 {
        return Err(TOO_LARGE.into());
    }
    let (start, size) = directory;
    if start.checked_add(size) != Some(directory_end) {
        return Err(INVALID.into());
    }
    if size > MAX_DIRECTORY {
        return Err(TOO_LARGE.into());
    }
    let headers = read_at(reader, start, size as usize)?;
    let (mut at, mut found) = (0, 0u64);
    while at < headers.len() {
        let header = headers.get(at..at + HEADER).ok_or(INVALID)?;
        let field = |from: usize, size: usize| little_endian(&header[from..from + size]);
        let local = field(42, 4);
        if header[..4] != *b"PK\x01\x02"
            || !matches!(field(10, 2), 0 | 8)
            || field(30, 2) != 0
            || local
                .checked_add(LOCAL as u64)
                .is_none_or(|end| end > start)
        {
            return Err(INVALID.into());
        }
        let block = read_at(reader, local, LOCAL)?;
        let data =
            local + (LOCAL as u64) + little_endian(&block[26..28]) + little_endian(&block[28..30]);
        if block[..4] != *b"PK\x03\x04" || data > start {
            return Err(INVALID.into());
        }
        at += HEADER + field(28, 2) as usize + field(32, 2) as usize;
        found += 1;
    }
    if at != headers.len() || found != count {
        return Err(INVALID.into());
    }
    Ok(count)
}

fn contains_key(bytes: &[u8], secrets: &[String]) -> bool {
    secrets
        .iter()
        .filter(|key| key.len() >= 8)
        .any(|key| bytes.windows(key.len()).any(|part| part == key.as_bytes()))
}

fn clean_json(bytes: Vec<u8>, secrets: &[String]) -> Result<Vec<u8>, String> {
    let text = String::from_utf8(bytes).map_err(|_| INVALID)?;
    Ok(super::super::host::scrub_secrets(&text, secrets).into_bytes())
}

/// A record as an archive holds it: as saved, or as a load repairs it (a
/// foreign `errorCode` dropped), since the archive must restore strictly.
fn portable_record(bytes: Vec<u8>, sid: &str) -> Result<Vec<u8>, String> {
    if decode_exact(&bytes, sid).is_ok() {
        return Ok(bytes);
    }
    encode_record(&decode(&bytes, sid)?)
}

/// A conversation member as an archive holds it: JSON without the known
/// keys, the record as a load repairs it, native history without local
/// paths, attachments unchanged (one holding a key blocks the backup).
fn archived(name: &str, bytes: Vec<u8>, secrets: &[String]) -> Result<Vec<u8>, String> {
    match name.split('/').collect::<Vec<_>>().as_slice() {
        [sid, SESSION_FILE] => clean_json(portable_record(bytes, sid)?, secrets),
        [_, NATIVE_HISTORY_FILE] => clean_json(native::portable(bytes)?, secrets),
        _ if contains_key(&bytes, secrets) => Err(CONTAINS_KEY.into()),
        _ => Ok(bytes),
    }
}

fn idle(record: &SessionRecord) -> Result<(), String> {
    if record.session.active_run.is_some()
        || record
            .operations
            .iter()
            .any(|op| op.status == OperationStatus::Running)
        || record
            .messages
            .iter()
            .any(|message| message.status == "streaming")
    {
        return Err(TRANSFER_BUSY.into());
    }
    Ok(())
}

fn excluded_video(file: &Attachment) -> bool {
    file.mime_type.starts_with("video/")
        && file.status == "missing"
        && file.reason.as_deref() == Some(EXCLUDED_VIDEO)
}
fn attachment_cap(file: &Attachment) -> u64 {
    if file.mime_type.starts_with("video/") {
        super::super::video::MAX_VIDEO_BYTES
    } else {
        MAX_ATTACHMENT
    }
}

/// Check one conversation's files under `root` against its record: each
/// listed attachment (its size, and the type its messages expect), the
/// draft's files, and the native history's shape and lineage. Returns the
/// member names the conversation consists of.
fn validate_session(
    root: &Path,
    sid: &str,
    record: &SessionRecord,
) -> Result<BTreeSet<String>, String> {
    let mut expected = BTreeSet::from([format!("{sid}/{SESSION_FILE}")]);
    let mut attachments = HashMap::new();
    for file in &record.attachments {
        validate_id(&file.id).map_err(|_| INVALID)?;
        if attachments.insert(file.id.as_str(), file).is_some() {
            return Err(INVALID.into());
        }
        let name = format!("{sid}/{ATTACHMENTS_DIR}/{}", file.id);
        if file.size_bytes > attachment_cap(file) {
            return Err(TOO_LARGE.into());
        }
        if excluded_video(file) {
            if root.join(&name).exists() {
                return Err(INVALID.into());
            }
            continue;
        }
        let bytes = read_file(&root.join(&name), attachment_cap(file))?.ok_or(INVALID)?;
        if bytes.len() as u64 != file.size_bytes {
            return Err(INVALID.into());
        }
        expected.insert(name);
    }
    for aid in &record.session.draft_attachment_ids {
        if !attachments.contains_key(aid.as_str()) {
            return Err(INVALID.into());
        }
    }
    for message in &record.messages {
        for file in &message.attachments {
            if !attachments.get(file.id.as_str()).is_some_and(|saved| {
                saved.size_bytes == file.size_bytes && saved.mime_type == file.mime_type
            }) {
                return Err(INVALID.into());
            }
        }
    }
    let native = format!("{sid}/{NATIVE_HISTORY_FILE}");
    let native_ids = match read_file(&root.join(&native), MAX_NATIVE)? {
        Some(bytes) => {
            expected.insert(native);
            native::validate(&bytes, record)?
        }
        None => HashMap::new(),
    };
    for message in &record.messages {
        if let Some(id) = &message.native_entry_id {
            let entry = native_ids.get(id).ok_or(INVALID)?;
            if entry.role.as_ref() != Some(&message.role)
                || entry.run.as_ref().map(|(run, _)| run) != message.run_id.as_ref()
            {
                return Err(INVALID.into());
            }
        }
    }
    Ok(expected)
}

fn options() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600)
}

fn add_member(
    writer: &mut zip::ZipWriter<fs::File>,
    manifest: &mut Manifest,
    total: &mut u64,
    name: String,
    bytes: Vec<u8>,
    stored: bool,
) -> Result<(), String> {
    if bytes.len() as u64 > limit(&name)?
        || *total + bytes.len() as u64 > MAX_TOTAL
        || manifest.files.len() + 1 >= MAX_FILES
    {
        return Err(TOO_LARGE.into());
    }
    *total += bytes.len() as u64;
    manifest.files.insert(name.clone(), digest(&bytes));
    let opts = if stored {
        options().compression_method(zip::CompressionMethod::Stored)
    } else {
        options()
    };
    writer.start_file(name, opts).map_err(|_| FAILED)?;
    writer.write_all(&bytes).map_err(|_| FAILED.to_string())
}

impl AssistantStore {
    /// Per-session consistent snapshots; damaged conversations are left out
    /// and counted, a changed session inventory aborts the archive. The final
    /// destination is replaced only after the ZIP is complete.
    pub fn backup(
        &self,
        path: &Path,
        secrets: &[String],
        admit: impl Fn() -> Result<(), String>,
    ) -> Result<BackupReport, String> {
        self.backup_with_options(path, secrets, false, admit)
    }
    pub fn backup_with_options(
        &self,
        path: &Path,
        secrets: &[String],
        exclude_videos: bool,
        admit: impl Fn() -> Result<(), String>,
    ) -> Result<BackupReport, String> {
        let lock = file_lock(&self.root.join(".transfer-lock"));
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        admit()?;
        let path = super::super::transfers::destination(path, &self.config_dir)?;
        let mut ids = if real_dir(&self.root)? {
            self.session_ids()?
        } else {
            vec![]
        };
        ids.sort();
        let mut manifest = Manifest {
            version: 1,
            native_version: 3,
            owner: self.owner.clone(),
            sessions: vec![],
            files: BTreeMap::new(),
            excluded_videos: vec![],
        };
        let mut writer = zip::ZipWriter::new(tempfile::tempfile().map_err(|_| FAILED)?);
        let mut total = 0;
        let mut report = BackupReport::default();
        // What a listing shows, archived or damaged; see `listed_ids`.
        let mut listed = vec![];
        for sid in &ids {
            self.with_attachment_lock(sid, || {
                let lock = file_lock(&self.root.join(sid).join(SESSION_FILE));
                let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                admit()?;
                let (record, mut names) = match self.inspect(sid)? {
                    Found::Empty => return Ok(()),
                    Found::Damaged => {
                        listed.push(sid.clone());
                        report.skipped += 1;
                        return Ok(());
                    }
                    Found::Ready { record, names } => (record, names),
                };
                let mut saved = decode(&record, sid)?;
                let media_names: BTreeSet<_> = saved
                    .attachments
                    .iter()
                    .filter(|file| {
                        file.mime_type.starts_with("image/") || file.mime_type.starts_with("video/")
                    })
                    .map(|file| format!("{sid}/{ATTACHMENTS_DIR}/{}", file.id))
                    .collect();
                let mut missing = BTreeSet::new();
                for file in &mut saved.attachments {
                    if (exclude_videos && file.mime_type.starts_with("video/"))
                        || excluded_video(file)
                    {
                        let name = format!("{sid}/{ATTACHMENTS_DIR}/{}", file.id);
                        manifest.excluded_videos.push(name.clone());
                        names.remove(&name);
                        missing.insert(file.id.clone());
                        file.status = "missing".into();
                        file.reason = Some(EXCLUDED_VIDEO.into());
                        file.preview_url = None;
                    }
                }
                for file in saved
                    .messages
                    .iter_mut()
                    .flat_map(|message| &mut message.attachments)
                {
                    if missing.contains(&file.id) {
                        file.status = "missing".into();
                        file.reason = Some(EXCLUDED_VIDEO.into());
                        file.preview_url = None;
                    }
                }
                let record = if missing.is_empty() {
                    record
                } else {
                    encode_record(&saved)?
                };
                let record_name = format!("{sid}/{SESSION_FILE}");
                add_member(
                    &mut writer,
                    &mut manifest,
                    &mut total,
                    record_name.clone(),
                    archived(&record_name, record, secrets)?,
                    false,
                )?;
                // Checked under the same locks just now; read one at a time.
                for name in names.into_iter().filter(|name| *name != record_name) {
                    let bytes = read_file(&self.root.join(&name), limit(&name)?)?.ok_or(INVALID)?;
                    let bytes = archived(&name, bytes, secrets)?;
                    let stored = media_names.contains(&name);
                    add_member(&mut writer, &mut manifest, &mut total, name, bytes, stored)?;
                }
                listed.push(sid.clone());
                manifest.sessions.push(sid.clone());
                report.sessions += 1;
                Ok::<_, String>(())
            })?;
        }
        if let Some(prefs) = read_file(&self.root.join(PREFS_FILE), limit(PREFS_FILE)?)? {
            // Only the known fields: an unknown one may hold anything.
            let (value, _) = Prefs::parse(&prefs).ok_or(INVALID)?;
            add_member(
                &mut writer,
                &mut manifest,
                &mut total,
                PREFS_FILE.into(),
                clean_json(
                    serde_json::to_vec(&value.portable()).map_err(|_| FAILED)?,
                    secrets,
                )?,
                false,
            )?;
        }
        if self.listed_ids()? != listed {
            return Err(CONFLICT.into());
        }
        let bytes = serde_json::to_vec(&manifest).map_err(|_| FAILED)?;
        if bytes.len() as u64 > MAX_MANIFEST || total + bytes.len() as u64 > MAX_TOTAL {
            return Err(TOO_LARGE.into());
        }
        writer.start_file(MANIFEST, options()).map_err(|_| FAILED)?;
        writer.write_all(&bytes).map_err(|_| FAILED)?;
        let mut zip = writer.finish().map_err(|_| FAILED)?;
        if zip.metadata().map_err(|_| FAILED)?.len() > MAX_ZIP {
            return Err(TOO_LARGE.into());
        }
        zip.seek(SeekFrom::Start(0)).map_err(|_| FAILED)?;
        admit()?;
        let path = super::super::transfers::destination(&path, &self.config_dir)?;
        crate::config::atomic_copy_with(
            &path,
            &mut zip,
            crate::config::AtomicOpts { mode: Some(0o600) },
        )
        .map_err(|_| FAILED)?;
        Ok(report)
    }

    /// One conversation for a backup, under its attachment and record locks.
    /// Only a live reply is an error; any other problem makes it damaged.
    fn inspect(&self, sid: &str) -> Result<Found, String> {
        let dir = self.root.join(sid);
        let checked = (|| {
            if !real_dir(&dir)? {
                return Ok(None);
            }
            let Some(record) = read_file(&dir.join(SESSION_FILE), MAX_RECORD)? else {
                // As in a listing: files without a record are a damaged
                // conversation, an empty folder is none.
                return if has_content(&dir)? {
                    Err(INVALID.to_string())
                } else {
                    Ok(None)
                };
            };
            // As a load reads it; the archive gets the repaired record.
            let saved = decode(&record, sid)?;
            idle(&saved)?;
            let names = validate_session(&self.root, sid, &saved)?;
            Ok(Some((record, names)))
        })();
        match checked {
            Ok(Some((record, names))) => Ok(Found::Ready { record, names }),
            Ok(None) => Ok(Found::Empty),
            Err(error) if error == TRANSFER_BUSY => Err(error),
            Err(error) => {
                log::warn!("[Assistant] the backup leaves out damaged conversation {sid}: {error}");
                Ok(Found::Damaged)
            }
        }
    }

    /// Conversation folders a listing shows, readable or not, sorted. The
    /// empty folder of an unfinished creation is not one.
    fn listed_ids(&self) -> Result<Vec<String>, String> {
        if !real_dir(&self.root)? {
            return Ok(vec![]);
        }
        let mut ids = vec![];
        for sid in self.session_ids()? {
            let dir = self.root.join(&sid);
            let shown = match real_dir(&dir) {
                Ok(true) => has_content(&dir)?,
                Ok(false) => false,
                Err(_) => true,
            };
            if shown {
                ids.push(sid);
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Validate the complete archive, then add the missing sessions. Never
    /// replace a local conversation or its preferences: one that differs or
    /// is replying is counted as a conflict. Publication failure after some
    /// imports returns their counts and a warning, so retries stay idempotent.
    pub fn restore(
        &self,
        path: &Path,
        secrets: &[String],
        admit: impl Fn() -> Result<(), String>,
    ) -> Result<RestoreReport, String> {
        let lock = file_lock(&self.root.join(".transfer-lock"));
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        admit()?;
        ensure_regular_or_absent(path)?;
        let mut file = fs::File::open(path).map_err(|_| INVALID)?;
        if file.metadata().map_err(|_| INVALID)?.len() > MAX_ZIP {
            return Err(TOO_LARGE.into());
        }
        let count = checked_entries(&mut file)?;
        let mut zip = crate::zip_security::open(file, MAX_FILES).map_err(|_| INVALID)?;
        if zip.len() as u64 != count {
            return Err(INVALID.into());
        }
        crate::zip_security::validate_headers(&mut zip).map_err(|_| INVALID)?;
        let mut names = HashSet::new();
        let mut declared = 0u64;
        for index in 0..zip.len() {
            let entry = zip.by_index(index).map_err(|_| INVALID)?;
            let name = entry.name().map_err(|_| INVALID)?;
            let max = limit(&name)?;
            if !entry.is_file()
                || entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err(INVALID.into());
            }
            // Unique: the crate keeps one entry per name and the count matched.
            names.insert(name.into_owned());
            declared = declared.checked_add(entry.size()).ok_or(TOO_LARGE)?;
            if entry.size() > max || declared > MAX_TOTAL {
                return Err(TOO_LARGE.into());
            }
        }
        let manifest_bytes = read_member(&mut zip, MANIFEST, MAX_MANIFEST)?;
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|_| INVALID)?;
        if manifest.version != 1 || manifest.native_version != 3 {
            return Err(INVALID.into());
        }
        if manifest.owner != self.owner {
            return Err("assistant_backup_owner_mismatch".into());
        }
        let sessions: HashSet<_> = manifest.sessions.iter().cloned().collect();
        if sessions.len() != manifest.sessions.len()
            || manifest
                .sessions
                .iter()
                .any(|sid| validate_id(sid).is_err())
        {
            return Err(INVALID.into());
        }
        names.remove(MANIFEST);
        if names != manifest.files.keys().cloned().collect() {
            return Err(INVALID.into());
        }
        for name in &names {
            limit(name)?;
            if name != PREFS_FILE && !sessions.contains(name.split('/').next().ok_or(INVALID)?) {
                return Err(INVALID.into());
            }
        }
        let excluded: BTreeSet<_> = manifest.excluded_videos.iter().cloned().collect();
        if excluded.len() != manifest.excluded_videos.len() {
            return Err(INVALID.into());
        }
        for name in &excluded {
            let parts: Vec<_> = name.split('/').collect();
            if parts.len() != 3
                || parts[1] != ATTACHMENTS_DIR
                || !sessions.contains(parts[0])
                || validate_id(parts[2]).is_err()
                || names.contains(name)
            {
                return Err(INVALID.into());
            }
        }
        let mut found_excluded = BTreeSet::new();
        let root = self.ensure_root()?;
        let staging = tempfile::Builder::new()
            .prefix(&format!(".restore-{}-", id()))
            .tempdir_in(&root)
            .map_err(|_| FAILED)?;
        let mut total = manifest_bytes.len() as u64;
        for (name, fingerprint) in &manifest.files {
            admit()?;
            let bytes = read_member(&mut zip, name, limit(name)?)?;
            total = total.checked_add(bytes.len() as u64).ok_or(TOO_LARGE)?;
            if total > MAX_TOTAL {
                return Err(TOO_LARGE.into());
            }
            if digest(&bytes) != *fingerprint {
                return Err(INVALID.into());
            }
            if contains_key(&bytes, secrets) {
                return Err(CONTAINS_KEY.into());
            }
            let target = staging.path().join(name);
            crate::config::create_private_dir_all(target.parent().ok_or(INVALID)?)
                .map_err(|_| FAILED)?;
            write_bytes(&target, &bytes)?;
        }
        for sid in &manifest.sessions {
            let record = read_file(&staging.path().join(sid).join(SESSION_FILE), MAX_RECORD)?
                .ok_or(INVALID)?;
            let record = decode_exact(&record, sid).map_err(|_| INVALID)?;
            for file in record
                .attachments
                .iter()
                .filter(|file| excluded_video(file))
            {
                found_excluded.insert(format!("{sid}/{ATTACHMENTS_DIR}/{}", file.id));
            }
            // A backup never holds a conversation that was replying.
            idle(&record).map_err(|_| INVALID)?;
            let prefix = format!("{sid}/");
            let members: BTreeSet<_> = names
                .iter()
                .filter(|name| name.starts_with(&prefix))
                .cloned()
                .collect();
            if validate_session(staging.path(), sid, &record)? != members {
                return Err(INVALID.into());
            }
        }
        if found_excluded != excluded {
            return Err(INVALID.into());
        }
        let archived_prefs = if names.contains(PREFS_FILE) {
            let bytes =
                read_file(&staging.path().join(PREFS_FILE), limit(PREFS_FILE)?)?.ok_or(INVALID)?;
            Some(Prefs::parse(&bytes).ok_or(INVALID)?.0)
        } else {
            None
        };
        let mut report = RestoreReport::default();
        for sid in &manifest.sessions {
            let result = self.with_attachment_lock(sid, || {
                let lock = file_lock(&root.join(sid).join(SESSION_FILE));
                let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                admit()?;
                match self.local_copy(sid, &manifest, secrets) {
                    Local::Same => return Ok(Placed::Skipped),
                    Local::Different => return Ok(Placed::Conflict),
                    Local::Missing => {}
                }
                // A deletion of this id that never finished must not outlive
                // the restore: the conversation could not be deleted again.
                if let Err(error) = self.finish_pending_delete(sid) {
                    log::warn!("[Assistant] restore keeps out {sid}: its old deletion is pending ({error})");
                    return Ok(Placed::Conflict);
                }
                crate::config::rename_dir(&staging.path().join(sid), &root.join(sid))
                    .map_err(|_| FAILED)?;
                Ok(Placed::Imported)
            });
            match result {
                Ok(Placed::Imported) => report.imported += 1,
                Ok(Placed::Skipped) => report.skipped += 1,
                Ok(Placed::Conflict) => report.conflicts += 1,
                Err(error) if report.imported > 0 => {
                    report.warning = Some(error);
                    return Ok(report);
                }
                Err(error) => return Err(error),
            }
        }
        // Existing preferences win field by field, and an unreadable file wins
        // whole: the archive only fills in what this profile never saved (an
        // older archive has no settings, so the current ones stay). A failed
        // optional preference import does not turn successful session imports
        // into failure.
        if let Some(archived) = archived_prefs {
            let result = (|| {
                let path = root.join(PREFS_FILE);
                let lock = file_lock(&path);
                let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                admit()?;
                ensure_regular_or_absent(&path)?;
                let (mut local, object) = match read_regular(&path)? {
                    None => Default::default(),
                    Some(bytes) => match Prefs::parse(&bytes) {
                        Some(found) => found,
                        None => return Ok(()),
                    },
                };
                if local.fill_from(&archived) {
                    local.version = FORMAT_VERSION;
                    write_bytes(&path, &local.to_bytes(object)?)?;
                }
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                report.warning = Some(error);
            }
        }
        Ok(report)
    }

    pub fn cleanup_restore_staging(&self) -> Result<(), String> {
        let lock = file_lock(&self.root.join(".transfer-lock"));
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        if !real_dir(&self.root)? {
            return Ok(());
        }
        let mut failure = None;
        for entry in fs::read_dir(&self.root).map_err(|_| FAILED)? {
            let entry = entry.map_err(|_| FAILED)?;
            let name = entry.file_name();
            let Some(tail) = name
                .to_str()
                .and_then(|name| name.strip_prefix(".restore-"))
            else {
                continue;
            };
            if tail.len() != 43
                || !tail.is_ascii()
                || validate_id(&tail[..36]).is_err()
                || &tail[36..37] != "-"
                || !tail[37..].bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                continue;
            }
            let result = real_dir(&entry.path()).and_then(|exists| {
                if exists {
                    fs::remove_dir_all(entry.path()).map_err(|_| FAILED.to_string())
                } else {
                    Ok(())
                }
            });
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    /// The local conversation `sid` against the archive's copy, compared as a
    /// backup would archive it. Anything that keeps them from comparing equal
    /// makes it different.
    fn local_copy(&self, sid: &str, manifest: &Manifest, secrets: &[String]) -> Local {
        match real_dir(&self.root.join(sid)) {
            Ok(false) => return Local::Missing,
            Ok(true) => {}
            Err(_) => return Local::Different,
        }
        let same = (|| {
            idle(&read_record(&self.root, sid)?)?;
            let prefix = format!("{sid}/");
            for (name, expected) in manifest
                .files
                .iter()
                .filter(|(name, _)| name.starts_with(&prefix))
            {
                let Some(bytes) = read_file(&self.root.join(name), limit(name)?)? else {
                    return Ok(false);
                };
                // Archives made before blanking and repair hold the raw bytes.
                if digest(&bytes) != *expected
                    && digest(&archived(name, bytes, secrets)?) != *expected
                {
                    return Ok(false);
                }
            }
            let native = format!("{sid}/{NATIVE_HISTORY_FILE}");
            Ok::<_, String>(
                manifest.files.contains_key(&native)
                    || read_file(&self.root.join(native), MAX_NATIVE)?.is_none(),
            )
        })();
        if matches!(same, Ok(true)) {
            Local::Same
        } else {
            Local::Different
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn fixture() -> (tempfile::TempDir, AssistantStore, String) {
        let tmp = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(tmp.path(), &"a".repeat(64)).unwrap();
        let sid = store.create("test/model", true).unwrap().session.id;
        (tmp, store, sid)
    }

    fn seeded() -> (tempfile::TempDir, AssistantStore, String, String) {
        let (tmp, store, sid) = fixture();
        let attachment = id();
        fs::write(
            store.attachment_dir(&sid).unwrap().join(&attachment),
            "中文附件",
        )
        .unwrap();
        store.mutate(&sid, |record| {
            record.session.draft = "未发送草稿".into();
            record.session.draft_attachment_ids = vec![attachment.clone()];
            record.attachments.push(Attachment { origin: None, source_message_id: None, id:attachment.clone(), name:"中文.txt".into(), mime_type:"text/plain".into(),size_bytes:12,status:"ready".into(),reason:None,preview_url:None,details:None });
            record.usage = json!({"modelRequests":1,"estimatedCostUsd":"0.123456789123456789","searchCount":1});
            record.operations.push(Operation { media: None,
                id: "79346d31-7f3c-48d5-b3ac-81a889060608".into(),
                run_id: "a48cac04-e861-4e76-92e8-6a73896a095c".into(),
                status: OperationStatus::Completed,
                search_price: Some(serde_json::from_value(json!({"model":"test/search","perSearch":"0.014","perInputToken":"0.0000001","perOutputToken":"0.0000002"})).unwrap()),
            });
            record.messages = serde_json::from_value(json!([
                {"id":id(),"role":"user","content":"Read my portable note.","createdAt":1,"status":"completed","nativeEntryId":"8ed73709","runId":"a48cac04-e861-4e76-92e8-6a73896a095c"},
                {"id":id(),"role":"assistant","content":"Your portable note is ready.","createdAt":2,"status":"completed","nativeEntryId":"a005630a","runId":"a48cac04-e861-4e76-92e8-6a73896a095c"}
            ])).unwrap();
            Ok(())
        }).unwrap();
        fs::write(
            store.native_history_path(&sid).unwrap(),
            include_bytes!("../../../../../assistant-runtime/test/fixtures/portable-session.jsonl"),
        )
        .unwrap();
        store.remember_model("test/model").unwrap();
        let runtime = store.root.join(&sid).join("runtime-settings");
        fs::create_dir(&runtime).unwrap();
        fs::write(runtime.join("auth.json"), "must-not-export").unwrap();
        (tmp, store, sid, attachment)
    }

    fn seeded_native_media(kind: &str) -> (tempfile::TempDir, AssistantStore, String) {
        let (tmp, store, sid) = fixture();
        let run = id();
        let operation = id();
        let attachment = id();
        let bytes = b"synthetic media backup fixture";
        fs::write(store.attachment_dir(&sid).unwrap().join(&attachment), bytes).unwrap();
        let mime = if kind == "image" {
            "image/png"
        } else {
            "video/mp4"
        };
        store.mutate(&sid, |record| {
            let media = json!({"kind":kind,"model":"test/media","params":{},"status":"completed","outputs":[attachment],"cost":"0.08"});
            record.attachments = serde_json::from_value(json!([{"id":attachment,"name":"synthetic-media","mimeType":mime,"sizeBytes":bytes.len(),"status":"ready","origin":"generated"}])).unwrap();
            record.operations = serde_json::from_value(json!([{"id":operation,"runId":run,"status":"completed","media":media}])).unwrap();
            record.messages = serde_json::from_value(json!([
                {"id":id(),"role":"user","content":"Create synthetic media","createdAt":1,"status":"completed","nativeEntryId":"media-user","runId":run},
                {"id":id(),"role":"media","content":"Saved media","createdAt":2,"status":"completed","nativeEntryId":"media-result","runId":run,"media":media}
            ])).unwrap();
            Ok(())
        }).unwrap();
        let entries = [
            json!({"type":"session","version":3,"id":sid,"cwd":"","timestamp":"2026-01-01T00:00:00Z"}),
            json!({"type":"custom","id":"media-run","parentId":null,"timestamp":"2026-01-01T00:00:00Z","customType":"ofox-desktop-run","data":{"runId":run,"operationId":operation}}),
            json!({"type":"message","id":"media-user","parentId":"media-run","timestamp":"2026-01-01T00:00:00Z","message":{"role":"user","content":"Create synthetic media","timestamp":1}}),
            json!({"type":"custom_message","id":"media-result","parentId":"media-user","timestamp":"2026-01-01T00:00:00Z","customType":"ofox-desktop-media","content":"Saved media","display":false,"details":{}}),
        ];
        fs::write(
            store.native_history_path(&sid).unwrap(),
            entries
                .iter()
                .map(|v| serde_json::to_string(v).unwrap() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        (tmp, store, sid)
    }

    #[test]
    fn native_media_message_backups_round_trip_without_losing_lineage_or_rebilling() {
        for (kind, exclude_videos) in [("image", false), ("video", false), ("video", true)] {
            let (_tmp, store, sid) = seeded_native_media(kind);
            let out = tempfile::tempdir().unwrap();
            let archive = out.path().join("media.zip");
            let report = store
                .backup_with_options(&archive, &[], exclude_videos, || Ok(()))
                .unwrap();
            assert_eq!((report.sessions, report.skipped), (1, 0), "{kind}");
            let target = tempfile::tempdir().unwrap();
            let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
            assert_eq!(
                restored.restore(&archive, &[], || Ok(())).unwrap().imported,
                1
            );
            let source = store.load_record(&sid).unwrap();
            let copy = restored.load_record(&sid).unwrap();
            assert_eq!(source.operations, copy.operations);
            assert_eq!(
                source.messages[1].native_entry_id,
                copy.messages[1].native_entry_id
            );
            assert_eq!(source.messages[1].media, copy.messages[1].media);
            assert_eq!(copy.messages[1].role, "media");
            assert_eq!(
                copy.attachments[0].status,
                if exclude_videos { "missing" } else { "ready" }
            );
            assert_eq!(
                restored
                    .attachment_dir(&sid)
                    .unwrap()
                    .join(&copy.attachments[0].id)
                    .exists(),
                !exclude_videos
            );
            assert_eq!(
                restored.restore(&archive, &[], || Ok(())).unwrap().skipped,
                1
            );
        }
    }

    #[test]
    fn native_media_backup_still_rejects_forged_roles_kinds_and_missing_run_lineage() {
        for case in [
            "other-custom",
            "missing-media",
            "wrong-kind",
            "wrong-role",
            "assistant-role",
            "no-lineage",
            "wrong-run",
        ] {
            let (_tmp, store, sid) = seeded_native_media("image");
            let path = store.native_history_path(&sid).unwrap();
            let mut lines: Vec<Value> = fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(|s| serde_json::from_str(s).unwrap())
                .collect();
            store
                .mutate(&sid, |record| {
                    match case {
                        "other-custom" => lines[3]["customType"] = json!("other"),
                        "missing-media" => record.messages[1].media = None,
                        "wrong-kind" => {
                            record.messages[1].media.as_mut().unwrap().kind = "speech".into()
                        }
                        "wrong-role" => record.messages[1].role = "user".into(),
                        "assistant-role" => record.messages[1].role = "assistant".into(),
                        "no-lineage" => {
                            lines[3]["parentId"] = Value::Null;
                            record.messages[1].run_id = None;
                        }
                        "wrong-run" => record.messages[1].run_id = Some(id()),
                        _ => unreachable!(),
                    }
                    Ok(())
                })
                .unwrap();
            fs::write(
                &path,
                lines
                    .iter()
                    .map(|v| serde_json::to_string(v).unwrap() + "\n")
                    .collect::<String>(),
            )
            .unwrap();
            let out = tempfile::tempdir().unwrap();
            let report = store
                .backup(&out.path().join("invalid.zip"), &[], || Ok(()))
                .unwrap();
            assert_eq!((report.sessions, report.skipped), (0, 1), "{case}");
        }
    }

    #[test]
    fn backup_excludes_unknown_preferences_and_managed_keys_without_changing_sources() {
        let (_tmp, store, sid, _) = seeded();
        let key = "synthetic-managed-key-for-backup".to_string();
        store
            .mutate(&sid, |record| {
                record.session.draft = format!("User words {key}");
                Ok(())
            })
            .unwrap();
        fs::write(store.root.join(PREFS_FILE),serde_json::to_vec(&json!({"version":1,"modelId":"test/model","instructions":format!("Sign with {key}"),"unknownCredential":"must-not-export-extra"})).unwrap()).unwrap();
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("backup.zip");
        store
            .backup(&path, std::slice::from_ref(&key), || Ok(()))
            .unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
        for i in 0..zip.len() {
            let mut bytes = Vec::new();
            zip.by_index(i).unwrap().read_to_end(&mut bytes).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains(&key));
            assert!(!text.contains("must-not-export-extra"));
        }
        let (_, prefs) = members_of(&path)
            .into_iter()
            .find(|(name, _)| name == PREFS_FILE)
            .unwrap();
        let prefs: Value = serde_json::from_slice(&prefs).unwrap();
        assert_eq!(
            prefs["instructions"],
            format!("Sign with {}", crate::redact::REDACTED)
        );
        assert!(store
            .load_record(&sid)
            .unwrap()
            .session
            .draft
            .contains(&key));
    }

    // ── settings ─────────────────────────────────────────────────────────

    fn chosen() -> Preferences {
        Preferences {
            instructions: "回答用简体中文。".into(),
            auto_title: false,
            notify_on_finish: false,
        }
    }

    fn backed_up(store: &AssistantStore) -> (tempfile::TempDir, PathBuf) {
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        (output, archive)
    }

    fn archived_prefs(archive: &Path) -> Value {
        let (_, bytes) = members_of(archive)
            .into_iter()
            .find(|(name, _)| name == PREFS_FILE)
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// An empty profile of `owner`.
    fn profile(owner: &str) -> (tempfile::TempDir, AssistantStore) {
        let target = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(target.path(), owner).unwrap();
        (target, store)
    }

    #[test]
    fn the_settings_are_backed_up_and_an_empty_profile_receives_them() {
        let (_source, store, _sid, _) = seeded();
        store.save_preferences(&chosen()).unwrap();
        let (_output, archive) = backed_up(&store);
        assert_eq!(
            archived_prefs(&archive),
            json!({
                "version": 1, "modelId": "test/model", "instructions": "回答用简体中文。",
                "autoTitle": false, "notifyOnFinish": false,
            })
        );
        let (_target, restored) = profile(&store.owner);
        restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(restored.preferences().unwrap(), chosen());
        assert_eq!(
            restored.remembered_model().unwrap().as_deref(),
            Some("test/model")
        );
    }

    #[test]
    fn restore_fills_in_only_the_settings_this_profile_never_saved() {
        let (_source, store, _sid, _) = seeded();
        store.save_preferences(&chosen()).unwrap();
        let (_output, archive) = backed_up(&store);
        // Only a model of its own: it keeps the model and takes the settings.
        let (_target, restored) = profile(&store.owner);
        restored.remember_model("local/model").unwrap();
        restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(restored.preferences().unwrap(), chosen());
        assert_eq!(
            restored.remembered_model().unwrap().as_deref(),
            Some("local/model")
        );
        // Saved settings stay, empty instructions included; the model the
        // profile never remembered comes from the archive.
        let mine = Preferences::default();
        let (_target, restored) = profile(&store.owner);
        restored.save_preferences(&mine).unwrap();
        restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(restored.preferences().unwrap(), mine);
        assert_eq!(
            restored.remembered_model().unwrap().as_deref(),
            Some("test/model")
        );
    }

    #[test]
    fn an_archive_from_before_the_settings_keeps_the_current_ones() {
        // `seeded` remembers a model only, as every profile did before.
        let (_source, store, _sid, _) = seeded();
        let (_output, archive) = backed_up(&store);
        assert_eq!(
            archived_prefs(&archive),
            json!({"version": 1, "modelId": "test/model"})
        );
        let (_target, restored) = profile(&store.owner);
        restored.save_preferences(&chosen()).unwrap();
        restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(restored.preferences().unwrap(), chosen());
        // An empty profile: the defaults, and the archive's model.
        let (_target, restored) = profile(&store.owner);
        restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(restored.preferences().unwrap(), Preferences::default());
        assert_eq!(
            restored.remembered_model().unwrap().as_deref(),
            Some("test/model")
        );
        // A damaged file is kept as it is.
        let (_target, restored) = profile(&store.owner);
        restored.remember_model("local/model").unwrap();
        fs::write(restored.root.join(PREFS_FILE), "damaged").unwrap();
        let report = restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(
            fs::read(restored.root.join(PREFS_FILE)).unwrap(),
            b"damaged"
        );
    }

    #[test]
    fn restore_refuses_archived_settings_this_version_cannot_read() {
        let (_source, store, _sid, _) = seeded();
        let (_output, archive) = backed_up(&store);
        for prefs in [
            json!({"version": 1, "instructions": "a".repeat(4001)}),
            json!({"version": 1, "autoTitle": "yes"}),
        ] {
            let mut members = members_of(&archive);
            replace_member(
                &mut members,
                PREFS_FILE,
                serde_json::to_vec(&prefs).unwrap(),
            );
            let edited = archive.with_file_name("edited.zip");
            write_zip(&edited, &members, false);
            assert_refused(&edited, &store.owner, INVALID);
        }
    }

    #[test]
    fn startup_cleanup_removes_only_abandoned_restore_staging_and_keeps_link_targets() {
        let (_tmp, store, _sid) = fixture();
        let abandoned = store.root.join(format!(".restore-{}-abcdef", id()));
        fs::create_dir(&abandoned).unwrap();
        fs::write(abandoned.join("partial"), "copy").unwrap();
        let unknown = store.root.join(".restore-not-ours");
        fs::create_dir(&unknown).unwrap();
        store.cleanup_restore_staging().unwrap();
        assert!(!abandoned.exists());
        assert!(unknown.is_dir());
        #[cfg(unix)]
        {
            let target = tempfile::tempdir().unwrap();
            fs::write(target.path().join("keep"), "keep").unwrap();
            let linked = store.root.join(format!(".restore-{}-abcdef", id()));
            std::os::unix::fs::symlink(target.path(), linked).unwrap();
            assert!(store.cleanup_restore_staging().is_err());
            assert!(target.path().join("keep").is_file());
        }
    }

    #[test]
    fn rejects_inconsistent_zip_headers_before_restoring_sessions() {
        let (_source, store, _sid, _attachment) = seeded();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("inconsistent.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let mut bytes = fs::read(&archive).unwrap();
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        bytes[30] ^= 1;
        fs::write(&archive, bytes).unwrap();

        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        assert_eq!(
            restored.restore(&archive, &[], || Ok(())).unwrap_err(),
            INVALID
        );
        assert!(restored.list(false, "").unwrap().sessions.is_empty());
    }

    #[test]
    fn backup_roundtrip_keeps_native_history_drafts_files_usage_and_operation_prices() {
        let (_source, store, sid, attachment) = seeded();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("完整备份.zip");
        let report = store.backup(&archive, &[], || Ok(())).unwrap();
        assert_eq!((report.sessions, report.skipped), (1, 0));
        #[cfg(unix)]
        assert_eq!(crate::config::file_mode(&archive), Some(0o600));
        let mut zip = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
        assert!(!zip
            .file_names()
            .map(Result::unwrap)
            .any(|name| name.contains("runtime-settings") || name.contains("auth.json")));
        let mut manifest = String::new();
        zip.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut manifest)
            .unwrap();
        assert!(!manifest.contains("must-not-export"));
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        let report = restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(report.imported, 1);
        assert!(report.warning.is_none());
        assert_eq!(
            restored.load_record(&sid).unwrap().usage,
            store.load_record(&sid).unwrap().usage
        );
        assert_eq!(
            restored.load_record(&sid).unwrap().operations,
            store.load_record(&sid).unwrap().operations
        );
        assert_eq!(
            restored.load_record(&sid).unwrap().session.draft,
            "未发送草稿"
        );
        assert_eq!(
            fs::read(restored.attachment_root(&sid).unwrap().join(attachment)).unwrap(),
            "中文附件".as_bytes()
        );
        assert_eq!(
            fs::read(restored.native_history_path(&sid).unwrap()).unwrap(),
            fs::read(store.native_history_path(&sid).unwrap()).unwrap()
        );
        assert_eq!(
            restored.remembered_model().unwrap(),
            Some("test/model".into())
        );
        let repeat = restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!((repeat.imported, repeat.skipped), (0, 1));
    }

    #[test]
    fn request_provenance_survives_real_archive_roundtrip_without_rebilling() {
        let (_source, store, sid, _) = seeded();
        let native = store.native_history_path(&sid).unwrap();
        let mut history = fs::read_to_string(&native).unwrap();
        history.push_str(&json!({"type":"custom","customType":"ofox-desktop-request","id":"61ce1a09","parentId":"a005630a","timestamp":"2026-10-30T00:00:00Z",
            "data":{"kind":"chat","modelId":"test/model","apex":"ofox.io","runId":"a48cac04-e861-4e76-92e8-6a73896a095c","operationId":"79346d31-7f3c-48d5-b3ac-81a889060608","status":"completed","httpStatus":200,"requestId":"req_archived","requestIdSource":"x-ofox-request-id","usage":{"input":10},"priceSnapshot":{"cost":{"input":2}},"billedCostUsd":null}}).to_string());
        history.push('\n');
        fs::write(&native, &history).unwrap();
        store.mutate(&sid, |r| {
            r.operations[0].media = Some(serde_json::from_value(json!({"kind":"image","model":"test/image","params":{},"status":"completed","apex":"ofox.ai","cost":"0.02","requestAttempts":[{"httpStatus":200,"requestId":"req_image_archived","requestIdSource":"x-request-id"}],"responseUsage":{"output_tokens":100},"priceSnapshot":{"perImagePrice":"0.02"}})).unwrap());
            Ok(())
        }).unwrap();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("receipts.zip");
        assert_eq!(store.backup(&archive, &[], || Ok(())).unwrap().sessions, 1);
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        assert_eq!(
            restored.restore(&archive, &[], || Ok(())).unwrap().imported,
            1
        );
        assert_eq!(
            restored.load_record(&sid).unwrap().operations,
            store.load_record(&sid).unwrap().operations
        );
        assert_eq!(
            fs::read_to_string(restored.native_history_path(&sid).unwrap()).unwrap(),
            history
        );
        let detail = restored.load_detail(&sid).unwrap();
        assert_eq!(
            detail.usage["requests"][0]["requestId"],
            "req_image_archived"
        );
        assert_eq!(
            detail.usage["requests"][0]["priceSnapshot"]["perImagePrice"],
            "0.02"
        );
        assert!(detail.usage["requests"][0]["billedCostUsd"].is_null());
        assert_eq!(
            restored.restore(&archive, &[], || Ok(())).unwrap().skipped,
            1
        );
    }

    /// A generated title's cost is kept under the turn it named.
    #[test]
    fn a_titles_usage_entry_is_backed_up_and_restored_with_its_history() {
        let (_source, store, sid, _) = seeded();
        let native = store.native_history_path(&sid).unwrap();
        let mut history = fs::read_to_string(&native).unwrap();
        history.push_str(
            &json!({
                "type": "custom", "customType": "ofox-desktop-usage", "id": "7d1e9a20",
                "parentId": "a005630a", "timestamp": "2026-10-29T09:00:00.000Z",
                "data": {
                    "runId": "a48cac04-e861-4e76-92e8-6a73896a095c",
                    "operationId": "79346d31-7f3c-48d5-b3ac-81a889060608",
                    "type": "title", "usageCoverage": "complete", "pricingKnown": true,
                    "usage": {"input": 168, "output": 17, "cacheRead": 0, "cacheWrite": 0,
                              "totalTokens": 185, "cost": {"input": 0.0000084,
                              "output": 0.0000068, "cacheRead": 0, "cacheWrite": 0,
                              "total": 0.0000152}},
                    "priceSnapshot": {"modelId": "openai/gpt-5-nano",
                                      "api": "openai-completions", "pricingKnown": true,
                                      "cost": {"input": 0.05, "output": 0.4}},
                },
            })
            .to_string(),
        );
        history.push('\n');
        fs::write(&native, &history).unwrap();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("backup.zip");
        let report = store.backup(&archive, &[], || Ok(())).unwrap();
        assert_eq!((report.sessions, report.skipped), (1, 0));
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        let report = restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(
            fs::read_to_string(restored.native_history_path(&sid).unwrap()).unwrap(),
            history
        );
    }

    /// A version switch's leaf entry, and what each message says of its
    /// model, usage, end and the turn it follows, come back as they were.
    #[test]
    fn a_version_switch_and_the_message_meta_are_backed_up_and_restored() {
        let (_source, store, sid, _) = seeded();
        let native = store.native_history_path(&sid).unwrap();
        let mut history = fs::read_to_string(&native).unwrap();
        history.push_str(
            &json!({
                "type": "custom", "customType": "ofox-desktop-leaf", "id": "5b2c7e10",
                "parentId": "a005630a", "timestamp": "2026-10-29T09:00:00.000Z",
                "data": {"entryId": "8ed73709"},
            })
            .to_string(),
        );
        history.push('\n');
        fs::write(&native, &history).unwrap();
        store
            .mutate(&sid, |record| {
                record.messages[0].branch_from = Some(None);
                let reply = &mut record.messages[1];
                reply.model_id = Some("test/model".into());
                reply.completed_at = Some(3);
                reply.usage = Some(json!({
                    "inputTokens": 10, "outputTokens": 5, "modelRequests": 1,
                    "estimatedCostUsd": "0.00007", "searchCount": 0,
                }));
                Ok(())
            })
            .unwrap();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("backup.zip");
        let report = store.backup(&archive, &[], || Ok(())).unwrap();
        assert_eq!((report.sessions, report.skipped), (1, 0));
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        let report = restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(
            fs::read_to_string(restored.native_history_path(&sid).unwrap()).unwrap(),
            history
        );
        assert_eq!(
            serde_json::to_value(restored.load_record(&sid).unwrap().messages).unwrap(),
            serde_json::to_value(store.load_record(&sid).unwrap().messages).unwrap()
        );
    }

    #[test]
    fn backup_restore_refuses_other_owner_and_stale_authority_and_never_overwrites() {
        let (_tmp, store, sid, _) = seeded();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let other = AssistantStore::for_owner(output.path(), &"b".repeat(64)).unwrap();
        assert_eq!(
            other.restore(&archive, &[], || Ok(())).unwrap_err(),
            "assistant_backup_owner_mismatch"
        );
        store
            .mutate(&sid, |record| {
                record.session.draft = "newer local draft".into();
                Ok(())
            })
            .unwrap();
        let report = store.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(
            (report.imported, report.skipped, report.conflicts),
            (0, 0, 1)
        );
        assert_eq!(
            store.load_record(&sid).unwrap().session.draft,
            "newer local draft"
        );
        assert_eq!(
            store
                .restore(&archive, &[], || Err("assistant_identity_changed".into()))
                .unwrap_err(),
            "assistant_identity_changed"
        );
    }

    #[test]
    fn backup_waits_for_live_runs_and_keeps_existing_output() {
        let (_tmp, store, sid, _) = seeded();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("backup.zip");
        fs::write(&archive, "keep old backup").unwrap();
        store
            .mutate(&sid, |record| {
                record.session.active_run = Some(RunRef {
                    run_id: id(),
                    operation_id: id(),
                });
                Ok(())
            })
            .unwrap();
        assert_eq!(
            store.backup(&archive, &[], || Ok(())).unwrap_err(),
            TRANSFER_BUSY
        );
        assert_eq!(fs::read_to_string(&archive).unwrap(), "keep old backup");
    }

    fn message_json(value: Value) -> Message {
        let mut message = json!({
            "id": id(), "role": "user", "content": "hello", "createdAt": 1,
            "reasoning": null, "status": "completed", "revisionOf": null, "nativeEntryId": null,
        });
        message
            .as_object_mut()
            .unwrap()
            .extend(value.as_object().unwrap().clone());
        serde_json::from_value(message).unwrap()
    }

    #[test]
    fn backup_ignores_empty_folders_and_counts_damaged_conversations() {
        let (_tmp, store, sid, _) = seeded();
        // Unfinished creations: empty, or holding only the record's temp file.
        fs::create_dir(store.root.join(id())).unwrap();
        let unfinished = store.root.join(id());
        fs::create_dir(&unfinished).unwrap();
        fs::write(unfinished.join(".session.json.ofox-tmp-1-2"), "{").unwrap();
        let unreadable = store.root.join(id());
        fs::create_dir(&unreadable).unwrap();
        fs::write(unreadable.join(SESSION_FILE), "{not json").unwrap();
        let lost_history = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&lost_history, |record| {
                record.messages = vec![message_json(json!({"nativeEntryId": "8ed73709"}))];
                Ok(())
            })
            .unwrap();
        let lost_file = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&lost_file, |record| {
                record.attachments.push(Attachment {
                    origin: None,
                    source_message_id: None,
                    id: id(),
                    name: "gone.txt".into(),
                    mime_type: "text/plain".into(),
                    size_bytes: 4,
                    status: "ready".into(),
                    reason: None,
                    preview_url: None,
                    details: None,
                });
                Ok(())
            })
            .unwrap();
        let oversized = store.create("test/model", false).unwrap().session.id;
        let big = id();
        fs::File::create(store.attachment_dir(&oversized).unwrap().join(&big))
            .unwrap()
            .set_len(MAX_ATTACHMENT + 1)
            .unwrap();
        store
            .mutate(&oversized, |record| {
                record.attachments.push(Attachment {
                    origin: None,
                    source_message_id: None,
                    id: big.clone(),
                    name: "big.pdf".into(),
                    mime_type: "application/pdf".into(),
                    size_bytes: MAX_ATTACHMENT + 1,
                    status: "ready".into(),
                    reason: None,
                    preview_url: None,
                    details: None,
                });
                Ok(())
            })
            .unwrap();
        // Loads fine with the code dropped, so it is archived that way.
        let foreign_code = store.create("test/model", false).unwrap().session.id;
        store
            .mutate(&foreign_code, |record| {
                record.messages = vec![message_json(
                    json!({"status": "error", "errorCode": "assistant_network_error"}),
                )];
                Ok(())
            })
            .unwrap();
        let file = store.root.join(&foreign_code).join(SESSION_FILE);
        let text = fs::read_to_string(&file)
            .unwrap()
            .replace("assistant_network_error", "Upstream said no");
        fs::write(&file, text).unwrap();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        let report = store.backup(&archive, &[], || Ok(())).unwrap();
        assert_eq!((report.sessions, report.skipped), (2, 4));
        let mut zip = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
        let mut bytes = Vec::new();
        zip.by_name(MANIFEST)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        let manifest: Manifest = serde_json::from_slice(&bytes).unwrap();
        let mut archived = vec![sid.clone(), foreign_code.clone()];
        archived.sort();
        assert_eq!(manifest.sessions, archived);
        let report = store.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!((report.skipped, report.conflicts), (2, 0));
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        assert_eq!(
            restored.restore(&archive, &[], || Ok(())).unwrap().imported,
            2
        );
        let repaired = &restored.load_record(&foreign_code).unwrap().messages[0];
        assert_eq!(repaired.status, "error");
        assert!(repaired.error_code.is_none());
        assert!(
            !fs::read_to_string(restored.root.join(&foreign_code).join(SESSION_FILE))
                .unwrap()
                .contains("Upstream said no")
        );
    }

    #[test]
    fn restore_adds_missing_conversations_and_counts_changed_or_replying_ones() {
        let (_tmp, store, changed) = fixture();
        let deleted = store.create("test/model", false).unwrap().session.id;
        let replying = store.create("test/model", false).unwrap().session.id;
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        assert_eq!(store.backup(&archive, &[], || Ok(())).unwrap().sessions, 3);
        store
            .mutate(&changed, |record| {
                record.session.draft = "newer local draft".into();
                Ok(())
            })
            .unwrap();
        store
            .update(
                &serde_json::from_value(json!({"sessionId":deleted,"deleted":true})).unwrap(),
                || Ok(()),
            )
            .unwrap();
        store.delete_trashed(&deleted, || Ok(())).unwrap();
        store
            .mutate(&replying, |record| {
                record.session.active_run = Some(RunRef {
                    run_id: id(),
                    operation_id: id(),
                });
                Ok(())
            })
            .unwrap();
        let report = store.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(
            (report.imported, report.skipped, report.conflicts),
            (1, 0, 2)
        );
        assert!(report.warning.is_none());
        assert_eq!(
            store.load_record(&changed).unwrap().session.draft,
            "newer local draft"
        );
        assert!(store.load_record(&deleted).is_ok());
        assert!(store
            .load_record(&replying)
            .unwrap()
            .session
            .active_run
            .is_some());
    }

    #[test]
    fn a_retry_after_a_partial_restore_imports_the_rest_beside_a_changed_one() {
        let (_tmp, store, first) = fixture();
        let mut sids = [
            first,
            store.create("test/model", false).unwrap().session.id,
            store.create("test/model", false).unwrap().session.id,
        ];
        sids.sort();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        let published = restored.root.join(&sids[0]);
        let report = restored
            .restore(&archive, &[], || {
                if published.exists() {
                    Err("assistant_identity_changed".into())
                } else {
                    Ok(())
                }
            })
            .unwrap();
        assert_eq!(report.imported, 1);
        assert!(report.warning.is_some());
        restored
            .mutate(&sids[0], |record| {
                record.session.title = "Renamed after the first attempt".into();
                Ok(())
            })
            .unwrap();
        let retry = restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!((retry.imported, retry.skipped, retry.conflicts), (2, 0, 1));
        assert_eq!(restored.list(false, "").unwrap().sessions.len(), 3);
    }

    #[test]
    fn backup_blanks_the_native_working_folder_and_restore_still_finds_it_unchanged() {
        let (_tmp, store, sid, _) = seeded();
        let path = store.native_history_path(&sid).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        let (header, rest) = raw.split_once('\n').unwrap();
        let mut header: Value = serde_json::from_str(header).unwrap();
        header["cwd"] = json!("/Users/someone/Library/private/workspace");
        header["parentSession"] = json!("/Users/someone/Library/private/parent.jsonl");
        let local = format!("{}\n{rest}", serde_json::to_string(&header).unwrap());
        fs::write(&path, &local).unwrap();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let name = format!("{sid}/{NATIVE_HISTORY_FILE}");
        let (_, archived) = members_of(&archive)
            .into_iter()
            .find(|(member, _)| *member == name)
            .unwrap();
        let archived = String::from_utf8(archived).unwrap();
        assert!(!archived.contains("/Users/someone"));
        let (header, kept) = archived.split_once('\n').unwrap();
        let header: Value = serde_json::from_str(header).unwrap();
        assert_eq!(header["cwd"], "");
        assert!(header.get("parentSession").is_none());
        assert_eq!(kept, rest);
        // Restoring into the profile it came from finds the same conversation.
        let report = store.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!((report.skipped, report.conflicts), (1, 0));
        assert_eq!(fs::read_to_string(&path).unwrap(), local);
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        assert_eq!(
            restored.restore(&archive, &[], || Ok(())).unwrap().imported,
            1
        );
        assert_eq!(
            fs::read_to_string(restored.native_history_path(&sid).unwrap()).unwrap(),
            archived
        );
    }

    #[test]
    fn an_archive_from_before_the_blanked_working_folder_still_matches_its_profile() {
        let (_tmp, store, sid, _) = seeded();
        let path = store.native_history_path(&sid).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        let (header, rest) = raw.split_once('\n').unwrap();
        let mut header: Value = serde_json::from_str(header).unwrap();
        header["cwd"] = json!("/Users/someone/Library/private/workspace");
        let local = format!("{}\n{rest}", serde_json::to_string(&header).unwrap());
        fs::write(&path, &local).unwrap();
        let out = tempfile::tempdir().unwrap();
        let current = out.path().join("current.zip");
        store.backup(&current, &[], || Ok(())).unwrap();
        // Earlier archives kept the native history byte for byte.
        let mut members = members_of(&current);
        replace_member(
            &mut members,
            &format!("{sid}/{NATIVE_HISTORY_FILE}"),
            local.clone().into_bytes(),
        );
        let older = out.path().join("older.zip");
        write_zip(&older, &members, false);
        let report = store.restore(&older, &[], || Ok(())).unwrap();
        assert_eq!((report.skipped, report.conflicts), (1, 0));
        assert_eq!(fs::read_to_string(&path).unwrap(), local);
    }

    #[test]
    fn restore_refuses_a_saved_error_that_is_not_an_assistant_code() {
        let (_tmp, store, sid, _) = seeded();
        store
            .mutate(&sid, |record| {
                record.messages[1].status = "error".into();
                record.messages[1].error_code = Some("assistant_network_error".into());
                Ok(())
            })
            .unwrap();
        let out = tempfile::tempdir().unwrap();
        let original = out.path().join("original.zip");
        store.backup(&original, &[], || Ok(())).unwrap();
        let name = format!("{sid}/{SESSION_FILE}");
        let mut members = members_of(&original);
        let (_, record) = members.iter().find(|(member, _)| *member == name).unwrap();
        let record = String::from_utf8(record.clone())
            .unwrap()
            .replace("assistant_network_error", "ECONNRESET");
        replace_member(&mut members, &name, record.into_bytes());
        let changed = out.path().join("changed.zip");
        write_zip(&changed, &members, false);
        assert_refused(&changed, &store.owner, INVALID);
    }

    #[test]
    fn backup_restore_rejects_unlisted_paths_before_importing() {
        let (_tmp, store, _sid, _) = seeded();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        for extra in ["../escape", "runtime-settings/auth.json"] {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&archive)
                .unwrap();
            let mut zip = zip::ZipWriter::new_append(file).unwrap();
            zip.start_file(extra, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"bad").unwrap();
            zip.finish().unwrap();
            let target = tempfile::tempdir().unwrap();
            let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
            assert_eq!(
                restored.restore(&archive, &[], || Ok(())).unwrap_err(),
                "assistant_backup_invalid"
            );
            assert!(restored.list(false, "").unwrap().sessions.is_empty());
        }
    }
    #[test]
    fn restore_detects_modified_payload_before_publishing_any_session() {
        let (_tmp, store, _sid, _) = seeded();
        let out = tempfile::tempdir().unwrap();
        let original = out.path().join("original.zip");
        store.backup(&original, &[], || Ok(())).unwrap();
        let mut old = zip::ZipArchive::new(fs::File::open(original).unwrap()).unwrap();
        let changed = out.path().join("changed.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&changed).unwrap());
        for i in 0..old.len() {
            let mut entry = old.by_index(i).unwrap();
            let name = entry.name().unwrap().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            if name.ends_with("/session.json") {
                bytes.push(b' ');
            }
            writer.start_file(name, options()).unwrap();
            writer.write_all(&bytes).unwrap();
        }
        writer.finish().unwrap();
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        assert_eq!(
            restored.restore(&changed, &[], || Ok(())).unwrap_err(),
            INVALID
        );
        assert!(restored.list(false, "").unwrap().sessions.is_empty());
        assert!(!fs::read_dir(&restored.root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".restore-")));
    }

    #[test]
    fn interrupted_restore_preserves_published_sessions_and_retry_is_idempotent() {
        let (_tmp, store, first) = fixture();
        let second = store.create("test/model", false).unwrap().session.id;
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        let first_path = restored.root.join(std::cmp::min(first, second));
        let report = restored
            .restore(&archive, &[], || {
                if first_path.exists() {
                    Err("assistant_identity_changed".into())
                } else {
                    Ok(())
                }
            })
            .unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(
            report.warning.as_deref(),
            Some("assistant_identity_changed")
        );
        assert_eq!(restored.list(false, "").unwrap().sessions.len(), 1);
        let retry = restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!((retry.imported, retry.skipped, retry.conflicts), (1, 1, 0));
        assert!(retry.warning.is_none());
    }

    #[test]
    fn backup_rejects_managed_keys_in_attachments_without_modifying_them_or_prior_backup() {
        let (_tmp, store, sid, aid) = seeded();
        let key = "secret-managed-key-in-attachment".to_string();
        let file = store.attachment_root(&sid).unwrap().join(aid);
        fs::write(&file, &key).unwrap();
        store
            .mutate(&sid, |record| {
                record.attachments[0].size_bytes = key.len() as u64;
                Ok(())
            })
            .unwrap();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        fs::write(&archive, "keep").unwrap();
        assert_eq!(
            store
                .backup(&archive, std::slice::from_ref(&key), || Ok(()))
                .unwrap_err(),
            CONTAINS_KEY
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), key);
        assert_eq!(fs::read_to_string(&archive).unwrap(), "keep");
    }
    fn rewrite_native(
        original: &Path,
        changed: &Path,
        sid: &str,
        mutate: impl FnOnce(&mut Vec<Value>),
    ) {
        let mut zip = zip::ZipArchive::new(fs::File::open(original).unwrap()).unwrap();
        let mut entries = BTreeMap::new();
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).unwrap();
            let name = file.name().unwrap().into_owned();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            entries.insert(name, bytes);
        }
        let name = format!("{sid}/{NATIVE_HISTORY_FILE}");
        let mut lines: Vec<Value> = std::str::from_utf8(&entries[&name])
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        mutate(&mut lines);
        let bytes = lines
            .into_iter()
            .map(|value| serde_json::to_string(&value).unwrap() + "\n")
            .collect::<String>()
            .into_bytes();
        let mut manifest: Manifest = serde_json::from_slice(&entries[MANIFEST]).unwrap();
        manifest.files.insert(name.clone(), digest(&bytes));
        entries.insert(name, bytes);
        entries.insert(MANIFEST.into(), serde_json::to_vec(&manifest).unwrap());
        let mut writer = zip::ZipWriter::new(fs::File::create(changed).unwrap());
        for (name, bytes) in entries {
            writer.start_file(name, options()).unwrap();
            writer.write_all(&bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn restore_rejects_native_message_shapes_and_broken_run_links_before_publication() {
        let (_tmp, store, sid, _) = seeded();
        let out = tempfile::tempdir().unwrap();
        let original = out.path().join("original.zip");
        store.backup(&original, &[], || Ok(())).unwrap();
        let mut accepted = Vec::new();
        for case in [
            "no-message",
            "no-marker",
            "unknown-run",
            "wrong-role",
            "null-part",
        ] {
            let changed = out.path().join(format!("{case}.zip"));
            rewrite_native(&original, &changed, &sid, |lines| match case {
                "no-message" => {
                    lines[4].as_object_mut().unwrap().remove("message");
                }
                "no-marker" => {
                    lines[2]["customType"] = json!("other");
                }
                "unknown-run" => {
                    lines[2]["data"]["runId"] = json!(id());
                }
                "wrong-role" => {
                    lines[4]["message"]["role"] = json!("user");
                }
                "null-part" => {
                    lines[4]["message"]["content"] = json!([null]);
                }
                _ => unreachable!(),
            });
            let target = tempfile::tempdir().unwrap();
            let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
            if restored.restore(&changed, &[], || Ok(())).is_ok() {
                accepted.push(case);
            } else {
                assert!(restored.list(false, "").unwrap().sessions.is_empty());
            }
        }
        assert!(
            accepted.is_empty(),
            "invalid histories were accepted: {accepted:?}"
        );
    }

    #[test]
    fn backup_restores_a_supported_image_message_larger_than_eight_mib() {
        use base64::Engine;
        let (_tmp, store, sid, _) = seeded();
        let path = store.native_history_path(&sid).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        let mut lines: Vec<Value> = raw
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        // A valid 1px PNG with padding, still below the supported 10 MiB file cap.
        let mut png=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jfH8AAAAASUVORK5CYII=").unwrap();
        png.resize(7 * 1024 * 1024, 0);
        lines[3]["message"]["content"] = json!([{"type":"image","data":base64::engine::general_purpose::STANDARD.encode(png),"mimeType":"image/png"}]);
        let native = lines
            .into_iter()
            .map(|line| serde_json::to_string(&line).unwrap() + "\n")
            .collect::<String>();
        assert!(native.len() > 8 * 1024 * 1024);
        fs::write(&path, &native).unwrap();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("images.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        restored.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(
            fs::read(restored.native_history_path(&sid).unwrap()).unwrap(),
            native.as_bytes()
        );
    }
    /// Every member of `archive`, in archive order.
    fn members_of(archive: &Path) -> Vec<(String, Vec<u8>)> {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut file = zip.by_index(i).unwrap();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                (file.name().unwrap().into_owned(), bytes)
            })
            .collect()
    }

    fn write_zip(path: &Path, members: &[(String, Vec<u8>)], zip64: bool) {
        let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
        if zip64 {
            writer.set_raw_zip64_extensible_data_sector(Box::default());
        }
        for (name, bytes) in members {
            writer.start_file(name.as_str(), options()).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    /// Where the last `magic` record starts.
    fn last(bytes: &[u8], magic: &[u8]) -> usize {
        bytes
            .windows(magic.len())
            .rposition(|part| part == magic)
            .unwrap()
    }

    /// `members` with `name` holding `bytes`, and the manifest saying so.
    fn replace_member(members: &mut [(String, Vec<u8>)], name: &str, bytes: Vec<u8>) {
        let at = |name: &str| {
            members
                .iter()
                .position(|(member, _)| member == name)
                .unwrap()
        };
        let mut manifest: Manifest = serde_json::from_slice(&members[at(MANIFEST)].1).unwrap();
        manifest.files.insert(name.into(), digest(&bytes));
        let (named, list) = (at(name), at(MANIFEST));
        members[named].1 = bytes;
        members[list].1 = serde_json::to_vec(&manifest).unwrap();
    }

    fn assert_refused(archive: &Path, owner: &str, code: &str) {
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), owner).unwrap();
        assert_eq!(restored.restore(archive, &[], || Ok(())).unwrap_err(), code);
        assert!(restored.list(false, "").unwrap().sessions.is_empty());
    }

    #[test]
    fn restore_refuses_two_members_with_one_name() {
        let (_tmp, store, _sid, _) = seeded();
        let out = tempfile::tempdir().unwrap();
        let original = out.path().join("original.zip");
        store.backup(&original, &[], || Ok(())).unwrap();
        // The zip crate keeps one entry per name: an earlier copy with other
        // bytes would be invisible to every check made by name.
        let mut members = vec![("manifesT.json".to_string(), b"{}".to_vec())];
        members.extend(members_of(&original));
        let doubled = out.path().join("doubled.zip");
        write_zip(&doubled, &members, false);
        let mut bytes = fs::read(&doubled).unwrap();
        while let Some(at) = bytes.windows(13).position(|part| part == b"manifesT.json") {
            bytes[at..at + 13].copy_from_slice(MANIFEST.as_bytes());
        }
        fs::write(&doubled, bytes).unwrap();
        assert_refused(&doubled, &store.owner, INVALID);
    }

    #[test]
    fn restore_refuses_a_declared_entry_count_over_the_limit_before_reading_entries() {
        let (_tmp, store, _sid, _) = seeded();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let mut bytes = fs::read(&archive).unwrap();
        let end = last(&bytes, b"PK\x05\x06");
        for field in [end + 8, end + 10] {
            bytes[field..field + 2].copy_from_slice(&30_000u16.to_le_bytes());
        }
        fs::write(&archive, bytes).unwrap();
        assert_refused(&archive, &store.owner, TOO_LARGE);
    }

    #[test]
    fn restore_reads_the_zip64_entry_count() {
        let (_tmp, store, _sid, _) = seeded();
        let out = tempfile::tempdir().unwrap();
        let original = out.path().join("original.zip");
        store.backup(&original, &[], || Ok(())).unwrap();
        let archive = out.path().join("zip64.zip");
        write_zip(&archive, &members_of(&original), true);
        // Send readers to the ZIP64 record, as an archive of 65,535 or more
        // members must.
        let mut bytes = fs::read(&archive).unwrap();
        let end = last(&bytes, b"PK\x05\x06");
        bytes[end + 8..end + 12].copy_from_slice(&[0xff; 4]);
        fs::write(&archive, &bytes).unwrap();
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        assert_eq!(
            restored.restore(&archive, &[], || Ok(())).unwrap().imported,
            1
        );
        let locator = end - 20;
        assert_eq!(&bytes[locator..locator + 4], b"PK\x06\x07");
        let record =
            u64::from_le_bytes(bytes[locator + 8..locator + 16].try_into().unwrap()) as usize;
        for field in [record + 24, record + 32] {
            bytes[field..field + 8].copy_from_slice(&30_000u64.to_le_bytes());
        }
        fs::write(&archive, bytes).unwrap();
        assert_refused(&archive, &store.owner, TOO_LARGE);
    }

    #[test]
    fn restore_refuses_a_fake_end_record_after_the_real_one() {
        let (_tmp, store, _sid, _) = seeded();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let mut bytes = fs::read(&archive).unwrap();
        // The crate skips a last end record that fails its checks (here: a
        // directory starting at the record itself) and parses the one before,
        // so a count taken from the fake says nothing about what is parsed.
        let end = last(&bytes, b"PK\x05\x06");
        let mut fake = bytes[end..end + 22].to_vec();
        fake[16..20].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        bytes.extend(fake);
        assert!(checked_entries(&mut std::io::Cursor::new(&bytes)).is_err());
        fs::write(&archive, bytes).unwrap();
        assert_refused(&archive, &store.owner, INVALID);
    }

    /// Every shorter prefix is refused; any single changed byte near the end
    /// is refused or still consistent, never a panic.
    fn survives_damage(bytes: &[u8]) {
        for len in 0..bytes.len() {
            assert!(
                checked_entries(&mut std::io::Cursor::new(&bytes[..len])).is_err(),
                "a prefix of {len} bytes"
            );
        }
        for at in bytes.len().saturating_sub(800)..bytes.len() {
            for value in [0x00, 0x7f, 0xff] {
                let mut changed = bytes.to_vec();
                changed[at] = value;
                let _ = checked_entries(&mut std::io::Cursor::new(&changed));
            }
        }
    }

    #[test]
    fn truncated_or_damaged_archive_ends_are_refused_without_panicking() {
        let (_tmp, store, _sid, _) = seeded();
        let out = tempfile::tempdir().unwrap();
        let original = out.path().join("backup.zip");
        store.backup(&original, &[], || Ok(())).unwrap();
        let bytes = fs::read(&original).unwrap();
        let members = members_of(&original);
        assert_eq!(
            checked_entries(&mut std::io::Cursor::new(&bytes)).unwrap(),
            members.len() as u64
        );
        survives_damage(&bytes);
        let zip64 = out.path().join("zip64.zip");
        write_zip(&zip64, &members, true);
        let mut bytes = fs::read(&zip64).unwrap();
        let end = last(&bytes, b"PK\x05\x06");
        bytes[end + 8..end + 12].copy_from_slice(&[0xff; 4]);
        assert_eq!(
            checked_entries(&mut std::io::Cursor::new(&bytes)).unwrap(),
            members.len() as u64
        );
        survives_damage(&bytes);
        let empty = out.path().join("empty.zip");
        fs::write(&empty, b"").unwrap();
        assert_refused(&empty, &store.owner, INVALID);
        let cut = out.path().join("cut.zip");
        fs::write(&cut, &bytes[..bytes.len() - 1]).unwrap();
        assert_refused(&cut, &store.owner, INVALID);
    }

    #[test]
    fn restore_refuses_a_member_that_inflates_past_its_declared_size() {
        let (_tmp, store, _sid, _) = seeded();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let mut bytes = fs::read(&archive).unwrap();
        let header = (0..bytes.len() - 4)
            .filter(|&at| &bytes[at..at + 4] == b"PK\x01\x02")
            .find(|&at| {
                let length = u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]) as usize;
                bytes[at + 46..at + 46 + length].ends_with(b"/session.json")
            })
            .unwrap();
        let size = u32::from_le_bytes(bytes[header + 24..header + 28].try_into().unwrap());
        bytes[header + 24..header + 28].copy_from_slice(&(size - 1).to_le_bytes());
        fs::write(&archive, bytes).unwrap();
        assert_refused(&archive, &store.owner, INVALID);
    }

    #[test]
    fn restoring_a_conversation_finishes_its_interrupted_deletion_first() {
        let (_tmp, store, sid) = fixture();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("backup.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let trash = || {
            store
                .update(
                    &serde_json::from_value(json!({"sessionId":sid,"deleted":true})).unwrap(),
                    || Ok(()),
                )
                .unwrap();
        };
        trash();
        // A permanent deletion that stopped after its move, before removal.
        let tombstone = store.root.join(format!(".deleting-{sid}"));
        fs::rename(store.root.join(&sid), &tombstone).unwrap();
        fs::write(tombstone.join("leftover"), b"old copy").unwrap();
        assert_eq!(store.restore(&archive, &[], || Ok(())).unwrap().imported, 1);
        assert!(!tombstone.exists());
        trash();
        store.delete_trashed(&sid, || Ok(())).unwrap();
        assert!(store.list(true, "").unwrap().sessions.is_empty());
        assert!(!store.root.join(&sid).exists());
    }

    #[test]
    fn restore_rejects_symlink_members_and_oversized_expanded_manifest() {
        let (_tmp, store, _sid, _) = seeded();
        let out = tempfile::tempdir().unwrap();
        let original = out.path().join("original.zip");
        store.backup(&original, &[], || Ok(())).unwrap();
        // Mark one central-directory member as a Unix symlink while leaving
        // its path and payload otherwise unchanged.
        let mut bytes = fs::read(&original).unwrap();
        let offset = bytes
            .windows(4)
            .position(|part| part == b"PK\x01\x02")
            .unwrap();
        bytes[offset + 5] = 3;
        bytes[offset + 38..offset + 42].copy_from_slice(&(0o120777u32 << 16).to_le_bytes());
        let linked = out.path().join("linked.zip");
        fs::write(&linked, bytes).unwrap();
        let target = tempfile::tempdir().unwrap();
        let restored = AssistantStore::for_owner(target.path(), &store.owner).unwrap();
        assert_eq!(
            restored.restore(&linked, &[], || Ok(())).unwrap_err(),
            INVALID
        );
        let large = out.path().join("large.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&large).unwrap());
        writer.start_file(MANIFEST, options()).unwrap();
        let chunk = vec![b' '; 1024 * 1024];
        for _ in 0..17 {
            writer.write_all(&chunk).unwrap();
        }
        writer.finish().unwrap();
        assert_eq!(
            restored.restore(&large, &[], || Ok(())).unwrap_err(),
            TOO_LARGE
        );
        assert!(restored.list(false, "").unwrap().sessions.is_empty());
    }
    #[test]
    fn video_backup_keeps_large_media_stored_and_can_exclude_with_a_manifest() {
        let (_tmp, store, sid) = fixture();
        let aid = id();
        let mid = id();
        let size = MAX_ATTACHMENT + 1;
        let source = store.attachment_dir(&sid).unwrap().join(&aid);
        let mut file = fs::File::create(&source).unwrap();
        file.write_all(b"\0\0\0\x18ftypisomfixture").unwrap();
        file.set_len(size).unwrap();
        let attachment:Attachment=serde_json::from_value(json!({"id":aid,"name":"movie.mp4","mimeType":"video/mp4","sizeBytes":size,"status":"ready","origin":"generated","sourceMessageId":mid})).unwrap();
        store.mutate(&sid,|record|{record.attachments.push(attachment.clone());record.messages.push(serde_json::from_value(json!({"id":mid,"role":"media","content":"movie","createdAt":1,"status":"completed","attachments":[attachment],"media":{"kind":"video","model":"test/video","params":{},"status":"completed","outputs":[aid],"cost":"0.123"}})).unwrap());Ok(())}).unwrap();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("video.zip");
        let report = store.backup(&archive, &[], || Ok(())).unwrap();
        assert_eq!(report.sessions, 1);
        assert_eq!(report.skipped, 0);
        let mut zip = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
        let member = format!("{sid}/{ATTACHMENTS_DIR}/{aid}");
        assert_eq!(
            zip.by_name(&member).unwrap().compression(),
            zip::CompressionMethod::Stored
        );
        let restored = tempfile::tempdir().unwrap();
        let target = AssistantStore::for_owner(restored.path(), &store.owner).unwrap();
        target.restore(&archive, &[], || Ok(())).unwrap();
        assert_eq!(
            fs::metadata(target.attachment_root(&sid).unwrap().join(&aid))
                .unwrap()
                .len(),
            size
        );
        let excluded = out.path().join("excluded.zip");
        store
            .backup_with_options(&excluded, &[], true, || Ok(()))
            .unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(&excluded).unwrap()).unwrap();
        assert!(zip.by_name(&member).is_err());
        let manifest: Value =
            serde_json::from_slice(&read_member(&mut zip, MANIFEST, MAX_MANIFEST).unwrap())
                .unwrap();
        assert_eq!(manifest["excludedVideos"], json!([member]));
        let blank = tempfile::tempdir().unwrap();
        let target = AssistantStore::for_owner(blank.path(), &store.owner).unwrap();
        target.restore(&excluded, &[], || Ok(())).unwrap();
        let record = target.load_record(&sid).unwrap();
        assert_eq!(record.attachments[0].status, "missing");
        assert_eq!(
            record.attachments[0].reason.as_deref(),
            Some("assistant_attachment_not_in_backup")
        );
        assert_eq!(record.messages[0].attachments[0].status, "missing");
        assert!(!target.attachment_root(&sid).unwrap().join(&aid).exists());
        assert_eq!(
            store.load_record(&sid).unwrap().attachments[0].status,
            "ready"
        );
        assert!(source.exists());
        let again = out.path().join("again.zip");
        assert_eq!(target.backup(&again, &[], || Ok(())).unwrap().sessions, 1);
        let third = tempfile::tempdir().unwrap();
        AssistantStore::for_owner(third.path(), &store.owner)
            .unwrap()
            .restore(&again, &[], || Ok(()))
            .unwrap();
    }
    #[test]
    fn audio_models_round_trip_in_backups_and_keep_existing_local_choices() {
        use super::super::super::catalog::Kind;
        let (_source, store, _sid, _) = seeded();
        store
            .remember_model_kind("speech/archive", Kind::Speech)
            .unwrap();
        store
            .remember_model_kind("asr/archive", Kind::Transcription)
            .unwrap();
        store
            .remember_speech_choices("google/gemini-3.8-flash-tts", "Kore", "wav")
            .unwrap();
        let (_output, archive) = backed_up(&store);
        let (_target, restored) = profile(&store.owner);
        restored.ensure_root().unwrap();
        fs::write(restored.root.join(PREFS_FILE),br#"{"version":1,"future":true,"speechPreferences":{"google/gemini-3.8-flash-tts":{"voice":"Puck","futureChoice":true}}}"#).unwrap();
        restored
            .remember_model_kind("speech/local", Kind::Speech)
            .unwrap();
        restored.restore(&archive, &[], || Ok(())).unwrap();
        let remembered = restored.remembered_models().unwrap();
        assert_eq!(remembered["speech"], "speech/local");
        assert_eq!(remembered["transcription"], "asr/archive");
        assert_eq!(remembered["chat"], "test/model");
        assert_eq!(
            restored.speech_preferences().unwrap()["google/gemini-3.8-flash-tts"],
            json!({"voice":"Puck","format":"wav"})
        );
        let local: Value =
            serde_json::from_slice(&fs::read(restored.root.join(PREFS_FILE)).unwrap()).unwrap();
        assert_eq!(local["future"], true);
        assert_eq!(
            local["speechPreferences"]["google/gemini-3.8-flash-tts"]["futureChoice"],
            true
        );
    }
    #[test]
    fn audio_retention_review_backup_excludes_unknown_nested_preferences() {
        let (_root, store, _sid, _aid) = seeded();
        fs::write(store.root.join(PREFS_FILE),serde_json::to_vec(&json!({"version":1,"speechPreferences":{"google/gemini-3.8-flash-tts":{"voice":"Kore","format":"wav","unknownCredential":"must-not-export-nested"}}})).unwrap()).unwrap();
        let (_out, archive) = backed_up(&store);
        let bytes = members_of(&archive)
            .into_iter()
            .find(|(name, _)| name == PREFS_FILE)
            .unwrap()
            .1;
        assert!(!String::from_utf8_lossy(&bytes).contains("must-not-export-nested"));
        assert!(
            String::from_utf8_lossy(&fs::read(store.root.join(PREFS_FILE)).unwrap())
                .contains("must-not-export-nested")
        );
    }
    #[test]
    fn audio_review_portable_preferences_still_restore_after_managed_key_redaction() {
        let (_root, store, _sid, _aid) = seeded();
        store
            .remember_speech_choices("google/gemini-3.8-flash-tts", "SyntheticSecretAscii", "wav")
            .unwrap();
        let out = tempfile::tempdir().unwrap();
        let archive = out.path().join("redacted-prefs.zip");
        store
            .backup(&archive, &["SyntheticSecretAscii".into()], || Ok(()))
            .unwrap();
        let (_target, restored) = profile(&store.owner);
        assert_eq!(
            restored.restore(&archive, &[], || Ok(())).unwrap().imported,
            1
        );
        assert_eq!(
            restored.speech_preferences().unwrap()["google/gemini-3.8-flash-tts"]["voice"],
            crate::redact::REDACTED
        );
    }
}
