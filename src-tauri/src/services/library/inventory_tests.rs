use super::*;
use std::{fs, path::Path};

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
#[serial_test::serial]
fn an_oversized_skill_is_discovered_without_inspecting_its_payload() {
    for entry_limit in [true, false] {
        let sandbox = crate::test_sandbox::sandbox();
        let db = Database::memory().unwrap();
        let rejected = sandbox.home().join(".claude/skills/a-oversized");
        write(&rejected.join("SKILL.md"), "# Oversized fixture");
        if entry_limit {
            for index in 0..10_001 {
                fs::File::create(rejected.join(format!("file-{index}"))).unwrap();
            }
        } else {
            // A sparse file exercises the byte limit without allocating it.
            fs::File::create(rejected.join("large.bin"))
                .unwrap()
                .set_len(128 * 1024 * 1024 + 1)
                .unwrap();
        }
        for (directory, name) in [
            (".claude/skills", "z-healthy"),
            (".codex/skills", "native-healthy"),
            (".agents/skills", "academic-humanizer"),
            (".agents/skills", "bestpaper-writing"),
        ] {
            write(
                &sandbox.home().join(directory).join(name).join("SKILL.md"),
                &format!("---\nname: {name}\ndescription: Fixture {name}\n---\n# {name}"),
            );
        }

        let view = scan(&db, ResourceKind::Skills).unwrap();
        for name in [
            "z-healthy",
            "native-healthy",
            "academic-humanizer",
            "bestpaper-writing",
        ] {
            let entry = view
                .entries
                .iter()
                .find(|entry| entry.name == name)
                .unwrap_or_else(|| {
                    panic!(
                        "{name} was hidden after an oversized skill (entry limit: {entry_limit})"
                    )
                });
            if name == "academic-humanizer" || name == "bestpaper-writing" {
                for reader in ["codex", "copilot", "dsh", "gemini", "opencode"] {
                    assert!(
                        entry.sources.iter().any(|source| source.tool_id == reader),
                        "{name}: missing {reader}"
                    );
                }
                assert!(entry
                    .locations
                    .iter()
                    .any(|location| location.target_id == "shared"));
            }
        }
        let oversized = view
            .entries
            .iter()
            .find(|entry| entry.directory.as_deref() == Some("a-oversized"))
            .expect("a valid SKILL.md must remain discoverable even with a large payload");
        assert!(view
            .sources
            .iter()
            .filter(|source| { source.tool_id == "claude" || source.tool_id == "opencode" })
            .all(|source| source.reason.is_none()));
        assert!(db.get_all_installed_skills().unwrap().is_empty());
        let healthy = view
            .entries
            .iter()
            .find(|entry| entry.name == "z-healthy")
            .unwrap();
        let result = import(
            &db,
            &view.ticket_id,
            &[oversized.entry_id.clone(), healthy.entry_id.clone()],
            0,
        )
        .unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.items[0].status, "failed");
        assert_eq!(
            result.items[0].reason.as_deref(),
            Some("library.skillTooLarge")
        );
        assert_eq!(result.items[1].status, "imported");
        assert_eq!(db.get_all_installed_skills().unwrap().len(), 1);
        assert_eq!(
            fs::read_to_string(rejected.join("SKILL.md")).unwrap(),
            "# Oversized fixture"
        );
    }
}

