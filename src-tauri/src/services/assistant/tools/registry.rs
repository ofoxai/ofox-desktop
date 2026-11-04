//! Explicit tool metadata. Registering a name alone never grants authority.
use super::policy::{Category, Operation};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug)]
pub struct Tool {
    pub name: &'static str,
    pub category: Category,
}

impl Tool {
    pub fn operation(self, forbidden: bool, read_only_command: bool) -> Operation {
        Operation {
            category: self.category,
            read_only_command,
            forbidden,
        }
    }
}

pub fn get(name: &str) -> Result<Tool, String> {
    const READ: &[&str] = &[
        "use_skill",
        "read",
        "stat",
        "ls",
        "find",
        "grep",
        "tool_status",
        "diagnose_in_folder",
        "verify_in_folder",
    ];
    const WRITE: &[&str] = &[
        "write",
        "edit",
        "repair_launcher",
        "rebind_tool",
        "set_tool_model",
    ];
    const EXEC: &[&str] = &[
        "bash",
        "powershell",
        "install_tool",
        "update_tool",
        "uninstall_tool",
    ];
    for (names, category) in [
        (READ, Category::Read),
        (WRITE, Category::WriteConfig),
        (EXEC, Category::Exec),
    ] {
        if let Some(name) = names.iter().find(|known| **known == name) {
            return Ok(Tool { name, category });
        }
    }
    match name {
        #[cfg(test)]
        "probe_write" => Ok(Tool {
            name: "probe_write",
            category: Category::WriteConfig,
        }),
        _ => Err("assistant_unknown_tool".into()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Call {
    pub session_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub tool_call_id: String,
    pub name: String,
    pub args: Value,
}

pub fn validate_args(tool: Tool, args: &Value) -> Result<(), String> {
    match tool.name {
        "use_skill" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct SkillArgs {
                name: String,
            }
            let a: SkillArgs = decode(args)?;
            if a.name.is_empty()
                || a.name.len() > 128
                || a.name.contains("..")
                || a.name.contains(['/', '\\'])
            {
                Err("not_available: invalid name".into())
            } else {
                Ok(())
            }
        }
        "read" | "stat" => {
            let args: PathArgs = decode(args)?;
            path_ok(&args.path)
        }
        "ls" => {
            let args: ListArgs = decode(args)?;
            path_ok(&args.path)?;
            limit_ok(args.limit)
        }
        "find" | "grep" => {
            let args: SearchArgs = decode(args)?;
            path_ok(&args.path)?;
            limit_ok(args.limit)?;
            if args.pattern.len() > 512 {
                Err("assistant_invalid_tool".into())
            } else {
                Ok(())
            }
        }
        "tool_status" | "repair_launcher" | "rebind_tool" | "install_tool" | "update_tool"
        | "uninstall_tool" => {
            let args: ToolArgs = decode(args)?;
            supported_tool(&args.tool).map(|_| ())
        }
        "diagnose_in_folder" | "verify_in_folder" => {
            let args: FolderArgs = decode(args)?;
            path_ok(&args.folder)?;
            supported_tool(&args.tool).map(|_| ())
        }
        "write" | "edit" => {
            let args: WriteArgs = decode(args)?;
            path_ok(&args.path)?;
            if args.content.len() > 384 * 1024
                || args.expected.as_ref().is_some_and(|s| s.len() > 384 * 1024)
            {
                Err("assistant_invalid_tool".into())
            } else {
                Ok(())
            }
        }
        "bash" | "powershell" => {
            let args: ExecArgs = decode(args)?;
            path_ok(&args.folder)?;
            if args.command.is_empty()
                || args.command.len() > 16000
                || !(1..=600).contains(&args.timeout)
                || (tool.name == "powershell") != cfg!(windows)
            {
                Err("assistant_invalid_tool".into())
            } else {
                Ok(())
            }
        }
        "set_tool_model" => {
            let args: ModelArgs = decode(args)?;
            supported_tool(&args.tool)?;
            if args.model.is_empty() || args.model.len() > 256 {
                Err("assistant_invalid_tool".into())
            } else {
                Ok(())
            }
        }
        #[cfg(test)]
        "probe_write" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Probe {
                command: String,
            }
            let probe: Probe =
                serde_json::from_value(args.clone()).map_err(|_| "assistant_invalid_tool")?;
            if probe.command.is_empty() || probe.command.len() > 200 {
                return Err("assistant_invalid_tool".into());
            }
            Ok(())
        }
        _ => {
            let _ = args;
            Err("assistant_unknown_tool".into())
        }
    }
}

pub fn decode<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, String> {
    serde_json::from_value(args.clone()).map_err(|_| "assistant_invalid_tool".into())
}
fn path_ok(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 4096 {
        Err("assistant_invalid_tool".into())
    } else {
        Ok(())
    }
}
fn limit_ok(limit: usize) -> Result<(), String> {
    if (1..=1000).contains(&limit) {
        Ok(())
    } else {
        Err("assistant_invalid_tool".into())
    }
}
pub fn supported_tool(tool: &str) -> Result<crate::app_config::BindableTool, String> {
    use std::str::FromStr;
    crate::app_config::BindableTool::from_str(tool).map_err(|_| "assistant_invalid_tool".into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathArgs {
    pub path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListArgs {
    pub path: String,
    pub limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    pub path: String,
    pub pattern: String,
    pub limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolArgs {
    pub tool: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderArgs {
    pub tool: String,
    pub folder: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteArgs {
    pub path: String,
    pub content: String,
    pub expected: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecArgs {
    pub command: String,
    pub folder: String,
    pub timeout: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelArgs {
    pub tool: String,
    pub model: String,
}
