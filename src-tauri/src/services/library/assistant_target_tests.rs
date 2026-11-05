use super::*;
use crate::app_config::InstalledSkill;
use std::fs;

fn local(name: &str, body: &[u8]) -> PathBuf {
    let dir = skills_dir().join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), body).unwrap();
    dir
}
fn body(name: &str) -> String {
    format!("---\nname: {name}\ndescription: A fixture\n---\nHello")
}
fn managed(db: &Database, owner: &str, name: &str, source_dir: &str) -> InstalledSkill {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("SKILL.md"), body(name)).unwrap();
    let row = InstalledSkill {
        id: format!("{owner}/skills:{source_dir}"),
        name: "Renamed title".into(),
        description: Some("Display description".into()),
        directory: name.into(),
        repo_owner: Some(owner.into()),
        repo_name: Some("skills".into()),
        repo_branch: Some("main".into()),
        readme_url: Some(format!(
            "https://github.com/{owner}/skills/blob/{}/{source_dir}/SKILL.md",
            "a".repeat(40)
        )),
        apps: Default::default(),
        installed_at: 0,
        updated_at: 0,
        content_hash: None,
    };
    let row = skill_resource::save_draft(db, row, temp.path(), None).unwrap();
    let op = skill_resource::preview(db, &row.id, &[ID.into()]).unwrap();
    assert_eq!(
        crate::services::library::apply_operation(db, &op.id)
            .unwrap()
            .status,
        "applied"
    );
    row
}

#[test]
#[serial_test::serial]
fn skill_modes_default_to_auto_and_round_trip() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let dir = local("sample", body("sample").as_bytes());
    assert!(skill_modes().unwrap().is_empty());
    assert_eq!(
        list_assistant_skills(&db, &reviewed_names()).unwrap()[0].mode,
        SkillMode::Auto
    );
    set_skill_mode("sample", SkillMode::Manual).unwrap();
    set_skill_mode("other", SkillMode::Auto).unwrap();
    assert_eq!(skill_modes().unwrap()["sample"], SkillMode::Manual);
    assert_eq!(
        fs::read_to_string(dir.join("SKILL.md")).unwrap(),
        body("sample")
    );
    assert_eq!(mcp_config_path(), assistant_dir().join("mcp.json"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(assistant_dir().join("skill-modes.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
#[serial_test::serial]
fn list_reports_a_broken_skill_instead_of_dropping_it() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    local("no-header", b"name: missing header");
    local("no-description", b"---\nname: no-description\n---\nBody");
    local("huge", &vec![b'a'; 1024 * 1024 + 1]);
    local("binary", &[0, 255, 1]);
    let rows = list_assistant_skills(&db, &reviewed_names()).unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|row| row.broken.is_some()));
    assert!(rows.iter().all(|row| !row.reviewed && row.origin.is_none()));
}

#[test]
#[serial_test::serial]
fn corrupt_global_state_is_an_error_and_never_overwritten() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    local("sample", body("sample").as_bytes());
    for file in [
        "skill-modes.json",
        "agent-skills.json",
        "agent-skill-origins.json",
    ] {
        let path = assistant_dir().join(file);
        fs::write(&path, "{").unwrap();
        assert!(
            list_assistant_skills(&db, &reviewed_names()).is_err(),
            "{file}"
        );
        assert!(set_skill_mode("sample", SkillMode::Auto).is_err(), "{file}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{");
        fs::remove_file(path).unwrap();
    }
}

#[test]
#[serial_test::serial]
fn has_scripts_detects_script_files_and_scripts_folder() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    for (name, file) in [
        ("py", "run.py"),
        ("sh", "nested/run.sh"),
        ("js", "run.js"),
        ("ts", "run.ts"),
        ("folder", "scripts/"),
    ] {
        let dir = local(name, body(name).as_bytes());
        let path = dir.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if file.ends_with('/') {
            fs::create_dir_all(path).unwrap();
        } else {
            fs::write(path, "fixture").unwrap();
        }
    }
    local("plain", body("plain").as_bytes());
    let rows = list_assistant_skills(&db, &reviewed_names()).unwrap();
    assert!(rows.iter().all(|r| r.has_scripts == (r.name != "plain")));
}

