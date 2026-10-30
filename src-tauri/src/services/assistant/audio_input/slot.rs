//! One private short-audio draft per host. Replacing it releases the earlier
//! bytes; authority revocation, expiry and explicit discard do the same.
use super::{Input, Preview};
use crate::services::assistant::{host::IDENTITY_CHANGED, types::DeleteRequest};
use std::time::{Duration, Instant};

pub(super) const TTL: Duration = Duration::from_secs(15 * 60);
#[derive(Default)]
pub(crate) struct Slot {
    pending: Option<Pending>,
    timer: Option<tokio::task::AbortHandle>,
}
struct Pending {
    id: String,
    owner: String,
    generation: u64,
    session_id: String,
    at: Instant,
    input: Input,
}
impl Slot {
    pub(super) fn put(&mut self, authority: &DeleteRequest, input: Input, now: Instant) -> Preview {
        self.clear();
        let id = uuid::Uuid::new_v4().to_string();
        let preview = Preview {
            input_id: id.clone(),
            name: input.name.clone(),
            size_bytes: input.bytes.len() as u64,
            info: input.info.clone(),
        };
        self.pending = Some(Pending {
            id,
            owner: authority.expected_owner.clone(),
            generation: authority.expected_generation,
            session_id: authority.session_id.clone(),
            at: now,
            input,
        });
        preview
    }
    pub(super) fn set_timer(&mut self, timer: tokio::task::AbortHandle) {
        self.timer = Some(timer);
    }
    pub(super) fn expire(&mut self, id: &str, now: Instant) {
        if self
            .pending
            .as_ref()
            .is_some_and(|v| v.id == id && now.saturating_duration_since(v.at) >= TTL)
        {
            self.clear();
        }
    }
    pub(super) fn discard(
        &mut self,
        authority: &DeleteRequest,
        id: &str,
        now: Instant,
    ) -> Result<bool, String> {
        let Some(pending) = self.pending.as_ref().filter(|v| v.id == id) else {
            return Ok(false);
        };
        if pending.owner != authority.expected_owner
            || pending.generation != authority.expected_generation
            || pending.session_id != authority.session_id
        {
            return Err(IDENTITY_CHANGED.into());
        }
        let expired = now.saturating_duration_since(pending.at) >= TTL;
        self.clear();
        Ok(!expired)
    }
    pub(crate) fn clear(&mut self) {
        if let Some(pending) = self.pending.take() {
            drop(pending.input);
        }
        if let Some(timer) = self.timer.take() {
            timer.abort();
        }
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> DeleteRequest {
        DeleteRequest {
            session_id: uuid::Uuid::new_v4().to_string(),
            expected_owner: "a".repeat(64),
            expected_generation: 7,
        }
    }
    fn input() -> Input {
        Input::from_bytes(
            "tone.mp3",
            include_bytes!("../../../../tests/fixtures/assistant-audio/tone-16k-mono.mp3").to_vec(),
        )
        .unwrap()
    }
    #[test]
    fn replacing_or_discarding_audio_releases_only_the_matching_input() {
        let mut slot = Slot::default();
        let target = target();
        let now = Instant::now();
        let first = slot.put(&target, input(), now);
        let second = slot.put(&target, input(), now);
        assert!(!slot.discard(&target, &first.input_id, now).unwrap());
        assert!(slot.discard(&target, &second.input_id, now).unwrap());
        assert!(!slot.discard(&target, &second.input_id, now).unwrap());
    }
    #[test]
    fn an_input_is_bound_to_its_account_generation_and_conversation() {
        let mut slot = Slot::default();
        let target = target();
        let now = Instant::now();
        let pending = slot.put(&target, input(), now);
        for value in [
            serde_json::json!({"sessionId": target.session_id, "expectedOwner":"other", "expectedGeneration":7}),
            serde_json::json!({"sessionId": target.session_id, "expectedOwner":target.expected_owner,"expectedGeneration":8}),
            serde_json::json!({"sessionId":"other", "expectedOwner":target.expected_owner,"expectedGeneration":7}),
        ] {
            assert_eq!(
                slot.discard(
                    &serde_json::from_value(value).unwrap(),
                    &pending.input_id,
                    now
                )
                .unwrap_err(),
                IDENTITY_CHANGED
            );
        }
        assert!(slot.discard(&target, &pending.input_id, now).unwrap());
    }
    #[test]
    fn expiry_and_revocation_release_raw_audio_without_a_persistent_file() {
        let mut slot = Slot::default();
        let target = target();
        let now = Instant::now();
        let pending = slot.put(&target, input(), now);
        slot.expire("another", now + TTL);
        assert!(slot.pending.is_some());
        slot.expire(&pending.input_id, now + TTL);
        assert!(slot.pending.is_none());
        let pending = slot.put(&target, input(), now);
        slot.clear();
        assert!(!slot.discard(&target, &pending.input_id, now).unwrap());
    }
}
