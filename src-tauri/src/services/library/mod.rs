//! Local resource ownership. Definitions and machine execution state are distinct.
pub(crate) mod batch;
pub(crate) mod catalog;
pub(crate) mod coordinator;
mod display;
mod instructions;
pub(crate) mod inventory;
pub(crate) mod mcp_actions;
pub(crate) mod mcp_check;
pub(crate) mod mcp_codec;
pub(crate) mod mcp_resource;
#[cfg(test)]
mod native_acceptance_profile;
pub(crate) mod native_json;
pub(crate) mod resource_pack;
pub(crate) mod skill_actions;
pub(crate) mod skill_artifact;
pub(crate) mod skill_resource;
pub(crate) mod skill_view;
mod store;
mod view;
pub(crate) mod yaml_paths;
pub use coordinator::{apply_operation, preview_instructions, recover_operation, save_instruction};
pub use mcp_resource::{
    delete_draft as delete_mcp_draft, preview as preview_mcp_resource, save_draft as save_mcp_draft,
};
pub(crate) use store::load as snapshot_from_connection;
pub(crate) use store::restore_machine_state;
pub use view::{instruction_content_view, instruction_status, operation_view};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use instructions::{patch_instructions, InstructionPatch};
pub(crate) const DEFINITIONS_KEY: &str = "library_definitions_v1";
pub(crate) const MACHINE_KEY: &str = "library_machine_v1";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    #[default]
    Instructions,
    Mcp,
    Skills,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceRevision {
    pub revision: u64,
    pub hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionSet {
    pub id: String,
    pub name: String,
    pub content: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Definitions {
    pub schema_version: u32,
    pub revision: u64,
    pub instructions: BTreeMap<String, InstructionSet>,
    #[serde(default)]
    pub mcp: BTreeMap<String, ResourceRevision>,
    #[serde(default)]
    pub skills: BTreeMap<String, ResourceRevision>,
}

impl Default for Definitions {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            instructions: BTreeMap::new(),
            mcp: BTreeMap::new(),
            skills: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    #[serde(default)]
    pub kind: ResourceKind,
    pub resource_id: String,
    pub resource_revision: u64,
    pub definition_hash: String,
    pub physical_target: String,
    pub path: String,
    pub owners: Vec<String>,
    pub owned_segment: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceFingerprint {
    pub physical_target: String,
    pub hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetWrite {
    #[serde(default)]
    pub directory: Option<skill_resource::DirectoryPlan>,
    #[serde(default)]
    pub transfer: Option<SkillTransfer>,
    pub tools: Vec<String>,
    pub readers: Vec<String>,
    pub path: String,
    pub physical_target: String,
    pub receipt_key: String,
    pub previous_receipt: Option<Receipt>,
    pub assignment_updates: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub source_fingerprints: BTreeMap<String, SourceFingerprint>,
    pub before_hash: String,
    pub after_hash: String,
    pub after_text: String,
    /// The written entry holds plain-text env/header values; the file becomes owner-only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private_values: bool,
    pub receipt: Option<Receipt>,
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillTransfer {
    pub artifact_hash: String,
    pub planned_method: Option<String>,
    pub method: String,
    pub fallback_copy: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Operation {
    #[serde(default)]
    pub kind: ResourceKind,
    pub id: String,
    pub epoch: u64,
    pub resource_id: String,
    pub resource_revision: u64,
    pub resource_hash: String,
    pub created_at: i64,
    pub status: String,
    pub targets: Vec<TargetWrite>,
    pub desired_updates: BTreeMap<String, Option<String>>,
    pub previous_assignments: BTreeMap<String, Option<String>>,
    pub resolved_paths: BTreeMap<String, String>,
    pub resolved_physical: BTreeMap<String, String>,
    #[serde(default)]
    pub routing: Option<RoutingDependencies>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoutingDependencies {
    pub version: u32,
    pub paths: BTreeMap<String, String>,
    pub environment_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MachineState {
    pub schema_version: u32,
    pub revision: u64,
    pub epoch: u64,
    pub assignments: BTreeMap<String, String>,
    pub receipts: BTreeMap<String, Receipt>,
    pub operations: BTreeMap<String, Operation>,
}

impl Default for MachineState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            epoch: 0,
            assignments: BTreeMap::new(),
            receipts: BTreeMap::new(),
            operations: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySnapshot {
    pub definitions: Definitions,
    pub machine: MachineState,
}
