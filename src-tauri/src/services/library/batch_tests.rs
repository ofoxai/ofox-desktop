use super::*;
use crate::services::library::{self, coordinator, mcp_resource, InstructionSet};
use serde_json::json;

fn mcp(db: &Database, id: &str) -> BatchResource {
    mcp_resource::save_draft(
        db,
        crate::app_config::McpServer {
            id: id.into(),
            name: id.into(),
            server: json!({"command":"fixture-server","args":[id]}),
            apps: Default::default(),
            description: None,
            homepage: None,
            docs: None,
            tags: vec![],
        },
        None,
    )
    .unwrap();
    BatchResource {
        resource_id: id.into(),
        expected_revision: 1,
        add_tools: vec![],
        withdraw_tools: vec![],
    }
}
fn instruction(db: &Database, id: &str) -> BatchResource {
    library::save_instruction(
        db,
        InstructionSet {
            id: id.into(),
            name: id.into(),
            content: format!("Shared {id}"),
            revision: 0,
        },
        None,
    )
    .unwrap();
    BatchResource {
        resource_id: id.into(),
        expected_revision: 1,
        add_tools: vec![],
        withdraw_tools: vec![],
    }
}
fn id(value: &Value) -> &str {
    value["operationId"].as_str().unwrap()
}

#[test]
#[serial_test::serial]
fn two_mcp_resources_share_one_final_file_and_both_receipts() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let b = mcp(&db, "beta");
    let path = mcp_resource::native_paths().unwrap()["codex"].clone();
    crate::config::atomic_write(&path, b"# keep user comment\nmodel = \"user-model\"\n").unwrap();
    let before = std::fs::read(&path).unwrap();
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a, b],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert_eq!(plan["groups"].as_array().unwrap().len(), 1);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "preview must not write native files"
    );
    let result = run_batch(&db, id(&plan)).unwrap();
    assert_eq!(result["status"], "applied");
    assert_eq!(result["completed"], 1);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# keep user comment"));
    let native = library::mcp_codec::parse_native(&text, "codex").unwrap();
    assert_eq!(native["mcp_servers"]["alpha"]["command"], "fixture-server");
    assert_eq!(native["mcp_servers"]["beta"]["command"], "fixture-server");
    let state = db.library_snapshot().unwrap();
    assert_eq!(state.machine.receipts.len(), 2);
    assert_eq!(state.machine.assignments["mcp:alpha:codex"], "alpha");
    assert_eq!(state.machine.assignments["mcp:beta:codex"], "beta");
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn batch_mcp_with_a_literal_secret_leaves_the_config_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    mcp_resource::save_draft(
        &db,
        crate::app_config::McpServer {
            id: "secret".into(),
            name: "secret".into(),
            server: json!({"command":"fixture-server","env":{"TOKEN":"literal-token"}}),
            apps: Default::default(),
            description: None,
            homepage: None,
            docs: None,
            tags: vec![],
        },
        None,
    )
    .unwrap();
    let resource = BatchResource {
        resource_id: "secret".into(),
        expected_revision: 1,
        add_tools: vec![],
        withdraw_tools: vec![],
    };
    let path = mcp_resource::native_paths().unwrap()["codex"].clone();
    crate::config::atomic_write(&path, b"model = \"user-model\"\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![resource],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();

    assert_eq!(run_batch(&db, id(&plan)).unwrap()["status"], "applied");

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
#[serial_test::serial]
fn a_batch_applied_and_withdrawn_leaves_each_config_as_it_was() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let paths = mcp_resource::native_paths().unwrap();
    let originals = [
        ("codex", "model = \"user-model\"\n"),
        (
            "gemini",
            "{\n  \"model\": {\n    \"name\": \"personal\"\n  }\n}\n",
        ),
        ("hermes", "model:\n  default: personal\n"),
        ("openclaw", "{\n  models: {mode: 'merge'}\n}\n"),
        ("workbuddy", "{\n  \"userField\": true\n}\n"),
        ("pi", "{\n  \"userField\": true\n}\n"),
        ("dsh", "# model routing stays untouched\n- id: model\n  name: provider\n  config:\n    baseURL: https://ofox.ai/v1\n"),
    ];
    for (tool, text) in originals {
        crate::config::atomic_write(&paths[tool], text.as_bytes()).unwrap();
    }
    let tools: Vec<String> = originals.iter().map(|(tool, _)| tool.to_string()).collect();
    let (a, b) = (mcp(&db, "alpha"), mcp(&db, "beta"));
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a.clone(), b.clone()],
        tools.clone(),
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert_eq!(run_batch(&db, id(&plan)).unwrap()["status"], "applied");
    let epoch = db.library_snapshot().unwrap().machine.epoch;
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a, b],
        tools,
        BatchAction::Withdraw,
        epoch,
    )
    .unwrap();
    assert_eq!(run_batch(&db, id(&plan)).unwrap()["status"], "applied");
    for (tool, text) in originals {
        assert_eq!(
            std::fs::read_to_string(&paths[tool]).unwrap(),
            text,
            "{tool}"
        );
    }
}

