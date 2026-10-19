use super::*;
use std::{fs, path::Path};

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}
#[test]
#[serial_test::serial]
fn instructions_only_import_selected_user_content_without_native_changes() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let paths = super::super::coordinator::native_instruction_paths().unwrap();
    let original = "Personal rules\n<!-- OFOX:BEGIN instructions -->\nManaged rules\n<!-- OFOX:END instructions -->\nAfter\n";
    write(&paths["codex"], original);
    write(&paths["claude"], "Claude personal rules");
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    let codex = view
        .entries
        .iter()
        .find(|entry| entry.sources.iter().any(|s| s.tool_id == "codex"))
        .unwrap();
    let result = import(
        &db,
        &view.ticket_id,
        std::slice::from_ref(&codex.entry_id),
        0,
    )
    .unwrap();
    assert_eq!(result.imported, 1);
    let snapshot = db.library_snapshot().unwrap();
    assert_eq!(snapshot.definitions.instructions.len(), 1);
    assert_eq!(
        snapshot
            .definitions
            .instructions
            .values()
            .next()
            .unwrap()
            .content,
        "Personal rules\nAfter\n"
    );
    assert!(snapshot.machine.assignments.is_empty());
    assert_eq!(fs::read_to_string(&paths["codex"]).unwrap(), original);
}
#[test]
#[serial_test::serial]
fn mcp_private_values_stay_backend_and_missing_is_distinct_from_unreadable() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let paths = super::super::mcp_resource::native_paths().unwrap();
    let native = r#"{"mcpServers":{"private-server":{"command":"node","args":["private-path","--password=TOPSECRET"],"env":{"TOKEN":"TOKEN-PRIVATE"}}}}"#;
    write(&paths["claude"], native);
    write(&paths["gemini"], "broken-json");
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    assert!(view
        .sources
        .iter()
        .any(|source| source.tool_id == "gemini" && source.state == "unreadable"));
    assert!(view
        .sources
        .iter()
        .any(|source| source.tool_id == "codex" && source.state == "missing"));
    let dto = serde_json::to_string(&view).unwrap();
    for secret in ["TOPSECRET", "TOKEN-PRIVATE", "private-path"] {
        assert!(!dto.contains(secret));
    }
    let result = import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).unwrap();
    assert_eq!(result.imported, 1);
    let rows = db.get_all_mcp_servers().unwrap();
    assert_eq!(
        rows.values().next().unwrap().server["env"]["TOKEN"],
        "TOKEN-PRIVATE"
    );
    assert!(rows.values().all(|row| row.apps == Default::default()));
    assert_eq!(fs::read_to_string(&paths["claude"]).unwrap(), native);
}
#[test]
#[serial_test::serial]
fn changed_native_content_is_rejected_before_any_draft_is_created() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::coordinator::native_instruction_paths().unwrap()["codex"].clone();
    write(&path, "Before");
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    write(&path, "After");
    let result = import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).unwrap();
    assert_eq!(result.imported, 0);
    assert_eq!(result.items[0].status, "stale");
    assert!(db
        .library_snapshot()
        .unwrap()
        .definitions
        .instructions
        .is_empty());
    assert_eq!(fs::read_to_string(path).unwrap(), "After");
}

#[test]
#[serial_test::serial]
fn legacy_prompts_are_a_separate_read_only_source_and_enabled_does_not_assign() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    db.save_prompt(
        "codex",
        &crate::prompt::Prompt {
            id: "old".into(),
            name: "Old instructions".into(),
            content: "Legacy body".into(),
            description: None,
            enabled: true,
            created_at: None,
            updated_at: None,
        },
    )
    .unwrap();
    let before = serde_json::to_value(db.get_prompts("codex").unwrap()).unwrap();
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    assert_eq!(view.entries.len(), 1);
    let entry = &view.entries[0];
    assert!(view
        .sources
        .iter()
        .any(|source| source.label == "legacyPrompts"
            && source.tool_id == "codex"
            && source.count == 1));
    assert_eq!(
        import(
            &db,
            &view.ticket_id,
            std::slice::from_ref(&entry.entry_id),
            0
        )
        .unwrap()
        .imported,
        1
    );
    assert_eq!(
        serde_json::to_value(db.get_prompts("codex").unwrap()).unwrap(),
        before
    );
    assert!(db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());
}

