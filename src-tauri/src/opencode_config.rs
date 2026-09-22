use crate::config::write_json_file;
use crate::error::AppError;
use crate::provider::OpenCodeProviderConfig;
use crate::settings::get_opencode_override_dir;
use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const STANDARD_OMO_PLUGIN_PREFIXES: [&str; 2] = ["oh-my-openagent", "oh-my-opencode"];
const SLIM_OMO_PLUGIN_PREFIXES: [&str; 1] = ["oh-my-opencode-slim"];
pub(crate) const OFOX_PROVIDER_ID: &str = "ofox-opencode";
pub(crate) const OFOX_RESPONSES_NPM: &str = "@ai-sdk/openai";
pub(crate) const OFOX_CHAT_NPM: &str = "@ai-sdk/openai-compatible";

const GLM_53_CHAT_MODELS: [&str; 4] = [
    "glm-5.3",
    "glm-5.3-flash",
    "z-ai/glm-5.3",
    "z-ai/glm-5.3-flash",
];

fn opencode_config_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn matches_plugin_prefix(plugin_name: &str, prefix: &str) -> bool {
    plugin_name == prefix
        || plugin_name
            .strip_prefix(prefix)
            .map(|suffix| suffix.starts_with('@'))
            .unwrap_or(false)
}

fn matches_any_plugin_prefix(plugin_name: &str, prefixes: &[&str]) -> bool {
    prefixes
        .iter()
        .any(|prefix| matches_plugin_prefix(plugin_name, prefix))
}

fn canonicalize_plugin_name(plugin_name: &str) -> String {
    if let Some(suffix) = plugin_name.strip_prefix("oh-my-opencode") {
        if suffix.is_empty() || suffix.starts_with('@') {
            return format!("oh-my-openagent{suffix}");
        }
    }
    plugin_name.to_string()
}

pub fn get_opencode_dir() -> PathBuf {
    if let Some(override_dir) = get_opencode_override_dir() {
        return override_dir;
    }

    crate::config::get_home_dir()
        .join(".config")
        .join("opencode")
}

pub fn get_opencode_config_path() -> PathBuf {
    get_opencode_dir().join("opencode.json")
}

#[allow(dead_code)]
pub fn get_opencode_env_path() -> PathBuf {
    get_opencode_dir().join(".env")
}

fn read_opencode_config_from_path(path: &Path) -> Result<Value, AppError> {
    if !path.exists() {
        return Ok(json!({
            "$schema": "https://opencode.ai/config.json"
        }));
    }

    let content = std::fs::read_to_string(path).map_err(|e| AppError::io(path, e))?;
    let value: Value = json5::from_str(&content).map_err(|e| {
        AppError::Config(format!(
            "Failed to parse OpenCode config: {}: {e}",
            path.display()
        ))
    })?;
    if !value.is_object() {
        return Err(AppError::Config(format!(
            "OpenCode 配置文件根节点必须是 JSON 对象: {}",
            path.display()
        )));
    }
    Ok(value)
}

pub fn read_opencode_config() -> Result<Value, AppError> {
    read_opencode_config_from_path(&get_opencode_config_path())
}

fn write_opencode_config_to_path(path: &Path, config: &Value) -> Result<(), AppError> {
    write_json_file(path, config)?;

    log::debug!("OpenCode config written to {path:?}");
    Ok(())
}

#[allow(dead_code)]
pub fn write_opencode_config(config: &Value) -> Result<(), AppError> {
    let _guard = opencode_config_lock().lock()?;
    write_opencode_config_to_path(&get_opencode_config_path(), config)
}

pub fn get_providers() -> Result<Map<String, Value>, AppError> {
    let config = read_opencode_config()?;
    Ok(config
        .get("provider")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default())
}

