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

/// Isolate platform defaults before loading settings. In particular Windows
/// Hermes uses LOCALAPPDATA, and an inherited HERMES_HOME can override HOME.
struct Home {
    dir: tempfile::TempDir,
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("ofox binding 中文 ")
            .tempdir()
            .expect("temp home");
        let isolated = [
            ("HOME", Some(dir.path().to_path_buf())),
            ("USERPROFILE", Some(dir.path().to_path_buf())),
            ("CC_SWITCH_TEST_HOME", Some(dir.path().to_path_buf())),
            (
                "LOCALAPPDATA",
                Some(dir.path().join("AppData").join("Local")),
            ),
            ("APPDATA", Some(dir.path().join("AppData").join("Roaming"))),
            ("HERMES_HOME", None),
            ("OFOX_USE_LOCAL", None),
        ];
        let saved = isolated
            .into_iter()
            .map(|(name, isolated)| {
                let previous = std::env::var_os(name);
                match isolated {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
                (name, previous)
            })
            .collect();
        // Construct the RAII guard before assertions/reload, so a failure also
        // restores the process environment instead of leaking it to later tests.
        let home = Self { dir, saved };
        assert!(crate::config::get_app_config_dir().starts_with(home.dir.path()));
        crate::settings::reload_settings().expect("reload settings");
        for tool in [
            Tool::Codex,
            Tool::Claude,
            Tool::Gemini,
            Tool::OpenCode,
            Tool::OpenClaw,
            Tool::Hermes,
        ] {
            for file in tool.files() {
                assert!(
                    file.current_path().starts_with(home.dir.path()),
                    "{} escaped temporary home",
                    tool.label()
                );
            }
        }
        assert!(crate::workbuddy_config::models_path().starts_with(home.dir.path()));
        home
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
        codex::migrate_legacy_shape(|| None).unwrap(),
        "no Ofox key anywhere: still stops relying on the user's login"
    );
    assert_eq!(
        toml_at(&config)["model_providers"]["ofox"]["requires_openai_auth"].as_bool(),
        Some(false)
    );
    assert_eq!(
        fs::read_to_string(&auth).unwrap(),
        "{\"OPENAI_API_KEY\": \"sk-proj-mine\"}",
        "the user's own key is untouched"
    );
    assert!(!codex::migrate_legacy_shape(|| None).unwrap());

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
    for id in std::iter::once("relay").chain(tool.official_id()) {
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

/// 读 JSON / JSONC（绑定后用户的注释还在）。
fn json_at(path: &Path) -> Value {
    json_file::parse_object(Some(&fs::read_to_string(path).unwrap()), "test").unwrap()
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

// ─── OpenCode / OpenClaw / Hermes ───────────────────────────────────────────

const OPENCODE_CONFIG: &str = "{\n  // my providers\n  \"provider\": {\n    \"deepseek\": { \"options\": { \"apiKey\": \"sk-ds\" } }\n  },\n  \"mcp\": {}\n}\n";
const OPENCLAW_CONFIG: &str = "{\n  // routing\n  models: {\n    mode: 'merge',\n    providers: {},\n  },\n  agents: {\n    defaults: {\n      model: { primary: 'existing/old-model', fallbacks: ['existing/fallback'] },\n      timeoutSeconds: 120,\n    },\n  },\n}\n";
const HERMES_CONFIG: &str = "# Hermes\nmodel:\n  default: existing/old-model\n  provider: existing-provider\n  context_length: 32000\ncustom_providers: []\n";

fn opencode_template(model: Option<&str>) -> Value {
    let models = model.map_or(json!({}), |id| json!({ id: { "name": id } }));
    json!({
        "npm": "@ai-sdk/openai",
        "name": "OfoxAI",
        "options": { "baseURL": "https://api.ofox.ai/v1", "apiKey": "" },
        "models": models,
    })
}

fn openclaw_template() -> Value {
    json!({
        "baseUrl": "https://api.ofox.ai/v1",
        "apiKey": "",
        "api": "openai-completions",
        "models": [{ "id": "openai/gpt-x", "name": "openai/gpt-x" }],
    })
}

fn hermes_template() -> Value {
    json!({
        "name": "ofox",
        "base_url": "https://api.ofox.ai/v1",
        "api_key": "",
        "api_mode": "chat_completions",
        "models": { "openai/gpt-x": {} },
    })
}

#[tokio::test]
#[serial]
async fn opencode_commented_config_comes_back_byte_for_byte_after_a_model_change() {
    let _home = Home::new();
    let path = crate::opencode_config::get_opencode_config_path();
    write(&path, OPENCODE_CONFIG);
    let db = db_for(Tool::OpenCode, opencode_template(None));

    bind(&db, Tool::OpenCode, "opencode", KEY)
        .await
        .expect("bind");
    let providers = crate::opencode_config::get_providers().unwrap();
    assert_eq!(providers["ofox-opencode"]["options"]["apiKey"], KEY);
    assert_eq!(providers["deepseek"]["options"]["apiKey"], "sk-ds");

    save_tool_template(&db, Tool::OpenCode, opencode_template(Some("openai/gpt-x")));
    rewrite_bound_config(&db, Tool::OpenCode, KEY)
        .await
        .expect("model change");
    assert!(
        crate::opencode_config::get_providers().unwrap()["ofox-opencode"]["models"]
            .get("openai/gpt-x")
            .is_some()
    );

    let report = unbind(&db, Tool::OpenCode, "opencode", &[], false)
        .await
        .expect("unbind");
    assert_eq!(fs::read_to_string(&path).unwrap(), OPENCODE_CONFIG);
    assert_eq!(report.exact_files.len(), 1);
    assert_eq!(
        current_provider(&db, Tool::OpenCode).as_deref(),
        Some("relay")
    );
}

#[tokio::test]
#[serial]
async fn opencode_ofox_entry_goes_away_whole_while_other_changes_stay() {
    // 决定 1：Ofox 条目整条还原（绑定期间往里加的模型也一起去掉）；条目以外的改动保留。
    let _home = Home::new();
    let path = crate::opencode_config::get_opencode_config_path();
    write(&path, OPENCODE_CONFIG);
    let db = db_for(Tool::OpenCode, opencode_template(None));
    bind(&db, Tool::OpenCode, "opencode", KEY)
        .await
        .expect("bind");

    let mut edited = json_at(&path);
    edited["provider"]["ofox-opencode"]["models"] = json!({ "qwen3-coder-plus": {} });
    edited["provider"]["added"] = json!({ "options": { "apiKey": "sk-added" } });
    write(&path, &serde_json::to_string_pretty(&edited).unwrap());

    unbind(&db, Tool::OpenCode, "opencode", &[], false)
        .await
        .expect("unbind");
    let restored = json_at(&path);
    assert!(restored["provider"].get("ofox-opencode").is_none());
    assert_eq!(
        restored["provider"]["added"]["options"]["apiKey"],
        "sk-added"
    );
    assert_eq!(
        restored["provider"]["deepseek"]["options"]["apiKey"],
        "sk-ds"
    );
}

#[tokio::test]
#[serial]
async fn opencode_config_created_by_bind_is_removed_again() {
    let _home = Home::new();
    let path = crate::opencode_config::get_opencode_config_path();
    let db = db_for(Tool::OpenCode, opencode_template(None));
    bind(&db, Tool::OpenCode, "opencode", KEY)
        .await
        .expect("bind");
    assert!(path.exists());

    unbind(&db, Tool::OpenCode, "opencode", &[], false)
        .await
        .expect("unbind");
    assert!(!path.exists());
    assert!(!path.parent().unwrap().exists());
}

#[tokio::test]
#[serial]
async fn openclaw_default_model_and_entry_are_restored_byte_for_byte() {
    let _home = Home::new();
    let path = crate::openclaw_config::get_openclaw_config_path();
    write(&path, OPENCLAW_CONFIG);
    let db = db_for(Tool::OpenClaw, openclaw_template());

    bind(&db, Tool::OpenClaw, "openclaw", KEY)
        .await
        .expect("bind");
    let bound = crate::openclaw_config::get_default_model()
        .unwrap()
        .unwrap();
    assert_eq!(bound.primary, "ofox-openclaw/openai/gpt-x");
    assert_eq!(bound.fallbacks, ["existing/fallback"]);
    assert_eq!(
        crate::openclaw_config::get_provider("ofox-openclaw")
            .unwrap()
            .unwrap()["apiKey"],
        KEY
    );

    unbind(&db, Tool::OpenClaw, "openclaw", &[], false)
        .await
        .expect("unbind");
    assert_eq!(fs::read_to_string(&path).unwrap(), OPENCLAW_CONFIG);
    assert_eq!(
        current_provider(&db, Tool::OpenClaw).as_deref(),
        Some("relay")
    );
}

#[tokio::test]
#[serial]
async fn openclaw_legacy_binding_restores_the_recorded_default_model() {
    let _home = Home::new();
    let path = crate::openclaw_config::get_openclaw_config_path();
    write(
        &path,
        "{\n  models: { mode: 'merge', providers: { 'ofox-openclaw': { baseUrl: 'https://api.ofox.ai/v1', apiKey: 'sk-of-OLD' } } },\n  agents: { defaults: { model: { primary: 'ofox-openclaw/x' }, timeoutSeconds: 120 } },\n}\n",
    );
    let db = db_for(Tool::OpenClaw, openclaw_template());
    set_current_provider(&db, &AppType::OpenClaw, "ofox-openclaw").unwrap();
    // 从 proxy_live_backup 搬来的旧版记录：只有补丁和绑定前的默认模型。
    db.upsert_bind_record(
        "openclaw",
        r#"{"__ofoxDirectBackupVersion":1,"patch":{"apiKey":"sk-of-OLD"},"runtimeDefault":{"primary":"existing/old-model","fallbacks":["existing/fallback"]}}"#,
    )
    .unwrap();

    let report = unbind(&db, Tool::OpenClaw, "openclaw", &[], false)
        .await
        .expect("unbind");
    assert!(report.legacy);
    assert!(crate::openclaw_config::get_provider("ofox-openclaw")
        .unwrap()
        .is_none());
    let default = crate::openclaw_config::get_default_model()
        .unwrap()
        .unwrap();
    assert_eq!(default.primary, "existing/old-model");
    assert_eq!(default.fallbacks, ["existing/fallback"]);
    assert_eq!(
        crate::openclaw_config::read_openclaw_config().unwrap()["agents"]["defaults"]
            ["timeoutSeconds"],
        120
    );
    assert_eq!(current_provider(&db, Tool::OpenClaw), None);
}

#[tokio::test]
#[serial]
async fn hermes_routing_comes_back_and_edits_made_while_bound_stay() {
    let _home = Home::new();
    let path = crate::hermes_config::get_hermes_config_path();
    write(&path, HERMES_CONFIG);
    let db = db_for(Tool::Hermes, hermes_template());

    bind(&db, Tool::Hermes, "hermes", KEY).await.expect("bind");
    let bound = crate::hermes_config::get_model_config().unwrap().unwrap();
    assert_eq!(bound.provider.as_deref(), Some("ofox-hermes"));
    assert_eq!(bound.default.as_deref(), Some("openai/gpt-x"));
    assert_eq!(
        crate::hermes_config::get_provider("ofox-hermes")
            .unwrap()
            .unwrap()["api_key"],
        KEY
    );

    // 绑定期间用户改了一个无关的上限。
    let mut edited = bound;
    edited.max_tokens = Some(8192);
    crate::hermes_config::set_model_config(&edited).unwrap();

    unbind(&db, Tool::Hermes, "hermes", &[], false)
        .await
        .expect("unbind");
    let restored = crate::hermes_config::get_model_config().unwrap().unwrap();
    assert_eq!(restored.provider.as_deref(), Some("existing-provider"));
    assert_eq!(restored.default.as_deref(), Some("existing/old-model"));
    assert_eq!(restored.context_length, Some(32000));
    assert_eq!(restored.max_tokens, Some(8192));
    assert!(crate::hermes_config::get_provider("ofox-hermes")
        .unwrap()
        .is_none());
    assert!(fs::read_to_string(&path).unwrap().starts_with("# Hermes\n"));
}

#[tokio::test]
#[serial]
async fn hermes_comes_back_byte_for_byte_when_nothing_else_changed() {
    let _home = Home::new();
    let path = crate::hermes_config::get_hermes_config_path();
    write(&path, HERMES_CONFIG);
    let db = db_for(Tool::Hermes, hermes_template());
    bind(&db, Tool::Hermes, "hermes", KEY).await.expect("bind");

    let report = unbind(&db, Tool::Hermes, "hermes", &[], false)
        .await
        .expect("unbind");
    assert_eq!(fs::read_to_string(&path).unwrap(), HERMES_CONFIG);
    assert_eq!(report.exact_files.len(), 1);
}

#[tokio::test]
#[serial]
async fn hermes_legacy_binding_without_a_routing_snapshot_clears_ofox_routing() {
    let _home = Home::new();
    let path = crate::hermes_config::get_hermes_config_path();
    write(
        &path,
        "model:\n  default: openai/gpt-x\n  provider: ofox-hermes\n  context_length: 32000\ncustom_providers:\n- name: deepseek\n  base_url: https://api.deepseek.com/v1\n- name: ofox-hermes\n  base_url: https://api.ofox.ai/v1\n  api_key: sk-of-OLD\n",
    );
    let db = db_for(Tool::Hermes, hermes_template());
    // 旧格式：记录就是补丁本身，没有绑定前的路由。
    db.upsert_bind_record("hermes", r#"{"api_key":"sk-of-OLD"}"#)
        .unwrap();

    let report = unbind(&db, Tool::Hermes, "hermes", &[], false)
        .await
        .expect("unbind");
    assert!(report.legacy);
    let model = crate::hermes_config::get_model_config().unwrap().unwrap();
    assert_eq!(model.provider, None);
    assert_eq!(model.default, None);
    assert_eq!(model.context_length, Some(32000));
    let providers = crate::hermes_config::get_providers().unwrap();
    assert!(providers.get("ofox-hermes").is_none());
    assert!(providers.get("deepseek").is_some());
}

// ─── WorkBuddy ──────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn workbuddy_unbind_restores_even_after_edits_in_workbuddy() {
    let _home = Home::new();
    let path = crate::workbuddy_config::models_path();
    let original = "[\n  {\n    \"id\": \"openai/gpt-x\",\n    \"vendor\": \"Mine\"\n  },\n  {\n    \"id\": \"local\"\n  }\n]";
    write(&path, original);
    let db = Database::memory().expect("db");
    let selection = crate::workbuddy_config::WorkBuddyModelSelection {
        id: "openai/gpt-x".into(),
        name: "GPT X".into(),
        supports_tool_call: true,
        supports_images: false,
        supports_reasoning: false,
    };
    crate::workbuddy_config::sync_selected_models(&db, KEY, &[selection])
        .await
        .expect("bind");

    // 用户在 WorkBuddy 里改了 Ofox 的条目。
    let mut models: Vec<Value> = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    models[0]["name"] = json!("Renamed in WorkBuddy");
    write(&path, &serde_json::to_string_pretty(&models).unwrap());
    let edited = fs::read_to_string(&path).unwrap();

    let preview = crate::workbuddy_config::unbind(&db, true)
        .await
        .expect("preview");
    assert_eq!(
        preview.restored_keys,
        [format!("{}: openai/gpt-x", display_path(&path))]
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), edited);

    crate::workbuddy_config::unbind(&db, false)
        .await
        .expect("unbind");
    assert_eq!(
        serde_json::from_str::<Value>(&fs::read_to_string(&path).unwrap()).unwrap(),
        serde_json::from_str::<Value>(original).unwrap()
    );
    assert!(db.get_bind_record("workbuddy").unwrap().is_none());
}

// ─── 审查发现的问题 ─────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn a_stale_record_is_replaced_by_a_fresh_snapshot() {
    // 上次解绑后记录没删掉（或磁盘被手动改回去了）：再绑定要重新拍快照，不能
    // 把更早的快照当成现在的原样。
    let home = Home::new();
    let config = home.codex("config.toml");
    write(&config, USER_CONFIG);
    let db = db_with_default_provider();
    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");
    let stale = db.get_bind_record(CODEX_RECORD).unwrap().unwrap().record;
    unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind");
    db.upsert_bind_record(CODEX_RECORD, &stale).unwrap();

    let newer = USER_CONFIG.replace("gpt-5-codex", "gpt-6-codex");
    write(&config, &newer);
    bind(&db, Tool::Codex, "codex", KEY).await.expect("rebind");
    unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind again");
    assert_eq!(fs::read_to_string(&config).unwrap(), newer);
}

#[tokio::test]
#[serial]
async fn rebinding_over_a_legacy_record_keeps_its_saved_routing() {
    // 「添加工具」会把已绑定的工具再绑一次：旧版本记录里的绑定前默认模型不能丢。
    let _home = Home::new();
    let path = crate::openclaw_config::get_openclaw_config_path();
    write(
        &path,
        "{\n  models: { mode: 'merge', providers: { 'ofox-openclaw': { baseUrl: 'https://api.ofox.ai/v1', apiKey: 'sk-of-OLD' } } },\n  agents: { defaults: { model: { primary: 'ofox-openclaw/x' } } },\n}\n",
    );
    let db = db_for(Tool::OpenClaw, openclaw_template());
    db.upsert_bind_record(
        "openclaw",
        r#"{"__ofoxDirectBackupVersion":1,"patch":{"apiKey":"sk-of-OLD"},"runtimeDefault":{"primary":"existing/old-model"}}"#,
    )
    .unwrap();

    bind(&db, Tool::OpenClaw, "openclaw", KEY)
        .await
        .expect("rebind");
    let report = unbind(&db, Tool::OpenClaw, "openclaw", &[], false)
        .await
        .expect("unbind");
    assert!(report.legacy);
    assert_eq!(
        crate::openclaw_config::get_default_model()
            .unwrap()
            .unwrap()
            .primary,
        "existing/old-model"
    );
}

#[tokio::test]
#[serial]
async fn a_retried_legacy_unbind_still_moves_the_provider_off_ofox() {
    // 上次清理完文件、切服务商时出了错：这次文件已经干净，服务商也得切回来。
    let home = Home::new();
    write(
        &home.claude("settings.json"),
        "{\n  \"permissions\": {}\n}\n",
    );
    let db = db_for(Tool::Claude, claude_template(None));
    set_current_provider(&db, &AppType::Claude, "ofox-claude").unwrap();

    let report = unbind(&db, Tool::Claude, "claude", &[], false)
        .await
        .expect("unbind");
    assert!(!report.already_unbound);
    assert_eq!(
        report.provider_restored_to.as_deref(),
        Some("claude-official")
    );
    assert_eq!(
        current_provider(&db, Tool::Claude).as_deref(),
        Some("claude-official")
    );

    let again = unbind(&db, Tool::Claude, "claude", &[], false)
        .await
        .expect("unbind again");
    assert!(again.already_unbound);
}

#[tokio::test]
#[serial]
async fn ofox_config_at_a_moved_path_is_cleaned_up_too() {
    // 绑定后改了 Codex 的配置目录，并在新目录里换过模型：解绑要两边都处理。
    let home = Home::new();
    let old_config = home.codex("config.toml");
    write(&old_config, USER_CONFIG);
    let db = db_with_default_provider();
    bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");

    let moved_dir = home.dir.path().join("codex-moved");
    let moved_config = moved_dir.join("config.toml");
    fs::create_dir_all(&moved_dir).unwrap();
    fs::copy(&old_config, &moved_config).unwrap();
    crate::settings::mutate_settings(|settings| {
        settings.codex_config_dir = Some(moved_dir.to_string_lossy().into_owned());
    })
    .unwrap();

    let preview = unbind(&db, Tool::Codex, "codex", &[], true)
        .await
        .expect("preview");
    assert!(preview.warnings.iter().any(|w| w.code == "leftoverRemoved"));
    let report = unbind(&db, Tool::Codex, "codex", &[], false)
        .await
        .expect("unbind");
    assert!(report.warnings.iter().any(|w| w.code == "leftoverRemoved"));
    assert_eq!(fs::read_to_string(&old_config).unwrap(), USER_CONFIG);
    assert!(!codex::is_ofox_bound(
        &fs::read_to_string(&moved_config).unwrap()
    ));
}

#[tokio::test]
#[serial]
async fn legacy_codex_without_a_key_stops_using_the_chatgpt_login() {
    let home = Home::new();
    let (auth, config) = (home.codex("auth.json"), home.codex("config.toml"));
    write(&config, TEMPLATE);
    write(&auth, OAUTH_AUTH);

    assert!(codex::migrate_legacy_shape(|| None).unwrap());
    let provider = &toml_at(&config)["model_providers"]["ofox"];
    assert_eq!(provider["requires_openai_auth"].as_bool(), Some(false));
    assert!(provider.get("experimental_bearer_token").is_none());
    assert_eq!(fs::read_to_string(&auth).unwrap(), OAUTH_AUTH);
    assert!(!codex::migrate_legacy_shape(|| None).unwrap(), "idempotent");

    assert!(codex::migrate_legacy_shape(|| Some("sk-of-LATER".into())).unwrap());
    assert_eq!(
        toml_at(&config)["model_providers"]["ofox"]["experimental_bearer_token"].as_str(),
        Some("sk-of-LATER")
    );
}

#[tokio::test]
#[serial]
async fn preview_and_unbind_agree_when_the_previous_provider_is_gone() {
    let home = Home::new();
    write(&home.claude("settings.json"), CLAUDE_RELAY);
    let db = db_for(Tool::Claude, claude_template(None));
    bind(&db, Tool::Claude, "claude", KEY).await.expect("bind");
    db.delete_provider("claude", "relay").unwrap();

    let preview = unbind(&db, Tool::Claude, "claude", &[], true)
        .await
        .expect("preview");
    let report = unbind(&db, Tool::Claude, "claude", &[], false)
        .await
        .expect("unbind");
    assert_eq!(
        preview.provider_restored_to.as_deref(),
        Some("claude-official")
    );
    assert_eq!(preview.provider_restored_to, report.provider_restored_to);
    assert_eq!(preview.warnings, report.warnings);
    assert!(report
        .warnings
        .iter()
        .any(|w| w.code == "previousProviderMissing"));
    assert_eq!(
        current_provider(&db, Tool::Claude).as_deref(),
        Some("claude-official")
    );
}

#[tokio::test]
#[serial]
async fn broken_records_are_rejected_instead_of_deleting_user_config() {
    let home = Home::new();
    write(&home.claude("settings.json"), CLAUDE_RELAY);
    let db = db_for(Tool::Claude, claude_template(None));
    db.upsert_bind_record("claude", r#"{"v":1,"kind":"snapshot","files":[]}"#)
        .unwrap();
    assert!(unbind(&db, Tool::Claude, "claude", &[], false)
        .await
        .is_err());
    assert_eq!(
        fs::read_to_string(home.claude("settings.json")).unwrap(),
        CLAUDE_RELAY
    );
}

#[tokio::test]
#[serial]
async fn workbuddy_unbind_drops_ofox_entries_the_record_does_not_track() {
    let _home = Home::new();
    let path = crate::workbuddy_config::models_path();
    write(&path, "[]");
    let db = Database::memory().expect("db");
    let selection = |id: &str| crate::workbuddy_config::WorkBuddyModelSelection {
        id: id.into(),
        name: id.into(),
        supports_tool_call: true,
        supports_images: false,
        supports_reasoning: false,
    };
    crate::workbuddy_config::sync_selected_models(&db, KEY, &[selection("openai/gpt-x")])
        .await
        .expect("bind");
    // 更早的绑定留下的条目，记录里没有它。
    let read_models =
        || -> Vec<Value> { serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap() };
    let mut models = read_models();
    let mut stray = models[0].clone();
    stray["id"] = json!("openai/gpt-old");
    models.push(stray);
    models.push(json!({ "id": "local", "url": "http://localhost:11434/v1" }));
    write(&path, &serde_json::to_string_pretty(&models).unwrap());

    crate::workbuddy_config::unbind(&db, false)
        .await
        .expect("unbind");
    assert_eq!(
        read_models(),
        [json!({ "id": "local", "url": "http://localhost:11434/v1" })]
    );
}

fn binding_cases() -> Vec<(Tool, Value)> {
    vec![
        (Tool::Codex, json!({"config": TEMPLATE})),
        (
            Tool::Claude,
            claude_template(Some("anthropic/claude-saved")),
        ),
        (Tool::Gemini, gemini_template(Some("google/gemini-saved"))),
        (Tool::OpenCode, opencode_template(Some("openai/gpt-saved"))),
        (Tool::OpenClaw, openclaw_template()),
        (Tool::Hermes, hermes_template()),
    ]
}

fn original_for(file: ManagedFile) -> &'static str {
    match file {
        ManagedFile::CodexConfig => USER_CONFIG,
        ManagedFile::ClaudeSettings => CLAUDE_RELAY,
        ManagedFile::GeminiEnv => GEMINI_ENV,
        ManagedFile::GeminiSettings => {
            r#"{"security":{"auth":{"selectedType":"oauth-personal"}},"theme":"dark"}"#
        }
        ManagedFile::OpenCodeConfig => OPENCODE_CONFIG,
        ManagedFile::OpenClawConfig => OPENCLAW_CONFIG,
        ManagedFile::HermesConfig => HERMES_CONFIG,
    }
}

#[tokio::test]
#[serial]
async fn deleted_configuration_is_not_recreated_by_preview_unbind_or_startup_checks() {
    for (tool, template) in binding_cases() {
        let home = Home::new();
        let db = db_for(tool, template);
        for file in tool.files() {
            let path = file.current_path();
            assert!(path.starts_with(home.dir.path()));
            write(&path, original_for(*file));
        }
        let holder = tool.app().as_str();
        bind(&db, tool, holder, KEY).await.unwrap();
        assert!(all_config_files_present(tool));
        let directory = tool.files()[0]
            .current_path()
            .parent()
            .unwrap()
            .to_path_buf();
        fs::remove_dir_all(&directory).unwrap();
        assert!(!all_config_files_present(tool));
        for dry_run in [true, false] {
            let report = unbind(&db, tool, holder, &[], dry_run).await.unwrap();
            assert_eq!(
                report
                    .warnings
                    .iter()
                    .filter(|warning| warning.code == "configAlreadyMissing")
                    .count(),
                tool.files().len()
            );
            assert!(report.restored_keys.is_empty());
            assert!(!directory.exists());
        }
        assert!(db.get_bind_record(tool.record_key()).unwrap().is_none());
    }
}

#[tokio::test]
#[serial]
async fn missing_bindings_restore_saved_models_and_selected_apex_without_replacing_snapshots() {
    for apex in ["ofox.ai", "ofox.io"] {
        for (tool, template) in binding_cases() {
            let home = Home::new();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(apex.into()))
                .unwrap();
            let db = db_for(tool, template);
            for file in tool.files() {
                let path = file.current_path();
                assert!(path.starts_with(home.dir.path()));
                write(&path, original_for(*file));
            }
            let holder = tool.app().as_str();
            bind(&db, tool, holder, KEY).await.unwrap();
            let before: Vec<_> = tool
                .files()
                .iter()
                .map(|file| fs::read(file.current_path()).unwrap())
                .collect();
            let record = db
                .get_bind_record(tool.record_key())
                .unwrap()
                .unwrap()
                .record;
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Configured
            );
            let directory = tool.files()[0]
                .current_path()
                .parent()
                .unwrap()
                .to_path_buf();
            fs::remove_dir_all(&directory).unwrap();
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Missing
            );
            restore_missing_binding(&db, tool, holder, &[], KEY)
                .await
                .unwrap();
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Configured
            );
            assert_eq!(
                db.get_bind_record(tool.record_key())
                    .unwrap()
                    .unwrap()
                    .record,
                record
            );
            for (file, original_bound) in tool.files().iter().zip(&before) {
                let restored = fs::read_to_string(file.current_path()).unwrap();
                // Every model/token-bearing value survived; unrelated deleted settings are not resurrected.
                let expected = String::from_utf8(original_bound.clone()).unwrap();
                match file {
                    ManagedFile::CodexConfig => {
                        assert_eq!(
                            toml_at(&file.current_path())["model"],
                            toml::from_str::<toml::Table>(&expected).unwrap()["model"]
                        );
                    }
                    ManagedFile::ClaudeSettings => assert_eq!(
                        json_at(&file.current_path())["env"]["ANTHROPIC_MODEL"],
                        "anthropic/claude-saved"
                    ),
                    ManagedFile::GeminiEnv => {
                        assert!(restored.contains("GEMINI_MODEL=gemini-saved"))
                    }
                    ManagedFile::OpenCodeConfig => assert!(restored.contains("openai/gpt-saved")),
                    ManagedFile::OpenClawConfig => assert!(restored.contains("openai/gpt-x")),
                    ManagedFile::HermesConfig => assert!(restored.contains("openai/gpt-x")),
                    ManagedFile::GeminiSettings => continue,
                }
                assert!(restored.contains(&format!("https://api.{apex}")));
            }
        }
    }
}