#[test]
#[serial_test::serial]
fn codex_override_is_an_explicit_source_without_claiming_or_rewriting_native_precedence() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::coordinator::native_instruction_paths().unwrap()["codex"].clone();
    let override_path = path.with_file_name("AGENTS.override.md");
    write(&path, "Global base text");
    write(&override_path, "Global override text");
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    assert_eq!(view.entries.len(), 2);
    let source = view
        .sources
        .iter()
        .find(|source| source.label == "nativeInstructionsOverride")
        .unwrap();
    let entry = view
        .entries
        .iter()
        .find(|entry| {
            entry
                .sources
                .iter()
                .any(|reference| reference.source_id == source.source_id)
        })
        .unwrap();
    assert_eq!(
        import(
            &db,
            &view.ticket_id,
            std::slice::from_ref(&entry.entry_id),
            0
        )
        .unwrap()
        .imported,
        1
    );
    assert_eq!(
        db.library_snapshot()
            .unwrap()
            .definitions
            .instructions
            .values()
            .next()
            .unwrap()
            .content,
        "Global override text"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "Global base text");
    assert_eq!(
        fs::read_to_string(&override_path).unwrap(),
        "Global override text"
    );
    assert!(db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());
}

#[test]
#[serial_test::serial]
fn skill_variants_import_to_unique_directories_and_equal_copies_deduplicate() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let paths = super::super::skill_resource::native_paths().unwrap();
    let first = "---\nname: sample\ndescription: First\n---\nFirst body";
    let second = "---\nname: sample\ndescription: Second\n---\nSecond body";
    write(&paths["claude"].join("sample/SKILL.md"), first);
    write(&paths["codex"].join("sample/SKILL.md"), first);
    write(&paths["gemini"].join("sample/SKILL.md"), second);
    let view = scan(&db, ResourceKind::Skills).unwrap();
    assert_eq!(view.entries.len(), 2);
    assert!(view
        .entries
        .iter()
        .all(|entry| entry.conflict && entry.variant.is_some()));
    assert!(view.entries.iter().any(|entry| entry
        .sources
        .iter()
        .any(|source| source.tool_id == "codex")
        && entry
            .sources
            .iter()
            .any(|source| source.tool_id == "claude")));
    let ids = view
        .entries
        .iter()
        .map(|entry| entry.entry_id.clone())
        .collect::<Vec<_>>();
    let result = import(&db, &view.ticket_id, &ids, 0).unwrap();
    assert_eq!(result.imported, 2);
    let rows = db.get_all_installed_skills().unwrap();
    assert_eq!(
        rows.values()
            .map(|row| &row.directory)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        2
    );
    assert!(rows.values().all(|row| row.apps.is_empty()));
    assert!(db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());
    assert_eq!(
        fs::read_to_string(paths["claude"].join("sample/SKILL.md")).unwrap(),
        first
    );
    assert_eq!(
        fs::read_to_string(paths["gemini"].join("sample/SKILL.md")).unwrap(),
        second
    );
}

#[test]
#[serial_test::serial]
fn opencode_effective_layers_are_merged_without_running_a_command() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let dir = crate::opencode_config::get_opencode_dir();
    let first = r#"{"mcp":{"sample":{"type":"local","command":["must-never-be-executed"],"environment":{"PRIVATE":"kept"}}}}"#;
    let overlay = r#"{"mcp":{"sample":{"environment":{"SECOND":"also-kept"},"enabled":true}}}"#;
    write(&dir.join("opencode.json"), first);
    write(&dir.join("opencode.jsonc"), overlay);
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    assert_eq!(view.entries.len(), 1);
    assert_eq!(view.entries[0].sources.len(), 2);
    assert_eq!(
        import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0)
            .unwrap()
            .imported,
        1
    );
    let rows = db.get_all_mcp_servers().unwrap();
    let spec = &rows.values().next().unwrap().server;
    assert_eq!(spec["command"], "must-never-be-executed");
    assert_eq!(spec["env"]["PRIVATE"], "kept");
    assert_eq!(spec["env"]["SECOND"], "also-kept");
    assert!(spec.get("enabled").is_none());
    assert_eq!(
        fs::read_to_string(dir.join("opencode.json")).unwrap(),
        first
    );
}

