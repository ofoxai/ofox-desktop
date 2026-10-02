//! Ofox 绑定记录：每个工具绑定前的状态，存在本机专属的 `ofox_bind_snapshot`
//! 表里（见 `database/dao/ofox_bind.rs`）。
//!
//! 所有直接改配置文件的工具（Codex 含 ChatGPT 桌面版的 Codex 模式、Claude Code、
//! Gemini CLI、OpenCode、OpenClaw、Hermes）都走这里的「绑定前快照 + 精确还原」：
//! 解绑时只把接入方式（地址、key、登录方式、模型）还原成绑定前的样子，MCP、
//! skills、插件等其余配置一律不动。WorkBuddy 走 `workbuddy_config`。

pub(crate) mod claude;
pub(crate) mod codex;
mod env_file;
pub(crate) mod gemini;
mod hermes;
mod json_file;
mod openclaw;
mod opencode;
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
    OpenCode,
    OpenClaw,
    Hermes,
}

impl Tool {
    pub(crate) fn from_app(app: &AppType) -> Self {
        match app {
            AppType::Codex => Self::Codex,
            AppType::Claude => Self::Claude,
            AppType::Gemini => Self::Gemini,
            AppType::OpenCode => Self::OpenCode,
            AppType::OpenClaw => Self::OpenClaw,
            AppType::Hermes => Self::Hermes,
        }
    }

    pub(crate) fn app(self) -> AppType {
        match self {
            Self::Codex => AppType::Codex,
            Self::Claude => AppType::Claude,
            Self::Gemini => AppType::Gemini,
            Self::OpenCode => AppType::OpenCode,
            Self::OpenClaw => AppType::OpenClaw,
            Self::Hermes => AppType::Hermes,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
            Self::Gemini => "Gemini",
            Self::OpenCode => "OpenCode",
            Self::OpenClaw => "OpenClaw",
            Self::Hermes => "Hermes",
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
            Self::OpenCode => "ofox-opencode",
            Self::OpenClaw => "ofox-openclaw",
            Self::Hermes => "ofox-hermes",
        }
    }

    /// 官方服务商（`providers_seed.rs` 的 OFFICIAL_SEEDS）；可同时配多家的工具没有。
    fn official_id(self) -> Option<&'static str> {
        match self {
            Self::Codex => Some("codex-official"),
            Self::Claude => Some("claude-official"),
            Self::Gemini => Some("gemini-official"),
            Self::OpenCode | Self::OpenClaw | Self::Hermes => None,
        }
    }

    fn files(self) -> &'static [ManagedFile] {
        match self {
            Self::Codex => &[ManagedFile::CodexConfig],
            Self::Claude => &[ManagedFile::ClaudeSettings],
            Self::Gemini => &[ManagedFile::GeminiEnv, ManagedFile::GeminiSettings],
            Self::OpenCode => &[ManagedFile::OpenCodeConfig],
            Self::OpenClaw => &[ManagedFile::OpenClawConfig],
            Self::Hermes => &[ManagedFile::HermesConfig],
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
        "opencode" => Some((Tool::OpenCode, "opencode")),
        "openclaw" => Some((Tool::OpenClaw, "openclaw")),
        "hermes" => Some((Tool::Hermes, "hermes")),
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
            Self::OpenCodeConfig => opencode::config_path(),
            Self::OpenClawConfig => openclaw::config_path(),
            Self::HermesConfig => hermes::config_path(),
        }
    }

    fn validate(self, text: &str) -> Result<(), String> {
        match self {
            Self::CodexConfig => codex::validate(text),
            Self::ClaudeSettings => claude::validate(text),
            // .env 按行读，没有「解析失败」。
            Self::GeminiEnv => Ok(()),
            Self::GeminiSettings => gemini::validate_settings(text),
            Self::OpenCodeConfig => opencode::validate(text),
            Self::OpenClawConfig => openclaw::validate(text),
            Self::HermesConfig => hermes::validate(text),
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
            Self::OpenCodeConfig => opencode::plan_restore(original, current),
            Self::OpenClawConfig => openclaw::plan_restore(original, current),
            Self::HermesConfig => hermes::plan_restore(original, current),
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
        Tool::OpenCode => (opencode::config_path(), opencode::is_ofox_bound),
        Tool::OpenClaw => (openclaw::config_path(), openclaw::is_ofox_bound),
        Tool::Hermes => (hermes::config_path(), hermes::is_ofox_bound),
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
        Tool::OpenCode => opencode::write_bound(template, api_key, txn),
        Tool::OpenClaw => openclaw::write_bound(template, api_key, txn),
        Tool::Hermes => hermes::write_bound(template, api_key, txn),
    }
}

