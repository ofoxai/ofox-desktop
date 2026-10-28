//! Portable archives contain conversation records, native history, referenced
//! attachments and preferences. Each session is published atomically on restore;
//! existing conversations are compared and never overwritten.
mod native;
use super::*;
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};

const INVALID: &str = "assistant_backup_invalid";
const TOO_LARGE: &str = "assistant_backup_too_large";
const CONFLICT: &str = "assistant_backup_conflict";
const FAILED: &str = "assistant_backup_failed";
const CONTAINS_KEY: &str = "assistant_backup_contains_key";
const MAX_ZIP: u64 = 512 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 20_000;
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;
const MAX_RECORD: u64 = 64 * 1024 * 1024;
const MAX_NATIVE: u64 = 256 * 1024 * 1024;
const MAX_ATTACHMENT: u64 = 50 * 1024 * 1024;
const MANIFEST: &str = "manifest.json";

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub imported: usize,
    pub skipped: usize,
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
            Ok(MAX_ATTACHMENT)
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
        return Err("assistant_session_busy".into());
    }
    Ok(())
}

fn validate_session(root: &Path, sid: &str, names: &HashSet<String>) -> Result<(), String> {
    let bytes = read_file(&root.join(sid).join(SESSION_FILE), MAX_RECORD)?.ok_or(INVALID)?;
    let record = decode(&bytes, sid).map_err(|_| INVALID)?;
    idle(&record)?;
    let mut expected = HashSet::from([format!("{sid}/{SESSION_FILE}")]);
    let mut attachments = HashMap::new();
    for file in &record.attachments {
        validate_id(&file.id).map_err(|_| INVALID)?;
        if attachments.insert(file.id.as_str(), file).is_some() {
            return Err(INVALID.into());
        }
        let name = format!("{sid}/{ATTACHMENTS_DIR}/{}", file.id);
        let bytes = read_file(&root.join(&name), MAX_ATTACHMENT)?.ok_or(INVALID)?;
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
            native::validate(&bytes, &record)?
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
    let prefix = format!("{sid}/");
    if names
        .iter()
        .filter(|name| name.starts_with(&prefix))
        .cloned()
        .collect::<HashSet<_>>()
        != expected
    {
        return Err(INVALID.into());
    }
    Ok(())
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
) -> Result<(), String> {
    if bytes.len() as u64 > limit(&name)?
        || *total + bytes.len() as u64 > MAX_TOTAL
        || manifest.files.len() + 1 >= MAX_FILES
    {
        return Err(TOO_LARGE.into());
    }
    *total += bytes.len() as u64;
    manifest.files.insert(name.clone(), digest(&bytes));
    writer.start_file(name, options()).map_err(|_| FAILED)?;
    writer.write_all(&bytes).map_err(|_| FAILED.to_string())
}

impl AssistantStore {
    /// Per-session consistent snapshots; a changed session inventory aborts the
    /// archive. The final destination is replaced only after the ZIP is complete.
    pub fn backup(
        &self,
        path: &Path,
        secrets: &[String],
        admit: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        let lock = file_lock(&self.root.join(".transfer-lock"));
        let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        admit()?;
        let path = super::super::transfers::destination(path, &self.config_dir)?;
        let mut sessions = if real_dir(&self.root)? {
            self.session_ids()?
        } else {
            vec![]
        };
        sessions.sort();
        let mut manifest = Manifest {
            version: 1,
            native_version: 3,
            owner: self.owner.clone(),
            sessions: sessions.clone(),
            files: BTreeMap::new(),
        };
        let mut writer = zip::ZipWriter::new(tempfile::tempfile().map_err(|_| FAILED)?);
        let mut total = 0;
        for sid in &sessions {
            self.with_attachment_lock(sid, || {
                let lock = file_lock(&self.root.join(sid).join(SESSION_FILE));
                let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                admit()?;
                let raw = read_file(&self.root.join(sid).join(SESSION_FILE), MAX_RECORD)?
                    .ok_or(INVALID)?;
                let record = decode(&raw, sid)?;
                idle(&record)?;
                add_member(
                    &mut writer,
                    &mut manifest,
                    &mut total,
                    format!("{sid}/{SESSION_FILE}"),
                    clean_json(raw, secrets)?,
                )?;
                if let Some(native) = read_file(&self.native_history_path(sid)?, MAX_NATIVE)? {
                    add_member(
                        &mut writer,
                        &mut manifest,
                        &mut total,
                        format!("{sid}/{NATIVE_HISTORY_FILE}"),
                        clean_json(native, secrets)?,
                    )?;
                }
                for file in &record.attachments {
                    validate_id(&file.id)?;
                    let bytes =
                        read_file(&self.attachment_root(sid)?.join(&file.id), MAX_ATTACHMENT)?
                            .ok_or(INVALID)?;
                    if bytes.len() as u64 != file.size_bytes {
                        return Err(INVALID.into());
                    }
                    if contains_key(&bytes, secrets) {
                        return Err(CONTAINS_KEY.into());
                    }
                    add_member(
                        &mut writer,
                        &mut manifest,
                        &mut total,
                        format!("{sid}/{ATTACHMENTS_DIR}/{}", file.id),
                        bytes,
                    )?;
                }
                validate_session(&self.root, sid, &manifest.files.keys().cloned().collect())
            })?;
        }
        if let Some(prefs) = read_file(&self.root.join(PREFS_FILE), limit(PREFS_FILE)?)? {
            let value: Prefs = serde_json::from_slice(&prefs).map_err(|_| INVALID)?;
            if value.version != FORMAT_VERSION {
                return Err(INVALID.into());
            }
            add_member(
                &mut writer,
                &mut manifest,
                &mut total,
                PREFS_FILE.into(),
                clean_json(serde_json::to_vec(&value).map_err(|_| FAILED)?, secrets)?,
            )?;
        }
        let mut after = if real_dir(&self.root)? {
            self.session_ids()?
        } else {
            vec![]
        };
        after.sort();
        if after != sessions {
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
        let bytes = read_bounded(zip, MAX_ZIP)?;
        admit()?;
        let path = super::super::transfers::destination(&path, &self.config_dir)?;
        crate::config::atomic_write_with(
            &path,
            &bytes,
            crate::config::AtomicOpts { mode: Some(0o600) },
        )
        .map_err(|_| FAILED.to_string())
    }

    /// Validate the complete archive, then add missing sessions. Never replace
    /// a local conversation or its preferences. Publication failure after some
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
        let file = fs::File::open(path).map_err(|_| INVALID)?;
        if file.metadata().map_err(|_| INVALID)?.len() > MAX_ZIP {
            return Err(TOO_LARGE.into());
        }
        let mut zip = zip::ZipArchive::new(file).map_err(|_| INVALID)?;
        if zip.len() > MAX_FILES {
            return Err(TOO_LARGE.into());
        }
        let mut names = HashSet::new();
        let mut declared = 0u64;
        for index in 0..zip.len() {
            let entry = zip.by_index(index).map_err(|_| INVALID)?;
            let max = limit(entry.name())?;
            if !entry.is_file()
                || entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
                || !names.insert(entry.name().to_string())
            {
                return Err(INVALID.into());
            }
            declared = declared.checked_add(entry.size()).ok_or(TOO_LARGE)?;
            if entry.size() > max || declared > MAX_TOTAL {
                return Err(TOO_LARGE.into());
            }
        }
        let manifest_bytes =
            read_bounded(zip.by_name(MANIFEST).map_err(|_| INVALID)?, MAX_MANIFEST)?;
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
        let root = self.ensure_root()?;
        let staging = tempfile::Builder::new()
            .prefix(&format!(".restore-{}-", id()))
            .tempdir_in(&root)
            .map_err(|_| FAILED)?;
        let mut total = manifest_bytes.len() as u64;
        for (name, fingerprint) in &manifest.files {
            admit()?;
            let bytes = read_bounded(zip.by_name(name).map_err(|_| INVALID)?, limit(name)?)?;
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
            validate_session(staging.path(), sid, &names)?;
        }
        if names.contains(PREFS_FILE) {
            let prefs: Prefs = serde_json::from_slice(
                &read_file(&staging.path().join(PREFS_FILE), limit(PREFS_FILE)?)?.ok_or(INVALID)?,
            )
            .map_err(|_| INVALID)?;
            if prefs.version != FORMAT_VERSION {
                return Err(INVALID.into());
            }
        }
        // Preflight all conflicts before publishing the first conversation.
        for sid in &manifest.sessions {
            self.with_attachment_lock(sid, || {
                let lock = file_lock(&root.join(sid).join(SESSION_FILE));
                let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                admit()?;
                self.same_backup_session(sid, &manifest).map(|_| ())
            })?;
        }
        let mut report = RestoreReport::default();
        for sid in &manifest.sessions {
            let result = self.with_attachment_lock(sid, || {
                let lock = file_lock(&root.join(sid).join(SESSION_FILE));
                let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                admit()?;
                if self.same_backup_session(sid, &manifest)? {
                    return Ok(false);
                }
                fs::rename(staging.path().join(sid), root.join(sid)).map_err(|_| FAILED)?;
                Ok(true)
            });
            match result {
                Ok(true) => report.imported += 1,
                Ok(false) => report.skipped += 1,
                Err(error) if report.imported > 0 => {
                    report.warning = Some(error);
                    return Ok(report);
                }
                Err(error) => return Err(error),
            }
        }
        // Existing preferences win, including unreadable ones. A failed optional
        // preference import does not turn successful session imports into failure.
        if names.contains(PREFS_FILE) {
            let result = (|| {
                let path = root.join(PREFS_FILE);
                let lock = file_lock(&path);
                let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                admit()?;
                ensure_regular_or_absent(&path)?;
                if !path.exists() {
                    write_bytes(
                        &path,
                        &read_file(&staging.path().join(PREFS_FILE), limit(PREFS_FILE)?)?
                            .ok_or(INVALID)?,
                    )?;
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

    fn same_backup_session(&self, sid: &str, manifest: &Manifest) -> Result<bool, String> {
        if !real_dir(&self.root.join(sid))? {
            return Ok(false);
        }
        let record = read_record(&self.root, sid).map_err(|_| CONFLICT)?;
        idle(&record)?;
        let prefix = format!("{sid}/");
        for (name, expected) in manifest
            .files
            .iter()
            .filter(|(name, _)| name.starts_with(&prefix))
        {
            let bytes = read_file(&self.root.join(name), limit(name)?)?.ok_or(CONFLICT)?;
            if digest(&bytes) != *expected {
                return Err(CONFLICT.into());
            }
        }
        let native = format!("{sid}/{NATIVE_HISTORY_FILE}");
        if !manifest.files.contains_key(&native)
            && read_file(&self.root.join(native), MAX_NATIVE)?.is_some()
        {
            return Err(CONFLICT.into());
        }
        Ok(true)
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
            record.attachments.push(Attachment { id:attachment.clone(), name:"中文.txt".into(), mime_type:"text/plain".into(),size_bytes:12,status:"ready".into(),reason:None,preview_url:None,details:None });
            record.usage = json!({"modelRequests":1,"estimatedCostUsd":"0.123456789123456789","searchCount":1});
            record.operations.push(Operation {
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
        fs::write(store.root.join(PREFS_FILE),serde_json::to_vec(&json!({"version":1,"modelId":"test/model","unknownCredential":"must-not-export-extra"})).unwrap()).unwrap();
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
        assert!(store
            .load_record(&sid)
            .unwrap()
            .session
            .draft
            .contains(&key));
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
    fn backup_roundtrip_keeps_native_history_drafts_files_usage_and_operation_prices() {
        let (_source, store, sid, attachment) = seeded();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("完整备份.zip");
        store.backup(&archive, &[], || Ok(())).unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
        assert!(!zip
            .file_names()
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
    fn backup_restore_refuses_other_owner_conflicts_and_stale_authority_without_overwrite() {
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
        assert_eq!(
            store.restore(&archive, &[], || Ok(())).unwrap_err(),
            "assistant_backup_conflict"
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
    fn backup_rejects_missing_files_and_live_runs_and_keeps_existing_output() {
        let (_tmp, store, sid, attachment) = seeded();
        let output = tempfile::tempdir().unwrap();
        let archive = output.path().join("backup.zip");
        fs::write(&archive, "keep old backup").unwrap();
        fs::remove_file(store.attachment_root(&sid).unwrap().join(attachment)).unwrap();
        assert!(store.backup(&archive, &[], || Ok(())).is_err());
        assert_eq!(fs::read_to_string(&archive).unwrap(), "keep old backup");
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
            "assistant_session_busy"
        );
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
            let name = entry.name().to_string();
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
        assert_eq!((retry.imported, retry.skipped), (1, 1));
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
            let name = file.name().to_string();
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
}
