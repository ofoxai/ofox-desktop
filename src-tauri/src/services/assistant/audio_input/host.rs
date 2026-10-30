//! Audio import is an unbilled, authority-bound temporary input. It neither
//! writes chat history nor changes the conversation's ordinary draft files.
use super::{slot, Info, Input};
use crate::services::assistant::{
    attachments,
    host::{blocking, AssistantHost, Identity, IDENTITY_CHANGED},
    types::{validate_id, DeleteRequest},
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc, time::Instant};
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub input_id: String,
    pub name: String,
    pub size_bytes: u64,
    #[serde(flatten)]
    pub info: Info,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardRequest {
    #[serde(flatten)]
    pub authority: DeleteRequest,
    pub input_id: String,
}

impl AssistantHost {
    async fn audio_target(&self, authority: &DeleteRequest) -> Result<Identity, String> {
        validate_id(&authority.session_id)?;
        self.admit(authority.expected_generation)?;
        let identity = self.identity().await?;
        if identity.owner != authority.expected_owner {
            return Err(IDENTITY_CHANGED.into());
        }
        self.ensure_current(&identity, authority.expected_generation)
            .await?;
        let record = blocking({
            let store = self.store(&identity.owner)?;
            let sid = authority.session_id.clone();
            move || store.load_record(&sid)
        })
        .await?;
        if record.session.deleted_at.is_some() {
            return Err("assistant_session_deleted".into());
        }
        Ok(identity)
    }
    async fn hold_audio(
        self: &Arc<Self>,
        authority: DeleteRequest,
        identity: Identity,
        input: Input,
    ) -> Result<Preview, String> {
        self.audio_target(&authority).await?;
        self.ensure_current(&identity, authority.expected_generation)
            .await?;
        let mut slot = self
            .audio_inputs
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        // Revocation clears this slot under the same lock. If it finished
        // first, the new generation refuses this late import.
        self.admit(authority.expected_generation)?;
        let preview = slot.put(&authority, input, Instant::now());
        let weak = Arc::downgrade(self);
        let id = preview.input_id.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(slot::TTL).await;
            if let Some(host) = weak.upgrade() {
                if let Ok(mut inputs) = host.audio_inputs.lock() {
                    inputs.expire(&id, Instant::now());
                };
            }
        });
        slot.set_timer(task.abort_handle());
        Ok(preview)
    }
    /// The path comes only from a native picker, never an IPC argument.
    pub async fn pick_audio(
        self: &Arc<Self>,
        authority: DeleteRequest,
        pick: impl FnOnce() -> Result<Option<PathBuf>, String> + Send + 'static,
    ) -> Result<Option<Preview>, String> {
        let _import = self
            .audio_importing
            .try_lock()
            .map_err(|_| "assistant_session_busy")?;
        let identity = self.audio_target(&authority).await?;
        let Some(path) = blocking(pick).await? else {
            return Ok(None);
        };
        self.ensure_current(&identity, authority.expected_generation)
            .await?;
        let input = blocking(move || Input::from_path(&path)).await?;
        self.hold_audio(authority, identity, input).await.map(Some)
    }
    /// Raw recorder bytes undergo exactly the file import's native checks.
    pub async fn import_audio(
        self: &Arc<Self>,
        authority: DeleteRequest,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<Preview, String> {
        if bytes.len() > super::MAX_BYTES {
            return Err(attachments::TOO_LARGE.into());
        }
        let _import = self
            .audio_importing
            .try_lock()
            .map_err(|_| "assistant_session_busy")?;
        let identity = self.audio_target(&authority).await?;
        let input = blocking(move || Input::from_bytes(&name, bytes)).await?;
        self.hold_audio(authority, identity, input).await
    }
    pub async fn discard_audio(&self, request: DiscardRequest) -> Result<bool, String> {
        validate_id(&request.input_id)?;
        validate_id(&request.authority.session_id)?;
        self.admit(request.authority.expected_generation)?;
        let identity = self.identity().await?;
        if identity.owner != request.authority.expected_owner {
            return Err(IDENTITY_CHANGED.into());
        }
        self.ensure_current(&identity, request.authority.expected_generation)
            .await?;
        let mut slot = self
            .audio_inputs
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        self.admit(request.authority.expected_generation)?;
        slot.discard(&request.authority, &request.input_id, Instant::now())
    }
}