/// `legacy` 是旧版本留下的绑定记录（OpenClaw / Hermes 的记录里可能有绑定前的路由）。
fn legacy_edits(tool: Tool, legacy: Option<&Value>) -> Result<Vec<FileEdit>, String> {
    match tool {
        Tool::Codex => codex::legacy_edits(),
        Tool::Claude => claude::legacy_edits(),
        Tool::Gemini => gemini::legacy_edits(),
        Tool::OpenCode => opencode::legacy_edits(),
        Tool::OpenClaw => openclaw::legacy_edits(legacy),
        Tool::Hermes => hermes::legacy_edits(legacy),
    }
}

/// 回滚文件，把回滚结果附在 `error` 后面。
pub(crate) fn rollback_with(txn: FileTxn, error: String) -> String {
    match txn.rollback() {
        Ok(()) => format!("{error}（已回滚文件）"),
        Err(rollback_error) => {
            format!("{error}；回滚文件也失败：{rollback_error}，请检查该工具的配置文件")
        }
    }
}

fn previous_provider(db: &Database, app: &AppType) -> Result<PreviousProvider, String> {
    Ok(PreviousProvider {
        settings: crate::settings::get_current_provider(app),
        db: db
            .get_current_provider(app.as_str())
            .map_err(|e| format!("读取当前服务商失败：{e}"))?,
    })
}

fn set_current_provider(db: &Database, app: &AppType, id: &str) -> Result<(), String> {
    crate::settings::set_current_provider(app, Some(id))
        .map_err(|e| format!("设置 {} 当前服务商失败：{e}", app.as_str()))?;
    db.set_current_provider(app.as_str(), id)
        .map_err(|e| format!("更新 {} 当前服务商失败：{e}", app.as_str()))
}

/// 把当前服务商写回 `previous`（settings 和 DB 各自）。
fn write_previous_provider(
    db: &Database,
    app: &AppType,
    previous: &PreviousProvider,
) -> Result<(), String> {
    crate::settings::set_current_provider(app, previous.settings.as_deref())
        .map_err(|e| format!("还原 {} 当前服务商失败：{e}", app.as_str()))?;
    match previous.db.as_deref() {
        Some(id) => db.set_current_provider(app.as_str(), id),
        None => db.clear_current_provider(app.as_str()),
    }
    .map_err(|e| format!("还原 {} 当前服务商失败：{e}", app.as_str()))
}

fn provider_exists(db: &Database, app: &AppType, id: &str) -> Result<bool, String> {
    db.get_provider_by_id(id, app.as_str())
        .map(|provider| provider.is_some())
        .map_err(|e| format!("读取服务商 {id} 失败：{e}"))
}

/// 当前服务商（settings 或 DB）还是 Ofox。
fn provider_is_ofox(db: &Database, tool: Tool) -> Result<bool, String> {
    let current = previous_provider(db, &tool.app())?;
    Ok([current.settings, current.db]
        .iter()
        .any(|id| id.as_deref() == Some(tool.provider_id())))
}

/// 解绑后当前服务商该是什么。
#[derive(Debug, PartialEq)]
enum ProviderTarget {
    /// 绑定前的那个。
    Previous(PreviousProvider),
    Official,
    /// 没有官方服务商的工具：还指着 Ofox 就清掉。
    ClearIfOfox,
}