/// Keep Responses as Ofox's default OpenCode transport, while routing models
/// with a known Responses-stream incompatibility through Chat Completions.
///
/// GLM 5.3 Responses streams currently emit a signed reasoning item id in
/// `response.output_item.added`, then refer to the same item with the signature
/// removed in later reasoning events. OpenCode cannot match those events and
/// fails with `reasoning part ... not found`. The Chat Completions stream is
/// internally consistent, so OpenCode's per-model provider override is the
/// narrowest safe workaround until the gateway normalizes those ids.
pub(crate) fn normalize_ofox_provider_transport(config: &mut Value) -> bool {
    let Some(root) = config.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    if root.get("npm").and_then(Value::as_str) != Some(OFOX_RESPONSES_NPM) {
        root.insert("npm".to_string(), json!(OFOX_RESPONSES_NPM));
        changed = true;
    }

    let Some(models) = root.get_mut("models").and_then(Value::as_object_mut) else {
        return changed;
    };
    for model_id in GLM_53_CHAT_MODELS {
        let Some(model) = models.get_mut(model_id).and_then(Value::as_object_mut) else {
            continue;
        };

        if model.get("reasoning").and_then(Value::as_bool) != Some(true) {
            model.insert("reasoning".to_string(), json!(true));
            changed = true;
        }
        if model.get("interleaved").and_then(Value::as_str) != Some("reasoning_content") {
            model.insert("interleaved".to_string(), json!("reasoning_content"));
            changed = true;
        }

        if !model.get("provider").is_some_and(Value::is_object) {
            model.insert("provider".to_string(), json!({}));
            changed = true;
        }
        let provider = model
            .get_mut("provider")
            .and_then(Value::as_object_mut)
            .expect("provider was normalized to an object");
        if provider.get("npm").and_then(Value::as_str) != Some(OFOX_CHAT_NPM) {
            provider.insert("npm".to_string(), json!(OFOX_CHAT_NPM));
            changed = true;
        }
    }

    changed
}

/// Upgrade an already-bound OpenCode live config in place. This runs at app
/// startup and is deliberately scoped to the Ofox-managed provider id.
pub fn migrate_ofox_provider_transport() -> Result<bool, AppError> {
    let _guard = opencode_config_lock().lock()?;
    let path = get_opencode_config_path();
    let mut full_config = read_opencode_config_from_path(&path)?;
    let Some(provider) = full_config
        .get_mut("provider")
        .and_then(Value::as_object_mut)
        .and_then(|providers| providers.get_mut(OFOX_PROVIDER_ID))
    else {
        return Ok(false);
    };

    if !normalize_ofox_provider_transport(provider) {
        return Ok(false);
    }
    write_opencode_config_to_path(&path, &full_config)?;
    Ok(true)
}

pub fn set_provider(id: &str, mut config: Value) -> Result<(), AppError> {
    let _guard = opencode_config_lock().lock()?;
    let path = get_opencode_config_path();
    let mut full_config = read_opencode_config_from_path(&path)?;

    if id == OFOX_PROVIDER_ID {
        normalize_ofox_provider_transport(&mut config);
    }

    if !full_config.get("provider").is_some_and(Value::is_object) {
        if full_config.get("provider").is_some() {
            log::warn!("opencode.json 的 provider 不是对象，已重置为空对象");
        }
        full_config["provider"] = json!({});
    }

    if let Some(providers) = full_config
        .get_mut("provider")
        .and_then(|v| v.as_object_mut())
    {
        providers.insert(id.to_string(), config);
    }

    write_opencode_config_to_path(&path, &full_config)
}

pub fn remove_provider(id: &str) -> Result<(), AppError> {
    let _guard = opencode_config_lock().lock()?;
    let path = get_opencode_config_path();
    let mut config = read_opencode_config_from_path(&path)?;

    if let Some(providers) = config.get_mut("provider").and_then(|v| v.as_object_mut()) {
        providers.remove(id);
    } else if config.get("provider").is_some() {
        log::warn!("opencode.json 的 provider 不是对象，无法删除供应商 '{id}'");
    }

    write_opencode_config_to_path(&path, &config)
}

fn fill_missing_model_names(value: &mut Value) {
    let Some(models) = value.get_mut("models").and_then(Value::as_object_mut) else {
        return;
    };
    for (model_id, model) in models {
        if let Some(model) = model.as_object_mut() {
            model
                .entry("name".to_string())
                .or_insert_with(|| Value::String(model_id.clone()));
        }
    }
}

