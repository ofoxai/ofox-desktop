//! Draft-only resource catalog actions. These never project into native files.
use super::{InstructionSet, ResourceKind};
use crate::{app_config::InstalledSkill, error::AppError, Database};
use std::path::Path;

fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}
pub fn verify_epoch(db: &Database, expected: u64) -> Result<(), AppError> {
    if db.library_snapshot()?.machine.epoch != expected {
        return Err(error("epochChanged"));
    }
    Ok(())
}
pub fn verify_mcp_delete_revision(db: &Database, id: &str, expected: u64) -> Result<(), AppError> {
    if db
        .library_snapshot()?
        .definitions
        .mcp
        .get(id)
        .map(|row| row.revision)
        != Some(expected)
    {
        return Err(error("revisionChanged"));
    }
    Ok(())
}
pub fn verify_skill_delete_revision(
    db: &Database,
    id: &str,
    expected: u64,
) -> Result<(), AppError> {
    if db
        .library_snapshot()?
        .definitions
        .skills
        .get(id)
        .map(|row| row.revision)
        != Some(expected)
    {
        return Err(error("revisionChanged"));
    }
    Ok(())
}
pub fn import_instruction(
    db: &Database,
    name: &str,
    content: &str,
) -> Result<InstructionSet, AppError> {
    let _io = crate::config_io::lock();
    let id = format!(
        "import:{}",
        super::coordinator::fingerprint(Some(
            &serde_json::to_vec(&(name, content)).map_err(|_| error("invalidInstructions"))?
        ))
    );
    let before = db.library_snapshot()?;
    if let Some(existing) = before.definitions.instructions.get(&id) {
        if existing.name == name && existing.content == content {
            return Ok(existing.clone());
        }
        return Err(error("revisionChanged"));
    }
    super::save_instruction(
        db,
        InstructionSet {
            id,
            name: name.into(),
            content: content.into(),
            revision: 0,
        },
        None,
    )
}
pub fn delete_instruction(db: &Database, id: &str, revision: u64) -> Result<bool, AppError> {
    let _io = crate::config_io::lock();
    let before = db.library_snapshot()?;
    let Some(set) = before.definitions.instructions.get(id) else {
        return Ok(false);
    };
    if set.revision != revision {
        return Err(error("revisionChanged"));
    }
    if before
        .machine
        .assignments
        .iter()
        .any(|(key, value)| key.starts_with("instructions:") && value == id)
        || before
            .machine
            .receipts
            .values()
            .any(|r| r.kind == ResourceKind::Instructions && r.resource_id == id)
        || before.machine.operations.values().any(|op| {
            op.kind == ResourceKind::Instructions
                && op.resource_id == id
                && !["previewed", "applied", "cancelled"].contains(&op.status.as_str())
        })
    {
        return Err(error("withdrawRequired"));
    }
    let mut next = before.clone();
    next.definitions.instructions.remove(id);
    for op in next.machine.operations.values_mut().filter(|op| {
        op.kind == ResourceKind::Instructions && op.resource_id == id && op.status == "previewed"
    }) {
        op.status = "cancelled".into();
    }
    db.library_commit(&before, &next)?;
    Ok(true)
}
pub fn import_directory(
    db: &Database,
    source: &Path,
    directory: &str,
    name: &str,
) -> Result<InstalledSkill, AppError> {
    let _io = crate::config_io::lock();
    super::skill_artifact::safe_leaf(directory)?;
    if name.trim().is_empty() {
        return Err(error("invalidSkill"));
    }
    let hash = super::skill_artifact::publish(source)?;
    if let Some(existing) = db
        .get_all_installed_skills()?
        .into_values()
        .find(|row| row.directory.eq_ignore_ascii_case(directory))
    {
        if existing.content_hash.as_ref() == Some(&hash) && existing.name == name {
            super::skill_resource::artifact_for(db, &existing.id)?;
            return Ok(existing);
        }
        return Err(error("skillNameConflict"));
    }
    let now = chrono::Utc::now().timestamp();
    super::skill_resource::save_draft(
        db,
        InstalledSkill {
            id: format!("local:{}", uuid::Uuid::new_v4()),
            name: name.into(),
            description: None,
            directory: directory.into(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: Default::default(),
            installed_at: now,
            updated_at: now,
            content_hash: None,
        },
        source,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[serial_test::serial]
    fn obsolete_failed_instruction_can_be_cancelled_after_successful_withdrawal() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let row = import_instruction(&db, "Draft", "Shared").unwrap();
        let path = super::super::coordinator::native_instruction_paths().unwrap()["codex"].clone();
        let old = super::super::preview_instructions(&db, &row.id, &["codex".into()]).unwrap();
        crate::config::atomic_write(&path, b"Personal change").unwrap();
        assert_eq!(
            super::super::apply_operation(&db, &old.id).unwrap().status,
            "partial"
        );
        let fresh = super::super::preview_instructions(&db, &row.id, &["codex".into()]).unwrap();
        super::super::apply_operation(&db, &fresh.id).unwrap();
        let withdraw = super::super::preview_instructions(&db, &row.id, &[]).unwrap();
        super::super::apply_operation(&db, &withdraw.id).unwrap();
        assert!(delete_instruction(&db, &row.id, row.revision).is_err());
        let contents = std::fs::read(&path).unwrap();
        let cancelled = super::super::coordinator::cancel_operation(&db, &old.id).unwrap();
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(std::fs::read(&path).unwrap(), contents);
        assert!(super::super::apply_operation(&db, &old.id).is_err());
        assert_eq!(
            super::super::recover_operation(&db, &old.id)
                .unwrap()
                .status,
            "cancelled"
        );
        assert!(delete_instruction(&db, &row.id, row.revision).unwrap());
    }
    #[test]
    #[serial_test::serial]
    fn restoration_invalidates_an_open_form_even_when_resource_revision_is_unchanged() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let row = import_instruction(&db, "Draft", "Before restore").unwrap();
        let before = db.library_snapshot().unwrap();
        let mut restored = before.clone();
        restored.machine.epoch += 1;
        restored
            .definitions
            .instructions
            .get_mut(&row.id)
            .unwrap()
            .content = "Restored content".into();
        db.library_commit(&before, &restored).unwrap();
        assert!(verify_epoch(&db, before.machine.epoch).is_err());
    }
    #[test]
    #[serial_test::serial]
    fn delete_confirmation_cannot_remove_a_newer_mcp_or_skill_draft() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        super::super::mcp_resource::save_edits(&db, "demo", "Demo", None, &[]).unwrap();
        super::super::mcp_resource::save_edits(&db, "demo", "Newer", Some(1), &[]).unwrap();
        assert!(verify_mcp_delete_revision(&db, "demo", 1).is_err());
        assert!(verify_mcp_delete_revision(&db, "demo", 2).is_ok());
        let source = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("SKILL.md"),
            "---\nname: sample\ndescription: Sample\n---\nBody",
        )
        .unwrap();
        let row = import_directory(&db, source.path(), "sample", "Sample").unwrap();
        super::super::skill_resource::save_draft(&db, row.clone(), source.path(), Some(1)).unwrap();
        assert!(verify_skill_delete_revision(&db, &row.id, 1).is_err());
        assert!(verify_skill_delete_revision(&db, &row.id, 2).is_ok());
    }
    #[test]
    #[serial_test::serial]
    fn markdown_import_is_idempotent_and_does_not_assign_or_touch_native_files() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let first = import_instruction(&db, "Shared", "Use careful tests").unwrap();
        let again = import_instruction(&db, "Shared", "Use careful tests").unwrap();
        assert_eq!(first, again);
        assert_eq!(
            db.library_snapshot()
                .unwrap()
                .definitions
                .instructions
                .len(),
            1
        );
        assert!(db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
        assert!(!super::super::coordinator::native_instruction_paths().unwrap()["codex"].exists());
        assert!(import_instruction(&db, "bad", "<!-- OFOX:bad -->").is_err());
    }
    #[test]
    #[serial_test::serial]
    fn deleting_an_instruction_requires_withdrawal_and_matching_revision() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let draft = import_instruction(&db, "Shared", "Keep user content").unwrap();
        assert!(delete_instruction(&db, &draft.id, draft.revision + 1).is_err());
        let op = super::super::preview_instructions(&db, &draft.id, &["codex".into()]).unwrap();
        super::super::apply_operation(&db, &op.id).unwrap();
        assert!(delete_instruction(&db, &draft.id, draft.revision).is_err());
        let withdraw = super::super::preview_instructions(&db, &draft.id, &[]).unwrap();
        super::super::apply_operation(&db, &withdraw.id).unwrap();
        assert!(delete_instruction(&db, &draft.id, draft.revision).unwrap());
    }
    #[test]
    #[serial_test::serial]
    fn directory_import_preserves_source_and_refuses_changed_name_collisions() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let source = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("SKILL.md"),
            "---\nname: sample\ndescription: Sample\n---\nBody",
        )
        .unwrap();
        let first = import_directory(&db, source.path(), "sample", "Sample").unwrap();
        let again = import_directory(&db, source.path(), "sample", "Sample").unwrap();
        assert_eq!(first.content_hash, again.content_hash);
        assert!(db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
        std::fs::write(source.path().join("extra.txt"), "changed").unwrap();
        assert!(import_directory(&db, source.path(), "sample", "Sample").is_err());
        assert!(source.path().join("SKILL.md").exists());
        assert_eq!(db.get_all_installed_skills().unwrap().len(), 1);
    }
}
