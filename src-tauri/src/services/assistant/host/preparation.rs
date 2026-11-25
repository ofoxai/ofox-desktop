//! A cancellable send waiter, separate from the shared native document proof.
use super::*;

pub(in crate::services::assistant) const PREPARATION_TIMEOUT: &str =
    "assistant_document_preparation_timeout";
// A cold native proof includes several serial engine/negative checks. A short
// model connect timeout would reject legitimate first launches.
const WAIT_LIMIT: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    owner: String,
    generation: u64,
    session: String,
    run: String,
    operation: String,
}

#[derive(Default)]
pub(super) struct PendingPreparations(Mutex<HashMap<Key, watch::Sender<bool>>>);

impl PendingPreparations {
    pub(super) fn cancel(&self, identity: &Identity, sid: &str, rid: &str, op: &str) {
        for (key, sender) in self.0.lock().unwrap_or_else(PoisonError::into_inner).iter() {
            if key.owner == identity.owner
                && key.session == sid
                && key.run == rid
                && key.operation == op
            {
                sender.send_replace(true);
            }
        }
    }

    pub(super) fn cancel_session(&self, sid: &str) {
        for (key, sender) in self.0.lock().unwrap_or_else(PoisonError::into_inner).iter() {
            if key.session == sid {
                sender.send_replace(true);
            }
        }
    }
}

pub(in crate::services::assistant) struct PreparationGuard {
    host: Arc<AssistantHost>,
    key: Key,
    cancelled: watch::Receiver<bool>,
}

impl PreparationGuard {
    /// Checked under the runs lock when the prepared send is registered.
    pub(in crate::services::assistant) fn check(&self) -> Result<(), String> {
        if *self.cancelled.borrow() {
            Err(CANCELLED.into())
        } else {
            Ok(())
        }
    }
}

impl Drop for PreparationGuard {
    fn drop(&mut self) {
        self.host
            .preparation_sends
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.key);
    }
}

impl AssistantHost {
    pub(in crate::services::assistant) async fn prepare_document_send(
        self: &Arc<Self>,
        identity: &Identity,
        generation: u64,
        request: &SendRequest,
    ) -> Result<Option<PreparationGuard>, String> {
        self.prepare_document_send_until(
            identity,
            generation,
            request,
            tokio::time::Instant::now() + WAIT_LIMIT,
        )
        .await
    }

    pub(in crate::services::assistant) async fn prepare_document_send_until(
        self: &Arc<Self>,
        identity: &Identity,
        generation: u64,
        request: &SendRequest,
        deadline: tokio::time::Instant,
    ) -> Result<Option<PreparationGuard>, String> {
        let mut authority = self.video_revocation.subscribe();
        self.ensure_current(identity, generation).await?;
        self.start_needed_document_preparation();
        // macOS and a warm Windows runtime have nothing to wait for. Avoid
        // extra history reads or a misleading preparation event in that case.
        if self.document_preparation.load(Ordering::SeqCst) != 1 {
            return Ok(None);
        }
        let key = Key {
            owner: identity.owner.clone(),
            generation,
            session: request.session_id.clone(),
            run: request.run_id.clone(),
            operation: request.operation_id.clone(),
        };
        let mut waiter = blocking({
            let host = self.clone();
            let key = key.clone();
            move || {
                // Stop uses this same lock through its durable receipt. It
                // either reaches this channel or is seen by the initial read.
                let runs = host.runs.lock().map_err(|_| STATE)?;
                host.admit(generation)?;
                if host.document_preparation.load(Ordering::SeqCst) != 1 {
                    return Ok(None);
                }
                if runs.contains_key(&key.session) {
                    return Err(SESSION_BUSY.into());
                }
                check_operation(&host, &key)?;
                let mut pending = host.preparation_sends.0.lock().map_err(|_| STATE)?;
                let std::collections::hash_map::Entry::Vacant(entry) = pending.entry(key.clone())
                else {
                    return Err(SESSION_BUSY.into());
                };
                let (sender, cancelled) = watch::channel(false);
                entry.insert(sender);
                drop(pending);
                drop(runs);
                Ok(Some(PreparationGuard {
                    host,
                    key,
                    cancelled,
                }))
            }
        })
        .await?;
        let Some(mut waiter) = waiter.take() else {
            return Ok(None);
        };
        let publish = |active| {
            if self.generation() == generation {
                self.env.emit(Event {
                    session_id: key.session.clone(),
                    run_id: key.run.clone(),
                    operation_id: key.operation.clone(),
                    r#type: "run_preparing".into(),
                    data: json!({"stage":"documents", "active":active,
                        "owner":identity.owner, "generation":generation}),
                });
            }
        };
        let result = async {
            self.ensure_current(identity, generation).await?;
            if *waiter.cancelled.borrow() {
                return Err(CANCELLED.into());
            }
            publish(true);
            let result: Result<(), String> = tokio::select! {
                biased;
                _ = authority.wait_for(|current| *current != generation) => Err(IDENTITY_CHANGED.into()),
                _ = waiter.cancelled.wait_for(|cancelled| *cancelled) => Err(CANCELLED.into()),
                _ = tokio::time::sleep_until(deadline) => Err(PREPARATION_TIMEOUT.into()),
                _ = self.await_document_preparation() => Ok(()),
            };
            self.ensure_current(identity, generation).await?;
            if *waiter.cancelled.borrow() {
                return Err(CANCELLED.into());
            }
            result?;
            blocking({
                let host = self.clone();
                let key = key.clone();
                move || check_operation(&host, &key)
            })
            .await
        }
        .await;
        publish(false);
        result?;
        waiter.check()?;
        // Keep cancellation live through the final run-registration fence.
        Ok(Some(waiter))
    }
}

fn check_operation(host: &AssistantHost, key: &Key) -> Result<(), String> {
    host.admit(key.generation)?;
    let record = host.store(&key.owner)?.load_record(&key.session)?;
    if record.session.deleted_at.is_some() {
        return Err("assistant_session_deleted".into());
    }
    super::super::store::ensure_new_operation(&record, &key.operation)
}
