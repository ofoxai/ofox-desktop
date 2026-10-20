use super::*;
use serde_json::json;
use std::{fs, path::Path};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn native(view: &LibraryInventory, tool: &str) -> (String, String) {
    let source = view
        .sources
        .iter()
        .find(|source| source.tool_id == tool && source.count > 0)
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
    (entry.entry_id.clone(), source.source_ref.clone().unwrap())
}
fn server(id: &str, spec: serde_json::Value) -> McpServer {
    McpServer {
        id: id.into(),
        name: id.into(),
        server: spec,
        apps: Default::default(),
        description: None,
        homepage: None,
        docs: None,
        tags: vec![],
    }
}
#[test]
#[serial_test::serial]
fn source_details_are_explicit_and_never_reveal_config_values() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["claude"].clone();
    let original = r#"{"mcpServers":{"sample":{"command":"PRIVATE-COMMAND","args":["PRIVATE-ARG"],"env":{"TOKEN":"PRIVATE-TOKEN"}}}}"#;
    write(&path, original);
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    let (entry, source) = native(&view, "claude");
    let detail = source_detail(&db, &view.ticket_id, &entry, &source, 0).unwrap();
    assert_eq!(detail.logical_path.as_deref(), path.to_str());
    assert!(detail.can_reveal);
    assert!(detail
        .fields
        .iter()
        .any(|field| field.path == ["env", "TOKEN"] && field.value_type == "string"));
    let output = serde_json::to_string(&detail).unwrap();
    for secret in ["PRIVATE-COMMAND", "PRIVATE-ARG", "PRIVATE-TOKEN"] {
        assert!(!output.contains(secret));
    }
    let mut revealed = None;
    reveal_source(&db, &view.ticket_id, &entry, &source, 0, |path| {
        revealed = Some(path.to_owned());
        Ok(())
    })
    .unwrap();
    assert_eq!(
        revealed.as_deref(),
        Some(path.canonicalize().unwrap().as_path())
    );
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    assert!(db.library_snapshot().unwrap().machine.receipts.is_empty());
}
#[test]
#[serial_test::serial]
fn detail_rechecks_source_entry_owner_and_all_ticket_invalidation_boundaries() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let other = Database::memory().unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["claude"].clone();
    write(
        &path,
        r#"{"mcpServers":{"a":{"command":"node"},"b":{"command":"other"}}}"#,
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    let (entry, source) = native(&view, "claude");
    assert!(source_detail(&other, &view.ticket_id, &entry, &source, 0).is_err());
    assert!(source_detail(&db, &view.ticket_id, "unrelated", &source, 0).is_err());
    assert!(source_detail(&db, &view.ticket_id, &entry, "arbitrary-path", 0).is_err());
    let before = db.library_snapshot().unwrap();
    let mut after = before.clone();
    after.machine.epoch += 1;
    db.library_commit(&before, &after).unwrap();
    assert!(source_detail(&db, &view.ticket_id, &entry, &source, 0).is_err());
    let fresh = scan(&db, ResourceKind::Mcp).unwrap();
    let (entry, source) = native(&fresh, "claude");
    std::env::set_var("OPENCODE_CONFIG", sandbox.home().join("other-config.json"));
    assert!(source_detail(&db, &fresh.ticket_id, &entry, &source, 1).is_err());
    std::env::remove_var("OPENCODE_CONFIG");
    let fresh = scan(&db, ResourceKind::Mcp).unwrap();
    let (entry, source) = native(&fresh, "claude");
    fs::remove_file(&path).unwrap();
    let mut called = false;
    assert!(
        reveal_source(&db, &fresh.ticket_id, &entry, &source, 1, |_| {
            called = true;
            Ok(())
        })
        .is_err()
    );
    assert!(!called);
    write(&path, r#"{"mcpServers":{"a":{"command":"node"}}}"#);
    let expired = scan(&db, ResourceKind::Mcp).unwrap();
    let (entry, source) = native(&expired, "claude");
    pending()
        .lock()
        .unwrap()
        .get_mut(&expired.ticket_id)
        .unwrap()
        .created = Instant::now() - TTL - Duration::from_secs(1);
    assert!(source_detail(&db, &expired.ticket_id, &entry, &source, 1).is_err());
}
#[test]
#[serial_test::serial]
fn inline_and_legacy_sources_have_no_filesystem_action() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    std::env::set_var(
        "OPENCODE_CONFIG_CONTENT",
        r#"{"mcp":{"remote":{"type":"remote","url":"https://example.invalid/mcp"}}}"#,
    );
    let mcp = scan(&db, ResourceKind::Mcp).unwrap();
    let (entry, source) = native(&mcp, "opencode");
    let detail = source_detail(&db, &mcp.ticket_id, &entry, &source, 0).unwrap();
    assert_eq!(detail.location_kind, "inline");
    assert!(!detail.can_reveal);
    assert!(detail.logical_path.is_none());
    assert!(
        reveal_source(&db, &mcp.ticket_id, &entry, &source, 0, |_| panic!(
            "must not reveal inline config"
        ))
        .is_err()
    );
    db.save_prompt(
        "codex",
        &crate::prompt::Prompt {
            id: "old".into(),
            name: "old".into(),
            content: "personal".into(),
            description: None,
            enabled: true,
            created_at: None,
            updated_at: None,
        },
    )
    .unwrap();
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    let (entry, source) = native(&view, "codex");
    let detail = source_detail(&db, &view.ticket_id, &entry, &source, 0).unwrap();
    assert_eq!(detail.location_kind, "legacy");
    assert!(!detail.can_reveal);
}
#[test]
#[serial_test::serial]
fn mcp_comparison_returns_equality_and_missing_metadata_without_values() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    super::super::mcp_resource::save_draft(&db,server("left",json!({"type":"stdio","command":"PRIVATE-COMMAND","env":{"TOKEN":"PRIVATE-LEFT"},"args":["PRIVATE-ARG"]})),None).unwrap();
    super::super::mcp_resource::save_draft(&db,server("right",json!({"type":"stdio","command":"PRIVATE-COMMAND","env":{"TOKEN":"PRIVATE-RIGHT"},"headers":{}})),None).unwrap();
    let left = McpComparisonSubject::Resource {
        resource_id: "left".into(),
        expected_revision: 1,
    };
    let right = McpComparisonSubject::Resource {
        resource_id: "right".into(),
        expected_revision: 1,
    };
    let result = compare_mcp(&db, &left, &right, 0).unwrap();
    let state = |path: &[&str]| {
        result
            .fields
            .iter()
            .find(|field| field.path.iter().map(String::as_str).collect::<Vec<_>>() == path)
            .unwrap()
            .state
            .as_str()
    };
    assert_eq!(state(&["command"]), "equal");
    assert_eq!(state(&["env", "TOKEN"]), "different");
    assert_eq!(state(&["args"]), "missingRight");
    assert_eq!(state(&["headers"]), "missingLeft");
    let output = serde_json::to_string(&result).unwrap();
    for secret in [
        "PRIVATE-COMMAND",
        "PRIVATE-LEFT",
        "PRIVATE-RIGHT",
        "PRIVATE-ARG",
    ] {
        assert!(!output.contains(secret));
    }
    super::super::mcp_resource::save_draft(
        &db,
        server("left", json!({"type":"stdio","command":"new"})),
        Some(1),
    )
    .unwrap();
    assert!(compare_mcp(&db, &left, &right, 0).is_err());
}
#[test]
#[serial_test::serial]
fn native_comparison_revalidates_both_sources() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let paths = super::super::mcp_resource::native_paths().unwrap();
    write(
        &paths["claude"],
        r#"{"mcpServers":{"same":{"command":"node","env":{"TOKEN":"PRIVATE-A"}}}}"#,
    );
    write(
        &paths["gemini"],
        r#"{"mcpServers":{"same":{"command":"node","env":{"TOKEN":"PRIVATE-B"}}}}"#,
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    let (left, _) = native(&view, "claude");
    let (right, _) = native(&view, "gemini");
    let left = McpComparisonSubject::Native {
        ticket_id: view.ticket_id.clone(),
        entry_id: left,
    };
    let right = McpComparisonSubject::Native {
        ticket_id: view.ticket_id.clone(),
        entry_id: right,
    };
    assert!(compare_mcp(&db, &left, &right, 0)
        .unwrap()
        .fields
        .iter()
        .any(|field| field.path == ["env", "TOKEN"] && field.state == "different"));
    write(
        &paths["gemini"],
        r#"{"mcpServers":{"same":{"command":"other"}}}"#,
    );
    assert!(compare_mcp(&db, &left, &right, 0).is_err());
}
#[test]
#[serial_test::serial]
fn recipe_matching_distinguishes_exact_library_and_native_from_similar() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let exact = json!({"type":"http","url":"https://example.invalid/mcp"});
    let other = json!({"type":"http","url":"https://example.invalid/mcp","headers":{"Authorization":"PRIVATE-TOKEN"}});
    super::super::mcp_resource::save_draft(&db, server("exact", exact.clone()), None).unwrap();
    super::super::mcp_resource::save_draft(&db, server("different", other.clone()), None).unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["claude"].clone();
    write(
        &path,
        &json!({"mcpServers":{"native-exact":exact,"native-other":other}}).to_string(),
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    let recipes = [McpRecipe {
        recipe_id: "recipe".into(),
        name: Some("Friendly".into()),
        definition: exact,
    }];
    let result = match_recipes(&db, &recipes, Some(&view.ticket_id), 0).unwrap();
    assert_eq!(result[0].exact_resource_ids, ["exact"]);
    assert_eq!(result[0].similar_resource_ids, ["different"]);
    assert_eq!(result[0].native_entry_ids.len(), 1);
    assert_eq!(result[0].similar_native_entry_ids.len(), 1);
    assert!(!serde_json::to_string(&result)
        .unwrap()
        .contains("PRIVATE-TOKEN"));
    write(&path, "{}");
    assert!(match_recipes(&db, &recipes, Some(&view.ticket_id), 0).is_err());
    assert!(match_recipes(&db, &[], None, 0).is_err());
}
#[cfg(unix)]
#[test]
#[serial_test::serial]
fn symlink_source_details_preserve_logical_location_and_retargeting_invalidates_ticket() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["claude"].clone();
    let real = sandbox.home().join("real-mcp.json");
    let replacement = sandbox.home().join("replacement.json");
    write(&real, r#"{"mcpServers":{"demo":{"command":"node"}}}"#);
    write(
        &replacement,
        r#"{"mcpServers":{"demo":{"command":"node"}}}"#,
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &path).unwrap();
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    let (entry, source) = native(&view, "claude");
    let detail = source_detail(&db, &view.ticket_id, &entry, &source, 0).unwrap();
    assert!(detail.is_symlink);
    assert_eq!(detail.logical_path.as_deref(), path.to_str());
    assert_eq!(
        detail.canonical_path.as_deref(),
        real.canonicalize().unwrap().to_str()
    );
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&replacement, &path).unwrap();
    assert!(source_detail(&db, &view.ticket_id, &entry, &source, 0).is_err());
}

