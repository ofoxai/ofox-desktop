//! Captured per-run MCP configuration. No Pi global config or OAuth credentials.
use super::host::{blocking, AssistantHost, RunControl};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

fn expand_environment(
    raw: &str,
    resolve: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut rest = raw;
    let mut expanded = String::new();
    while let Some(start) = rest.find("${") {
        expanded.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find('}').ok_or("assistant_mcp_environment_missing")?;
        expanded.push_str(&resolve(&after[..end]).ok_or("assistant_mcp_environment_missing")?);
        if expanded.len() > 65536 {
            return Err("assistant_mcp_config_unavailable".into());
        }
        rest = &after[end + 1..];
    }
    expanded.push_str(rest);
    if expanded.len() > 65536 || expanded.contains('\0') {
        return Err("assistant_mcp_config_unavailable".into());
    }
    Ok(expanded)
}

impl AssistantHost {
    pub(super) async fn save_mcp_step(
        &self,
        run: &RunControl,
        data: &Value,
        status: &str,
    ) -> Result<(), String> {
        let unavailable = status == "unavailable";
        let server = if unavailable {
            data["name"].as_str()
        } else {
            data["mcp"]["server"].as_str()
        };
        if server.is_some_and(|s| run.mcp["servers"].get(s).is_none()) {
            return Err("assistant_mcp_unknown_server".into());
        }
        let tool = if unavailable {
            ""
        } else {
            data["mcp"]["tool"]
                .as_str()
                .filter(|s| s.len() <= 128)
                .ok_or("assistant_runtime_protocol")?
        };
        let id = if unavailable {
            format!("unavailable:{}", server.unwrap_or("mcp"))
        } else {
            data["toolCallId"]
                .as_str()
                .filter(|s| s.len() <= 200)
                .ok_or("assistant_runtime_protocol")?
                .to_owned()
        };
        let icon = server
            .and_then(|s| crate::services::library::display::mcp_display(&run.mcp["servers"][s]).1);
        let mut step = json!({"id":id,"server":server,"tool":tool,"status":status,"recipeId":icon});
        if unavailable {
            step["reason"] = json!(match data["reason"].as_str() {
                Some("needs_node") => "needs_node",
                Some("needs_uv") => "needs_uv",
                _ => "unavailable",
            });
        }
        let store = self.store(&run.identity.owner)?;
        let sid = run.session_id.clone();
        let mid = run.reply_id();
        let target = mid.clone();
        let copy = step.clone();
        blocking(move || {
            store.mutate(&sid, |record| {
                let message = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == target && m.role == "assistant")
                    .ok_or("assistant_state_unavailable")?;
                if let Some(old) = message.mcp_steps.iter_mut().find(|s| s["id"] == copy["id"]) {
                    *old = copy;
                } else if message.mcp_steps.len() < 100 {
                    message.mcp_steps.push(copy);
                }
                Ok(())
            })
        })
        .await?;
        self.emit(run, "mcp_step", json!({"messageId":mid,"step":step}));
        Ok(())
    }
}

