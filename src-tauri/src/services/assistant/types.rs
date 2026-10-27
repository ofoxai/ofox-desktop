use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use super::policy::PermissionMode;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AssistantKind {
    General,
    Configuration,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRef {
    pub run_id: String,
    pub operation_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub kind: AssistantKind,
    pub title: String,
    pub model_id: String,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    pub web_search: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    #[serde(default)]
    pub active_run: Option<RunRef>,
    #[serde(default)]
    pub draft: String,
    #[serde(default)]
    pub draft_attachment_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub preview_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: i64,
    pub reasoning: Option<String>,
    pub status: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub sources: Vec<Value>,
    #[serde(default)]
    pub superseded: bool,
    pub revision_of: Option<String>,
    pub native_entry_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    pub content: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub session: Session,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_approval: Option<Value>,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub memories: Vec<Memory>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub usage: Value,
    #[serde(default)]
    pub operations: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequest {
    pub session_id: String,
    pub expected_owner: Option<String>,
    pub expected_generation: Option<u64>,
    pub title: Option<String>,
    pub model_id: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub full_access_acknowledged: bool,
    pub web_search: Option<bool>,
    pub deleted: Option<bool>,
    pub draft: Option<String>,
    pub draft_attachment_ids: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    pub session_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub text: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    pub edit_message_id: Option<String>,
    pub regenerate_message_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRequest {
    pub session_id: String,
    pub id: Option<String>,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub session_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub approval_id: String,
    pub decision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub session_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub r#type: String,
    pub data: Value,
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn validate_id(value: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| "assistant_invalid_identifier".to_string())
}
