//! 绑定 / 解绑的端到端测试：临时 HOME 下的真实文件 + 内存数据库。

use super::*;
use crate::provider::Provider;
use serde_json::json;
use serial_test::serial;
use std::fs;

const TEMPLATE: &str = "model_provider = \"ofox\"\nmodel = \"openai/gpt-6-luna\"\nmodel_reasoning_effort = \"high\"\ndisable_response_storage = true\n\n[model_providers.ofox]\nname = \"ofox\"\nbase_url = \"https://api.ofox.ai/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n";
const OAUTH_AUTH: &str = "{\n  \"auth_mode\": \"chatgpt\",\n  \"OPENAI_API_KEY\": null,\n  \"tokens\": {\n    \"id_token\": \"eyJid\",\n    \"access_token\": \"eyJaccess\",\n    \"refresh_token\": \"rt\",\n    \"account_id\": \"acct\"\n  },\n  \"last_refresh\": \"2026-09-30T00:00:00Z\"\n}\n";
const USER_CONFIG: &str = "# my codex setup\nmodel = \"gpt-5-codex\"\nmodel_reasoning_effort = \"medium\"\n\n[projects.\"/work\"]\ntrust_level = \"trusted\"\n\n[mcp_servers.docs]\ncommand = \"docs-mcp\"\n";
const KEY: &str = "sk-of-TEST";
const CODEX_RECORD: &str = "codex";
const CODEX_PROVIDER: &str = "ofox-codex";
const CODEX_OFFICIAL: &str = "codex-official";

