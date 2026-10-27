//! What each `assistant_*` command does. The commands only find the host and
//! call these, so tests drive exactly what the renderer drives.
use super::{
    catalog,
    host::{blocking, local_gateway_allowed, AssistantHost, Identity, RunControl},
    store::{self, SessionList, SessionRecord},
    types::*,
};
use serde_json::{json, Value};
use std::sync::Arc;

const MAX_MESSAGE_BYTES: usize = 128_000;
const TITLE_CHARS: usize = 48;

impl AssistantHost {
    /// Everything the assistant page needs to open. Never mints a key.
    pub async fn bootstrap(&self) -> Result<Value, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let (models, model_error) = match self.models(&identity.apex).await {
            Ok(models) => (models, None),
            Err(error) => (vec![], Some(error)),
        };
        // A damaged preferences file only loses the remembered choice.
        let remembered = blocking({
            let store = self.store(&identity.owner)?;
            move || store.remembered_model()
        })
        .await
        .unwrap_or_default();
        let default_model_id = catalog::default_model_id(&models, remembered.as_deref());
        let runtime = match self.runtime() {
            Ok(runtime) => json!({"available": true, "reason": null, "version": runtime.version}),
            Err(reason) => json!({"available": false, "reason": reason, "version": null}),
        };
        let key = if self.env.key_ready(&identity).await {
            "ready"
        } else {
            "missing"
        };
        self.ensure_current(&identity, generation).await?;
        Ok(json!({
            "owner": identity.owner,
            "authorityGeneration": generation,
            "runtime": runtime,
            "models": models,
            "defaultModelId": default_model_id,
            "modelUnavailableReason": model_error,
            "key": {"status": key},
        }))
    }

    pub async fn list_sessions(&self, deleted: bool, search: &str) -> Result<SessionList, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let mut list = blocking({
            let store = self.store(&identity.owner)?;
            let search = search.to_owned();
            move || store.list(deleted, &search)
        })
        .await?;
        self.ensure_current(&identity, generation).await?;
        for session in &mut list.sessions {
            self.decorate(&identity.owner, session);
        }
        Ok(list)
    }

    pub async fn create_session(&self, request: CreateRequest) -> Result<Session, String> {
        let identity = self.identity().await?;
        let generation = self.generation();
        let store = self.store(&identity.owner)?;
        let selected = match request.model_id {
            Some(model) => self.model(&identity.apex, &model).await?.id,
            None => {
                let models = self.models(&identity.apex).await?;
                let remembered = blocking({
                    let store = self.store(&identity.owner)?;
                    move || store.remembered_model()
                })
                .await
                .unwrap_or_default();
                catalog::default_model_id(&models, remembered.as_deref())
                    .ok_or("assistant_choose_model")?
            }
        };
        self.ensure_current(&identity, generation).await?;
        let detail = blocking(move || {
            let detail = store.create(&selected, request.web_search)?;
            store.remember_model(&selected)?;
            Ok(detail)
        })
        .await?;
        Ok(detail.session)
    }

    /// One conversation. A run the previous process left behind is settled
    /// first; if that fails the saved state is shown as it is.
    pub async fn get_session(&self, sid: &str) -> Result<Detail, String> {
        validate_id(sid)?;
        let identity = self.identity().await?;
        let generation = self.generation();
        if let Err(error) = self.recover(&identity, sid).await {
            log::warn!("[Assistant] could not recover session {sid}: {error}");
        }
        let mut detail = blocking({
            let store = self.store(&identity.owner)?;
            let sid = sid.to_owned();
            move || store.load_detail(&sid)
        })
        .await?;
        self.ensure_current(&identity, generation).await?;
        self.decorate(&identity.owner, &mut detail.session);
        if detail.session.active_run.is_none() {
            for message in &mut detail.messages {
                if message.status == "streaming" {
                    message.status = "stopped".into();
                }
            }
        }
        Ok(detail)
    }

    /// Title, model, draft, web search and trash state. Changing the model or
    /// trashing the session stops its run, once the change is saved.
    pub async fn update_session(
        self: &Arc<Self>,
        mut request: UpdateRequest,
    ) -> Result<Session, String> {
        validate_id(&request.session_id)?;
        let identity = self.identity().await?;
        let generation = self.generation();
        if request
            .expected_owner
            .as_ref()
            .is_some_and(|owner| *owner != identity.owner)
            || request
                .expected_generation
                .is_some_and(|expected| expected != generation)
        {
            return Err(super::host::IDENTITY_CHANGED.into());
        }
        if let Some(model) = &request.model_id {
            self.model(&identity.apex, model).await?;
        }
        // The user's own words: only a key this process handed out goes.
        let keys = self.known_keys();
        for text in [&mut request.title, &mut request.draft]
            .into_iter()
            .flatten()
        {
            *text = super::host::scrub_secrets(text, &keys);
        }
        let stops_run = request.model_id.is_some() || request.deleted == Some(true);
        let sid = request.session_id.clone();
        self.ensure_current(&identity, generation).await?;
        let (mut session, remembered) = blocking({
            let store = self.store(&identity.owner)?;
            let host = self.clone();
            move || {
                let session = store.update(&request, || {
                    if host.generation() != generation {
                        return Err(super::host::IDENTITY_CHANGED.into());
                    }
                    Ok(())
                })?;
                let remembered = match &request.model_id {
                    Some(_) => store.remember_model(&session.model_id),
                    None => Ok(()),
                };
                Ok((session, remembered))
            }
        })
        .await?;
        // The change is saved: the old model's run stops even if remembering
        // the choice failed.
        if stops_run {
            self.cancel_session(&sid);
        }
        remembered?;
        self.decorate(&identity.owner, &mut session);
        Ok(session)
    }

    /// Send a message, or edit / regenerate an earlier turn. Returns once the
    /// run is registered; the reply arrives as `assistant:event`s.
    pub async fn send(self: &Arc<Self>, request: SendRequest) -> Result<RunRef, String> {
        validate_id(&request.session_id)?;
        validate_id(&request.run_id)?;
        validate_id(&request.operation_id)?;
        if request.text.len() > MAX_MESSAGE_BYTES {
            return Err("assistant_message_too_large".into());
        }
        if (request.text.trim().is_empty() && request.regenerate_message_id.is_none())
            || (request.edit_message_id.is_some() && request.regenerate_message_id.is_some())
        {
            return Err("assistant_invalid_message".into());
        }
        // Attachments arrive in a later stage; none can be sent yet.
        if !request.attachment_ids.is_empty() {
            return Err("assistant_attachment_not_found".into());
        }
        self.ensure_accepting_runs()?;
        let generation = self.generation();
        let identity = self.identity().await?;
        self.recover(&identity, &request.session_id).await?;
        let (record, native_file) = blocking({
            let store = self.store(&identity.owner)?;
            let sid = request.session_id.clone();
            move || Ok((store.load_record(&sid)?, store.native_history_path(&sid)?))
        })
        .await?;
        if record.session.deleted_at.is_some() {
            return Err("assistant_session_deleted".into());
        }
        self.runtime()?;
        let model = self.model(&identity.apex, &record.session.model_id).await?;
        // Native history is the conversation's context: once a turn reached
        // the runtime, its history must still be there.
        if !native_file.is_file() && record.messages.iter().any(|m| m.native_entry_id.is_some()) {
            return Err("assistant_native_history_missing".into());
        }
        let branch = Branch::resolve(&record, &request)?;
        let base_url = self.env.gateway_base(&identity.apex)?;
        let proxy = blocking({
            let env = self.env.clone();
            move || Ok(env.proxy())
        })
        .await?;
        // Checked again under the key lock: a logout that revokes keys while
        // this send waits for the lock leaves it with no key.
        self.ensure_current(&identity, generation).await?;
        let admit = move || self.admit(generation);
        let key = self.env.api_key(&identity, None, &admit).await?;
        self.ensure_current(&identity, generation).await?;

        self.remember_key(&key);
        let control = Arc::new(RunControl::new(identity.clone(), &request, key.clone()));
        let text = super::host::scrub_secrets(&request.text, std::slice::from_ref(&key));
        blocking({
            let host = self.clone();
            let store = self.store(&identity.owner)?;
            let control = control.clone();
            let request = request.clone();
            let branch = branch.clone();
            let text = text.clone();
            move || {
                host.register_run(control.clone(), generation, || {
                    store.mutate(&request.session_id, |record| {
                        publish_turn(record, &control, &request, &branch, &text)
                    })
                })
            }
        })
        .await?;

        let params = json!({
            "sessionId": request.session_id,
            "runId": request.run_id,
            "operationId": request.operation_id,
            "sessionFile": native_file,
            "model": model,
            "apiKey": key,
            "apex": identity.apex,
            "baseUrl": base_url,
            "allowLocalGateway": local_gateway_allowed(&base_url),
            "proxy": proxy,
            "prompt": text,
            "editEntryId": branch.edit_entry,
            "regenerateEntryId": branch.regenerate_entry,
        });
        self.emit_start(&control);
        self.spawn_run(control, params);
        Ok(RunRef {
            run_id: request.run_id,
            operation_id: request.operation_id,
        })
    }

    pub async fn stop(self: &Arc<Self>, sid: &str, rid: &str, op: &str) -> Result<(), String> {
        validate_id(sid)?;
        validate_id(rid)?;
        validate_id(op)?;
        let identity: Identity = self.identity().await?;
        let host = self.clone();
        let store = self.store(&identity.owner)?;
        let (sid, rid, op) = (sid.to_owned(), rid.to_owned(), op.to_owned());
        blocking(move || host.request_stop(&store, &identity, &sid, &rid, &op)).await
    }
}

