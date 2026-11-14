//! Global native MCP targets. ChatGPT shares Codex's file and ownership target.
pub use super::extension_targets::TOOLS;
use crate::{app_config::BindableTool, error::AppError};
use std::path::PathBuf;

/// The builtin consumer is deliberately separate from the external agent catalogue.
pub fn supported(tool: &str) -> bool {
    tool == super::assistant_target::ID || TOOLS.contains(&tool)
}
pub fn all_tools() -> Vec<&'static str> {
    TOOLS
        .iter()
        .copied()
        .chain([super::assistant_target::ID])
        .collect()
}
pub fn validate_assistant_server(spec: &serde_json::Value) -> Result<(), AppError> {
    if let Some(raw) = spec.get("url").and_then(serde_json::Value::as_str) {
        let url =
            url::Url::parse(raw).map_err(|_| AppError::Config("library.invalidMcp".into()))?;
        let host = url
            .host_str()
            .unwrap_or("")
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if super::assistant_target::SIGN_IN_HOSTS.contains(&host.as_str()) {
            return Err(AppError::Config(
                "library.assistantMcpSignInUnsupported".into(),
            ));
        }
    }
    Ok(())
}
/// Revalidate frozen plans as well as new previews, including metadata-only/resumed writes.
pub(super) fn validate_operation(
    db: &crate::Database,
    op: &super::Operation,
) -> Result<(), AppError> {
    if op.kind != super::ResourceKind::Mcp {
        return Ok(());
    }
    if op.desired_updates.iter().any(|(key, value)| {
        key.ends_with(&format!(":{}", super::assistant_target::ID)) && value.is_some()
    }) {
        let servers = db.get_all_mcp_servers()?;
        let server = servers
            .get(&op.resource_id)
            .ok_or_else(|| AppError::Config("library.resourceMissing".into()))?;
        validate_assistant_server(&server.server)?;
    }
    for target in &op.targets {
        if target.path == super::assistant_target::mcp_config_path().to_string_lossy()
            && !target.after_text.is_empty()
        {
            let root =
                super::mcp_codec::parse_native(&target.after_text, super::assistant_target::ID)?;
            for server in super::mcp_codec::servers(&root, super::assistant_target::ID)
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
            {
                validate_assistant_server(server)?;
            }
        }
    }
    Ok(())
}

/// Assistant MCP JSON has its own private writer; it never touches skill state.
pub(super) fn write_assistant(text: &str) -> Result<(), AppError> {
    let path =
        super::assistant_target::state::private_path(&super::assistant_target::mcp_config_path())?;
    let root = super::mcp_codec::parse_native(text, super::assistant_target::ID)?;
    for server in super::mcp_codec::servers(&root, super::assistant_target::ID)
        .as_object()
        .into_iter()
        .flat_map(|v| v.values())
    {
        validate_assistant_server(server)?;
    }
    crate::config::create_private_dir_all(
        path.parent()
            .ok_or_else(|| AppError::Config("library.writeFailed".into()))?,
    )?;
    #[cfg(unix)]
    crate::config::atomic_write_private_with(&path, text.as_bytes(), || {})?;
    #[cfg(not(unix))]
    crate::config::atomic_write_with(
        &path,
        text.as_bytes(),
        crate::config::AtomicOpts { mode: Some(0o600) },
    )?;
    if super::assistant_target::state::read_regular(&path, 2 * 1024 * 1024)?.as_deref()
        != Some(text.as_bytes())
    {
        return Err(AppError::Config("library.verifyFailed".into()));
    }
    Ok(())
}

fn absolute_env(name: &str) -> Result<Option<PathBuf>, AppError> {
    let Some(value) = std::env::var_os(name).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let value = value
        .into_string()
        .map_err(|_| AppError::Config("library.mcpScopeUnsupported".into()))?;
    let path = if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        crate::config::get_home_dir().join(rest)
    } else {
        PathBuf::from(value)
    };
    if !path.is_absolute() {
        return Err(AppError::Config("library.mcpScopeUnsupported".into()));
    }
    Ok(Some(path))
}
pub fn openclaw_path() -> Result<PathBuf, AppError> {
    crate::openclaw_config::resolve_openclaw_config_path()
}
pub fn workbuddy_path() -> Result<PathBuf, AppError> {
    Ok(absolute_env("WORKBUDDY_CONFIG_DIR")?
        .unwrap_or_else(|| crate::config::get_home_dir().join(".workbuddy"))
        .join("mcp.json"))
}
pub fn native_tool_path(tool: BindableTool, file: &str) -> Result<PathBuf, AppError> {
    crate::native_tools::config_dir(tool)
        .map(|dir| dir.join(file))
        .map_err(|_| AppError::Config("library.mcpScopeUnsupported".into()))
}

/// Home patches affect every profile. Track the complete profile roster, including
/// absent patch files, so a new layer invalidates an outstanding preview.
pub fn dsh_profile_paths() -> Result<Vec<PathBuf>, AppError> {
    let directory = native_tool_path(BindableTool::Dsh, "profiles")?;
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => return Err(AppError::Config("library.targetUnreadable".into())),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| AppError::Config("library.targetUnreadable".into()))?;
        let kind = entry
            .file_type()
            .map_err(|_| AppError::Config("library.targetUnreadable".into()))?;
        if kind.is_symlink() {
            return Err(AppError::Config("library.mcpScopeUnsupported".into()));
        }
        if kind.is_dir() {
            paths.push(entry.path().join("cordis.patch.yml"));
        }
    }
    paths.sort();
    Ok(paths)
}

/// Pi shipped native MCP in 0.99.0 (upstream CHANGELOG, 2026-09-29).
pub fn version_reason(tool: &str, version: Option<&str>) -> Option<&'static str> {
    if tool != "pi" {
        return None;
    }
    let version = version.and_then(|raw| {
        raw.split_whitespace()
            .find_map(|word| semver::Version::parse(word.trim_start_matches('v')).ok())
    });
    if version.is_none_or(|v| v < semver::Version::new(0, 99, 0)) {
        Some("library.piMcpVersion")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_pi_is_not_advertised_as_native_mcp_capable() {
        for version in [None, Some("0.84.4"), Some("unknown")] {
            assert!(version_reason("pi", version).is_some());
        }
        for version in [Some("0.99.0"), Some("v1.3.0"), Some("pi 1.3.0")] {
            assert!(version_reason("pi", version).is_none());
        }
    }
    #[test]
    #[serial_test::serial]
    fn global_paths_follow_native_overrides_and_keep_the_alias_single() {
        let sandbox = crate::test_sandbox::sandbox();
        for key in [
            "OPENCLAW_CONFIG_PATH",
            "WORKBUDDY_CONFIG_DIR",
            "PI_CODING_AGENT_DIR",
            "DSH_HOME",
        ] {
            let path = sandbox.home().join(key);
            std::env::set_var(key, &path);
        }
        let paths = super::super::mcp_resource::native_paths().unwrap();
        assert_eq!(paths.len(), TOOLS.len() + 1);
        assert!(!paths.contains_key("chatgpt"));
        assert_eq!(
            paths["openclaw"],
            sandbox.home().join("OPENCLAW_CONFIG_PATH")
        );
        assert_eq!(
            paths["workbuddy"],
            sandbox.home().join("WORKBUDDY_CONFIG_DIR/mcp.json")
        );
        assert_eq!(
            paths["pi"],
            sandbox.home().join("PI_CODING_AGENT_DIR/mcp.json")
        );
        assert_eq!(
            paths["dsh"],
            sandbox.home().join("DSH_HOME/cordis.patch.yml")
        );
    }
}