/// 临时 HOME：HOME / USERPROFILE / CC_SWITCH_TEST_HOME 都指过去，Drop 时还原。
struct Home {
    dir: tempfile::TempDir,
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp home");
        let saved = ["HOME", "USERPROFILE", "CC_SWITCH_TEST_HOME"]
            .into_iter()
            .map(|name| {
                let previous = std::env::var_os(name);
                std::env::set_var(name, dir.path());
                (name, previous)
            })
            .collect();
        crate::settings::reload_settings().expect("reload settings");
        Self { dir, saved }
    }

    fn codex(&self, file: &str) -> PathBuf {
        self.dir.path().join(".codex").join(file)
    }

    fn claude(&self, file: &str) -> PathBuf {
        self.dir.path().join(".claude").join(file)
    }

    fn gemini(&self, file: &str) -> PathBuf {
        self.dir.path().join(".gemini").join(file)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        for (name, previous) in self.saved.drain(..) {
            match previous {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        let _ = crate::settings::reload_settings();
    }
}

fn save_template(db: &Database, config: &str) {
    db.save_provider(
        "codex",
        &Provider::with_id(
            CODEX_PROVIDER.into(),
            "OfoxAI".into(),
            json!({ "auth": { "OPENAI_API_KEY": "" }, "config": config }),
            None,
        ),
    )
    .expect("save template");
}

fn db_with_default_provider() -> Database {
    let db = Database::memory().expect("db");
    save_template(&db, TEMPLATE);
    db.save_provider(
        "codex",
        &Provider::with_id("default".into(), "Default".into(), json!({}), None),
    )
    .expect("save default");
    db.save_provider(
        "codex",
        &Provider::with_id(CODEX_OFFICIAL.into(), "OpenAI".into(), json!({}), None),
    )
    .expect("save official");
    set_current_provider(&db, &AppType::Codex, "default").expect("current");
    db
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn toml_at(path: &Path) -> toml::Table {
    toml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[tokio::test]
#[serial]
async fn chatgpt_oauth_login_is_untouched_and_config_restored_byte_for_byte() {
    let home = Home::new();
    let (auth, config) = (home.codex("auth.json"), home.codex("config.toml"));
    write(&auth, OAUTH_AUTH);
    write(&config, USER_CONFIG);
    crate::config::set_file_mode(&config, 0o644).unwrap();
    let db = db_with_default_provider();

    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");
    assert_eq!(fs::read_to_string(&auth).unwrap(), OAUTH_AUTH);
    let bound = toml_at(&config);
    let provider = &bound["model_providers"]["ofox"];
    assert_eq!(provider["experimental_bearer_token"].as_str(), Some(KEY));
    assert_eq!(provider["requires_openai_auth"].as_bool(), Some(false));
    #[cfg(unix)]
    assert_eq!(crate::config::file_mode(&config), Some(0o600));
    assert_eq!(
        db.get_current_provider("codex").unwrap().as_deref(),
        Some(CODEX_PROVIDER)
    );

    // 绑定期间切换模型（DB 模板变了，再按模板重写）。
    save_template(
        &db,
        &TEMPLATE.replace("openai/gpt-6-luna", "anthropic/claude-x"),
    );
    rewrite_bound_config(&db, Tool::Codex, KEY)
        .await
        .expect("model change");
    assert_eq!(
        toml_at(&config)["model"].as_str(),
        Some("anthropic/claude-x")
    );

    let report = unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind");
    assert_eq!(fs::read_to_string(&config).unwrap(), USER_CONFIG);
    assert_eq!(fs::read_to_string(&auth).unwrap(), OAUTH_AUTH);
    #[cfg(unix)]
    assert_eq!(crate::config::file_mode(&config), Some(0o644));
    assert_eq!(report.exact_files.len(), 1);
    assert!(!report.legacy);
    assert_eq!(report.provider_restored_to.as_deref(), Some("default"));
    assert_eq!(
        db.get_current_provider("codex").unwrap().as_deref(),
        Some("default")
    );
    assert_eq!(
        crate::settings::get_current_provider(&AppType::Codex).as_deref(),
        Some("default")
    );
    assert!(db.get_bind_record(CODEX_RECORD).unwrap().is_none());
}

#[tokio::test]
#[serial]
async fn bind_never_creates_auth_json_and_unbind_removes_what_bind_created() {
    let home = Home::new();
    let db = db_with_default_provider();

    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");
    assert!(!home.codex("auth.json").exists());
    assert!(home.codex("config.toml").exists());

    let report = unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind");
    assert!(!home.codex("config.toml").exists());
    assert!(!home.dir.path().join(".codex").exists());
    assert_eq!(report.files_removed.len(), 1);
}

#[tokio::test]
#[serial]
async fn managed_keys_revert_even_if_edited_while_bound_and_new_mcp_is_kept() {
    let home = Home::new();
    let config = home.codex("config.toml");
    write(&config, USER_CONFIG);
    let db = db_with_default_provider();
    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");

    let edited = fs::read_to_string(&config).unwrap().replace(
        "model_reasoning_effort = \"high\"",
        "model_reasoning_effort = \"low\"",
    ) + "\n[mcp_servers.added]\ncommand = \"x\"\n";
    write(&config, &edited);
    // 再绑一次（添加工具会重绑所有工具）：快照不能被覆盖。
    bind(&db, Tool::Codex, "codex", KEY).await.expect("rebind");

    unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind");
    let restored = toml_at(&config);
    assert_eq!(restored["model"].as_str(), Some("gpt-5-codex"));
    assert_eq!(restored["model_reasoning_effort"].as_str(), Some("medium"));
    assert!(restored.get("model_provider").is_none());
    assert!(restored.get("model_providers").is_none());
    assert_eq!(
        restored["mcp_servers"]["added"]["command"].as_str(),
        Some("x")
    );
    assert_eq!(
        restored["mcp_servers"]["docs"]["command"].as_str(),
        Some("docs-mcp")
    );
}

#[tokio::test]
#[serial]
async fn unbind_is_idempotent() {
    let home = Home::new();
    write(&home.codex("config.toml"), USER_CONFIG);
    let db = db_with_default_provider();
    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");
    unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind");

    let again = unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind again");
    assert!(again.already_unbound);
    assert_eq!(
        fs::read_to_string(home.codex("config.toml")).unwrap(),
        USER_CONFIG
    );
}

#[tokio::test]
#[serial]
async fn failed_rebind_keeps_the_existing_binding() {
    let home = Home::new();
    write(&home.codex("config.toml"), USER_CONFIG);
    let db = db_with_default_provider();
    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");
    let bound = fs::read_to_string(home.codex("config.toml")).unwrap();

    db.delete_provider("codex", CODEX_PROVIDER)
        .expect("drop template");
    assert!(bind(&db, Tool::Codex, "codex", KEY).await.is_err());

    assert_eq!(
        fs::read_to_string(home.codex("config.toml")).unwrap(),
        bound
    );
    assert!(db.get_bind_record(CODEX_RECORD).unwrap().is_some());
}

#[tokio::test]
#[serial]
async fn shared_codex_config_is_restored_only_after_the_last_holder_unbinds() {
    let home = Home::new();
    write(&home.codex("config.toml"), USER_CONFIG);
    let db = db_with_default_provider();
    bind(&db, Tool::Codex, "codex", KEY)
        .await
        .expect("bind codex");
    bind(&db, Tool::Codex, "chatgpt", KEY)
        .await
        .expect("bind chatgpt");

    let first = unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind codex");
    assert_eq!(first.shared_kept_by, ["chatgpt"]);
    assert!(codex::is_ofox_bound(
        &fs::read_to_string(home.codex("config.toml")).unwrap()
    ));

    unbind(&db, Tool::Codex, "chatgpt", &[], false)
        .await
        .expect("unbind chatgpt");
    assert_eq!(
        fs::read_to_string(home.codex("config.toml")).unwrap(),
        USER_CONFIG
    );
}

#[tokio::test]
#[serial]
async fn still_bound_hint_keeps_config_for_frontend_only_chatgpt_binding() {
    let home = Home::new();
    write(&home.codex("config.toml"), USER_CONFIG);
    let db = db_with_default_provider();
    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");

    let report = unbind(&db, Tool::Codex, "codex", &["chatgpt".to_string()], false)
        .await
        .expect("unbind");
    assert_eq!(report.shared_kept_by, ["chatgpt"]);
    assert!(codex::is_ofox_bound(
        &fs::read_to_string(home.codex("config.toml")).unwrap()
    ));

    unbind(&db, Tool::Codex, "chatgpt", &[], false)
        .await
        .expect("unbind chatgpt");
    assert_eq!(
        fs::read_to_string(home.codex("config.toml")).unwrap(),
        USER_CONFIG
    );
}

#[tokio::test]
#[serial]
async fn preview_reports_the_restore_without_changing_anything() {
    let home = Home::new();
    write(&home.codex("config.toml"), USER_CONFIG);
    let db = db_with_default_provider();
    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");
    let bound = fs::read_to_string(home.codex("config.toml")).unwrap();

    let preview = unbind(&db, Tool::Codex, "codex", &[], true)
        .await
        .expect("preview");
    assert!(preview.dry_run);
    assert_eq!(preview.exact_files.len(), 1);
    assert_eq!(
        fs::read_to_string(home.codex("config.toml")).unwrap(),
        bound
    );
    assert!(db.get_bind_record(CODEX_RECORD).unwrap().is_some());
}

#[tokio::test]
#[serial]
async fn legacy_bound_install_is_migrated_adopted_and_cleaned_up() {
    let home = Home::new();
    let (auth, config) = (home.codex("auth.json"), home.codex("config.toml"));
    // 旧版本绑定后的样子：Ofox 的顶层字段在前，用户自己的表在后。
    let old_shape = format!("{TEMPLATE}\n[mcp_servers.docs]\ncommand = \"docs-mcp\"\n");
    write(&config, &old_shape);
    write(
        &auth,
        &OAUTH_AUTH.replace(
            "\"OPENAI_API_KEY\": null",
            "\"OPENAI_API_KEY\": \"sk-of-OLD\"",
        ),
    );
    let db = db_with_default_provider();

    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");
    let auth_after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&auth).unwrap()).unwrap();
    assert!(auth_after["OPENAI_API_KEY"].is_null());
    assert_eq!(auth_after["tokens"]["access_token"], "eyJaccess");
    let record = parse_record(&db.get_bind_record(CODEX_RECORD).unwrap().unwrap().record).unwrap();
    assert!(matches!(record, StoredRecord::Envelope(ref e) if e.kind == RecordKind::LegacyAdopted));

    let report = unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind");
    assert!(report.legacy);
    let cleaned = toml_at(&config);
    assert!(cleaned.get("model_provider").is_none());
    assert!(cleaned.get("model").is_none());
    assert!(cleaned.get("model_providers").is_none());
    assert_eq!(
        cleaned["mcp_servers"]["docs"]["command"].as_str(),
        Some("docs-mcp")
    );
    assert_eq!(
        db.get_current_provider("codex").unwrap().as_deref(),
        Some(CODEX_OFFICIAL)
    );
}

#[tokio::test]
#[serial]
async fn legacy_migration_moves_the_key_once_and_ignores_user_keys() {
    let home = Home::new();
    let (auth, config) = (home.codex("auth.json"), home.codex("config.toml"));
    write(&config, TEMPLATE);
    write(&auth, "{\"OPENAI_API_KEY\": \"sk-proj-mine\"}");
    assert!(
        !codex::migrate_legacy_shape(|| None).unwrap(),
        "no Ofox key anywhere"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), TEMPLATE);

    write(
        &auth,
        &OAUTH_AUTH.replace(
            "\"OPENAI_API_KEY\": null",
            "\"OPENAI_API_KEY\": \"sk-of-OLD\"",
        ),
    );
    assert!(codex::migrate_legacy_shape(|| None).unwrap());
    assert_eq!(
        toml_at(&config)["model_providers"]["ofox"]["experimental_bearer_token"].as_str(),
        Some("sk-of-OLD")
    );
    assert!(
        !codex::migrate_legacy_shape(|| None).unwrap(),
        "second run is a no-op"
    );
}

