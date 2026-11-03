//! Explicit native acceptance. Uses a unique OS credential entry, never an app
//! credential or journal key, and deletes that entry on completion.
#![cfg(any(target_os = "macos", target_os = "windows"))]

use crate::{
    ofox_secret::{SecretStore, Slot},
    services::assistant::tools::journal::{key_from_store, Journal},
};
use std::{path::PathBuf, process::Command};

struct IsolatedKeychain {
    service: String,
    #[cfg(target_os = "macos")]
    path: PathBuf,
}
#[cfg(target_os = "windows")]
impl IsolatedKeychain {
    fn entry(&self, slot: Slot) -> Result<keyring::Entry, String> {
        assert_eq!(slot, Slot::AssistantJournalKey);
        assert!(self.service.starts_with("ai.ofox.desktop.acceptance."));
        keyring::Entry::new(&self.service, "snapshot-encryption")
            .map_err(|_| "acceptance_keychain_entry_failed".into())
    }
}
#[cfg(target_os = "windows")]
impl SecretStore for IsolatedKeychain {
    fn load(&self, slot: Slot) -> Result<Option<String>, String> {
        match self.entry(slot)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("acceptance_keychain_read_failed: {error:?}")),
        }
    }
    fn save(&self, slot: Slot, value: &str) -> Result<(), String> {
        self.entry(slot)?
            .set_password(value)
            .map_err(|error| format!("acceptance_keychain_write_failed: {error:?}"))
    }
    fn clear(&self, slot: Slot) -> Result<(), String> {
        match self.entry(slot)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("acceptance_keychain_delete_failed".into()),
        }
    }
}

#[cfg(target_os = "macos")]
impl IsolatedKeychain {
    fn keychain(
        &self,
        slot: Slot,
    ) -> Result<security_framework::os::macos::keychain::SecKeychain, String> {
        assert_eq!(slot, Slot::AssistantJournalKey);
        assert!(self.service.starts_with("ai.ofox.desktop.acceptance."));
        security_framework::os::macos::keychain::SecKeychain::open(&self.path)
            .map_err(|error| format!("acceptance_keychain_open_failed: {error:?}"))
    }
}
#[cfg(target_os = "macos")]
impl SecretStore for IsolatedKeychain {
    fn load(&self, slot: Slot) -> Result<Option<String>, String> {
        match self
            .keychain(slot)?
            .find_generic_password(&self.service, "snapshot-encryption")
        {
            Ok((password, _)) => String::from_utf8(password.to_vec())
                .map(Some)
                .map_err(|_| "acceptance_password_encoding_failed".into()),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(error) => Err(format!("acceptance_keychain_read_failed: {error:?}")),
        }
    }
    fn save(&self, slot: Slot, value: &str) -> Result<(), String> {
        self.keychain(slot)?
            .set_generic_password(&self.service, "snapshot-encryption", value.as_bytes())
            .map_err(|error| format!("acceptance_keychain_write_failed: {error:?}"))
    }
    fn clear(&self, slot: Slot) -> Result<(), String> {
        match self
            .keychain(slot)?
            .find_generic_password(&self.service, "snapshot-encryption")
        {
            Ok((_, item)) => {
                item.delete();
                Ok(())
            }
            Err(error) if error.code() == -25300 => Ok(()),
            Err(error) => Err(format!("acceptance_keychain_delete_failed: {error:?}")),
        }
    }
}

#[test]
#[ignore = "explicit native OS credential acceptance; creates and removes a unique test entry"]
fn os_keychain_restores_encrypted_snapshot_in_a_new_process() {
    const ORIGINAL: &str = "original-test-credential-canary";
    if let Ok(phase) = std::env::var("OFOX_KEYCHAIN_ACCEPTANCE_PHASE") {
        let root = PathBuf::from(std::env::var_os("OFOX_KEYCHAIN_ACCEPTANCE_ROOT").unwrap());
        let store = IsolatedKeychain {
            service: std::env::var("OFOX_KEYCHAIN_ACCEPTANCE_SERVICE").unwrap(),
            #[cfg(target_os = "macos")]
            path: root.join("acceptance.keychain-db"),
        };
        let file = root.join("tool.json");
        if phase == "restore" {
            // The second process must find the existing key, not create one.
            assert!(store.load(Slot::AssistantJournalKey).unwrap().is_some());
        }
        let key = key_from_store(&store).unwrap();
        let mut journal = Journal::open(&root.join("changes"), &key, "acceptance/session").unwrap();
        match phase.as_str() {
            "write" => {
                std::fs::write(&file, ORIGINAL).unwrap();
                let id = journal
                    .prepare(
                        "write",
                        std::slice::from_ref(&file),
                        serde_json::Value::Null,
                    )
                    .unwrap();
                std::fs::write(&file, "changed").unwrap();
                journal.finish(&id, serde_json::Value::Null).unwrap();
                let bytes = std::fs::read(root.join("changes/journal.enc")).unwrap();
                assert!(!bytes
                    .windows(ORIGINAL.len())
                    .any(|chunk| chunk == ORIGINAL.as_bytes()));
            }
            "restore" => {
                assert_eq!(std::fs::read_to_string(&file).unwrap(), "changed");
                journal.restore_files().unwrap();
                assert_eq!(std::fs::read_to_string(&file).unwrap(), ORIGINAL);
                assert!(journal.changes().iter().all(|change| change.restored));
            }
            _ => panic!("unknown acceptance phase"),
        }
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let store = IsolatedKeychain {
        service: format!("ai.ofox.desktop.acceptance.{}", uuid::Uuid::new_v4()),
        #[cfg(target_os = "macos")]
        path: root.path().join("acceptance.keychain-db"),
    };
    #[cfg(target_os = "macos")]
    security_framework::os::macos::keychain::CreateOptions::new()
        .password("isolated-acceptance-only")
        .prompt_user(false)
        .create(&store.path)
        .unwrap();
    struct Cleanup<'a>(&'a IsolatedKeychain);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.0.clear(Slot::AssistantJournalKey);
        }
    }
    let _cleanup = Cleanup(&store);
    assert!(store.load(Slot::AssistantJournalKey).unwrap().is_none());
    for phase in ["write", "restore"] {
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "services::assistant::tools::keychain_acceptance::os_keychain_restores_encrypted_snapshot_in_a_new_process",
                "--ignored",
                "--nocapture",
            ])
            .env("OFOX_KEYCHAIN_ACCEPTANCE_PHASE", phase)
            .env("OFOX_KEYCHAIN_ACCEPTANCE_ROOT", root.path())
            .env("OFOX_KEYCHAIN_ACCEPTANCE_SERVICE", &store.service)
            .status()
            .unwrap();
        assert!(status.success(), "native acceptance phase failed: {phase}");
    }
    assert_eq!(
        std::fs::read_to_string(root.path().join("tool.json")).unwrap(),
        ORIGINAL
    );
    store.clear(Slot::AssistantJournalKey).unwrap();
    assert!(store.load(Slot::AssistantJournalKey).unwrap().is_none());
}
