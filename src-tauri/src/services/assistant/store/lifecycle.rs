//! Permanent deletion starts with an atomic move out of the session namespace.
//! A crash during physical removal leaves a private tombstone for startup retry.
use super::*;

const DELETING: &str = ".deleting-";
const DELETE_PENDING: &str = "assistant_delete_pending";

impl AssistantStore {
    /// Only an already trashed, idle session can leave the namespace. All
    /// imports and record writers finish before the atomic move. A late send
    /// then sees a missing session and cannot recreate or read its files.
    pub fn delete_trashed(
        &self,
        sid: &str,
        precondition: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        self.delete_trashed_using(sid, precondition, |path| fs::remove_dir_all(path))
    }

    fn delete_trashed_using(
        &self,
        sid: &str,
        precondition: impl FnOnce() -> Result<(), String>,
        remove: impl FnOnce(&Path) -> std::io::Result<()>,
    ) -> Result<(), String> {
        self.with_attachment_lock(sid, || {
            if !real_dir(&self.root)? {
                return Err(NOT_FOUND.into());
            }
            let source = self.root.join(sid);
            let tombstone = self.root.join(format!("{DELETING}{sid}"));
            let lock = file_lock(&source.join(SESSION_FILE));
            let _guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
            precondition()?;
            if real_dir(&source)? {
                let record = read_record(&self.root, sid)?;
                if record.session.deleted_at.is_none() {
                    return Err("assistant_session_not_trashed".into());
                }
                if record.session.active_run.is_some() {
                    return Err("assistant_session_busy".into());
                }
                if record
                    .messages
                    .iter()
                    .filter_map(|message| message.media.as_ref())
                    .any(|media| {
                        media.kind == "video"
                            && super::super::video::occupies_generation_slot(&media.status)
                    })
                {
                    return Err("assistant_media_delete_pending".into());
                }
                // An earlier deletion of this id never finished, and the
                // conversation was restored since: its old copy goes first.
                if real_dir(&tombstone)? {
                    fs::remove_dir_all(&tombstone).map_err(|_| UNAVAILABLE)?;
                }
                crate::config::rename_dir(&source, &tombstone).map_err(|_| UNAVAILABLE)?;
            } else if !real_dir(&tombstone)? {
                return Err(NOT_FOUND.into());
            }
            // std::fs does not follow links found within the tree. The root
            // itself must be a real directory, including on Windows.
            remove(&tombstone).map_err(|_| DELETE_PENDING.to_string())
        })
    }

    /// Remove what an interrupted deletion of `sid` left: an old copy no
    /// record points at, even when the conversation was restored since.
    /// Requires the attachment lock. A link in its place is refused.
    pub(super) fn finish_pending_delete(&self, sid: &str) -> Result<(), String> {
        let tombstone = self.root.join(format!("{DELETING}{sid}"));
        if real_dir(&tombstone)? {
            fs::remove_dir_all(&tombstone).map_err(|_| DELETE_PENDING)?;
        }
        Ok(())
    }

    /// Retry only this store's well-formed tombstones, never foreign names or
    /// links. Successful calls leave no session bytes behind.
    pub fn cleanup_deleted(&self) -> Result<(), String> {
        if !real_dir(&self.root)? {
            return Ok(());
        }
        let mut failure = None;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|_| UNAVAILABLE)? {
            match entry {
                Ok(entry) => entries.push(entry),
                Err(_) => {
                    failure.get_or_insert_with(|| UNAVAILABLE.to_string());
                }
            }
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            let Some(sid) = name.to_str().and_then(|name| name.strip_prefix(DELETING)) else {
                continue;
            };
            if validate_id(sid).is_err() {
                continue;
            }
            let result = self.with_attachment_lock(sid, || self.finish_pending_delete(sid));
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, AssistantStore, String) {
        let temp = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(temp.path(), &"a".repeat(64)).unwrap();
        let sid = store.create("test/model", false).unwrap().session.id;
        (temp, store, sid)
    }

    fn trash(store: &AssistantStore, sid: &str) {
        store
            .update(
                &serde_json::from_value(json!({"sessionId":sid,"deleted":true})).unwrap(),
                || Ok(()),
            )
            .unwrap();
    }

    #[test]
    fn permanent_deletion_removes_native_history_files_and_runtime_state() {
        let (_temp, store, sid) = fixture();
        fs::write(store.native_history_path(&sid).unwrap(), b"native history").unwrap();
        fs::write(store.attachment_dir(&sid).unwrap().join(id()), b"copy").unwrap();
        let settings = store.root.join(&sid).join("runtime-settings");
        fs::create_dir(&settings).unwrap();
        fs::write(settings.join("settings.json"), b"{}").unwrap();
        trash(&store, &sid);
        store.delete_trashed(&sid, || Ok(())).unwrap();
        assert!(!store.root.join(&sid).exists());
        assert!(store.list(true, "").unwrap().sessions.is_empty());
        assert_eq!(fs::read_dir(&store.root).unwrap().count(), 0);
    }

