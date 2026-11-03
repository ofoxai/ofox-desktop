use super::{files, scope::Scope};
use crate::{app_config::BindableTool, commands::tool_update};
use serde_json::{json, Map, Value};
use std::path::Path;

pub fn folder_metadata(scope: &Scope, folder: &Path, outside: bool) -> Value {
    let mut metadata = Map::new();
    for name in [
        ".nvmrc",
        ".node-version",
        "mise.toml",
        ".tool-versions",
        "package.json",
    ] {
        let path = folder.join(name);
        if !path.exists() {
            continue;
        }
        let value = match files::read(scope, &path, outside) {
            Ok(text) if name == "package.json" => match serde_json::from_str::<Value>(&text) {
                Ok(value) => {
                    json!({"engines":value.get("engines"),"volta":value.get("volta"),"packageManager":value.get("packageManager")})
                }
                Err(_) => json!({"error":"assistant_file_unreadable"}),
            },
            Ok(text) => json!(text),
            Err(code) => json!({"error":code}),
        };
        metadata.insert(name.into(), value);
    }
    Value::Object(metadata)
}

/// macOS asks before an app's child process may open Documents, Desktop,
/// Downloads or iCloud Drive; until the user answers, a probe there hangs.
fn permission_hint(folder: &Path, home: &Path, error: &str) -> Option<&'static str> {
    let protected = [
        "Documents",
        "Desktop",
        "Downloads",
        "Library/Mobile Documents",
    ]
    .iter()
    .any(|dir| folder.starts_with(home.join(dir)));
    (cfg!(target_os = "macos") && protected && error.contains("timed out")).then_some(
        "macOS may be asking whether Ofox Desktop can access this folder. Ask the user to click Allow in the system dialog, then check again; this is not a fault of the tool.",
    )
}

pub async fn probe(tool: BindableTool, folder: &Path, home: &Path) -> Value {
    let mut result = probe_once(tool, folder, home).await;
    if let Some(hint) = result["error"]
        .as_str()
        .and_then(|error| permission_hint(folder, home, error))
    {
        result["hint"] = json!(hint);
    }
    result
}

async fn probe_once(tool: BindableTool, folder: &Path, home: &Path) -> Value {
    let name = tool.as_str();
    match tool_update::probe_executable_in(name, folder).await {
        Ok(install) => {
            let node_manager =
                crate::commands::node_requirement::node_manager(&install.path, &install.real);
            #[cfg(unix)]
            let launcher = {
                let path = crate::commands::node_launcher::launcher_dir(home).join(name);
                let target = crate::commands::node_launcher::launcher_target(&path, home);
                json!({"path":path,"exists":path.exists(),"managed":target.is_some(),"stale":target.as_ref().is_some_and(|t|!t.exists())})
            };
            #[cfg(not(unix))]
            let launcher = {
                let _ = home;
                json!({"supported":false})
            };
            json!({"tool":name,"folder":folder,"installed":true,"path":install.path,"realPath":install.real,"version":install.version,"verified":install.error.is_none()&&!install.version.is_empty(),"error":install.error.map(crate::redact::scrub_error),"nodeManager":node_manager,"launcher":launcher})
        }
        Err(tool_update::ProbeError::NotFound) => {
            json!({"tool":name,"folder":folder,"installed":false,"verified":false})
        }
        Err(tool_update::ProbeError::Failed(error)) => {
            json!({"tool":name,"folder":folder,"installed":null,"verified":false,"error":crate::redact::scrub(&error)})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn a_probe_timeout_in_a_protected_folder_points_at_the_macos_permission_prompt() {
        let home = Path::new("/Users/me");
        let timeout = "Active executable failed its version check: Version probe timed out";
        assert!(permission_hint(&home.join("Documents/奖学金"), home, timeout).is_some());
        assert!(permission_hint(&home.join("Library/Mobile Documents/x"), home, timeout).is_some());
        assert!(permission_hint(&home.join("code/app"), home, timeout).is_none());
        assert!(permission_hint(&home.join("Desktop"), home, "not found").is_none());
    }
    #[test]
    fn metadata_limits_package_fields_and_scrubs_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let scope = Scope::new(dir.path().into(), vec![folder.clone()], vec![]).unwrap();
        std::fs::write(folder.join(".nvmrc"), "18.20.0").unwrap();
        std::fs::write(folder.join("package.json"),r#"{"engines":{"node":">=22"},"volta":{"node":"22.16.0"},"scripts":{"evil":"never execute"}}"#).unwrap();
        let result = folder_metadata(&scope, &folder, false);
        assert_eq!(result[".nvmrc"], "18.20.0");
        assert_eq!(result["package.json"]["volta"]["node"], "22.16.0");
        assert!(result["package.json"].get("scripts").is_none());
        assert_eq!(
            folder_metadata(
                &Scope::new(dir.path().into(), vec![], vec![]).unwrap(),
                &folder,
                false
            )[".nvmrc"]["error"],
            "assistant_path_denied"
        );
    }
}
