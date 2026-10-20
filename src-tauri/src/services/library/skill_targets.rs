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

pub struct SkillTarget {
    pub id: &'static str,
    pub label: &'static str,
    /// The id `get_tool_versions` knows the agent by, when it can detect it.
    pub tool: Option<&'static str>,
    /// Reads ~/.agents/skills as well as its own folder.
    pub reads_shared: bool,
}

pub const TARGETS: &[SkillTarget] = &[
    SkillTarget {
        id: "claude",
        label: "Claude Code",
        tool: Some("claude"),
        reads_shared: false,
    },
    SkillTarget {
        id: "codex",
        label: "Codex",
        tool: Some("codex"),
        reads_shared: true,
    },
    SkillTarget {
        id: "gemini",
        label: "Gemini CLI",
        tool: Some("gemini"),
        reads_shared: true,
    },
    SkillTarget {
        id: "opencode",
        label: "OpenCode",
        tool: Some("opencode"),
        reads_shared: true,
    },
    SkillTarget {
        id: "hermes",
        label: "Hermes",
        tool: Some("hermes"),
        reads_shared: false,
    },
    SkillTarget {
        id: "openclaw",
        label: "OpenClaw",
        tool: Some("openclaw"),
        reads_shared: false,
    },
    SkillTarget {
        id: "pi",
        label: "Pi",
        tool: Some("pi"),
        reads_shared: false,
    },
    SkillTarget {
        id: "dsh",
        label: "DeepSeek Harness",
        tool: Some("dsh"),
        reads_shared: true,
    },
    SkillTarget {
        id: "workbuddy",
        label: "WorkBuddy",
        tool: Some("workbuddy"),
        reads_shared: false,
    },
    SkillTarget {
        id: "cursor",
        label: "Cursor",
        tool: None,
        reads_shared: false,
    },
    SkillTarget {
        id: "codebuddy",
        label: "CodeBuddy",
        tool: None,
        reads_shared: false,
    },
];

pub fn find(id: &str) -> Option<&'static SkillTarget> {
    TARGETS.iter().find(|target| target.id == id)
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

/// The agent's own folder: its presence alone says the agent is here.
pub fn home(id: &str) -> Result<PathBuf, AppError> {
    let home = crate::config::get_home_dir();
    Ok(match id {
        "claude" => crate::config::get_claude_config_dir(),
        "codex" => crate::codex_config::get_codex_config_dir(),
        "gemini" => crate::gemini_config::get_gemini_dir(),
        "opencode" => crate::opencode_config::get_opencode_dir(),
        "hermes" => crate::hermes_config::get_hermes_dir(),
        "openclaw" => home.join(".openclaw"),
        "pi" => env_dir("PI_CODING_AGENT_DIR", ".pi/agent")?,
        "dsh" => env_dir("DSH_HOME", ".dsh")?,
        "workbuddy" => env_dir("WORKBUDDY_CONFIG_DIR", ".workbuddy")?,
        "cursor" => home.join(".cursor"),
        "codebuddy" => env_dir("CODEBUDDY_CONFIG_DIR", ".codebuddy")?,
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
    home(id).is_ok_and(|home| home.is_dir()) || (id == "cursor" && app_installed(&["Cursor.app"]))
}

#[cfg(test)]
mod tests {
    use super::*;

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