#[test]
#[serial_test::serial]
fn reviewed_requires_actual_source_and_fixed_artifact_not_display_name() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let row = managed(
        &db,
        "anthropics",
        "frontend-design",
        "skills/frontend-design",
    );
    let snapshot = assistant_skill_snapshot(&db, &reviewed_names()).unwrap();
    assert_eq!(snapshot.skills[0].origin, Some(SkillOrigin::Reviewed));
    assert_eq!(snapshot.skills[0].name, "frontend-design");
    let proof = snapshot.proof("frontend-design").unwrap();
    assert!(with_revalidated_skill(&db, proof, SkillAccess::AutoUse, |_| Ok(())).is_ok());
    set_skill_mode("frontend-design", SkillMode::Manual).unwrap();
    assert!(with_revalidated_skill(&db, proof, SkillAccess::AutoUse, |_| Ok(())).is_err());
    let fresh = assistant_skill_snapshot(&db, &reviewed_names()).unwrap();
    assert!(with_revalidated_skill(
        &db,
        fresh.proof("frontend-design").unwrap(),
        SkillAccess::UserSelected,
        |_| Ok(())
    )
    .is_ok());
    let op = skill_resource::preview(&db, &row.id, &[]).unwrap();
    crate::services::library::apply_operation(&db, &op.id).unwrap();
    assert!(with_revalidated_skill(
        &db,
        fresh.proof("frontend-design").unwrap(),
        SkillAccess::UserSelected,
        |_| Ok(())
    )
    .is_err());
    let op = skill_resource::preview(&db, &row.id, &[ID.into()]).unwrap();
    crate::services::library::apply_operation(&db, &op.id).unwrap();
    assert_eq!(
        skill_modes()
            .unwrap()
            .get("frontend-design")
            .copied()
            .unwrap_or(SkillMode::Auto),
        SkillMode::Auto
    );
    assert!(!skill_modes().unwrap().contains_key("Renamed title"));
}

#[test]
#[serial_test::serial]
fn foreign_same_name_and_names_array_do_not_grant_reviewed_or_created() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    managed(
        &db,
        "third-party",
        "frontend-design",
        "skills/frontend-design",
    );
    fs::write(
        assistant_dir().join("agent-skills.json"),
        r#"["frontend-design"]"#,
    )
    .unwrap();
    let rows = list_assistant_skills(&db, &reviewed_names()).unwrap();
    assert!(!rows[0].reviewed);
    assert_eq!(rows[0].origin, Some(SkillOrigin::Unreviewed));
}

#[test]
#[serial_test::serial]
fn local_snapshot_rejects_changed_tree_and_duplicate_names() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let dir = local("one", body("same").as_bytes());
    let snapshot = assistant_skill_snapshot(&db, &reviewed_names()).unwrap();
    assert_eq!(snapshot.skills[0].origin, Some(SkillOrigin::Local));
    fs::write(dir.join("reference.txt"), "changed").unwrap();
    assert!(with_revalidated_skill(
        &db,
        snapshot.proof("same").unwrap(),
        SkillAccess::AutoUse,
        |_| Ok(())
    )
    .is_err());
    local("two", body("same").as_bytes());
    let snapshot = assistant_skill_snapshot(&db, &reviewed_names()).unwrap();
    assert!(snapshot.proof("same").is_none());
    assert!(snapshot.skills.iter().all(|s| s.broken.is_some()));
}

#[test]
#[serial_test::serial]
fn modified_managed_target_never_becomes_local_or_loadable() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    managed(
        &db,
        "anthropics",
        "frontend-design",
        "skills/frontend-design",
    );
    fs::write(
        skills_dir().join("frontend-design/SKILL.md"),
        body("foreign"),
    )
    .unwrap();
    let snapshot = assistant_skill_snapshot(&db, &reviewed_names()).unwrap();
    assert!(snapshot.skills[0].broken.is_some());
    assert!(snapshot.proof("foreign").is_none());
    assert!(!snapshot.skills[0].reviewed);
}

