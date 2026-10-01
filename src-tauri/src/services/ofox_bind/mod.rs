//! Ofox 绑定记录：每个工具绑定前的状态，存在本机专属的 `ofox_bind_snapshot`
//! 表里（见 `database/dao/ofox_bind.rs`）。
//!
//! Codex（含 ChatGPT 桌面版的 Codex 模式）、Claude Code、Gemini CLI 走这里的
//! 「绑定前快照 + 精确还原」：解绑时只把接入方式（地址、key、登录方式、模型）
//! 还原成绑定前的样子，MCP、skills、插件等其余配置一律不动。OpenCode /
//! OpenClaw / Hermes 仍走 `ProxyService::ofox_backup_live_config` /
//! `ofox_restore_from_backup`。

pub(crate) mod claude;
pub(crate) mod codex;
mod env_file;
pub(crate) mod gemini;
mod json_file;
mod plan;
pub(crate) mod record;
pub(crate) mod relocate;
pub(crate) mod report;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::app_config::AppType;
use crate::config::{file_mode, FileTxn};
use crate::database::Database;

use plan::{read_text, FileEdit, RestorePlan};
use record::{
    parse_record, serialize_record, BindEnvelope, FileBaseline, ManagedFile, PreviousProvider,
    RecordKind, StoredRecord,
};
use report::{display_path, UnbindReport};

/// 绑定、解绑、切换模型都会改同一批文件和记录，串行执行。
pub(crate) static BIND_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 走「快照 + 精确还原」的工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Codex,
    Claude,
    Gemini,
}

impl Tool {
    pub(crate) fn from_app(app: &AppType) -> Option<Self> {
        match app {
            AppType::Codex => Some(Self::Codex),
            AppType::Claude => Some(Self::Claude),
            AppType::Gemini => Some(Self::Gemini),
            AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => None,
        }
    }

    pub(crate) fn app(self) -> AppType {
        match self {
            Self::Codex => AppType::Codex,
            Self::Claude => AppType::Claude,
            Self::Gemini => AppType::Gemini,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
            Self::Gemini => "Gemini",
        }
    }

    /// 绑定记录的主键。Codex 和 ChatGPT 共用 `codex` 这一行（同一份 `~/.codex`）。
    fn record_key(self) -> &'static str {
        self.app().as_str()
    }

    fn provider_id(self) -> &'static str {
        match self {
            Self::Codex => "ofox-codex",
            Self::Claude => "ofox-claude",
            Self::Gemini => "ofox-gemini",
        }
    }

    fn official_id(self) -> &'static str {
        match self {
            Self::Codex => "codex-official",
            Self::Claude => "claude-official",
            Self::Gemini => "gemini-official",
        }
    }

    fn files(self) -> &'static [ManagedFile] {
        match self {
            Self::Codex => &[ManagedFile::CodexConfig],
            Self::Claude => &[ManagedFile::ClaudeSettings],
            Self::Gemini => &[ManagedFile::GeminiEnv, ManagedFile::GeminiSettings],
        }
    }
}

/// 前端的工具名 → (工具, 绑定方)。`codex` 和 `chatgpt` 都绑定到 `~/.codex`。
pub(crate) fn tool_for(app: &str) -> Option<(Tool, &'static str)> {
    match app.trim().to_ascii_lowercase().as_str() {
        "codex" => Some((Tool::Codex, "codex")),
        "chatgpt" => Some((Tool::Codex, "chatgpt")),
        "claude" => Some((Tool::Claude, "claude")),
        "gemini" => Some((Tool::Gemini, "gemini")),
        _ => None,
    }
}

impl ManagedFile {
    fn current_path(self) -> PathBuf {
        match self {
            Self::CodexConfig => codex::config_path(),
            Self::ClaudeSettings => claude::settings_path(),
            Self::GeminiEnv => gemini::env_path(),
            Self::GeminiSettings => gemini::settings_path(),
        }
    }

    fn validate(self, text: &str) -> Result<(), String> {
        match self {
            Self::CodexConfig => codex::validate(text),
            Self::ClaudeSettings => claude::validate(text),
            // .env 按行读，没有「解析失败」。
            Self::GeminiEnv => Ok(()),
            Self::GeminiSettings => gemini::validate_settings(text),
        }
    }