#[test]
#[serial_test::serial]
fn resource_and_native_comparisons_are_allowed_but_library_revision_changes_expire_native_views() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let spec = json!({"type":"stdio","command":"node"});
    super::super::mcp_resource::save_draft(&db, server("demo", spec.clone()), None).unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["claude"].clone();
    write(&path, &json!({"mcpServers":{"demo":spec}}).to_string());
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    let (entry, source) = native(&view, "claude");
    assert_eq!(
        source_detail(&db, &view.ticket_id, &entry, &source, 0)
            .unwrap()
            .matched_resource_ids,
        ["demo"]
    );
    let resource = McpComparisonSubject::Resource {
        resource_id: "demo".into(),
        expected_revision: 1,
    };
    let native = McpComparisonSubject::Native {
        ticket_id: view.ticket_id.clone(),
        entry_id: entry.clone(),
    };
    assert!(compare_mcp(&db, &resource, &native, 0)
        .unwrap()
        .fields
        .iter()
        .all(|field| field.state == "equal"));
    super::super::mcp_resource::save_draft(&db, server("other", json!({"command":"other"})), None)
        .unwrap();
    assert!(source_detail(&db, &view.ticket_id, &entry, &source, 0).is_err());
    assert!(compare_mcp(&db, &resource, &native, 0).is_err());
}

