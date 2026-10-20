use crate::config::write_json_file;
use crate::error::AppError;
use crate::provider::OpenCodeProviderConfig;
use crate::settings::get_opencode_override_dir;
use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

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

// 所有读改写 opencode.json 的路径使用 config_io 共享屏障。

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

    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::config::get_home_dir().join(".config"))
        .join("opencode")
}

pub fn get_opencode_config_path() -> PathBuf {
    get_opencode_dir().join("opencode.json")
}

pub(crate) fn global_config_paths() -> Vec<PathBuf> {
    let dir = get_opencode_dir();
    ["config.json", "opencode.json", "opencode.jsonc"]
        .into_iter()
        .map(|name| dir.join(name))
        .collect()
}
pub(crate) fn has_global_config() -> bool {
    global_config_paths().iter().any(|path| path.exists())
}

pub(crate) fn merge_global_value(base: &mut Value, overlay: Value) {
    if let (Some(base_map), Some(overlay_map)) = (base.as_object_mut(), overlay.as_object()) {
        for (key, value) in overlay_map {
            if let Some(current) = base_map.get_mut(key) {
                merge_global_value(current, value.clone());
            } else {
                base_map.insert(key.clone(), value.clone());
            }
        }
    } else {
        *base = overlay;
    }
}

pub(crate) fn get_opencode_active_config_path() -> PathBuf {
    let dir = get_opencode_dir();
    ["opencode.jsonc", "opencode.json", "config.json"]
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.exists())
        .unwrap_or_else(|| dir.join("opencode.json"))
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

    let content = crate::services::library::coordinator::read(path)?
        .ok_or_else(|| AppError::Config("library.targetUnreadable".into()))?;
    crate::services::library::mcp_codec::parse_native(&content, "opencode")
}

pub fn read_opencode_config() -> Result<Value, AppError> {
    let _io = crate::config_io::lock();
    let mut value = json!({"$schema":"https://opencode.ai/config.json"});
    for path in global_config_paths() {
        if path.exists() {
            merge_global_value(&mut value, read_opencode_config_from_path(&path)?);
        }
    }
    Ok(value)
}

fn write_opencode_config_to_path(path: &Path, config: &Value) -> Result<(), AppError> {
    write_json_file(path, config)?;

    log::debug!("OpenCode config written to {path:?}");
    Ok(())
}

#[allow(dead_code)]
pub fn write_opencode_config(config: &Value) -> Result<(), AppError> {
    let _guard = crate::config_io::lock();
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

        if !model.get("provider").is_some_and(Value::is_object) {
            model.insert("provider".to_string(), json!({}));
            changed = true;
        }
        let provider = model
            .get_mut("provider")
            .and_then(Value::as_object_mut)
            .expect("provider was normalized to an object");
        // Keep an explicit result from the streaming compatibility check.
        // Legacy GLM entries without a transport still receive the safe Chat
        // default, but startup migration must not undo a user's newer choice.
        if provider.get("npm").and_then(Value::as_str).is_none() {
            provider.insert("npm".to_string(), json!(OFOX_CHAT_NPM));
            changed = true;
        }
        if provider.get("npm").and_then(Value::as_str) == Some(OFOX_CHAT_NPM) {
            if model.get("reasoning").and_then(Value::as_bool) != Some(true) {
                model.insert("reasoning".to_string(), json!(true));
                changed = true;
            }
            if model.get("interleaved").and_then(Value::as_str) != Some("reasoning_content") {
                model.insert("interleaved".to_string(), json!("reasoning_content"));
                changed = true;
            }
        }
    }

    changed
}

/// Upgrade an already-bound OpenCode live config in place. This runs at app
/// startup and is deliberately scoped to the Ofox-managed provider id.
pub fn migrate_ofox_provider_transport() -> Result<bool, AppError> {
    let _guard = crate::config_io::lock();
    let mut full_config = read_opencode_config()?;
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
    set_provider(OFOX_PROVIDER_ID, provider.clone())?;
    Ok(true)
}

