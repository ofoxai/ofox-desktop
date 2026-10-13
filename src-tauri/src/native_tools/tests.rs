//! Magpie-derived fixtures run only under a temporary, isolated home.
use super::*;
use serial_test::serial;
use std::{ffi::OsString, fs, path::Path};

struct Home {
    dir: tempfile::TempDir,
    saved: Vec<(&'static str, Option<OsString>)>,
}
impl Home {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("ofox native 中文 ")
            .tempdir()
            .unwrap();
        let vars = [
            ("CC_SWITCH_TEST_HOME", Some(dir.path().to_path_buf())),
            ("PI_CODING_AGENT_DIR", None),
            ("DSH_HOME", None),
            ("OFOX_USE_LOCAL", None),
        ];
        let saved = vars
            .into_iter()
            .map(|(name, next)| {
                let old = std::env::var_os(name);
                if let Some(path) = next {
                    std::env::set_var(name, path);
                } else {
                    std::env::remove_var(name);
                }
                (name, old)
            })
            .collect();
        crate::settings::reload_settings().unwrap();
        Self { dir, saved }
    }
    fn pi(&self, name: &str) -> PathBuf {
        self.dir.path().join(".pi/agent").join(name)
    }
    fn dsh(&self, name: &str) -> PathBuf {
        self.dir.path().join(".dsh").join(name)
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        for (name, value) in &self.saved {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
        let _ = crate::settings::reload_settings();
    }
}
fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn model(id: &str, endpoint: &str) -> FetchedModel {
    FetchedModel {
        id: id.into(),
        name: Some(id.into()),
        owned_by: None,
        pricing_prompt: None,
        supported_endpoints: Some(vec![endpoint.into()]),
        supported_parameters: Some(vec!["tools".into()]),
        input_modalities: Some(vec!["text".into()]),
        output_modalities: None,
    }
}
fn catalog() -> Vec<FetchedModel> {
    vec![
        model("deepseek/chat", "/v1/chat/completions"),
        model("openai/response", "/v1/responses"),
        model("anthropic/claude", "/anthropic/v1/messages"),
    ]
}
const KEY: &str = "sk-of-ISOLATED-NATIVE-TEST";
const PROFILE:&str="# Your patch layer for this dsh profile.\n- id: llm-deepseek\n  config:\n    apiKeyEnv: DEEPSEEK_API_KEY\n    thinking: enabled\n- id: llm-pi-ai\n  name: \"@deepseek-ai/dsh-llm-pi-ai\"\n  config:\n    providers:\n      mine:\n        displayName: Mine\n        apiKeyEnv: MINE_API_KEY\n        api: openai-completions\n        baseURL: https://mine.example/v1\n        models:\n          - id: m1\n";

#[test]
#[serial]
fn resolver_uses_native_overrides_and_refuses_relative_paths() {
    let h = Home::new();
    assert_eq!(config_dir(BindableTool::Pi).unwrap(), h.pi(""));
    std::env::set_var("PI_CODING_AGENT_DIR", "~/pi custom");
    assert_eq!(
        config_dir(BindableTool::Pi).unwrap(),
        h.dir.path().join("pi custom")
    );
    std::env::set_var("DSH_HOME", h.dir.path().join("custom dsh"));
    assert_eq!(
        config_dir(BindableTool::Dsh).unwrap(),
        h.dir.path().join("custom dsh")
    );
    std::env::set_var("DSH_HOME", "relative");
    assert!(config_dir(BindableTool::Dsh).is_err());
}

#[tokio::test]
#[serial]
async fn pi_native_protocols_scope_and_precise_restore() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    let original="{\n // my custom scope\n \"enabledModels\": [\"anthropic/*:high\"],\n \"theme\": \"mine\"\n}\n";
    write(&h.pi("settings.json"), original);
    bind_with_catalog(&db, BindableTool::Pi, KEY, &all[0], &all)
        .await
        .unwrap();
    let settings = common::parse(
        Some(&fs::read_to_string(h.pi("settings.json")).unwrap()),
        Format::Json,
    )
    .unwrap();
    assert_eq!(
        settings["enabledModels"],
        json!(["anthropic/*:high", "ofox-pi/deepseek/chat"])
    );
    let models = common::parse(
        Some(&fs::read_to_string(h.pi("models.json")).unwrap()),
        Format::Json,
    )
    .unwrap();
    assert_eq!(
        models["providers"]["ofox-pi"]["models"][1]["api"],
        "openai-responses"
    );
    assert_eq!(
        models["providers"]["ofox-pi"]["models"][2]["baseUrl"],
        crate::ofox_endpoints::anthropic_base_url()
    );
    assert!(models["providers"]["ofox-pi"]["models"][0]
        .get("contextWindow")
        .is_none());
    let snap = snapshot(&db, BindableTool::Pi).await.unwrap();
    assert_eq!(snap.model_ids, ["deepseek/chat"]);
    assert!(!serde_json::to_string(&snap).unwrap().contains(KEY));
    set_model(&db, BindableTool::Pi, KEY, &all[1])
        .await
        .unwrap();
    assert_eq!(
        snapshot(&db, BindableTool::Pi).await.unwrap().model_ids,
        ["openai/response"]
    );
    unbind(&db, BindableTool::Pi, false).await.unwrap();
    assert_eq!(fs::read_to_string(h.pi("settings.json")).unwrap(), original);
    assert!(!h.pi("models.json").exists());
}