/// Which earlier turn a send rewrites, if any.
#[derive(Clone, Default)]
struct Branch {
    /// The user message an edit or a regenerate replaces from.
    revision_of: Option<String>,
    /// For a regenerate: the user text it sends again.
    resent_text: Option<String>,
    edit_entry: Option<String>,
    regenerate_entry: Option<String>,
}

impl Branch {
    fn resolve(record: &SessionRecord, request: &SendRequest) -> Result<Self, String> {
        const UNAVAILABLE: &str = "assistant_branch_unavailable";
        let find = |id: &str, role: &str| {
            record
                .messages
                .iter()
                .find(|m| m.id == id && m.role == role)
        };
        if let Some(mid) = &request.edit_message_id {
            let user = find(mid, "user").ok_or(UNAVAILABLE)?;
            return Ok(Self {
                revision_of: Some(user.id.clone()),
                edit_entry: Some(user.native_entry_id.clone().ok_or(UNAVAILABLE)?),
                ..Self::default()
            });
        }
        if let Some(mid) = &request.regenerate_message_id {
            let assistant = find(mid, "assistant").ok_or(UNAVAILABLE)?;
            let entry = assistant.native_entry_id.clone().ok_or(UNAVAILABLE)?;
            let run = assistant.run_id.as_deref().ok_or(UNAVAILABLE)?;
            let user = record
                .messages
                .iter()
                .find(|m| m.role == "user" && m.run_id.as_deref() == Some(run))
                .ok_or(UNAVAILABLE)?;
            return Ok(Self {
                revision_of: Some(user.id.clone()),
                resent_text: Some(user.content.clone()),
                regenerate_entry: Some(entry),
                ..Self::default()
            });
        }
        Ok(Self::default())
    }
}