#[test]
#[serial_test::serial]
fn batch_children_cannot_be_executed_by_single_resource_api() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    let child = db
        .library_snapshot()
        .unwrap()
        .machine
        .operations
        .values()
        .find(|op| op.id.starts_with("batch-child:"))
        .unwrap()
        .id
        .clone();
    assert!(coordinator::apply_operation(&db, &child).is_err());
    assert!(coordinator::cancel_operation(&db, &child).is_err());
    assert!(coordinator::recover_operation(&db, &child).is_err());
    assert_eq!(run_batch(&db, id(&plan)).unwrap()["status"], "applied");
}

#[test]
#[serial_test::serial]
fn multiple_instruction_sets_for_one_agent_are_blocked_without_native_changes() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = instruction(&db, "alpha");
    let b = instruction(&db, "beta");
    let plan = preview_batch(
        &db,
        ResourceKind::Instructions,
        vec![a, b],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert_eq!(plan["status"], "blocked");
    let path = coordinator::native_instruction_paths().unwrap()["codex"].clone();
    assert!(!path.exists());
    assert_ne!(run_batch(&db, id(&plan)).unwrap()["status"], "applied");
}

#[test]
#[serial_test::serial]
fn withdrawing_selected_agent_preserves_another_agents_assignment() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let initial = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a.clone()],
        vec!["claude".into(), "codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    run_batch(&db, id(&initial)).unwrap();
    let path = mcp_resource::native_paths().unwrap()["codex"].clone();
    let unchanged = std::fs::read(&path).unwrap();
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["claude".into()],
        BatchAction::Withdraw,
        0,
    )
    .unwrap();
    run_batch(&db, id(&plan)).unwrap();
    let state = db.library_snapshot().unwrap();
    assert!(!state.machine.assignments.contains_key("mcp:alpha:claude"));
    assert_eq!(state.machine.assignments["mcp:alpha:codex"], "alpha");
    assert_eq!(std::fs::read(path).unwrap(), unchanged);
}

#[test]
#[serial_test::serial]
fn mixed_deltas_use_one_confirmation_without_erasing_unselected_targets() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let b = mcp(&db, "beta");
    let initial = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a.clone()],
        vec!["claude".into(), "codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    run_batch(&db, id(&initial)).unwrap();
    let mut a = a;
    a.withdraw_tools = vec!["claude".into()];
    let mut b = b;
    b.add_tools = vec!["codex".into()];
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a, b],
        vec![],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert_eq!(run_batch(&db, id(&plan)).unwrap()["status"], "applied");
    let state = db.library_snapshot().unwrap();
    assert!(!state.machine.assignments.contains_key("mcp:alpha:claude"));
    assert_eq!(state.machine.assignments["mcp:alpha:codex"], "alpha");
    assert_eq!(state.machine.assignments["mcp:beta:codex"], "beta");
}

