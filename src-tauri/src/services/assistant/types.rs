//! Data model shared by the assistant store, host and commands. Field names are
//! the renderer's camelCase contract.
use super::catalog::SearchPrice;
pub use super::media::ImageRequest;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Persisted media state, also kept in the operation ledger so a later
/// catalog change or branch switch cannot change what the request cost.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaRecord {
    /// The model's tool call that asked for it; `None` for a direct media turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// The prompt (or, for speech, the text) that was sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub request_attempts: Vec<super::provenance::Receipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_usage: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_snapshot: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<super::audio::Trace>,
    pub kind: String,
    pub model: String,
    pub params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_started_at: Option<i64>,
    #[serde(default)]
    pub input_ids: Vec<String>,
    pub status: String,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub cost: Option<String>,
    /// Why a generation tool's job failed; a direct media turn keeps its
    /// code on the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

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

/// Last SDK context estimate. Unknown after compaction until a new reply.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub model_id: String,
    pub tokens: Option<u64>,
    pub context_window: u64,
}
impl ContextUsage {
    pub(super) fn parse(value: &Value) -> Option<Self> {
        let usage: Self = serde_json::from_value(value.clone()).ok()?;
        (usage.context_window > 0
            && usage.context_window <= 9_007_199_254_740_991
            && usage.tokens.is_none_or(|n| n <= 9_007_199_254_740_991)
            && !usage.model_id.is_empty())
        .then_some(usage)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    #[serde(default = "super::capabilities::default_assistant_id")]
    pub assistant_id: String,
    #[serde(default)]
    pub capabilities: Vec<super::capabilities::CapabilityRef>,
    /// Live audio task, never resumed from a saved snapshot.
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub audio_only: Option<bool>,
    /// Live runtime state only; never resumed from a saved record/backup.
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub compacting: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_usage: Option<ContextUsage>,
    pub id: String,
    pub title: String,
    /// Who named the conversation. `None` only in a record saved before this
    /// field existed: every read derives it (see `store`), so a session the
    /// renderer or host holds always has one.
    #[serde(default)]
    pub title_source: Option<TitleSource>,
    pub model_id: String,
    /// Web search (S3) stays off until the user turns it on.
    #[serde(default)]
    pub web_search: bool,
    /// What the generation tools use unless the user says otherwise.
    #[serde(default)]
    pub media: super::media_tools::MediaDefaults,
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
    /// When the conversation was pinned to the top of the list.
    #[serde(default)]
    pub pinned_at: Option<i64>,
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

/// Who named a conversation: the first message (`auto`; a generated title
/// may still replace it) or the user (`user`; never replaced).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TitleSource {
    Auto,
    User,
}

/// How many characters of the first message name a conversation.
const TITLE_CHARS: usize = 48;

/// What a conversation is called until it is renamed: the start of its first
/// message, or of the first file's name when a file is sent alone.
pub fn auto_title(text: &str, attachments: &[Attachment]) -> String {
    let named = match text.trim() {
        "" => attachments.first().map_or("", |a| a.name.as_str()),
        text => text,
    };
    named.chars().take(TITLE_CHARS).collect()
}

/// A file imported into a conversation, as saved in its folder. On the
/// session it carries the last outcome the runtime reported for it, on a user
/// message the outcome of that turn: `status` (see [`ATTACHMENT_STATUSES`]),
/// `reason` (the outcome's `assistant_*` code, if any) and `details`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaRecord>,
    /// What the reply's generation tools made, one record per tool call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generations: Vec<MediaRecord>,
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
    /// The model a reply was asked of; `None` on a user turn and on a reply
    /// saved before replies named it.
    #[serde(default)]
    pub model_id: Option<String>,
    /// What a reply's run used and cost, shaped like [`Detail::usage`]:
    /// counted from native history like the conversation's, without its
    /// title or separately requested compaction. Adding those operations' usage
    /// to the replies' reaches the conversation total (costs nearly so, as each
    /// is a sum of floats), as
    /// long as every native usage record names its run (older histories may
    /// hold some that do not, counted only in the conversation's). `None` on
    /// a user turn and until native history counts the run.
    #[serde(default)]
    pub usage: Option<Value>,
    /// When a reply's run ended, however it ended.
    #[serde(default)]
    pub completed_at: Option<i64>,
    /// For a user turn native history holds: the native message it follows
    /// (`Some(None)` when it starts the conversation). Every version of a
    /// turn (edited, regenerated) follows the same one, so the turns that
    /// share it are the versions to switch between. `None` (left out of the
    /// JSON) when not known: not a native turn, or saved before this existed.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub branch_from: Option<Option<String>>,
    /// For a turn sent after this existed: the native message it was sent
    /// after (the last one shown then), `None` when there was none. A turn
    /// native history never got (its run failed before reaching it) is shown
    /// with the version this message is on. A retried turn has none: it stays
    /// hidden on every version.
    #[serde(default)]
    pub anchor_entry_id: Option<String>,
}

