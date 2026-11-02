//! In-memory confirmations, bound to the UI authority and the exact run.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::Duration,
};
use tokio::sync::{oneshot, watch};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub owner: String,
    pub session_id: String,
    pub run_id: String,
    pub operation_id: String,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Choice {
    Once,
    Session,
    Deny,
}

struct Pending {
    binding: Binding,
    sender: oneshot::Sender<Choice>,
}
#[derive(Default)]
pub struct Broker {
    pending: Mutex<HashMap<String, Pending>>,
}

impl Broker {
    pub fn request(&self, binding: Binding) -> Result<(String, oneshot::Receiver<Choice>), String> {
        let id = super::super::types::id();
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| "assistant_state_unavailable")?
            .insert(id.clone(), Pending { binding, sender });
        Ok((id, receiver))
    }

    pub fn respond(&self, id: &str, binding: &Binding, choice: Choice) -> Result<(), String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        if pending.get(id).is_none_or(|p| p.binding != *binding) {
            return Err("assistant_approval_expired".into());
        }
        pending
            .remove(id)
            .ok_or("assistant_approval_expired")?
            .sender
            .send(choice)
            .map_err(|_| "assistant_approval_expired".into())
    }

    pub async fn wait(
        &self,
        id: &str,
        receiver: oneshot::Receiver<Choice>,
        mut cancel: watch::Receiver<bool>,
        timeout: Duration,
    ) -> Result<Choice, String> {
        // Cleanup also runs if the host future is aborted during shutdown.
        struct Cleanup<'a>(&'a Broker, &'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                self.0
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(self.1);
            }
        }
        let _cleanup = Cleanup(self, id);
        if *cancel.borrow() {
            return Err("assistant_cancelled".into());
        }
        tokio::select! {
            biased;
            _ = cancel.changed() => Err("assistant_cancelled".into()),
            result = receiver => result.map_err(|_| "assistant_approval_expired".into()),
            _ = tokio::time::sleep(timeout) => Err("assistant_approval_timeout".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_approval_is_once_bound_and_cancellable() {
        let broker = Broker::default();
        let (cancel, _) = tokio::sync::watch::channel(false);
        let binding = Binding {
            owner: "owner".into(),
            session_id: "session".into(),
            run_id: "run".into(),
            operation_id: "op".into(),
            generation: 1,
        };
        let (id, pending) = broker.request(binding.clone()).unwrap();
        let mut stale = binding.clone();
        stale.generation += 1;
        assert!(broker.respond(&id, &stale, Choice::Once).is_err());
        broker.respond(&id, &binding, Choice::Once).unwrap();
        assert!(broker.respond(&id, &binding, Choice::Once).is_err());
        assert_eq!(
            broker
                .wait(
                    &id,
                    pending,
                    cancel.subscribe(),
                    std::time::Duration::from_millis(50)
                )
                .await
                .unwrap(),
            Choice::Once
        );
        let (id, pending) = broker.request(binding.clone()).unwrap();
        assert_eq!(
            broker
                .wait(
                    &id,
                    pending,
                    cancel.subscribe(),
                    std::time::Duration::from_millis(1)
                )
                .await
                .unwrap_err(),
            "assistant_approval_timeout"
        );
        assert!(broker.respond(&id, &binding, Choice::Once).is_err());
        let (id, pending) = broker.request(binding.clone()).unwrap();
        cancel.send_replace(true);
        assert_eq!(
            broker
                .wait(
                    &id,
                    pending,
                    cancel.subscribe(),
                    std::time::Duration::from_secs(1)
                )
                .await
                .unwrap_err(),
            "assistant_cancelled"
        );
        assert!(broker.respond(&id, &binding, Choice::Once).is_err());
        let (id, pending) = broker.request(binding.clone()).unwrap();
        cancel.send_replace(false);
        broker.respond(&id, &binding, Choice::Deny).unwrap();
        assert_eq!(
            broker
                .wait(
                    &id,
                    pending,
                    cancel.subscribe(),
                    std::time::Duration::from_millis(50)
                )
                .await
                .unwrap(),
            Choice::Deny
        );
    }
}