    #[test]
    fn deletion_refuses_live_untrashed_and_wrong_authority_sessions() {
        let (_temp, store, sid) = fixture();
        assert_eq!(
            store.delete_trashed(&sid, || Ok(())).unwrap_err(),
            "assistant_session_not_trashed"
        );
        trash(&store, &sid);
        assert_eq!(
            store
                .delete_trashed(&sid, || Err("assistant_identity_changed".into()))
                .unwrap_err(),
            "assistant_identity_changed"
        );
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
            store.delete_trashed(&sid, || Ok(())).unwrap_err(),
            "assistant_session_busy"
        );
        assert!(store.load_record(&sid).is_ok());
    }

    #[test]
    fn deletion_preserves_unresolved_remote_video_jobs_but_allows_finished_downloads() {
        for status in [
            "submitting",
            "queued",
            "in_progress",
            "paused",
            "unknown",
            "query_expired",
            "downloading",
            "download_failed",
            "completed",
            "cancelled",
            "failed",
            "expired",
        ] {
            let (_temp, store, sid) = fixture();
            store
                .mutate(&sid, |record| {
                    record.messages.push(serde_json::from_value(json!({
                    "id":id(),"role":"media","content":"","createdAt":1,"status":status,
                    "media":{"kind":"video","model":"test/video","params":{},"status":status}
                })).unwrap());
                    Ok(())
                })
                .unwrap();
            trash(&store, &sid);
            let result = store.delete_trashed(&sid, || Ok(()));
            if matches!(
                status,
                "downloading"
                    | "download_failed"
                    | "completed"
                    | "cancelled"
                    | "failed"
                    | "expired"
            ) {
                result.unwrap();
            } else {
                assert_eq!(
                    result.unwrap_err(),
                    "assistant_media_delete_pending",
                    "{status}"
                );
                assert!(store.load_record(&sid).is_ok());
            }
        }
    }

    #[test]
    fn a_partial_removal_stays_hidden_and_can_be_retried_without_its_record() {
        let (_temp, store, sid) = fixture();
        trash(&store, &sid);
        fs::write(store.native_history_path(&sid).unwrap(), b"native").unwrap();
        let error = store
            .delete_trashed_using(
                &sid,
                || Ok(()),
                |path| {
                    fs::remove_file(path.join(SESSION_FILE))?;
                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                },
            )
            .unwrap_err();
        assert_eq!(error, DELETE_PENDING);
        assert!(!store.root.join(&sid).exists());
        assert!(store.list(true, "").unwrap().unreadable.is_empty());
        assert!(store.list(true, "").unwrap().sessions.is_empty());
        store.delete_trashed(&sid, || Ok(())).unwrap();
        assert_eq!(fs::read_dir(&store.root).unwrap().count(), 0);
    }

    #[test]
    fn startup_finishes_interrupted_deletion_and_preserves_foreign_folders() {
        let (_temp, store, sid) = fixture();
        trash(&store, &sid);
        let tombstone = store.root.join(format!(".deleting-{sid}"));
        fs::rename(store.root.join(&sid), &tombstone).unwrap();
        fs::write(tombstone.join("leftover"), b"private").unwrap();
        let foreign = store.root.join(".deleting-not-a-session");
        fs::create_dir(&foreign).unwrap();
        store.cleanup_deleted().unwrap();
        store.cleanup_deleted().unwrap();
        assert!(!tombstone.exists());
        assert!(foreign.is_dir());
    }

    #[test]
    fn an_old_tombstone_of_the_same_id_does_not_block_deleting_it_again() {
        let (_temp, store, sid) = fixture();
        // Left by an earlier deletion of this id; the conversation came back
        // from a backup since.
        let tombstone = store.root.join(format!(".deleting-{sid}"));
        fs::create_dir(&tombstone).unwrap();
        fs::write(tombstone.join("leftover"), b"old copy").unwrap();
        trash(&store, &sid);
        store.delete_trashed(&sid, || Ok(())).unwrap();
        assert_eq!(fs::read_dir(&store.root).unwrap().count(), 0);
    }

    #[test]
    fn startup_removes_a_tombstone_beside_a_restored_conversation() {
        let (_temp, store, sid) = fixture();
        let tombstone = store.root.join(format!(".deleting-{sid}"));
        fs::create_dir(&tombstone).unwrap();
        fs::write(tombstone.join("leftover"), b"old copy").unwrap();
        store.cleanup_deleted().unwrap();
        assert!(!tombstone.exists());
        assert!(store.load_record(&sid).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn startup_deletion_continues_past_a_failed_tombstone() {
        let (temp, store, sid) = fixture();
        trash(&store, &sid);
        let outside = temp.path().join("untouched");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"outside").unwrap();
        let blocked = store
            .root
            .join(".deleting-00000000-0000-0000-0000-000000000000");
        std::os::unix::fs::symlink(&outside, &blocked).unwrap();
        let ready = store
            .root
            .join(".deleting-ffffffff-ffff-ffff-ffff-ffffffffffff");
        fs::rename(store.root.join(sid), &ready).unwrap();
        assert!(store.cleanup_deleted().is_err());
        assert!(
            !ready.exists(),
            "a failed item must not block other removals"
        );
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"outside");
    }

    #[cfg(unix)]
    #[test]
    fn deletion_does_not_follow_links_out_of_the_conversation() {
        let (temp, store, sid) = fixture();
        let outside = temp.path().join("keep");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("private"), b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, store.root.join(&sid).join("linked-folder")).unwrap();
        trash(&store, &sid);
        store.delete_trashed(&sid, || Ok(())).unwrap();
        assert_eq!(fs::read(outside.join("private")).unwrap(), b"keep");
        let tombstone = store.root.join(format!(".deleting-{}", id()));
        std::os::unix::fs::symlink(&outside, &tombstone).unwrap();
        assert!(store.cleanup_deleted().is_err());
        assert_eq!(fs::read(outside.join("private")).unwrap(), b"keep");
    }
}
