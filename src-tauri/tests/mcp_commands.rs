use std::fs;

use serde_json::json;

use cc_switch_lib::{
    get_claude_mcp_path, get_claude_settings_path, import_default_config_test_hook, AppError,
    AppType, McpApps, McpServer, McpService, MultiAppConfig,
};

#[path = "support.rs"]
mod support;
use support::{create_test_state, create_test_state_with_config, sandbox};

#[test]
#[serial_test::serial]
fn import_default_config_claude_persists_provider() {
    let sandbox = sandbox();
    let home = sandbox.home();

    let settings_path = get_claude_settings_path();
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).expect("create claude settings dir");
    }
    let settings = json!({
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "test-key",
            "ANTHROPIC_BASE_URL": "https://api.test"
        }
    });
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings).expect("serialize settings"),
    )
    .expect("seed claude settings.json");

    let mut config = MultiAppConfig::default();
    config.ensure_app(&AppType::Claude);
    let state = create_test_state_with_config(&config).expect("create test state");

    import_default_config_test_hook(&state, AppType::Claude)
        .expect("import default config succeeds");

    // 验证内存状态
    let providers = state
        .db
        .get_all_providers(AppType::Claude.as_str())
        .expect("get all providers");
    let current_id = state
        .db
        .get_current_provider(AppType::Claude.as_str())
        .expect("get current provider");
    assert_eq!(current_id.as_deref(), Some("default"));
    let default_provider = providers.get("default").expect("default provider");
    assert_eq!(
        default_provider.settings_config, settings,
        "default provider should capture live settings"
    );

    // 验证数据已持久化到数据库（v3.7.0+ 使用 SQLite 而非 config.json）
    let db_path = home.join(".ofox-desktop").join("ofox-desktop.db");
    assert!(
        db_path.exists(),
        "importing default config should persist to ofox-desktop.db"
    );
}

#[test]
#[serial_test::serial]
fn import_default_config_without_live_file_returns_error() {
    use support::create_test_state;

    let _sandbox = sandbox();

    let state = create_test_state().expect("create test state");

    let err = import_default_config_test_hook(&state, AppType::Claude)
        .expect_err("missing live file should error");
    match err {
        AppError::Localized { zh, .. } => assert!(
            zh.contains("Claude Code 配置文件不存在"),
            "unexpected error message: {zh}"
        ),
        AppError::Message(msg) => assert!(
            msg.contains("Claude Code 配置文件不存在"),
            "unexpected error message: {msg}"
        ),
        other => panic!("unexpected error variant: {other:?}"),
    }

    // 使用数据库架构，不再检查 config.json
    // 失败的导入不应该向数据库写入任何供应商
    let providers = state
        .db
        .get_all_providers(AppType::Claude.as_str())
        .expect("get all providers");
    assert!(
        providers.is_empty(),
        "failed import should not create any providers in database"
    );
}