#[tokio::test]
#[serial]
async fn status_and_restore_keep_modified_or_invalid_configuration_and_hide_secrets() {
    let home = Home::new();
    let path = home.claude("settings.json");
    let db = db_for(
        Tool::Claude,
        claude_template(Some("anthropic/claude-saved")),
    );
    bind(&db, Tool::Claude, "claude", KEY).await.unwrap();
    let mut current = json_at(&path);
    current["permissions"] = json!({"allow":["Bash"]});
    current["env"]["ANTHROPIC_MODEL"] = json!("user/other-model");
    write(&path, &serde_json::to_string(&current).unwrap());
    let edited = fs::read(&path).unwrap();
    let record = db.get_bind_record("claude").unwrap().unwrap().record;
    let health = status::binding_status(&db, Tool::Claude, Some(KEY)).await;
    assert_eq!(health.status, status::BindingStatus::Modified);
    assert!(!serde_json::to_string(&health).unwrap().contains(KEY));
    assert!(
        restore_missing_binding(&db, Tool::Claude, "claude", &[], KEY)
            .await
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), edited);
    assert_eq!(
        db.get_bind_record("claude").unwrap().unwrap().record,
        record
    );
    write(&path, &format!("{{ token: '{KEY}' invalid"));
    let health = status::binding_status(&db, Tool::Claude, Some(KEY)).await;
    assert_eq!(health.status, status::BindingStatus::Unknown);
    assert!(!serde_json::to_string(&health).unwrap().contains(KEY));
    assert!(
        restore_missing_binding(&db, Tool::Claude, "claude", &[], KEY)
            .await
            .is_err()
    );
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(
        status::binding_status(&db, Tool::Claude, Some(KEY))
            .await
            .status,
        status::BindingStatus::Unknown
    );
    assert!(
        restore_missing_binding(&db, Tool::Claude, "claude", &[], KEY)
            .await
            .is_err()
    );
    assert!(path.is_dir());
}