#[tokio::test]
#[serial]
async fn pi_preserves_user_scope_additions_and_reports_auth_conflicts() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    write(&h.pi("settings.json"), "{\"enabledModels\":[\"mine/*\"]}");
    bind_with_catalog(&db, BindableTool::Pi, KEY, &all[0], &all)
        .await
        .unwrap();
    let mut settings = common::parse(
        common::read(&h.pi("settings.json")).unwrap().as_deref(),
        Format::Json,
    )
    .unwrap();
    settings["enabledModels"]
        .as_array_mut()
        .unwrap()
        .push(json!("another/*:low"));
    settings["theme"] = json!("new");
    write(
        &h.pi("settings.json"),
        &serde_json::to_string(&settings).unwrap(),
    );
    unbind(&db, BindableTool::Pi, false).await.unwrap();
    let settings = common::parse(
        common::read(&h.pi("settings.json")).unwrap().as_deref(),
        Format::Json,
    )
    .unwrap();
    assert_eq!(
        settings["enabledModels"],
        json!(["mine/*", "another/*:low"])
    );
    assert_eq!(settings["theme"], "new");
    write(
        &h.pi("auth.json"),
        "{\"ofox-pi\":{\"type\":\"api_key\",\"key\":\"personal\"}}",
    );
    assert!(bind_with_catalog(&db, BindableTool::Pi, KEY, &all[0], &all)
        .await
        .is_err());
    assert!(!h.pi("models.json").exists());
}

#[tokio::test]
#[serial]
async fn dsh_profiles_routes_credentials_and_model_scope_undo() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    let web = h.dsh("profiles/web/cordis.patch.yml");
    let tui = h.dsh("profiles/tui/cordis.patch.yml");
    write(&web, PROFILE);
    write(&tui, "# TUI profile\n[]\n");
    let meta="# Enable bundled plugins\n- id: llm-pi-ai\n  disabled: false\n- id: agent-default-model\n  disabled: false\n";
    write(&h.dsh("cordis.patch.yml"), meta);
    let creds="version: 1\nrefs:\n  # unrelated ref comment\n  MINE_API_KEY: mine\nrecords:\n  llm-pi-ai/bedrock:\n    kind: api-key\n    env: {AWS_PROFILE: prod}\n";
    write(&h.dsh(".credentials.yaml"), creds);
    bind_with_catalog(&db, BindableTool::Dsh, KEY, &all[0], &all)
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(h.dsh("cordis.patch.yml")).unwrap(), meta);
    let value = dsh::patch_for_test(&web);
    assert_eq!(
        value["llm-pi-ai"]["config"]["providers"]["mine"]["baseURL"],
        "https://mine.example/v1"
    );
    assert_eq!(
        value["llm-pi-ai"]["config"]["providers"]["ofox-dsh-responses"]["models"][0]["id"],
        "openai/response"
    );
    assert_eq!(
        value["llm-pi-ai"]["config"]["providers"]["ofox-dsh-anthropic"]["baseURL"],
        crate::ofox_endpoints::anthropic_base_url()
    );
    assert!(fs::read_to_string(h.dsh(".credentials.yaml"))
        .unwrap()
        .contains("# unrelated ref comment"));
    let mut value = value;
    value["agent-default-model"]["config"]["provider"] = json!("ofox-dsh-responses");
    value["agent-default-model"]["config"]["model"] = json!("openai/response");
    let current = fs::read_to_string(&web).unwrap();
    let before = common::parse(Some(&current), Format::Patch).unwrap();
    write(
        &web,
        &common::render(&current, Format::Patch, &before, &value).unwrap(),
    );
    assert_eq!(
        binding_status(&db, BindableTool::Dsh).await.status,
        BindingStatus::Configured
    );
    let old = snapshot(&db, BindableTool::Dsh).await.unwrap();
    assert_eq!(old.model_ids.len(), 2);
    set_model(&db, BindableTool::Dsh, KEY, &all[2])
        .await
        .unwrap();
    restore_scope_models(&db, BindableTool::Dsh, KEY, &old.scope_models, &all)
        .await
        .unwrap();
    assert_eq!(
        snapshot(&db, BindableTool::Dsh).await.unwrap().scope_models,
        old.scope_models
    );
    write(&h.dsh("profiles/new/cordis.patch.yml"), "[]\n");
    set_model(&db, BindableTool::Dsh, KEY, &all[1])
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(h.dsh("profiles/new/cordis.patch.yml")).unwrap(),
        "[]\n"
    );
    unbind(&db, BindableTool::Dsh, false).await.unwrap();
    assert_eq!(fs::read_to_string(&web).unwrap(), PROFILE);
    assert_eq!(fs::read_to_string(&tui).unwrap(), "# TUI profile\n[]\n");
    assert_eq!(
        fs::read_to_string(h.dsh(".credentials.yaml")).unwrap(),
        creds
    );
    assert!(!h.dsh(".env").exists());
}