pub fn set_provider(id: &str, mut config: Value) -> Result<(), AppError> {
    let _io = crate::config_io::lock();
    let path = get_opencode_active_config_path();
    if id == OFOX_PROVIDER_ID {
        normalize_ofox_provider_transport(&mut config);
    }
    let original =
        crate::services::library::coordinator::read(&path)?.unwrap_or_else(|| "{}".into());
    crate::services::library::mcp_codec::parse_native(&original, "opencode")?;
    let output = crate::json5_sections::edit_object_paths(
        &original,
        crate::json5_sections::KeyStyle::Json,
        &[(vec!["provider".into(), id.into()], Some(config))],
    )?;
    crate::config::atomic_write(&path, output.as_bytes())
}
pub fn remove_provider(id: &str) -> Result<(), AppError> {
    let _io = crate::config_io::lock();
    // A lower-layer entry would reappear after deleting only the highest one.
    // Validate and prepare all files first, then commit with rollback.
    let mut writes = Vec::new();
    for path in global_config_paths() {
        let Some(original) = crate::services::library::coordinator::read(&path)? else {
            continue;
        };
        let root = crate::services::library::mcp_codec::parse_native(&original, "opencode")?;
        if root.get("provider").and_then(|v| v.get(id)).is_none() {
            continue;
        }
        let output = crate::json5_sections::edit_object_paths(
            &original,
            crate::json5_sections::KeyStyle::Json,
            &[(vec!["provider".into(), id.into()], None)],
        )?;
        writes.push((path, output));
    }
    let mut txn = crate::config::FileTxn::new();
    for (path, output) in writes {
        if let Err(error) = txn.write(&path, output.as_bytes()) {
            txn.rollback()?;
            return Err(error);
        }
    }
    Ok(())
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

#[cfg(test)]
pub fn get_mcp_servers() -> Result<Map<String, Value>, AppError> {
    let config = read_opencode_config()?;
    Ok(config
        .get("mcp")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default())
}

#[cfg(test)]
pub fn set_mcp_server(id: &str, config: Value) -> Result<(), AppError> {
    let _guard = crate::config_io::lock();
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

#[cfg(test)]
pub fn remove_mcp_server(id: &str) -> Result<(), AppError> {
    let _guard = crate::config_io::lock();
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
    let _guard = crate::config_io::lock();
    let path = get_opencode_active_config_path();
    let original =
        crate::services::library::coordinator::read(&path)?.unwrap_or_else(|| "{}".into());
    crate::services::library::mcp_codec::parse_native(&original, "opencode")?;
    let mut config = read_opencode_config()?;
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

    let output = crate::json5_sections::edit_object_paths(
        &original,
        crate::json5_sections::KeyStyle::Json,
        &[(vec!["plugin".into()], config.get("plugin").cloned())],
    )?;
    crate::config::atomic_write(&path, output.as_bytes())
}

pub fn remove_plugins_by_prefixes(prefixes: &[&str]) -> Result<(), AppError> {
    let _guard = crate::config_io::lock();
    let path = get_opencode_active_config_path();
    let original =
        crate::services::library::coordinator::read(&path)?.unwrap_or_else(|| "{}".into());
    crate::services::library::mcp_codec::parse_native(&original, "opencode")?;
    let mut config = read_opencode_config()?;

    if let Some(arr) = config.get_mut("plugin").and_then(|v| v.as_array_mut()) {
        arr.retain(|v| {
            v.as_str()
                .map(|s| !matches_any_plugin_prefix(s, prefixes))
                .unwrap_or(true)
        });

        // An explicit empty overlay prevents lower-layer plugins reappearing.
    }

    let output = crate::json5_sections::edit_object_paths(
        &original,
        crate::json5_sections::KeyStyle::Json,
        &[(vec!["plugin".into()], config.get("plugin").cloned())],
    )?;
    crate::config::atomic_write(&path, output.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn removing_the_last_overlay_plugin_does_not_revive_a_lower_layer() {
        let _sandbox = crate::test_sandbox::sandbox();
        let dir = get_opencode_dir();
        crate::config::atomic_write(
            &dir.join("config.json"),
            b"{/* MCP */\"mcp\":{},\"plugin\":[\"oh-my-openagent\"]}",
        )
        .unwrap();
        crate::config::atomic_write(&dir.join("opencode.jsonc"), b"{}").unwrap();
        remove_plugins_by_prefixes(&["oh-my-openagent"]).unwrap();
        assert!(!read_opencode_config().unwrap()["plugin"]
            .as_array()
            .is_some_and(|plugins| plugins.iter().any(|plugin| plugin == "oh-my-openagent")));
        assert!(std::fs::read_to_string(dir.join("config.json"))
            .unwrap()
            .contains("/* MCP */"));
    }

    #[test]
    #[serial_test::serial]
    fn native_provider_read_write_use_the_same_global_layers() {
        let _sandbox = crate::test_sandbox::sandbox();
        let dir = get_opencode_dir();
        crate::config::atomic_write(
            &dir.join("config.json"),
            br#"{"provider":{"lower":{"options":{"baseURL":"https://example.org"}}}}"#,
        )
        .unwrap();
        for shadow in [false, true] {
            if shadow {
                crate::config::atomic_write(&dir.join("opencode.json"), br#"{"model":"shadow"}"#)
                    .unwrap();
            }
            crate::config::atomic_write(&dir.join("opencode.jsonc"), b"{/* MCP */\"mcp\":{}}")
                .unwrap();
            set_provider(
                "demo",
                json!({"options":{"baseURL":"https://example.org/a"}}),
            )
            .unwrap();
            assert!(get_providers().unwrap().contains_key("demo"));
            assert!(get_providers().unwrap().contains_key("lower"));
            set_provider(
                "demo",
                json!({"options":{"baseURL":"https://example.org/b"}}),
            )
            .unwrap();
            assert_eq!(
                get_providers().unwrap()["demo"]["options"]["baseURL"],
                "https://example.org/b"
            );
            remove_provider("demo").unwrap();
            assert!(!get_providers().unwrap().contains_key("demo"));
            assert!(std::fs::read_to_string(dir.join("opencode.jsonc"))
                .unwrap()
                .contains("/* MCP */"));
        }
    }

    #[test]
    #[serial_test::serial]
    fn startup_and_plugins_preserve_mcp_comments_in_the_active_file() {
        let _sandbox = crate::test_sandbox::sandbox();
        let path = get_opencode_dir().join("opencode.jsonc");
        let source = "{\n/* private MCP */\n\"mcp\":{\"demo\":{\"type\":\"local\",\"command\":[\"server\", /* args note */ \"a\"]}},\"provider\":{\"ofox-opencode\":{\"npm\":\"old\"}},\"plugin\":[\"oh-my-openagent\"]\n}";
        for action in ["migration", "add", "remove"] {
            crate::config::atomic_write(&path, source.as_bytes()).unwrap();
            match action {
                "migration" => assert!(migrate_ofox_provider_transport().unwrap()),
                "add" => add_plugin("other-plugin").unwrap(),
                _ => remove_plugins_by_prefixes(&["oh-my-openagent"]).unwrap(),
            }
            let result = std::fs::read_to_string(&path).unwrap();
            assert!(result.contains("/* private MCP */"), "{action}");
            assert!(result.contains("/* args note */"), "{action}");
            let before =
                crate::services::library::mcp_codec::parse_native(source, "opencode").unwrap();
            let after =
                crate::services::library::mcp_codec::parse_native(&result, "opencode").unwrap();
            assert_eq!(before["mcp"], after["mcp"]);
        }
    }

    #[test]
    #[serial_test::serial]
    fn provider_changes_preserve_mcp_comments_and_jsonc_precedence() {
        let _sandbox = crate::test_sandbox::sandbox();
        let dir = get_opencode_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("opencode.json"), "{\"shadow\":true}").unwrap();
        let original =
            "{\n // private MCP\n \"mcp\": {\"demo\":{\"command\":[\"personal\"],\"type\":\"local\"}}, \"model\":\"mine\"\n}";
        std::fs::write(dir.join("opencode.jsonc"), original).unwrap();
        set_provider("test", json!({"npm":"test"})).unwrap();
        let added = std::fs::read_to_string(dir.join("opencode.jsonc")).unwrap();
        assert!(added.contains("// private MCP"));
        assert!(json5::from_str::<Value>(&added).unwrap()["provider"]
            .get("test")
            .is_some());
        remove_provider("test").unwrap();
        let result = std::fs::read_to_string(dir.join("opencode.jsonc")).unwrap();
        assert!(result.contains("// private MCP"));
        assert_eq!(
            json5::from_str::<Value>(&result).unwrap()["mcp"]["demo"]["command"],
            json!(["personal"])
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("opencode.json")).unwrap(),
            "{\"shadow\":true}"
        );
    }

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

    #[test]
    fn explicit_glm_transport_survives_normalization() {
        let mut provider = json!({
            "npm": "@ai-sdk/openai",
            "models": {
                "z-ai/glm-5.3": {
                    "name": "GLM 5.3",
                    "provider": { "npm": "@ai-sdk/openai" }
                }
            }
        });
        assert!(!normalize_ofox_provider_transport(&mut provider));
        assert_eq!(
            provider.pointer("/models/z-ai~1glm-5.3/provider/npm"),
            Some(&json!("@ai-sdk/openai"))
        );
    }
}
