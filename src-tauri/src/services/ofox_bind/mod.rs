//! Ofox 绑定记录：每个工具绑定前后的状态，存在本机专属的 `ofox_bind_snapshot`
//! 表里（见 `database/dao/ofox_bind.rs`）。
//!
//! 目前 Codex（含 ChatGPT 桌面版的 Codex 模式）走这里的「绑定前快照 + 精确还原」；
//! 其它工具仍走 `ProxyService::ofox_backup_live_config` / `ofox_restore_from_backup`。

pub(crate) mod codex;
pub(crate) mod record;
pub(crate) mod relocate;
pub(crate) mod report;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::app_config::AppType;
use crate::config::FileTxn;
use crate::database::Database;

use record::{
    parse_record, serialize_record, BindEnvelope, PreviousProvider, RecordKind, StoredRecord,
};
use report::{display_path, UnbindReport};

/// 绑定、解绑、切换模型都会改同一批文件和记录，串行执行。
pub(crate) static BIND_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Codex 和 ChatGPT 共用这一行记录（同一份 `~/.codex`）。
const CODEX_RECORD: &str = "codex";
const CODEX_PROVIDER: &str = "ofox-codex";
const CODEX_OFFICIAL: &str = "codex-official";

/// `codex` / `chatgpt` 都绑定到 `~/.codex`；返回绑定方名，其它工具返回 `None`。
pub(crate) fn codex_holder(app: &str) -> Option<&'static str> {
    match app.trim().to_ascii_lowercase().as_str() {
        "codex" => Some("codex"),
        "chatgpt" => Some("chatgpt"),
        _ => None,
    }
}

fn codex_template(db: &Database) -> Result<String, String> {
    let provider = db
        .get_provider_by_id(CODEX_PROVIDER, AppType::Codex.as_str())
        .map_err(|e| format!("读取 Ofox Codex 模板失败：{e}"))?
        .ok_or_else(|| "Ofox Codex 模板缺失".to_string())?;
    provider
        .settings_config
        .get("config")
        .and_then(|config| config.as_str())
        .map(str::to_string)
        .ok_or_else(|| "Ofox Codex 模板缺少 config".to_string())
}

fn previous_provider(db: &Database, app: &AppType) -> Result<PreviousProvider, String> {
    Ok(PreviousProvider {
        settings: crate::settings::get_current_provider(app),
        db: db
            .get_current_provider(app.as_str())
            .map_err(|e| format!("读取当前供应商失败：{e}"))?,
    })
}

fn set_current_provider(db: &Database, app: &AppType, id: &str) -> Result<(), String> {
    crate::settings::set_current_provider(app, Some(id))
        .map_err(|e| format!("设置 {} 当前供应商失败：{e}", app.as_str()))?;
    db.set_current_provider(app.as_str(), id)
        .map_err(|e| format!("更新 {} 当前供应商失败：{e}", app.as_str()))
}

fn provider_exists(db: &Database, app: &AppType, id: &str) -> bool {
    matches!(db.get_provider_by_id(id, app.as_str()), Ok(Some(_)))
}

/// 还原绑定前的当前服务商；那个服务商已经不在了就退回官方服务商。
fn restore_previous_provider(
    db: &Database,
    app: &AppType,
    previous: &PreviousProvider,
    report: &mut UnbindReport,
) -> Result<(), String> {
    let missing = |id: &Option<String>| {
        id.as_deref()
            .is_some_and(|id| !provider_exists(db, app, id))
    };
    if missing(&previous.settings) || missing(&previous.db) {
        report.warn("previousProviderMissing", None);
        report.provider_restored_to = Some(CODEX_OFFICIAL.to_string());
        return set_current_provider(db, app, CODEX_OFFICIAL);
    }
    crate::settings::set_current_provider(app, previous.settings.as_deref())
        .map_err(|e| format!("还原 {} 当前供应商失败：{e}", app.as_str()))?;
    match previous.db.as_deref() {
        Some(id) => db.set_current_provider(app.as_str(), id),
        None => db.clear_current_provider(app.as_str()),
    }
    .map_err(|e| format!("还原 {} 当前供应商失败：{e}", app.as_str()))?;
    report.provider_restored_to = previous.settings.clone().or_else(|| previous.db.clone());
    Ok(())
}