#[tokio::test]
#[serial]
async fn missing_recovery_key_rotation_region_and_conflicts() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    bind_with_catalog(&db, BindableTool::Pi, KEY, &all[0], &all)
        .await
        .unwrap();
    let before = revision_material(&db, BindableTool::Pi).unwrap();
    fs::remove_file(h.pi("settings.json")).unwrap();
    assert_eq!(
        binding_status(&db, BindableTool::Pi).await.status,
        BindingStatus::Missing
    );
    restore_missing(&db, BindableTool::Pi, KEY).await.unwrap();
    replace_key(&db, BindableTool::Pi, KEY, "sk-of-NEW")
        .await
        .unwrap();
    assert_ne!(revision_material(&db, BindableTool::Pi).unwrap(), before);
    for apex in ["ofox.io", "ofox.ai"] {
        crate::settings::mutate_settings(|s| s.ofox_apex = Some(apex.into())).unwrap();
        reconcile_endpoints(&db).await.unwrap();
        let config = common::parse(
            common::read(&h.pi("models.json")).unwrap().as_deref(),
            Format::Json,
        )
        .unwrap();
        assert_eq!(
            config["providers"]["ofox-pi"]["baseUrl"],
            crate::ofox_endpoints::openai_v1_base_url()
        );
        assert_eq!(
            config["providers"]["ofox-pi"]["models"][2]["baseUrl"],
            crate::ofox_endpoints::anthropic_base_url()
        );
    }
    let mut config = common::parse(
        common::read(&h.pi("models.json")).unwrap().as_deref(),
        Format::Json,
    )
    .unwrap();
    config["providers"]["ofox-pi"]["baseUrl"] = json!("https://user.example/v1");
    write(
        &h.pi("models.json"),
        &serde_json::to_string(&config).unwrap(),
    );
    let preserved = fs::read_to_string(h.pi("models.json")).unwrap();
    assert!(reconcile_endpoints(&db).await.is_err());
    assert!(replace_key(&db, BindableTool::Pi, "sk-of-NEW", "other")
        .await
        .is_err());
    assert_eq!(fs::read_to_string(h.pi("models.json")).unwrap(), preserved);
}

#[tokio::test]
#[serial]
async fn dsh_rejects_unknown_or_shadowing_credentials_without_partial_write() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    let profile = h.dsh("profiles/web/cordis.patch.yml");
    write(&profile, PROFILE);
    for credentials in [
        "OFOX_DSH_API_KEY: personal\n",
        "version: 2\nrefs: {}\n",
        "version: 1\nrefs:\n  OFOX_DSH_API_KEY: personal\n",
    ] {
        write(&h.dsh(".credentials.yaml"), credentials);
        assert!(
            bind_with_catalog(&db, BindableTool::Dsh, KEY, &all[0], &all)
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(&profile).unwrap(), PROFILE);
        assert!(!h.dsh(".env").exists());
    }
}

