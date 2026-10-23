//! The agents Ofox can give skills to: each one's skills folder, whether it
//! also reads the shared ~/.agents/skills, and how to tell it is on this machine.
use crate::{app_config::AppType, error::AppError};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

fn error(key: &str) -> AppError {
    AppError::Config(format!("library.{key}"))
}

pub use super::extension_targets::SkillTarget;

const LEGACY_TARGETS: &[SkillTarget] = &[
    SkillTarget {
        id: "cursor",
        label: "Cursor",
        reads_shared: false,
    },
    SkillTarget {
        id: "codebuddy",
        label: "CodeBuddy",
        reads_shared: false,
    },
    SkillTarget {
        id: "claude-desktop",
        label: "Claude Desktop",
        reads_shared: false,
    },
];

pub fn all() -> impl Iterator<Item = &'static SkillTarget> {
    super::extension_targets::TARGETS
        .iter()
        .chain(LEGACY_TARGETS)
}
pub fn find(id: &str) -> Option<&'static SkillTarget> {
    all().find(|target| target.id == id)
}

/// `$VARIABLE` (absolute, or under `~/`), else `~/<default>`.
fn env_dir(variable: &str, default: &str) -> Result<PathBuf, AppError> {
    let home = crate::config::get_home_dir();
    let Some(raw) = std::env::var_os(variable) else {
        return Ok(home.join(default));
    };
    let raw = raw.to_string_lossy();
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(home.join(default));
    }
    let path = if raw == "~" {
        home
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(raw)
    };
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(error("skillScopeUnsupported"))
    }
}

/// Where desktop apps keep their data for this user.
pub(crate) fn app_data_dir() -> PathBuf {
    let home = crate::config::get_home_dir();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .filter(|value| !value.is_empty() && !cfg!(test))
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
    } else {
        home.join(".config")
    }
}
/// Claude Desktop's Cowork reads the user's skills from a plugin of its own,
/// one for each account it has been signed in with (and Claude-3p's alike):
/// `<app>/local-agent-mode-sessions/skills-plugin/<org>/<account>/`, holding
/// `manifest.json` and `skills/<name>/`. The most recently used one first.
pub fn desktop_accounts() -> Vec<PathBuf> {
    let mut accounts: Vec<(Option<std::time::SystemTime>, PathBuf)> = vec![];
    for app in ["Claude", "Claude-3p"] {
        let base = app_data_dir()
            .join(app)
            .join("local-agent-mode-sessions/skills-plugin");
        for org in std::fs::read_dir(&base).into_iter().flatten().flatten() {
            for account in std::fs::read_dir(org.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let path = account.path();
                let manifest = path.join(DESKTOP_MANIFEST);
                if path.is_dir()
                    && (manifest.is_file() || path.join(".claude-plugin/plugin.json").is_file())
                {
                    let used = std::fs::metadata(&manifest)
                        .and_then(|meta| meta.modified())
                        .ok();
                    accounts.push((used, path));
                }
            }
        }
    }
    accounts.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    accounts.into_iter().map(|(_, path)| path).collect()
}
pub const DESKTOP_MANIFEST: &str = "manifest.json";

/// The agent's own folder: its presence alone says the agent is here.
pub fn home(id: &str) -> Result<PathBuf, AppError> {
    let home = crate::config::get_home_dir();
    Ok(match id {
        "claude" => crate::config::get_claude_config_dir(),
        "codex" => crate::codex_config::get_codex_config_dir(),
        "gemini" => crate::gemini_config::get_gemini_dir(),
        "opencode" => crate::opencode_config::get_opencode_dir(),
        "hermes" => crate::hermes_config::get_hermes_dir(),
        "openclaw" => crate::openclaw_config::resolve_openclaw_state_dir()?,
        "pi" => env_dir("PI_CODING_AGENT_DIR", ".pi/agent")?,
        "dsh" => env_dir("DSH_HOME", ".dsh")?,
        "workbuddy" => env_dir("WORKBUDDY_CONFIG_DIR", ".workbuddy")?,
        "cursor" => home.join(".cursor"),
        "codebuddy" => env_dir("CODEBUDDY_CONFIG_DIR", ".codebuddy")?,
        "claude-desktop" => desktop_accounts()
            .into_iter()
            .next()
            .ok_or_else(|| error("targetUnreadable"))?,
        _ => return Err(error("targetUnsupported")),
    })
}

/// Where the agent finds its user-wide skills.
pub fn root(id: &str) -> Result<PathBuf, AppError> {
    let path = match AppType::from_str(id) {
        Ok(app) if find(id).is_some() => {
            crate::services::skill::SkillService::get_app_skills_dir(&app)
                .map_err(|_| error("targetUnreadable"))?
        }
        _ => home(id)?.join("skills"),
    };
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(error("skillScopeUnsupported"))
    }
}

fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![crate::config::get_home_dir().join("Applications")];
    // Tests run against a sandboxed home only.
    if cfg!(not(test)) {
        dirs.push(Path::new("/Applications").to_path_buf());
    }
    dirs
}
fn app_installed(names: &[&str]) -> bool {
    names
        .iter()
        .any(|name| app_dirs().iter().any(|dir| dir.join(name).is_dir()))
}

/// The agent's folder is here (a file in its place does not count), or its app is.
pub fn detected_locally(id: &str) -> bool {
    if id == "claude-desktop" {
        return !desktop_accounts().is_empty();
    }
    home(id).is_ok_and(|home| home.is_dir()) || (id == "cursor" && app_installed(&["Cursor.app"]))
}

