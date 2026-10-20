use super::Failure;
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::{oneshot, watch};

pub(super) struct Entry<P> {
    pub plan: P,
    pub cancel: watch::Sender<bool>,
    pub gate: Arc<Mutex<()>>,
    created: Instant,
    running: bool,
}
pub(super) struct Jobs<P> {
    entries: Mutex<BTreeMap<String, Arc<Mutex<Entry<P>>>>>,
}
impl<P: Clone + Send + 'static> Jobs<P> {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(BTreeMap::new()),
        })
    }
    pub fn reserve(&self, plan: P) -> Result<String, Failure> {
        let mut entries = self.entries.lock().map_err(|_| Failure::Changed)?;
        entries.retain(|_, entry| {
            entry.lock().is_ok_and(|entry| {
                entry.running || entry.created.elapsed() < std::time::Duration::from_secs(60)
            })
        });
        if !entries.is_empty() {
            return Err(Failure::Changed);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (cancel, _) = watch::channel(false);
        entries.insert(
            id.clone(),
            Arc::new(Mutex::new(Entry {
                plan,
                cancel,
                gate: Arc::new(Mutex::new(())),
                created: Instant::now(),
                running: false,
            })),
        );
        Ok(id)
    }
    pub fn cancel(&self, id: &str) -> Result<bool, Failure> {
        let mut entries = self.entries.lock().map_err(|_| Failure::Changed)?;
        let Some(entry) = entries.get(id).cloned() else {
            return Ok(false);
        };
        let entry = entry.lock().map_err(|_| Failure::Changed)?;
        let gate = entry.gate.lock().map_err(|_| Failure::Changed)?;
        entry.cancel.send_replace(true);
        let running = entry.running;
        drop(gate);
        drop(entry);
        if !running {
            entries.remove(id);
        }
        Ok(true)
    }
    pub fn start<T: Send + 'static, F, Fut>(
        self: &Arc<Self>,
        id: &str,
        run: F,
    ) -> Result<oneshot::Receiver<T>, Failure>
    where
        F: FnOnce(P, watch::Receiver<bool>, Arc<Mutex<()>>) -> Fut + Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let (plan, cancel, gate) = {
            let entries = self.entries.lock().map_err(|_| Failure::Changed)?;
            let entry = entries.get(id).ok_or(Failure::Changed)?;
            let mut entry = entry.lock().map_err(|_| Failure::Changed)?;
            if entry.running
                || entry.created.elapsed() >= std::time::Duration::from_secs(60)
                || *entry.cancel.borrow()
            {
                return Err(Failure::Changed);
            }
            entry.running = true;
            (
                entry.plan.clone(),
                entry.cancel.subscribe(),
                entry.gate.clone(),
            )
        };
        let release = Release {
            owner: self.clone(),
            id: id.into(),
        };
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let result = run(plan, cancel, gate).await;
            drop(release);
            let _ = sender.send(result);
        });
        Ok(receiver)
    }
}
struct Release<P> {
    owner: Arc<Jobs<P>>,
    id: String,
}
impl<P> Drop for Release<P> {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.owner.entries.lock() {
            entries.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn expiry_releases_prepared_but_never_a_running_owner() {
        let jobs = Jobs::new();
        let id = jobs.reserve(1).unwrap();
        jobs.entries.lock().unwrap()[&id].lock().unwrap().created =
            Instant::now() - std::time::Duration::from_secs(61);
        assert!(jobs.start(&id, |_, _, _| async {}).is_err());
        let id = jobs.reserve(2).unwrap();
        let (release, finish) = oneshot::channel();
        let result = jobs
            .start(&id, |_, _, _| async move { finish.await.unwrap() })
            .unwrap();
        jobs.entries.lock().unwrap()[&id].lock().unwrap().created =
            Instant::now() - std::time::Duration::from_secs(61);
        assert!(jobs.reserve(3).is_err());
        release.send(()).unwrap();
        result.await.unwrap();
        assert!(jobs.reserve(3).is_ok());
    }
    #[tokio::test]
    async fn cancellation_before_the_first_owner_poll_is_already_visible() {
        let jobs = Jobs::new();
        let id = jobs.reserve(1).unwrap();
        let result = jobs
            .start(&id, |_, cancel, _| async move { assert!(*cancel.borrow()) })
            .unwrap();
        assert!(jobs.cancel(&id).unwrap());
        result.await.unwrap();
    }
    #[tokio::test]
    async fn dropped_ipc_waiter_does_not_release_the_running_owner_or_allow_duplicate_start() {
        let jobs = Jobs::new();
        let id = jobs.reserve("frozen-plan").unwrap();
        let (entered, entry) = oneshot::channel();
        let (release, finish) = oneshot::channel();
        let waiter = jobs
            .start(&id, move |plan, mut cancel, _gate| async move {
                assert_eq!(plan, "frozen-plan");
                entered.send(()).unwrap();
                tokio::select! {_ =finish=>{},_ =cancel.changed()=>{}}
            })
            .unwrap();
        entry.await.unwrap();
        drop(waiter);
        assert!(jobs.reserve("competing-write").is_err());
        assert!(jobs.start(&id, |_, _, _| async {}).is_err());
        release.send(()).unwrap();
        for _ in 0..20 {
            tokio::task::yield_now().await;
            if jobs.entries.lock().unwrap().is_empty() {
                break;
            }
        }
        assert!(jobs.reserve("next-write").is_ok());
    }
    #[tokio::test]
    async fn prepared_cancel_prevents_start_but_running_cancel_remains_reserved_until_cleanup() {
        let jobs = Jobs::new();
        let id = jobs.reserve(1).unwrap();
        assert!(jobs.cancel(&id).unwrap());
        assert!(jobs.start(&id, |_, _, _| async {}).is_err());
        let id = jobs.reserve(2).unwrap();
        let (entered, entry) = oneshot::channel();
        let (release, finish) = oneshot::channel();
        let result = jobs
            .start(&id, move |_, mut cancel, _gate| async move {
                entered.send(()).unwrap();
                cancel.changed().await.unwrap();
                assert!(*cancel.borrow());
                finish.await.unwrap();
                "cleaned"
            })
            .unwrap();
        entry.await.unwrap();
        assert!(jobs.cancel(&id).unwrap());
        assert!(jobs.reserve(3).is_err());
        release.send(()).unwrap();
        assert_eq!(result.await.unwrap(), "cleaned");
        assert!(jobs.reserve(3).is_ok());
    }
}