#[test]
#[serial_test::serial]
fn native_change_after_preview_is_never_overwritten_or_marked_success() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let b = mcp(&db, "beta");
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a, b],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    let path = mcp_resource::native_paths().unwrap()["codex"].clone();
    crate::config::atomic_write(
        &path,
        b"# User changed this after preview\nmodel='private'\n",
    )
    .unwrap();
    let changed = std::fs::read(&path).unwrap();
    let result = run_batch(&db, id(&plan)).unwrap();
    assert_ne!(result["status"], "applied");
    assert_eq!(std::fs::read(&path).unwrap(), changed);
    assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
}

#[test]
#[serial_test::serial]
fn cancelling_preview_has_no_native_effect_and_cannot_be_run_later() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert_eq!(cancel_batch(&db, id(&plan)).unwrap()["status"], "cancelled");
    assert!(run_batch(&db, id(&plan)).is_err());
    assert!(!mcp_resource::native_paths().unwrap()["codex"].exists());
    assert!(db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());
}

#[test]
#[serial_test::serial]
fn reconciliation_proves_written_group_but_never_executes_unwritten_groups() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["codex".into(), "claude".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert!(run_inner(&db, id(&plan), &AtomicBool::new(false), true).is_err());
    let paths = mcp_resource::native_paths().unwrap();
    let existed: Vec<_> = ["claude", "codex"]
        .into_iter()
        .filter(|tool| paths[*tool].exists())
        .collect();
    assert_eq!(existed.len(), 1);
    assert!(
        db.library_snapshot().unwrap().machine.receipts.is_empty(),
        "simulate crash after physical write and before receipt transaction"
    );
    let result = reconcile_batch(&db, id(&plan)).unwrap();
    assert_eq!(result["completed"], 1);
    assert_eq!(db.library_snapshot().unwrap().machine.receipts.len(), 1);
    assert_eq!(
        ["claude", "codex"]
            .into_iter()
            .filter(|tool| paths[*tool].exists())
            .count(),
        1,
        "recovery may not replay the other group"
    );
}

#[test]
#[serial_test::serial]
fn cancelling_prepared_mcp_accounts_proven_write_without_running_the_remaining_group() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["codex".into(), "claude".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert!(run_inner(&db, id(&plan), &AtomicBool::new(false), true).is_err());
    let result = cancel_batch(&db, id(&plan)).unwrap();
    assert_eq!(result["status"], "cancelled");
    assert_eq!(result["completed"], 1);
    assert_eq!(db.library_snapshot().unwrap().machine.receipts.len(), 1);
    let paths = mcp_resource::native_paths().unwrap();
    assert_eq!(
        ["claude", "codex"]
            .into_iter()
            .filter(|tool| paths[*tool].exists())
            .count(),
        1
    );
}

#[test]
#[serial_test::serial]
fn cancelling_prepared_mcp_with_unknown_native_changes_retains_recovery_evidence() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert!(run_inner(&db, id(&plan), &AtomicBool::new(false), true).is_err());
    let path = &mcp_resource::native_paths().unwrap()["codex"];
    let edited = format!(
        "{}\n# external change\n",
        std::fs::read_to_string(path).unwrap()
    );
    crate::config::atomic_write(path, edited.as_bytes()).unwrap();
    let result = cancel_batch(&db, id(&plan)).unwrap();
    assert_eq!(result["status"], "needsReview");
    let (_, batches) = paired(&db).unwrap();
    assert!(!prunable(&batches.operations[id(&plan)]));
    assert_eq!(std::fs::read_to_string(path).unwrap(), edited);
    assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
}

