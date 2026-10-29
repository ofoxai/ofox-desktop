//! Context estimates and explicitly requested, billed native compaction.
use super::{
    catalog,
    host::{
        blocking, local_gateway_allowed, session_key, AssistantHost, RunControl, IDENTITY_CHANGED,
        STATE,
    },
    requests::run_thinking_level,
    store,
    types::*,
};
use serde_json::{json, Value};
use std::sync::Arc;

impl AssistantHost {
    pub(super) async fn save_context_usage(
        &self,
        control: &RunControl,
        value: &Value,
    ) -> Result<(), String> {
        let store = self.store(&control.identity.owner)?;
        let sid = control.session_id.clone();
        let rid = control.run_id.clone();
        let usage = ContextUsage::parse(value);
        blocking(move || {
            store.mutate(&sid, |record| {
                if record
                    .session
                    .active_run
                    .as_ref()
                    .is_some_and(|run| run.run_id == rid)
                {
                    record.session.context_usage =
                        usage.filter(|usage| usage.model_id == record.session.model_id);
                }
                Ok(())
            })
        })
        .await
    }

    /// No message is fabricated: only an operation and active-run reference
    /// are published. Admission and native-history exclusion match version changes.
    pub async fn compact_session(
        self: &Arc<Self>,
        request: CompactRequest,
    ) -> Result<RunRef, String> {
        let sid = &request.authority.session_id;
        for value in [sid, &request.run_id, &request.operation_id] {
            validate_id(value)?;
        }
        self.ensure_accepting_runs()?;
        let identity = self.identity().await?;
        let generation = self.generation();
        if request.authority.expected_owner != identity.owner
            || request.authority.expected_generation != generation
        {
            return Err(IDENTITY_CHANGED.into());
        }
        self.ensure_idle(sid)?;
        self.recover(&identity, sid).await?;
        self.repair_media_history(&identity, sid).await?;
        let (record, native) = blocking({
            let store = self.store(&identity.owner)?;
            let sid = sid.clone();
            move || Ok((store.load_record(&sid)?, store.native_history_path(&sid)?))
        })
        .await?;
        if record.session.deleted_at.is_some() {
            return Err("assistant_session_deleted".into());
        }
        if !native.is_file() || !record.messages.iter().any(|m| !m.superseded) {
            return Err("assistant_compact_unavailable".into());
        }
        let model = self.model(&identity.apex, &record.session.model_id).await?;
        if model.kind != catalog::Kind::Chat {
            return Err("assistant_compact_unavailable".into());
        }
        self.runtime()?;
        let base = self.env.gateway_base(&identity.apex)?;
        let proxy = blocking({
            let env = self.env.clone();
            move || Ok(env.proxy())
        })
        .await?;
        self.ensure_current(&identity, generation).await?;
        let admit = || self.admit(generation);
        let key = self.env.api_key(&identity, None, &admit).await?;
        self.ensure_current(&identity, generation).await?;
        self.remember_key(&key);
        let send:SendRequest=serde_json::from_value(json!({"sessionId":sid,"runId":request.run_id,"operationId":request.operation_id,"text":""})).map_err(|_|STATE)?;
        let mut control = RunControl::new(identity.clone(), &send, key.clone());
        control.compact_only = true;
        let control = Arc::new(control);
        let history = self
            .native_lock(&session_key(&identity.owner, sid))?
            .lock_owned()
            .await;
        let navigation = self.begin_navigation(sid)?;
        self.ensure_current(&identity, generation).await?;
        let model_id = model.id.clone();
        blocking({
            let store = self.store(&identity.owner)?;
            let host = self.clone();
            let control = control.clone();
            let sid = sid.clone();
            move || {
                // The history lock is still held. A competing admission in this
                // small handover makes register_run refuse us before any write.
                drop(navigation);
                host.register_run(control.clone(), generation, || {
                    store.mutate(&sid, |r| {
                        if r.session.deleted_at.is_some() {
                            return Err("assistant_session_deleted".into());
                        }
                        if r.session.model_id != model_id || r.session.active_run.is_some() {
                            return Err("assistant_session_busy".into());
                        }
                        store::ensure_new_operation(r, &control.operation_id)?;
                        if r.operations.iter().any(|op| op.run_id == control.run_id)
                            || r.messages
                                .iter()
                                .any(|m| m.run_id.as_deref() == Some(&control.run_id))
                        {
                            return Err("assistant_stale_operation".into());
                        }
                        r.operations.push(Operation {
                            id: control.operation_id.clone(),
                            run_id: control.run_id.clone(),
                            status: OperationStatus::Running,
                            media: None,
                            search_price: None,
                        });
                        r.session.active_run = Some(RunRef {
                            run_id: control.run_id.clone(),
                            operation_id: control.operation_id.clone(),
                        });
                        r.session.context_usage = None;
                        r.session.updated_at = now();
                        Ok(())
                    })
                })
            }
        })
        .await?;
        let thinking = run_thinking_level(&record.session, &model);
        let mut params = json!({"sessionId":sid,"runId":request.run_id,"operationId":request.operation_id,"sessionFile":native,
            "model":model.for_runtime(thinking.is_some()),"apiKey":key,"apex":identity.apex,"baseUrl":base,"allowLocalGateway":local_gateway_allowed(&base),"proxy":proxy,"prompt":""});
        let object = params.as_object_mut().ok_or(STATE)?;
        if let Some(level) = thinking {
            object.insert("thinkingLevel".into(), json!(level));
        }
        self.emit_start(&control, false, thinking);
        self.spawn_run(control, params);
        drop(history);
        Ok(RunRef {
            run_id: request.run_id,
            operation_id: request.operation_id,
        })
    }
}
