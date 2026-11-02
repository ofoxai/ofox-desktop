//! Encrypted local change journal. Plaintext originals never leave Rust.
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    Aes256Gcm, Nonce,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
const FAILED: &str = "assistant_journal_unavailable";
const CONFLICT: &str = "assistant_restore_conflict";
const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_JOURNAL: u64 = 32 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub struct FileState {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
    mode: Option<u32>,
    readonly: bool,
    missing_parents: Vec<PathBuf>,
}
impl FileState {
    fn capture(path: &Path) -> Result<Self, String> {
        if super::scope::resolve(path)? != path {
            return Err(CONFLICT.into());
        }
        let mut missing_parents = Vec::new();
        let mut parent = path.parent();
        while let Some(dir) = parent {
            if dir.exists() {
                break;
            }
            missing_parents.push(dir.to_path_buf());
            parent = dir.parent();
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(value) => Some(value),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(FAILED.into()),
        };
        if metadata
            .as_ref()
            .is_some_and(|m| !m.is_file() || m.len() > MAX_FILE)
        {
            return Err(FAILED.into());
        }
        let bytes = if metadata.is_some() {
            Some({
                use std::io::Read;
                let mut bytes = Vec::new();
                fs::File::open(path)
                    .map_err(|_| FAILED)?
                    .take(MAX_FILE + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| FAILED)?;
                bytes
            })
        } else {
            None
        };
        if bytes.as_ref().is_some_and(|b| b.len() as u64 > MAX_FILE) {
            return Err(FAILED.into());
        }
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.as_ref().map(|m| m.permissions().mode() & 0o777)
        };
        #[cfg(not(unix))]
        let mode = None;
        Ok(Self {
            path: path.into(),
            bytes,
            mode,
            readonly: metadata.is_some_and(|m| m.permissions().readonly()),
            missing_parents,
        })
    }
    fn digest(&self) -> Result<String, String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&self.path, &self.bytes, self.mode, self.readonly))
                    .map_err(|_| FAILED)?
            )
        ))
    }
    fn restore(&self) -> Result<(), String> {
        if super::scope::resolve(&self.path)? != self.path {
            return Err(CONFLICT.into());
        }
        if let Some(bytes) = &self.bytes {
            crate::config::atomic_write_with(
                &self.path,
                bytes,
                crate::config::AtomicOpts { mode: self.mode },
            )
            .map_err(|_| FAILED)?;
            #[cfg(windows)]
            {
                let mut permissions = fs::metadata(&self.path).map_err(|_| FAILED)?.permissions();
                permissions.set_readonly(self.readonly);
                fs::set_permissions(&self.path, permissions).map_err(|_| FAILED)?;
            }
        } else if self.path.exists() {
            fs::remove_file(&self.path).map_err(|_| FAILED)?;
        }
        for parent in &self.missing_parents {
            match fs::remove_dir(parent) {
                Ok(()) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(_) => return Err(FAILED.into()),
            }
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    id: String,
    operation: String,
    before: Vec<FileState>,
    after: Option<Vec<String>>,
    before_state: Value,
    after_state: Option<Value>,
    restored: bool,
}
#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    version: u32,
    restoring: bool,
    entries: Vec<Entry>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub id: String,
    pub operation: String,
    pub paths: Vec<PathBuf>,
    pub restored: bool,
    pub outcome_known: bool,
    pub changed: bool,
}