/// 绑定期间不再需要接管模式的代理开关；留着会让启动自愈把刚解绑的工具又绑回去。
async fn disable_proxy_flag(db: &Database, app: &AppType) {
    if let Ok(mut config) = db.get_proxy_config_for_app(app.as_str()).await {
        if config.enabled {
            config.enabled = false;
            if let Err(e) = db.update_proxy_config_for_app(config).await {
                log::warn!("[ofox_bind] 清除 {} 代理开关失败：{e}", app.as_str());
            }
        }
    }
}

fn load_record(db: &Database, tool: &str) -> Result<Option<(String, StoredRecord)>, String> {
    let Some(row) = db
        .get_bind_record(tool)
        .map_err(|e| format!("读取 {tool} 绑定记录失败：{e}"))?
    else {
        return Ok(None);
    };
    let parsed = parse_record(&row.record)?;
    Ok(Some((row.record, parsed)))
}

/// 已经绑定的 Codex（切换模型时）：按 DB 模板重写 config.toml，不碰 auth.json。
pub(crate) async fn rewrite_codex_bound_config(db: &Database, api_key: &str) -> Result<(), String> {
    let _guard = BIND_LOCK.lock().await;
    codex::migrate_legacy_shape(Some(api_key))?;
    let template = codex_template(db)?;
    let mut txn = FileTxn::new();
    if let Err(error) = codex::write_bound_config(&template, api_key, &mut txn) {
        let _ = txn.rollback();
        return Err(error);
    }
    Ok(())
}

/// 启动时把旧版本绑定的 Codex 迁成「服务商自带 key」，堵住 ChatGPT 令牌外发。
pub(crate) async fn migrate_codex_on_startup() {
    let _guard = BIND_LOCK.lock().await;
    let stored_key = crate::ofox_secret::default_store()
        .load(crate::ofox_secret::Slot::ApiKey {
            tool: AppType::Codex.into(),
        })
        .ok()
        .flatten();
    match codex::migrate_legacy_shape(stored_key.as_deref()) {
        Ok(true) => log::info!("✓ Migrated legacy Codex Ofox binding"),
        Ok(false) => {}
        Err(e) => log::warn!("✗ Failed to migrate legacy Codex Ofox binding: {e}"),
    }
}

/// 把 Codex（或 ChatGPT）绑定到 Ofox。第一次绑定时记下 config.toml 原样和当前
/// 服务商，之后重复绑定只增加绑定方，不覆盖快照。
pub(crate) async fn bind_codex(db: &Database, holder: &str, api_key: &str) -> Result<(), String> {
    let _guard = BIND_LOCK.lock().await;
    codex::migrate_legacy_shape(Some(api_key))?;
    let template = codex_template(db)?;

    let existing = load_record(db, CODEX_RECORD)?;
    let previous_text = existing.as_ref().map(|(text, _)| text.clone());
    let envelope = match existing {
        Some((_, StoredRecord::Envelope(mut envelope))) => {
            envelope.holders.insert(holder.to_string());
            envelope
        }
        // 旧版本留下的补丁记录没有绑定前快照。
        Some((_, StoredRecord::Legacy(_))) => BindEnvelope::legacy_adopted(holder),
        None => {
            let current = codex::read_config()?.unwrap_or_default();
            if codex::is_ofox_bound(&current) {
                // 磁盘上已经是 Ofox 的配置（旧版本绑定、记录丢了）：不能把它当成原样。
                BindEnvelope::legacy_adopted(holder)
            } else {
                BindEnvelope::snapshot(
                    holder,
                    previous_provider(db, &AppType::Codex)?,
                    vec![codex::capture_baseline()?],
                )
            }
        }
    };
    let record_text = serialize_record(&envelope)?;
    db.upsert_bind_record(CODEX_RECORD, &record_text)
        .map_err(|e| format!("保存 Codex 绑定记录失败：{e}"))?;

    let mut txn = FileTxn::new();
    let result = codex::write_bound_config(&template, api_key, &mut txn)
        .and_then(|()| set_current_provider(db, &AppType::Codex, CODEX_PROVIDER));
    if let Err(error) = result {
        let rollback = txn.rollback().err();
        let record_rollback = match previous_text {
            Some(text) => db.upsert_bind_record(CODEX_RECORD, &text),
            None => db.delete_bind_record(CODEX_RECORD),
        }
        .err();
        if rollback.is_some() || record_rollback.is_some() {
            log::error!(
                "[ofox_bind] Codex 绑定失败且回滚不完整：files={rollback:?} record={record_rollback:?}"
            );
        }
        return Err(error);
    }
    Ok(())
}