#[tokio::test]
#[serial]
async fn partial_gemini_deletion_restores_only_missing_connection_and_unbind_skips_missing_file() {
    let home = Home::new();
    let db = db_for(Tool::Gemini, gemini_template(Some("google/gemini-saved")));
    let env = home.gemini(".env");
    let settings = home.gemini("settings.json");
    write(&env, GEMINI_ENV);
    write(
        &settings,
        r#"{"security":{"auth":{"selectedType":"oauth-personal"}},"theme":"dark"}"#,
    );
    bind(&db, Tool::Gemini, "gemini", KEY).await.unwrap();
    let kept_settings = fs::read(&settings).unwrap();
    fs::remove_file(&env).unwrap();
    assert_eq!(
        status::binding_status(&db, Tool::Gemini, Some(KEY))
            .await
            .status,
        status::BindingStatus::Missing
    );
    restore_missing_binding(&db, Tool::Gemini, "gemini", &[], KEY)
        .await
        .unwrap();
    assert_eq!(fs::read(&settings).unwrap(), kept_settings);
    fs::remove_file(&env).unwrap();
    let report = unbind(&db, Tool::Gemini, "gemini", &[], false)
        .await
        .unwrap();
    assert_eq!(report.warnings[0].code, "configAlreadyMissing");
    assert!(!env.exists());
    assert_eq!(
        json_at(&settings)["security"]["auth"]["selectedType"],
        "oauth-personal"
    );
    assert_eq!(json_at(&settings)["theme"], "dark");
}

