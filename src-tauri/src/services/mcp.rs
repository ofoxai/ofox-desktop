use crate::{
    app_config::{AppType, McpApps, McpServer},
    error::AppError,
    services::library::{mcp_codec, mcp_resource},
    store::AppState,
};
use indexmap::IndexMap;
use serde_json::Value;
use std::collections::HashMap;

/// Legacy entry points can save drafts; native writes require a revisioned library preview.
pub struct McpService;
impl McpService {
    pub fn get_all_servers(state: &AppState) -> Result<IndexMap<String, McpServer>, AppError> {
        state.db.get_all_mcp_servers()
    }
    pub fn upsert_server(state: &AppState, server: McpServer) -> Result<(), AppError> {
        let _guard = crate::config_io::lock();
        if state.db.get_all_mcp_servers()?.contains_key(&server.id) {
            return Err(AppError::Config("library.revisionRequired".into()));
        }
        mcp_resource::save_draft(&state.db, server, None).map(|_| ())
    }
    pub fn delete_server(state: &AppState, id: &str) -> Result<bool, AppError> {
        mcp_resource::delete_draft(&state.db, id)
    }
    pub fn toggle_app(
        _state: &AppState,
        _id: &str,
        _app: AppType,
        _enabled: bool,
    ) -> Result<(), AppError> {
        Err(AppError::Config("library.previewRequired".into()))
    }
    /// Model switching, startup and restore may inspect resource state, never project legacy flags.
    pub fn sync_all_enabled(state: &AppState) -> Result<(), AppError> {
        state.db.library_snapshot().map(|_| ())
    }
    #[deprecated(since = "3.7.0", note = "Use library definition views")]
    pub fn get_servers(state: &AppState, app: AppType) -> Result<HashMap<String, Value>, AppError> {
        let snapshot = state.db.library_snapshot()?;
        Ok(Self::get_all_servers(state)?
            .into_iter()
            .filter(|(id, _)| {
                snapshot
                    .machine
                    .assignments
                    .contains_key(&format!("mcp:{id}:{}", app.as_str()))
            })
            .map(|(id, server)| (id, mcp_resource::definition_view(&server, &snapshot)))
            .collect())
    }
    #[deprecated(since = "3.7.0", note = "Use library preview/apply")]
    pub fn set_enabled(
        state: &AppState,
        app: AppType,
        id: &str,
        enabled: bool,
    ) -> Result<bool, AppError> {
        Self::toggle_app(state, id, app, enabled).map(|_| true)
    }
    #[deprecated(since = "3.7.0", note = "Resource application is explicit")]
    pub fn sync_enabled(state: &AppState, _app: AppType) -> Result<(), AppError> {
        Self::sync_all_enabled(state)
    }
    fn import_native(state: &AppState, tool: &str) -> Result<usize, AppError> {
        let _guard = crate::config_io::lock();
        let root = if tool == "opencode" {
            crate::opencode_config::read_opencode_config()?
        } else {
            let path = mcp_resource::native_paths()?
                .remove(tool)
                .ok_or_else(|| AppError::Config("library.targetUnsupported".into()))?;
            let Some(text) = super::library::coordinator::read(&path)? else {
                return Ok(0);
            };
            mcp_codec::parse_native(&text, tool)?
        };
        let Some(servers) = root
            .get(mcp_codec::section(tool))
            .and_then(Value::as_object)
        else {
            return Ok(0);
        };
        let mut existing = state.db.get_all_mcp_servers()?;
        let mut count = 0;
        for (id, native) in servers {
            let spec = mcp_codec::canonical_spec(tool, native)?;
            let hash = mcp_resource::match_hash(&spec)?;
            let id = if let Some(old) = existing.get(id) {
                if mcp_resource::match_hash(&old.server)? == hash {
                    continue;
                }
                format!("{id}-import-{}", &hash[..8])
            } else {
                id.clone()
            };
            if let Some(old) = existing.get(&id) {
                if mcp_resource::match_hash(&old.server)? == hash {
                    continue;
                }
                return Err(AppError::Config("library.importNameConflict".into()));
            }
            let server = McpServer {
                id: id.clone(),
                name: id.clone(),
                server: spec,
                apps: McpApps::default(),
                description: None,
                homepage: None,
                docs: None,
                tags: vec!["imported".into()],
            };
            mcp_resource::save_draft(&state.db, server.clone(), None)?;
            existing.insert(id, server);
            count += 1;
        }
        Ok(count)
    }
    pub fn import_from_claude(state: &AppState) -> Result<usize, AppError> {
        Self::import_native(state, "claude")
    }
    pub fn import_from_codex(state: &AppState) -> Result<usize, AppError> {
        Self::import_native(state, "codex")
    }
    pub fn import_from_gemini(state: &AppState) -> Result<usize, AppError> {
        Self::import_native(state, "gemini")
    }
    pub fn import_from_opencode(state: &AppState) -> Result<usize, AppError> {
        Self::import_native(state, "opencode")
    }
    pub fn import_from_hermes(state: &AppState) -> Result<usize, AppError> {
        Self::import_native(state, "hermes")
    }
}

