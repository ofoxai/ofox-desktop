use super::Operation;

pub fn instruction_content_view(
    path: &std::path::Path,
    receipt: Option<&super::Receipt>,
) -> serde_json::Value {
    use serde_json::json;
    let unknown =
        |state| json!({"nativeState":state,"userContentLines":null,"managedContentLines":null});
    let text = match super::coordinator::read(path) {
        Ok(Some(text)) => text,
        Ok(None) => {
            return json!({"nativeState":"missing","userContentLines":0,"managedContentLines":0})
        }
        Err(_) => return unknown("unreadable"),
    };
    let lines = |text: &str| {
        text.strip_prefix('\u{feff}')
            .unwrap_or(text)
            .lines()
            .count()
    };
    if let Some(receipt) = receipt {
        match super::patch_instructions(&text, Some(&receipt.owned_segment), None) {
            Ok(withdrawn) => {
                let block = receipt.owned_segment.trim_start_matches(['\r', '\n']);
                let managed = block.lines().count().saturating_sub(2);
                json!({"nativeState":"intact","userContentLines":lines(&withdrawn.text),"managedContentLines":managed})
            }
            Err(_) => unknown("conflict"),
        }
    } else if text.contains("<!-- OFOX:") {
        unknown("conflict")
    } else {
        json!({"nativeState":"unmanaged","userContentLines":lines(&text),"managedContentLines":0})
    }
}

pub fn instruction_status<'a>(
    native_state: &'a str,
    receipt: &super::Receipt,
    definition: &super::InstructionSet,
) -> &'a str {
    if native_state != "intact" {
        return native_state;
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
    fn status_for(
        path: &std::path::Path,
        receipt: &super::super::Receipt,
        definition: &InstructionSet,
    ) -> String {
        let content = instruction_content_view(path, Some(receipt));
        instruction_status(
            content["nativeState"].as_str().unwrap(),
            receipt,
            definition,
        )
        .to_owned()
    }

    #[test]
    fn instruction_counts_preserve_private_text_and_report_unknown_instead_of_empty() {
        let (_dir, path, receipt, _) = applied_fixture();
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("private-sentinel outside\nlast line");
        std::fs::write(&path, &text).unwrap();
        let view = instruction_content_view(&path, Some(&receipt));
        assert_eq!(view["userContentLines"], 3);
        assert_eq!(view["managedContentLines"], 1);
        assert_eq!(view["nativeState"], "intact");
        assert!(!view.to_string().contains("private-sentinel"));
        std::fs::write(&path, text.replace("shared", "user modified")).unwrap();
        let conflict = instruction_content_view(&path, Some(&receipt));
        assert!(conflict["userContentLines"].is_null());
        assert!(conflict["managedContentLines"].is_null());
        assert_eq!(conflict["nativeState"], "conflict");
    }

    #[test]
    fn instruction_counts_handle_bom_newlines_missing_foreign_and_unreadable_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        for (body, count) in [("\u{feff}", 0), ("\u{feff}one\r\ntwo", 2), ("a\n\n", 2)] {
            std::fs::write(&path, body).unwrap();
            let view = instruction_content_view(&path, None);
            assert_eq!(view["userContentLines"], count);
            assert_eq!(view["managedContentLines"], 0);
            assert_eq!(view["nativeState"], "unmanaged");
        }
        for body in ["<!-- OFOX:BEGIN instructions -->", "<!-- OFOX:unknown -->"] {
            std::fs::write(&path, body).unwrap();
            let view = instruction_content_view(&path, None);
            assert_eq!(view["nativeState"], "conflict");
            assert!(view["userContentLines"].is_null());
        }
        std::fs::write(&path, [0xff]).unwrap();
        assert_eq!(
            instruction_content_view(&path, None)["nativeState"],
            "unreadable"
        );
        std::fs::write(&path, vec![b'a'; 2 * 1024 * 1024 + 1]).unwrap();
        assert!(instruction_content_view(&path, None)["userContentLines"].is_null());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            instruction_content_view(&path, None)["nativeState"],
            "missing"
        );
        assert_eq!(instruction_content_view(&path, None)["userContentLines"], 0);
    }

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
        assert_eq!(status_for(&path, &receipt, &set), "missing");
    }

    #[test]
    fn a_manually_changed_block_is_not_reported_applied() {
        let (_dir, path, receipt, set) = applied_fixture();
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("shared", "manual");
        std::fs::write(&path, text).unwrap();
        assert_eq!(status_for(&path, &receipt, &set), "conflict");
    }

    #[test]
    fn changes_outside_the_managed_block_stay_applied() {
        let (_dir, path, receipt, set) = applied_fixture();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}new personal\n")).unwrap();
        assert_eq!(status_for(&path, &receipt, &set), "applied");
    }

    #[test]
    fn a_restored_definition_with_the_same_revision_is_still_pending() {
        let (_dir, path, receipt, mut set) = applied_fixture();
        set.content = "different content from a backup".into();
        assert_eq!(status_for(&path, &receipt, &set), "pending");
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
