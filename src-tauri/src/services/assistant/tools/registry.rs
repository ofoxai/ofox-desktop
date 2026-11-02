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
