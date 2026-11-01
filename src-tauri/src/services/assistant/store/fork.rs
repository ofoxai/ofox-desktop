//! Private staging and inherited billing for copied native branches.
use super::*;

use std::io::{Read, Write};

const FORK_FAILED: &str = "assistant_fork_failed";
const FORK_PENDING: &str = "assistant_fork_pending_media";

/// Copied paid facts are visible in the new conversation, but counted only
/// once across an owner's conversations (including orphaned/nested forks).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InheritedUsage {
    pub key: String,
    pub run_id: String,
    #[serde(default)]
    pub source_session_id: String,
    pub usage: Value,
}

pub struct ForkStage {
    store: AssistantStore,
    source: String,
    sid: String,
    directory: PathBuf,
    published: bool,
}

impl AssistantStore {
    pub fn stage_fork(&self, source: &str) -> Result<ForkStage, String> {
        validate_id(source)?;
        self.native_history_path(source)?;
        let sid = id();
        let directory = private_subdir(&self.ensure_root()?, &format!(".fork-{sid}"))?;
        Ok(ForkStage {
            store: self.clone(),
            source: source.into(),
            sid,
            directory,
            published: false,
        })
    }
}

impl ForkStage {
    pub fn native_path(&self) -> PathBuf {
        self.directory.join(NATIVE_HISTORY_FILE)
    }