pub fn get_typed_providers() -> Result<IndexMap<String, OpenCodeProviderConfig>, AppError> {
    let providers = get_providers()?;
    let mut result = IndexMap::new();

    for (id, mut value) in providers {
        // Ofox versions before 1.3.2 stored a selected model as
        // `"models": {"id": {}}`. OpenCode accepts it, but our typed model
        // requires a display name and would skip the whole provider on app
        // restart. Heal that legacy shape in memory; the next model save will
        // persist the canonical `{ "name": "id" }` representation.
        fill_missing_model_names(&mut value);

        match serde_json::from_value::<OpenCodeProviderConfig>(value) {
            Ok(config) => {
                result.insert(id, config);
            }
            Err(e) => {
                log::warn!("Failed to parse provider '{id}': {e}");
            }
        }
    }

    Ok(result)
}

pub fn set_typed_provider(id: &str, config: &OpenCodeProviderConfig) -> Result<(), AppError> {
    let value = serde_json::to_value(config).map_err(|e| AppError::JsonSerialize { source: e })?;
    set_provider(id, value)
}

pub fn get_mcp_servers() -> Result<Map<String, Value>, AppError> {
    let config = read_opencode_config()?;
    Ok(config
        .get("mcp")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default())
}

pub fn set_mcp_server(id: &str, config: Value) -> Result<(), AppError> {
    let _guard = opencode_config_lock().lock()?;
    let path = get_opencode_config_path();
    let mut full_config = read_opencode_config_from_path(&path)?;

    if !full_config.get("mcp").is_some_and(Value::is_object) {
        if full_config.get("mcp").is_some() {
            log::warn!("opencode.json 的 mcp 不是对象，已重置为空对象");
        }
        full_config["mcp"] = json!({});
    }

    if let Some(mcp) = full_config.get_mut("mcp").and_then(|v| v.as_object_mut()) {
        mcp.insert(id.to_string(), config);
    }

    write_opencode_config_to_path(&path, &full_config)
}

pub fn remove_mcp_server(id: &str) -> Result<(), AppError> {
    let _guard = opencode_config_lock().lock()?;
    let path = get_opencode_config_path();
    let mut config = read_opencode_config_from_path(&path)?;

    if let Some(mcp) = config.get_mut("mcp").and_then(|v| v.as_object_mut()) {
        mcp.remove(id);
    } else if config.get("mcp").is_some() {
        log::warn!("opencode.json 的 mcp 不是对象，无法删除服务器 '{id}'");
    }

    write_opencode_config_to_path(&path, &config)
}

pub fn add_plugin(plugin_name: &str) -> Result<(), AppError> {
    let _guard = opencode_config_lock().lock()?;
    let path = get_opencode_config_path();
    let mut config = read_opencode_config_from_path(&path)?;
    let normalized_plugin_name = canonicalize_plugin_name(plugin_name);

    let plugins = config.get_mut("plugin").and_then(|v| v.as_array_mut());

    match plugins {
        Some(arr) => {
            // Mutual exclusion: standard OMO and OMO Slim cannot coexist as plugins
            if matches_any_plugin_prefix(&normalized_plugin_name, &STANDARD_OMO_PLUGIN_PREFIXES) {
                arr.retain(|v| {
                    v.as_str()
                        .map(|s| {
                            !matches_any_plugin_prefix(s, &STANDARD_OMO_PLUGIN_PREFIXES)
                                && !matches_any_plugin_prefix(s, &SLIM_OMO_PLUGIN_PREFIXES)
                        })
                        .unwrap_or(true)
                });
            } else if matches_any_plugin_prefix(&normalized_plugin_name, &SLIM_OMO_PLUGIN_PREFIXES)
            {
                arr.retain(|v| {
                    v.as_str()
                        .map(|s| {
                            !matches_any_plugin_prefix(s, &STANDARD_OMO_PLUGIN_PREFIXES)
                                && !matches_any_plugin_prefix(s, &SLIM_OMO_PLUGIN_PREFIXES)
                        })
                        .unwrap_or(true)
                });
            }

            let already_exists = arr
                .iter()
                .any(|v| v.as_str() == Some(normalized_plugin_name.as_str()));
            if !already_exists {
                arr.push(Value::String(normalized_plugin_name));
            }
        }
        None => {
            config["plugin"] = json!([normalized_plugin_name]);
        }
    }

    write_opencode_config_to_path(&path, &config)
}