#[test]
#[serial_test::serial]
fn official_frontmatter_identity_is_separate_from_curated_exact_directory() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    managed(
        &db,
        "vercel-labs",
        "find-skills",
        "arbitrary/actual-directory",
    );
    assert!(list_assistant_skills(&db, &reviewed_names()).unwrap()[0].reviewed);
    managed(&db, "anthropics", "webapp-testing", "different-directory");
    let rows = list_assistant_skills(&db, &reviewed_names()).unwrap();
    assert!(
        !rows
            .iter()
            .find(|s| s.name == "webapp-testing")
            .unwrap()
            .reviewed
    );
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn assistant_state_and_unmanaged_skills_reject_links() {
    use std::os::unix::fs::symlink;
    let sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let outside = sandbox.home().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("SKILL.md"), body("outside")).unwrap();
    fs::create_dir_all(skills_dir()).unwrap();
    symlink(&outside, skills_dir().join("escaped")).unwrap();
    assert!(list_assistant_skills(&db, &reviewed_names()).unwrap()[0]
        .broken
        .is_some());
    fs::write(outside.join("modes.json"), "{}").unwrap();
    symlink(
        outside.join("modes.json"),
        assistant_dir().join("skill-modes.json"),
    )
    .unwrap();
    assert!(set_skill_mode("outside", SkillMode::Manual).is_err());
    assert_eq!(
        fs::read_to_string(outside.join("modes.json")).unwrap(),
        "{}"
    );
}

#[test]
#[serial_test::serial]
fn created_origin_needs_current_receipt_and_survives_unrelated_epoch_changes() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let row = managed(
        &db,
        "anthropics",
        "frontend-design",
        "skills/frontend-design",
    );
    let before = db.library_snapshot().unwrap();
    let receipt = before
        .machine
        .receipts
        .values()
        .find(|r| r.resource_id == row.id)
        .unwrap()
        .clone();
    let marker = AgentSkillOrigin {
        version: 1,
        creation_id: uuid::Uuid::new_v4().to_string(),
        revision: before.definitions.skills[&row.id].clone(),
        artifact_hash: row.content_hash.clone().unwrap(),
        receipt,
    };
    fs::write(
        assistant_dir().join("agent-skills.json"),
        r#"["frontend-design"]"#,
    )
    .unwrap();
    fs::write(
        assistant_dir().join("agent-skill-origins.json"),
        serde_json::to_vec(&BTreeMap::from([(row.id.clone(), marker)])).unwrap(),
    )
    .unwrap();
    assert_eq!(
        list_assistant_skills(&db, &reviewed_names()).unwrap()[0].origin,
        Some(SkillOrigin::AssistantCreated)
    );
    assert!(!list_assistant_skills(&db, &reviewed_names()).unwrap()[0].reviewed);
    let mut next = before.clone();
    next.machine.epoch += 1;
    db.library_commit(&before, &next).unwrap();
    assert_eq!(
        origin_for_resource(&db, &row).unwrap(),
        SkillOrigin::AssistantCreated
    );
    fs::write(
        skills_dir().join("frontend-design/SKILL.md"),
        body("foreign"),
    )
    .unwrap();
    assert_ne!(
        origin_for_resource(&db, &row).unwrap(),
        SkillOrigin::AssistantCreated
    );
}

#[test]
#[serial_test::serial]
fn invalid_modes_block_distribution_before_assignments_or_files_change() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let row = managed(&db, "fixture", "sample", "nested/sample");
    let op = skill_resource::preview(&db, &row.id, &[]).unwrap();
    let before = db.library_snapshot().unwrap();
    let path = assistant_dir().join("skill-modes.json");
    for text in [
        r#"{"sample":"manual","sample":"auto"}"#,
        "null",
        r#"{"sample":"off"}"#,
    ] {
        fs::write(&path, text).unwrap();
        assert!(crate::services::library::apply_operation(&db, &op.id).is_err());
        assert_eq!(
            db.library_snapshot().unwrap().machine.assignments,
            before.machine.assignments
        );
        assert!(skills_dir().join("sample/SKILL.md").is_file());
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
    }
}