/// 绑定前的服务商还在就回到它；不在了（或根本没有快照）就退回官方服务商。
/// 预览和实际解绑共用这个决定，报告里的内容两边一致。
fn provider_target(
    db: &Database,
    tool: Tool,
    previous: Option<&PreviousProvider>,
    report: &mut UnbindReport,
) -> Result<ProviderTarget, String> {
    if let Some(previous) = previous {
        let app = tool.app();
        let mut missing = false;
        for id in [previous.settings.as_deref(), previous.db.as_deref()]
            .into_iter()
            .flatten()
        {
            missing |= !provider_exists(db, &app, id)?;
        }
        if !missing {
            report.provider_restored_to = previous.settings.clone().or_else(|| previous.db.clone());
            return Ok(ProviderTarget::Previous(previous.clone()));
        }
        report.warn("previousProviderMissing", None);
    }
    report.provider_restored_to = tool.official_id().map(str::to_string);
    Ok(match tool.official_id() {
        Some(_) => ProviderTarget::Official,
        None => ProviderTarget::ClearIfOfox,
    })
}

fn apply_provider_target(db: &Database, tool: Tool, target: ProviderTarget) -> Result<(), String> {
    let app = tool.app();
    match target {
        ProviderTarget::Previous(previous) => write_previous_provider(db, &app, &previous),
        ProviderTarget::Official => {
            set_current_provider(db, &app, tool.official_id().expect("has official"))
        }
        ProviderTarget::ClearIfOfox => {
            if provider_is_ofox(db, tool)? {
                write_previous_provider(db, &app, &PreviousProvider::default())
            } else {
                Ok(())
            }
        }
    }
}

/// 绑定期间不再需要接管模式的代理开关；留着会让启动自愈把刚解绑的工具又绑回去。
/// 先关它再动文件：关不掉就不解绑。
async fn disable_proxy_flag(db: &Database, app: &AppType) -> Result<(), String> {
    let mut config = db
        .get_proxy_config_for_app(app.as_str())
        .await
        .map_err(|e| format!("读取 {} 代理开关失败：{e}", app.as_str()))?;
    if config.enabled {
        config.enabled = false;
        db.update_proxy_config_for_app(config)
            .await
            .map_err(|e| format!("关闭 {} 代理开关失败：{e}", app.as_str()))?;
    }
    Ok(())
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

/// 工具配置文件所在模块的锁：和应用里其它改同一份文件的路径互斥。拿到之后
/// 不能再 `.await`。
fn file_locks(tool: Tool) -> Vec<std::sync::MutexGuard<'static, ()>> {
    let lock = |mutex: &'static std::sync::Mutex<()>| {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    };
    match tool {
        Tool::OpenCode => vec![lock(crate::opencode_config::opencode_config_lock())],
        Tool::OpenClaw => vec![lock(crate::openclaw_config::openclaw_write_lock())],
        Tool::Hermes => vec![lock(crate::hermes_config::hermes_write_lock())],
        Tool::Codex | Tool::Claude | Tool::Gemini => Vec::new(),
    }
}

/// 已经绑定的工具（切换模型时）：按 DB 模板重写接入字段，其余内容不动。
pub(crate) async fn rewrite_bound_config(
    db: &Database,
    tool: Tool,
    api_key: &str,
) -> Result<(), String> {
    let _guard = BIND_LOCK.lock().await;
    let _file_locks = file_locks(tool);
    if tool == Tool::Codex {
        codex::migrate_legacy_shape(|| Some(api_key.to_string()))?;
    }
    let template = template(db, tool)?;
    let mut txn = FileTxn::new();
    if let Err(error) = write_bound(tool, &template, api_key, &mut txn) {
        return Err(rollback_with(txn, error));
    }
    Ok(())
}

