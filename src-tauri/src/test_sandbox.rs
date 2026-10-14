//! Process-environment sandbox shared by every test harness.
//!
//! Anything that resolves a user directory (the home, `%APPDATA%`, the XDG dirs,
//! an agent's own `CODEX_HOME`...) reads the process environment, which is shared
//! by every test thread. [`sandbox`] serializes the tests that need a private
//! home, points all of those variables at a fresh temp dir, and puts the
//! environment back when the guard drops.
//!
//! This is the single implementation: the in-crate unit tests use it as
//! `crate::test_sandbox::sandbox`, and `tests/support.rs` re-exports it for the
//! integration tests (they link the library built without `cfg(test)`, so the
//! module cannot be `#[cfg(test)]`). The application never calls it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tempfile::TempDir;

use crate::settings::{reload_settings, update_settings, AppSettings};

/// Variables set into the sandbox home, with their location below it.
const SANDBOX_VARS: &[(&str, &str)] = &[
    ("HOME", ""),
    ("USERPROFILE", ""),
    ("CC_SWITCH_TEST_HOME", ""),
    ("APPDATA", "AppData/Roaming"),
    ("LOCALAPPDATA", "AppData/Local"),
    ("XDG_CONFIG_HOME", ".config"),
    ("XDG_DATA_HOME", ".local/share"),
    ("XDG_CACHE_HOME", ".cache"),
    ("XDG_STATE_HOME", ".local/state"),
];

static SANDBOX_LOCK: Mutex<()> = Mutex::new(());

/// Guard returned by [`sandbox`]. Field order matters: the temp dir is removed
/// after the environment is restored, and the lock is released last.
pub struct Sandbox {
    dir: TempDir,
    saved: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl Sandbox {
    /// The sandbox home directory.
    pub fn home(&self) -> &Path {
        self.dir.path()
    }
}

/// Serialize the test, give it a fresh home, and route every user-directory
/// variable into it (the agents' own `AGENT_DIR_VARS` are cleared). The real home
/// is never read. Everything is put back when the guard drops.
pub fn sandbox() -> Sandbox {
    enter(SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner()))
}

fn enter(lock: MutexGuard<'static, ()>) -> Sandbox {
    let dir = tempfile::Builder::new()
        .prefix("ofox-sandbox-")
        .tempdir()
        .expect("create the sandbox home");

    let mut saved = Vec::new();
    for (name, relative) in SANDBOX_VARS {
        saved.push((*name, std::env::var_os(name)));
        std::env::set_var(name, location(dir.path(), relative));
    }
    for name in crate::config::AGENT_DIR_VARS {
        saved.push((*name, std::env::var_os(name)));
        std::env::remove_var(name);
    }

    // Built before `reload_settings` so a failure there still restores the environment.
    let sandbox = Sandbox {
        dir,
        saved,
        _lock: lock,
    };
    reload_settings().expect("reload settings for the sandbox");
    sandbox
}

/// `home` itself for an empty `relative` (no trailing separator), else `home` plus
/// the `/`-separated components of `relative` (joined one by one so Windows paths
/// do not end up with mixed separators).
fn location(home: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(home.to_path_buf(), |path, part| path.join(part))
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Reset the in-memory settings while the environment still points into the
        // sandbox: saving defaults must land in the sandbox, and reloading after the
        // restore below would read whatever home the environment goes back to (the
        // real one, in an integration-test binary).
        if let Err(err) = update_settings(AppSettings::default()) {
            eprintln!("sandbox: could not reset the settings: {err}");
        }
        for (name, value) in self.saved.iter().rev() {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Sets a variable the way a developer's shell would, and removes it again.
    struct ShellVar(&'static str);

    impl ShellVar {
        fn set(name: &'static str, value: &str) -> Self {
            std::env::set_var(name, value);
            Self(name)
        }
    }

    impl Drop for ShellVar {
        fn drop(&mut self) {
            std::env::remove_var(self.0);
        }
    }

    #[test]
    #[serial]
    fn sandbox_routes_user_dirs_into_a_fresh_home_and_restores_the_environment() {
        let lock = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _codex = ShellVar::set("CODEX_HOME", "/developer/codex");
        let _xdg = ShellVar::set("XDG_DATA_HOME", "/developer/data");

        let sandbox = enter(lock);
        let home = sandbox.home().to_path_buf();
        assert_eq!(crate::config::get_home_dir(), home);
        for (name, relative) in SANDBOX_VARS {
            assert_eq!(
                std::env::var_os(name),
                Some(location(&home, relative).into_os_string()),
                "{name} should point into the sandbox"
            );
        }
        for name in crate::config::AGENT_DIR_VARS {
            assert!(std::env::var_os(name).is_none(), "{name} should be cleared");
        }

        drop(sandbox);

        assert!(!home.exists(), "the sandbox home is removed on drop");
        assert_eq!(
            std::env::var_os("CODEX_HOME"),
            Some(OsString::from("/developer/codex"))
        );
        assert_eq!(
            std::env::var_os("XDG_DATA_HOME"),
            Some(OsString::from("/developer/data"))
        );
    }

    #[test]
    #[serial]
    fn every_sandbox_gets_its_own_home() {
        let first = {
            let sandbox = sandbox();
            let marker = sandbox.home().join("marker");
            std::fs::write(&marker, "x").unwrap();
            sandbox.home().to_path_buf()
        };

        let sandbox = sandbox();

        assert_ne!(sandbox.home(), first);
        assert!(!sandbox.home().join("marker").exists());
    }

    #[test]
    #[serial]
    fn settings_a_test_changed_do_not_outlive_its_sandbox() {
        {
            let _sandbox = sandbox();
            let mut changed = crate::settings::get_settings();
            changed.use_app_window_controls = true;
            update_settings(changed).expect("change a setting");
            assert!(crate::settings::get_settings().use_app_window_controls);
        }

        assert!(!crate::settings::get_settings().use_app_window_controls);
    }
}
