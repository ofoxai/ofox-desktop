use super::env_checker::EnvConflict;
#[cfg(any(test, target_os = "windows"))]
use super::env_checker::EnvEnvironment;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use winreg::enums::*;
#[cfg(target_os = "windows")]
use winreg::RegKey;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub backup_path: String,
    pub timestamp: String,
    pub conflicts: Vec<EnvConflict>,
}

/// Delete environment variables with automatic backup
pub fn delete_env_vars(conflicts: Vec<EnvConflict>) -> Result<BackupInfo, String> {
    // Step 1: Create backup
    let backup_info = create_backup(&conflicts)?;

    // Step 2: Delete variables
    for conflict in &conflicts {
        match delete_single_env(conflict) {
            Ok(_) => {}
            Err(e) => {
                // If deletion fails, we keep the backup but return error
                return Err(format!(
                    "删除环境变量失败: {}. 备份已保存到: {}",
                    e, backup_info.backup_path
                ));
            }
        }
    }

    Ok(backup_info)
}

/// Create backup file before deletion
fn create_backup(conflicts: &[EnvConflict]) -> Result<BackupInfo, String> {
    // Get backup directory
    let backup_dir = get_backup_dir()?;
    fs::create_dir_all(&backup_dir).map_err(|e| format!("创建备份目录失败: {e}"))?;

    // Generate backup file name with timestamp
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_file = backup_dir.join(format!("env-backup-{timestamp}.json"));

    // Create backup data
    let backup_info = BackupInfo {
        backup_path: backup_file.to_string_lossy().to_string(),
        timestamp: timestamp.clone(),
        conflicts: conflicts.to_vec(),
    };

    // Write backup file
    let json = serde_json::to_string_pretty(&backup_info)
        .map_err(|e| format!("序列化备份数据失败: {e}"))?;

    fs::write(&backup_file, json).map_err(|e| format!("写入备份文件失败: {e}"))?;

    Ok(backup_info)
}

/// Get backup directory path
fn get_backup_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    Ok(home.join(".ofox-desktop").join("backups"))
}

/// Delete a single environment variable
#[cfg(target_os = "windows")]
fn delete_single_env(conflict: &EnvConflict) -> Result<(), String> {
    if conflict.environment == EnvEnvironment::Wsl {
        return delete_wsl_file_conflict(conflict);
    }
    match conflict.source_type.as_str() {
        "system" => {
            if conflict.source_path.contains("HKEY_CURRENT_USER") {
                let hkcu = RegKey::predef(HKEY_CURRENT_USER)
                    .open_subkey_with_flags("Environment", KEY_ALL_ACCESS)
                    .map_err(|e| format!("打开注册表失败: {}", e))?;

                hkcu.delete_value(&conflict.var_name)
                    .map_err(|e| format!("删除注册表项失败: {}", e))?;
            } else if conflict.source_path.contains("HKEY_LOCAL_MACHINE") {
                let hklm = RegKey::predef(HKEY_LOCAL_MACHINE)
                    .open_subkey_with_flags(
                        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
                        KEY_ALL_ACCESS,
                    )
                    .map_err(|e| format!("打开系统注册表失败 (需要管理员权限): {}", e))?;

                hklm.delete_value(&conflict.var_name)
                    .map_err(|e| format!("删除系统注册表项失败: {}", e))?;
            }
            Ok(())
        }
        "file" => Err("原生 Windows 环境不支持文件类型的环境变量".to_string()),
        _ => Err(format!("未知的环境变量来源类型: {}", conflict.source_type)),
    }
}

#[cfg(not(target_os = "windows"))]
fn delete_single_env(conflict: &EnvConflict) -> Result<(), String> {
    match conflict.source_type.as_str() {
        "file" => delete_native_file_conflict(conflict),
        "system" => {
            // On Unix, we can't directly delete process environment variables
            Ok(())
        }
        _ => Err(format!("未知的环境变量来源类型: {}", conflict.source_type)),
    }
}

/// Restore environment variables from backup
pub fn restore_from_backup(backup_path: String) -> Result<(), String> {
    // Read backup file
    let content = fs::read_to_string(&backup_path).map_err(|e| format!("读取备份文件失败: {e}"))?;

    let backup_info: BackupInfo =
        serde_json::from_str(&content).map_err(|e| format!("解析备份文件失败: {e}"))?;

    // Restore each variable
    for conflict in &backup_info.conflicts {
        restore_single_env(conflict)?;
    }

    Ok(())
}