#[cfg(test)]
mod library_regression_tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;
    use std::sync::Arc;
    fn fixture() -> McpServer {
        McpServer {
            id: "demo".into(),
            name: "Demo".into(),
            server: json!({"command":"server"}),
            apps: Default::default(),
            description: None,
            homepage: None,
            docs: None,
            tags: vec![],
        }
    }
    #[test]
    #[serial]
    fn opencode_import_uses_the_effective_merged_global_configuration() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let dir = crate::opencode_config::get_opencode_dir();
        crate::config::atomic_write(&dir.join("config.json"), br#"{"mcp":{"demo":{"type":"local","command":["server"],"environment":{"A":"${A}"}},"lower":{"type":"local","command":["other"]}}}"#).unwrap();
        crate::config::atomic_write(
            &dir.join("opencode.json"),
            br#"{"mcp":{"demo":{"environment":{"B":"{env:B}"}}}}"#,
        )
        .unwrap();
        crate::config::atomic_write(&dir.join("opencode.jsonc"), b"{}").unwrap();
        assert_eq!(McpService::import_from_opencode(&state).unwrap(), 2);
        let servers = state.db.get_all_mcp_servers().unwrap();
        assert_eq!(servers["demo"].server["env"]["B"], "${B}");
        assert_eq!(servers["demo"].server["command"], "server");
        assert!(servers["demo"].apps.enabled_apps().is_empty());
        assert_eq!(McpService::import_from_opencode(&state).unwrap(), 0);
    }

    #[test]
    #[serial]
    fn repeated_deeplink_import_does_not_change_an_existing_definition_revision() {
        use base64::Engine;
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        McpService::upsert_server(&state, fixture()).unwrap();
        let before = state.db.library_snapshot().unwrap();
        let request = crate::deeplink::DeepLinkImportRequest {
            resource: "mcp".into(),
            apps: Some("claude".into()),
            config: Some(
                base64::engine::general_purpose::STANDARD
                    .encode(r#"{"mcpServers":{"demo":{"command":"server"}}}"#),
            ),
            ..Default::default()
        };
        crate::deeplink::import_mcp_from_deeplink(&state, request).unwrap();
        assert_eq!(state.db.library_snapshot().unwrap(), before);
    }

    #[test]
    #[serial]
    fn saving_a_definition_does_not_implicitly_assign_or_write() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let path = crate::config::get_claude_mcp_path();
        crate::config::atomic_write(&path, b"{\"personal\":true}").unwrap();
        let mut server = fixture();
        server.apps.claude = true;
        McpService::upsert_server(&state, server).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"personal\":true}"
        );
        assert!(state
            .db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
        assert!(state.db.get_all_mcp_servers().unwrap()["demo"]
            .apps
            .enabled_apps()
            .is_empty());
    }
    #[test]
    #[serial]
    fn background_reconcile_never_deletes_an_unowned_same_name() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let path = crate::config::get_claude_mcp_path();
        let original = r#"{"mcpServers":{"demo":{"command":"personal"}}}"#;
        crate::config::atomic_write(&path, original.as_bytes()).unwrap();
        state.db.save_mcp_server(&fixture()).unwrap();
        McpService::sync_all_enabled(&state).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
    #[test]
    #[serial]
    fn importing_native_servers_does_not_assign_or_modify_native_files() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let path = crate::config::get_claude_mcp_path();
        let original = r#"{"mcpServers":{"demo":{"command":"personal"}}}"#;
        crate::config::atomic_write(&path, original.as_bytes()).unwrap();
        assert_eq!(McpService::import_from_claude(&state).unwrap(), 1);
        assert!(state.db.get_all_mcp_servers().unwrap()["demo"]
            .apps
            .enabled_apps()
            .is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(McpService::import_from_claude(&state).unwrap(), 0);
    }
}
