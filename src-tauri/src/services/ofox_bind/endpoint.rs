//! Region changes patch only existing, recognized OFox URL fields. Deleted
//! files/fields and user endpoints are never replaced by a generated template.

use serde_json::Value;

use super::json_file::{self, JsonPath, KeyStyle};
use super::plan::read_text;
use super::{env_file, file_locks, load_record, rollback_with, Tool, BIND_LOCK};
use crate::config::FileTxn;
use crate::database::Database;

const CONFLICT: &str = "工具接入地址已被修改，无法自动同步区域，请检查现有配置。";

fn next_url(tool: Tool) -> String {
    match tool {
        Tool::Claude => crate::ofox_endpoints::anthropic_base_url(),
        Tool::Gemini => crate::ofox_endpoints::gemini_base_url(),
        _ => crate::ofox_endpoints::openai_v1_base_url(),
    }
}

/// Recognize a complete gateway URL, rather than a substring that could also
/// occur in a user host, query, or credentials. The local dev gateway is the
/// explicitly supported OFOX_USE_LOCAL exception in ofox_apex.rs.
fn managed_url(current: &str, next: &str) -> bool {
    let (Ok(current), Ok(next)) = (url::Url::parse(current), url::Url::parse(next)) else {
        return false;
    };
    let known_host = current.scheme() == "https"
        && current.port().is_none()
        && current
            .host_str()
            .and_then(|host| host.strip_prefix("api."))
            .is_some_and(crate::ofox_apex::is_known_apex);
    let local_gateway = current.scheme() == "http"
        && current.host_str() == Some("localhost")
        && current.port() == Some(8088);
    (known_host || local_gateway)
        && current.username().is_empty()
        && current.password().is_none()
        && current.query().is_none()
        && current.fragment().is_none()
        && current.path().trim_end_matches('/') == next.path().trim_end_matches('/')
}

fn changed_url(current: &Value, next: &str) -> Result<bool, String> {
    let current = current.as_str().ok_or_else(|| CONFLICT.to_string())?;
    if !managed_url(current, next) {
        return Err(CONFLICT.into());
    }
    Ok(current != next)
}

fn patch_json(value: &mut Value, path: JsonPath, next: &str) -> Result<bool, String> {
    let mut node = &*value;
    for key in path {
        if !node.is_object() {
            return Err(CONFLICT.into());
        }
        let Some(child) = node.get(*key) else {
            return Ok(false);
        };
        node = child;
    }
    if !changed_url(node, next)? {
        return Ok(false);
    }
    json_file::set(value, path, next.into());
    Ok(true)
}

fn patch_codex(source: &str, next: &str) -> Result<Option<String>, String> {
    let mut doc = source
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| CONFLICT.to_string())?;
    let Some(providers) = doc.get_mut("model_providers") else {
        return Ok(None);
    };
    let providers = providers
        .as_table_like_mut()
        .ok_or_else(|| CONFLICT.to_string())?;
    let Some(provider) = providers.get_mut("ofox") else {
        return Ok(None);
    };
    let provider = provider
        .as_table_like_mut()
        .ok_or_else(|| CONFLICT.to_string())?;
    let Some(current) = provider.get("base_url") else {
        return Ok(None);
    };
    if !changed_url(
        &Value::String(current.as_str().ok_or_else(|| CONFLICT.to_string())?.into()),
        next,
    )? {
        return Ok(None);
    }
    let decor = current
        .as_value()
        .ok_or_else(|| CONFLICT.to_string())?
        .decor()
        .clone();
    let mut updated = toml_edit::Value::from(next);
    *updated.decor_mut() = decor;
    provider.insert("base_url", toml_edit::Item::Value(updated));
    Ok(Some(doc.to_string()))
}