#[test]
#[serial_test::serial]
fn adding_a_previously_missing_opencode_overlay_invalidates_the_effective_candidate() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let dir = crate::opencode_config::get_opencode_dir();
    write(
        &dir.join("opencode.json"),
        r#"{"mcp":{"sample":{"type":"local","command":["original"]}}}"#,
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    assert_eq!(view.entries.len(), 1);
    write(
        &dir.join("opencode.jsonc"),
        r#"{"mcp":{"sample":{"command":["changed"]}}}"#,
    );
    let result = import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).unwrap();
    assert_eq!(result.imported, 0);
    assert_eq!(result.items[0].status, "stale");
    assert!(db.get_all_mcp_servers().unwrap().is_empty());
}

#[test]
#[serial_test::serial]
fn tickets_expire_are_database_scoped_and_cannot_cross_restore_epochs() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let other = Database::memory().unwrap();
    let path = super::super::coordinator::native_instruction_paths().unwrap()["codex"].clone();
    write(&path, "Native personal rules");
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    let ids = [view.entries[0].entry_id.clone()];
    assert!(import(&other, &view.ticket_id, &ids, 0).is_err());
    let before = db.library_snapshot().unwrap();
    let mut next = before.clone();
    next.machine.epoch += 1;
    db.library_commit(&before, &next).unwrap();
    assert!(import(&db, &view.ticket_id, &ids, 1).is_err());
    let fresh = scan(&db, ResourceKind::Instructions).unwrap();
    pending()
        .lock()
        .unwrap()
        .get_mut(&fresh.ticket_id)
        .unwrap()
        .created = Instant::now() - TTL - Duration::from_secs(1);
    assert!(import(
        &db,
        &fresh.ticket_id,
        &[fresh.entries[0].entry_id.clone()],
        1
    )
    .is_err());
    assert!(db
        .library_snapshot()
        .unwrap()
        .definitions
        .instructions
        .is_empty());
}

#[test]
#[serial_test::serial]
fn closing_an_inventory_discards_private_definitions_and_never_imports() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::coordinator::native_instruction_paths().unwrap()["codex"].clone();
    write(&path, "Private source text");
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    assert!(discard(&view.ticket_id));
    assert!(!discard(&view.ticket_id));
    assert!(import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).is_err());
    assert!(db
        .library_snapshot()
        .unwrap()
        .definitions
        .instructions
        .is_empty());
    assert_eq!(fs::read_to_string(path).unwrap(), "Private source text");
}

#[test]
#[serial_test::serial]
fn malformed_managed_blocks_are_not_treated_as_user_content_or_empty_files() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let paths = super::super::coordinator::native_instruction_paths().unwrap();
    write(
        &paths["codex"],
        "Personal\n<!-- OFOX:BEGIN instructions -->\nMissing end",
    );
    write(&paths["claude"], "<!-- magpie:begin · written by magpie from its Library -->\nShared\n<!-- magpie:end -->\nPersonal Claude");
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    assert!(view
        .sources
        .iter()
        .any(|source| source.tool_id == "codex" && source.state == "unreadable"));
    assert_eq!(view.entries.len(), 1);
    assert_eq!(
        import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0)
            .unwrap()
            .imported,
        1
    );
    assert_eq!(
        db.library_snapshot()
            .unwrap()
            .definitions
            .instructions
            .values()
            .next()
            .unwrap()
            .content,
        "Personal Claude"
    );
}

