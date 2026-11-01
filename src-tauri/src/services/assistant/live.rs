//! Inputs sent to an existing chat sidecar. The original run owns billing;
//! each admitted input has a ledger row and a persisted user message.
use super::{
    host::{blocking, scrub_secrets, AssistantHost, RunControl, STATE},
    types::*,
};
use serde_json::{json, Value};
use std::sync::{Arc, PoisonError};
use tokio::sync::oneshot;

pub(super) struct LiveCommand {
    pub request: SteerRequest,
    pub reply: oneshot::Sender<Result<(), String>>,
}

impl AssistantHost {
    /// Persist before crossing the pipe. A duplicate id never reaches Pi twice.
    pub(super) async fn admit_live_input(
        &self,
        run: &Arc<RunControl>,
        request: &SteerRequest,
    ) -> Result<bool, String> {
        self.validate_run(run)?;
        if run.compact_only {
            return Err("assistant_session_busy".into());
        }
        let text = scrub_secrets(&request.text, &self.known_keys());
        if text.trim().is_empty() {
            return Err("assistant_invalid_message".into());
        }
        let store = self.store(&run.identity.owner)?;
        let request = request.clone();
        let run = run.clone();
        let registry = run.clone();
        let input_id = request.id.clone();
        let accepted = blocking(move || store.mutate(&run.session_id, |record| {
                if run.cancelled() {return Err("assistant_cancelled".into());}
            if !record.session.active_run.as_ref().is_some_and(|active| active.run_id == run.run_id && active.operation_id == run.operation_id) {
                return Err("assistant_session_busy".into());
            }
            if let Some(op) = record.operations.iter().find(|op| op.id == request.id) {
                return if op.run_id == request.id && run.owns_native_run(&request.id) && op.status != OperationStatus::Failed && record.messages.iter().any(|m|m.id == request.id && m.content == text) {
                    Ok(false)
                } else { Err("assistant_stale_operation".into()) };
            }
            if record.messages.iter().any(|m|m.id==request.id) || run.live_runs.lock().unwrap_or_else(PoisonError::into_inner).len() >= 100 {
                return Err("assistant_session_busy".into());
            }
            let price = record.operations.iter().find(|op|op.id == run.operation_id).and_then(|op|op.search_price.clone());
            record.operations.push(Operation {id:request.id.clone(),run_id:request.id.clone(),status:OperationStatus::Running,search_price:price,media:None});
            record.messages.push(serde_json::from_value(json!({
                "id":request.id,"role":"user","content":text,"createdAt":now(),"status":"queued","runId":request.id
            })).map_err(|_|STATE)?);
            Ok(true)
        })).await?;
        if accepted {
            registry
                .live_runs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(input_id);
        }
        Ok(accepted)
    }

    pub(super) async fn refuse_live_input(&self, run: &RunControl, id: &str) -> Result<(), String> {
        let store = self.store(&run.identity.owner)?;
        let sid = run.session_id.clone();
        let id = id.to_owned();
        blocking(move || {
            store.mutate(&sid, |record| {
                if let Some(op) = record.operations.iter_mut().find(|op| op.id == id) {
                    op.status = OperationStatus::Failed;
                }
                if let Some(message) = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == id && m.native_entry_id.is_none())
                {
                    message.status = "error".into();
                    message.error_code = Some("assistant_session_busy".into());
                }
                Ok(())
            })
        })
        .await
    }

    /// The current response finishes before Pi consumes the steering input.
    /// The pipe envelope stays on the original run; only its visible reply moves.
    pub(super) async fn apply_live_input(
        &self,
        run: &RunControl,
        data: &Value,
    ) -> Result<Value, String> {
        let id = data["id"].as_str().ok_or(STATE)?.to_owned();
        let store = self.store(&run.identity.owner)?;
        let sid = run.session_id.clone();
        let rid = id.clone();
        let previous_entry = data["previousEntryId"].as_str().ok_or(STATE)?.to_owned();
        let previous = run.reply_id();
        let mid = super::types::id();
        let reply_id = mid.clone();
        let shown=blocking(move || store.mutate(&sid, |record| {
            let user=record.messages.iter_mut().find(|m|m.id==id && m.run_id.as_ref()==Some(&rid)).ok_or(STATE)?;
            if user.status != "queued" { return Err(STATE.into()); }
            user.status="completed".into();
            let user=user.clone();
            let old=record.messages.iter_mut().find(|m|m.id==previous).ok_or(STATE)?;
            old.native_entry_id=Some(previous_entry);
            old.status="completed".into(); old.completed_at=Some(now());
            let old=old.clone();
            let next:Message=serde_json::from_value(json!({"id":mid,"role":"assistant","content":"","createdAt":now(),"status":"streaming","runId":rid,"modelId":record.session.model_id})).map_err(|_|STATE)?;
            record.messages.push(next.clone());
            Ok(json!({"messageId":mid,"messages":[old,user,next]}))
        })).await?;
        *run.live_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(reply_id);
        Ok(shown)
    }
}