#[test]
#[serial_test::serial]
fn recipe_matching_also_revalidates_an_empty_native_scan() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    assert!(view.entries.is_empty());
    let recipe = [McpRecipe {
        recipe_id: "demo".into(),
        name: None,
        definition: json!({"command":"node"}),
    }];
    std::env::set_var(
        "OPENCODE_CONFIG_CONTENT",
        r#"{"mcp":{"new":{"type":"local","command":["node"]}}}"#,
    );
    assert!(
        match_recipes(&db, &recipe, Some(&view.ticket_id), 0).is_err(),
        "empty inventories must not bypass routing validation"
    );
}

#[test]
#[serial_test::serial]
fn layered_sources_reveal_only_the_selected_layer_and_recheck_all_layers() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let base = crate::opencode_config::global_config_paths()
        .into_iter()
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    let overlay = crate::test_sandbox::native(sandbox.home().join(".opencode/opencode.json"));
    write(
        &base,
        r#"{"mcp":{"layered":{"type":"local","command":["node","start"],"environment":{"FIRST":"PRIVATE-FIRST"}}}}"#,
    );
    write(
        &overlay,
        r#"{"mcp":{"layered":{"environment":{"SECOND":"PRIVATE-SECOND"}}}}"#,
    );
    let view = scan(&db, ResourceKind::Mcp).unwrap();
    let entry = view
        .entries
        .iter()
        .find(|entry| entry.name == "layered")
        .unwrap();
    assert_eq!(entry.sources.len(), 2);
    for reference in &entry.sources {
        let source = view
            .sources
            .iter()
            .find(|source| source.source_id == reference.source_id)
            .unwrap();
        let detail = source_detail(
            &db,
            &view.ticket_id,
            &entry.entry_id,
            source.source_ref.as_deref().unwrap(),
            0,
        )
        .unwrap();
        assert!(
            detail.logical_path.as_deref() == base.to_str()
                || detail.logical_path.as_deref() == overlay.to_str()
        );
        assert!(!serde_json::to_string(&detail).unwrap().contains("PRIVATE-"));
    }
    write(
        &overlay,
        r#"{"mcp":{"layered":{"environment":{"SECOND":"CHANGED"}}}}"#,
    );
    let reference = &entry.sources[0];
    let source = view
        .sources
        .iter()
        .find(|source| source.source_id == reference.source_id)
        .unwrap();
    assert!(source_detail(
        &db,
        &view.ticket_id,
        &entry.entry_id,
        source.source_ref.as_deref().unwrap(),
        0
    )
    .is_err());
}