#[test]
#[serial_test::serial]
fn duplicate_selection_is_rejected_and_exact_existing_definition_is_not_duplicated() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::coordinator::native_instruction_paths().unwrap()["codex"].clone();
    write(&path, "Same body");
    let existing = super::super::catalog::import_instruction(&db, "Saved", "Same body").unwrap();
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    let entry = &view.entries[0];
    assert_eq!(
        entry.matched_resource_ids.as_slice(),
        std::slice::from_ref(&existing.id)
    );
    assert!(import(
        &db,
        &view.ticket_id,
        &[entry.entry_id.clone(), entry.entry_id.clone()],
        0
    )
    .is_err());
    let result = import(
        &db,
        &view.ticket_id,
        std::slice::from_ref(&entry.entry_id),
        0,
    )
    .unwrap();
    assert_eq!(result.imported, 0);
    assert_eq!(result.items[0].status, "existing");
    assert_eq!(result.items[0].resource_id, Some(existing.id));
}

#[test]
#[serial_test::serial]
fn unreadable_skill_does_not_hide_a_valid_sibling_and_changes_are_stale() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let root = super::super::skill_resource::native_paths().unwrap()["codex"].clone();
    write(&root.join("a-bad/SKILL.md"), "Bad");
    // A nested directory is invalid as a skill artifact and must not abort its siblings.
    fs::create_dir_all(root.join("a-bad/SKILL.md/nested")).unwrap_err();
    fs::remove_file(root.join("a-bad/SKILL.md")).unwrap();
    fs::create_dir_all(root.join("a-bad/SKILL.md/nested")).unwrap();
    write(
        &root.join("z-good/SKILL.md"),
        "---\nname: good\n---\nGood body",
    );
    let view = scan(&db, ResourceKind::Skills).unwrap();
    assert!(view
        .sources
        .iter()
        .any(|source| source.tool_id == "codex" && source.state == "unreadable"));
    let good = view
        .entries
        .iter()
        .find(|entry| entry.name == "good")
        .unwrap();
    write(&root.join("z-good/extra.txt"), "Native edit after scan");
    let result = import(
        &db,
        &view.ticket_id,
        std::slice::from_ref(&good.entry_id),
        0,
    )
    .unwrap();
    assert_eq!(result.items[0].status, "stale");
    assert!(db.get_all_installed_skills().unwrap().is_empty());
}

#[test]
#[serial_test::serial]
fn mcp_same_name_variants_preserve_a_native_id_and_never_overwrite_library_definitions() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let paths = super::super::mcp_resource::native_paths().unwrap();
    write(
        &paths["claude"],
        r#"{"mcpServers":{"sample":{"command":"first"}}}"#,
    );
    write(
        &paths["codex"],
        "[mcp_servers.sample]\ncommand = \"second\"\n",
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    assert_eq!(view.entries.len(), 2);
    assert!(view.entries.iter().all(|entry| entry.conflict));
    let ids = view
        .entries
        .iter()
        .map(|entry| entry.entry_id.clone())
        .collect::<Vec<_>>();
    let result = import(&db, &view.ticket_id, &ids, 0).unwrap();
    assert_eq!(result.imported, 2);
    let rows = db.get_all_mcp_servers().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.contains_key("sample"));
    assert!(rows.values().all(|row| row.name == "sample"));
    assert!(db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());
}

#[test]
#[serial_test::serial]
fn equal_mcp_specs_with_different_native_names_preserve_distinct_namespaces() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let spec = serde_json::json!({"type":"stdio","command":"same-command"});
    super::super::mcp_resource::save_draft(
        &db,
        crate::app_config::McpServer {
            id: "alpha".into(),
            name: "alpha".into(),
            server: spec,
            apps: Default::default(),
            description: None,
            homepage: None,
            docs: None,
            tags: vec![],
        },
        None,
    )
    .unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["claude"].clone();
    write(
        &path,
        r#"{"mcpServers":{"beta":{"command":"same-command"}}}"#,
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    assert!(view.entries[0].matched_resource_ids.is_empty());
    let result = import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).unwrap();
    assert_eq!(result.imported, 1);
    assert_eq!(result.items[0].resource_id.as_deref(), Some("beta"));
    let rows = db.get_all_mcp_servers().unwrap();
    assert!(rows.contains_key("alpha") && rows.contains_key("beta"));
}

