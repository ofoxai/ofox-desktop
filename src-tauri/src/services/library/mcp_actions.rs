//! Explicit MCP duplication and native adoption. Reading a list never claims ownership.
use super::{mcp_resource, Operation};
use crate::{error::AppError, Database};
fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}
pub fn duplicate(
    db: &Database,
    source: &str,
    revision: u64,
    epoch: u64,
    id: &str,
    name: &str,
) -> Result<serde_json::Value, AppError> {
    let _io = crate::config_io::lock();
    super::catalog::verify_epoch(db, epoch)?;
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || [b'_', b'-', b'.'].contains(&b))
        || name.trim().is_empty()
        || name.len() > 1024
    {
        return Err(error("invalidMcp"));
    }
    let snapshot = db.library_snapshot()?;
    let meta = snapshot
        .definitions
        .mcp
        .get(source)
        .ok_or_else(|| error("resourceMissing"))?;
    let mut rows = db.get_all_mcp_servers()?;
    if rows.contains_key(id) {
        return Err(error("mcpNameConflict"));
    }
    let mut row = rows
        .shift_remove(source)
        .ok_or_else(|| error("resourceMissing"))?;
    if meta.revision != revision || meta.hash != mcp_resource::spec_hash(&row.server)? {
        return Err(error("revisionChanged"));
    }
    row.id = id.into();
    row.name = name.into();
    row.apps = Default::default();
    mcp_resource::save_draft(db, row.clone(), None)?;
    Ok(mcp_resource::definition_view(&row, &db.library_snapshot()?))
}
/// `tools` is always the full desired set; only `adopt` opts into native ownership.
pub fn preview_adoption(
    db: &Database,
    id: &str,
    tools: &[String],
    adopt: &[String],
) -> Result<Operation, AppError> {
    let _io = crate::config_io::lock();
    let paths = mcp_resource::native_paths()?;
    let selected = tools
        .iter()
        .map(|tool| {
            Ok((
                tool.clone(),
                paths
                    .get(tool)
                    .cloned()
                    .ok_or_else(|| error("targetUnsupported"))?,
            ))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    mcp_resource::preview_at_adoption(db, id, &selected, Some(paths), adopt)
}

#[cfg(test)]
mod tests {
    use super::super::{self as library, mcp_codec, mcp_resource};
    use super::*;
    use crate::app_config::McpServer;
    use serde_json::json;
    use std::fs;
    fn draft(db: &Database) {
        mcp_resource::save_draft(db,McpServer{id:"example".into(),name:"Example".into(),server:json!({"type":"stdio","command":"fixture-server","args":["--fixture"],"env":{"TOKEN":"${TOKEN}"}}),apps:Default::default(),description:None,homepage:None,docs:None,tags:vec![]},None).unwrap();
    }
    #[test]
    #[serial_test::serial]
    fn duplicate_keeps_private_values_on_backend_and_requires_open_revision_and_epoch() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        draft(&db);
        let before = db.library_snapshot().unwrap();
        let source = db.get_all_mcp_servers().unwrap()["example"].clone();
        duplicate(&db, "example", 1, before.machine.epoch, "renamed", "Copy").unwrap();
        assert_eq!(
            db.get_all_mcp_servers().unwrap()["renamed"].server,
            source.server
        );
        assert!(db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
        assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
        assert!(duplicate(&db, "example", 2, before.machine.epoch, "stale", "Stale").is_err());
        let before = db.library_snapshot().unwrap();
        let mut after = before.clone();
        after.machine.epoch += 1;
        db.library_commit(&before, &after).unwrap();
        assert!(duplicate(&db, "example", 1, before.machine.epoch, "stale", "Stale").is_err());
        assert!(duplicate(
            &db,
            "example",
            1,
            after.machine.epoch,
            "renamed",
            "Overwrite"
        )
        .is_err());
        assert!(!db.get_all_mcp_servers().unwrap().contains_key("stale"));
    }
    #[test]
    #[serial_test::serial]
    fn all_five_adapters_require_explicit_adoption_and_confirmation_before_ownership() {
        let _sandbox = crate::test_sandbox::sandbox();
        for tool in ["claude", "codex", "gemini", "opencode", "hermes"] {
            let db = Database::memory().unwrap();
            draft(&db);
            let path = mcp_resource::native_paths().unwrap()[tool].clone();
            let server = db.get_all_mcp_servers().unwrap()["example"].clone();
            let mut spec = mcp_codec::native_spec(tool, &server.server).unwrap();
            if tool == "claude" {
                crate::claude_mcp::prepare_mcp_spec_for_path(&mut spec, &path);
            }
            let (original, _) =
                mcp_codec::patch_server(None, tool, "example", None, Some(&spec)).unwrap();
            crate::config::atomic_write(&path, original.as_bytes()).unwrap();
            let ordinary = mcp_resource::preview(&db, "example", &[tool.into()]).unwrap();
            assert_eq!(ordinary.targets[0].status, "conflict", "{tool}");
            let op = preview_adoption(&db, "example", &[tool.into()], &[tool.into()]).unwrap();
            assert_eq!(op.targets[0].status, "pending", "{tool}");
            assert_eq!(op.targets[0].after_text, original);
            assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
            let owned: mcp_codec::OwnedFields =
                serde_json::from_str(&op.targets[0].receipt.as_ref().unwrap().owned_segment)
                    .unwrap();
            assert!(owned.created_paths.is_empty());
            assert_eq!(
                library::apply_operation(&db, &op.id).unwrap().status,
                "applied",
                "{tool}"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
            // A definition update must retain explicit container adoption proof.
            let source = db.get_all_mcp_servers().unwrap()["example"].clone();
            mcp_resource::save_draft(&db, source, Some(1)).unwrap();
            let update = mcp_resource::preview(&db, "example", &[tool.into()]).unwrap();
            let updated: mcp_codec::OwnedFields =
                serde_json::from_str(&update.targets[0].receipt.as_ref().unwrap().owned_segment)
                    .unwrap();
            assert!(updated.adopted_server);
            assert_eq!(
                library::apply_operation(&db, &update.id).unwrap().status,
                "applied"
            );
            let withdraw = mcp_resource::preview(&db, "example", &[]).unwrap();
            assert_eq!(
                library::apply_operation(&db, &withdraw.id).unwrap().status,
                "applied",
                "{tool}"
            );
            assert!(
                mcp_codec::parse_native(&fs::read_to_string(&path).unwrap(), tool).unwrap()
                    [mcp_codec::section(tool)]
                .get("example")
                .is_none(),
                "empty server left for {tool}"
            );
            fs::remove_file(path).unwrap();
        }
    }
    #[test]
    #[serial_test::serial]
    fn adoption_keeps_user_fields_and_detects_edits_between_preview_and_apply() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        draft(&db);
        let path = mcp_resource::native_paths().unwrap()["codex"].clone();
        let original="# User comment\n[ mcp_servers.example ]\ncommand='fixture-server'\nargs=['--fixture']\nenv_vars=['TOKEN']\nuser_extension='untouched'\n";
        crate::config::atomic_write(&path, original.as_bytes()).unwrap();
        let op = preview_adoption(&db, "example", &["codex".into()], &["codex".into()]).unwrap();
        assert_eq!(op.targets[0].after_text, original);
        library::apply_operation(&db, &op.id).unwrap();
        let withdraw = mcp_resource::preview(&db, "example", &[]).unwrap();
        assert_eq!(withdraw.targets[0].status, "conflict");
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let db = Database::memory().unwrap();
        draft(&db);
        let op = preview_adoption(&db, "example", &["codex".into()], &["codex".into()]).unwrap();
        let changed = format!("{original}\n# A later user edit\n");
        crate::config::atomic_write(&path, changed.as_bytes()).unwrap();
        assert_eq!(
            library::apply_operation(&db, &op.id).unwrap().status,
            "partial"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), changed);
        assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
    }
    #[test]
    #[serial_test::serial]
    fn adoption_refuses_different_values_and_preserves_the_complete_desired_set() {
        let _sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        draft(&db);
        let first = mcp_resource::preview(&db, "example", &["claude".into()]).unwrap();
        library::apply_operation(&db, &first.id).unwrap();
        let path = mcp_resource::native_paths().unwrap()["codex"].clone();
        crate::config::atomic_write(&path, b"[mcp_servers.example]\ncommand='other-server'\n")
            .unwrap();
        let refused = preview_adoption(
            &db,
            "example",
            &["claude".into(), "codex".into()],
            &["codex".into()],
        )
        .unwrap();
        assert_eq!(
            refused
                .targets
                .iter()
                .find(|t| t.tools.contains(&"codex".into()))
                .unwrap()
                .status,
            "conflict"
        );
        let op = preview_adoption(&db, "example", &["claude".into(), "codex".into()], &[]).unwrap();
        assert!(!op.desired_updates.values().any(Option::is_none));
        assert!(db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .contains_key("mcp:example:claude"));
        assert!(preview_adoption(&db, "example", &["codex".into()], &["hermes".into()]).is_err());
    }
    #[test]
    #[serial_test::serial]
    fn removing_the_last_adopted_nested_field_does_not_strand_empty_user_containers() {
        let _sandbox = crate::test_sandbox::sandbox();
        for tool in ["claude", "codex", "gemini", "opencode", "hermes"] {
            let db = Database::memory().unwrap();
            draft(&db);
            let path = mcp_resource::native_paths().unwrap()[tool].clone();
            let mut row = db.get_all_mcp_servers().unwrap()["example"].clone();
            let mut spec = mcp_codec::native_spec(tool, &row.server).unwrap();
            if tool == "claude" {
                crate::claude_mcp::prepare_mcp_spec_for_path(&mut spec, &path);
            }
            let (text, _) =
                mcp_codec::patch_server(None, tool, "example", None, Some(&spec)).unwrap();
            crate::config::atomic_write(&path, text.as_bytes()).unwrap();
            let op = preview_adoption(&db, "example", &[tool.into()], &[tool.into()]).unwrap();
            assert_eq!(
                library::apply_operation(&db, &op.id).unwrap().status,
                "applied"
            );
            row.server.as_object_mut().unwrap().remove("env");
            mcp_resource::save_draft(&db, row, Some(1)).unwrap();
            let update = mcp_resource::preview(&db, "example", &[tool.into()]).unwrap();
            assert_eq!(
                library::apply_operation(&db, &update.id).unwrap().status,
                "applied"
            );
            let withdraw = mcp_resource::preview(&db, "example", &[]).unwrap();
            assert_eq!(
                library::apply_operation(&db, &withdraw.id).unwrap().status,
                "applied",
                "{tool}"
            );
            fs::remove_file(path).unwrap();
        }
    }
}
