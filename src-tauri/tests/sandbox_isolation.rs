//! The integration-test sandbox keeps every user-directory lookup inside its own
//! temp home, and puts the process environment back afterwards.

mod support;

use std::ffi::OsString;
use std::path::PathBuf;

use cc_switch_lib::{get_claude_mcp_path, AGENT_DIR_VARS};

#[test]
fn sandbox_confines_the_app_to_its_home_and_restores_the_environment() {
    // This is the only test in this binary, so nothing else touches the variable.
    std::env::set_var("CODEX_HOME", "/developer/codex");
    let real_home: Option<OsString> = std::env::var_os("HOME");

    let sandbox = support::sandbox();
    let home: PathBuf = sandbox.home().to_path_buf();

    assert_eq!(
        std::env::var_os("HOME"),
        Some(home.clone().into_os_string())
    );
    assert_eq!(
        get_claude_mcp_path(),
        home.join(".claude.json"),
        "the app resolves its paths below the sandbox home"
    );
    for name in AGENT_DIR_VARS {
        assert!(std::env::var_os(name).is_none(), "{name} should be cleared");
    }

    drop(sandbox);

    assert!(!home.exists(), "the sandbox home is removed on drop");
    assert_eq!(std::env::var_os("HOME"), real_home);
    assert_eq!(
        std::env::var_os("CODEX_HOME"),
        Some(OsString::from("/developer/codex"))
    );
    std::env::remove_var("CODEX_HOME");
}
