use std::fs;
use std::path::PathBuf;

use crate::error::AppError;

const CLAUDE_CONFIG_FILE: &str = "config.json";

/// 设置里的覆盖目录 → `CLAUDE_CONFIG_DIR` → `~/.claude`（见 `agent_env`）。
fn claude_dir() -> Result<PathBuf, AppError> {
    Ok(crate::config::get_claude_config_dir())
}

pub fn claude_config_path() -> Result<PathBuf, AppError> {
    Ok(claude_dir()?.join(CLAUDE_CONFIG_FILE))
}

pub fn ensure_claude_dir_exists() -> Result<PathBuf, AppError> {
    let dir = claude_dir()?;
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| AppError::io(&dir, e))?;
    }
    Ok(dir)
}

pub fn read_claude_config() -> Result<Option<String>, AppError> {
    let path = claude_config_path()?;
    if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;
        Ok(Some(content))
    } else {
        Ok(None)
    }
}

fn is_managed_config(content: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(value) => value
            .get("primaryApiKey")
            .and_then(|v| v.as_str())
            .map(|val| val == "any")
            .unwrap_or(false),
        Err(_) => false,
    }
}

pub fn write_claude_config() -> Result<bool, AppError> {
    // 增量写入：仅设置 primaryApiKey = "any"，保留其它字段
    let path = claude_config_path()?;
    ensure_claude_dir_exists()?;

    // 尝试读取并解析为对象
    let mut obj = match read_claude_config()? {
        Some(existing) => match serde_json::from_str::<serde_json::Value>(&existing) {
            Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
            _ => serde_json::json!({}),
        },
        None => serde_json::json!({}),
    };

    let mut changed = false;
    if let Some(map) = obj.as_object_mut() {
        let cur = map
            .get("primaryApiKey")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if cur != "any" {
            map.insert(
                "primaryApiKey".to_string(),
                serde_json::Value::String("any".to_string()),
            );
            changed = true;
        }
    }

    if changed || !path.exists() {
        let serialized = serde_json::to_string_pretty(&obj)
            .map_err(|e| AppError::JsonSerialize { source: e })?;
        crate::config::atomic_write(&path, format!("{serialized}\n").as_bytes())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn clear_claude_config() -> Result<bool, AppError> {
    let path = claude_config_path()?;
    if !path.exists() {
        return Ok(false);
    }

    let content = match read_claude_config()? {
        Some(content) => content,
        None => return Ok(false),
    };

    let mut value = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };

    let obj = match value.as_object_mut() {
        Some(obj) => obj,
        None => return Ok(false),
    };

    if obj.remove("primaryApiKey").is_none() {
        return Ok(false);
    }

    let serialized =
        serde_json::to_string_pretty(&value).map_err(|e| AppError::JsonSerialize { source: e })?;
    crate::config::atomic_write(&path, format!("{serialized}\n").as_bytes())?;
    Ok(true)
}

pub fn claude_config_status() -> Result<(bool, PathBuf), AppError> {
    let path = claude_config_path()?;
    Ok((path.exists(), path))
}

pub fn is_claude_config_applied() -> Result<bool, AppError> {
    match read_claude_config()? {
        Some(content) => Ok(is_managed_config(&content)),
        None => Ok(false),
    }
}

#[cfg(test)]
mod dir_tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn the_config_is_in_the_default_directory_without_claude_config_dir() {
        let sandbox = crate::test_sandbox::sandbox();

        assert_eq!(
            claude_config_path().unwrap(),
            sandbox.home().join(".claude").join("config.json")
        );
    }

    #[test]
    #[serial]
    fn claude_config_dir_moves_the_config_and_nothing_is_written_to_the_default() {
        let sandbox = crate::test_sandbox::sandbox();
        let moved = sandbox.home().join("data").join("claude-state");
        std::env::set_var("CLAUDE_CONFIG_DIR", &moved);

        assert_eq!(claude_config_path().unwrap(), moved.join("config.json"));
        assert!(write_claude_config().unwrap());

        assert!(is_claude_config_applied().unwrap());
        assert!(moved.join("config.json").is_file());
        assert!(!sandbox.home().join(".claude").exists());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn writing_the_claude_config_goes_through_a_symlink_and_leaves_no_temp_file() {
        let sandbox = crate::test_sandbox::sandbox();
        let dotfiles = sandbox.home().join("dotfiles/claude-config.json");
        fs::create_dir_all(dotfiles.parent().unwrap()).unwrap();
        fs::write(&dotfiles, "{\"theme\":\"dark\"}").unwrap();
        let path = claude_config_path().unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&dotfiles, &path).unwrap();

        assert!(write_claude_config().unwrap());

        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&dotfiles).unwrap()).unwrap();
        assert_eq!(written["primaryApiKey"], "any");
        assert_eq!(written["theme"], "dark");
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "{leftovers:?}");

        assert!(clear_claude_config().unwrap());
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!fs::read_to_string(&dotfiles)
            .unwrap()
            .contains("primaryApiKey"));
    }

    #[test]
    #[serial]
    fn a_claude_config_created_by_ofox_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let _sandbox = crate::test_sandbox::sandbox();

        assert!(write_claude_config().unwrap());

        let mode = fs::metadata(claude_config_path().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
