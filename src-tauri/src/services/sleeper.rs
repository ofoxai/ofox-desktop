//! Async clock abstraction so background loops can be tested without real
//! wall time.
//!
//! Production code passes [`TokioSleeper`]; tests pass `MockSleeper` (under
//! `#[cfg(test)]`). The sleeper is taken as a generic `<S: Sleeper>` so we
//! don't need `dyn`-compat or the `async-trait` crate.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub trait Sleeper: Send + Sync + 'static {
    /// Returns a future that resolves after `dur` of (real or virtual) time.
    fn sleep(&self, dur: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Production sleeper: real wall-clock delays via Tokio.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioSleeper;

impl Sleeper for TokioSleeper {
    fn sleep(&self, dur: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(dur))
    }
}

#[cfg(test)]
pub mod mock {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::{Mutex, Notify};

    use super::Sleeper;
    use std::future::Future;
    use std::pin::Pin;

    /// In-memory fake sleeper. Tests `advance()` virtual time; pending
    /// `sleep()` calls whose target has been reached resolve.
    pub struct MockSleeper {
        inner: Arc<MockState>,
    }

    struct MockState {
        elapsed: Mutex<Duration>,
        notify: Notify,
        history: Mutex<Vec<Duration>>,
    }

    impl Default for MockSleeper {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockSleeper {
        pub fn new() -> Self {
            Self {
                inner: Arc::new(MockState {
                    elapsed: Mutex::new(Duration::ZERO),
                    notify: Notify::new(),
                    history: Mutex::new(Vec::new()),
                }),
            }
        }

        pub fn handle(&self) -> MockSleeper {
            MockSleeper {
                inner: self.inner.clone(),
            }
        }

        pub async fn advance(&self, dur: Duration) {
            {
                let mut elapsed = self.inner.elapsed.lock().await;
                *elapsed += dur;
            }
            self.inner.notify.notify_waiters();
            tokio::task::yield_now().await;
        }

        pub async fn history(&self) -> Vec<Duration> {
            self.inner.history.lock().await.clone()
        }
    }

    impl Sleeper for MockSleeper {
        fn sleep(&self, dur: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            let inner = self.inner.clone();
            Box::pin(async move {
                inner.history.lock().await.push(dur);
                let target = {
                    let cur = *inner.elapsed.lock().await;
                    cur + dur
                };
                loop {
                    let cur = *inner.elapsed.lock().await;
                    if cur >= target {
                        return;
                    }
                    inner.notify.notified().await;
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockSleeper;
    use super::Sleeper;
    use std::time::Duration;

    #[tokio::test]
    async fn mock_advance_releases_pending_sleep() {
        let sleeper = MockSleeper::new();
        let h = sleeper.handle();
        let task = tokio::spawn(async move { h.sleep(Duration::from_secs(60)).await });
        tokio::task::yield_now().await;
        sleeper.advance(Duration::from_secs(60)).await;
        task.await.unwrap();
    }

    #[tokio::test]
    async fn mock_partial_advance_keeps_sleep_pending() {
        let sleeper = MockSleeper::new();
        let h = sleeper.handle();
        let task = tokio::spawn(async move { h.sleep(Duration::from_secs(60)).await });
        tokio::task::yield_now().await;
        sleeper.advance(Duration::from_secs(30)).await;
        // Should still be pending — race against a small real timeout.
        let res = tokio::time::timeout(Duration::from_millis(20), task).await;
        assert!(res.is_err(), "task should still be pending");
    }

    #[tokio::test]
    async fn mock_history_records_each_sleep() {
        let sleeper = MockSleeper::new();
        let h = sleeper.handle();
        let h2 = sleeper.handle();
        let t1 = tokio::spawn(async move { h.sleep(Duration::from_secs(5)).await });
        let t2 = tokio::spawn(async move { h2.sleep(Duration::from_secs(10)).await });
        tokio::task::yield_now().await;
        sleeper.advance(Duration::from_secs(10)).await;
        let _ = tokio::join!(t1, t2);
        let hist = sleeper.history().await;
        assert_eq!(hist.len(), 2);
        assert!(hist.contains(&Duration::from_secs(5)));
        assert!(hist.contains(&Duration::from_secs(10)));
    }
}