pub fn snapshot(root: &Path) -> Result<Value, String> {
    let root = crate::services::library::coordinator::physical(root)
        .map_err(|_| "assistant_mcp_config_unavailable")?;
    let path = root.join("assistant/mcp.json");
    let bytes = crate::services::library::assistant_target::state::read_regular(&path, 1024 * 1024)
        .map_err(|_| "assistant_mcp_config_unavailable")?;
    let value = match bytes {
        Some(bytes) => crate::services::library::mcp_codec::parse_native(
            std::str::from_utf8(&bytes).map_err(|_| "assistant_mcp_config_unavailable")?,
            crate::services::library::assistant_target::ID,
        )
        .map_err(|_| "assistant_mcp_config_unavailable")?,
        None => json!({}),
    };
    let mut servers = serde_json::Map::new();
    'servers: for (name, spec) in value["mcpServers"].as_object().into_iter().flatten() {
        if name.is_empty() || name.len() > 128 {
            return Err("assistant_mcp_config_unavailable".into());
        }
        if spec["enabled"] == false {
            continue;
        }
        crate::services::library::mcp_targets::validate_assistant_server(spec)
            .map_err(|e| e.to_string())?;
        if servers
            .keys()
            .any(|old: &String| old.replace('-', "_") == name.replace('-', "_"))
        {
            return Err("assistant_mcp_config_unavailable".into());
        }
        let mut config = crate::services::library::mcp_codec::native_server(
            crate::services::library::assistant_target::ID,
            name,
            spec,
        )
        .map_err(|_| "assistant_mcp_config_unavailable")?;
        for field in ["env", "headers"] {
            if let Some(values) = config[field].as_object_mut() {
                for value in values.values_mut() {
                    if let Some(raw) = value.as_str() {
                        match expand_environment(raw, |key| std::env::var(key).ok()) {
                            Ok(expanded) => *value = json!(expanded),
                            Err(_) => {
                                // Keep this selected service's identity in the captured
                                // snapshot, but never send unresolved credentials to Pi.
                                let identity = format!(
                                    "{:x}",
                                    Sha256::digest(
                                        serde_json::to_vec(spec)
                                            .map_err(|_| "assistant_mcp_config_unavailable")?
                                    )
                                );
                                servers.insert(
                                    name.clone(),
                                    json!({"unavailable":true,"identity":identity}),
                                );
                                continue 'servers;
                            }
                        }
                    }
                }
            }
        }
        servers.insert(name.clone(), config);
    }
    let hash = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&servers).map_err(|_| "assistant_mcp_config_unavailable")?
        )
    );
    let (npx, search) = crate::commands::resolve_cli_path("npx");
    let (uvx, _) = crate::commands::resolve_cli_path("uvx");
    Ok(
        json!({"servers":servers,"snapshot":hash,"npxPath":npx,"uvxPath":uvx,"uvAvailable":uvx.is_some(),"searchPath":search}),
    )
}

pub fn secrets(snapshot: &Value) -> Vec<String> {
    snapshot["servers"]
        .as_object()
        .into_iter()
        .flat_map(|v| v.values())
        .flat_map(|s| [&s["env"], &s["headers"]])
        .filter_map(Value::as_object)
        .flat_map(|v| v.iter())
        .filter_map(|(name, value)| Some((name, value.as_str()?)))
        .filter(|(name, value)| credential_value(name, value))
        .flat_map(|(_, value)| {
            let mut values = vec![value.to_owned()];
            if let Some((scheme, credential)) = value.split_once(' ') {
                if ["bearer", "basic", "token"].contains(&scheme.to_ascii_lowercase().as_str())
                    && !credential.trim_start().is_empty()
                {
                    values.push(credential.trim_start().to_owned());
                }
            }
            values
        })
        .collect()
}

/// Only a value named like a credential, or shaped like a generated token, is
/// private. Plain settings (DEBUG=1, LOG_LEVEL=info) would otherwise rewrite
/// every matching word in replies and history and collide with request ids.
/// Mirrors `credentialValue` in assistant-runtime/src/mcp-security.mjs.
fn credential_value(name: &str, value: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let named = [
        "key",
        "token",
        "secret",
        "passw",
        "credential",
        "cookie",
        "auth",
        "signature",
        "private",
    ]
    .iter()
    .any(|stem| name.contains(stem))
        || name
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| matches!(word, "pin" | "pwd" | "pass" | "pat"));
    // Flags and small counts (AUTH_ENABLED=true, TOKEN_TTL=60) are settings.
    let setting = matches!(
        value.to_ascii_lowercase().as_str(),
        "" | "true" | "false" | "null" | "yes" | "no" | "on" | "off"
    ) || (value.len() < 4 && value.bytes().all(|b| b.is_ascii_digit()));
    (named && !setting) || token_shaped(value)
}

fn token_shaped(value: &str) -> bool {
    value.chars().count() >= 20
        && !value.contains(|c: char| c.is_whitespace() || c == '/' || c == '\\')
        && value.chars().any(|c| c.is_ascii_alphabetic())
        && value.chars().any(|c| c.is_ascii_digit())
}