#[test]
#[serial_test::serial]
fn every_native_reader_discovers_metadata_without_validating_deep_payloads() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let roots = super::super::skill_resource::native_paths().unwrap();
    for (tool, root) in roots
        .iter()
        .filter(|(tool, _)| tool.as_str() != "claude-desktop")
    {
        let skill = root.join(format!("metadata-{tool}"));
        write(
            &skill.join("SKILL.md"),
            &format!("---\nname: metadata-{tool}\n---\nInstructions"),
        );
        let mut deep = skill.join("payload");
        for _ in 0..34 {
            deep = deep.join("nested");
        }
        write(&deep.join("resource.txt"), "Not part of metadata discovery");
    }
    let view = scan(&db, ResourceKind::Skills).unwrap();
    for tool in roots
        .keys()
        .filter(|tool| tool.as_str() != "claude-desktop")
    {
        let name = format!("metadata-{tool}");
        let entry = view
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("missing {tool}"));
        assert!(
            entry.sources.iter().any(|source| source.tool_id == *tool),
            "missing reader {tool}"
        );
        assert_eq!(entry.content_verified, Some(false));
    }
    assert!(view.sources.iter().all(|source| source.reason.is_none()));
    assert!(discard(&view.ticket_id));
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn large_payloads_and_their_aliases_do_not_exhaust_metadata_discovery() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let claude = sandbox.home().join(".claude/skills");
    let shared = sandbox.home().join(".agents/skills");
    let codex = sandbox.home().join(".codex/skills");
    let oversized = |root: &Path| {
        write(&root.join("SKILL.md"), "# Rejected fixture");
        fs::File::create(root.join("large.bin"))
            .unwrap()
            .set_len(128 * 1024 * 1024 + 1)
            .unwrap();
    };
    write(&claude.join("0-accepted/SKILL.md"), "# Already accepted");
    oversized(&shared.join("a-rejected"));
    std::os::unix::fs::symlink(shared.join("a-rejected"), claude.join("a-alias")).unwrap();
    for index in 0..6 {
        oversized(&claude.join(format!("b-rejected-{index}")));
    }
    write(&shared.join("healthy-shared/SKILL.md"), "# Healthy shared");
    fs::create_dir_all(&codex).unwrap();
    std::os::unix::fs::symlink(claude.join("0-accepted"), codex.join("0-accepted")).unwrap();
    oversized(&codex.join("z-eighth-rejection"));
    write(
        &codex.join("zzz-unchecked/SKILL.md"),
        "# Still discoverable",
    );

    let view = scan(&db, ResourceKind::Skills).unwrap();
    let names: BTreeSet<_> = view
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(view.entries.len(), 11);
    for name in ["0-accepted", "healthy-shared", "zzz-unchecked"] {
        assert!(names.contains(name), "missing {name}");
    }
    assert!(view
        .entries
        .iter()
        .all(|entry| entry.content_verified == Some(false)));
    let shared_entry = view
        .entries
        .iter()
        .find(|entry| entry.name == "healthy-shared")
        .unwrap();
    for reader in ["codex", "copilot", "dsh", "gemini", "opencode"] {
        assert!(
            shared_entry
                .sources
                .iter()
                .any(|source| source.tool_id == reader),
            "{reader}"
        );
    }
    let accepted = view
        .entries
        .iter()
        .find(|entry| entry.name == "0-accepted")
        .unwrap();
    assert!(accepted
        .sources
        .iter()
        .any(|source| source.tool_id == "codex"));
    assert!(view.sources.iter().all(|source| source.reason.is_none()));
    assert!(codex.join("zzz-unchecked/SKILL.md").is_file());
    assert!(discard(&view.ticket_id));
}

#[test]
#[serial_test::serial]
fn equal_manifests_do_not_make_different_skill_payloads_equal() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let roots = super::super::skill_resource::native_paths().unwrap();
    let manifest = "---\nname: shared-title\n---\nSame instructions";
    for (tool, content) in [("claude", "First assets"), ("codex", "Different assets")] {
        write(&roots[tool].join("same/SKILL.md"), manifest);
        write(&roots[tool].join("same/assets/icon.svg"), content);
    }
    let view = scan(&db, ResourceKind::Skills).unwrap();
    assert_eq!(view.entries.len(), 2);
    assert!(view
        .entries
        .iter()
        .all(|entry| entry.matched_resource_ids.is_empty()));
    let result = import(
        &db,
        &view.ticket_id,
        &view
            .entries
            .iter()
            .map(|entry| entry.entry_id.clone())
            .collect::<Vec<_>>(),
        0,
    )
    .unwrap();
    assert_eq!(result.imported, 2);
    let rows = db.get_all_installed_skills().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.values()
            .filter_map(|row| row.content_hash.clone())
            .collect::<BTreeSet<_>>()
            .len(),
        2
    );
    for (tool, content) in [("claude", "First assets"), ("codex", "Different assets")] {
        assert_eq!(
            fs::read_to_string(roots[tool].join("same/assets/icon.svg")).unwrap(),
            content
        );
    }
}

