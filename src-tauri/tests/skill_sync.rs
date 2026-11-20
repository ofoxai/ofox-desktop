use std::fs;

use cc_switch_lib::{
    migrate_skills_to_ssot, AppType, ImportSkillSelection, InstalledSkill, SkillApps, SkillService,
};

#[path = "support.rs"]
mod support;
use support::{create_test_state, sandbox};

fn write_skill(dir: &std::path::Path, name: &str) {
    fs::create_dir_all(dir).expect("create skill dir");
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Test skill\n---\n"),
    )
    .expect("write SKILL.md");
}

#[cfg(unix)]
fn directory_link(src: &std::path::Path, dest: &std::path::Path) {
    std::os::unix::fs::symlink(src, dest).expect("create symlink");
}

#[cfg(windows)]
fn directory_link(src: &std::path::Path, dest: &std::path::Path) {
    use std::os::windows::process::CommandExt;

    match std::os::windows::fs::symlink_dir(src, dest) {
        Ok(()) => {}
        // Non-administrator accounts may lack SeCreateSymbolicLinkPrivilege.
        // A junction still exercises preservation of a real directory link.
        Err(error) if error.raw_os_error() == Some(1314) => {
            let output = std::process::Command::new("cmd.exe")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(dest)
                .arg(src)
                .creation_flags(0x0800_0000)
                .output()
                .expect("create junction");
            assert!(output.status.success(), "create junction: {output:?}");
        }
        Err(error) => panic!("create symlink: {error}"),
    }
}

fn assert_directory_link(src: &std::path::Path, dest: &std::path::Path) {
    let metadata = fs::symlink_metadata(dest).expect("directory link metadata");
    #[cfg(unix)]
    assert!(metadata.file_type().is_symlink(), "a real directory link");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        assert_ne!(
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0,
            0
        );
    }
    assert_eq!(
        fs::canonicalize(dest).expect("directory link target"),
        fs::canonicalize(src).expect("source directory")
    );
}

#[test]
#[serial_test::serial]
fn import_from_apps_respects_explicit_app_selection() {
    let sandbox = sandbox();
    let home = sandbox.home();

    write_skill(
        &home.join(".claude").join("skills").join("shared-skill"),
        "Shared",
    );
    write_skill(
        &home
            .join(".config")
            .join("opencode")
            .join("skills")
            .join("shared-skill"),
        "Shared",
    );

    let state = create_test_state().expect("create test state");

    let imported = SkillService::import_from_apps(
        &state.db,
        vec![ImportSkillSelection {
            directory: "shared-skill".to_string(),
            apps: SkillApps {
                opencode: true,
                ..Default::default()
            },
        }],
    )
    .expect("import skills");

    assert_eq!(imported.len(), 1, "expected exactly one imported skill");
    let skill = imported.first().expect("imported skill");
    assert!(
        !skill.apps.opencode,
        "source selection does not assign a native target"
    );
    assert!(home
        .join(".config/opencode/skills/shared-skill/SKILL.md")
        .is_file());
    assert!(state
        .db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());
    assert!(
        !skill.apps.claude && !skill.apps.codex && !skill.apps.gemini,
        "import should no longer infer apps from every matching source path"
    );
}