// Only credential-bearing data is rewritten. Routing IDs, permission/status
// booleans and usage accounting retain their validated engine-neutral types.
fn scrub_payload(value: &Value, pattern: &regex::Regex) -> Result<Value, String> {
    let text = |text: &str| pattern.replace_all(text, "***").into_owned();
    Ok(match value {
        Value::String(value) => json!(text(value)),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| scrub_payload(value, pattern))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(values) => {
            let mut result = serde_json::Map::new();
            for (key, value) in values {
                if result
                    .insert(text(key), scrub_payload(value, pattern)?)
                    .is_some()
                {
                    return Err("assistant_mcp_data_unavailable".into());
                }
            }
            Value::Object(result)
        }
        scalar if pattern.is_match(&scalar.to_string()) => json!("***"),
        scalar => scalar.clone(),
    })
}
fn payload_pattern(snapshot: &Value) -> Result<Option<regex::Regex>, String> {
    let mut values = secrets(snapshot);
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    if values.is_empty() {
        return Ok(None);
    }
    regex::Regex::new(
        &values
            .iter()
            .map(|value| regex::escape(value))
            .collect::<Vec<_>>()
            .join("|"),
    )
    .map(Some)
    .map_err(|_| "assistant_mcp_data_unavailable".into())
}
pub(super) fn arguments_preview(args: &Value, snapshot: &Value) -> Result<(String, bool), String> {
    let safe = match payload_pattern(snapshot)? {
        Some(pattern) => scrub_payload(args, &pattern)?,
        None => args.clone(),
    };
    let text = serde_json::to_string_pretty(&safe).map_err(|_| "assistant_mcp_data_unavailable")?;
    let truncated = text.chars().count() > 8000;
    Ok((text.chars().take(8000).collect(), truncated))
}