fn remove_dir_if_empty(path: &Path) {
    // 目录里还有别的东西（失败）就留着。
    let _ = std::fs::remove_dir(path);
}

fn restore_codex_snapshot(
    envelope: &BindEnvelope,
    report: &mut UnbindReport,
    dry_run: bool,
) -> Result<Vec<PathBuf>, String> {
    let mut txn = FileTxn::new();
    let mut dirs_to_prune = Vec::new();
    let result = (|| {
        for baseline in &envelope.files {
            let path = PathBuf::from(&baseline.path);
            let shown = display_path(&path);
            let current = crate::config::read_file_bytes(&path)
                .map_err(|e| e.to_string())?
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            let plan = codex::plan_restore(baseline.original.as_deref(), current.as_deref())?;
            report.restored_keys.extend(
                plan.restored_keys
                    .iter()
                    .map(|key| format!("{shown}: {key}")),
            );
            report.removed_keys.extend(
                plan.removed_keys
                    .iter()
                    .map(|key| format!("{shown}: {key}")),
            );
            match plan.content {
                Some(text) => {
                    if plan.exact {
                        report.exact_files.push(shown.clone());
                    }
                    if !baseline.existed {
                        report.warn("fileKeptHasOtherContent", Some(shown.clone()));
                    }
                    if !dry_run {
                        txn.write(&path, text.as_bytes())
                            .map_err(|e| e.to_string())?;
                        if let Some(mode) = baseline.mode {
                            txn.set_mode(&path, mode).map_err(|e| e.to_string())?;
                        }
                    }
                }
                None => {
                    if current.is_some() {
                        report.files_removed.push(shown.clone());
                        if !dry_run {
                            txn.remove(&path).map_err(|e| e.to_string())?;
                        }
                    }
                    if !baseline.dir_existed {
                        if let Some(parent) = path.parent() {
                            dirs_to_prune.push(parent.to_path_buf());
                        }
                    }
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = txn.rollback();
        return Err(error);
    }
    Ok(dirs_to_prune)
}

fn cleanup_legacy_codex(report: &mut UnbindReport, dry_run: bool) -> Result<bool, String> {
    let config_path = codex::config_path();
    let shown = display_path(&config_path);
    let mut txn = FileTxn::new();
    let result = (|| {
        let mut changed = false;
        if let Some(config) = codex::read_config()? {
            let (cleaned, removed) = codex::legacy_cleanup_config(&config)?;
            report
                .removed_keys
                .extend(removed.iter().map(|key| format!("{shown}: {key}")));
            if let Some(text) = cleaned {
                changed = true;
                if !dry_run {
                    txn.write(&config_path, text.as_bytes())
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        if codex::legacy_cleanup_auth(&mut txn, dry_run)? {
            changed = true;
            report.removed_keys.push(format!(
                "{}: OPENAI_API_KEY",
                display_path(&codex::auth_path())
            ));
        }
        Ok(changed)
    })();
    if result.is_err() {
        let _ = txn.rollback();
    }
    result
}

/// 解除 Codex（或 ChatGPT）的绑定。`still_bound` 是前端认为仍然绑定的其它工具，
/// 用来兼容只在前端记过 ChatGPT 绑定的旧安装。`dry_run` 只算不写（预览）。
pub(crate) async fn unbind_codex(
    db: &Database,
    holder: &str,
    still_bound: &[String],
    dry_run: bool,
) -> Result<UnbindReport, String> {
    let _guard = BIND_LOCK.lock().await;
    let mut report = UnbindReport::new(holder, dry_run);
    let stored = load_record(db, CODEX_RECORD)?.map(|(_, record)| record);

    let mut remaining: BTreeSet<String> = match &stored {
        Some(StoredRecord::Envelope(envelope)) => envelope.holders.clone(),
        _ => BTreeSet::new(),
    };
    remaining.extend(
        still_bound
            .iter()
            .filter_map(|tool| codex_holder(tool))
            .map(str::to_string),
    );
    remaining.remove(holder);
    if !remaining.is_empty() {
        report.shared_kept_by = remaining.into_iter().collect();
        if !dry_run {
            if let Some(StoredRecord::Envelope(mut envelope)) = stored {
                envelope.holders.remove(holder);
                db.upsert_bind_record(CODEX_RECORD, &serialize_record(&envelope)?)
                    .map_err(|e| format!("更新 Codex 绑定记录失败：{e}"))?;
            }
        }
        return Ok(report);
    }

    match stored {
        Some(StoredRecord::Envelope(envelope)) if envelope.kind == RecordKind::Snapshot => {
            let dirs_to_prune = restore_codex_snapshot(&envelope, &mut report, dry_run)?;
            if !dry_run {
                restore_previous_provider(
                    db,
                    &AppType::Codex,
                    &envelope.previous_provider,
                    &mut report,
                )?;
                dirs_to_prune
                    .iter()
                    .for_each(|dir| remove_dir_if_empty(dir));
            } else {
                report.provider_restored_to = envelope
                    .previous_provider
                    .settings
                    .clone()
                    .or_else(|| envelope.previous_provider.db.clone());
            }
        }
        _ => {
            if !cleanup_legacy_codex(&mut report, dry_run)? {
                report.already_unbound = true;
            } else {
                report.legacy = true;
                report.provider_restored_to = Some(CODEX_OFFICIAL.to_string());
                if !dry_run {
                    set_current_provider(db, &AppType::Codex, CODEX_OFFICIAL)?;
                }
            }
        }
    }

    if !dry_run {
        disable_proxy_flag(db, &AppType::Codex).await;
        if let Err(e) = db.delete_bind_record(CODEX_RECORD) {
            // 文件已经还原；记录删不掉时再解绑一次也是同样结果。
            log::warn!("[ofox_bind] 删除 Codex 绑定记录失败：{e}");
            report.warn("recordCleanupFailed", None);
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Provider;
    use serde_json::json;
    use serial_test::serial;
    use std::fs;

    const TEMPLATE: &str = "model_provider = \"ofox\"\nmodel = \"openai/gpt-6-luna\"\nmodel_reasoning_effort = \"high\"\ndisable_response_storage = true\n\n[model_providers.ofox]\nname = \"ofox\"\nbase_url = \"https://api.ofox.ai/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n";
    const OAUTH_AUTH: &str = "{\n  \"auth_mode\": \"chatgpt\",\n  \"OPENAI_API_KEY\": null,\n  \"tokens\": {\n    \"id_token\": \"eyJid\",\n    \"access_token\": \"eyJaccess\",\n    \"refresh_token\": \"rt\",\n    \"account_id\": \"acct\"\n  },\n  \"last_refresh\": \"2026-09-30T00:00:00Z\"\n}\n";
    const USER_CONFIG: &str = "# my codex setup\nmodel = \"gpt-5-codex\"\nmodel_reasoning_effort = \"medium\"\n\n[projects.\"/work\"]\ntrust_level = \"trusted\"\n\n[mcp_servers.docs]\ncommand = \"docs-mcp\"\n";
    const KEY: &str = "sk-of-TEST";

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

        bind_codex(&db, "codex", KEY).await.expect("bind");
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
        rewrite_codex_bound_config(&db, KEY)
            .await
            .expect("model change");
        assert_eq!(
            toml_at(&config)["model"].as_str(),
            Some("anthropic/claude-x")
        );

        let report = unbind_codex(&db, "codex", &[], false)
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

        bind_codex(&db, "codex", KEY).await.expect("bind");
        assert!(!home.codex("auth.json").exists());
        assert!(home.codex("config.toml").exists());

        let report = unbind_codex(&db, "codex", &[], false)
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
        bind_codex(&db, "codex", KEY).await.expect("bind");

        let edited = fs::read_to_string(&config).unwrap().replace(
            "model_reasoning_effort = \"high\"",
            "model_reasoning_effort = \"low\"",
        ) + "\n[mcp_servers.added]\ncommand = \"x\"\n";
        write(&config, &edited);
        // 再绑一次（添加工具会重绑所有工具）：快照不能被覆盖。
        bind_codex(&db, "codex", KEY).await.expect("rebind");

        unbind_codex(&db, "codex", &[], false)
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
        bind_codex(&db, "codex", KEY).await.expect("bind");
        unbind_codex(&db, "codex", &[], false)
            .await
            .expect("unbind");

        let again = unbind_codex(&db, "codex", &[], false)
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
        bind_codex(&db, "codex", KEY).await.expect("bind");
        let bound = fs::read_to_string(home.codex("config.toml")).unwrap();

        db.delete_provider("codex", CODEX_PROVIDER)
            .expect("drop template");
        assert!(bind_codex(&db, "codex", KEY).await.is_err());

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
        bind_codex(&db, "codex", KEY).await.expect("bind codex");
        bind_codex(&db, "chatgpt", KEY).await.expect("bind chatgpt");

        let first = unbind_codex(&db, "codex", &[], false)
            .await
            .expect("unbind codex");
        assert_eq!(first.shared_kept_by, ["chatgpt"]);
        assert!(codex::is_ofox_bound(
            &fs::read_to_string(home.codex("config.toml")).unwrap()
        ));

        unbind_codex(&db, "chatgpt", &[], false)
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
        bind_codex(&db, "codex", KEY).await.expect("bind");

        let report = unbind_codex(&db, "codex", &["chatgpt".to_string()], false)
            .await
            .expect("unbind");
        assert_eq!(report.shared_kept_by, ["chatgpt"]);
        assert!(codex::is_ofox_bound(
            &fs::read_to_string(home.codex("config.toml")).unwrap()
        ));

        unbind_codex(&db, "chatgpt", &[], false)
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
        bind_codex(&db, "codex", KEY).await.expect("bind");
        let bound = fs::read_to_string(home.codex("config.toml")).unwrap();

        let preview = unbind_codex(&db, "codex", &[], true)
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

        bind_codex(&db, "codex", KEY).await.expect("bind");
        let auth_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&auth).unwrap()).unwrap();
        assert!(auth_after["OPENAI_API_KEY"].is_null());
        assert_eq!(auth_after["tokens"]["access_token"], "eyJaccess");
        let record =
            parse_record(&db.get_bind_record(CODEX_RECORD).unwrap().unwrap().record).unwrap();
        assert!(
            matches!(record, StoredRecord::Envelope(ref e) if e.kind == RecordKind::LegacyAdopted)
        );

        let report = unbind_codex(&db, "codex", &[], false)
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
            !codex::migrate_legacy_shape(None).unwrap(),
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
        assert!(codex::migrate_legacy_shape(None).unwrap());
        assert_eq!(
            toml_at(&config)["model_providers"]["ofox"]["experimental_bearer_token"].as_str(),
            Some("sk-of-OLD")
        );
        assert!(
            !codex::migrate_legacy_shape(None).unwrap(),
            "second run is a no-op"
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
        bind_codex(&db, "codex", KEY).await.expect("bind");

        unbind_codex(&db, "codex", &[], false)
            .await
            .expect("unbind");
        assert!(!db.get_proxy_config_for_app("codex").await.unwrap().enabled);
        drop(home);
    }
}