pub struct Journal {
    path: PathBuf,
    cipher: Aes256Gcm,
    context: String,
    ledger: Ledger,
}
impl Journal {
    pub fn open(directory: &Path, key: &[u8; 32], context: &str) -> Result<Self, String> {
        let path = directory.join("journal.enc");
        let cipher = Aes256Gcm::new(key.into());
        let ledger = if path.exists() {
            let metadata = fs::symlink_metadata(&path).map_err(|_| FAILED)?;
            if !metadata.is_file() || metadata.len() > MAX_JOURNAL {
                return Err(FAILED.into());
            }
            let bytes = fs::read(&path).map_err(|_| FAILED)?;
            if bytes.len() < 28 {
                return Err(FAILED.into());
            }
            let plaintext = zeroize::Zeroizing::new(
                cipher
                    .decrypt(
                        Nonce::from_slice(&bytes[..12]),
                        Payload {
                            msg: &bytes[12..],
                            aad: context.as_bytes(),
                        },
                    )
                    .map_err(|_| FAILED)?,
            );
            let ledger: Ledger = serde_json::from_slice(&plaintext).map_err(|_| FAILED)?;
            if ledger.version != 1 || ledger.entries.len() > 512 {
                return Err(FAILED.into());
            }
            ledger
        } else {
            Ledger {
                version: 1,
                ..Ledger::default()
            }
        };
        Ok(Self {
            path,
            cipher,
            context: context.into(),
            ledger,
        })
    }
    fn save(&self) -> Result<(), String> {
        let plain = zeroize::Zeroizing::new(serde_json::to_vec(&self.ledger).map_err(|_| FAILED)?);
        if plain.len() as u64 > MAX_JOURNAL - 28 {
            return Err(FAILED.into());
        }
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let mut bytes = nonce.to_vec();
        bytes.extend(
            self.cipher
                .encrypt(
                    &nonce,
                    Payload {
                        msg: &plain,
                        aad: self.context.as_bytes(),
                    },
                )
                .map_err(|_| FAILED)?,
        );
        crate::config::create_private_dir_all(self.path.parent().ok_or(FAILED)?)
            .map_err(|_| FAILED)?;
        crate::config::atomic_write_with(
            &self.path,
            &bytes,
            crate::config::AtomicOpts { mode: Some(0o600) },
        )
        .map_err(|_| FAILED.into())
    }
    pub fn prepare(
        &mut self,
        operation: &str,
        paths: &[PathBuf],
        state: Value,
    ) -> Result<String, String> {
        if self.ledger.restoring || self.ledger.entries.len() >= 512 || paths.len() > 64 {
            return Err(FAILED.into());
        }
        if self
            .ledger
            .entries
            .iter()
            .any(|e| !e.restored && e.after.is_none())
        {
            return Err("unknown_outcome".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let before = paths
            .iter()
            .map(|p| {
                if fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
                    return Err(CONFLICT.into());
                }
                FileState::capture(&super::scope::resolve(p)?)
            })
            .collect::<Result<_, _>>()?;
        self.ledger.entries.push(Entry {
            id: id.clone(),
            operation: operation.into(),
            before,
            after: None,
            before_state: state,
            after_state: None,
            restored: false,
        });
        self.save()?;
        Ok(id)
    }
    pub fn finish(&mut self, id: &str, state: Value) -> Result<(), String> {
        let entry = self
            .ledger
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(FAILED)?;
        entry.after = Some(
            entry
                .before
                .iter()
                .map(|f| FileState::capture(&f.path)?.digest())
                .collect::<Result<_, _>>()?,
        );
        entry.after_state = Some(state);
        self.save()
    }
    pub fn changes(&self) -> Vec<Change> {
        self.ledger
            .entries
            .iter()
            .map(|e| Change {
                id: e.id.clone(),
                operation: e.operation.clone(),
                paths: e.before.iter().map(|f| f.path.clone()).collect(),
                restored: e.restored,
                outcome_known: e.after.is_some(),
                changed: e.after.as_ref().is_none_or(|after| {
                    e.before
                        .iter()
                        .zip(after)
                        .any(|(before, after)| before.digest().is_ok_and(|digest| digest != *after))
                }) || e
                    .after_state
                    .as_ref()
                    .is_some_and(|after| after != &e.before_state),
            })
            .collect()
    }
    /// Validate the complete reverse chain before changing any file. Repeated
    /// writes to a path are collapsed to its first original, so partial restore
    /// can resume safely after restart once the restoring marker was saved.
    fn restore_plan(&self) -> Result<BTreeMap<PathBuf, FileState>, String> {
        let mut first = BTreeMap::new();
        let mut expected = BTreeMap::new();
        for entry in self.ledger.entries.iter().filter(|e| !e.restored) {
            let after = entry.after.as_ref().ok_or("unknown_outcome")?;
            if after.len() != entry.before.len() {
                return Err(FAILED.into());
            }
            for (before, after) in entry.before.iter().zip(after) {
                if let Some(previous) = expected.get(&before.path) {
                    if *previous != before.digest()? {
                        return Err(CONFLICT.into());
                    }
                }
                first
                    .entry(before.path.clone())
                    .or_insert_with(|| before.clone());
                expected.insert(before.path.clone(), after.clone());
            }
        }
        for (path, after) in expected {
            let actual = FileState::capture(&path)?.digest()?;
            if actual != after && !(self.ledger.restoring && actual == first[&path].digest()?) {
                return Err(CONFLICT.into());
            }
        }
        Ok(first)
    }
    pub fn restore_files(&mut self) -> Result<(), String> {
        if self
            .ledger
            .entries
            .iter()
            .any(|e| !e.restored && !e.before_state.is_null())
        {
            return Err(FAILED.into());
        }
        self.restore_with(|_| Ok(Value::Null), |_, _| Ok(()))
    }
    pub fn restore_with(
        &mut self,
        read_state: impl Fn(&Value) -> Result<Value, String>,
        restore_state: impl Fn(&Value, &Value) -> Result<(), String>,
    ) -> Result<(), String> {
        let files = self.restore_plan()?;
        // Extra state is keyed by its scope field (tool-specific DB/settings).
        let mut states: BTreeMap<String, (Value, Value)> = BTreeMap::new();
        for entry in self
            .ledger
            .entries
            .iter()
            .filter(|e| !e.restored && !e.before_state.is_null())
        {
            let scope = entry.before_state["scope"]
                .as_str()
                .ok_or(FAILED)?
                .to_owned();
            let after = entry.after_state.as_ref().ok_or("unknown_outcome")?;
            if let Some((_, previous)) = states.get(&scope) {
                if previous != &entry.before_state {
                    return Err(CONFLICT.into());
                }
            }
            states
                .entry(scope)
                .and_modify(|(_, last)| *last = after.clone())
                .or_insert((entry.before_state.clone(), after.clone()));
        }
        for (before, after) in states.values() {
            let actual = read_state(before)?;
            if actual != *after && !(self.ledger.restoring && actual == *before) {
                return Err(CONFLICT.into());
            }
        }
        self.ledger.restoring = true;
        self.save()?;
        for file in files.values() {
            file.restore()?;
        }
        for (before, after) in states.values() {
            restore_state(before, after)?;
        }
        for entry in &mut self.ledger.entries {
            entry.restored = true;
        }
        self.ledger.restoring = false;
        self.save()
    }
}

/// Unlike application debug credentials, this key must never use FileStore.
/// A mutex makes first creation atomic across concurrent sessions in this app.
pub fn system_key() -> Result<zeroize::Zeroizing<[u8; 32]>, String> {
    use crate::ofox_secret::KeyringStore;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().map_err(|_| FAILED)?;
    key_from_store(&KeyringStore::new())
}
pub fn key_from_store(
    store: &dyn crate::ofox_secret::SecretStore,
) -> Result<zeroize::Zeroizing<[u8; 32]>, String> {
    use crate::ofox_secret::Slot;
    use base64::Engine;
    let key = match store
        .load(Slot::AssistantJournalKey)
        .map_err(|_| "assistant_key_storage_unavailable")?
    {
        Some(encoded) => {
            let bytes = zeroize::Zeroizing::new(
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|_| FAILED)?,
            );
            <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| FAILED)?
        }
        None => {
            let key = Aes256Gcm::generate_key(&mut OsRng);
            let encoded =
                zeroize::Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(key));
            store
                .save(Slot::AssistantJournalKey, &encoded)
                .map_err(|_| "assistant_key_storage_unavailable")?;
            key.into()
        }
    };
    Ok(zeroize::Zeroizing::new(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_key_store_reuses_a_persistent_key_without_using_assistant_credentials() {
        use crate::ofox_secret::{InMemoryStore, SecretStore, Slot};
        let store = InMemoryStore::default();
        let first = key_from_store(&store).unwrap();
        let second = key_from_store(&store).unwrap();
        assert!(*first == *second);
        assert!(store.load(Slot::AssistantJournalKey).unwrap().is_some());
        assert!(store.load(Slot::AssistantKey).unwrap().is_none());
    }

    #[test]
    fn journal_survives_restart_without_plaintext_and_restores_modes() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("settings.json");
        let original = b"{\"api_key\":\"private-third-party-secret\"}";
        std::fs::write(&file, original).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        let directory = root.path().join("changes");
        let key = [7u8; 32];
        let mut journal = Journal::open(&directory, &key, "owner/session").unwrap();
        let id = journal
            .prepare(
                "write",
                std::slice::from_ref(&file),
                serde_json::Value::Null,
            )
            .unwrap();
        std::fs::write(&file, b"changed").unwrap();
        journal.finish(&id, serde_json::Value::Null).unwrap();
        drop(journal);
        let bytes = std::fs::read(directory.join("journal.enc")).unwrap();
        assert!(!bytes.windows(original.len()).any(|w| w == original));
        let mut journal = Journal::open(&directory, &key, "owner/session").unwrap();
        journal.restore_files().unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), original);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
        assert!(journal.changes().iter().all(|c| c.restored));
    }
    #[test]
    fn journal_conflicts_are_detected_before_any_restoration() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("changes");
        let a = root.path().join("a");
        let b = root.path().join("b");
        std::fs::write(&a, "original").unwrap();
        let mut journal = Journal::open(&directory, &[1; 32], "context").unwrap();
        let id = journal
            .prepare("write", &[a.clone(), b.clone()], serde_json::Value::Null)
            .unwrap();
        std::fs::write(&a, "assistant").unwrap();
        std::fs::write(&b, "new").unwrap();
        journal.finish(&id, serde_json::Value::Null).unwrap();
        std::fs::write(&a, "user edit").unwrap();
        assert_eq!(
            journal.restore_files().unwrap_err(),
            "assistant_restore_conflict"
        );
        assert_eq!(std::fs::read_to_string(b).unwrap(), "new");
    }
    #[test]
    fn journal_repeated_writes_restore_to_the_first_original() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("changes");
        let file = root.path().join("a");
        let mut journal = Journal::open(&directory, &[1; 32], "context").unwrap();
        for value in ["one", "two"] {
            let id = journal
                .prepare(
                    "write",
                    std::slice::from_ref(&file),
                    serde_json::Value::Null,
                )
                .unwrap();
            std::fs::write(&file, value).unwrap();
            journal.finish(&id, serde_json::Value::Null).unwrap();
        }
        journal.restore_files().unwrap();
        assert!(!file.exists());
    }
    #[test]
    fn journal_rejects_tampering_wrong_key_and_foreign_session() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("a");
        let directory = root.path().join("changes");
        let mut journal = Journal::open(&directory, &[1; 32], "context").unwrap();
        journal
            .prepare("write", &[file], serde_json::Value::Null)
            .unwrap();
        assert!(Journal::open(&directory, &[2; 32], "context").is_err());
        assert!(Journal::open(&directory, &[1; 32], "other-session").is_err());
        let path = directory.join("journal.enc");
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(path, bytes).unwrap();
        assert!(Journal::open(&directory, &[1; 32], "context").is_err());
    }
    #[test]
    fn incomplete_journal_entry_is_not_automatically_replayed_or_restored() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("a");
        let directory = root.path().join("changes");
        let mut journal = Journal::open(&directory, &[1; 32], "context").unwrap();
        journal
            .prepare("write", &[file], serde_json::Value::Null)
            .unwrap();
        assert_eq!(journal.restore_files().unwrap_err(), "unknown_outcome");
    }
}