pub fn remove_plugins_by_prefixes(prefixes: &[&str]) -> Result<(), AppError> {
    let _guard = opencode_config_lock().lock()?;
    let path = get_opencode_config_path();
    let mut config = read_opencode_config_from_path(&path)?;

    if let Some(arr) = config.get_mut("plugin").and_then(|v| v.as_array_mut()) {
        arr.retain(|v| {
            v.as_str()
                .map(|s| !matches_any_plugin_prefix(s, prefixes))
                .unwrap_or(true)
        });

        if arr.is_empty() {
            config.as_object_mut().map(|obj| obj.remove("plugin"));
        }
    }

    write_opencode_config_to_path(&path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_empty_model_entries_receive_display_names() {
        let mut provider = json!({
            "models": {
                "openai/gpt-5.6-terra": {},
                "named": { "name": "Custom name" }
            }
        });

        fill_missing_model_names(&mut provider);

        assert_eq!(
            provider.pointer("/models/openai~1gpt-5.6-terra/name"),
            Some(&json!("openai/gpt-5.6-terra"))
        );
        assert_eq!(
            provider.pointer("/models/named/name"),
            Some(&json!("Custom name"))
        );
    }

    #[test]
    fn ofox_provider_uses_responses_transport() {
        let mut provider = json!({
            "npm": "@ai-sdk/openai-compatible",
            "name": "OfoxAI",
            "options": { "baseURL": "https://api.ofox.ai/v1" }
        });

        assert!(normalize_ofox_provider_transport(&mut provider));
        assert_eq!(provider.get("npm"), Some(&json!("@ai-sdk/openai")));
        assert!(!normalize_ofox_provider_transport(&mut provider));
    }

    #[test]
    fn glm_53_models_use_chat_transport_without_changing_other_models() {
        let mut provider = json!({
            "npm": "@ai-sdk/openai",
            "models": {
                "z-ai/glm-5.3": { "name": "GLM 5.3" },
                "z-ai/glm-5.3-flash": { "name": "GLM 5.3 Flash" },
                "openai/gpt-5.6-terra": { "name": "GPT 5.6 Terra" }
            }
        });

        assert!(normalize_ofox_provider_transport(&mut provider));
        for model_id in ["z-ai~1glm-5.3", "z-ai~1glm-5.3-flash"] {
            assert_eq!(
                provider.pointer(&format!("/models/{model_id}/provider/npm")),
                Some(&json!("@ai-sdk/openai-compatible"))
            );
            assert_eq!(
                provider.pointer(&format!("/models/{model_id}/interleaved")),
                Some(&json!("reasoning_content"))
            );
            assert_eq!(
                provider.pointer(&format!("/models/{model_id}/reasoning")),
                Some(&json!(true))
            );
        }
        assert_eq!(
            provider.pointer("/models/openai~1gpt-5.6-terra"),
            Some(&json!({ "name": "GPT 5.6 Terra" }))
        );
        assert!(!normalize_ofox_provider_transport(&mut provider));
    }

    #[test]
    fn glm_53_chat_override_preserves_existing_provider_fields() {
        let mut provider = json!({
            "npm": "@ai-sdk/openai",
            "models": {
                "glm-5.3": {
                    "name": "GLM 5.3",
                    "provider": { "api": "https://example.test/v1" },
                    "custom": "keep-me"
                }
            }
        });

        assert!(normalize_ofox_provider_transport(&mut provider));
        assert_eq!(
            provider.pointer("/models/glm-5.3/provider/api"),
            Some(&json!("https://example.test/v1"))
        );
        assert_eq!(
            provider.pointer("/models/glm-5.3/custom"),
            Some(&json!("keep-me"))
        );
    }
}
