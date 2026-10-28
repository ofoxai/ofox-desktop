//! Data model shared by the assistant store, host and commands. Field names are
//! the renderer's camelCase contract.
use super::catalog::SearchPrice;
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
    /// The thinking level chosen for the model; `None` is the default, which
    /// sends no thinking parameter. A level the model no longer offers is
    /// not sent.
    #[serde(default)]
    pub thinking_level: Option<String>,
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
    /// A committed update succeeded, but unused disk copies need a retry.
    /// Transient response only; never restored from a saved session.
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub cleanup_warning: Option<String>,
}

/// A file imported into a conversation, as saved in its folder. On the
/// session it carries the last outcome the runtime reported for it, on a user
/// message the outcome of that turn: `status` (see [`ATTACHMENT_STATUSES`]),
/// `reason` (the outcome's `assistant_*` code, if any) and `details`.
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
    /// The runtime's outcome details (format, pages, limits); `None` until a
    /// run has read the file.
    #[serde(default)]
    pub details: Option<Value>,
}

/// `ready` once imported; afterwards the runtime's outcome of the last run
/// that read the file.
pub const ATTACHMENT_STATUSES: [&str; 6] = [
    "ready",
    "partial",
    "unreadable",
    "unsupported",
    "too_large",
    "model_unsupported",
];

impl Attachment {
    /// Whether a draft or a send may select it. An outcome belongs to one run
    /// (its model, the other files of the turn), so a failed one does not rule
    /// the file out: the runtime reads it again and reports again, and a
    /// blocked turn costs nothing.
    pub fn selectable(&self) -> bool {
        ATTACHMENT_STATUSES.contains(&self.status.as_str())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: i64,
    pub reasoning: Option<String>,
    /// How long the reply streamed its reasoning, in milliseconds: each span
    /// from a thinking delta to the next text, tool call or end of message.
    #[serde(default)]
    pub reasoning_ms: Option<u64>,
    pub status: String,
    #[serde(default)]
    pub error_code: Option<String>,
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
    /// The search price the run was given, when it could search: its
    /// searches are priced with it for good, whatever the catalog says later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_price: Option<SearchPrice>,
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
    /// The user picked `model_id` rather than taking the default shown: it is
    /// remembered even while some catalog source cannot be read.
    #[serde(default)]
    pub model_picked: bool,
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
    /// One of the model's thinking levels, or `"default"` to clear the choice.
    pub thinking_level: Option<String>,
    pub deleted: Option<bool>,
    pub draft: Option<String>,
    pub draft_attachment_ids: Option<Vec<String>>,
}

/// Destructive requests must name the authority shown by their confirmation.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRequest {
    pub session_id: String,
    pub expected_owner: String,
    pub expected_generation: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    pub expected_owner: String,
    pub expected_generation: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMarkdownRequest {
    #[serde(flatten)]
    pub authority: DeleteRequest,
    pub markdown: String,
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
    /// Explicit edit selection, including an empty list. Old clients that
    /// omit this flag keep the original files when their list is empty.
    #[serde(default)]
    pub replace_attachments: bool,
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

/// An `assistant_*` code: lowercase letters, digits and underscores.
pub fn is_assistant_code(code: &str) -> bool {
    code.len() <= 100
        && code.strip_prefix("assistant_").is_some_and(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
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
        assert!(session.thinking_level.is_none());
    }

    #[test]
    fn a_saved_message_without_a_reasoning_time_reads_without_one() {
        let saved = json!({
            "id": id(), "role": "assistant", "content": "a", "createdAt": 1, "reasoning": null,
            "status": "completed", "revisionOf": null, "nativeEntryId": null,
        });
        let message: Message = serde_json::from_value(saved).unwrap();
        assert!(message.reasoning_ms.is_none());
        let mut value = serde_json::to_value(message).unwrap();
        value["reasoningMs"] = json!(8200);
        let message: Message = serde_json::from_value(value).unwrap();
        assert_eq!(message.reasoning_ms, Some(8200));
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
    fn an_attachment_saved_before_outcomes_reads_without_details() {
        let saved = json!({
            "id": id(), "name": "a.txt", "mimeType": "text/plain", "sizeBytes": 1, "status": "ready"
        });
        let attachment: Attachment = serde_json::from_value(saved).unwrap();
        assert!(attachment.details.is_none());
        let mut value = serde_json::to_value(attachment).unwrap();
        value["details"] = json!({"format": "pdf", "pageCount": 3});
        let attachment: Attachment = serde_json::from_value(value).unwrap();
        assert_eq!(attachment.details.unwrap()["pageCount"], 3);
    }

    #[test]
    fn an_attachment_stays_selectable_whatever_its_last_outcome() {
        let with = |status: &str| -> Attachment {
            serde_json::from_value(json!({
                "id": id(), "name": "a.png", "mimeType": "image/png", "sizeBytes": 1,
                "status": status,
            }))
            .unwrap()
        };
        // An outcome belongs to one run: another model or selection may read it.
        for status in [
            "ready",
            "partial",
            "unreadable",
            "unsupported",
            "too_large",
            "model_unsupported",
        ] {
            assert!(with(status).selectable(), "{status}");
        }
        for status in ["processing", "", "READY"] {
            assert!(!with(status).selectable(), "{status}");
        }
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
