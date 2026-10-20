use super::*;
use std::fs;

fn recipe() -> BuiltinRecipe {
    BuiltinRecipe {
        recipe_id: "frontend-design".into(),
        content: None,
        repo_owner: Some("anthropics".into()),
        repo_name: Some("skills".into()),
        directory: Some("skills/frontend-design".into()),
    }
}
fn fixture(db: &Database, root: &std::path::Path, id: &str, pinned: bool) -> InstalledSkill {
    let source = root.join(id);
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        format!("---\nname: {id}\n---\nContent"),
    )
    .unwrap();
    let reference = if pinned {
        "b".repeat(40)
    } else {
        "main".into()
    };
    let row = InstalledSkill {
        id: id.into(),
        name: "Different title".into(),
        description: None,
        directory: id.into(),
        repo_owner: Some("anthropics".into()),
        repo_name: Some("skills".into()),
        repo_branch: Some("main".into()),
        readme_url: Some(format!(
            "https://github.com/anthropics/skills/blob/{reference}/skills/frontend-design/SKILL.md"
        )),
        apps: Default::default(),
        installed_at: 0,
        updated_at: 0,
        content_hash: None,
    };
    super::super::skill_resource::save_draft(db, row, &source, None).unwrap()
}
#[test]
#[serial_test::serial]
fn builtin_skills_require_fixed_verified_repository_provenance_and_artifact() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let exact = fixture(&db, sandbox.home(), "different-local-directory", true);
    fixture(&db, sandbox.home(), "unfixed-ref", false);
    let result = match_builtin_recipes(&db, ResourceKind::Skills, &[recipe()], None, 0).unwrap();
    assert_eq!(
        result[0].exact_resource_ids.as_slice(),
        std::slice::from_ref(&exact.id)
    );
    assert_eq!(result[0].similar_resource_ids, ["unfixed-ref"]);
    let artifact =
        super::super::skill_artifact::path(exact.content_hash.as_deref().unwrap()).unwrap();
    fs::write(artifact.join("SKILL.md"), "tampered").unwrap();
    let result = match_builtin_recipes(&db, ResourceKind::Skills, &[recipe()], None, 0).unwrap();
    assert!(result[0].exact_resource_ids.is_empty());
    assert!(result[0].similar_resource_ids.contains(&exact.id));
}
#[test]
#[serial_test::serial]
fn builtin_skills_native_exact_is_linked_to_verified_library_source_not_just_a_name() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let exact = fixture(&db, sandbox.home(), "imported", true);
    let native =
        super::super::skill_resource::native_paths().unwrap()["claude"].join("native-copy");
    fs::create_dir_all(&native).unwrap();
    let artifact =
        super::super::skill_artifact::path(exact.content_hash.as_deref().unwrap()).unwrap();
    fs::copy(artifact.join("SKILL.md"), native.join("SKILL.md")).unwrap();
    let view = scan(&db, ResourceKind::Skills).unwrap();
    let result = match_builtin_recipes(
        &db,
        ResourceKind::Skills,
        &[recipe()],
        Some(&view.ticket_id),
        0,
    )
    .unwrap();
    assert_eq!(result[0].native_entry_ids.len(), 1);
    fs::write(native.join("SKILL.md"), "changed source").unwrap();
    assert!(match_builtin_recipes(
        &db,
        ResourceKind::Skills,
        &[recipe()],
        Some(&view.ticket_id),
        0
    )
    .is_err());
}
#[test]
#[serial_test::serial]
fn builtin_empty_skill_inventory_and_legacy_instructions_recheck_new_sources() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let view = scan(&db, ResourceKind::Skills).unwrap();
    assert!(view.entries.is_empty());
    let native = super::super::skill_resource::native_paths().unwrap()["codex"].join("new");
    fs::create_dir_all(&native).unwrap();
    fs::write(native.join("SKILL.md"), "New skill").unwrap();
    assert!(match_builtin_recipes(
        &db,
        ResourceKind::Skills,
        &[recipe()],
        Some(&view.ticket_id),
        0
    )
    .is_err());
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    db.save_prompt(
        "codex",
        &crate::prompt::Prompt {
            id: "new".into(),
            name: "New".into(),
            content: "personal".into(),
            description: None,
            enabled: false,
            created_at: None,
            updated_at: None,
        },
    )
    .unwrap();
    let instruction = BuiltinRecipe {
        recipe_id: "rules".into(),
        content: Some("personal".into()),
        repo_owner: None,
        repo_name: None,
        directory: None,
    };
    assert!(match_builtin_recipes(
        &db,
        ResourceKind::Instructions,
        &[instruction],
        Some(&view.ticket_id),
        0
    )
    .is_err());
}
#[test]
#[serial_test::serial]
fn builtin_recipe_matching_rejects_bad_kind_payload_and_epoch() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    assert!(match_builtin_recipes(&db, ResourceKind::Mcp, &[recipe()], None, 0).is_err());
    assert!(
        match_builtin_recipes(&db, ResourceKind::Skills, &[recipe(), recipe()], None, 0).is_err()
    );
    let mut invalid = recipe();
    invalid.directory = Some("../outside".into());
    assert!(match_builtin_recipes(&db, ResourceKind::Skills, &[invalid], None, 0).is_err());
    assert!(match_builtin_recipes(&db, ResourceKind::Skills, &[recipe()], None, 2).is_err());
}

#[test]
#[serial_test::serial]
fn builtin_skill_matching_does_not_confuse_other_directories_in_the_same_repository() {
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let mut row = fixture(&db, sandbox.home(), "other-directory", true);
    row.readme_url = Some(format!(
        "https://github.com/anthropics/skills/blob/{}/skills/some-other-skill/SKILL.md",
        "b".repeat(40)
    ));
    db.save_skill(&row).unwrap();
    let before = db.library_snapshot().unwrap();
    let mut after = before.clone();
    after.definitions.skills.get_mut(&row.id).unwrap().hash =
        super::super::skill_resource::spec_hash(&row).unwrap();
    db.library_commit(&before, &after).unwrap();
    let result = match_builtin_recipes(&db, ResourceKind::Skills, &[recipe()], None, 0).unwrap();
    assert!(result[0].exact_resource_ids.is_empty());
    assert!(
        result[0].similar_resource_ids.is_empty(),
        "sharing the official repository does not identify the same skill"
    );
}