#[test]
#[serial_test::serial]
fn sync_to_app_preserves_unowned_disabled_and_orphaned_links() {
    let sandbox = sandbox();
    let home = sandbox.home();

    let ssot_dir = home.join(".ofox-desktop").join("skills");
    let disabled_skill = ssot_dir.join("disabled-skill");
    let orphan_skill = ssot_dir.join("orphan-skill");
    write_skill(&disabled_skill, "Disabled");
    write_skill(&orphan_skill, "Orphan");

    let opencode_skills_dir = home.join(".config").join("opencode").join("skills");
    fs::create_dir_all(&opencode_skills_dir).expect("create opencode skills dir");
    let links = [
        (&disabled_skill, opencode_skills_dir.join("disabled-skill")),
        (&orphan_skill, opencode_skills_dir.join("orphan-skill")),
    ];
    let originals: Vec<_> = links
        .iter()
        .map(|(source, link)| {
            directory_link(source, link);
            assert_directory_link(source, link);
            fs::read(source.join("SKILL.md")).expect("original skill bytes")
        })
        .collect();

    let state = create_test_state().expect("create test state");
    state
        .db
        .save_skill(&InstalledSkill {
            id: "local:disabled-skill".to_string(),
            name: "Disabled".to_string(),
            description: None,
            directory: "disabled-skill".to_string(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps::default(),
            installed_at: 0,
            content_hash: None,
            updated_at: 0,
        })
        .expect("save disabled skill");

    SkillService::sync_to_app(&state.db, &AppType::OpenCode).expect("reconcile skills");

    assert!(
        opencode_skills_dir.join("disabled-skill").exists(),
        "old enabled flags are not ownership proof"
    );
    assert!(
        opencode_skills_dir.join("orphan-skill").exists(),
        "unowned orphan links remain user files"
    );
    for ((source, link), original) in links.iter().zip(originals) {
        assert_directory_link(source, link);
        assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), original);
        assert_eq!(fs::read(link.join("SKILL.md")).unwrap(), original);
    }
}

#[test]
#[serial_test::serial]
fn removing_a_draft_backs_up_content_and_keeps_legacy_sources() {
    let sandbox = sandbox();
    let home = sandbox.home();

    let ssot_skill_dir = home
        .join(".ofox-desktop")
        .join("skills")
        .join("backup-skill");
    write_skill(&ssot_skill_dir, "Backup Skill");
    fs::write(ssot_skill_dir.join("prompt.md"), "backup me").expect("write prompt.md");

    let state = create_test_state().expect("create test state");
    state
        .db
        .save_skill(&InstalledSkill {
            id: "local:backup-skill".to_string(),
            name: "Backup Skill".to_string(),
            description: Some("Back me up before uninstall".to_string()),
            directory: "backup-skill".to_string(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps {
                claude: true,
                ..Default::default()
            },
            installed_at: 123,
            content_hash: None,
            updated_at: 0,
        })
        .expect("save skill");

    let result = SkillService::uninstall(&state.db, "local:backup-skill").expect("uninstall skill");
    let backup_path = result.backup_path.expect("backup path should be returned");
    let backup_dir = std::path::PathBuf::from(&backup_path);

    assert!(backup_dir.exists(), "backup directory should exist");
    assert!(
        backup_dir.join("skill").join("SKILL.md").exists(),
        "backup should include SKILL.md"
    );
    assert_eq!(
        fs::read_to_string(backup_dir.join("skill").join("prompt.md"))
            .expect("read backed up prompt"),
        "backup me"
    );

    let metadata: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(backup_dir.join("meta.json")).expect("read backup metadata"),
    )
    .expect("parse backup metadata");
    assert_eq!(metadata["skill"]["directory"], "backup-skill");
    assert_eq!(metadata["skill"]["name"], "Backup Skill");

    assert!(
        ssot_skill_dir.exists(),
        "draft deletion preserves the legacy source"
    );
    assert!(
        state
            .db
            .get_installed_skill("local:backup-skill")
            .expect("query skill")
            .is_none(),
        "database row should be deleted after uninstall"
    );
}