impl Message {
    /// The generation a tool call of this reply made.
    pub fn generation_mut(&mut self, call: &str) -> Option<&mut MediaRecord> {
        self.generations
            .iter_mut()
            .find(|g| g.tool_call_id.as_deref() == Some(call))
    }

    /// A direct media turn's record (`artifact` `None`), or the generation of
    /// the tool call `artifact`.
    pub fn media_for(&self, artifact: Option<&str>) -> Option<&MediaRecord> {
        match artifact {
            None => self.media.as_ref(),
            Some(call) => self
                .generations
                .iter()
                .find(|g| g.tool_call_id.as_deref() == Some(call)),
        }
    }

    pub fn media_for_mut(&mut self, artifact: Option<&str>) -> Option<&mut MediaRecord> {
        match artifact {
            None => self.media.as_mut(),
            Some(call) => self.generation_mut(call),
        }
    }

    /// Every media record of the message: its own and its generations'.
    pub fn all_media(&self) -> impl Iterator<Item = &MediaRecord> {
        self.media.iter().chain(self.generations.iter())
    }

    /// Save `media` as its tool call's generation, replacing an older copy.
    pub fn put_generation(&mut self, media: MediaRecord) {
        match media
            .tool_call_id
            .clone()
            .and_then(|call| self.generation_mut(&call))
        {
            Some(saved) => *saved = media,
            None => self.generations.push(media),
        }
    }
}

/// A field that is there, `null` included (`Some(None)`); one left out is
/// `None` through `#[serde(default)]`.
fn present<'de, D: Deserializer<'de>>(value: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(value).map(Some)
}

/// One row of a session's internal run ledger: an assistant request keyed by
/// the renderer's operation id. Lives in `session.json` so a resubmitted or
/// already-stopped operation is refused; never part of [`Detail`].
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaRecord>,
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
    /// A remote job is known; querying it resumes without repeating submission.
    PendingMedia,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pending_approvals: Vec<Value>,
    /// Presentation only; never stored in a session or portable backup.
    pub permission_mode: super::tools::policy::PermissionMode,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub audio_operations: Vec<super::audio::ResultRecord>,
    pub session: Session,
    pub messages: Vec<Message>,
    pub attachments: Vec<Attachment>,
    pub usage: Value,
}

/// The most characters the custom instructions may have.
pub const MAX_INSTRUCTIONS_CHARS: usize = 4_000;

/// The assistant settings of one account: standing instructions every run of
/// every conversation follows from its next message, whether a conversation's
/// first answer gets it a generated title, and whether a finished reply is
/// announced. Kept in `prefs.json` beside the remembered model.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    /// At most [`MAX_INSTRUCTIONS_CHARS`] characters; empty for none.
    pub instructions: String,
    pub auto_title: bool,
    pub notify_on_finish: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            instructions: String::new(),
            auto_title: true,
            notify_on_finish: true,
        }
    }
}