pub(super) fn scrub_event_data(data: &mut Value, snapshot: &Value) -> Result<(), String> {
    let Some(pattern) = payload_pattern(snapshot)? else {
        return Ok(());
    };
    for field in ["args", "details", "delta"] {
        if let Some(value) = data.get_mut(field) {
            *value = scrub_payload(value, &pattern)?;
        }
    }
    for field in ["message", "errorMessage"] {
        if let Some(value) = data.get_mut(field).filter(|value| value.is_string()) {
            *value = scrub_payload(value, &pattern)?;
        }
    }
    Ok(())
}
pub(super) fn scrub_native_data(entries: &mut Value, snapshot: &Value) -> Result<(), String> {
    let Some(pattern) = payload_pattern(snapshot)? else {
        return Ok(());
    };
    for entry in entries.as_array_mut().into_iter().flatten() {
        if let Some(value) = entry.get_mut("summary").filter(|value| value.is_string()) {
            *value = scrub_payload(value, &pattern)?;
        }
        let Some(message) = entry.get_mut("message") else {
            continue;
        };
        if let Some(value) = message.get_mut("details") {
            *value = scrub_payload(value, &pattern)?;
        }
        if let Some(content) = message.get_mut("content") {
            if content.is_string() {
                *content = scrub_payload(content, &pattern)?;
            } else {
                for block in content.as_array_mut().into_iter().flatten() {
                    for field in ["text", "thinking", "arguments"] {
                        if let Some(value) = block.get_mut(field) {
                            *value = scrub_payload(value, &pattern)?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_mcp_preview_scrubs_before_scalar_truncation_and_fails_on_key_collision() {
        let snapshot = json!({"servers":{"fixture":{"env":{"SHORT_SECRET":"xy7","PIN":"418731","BOOL":"false","EMPTY":"null"},"headers":{"Authorization":"Bearer credential"}}}});
        let args = json!({"target":"draft","body":"Readable body","nested":{"text":"xy7 credential","pin":418731,"flag":false,"empty":null,"418731":"Readable key"}});
        let original = args.clone();
        let (preview, truncated) = arguments_preview(&args, &snapshot).unwrap();
        assert!(!truncated);
        assert_eq!(
            args, original,
            "preview must never mutate permission/execution args"
        );
        for value in ["xy7", "418731", "credential"] {
            assert!(!preview.contains(value));
        }
        // BOOL/EMPTY are plain settings: the preview keeps the real values.
        assert!(preview.contains("\"flag\": false") && preview.contains("\"empty\": null"));
        assert!(preview.contains("Readable body") && preview.contains("Readable key"));
        assert_eq!(
            arguments_preview(&json!({}), &snapshot).unwrap(),
            ("{}".into(), false)
        );
        let full = json!({"body":"😀".repeat(9000)});
        let (preview, truncated) = arguments_preview(&full, &snapshot).unwrap();
        assert!(truncated);
        assert_eq!(preview.chars().count(), 8000);
        assert_eq!(
            arguments_preview(&json!({"418731":1,"***":2}), &snapshot).unwrap_err(),
            "assistant_mcp_data_unavailable"
        );
    }

    #[test]
    fn only_credential_named_or_token_shaped_values_are_secrets() {
        let mut found = secrets(&json!({"servers":{"s":{
            "env":{"DEBUG":"1","LOG_LEVEL":"info","NODE_ENV":"production","AUTH_ENABLED":"true",
                   "BRAVE_API_KEY":"brv-live-123","PIN":"4187",
                   "UNNAMED":"ghp0123456789abcdefghijKLMN","DATA_DIR":"/Users/me/projects/data2026"},
            "headers":{"Content-Type":"application/json","Authorization":"Bearer tok-123456"}}}}));
        found.sort();
        let mut want = vec![
            "4187",
            "Bearer tok-123456",
            "brv-live-123",
            "ghp0123456789abcdefghijKLMN",
            "tok-123456",
        ];
        want.sort();
        assert_eq!(found, want);
    }
    #[test]
    fn assistant_mcp_startup_rejects_sign_in_corruption_and_namespace_collisions() {
        assert_eq!(
            expand_environment("Bearer ${MCP_TOKEN}", |key| (key == "MCP_TOKEN")
                .then(|| "fixture-token".into()))
            .unwrap(),
            "Bearer fixture-token"
        );
        assert!(expand_environment("Bearer ${MISSING}", |_| None).is_err());
        assert_eq!(
            secrets(
                &json!({"servers":{"remote":{"headers":{"Authorization":"Bearer fixture-token"}}}})
            ),
            vec!["Bearer fixture-token", "fixture-token"]
        );
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let config = root.join("assistant/mcp.json");
        assert!(snapshot(&root).unwrap()["servers"]
            .as_object()
            .unwrap()
            .is_empty());
        crate::config::create_private_dir_all(config.parent().unwrap()).unwrap();
        for host in crate::services::library::assistant_target::SIGN_IN_HOSTS {
            crate::config::atomic_write(&config,serde_json::to_string(&json!({"mcpServers":{"blocked":{"url":format!("https://{}.:443/mcp",host.to_uppercase())}}})).unwrap().as_bytes()).unwrap();
            assert!(snapshot(&root).is_err());
        }
        crate::config::atomic_write(&config, b"not JSON").unwrap();
        assert!(snapshot(&root).is_err());
        assert_eq!(std::fs::read(&config).unwrap(), b"not JSON");
        crate::config::atomic_write(
            &config,
            br#"{"mcpServers":{"a-b":{"command":"node"},"a_b":{"command":"node"}}}"#,
        )
        .unwrap();
        assert!(snapshot(&root).is_err());
        crate::config::atomic_write(&config,br#"{"mcpServers":{"public":{"url":"https://mcp.deepwiki.com/mcp"},"custom":{"url":"https://example.test/mcp"},"disabled":{"command":"npx","enabled":false}}}"#).unwrap();
        let captured = snapshot(&root).unwrap();
        assert_eq!(captured["servers"].as_object().unwrap().len(), 2);
        let missing = json!({"mcpServers":{"public":{"url":"https://mcp.deepwiki.com/mcp"},"missing":{"command":"node","env":{"TOKEN":"${OFOX_TASK7_MISSING_ENV_7A}"}}}});
        crate::config::atomic_write(&config, serde_json::to_vec(&missing).unwrap().as_slice())
            .unwrap();
        let unavailable = snapshot(&root).unwrap();
        assert_eq!(unavailable["servers"]["missing"]["unavailable"], true);
        assert!(unavailable["servers"]["public"]
            .get("unavailable")
            .is_none());
        let mut resolved = missing;
        resolved["mcpServers"]["missing"]["env"]["TOKEN"] = json!("fixture-token");
        crate::config::atomic_write(&config, serde_json::to_vec(&resolved).unwrap().as_slice())
            .unwrap();
        assert_ne!(
            snapshot(&root).unwrap()["snapshot"],
            unavailable["snapshot"],
            "a newly resolved credential invalidates old approval identity"
        );
        crate::config::atomic_write(&config, br#"{"mcpServers":{}}"#).unwrap();
        assert!(snapshot(&root).unwrap()["servers"]
            .as_object()
            .unwrap()
            .is_empty());
    }
}