#[test]
#[serial]
fn absent_native_revision_and_new_tool_metadata_are_safe() {
    let _h = Home::new();
    let db = Database::memory().unwrap();
    for tool in [BindableTool::Pi, BindableTool::Dsh] {
        assert_eq!(revision_material(&db, tool).unwrap()["absent"], true);
        assert_eq!(
            serde_json::from_str::<BindableTool>(&serde_json::to_string(&tool).unwrap()).unwrap(),
            tool
        );
        assert_eq!(tool.as_str().parse::<BindableTool>().unwrap(), tool);
    }
    assert!("pi".parse::<crate::app_config::AppType>().is_err());
}

#[tokio::test]
#[serial]
async fn dsh_duplicate_config_rows_comment_only_and_flow_patches() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    let web = h.dsh("profiles/web/cordis.patch.yml");
    let duplicate="# Duplicate entries are full-field overlays\n- id: llm-pi-ai\n  config:\n    providers:\n      older: {baseURL: https://old.example}\n- id: llm-pi-ai\n  config: # keep my config comment\n    providers:\n      mine: # keep my provider comment\n        baseURL: https://mine.example/v1\n- id: llm-pi-ai\n  disabled: false\n";
    write(&web, duplicate);
    let comment = h.dsh("profiles/desktop/cordis.patch.yml");
    write(&comment, "# Your patch layer for this dsh profile.\n");
    let flow = h.dsh("profiles/tui/cordis.patch.yml");
    write(&flow,"# flow patch\n[{id: agent-default-model, config: {provider: deepseek-official, model: old}}]\n");
    bind_with_catalog(&db, BindableTool::Dsh, KEY, &all[0], &all)
        .await
        .unwrap();
    let text = fs::read_to_string(&web).unwrap();
    assert!(text.contains("# keep my config comment"));
    assert!(text.contains("# keep my provider comment"));
    assert!(text.contains("older: {baseURL: https://old.example}"));
    let value = dsh::patch_for_test(&web);
    assert!(value["llm-pi-ai"]["config"]["providers"]
        .get("older")
        .is_none());
    assert_eq!(
        value["llm-pi-ai"]["config"]["providers"]["mine"]["baseURL"],
        "https://mine.example/v1"
    );
    assert_eq!(value["llm-pi-ai"]["disabled"], false);
    unbind(&db, BindableTool::Dsh, false).await.unwrap();
    assert_eq!(fs::read_to_string(&web).unwrap(), duplicate);
    assert_eq!(
        fs::read_to_string(&comment).unwrap(),
        "# Your patch layer for this dsh profile.\n"
    );
    assert_eq!(fs::read_to_string(&flow).unwrap(),"# flow patch\n[{id: agent-default-model, config: {provider: deepseek-official, model: old}}]\n");
}

#[tokio::test]
#[serial]
async fn dsh_home_config_overlays_are_owned_only_when_present_at_bind() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    write(&h.dsh("profiles/web/cordis.patch.yml"), "[]\n");
    let home="# Shared choice\n- id: agent-default-model\n  config:\n    provider: deepseek-official\n    model: old\n    reasoningEffort: low\n- id: llm-pi-ai\n  config:\n    providers:\n      mine: {baseURL: https://mine.example/v1}\n";
    write(&h.dsh("cordis.patch.yml"), home);
    bind_with_catalog(&db, BindableTool::Dsh, KEY, &all[0], &all)
        .await
        .unwrap();
    let snapshot = snapshot(&db, BindableTool::Dsh).await.unwrap();
    assert_eq!(snapshot.scope_models.len(), 2);
    set_model(&db, BindableTool::Dsh, KEY, &all[1])
        .await
        .unwrap();
    let value = dsh::patch_for_test(&h.dsh("cordis.patch.yml"));
    assert_eq!(
        value["agent-default-model"]["config"]["model"],
        "openai/response"
    );
    assert_eq!(
        value["agent-default-model"]["config"]["reasoningEffort"],
        "low"
    );
    unbind(&db, BindableTool::Dsh, false).await.unwrap();
    assert_eq!(fs::read_to_string(h.dsh("cordis.patch.yml")).unwrap(), home);
}

