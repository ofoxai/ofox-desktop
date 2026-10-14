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

use super::json_file::KeyStyle;
use super::{env_file, json_file, read_text, ManagedFile, Tool};
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
            .filter(|selection| selection.provider == super::opencode_state::PROVIDER_ID)
            .map(|selection| selection.model),
    };
    Ok(model.filter(|model| !model.trim().is_empty()))
}

/// Claude、Gemini 启动时 env 里的模型优先级比 settings 里的高，而用户在 agent 里
/// 「记住」的模型（Claude 的 `model`、Gemini 的 `model.name`）写在 settings 里。这里把
/// env 对齐到它，不然用户的选择下次启动不生效。只认具体的模型名，别名（`sonnet`、
/// `auto-gemini-3` 之类）由 agent 自己解析，不动。返回是否改了文件。
fn align_env_with_remembered_model(tool: Tool) -> Result<bool, String> {
    let mut txn = FileTxn::new();
    let changed = match tool {
        Tool::Claude => {
            let path = ManagedFile::ClaudeSettings.current_path();
            let Some(text) = read_text(&path)? else {
                return Ok(false);
            };
            let before = json_file::parse_object(Some(&text), "Claude")?;
            let Some(model) = before
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|model| model.contains('/'))
            else {
                return Ok(false);
            };
            let env_model =
                json_file::get(&before, &["env", "ANTHROPIC_MODEL"]).and_then(Value::as_str);
            if env_model == Some(model) {
                return Ok(false);
            }
            let mut after = before.clone();
            json_file::set(&mut after, &["env", "ANTHROPIC_MODEL"], model.into());
            let rendered = json_file::render_changed(&text, KeyStyle::Json, &before, &after)?;
            txn.write(&path, rendered.as_bytes())
                .map_err(|e| format!("写入 Claude 配置失败：{e}"))?;
            true
        }
        Tool::Gemini => {
            let settings_path = ManagedFile::GeminiSettings.current_path();
            let Some(settings) = read_text(&settings_path)? else {
                return Ok(false);
            };
            let settings = json_file::parse_object(Some(&settings), "Gemini")?;
            let Some(model) = json_file::get(&settings, &["model", "name"])
                .and_then(Value::as_str)
                .map(crate::gemini_config::normalize_gemini_env_model)
                .map(|model| model.trim().to_string())
                .filter(|model| is_concrete_gemini_model(model))
            else {
                return Ok(false);
            };
            let env_path = ManagedFile::GeminiEnv.current_path();
            let Some(env) = read_text(&env_path)? else {
                return Ok(false);
            };
            if env_file::get_value(&env, "GEMINI_MODEL").as_deref() == Some(model.as_str()) {
                return Ok(false);
            }
            let updated = env_file::set_value(&env, "GEMINI_MODEL", Some(&model));
            // `.env` 里有 key，和绑定时一样只给自己读写。
            txn.write_with_mode(&env_path, updated.as_bytes(), super::gemini::ENV_MODE)
                .map_err(|e| format!("写入 Gemini 配置失败：{e}"))?;
            true
        }
        _ => false,
    };
    Ok(changed)
}

fn is_concrete_gemini_model(model: &str) -> bool {
    !model.is_empty()
        && !model.starts_with("auto")
        && !matches!(model, "pro" | "flash" | "flash-lite")
}

/// 把配置文件里的模型写进模板（数据库里的 `ofox-<app>` 服务商）；模板已经是它就
/// 不动。返回是否改了模板。调用方要先确认配置还是 Ofox 的接入方式。
pub(super) fn adopt_live_model(db: &Database, tool: Tool) -> Result<bool, String> {
    align_env_with_remembered_model(tool)?;
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