#[tokio::test]
#[serial]
async fn legacy_migration_reads_the_keychain_only_when_it_has_to() {
    // 读钥匙串可能弹系统授权窗口：不需要迁移、或 auth.json 里已有 key 时都不能读。
    let home = Home::new();
    let (auth, config) = (home.codex("auth.json"), home.codex("config.toml"));
    let keychain = || -> Option<String> { panic!("keychain must not be read") };

    write(&config, USER_CONFIG);
    assert!(!codex::migrate_legacy_shape(keychain).unwrap());

    write(&config, TEMPLATE);
    write(&auth, "{\"OPENAI_API_KEY\": \"sk-of-OLD\"}");
    assert!(codex::migrate_legacy_shape(keychain).unwrap());
    assert!(!auth.exists(), "auth.json held only the Ofox key");

    let mut asked = false;
    write(&config, TEMPLATE);
    assert!(codex::migrate_legacy_shape(|| {
        asked = true;
        Some("sk-of-STORED".to_string())
    })
    .unwrap());
    assert!(
        asked,
        "falls back to the stored key when auth.json has none"
    );
}

#[tokio::test]
#[serial]
async fn template_sync_paths_never_write_the_ofox_provider_to_disk() {
    let home = Home::new();
    write(&home.codex("auth.json"), OAUTH_AUTH);
    let db = db_with_default_provider();
    let template = db
        .get_provider_by_id(CODEX_PROVIDER, "codex")
        .unwrap()
        .unwrap();
    crate::services::provider::write_live_with_common_config(&db, &AppType::Codex, &template)
        .expect("sync");
    assert_eq!(
        fs::read_to_string(home.codex("auth.json")).unwrap(),
        OAUTH_AUTH
    );
    assert!(!home.codex("config.toml").exists());
}