#[tokio::test]
#[serial]
async fn dsh_region_restart_rotation_recovery_and_store_precedence() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    let profile = h.dsh("profiles/web/cordis.patch.yml");
    write(&profile, PROFILE);
    bind_with_catalog(&db, BindableTool::Dsh, KEY, &all[2], &all)
        .await
        .unwrap();
    for apex in ["ofox.io", "ofox.ai"] {
        crate::settings::mutate_settings(|s| s.ofox_apex = Some(apex.into())).unwrap();
        reconcile_endpoints(&db).await.unwrap();
        let loaded = common::load(&db, "dsh").unwrap().unwrap();
        assert_eq!(
            health(&loaded, BindableTool::Dsh).unwrap().status,
            BindingStatus::Configured
        );
        let value = dsh::patch_for_test(&profile);
        for (route, api) in dsh::routes_for_test() {
            assert_eq!(
                value["llm-pi-ai"]["config"]["providers"][route]["baseURL"],
                url(api)
            );
        }
        assert!(!reconcile_endpoints(&db).await.unwrap());
    }
    replace_key(&db, BindableTool::Dsh, KEY, "sk-of-ROTATED")
        .await
        .unwrap();
    let credentials = common::parse(
        common::read(&h.dsh(".credentials.yaml"))
            .unwrap()
            .as_deref(),
        Format::YamlMap,
    )
    .unwrap();
    assert_eq!(credentials["refs"]["OFOX_DSH_API_KEY"], "sk-of-ROTATED");
    assert_eq!(
        common::parse(
            common::read(&h.dsh(".env")).unwrap().as_deref(),
            Format::Env
        )
        .unwrap()["OFOX_DSH_API_KEY"],
        "sk-of-ROTATED"
    );
    fs::remove_file(&profile).unwrap();
    fs::remove_file(h.dsh(".credentials.yaml")).unwrap();
    assert_eq!(
        binding_status(&db, BindableTool::Dsh).await.status,
        BindingStatus::Missing
    );
    restore_missing(&db, BindableTool::Dsh, "sk-of-ROTATED")
        .await
        .unwrap();
    assert_eq!(
        snapshot(&db, BindableTool::Dsh).await.unwrap().model_ids,
        ["anthropic/claude"]
    );
    assert_eq!(
        binding_status(&db, BindableTool::Dsh).await.status,
        BindingStatus::Configured
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(h.dsh(".credentials.yaml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    write(
        &h.dsh(".credentials.yaml"),
        "version: 1\nrefs:\n  OFOX_DSH_API_KEY: personal-shadow\n",
    );
    let preserved = fs::read_to_string(&profile).unwrap();
    assert_eq!(
        binding_status(&db, BindableTool::Dsh).await.status,
        BindingStatus::Modified
    );
    assert!(replace_key(&db, BindableTool::Dsh, "sk-of-ROTATED", "next")
        .await
        .is_err());
    assert!(reconcile_endpoints(&db).await.is_err());
    assert_eq!(fs::read_to_string(&profile).unwrap(), preserved);
}

#[test]
#[serial]
fn file_write_and_database_failures_compensate_every_native_file() {
    let h = Home::new();
    let all = catalog();
    let directory = config_dir(BindableTool::Pi).unwrap();
    write(&h.pi("settings.json"), "{\"theme\":\"mine\"}\n");
    let mut record = pi::plan(&directory, KEY, &all[0], &all, None).unwrap();
    record.files.reverse();
    let db = Database::memory().unwrap();
    fs::create_dir_all(h.pi("models.json")).unwrap();
    assert!(common::commit(&db, "pi", &record).is_err());
    assert_eq!(
        fs::read_to_string(h.pi("settings.json")).unwrap(),
        "{\"theme\":\"mine\"}\n"
    );
    assert!(db.get_bind_record("pi").unwrap().is_none());
    fs::remove_dir(h.pi("models.json")).unwrap();
    // Real SQLite failure after both external files have been written.
    db.conn.lock().unwrap().execute_batch("CREATE TRIGGER native_record_failure BEFORE INSERT ON ofox_bind_snapshot BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    assert!(common::commit(&db, "pi", &record).is_err());
    assert_eq!(
        fs::read_to_string(h.pi("settings.json")).unwrap(),
        "{\"theme\":\"mine\"}\n"
    );
    assert!(!h.pi("models.json").exists());
    assert!(db.get_bind_record("pi").unwrap().is_none());
}

#[tokio::test]
#[serial]
async fn pi_recovery_restores_all_deleted_files_and_scope_patterns_need_no_additions() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    write(
        &h.pi("settings.json"),
        "{\"enabledModels\":[\"OFOX-PI/*/*:high\"]}\n",
    );
    bind_with_catalog(&db, BindableTool::Pi, KEY, &all[0], &all)
        .await
        .unwrap();
    assert_eq!(
        common::parse(
            common::read(&h.pi("settings.json")).unwrap().as_deref(),
            Format::Json
        )
        .unwrap()["enabledModels"],
        json!(["OFOX-PI/*/*:high"])
    );
    fs::remove_file(h.pi("settings.json")).unwrap();
    fs::remove_file(h.pi("models.json")).unwrap();
    restore_missing(&db, BindableTool::Pi, KEY).await.unwrap();
    assert_eq!(
        binding_status(&db, BindableTool::Pi).await.status,
        BindingStatus::Configured
    );
    assert_eq!(
        snapshot(&db, BindableTool::Pi).await.unwrap().model_ids,
        ["deepseek/chat"]
    );
    unbind(&db, BindableTool::Pi, false).await.unwrap();
    write(&h.pi("settings.json"), "{}\n");
    bind_with_catalog(&db, BindableTool::Pi, KEY, &all[0], &all)
        .await
        .unwrap();
    assert!(common::parse(
        common::read(&h.pi("settings.json")).unwrap().as_deref(),
        Format::Json
    )
    .unwrap()
    .get("enabledModels")
    .is_none());
}

#[test]
#[serial]
fn settings_roundtrip_retains_legacy_and_native_key_metadata() {
    let _h = Home::new();
    let mut settings = crate::settings::AppSettings::default();
    for tool in [
        BindableTool::Claude,
        BindableTool::Codex,
        BindableTool::Gemini,
        BindableTool::OpenCode,
        BindableTool::OpenClaw,
        BindableTool::Hermes,
        BindableTool::WorkBuddy,
        BindableTool::Dsh,
        BindableTool::Pi,
    ] {
        settings.ofox_api_keys.push(crate::settings::ApiKeyMeta {
            tool,
            key_id: format!("id-{}", tool.as_str()),
            name: None,
            alias: None,
            key_start: None,
            created_at: 1,
            last_used_at: None,
            owner: None,
            revoked: false,
        });
    }
    let encoded = serde_json::to_string(&settings).unwrap();
    let restored: crate::settings::AppSettings = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored.ofox_api_keys, settings.ofox_api_keys);
}

