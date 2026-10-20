//! 用户在 CLI 里选的模型，和桌面端存的模板保持一致。
//!
//! 模型是用户的选择，不算接入配置，改了不算冲突（见 `status`）。但「换 key」和
//! 「启动修复」会按模板整份重写受管字段，模板里还是旧模型的话，用户选的模型就被
//! 改回去了。所以重写之前先把配置文件里的模型采纳进模板。
//!
//! Codex、Claude、Gemini 的「当前模型」就是配置里的一个值。OpenClaw、Hermes 是默认
//! 路由里 Ofox 条目下的模型 id（模板里是只有这一个模型的列表）。OpenCode 的当前模型
//! 在 TUI 状态文件里（见 `opencode_state`），也是 Ofox 条目下的模型 id。

use std::path::PathBuf;

use serde_json::Value;

use super::{json_file, read_text, ManagedFile, Tool};
use crate::commands::manage_tool::{read_model_from_settings, write_model_into_settings};
use crate::config::FileTxn;
use crate::database::Database;

/// 配置文件里当前选的模型；没有这个字段（或是空的）、文件不在、不是支持的工具，
/// 都返回 `None`，不当成「用户想清空模型」。
fn read_live_model(tool: Tool) -> Result<Option<String>, String> {
    let model = match tool {
        Tool::Codex => {
            let Some(text) = read_text(&ManagedFile::CodexConfig.current_path())? else {
                return Ok(None);
            };
            let config: toml::Value =
                toml::from_str(&text).map_err(|_| "Codex 配置不是有效的 TOML".to_string())?;
            config
                .get("model")
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        }
        Tool::Claude => {
            let Some(text) = read_text(&ManagedFile::ClaudeSettings.current_path())? else {
                return Ok(None);
            };
            super::claude::live_model(&text, &super::claude::catalog_families())?
        }
        Tool::Gemini => {
            let Some(text) = read_text(&ManagedFile::GeminiSettings.current_path())? else {
                return Ok(None);
            };
            super::gemini::live_model(&text)?
        }
        Tool::OpenClaw => {
            let Some(text) = read_text(&ManagedFile::OpenClawConfig.current_path())? else {
                return Ok(None);
            };
            let config = json_file::parse_object(Some(&text), "OpenClaw")?;
            json_file::get(&config, &["agents", "defaults", "model"])
                .and_then(|model| model.as_str().or_else(|| model.get("primary")?.as_str()))
                .and_then(|primary| primary.strip_prefix("ofox-openclaw/"))
                .map(str::to_string)
        }
        Tool::Hermes => {
            let Some(text) = read_text(&ManagedFile::HermesConfig.current_path())? else {
                return Ok(None);
            };
            let config = crate::hermes_config::parse_config_text(&text)
                .map_err(|_| "Hermes 配置不是有效的 YAML".to_string())?;
            let model = config.get("model");
            let routed = model
                .and_then(|m| m.get("provider"))
                .and_then(|p| p.as_str())
                == Some("ofox-hermes");
            model
                .filter(|_| routed)
                .and_then(|m| m.get("default")?.as_str())
                .map(str::to_string)
        }
        Tool::OpenCode => super::opencode_state::live_selection()?
            .filter(|selection| selection.provider == super::opencode_state::PROVIDER_ID)
            .map(|selection| selection.model),
    };
    Ok(model.filter(|model| !model.trim().is_empty()))
}

/// 采纳之前先把配置规整好（见 [`normalize_edits`]）。写了就和其它 Ofox 写入一样记进
/// 绑定记录（记不进去就回滚）。返回是否改了文件。
fn normalize_before_adopt(db: &Database, tool: Tool) -> Result<bool, String> {
    write_recorded(db, tool, normalize_edits(tool)?)
}

/// 采纳前要写的文件、内容和权限：Gemini 旧版本写在 `.env` 的 `GEMINI_MODEL` 会盖掉
/// 用户记住的 `model.name`，先去掉它（见 [`super::gemini::migrated`]），采纳时读到的才是
/// 用户的选择。
fn normalize_edits(tool: Tool) -> Result<Vec<(PathBuf, String, Option<u32>)>, String> {
    match tool {
        Tool::Gemini => super::gemini::migration_edits(),
        _ => Ok(Vec::new()),
    }
}

/// 采纳之后按模板规整：Claude 旧版本写的 `ANTHROPIC_MODEL` 比 `/model` 记住的模型
/// 优先，迁成默认模型，家族别名跟上最新的模型目录（见 [`super::claude::normalized`]）。
/// 先采纳再规整：旧绑定里用户的模型可能只在 `ANTHROPIC_MODEL` 里。
fn normalize_after_adopt(db: &Database, tool: Tool) -> Result<bool, String> {
    if tool != Tool::Claude {
        return Ok(false);
    }
    let path = ManagedFile::ClaudeSettings.current_path();
    let Some(text) = read_text(&path)? else {
        return Ok(false);
    };
    let template = super::template(db, tool)?;
    let edits = super::claude::normalized(&text, &template, &super::claude::catalog_families())?
        .map(|updated| (path, updated, None));
    write_recorded(db, tool, edits.into_iter().collect())
}