#[test]
#[serial_test::serial]
fn detached_or_edited_child_invalidates_parent_execution() {
    let _sandbox = crate::test_sandbox::sandbox();
    for remove in [false, true] {
        let db = Database::memory().unwrap();
        let a = mcp(&db, "alpha");
        let plan = preview_batch(
            &db,
            ResourceKind::Mcp,
            vec![a],
            vec!["codex".into()],
            BatchAction::Apply,
            0,
        )
        .unwrap();
        let mut current = db.library_snapshot().unwrap();
        let child = current
            .machine
            .operations
            .keys()
            .find(|id| id.starts_with(CHILD_PREFIX))
            .unwrap()
            .clone();
        if remove {
            current.machine.operations.remove(&child);
        } else {
            current
                .machine
                .operations
                .get_mut(&child)
                .unwrap()
                .resource_id = "other".into();
        }
        db.set_setting(
            library::MACHINE_KEY,
            &serde_json::to_string(&current.machine).unwrap(),
        )
        .unwrap();
        assert!(run_batch(&db, id(&plan)).is_err());
        assert!(!mcp_resource::native_paths().unwrap()["codex"].exists());
    }
}

#[test]
#[serial_test::serial]
fn resource_revision_change_blocks_batch_and_preserves_user_file() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    let mut row = db.get_all_mcp_servers().unwrap()["alpha"].clone();
    row.server["command"] = json!("changed");
    mcp_resource::save_draft(&db, row, Some(1)).unwrap();
    let result = run_batch(&db, id(&plan)).unwrap();
    assert_ne!(result["status"], "applied");
    assert!(!mcp_resource::native_paths().unwrap()["codex"].exists());
}

#[test]
#[serial_test::serial]
fn batch_sql_exports_are_machine_free_and_restore_keeps_only_local_parent() {
    let _sandbox = crate::test_sandbox::sandbox();
    let local = Database::memory().unwrap();
    let remote = Database::memory().unwrap();
    let a = mcp(&local, "alpha");
    let local_plan = preview_batch(
        &local,
        ResourceKind::Mcp,
        vec![a],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    let b = mcp(&remote, "beta");
    let remote_plan = preview_batch(
        &remote,
        ResourceKind::Mcp,
        vec![b],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    for dump in [
        local.export_sql_string().unwrap(),
        local.export_sql_string_for_sync().unwrap(),
    ] {
        assert!(!dump.contains(SETTING_KEY));
        assert!(!dump.contains(id(&local_plan)));
    }
    let remote_raw = remote
        .get_setting(SETTING_KEY)
        .unwrap()
        .unwrap()
        .replace('\'', "''");
    let imported = format!(
        "{}\nINSERT OR REPLACE INTO settings(key,value) VALUES('{}','{}');",
        remote.export_sql_string().unwrap(),
        SETTING_KEY,
        remote_raw
    );
    local.import_sql_string(&imported).unwrap();
    let list = list_batches(&local, None).unwrap();
    assert!(list
        .as_array()
        .unwrap()
        .iter()
        .any(|b| id(b) == id(&local_plan)));
    assert!(!list
        .as_array()
        .unwrap()
        .iter()
        .any(|b| id(b) == id(&remote_plan)));
    assert!(run_batch(&local, id(&local_plan)).is_err());
    assert!(!mcp_resource::native_paths().unwrap()["codex"].exists());
}

#[test]
#[serial_test::serial]
fn corrupted_batch_journal_is_not_an_empty_success() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    db.set_setting(SETTING_KEY, "broken evidence").unwrap();
    assert!(preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["codex".into()],
        BatchAction::Apply,
        0
    )
    .is_err());
    assert!(list_batches(&db, None).is_err());
    assert_eq!(
        db.get_setting(SETTING_KEY).unwrap().as_deref(),
        Some("broken evidence")
    );
}