#[test]
#[serial_test::serial]
fn restore_skill_backup_creates_an_unassigned_immutable_draft() {
    let sandbox = sandbox();
    let home = sandbox.home();

    let ssot_skill_dir = home
        .join(".ofox-desktop")
        .join("skills")
        .join("restore-skill");
    write_skill(&ssot_skill_dir, "Restore Skill");
    fs::write(ssot_skill_dir.join("prompt.md"), "restore me").expect("write prompt.md");

    let state = create_test_state().expect("create test state");
    state
        .db
        .save_skill(&InstalledSkill {
            id: "local:restore-skill".to_string(),
            name: "Restore Skill".to_string(),
            description: Some("Bring the files back".to_string()),
            directory: "restore-skill".to_string(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps {
                claude: true,
                ..Default::default()
            },
            installed_at: 456,
            content_hash: None,
            updated_at: 0,
        })
        .expect("save skill");

    let uninstall =
        SkillService::uninstall(&state.db, "local:restore-skill").expect("uninstall skill");
    let backup_id = std::path::Path::new(
        &uninstall
            .backup_path
            .expect("backup path should be returned on uninstall"),
    )
    .file_name()
    .expect("backup dir name")
    .to_string_lossy()
    .to_string();

    let restored = SkillService::restore_from_backup(&state.db, &backup_id, &AppType::Claude)
        .expect("restore from backup");

    assert_eq!(restored.directory, "restore-skill");
    assert!(
        !restored.apps.claude,
        "restore does not assign a native target"
    );
    assert!(
        !restored.apps.codex && !restored.apps.gemini && !restored.apps.opencode,
        "restore never infers native ownership"
    );
    assert!(
        home.join(".ofox-desktop")
            .join("skills")
            .join("restore-skill")
            .join("prompt.md")
            .exists(),
        "restored skill should exist in SSOT"
    );
    assert!(
        !home
            .join(".claude")
            .join("skills")
            .join("restore-skill")
            .join("prompt.md")
            .exists(),
        "restore never writes the native app root"
    );
    assert!(
        state
            .db
            .get_installed_skill("local:restore-skill")
            .expect("query restored skill")
            .is_some(),
        "restored skill should be written back to the database"
    );
}

#[test]
#[serial_test::serial]
fn delete_skill_backup_removes_backup_directory() {
    let sandbox = sandbox();
    let home = sandbox.home();

    let ssot_skill_dir = home
        .join(".ofox-desktop")
        .join("skills")
        .join("delete-backup-skill");
    write_skill(&ssot_skill_dir, "Delete Backup Skill");

    let state = create_test_state().expect("create test state");
    state
        .db
        .save_skill(&InstalledSkill {
            id: "local:delete-backup-skill".to_string(),
            name: "Delete Backup Skill".to_string(),
            description: Some("Remove my backup".to_string()),
            directory: "delete-backup-skill".to_string(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps {
                claude: true,
                ..Default::default()
            },
            installed_at: 789,
            content_hash: None,
            updated_at: 0,
        })
        .expect("save skill");

    let uninstall =
        SkillService::uninstall(&state.db, "local:delete-backup-skill").expect("uninstall skill");
    let backup_path = uninstall
        .backup_path
        .expect("backup path should be returned on uninstall");
    let backup_id = std::path::Path::new(&backup_path)
        .file_name()
        .expect("backup dir name")
        .to_string_lossy()
        .to_string();

    assert!(
        std::path::Path::new(&backup_path).exists(),
        "backup directory should exist before deletion"
    );

    SkillService::delete_backup(&backup_id).expect("delete backup");

    assert!(
        !std::path::Path::new(&backup_path).exists(),
        "backup directory should be removed"
    );
    assert!(
        SkillService::list_backups()
            .expect("list backups")
            .into_iter()
            .all(|entry| entry.backup_id != backup_id),
        "deleted backup should no longer appear in backup list"
    );
}

#[test]
#[serial_test::serial]
fn startup_migration_does_not_claim_native_directory_ownership() {
    let sandbox = sandbox();
    let home = sandbox.home();

    write_skill(
        &home.join(".claude").join("skills").join("demo-skill"),
        "Demo",
    );
    write_skill(
        &home
            .join(".config")
            .join("opencode")
            .join("skills")
            .join("demo-skill"),
        "Demo",
    );

    let state = create_test_state().expect("create test state");
    state
        .db
        .set_setting(
            "skills_ssot_migration_snapshot",
            r#"[{"directory":"demo-skill","app_type":"claude"}]"#,
        )
        .expect("seed migration snapshot");

    let count = migrate_skills_to_ssot(&state.db).expect("migrate skills to ssot");
    assert_eq!(count, 0, "native import requires explicit selection");
    assert!(state.db.get_all_installed_skills().unwrap().is_empty());
    assert!(home.join(".claude/skills/demo-skill/SKILL.md").is_file());
    assert!(home
        .join(".config/opencode/skills/demo-skill/SKILL.md")
        .is_file());
    assert!(state
        .db
        .library_snapshot()
        .unwrap()
        .machine
        .receipts
        .is_empty());
}