/// Save all of the assistant settings at once.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPreferencesRequest {
    /// Checked by the host against the signed-in owner, not by the store.
    pub expected_owner: Option<String>,
    /// Checked by the host against its authority generation, not by the store.
    pub expected_generation: Option<u64>,
    #[serde(flatten)]
    pub preferences: Preferences,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    #[serde(default = "super::capabilities::default_assistant_id")]
    pub assistant_id: String,
    #[serde(default)]
    pub capabilities: Vec<super::capabilities::CapabilityRef>,
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
    pub assistant_id: Option<String>,
    pub capabilities: Option<Vec<super::capabilities::CapabilityRef>>,
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
    /// Pin to, or unpin from, the top of the list.
    pub pinned: Option<bool>,
    pub draft: Option<String>,
    pub draft_attachment_ids: Option<Vec<String>>,
    pub media: Option<super::media_tools::MediaDefaults>,
}

/// Destructive requests must name the authority shown by their confirmation.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRequest {
    pub session_id: String,
    pub expected_owner: String,
    pub expected_generation: u64,
}

/// Emptying the trash names the authority shown by its confirmation, like a
/// permanent deletion.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmptyTrashRequest {
    pub expected_owner: String,
    pub expected_generation: u64,
}

/// What emptying the trash did: conversations removed for good, and those not
/// fully removed. A conversation still replying stays in the trash; one whose
/// files could not all be removed is hidden and cleaned up at the next start.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmptyTrashReport {
    pub deleted: usize,
    pub failed: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    #[serde(default)]
    pub exclude_videos: bool,
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
    /// The renderer's resolved language, including first-launch OS detection.
    /// Older callers omit it and use the saved application preference.
    #[serde(default)]
    pub ui_language: Option<String>,
    /// Optional together for older callers; new sends freeze the account and
    /// region generation before IPC admission, including ordinary queues.
    #[serde(default)]
    pub expected_owner: Option<String>,
    #[serde(default)]
    pub expected_generation: Option<u64>,
    #[serde(default)]
    pub media: Option<ImageRequest>,
    #[serde(default)]
    pub video: Option<super::video::VideoRequest>,
    #[serde(default)]
    pub quote_id: Option<String>,
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
    /// A failed or stopped reply to ask again for, with this send's own text
    /// and files; that turn is superseded. When native history holds its
    /// question (checked against the history first, which a failed run may
    /// have left unsaved), it is asked again from before it, like an edit;
    /// otherwise from the end of the conversation, and the turn is hidden on
    /// every version.
    #[serde(default)]
    pub retry_message_id: Option<String>,
}

/// Text admitted into a running chat, tied to its authority and operation.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SteerRequest {
    pub session_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub id: String,
    pub text: String,
    pub expected_owner: String,
    pub expected_generation: u64,
}

/// Show another version of a turn: `message_id` is the user turn to show.
/// Names the authority the page showed, like a deletion.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchVersionRequest {
    #[serde(flatten)]
    pub authority: DeleteRequest,
    pub message_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactRequest {
    #[serde(flatten)]
    pub authority: DeleteRequest,
    pub run_id: String,
    pub operation_id: String,
}

/// Copy a selected native branch into an independent conversation.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForkRequest {
    #[serde(flatten)]
    pub authority: DeleteRequest,
    pub message_id: String,
    pub title: String,
}