#[test]
#[serial_test::serial]
fn mixed_instruction_withdraw_and_add_can_replace_one_set_in_one_group() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = instruction(&db, "alpha");
    let b = instruction(&db, "beta");
    let initial = preview_batch(
        &db,
        ResourceKind::Instructions,
        vec![a.clone()],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    run_batch(&db, id(&initial)).unwrap();
    let mut a = a;
    a.withdraw_tools = vec!["codex".into()];
    let mut b = b;
    b.add_tools = vec!["codex".into()];
    let plan = preview_batch(
        &db,
        ResourceKind::Instructions,
        vec![a, b],
        vec![],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    let changes = plan["groups"][0]["changes"].as_array().unwrap();
    assert!(
        changes
            .iter()
            .any(|change| change["resourceId"] == "alpha" && change["action"] == "withdraw"),
        "the logical withdrawal remains a withdrawal when another set replaces its assignment key"
    );
    assert!(changes
        .iter()
        .any(|change| change["resourceId"] == "beta" && change["action"] == "apply"));
    assert_eq!(run_batch(&db, id(&plan)).unwrap()["status"], "applied");
    let text = std::fs::read_to_string(&coordinator::native_instruction_paths().unwrap()["codex"])
        .unwrap();
    assert!(text.contains("Shared beta"));
    assert!(!text.contains("Shared alpha"));
    assert_eq!(
        db.library_snapshot().unwrap().machine.assignments["instructions:codex"],
        "beta"
    );
}

#[test]
#[serial_test::serial]
fn multiple_skill_entries_use_independent_stages_and_preserve_sources() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let mut resources = Vec::new();
    let mut sources = Vec::new();
    for name in ["alpha", "beta"] {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("SKILL.md"),
            format!("---\nname: {name}\ndescription: fixture\n---\nBody {name}"),
        )
        .unwrap();
        let row = library::catalog::import_directory(&db, source.path(), name, name).unwrap();
        resources.push(BatchResource {
            resource_id: row.id,
            expected_revision: 1,
            add_tools: vec![],
            withdraw_tools: vec![],
        });
        sources.push(source);
    }
    let plan = preview_batch(
        &db,
        ResourceKind::Skills,
        resources.clone(),
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert_eq!(plan["groups"].as_array().unwrap().len(), 2);
    assert_eq!(run_batch(&db, id(&plan)).unwrap()["status"], "applied");
    let root = library::skill_resource::native_paths().unwrap()["codex"].clone();
    assert!(root.join("alpha/SKILL.md").exists());
    assert!(root.join("beta/SKILL.md").exists());
    let remove = preview_batch(
        &db,
        ResourceKind::Skills,
        resources,
        vec!["codex".into()],
        BatchAction::Withdraw,
        0,
    )
    .unwrap();
    assert_eq!(run_batch(&db, id(&remove)).unwrap()["status"], "applied");
    assert!(!root.join("alpha").exists());
    assert!(!root.join("beta").exists());
    assert!(sources
        .iter()
        .all(|source| source.path().join("SKILL.md").exists()));
}

#[test]
#[serial_test::serial]
fn conflicting_group_is_skipped_but_other_group_can_apply_and_dto_stays_private() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let path = mcp_resource::native_paths().unwrap()["codex"].clone();
    crate::config::atomic_write(
        &path,
        b"[mcp_servers.alpha]\ncommand='foreign-private-command'\n",
    )
    .unwrap();
    let original = std::fs::read(&path).unwrap();
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["claude".into(), "codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert!(!plan.to_string().contains("foreign-private-command"));
    assert!(!plan.to_string().contains("fixture-server"));
    let result = run_batch(&db, id(&plan)).unwrap();
    assert_eq!(result["status"], "partial");
    assert_eq!(result["completed"], 1);
    assert_eq!(std::fs::read(path).unwrap(), original);
    assert_eq!(db.library_snapshot().unwrap().machine.receipts.len(), 1);
}