/// Save the new turn as the run registers: the user message, the empty
/// assistant message the stream fills, the title, and the running operation.
/// An edit or regenerate marks the replaced turns superseded.
fn publish_turn(
    record: &mut SessionRecord,
    control: &RunControl,
    request: &SendRequest,
    branch: &Branch,
    text: &str,
) -> Result<(), String> {
    store::ensure_new_operation(record, &request.operation_id)?;
    if let Some(mid) = &branch.revision_of {
        if let Some(index) = record.messages.iter().position(|m| &m.id == mid) {
            for message in &mut record.messages[index..] {
                message.superseded = true;
            }
        }
    }
    let at = now();
    let user_text = branch.resent_text.as_deref().unwrap_or(text);
    record.messages.push(Message {
        id: id(),
        role: "user".into(),
        content: user_text.into(),
        created_at: at,
        reasoning: None,
        status: "completed".into(),
        attachments: vec![],
        sources: vec![],
        superseded: false,
        revision_of: branch.revision_of.clone(),
        native_entry_id: None,
        run_id: Some(request.run_id.clone()),
    });
    record.messages.push(Message {
        id: control.message_id.clone(),
        role: "assistant".into(),
        content: String::new(),
        created_at: at,
        reasoning: None,
        status: "streaming".into(),
        attachments: vec![],
        sources: vec![],
        superseded: false,
        revision_of: request.regenerate_message_id.clone(),
        native_entry_id: None,
        run_id: Some(request.run_id.clone()),
    });
    if record.session.title.is_empty() {
        record.session.title = user_text.trim().chars().take(TITLE_CHARS).collect();
    }
    store::consume_draft(&mut record.session, request);
    record.session.updated_at = at;
    record.session.active_run = Some(RunRef {
        run_id: request.run_id.clone(),
        operation_id: request.operation_id.clone(),
    });
    record.operations.push(Operation {
        id: request.operation_id.clone(),
        run_id: request.run_id.clone(),
        status: OperationStatus::Running,
    });
    Ok(())
}