#[test]
#[serial_test::serial]
fn recipe_matching_rejects_created_or_populated_sources_even_with_no_old_candidates() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = super::super::mcp_resource::native_paths().unwrap()["claude"].clone();
    let recipes = [McpRecipe {
        recipe_id: "demo".into(),
        name: None,
        definition: json!({"type":"stdio","command":"node"}),
    }];
    let missing = scan(&db, ResourceKind::Mcp).unwrap();
    assert!(missing.entries.is_empty());
    write(&path, r#"{"mcpServers":{"new":{"command":"node"}}}"#);
    assert!(
        match_recipes(&db, &recipes, Some(&missing.ticket_id), 0).is_err(),
        "missing sources created after scanning must invalidate a negative match"
    );
    write(&path, "{}");
    let empty = scan(&db, ResourceKind::Mcp).unwrap();
    assert!(empty.entries.is_empty());
    write(&path, r#"{"mcpServers":{"new":{"command":"node"}}}"#);
    assert!(
        match_recipes(&db, &recipes, Some(&empty.ticket_id), 0).is_err(),
        "existing empty source files must also be fingerprinted"
    );
}

#[test]
#[serial_test::serial]
fn recipe_matching_cannot_treat_unreadable_or_unsupported_sources_as_no_configuration() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let paths = super::super::mcp_resource::native_paths().unwrap();
    let recipes = [McpRecipe {
        recipe_id: "demo".into(),
        name: None,
        definition: json!({"type":"stdio","command":"node"}),
    }];
    write(&paths["claude"], "broken-json");
    let unreadable = scan(&db, ResourceKind::Mcp).unwrap();
    assert!(unreadable
        .sources
        .iter()
        .any(|source| source.state == "unreadable"));
    assert!(
        match_recipes(&db, &recipes, Some(&unreadable.ticket_id), 0).is_err(),
        "unreadable sources cannot establish an absent native configuration"
    );
    fs::remove_file(&paths["claude"]).unwrap();
    write(
        &paths["gemini"],
        r#"{"mcpServers":{"automatic":{"url":"https://example.invalid/mcp"}}}"#,
    );
    let unsupported = scan(&db, ResourceKind::Mcp).unwrap();
    assert!(unsupported
        .sources
        .iter()
        .any(|source| source.state == "unsupported"));
    assert!(match_recipes(&db, &recipes, Some(&unsupported.ticket_id), 0).is_err());
    assert_eq!(
        match_recipes(&db, &recipes, None, 0).unwrap()[0]
            .exact_resource_ids
            .len(),
        0,
        "library-only matching remains available"
    );
}
