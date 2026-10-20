//! Compatibility APIs never project legacy enabled flags into native files.
//! Resource writes require the persistent library preview/apply coordinator.
use crate::{
    app_config::{McpApps, McpServer, MultiAppConfig},
    error::AppError,
};
use serde_json::Value;

fn legacy_import(config: &mut MultiAppConfig, tool: &str) -> Result<usize, AppError> {
    let _io = crate::config_io::lock();
    let root = if tool == "opencode" {
        crate::opencode_config::read_opencode_config()?
    } else {
        let path = crate::services::library::mcp_resource::native_paths()?
            .remove(tool)
            .ok_or_else(|| AppError::Config("library.targetUnsupported".into()))?;
        let Some(text) = crate::services::library::coordinator::read(&path)? else {
            return Ok(0);
        };
        crate::services::library::mcp_codec::parse_native(&text, tool)?
    };
    let Some(native) = root
        .get(crate::services::library::mcp_codec::section(tool))
        .and_then(Value::as_object)
    else {
        return Ok(0);
    };
    let servers = config.mcp.servers.get_or_insert_with(Default::default);
    let mut count = 0;
    for (id, spec) in native {
        if servers.contains_key(id) {
            continue;
        }
        let server = crate::services::library::mcp_codec::canonical_spec(tool, spec)?;
        servers.insert(
            id.clone(),
            McpServer {
                id: id.clone(),
                name: id.clone(),
                server,
                apps: McpApps::default(),
                description: None,
                homepage: None,
                docs: None,
                tags: Vec::new(),
            },
        );
        count += 1;
    }
    Ok(count)
}
macro_rules! legacy_api {
    ($tool:literal,$import:ident,$single:ident,$remove:ident) => {
        pub fn $import(config: &mut MultiAppConfig) -> Result<usize, AppError> {
            legacy_import(config, $tool)
        }
        pub fn $single(_config: &MultiAppConfig, _id: &str, _spec: &Value) -> Result<(), AppError> {
            Err(AppError::Config("library.previewRequired".into()))
        }
        pub fn $remove(_id: &str) -> Result<(), AppError> {
            Err(AppError::Config("library.previewRequired".into()))
        }
    };
}
legacy_api!(
    "claude",
    import_from_claude,
    sync_single_server_to_claude,
    remove_server_from_claude
);
legacy_api!(
    "codex",
    import_from_codex,
    sync_single_server_to_codex,
    remove_server_from_codex
);
legacy_api!(
    "gemini",
    import_from_gemini,
    sync_single_server_to_gemini,
    remove_server_from_gemini
);
legacy_api!(
    "opencode",
    import_from_opencode,
    sync_single_server_to_opencode,
    remove_server_from_opencode
);
legacy_api!(
    "hermes",
    import_from_hermes,
    sync_single_server_to_hermes,
    remove_server_from_hermes
);
pub fn sync_enabled_to_claude(_config: &MultiAppConfig) -> Result<(), AppError> {
    Err(AppError::Config("library.previewRequired".into()))
}
pub fn sync_enabled_to_codex(_config: &MultiAppConfig) -> Result<(), AppError> {
    Err(AppError::Config("library.previewRequired".into()))
}
pub fn sync_enabled_to_gemini(_config: &MultiAppConfig) -> Result<(), AppError> {
    Err(AppError::Config("library.previewRequired".into()))
}
pub fn strip_ignored_type_from_codex_config() -> Result<bool, AppError> {
    Err(AppError::Config("library.previewRequired".into()))
}

#[cfg(test)]
#[allow(dead_code)] // Retained legacy conversion fixtures; no production writer.
mod claude;
#[cfg(test)]
#[allow(dead_code)] // Retained legacy conversion fixtures; no production writer.
mod codex;
#[cfg(test)]
#[allow(dead_code)] // Retained legacy conversion fixtures; no production writer.
mod gemini;
#[cfg(test)]
#[allow(dead_code)] // Retained legacy conversion fixtures; no production writer.
mod hermes;
#[cfg(test)]
#[allow(dead_code)] // Retained legacy conversion fixtures; no production writer.
mod opencode;
#[cfg(test)]
#[allow(dead_code)] // Retained legacy conversion fixtures; no production writer.
mod validation;

#[cfg(test)]
mod library_barrier_tests {
    use super::*;
    use serde_json::json;
    #[test]
    #[serial_test::serial]
    fn legacy_native_writers_require_library_confirmation() {
        let _sandbox = crate::test_sandbox::sandbox();
        let paths = crate::services::library::mcp_resource::native_paths().unwrap();
        for (tool, path) in &paths {
            let text = if tool == "codex" {
                "model = \"personal\"\n"
            } else if tool == "hermes" {
                "model: personal\n"
            } else {
                "{\"personal\":true}"
            };
            crate::config::atomic_write(path, text.as_bytes()).unwrap();
        }
        let original: Vec<_> = paths
            .values()
            .map(|path| std::fs::read_to_string(path).unwrap())
            .collect();
        let config = crate::app_config::MultiAppConfig::default();
        type Writer = fn(
            &crate::app_config::MultiAppConfig,
            &str,
            &serde_json::Value,
        ) -> Result<(), crate::error::AppError>;
        for writer in [
            sync_single_server_to_claude as Writer,
            sync_single_server_to_codex,
            sync_single_server_to_gemini,
            sync_single_server_to_opencode,
            sync_single_server_to_hermes,
        ] {
            assert!(
                writer(&config, "demo", &json!({"type":"stdio","command":"server"}))
                    .unwrap_err()
                    .to_string()
                    .contains("previewRequired")
            );
        }
        for remove in [
            remove_server_from_claude,
            remove_server_from_codex,
            remove_server_from_gemini,
            remove_server_from_opencode,
            remove_server_from_hermes,
        ] {
            assert!(remove("demo")
                .unwrap_err()
                .to_string()
                .contains("previewRequired"));
        }
        for batch in [
            sync_enabled_to_claude,
            sync_enabled_to_codex,
            sync_enabled_to_gemini,
        ] {
            assert!(batch(&config)
                .unwrap_err()
                .to_string()
                .contains("previewRequired"));
        }
        assert_eq!(
            paths
                .values()
                .map(|path| std::fs::read_to_string(path).unwrap())
                .collect::<Vec<_>>(),
            original
        );
    }
}