/// 写入这些文件并记进绑定记录；记不进去就整批回滚。没有要写的返回 `false`。
fn write_recorded(
    db: &Database,
    tool: Tool,
    edits: Vec<(PathBuf, String, Option<u32>)>,
) -> Result<bool, String> {
    if edits.is_empty() {
        return Ok(false);
    }
    let before = super::read_leaves(tool)?;
    let mut txn = FileTxn::new();
    let result = edits
        .iter()
        .try_for_each(|(path, text, mode)| match mode {
            Some(mode) => txn.write_with_mode(path, text.as_bytes(), *mode),
            None => txn.write(path, text.as_bytes()),
        })
        .map_err(|e| format!("写入 {} 配置失败：{e}", tool.label()))
        .and_then(|()| super::record_written(db, tool, &before));
    if let Err(error) = result {
        return Err(super::rollback_with(txn, error));
    }
    Ok(true)
}

/// 把配置文件里的模型写进模板（数据库里的 `ofox-<app>` 服务商）；模板已经是它就
/// 不动。返回是否改了模板。调用方要先确认配置还是 Ofox 的接入方式。
pub(super) fn adopt_live_model(db: &Database, tool: Tool) -> Result<bool, String> {
    normalize_before_adopt(db, tool)?;
    let adopted = adopt(db, tool)?;
    normalize_after_adopt(db, tool)?;
    Ok(adopted)
}

fn adopt(db: &Database, tool: Tool) -> Result<bool, String> {
    let Some(live) = read_live_model(tool)? else {
        return Ok(false);
    };
    let app = tool.app();
    let provider = db
        .get_provider_by_id(tool.provider_id(), app.as_str())
        .map_err(|e| format!("读取 Ofox {} 模板失败：{e}", tool.label()))?
        .ok_or_else(|| format!("Ofox {} 模板缺失", tool.label()))?;
    let mut updated = provider.settings_config.clone();
    if tool == Tool::OpenCode {
        // 模板里是用户选的一组模型：在 TUI 里选了组外的就加进去，不能把别的丢掉。
        let models = updated
            .get_mut("models")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "Ofox OpenCode 模板缺少 models".to_string())?;
        if models.contains_key(&live) {
            return Ok(false);
        }
        models.insert(live.clone(), serde_json::json!({ "name": live }));
        db.update_provider_settings_config(app.as_str(), tool.provider_id(), &updated)
            .map_err(|e| format!("保存 {} 的模型失败：{e}", tool.label()))?;
        return Ok(true);
    }
    write_model_into_settings(&app, &mut updated, &live)?;
    // 比的是读出来的模型：Gemini 会把前缀规整掉，直接比文件里的值会永远「不同」。
    if read_model_from_settings(&app, &updated)
        == read_model_from_settings(&app, &provider.settings_config)
    {
        return Ok(false);
    }
    db.update_provider_settings_config(app.as_str(), tool.provider_id(), &updated)
        .map_err(|e| format!("保存 {} 的模型失败：{e}", tool.label()))?;
    Ok(true)
}

/// 主页显示用：用户在 agent 里实际选的模型。OpenCode 的模板里是一组模型，当前用哪个
/// 以 TUI 里选的为准（Ofox 条目或用户自己配的、同样指向 Ofox 网关的条目）；OpenClaw、
/// Hermes 选了自己配的 Ofox 网关条目时显示它的模型（不写进模板，那个条目不归桌面端
/// 管）。选了别家，或者不是这几个工具，返回 `None`。
pub(super) fn display_model(tool: Tool) -> Result<Option<String>, String> {
    let model = match tool {
        Tool::OpenCode => {
            let Some(selection) = super::opencode_state::live_selection()? else {
                return Ok(None);
            };
            (selection.provider == super::opencode_state::PROVIDER_ID
                || super::opencode_state::is_gateway(&selection.provider))
            .then_some(selection.model)
        }
        Tool::OpenClaw => {
            let config = own_config(&ManagedFile::OpenClawConfig, "OpenClaw")?;
            json_file::get(&config, &["agents", "defaults", "model"])
                .and_then(|model| model.as_str().or_else(|| model.get("primary")?.as_str()))
                .and_then(|primary| primary.split_once('/'))
                .filter(|(provider, _)| {
                    *provider != "ofox-openclaw"
                        && super::route::openclaw_is_gateway(&config, provider)
                })
                .map(|(_, model)| model.to_string())
        }
        Tool::Hermes => {
            let Some(text) = read_text(&ManagedFile::HermesConfig.current_path())? else {
                return Ok(None);
            };
            let config = crate::hermes_config::parse_config_text(&text)
                .map_err(|_| "Hermes 配置不是有效的 YAML".to_string())
                .and_then(|yaml| serde_json::to_value(yaml).map_err(|e| e.to_string()))?;
            let model = config.get("model");
            let provider = model.and_then(|m| m.get("provider")?.as_str());
            provider
                .filter(|name| {
                    *name != "ofox-hermes" && super::route::hermes_is_gateway(&config, name)
                })
                .and_then(|_| model?.get("default")?.as_str())
                .map(str::to_string)
        }
        _ => None,
    };
    Ok(model.filter(|model| !model.trim().is_empty()))
}

fn own_config(file: &ManagedFile, label: &str) -> Result<Value, String> {
    let text = read_text(&file.current_path())?;
    json_file::parse_object(text.as_deref(), label)
}

/// 主页显示用：用户选的是自己配的 Ofox 网关条目时显示它的模型，否则用 `saved`。
pub(crate) fn display_model_or(tool: Tool, saved: String) -> String {
    match display_model(tool) {
        Ok(Some(live)) => live,
        _ => saved,
    }
}