/// Restore a single environment variable
#[cfg(target_os = "windows")]
fn restore_single_env(conflict: &EnvConflict) -> Result<(), String> {
    if conflict.environment == EnvEnvironment::Wsl {
        return restore_wsl_file_conflict(conflict);
    }
    match conflict.source_type.as_str() {
        "system" => {
            if conflict.source_path.contains("HKEY_CURRENT_USER") {
                let (hkcu, _) = RegKey::predef(HKEY_CURRENT_USER)
                    .create_subkey("Environment")
                    .map_err(|e| format!("打开注册表失败: {}", e))?;

                hkcu.set_value(&conflict.var_name, &conflict.var_value)
                    .map_err(|e| format!("恢复注册表项失败: {}", e))?;
            } else if conflict.source_path.contains("HKEY_LOCAL_MACHINE") {
                let (hklm, _) = RegKey::predef(HKEY_LOCAL_MACHINE)
                    .create_subkey(
                        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
                    )
                    .map_err(|e| format!("打开系统注册表失败 (需要管理员权限): {}", e))?;

                hklm.set_value(&conflict.var_name, &conflict.var_value)
                    .map_err(|e| format!("恢复系统注册表项失败: {}", e))?;
            }
            Ok(())
        }
        _ => Err(format!(
            "无法恢复类型为 {} 的环境变量",
            conflict.source_type
        )),
    }
}

#[cfg(not(target_os = "windows"))]
fn restore_single_env(conflict: &EnvConflict) -> Result<(), String> {
    match conflict.source_type.as_str() {
        "file" => restore_native_file_conflict(conflict),
        _ => Err(format!(
            "无法恢复类型为 {} 的环境变量",
            conflict.source_type
        )),
    }
}

fn conflict_file_location(conflict: &EnvConflict) -> Result<(&str, Option<usize>), String> {
    if conflict.source_path.is_empty() {
        return Err("环境变量来源文件路径为空".to_string());
    }
    if let Some(line_number) = conflict.line_number {
        if line_number == 0 {
            return Err("环境变量来源行号必须大于 0".to_string());
        }
        return Ok((&conflict.source_path, Some(line_number)));
    }

    // Backward compatibility: old backups encoded the line as `path:line`.
    if let Some((path, suffix)) = conflict.source_path.rsplit_once(':') {
        if let Ok(line_number) = suffix.parse::<usize>() {
            if !path.is_empty() && line_number > 0 {
                return Ok((path, Some(line_number)));
            }
        }
    }
    Ok((&conflict.source_path, None))
}

fn assignment_name(line: &str) -> Option<&str> {
    let assignment = line.trim().strip_prefix("export ").unwrap_or(line.trim());
    assignment.split_once('=').map(|(name, _)| name.trim())
}

fn remove_conflict_line(content: &str, conflict: &EnvConflict) -> Result<String, String> {
    let (_, requested_line) = conflict_file_location(conflict)?;
    let trailing_newline = content.ends_with('\n');
    let mut lines: Vec<&str> = content.lines().collect();
    let index = if let Some(line_number) = requested_line {
        let index = line_number - 1;
        let line = lines
            .get(index)
            .ok_or_else(|| format!("环境变量来源行号已失效: {line_number}"))?;
        if assignment_name(line) != Some(conflict.var_name.as_str()) {
            return Err(format!(
                "环境变量来源行已变化，拒绝删除: {}:{}",
                conflict.source_path, line_number
            ));
        }
        index
    } else {
        lines
            .iter()
            .position(|line| assignment_name(line) == Some(conflict.var_name.as_str()))
            .ok_or_else(|| format!("未找到环境变量 {}", conflict.var_name))?
    };
    lines.remove(index);
    let mut updated = lines.join("\n");
    if trailing_newline && !updated.is_empty() {
        updated.push('\n');
    }
    Ok(updated)
}

fn shell_export_line(conflict: &EnvConflict) -> String {
    let escaped = conflict.var_value.replace('\'', "'\"'\"'");
    format!("export {}='{escaped}'", conflict.var_name)
}

fn restore_conflict_line(content: &str, conflict: &EnvConflict) -> Result<String, String> {
    let (_, requested_line) = conflict_file_location(conflict)?;
    let trailing_newline = content.ends_with('\n');
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let index = requested_line
        .map(|line_number| line_number.saturating_sub(1).min(lines.len()))
        .unwrap_or(lines.len());
    lines.insert(index, shell_export_line(conflict));
    let mut updated = lines.join("\n");
    if trailing_newline || !updated.is_empty() {
        updated.push('\n');
    }
    Ok(updated)
}

#[cfg(not(target_os = "windows"))]
fn delete_native_file_conflict(conflict: &EnvConflict) -> Result<(), String> {
    let (file_path, _) = conflict_file_location(conflict)?;
    let content = fs::read_to_string(file_path)
        .map_err(|error| format!("读取文件失败 {file_path}: {error}"))?;
    let updated = remove_conflict_line(&content, conflict)?;
    fs::write(file_path, updated).map_err(|error| format!("写入文件失败 {file_path}: {error}"))
}

#[cfg(not(target_os = "windows"))]
fn restore_native_file_conflict(conflict: &EnvConflict) -> Result<(), String> {
    let (file_path, _) = conflict_file_location(conflict)?;
    let content = fs::read_to_string(file_path)
        .map_err(|error| format!("读取文件失败 {file_path}: {error}"))?;
    let updated = restore_conflict_line(&content, conflict)?;
    fs::write(file_path, updated).map_err(|error| format!("写入文件失败 {file_path}: {error}"))
}

