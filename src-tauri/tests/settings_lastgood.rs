//! A settings.json that does not parse is recovered from its `.bak` or blocks
//! writes; it is never read as "no settings" and saved over.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use cc_switch_lib::{
    get_settings, reload_settings, reset_unreadable, settings_health, update_settings, AppSettings,
    SettingsHealthState,
};

fn settings_file(home: &Path) -> PathBuf {
    home.join(".ofox-desktop").join("settings.json")
}

fn bak_file(home: &Path) -> PathBuf {
    home.join(".ofox-desktop").join("settings.json.bak")
}

fn bad_copies(home: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(home.join(".ofox-desktop"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("settings.json.bad-")
        })
        .collect();
    found.sort();
    found
}

fn with_language(language: &str) -> AppSettings {
    AppSettings {
        language: Some(language.to_string()),
        ..AppSettings::default()
    }
}

fn language() -> Option<String> {
    futures::executor::block_on(get_settings())
        .expect("read settings")
        .language
}

/// Two saves leave `zh` as the last good generation next to an `en` file.
fn save_zh_then_en(home: &Path) {
    update_settings(with_language("zh")).unwrap();
    update_settings(with_language("en")).unwrap();
    assert!(bak_file(home).exists(), "the second save keeps the first");
}

#[test]
fn a_missing_settings_file_is_ok() {
    let _sandbox = support::sandbox();

    reload_settings().unwrap();

    assert_eq!(settings_health().state, SettingsHealthState::Ok);
}

#[test]
fn a_corrupt_settings_file_with_a_good_backup_loads_the_backup_and_reports_recovered() {
    let sandbox = support::sandbox();
    let home = sandbox.home();
    save_zh_then_en(home);
    let backup = fs::read(bak_file(home)).unwrap();
    fs::write(settings_file(home), vec![0u8; 4096]).unwrap();

    reload_settings().unwrap();

    let health = settings_health();
    assert_eq!(health.state, SettingsHealthState::Recovered);
    assert_eq!(health.path, settings_file(home).display().to_string());
    assert_eq!(language().as_deref(), Some("zh"), "the backup is loaded");
    let bad = bad_copies(home);
    assert_eq!(bad.len(), 1);
    assert_eq!(
        health.bad_copy.as_deref(),
        Some(bad[0].display().to_string().as_str())
    );
    assert_eq!(fs::read(&bad[0]).unwrap(), vec![0u8; 4096]);
    assert_eq!(
        fs::read(settings_file(home)).unwrap(),
        backup,
        "the recovered content is written back"
    );
}

#[test]
fn a_recovered_settings_file_is_clean_on_the_next_start() {
    let sandbox = support::sandbox();
    let home = sandbox.home();
    save_zh_then_en(home);
    fs::write(settings_file(home), b"{\"language\":").unwrap();
    reload_settings().unwrap();
    assert_eq!(settings_health().state, SettingsHealthState::Recovered);

    reload_settings().unwrap();

    assert_eq!(settings_health().state, SettingsHealthState::Ok);
    assert_eq!(language().as_deref(), Some("zh"));
    assert_eq!(bad_copies(home).len(), 1, "the bad bytes are kept once");
}

#[test]
fn a_recovered_settings_file_can_be_saved_again() {
    let sandbox = support::sandbox();
    let home = sandbox.home();
    save_zh_then_en(home);
    fs::write(settings_file(home), vec![0u8; 64]).unwrap();
    reload_settings().unwrap();

    update_settings(with_language("ja")).unwrap();

    assert_eq!(language().as_deref(), Some("ja"));
}