#[tokio::test]
#[serial]
async fn unbind_turns_off_the_proxy_flag() {
    let home = Home::new();
    write(&home.codex("config.toml"), USER_CONFIG);
    let db = db_with_default_provider();
    let mut proxy = db.get_proxy_config_for_app("codex").await.unwrap();
    proxy.enabled = true;
    db.update_proxy_config_for_app(proxy).await.unwrap();
    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");

    unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind");
    assert!(!db.get_proxy_config_for_app("codex").await.unwrap().enabled);
    drop(home);
}

// ─── Claude / Gemini ────────────────────────────────────────────────────────

const CLAUDE_RELAY: &str = "{\n  \"env\": {\n    \"ANTHROPIC_BASE_URL\": \"https://relay.example\",\n    \"ANTHROPIC_API_KEY\": \"sk-relay\",\n    \"ANTHROPIC_MODEL\": \"claude-opus-x\"\n  },\n  \"permissions\": {\n    \"allow\": [\"Bash\"]\n  }\n}\n";
const GEMINI_ENV: &str = "# my own key\nGEMINI_API_KEY=AIza-mine\nHTTPS_PROXY=http://proxy:7890\n";
const GEMINI_API_KEY_SETTINGS: &str = "{\n  // picked in the CLI\n  \"security\": { \"auth\": { \"selectedType\": \"gemini-api-key\" } },\n  \"theme\": \"Dracula\"\n}\n";
const GEMINI_GOOGLE_LOGIN_SETTINGS: &str =
    "{\n  \"security\": {\n    \"auth\": {\n      \"selectedType\": \"oauth-personal\"\n    }\n  }\n}\n";