#[test]
#[serial_test::serial]
fn gemini_auto_transport_is_reported_unsupported_without_coercing_other_valid_entries() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["gemini"].clone();
    let original = r#"{"mcpServers":{"automatic":{"url":"https://example.invalid/mcp"},"explicit":{"url":"https://example.invalid/mcp","type":"sse"},"legacyHttp":{"httpUrl":"https://example.invalid/mcp"}}}"#;
    write(&path, original);
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    assert_eq!(view.entries.len(), 2);
    assert!(!view.entries.iter().any(|entry| entry.name == "automatic"));
    let source = view
        .sources
        .iter()
        .find(|source| source.tool_id == "gemini")
        .unwrap();
    assert_eq!(source.state, "unsupported");
    assert_eq!(source.count, 2);
    assert_eq!(
        source.reason.as_deref(),
        Some("library.inventoryTransportUndetermined")
    );
    assert_eq!(
        import(
            &db,
            &view.ticket_id,
            &view
                .entries
                .iter()
                .map(|entry| entry.entry_id.clone())
                .collect::<Vec<_>>(),
            0
        )
        .unwrap()
        .imported,
        2
    );
    let rows = db.get_all_mcp_servers().unwrap();
    assert_eq!(rows["explicit"].server["type"], "sse");
    assert_eq!(rows["legacyHttp"].server["type"], "http");
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn replacing_an_alias_with_identical_bytes_at_another_physical_path_is_stale() {
    use std::os::unix::fs::symlink;
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::coordinator::native_instruction_paths().unwrap()["codex"].clone();
    let first = sandbox.home().join("first.md");
    let second = sandbox.home().join("second.md");
    write(&first, "Same text");
    write(&second, "Same text");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    symlink(&first, &path).unwrap();
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&second, &path).unwrap();
    let result = import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).unwrap();
    assert_eq!(result.items[0].status, "stale");
    assert!(db
        .library_snapshot()
        .unwrap()
        .definitions
        .instructions
        .is_empty());
}