#[test]
#[serial_test::serial]
fn parent_plan_edits_cannot_reuse_unchanged_child_authorization() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    let mut store: Value =
        serde_json::from_str(&db.get_setting(SETTING_KEY).unwrap().unwrap()).unwrap();
    let parent = &mut store["operations"][id(&plan)];
    let edited = format!(
        "{}\n# unrelated injected parent change\n",
        parent["groups"][0]["afterText"].as_str().unwrap()
    );
    let hash = coordinator::fingerprint(Some(edited.as_bytes()));
    parent["groups"][0]["afterText"] = json!(edited);
    parent["groups"][0]["afterHash"] = json!(hash);
    parent["plans"][0]["targets"][0]["afterHash"] = json!(hash);
    db.set_setting(SETTING_KEY, &serde_json::to_string(&store).unwrap())
        .unwrap();
    assert!(
        run_batch(&db, id(&plan)).is_err(),
        "private parent plan needs independently matching child evidence"
    );
    assert!(!mcp_resource::native_paths().unwrap()["codex"].exists());
}

#[test]
#[serial_test::serial]
fn withdrawing_pending_assignment_without_native_receipt_still_updates_desired() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let before = db.library_snapshot().unwrap();
    let mut next = before.clone();
    next.machine
        .assignments
        .insert("mcp:alpha:codex".into(), "alpha".into());
    db.library_commit(&before, &next).unwrap();
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![a],
        vec!["codex".into()],
        BatchAction::Withdraw,
        0,
    )
    .unwrap();
    assert!(plan["groups"].as_array().unwrap().is_empty());
    assert_eq!(run_batch(&db, id(&plan)).unwrap()["status"], "applied");
    assert!(!db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .contains_key("mcp:alpha:codex"));
    assert!(!mcp_resource::native_paths().unwrap()["codex"].exists());
}

#[test]
#[serial_test::serial]
fn metadata_only_changes_confirm_names_intent_and_completed_status_without_a_fake_file() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = mcp(&db, "alpha");
    let before = db.library_snapshot().unwrap();
    let mut next = before.clone();
    next.machine
        .assignments
        .insert("mcp:alpha:codex".into(), "alpha".into());
    db.library_commit(&before, &next).unwrap();
    let preview = |resource: BatchResource| {
        preview_batch(
            &db,
            ResourceKind::Mcp,
            vec![resource],
            vec!["codex".into()],
            BatchAction::Withdraw,
            0,
        )
        .unwrap()
    };
    let cancelled = preview(a.clone());
    let expected = json!({
        "resourceId": "alpha", "name": "alpha", "action": "withdraw",
        "tools": ["codex"], "changedFields": [], "method": null, "status": "pending"
    });
    assert!(cancelled["groups"].as_array().unwrap().is_empty());
    assert_eq!(cancelled["metadataChanges"], json!([expected]));
    assert_eq!(
        cancel_batch(&db, id(&cancelled)).unwrap()["metadataChanges"][0]["status"],
        "cancelled"
    );
    assert_eq!(
        db.library_snapshot().unwrap().machine.assignments["mcp:alpha:codex"],
        "alpha"
    );
    let plan = preview(a);
    let applied = run_batch(&db, id(&plan)).unwrap();
    assert_eq!(applied["metadataChanges"][0]["status"], "applied");
    assert_eq!(applied["metadataChanges"][0]["action"], "withdraw");
    assert!(!mcp_resource::native_paths().unwrap()["codex"].exists());
}

