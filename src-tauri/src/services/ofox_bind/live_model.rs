//! 用户在 CLI 里选的模型，和桌面端存的模板保持一致。
//!
//! 模型是用户的选择，不算接入配置，改了不算冲突（见 `status`）。但「换 key」和
//! 「启动修复」会按模板整份重写受管字段，模板里还是旧模型的话，用户选的模型就被
//! 改回去了。所以重写之前先把配置文件里的模型采纳进模板。
//!
//! Codex、Claude、Gemini 的「当前模型」就是配置里的一个值。OpenClaw、Hermes 是默认
//! 路由里 Ofox 条目下的模型 id（模板里是只有这一个模型的列表）。OpenCode 的当前模型
//! 在 TUI 状态文件里（见 `opencode_state`），也是 Ofox 条目下的模型 id。

use serde_json::Value;

use super::{env_file, json_file, read_text, ManagedFile, Tool};
use crate::commands::manage_tool::{read_model_from_settings, write_model_into_settings};
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
            json_file::parse_object(Some(&text), "Claude")?
                .get("env")
                .and_then(|env| env.get("ANTHROPIC_MODEL"))
                .and_then(Value::as_str)
                .map(str::to_string)
        }
        Tool::Gemini => {
            let Some(text) = read_text(&ManagedFile::GeminiEnv.current_path())? else {
                return Ok(None);
            };
            env_file::get_value(&text, "GEMINI_MODEL")
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
            .filter(|(provider, _)| provider == super::opencode_state::PROVIDER_ID)
            .map(|(_, model)| model),
    };
    Ok(model.filter(|model| !model.trim().is_empty()))
}

/// 把配置文件里的模型写进模板（数据库里的 `ofox-<app>` 服务商）；模板已经是它就
/// 不动。返回是否改了模板。调用方要先确认配置还是 Ofox 的接入方式。
pub(super) fn adopt_live_model(db: &Database, tool: Tool) -> Result<bool, String> {
    let Some(live) = read_live_model(tool)? else {
        return Ok(false);
    };
    let app = tool.app();
    let provider = db
        .get_provider_by_id(tool.provider_id(), app.as_str())
        .map_err(|e| format!("读取 Ofox {} 模板失败：{e}", tool.label()))?
        .ok_or_else(|| format!("Ofox {} 模板缺失", tool.label()))?;
    let mut updated = provider.settings_config.clone();
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
