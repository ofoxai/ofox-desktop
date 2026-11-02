//! Explicit tool metadata. Registering a name alone never grants authority.
use super::policy::{Category, Operation};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug)]
pub struct Tool {
    pub name: &'static str,
    pub category: Category,
    pub retry_safe: bool,
    pub reversible: bool,
    pub structured: bool,
    pub irreversible: bool,
}

impl Tool {
    pub fn operation(self, in_scope: bool, forbidden: bool, read_only_command: bool) -> Operation {
        Operation {
            category: self.category,
            in_scope,
            forbidden,
            read_only_command,
            reversible: self.reversible,
            structured: self.structured,
            irreversible: self.irreversible,
        }
    }
}

pub fn get(name: &str) -> Result<Tool, String> {
    const READ: &[&str] = &[
        "read",
        "stat",
        "ls",
        "find",
        "grep",
        "tool_status",
        "diagnose_in_folder",
        "verify_in_folder",
        "request_folder",
    ];
    if let Some(name) = READ.iter().find(|known| **known == name) {
        return Ok(Tool {
            name,
            category: Category::Read,
            retry_safe: true,
            reversible: false,
            structured: true,
            irreversible: false,
        });
    }
    match name {
        #[cfg(test)]
        "probe_write" => Ok(Tool {
            name: "probe_write",
            category: Category::WriteConfig,
            retry_safe: false,
            reversible: false,
            structured: false,
            irreversible: false,
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
        "tool_status" => {
            let args: ToolArgs = decode(args)?;
            supported_tool(&args.tool).map(|_| ())
        }
        "diagnose_in_folder" | "verify_in_folder" => {
            let args: FolderArgs = decode(args)?;
            path_ok(&args.folder)?;
            supported_tool(&args.tool).map(|_| ())
        }
        "request_folder" => {
            if args.as_object().is_some_and(|a| a.is_empty()) {
                Ok(())
            } else {
                Err("assistant_invalid_tool".into())
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
