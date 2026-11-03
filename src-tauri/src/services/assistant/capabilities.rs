//! Versioned, engine-independent capability declarations. Only UI requests can
//! change a session's selected capabilities; tool callbacks never can.
use serde::{Deserialize, Serialize};

pub const LOCAL_CONFIG: &str = "local_config";
pub const LOCAL_CONFIG_VERSION: u32 = 1;

pub fn default_assistant_id() -> String {
    "ofox".into()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityRef {
    pub id: String,
    pub version: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub repeat_warning: usize,
    pub repeat_stop: usize,
    pub max_duration_seconds: u64,
    pub max_cost_usd: f64,
    pub max_tool_calls: usize,
    pub completion_retries: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Definition {
    pub id: &'static str,
    pub version: u32,
    pub limits: Limits,
    pub recommended_model: &'static str,
    pub suitable_models: &'static [&'static str],
    pub skill: &'static str,
}

pub fn local_config() -> Definition {
    Definition {
        id: LOCAL_CONFIG,
        version: LOCAL_CONFIG_VERSION,
        limits: Limits {
            repeat_warning: 3,
            repeat_stop: 5,
            max_duration_seconds: 900,
            max_cost_usd: 1.0,
            max_tool_calls: 60,
            completion_retries: 2,
        },
        // User-selected model, using the canonical ID in both regional catalogs.
        recommended_model: "openai/gpt-6.1-sol",
        suitable_models: &["openai/gpt-6.1-sol"],
        skill: "skills/local-config/SKILL.md",
    }
}

pub fn validate(assistant_id: &str, capabilities: &[CapabilityRef]) -> Result<(), String> {
    if assistant_id != "ofox" {
        return Err("assistant_invalid_assistant".into());
    }
    if capabilities.len() > 1
        || capabilities
            .iter()
            .any(|c| c.id != LOCAL_CONFIG || c.version != LOCAL_CONFIG_VERSION)
    {
        return Err("assistant_invalid_capability".into());
    }
    Ok(())
}

pub fn enabled(capabilities: &[CapabilityRef]) -> bool {
    capabilities
        .iter()
        .any(|c| c.id == LOCAL_CONFIG && c.version == LOCAL_CONFIG_VERSION)
}