#[test]
fn a_corrupt_settings_file_without_a_backup_is_unreadable_and_refuses_saves() {
    let sandbox = support::sandbox();
    let home = sandbox.home();
    fs::create_dir_all(home.join(".ofox-desktop")).unwrap();
    let corrupt = b"{\"language\": \"zh\", ".to_vec();
    fs::write(settings_file(home), &corrupt).unwrap();

    reload_settings().unwrap();

    let health = settings_health();
    assert_eq!(health.state, SettingsHealthState::Unreadable);
    assert_eq!(
        health.bad_copy.as_deref().map(Path::new),
        bad_copies(home).first().map(|p| p.as_path())
    );
    assert!(health.detail.is_some());
    let err = update_settings(with_language("en")).unwrap_err();
    assert!(
        err.to_string().starts_with("settings_unreadable"),
        "error carries the code, got: {err}"
    );
    assert_eq!(
        fs::read(settings_file(home)).unwrap(),
        corrupt,
        "the unreadable file is not written over"
    );
    assert!(!bak_file(home).exists());
}

#[test]
fn resetting_unreadable_settings_keeps_the_bad_copy_and_lets_saves_work() {
    let sandbox = support::sandbox();
    let home = sandbox.home();
    fs::create_dir_all(home.join(".ofox-desktop")).unwrap();
    let corrupt = vec![0u8; 256];
    fs::write(settings_file(home), &corrupt).unwrap();
    reload_settings().unwrap();
    assert_eq!(settings_health().state, SettingsHealthState::Unreadable);

    let health = reset_unreadable().unwrap();

    assert_eq!(health.state, SettingsHealthState::Ok);
    assert_eq!(settings_health().state, SettingsHealthState::Ok);
    let bad = bad_copies(home);
    assert_eq!(bad.len(), 1);
    assert_eq!(fs::read(&bad[0]).unwrap(), corrupt, "the bad bytes survive");
    let written: AppSettings =
        serde_json::from_slice(&fs::read(settings_file(home)).unwrap()).unwrap();
    assert_eq!(written.language, None, "defaults are written");
    update_settings(with_language("en")).unwrap();
    assert_eq!(language().as_deref(), Some("en"));
}

#[test]
fn resetting_settings_that_are_not_unreadable_changes_nothing() {
    let sandbox = support::sandbox();
    let home = sandbox.home();
    update_settings(with_language("zh")).unwrap();
    let before = fs::read(settings_file(home)).unwrap();

    let health = reset_unreadable().unwrap();

    assert_eq!(health.state, SettingsHealthState::Ok);
    assert_eq!(fs::read(settings_file(home)).unwrap(), before);
    assert_eq!(language().as_deref(), Some("zh"));
}

#[test]
fn a_valid_settings_file_is_ok_and_a_save_keeps_the_previous_generation() {
    let sandbox = support::sandbox();
    let home = sandbox.home();
    update_settings(with_language("zh")).unwrap();
    assert!(
        !bak_file(home).exists(),
        "nothing to keep on the first save"
    );
    let first = fs::read(settings_file(home)).unwrap();

    update_settings(with_language("en")).unwrap();

    reload_settings().unwrap();
    assert_eq!(settings_health().state, SettingsHealthState::Ok);
    assert_eq!(fs::read(bak_file(home)).unwrap(), first);
    assert_eq!(language().as_deref(), Some("en"));
    assert!(bad_copies(home).is_empty());
}

#[cfg(unix)]
#[test]
fn the_settings_file_and_its_backup_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = support::sandbox();
    let home = sandbox.home();
    save_zh_then_en(home);

    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&settings_file(home)), 0o600);
    assert_eq!(mode(&bak_file(home)), 0o600);
}

#[test]
fn a_test_that_left_settings_unreadable_does_not_block_the_next_sandbox() {
    {
        let sandbox = support::sandbox();
        fs::create_dir_all(sandbox.home().join(".ofox-desktop")).unwrap();
        fs::write(settings_file(sandbox.home()), vec![0u8; 16]).unwrap();
        reload_settings().unwrap();
        assert_eq!(settings_health().state, SettingsHealthState::Unreadable);
    }
    assert_eq!(
        settings_health().state,
        SettingsHealthState::Ok,
        "the sandbox forgets the state when it goes"
    );

    let _sandbox = support::sandbox();

    assert_eq!(settings_health().state, SettingsHealthState::Ok);
    update_settings(with_language("en")).expect("a clean sandbox can save");
}
