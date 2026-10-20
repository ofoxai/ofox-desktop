//! Short synchronous configuration transactions. Never hold a guard across await.
use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    sync::{Mutex, MutexGuard},
};

static IO: Mutex<()> = Mutex::new(());
struct ThreadState {
    depth: usize,
    guard: Option<MutexGuard<'static, ()>>,
}
thread_local! { static STATE: RefCell<ThreadState> = const { RefCell::new(ThreadState { depth: 0, guard: None }) }; }

pub(crate) struct IoGuard {
    _not_send: PhantomData<Rc<()>>,
}

/// A thread retains the mutex until its last lease drops, including non-LIFO drops.
pub(crate) fn lock() -> IoGuard {
    let outer = STATE.with(|state| state.borrow().depth == 0);
    let guard = outer.then(|| IO.lock().unwrap_or_else(|error| error.into_inner()));
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if outer {
            state.guard = guard;
        }
        state.depth += 1;
    });
    IoGuard {
        _not_send: PhantomData,
    }
}

impl Drop for IoGuard {
    fn drop(&mut self) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.depth -= 1;
            if state.depth == 0 {
                state.guard.take();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[serial_test::serial]
    fn native_writer_waits_for_resource_transaction() {
        let _sandbox = crate::test_sandbox::sandbox();
        let guard = lock();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            crate::opencode_config::write_opencode_config(&serde_json::json!({"model":"personal"}))
                .unwrap();
            done_tx.send(()).unwrap();
        });
        started_rx.recv().unwrap();
        let completed_early = done_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_ok();
        drop(guard);
        worker.join().unwrap();
        assert!(
            !completed_early,
            "native writer bypassed shared resource barrier"
        );
    }

    #[test]
    fn an_outer_guard_dropped_early_does_not_release_an_inner_guard() {
        let outer = lock();
        let inner = lock();
        drop(outer);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let _guard = lock();
            tx.send(()).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(30))
            .is_err());
        drop(inner);
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
    }
    #[test]
    fn nested_writer_is_reentrant_but_other_threads_wait() {
        let outer = lock();
        {
            let _inner = lock();
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _guard = lock();
            tx.send(()).unwrap();
        });
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(30))
            .is_err());
        drop(outer);
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
    }
}