/// The manifest Claude Desktop lists its skills from, when `entry` is one of them.
pub fn desktop_manifest_for(entry: &Path) -> Option<PathBuf> {
    let account = home("claude-desktop").ok()?;
    (entry.parent() == Some(account.join("skills").as_path()))
        .then(|| account.join(DESKTOP_MANIFEST))
}
fn iso_now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}
/// Add (or renew) `name`'s entry, or drop it, in Claude Desktop's manifest. A
/// skill is listed only once the manifest has it. Other entries and keys stay
/// as they are, an entry's `enabled` too; Desktop reads it again on reload.
pub fn desktop_manifest_set(
    manifest: &Path,
    name: &str,
    description: Option<&str>,
    listed: bool,
) -> Result<(), AppError> {
    use serde_json::{json, Value};
    let text = crate::services::library::coordinator::read(manifest)?;
    let mut root: Value = match &text {
        Some(text) => serde_json::from_str(text).map_err(|_| error("nativeInvalid"))?,
        None => json!({ "lastUpdated": 0, "skills": [] }),
    };
    let object = root.as_object_mut().ok_or_else(|| error("nativeInvalid"))?;
    let stamp = if object.get("lastUpdated").is_some_and(Value::is_string) {
        json!(iso_now())
    } else {
        json!(chrono::Utc::now().timestamp_millis())
    };
    let skills = object
        .entry("skills")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| error("nativeInvalid"))?;
    let index = skills
        .iter()
        .position(|entry| entry.get("skillId").and_then(Value::as_str) == Some(name));
    if listed {
        // In the order Desktop writes them; any other key of an old entry after.
        let old = index.map(|index| skills[index].clone());
        let mut entry = serde_json::Map::new();
        entry.insert("skillId".into(), json!(name));
        entry.insert("name".into(), json!(name));
        entry.insert("description".into(), json!(description.unwrap_or_default()));
        entry.insert("creatorType".into(), json!("user"));
        entry.insert("syncManaged".into(), json!(false));
        entry.insert("updatedAt".into(), json!(iso_now()));
        entry.insert(
            "enabled".into(),
            old.as_ref()
                .and_then(|old| old.get("enabled").cloned())
                .unwrap_or(json!(true)),
        );
        if let Some(Value::Object(old)) = old {
            for (key, value) in old {
                entry.entry(key).or_insert(value);
            }
        }
        match index {
            Some(index) => skills[index] = Value::Object(entry),
            None => skills.push(Value::Object(entry)),
        }
    } else if let Some(index) = index {
        skills.remove(index);
    } else {
        return Ok(());
    }
    object.insert("lastUpdated".into(), stamp);
    let mut body = serde_json::to_string_pretty(&root).map_err(|_| error("nativeInvalid"))?;
    if text.as_deref().is_none_or(|text| text.ends_with('\n')) {
        body.push('\n');
    }
    crate::config::atomic_write(manifest, body.as_bytes()).map_err(|_| error("skillWriteFailed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn openclaw_resources_follow_state_override_but_not_config_file_parent() {
        let sandbox = crate::test_sandbox::sandbox();
        let state = sandbox.home().join("state");
        let config = sandbox.home().join("config/custom.json");
        std::env::set_var("OPENCLAW_STATE_DIR", &state);
        std::env::set_var("OPENCLAW_CONFIG_PATH", &config);
        assert_eq!(root("openclaw").unwrap(), state.join("skills"));
        assert_eq!(super::super::mcp_targets::openclaw_path().unwrap(), config);
        std::env::set_var("OPENCLAW_STATE_DIR", "relative/state");
        assert!(root("openclaw").is_err());
    }

    #[test]
    #[serial_test::serial]
    fn each_agent_keeps_its_skills_in_its_own_folder() {
        let sandbox = crate::test_sandbox::sandbox();
        let home = sandbox.home().to_path_buf();
        for (id, expected) in [
            ("openclaw", home.join(".openclaw/skills")),
            ("pi", home.join(".pi/agent/skills")),
            ("dsh", home.join(".dsh/skills")),
            ("workbuddy", home.join(".workbuddy/skills")),
            ("cursor", home.join(".cursor/skills")),
            ("codebuddy", home.join(".codebuddy/skills")),
        ] {
            assert_eq!(root(id).unwrap(), expected, "{id}");
        }
        assert!(root("chatgpt").is_err());
    }

    #[test]
    #[serial_test::serial]
    fn folder_variables_move_an_agent_and_a_relative_one_is_refused() {
        let sandbox = crate::test_sandbox::sandbox();
        let elsewhere = sandbox.home().join("elsewhere");
        for variable in [
            "PI_CODING_AGENT_DIR",
            "DSH_HOME",
            "WORKBUDDY_CONFIG_DIR",
            "CODEBUDDY_CONFIG_DIR",
        ] {
            std::env::set_var(variable, &elsewhere);
        }
        for id in ["pi", "dsh", "workbuddy", "codebuddy"] {
            assert_eq!(root(id).unwrap(), elsewhere.join("skills"), "{id}");
        }
        std::env::set_var("DSH_HOME", "relative/dsh");
        assert!(root("dsh").is_err());
        assert!(root("pi").is_ok());
        for variable in [
            "PI_CODING_AGENT_DIR",
            "DSH_HOME",
            "WORKBUDDY_CONFIG_DIR",
            "CODEBUDDY_CONFIG_DIR",
        ] {
            std::env::remove_var(variable);
        }
    }

    #[test]
    #[serial_test::serial]
    fn an_agent_is_detected_by_its_folder_but_not_by_a_file_in_its_place() {
        let sandbox = crate::test_sandbox::sandbox();
        assert!(!detected_locally("cursor"));
        std::fs::create_dir_all(sandbox.home().join(".cursor")).unwrap();
        assert!(detected_locally("cursor"));
        std::fs::write(sandbox.home().join(".dsh"), "").unwrap();
        assert!(!detected_locally("dsh"));
    }
}