#[tokio::test]
#[serial]
async fn a_key_store_failure_rolls_back_native_files_record_and_metadata() {
    use crate::ofox_secret::{SecretStore, Slot};
    struct FailNew(crate::ofox_secret::InMemoryStore);
    impl SecretStore for FailNew {
        fn load(&self, slot: Slot) -> Result<Option<String>, String> {
            self.0.load(slot)
        }
        fn save(&self, slot: Slot, value: &str) -> Result<(), String> {
            if value == "sk-of-FAIL" {
                Err("fixture keychain unavailable".into())
            } else {
                self.0.save(slot, value)
            }
        }
        fn clear(&self, slot: Slot) -> Result<(), String> {
            self.0.clear(slot)
        }
    }
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    bind_with_catalog(&db, BindableTool::Pi, KEY, &all[0], &all)
        .await
        .unwrap();
    let store: std::sync::Arc<dyn SecretStore> =
        std::sync::Arc::new(FailNew(crate::ofox_secret::InMemoryStore::new()));
    store
        .save(
            Slot::ApiKey {
                tool: BindableTool::Pi,
            },
            KEY,
        )
        .unwrap();
    let meta = crate::settings::ApiKeyMeta {
        tool: BindableTool::Pi,
        key_id: "old-id".into(),
        name: None,
        alias: None,
        key_start: None,
        created_at: 1,
        last_used_at: None,
        owner: None,
        revoked: false,
    };
    crate::settings::upsert_api_key_meta(meta.clone()).unwrap();
    let before = fs::read_to_string(h.pi("models.json")).unwrap();
    let revision = revision_material(&db, BindableTool::Pi).unwrap();
    assert!(replace_key_with_metadata(
        &db,
        BindableTool::Pi,
        KEY,
        "sk-of-FAIL",
        crate::settings::ApiKeyMeta {
            key_id: "new-id".into(),
            ..meta.clone()
        },
        &store
    )
    .await
    .is_err());
    assert_eq!(fs::read_to_string(h.pi("models.json")).unwrap(), before);
    assert_eq!(revision_material(&db, BindableTool::Pi).unwrap(), revision);
    assert_eq!(
        store
            .load(Slot::ApiKey {
                tool: BindableTool::Pi
            })
            .unwrap()
            .as_deref(),
        Some(KEY)
    );
    assert_eq!(
        crate::settings::get_api_key_meta(BindableTool::Pi),
        Some(meta)
    );
}

