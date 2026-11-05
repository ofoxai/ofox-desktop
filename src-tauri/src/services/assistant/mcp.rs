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
    let limits = (!servers.is_empty()).then(|| super::capabilities::local_config().limits);
    Ok(
        json!({"servers":servers,"snapshot":hash,"npxPath":npx,"uvxPath":uvx,"uvAvailable":uvx.is_some(),"searchPath":search,"limits":limits}),
    )
}

pub fn secrets(snapshot: &Value) -> Vec<String> {
    snapshot["servers"]
        .as_object()
        .into_iter()
        .flat_map(|v| v.values())
        .flat_map(|s| [&s["env"], &s["headers"]])
        .filter_map(Value::as_object)
        .flat_map(|v| v.values())
        .filter_map(Value::as_str)
        .filter(|s| !s.is_empty())
        .flat_map(|value| {
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

#[cfg(test)]
mod tests {
    use super::*;
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