fn claude_template(model: Option<&str>) -> Value {
    let mut env = json!({
        "ANTHROPIC_BASE_URL": "https://api.ofox.ai/anthropic",
        "ANTHROPIC_AUTH_TOKEN": "",
    });
    if let Some(model) = model {
        env["ANTHROPIC_MODEL"] = json!(model);
    }
    json!({ "env": env })
}

fn gemini_template(model: Option<&str>) -> Value {
    let mut env = json!({
        "GOOGLE_GEMINI_BASE_URL": "https://api.ofox.ai/gemini",
        "GEMINI_API_KEY": "",
    });
    if let Some(model) = model {
        env["GEMINI_MODEL"] = json!(model);
    }
    json!({ "env": env })
}

fn save_tool_template(db: &Database, tool: Tool, template: Value) {
    db.save_provider(
        tool.app().as_str(),
        &Provider::with_id(tool.provider_id().into(), "OfoxAI".into(), template, None),
    )
    .expect("save template");
}

/// Ofox 模板、绑定前在用的服务商 `relay`、官方服务商。
fn db_for(tool: Tool, template: Value) -> Database {
    let db = Database::memory().expect("db");
    save_tool_template(&db, tool, template);
    for id in ["relay", tool.official_id()] {
        db.save_provider(
            tool.app().as_str(),
            &Provider::with_id(id.into(), id.into(), json!({}), None),
        )
        .expect("save provider");
    }
    set_current_provider(&db, &tool.app(), "relay").expect("current");
    db
}

fn current_provider(db: &Database, tool: Tool) -> Option<String> {
    db.get_current_provider(tool.app().as_str()).unwrap()
}