#[tokio::test]
#[serial]
async fn unplanned_files_preserve_post_bind_edits_during_model_key_region_and_undo() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    write(&h.dsh("profiles/web/cordis.patch.yml"), PROFILE);
    bind_with_catalog(&db, BindableTool::Dsh, KEY, &all[0], &all)
        .await
        .unwrap();
    let old = snapshot(&db, BindableTool::Dsh).await.unwrap();
    let env_path = h.dsh(".env");
    let creds_path = h.dsh(".credentials.yaml");
    let mut env = fs::read_to_string(&env_path).unwrap();
    env.push_str("FOO=bar\n");
    write(&env_path, &env);
    let mut creds = fs::read_to_string(&creds_path).unwrap();
    creds.push_str("records:\n  extra: {kind: api-key}\n");
    write(&creds_path, &creds);
    set_model(&db, BindableTool::Dsh, KEY, &all[1])
        .await
        .unwrap();
    restore_scope_models(&db, BindableTool::Dsh, KEY, &old.scope_models, &all)
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(&env_path).unwrap(), env);
    assert_eq!(fs::read_to_string(&creds_path).unwrap(), creds);
    replace_key(&db, BindableTool::Dsh, KEY, "sk-of-D2")
        .await
        .unwrap();
    assert!(fs::read_to_string(&env_path).unwrap().contains("FOO=bar"));
    assert!(fs::read_to_string(&creds_path)
        .unwrap()
        .contains("extra: {kind: api-key}"));
    crate::settings::mutate_settings(|s| s.ofox_apex = Some("ofox.io".into())).unwrap();
    reconcile_endpoints(&db).await.unwrap();
    assert!(fs::read_to_string(&env_path).unwrap().contains("FOO=bar"));
    assert!(fs::read_to_string(&creds_path)
        .unwrap()
        .contains("extra: {kind: api-key}"));
    bind_with_catalog(&db, BindableTool::Pi, KEY, &all[0], &all)
        .await
        .unwrap();
    let settings_path = h.pi("settings.json");
    let mut settings = common::parse(
        common::read(&settings_path).unwrap().as_deref(),
        Format::Json,
    )
    .unwrap();
    settings["defaultModel"] = json!("openai/response");
    settings["extensions"] = json!(["npm:user-extension"]);
    write(&settings_path, &serde_json::to_string(&settings).unwrap());
    let raw = fs::read_to_string(&settings_path).unwrap();
    replace_key(&db, BindableTool::Pi, KEY, "sk-of-P2")
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(&settings_path).unwrap(), raw);
    crate::settings::mutate_settings(|s| s.ofox_apex = Some("ofox.ai".into())).unwrap();
    reconcile_endpoints(&db).await.unwrap();
    assert_eq!(fs::read_to_string(&settings_path).unwrap(), raw);
    assert_eq!(
        snapshot(&db, BindableTool::Pi).await.unwrap().model_ids,
        ["openai/response"]
    );
}

#[test]
#[serial]
fn planned_files_refuse_external_edits_instead_of_overwriting_them() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    let record = pi::plan(
        &config_dir(BindableTool::Pi).unwrap(),
        KEY,
        &all[0],
        &all,
        None,
    )
    .unwrap();
    write(&h.pi("settings.json"), "{\"theme\":\"external\"}\n");
    assert!(common::commit(&db, "pi", &record).is_err());
    assert_eq!(
        fs::read_to_string(h.pi("settings.json")).unwrap(),
        "{\"theme\":\"external\"}\n"
    );
    assert!(!h.pi("models.json").exists());
    assert!(db.get_bind_record("pi").unwrap().is_none());
}