    pub fn publish(
        mut self,
        mut record: SessionRecord,
        entries: &[Value],
        title: &str,
        precondition: impl FnOnce() -> Result<(), String>,
    ) -> Result<SessionRecord, String> {
        let ids: HashSet<&str> = entries.iter().filter_map(|e| e["id"].as_str()).collect();
        let runs: HashSet<&str> = entries.iter().filter_map(|e| e["runId"].as_str()).collect();
        record.messages.retain(|m| {
            m.native_entry_id
                .as_deref()
                .is_some_and(|id| ids.contains(id))
        });
        record
            .operations
            .retain(|op| runs.contains(op.run_id.as_str()));
        if record.operations.iter().any(|op| {
            op.media.as_ref().is_some_and(|m| {
                m.kind == "video" && super::super::video::occupies_generation_slot(&m.status)
            })
        }) {
            return Err(FORK_PENDING.into());
        }
        // A user-only fork must not inherit the later result/cost of that run.
        let media_runs: HashSet<String> = record
            .messages
            .iter()
            .filter(|m| m.media.is_some())
            .filter_map(|m| m.run_id.clone())
            .collect();
        record
            .operations
            .retain(|op| op.media.is_none() || media_runs.contains(&op.run_id));
        let mut needed = HashSet::new();
        for message in &mut record.messages {
            message.superseded = false;
            message.usage = None;
            for a in &message.attachments {
                needed.insert(a.id.clone());
            }
            if let Some(media) = &message.media {
                needed.extend(media.input_ids.iter().cloned());
                needed.extend(media.outputs.iter().cloned());
            }
        }
        record.attachments.retain(|a| needed.contains(&a.id));
        let source = self.store.attachment_root(&self.source)?;
        if needed.len() != record.attachments.len()
            || record.attachments.len() > super::super::attachments::MAX_PER_SESSION
            || record
                .attachments
                .iter()
                .try_fold(0u64, |total, a| total.checked_add(a.size_bytes))
                .is_none_or(|total| total > super::super::attachments::MAX_SESSION_BYTES)
        {
            return Err(FORK_FAILED.into());
        }
        let has_files = record
            .attachments
            .iter()
            .any(|a| a.reason.as_deref() != Some("assistant_attachment_not_in_backup"));
        if has_files && !real_dir(&source)? {
            return Err(FORK_FAILED.into());
        }
        let destination = private_subdir(&self.directory, ATTACHMENTS_DIR)?;
        for attachment in &record.attachments {
            validate_id(&attachment.id)?;
            if attachment.reason.as_deref() == Some("assistant_attachment_not_in_backup") {
                continue;
            }
            let path = source.join(&attachment.id);
            ensure_regular_or_absent(&path)?;
            let mut options = fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            let file = options.open(&path).map_err(|_| FORK_FAILED)?;
            let meta = file.metadata().map_err(|_| FORK_FAILED)?;
            let cap = if attachment.mime_type.starts_with("video/") {
                super::super::video::MAX_VIDEO_BYTES
            } else {
                50 * 1024 * 1024
            };
            if !meta.is_file() || meta.len() != attachment.size_bytes || meta.len() > cap {
                return Err(FORK_FAILED.into());
            }
            let target = destination.join(&attachment.id);
            let mut output = fs::OpenOptions::new();
            output.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                output.mode(0o600);
            }
            let mut output = output.open(target).map_err(|_| FORK_FAILED)?;
            let written =
                std::io::copy(&mut file.take(cap + 1), &mut output).map_err(|_| FORK_FAILED)?;
            if written != attachment.size_bytes {
                return Err(FORK_FAILED.into());
            }
            output.flush().map_err(|_| FORK_FAILED)?;
            output.sync_all().map_err(|_| FORK_FAILED)?;
        }
        record.session.id = self.sid.clone();
        record.session.context_usage = None;
        record.session.title = title
            .chars()
            .scan(0usize, |bytes, c| {
                *bytes += c.len_utf8();
                (*bytes <= MAX_TITLE_BYTES).then_some(c)
            })
            .collect();
        record.session.title_source = Some(TitleSource::User);
        record.session.created_at = now();
        record.session.updated_at = record.session.created_at;
        record.session.draft.clear();
        record.session.draft_attachment_ids.clear();
        record.session.active_run = None;
        record.session.deleted_at = None;
        record.session.pinned_at = None;
        record.session.cleanup_warning = None;
        record.usage = json!({});
        super::super::projection::merge_native(&mut record, &json!(entries), &[])?;
        record.inherited_usage = inherited_usage(&record, entries, &self.source)?;
        check_record(&record, &self.sid)?;
        write_record(&self.directory.join(SESSION_FILE), &record)?;
        if !fs::symlink_metadata(self.native_path())
            .is_ok_and(|meta| meta.file_type().is_file() && meta.len() > 0)
        {
            return Err(FORK_FAILED.into());
        }
        let source_lock = file_lock(&self.store.root.join(&self.source).join(SESSION_FILE));
        let _source = source_lock.lock().unwrap_or_else(PoisonError::into_inner);
        if self
            .store
            .load_record(&self.source)?
            .session
            .deleted_at
            .is_some()
        {
            return Err("assistant_session_deleted".into());
        }
        precondition()?;
        let final_dir = self.store.root.join(&self.sid);
        if final_dir.exists() {
            return Err(FORK_FAILED.into());
        }
        crate::config::rename_dir(&self.directory, &final_dir).map_err(|_| FORK_FAILED)?;
        self.published = true;
        Ok(record)
    }
}
impl Drop for ForkStage {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

fn inherited_usage(
    record: &SessionRecord,
    entries: &[Value],
    source: &str,
) -> Result<Vec<InheritedUsage>, String> {
    let mut result = vec![];
    let mut copy = record.clone();
    copy.messages.clear();
    copy.attachments.clear();
    copy.inherited_usage.clear();
    copy.operations.retain(|op| op.media.is_none());
    for entry in entries.iter().filter(|e| {
        matches!(e["role"].as_str(), Some("assistant" | "usage"))
            || (e["role"] == "tool" && e["toolName"] == "web_search")
    }) {
        copy.usage = json!({});
        let is_tool = entry["role"] == "tool";
        let mut facts = vec![entry.clone()];
        if is_tool {
            facts.push(json!({"role":"usage","pricingKnown":true,"usage":{"cost":{"total":0}}}));
        }
        super::super::projection::recount_usage(&mut copy, &facts);
        if is_tool {
            copy.usage["modelRequests"] = json!(0);
        }
        result.push(InheritedUsage {
            key: format!(
                "native:{}:{}",
                entry["runId"].as_str().unwrap_or(""),
                entry["id"].as_str().ok_or(FORK_FAILED)?
            ),
            run_id: entry["runId"].as_str().unwrap_or("").into(),
            source_session_id: String::new(),
            usage: copy.usage.clone(),
        });
    }
    for op in record.operations.iter().filter(|op| op.media.is_some()) {
        copy.usage = json!({});
        copy.operations = vec![op.clone()];
        super::super::projection::recount_usage(&mut copy, &[]);
        result.push(InheritedUsage {
            key: format!("media:{}", op.id),
            run_id: op.run_id.clone(),
            source_session_id: String::new(),
            usage: copy.usage.clone(),
        });
    }
    // Even a user-only branch inherits its run marker. Keeping that run out
    // of the owner's owned-runs set prevents a cost-free prefix from hiding a
    // paid sibling after the original conversation is permanently deleted.
    for op in &record.operations {
        if !result.iter().any(|row| row.run_id == op.run_id) {
            result.push(InheritedUsage {
                key: format!("run:{}", op.run_id),
                run_id: op.run_id.clone(),
                source_session_id: String::new(),
                usage: json!({"modelRequests":0}),
            });
        }
    }
    for row in &mut result {
        row.source_session_id = record
            .inherited_usage
            .iter()
            .find(|old| old.key == row.key)
            .map(|old| old.source_session_id.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(source)
            .to_owned();
    }
    Ok(result)
}

/// Remove the copied contributions from a conversation's total before the
/// owner's summary adds those facts by native identity. Unknown stays unknown.
pub(super) fn own_usage(record: &SessionRecord) -> Value {
    let mut usage = record.usage.clone();
    for row in &record.inherited_usage {
        for key in [
            "modelRequests",
            "inputTokens",
            "outputTokens",
            "cacheReadTokens",
            "cacheWriteTokens",
            "searchCount",
        ] {
            usage[key] = json!(usage[key]
                .as_u64()
                .unwrap_or(0)
                .saturating_sub(row.usage[key].as_u64().unwrap_or(0)));
        }
        for key in ["estimatedCostUsd", "searchCostUsd", "mediaCostUsd"] {
            if let Some(b) = decimal(&row.usage[key]) {
                usage[key] = json!(decimal(&usage[key]).map(|a| (a - b)
                    .max(rust_decimal::Decimal::ZERO)
                    .normalize()
                    .to_string()));
            }
        }
    }
    usage
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, AssistantStore, SessionRecord, Vec<Value>) {
        let temp = tempfile::tempdir().unwrap();
        let store = AssistantStore::for_owner(temp.path(), &"a".repeat(64)).unwrap();
        let session = store.create("model", false).unwrap();
        let rid = id();
        let aid = id();
        let attachments =
            private_subdir(&store.root.join(&session.session.id), ATTACHMENTS_DIR).unwrap();
        write_bytes(&attachments.join(&aid), b"hello").unwrap();
        let mut record = store.load_record(&session.session.id).unwrap();
        record.session.context_usage = Some(ContextUsage {
            model_id: record.session.model_id.clone(),
            tokens: Some(15),
            context_window: 32000,
        });
        record.session.draft = "private draft".into();
        record.attachments = vec![serde_json::from_value(json!({
            "id":aid,"name":"hello.txt","mimeType":"text/plain","sizeBytes":5,"status":"ready"
        }))
        .unwrap()];
        record.messages =
            vec![serde_json::from_value(json!({
            "id":id(),"role":"user","content":"question","status":"completed","createdAt":1,
            "nativeEntryId":"user1","runId":rid,"attachments":record.attachments
        })).unwrap(), serde_json::from_value(json!({
            "id":id(),"role":"assistant","content":"answer","status":"completed","createdAt":2,
            "nativeEntryId":"answer1","runId":rid
        })).unwrap()];
        record.operations = vec![Operation {
            id: id(),
            run_id: rid.clone(),
            status: OperationStatus::Completed,
            media: None,
            search_price: None,
        }];
        let entries = vec![
            json!({"id":"user1","role":"user","runId":rid,"content":"question","active":true}),
            json!({"id":"answer1","role":"assistant","runId":rid,"content":"answer","active":true,"pricingKnown":true,
                "usage":{"input":10,"output":5,"cost":{"total":0.25}}}),
        ];
        super::super::super::projection::merge_native(&mut record, &json!(entries), &[]).unwrap();
        store
            .mutate(&session.session.id, |r| {
                *r = record.clone();
                Ok(())
            })
            .unwrap();
        (temp, store, record, entries)
    }

    #[test]
    fn fork_stage_copies_context_files_ledger_and_deduplicates_inherited_cost() {
        let (_temp, store, record, entries) = fixture();
        let original = fs::read(store.root.join(&record.session.id).join(SESSION_FILE)).unwrap();
        let stage = store.stage_fork(&record.session.id).unwrap();
        write_bytes(&stage.native_path(), b"native copy\n").unwrap();
        let fork = stage
            .publish(record.clone(), &entries, "fork title", || Ok(()))
            .unwrap();
        assert_ne!(fork.session.id, record.session.id);
        assert_eq!(fork.messages.len(), 2);
        assert!(fork.session.draft.is_empty());
        assert!(fork.session.context_usage.is_none());
        assert_eq!(fork.operations, record.operations);
        assert_eq!(fork.usage["inputTokens"], 10);
        assert_eq!(
            fs::read(
                store
                    .attachment_root(&fork.session.id)
                    .unwrap()
                    .join(&record.attachments[0].id)
            )
            .unwrap(),
            b"hello"
        );
        assert_eq!(
            fs::read(store.root.join(&record.session.id).join(SESSION_FILE)).unwrap(),
            original
        );
        assert_eq!(store.usage_summary().unwrap()["estimatedCostUsd"], "0.25");
        // A fork of the fork and an orphaned copy still count the paid request once.
        let stage = store.stage_fork(&fork.session.id).unwrap();
        write_bytes(&stage.native_path(), b"another native copy\n").unwrap();
        stage
            .publish(fork.clone(), &entries, "second fork", || Ok(()))
            .unwrap();
        assert_eq!(store.usage_summary().unwrap()["inputTokens"], 10);
        fs::remove_dir_all(store.root.join(&record.session.id)).unwrap();
        assert_eq!(store.usage_summary().unwrap()["estimatedCostUsd"], "0.25");
        assert_eq!(store.usage_summary().unwrap()["inputTokens"], 10);
    }

    #[test]
    fn user_only_fork_does_not_hide_a_paid_sibling_after_original_deletion() {
        let (_temp, store, record, entries) = fixture();
        let stage = store.stage_fork(&record.session.id).unwrap();
        write_bytes(&stage.native_path(), b"full copy\n").unwrap();
        let full = stage
            .publish(record.clone(), &entries, "full", || Ok(()))
            .unwrap();
        let stage = store.stage_fork(&record.session.id).unwrap();
        write_bytes(&stage.native_path(), b"user copy\n").unwrap();
        let prefix = stage
            .publish(record.clone(), &entries[..1], "user", || Ok(()))
            .unwrap();
        assert_eq!(prefix.usage["modelRequests"], 0);
        assert_eq!(prefix.messages.len(), 1);
        fs::remove_dir_all(store.root.join(&record.session.id)).unwrap();
        assert_eq!(store.usage_summary().unwrap()["estimatedCostUsd"], "0.25");
        assert_eq!(store.usage_summary().unwrap()["runCount"], 1);
        assert_ne!(full.session.id, prefix.session.id);
    }

    #[test]
    fn fork_keeps_the_original_search_price_and_its_account_total_once() {
        let (_temp, store, mut record, mut entries) = fixture();
        record.operations[0].search_price = Some(serde_json::from_value(json!({
            "model":"search-model","perSearch":"0.014","perInputToken":"0.001","perOutputToken":"0.002"
        })).unwrap());
        entries.insert(1,json!({"id":"search1","role":"tool","toolName":"web_search","runId":record.operations[0].run_id,
            "usageCoverage":"complete","isError":false,"details":{"usage":{"searchCalls":1,"inputTokens":2,"outputTokens":3,"model":"search-model"}}}));
        super::super::super::projection::merge_native(&mut record, &json!(entries), &[]).unwrap();
        store
            .mutate(&record.session.id, |r| {
                *r = record.clone();
                Ok(())
            })
            .unwrap();
        let stage = store.stage_fork(&record.session.id).unwrap();
        write_bytes(&stage.native_path(), b"search copy\n").unwrap();
        let fork = stage
            .publish(record.clone(), &entries, "search", || Ok(()))
            .unwrap();
        assert_eq!(
            fork.operations[0].search_price,
            record.operations[0].search_price
        );
        assert_eq!(fork.usage["searchCount"], 1);
        assert_eq!(fork.usage["searchCostUsd"], "0.022");
        fs::remove_dir_all(store.root.join(&record.session.id)).unwrap();
        let summary = store.usage_summary().unwrap();
        assert_eq!(summary["estimatedCostUsd"], "0.272");
        assert_eq!(summary["searchCostUsd"], "0.022");
        assert_eq!(summary["searchCount"], 1);
    }

    #[test]
    fn legacy_usage_without_a_run_is_not_counted_again_by_a_fork() {
        let (_temp, store, mut record, mut entries) = fixture();
        entries.push(
            json!({"id":"legacy-usage","role":"usage","pricingKnown":true,
            "usage":{"input":2,"output":1,"cost":{"total":0.125}}}),
        );
        super::super::super::projection::merge_native(&mut record, &json!(entries), &[]).unwrap();
        store
            .mutate(&record.session.id, |r| {
                *r = record.clone();
                Ok(())
            })
            .unwrap();
        let stage = store.stage_fork(&record.session.id).unwrap();
        write_bytes(&stage.native_path(), b"legacy copy\n").unwrap();
        let fork = stage
            .publish(record.clone(), &entries, "legacy", || Ok(()))
            .unwrap();
        assert_eq!(store.usage_summary().unwrap()["estimatedCostUsd"], "0.375");
        let stage = store.stage_fork(&fork.session.id).unwrap();
        write_bytes(&stage.native_path(), b"nested legacy copy\n").unwrap();
        stage.publish(fork, &entries, "nested", || Ok(())).unwrap();
        fs::remove_dir_all(store.root.join(&record.session.id)).unwrap();
        assert_eq!(store.usage_summary().unwrap()["estimatedCostUsd"], "0.375");
    }

    #[test]
    fn failed_new_fork_request_keeps_the_account_cost_unknown() {
        let (_temp, store, record, entries) = fixture();
        let stage = store.stage_fork(&record.session.id).unwrap();
        write_bytes(&stage.native_path(), b"copy\n").unwrap();
        let fork = stage.publish(record, &entries, "fork", || Ok(())).unwrap();
        store
            .mutate(&fork.session.id, |r| {
                r.operations.push(Operation {
                    id: id(),
                    run_id: id(),
                    status: OperationStatus::Failed,
                    media: None,
                    search_price: None,
                });
                r.usage["usageCoverage"] = json!("incomplete");
                r.usage["estimatedCostUsd"] = Value::Null;
                Ok(())
            })
            .unwrap();
        assert!(store.usage_summary().unwrap()["estimatedCostUsd"].is_null());
    }

    #[test]
    fn fork_preserves_excluded_backup_files_as_missing() {
        let (_temp, store, mut record, entries) = fixture();
        record.attachments[0].status = "unreadable".into();
        record.attachments[0].reason = Some("assistant_attachment_not_in_backup".into());
        record.messages[0].attachments = record.attachments.clone();
        fs::remove_dir_all(store.attachment_root(&record.session.id).unwrap()).unwrap();
        let stage = store.stage_fork(&record.session.id).unwrap();
        write_bytes(&stage.native_path(), b"excluded copy\n").unwrap();
        let fork = stage
            .publish(record, &entries, "excluded", || Ok(()))
            .unwrap();
        assert_eq!(
            fork.attachments[0].reason.as_deref(),
            Some("assistant_attachment_not_in_backup")
        );
        assert!(!store
            .attachment_root(&fork.session.id)
            .unwrap()
            .join(&fork.attachments[0].id)
            .exists());
    }

    #[test]
    fn fork_failure_is_not_published_and_never_follows_attachment_links() {
        let (_temp, store, record, entries) = fixture();
        let stage = store.stage_fork(&record.session.id).unwrap();
        let staging = stage.native_path().parent().unwrap().to_path_buf();
        write_bytes(&stage.native_path(), b"copy\n").unwrap();
        assert!(stage
            .publish(record.clone(), &entries, "fork", || Err("stale".into()))
            .is_err());
        assert!(!staging.exists());
        assert_eq!(store.list(false, "").unwrap().sessions.len(), 1);
        #[cfg(unix)]
        {
            let path = store
                .attachment_root(&record.session.id)
                .unwrap()
                .join(&record.attachments[0].id);
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(
                store.root.join(&record.session.id).join(SESSION_FILE),
                &path,
            )
            .unwrap();
            let stage = store.stage_fork(&record.session.id).unwrap();
            write_bytes(&stage.native_path(), b"copy\n").unwrap();
            assert!(stage.publish(record, &entries, "fork", || Ok(())).is_err());
            assert_eq!(store.list(false, "").unwrap().sessions.len(), 1);
        }
    }
    #[test]
    fn independent_audio_stays_in_its_original_conversation_and_is_not_rebilled_by_a_fork() {
        let (_root, store, mut record, entries) = fixture();
        let aid = id();
        let original = store
            .attachment_root(&record.session.id)
            .unwrap()
            .join(&aid);
        write_bytes(&original, b"audio").unwrap();
        record.attachments.push(serde_json::from_value(json!({"id":aid,"name":"audio.wav","mimeType":"audio/wav","sizeBytes":5,"status":"ready","origin":"audio-input"})).unwrap());
        record.operations.push(serde_json::from_value(json!({"id":id(),"runId":id(),"status":"completed","media":{"kind":"transcription","model":"test/asr","params":{"keepOriginal":true},"status":"completed","cost":"0.1","inputIds":[aid],"audio":{"pricing":{"unit":"seconds"},"text":"Audio stays here"}}})).unwrap());
        super::super::super::projection::recount_usage(&mut record, &entries);
        store
            .mutate(&record.session.id, |r| {
                *r = record.clone();
                Ok(())
            })
            .unwrap();
        assert_eq!(store.usage_summary().unwrap()["estimatedCostUsd"], "0.35");
        let stage = store.stage_fork(&record.session.id).unwrap();
        write_bytes(&stage.native_path(), b"copy\n").unwrap();
        let fork = stage
            .publish(record.clone(), &entries, "fork", || Ok(()))
            .unwrap();
        assert!(fork.operations.iter().all(|o| o.media.is_none()));
        assert!(!fork.attachments.iter().any(|a| a.id == aid));
        assert!(original.exists());
        assert_eq!(store.usage_summary().unwrap()["estimatedCostUsd"], "0.35");
        assert_eq!(
            store
                .load_record(&record.session.id)
                .unwrap()
                .operations
                .len(),
            2
        );
    }
}