#[test]
#[serial_test::serial]
fn replacing_a_local_directory_with_identical_bytes_revokes_its_proof() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let path = local("same", body("same").as_bytes());
    let snapshot = assistant_skill_snapshot(&db, &reviewed_names()).unwrap();
    fs::rename(&path, skills_dir().join(".old-same")).unwrap();
    local("same", body("same").as_bytes());
    assert!(with_revalidated_skill(
        &db,
        snapshot.proof("same").unwrap(),
        SkillAccess::AutoUse,
        |_| Ok(())
    )
    .is_err());
}

#[test]
#[serial_test::serial]
fn run_proof_same_identity_ignores_only_epoch() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    local("same", body("same").as_bytes());
    let snap = assistant_skill_snapshot(&db, &reviewed_names()).unwrap();
    let proof = snap.proof("same").unwrap();
    let mut epoch = proof.clone();
    epoch.epoch += 1;
    assert!(proof.same_identity(&epoch));
    let mut variants = vec![];
    macro_rules! changed {
        ($field:ident,$value:expr) => {{
            let mut p = proof.clone();
            p.$field = $value;
            variants.push(p);
        }};
    }
    changed!(name, "other".into());
    changed!(lexical_root, PathBuf::from("/other"));
    changed!(canonical_root, PathBuf::from("/other"));
    changed!(root_identity, "other".into());
    changed!(lexical_target, PathBuf::from("/other"));
    changed!(canonical_dir, PathBuf::from("/other"));
    changed!(directory_identity, "other".into());
    changed!(mode, SkillMode::Manual);
    changed!(
        identity,
        Identity::Local {
            tree_hash: "other".into()
        }
    );
    for changed in variants {
        assert!(!proof.same_identity(&changed));
    }
}

#[test]
#[serial_test::serial]
fn skill_tools_pending_intent_never_becomes_local_auto() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    local("pending-copy", body("pending-copy").as_bytes());
    fs::write(
        assistant_dir().join("skill-transaction-v1.json"),
        b"{broken intent",
    )
    .unwrap();
    assert!(
        assistant_skill_snapshot(&db, &reviewed_names()).is_err(),
        "unresolved intent cannot authorize a copied target as Local/Auto"
    );
}

#[test]
#[serial_test::serial]
fn skill_tools_created_origin_survives_off_without_grant() {
    let _sandbox = crate::test_sandbox::sandbox();
    let db = Database::memory().unwrap();
    let row = managed(
        &db,
        "anthropics",
        "frontend-design",
        "skills/frontend-design",
    );
    let before = db.library_snapshot().unwrap();
    let receipt = before
        .machine
        .receipts
        .values()
        .find(|r| r.resource_id == row.id)
        .unwrap()
        .clone();
    let marker = AgentSkillOrigin {
        version: 1,
        creation_id: uuid::Uuid::new_v4().to_string(),
        revision: before.definitions.skills[&row.id].clone(),
        artifact_hash: row.content_hash.clone().unwrap(),
        receipt,
    };
    fs::write(
        assistant_dir().join("agent-skills.json"),
        r#"["frontend-design"]"#,
    )
    .unwrap();
    fs::write(
        assistant_dir().join("agent-skill-origins.json"),
        serde_json::to_vec(&BTreeMap::from([(row.id.clone(), marker)])).unwrap(),
    )
    .unwrap();
    assert_eq!(
        list_assistant_skills(&db, &reviewed_names()).unwrap()[0].origin,
        Some(SkillOrigin::AssistantCreated)
    );
    assert!(!list_assistant_skills(&db, &reviewed_names()).unwrap()[0].reviewed);
    let mut next = before.clone();
    next.machine.epoch += 1;
    db.library_commit(&before, &next).unwrap();
    assert_eq!(
        origin_for_resource(&db, &row).unwrap(),
        SkillOrigin::AssistantCreated
    );
    let off = skill_resource::preview(&db, &row.id, &[]).unwrap();
    assert_eq!(
        crate::services::library::apply_operation(&db, &off.id)
            .unwrap()
            .status,
        "applied"
    );
    assert_eq!(
        origin_for_resource(&db, &row).unwrap(),
        SkillOrigin::AssistantCreated
    );
}
