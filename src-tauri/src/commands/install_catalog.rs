use super::tool_update::{Installation, ProbeError};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryEntry {
    pub tool_id: String,
    pub status: String,
    pub version: Option<String>,
    pub path: Option<String>,
    pub error: Option<String>,
}

fn catalog_binary(tool: &str) -> Option<&'static str> {
    match tool {
        "copilot" => Some("copilot"),
        "cline" => Some("cline"),
        "crush" => Some("crush"),
        "omp" => Some("omp"),
        "codebuddy" => Some("codebuddy"),
        "aider" => Some("aider"),
        "goose-cli" => Some("goose"),
        _ => None,
    }
}

fn inventory_from_probe(tool: &str, result: Result<Installation, ProbeError>) -> InventoryEntry {
    let mut entry = InventoryEntry {
        tool_id: tool.into(),
        status: "unknown".into(),
        version: None,
        path: None,
        error: None,
    };
    match result {
        Ok(result) => {
            entry.status = "installed".into();
            entry.path = Some(result.path.to_string_lossy().into_owned());
            entry.version = (!result.version.is_empty()).then_some(result.version);
            entry.error = result.error;
        }
        Err(ProbeError::NotFound) => entry.status = "notInstalled".into(),
        Err(ProbeError::Failed(error)) => entry.error = Some(error),
    }
    entry
}

/// Read-only installation inventory; never requests or reconciles account keys.
#[tauri::command]
pub async fn get_install_catalog_inventory() -> Result<Vec<InventoryEntry>, String> {
    let mut rows = super::misc::get_tool_versions(None, None, Some(false))
        .await?
        .into_iter()
        .map(super::misc::ToolVersion::into_catalog_inventory)
        .collect::<Vec<_>>();
    let futures = [
        "copilot",
        "cline",
        "crush",
        "omp",
        "codebuddy",
        "deveco-code",
        "deveco-cli",
        "aider",
        "goose-cli",
    ]
    .into_iter()
    .map(|tool| async move {
        match catalog_binary(tool) {
            Some(binary) => {
                inventory_from_probe(tool, super::tool_update::probe_executable(binary).await)
            }
            None => InventoryEntry {
                tool_id: tool.into(),
                status: "unknown".into(),
                version: None,
                path: None,
                error: None,
            },
        }
    });
    rows.extend(futures::future::join_all(futures).await);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tool_update::{Installation, ProbeError};
    use std::path::PathBuf;

    #[test]
    fn failed_probe_is_not_an_uninstall() {
        let result =
            inventory_from_probe("crush", Err(ProbeError::Failed("fixture timeout".into())));
        assert_eq!(result.status, "unknown");
        assert_eq!(result.error.as_deref(), Some("fixture timeout"));
        assert_eq!(
            inventory_from_probe("crush", Err(ProbeError::NotFound)).status,
            "notInstalled"
        );
    }

    #[test]
    fn executable_evidence_survives_a_broken_version_check() {
        let result = inventory_from_probe(
            "crush",
            Ok(Installation {
                path: PathBuf::from("fixture/crush"),
                real: PathBuf::from("fixture/crush"),
                version: String::new(),
                error: Some("fixture version failed".into()),
                search_path: String::new(),
            }),
        );
        assert_eq!(result.status, "installed");
        assert_eq!(result.path.as_deref(), Some("fixture/crush"));
        assert!(result.version.is_none());
        assert!(result.error.is_some());
    }

    #[test]
    fn product_identity_is_not_a_guessed_binary_or_bind_target() {
        assert_eq!(catalog_binary("omp"), Some("omp"));
        assert_eq!(catalog_binary("goose-cli"), Some("goose"));
        assert_eq!(catalog_binary("deveco-code"), None);
        assert_eq!(catalog_binary("deveco-cli"), None);
        assert_eq!(catalog_binary("crush;echo injected"), None);
    }
}
