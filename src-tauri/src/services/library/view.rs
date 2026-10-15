use super::Operation;

pub fn instruction_block_health(path: &std::path::Path, receipt: &super::Receipt) -> &'static str {
    match super::coordinator::read(path) {
        Ok(None) => "missing",
        Err(_) => "unreadable",
        Ok(Some(text)) => {
            if super::patch_instructions(&text, Some(&receipt.owned_segment), None).is_ok() {
                "intact"
            } else {
                "conflict"
            }
        }
    }
}

pub fn instruction_status(
    path: &std::path::Path,
    receipt: &super::Receipt,
    definition: &super::InstructionSet,
) -> &'static str {
    let health = instruction_block_health(path, receipt);
    if health != "intact" {
        return health;
    }
    if receipt.resource_revision == definition.revision
        && receipt.definition_hash
            == super::coordinator::fingerprint(Some(definition.content.as_bytes()))
    {
        "applied"
    } else {
        "pending"
    }
}

/// Only this projection crosses IPC. The journal keeps full text privately.
pub fn operation_view(operation: &Operation) -> serde_json::Value {
    serde_json::json!({
        "operationId": operation.id,
        "kind": operation.kind,
        "resourceId": operation.resource_id,
        "resourceRevision": operation.resource_revision,
        "createdAt": operation.created_at,
        "status": operation.status,
        "targets": operation.targets.iter().map(|target| serde_json::json!({
            "tools":target.tools,
            "readers":target.readers,
            "path":target.path,
            "targetGroupId":target.physical_target,
            "status":target.status,
            "reason":target.reason,
            "method":target.transfer.as_ref().map(|summary|summary.method.as_str()).or_else(||target.directory.as_ref().map(|plan|plan.method.as_str())),
            "plannedMethod":target.transfer.as_ref().and_then(|summary|summary.planned_method.as_deref()),
            "artifactHash":target.transfer.as_ref().map(|summary|summary.artifact_hash.as_str()).or_else(||target.directory.as_ref().map(|plan|plan.artifact_hash.as_str())),
            "fallbackCopy":target.transfer.as_ref().map(|summary|summary.fallback_copy).or_else(||target.directory.as_ref().map(|plan|plan.fallback_copy)),
            "fallbackUsed":target.transfer.as_ref().and_then(|summary|summary.planned_method.as_ref().map(|method|method=="symlink"&&summary.method=="copy")),
            "beforeManaged":target.previous_receipt.as_ref().filter(|receipt|receipt.kind==super::ResourceKind::Instructions).map(|receipt| &receipt.owned_segment),
            "afterManaged":target.receipt.as_ref().filter(|receipt|receipt.kind==super::ResourceKind::Instructions).map(|receipt| &receipt.owned_segment),
            "changedFields": if operation.kind==super::ResourceKind::Mcp {
                [&target.previous_receipt,&target.receipt].into_iter().flatten().filter_map(|r|serde_json::from_str::<super::mcp_codec::OwnedFields>(&r.owned_segment).ok()).flat_map(|o|o.fields.into_iter().map(|(path,_)|path[2..].join("."))).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>()
            }else{vec![]},
        })).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::super::{coordinator::preview_at, save_instruction, InstructionSet};
    use super::*;

    fn applied_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        super::super::Receipt,
        InstructionSet,
    ) {
        let db = crate::Database::memory().unwrap();
        let set = save_instruction(
            &db,
            InstructionSet {
                id: "fixture".into(),
                name: "Fixture".into(),
                content: "shared".into(),
                revision: 0,
            },
            None,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "personal\n").unwrap();
        let op = preview_at(&db, "fixture", &[("codex".into(), path.clone())]).unwrap();
        super::super::apply_operation(&db, &op.id).unwrap();
        (dir, path, op.targets[0].receipt.clone().unwrap(), set)
    }

    #[test]
    fn a_deleted_native_file_is_not_reported_applied() {
        let (_dir, path, receipt, set) = applied_fixture();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(instruction_status(&path, &receipt, &set), "missing");
    }

    #[test]
    fn a_manually_changed_block_is_not_reported_applied() {
        let (_dir, path, receipt, set) = applied_fixture();
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("shared", "manual");
        std::fs::write(&path, text).unwrap();
        assert_eq!(instruction_status(&path, &receipt, &set), "conflict");
    }

    #[test]
    fn changes_outside_the_managed_block_stay_applied() {
        let (_dir, path, receipt, set) = applied_fixture();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}new personal\n")).unwrap();
        assert_eq!(instruction_status(&path, &receipt, &set), "applied");
    }

    #[test]
    fn a_restored_definition_with_the_same_revision_is_still_pending() {
        let (_dir, path, receipt, mut set) = applied_fixture();
        set.content = "different content from a backup".into();
        assert_eq!(instruction_status(&path, &receipt, &set), "pending");
        assert!(std::fs::read_to_string(&path).unwrap().contains("shared"));
    }

    #[test]
    fn public_preview_never_contains_the_internal_file_payload() {
        let db = crate::Database::memory().unwrap();
        save_instruction(
            &db,
            InstructionSet {
                id: "fixture".into(),
                name: "Fixture".into(),
                content: "public shared".into(),
                revision: 0,
            },
            None,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "private-key-fixture outside managed block").unwrap();
        let op = preview_at(&db, "fixture", &[("codex".into(), path)]).unwrap();
        let view = operation_view(&op);
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(!serialized.contains("private-key-fixture"));
        assert!(!serialized.contains("afterText"));
        assert!(!serialized.contains("previousReceipt"));
        assert!(serialized.contains("public shared"));
    }
}