/// 启动时把旧版本绑定的 Codex 迁成「服务商自带 key」，堵住 ChatGPT 令牌外发。
pub(crate) async fn migrate_codex_on_startup() {
    let _guard = BIND_LOCK.lock().await;
    let stored_key = || {
        let slot = crate::ofox_secret::Slot::ApiKey {
            tool: AppType::Codex.into(),
        };
        match crate::ofox_secret::default_store().load(slot) {
            Ok(key) => key,
            Err(e) => {
                log::warn!("[ofox_bind] 读取 Codex 的 Ofox key 失败：{e}");
                None
            }
        }
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
    let _file_locks = file_locks(tool);
    if tool == Tool::Codex {
        codex::migrate_legacy_shape(|| Some(api_key.to_string()))?;
    }
    let template = template(db, tool)?;
    let app = tool.app();
    let previous = previous_provider(db, &app)?;

    let existing = load_record(db, tool)?;
    let previous_text = existing.as_ref().map(|(text, _)| text.clone());
    let bound = bound_on_disk(tool)?;
    let envelope = match existing {
        // 磁盘上还是 Ofox 的配置：沿用记录，只加绑定方。
        Some((_, StoredRecord::Envelope(mut envelope))) if bound => {
            envelope.holders.insert(holder.to_string());
            envelope
        }
        // 旧版本的记录没有绑定前快照；带上它，解绑时还能用里面的默认路由。
        Some((_, StoredRecord::Legacy(record))) if bound => {
            BindEnvelope::legacy_adopted(holder, Some(record))
        }
        // 磁盘上已经是 Ofox 的配置、又没有记录（旧版本绑定的）：不能把它当成原样。
        None if bound => BindEnvelope::legacy_adopted(holder, None),
        // 没绑定过，或者上次解绑后记录没删掉：重新拍快照。
        _ => BindEnvelope::snapshot(
            holder,
            previous.clone(),
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
        .and_then(|()| set_current_provider(db, &app, tool.provider_id()));
    if let Err(error) = result {
        let mut error = rollback_with(txn, error);
        if let Err(e) = write_previous_provider(db, &app, &previous) {
            error.push_str(&format!("；还原当前服务商也失败：{e}"));
        }
        let record_rollback = match previous_text {
            Some(text) => db.upsert_bind_record(key, &text),
            None => db.delete_bind_record(key),
        };
        if let Err(e) = record_rollback {
            error.push_str(&format!("；还原绑定记录也失败：{e}"));
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
            let current = read_text(&path)?;
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
        return Err(rollback_with(txn, error));
    }
    Ok(dirs_to_prune)
}

/// 快照记的是拍快照时的路径；之后改过配置目录的话，现在的路径上可能也有 Ofox
/// 的配置。
fn paths_moved(envelope: &BindEnvelope) -> bool {
    envelope
        .files
        .iter()
        .any(|baseline| baseline.file.current_path() != Path::new(&baseline.path))
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
            let error = format!("清理 {} 失败：{e}", display_path(&edit.path));
            return Err(rollback_with(txn, error));
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

    if !dry_run {
        disable_proxy_flag(db, &tool.app()).await?;
    }
    let _file_locks = file_locks(tool);

    match stored {
        Some(StoredRecord::Envelope(envelope)) if envelope.kind == RecordKind::Snapshot => {
            let dirs_to_prune = restore_snapshot(&envelope, &mut report, dry_run)?;
            if paths_moved(&envelope) && bound_on_disk(tool)? {
                apply_legacy_edits(legacy_edits(tool, None)?, &mut report, dry_run)?;
                report.warn("leftoverRemoved", None);
            }
            let target = provider_target(db, tool, Some(&envelope.previous_provider), &mut report)?;
            if !dry_run {
                apply_provider_target(db, tool, target)?;
                dirs_to_prune
                    .iter()
                    .for_each(|dir| remove_dir_if_empty(dir));
            }
        }
        stored => {
            let legacy = match &stored {
                Some(StoredRecord::Legacy(value)) => Some(value),
                Some(StoredRecord::Envelope(envelope)) => envelope.legacy.as_ref(),
                None => None,
            };
            let found = apply_legacy_edits(legacy_edits(tool, legacy)?, &mut report, dry_run)?;
            // 上次清理后服务商没切回来（比如中途出错）：这次补上。
            if found || provider_is_ofox(db, tool)? {
                report.legacy = found;
                let target = provider_target(db, tool, None, &mut report)?;
                if !dry_run {
                    apply_provider_target(db, tool, target)?;
                }
            } else {
                report.already_unbound = true;
            }
        }
    }

    if !dry_run {
        if let Err(e) = db.delete_bind_record(key) {
            // 文件已经还原；下次绑定看到磁盘上不是 Ofox 的配置会重新拍快照。
            log::warn!("[ofox_bind] 删除 {} 绑定记录失败：{e}", tool.label());
            report.warn("recordCleanupFailed", None);
        }
    }
    Ok(report)
}