#[test]
#[serial_test::serial]
fn skill_import_captures_the_current_payload_after_metadata_discovery() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let root = super::super::skill_resource::native_paths().unwrap()["claude"].join("current");
    write(
        &root.join("SKILL.md"),
        "---\nname: current\n---\nInstructions",
    );
    let view = scan(&db, ResourceKind::Skills).unwrap();
    write(&root.join("new-resource.txt"), "Added after discovery");
    let result = import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).unwrap();
    assert_eq!(result.imported, 1);
    let row = db
        .get_installed_skill(result.items[0].resource_id.as_ref().unwrap())
        .unwrap()
        .unwrap();
    let artifact = super::super::skill_artifact::path(row.content_hash.as_ref().unwrap()).unwrap();
    assert_eq!(
        fs::read_to_string(artifact.join("new-resource.txt")).unwrap(),
        "Added after discovery"
    );
    assert_eq!(
        fs::read_to_string(root.join("new-resource.txt")).unwrap(),
        "Added after discovery"
    );
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn metadata_discovery_does_not_follow_payload_links_and_import_still_refuses_them() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let root = sandbox.home().join(".claude/skills/linked-payload");
    write(
        &root.join("SKILL.md"),
        "---\nname: linked-payload\n---\nInstructions",
    );
    let outside = sandbox.home().join("private.txt");
    write(&outside, "Leave this file alone");
    std::os::unix::fs::symlink(&outside, root.join("payload.txt")).unwrap();
    let view = scan(&db, ResourceKind::Skills).unwrap();
    assert_eq!(view.entries.len(), 1);
    assert!(view.sources.iter().all(|source| source.reason.is_none()));
    let result = import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).unwrap();
    assert_eq!(result.items[0].status, "failed");
    assert!(db.get_all_installed_skills().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "Leave this file alone"
    );
    assert!(fs::symlink_metadata(root.join("payload.txt"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn a_retargeted_skill_link_expires_selection_even_with_the_same_manifest() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let manifest = "---\nname: alias\n---\nInstructions";
    let first = sandbox.home().join("first");
    let second = sandbox.home().join("second");
    for root in [&first, &second] {
        write(&root.join("SKILL.md"), manifest);
    }
    let link = sandbox.home().join(".claude/skills/alias");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&first, &link).unwrap();
    let view = scan(&db, ResourceKind::Skills).unwrap();
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&second, &link).unwrap();
    let result = import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).unwrap();
    assert_eq!(result.items[0].status, "stale");
    assert!(db.get_all_installed_skills().unwrap().is_empty());
    for root in [&first, &second] {
        assert_eq!(fs::read_to_string(root.join("SKILL.md")).unwrap(), manifest);
    }
}

#[test]
#[serial_test::serial]
fn unreadable_and_oversized_skill_manifests_do_not_hide_healthy_metadata() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let root = sandbox.home().join(".claude/skills");
    write(&root.join("a-large/SKILL.md"), &"x".repeat(1024 * 1024 + 1));
    fs::create_dir_all(root.join("b-invalid")).unwrap();
    fs::write(root.join("b-invalid/SKILL.md"), [0xff, 0xfe]).unwrap();
    write(
        &root.join("z-valid/SKILL.md"),
        "---\nname: valid\n---\nInstructions",
    );
    let view = scan(&db, ResourceKind::Skills).unwrap();
    assert_eq!(view.entries.len(), 1);
    assert_eq!(view.entries[0].name, "valid");
    assert!(view
        .sources
        .iter()
        .any(|source| source.tool_id == "claude" && source.state == "unreadable"));
    assert!(discard(&view.ticket_id));
}