#[test]
#[serial_test::serial]
fn scan_exposes_safe_source_locations_without_private_paths_or_values() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["claude"].clone();
    write(
        &path,
        r#"{"mcpServers":{"sample":{"command":"PRIVATE-COMMAND","env":{"TOKEN":"PRIVATE-TOKEN"}}}}"#,
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    let json = serde_json::to_value(&view).unwrap();
    assert!(
        json["observedAtMs"].as_i64().is_some_and(|value| value > 0),
        "read-only discovery needs an observation timestamp"
    );
    let source = json["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["toolId"] == "claude")
        .unwrap();
    assert_eq!(source["locationKind"], "file");
    assert!(source["sourceRef"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    let serialized = serde_json::to_string(&view).unwrap();
    for private in [path.to_str().unwrap(), "PRIVATE-COMMAND", "PRIVATE-TOKEN"] {
        assert!(!serialized.contains(private));
    }
}

#[test]
#[serial_test::serial]
fn source_search_host_excludes_url_path_query_userinfo_and_non_http_schemes() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["claude"].clone();
    write(
        &path,
        r#"{"mcpServers":{"remote":{"type":"http","url":"https://EXAMPLE.invalid/PRIVATE-PATH?token=PRIVATE-QUERY"},"credentials":{"type":"http","url":"https://user:PRIVATE-PASSWORD@example.invalid/mcp"},"local":{"type":"http","url":"file:///PRIVATE-LOCAL-PATH"}}}"#,
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    let json = serde_json::to_value(&view).unwrap();
    let rows = json["entries"].as_array().unwrap();
    assert_eq!(
        rows.iter().find(|entry| entry["name"] == "remote").unwrap()["serviceHost"],
        "example.invalid"
    );
    for name in ["credentials", "local"] {
        assert!(rows.iter().find(|entry| entry["name"] == name).unwrap()["serviceHost"].is_null());
    }
    let output = json.to_string();
    for private in [
        "PRIVATE-PATH",
        "PRIVATE-QUERY",
        "PRIVATE-PASSWORD",
        "PRIVATE-LOCAL-PATH",
    ] {
        assert!(!output.contains(private));
    }
}

#[test]
#[serial_test::serial]
fn a_library_row_with_empty_collections_matches_its_native_entry() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    super::super::mcp_resource::save_draft(
        &db,
        crate::app_config::McpServer {
            id: "codegraph".into(),
            name: "codegraph".into(),
            server: serde_json::json!({"command":"codegraph","args":["serve","--mcp"],"env":{}}),
            apps: Default::default(),
            description: None,
            homepage: None,
            docs: None,
            tags: vec![],
        },
        None,
    )
    .unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["codex"].clone();
    write(
        &path,
        "[mcp_servers.codegraph]\ncommand = \"codegraph\"\nargs = [\"serve\", \"--mcp\"]\n",
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    assert_eq!(view.entries.len(), 1);
    assert_eq!(view.entries[0].matched_resource_ids, vec!["codegraph"]);
    assert!(!view.entries[0].conflict);
    import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).unwrap();
    assert_eq!(db.get_all_mcp_servers().unwrap().len(), 1);
}

#[test]
#[serial_test::serial]
fn a_disabled_native_entry_matches_its_definition_and_is_marked_disabled() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    super::super::mcp_resource::save_draft(
        &db,
        crate::app_config::McpServer {
            id: "context7".into(),
            name: "context7".into(),
            server: serde_json::json!({"type":"http","url":"https://mcp.context7.com/mcp"}),
            apps: Default::default(),
            description: None,
            homepage: None,
            docs: None,
            tags: vec![],
        },
        None,
    )
    .unwrap();
    let paths = super::super::mcp_resource::native_paths().unwrap();
    write(
        &paths["codex"],
        "[mcp_servers.context7]\nurl = \"https://mcp.context7.com/mcp\"\nenabled = false\n",
    );
    write(
        &paths["opencode"],
        r#"{"mcp":{"context7":{"type":"remote","url":"https://mcp.context7.com/mcp","enabled":false}}}"#,
    );
    write(
        &paths["claude"],
        r#"{"mcpServers":{"context7":{"type":"http","url":"https://mcp.context7.com/mcp"}}}"#,
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    assert_eq!(view.entries.len(), 1);
    let entry = &view.entries[0];
    assert_eq!(entry.matched_resource_ids, vec!["context7"]);
    let disabled = |tool: &str| {
        entry
            .sources
            .iter()
            .find(|source| source.tool_id == tool)
            .unwrap()
            .disabled
    };
    assert!(disabled("codex"));
    assert!(disabled("opencode"));
    assert!(!disabled("claude"));
    let dto = serde_json::to_value(&view).unwrap();
    let claude = dto["entries"][0]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["toolId"] == "claude")
        .unwrap()
        .clone();
    assert!(claude.get("disabled").is_none());
}

#[test]
#[serial_test::serial]
fn servers_an_app_writes_into_its_own_bundle_are_not_offered() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let paths = super::super::mcp_resource::native_paths().unwrap();
    write(
        &paths["codex"],
        concat!(
            "[mcp_servers.filesystem]\ncommand = \"npx\"\nargs = [\"-y\", \"@modelcontextprotocol/server-filesystem\"]\n",
            "[mcp_servers.node_repl]\ncommand = \"/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl\"\n",
            "[mcp_servers.computer-use]\ncommand = \"./Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient\"\nargs = [\"mcp\"]\ncwd = \".\"\nenabled = false\n",
        ),
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    let names = view
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["filesystem"]);
    let codex = view
        .sources
        .iter()
        .find(|source| source.tool_id == "codex")
        .unwrap();
    assert_eq!(codex.state, "ready");
    assert_eq!(codex.count, 1);
}