#[tokio::test]
#[serial]
async fn shared_codex_restore_preserves_both_holders_and_deleted_config_stays_deleted_on_unbind() {
    let home = Home::new();
    let path = home.codex("config.toml");
    let auth = home.codex("auth.json");
    let db = db_with_default_provider();
    write(&path, USER_CONFIG);
    write(&auth, OAUTH_AUTH);
    bind(&db, Tool::Codex, "codex", KEY).await.unwrap();
    bind(&db, Tool::Codex, "chatgpt", KEY).await.unwrap();
    fs::remove_file(&path).unwrap();
    restore_missing_binding(&db, Tool::Codex, "codex", &["chatgpt".into()], KEY)
        .await
        .unwrap();
    let (_, StoredRecord::Envelope(envelope)) = load_record(&db, Tool::Codex).unwrap().unwrap()
    else {
        panic!("envelope");
    };
    assert_eq!(
        envelope.holders,
        BTreeSet::from(["codex".into(), "chatgpt".into()])
    );
    assert_eq!(fs::read_to_string(&auth).unwrap(), OAUTH_AUTH);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
    let first = unbind(&db, Tool::Codex, "codex", &[], false).await.unwrap();
    assert_eq!(first.shared_kept_by, ["chatgpt"]);
    assert!(!path.parent().unwrap().exists());
    let last = unbind(&db, Tool::Codex, "chatgpt", &[], false)
        .await
        .unwrap();
    assert_eq!(last.warnings[0].code, "configAlreadyMissing");
    assert!(!path.parent().unwrap().exists());
    assert!(db.get_bind_record("codex").unwrap().is_none());
}

#[tokio::test]
#[serial]
async fn workbuddy_deleted_config_restores_saved_selection_or_unbinds_without_recreating_originals()
{
    for apex in ["ofox.ai", "ofox.io"] {
        let home = Home::new();
        crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(apex.into()))
            .unwrap();
        let db = Database::memory().unwrap();
        let path = crate::workbuddy_config::models_path();
        assert!(path.starts_with(home.dir.path()));
        write(
            &path,
            r#"[{"id":"model-a","vendor":"User","url":"https://user.example"}]"#,
        );
        let selections: Vec<_> = ["model-a", "model-b"]
            .into_iter()
            .map(|id| crate::workbuddy_config::WorkBuddyModelSelection {
                id: id.into(),
                name: format!("Saved {id}"),
                supports_tool_call: true,
                supports_images: id == "model-b",
                supports_reasoning: true,
            })
            .collect();
        crate::workbuddy_config::sync_selected_models(&db, KEY, &selections)
            .await
            .unwrap();
        let record = db.get_bind_record("workbuddy").unwrap().unwrap().record;
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert_eq!(
            crate::workbuddy_config::binding_status(&db).await.status,
            status::BindingStatus::Missing
        );
        crate::workbuddy_config::restore_missing_binding(&db, KEY)
            .await
            .unwrap();
        let models: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(models.as_array().unwrap().len(), 2);
        assert_eq!(models[0]["name"], "Saved model-a");
        assert_eq!(models[1]["supportsImages"], true);
        assert_eq!(
            models[0]["url"],
            format!("https://api.{apex}/v1/chat/completions")
        );
        assert_eq!(
            db.get_bind_record("workbuddy").unwrap().unwrap().record,
            record
        );
        assert_eq!(
            crate::workbuddy_config::binding_status(&db).await.status,
            status::BindingStatus::Configured
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
        let preview = crate::workbuddy_config::unbind(&db, true).await.unwrap();
        assert_eq!(preview.warnings[0].code, "configAlreadyMissing");
        assert!(db.get_bind_record("workbuddy").unwrap().is_some());
        let report = crate::workbuddy_config::unbind(&db, false).await.unwrap();
        assert_eq!(report.warnings[0].code, "configAlreadyMissing");
        assert!(!path.parent().unwrap().exists());
        assert!(db.get_bind_record("workbuddy").unwrap().is_none());
    }
}

#[tokio::test]
#[serial]
async fn model_save_and_startup_repair_recheck_deleted_files_at_the_final_lock() {
    for (tool, saved) in binding_cases() {
        let _home = Home::new();
        let db = db_for(tool, saved.clone());
        for file in tool.files() {
            write(&file.current_path(), original_for(*file));
        }
        bind(&db, tool, tool.app().as_str(), KEY).await.unwrap();
        assert_eq!(
            status::binding_status(&db, tool, Some(KEY)).await.status,
            status::BindingStatus::Configured
        );
        let directory = tool.files()[0]
            .current_path()
            .parent()
            .unwrap()
            .to_path_buf();
        let record = db
            .get_bind_record(tool.record_key())
            .unwrap()
            .unwrap()
            .record;
        fs::remove_dir_all(&directory).unwrap();
        let mut updated = saved.clone();
        updated["regressionModelSave"] = json!(true);
        assert!(persist_bound_settings(&db, tool, KEY, &saved, &updated)
            .await
            .is_err());
        assert_eq!(
            db.get_provider_by_id(tool.provider_id(), tool.app().as_str())
                .unwrap()
                .unwrap()
                .settings_config,
            saved
        );
        assert!(rewrite_bound_config(&db, tool, KEY).await.is_err());
        assert!(bind_existing(&db, tool, tool.app().as_str(), KEY)
            .await
            .is_err());
        assert!(!directory.exists());
        assert_eq!(
            db.get_bind_record(tool.record_key())
                .unwrap()
                .unwrap()
                .record,
            record
        );
    }
}

#[tokio::test]
#[serial]
async fn atomic_model_save_preserves_user_fields_and_rejects_stale_or_unbound_requests() {
    for apex in ["ofox.ai", "ofox.io"] {
        let home = Home::new();
        crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(apex.into()))
            .unwrap();
        let saved = claude_template(Some("anthropic/saved"));
        let db = db_for(Tool::Claude, saved.clone());
        let path = home.claude("settings.json");
        write(&path, CLAUDE_RELAY);
        bind(&db, Tool::Claude, "claude", KEY).await.unwrap();
        let mut updated = saved.clone();
        updated["env"]["ANTHROPIC_MODEL"] = json!("anthropic/next");
        persist_bound_settings(&db, Tool::Claude, KEY, &saved, &updated)
            .await
            .unwrap();
        let disk = json_at(&path);
        assert_eq!(disk["env"]["ANTHROPIC_MODEL"], "anthropic/next");
        assert_eq!(
            disk["env"]["ANTHROPIC_BASE_URL"],
            format!("https://api.{apex}/anthropic")
        );
        let original: Value = serde_json::from_str(CLAUDE_RELAY).unwrap();
        assert_eq!(disk["permissions"], original["permissions"]);
        let bytes = fs::read(&path).unwrap();
        assert!(
            persist_bound_settings(&db, Tool::Claude, KEY, &saved, &updated)
                .await
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        unbind(&db, Tool::Claude, "claude", &[], false)
            .await
            .unwrap();
        assert!(
            persist_bound_settings(&db, Tool::Claude, KEY, &updated, &saved)
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), CLAUDE_RELAY);
        assert_eq!(
            db.get_provider_by_id("ofox-claude", "claude")
                .unwrap()
                .unwrap()
                .settings_config,
            updated
        );
        assert!(bind_existing(&db, Tool::Claude, "claude", KEY)
            .await
            .is_err());
    }
}

