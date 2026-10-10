//! 会让工具绕过 Ofox 配置的环境变量。只报变量名和来源，从不带值。
//!
//! - Gemini CLI 读 key、地址、模型时，环境变量优先于 `~/.gemini/.env`：系统里留着
//!   一个 Google 的 `GEMINI_API_KEY`，请求就会带着它打到 Ofox，返回 401。
//! - Claude Code 用 Ofox 写进 settings.json 的 `ANTHROPIC_AUTH_TOKEN`（它会盖过同名
//!   环境变量），但环境里再有一个 `ANTHROPIC_API_KEY` 就会和它冲突。
//! - Codex、OpenCode 的 key 和地址写在各自的配置里，不受环境变量影响。

use serde::Serialize;
#[cfg(not(target_os = "windows"))]
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvOverride {
    pub name: String,
    /// `user` / `machine`：Windows 的用户、系统环境变量；`process`：Ofox 启动时
    /// 带着（Windows 上 Ofox 打开的终端会继承）；`file`：shell 配置文件。
    pub scope: &'static str,
    /// `file` 时是 `~/.zshrc:12`。
    pub location: Option<String>,
}

/// `tool` 的哪些环境变量会盖过 Ofox 写入的配置。
pub fn overriding_vars(tool: &str) -> &'static [&'static str] {
    match tool {
        "gemini" => &["GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL", "GEMINI_MODEL"],
        "claude" => &["ANTHROPIC_API_KEY"],
        _ => &[],
    }
}

/// 新开的终端里会盖过 Ofox 配置的环境变量。
#[cfg(target_os = "windows")]
pub fn find(tool: &str) -> Vec<EnvOverride> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .ok();
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment")
        .ok();
    let set_in = |key: &Option<RegKey>, name: &str| {
        key.as_ref()
            .and_then(|key| key.get_value::<String, _>(name).ok())
            .is_some_and(|value| !value.trim().is_empty())
    };
    overriding_vars(tool)
        .iter()
        .flat_map(|name| {
            scopes(
                name,
                set_in(&user, name),
                set_in(&machine, name),
                std::env::var_os(name).is_some_and(|value| !value.is_empty()),
            )
        })
        .collect()
}

/// 新开的终端里会盖过 Ofox 配置的环境变量。
#[cfg(not(target_os = "windows"))]
pub fn find(tool: &str) -> Vec<EnvOverride> {
    find_in_home(tool, &crate::config::get_home_dir())
}

/// 注册表里有就报注册表；只有 Ofox 进程里有，说明变量已删但 Ofox 还没重启。
#[cfg(any(target_os = "windows", test))]
fn scopes(name: &str, user: bool, machine: bool, process: bool) -> Vec<EnvOverride> {
    let found = |scope| EnvOverride {
        name: name.into(),
        scope,
        location: None,
    };
    let mut found_in: Vec<_> = [(user, "user"), (machine, "machine")]
        .into_iter()
        .filter(|(set, _)| *set)
        .map(|(_, scope)| found(scope))
        .collect();
    if found_in.is_empty() && process {
        found_in.push(found("process"));
    }
    found_in
}

/// 登录 shell（zsh、bash）会读的配置文件。
#[cfg(not(target_os = "windows"))]
const SHELL_FILES: &[&str] = &[
    ".zshenv",
    ".zprofile",
    ".zshrc",
    ".bash_profile",
    ".bashrc",
    ".profile",
];

#[cfg(not(target_os = "windows"))]
fn find_in_home(tool: &str, home: &Path) -> Vec<EnvOverride> {
    let names = overriding_vars(tool);
    if names.is_empty() {
        return Vec::new();
    }
    SHELL_FILES
        .iter()
        .flat_map(|file| {
            let text = std::fs::read_to_string(home.join(file)).unwrap_or_default();
            exports_in(&text, names)
                .into_iter()
                .map(move |(name, line)| EnvOverride {
                    name,
                    scope: "file",
                    location: Some(format!("~/{file}:{line}")),
                })
        })
        .collect()
}

/// `text` 里给 `names` 赋了非空值的行：`(变量名, 行号)`，行号从 1 开始。
#[cfg(any(not(target_os = "windows"), test))]
fn exports_in(text: &str, names: &[&str]) -> Vec<(String, usize)> {
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            let assignment = line.strip_prefix("export").map_or(line, str::trim_start);
            let (name, value) = assignment.split_once('=')?;
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            (names.contains(&name) && !value.is_empty()).then(|| (name.to_string(), index + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_variables_that_beat_the_tools_own_config_are_checked() {
        assert!(overriding_vars("gemini").contains(&"GEMINI_API_KEY"));
        assert_eq!(overriding_vars("claude"), ["ANTHROPIC_API_KEY"]);
        assert!(overriding_vars("codex").is_empty());
        assert!(overriding_vars("opencode").is_empty());
    }

    #[test]
    fn shell_exports_are_found_by_exact_name_with_their_line() {
        let text = "# export GEMINI_API_KEY=old\nexport GEMINI_API_KEY=\"AIza-mine\"\nGEMINI_SANDBOX=docker\nexport GEMINI_MODEL=\n  GOOGLE_GEMINI_BASE_URL='https://x'\n";
        assert_eq!(
            exports_in(text, overriding_vars("gemini")),
            [
                ("GEMINI_API_KEY".to_string(), 2),
                ("GOOGLE_GEMINI_BASE_URL".to_string(), 5),
            ]
        );
    }

    #[test]
    fn a_variable_ofox_still_carries_counts_only_once_removed_from_the_registry() {
        let machine = scopes("GEMINI_API_KEY", false, true, true);
        assert_eq!(machine.len(), 1);
        assert_eq!(machine[0].scope, "machine");
        let stale = scopes("GEMINI_API_KEY", false, false, true);
        assert_eq!(stale[0].scope, "process");
        assert!(scopes("GEMINI_API_KEY", false, false, false).is_empty());
        assert_eq!(scopes("GEMINI_API_KEY", true, true, true).len(), 2);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn shell_files_in_home_are_reported_without_values() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join(".zshrc"),
            "alias ll='ls -l'\nexport GEMINI_API_KEY=AIza-secret\n",
        )
        .unwrap();
        let found = find_in_home("gemini", home.path());
        assert_eq!(
            found,
            [EnvOverride {
                name: "GEMINI_API_KEY".into(),
                scope: "file",
                location: Some("~/.zshrc:2".into()),
            }]
        );
        assert!(!serde_json::to_string(&found).unwrap().contains("AIza"));
        assert!(find_in_home("codex", home.path()).is_empty());
    }
}