/// Import the files last dropped on the window. It names the token the
/// window's drop event gave the renderer, never a path (an unknown field is
/// refused), and the authority the page showed.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportDropRequest {
    pub session_id: String,
    pub token: String,
    pub expected_owner: String,
    pub expected_generation: u64,
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
        assert!(session.pinned_at.is_none());
        // Derived from the conversation when a saved one is read (see `store`).
        assert!(session.title_source.is_none());
    }

    #[test]
    fn the_title_source_and_pin_time_use_the_renderers_names() {
        let mut saved = minimal_session();
        saved["pinnedAt"] = json!(1700);
        saved["titleSource"] = json!("user");
        let session: Session = serde_json::from_value(saved).unwrap();
        assert_eq!(session.pinned_at, Some(1700));
        assert_eq!(session.title_source, Some(TitleSource::User));
        let value = serde_json::to_value(session).unwrap();
        assert_eq!(value["pinnedAt"], 1700);
        assert_eq!(value["titleSource"], "user");
        assert_eq!(serde_json::to_value(TitleSource::Auto).unwrap(), "auto");
    }

    #[test]
    fn an_automatic_title_is_the_first_48_characters_of_the_text_or_the_first_file() {
        let file = |name: &str| -> Attachment {
            serde_json::from_value(json!({
                "id": id(), "name": name, "mimeType": "text/plain", "sizeBytes": 1,
                "status": "ready",
            }))
            .unwrap()
        };
        assert_eq!(auto_title("  hello  ", &[file("a.txt")]), "hello");
        assert_eq!(auto_title("   ", &[file("a.txt"), file("b.txt")]), "a.txt");
        assert_eq!(auto_title("", &[]), "");
        let long = "字".repeat(60);
        assert_eq!(auto_title(&long, &[]).chars().count(), 48);
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
    fn a_message_saved_before_per_message_meta_reads_without_it() {
        let saved = json!({
            "id": id(), "role": "user", "content": "q", "createdAt": 1, "reasoning": null,
            "status": "completed", "revisionOf": null, "nativeEntryId": "n1",
        });
        let message: Message = serde_json::from_value(saved).unwrap();
        assert!(message.model_id.is_none());
        assert!(message.usage.is_none());
        assert!(message.completed_at.is_none());
        assert!(message.branch_from.is_none());
        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(value["modelId"], Value::Null);
        assert_eq!(value["completedAt"], Value::Null);
        // Not known is left out; a turn that starts the conversation is null.
        assert!(value.get("branchFrom").is_none());
        for (branch_from, expected) in [
            (json!(null), Some(None)),
            (json!("a1"), Some(Some("a1".to_string()))),
        ] {
            let mut saved = value.clone();
            saved["branchFrom"] = branch_from.clone();
            let message: Message = serde_json::from_value(saved).unwrap();
            assert_eq!(message.branch_from, expected);
            assert_eq!(
                serde_json::to_value(&message).unwrap()["branchFrom"],
                branch_from
            );
        }
    }

    #[test]
    fn a_send_without_a_retry_reads_as_none_and_a_switch_names_its_authority() {
        let send: SendRequest = serde_json::from_value(json!({
            "sessionId": id(), "runId": id(), "operationId": id(), "text": "hi",
        }))
        .unwrap();
        assert!(send.retry_message_id.is_none());
        let switch: SwitchVersionRequest = serde_json::from_value(json!({
            "sessionId": "s", "messageId": "m", "expectedOwner": "o", "expectedGeneration": 4,
        }))
        .unwrap();
        assert_eq!(switch.authority.session_id, "s");
        assert_eq!(switch.authority.expected_owner, "o");
        assert_eq!(switch.authority.expected_generation, 4);
        assert_eq!(switch.message_id, "m");
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
            pending_approvals: vec![],
            permission_mode: Default::default(),
            audio_operations: vec![],
            session: serde_json::from_value(minimal_session()).unwrap(),
            messages: vec![],
            attachments: vec![],
            usage: json!({}),
        };
        let value = serde_json::to_value(detail).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "attachments",
                "messages",
                "permissionMode",
                "session",
                "usage"
            ]
        );
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
    #[test]
    fn context_estimates_keep_unknown_distinct_from_zero_and_refuse_invalid_counts() {
        let valid = json!({"modelId":"m","tokens":15,"contextWindow":32000});
        assert_eq!(ContextUsage::parse(&valid).unwrap().tokens, Some(15));
        let unknown = json!({"modelId":"m","tokens":null,"contextWindow":32000});
        assert!(ContextUsage::parse(&unknown).unwrap().tokens.is_none());
        for value in [
            Value::Null,
            json!({}),
            json!({"modelId":"m","tokens":-1,"contextWindow":32000}),
            json!({"modelId":"m","tokens":"15","contextWindow":32000}),
            json!({"modelId":"m","tokens":15,"contextWindow":0}),
            json!({"modelId":"m","tokens":9007199254740992u64,"contextWindow":32000}),
        ] {
            assert!(ContextUsage::parse(&value).is_none());
        }
        let old: Session = serde_json::from_value(minimal_session()).unwrap();
        assert!(old.context_usage.is_none());
        let mut saved = minimal_session();
        saved["compacting"] = json!(true);
        assert!(serde_json::from_value::<Session>(saved)
            .unwrap()
            .compacting
            .is_none());
    }
}