#[test]
#[serial_test::serial]
fn mixed_instruction_preview_shows_final_set_even_when_withdrawal_row_is_last() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let a = instruction(&db, "alpha");
    let b = instruction(&db, "beta");
    let initial = preview_batch(
        &db,
        ResourceKind::Instructions,
        vec![a.clone()],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    run_batch(&db, id(&initial)).unwrap();
    let mut a = a;
    a.withdraw_tools = vec!["codex".into()];
    let mut b = b;
    b.add_tools = vec!["codex".into()];
    let plan = preview_batch(
        &db,
        ResourceKind::Instructions,
        vec![b, a],
        vec![],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert!(
        plan["groups"][0]["afterManaged"]
            .as_str()
            .is_some_and(|text| text.contains("Shared beta")),
        "confirmation must describe the final native result independent of resource order"
    );
}

#[test]
#[serial_test::serial]
fn cancelling_a_skill_preview_discards_only_new_staging_and_remains_readable() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("SKILL.md"),
        "---\nname: sample\ndescription: sample\n---\nBody",
    )
    .unwrap();
    let skill = library::catalog::import_directory(&db, source.path(), "sample", "Sample").unwrap();
    let plan = preview_batch(
        &db,
        ResourceKind::Skills,
        vec![BatchResource {
            resource_id: skill.id,
            expected_revision: 1,
            add_tools: vec![],
            withdraw_tools: vec![],
        }],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert_eq!(cancel_batch(&db, id(&plan)).unwrap()["status"], "cancelled");
    assert_eq!(list_batches(&db, None).unwrap()[0]["status"], "cancelled");
    assert!(source.path().join("SKILL.md").exists());
    assert!(!library::skill_resource::native_paths().unwrap()["codex"]
        .join("sample")
        .exists());
}

#[test]
#[serial_test::serial]
fn byte_budget_prunes_finished_history_before_count_limit_without_touching_active_work() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let alpha = mcp(&db, "alpha");
    let original = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![alpha],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    cancel_batch(&db, id(&original)).unwrap();
    let gamma = mcp(&db, "gamma");
    let active = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![gamma],
        vec!["gemini".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    let (current, batches) = paired(&db).unwrap();
    let mut next = current.clone();
    let mut history = batches.clone();
    let template = history.operations[id(&original)].clone();
    // Frozen terminal-history fixtures: each file stays below the native 2MiB
    // limit, while fewer than twenty entries exhaust the separate store budget.
    for sequence in 0..16 {
        let mut entry = template.clone();
        entry.operation_id = uuid::Uuid::new_v4().to_string();
        entry.created_at = sequence;
        entry.children.clear();
        let after = format!("{}\n#{}", entry.groups[0].after_text, "x".repeat(2_000_000));
        let hash = coordinator::fingerprint(Some(after.as_bytes()));
        entry.groups[0].after_text = after;
        entry.groups[0].after_hash = hash.clone();
        entry.plans[0].targets[0].after_hash = hash;
        project_children(&mut entry, &mut next).unwrap();
        history.operations.insert(entry.operation_id.clone(), entry);
    }
    db.library_commit_batch(&current, &next, &batches, &history)
        .unwrap();
    let beta = mcp(&db, "beta");
    let mut server = db.get_all_mcp_servers().unwrap()["beta"].clone();
    server.server["args"] = json!(["y".repeat(900_000)]);
    mcp_resource::save_draft(&db, server, Some(1)).unwrap();
    let candidate = BatchResource {
        expected_revision: 2,
        ..beta
    };
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![candidate],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .expect("old terminal history should make room before authorizing new work");
    let (current, batches) = paired(&db).unwrap();
    assert!(batches.operations.len() < 19);
    assert!(batches.operations.contains_key(id(&plan)));
    assert!(
        batches.operations.contains_key(id(&active)),
        "pending work must never be pruned for capacity"
    );
    assert!(batches
        .operations
        .values()
        .flat_map(|batch| batch.children.keys())
        .all(|child| current.machine.operations.contains_key(child)));
    assert!(!mcp_resource::native_paths().unwrap()["codex"].exists());
}