fn template_path(tool: Tool) -> JsonPath {
    match tool {
        Tool::Claude => &["env", "ANTHROPIC_BASE_URL"],
        Tool::Gemini => &["env", "GOOGLE_GEMINI_BASE_URL"],
        Tool::OpenCode => &["options", "baseURL"],
        Tool::OpenClaw => &["baseUrl"],
        Tool::Hermes => &["base_url"],
        Tool::Codex => unreachable!("Codex uses TOML"),
    }
}

fn patch_template(tool: Tool, previous: &Value, next: &str) -> Result<Option<Value>, String> {
    let mut updated = previous.clone();
    if tool == Tool::Codex {
        let config = previous
            .get("config")
            .and_then(Value::as_str)
            .ok_or_else(|| CONFLICT.to_string())?;
        let Some(config) = patch_codex(config, next)? else {
            return Ok(None);
        };
        updated["config"] = config.into();
    } else if !patch_json(&mut updated, template_path(tool), next)? {
        return Ok(None);
    }
    Ok(Some(updated))
}

fn patch_live(tool: Tool, source: &str, next: &str) -> Result<Option<String>, String> {
    match tool {
        Tool::Codex => patch_codex(source, next),
        Tool::Gemini => {
            let Some(current) = env_file::get_value(source, "GOOGLE_GEMINI_BASE_URL") else {
                return Ok(None);
            };
            if !changed_url(&Value::String(current), next)? {
                return Ok(None);
            }
            Ok(Some(env_file::set_value(
                source,
                "GOOGLE_GEMINI_BASE_URL",
                Some(next),
            )))
        }
        Tool::Hermes => {
            let mut config = crate::hermes_config::parse_config_text(source)
                .map_err(|_| CONFLICT.to_string())?;
            let Some(providers) = config.get_mut("custom_providers") else {
                return Ok(None);
            };
            let providers = providers
                .as_sequence_mut()
                .ok_or_else(|| CONFLICT.to_string())?;
            let indexes: Vec<_> = providers
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    entry.get("name").and_then(serde_yaml::Value::as_str) == Some("ofox-hermes")
                })
                .map(|(index, _)| index)
                .collect();
            let index = match indexes.as_slice() {
                [] => return Ok(None),
                [index] => *index,
                _ => return Err(CONFLICT.into()),
            };
            let Some(current) = providers[index].get("base_url") else {
                return Ok(None);
            };
            let current = current.as_str().ok_or_else(|| CONFLICT.to_string())?;
            if !changed_url(&Value::String(current.into()), next)? {
                return Ok(None);
            }
            providers[index]["base_url"] = serde_yaml::Value::String(next.into());
            crate::hermes_config::render_section(
                source,
                "custom_providers",
                config.get("custom_providers"),
            )
            .map(Some)
            .map_err(|_| CONFLICT.to_string())
        }
        _ => {
            let before = json_file::parse_object(Some(source), tool.label())
                .map_err(|_| CONFLICT.to_string())?;
            let mut after = before.clone();
            let path: JsonPath = match tool {
                Tool::Claude => &["env", "ANTHROPIC_BASE_URL"],
                Tool::OpenCode => &["provider", "ofox-opencode", "options", "baseURL"],
                Tool::OpenClaw => &["models", "providers", "ofox-openclaw", "baseUrl"],
                _ => unreachable!(),
            };
            if !patch_json(&mut after, path, next)? {
                return Ok(None);
            }
            let style = if tool == Tool::OpenClaw {
                KeyStyle::Json5
            } else {
                KeyStyle::Json
            };
            json_file::render_changed(source, style, &before, &after)
                .map(Some)
                .map_err(|_| CONFLICT.to_string())
        }
    }
}