fn json_at(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[tokio::test]
#[serial]
async fn claude_relay_setup_comes_back_byte_for_byte_after_a_model_change() {
    let home = Home::new();
    let settings = home.claude("settings.json");
    write(&settings, CLAUDE_RELAY);
    let db = db_for(Tool::Claude, claude_template(None));

    bind(&db, Tool::Claude, "claude", KEY).await.expect("bind");
    let bound = json_at(&settings);
    assert_eq!(
        bound["env"]["ANTHROPIC_BASE_URL"],
        "https://api.ofox.ai/anthropic"
    );
    assert_eq!(bound["env"]["ANTHROPIC_AUTH_TOKEN"], KEY);
    assert!(
        bound["env"].get("ANTHROPIC_API_KEY").is_none(),
        "the relay key must not be sent to Ofox"
    );
    assert!(bound["env"].get("ANTHROPIC_MODEL").is_none());
    assert_eq!(bound["permissions"]["allow"][0], "Bash");
    assert_eq!(
        current_provider(&db, Tool::Claude).as_deref(),
        Some("ofox-claude")
    );

    save_tool_template(
        &db,
        Tool::Claude,
        claude_template(Some("anthropic/claude-x")),
    );
    rewrite_bound_config(&db, Tool::Claude, KEY)
        .await
        .expect("model change");
    assert_eq!(
        json_at(&settings)["env"]["ANTHROPIC_MODEL"],
        "anthropic/claude-x"
    );

    let report = unbind(&db, Tool::Claude, "claude", &[], false)
        .await
        .expect("unbind");
    assert_eq!(fs::read_to_string(&settings).unwrap(), CLAUDE_RELAY);
    assert_eq!(report.exact_files, ["~/.claude/settings.json"]);
    assert!(!report.legacy);
    assert_eq!(report.provider_restored_to.as_deref(), Some("relay"));
    assert_eq!(
        current_provider(&db, Tool::Claude).as_deref(),
        Some("relay")
    );
    assert!(db.get_bind_record("claude").unwrap().is_none());
}

#[tokio::test]
#[serial]
async fn claude_edits_while_bound_are_reverted_and_new_settings_are_kept() {
    let home = Home::new();
    let settings = home.claude("settings.json");
    write(&settings, CLAUDE_RELAY);
    let db = db_for(Tool::Claude, claude_template(Some("anthropic/claude-x")));
    bind(&db, Tool::Claude, "claude", KEY).await.expect("bind");

    let mut edited = json_at(&settings);
    edited["env"]["ANTHROPIC_BASE_URL"] = json!("https://hand-edited.example");
    edited["statusLine"] = json!({ "type": "command", "command": "echo hi" });
    write(&settings, &serde_json::to_string_pretty(&edited).unwrap());

    unbind(&db, Tool::Claude, "claude", &[], false)
        .await
        .expect("unbind");
    let restored = json_at(&settings);
    let original: Value = serde_json::from_str(CLAUDE_RELAY).unwrap();
    assert_eq!(restored["env"], original["env"]);
    assert_eq!(restored["statusLine"]["command"], "echo hi");
}

#[tokio::test]
#[serial]
async fn claude_bind_refuses_settings_it_cannot_parse() {
    let home = Home::new();
    let settings = home.claude("settings.json");
    write(&settings, "{ \"env\": ");
    let db = db_for(Tool::Claude, claude_template(None));

    assert!(bind(&db, Tool::Claude, "claude", KEY).await.is_err());
    assert_eq!(fs::read_to_string(&settings).unwrap(), "{ \"env\": ");
    assert!(db.get_bind_record("claude").unwrap().is_none());
    assert_eq!(
        current_provider(&db, Tool::Claude).as_deref(),
        Some("relay")
    );
}

#[tokio::test]
#[serial]
async fn claude_settings_created_by_bind_are_removed_again() {
    let home = Home::new();
    let db = db_for(Tool::Claude, claude_template(None));
    bind(&db, Tool::Claude, "claude", KEY).await.expect("bind");
    assert!(home.claude("settings.json").exists());

    let report = unbind(&db, Tool::Claude, "claude", &[], false)
        .await
        .expect("unbind");
    assert!(!home.dir.path().join(".claude").exists());
    assert_eq!(report.files_removed, ["~/.claude/settings.json"]);
}

#[tokio::test]
#[serial]
async fn claude_legacy_binding_is_cleaned_up_with_a_notice() {
    let home = Home::new();
    let settings = home.claude("settings.json");
    write(
        &settings,
        r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.ofox.ai/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-of-OLD","ANTHROPIC_MODEL":"anthropic/x","USER_VAR":"keep"},"statusLine":{"type":"command"}}"#,
    );
    let db = db_for(Tool::Claude, claude_template(None));
    // 从 proxy_live_backup 搬来的旧版补丁：没有绑定前快照。
    db.upsert_bind_record(
        "claude",
        r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.ofox.ai/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-of-OLD"}}"#,
    )
    .unwrap();

    let report = unbind(&db, Tool::Claude, "claude", &[], false)
        .await
        .expect("unbind");
    assert!(report.legacy);
    let cleaned = json_at(&settings);
    assert_eq!(cleaned["env"], json!({ "USER_VAR": "keep" }));
    assert_eq!(cleaned["statusLine"]["type"], "command");
    assert_eq!(
        current_provider(&db, Tool::Claude).as_deref(),
        Some("claude-official")
    );
    assert!(db.get_bind_record("claude").unwrap().is_none());
}

#[tokio::test]
#[serial]
async fn gemini_api_key_user_gets_env_and_settings_back_byte_for_byte() {
    let home = Home::new();
    let (env, settings) = (home.gemini(".env"), home.gemini("settings.json"));
    write(&env, GEMINI_ENV);
    crate::config::set_file_mode(&env, 0o644).unwrap();
    write(&settings, GEMINI_API_KEY_SETTINGS);
    let db = db_for(Tool::Gemini, gemini_template(None));

    bind(&db, Tool::Gemini, "gemini", KEY).await.expect("bind");
    let bound = fs::read_to_string(&env).unwrap();
    assert!(bound.starts_with("# my own key\nGEMINI_API_KEY=sk-of-TEST\n"));
    assert!(bound.contains("GOOGLE_GEMINI_BASE_URL=https://api.ofox.ai/gemini\n"));
    assert!(bound.contains("HTTPS_PROXY=http://proxy:7890\n"));
    #[cfg(unix)]
    assert_eq!(crate::config::file_mode(&env), Some(0o600));
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        GEMINI_API_KEY_SETTINGS,
        "already in API key mode: settings.json is left alone"
    );

    save_tool_template(&db, Tool::Gemini, gemini_template(Some("google/gemini-x")));
    rewrite_bound_config(&db, Tool::Gemini, KEY)
        .await
        .expect("model change");
    assert!(fs::read_to_string(&env)
        .unwrap()
        .contains("GEMINI_MODEL=gemini-x\n"));

    let report = unbind(&db, Tool::Gemini, "gemini", &[], false)
        .await
        .expect("unbind");
    assert_eq!(fs::read_to_string(&env).unwrap(), GEMINI_ENV);
    #[cfg(unix)]
    assert_eq!(crate::config::file_mode(&env), Some(0o644));
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        GEMINI_API_KEY_SETTINGS
    );
    assert_eq!(report.exact_files.len(), 2);
    assert_eq!(
        current_provider(&db, Tool::Gemini).as_deref(),
        Some("relay")
    );
}