#[tokio::test]
#[serial]
async fn workbuddy_model_save_rechecks_deletion_and_external_modification_after_early_check() {
    let home = Home::new();
    let db = Database::memory().unwrap();
    let path = crate::workbuddy_config::models_path();
    let selection = |id: &str| crate::workbuddy_config::WorkBuddyModelSelection {
        id: id.into(),
        name: id.into(),
        supports_tool_call: true,
        supports_images: false,
        supports_reasoning: false,
    };
    crate::workbuddy_config::sync_selected_models(&db, KEY, &[selection("saved")])
        .await
        .unwrap();
    assert_eq!(
        crate::workbuddy_config::binding_status(&db).await.status,
        status::BindingStatus::Configured
    );
    let record = db.get_bind_record("workbuddy").unwrap().unwrap().record;
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
    assert!(
        crate::workbuddy_config::update_selected_models(&db, KEY, &[selection("next")])
            .await
            .is_err()
    );
    assert!(!path.parent().unwrap().exists());
    assert_eq!(
        db.get_bind_record("workbuddy").unwrap().unwrap().record,
        record
    );
    crate::workbuddy_config::restore_missing_binding(&db, KEY)
        .await
        .unwrap();
    let mut models: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    models[0]["url"] = json!("https://user.example/v1");
    write(&path, &serde_json::to_string(&models).unwrap());
    let bytes = fs::read(&path).unwrap();
    assert!(
        crate::workbuddy_config::update_selected_models(&db, KEY, &[selection("next")])
            .await
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        db.get_bind_record("workbuddy").unwrap().unwrap().record,
        record
    );
    assert!(path.starts_with(home.dir.path()));
}

#[tokio::test]
#[serial]
async fn workbuddy_partial_deletion_survives_region_switch_restart_and_explicit_restore() {
    for (from, to) in [("ofox.ai", "ofox.io"), ("ofox.io", "ofox.ai")] {
        let _home = Home::new();
        crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(from.into()))
            .unwrap();
        let db = Database::memory().unwrap();
        let path = crate::workbuddy_config::models_path();
        let selections: Vec<_> = ["deleted", "kept"]
            .into_iter()
            .map(|id| crate::workbuddy_config::WorkBuddyModelSelection {
                id: id.into(),
                name: format!("Saved {id}"),
                supports_tool_call: true,
                supports_images: id == "kept",
                supports_reasoning: true,
            })
            .collect();
        crate::workbuddy_config::sync_selected_models(&db, KEY, &selections)
            .await
            .unwrap();
        let mut models: Vec<Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        models.remove(0);
        write(&path, &serde_json::to_string(&models).unwrap());
        crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(to.into())).unwrap();
        assert!(crate::workbuddy_config::reconcile_managed_endpoint(&db)
            .await
            .unwrap());
        let migrated: Vec<Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(migrated.len(), 1);
        assert_eq!(migrated[0]["id"], "kept");
        assert_eq!(
            migrated[0]["url"],
            format!("https://api.{to}/v1/chat/completions")
        );
        let record: Value =
            serde_json::from_str(&db.get_bind_record("workbuddy").unwrap().unwrap().record)
                .unwrap();
        assert_eq!(
            record["managedModels"][0]["lastWrittenEntry"]["url"],
            format!("https://api.{from}/v1/chat/completions")
        );
        assert_eq!(
            record["managedModels"][1]["lastWrittenEntry"]["url"],
            format!("https://api.{to}/v1/chat/completions")
        );
        crate::settings::reload_settings().unwrap();
        assert!(!crate::workbuddy_config::reconcile_managed_endpoint(&db)
            .await
            .unwrap());
        assert_eq!(
            crate::workbuddy_config::binding_status(&db).await.status,
            status::BindingStatus::Missing
        );
        crate::workbuddy_config::restore_missing_binding(&db, KEY)
            .await
            .unwrap();
        let restored: Vec<Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(restored.len(), 2);
        assert!(restored
            .iter()
            .all(|entry| entry["url"] == format!("https://api.{to}/v1/chat/completions")));
        assert_eq!(
            crate::workbuddy_config::binding_status(&db).await.status,
            status::BindingStatus::Configured
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
        let record = db.get_bind_record("workbuddy").unwrap().unwrap().record;
        assert!(!crate::workbuddy_config::reconcile_managed_endpoint(&db)
            .await
            .unwrap());
        assert!(!path.parent().unwrap().exists());
        assert_eq!(
            db.get_bind_record("workbuddy").unwrap().unwrap().record,
            record
        );
    }
}

fn region_expected_url(tool: Tool, apex: &str) -> String {
    let protocol = match tool {
        Tool::Claude => "anthropic",
        Tool::Gemini => "gemini",
        _ => "v1",
    };
    format!("https://api.{apex}/{protocol}")
}

fn region_mask_endpoint(tool: Tool, value: &mut Value, live: bool) {
    let pointer = match (tool, live) {
        (Tool::Codex, false) => "/config/model_providers/ofox/base_url",
        (Tool::Codex, true) => "/model_providers/ofox/base_url",
        (Tool::Claude, _) => "/env/ANTHROPIC_BASE_URL",
        (Tool::Gemini, false) => "/env/GOOGLE_GEMINI_BASE_URL",
        (Tool::Gemini, true) => "/endpoint",
        (Tool::OpenCode, false) => "/options/baseURL",
        (Tool::OpenCode, true) => "/provider/ofox-opencode/options/baseURL",
        (Tool::OpenClaw, false) => "/baseUrl",
        (Tool::OpenClaw, true) => "/models/providers/ofox-openclaw/baseUrl",
        (Tool::Hermes, false) => "/base_url",
        (Tool::Hermes, true) => {
            if let Some(providers) = value
                .get_mut("custom_providers")
                .and_then(Value::as_array_mut)
            {
                for provider in providers {
                    if provider["name"] == "ofox-hermes" {
                        if let Some(endpoint) = provider.get_mut("base_url") {
                            *endpoint = json!("REGION_ENDPOINT");
                        }
                    }
                }
            }
            return;
        }
    };
    if let Some(endpoint) = value.pointer_mut(pointer) {
        *endpoint = json!("REGION_ENDPOINT");
    }
}