fn server(id: &str) -> McpServer {
    McpServer {
        id: id.into(),
        name: id.into(),
        server: json!({"type":"stdio","command":"echo"}),
        apps: McpApps::default(),
        description: None,
        homepage: None,
        docs: None,
        tags: Vec::new(),
    }
}
fn write(path: &std::path::Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
#[serial_test::serial]
fn importing_claude_creates_a_persistent_draft_without_changing_native_files() {
    let _sandbox = sandbox();
    let state = create_test_state().unwrap();
    let path = get_claude_mcp_path();
    let original = r#"{"personal":true,"mcpServers":{"echo":{"type":"stdio","command":"echo"}}}"#;
    write(&path, original);
    assert_eq!(McpService::import_from_claude(&state).unwrap(), 1);
    let definitions = state.db.get_all_mcp_servers().unwrap();
    assert!(definitions["echo"].apps.enabled_apps().is_empty());
    assert_eq!(definitions["echo"].server["command"], "echo");
    assert!(state
        .db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(McpService::import_from_claude(&state).unwrap(), 0);
}

#[test]
#[serial_test::serial]
fn malformed_native_import_keeps_database_and_file_unchanged() {
    let _sandbox = sandbox();
    let state = create_test_state().unwrap();
    let path = get_claude_mcp_path();
    let original = r#"{"mcpServers":"#;
    write(&path, original);
    let error = McpService::import_from_claude(&state).unwrap_err();
    assert!(error.to_string().contains("library.nativeInvalid"));
    assert!(state.db.get_all_mcp_servers().unwrap().is_empty());
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[test]
#[serial_test::serial]
fn matching_multi_tool_imports_remain_one_unassigned_definition() {
    let _sandbox = sandbox();
    let state = create_test_state().unwrap();
    let claude = get_claude_mcp_path();
    let codex = cc_switch_lib::get_codex_config_path();
    let c = r#"{"mcpServers":{"echo":{"type":"stdio","command":"echo"}}}"#;
    let x = "[mcp_servers.echo]\ncommand = \"echo\"\n";
    write(&claude, c);
    write(&codex, x);
    assert_eq!(McpService::import_from_claude(&state).unwrap(), 1);
    assert_eq!(McpService::import_from_codex(&state).unwrap(), 0);
    let definitions = state.db.get_all_mcp_servers().unwrap();
    assert_eq!(definitions.len(), 1);
    assert!(definitions["echo"].apps.enabled_apps().is_empty());
    assert_eq!(fs::read_to_string(claude).unwrap(), c);
    assert_eq!(fs::read_to_string(codex).unwrap(), x);
}

#[test]
#[serial_test::serial]
fn gemini_import_respects_native_http_and_explicit_sse_transports() {
    let _sandbox = sandbox();
    let state = create_test_state().unwrap();
    let path = cc_switch_lib::get_gemini_settings_path();
    let original = r#"{"mcpServers":{"http":{"url":"https://example.org/mcp"},"sse":{"type":"sse","url":"https://example.org/sse"}}}"#;
    write(&path, original);
    assert_eq!(McpService::import_from_gemini(&state).unwrap(), 2);
    let definitions = state.db.get_all_mcp_servers().unwrap();
    assert_eq!(definitions["http"].server["type"], "http");
    assert_eq!(definitions["sse"].server["type"], "sse");
    assert!(definitions
        .values()
        .all(|server| server.apps.enabled_apps().is_empty()));
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[test]
#[serial_test::serial]
fn legacy_toggle_never_initializes_any_native_directory() {
    let _sandbox = sandbox();
    let state = create_test_state().unwrap();
    McpService::upsert_server(&state, server("echo")).unwrap();
    for (app, path) in [
        (AppType::Claude, get_claude_mcp_path()),
        (AppType::Codex, cc_switch_lib::get_codex_config_path()),
        (AppType::Gemini, cc_switch_lib::get_gemini_settings_path()),
    ] {
        assert!(!path.exists());
        assert!(McpService::toggle_app(&state, "echo", app, true)
            .unwrap_err()
            .to_string()
            .contains("library.previewRequired"));
        assert!(!path.exists());
    }
    assert!(state
        .db
        .library_snapshot()
        .unwrap()
        .machine
        .assignments
        .is_empty());
}

#[test]
#[serial_test::serial]
fn an_explicit_preview_can_initialize_native_config_and_withdraw_only_its_fields() {
    let _sandbox = sandbox();
    let state = create_test_state().unwrap();
    let path = cc_switch_lib::get_codex_config_path();
    write(&path, "# personal\nmodel = \"personal\"\n");
    cc_switch_lib::save_mcp_draft(&state.db, server("echo"), None).unwrap();
    let op = cc_switch_lib::preview_mcp_resource(&state.db, "echo", &["codex".into()]).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "# personal\nmodel = \"personal\"\n"
    );
    assert_eq!(
        cc_switch_lib::apply_resource_operation(&state.db, &op.id)
            .unwrap()
            .status,
        "applied"
    );
    assert!(fs::read_to_string(&path)
        .unwrap()
        .contains("[mcp_servers.echo]"));
    assert!(cc_switch_lib::delete_mcp_draft(&state.db, "echo")
        .unwrap_err()
        .to_string()
        .contains("withdrawRequired"));
    let withdraw = cc_switch_lib::preview_mcp_resource(&state.db, "echo", &[]).unwrap();
    assert_eq!(
        cc_switch_lib::apply_resource_operation(&state.db, &withdraw.id)
            .unwrap()
            .status,
        "applied"
    );
    assert!(fs::read_to_string(&path).unwrap().contains("# personal"));
    assert!(!fs::read_to_string(&path)
        .unwrap()
        .contains("[mcp_servers.echo]"));
    assert!(cc_switch_lib::delete_mcp_draft(&state.db, "echo").unwrap());
}

#[test]
#[serial_test::serial]
fn legacy_upsert_cannot_replace_an_existing_definition_or_remove_a_user_server() {
    let _sandbox = sandbox();
    let state = create_test_state().unwrap();
    let path = get_claude_mcp_path();
    let original = r#"{"mcpServers":{"echo":{"command":"personal"}}}"#;
    write(&path, original);
    let mut definition = server("echo");
    definition.apps.claude = true;
    McpService::upsert_server(&state, definition.clone()).unwrap();
    assert!(state.db.get_all_mcp_servers().unwrap()["echo"]
        .apps
        .enabled_apps()
        .is_empty());
    definition.apps.claude = false;
    assert!(McpService::upsert_server(&state, definition)
        .unwrap_err()
        .to_string()
        .contains("revisionRequired"));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(McpService::delete_server(&state, "echo").unwrap());
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[test]
#[serial_test::serial]
fn background_sync_never_projects_legacy_flags_or_deletes_known_names() {
    let _sandbox = sandbox();
    let state = create_test_state().unwrap();
    let path = get_claude_mcp_path();
    let original = r#"{"mcpServers":{"managed-disabled":{"command":"user"},"external-only":{"command":"mine"}}}"#;
    write(&path, original);
    let disabled = server("managed-disabled");
    state.db.save_mcp_server(&disabled).unwrap();
    let mut enabled = server("managed-enabled");
    enabled.apps.claude = true;
    state.db.save_mcp_server(&enabled).unwrap();
    McpService::sync_all_enabled(&state).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    assert!(state
        .db
        .library_snapshot()
        .unwrap()
        .machine
        .receipts
        .is_empty());
}
