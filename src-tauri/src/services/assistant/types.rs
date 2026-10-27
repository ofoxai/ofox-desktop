//! Data model shared by the assistant store, host and commands. Field names are
//! the renderer's camelCase contract.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The proxy a run's model requests go through. `url` may carry the proxy's
/// credentials: it goes to the runtime only, never to a log or an event.
#[derive(Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunProxy {
    pub url: String,
    /// Hosts that bypass the proxy, comma separated (`NO_PROXY` syntax).
    pub no_proxy: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunRef {
    pub run_id: String,
    pub operation_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub title: String,
    pub model_id: String,
    /// Web search (S3) stays off until the user turns it on.
    #[serde(default)]
    pub web_search: bool,
    pub created_at: i64,
    pub updated_at: i64,
    /// Set while the session sits in the trash (S4).
    #[serde(default)]
    pub deleted_at: Option<i64>,
    #[serde(default)]
    pub active_run: Option<RunRef>,
    #[serde(default)]
    pub draft: String,
    /// Attachments (S2) picked in the composer but not sent yet.
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

/// One row of a session's internal run ledger: an assistant request keyed by
/// the renderer's operation id. Lives in `session.json` so a resubmitted or
/// already-stopped operation is refused; never part of [`Detail`].
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub id: String,
    pub run_id: String,
    pub status: OperationStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationStatus {
    Running,
    Completed,
    Failed,
    /// Stopped by the user, possibly before the request was registered.
    Cancelled,
    /// The app went away while the request was running.
    Interrupted,
}

/// What the renderer gets for one session. The run ledger stays in the host.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub session: Session,
    pub messages: Vec<Message>,
    pub attachments: Vec<Attachment>,
    pub usage: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    /// The catalog's default (or the remembered choice) when absent.
    pub model_id: Option<String>,
    #[serde(default)]
    pub web_search: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequest {
    pub session_id: String,
    /// Checked by the host against the signed-in owner, not by the store.
    pub expected_owner: Option<String>,
    /// Checked by the host against its authority generation, not by the store.
    pub expected_generation: Option<u64>,
    pub title: Option<String>,
    pub model_id: Option<String>,
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

/// Only the lowercase hyphenated form [`id`] produces. Ids name folders, so a
/// braced, URN, simple or uppercase spelling of the same UUID is refused.
pub fn validate_id(value: &str) -> Result<(), String> {
    match uuid::Uuid::parse_str(value) {
        Ok(parsed) if parsed.hyphenated().to_string() == value => Ok(()),
        _ => Err("assistant_invalid_identifier".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_session() -> Value {
        json!({"id": id(), "title": "", "modelId": "test/model", "createdAt": 1, "updatedAt": 1})
    }

    #[test]
    fn a_saved_session_without_later_stage_fields_reads_with_their_defaults() {
        let session: Session = serde_json::from_value(minimal_session()).unwrap();
        assert!(!session.web_search);
        assert!(session.deleted_at.is_none());
        assert!(session.active_run.is_none());
        assert!(session.draft.is_empty());
        assert!(session.draft_attachment_ids.is_empty());
    }

    #[test]
    fn draft_attachment_references_survive_a_round_trip() {
        let mut saved = minimal_session();
        saved["draftAttachmentIds"] = json!(["attachment-id"]);
        let session: Session = serde_json::from_value(saved).unwrap();
        assert_eq!(
            serde_json::to_value(session).unwrap()["draftAttachmentIds"],
            json!(["attachment-id"])
        );
    }

    #[test]
    fn detail_carries_no_run_ledger() {
        let detail = Detail {
            session: serde_json::from_value(minimal_session()).unwrap(),
            messages: vec![],
            attachments: vec![],
            usage: json!({}),
        };
        let value = serde_json::to_value(detail).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["attachments", "messages", "session", "usage"]);
    }

    #[test]
    fn only_lowercase_hyphenated_uuids_are_identifiers() {
        let canonical = id();
        assert!(validate_id(&canonical).is_ok());
        let simple = canonical.replace('-', "");
        // One spelling per id: other UUID forms would name a second folder for it.
        for bad in [
            String::new(),
            "../etc".into(),
            "not-a-uuid".into(),
            canonical.to_uppercase(),
            format!("{{{canonical}}}"),
            format!("urn:uuid:{canonical}"),
            simple,
        ] {
            assert_eq!(
                validate_id(&bad).unwrap_err(),
                "assistant_invalid_identifier",
                "{bad}"
            );
        }
    }
}