#[test]
#[serial_test::serial]
fn confirmation_names_actions_and_mcp_field_paths_are_safe_and_frozen() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let mut resource = mcp(&db, "internal-id");
    let mut row = db.get_all_mcp_servers().unwrap()["internal-id"].clone();
    row.name = "Friendly MCP".into();
    row.server["args"] = json!(["PRIVATE-ARGUMENT-SENTINEL"]);
    mcp_resource::save_draft(&db, row, Some(1)).unwrap();
    resource.expected_revision = 2;
    let plan = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![resource.clone()],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert_eq!(plan["resources"][0]["name"], "Friendly MCP");
    let change = &plan["groups"][0]["changes"][0];
    assert_eq!(change["name"], "Friendly MCP");
    assert_eq!(change["action"], "apply");
    assert!(change["changedFields"]
        .as_array()
        .unwrap()
        .contains(&json!("command")));
    assert_eq!(plan["groups"][0]["changedTools"], json!(["codex"]));
    assert!(!plan.to_string().contains("PRIVATE-ARGUMENT-SENTINEL"));
    assert!(!plan.to_string().contains("fixture-server"));
    run_batch(&db, id(&plan)).unwrap();
    let withdrawal = preview_batch(
        &db,
        ResourceKind::Mcp,
        vec![resource],
        vec!["codex".into()],
        BatchAction::Withdraw,
        0,
    )
    .unwrap();
    assert_eq!(withdrawal["groups"][0]["changes"][0]["action"], "withdraw");
}

#[test]
#[serial_test::serial]
fn skill_confirmation_names_transfer_method_without_exposing_contents() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("SKILL.md"),
        "---\nname: sample\ndescription: sample\n---\nPRIVATE-SKILL-BODY-SENTINEL",
    )
    .unwrap();
    let row =
        library::catalog::import_directory(&db, source.path(), "sample", "Friendly skill").unwrap();
    let plan = preview_batch(
        &db,
        ResourceKind::Skills,
        vec![BatchResource {
            resource_id: row.id,
            expected_revision: 1,
            add_tools: vec![],
            withdraw_tools: vec![],
        }],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    let change = &plan["groups"][0]["changes"][0];
    assert_eq!(change["name"], "Friendly skill");
    assert_eq!(change["action"], "apply");
    assert!(matches!(
        change["method"].as_str(),
        Some("symlink" | "copy")
    ));
    assert!(!plan.to_string().contains("PRIVATE-SKILL-BODY-SENTINEL"));
}

#[test]
#[serial_test::serial]
fn completed_skill_with_unresolved_cleanup_cannot_discard_its_recovery_journal() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("SKILL.md"),
        "---\nname: sample\ndescription: sample\n---\nBody",
    )
    .unwrap();
    let row = library::catalog::import_directory(&db, source.path(), "sample", "Sample").unwrap();
    let plan = preview_batch(
        &db,
        ResourceKind::Skills,
        vec![BatchResource {
            resource_id: row.id,
            expected_revision: 1,
            add_tools: vec![],
            withdraw_tools: vec![],
        }],
        vec!["codex".into()],
        BatchAction::Apply,
        0,
    )
    .unwrap();
    assert!(run_inner(&db, id(&plan), &AtomicBool::new(false), true).is_err());
    let (_, batches) = paired(&db).unwrap();
    let recovery = std::path::Path::new(
        &batches.operations[id(&plan)].plans[0].targets[0]
            .directory
            .as_ref()
            .unwrap()
            .stage_path,
    )
    .parent()
    .unwrap()
    .to_path_buf();
    let foreign = recovery.join("foreign-sentinel");
    std::fs::write(&foreign, "preserve unrelated recovery-directory content").unwrap();
    let checked = reconcile_batch(&db, id(&plan)).unwrap();
    assert_eq!(checked["status"], "needsReview");
    assert_eq!(checked["completed"], 1);
    assert!(
        cancel_batch(&db, id(&plan)).is_err(),
        "completed native content with unresolved recovery material must remain pinned for review"
    );
    assert_eq!(list_batches(&db, None).unwrap()[0]["status"], "needsReview");
    assert!(foreign.exists());
    assert!(library::skill_resource::native_paths().unwrap()["codex"]
        .join("sample/SKILL.md")
        .exists());
    std::fs::remove_file(foreign).unwrap();
    assert_eq!(
        reconcile_batch(&db, id(&plan)).unwrap()["status"],
        "applied"
    );
}