#[tokio::test]
#[serial]
async fn failed_keychain_save_preserves_cli_edits_made_during_the_keychain_call() {
    use crate::ofox_secret::{SecretStore, Slot};
    struct EditThenFail {
        store: crate::ofox_secret::InMemoryStore,
        path: PathBuf,
    }
    impl SecretStore for EditThenFail {
        fn load(&self, slot: Slot) -> Result<Option<String>, String> {
            self.store.load(slot)
        }
        fn save(&self, slot: Slot, value: &str) -> Result<(), String> {
            if value == "sk-of-FAIL" {
                let mut settings =
                    common::parse(common::read(&self.path)?.as_deref(), Format::Json)?;
                settings["extensions"] = json!(["npm:added-during-keychain"]);
                settings["defaultModel"] = json!("openai/response");
                fs::write(&self.path, serde_json::to_vec(&settings).unwrap()).unwrap();
                Err("fixture keychain rejected".into())
            } else {
                self.store.save(slot, value)
            }
        }
        fn clear(&self, slot: Slot) -> Result<(), String> {
            self.store.clear(slot)
        }
    }
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    bind_with_catalog(&db, BindableTool::Pi, KEY, &all[0], &all)
        .await
        .unwrap();
    let store: std::sync::Arc<dyn SecretStore> = std::sync::Arc::new(EditThenFail {
        store: crate::ofox_secret::InMemoryStore::new(),
        path: h.pi("settings.json"),
    });
    store
        .save(
            Slot::ApiKey {
                tool: BindableTool::Pi,
            },
            KEY,
        )
        .unwrap();
    let meta = crate::settings::ApiKeyMeta {
        tool: BindableTool::Pi,
        key_id: "new-id".into(),
        name: None,
        alias: None,
        key_start: None,
        created_at: 1,
        last_used_at: None,
        owner: None,
        revoked: false,
    };
    assert!(
        replace_key_with_metadata(&db, BindableTool::Pi, KEY, "sk-of-FAIL", meta, &store)
            .await
            .is_err()
    );
    let settings = common::parse(
        common::read(&h.pi("settings.json")).unwrap().as_deref(),
        Format::Json,
    )
    .unwrap();
    assert_eq!(settings["extensions"], json!(["npm:added-during-keychain"]));
    assert_eq!(settings["defaultModel"], "openai/response");
    assert_eq!(
        common::parse(
            common::read(&h.pi("models.json")).unwrap().as_deref(),
            Format::Json
        )
        .unwrap()["providers"]["ofox-pi"]["apiKey"],
        KEY
    );
    assert_eq!(
        binding_status(&db, BindableTool::Pi).await.status,
        BindingStatus::Configured
    );
}

#[tokio::test]
#[serial]
async fn deleted_native_configs_restore_to_the_new_region_in_both_directions() {
    let h = Home::new();
    let db = Database::memory().unwrap();
    let all = catalog();
    let profile = h.dsh("profiles/web/cordis.patch.yml");
    write(&profile, PROFILE);
    bind_with_catalog(&db, BindableTool::Pi, KEY, &all[2], &all)
        .await
        .unwrap();
    bind_with_catalog(&db, BindableTool::Dsh, KEY, &all[2], &all)
        .await
        .unwrap();
    for apex in ["ofox.io", "ofox.ai"] {
        fs::remove_file(h.pi("models.json")).unwrap();
        fs::remove_file(&profile).unwrap();
        crate::settings::mutate_settings(|settings| settings.ofox_apex = Some(apex.into()))
            .unwrap();
        assert!(reconcile_endpoints(&db).await.is_err());
        assert!(!h.pi("models.json").exists());
        assert!(!profile.exists());
        restore_missing(&db, BindableTool::Pi, KEY).await.unwrap();
        restore_missing(&db, BindableTool::Dsh, KEY).await.unwrap();
        let pi = common::parse(
            common::read(&h.pi("models.json")).unwrap().as_deref(),
            Format::Json,
        )
        .unwrap();
        assert_eq!(
            pi["providers"]["ofox-pi"]["baseUrl"],
            crate::ofox_endpoints::openai_v1_base_url()
        );
        assert_eq!(
            pi["providers"]["ofox-pi"]["models"][2]["baseUrl"],
            crate::ofox_endpoints::anthropic_base_url()
        );
        let dsh = dsh::patch_for_test(&profile);
        for (route, api) in dsh::routes_for_test() {
            assert_eq!(
                dsh["llm-pi-ai"]["config"]["providers"][route]["baseURL"],
                url(api)
            );
        }
        for tool in [BindableTool::Pi, BindableTool::Dsh] {
            assert_eq!(
                binding_status(&db, tool).await.status,
                BindingStatus::Configured
            );
            assert_eq!(
                snapshot(&db, tool).await.unwrap().model_ids,
                ["anthropic/claude"]
            );
        }
    }
}