fn region_provider_snapshot(db: &Database, tool: Tool) -> Value {
    let provider = db
        .get_provider_by_id(tool.provider_id(), tool.app().as_str())
        .unwrap()
        .unwrap();
    let mut value = serde_json::to_value(provider).unwrap();
    let settings = &mut value["settingsConfig"];
    if tool == Tool::Codex {
        settings["config"] = serde_json::to_value(
            toml::from_str::<toml::Value>(settings["config"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
    }
    region_mask_endpoint(tool, settings, false);
    value
}

fn region_live_snapshot(tool: Tool) -> Vec<Value> {
    tool.files()
        .iter()
        .map(|file| {
            let text = fs::read_to_string(file.current_path()).unwrap();
            let mut value = match file {
                ManagedFile::CodexConfig => {
                    serde_json::to_value(toml::from_str::<toml::Value>(&text).unwrap()).unwrap()
                }
                ManagedFile::GeminiEnv => json!({
                    "endpoint": env_file::get_value(&text, "GOOGLE_GEMINI_BASE_URL"),
                    "sourceWithoutEndpoint": env_file::set_value(&text, "GOOGLE_GEMINI_BASE_URL", None),
                }),
                ManagedFile::HermesConfig => serde_json::to_value(
                    crate::hermes_config::parse_config_text(&text).unwrap(),
                )
                .unwrap(),
                _ => json_file::parse_object(Some(&text), "region regression").unwrap(),
            };
            region_mask_endpoint(tool, &mut value, true);
            value
        })
        .collect()
}

fn region_live_endpoint(tool: Tool) -> Option<String> {
    let path = tool.files()[0].current_path();
    let text = fs::read_to_string(path).unwrap();
    if tool == Tool::Gemini {
        return env_file::get_value(&text, "GOOGLE_GEMINI_BASE_URL");
    }
    if tool == Tool::Codex {
        return toml::from_str::<toml::Value>(&text).unwrap()["model_providers"]["ofox"]
            ["base_url"]
            .as_str()
            .map(str::to_string);
    }
    if tool == Tool::Hermes {
        let config = crate::hermes_config::parse_config_text(&text).unwrap();
        return config["custom_providers"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|entry| entry["name"].as_str() == Some("ofox-hermes"))
            .and_then(|entry| entry["base_url"].as_str().map(str::to_string));
    }
    let config = json_file::parse_object(Some(&text), "region regression").unwrap();
    let pointer = match tool {
        Tool::Claude => "/env/ANTHROPIC_BASE_URL",
        Tool::OpenCode => "/provider/ofox-opencode/options/baseURL",
        Tool::OpenClaw => "/models/providers/ofox-openclaw/baseUrl",
        _ => unreachable!(),
    };
    config
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn region_remove_connection_field(tool: Tool) {
    let path = tool.files()[0].current_path();
    let text = fs::read_to_string(&path).unwrap();
    let missing = match tool {
        Tool::Codex => {
            let mut config = text.parse::<toml_edit::DocumentMut>().unwrap();
            config["model_providers"]["ofox"]
                .as_table_like_mut()
                .unwrap()
                .remove("experimental_bearer_token");
            config.to_string()
        }
        Tool::Gemini => env_file::set_value(&text, "GEMINI_API_KEY", None),
        Tool::Hermes => {
            let mut config = crate::hermes_config::parse_config_text(&text).unwrap();
            let provider = config["custom_providers"]
                .as_sequence_mut()
                .unwrap()
                .iter_mut()
                .find(|entry| entry["name"].as_str() == Some("ofox-hermes"))
                .unwrap();
            provider
                .as_mapping_mut()
                .unwrap()
                .remove(serde_yaml::Value::String("api_key".into()));
            serde_yaml::to_string(&config).unwrap()
        }
        _ => {
            let mut config = json_file::parse_object(Some(&text), "region regression").unwrap();
            let field = match tool {
                Tool::Claude => &["env", "ANTHROPIC_AUTH_TOKEN"][..],
                Tool::OpenCode => &["provider", "ofox-opencode"][..],
                Tool::OpenClaw => &["models", "providers", "ofox-openclaw", "apiKey"][..],
                _ => unreachable!(),
            };
            assert!(json_file::remove(&mut config, field));
            serde_json::to_string_pretty(&config).unwrap()
        }
    };
    fs::write(&path, missing).unwrap();
}

async fn region_bind_saved_template(db: &Database, tool: Tool) {
    let mut provider = db
        .get_provider_by_id(tool.provider_id(), tool.app().as_str())
        .unwrap()
        .unwrap();
    provider.notes = Some("Keep region regression metadata".into());
    provider.icon = Some("saved-icon".into());
    provider.meta = Some(serde_json::from_value(json!({ "providerType": "ofox" })).unwrap());
    db.save_provider(tool.app().as_str(), &provider).unwrap();
    for file in tool.files() {
        write(&file.current_path(), original_for(*file));
    }
    bind(db, tool, tool.app().as_str(), KEY).await.unwrap();
    if tool == Tool::Codex {
        bind(db, tool, "chatgpt", KEY).await.unwrap();
    }
    // The fixture templates start on .ai; normalize the saved endpoint before
    // taking the before-image, including a scenario that initially uses .io.
    assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(db).await);
}

#[tokio::test]
#[serial]
async fn region_reconciliation_changes_only_urls_and_restart_is_idempotent() {
    for (from, to) in [("ofox.ai", "ofox.io"), ("ofox.io", "ofox.ai")] {
        for (tool, saved) in binding_cases() {
            let _home = Home::new();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(from.into()))
                .unwrap();
            let db = db_for(tool, saved);
            region_bind_saved_template(&db, tool).await;
            let provider = region_provider_snapshot(&db, tool);
            let live = region_live_snapshot(tool);
            let record = db
                .get_bind_record(tool.record_key())
                .unwrap()
                .unwrap()
                .record;
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(to.into()))
                .unwrap();
            assert!(
                crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await,
                "{} {from} -> {to}",
                tool.label()
            );
            assert_eq!(
                region_live_endpoint(tool).as_deref(),
                Some(region_expected_url(tool, to).as_str())
            );
            assert_eq!(
                region_provider_snapshot(&db, tool),
                provider,
                "{} saved model and metadata",
                tool.label()
            );
            assert_eq!(
                region_live_snapshot(tool),
                live,
                "{} key, model and user fields",
                tool.label()
            );
            assert_eq!(
                db.get_bind_record(tool.record_key())
                    .unwrap()
                    .unwrap()
                    .record,
                record
            );
            let bytes: Vec<_> = tool
                .files()
                .iter()
                .map(|file| fs::read(file.current_path()).unwrap())
                .collect();
            let settings = db
                .get_provider_by_id(tool.provider_id(), tool.app().as_str())
                .unwrap()
                .unwrap()
                .settings_config;
            crate::settings::reload_settings().unwrap();
            assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(
                tool.files()
                    .iter()
                    .map(|file| fs::read(file.current_path()).unwrap())
                    .collect::<Vec<_>>(),
                bytes
            );
            assert_eq!(
                db.get_provider_by_id(tool.provider_id(), tool.app().as_str())
                    .unwrap()
                    .unwrap()
                    .settings_config,
                settings
            );
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Configured
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn region_reconciliation_preserves_missing_connection_fields_until_explicit_restore() {
    for (from, to) in [("ofox.ai", "ofox.io"), ("ofox.io", "ofox.ai")] {
        for (tool, saved) in binding_cases() {
            let _home = Home::new();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(from.into()))
                .unwrap();
            let db = db_for(tool, saved);
            region_bind_saved_template(&db, tool).await;
            let provider = region_provider_snapshot(&db, tool);
            let record = db
                .get_bind_record(tool.record_key())
                .unwrap()
                .unwrap()
                .record;
            region_remove_connection_field(tool);
            let partial = region_live_snapshot(tool);
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(to.into()))
                .unwrap();
            assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(
                region_live_snapshot(tool),
                partial,
                "{} does not restore missing fields",
                tool.label()
            );
            assert_eq!(region_provider_snapshot(&db, tool), provider);
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Missing,
                "{} {from} -> {to}",
                tool.label()
            );
            let bytes: Vec<_> = tool
                .files()
                .iter()
                .map(|file| fs::read(file.current_path()).unwrap())
                .collect();
            crate::settings::reload_settings().unwrap();
            assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(
                tool.files()
                    .iter()
                    .map(|file| fs::read(file.current_path()).unwrap())
                    .collect::<Vec<_>>(),
                bytes
            );
            restore_missing_binding(&db, tool, tool.app().as_str(), &[], KEY)
                .await
                .unwrap();
            assert_eq!(
                region_live_endpoint(tool).as_deref(),
                Some(region_expected_url(tool, to).as_str())
            );
            assert_eq!(
                region_provider_snapshot(&db, tool),
                provider,
                "{} restores saved model",
                tool.label()
            );
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Configured
            );
            assert_eq!(
                db.get_bind_record(tool.record_key())
                    .unwrap()
                    .unwrap()
                    .record,
                record
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn region_reconciliation_keeps_deleted_directories_missing_until_explicit_restore() {
    for (from, to) in [("ofox.ai", "ofox.io"), ("ofox.io", "ofox.ai")] {
        for (tool, saved) in binding_cases() {
            let _home = Home::new();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(from.into()))
                .unwrap();
            let db = db_for(tool, saved);
            region_bind_saved_template(&db, tool).await;
            let provider = region_provider_snapshot(&db, tool);
            let record = db
                .get_bind_record(tool.record_key())
                .unwrap()
                .unwrap()
                .record;
            let directory = tool.files()[0]
                .current_path()
                .parent()
                .unwrap()
                .to_path_buf();
            fs::remove_dir_all(&directory).unwrap();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(to.into()))
                .unwrap();
            assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            crate::settings::reload_settings().unwrap();
            assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert!(
                !directory.exists(),
                "{} region change must not mkdir",
                tool.label()
            );
            assert_eq!(region_provider_snapshot(&db, tool), provider);
            assert_eq!(
                db.get_bind_record(tool.record_key())
                    .unwrap()
                    .unwrap()
                    .record,
                record
            );
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Missing
            );
            restore_missing_binding(&db, tool, tool.app().as_str(), &[], KEY)
                .await
                .unwrap();
            assert_eq!(
                region_live_endpoint(tool).as_deref(),
                Some(region_expected_url(tool, to).as_str())
            );
            assert_eq!(region_provider_snapshot(&db, tool), provider);
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Configured
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn region_reconciliation_preserves_custom_or_pseudo_live_endpoints_and_can_retry() {
    for (from, to) in [("ofox.ai", "ofox.io"), ("ofox.io", "ofox.ai")] {
        for (tool, saved) in binding_cases() {
            for invalid in [
                format!(
                    "https://user.example/{}",
                    region_expected_url(tool, from).rsplit('/').next().unwrap()
                ),
                format!(
                    "https://api.{from}.evil.example/{}",
                    region_expected_url(tool, from).rsplit('/').next().unwrap()
                ),
                format!(
                    "{}?redirect=https://api.{from}",
                    region_expected_url(tool, from)
                ),
            ] {
                let _home = Home::new();
                crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(from.into()))
                    .unwrap();
                let db = db_for(tool, saved.clone());
                region_bind_saved_template(&db, tool).await;
                let path = tool.files()[0].current_path();
                let original = fs::read_to_string(&path).unwrap();
                let conflicting = original.replace(&region_expected_url(tool, from), &invalid);
                assert_ne!(conflicting, original);
                fs::write(&path, &conflicting).unwrap();
                let settings = db
                    .get_provider_by_id(tool.provider_id(), tool.app().as_str())
                    .unwrap()
                    .unwrap()
                    .settings_config;
                let record = db
                    .get_bind_record(tool.record_key())
                    .unwrap()
                    .unwrap()
                    .record;
                crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(to.into()))
                    .unwrap();
                assert!(!crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
                assert_eq!(fs::read_to_string(&path).unwrap(), conflicting);
                assert_eq!(
                    db.get_provider_by_id(tool.provider_id(), tool.app().as_str())
                        .unwrap()
                        .unwrap()
                        .settings_config,
                    settings
                );
                assert_eq!(
                    db.get_bind_record(tool.record_key())
                        .unwrap()
                        .unwrap()
                        .record,
                    record
                );
                crate::settings::reload_settings().unwrap();
                assert!(!crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
                assert_eq!(fs::read_to_string(&path).unwrap(), conflicting);
                fs::write(&path, original).unwrap();
                assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
                assert_eq!(
                    region_live_endpoint(tool).as_deref(),
                    Some(region_expected_url(tool, to).as_str())
                );
                assert_eq!(
                    status::binding_status(&db, tool, Some(KEY)).await.status,
                    status::BindingStatus::Configured
                );
            }
        }
    }
}

#[tokio::test]
#[serial]
async fn region_reconciliation_isolates_direct_tool_and_workbuddy_conflicts() {
    for (from, to) in [("ofox.ai", "ofox.io"), ("ofox.io", "ofox.ai")] {
        for conflict_is_workbuddy in [false, true] {
            let _home = Home::new();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(from.into()))
                .unwrap();
            let db = db_for(Tool::Claude, claude_template(Some("saved/claude")));
            region_bind_saved_template(&db, Tool::Claude).await;
            let selection = crate::workbuddy_config::WorkBuddyModelSelection {
                id: "saved/workbuddy".into(),
                name: "Saved WorkBuddy".into(),
                supports_tool_call: true,
                supports_images: true,
                supports_reasoning: true,
            };
            crate::workbuddy_config::sync_selected_models(&db, KEY, &[selection])
                .await
                .unwrap();
            let path = if conflict_is_workbuddy {
                crate::workbuddy_config::models_path()
            } else {
                claude::settings_path()
            };
            let original = fs::read_to_string(&path).unwrap();
            let conflicting =
                original.replace(&format!("https://api.{from}"), "https://user.example");
            fs::write(&path, &conflicting).unwrap();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(to.into()))
                .unwrap();
            assert!(!crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(fs::read_to_string(&path).unwrap(), conflicting);
            if conflict_is_workbuddy {
                assert_eq!(
                    region_live_endpoint(Tool::Claude).as_deref(),
                    Some(region_expected_url(Tool::Claude, to).as_str())
                );
            } else {
                let models: Value = serde_json::from_str(
                    &fs::read_to_string(crate::workbuddy_config::models_path()).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    models[0]["url"],
                    format!("https://api.{to}/v1/chat/completions")
                );
                assert_eq!(models[0]["apiKey"], KEY);
                assert_eq!(models[0]["name"], "Saved WorkBuddy");
                assert_eq!(models[0]["supportsImages"], true);
            }
            crate::settings::reload_settings().unwrap();
            assert!(!crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(fs::read_to_string(&path).unwrap(), conflicting);
            fs::write(&path, original).unwrap();
            assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(
                status::binding_status(&db, Tool::Claude, Some(KEY))
                    .await
                    .status,
                status::BindingStatus::Configured
            );
            assert_eq!(
                crate::workbuddy_config::binding_status(&db).await.status,
                status::BindingStatus::Configured
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn startup_existing_repair_accepts_only_unchanged_saved_managed_fields() {
    for tool in [Tool::Claude, Tool::Codex] {
        for modified in [None, Some(KEY), Some("saved-model")] {
            let _home = Home::new();
            let saved = binding_cases()
                .into_iter()
                .find(|(case, _)| *case == tool)
                .unwrap()
                .1;
            let db = db_for(tool, saved);
            region_bind_saved_template(&db, tool).await;
            let path = tool.files()[0].current_path();
            let record = db
                .get_bind_record(tool.record_key())
                .unwrap()
                .unwrap()
                .record;
            let original = fs::read_to_string(&path).unwrap();
            let changed = match modified {
                None => original,
                Some(KEY) => original.replace(KEY, "sk-user-changed"),
                Some(_) if tool == Tool::Claude => {
                    original.replace("anthropic/claude-saved", "user/changed-model")
                }
                Some(_) => original.replace("openai/gpt-6-luna", "user/changed-model"),
            };
            fs::write(&path, &changed).unwrap();
            set_current_provider(&db, &tool.app(), tool.official_id().unwrap()).unwrap();
            let result = bind_existing(&db, tool, tool.app().as_str(), KEY).await;
            if modified.is_none() {
                result.unwrap();
                assert_eq!(
                    current_provider(&db, tool).as_deref(),
                    Some(tool.provider_id())
                );
            } else {
                assert!(result.is_err());
                assert_eq!(current_provider(&db, tool).as_deref(), tool.official_id());
            }
            assert_eq!(fs::read_to_string(&path).unwrap(), changed);
            assert_eq!(
                db.get_bind_record(tool.record_key())
                    .unwrap()
                    .unwrap()
                    .record,
                record
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn region_reconciliation_preserves_external_models_and_keys_with_canonical_endpoints() {
    for (from, to) in [("ofox.ai", "ofox.io"), ("ofox.io", "ofox.ai")] {
        for (tool, saved) in binding_cases() {
            let _home = Home::new();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(from.into()))
                .unwrap();
            let db = db_for(tool, saved);
            region_bind_saved_template(&db, tool).await;
            let path = tool.files()[0].current_path();
            let original = fs::read_to_string(&path).unwrap();
            let saved_model = match tool {
                Tool::Codex => "openai/gpt-6-luna",
                Tool::Claude => "anthropic/claude-saved",
                Tool::Gemini => "gemini-saved",
                Tool::OpenCode => "openai/gpt-saved",
                Tool::OpenClaw | Tool::Hermes => "openai/gpt-x",
            };
            let changed = original
                .replace(KEY, "sk-user-changed")
                .replace(saved_model, "user/changed-model");
            assert!(changed.contains("sk-user-changed"));
            assert!(changed.contains("user/changed-model"));
            fs::write(&path, changed).unwrap();
            let live = region_live_snapshot(tool);
            let provider = region_provider_snapshot(&db, tool);
            let record = db
                .get_bind_record(tool.record_key())
                .unwrap()
                .unwrap()
                .record;
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(to.into()))
                .unwrap();
            assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(
                region_live_endpoint(tool).as_deref(),
                Some(region_expected_url(tool, to).as_str())
            );
            assert_eq!(
                region_live_snapshot(tool),
                live,
                "{} retains external model and key",
                tool.label()
            );
            assert_eq!(region_provider_snapshot(&db, tool), provider);
            assert_eq!(
                db.get_bind_record(tool.record_key())
                    .unwrap()
                    .unwrap()
                    .record,
                record
            );
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Modified
            );
            let bytes = fs::read(&path).unwrap();
            assert!(
                restore_missing_binding(&db, tool, tool.app().as_str(), &[], KEY)
                    .await
                    .is_err()
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
            crate::settings::reload_settings().unwrap();
            assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Modified
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn region_reconciliation_preserves_custom_saved_template_endpoints_and_can_retry() {
    for (from, to) in [("ofox.ai", "ofox.io"), ("ofox.io", "ofox.ai")] {
        for (tool, saved) in binding_cases() {
            let _home = Home::new();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(from.into()))
                .unwrap();
            let db = db_for(tool, saved);
            region_bind_saved_template(&db, tool).await;
            let original = db
                .get_provider_by_id(tool.provider_id(), tool.app().as_str())
                .unwrap()
                .unwrap()
                .settings_config;
            let custom: Value =
                serde_json::from_str(&serde_json::to_string(&original).unwrap().replace(
                    &region_expected_url(tool, from),
                    &format!("{}?source=user", region_expected_url(tool, from)),
                ))
                .unwrap();
            db.update_provider_settings_config(tool.app().as_str(), tool.provider_id(), &custom)
                .unwrap();
            let files: Vec<_> = tool
                .files()
                .iter()
                .map(|file| fs::read(file.current_path()).unwrap())
                .collect();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(to.into()))
                .unwrap();
            assert!(!crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(
                db.get_provider_by_id(tool.provider_id(), tool.app().as_str())
                    .unwrap()
                    .unwrap()
                    .settings_config,
                custom
            );
            assert_eq!(
                tool.files()
                    .iter()
                    .map(|file| fs::read(file.current_path()).unwrap())
                    .collect::<Vec<_>>(),
                files
            );
            db.update_provider_settings_config(tool.app().as_str(), tool.provider_id(), &original)
                .unwrap();
            assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(
                region_live_endpoint(tool).as_deref(),
                Some(region_expected_url(tool, to).as_str())
            );
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Configured
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn region_reconciliation_does_not_recreate_deleted_endpoint_fields() {
    for (from, to) in [("ofox.ai", "ofox.io"), ("ofox.io", "ofox.ai")] {
        for (tool, saved) in binding_cases() {
            let _home = Home::new();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(from.into()))
                .unwrap();
            let db = db_for(tool, saved);
            region_bind_saved_template(&db, tool).await;
            let before_provider = region_provider_snapshot(&db, tool);
            let path = tool.files()[0].current_path();
            let source = fs::read_to_string(&path).unwrap();
            let deleted = match tool {
                Tool::Codex => {
                    let mut doc = source.parse::<toml_edit::DocumentMut>().unwrap();
                    doc["model_providers"]["ofox"]
                        .as_table_like_mut()
                        .unwrap()
                        .remove("base_url");
                    doc.to_string()
                }
                Tool::Gemini => env_file::set_value(&source, "GOOGLE_GEMINI_BASE_URL", None),
                Tool::Hermes => {
                    let mut config = crate::hermes_config::parse_config_text(&source).unwrap();
                    let provider = config["custom_providers"]
                        .as_sequence_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|provider| provider["name"].as_str() == Some("ofox-hermes"))
                        .unwrap();
                    provider
                        .as_mapping_mut()
                        .unwrap()
                        .remove(serde_yaml::Value::String("base_url".into()));
                    serde_yaml::to_string(&config).unwrap()
                }
                _ => {
                    let mut config = json_file::parse_object(Some(&source), "regression").unwrap();
                    let path: json_file::JsonPath = match tool {
                        Tool::Claude => &["env", "ANTHROPIC_BASE_URL"],
                        Tool::OpenCode => &["provider", "ofox-opencode", "options", "baseURL"],
                        Tool::OpenClaw => &["models", "providers", "ofox-openclaw", "baseUrl"],
                        _ => unreachable!(),
                    };
                    assert!(json_file::remove(&mut config, path));
                    serde_json::to_string_pretty(&config).unwrap()
                }
            };
            fs::write(&path, &deleted).unwrap();
            crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(to.into()))
                .unwrap();
            assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                deleted,
                "{} must not restore URL",
                tool.label()
            );
            assert_eq!(region_provider_snapshot(&db, tool), before_provider);
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Missing
            );
            restore_missing_binding(&db, tool, tool.app().as_str(), &[], KEY)
                .await
                .unwrap();
            assert_eq!(
                region_live_endpoint(tool),
                Some(region_expected_url(tool, to))
            );
            assert_eq!(
                status::binding_status(&db, tool, Some(KEY)).await.status,
                status::BindingStatus::Configured
            );
        }
    }
}

#[test]
#[serial]
fn binding_test_home_isolates_platform_paths_and_restores_parent_environment() {
    let names = [
        "HOME",
        "USERPROFILE",
        "CC_SWITCH_TEST_HOME",
        "LOCALAPPDATA",
        "APPDATA",
        "HERMES_HOME",
        "OFOX_USE_LOCAL",
    ];
    let inherited: Vec<_> = names.iter().map(std::env::var_os).collect();
    {
        let home = Home::new();
        assert_eq!(crate::config::get_home_dir(), home.dir.path());
        assert_eq!(
            std::env::var_os("LOCALAPPDATA"),
            Some(
                home.dir
                    .path()
                    .join("AppData")
                    .join("Local")
                    .into_os_string()
            )
        );
        assert_eq!(
            std::env::var_os("APPDATA"),
            Some(
                home.dir
                    .path()
                    .join("AppData")
                    .join("Roaming")
                    .into_os_string()
            )
        );
        assert!(std::env::var_os("HERMES_HOME").is_none());
        assert!(std::env::var_os("OFOX_USE_LOCAL").is_none());
        assert_eq!(
            crate::workbuddy_config::models_path(),
            home.dir.path().join(".workbuddy").join("models.json")
        );
        assert_eq!(
            tool_for("codex").unwrap().0.files()[0].current_path(),
            tool_for("chatgpt").unwrap().0.files()[0].current_path()
        );
        #[cfg(windows)]
        assert_eq!(
            crate::hermes_config::get_hermes_config_path(),
            home.dir
                .path()
                .join("AppData")
                .join("Local")
                .join("hermes")
                .join("config.yaml")
        );
        #[cfg(not(windows))]
        assert_eq!(
            crate::hermes_config::get_hermes_config_path(),
            home.dir.path().join(".hermes").join("config.yaml")
        );
    }
    assert_eq!(
        names.iter().map(std::env::var_os).collect::<Vec<_>>(),
        inherited
    );
}

#[tokio::test]
#[serial]
async fn windows_crlf_configs_survive_region_changes_and_restore_original_bytes() {
    for (tool, saved) in binding_cases() {
        let _home = Home::new();
        let db = db_for(tool, saved);
        let originals: Vec<_> = tool
            .files()
            .iter()
            .map(|file| original_for(*file).replace('\n', "\r\n"))
            .collect();
        for (file, original) in tool.files().iter().zip(&originals) {
            write(&file.current_path(), original);
        }
        bind(&db, tool, tool.app().as_str(), KEY).await.unwrap();
        let before = region_live_snapshot(tool);
        crate::settings::mutate_settings(|settings| settings.ofox_apex = Some("ofox.io".into()))
            .unwrap();
        assert!(crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
        assert_eq!(
            region_live_snapshot(tool),
            before,
            "{} CRLF user content",
            tool.label()
        );
        assert_eq!(
            status::binding_status(&db, tool, Some(KEY)).await.status,
            status::BindingStatus::Configured
        );
        unbind(&db, tool, tool.app().as_str(), &[], false)
            .await
            .unwrap();
        for (file, original) in tool.files().iter().zip(&originals) {
            assert_eq!(
                fs::read_to_string(file.current_path()).unwrap(),
                *original,
                "{} byte restore",
                tool.label()
            );
        }
    }
}

#[cfg(windows)]
fn windows_exclusive_config_handle(path: &Path) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
        .expect("lock temporary configuration without sharing")
}

/// Sharing violations are real Windows read-access failures. They must not be
/// confused with NotFound, which would permit recovery to overwrite the file.
#[cfg(windows)]
#[tokio::test]
#[serial]
async fn windows_locked_binding_is_unknown_and_cannot_restore_or_unbind() {
    for (tool, saved) in binding_cases() {
        let _home = Home::new();
        let db = db_for(tool, saved);
        for file in tool.files() {
            write(&file.current_path(), original_for(*file));
        }
        bind(&db, tool, tool.app().as_str(), KEY).await.unwrap();
        let path = tool.files()[0].current_path();
        let bytes = fs::read(&path).unwrap();
        let record = db
            .get_bind_record(tool.record_key())
            .unwrap()
            .unwrap()
            .record;
        let previous = db
            .get_provider_by_id(tool.provider_id(), tool.app().as_str())
            .unwrap()
            .unwrap()
            .settings_config;
        let locked = windows_exclusive_config_handle(&path);
        let read_error = fs::read(&path).unwrap_err();
        assert_ne!(read_error.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            read_error.raw_os_error(),
            Some(32),
            "{} sharing violation",
            tool.label()
        );
        let health = status::binding_status(&db, tool, Some(KEY)).await;
        assert_eq!(health.status, status::BindingStatus::Unknown);
        assert!(health.missing_files.is_empty());
        assert!(!serde_json::to_string(&health).unwrap().contains(KEY));
        assert!(
            restore_missing_binding(&db, tool, tool.app().as_str(), &[], KEY)
                .await
                .is_err()
        );
        let mut updated = previous.clone();
        updated["regressionModelSave"] = json!(true);
        assert!(persist_bound_settings(&db, tool, KEY, &previous, &updated)
            .await
            .is_err());
        assert!(unbind(&db, tool, tool.app().as_str(), &[], false)
            .await
            .is_err());
        crate::settings::mutate_settings(|settings| settings.ofox_apex = Some("ofox.io".into()))
            .unwrap();
        assert!(!crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
        assert_eq!(
            db.get_bind_record(tool.record_key())
                .unwrap()
                .unwrap()
                .record,
            record
        );
        assert_eq!(
            db.get_provider_by_id(tool.provider_id(), tool.app().as_str())
                .unwrap()
                .unwrap()
                .settings_config,
            previous
        );
        assert_eq!(
            current_provider(&db, tool).as_deref(),
            Some(tool.provider_id())
        );
        drop(locked);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[cfg(windows)]
#[tokio::test]
#[serial]
async fn windows_locked_workbuddy_is_unknown_and_keeps_saved_binding() {
    let _home = Home::new();
    let db = Database::memory().unwrap();
    let selection = crate::workbuddy_config::WorkBuddyModelSelection {
        id: "saved".into(),
        name: "Saved model".into(),
        supports_tool_call: true,
        supports_images: false,
        supports_reasoning: true,
    };
    crate::workbuddy_config::sync_selected_models(&db, KEY, std::slice::from_ref(&selection))
        .await
        .unwrap();
    let path = crate::workbuddy_config::models_path();
    let bytes = fs::read(&path).unwrap();
    let record = db.get_bind_record("workbuddy").unwrap().unwrap().record;
    let locked = windows_exclusive_config_handle(&path);
    let read_error = fs::read(&path).unwrap_err();
    assert_ne!(read_error.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(read_error.raw_os_error(), Some(32));
    let health = crate::workbuddy_config::binding_status(&db).await;
    assert_eq!(health.status, status::BindingStatus::Unknown);
    assert!(health.missing_files.is_empty());
    assert!(!serde_json::to_string(&health).unwrap().contains(KEY));
    assert!(crate::workbuddy_config::restore_missing_binding(&db, KEY)
        .await
        .is_err());
    assert!(
        crate::workbuddy_config::update_selected_models(&db, KEY, &[selection])
            .await
            .is_err()
    );
    assert!(crate::workbuddy_config::unbind(&db, false).await.is_err());
    crate::settings::mutate_settings(|settings| settings.ofox_apex = Some("ofox.io".into()))
        .unwrap();
    assert!(!crate::commands::ofox_apex::reconcile_tool_endpoints(&db).await);
    assert_eq!(
        db.get_bind_record("workbuddy").unwrap().unwrap().record,
        record
    );
    drop(locked);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[cfg(windows)]
#[tokio::test]
#[serial]
async fn windows_locked_partial_gemini_settings_blocks_recovery_and_model_saving() {
    let home = Home::new();
    let db = db_for(Tool::Gemini, gemini_template(Some("google/saved")));
    let env = home.gemini(".env");
    let settings = home.gemini("settings.json");
    write(&env, GEMINI_ENV);
    write(&settings, GEMINI_GOOGLE_LOGIN_SETTINGS);
    bind(&db, Tool::Gemini, "gemini", KEY).await.unwrap();
    let env_bytes = fs::read(&env).unwrap();
    let settings_bytes = fs::read(&settings).unwrap();
    let record = db.get_bind_record("gemini").unwrap().unwrap().record;
    let previous = db
        .get_provider_by_id("ofox-gemini", "gemini")
        .unwrap()
        .unwrap()
        .settings_config;
    let locked = windows_exclusive_config_handle(&settings);
    assert_eq!(fs::read(&settings).unwrap_err().raw_os_error(), Some(32));
    assert_eq!(fs::read(&env).unwrap(), env_bytes);
    let health = status::binding_status(&db, Tool::Gemini, Some(KEY)).await;
    assert_eq!(health.status, status::BindingStatus::Unknown);
    assert!(health.missing_files.is_empty());
    assert!(
        restore_missing_binding(&db, Tool::Gemini, "gemini", &[], KEY)
            .await
            .is_err()
    );
    let mut updated = previous.clone();
    updated["env"]["GEMINI_MODEL"] = json!("google/next");
    assert!(
        persist_bound_settings(&db, Tool::Gemini, KEY, &previous, &updated)
            .await
            .is_err()
    );
    assert!(unbind(&db, Tool::Gemini, "gemini", &[], false)
        .await
        .is_err());
    assert_eq!(
        db.get_provider_by_id("ofox-gemini", "gemini")
            .unwrap()
            .unwrap()
            .settings_config,
        previous
    );
    assert_eq!(
        db.get_bind_record("gemini").unwrap().unwrap().record,
        record
    );
    assert_eq!(
        fs::read(&env).unwrap(),
        env_bytes,
        "failed multi-file unbind rolls back the readable file"
    );
    drop(locked);
    assert_eq!(fs::read(&settings).unwrap(), settings_bytes);
}