fn reconcile_tool(db: &Database, tool: Tool) -> Result<bool, String> {
    let _file_locks = file_locks(tool);
    let Some(provider) = db
        .get_provider_by_id(tool.provider_id(), tool.app().as_str())
        .map_err(|_| CONFLICT.to_string())?
    else {
        return Ok(false);
    };
    let record = load_record(db, tool).map_err(|_| CONFLICT.to_string())?;
    let tracked = record.is_some() || super::current_binding_is_ofox(db, tool)?;
    let next = next_url(tool);
    let updated = patch_template(tool, &provider.settings_config, &next)?;
    // Only the first managed file holds the endpoint; Gemini settings.json has
    // login mode, which region changes must neither restore nor modify.
    let path = tool.files()[0].current_path();
    let live = if tracked {
        read_text(&path)
            .map_err(|_| CONFLICT.to_string())?
            .map(|source| patch_live(tool, &source, &next))
            .transpose()?
            .flatten()
    } else {
        None
    };
    if updated.is_none() && live.is_none() {
        return Ok(false);
    }
    let mut txn = FileTxn::new();
    if let Some(text) = live.as_deref() {
        if txn.write(&path, text.as_bytes()).is_err() {
            return Err(rollback_with(txn, CONFLICT.into()));
        }
    }
    if let Some(updated) = updated.as_ref() {
        if db
            .update_provider_settings_config(tool.app().as_str(), tool.provider_id(), updated)
            .is_err()
        {
            return Err(rollback_with(txn, CONFLICT.into()));
        }
    }
    Ok(true)
}

/// Attempt every tool even if one has a user conflict. Errors contain tool
/// names and a fixed diagnostic only, never configuration values or keys.
pub(crate) async fn reconcile_managed_endpoints(db: &Database) -> Result<bool, String> {
    let _guard = BIND_LOCK.lock().await;
    let mut changed = false;
    let mut conflicts = Vec::new();
    for tool in [
        Tool::Codex,
        Tool::Claude,
        Tool::Gemini,
        Tool::OpenCode,
        Tool::OpenClaw,
        Tool::Hermes,
    ] {
        match reconcile_tool(db, tool) {
            Ok(updated) => changed |= updated,
            Err(_) => conflicts.push(tool.label()),
        }
    }
    if conflicts.is_empty() {
        Ok(changed)
    } else {
        Err(format!("{}：{CONFLICT}", conflicts.join("、")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn endpoint_ownership_checks_the_complete_url() {
        let next = "https://api.ofox.io/v1";
        for owned in [
            "https://api.ofox.ai/v1",
            "https://api.ofox.io/v1/",
            "http://localhost:8088/v1",
        ] {
            assert!(managed_url(owned, next), "{owned}");
        }
        for unrelated in [
            "https://api.ofox.ai.evil/v1",
            "https://evil.example/v1?next=https://api.ofox.ai/v1",
            "https://api.ofox.ai@evil.example/v1",
            "https://user:secret@api.ofox.ai/v1",
            "https://api.ofox.ai/v1?key=secret",
            "https://api.ofox.ai/v1#fragment",
            "https://api.ofox.ai:8443/v1",
            "https://api.ofox.ai/anthropic",
            "http://api.ofox.ai/v1",
            "http://localhost:11434/v1",
        ] {
            assert!(!managed_url(unrelated, next), "{unrelated}");
        }
    }

    #[test]
    fn missing_endpoint_fields_are_not_created_and_toml_comments_stay() {
        let mut missing = json!({"provider":{"ofox-opencode":{"options":{"apiKey":"saved"}}}});
        let original = missing.clone();
        assert!(!patch_json(
            &mut missing,
            &["provider", "ofox-opencode", "options", "baseURL"],
            "https://api.ofox.io/v1"
        )
        .unwrap());
        assert_eq!(missing, original);
        let source = "model = \"saved\"\n\n[model_providers.ofox]\nbase_url = \"https://api.ofox.ai/v1\" # custom note\nexperimental_bearer_token = \"saved-key\"\n";
        let updated = patch_codex(source, "https://api.ofox.io/v1")
            .unwrap()
            .unwrap();
        assert_eq!(
            updated,
            source.replace("https://api.ofox.ai/v1", "https://api.ofox.io/v1")
        );
    }
}
