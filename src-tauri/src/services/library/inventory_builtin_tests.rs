use super::*;

fn instruction_recipe(content: &str) -> BuiltinRecipe {
    BuiltinRecipe {
        recipe_id: "general-coding".into(),
        content: Some(content.into()),
        repo_owner: None,
        repo_name: None,
        directory: None,
    }
}
#[test]
#[serial_test::serial]
fn builtin_instructions_match_content_not_title_and_validate_native_ticket() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let set = super::super::catalog::import_instruction(&db, "Different title", "Use clear code\n")
        .unwrap();
    super::super::catalog::import_instruction(&db, "general-coding", "Different body").unwrap();
    let path = super::super::coordinator::native_instruction_paths().unwrap()["claude"].clone();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "Use clear code\n").unwrap();
    let view = scan(&db, ResourceKind::Instructions).unwrap();
    let result = match_builtin_recipes(
        &db,
        ResourceKind::Instructions,
        &[instruction_recipe("Use clear code\n")],
        Some(&view.ticket_id),
        0,
    )
    .unwrap();
    assert_eq!(result[0].exact_resource_ids, [set.id]);
    assert_eq!(result[0].native_entry_ids.len(), 1);
    std::fs::write(&path, "Use different code\n").unwrap();
    assert!(match_builtin_recipes(
        &db,
        ResourceKind::Instructions,
        &[instruction_recipe("Use clear code\n")],
        Some(&view.ticket_id),
        0
    )
    .is_err());
}
