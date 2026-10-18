//! Progress for a local, cancellable cache rebuild. No transcript or path leaves here.
use crate::error::AppError;
use serde::Serialize;
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildProgress {
    pub operation_id: String,
    pub state: String,
    pub completed_sources: usize,
    pub total_sources: usize,
    pub error_code: Option<String>,
}
pub(crate) struct Control {
    progress: Mutex<RebuildProgress>,
}
static CURRENT: LazyLock<Mutex<Option<Arc<Control>>>> = LazyLock::new(|| Mutex::new(None));
impl Control {
    pub(crate) fn request_cancel(&self) {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if progress.state == "running" {
            progress.state = "cancelling".into();
        }
    }
    pub(crate) fn new(total: usize) -> Arc<Self> {
        Arc::new(Self {
            progress: Mutex::new(RebuildProgress {
                operation_id: uuid::Uuid::new_v4().to_string(),
                state: "running".into(),
                completed_sources: 0,
                total_sources: total,
                error_code: None,
            }),
        })
    }
    pub(crate) fn check(&self) -> Result<(), AppError> {
        if matches!(
            self.progress
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .state
                .as_str(),
            "cancelling" | "cancelled"
        ) {
            Err(AppError::Message("usage.rebuild.cancelled".into()))
        } else {
            Ok(())
        }
    }
    pub(crate) fn advance(&self) {
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        p.completed_sources = p.completed_sources.saturating_add(1).min(p.total_sources);
    }
    pub(crate) fn finish(&self, result: &Result<(), AppError>) {
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(p.state.as_str(), "completed" | "failed" | "cancelled") {
            return;
        }
        p.state = if p.state == "cancelling" {
            "cancelled"
        } else if result.is_ok() {
            "completed"
        } else {
            "failed"
        }
        .into();
        p.error_code = (p.state == "failed").then(|| "usage.rebuild.retry".into());
    }
    pub(crate) fn publish(
        &self,
        publish: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        // Serialize cancel acknowledgement with the actual commit. A cancel
        // that arrives after publication receives completed, never cancelled.
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if p.state == "cancelling" {
            return Err(AppError::Message("usage.rebuild.cancelled".into()));
        }
        let result = publish();
        p.state = if result.is_ok() {
            "completed"
        } else {
            "failed"
        }
        .into();
        p.error_code = result.as_ref().err().map(|_| "usage.rebuild.retry".into());
        result
    }
}
pub(crate) fn begin(total: usize) -> Arc<Control> {
    let control = Control::new(total);
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&control));
    control
}
pub fn progress() -> Option<RebuildProgress> {
    CURRENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|c| c.progress.lock().unwrap_or_else(|e| e.into_inner()).clone())
}
pub fn cancel(operation_id: &str) -> Result<RebuildProgress, AppError> {
    let current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    let control = current
        .as_ref()
        .ok_or_else(|| AppError::InvalidInput("usage.rebuild.expired".into()))?;
    let p = control.progress.lock().unwrap_or_else(|e| e.into_inner());
    if p.operation_id != operation_id {
        return Err(AppError::InvalidInput("usage.rebuild.expired".into()));
    }
    drop(p);
    control.request_cancel();
    let result = control
        .progress
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn native_rebuild_cancel_before_publication_never_calls_the_writer() {
        let control = Control::new(1);
        let wrote = AtomicBool::new(false);
        control.request_cancel();
        let result = control.publish(|| {
            wrote.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(result.is_err());
        assert!(!wrote.load(Ordering::SeqCst));
        control.finish(&result);
        assert_eq!(control.progress.lock().unwrap().state, "cancelled");
    }

    #[test]
    fn native_rebuild_cancel_after_publication_keeps_the_completed_receipt() {
        let control = Control::new(1);
        control.publish(|| Ok(())).unwrap();
        control.request_cancel();
        control.finish(&Err(AppError::Message("late interruption".into())));
        assert_eq!(control.progress.lock().unwrap().state, "completed");
    }
}