#[test]
#[serial_test::serial]
fn truncated_discovery_directories_report_partial_even_when_children_are_hidden() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let root = sandbox.home().join(".claude/skills");
    fs::create_dir_all(&root).unwrap();
    for index in 0..10_002 {
        fs::File::create(root.join(format!(".hidden-{index}"))).unwrap();
    }
    write(&root.join("visible/SKILL.md"), "Visible instructions");
    let view = scan(&db, ResourceKind::Skills).unwrap();
    assert!(view.sources.iter().any(|source| source.tool_id == "claude"
        && source.reason.as_deref() == Some("library.inventoryLimit")));
    assert!(discard(&view.ticket_id));
}

#[test]
#[serial_test::serial]
fn explicit_skill_comparison_does_not_match_a_damaged_managed_payload() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let root = super::super::skill_resource::native_paths().unwrap()["claude"].join("compare");
    write(
        &root.join("SKILL.md"),
        "---\nname: compare\n---\nInstructions",
    );
    write(&root.join("assets.txt"), "Original assets");
    let view = scan(&db, ResourceKind::Skills).unwrap();
    let imported = import(&db, &view.ticket_id, &[view.entries[0].entry_id.clone()], 0).unwrap();
    let id = imported.items[0].resource_id.as_ref().unwrap();
    let row = db.get_installed_skill(id).unwrap().unwrap();
    let artifact = super::super::skill_artifact::path(row.content_hash.as_ref().unwrap()).unwrap();
    let view = scan(&db, ResourceKind::Skills).unwrap();
    let entry = &view.entries[0];
    let source = &entry.sources[0].source_id;
    assert_eq!(
        source_detail(&db, &view.ticket_id, &entry.entry_id, source, 0)
            .unwrap()
            .matched_resource_ids
            .as_slice(),
        std::slice::from_ref(id)
    );
    write(&artifact.join("assets.txt"), "Damaged managed assets");
    assert!(
        source_detail(&db, &view.ticket_id, &entry.entry_id, source, 0)
            .unwrap()
            .matched_resource_ids
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(root.join("assets.txt")).unwrap(),
        "Original assets"
    );
    assert!(discard(&view.ticket_id));
}

