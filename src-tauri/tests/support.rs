use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use cc_switch_lib::{
    update_settings, AppSettings, AppState, Database, MultiAppConfig, AGENT_DIR_VARS,
};
use tempfile::TempDir;

/// Variables pointed into the sandbox home, with their location below it.
/// `src/test_support.rs` has the same table for the in-crate unit tests; keep them in step.
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
    #[allow(dead_code)] // not every test binary asks for it
    pub fn home(&self) -> &Path {
        self.dir.path()
    }
}

/// Serialize the test, give it a fresh home, and route every user-directory
/// variable into it (the agents' own `AGENT_DIR_VARS` are cleared). The real home
/// is never read. Everything is put back when the guard drops.
///
/// The crate does not expose its settings reload, so the in-memory settings are
/// reset by saving defaults into the new home.
pub fn sandbox() -> Sandbox {
    let lock = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::Builder::new()
        .prefix("ofox-sandbox-")
        .tempdir()
        .expect("create the sandbox home");

    let mut saved = Vec::new();
    for (name, relative) in SANDBOX_VARS {
        saved.push((*name, std::env::var_os(name)));
        std::env::set_var(name, location(dir.path(), relative));
    }
    for name in AGENT_DIR_VARS {
        saved.push((*name, std::env::var_os(name)));
        std::env::remove_var(name);
    }

    // Built before touching settings so a failure there still restores the environment.
    let sandbox = Sandbox {
        dir,
        saved,
        _lock: lock,
    };
    update_settings(AppSettings::default()).expect("reset settings in the sandbox");
    sandbox
}

/// `home` itself for an empty `relative` (no trailing separator), else `home/relative`.
fn location(home: &Path, relative: &str) -> PathBuf {
    if relative.is_empty() {
        home.to_path_buf()
    } else {
        home.join(relative)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        for (name, value) in self.saved.iter().rev() {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

/// 创建测试用的 AppState，包含一个空的数据库
#[allow(dead_code)]
pub fn create_test_state() -> Result<AppState, Box<dyn std::error::Error>> {
    let db = Arc::new(Database::init()?);
    Ok(AppState::new(db))
}

/// 创建测试用的 AppState，并从 MultiAppConfig 迁移数据
#[allow(dead_code)]
pub fn create_test_state_with_config(
    config: &MultiAppConfig,
) -> Result<AppState, Box<dyn std::error::Error>> {
    let db = Arc::new(Database::init()?);
    db.migrate_from_json(config)?;
    Ok(AppState::new(db))
}