#[tokio::test]
#[serial]
async fn gemini_google_login_user_returns_to_google_login() {
    let home = Home::new();
    let (env, settings) = (home.gemini(".env"), home.gemini("settings.json"));
    write(&settings, GEMINI_GOOGLE_LOGIN_SETTINGS);
    let db = db_for(Tool::Gemini, gemini_template(None));

    bind(&db, Tool::Gemini, "gemini", KEY).await.expect("bind");
    assert_eq!(
        json_at(&settings)["security"]["auth"]["selectedType"],
        "gemini-api-key"
    );
    assert!(env.exists());

    let report = unbind(&db, Tool::Gemini, "gemini", &[], false)
        .await
        .expect("unbind");
    assert!(!env.exists(), ".env was created by bind");
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        GEMINI_GOOGLE_LOGIN_SETTINGS
    );
    assert_eq!(report.files_removed, ["~/.gemini/.env"]);
}

#[tokio::test]
#[serial]
async fn gemini_legacy_cleanup_falls_back_to_google_login_without_an_own_key() {
    let home = Home::new();
    let (env, settings) = (home.gemini(".env"), home.gemini("settings.json"));
    // 旧版本绑定后的样子：整个 .env 被按键排序重写。
    write(
        &env,
        "GEMINI_API_KEY=sk-of-OLD\nGEMINI_MODEL=gemini-x\nGOOGLE_GEMINI_BASE_URL=https://api.ofox.ai/gemini\nHTTPS_PROXY=http://proxy:7890\n",
    );
    write(
        &settings,
        r#"{"security":{"auth":{"selectedType":"gemini-api-key"}}}"#,
    );
    let db = db_for(Tool::Gemini, gemini_template(None));

    let report = unbind(&db, Tool::Gemini, "gemini", &[], false)
        .await
        .expect("unbind");
    assert!(report.legacy);
    assert_eq!(
        fs::read_to_string(&env).unwrap(),
        "HTTPS_PROXY=http://proxy:7890\n"
    );
    assert_eq!(
        json_at(&settings)["security"]["auth"]["selectedType"],
        "oauth-personal"
    );
    assert_eq!(
        current_provider(&db, Tool::Gemini).as_deref(),
        Some("gemini-official")
    );
}

#[tokio::test]
#[serial]
async fn gemini_legacy_cleanup_keeps_api_key_login_when_the_user_has_an_own_key() {
    let home = Home::new();
    let (env, settings) = (home.gemini(".env"), home.gemini("settings.json"));
    write(
        &env,
        "GEMINI_API_KEY=AIza-mine\nGOOGLE_GEMINI_BASE_URL=https://api.ofox.ai/gemini\n",
    );
    let settings_text = r#"{"security":{"auth":{"selectedType":"gemini-api-key"}}}"#;
    write(&settings, settings_text);
    let db = db_for(Tool::Gemini, gemini_template(None));

    let report = unbind(&db, Tool::Gemini, "gemini", &[], false)
        .await
        .expect("unbind");
    assert!(report.legacy);
    assert_eq!(
        fs::read_to_string(&env).unwrap(),
        "GEMINI_API_KEY=AIza-mine\n"
    );
    assert_eq!(fs::read_to_string(&settings).unwrap(), settings_text);
}