#[cfg(target_os = "windows")]
fn run_wsl_file_edit(conflict: &EnvConflict, script: &str, extra: &[&str]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let distro = conflict
        .wsl_distro
        .as_deref()
        .ok_or_else(|| "WSL 环境变量记录缺少发行版".to_string())?;
    if distro.is_empty()
        || distro.len() > 64
        || !distro
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err("WSL 发行版名称无效".to_string());
    }
    let (file_path, line_number) = conflict_file_location(conflict)?;
    let line_number = line_number.ok_or_else(|| "WSL 环境变量记录缺少行号".to_string())?;
    let line_number = line_number.to_string();
    let mut command = std::process::Command::new("wsl.exe");
    command.args([
        "-d",
        distro,
        "--",
        "sh",
        "-c",
        script,
        "ofox-env-edit",
        file_path,
        line_number.as_str(),
        conflict.var_name.as_str(),
    ]);
    command.args(extra);
    let output = command
        .creation_flags(0x08000000)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(target_os = "windows")]
fn delete_wsl_file_conflict(conflict: &EnvConflict) -> Result<(), String> {
    if conflict.source_type != "file" {
        return Err("WSL 环境变量必须来自配置文件".to_string());
    }
    let script = concat!(
        "file=$1; line=$2; var=$3; tmp=\"${file}.ofox.$$\"; ",
        "awk -v n=\"$line\" -v var=\"$var\" '",
        "NR==n{s=$0;sub(/^[[:space:]]*export[[:space:]]+/,\"\",s);",
        "split(s,a,\"=\");gsub(/^[[:space:]]+|[[:space:]]+$/,\"\",a[1]);",
        "if(a[1]==var){found=1;next}}{print}END{if(!found)exit 42}' ",
        "\"$file\" >\"$tmp\" && mv \"$tmp\" \"$file\"; ",
        "status=$?; [ $status -eq 0 ] || rm -f \"$tmp\"; exit $status"
    );
    run_wsl_file_edit(conflict, script, &[])
}

#[cfg(target_os = "windows")]
fn restore_wsl_file_conflict(conflict: &EnvConflict) -> Result<(), String> {
    if conflict.source_type != "file" {
        return Err("WSL 环境变量必须来自配置文件".to_string());
    }
    let export_line = shell_export_line(conflict);
    let script = concat!(
        "file=$1; line=$2; text=$4; tmp=\"${file}.ofox.$$\"; before=$((line-1)); ",
        "{ [ $before -le 0 ] || head -n \"$before\" \"$file\"; ",
        "printf '%s\\n' \"$text\"; tail -n \"+$line\" \"$file\"; } >\"$tmp\" ",
        "&& mv \"$tmp\" \"$file\"; status=$?; ",
        "[ $status -eq 0 ] || rm -f \"$tmp\"; exit $status"
    );
    run_wsl_file_edit(conflict, script, &[&export_line])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_conflict(source_path: &str, line_number: Option<usize>) -> EnvConflict {
        EnvConflict {
            var_name: "OPENAI_API_KEY".to_string(),
            var_value: "secret value".to_string(),
            source_type: "file".to_string(),
            source_path: source_path.to_string(),
            line_number,
            environment: EnvEnvironment::Native,
            wsl_distro: None,
        }
    }

    #[test]
    fn test_backup_dir_creation() {
        let backup_dir = get_backup_dir();
        assert!(backup_dir.is_ok());
    }

    #[test]
    fn structured_location_removes_only_the_reported_line() {
        let conflict = file_conflict("/home/alice/.zshrc", Some(2));
        let content = "export OPENAI_API_KEY=old\nexport OPENAI_API_KEY=target\nkeep=yes\n";

        assert_eq!(
            remove_conflict_line(content, &conflict).unwrap(),
            "export OPENAI_API_KEY=old\nkeep=yes\n"
        );
    }

    #[test]
    fn changed_source_line_is_not_deleted() {
        let conflict = file_conflict("/home/alice/.zshrc", Some(2));
        let error = remove_conflict_line("keep=yes\nOTHER=value\n", &conflict).unwrap_err();
        assert!(error.contains("拒绝删除"));
    }

    #[test]
    fn legacy_path_line_backup_is_still_understood() {
        let conflict = file_conflict("/home/alice/.zshrc:12", None);
        assert_eq!(
            conflict_file_location(&conflict).unwrap(),
            ("/home/alice/.zshrc", Some(12))
        );

        let windows = file_conflict(r"C:\Users\Alice\profile:9", None);
        assert_eq!(
            conflict_file_location(&windows).unwrap(),
            (r"C:\Users\Alice\profile", Some(9))
        );
    }

    #[test]
    fn restore_reinserts_at_original_line_with_safe_quoting() {
        let conflict = file_conflict("/home/alice/.zshrc", Some(2));
        assert_eq!(
            restore_conflict_line("first=yes\nthird=yes\n", &conflict).unwrap(),
            "first=yes\nexport OPENAI_API_KEY='secret value'\nthird=yes\n"
        );
    }
}