    fn plan_restore(
        self,
        original: Option<&str>,
        current: Option<&str>,
    ) -> Result<RestorePlan, String> {
        match self {
            Self::CodexConfig => codex::plan_restore(original, current),
            Self::ClaudeSettings => claude::plan_restore(original, current),
            Self::GeminiEnv => Ok(gemini::plan_restore_env(original, current)),
            Self::GeminiSettings => gemini::plan_restore_settings(original, current),
        }
    }
}

/// 绑定前的文件原样。解析不了就拒绝绑定——不能把一份坏掉的文件当成原样保存，
/// 再在它上面写配置。
fn capture_baseline(file: ManagedFile) -> Result<FileBaseline, String> {
    let path = file.current_path();
    let original = read_text(&path)?;
    if let Some(text) = original.as_deref() {
        file.validate(text)?;
    }
    Ok(FileBaseline {
        file,
        path: path.to_string_lossy().into_owned(),
        existed: original.is_some(),
        dir_existed: path.parent().is_some_and(Path::exists),
        mode: file_mode(&path),
        original,
    })
}

/// 磁盘上已经是 Ofox 的配置：旧版本绑定的，或者绑定记录丢了。
fn bound_on_disk(tool: Tool) -> Result<bool, String> {
    let (path, is_bound): (PathBuf, fn(&str) -> bool) = match tool {
        Tool::Codex => (codex::config_path(), codex::is_ofox_bound),
        Tool::Claude => (claude::settings_path(), claude::is_ofox_bound),
        Tool::Gemini => (gemini::env_path(), gemini::is_ofox_bound),
    };
    Ok(read_text(&path)?.as_deref().is_some_and(is_bound))
}

/// DB 里 `ofox-<app>` 服务商的 settings_config：Ofox 的地址和当前选的模型。
fn template(db: &Database, tool: Tool) -> Result<Value, String> {
    db.get_provider_by_id(tool.provider_id(), tool.app().as_str())
        .map_err(|e| format!("读取 Ofox {} 模板失败：{e}", tool.label()))?
        .map(|provider| provider.settings_config)
        .ok_or_else(|| format!("Ofox {} 模板缺失", tool.label()))
}

fn write_bound(
    tool: Tool,
    template: &Value,
    api_key: &str,
    txn: &mut FileTxn,
) -> Result<(), String> {
    match tool {
        Tool::Codex => {
            let config = template
                .get("config")
                .and_then(Value::as_str)
                .ok_or_else(|| "Ofox Codex 模板缺少 config".to_string())?;
            codex::write_bound_config(config, api_key, txn)
        }
        Tool::Claude => claude::write_bound(template, api_key, txn),
        Tool::Gemini => gemini::write_bound(template, api_key, txn),
    }
}

fn legacy_edits(tool: Tool) -> Result<Vec<FileEdit>, String> {
    match tool {
        Tool::Codex => codex::legacy_edits(),
        Tool::Claude => claude::legacy_edits(),
        Tool::Gemini => gemini::legacy_edits(),
    }
}