#[test]
#[ignore = "Read-only native acceptance; explicitly set OFOX_SKILL_DISCOVERY_HOME"]
#[serial_test::serial]
fn native_skill_metadata_discovery_acceptance() {
    let home = PathBuf::from(
        std::env::var_os("OFOX_SKILL_DISCOVERY_HOME").expect("explicit read-only source home"),
    );
    assert!(home.is_absolute());
    let _sandbox = crate::test_sandbox::sandbox();
    let mut settings = crate::settings::get_settings();
    // Only these source roots are real. Settings, DB, ownership and all writes
    // remain in the temporary home. Never import or remove a live skill here.
    settings.claude_config_dir = Some(home.join(".claude").to_string_lossy().into_owned());
    settings.codex_config_dir = Some(home.join(".agents").to_string_lossy().into_owned());
    crate::settings::update_settings(settings).unwrap();
    let db = Database::memory().unwrap();
    let started = Instant::now();
    let view = scan(&db, ResourceKind::Skills).unwrap();
    let elapsed = started.elapsed();
    for name in ["ppt-master", "academic-humanizer", "bestpaper-writing"] {
        assert!(
            view.entries.iter().any(|entry| entry.name == name),
            "missing {name}"
        );
    }
    for tool in ["claude", "codex"] {
        assert!(
            view.sources
                .iter()
                .filter(|source| source.tool_id == tool)
                .all(|source| source.reason.is_none()),
            "partial {tool}"
        );
    }
    assert!(db.get_all_installed_skills().unwrap().is_empty());
    assert!(db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());
    println!(
        "native skill metadata discovery: {} entries in {} ms; all three targets visible",
        view.entries.len(),
        elapsed.as_millis()
    );
    assert!(discard(&view.ticket_id));
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
fn dsh_generated_profile_comments_do_not_make_read_only_mcp_discovery_incomplete() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    // This is the header generated by dsh 0.2.1-rc.2, including the expression
    // documentation. Describing YAML tags in a comment must not activate them.
    let header = concat!(
        "# Your patch layer for this dsh profile, applied after every bundle layer:\n",
        "# a top-level YAML array of loader patch entries (id-targeted config\n",
        "# overrides, disables, and insert lists; `!!js` expressions allowed).\n",
    );
    let web =
        crate::test_sandbox::native(sandbox.home().join(".dsh/profiles/web/cordis.patch.yml"));
    let headless = crate::test_sandbox::native(
        sandbox
            .home()
            .join(".dsh/profiles/headless/cordis.patch.yml"),
    );
    let web_original = format!("{header}[]\n");
    let headless_original = format!(
        "{header}- id: agent-default-model\n  config:\n    provider: fixture\n    model: fixture-model\n"
    );
    write(&web, &web_original);
    write(&headless, &headless_original);

    let empty = scan(&db, ResourceKind::Mcp).unwrap();
    for path in [&web, &headless] {
        let source = empty
            .sources
            .iter()
            .find(|source| {
                matches!(&source.location, Some(SourceLocation::File(found)) if found == path)
            })
            .unwrap();
        assert_eq!(source.tool_id, "dsh");
        assert_eq!(source.state, "ready");
        assert_eq!(source.count, 0);
    }
    assert!(empty.entries.is_empty());
    assert!(db.get_all_mcp_servers().unwrap().is_empty());
    assert_eq!(fs::read_to_string(&web).unwrap(), web_original);
    assert_eq!(fs::read_to_string(&headless).unwrap(), headless_original);

    let web_with_mcp = format!(
        "{header}- id: fixture-mcp\n  name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    transport: stdio\n    serverName: fixture-mcp\n    command: fixture-command\n"
    );
    write(&web, &web_with_mcp);
    let found = scan(&db, ResourceKind::Mcp).unwrap();
    for (path, expected_count) in [(&web, 1), (&headless, 0)] {
        let source = found
            .sources
            .iter()
            .find(|source| {
                matches!(&source.location, Some(SourceLocation::File(found)) if found == path)
            })
            .unwrap();
        assert_eq!(source.state, "ready");
        assert_eq!(source.count, expected_count);
    }
    assert_eq!(found.entries.len(), 1);
    let entry = &found.entries[0];
    assert_eq!(entry.name, "fixture-mcp");
    assert!(entry.sources.iter().all(|source| source.tool_id == "dsh"));
    assert!(db.get_all_mcp_servers().unwrap().is_empty());
    assert!(db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());

    let result = import(
        &db,
        &found.ticket_id,
        std::slice::from_ref(&entry.entry_id),
        0,
    )
    .unwrap();
    assert_eq!(result.imported, 1);
    assert_eq!(db.get_all_mcp_servers().unwrap().len(), 1);
    assert!(db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());
    assert_eq!(fs::read_to_string(&web).unwrap(), web_with_mcp);
    assert_eq!(fs::read_to_string(&headless).unwrap(), headless_original);
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
    assert_eq!(view.entries.len(), 3);
    assert!(view
        .entries
        .iter()
        .all(|entry| entry.conflict && entry.variant.is_some()));
    assert!(!view.entries.iter().any(|entry| entry
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
    // Only the copy that kept the folder name takes over the agents it came from.
    let plain = rows.values().find(|row| row.directory == "sample").unwrap();
    let assigned: Vec<String> = db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .into_iter()
        .map(|(key, resource)| {
            assert_eq!(resource, plain.id);
            key
        })
        .collect();
    assert!(!assigned.is_empty());
    assert!(assigned
        .iter()
        .all(|key| key.starts_with(&format!("skills:{}:", plain.id))));
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
fn unreadable_manifest_does_not_hide_a_valid_sibling_and_manifest_changes_are_stale() {
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
    write(&root.join("z-good/SKILL.md"), "Changed manifest after scan");
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

#[cfg(unix)]
fn shared_with_a_claude_link(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let shared = super::super::skill_resource::target_paths().unwrap()["shared"].join(name);
    write(
        &shared.join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: Shared\n---\nbody"),
    );
    let claude = super::super::skill_resource::native_paths().unwrap()["claude"].clone();
    fs::create_dir_all(&claude).unwrap();
    std::os::unix::fs::symlink(format!("../../.agents/skills/{name}"), claude.join(name)).unwrap();
    (shared, claude.join(name))
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn importing_a_found_skill_takes_over_its_folder_and_links_in_place() {
    use std::os::unix::fs::MetadataExt;
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let (shared, link) = shared_with_a_claude_link("automate");
    let inode = fs::metadata(shared.join("SKILL.md")).unwrap().ino();
    let view = scan(&db, ResourceKind::Skills).unwrap();
    let entry = view
        .entries
        .iter()
        .find(|entry| entry.name == "automate")
        .unwrap();
    let mut places: Vec<(String, String, bool)> = entry
        .locations
        .iter()
        .map(|place| {
            (
                place.target_id.clone(),
                place.display_path.clone(),
                place.adoptable,
            )
        })
        .collect();
    places.sort();
    assert_eq!(
        places,
        vec![
            ("claude".into(), "~/.claude/skills/automate".into(), true),
            ("shared".into(), "~/.agents/skills/automate".into(), true),
        ]
    );
    let serialized = serde_json::to_string(&view).unwrap();
    assert!(!serialized.contains(&*sandbox.home().to_string_lossy()));
    let result = import(
        &db,
        &view.ticket_id,
        std::slice::from_ref(&entry.entry_id),
        0,
    )
    .unwrap();
    let item = &result.items[0];
    let mut adopted = item.adopted.clone();
    adopted.sort();
    assert_eq!(adopted, vec!["claude", "shared"]);
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::metadata(shared.join("SKILL.md")).unwrap().ino(), inode);
    let row = db
        .get_installed_skill(item.resource_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    let view = super::super::skill_view::definition_view(&db, &row).unwrap();
    let status = |tool: &str| {
        view["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["toolId"] == tool)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status("claude"), "applied");
    assert_eq!(status("shared"), "applied");
    assert_eq!(status("codex"), "shared");
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn a_found_skill_is_taken_out_of_its_agents_into_the_backups() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let (shared, link) = shared_with_a_claude_link("babysit");
    let view = scan(&db, ResourceKind::Skills).unwrap();
    let entry = view
        .entries
        .iter()
        .find(|entry| entry.name == "babysit")
        .unwrap();
    let removed = remove_found(&db, &view.ticket_id, &entry.entry_id, &[], 0).unwrap();
    assert_eq!(removed.len(), 2);
    assert!(fs::symlink_metadata(&link).is_err());
    assert!(fs::symlink_metadata(&shared).is_err());
    let backups = crate::services::skill::SkillService::list_backups().unwrap();
    let kept = backups
        .iter()
        .find(|backup| backup.skill.directory == "babysit")
        .unwrap();
    assert!(Path::new(&kept.backup_path).join("skill/SKILL.md").exists());
    let after = scan(&db, ResourceKind::Skills).unwrap();
    assert!(after.entries.iter().all(|entry| entry.name != "babysit"));
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn a_skill_the_library_owns_is_not_removed_as_a_found_one() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let (shared, _) = shared_with_a_claude_link("canvas");
    let view = scan(&db, ResourceKind::Skills).unwrap();
    let entry = view
        .entries
        .iter()
        .find(|entry| entry.name == "canvas")
        .unwrap();
    import(
        &db,
        &view.ticket_id,
        std::slice::from_ref(&entry.entry_id),
        0,
    )
    .unwrap();
    let epoch = db.library_snapshot().unwrap().machine.epoch;
    let again = scan(&db, ResourceKind::Skills).unwrap();
    let entry = again
        .entries
        .iter()
        .find(|entry| entry.name == "canvas")
        .unwrap();
    assert!(entry
        .locations
        .iter()
        .all(|place| place.reason.as_deref() == Some("library.skillOwned")));
    assert!(remove_found(&db, &again.ticket_id, &entry.entry_id, &[], epoch).is_err());
    assert!(shared.join("SKILL.md").exists());
}
