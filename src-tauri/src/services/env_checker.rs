use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "windows"))]
use std::fs;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
use winreg::enums::*;
#[cfg(target_os = "windows")]
use winreg::RegKey;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvEnvironment {
    #[default]
    Native,
    Wsl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvConflict {
    pub var_name: String,
    pub var_value: String,
    pub source_type: String,
    /// Registry key or a file path without an appended line-number suffix.
    pub source_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_number: Option<usize>,
    #[serde(default)]
    pub environment: EnvEnvironment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wsl_distro: Option<String>,
}

/// Check native and explicitly configured WSL environments for conflicts.
pub fn check_env_conflicts(app: &str) -> Result<Vec<EnvConflict>, String> {
    let keywords = get_keywords_for_app(app);
    let mut conflicts = check_system_env(&keywords)?;

    #[cfg(not(target_os = "windows"))]
    conflicts.extend(check_shell_configs(&keywords)?);

    #[cfg(target_os = "windows")]
    if let Some(distro) = configured_wsl_distro(app) {
        match check_wsl_shell_configs(&keywords, &distro) {
            Ok(found) => conflicts.extend(found),
            Err(error) => log::warn!("检查 WSL 环境变量冲突失败 ({distro}): {error}"),
        }
    }

    Ok(conflicts)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvKeyword {
    Exact(&'static str),
    Prefix(&'static str),
}

fn get_keywords_for_app(app: &str) -> Vec<EnvKeyword> {
    match app.to_lowercase().as_str() {
        "claude" => vec![EnvKeyword::Prefix("ANTHROPIC")],
        "codex" => vec![EnvKeyword::Prefix("OPENAI")],
        "gemini" => vec![
            EnvKeyword::Prefix("GEMINI"),
            EnvKeyword::Prefix("GOOGLE_GEMINI"),
        ],
        "grokbuild" | "grok" => vec![
            EnvKeyword::Exact("XAI_API_KEY"),
            EnvKeyword::Exact("GROK_DEFAULT_MODEL"),
        ],
        _ => vec![],
    }
}

fn matches_env_keyword(name: &str, keywords: &[EnvKeyword]) -> bool {
    let upper_name = name.to_uppercase();
    keywords.iter().any(|keyword| match keyword {
        EnvKeyword::Exact(name) => upper_name == *name,
        EnvKeyword::Prefix(prefix) => upper_name.starts_with(prefix),
    })
}

fn conflict(
    var_name: String,
    var_value: String,
    source_type: &str,
    source_path: String,
    line_number: Option<usize>,
    environment: EnvEnvironment,
    wsl_distro: Option<String>,
) -> EnvConflict {
    EnvConflict {
        var_name,
        var_value,
        source_type: source_type.to_string(),
        source_path,
        line_number,
        environment,
        wsl_distro,
    }
}

#[cfg(target_os = "windows")]
fn check_system_env(keywords: &[EnvKeyword]) -> Result<Vec<EnvConflict>, String> {
    let mut conflicts = Vec::new();
    if let Ok(hkcu) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment") {
        for (name, value) in hkcu.enum_values().filter_map(Result::ok) {
            if matches_env_keyword(&name, keywords) {
                conflicts.push(conflict(
                    name,
                    value.to_string(),
                    "system",
                    "HKEY_CURRENT_USER\\Environment".to_string(),
                    None,
                    EnvEnvironment::Native,
                    None,
                ));
            }
        }
    }
    if let Ok(hklm) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
    {
        for (name, value) in hklm.enum_values().filter_map(Result::ok) {
            if matches_env_keyword(&name, keywords) {
                conflicts.push(conflict(
                    name,
                    value.to_string(),
                    "system",
                    "HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment".to_string(),
                    None,
                    EnvEnvironment::Native,
                    None,
                ));
            }
        }
    }
    Ok(conflicts)
}

#[cfg(not(target_os = "windows"))]
fn check_system_env(keywords: &[EnvKeyword]) -> Result<Vec<EnvConflict>, String> {
    Ok(std::env::vars()
        .filter(|(name, _)| matches_env_keyword(name, keywords))
        .map(|(name, value)| {
            conflict(
                name,
                value,
                "system",
                "Process Environment".to_string(),
                None,
                EnvEnvironment::Native,
                None,
            )
        })
        .collect())
}

fn parse_shell_assignment(line: &str, keywords: &[EnvKeyword]) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return None;
    }
    let assignment = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let (name, value) = assignment.split_once('=')?;
    let name = name.trim();
    if !matches_env_keyword(name, keywords) {
        return None;
    }
    Some((
        name.to_string(),
        value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string(),
    ))
}

#[cfg(not(target_os = "windows"))]
fn check_shell_configs(keywords: &[EnvKeyword]) -> Result<Vec<EnvConflict>, String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let config_files = [
        format!("{home}/.bashrc"),
        format!("{home}/.bash_profile"),
        format!("{home}/.zshrc"),
        format!("{home}/.zprofile"),
        format!("{home}/.profile"),
        "/etc/profile".to_string(),
        "/etc/bashrc".to_string(),
    ];
    let mut conflicts = Vec::new();
    for file_path in config_files {
        let Ok(content) = fs::read_to_string(&file_path) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            if let Some((name, value)) = parse_shell_assignment(line, keywords) {
                conflicts.push(conflict(
                    name,
                    value,
                    "file",
                    file_path.clone(),
                    Some(index + 1),
                    EnvEnvironment::Native,
                    None,
                ));
            }
        }
    }
    Ok(conflicts)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn valid_wsl_distro(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn wsl_distro_from_unc(path: &str) -> Option<String> {
    let normalized = path.replace('/', "\\");
    let normalized = normalized
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or(normalized);
    let lower = normalized.to_ascii_lowercase();
    for prefix in [r"\\wsl$\", r"\\wsl.localhost\"] {
        if lower.starts_with(prefix) {
            let distro = normalized[prefix.len()..]
                .split('\\')
                .next()
                .unwrap_or_default();
            return valid_wsl_distro(distro).then(|| distro.to_string());
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn configured_wsl_distro(app: &str) -> Option<String> {
    let path = match app.to_lowercase().as_str() {
        "claude" => crate::settings::get_claude_override_dir(),
        "codex" => crate::settings::get_codex_override_dir(),
        "gemini" => crate::settings::get_gemini_override_dir(),
        "opencode" => crate::settings::get_opencode_override_dir(),
        _ => None,
    }?;
    wsl_distro_from_unc(&path.to_string_lossy())
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_wsl_scan_output(output: &str, keywords: &[EnvKeyword], distro: &str) -> Vec<EnvConflict> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let path = parts.next()?.trim();
            let line_number = parts.next()?.parse::<usize>().ok()?;
            let assignment = parts.next()?;
            let (name, value) = parse_shell_assignment(assignment, keywords)?;
            Some(conflict(
                name,
                value,
                "file",
                path.to_string(),
                Some(line_number),
                EnvEnvironment::Wsl,
                Some(distro.to_string()),
            ))
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn check_wsl_shell_configs(
    keywords: &[EnvKeyword],
    distro: &str,
) -> Result<Vec<EnvConflict>, String> {
    if !valid_wsl_distro(distro) {
        return Err("invalid WSL distro name".to_string());
    }
    let script = concat!(
        "for file in \"$HOME/.bashrc\" \"$HOME/.bash_profile\" ",
        "\"$HOME/.zshrc\" \"$HOME/.zprofile\" \"$HOME/.profile\" ",
        "\"/etc/profile\" \"/etc/bash.bashrc\"; do ",
        "[ -f \"$file\" ] || continue; ",
        "awk '{printf \"%s\\t%d\\t%s\\n\", FILENAME, FNR, $0}' \"$file\"; ",
        "done"
    );
    let output = std::process::Command::new("wsl.exe")
        .args(["-d", distro, "--", "sh", "-c", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(parse_wsl_scan_output(
        &String::from_utf8_lossy(&output.stdout),
        keywords,
        distro,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_match_only_supported_prefixes() {
        let claude = get_keywords_for_app("claude");
        assert!(matches_env_keyword("ANTHROPIC_API_KEY", &claude));
        assert!(!matches_env_keyword("MY_ANTHROPIC_API_KEY", &claude));
        let grok = get_keywords_for_app("grok");
        assert!(matches_env_keyword("XAI_API_KEY", &grok));
        assert!(!matches_env_keyword("XAI_API_KEY_BACKUP", &grok));
    }

    #[test]
    fn parses_native_assignment_without_path_line_suffix() {
        let keywords = get_keywords_for_app("codex");
        assert_eq!(
            parse_shell_assignment("export OPENAI_API_KEY='secret'", &keywords),
            Some(("OPENAI_API_KEY".to_string(), "secret".to_string()))
        );
        assert!(parse_shell_assignment("# OPENAI_API_KEY=ignored", &keywords).is_none());
    }

    #[test]
    fn parses_both_wsl_unc_forms() {
        assert_eq!(
            wsl_distro_from_unc(r"\\wsl$\Ubuntu\home\alice\.codex"),
            Some("Ubuntu".to_string())
        );
        assert_eq!(
            wsl_distro_from_unc(r"\\wsl.localhost\Debian\home\alice"),
            Some("Debian".to_string())
        );
        assert_eq!(wsl_distro_from_unc(r"C:\Users\alice"), None);
    }

    #[test]
    fn wsl_scan_records_structured_location() {
        let conflicts = parse_wsl_scan_output(
            "/home/alice/.zshrc\t12\texport OPENAI_API_KEY=secret\n",
            &get_keywords_for_app("codex"),
            "Ubuntu",
        );
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].source_path, "/home/alice/.zshrc");
        assert_eq!(conflicts[0].line_number, Some(12));
        assert_eq!(conflicts[0].environment, EnvEnvironment::Wsl);
        assert_eq!(conflicts[0].wsl_distro.as_deref(), Some("Ubuntu"));
    }

    #[test]
    fn legacy_backup_defaults_to_native_environment() {
        let conflict: EnvConflict = serde_json::from_value(serde_json::json!({
            "varName": "OPENAI_API_KEY",
            "varValue": "secret",
            "sourceType": "file",
            "sourcePath": "/home/alice/.zshrc:12"
        }))
        .unwrap();
        assert_eq!(conflict.environment, EnvEnvironment::Native);
        assert_eq!(conflict.line_number, None);
    }
}