fn rollback(txn: FileTxn, context: &str) {
    if let Err(e) = txn.rollback() {
        log::error!("[ofox_bind] {context}失败后回滚文件也失败：{e}");
    }
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
    tool: Tool,
    previous: &PreviousProvider,
    report: &mut UnbindReport,
) -> Result<(), String> {
    let app = tool.app();
    let missing = |id: &Option<String>| {
        id.as_deref()
            .is_some_and(|id| !provider_exists(db, &app, id))
    };
    if missing(&previous.settings) || missing(&previous.db) {
        report.warn("previousProviderMissing", None);
        report.provider_restored_to = Some(tool.official_id().to_string());
        return set_current_provider(db, &app, tool.official_id());
    }
    crate::settings::set_current_provider(&app, previous.settings.as_deref())
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

fn load_record(db: &Database, tool: Tool) -> Result<Option<(String, StoredRecord)>, String> {
    let key = tool.record_key();
    let Some(row) = db
        .get_bind_record(key)
        .map_err(|e| format!("读取 {key} 绑定记录失败：{e}"))?
    else {
        return Ok(None);
    };
    let parsed = parse_record(&row.record)?;
    Ok(Some((row.record, parsed)))
}

/// 已经绑定的工具（切换模型时）：按 DB 模板重写接入字段，其余内容不动。
pub(crate) async fn rewrite_bound_config(
    db: &Database,
    tool: Tool,
    api_key: &str,
) -> Result<(), String> {
    let _guard = BIND_LOCK.lock().await;
    if tool == Tool::Codex {
        codex::migrate_legacy_shape(|| Some(api_key.to_string()))?;
    }
    let template = template(db, tool)?;
    let mut txn = FileTxn::new();
    if let Err(error) = write_bound(tool, &template, api_key, &mut txn) {
        rollback(txn, "重写绑定配置");
        return Err(error);
    }
    Ok(())
}

/// 启动时把旧版本绑定的 Codex 迁成「服务商自带 key」，堵住 ChatGPT 令牌外发。
pub(crate) async fn migrate_codex_on_startup() {
    let _guard = BIND_LOCK.lock().await;
    let stored_key = || {
        crate::ofox_secret::default_store()
            .load(crate::ofox_secret::Slot::ApiKey {
                tool: AppType::Codex.into(),
            })
            .ok()
            .flatten()
    };
    match codex::migrate_legacy_shape(stored_key) {
        Ok(true) => log::info!("✓ Migrated legacy Codex Ofox binding"),
        Ok(false) => {}
        Err(e) => log::warn!("✗ Failed to migrate legacy Codex Ofox binding: {e}"),
    }
}

/// 把工具绑定到 Ofox。第一次绑定时记下受管文件的原样和当前服务商，之后重复
/// 绑定只增加绑定方，不覆盖快照。
pub(crate) async fn bind(
    db: &Database,
    tool: Tool,
    holder: &str,
    api_key: &str,
) -> Result<(), String> {
    let _guard = BIND_LOCK.lock().await;
    if tool == Tool::Codex {
        codex::migrate_legacy_shape(|| Some(api_key.to_string()))?;
    }
    let template = template(db, tool)?;

    let existing = load_record(db, tool)?;
    let previous_text = existing.as_ref().map(|(text, _)| text.clone());
    let envelope = match existing {
        Some((_, StoredRecord::Envelope(mut envelope))) => {
            envelope.holders.insert(holder.to_string());
            envelope
        }
        // 旧版本留下的补丁记录没有绑定前快照。
        Some((_, StoredRecord::Legacy(_))) => BindEnvelope::legacy_adopted(holder),
        // 磁盘上已经是 Ofox 的配置：不能把它当成原样。
        None if bound_on_disk(tool)? => BindEnvelope::legacy_adopted(holder),
        None => BindEnvelope::snapshot(
            holder,
            previous_provider(db, &tool.app())?,
            tool.files()
                .iter()
                .map(|file| capture_baseline(*file))
                .collect::<Result<_, _>>()?,
        ),
    };
    let key = tool.record_key();
    db.upsert_bind_record(key, &serialize_record(&envelope)?)
        .map_err(|e| format!("保存 {} 绑定记录失败：{e}", tool.label()))?;

    let mut txn = FileTxn::new();
    let result = write_bound(tool, &template, api_key, &mut txn)
        .and_then(|()| set_current_provider(db, &tool.app(), tool.provider_id()));
    if let Err(error) = result {
        rollback(txn, "绑定");
        let record_rollback = match previous_text {
            Some(text) => db.upsert_bind_record(key, &text),
            None => db.delete_bind_record(key),
        };
        if let Err(e) = record_rollback {
            log::error!(
                "[ofox_bind] {} 绑定失败后还原绑定记录也失败：{e}",
                tool.label()
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

fn prefixed<'a>(shown: &str, keys: &'a [String]) -> impl Iterator<Item = String> + 'a {
    let shown = shown.to_string();
    keys.iter().map(move |key| format!("{shown}: {key}"))
}

/// 按快照还原受管文件，返回绑定时新建、现在可能已空的目录。
fn restore_snapshot(
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
            let plan = baseline
                .file
                .plan_restore(baseline.original.as_deref(), current.as_deref())?;
            report
                .restored_keys
                .extend(prefixed(&shown, &plan.restored_keys));
            report
                .removed_keys
                .extend(prefixed(&shown, &plan.removed_keys));
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
        rollback(txn, "还原");
        return Err(error);
    }
    Ok(dirs_to_prune)
}

/// 没有快照时的尽力清理。返回是否找到了 Ofox 的配置。
fn apply_legacy_edits(
    edits: Vec<FileEdit>,
    report: &mut UnbindReport,
    dry_run: bool,
) -> Result<bool, String> {
    let mut txn = FileTxn::new();
    for edit in &edits {
        let shown = display_path(&edit.path);
        report
            .restored_keys
            .extend(prefixed(&shown, &edit.restored_keys));
        report
            .removed_keys
            .extend(prefixed(&shown, &edit.removed_keys));
        if edit.content.is_none() {
            report.files_removed.push(shown);
        }
        if dry_run {
            continue;
        }
        let result = match edit.content.as_deref() {
            Some(text) => txn.write(&edit.path, text.as_bytes()),
            None => txn.remove(&edit.path),
        };
        if let Err(e) = result {
            rollback(txn, "清理旧版绑定");
            return Err(format!("清理 {} 失败：{e}", display_path(&edit.path)));
        }
    }
    Ok(!edits.is_empty())
}

/// 解除工具的绑定。`still_bound` 是前端认为仍然绑定的其它工具，用来兼容只在
/// 前端记过 ChatGPT 绑定的旧安装。`dry_run` 只算不写（预览）。
pub(crate) async fn unbind(
    db: &Database,
    tool: Tool,
    holder: &str,
    still_bound: &[String],
    dry_run: bool,
) -> Result<UnbindReport, String> {
    let _guard = BIND_LOCK.lock().await;
    let mut report = UnbindReport::new(holder, dry_run);
    let stored = load_record(db, tool)?.map(|(_, record)| record);
    let key = tool.record_key();

    let mut remaining: BTreeSet<String> = match &stored {
        Some(StoredRecord::Envelope(envelope)) => envelope.holders.clone(),
        _ => BTreeSet::new(),
    };
    remaining.extend(
        still_bound
            .iter()
            .filter_map(|app| tool_for(app))
            .filter(|(other, _)| *other == tool)
            .map(|(_, other_holder)| other_holder.to_string()),
    );
    remaining.remove(holder);
    if !remaining.is_empty() {
        report.shared_kept_by = remaining.into_iter().collect();
        if !dry_run {
            if let Some(StoredRecord::Envelope(mut envelope)) = stored {
                envelope.holders.remove(holder);
                db.upsert_bind_record(key, &serialize_record(&envelope)?)
                    .map_err(|e| format!("更新 {} 绑定记录失败：{e}", tool.label()))?;
            }
        }
        return Ok(report);
    }

    match stored {
        Some(StoredRecord::Envelope(envelope)) if envelope.kind == RecordKind::Snapshot => {
            let dirs_to_prune = restore_snapshot(&envelope, &mut report, dry_run)?;
            if dry_run {
                let previous = &envelope.previous_provider;
                report.provider_restored_to =
                    previous.settings.clone().or_else(|| previous.db.clone());
            } else {
                restore_previous_provider(db, tool, &envelope.previous_provider, &mut report)?;
                dirs_to_prune
                    .iter()
                    .for_each(|dir| remove_dir_if_empty(dir));
            }
        }
        _ => {
            if apply_legacy_edits(legacy_edits(tool)?, &mut report, dry_run)? {
                report.legacy = true;
                report.provider_restored_to = Some(tool.official_id().to_string());
                if !dry_run {
                    set_current_provider(db, &tool.app(), tool.official_id())?;
                }
            } else {
                report.already_unbound = true;
            }
        }
    }

    if !dry_run {
        disable_proxy_flag(db, &tool.app()).await;
        if let Err(e) = db.delete_bind_record(key) {
            // 文件已经还原；记录删不掉时再解绑一次也是同样结果。
            log::warn!("[ofox_bind] 删除 {} 绑定记录失败：{e}", tool.label());
            report.warn("recordCleanupFailed", None);
        }
    }
    Ok(report)
}
